//! Voxel 世界連線層（`/voxel/ws`·AI 生態世界 voxel 基底·切片①）。
//!
//! **刻意完全隔離**：用自己的 WS 路由、自己的玩家登錄（模組內 `OnceLock<VoxelHub>`），
//! **不碰 AppState / ws.rs / protocol.rs**——與現有 2D/3D 協定零交集，純 additive。
//! 沿用 axum 既有的 `WebSocketUpgrade` 基礎設施，但訊息型別全是 voxel 自己的（JSON）。
//!
//! 一條連線做三件事（用 mpsc 把所有出站訊息匯到單一 writer，避免兩處同時寫 socket）：
//! 1. 入場：分配 id、送 `welcome`、送出生點周邊的 chunk。
//! 2. 轉發：把 hub 廣播的「玩家位置快照」轉給此客戶端。
//! 3. 讀取：處理 `move`（更新並廣播）與 `req`（補送 chunk）。

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::voxel::{self, Block, ChunkCoord, WorldDelta, BASE_HEIGHT, CHUNK, SEA_LEVEL};

/// 入場時串給玩家的 chunk 半徑（以 chunk 為單位，水平）。3 → 7×7 column。
const SPAWN_CHUNK_RADIUS: i32 = 3;
/// 垂直 chunk 範圍（cy）。0..=1 覆蓋世界 Y 0..31，含所有地形高度。
const CY_MIN: i32 = 0;
const CY_MAX: i32 = 1;
/// 安全上限：單次 req 最多回幾個 chunk（擋惡意客戶端狂要）。
const MAX_REQ_CHUNKS: usize = 8;

/// 一名 voxel 玩家的權威狀態（位置 + 朝向）。
#[derive(Clone, Debug, Serialize)]
struct VoxelPlayer {
    id: Uuid,
    name: String,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
}

/// voxel 世界的多人 hub：玩家表 + 方塊改動 overlay + 廣播頻道。模組內全域單例（不污染 AppState）。
struct VoxelHub {
    players: RwLock<HashMap<Uuid, VoxelPlayer>>,
    /// 方塊改動 delta 層（疊在程序生成地形之上）。切片②先記憶體存，session 內正確套用+廣播。
    /// 之後切片可把它接 DB 持久化；AI 蓋家也會共用這層。
    deltas: RwLock<WorldDelta>,
    tx: broadcast::Sender<Arc<String>>,
}

static HUB: OnceLock<VoxelHub> = OnceLock::new();

fn hub() -> &'static VoxelHub {
    HUB.get_or_init(|| {
        let (tx, _rx) = broadcast::channel(256);
        VoxelHub {
            players: RwLock::new(HashMap::new()),
            deltas: RwLock::new(WorldDelta::new()),
            tx,
        }
    })
}

/// 目前所有玩家序列化成 `players` 訊息字串（廣播用）。鎖只在這裡短暫持有。
fn players_snapshot_json() -> String {
    let list: Vec<VoxelPlayer> = {
        let players = hub().players.read().unwrap();
        players.values().cloned().collect()
    };
    serde_json::json!({ "t": "players", "players": list }).to_string()
}

/// 廣播一次最新玩家快照給所有連線。
fn broadcast_players() {
    let snap = Arc::new(players_snapshot_json());
    // 沒有訂閱者時 send 會 Err，無所謂（單人在線）。
    let _ = hub().tx.send(snap);
}

// ── WS 協定（JSON，全是 voxel 自己的型別）──────────────────────────────────────

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum ClientMsg {
    /// 入場：帶顯示名（可選）。
    Join { name: Option<String> },
    /// 位置更新（前端權威預測，伺服器照收並廣播給別人；切片①不做伺服器端反作弊）。
    Move { x: f32, y: f32, z: f32, yaw: f32 },
    /// 走到新區塊時補要 chunk（cx,cz 為 chunk 座標，伺服器補該 column 的 cy 範圍）。
    Req { cx: i32, cz: i32 },
    /// 破壞方塊：目標方塊世界座標。伺服器驗證觸及範圍/實心後挖掉並廣播。
    Break { x: i32, y: i32, z: i32 },
    /// 放置方塊：放置世界座標 + 方塊型別 id（對齊 Block enum）。伺服器驗證後套用並廣播。
    Place { x: i32, y: i32, z: i32, b: u8 },
}

/// 出生點：從原點向外螺旋找第一塊「高於海平面的陸地」，站到地表上方，確保不卡水/土裡。
fn spawn_pos() -> (f32, f32, f32) {
    let (mut bx, mut bz, mut bh) = (0, 0, voxel::height_at(0, 0));
    'search: for r in 0..64_i32 {
        for dx in -r..=r {
            for dz in -r..=r {
                if dx.abs().max(dz.abs()) != r {
                    continue;
                }
                let h = voxel::height_at(dx, dz);
                if h > SEA_LEVEL + 1 {
                    bx = dx;
                    bz = dz;
                    bh = h;
                    break 'search;
                }
            }
        }
    }
    // 站在地表方塊「之上」：方塊 bh 頂面在 y=bh+1，多給 1 格餘裕讓重力落穩。
    (bx as f32 + 0.5, (bh + 2) as f32, bz as f32 + 0.5)
}

/// 收集一批 chunk（指定 column 清單 × cy 範圍），套用 delta overlay、略過全空氣的，打包成
/// `chunks` 訊息。套 delta → late-join 玩家也看得到別人改過的世界。
fn pack_chunks_msg(columns: &[(i32, i32)]) -> String {
    #[derive(Serialize)]
    struct PackedChunk {
        cx: i32,
        cy: i32,
        cz: i32,
        data: String,
    }
    let mut out: Vec<PackedChunk> = Vec::new();
    // 鎖只在這段短暫持有（讀 delta）；打包是純計算。
    let deltas = hub().deltas.read().unwrap();
    for &(cx, cz) in columns {
        for cy in CY_MIN..=CY_MAX {
            let coord = ChunkCoord { cx, cy, cz };
            if let Some(data) = voxel::pack_chunk_with_delta(coord, deltas.get(&coord)) {
                out.push(PackedChunk { cx, cy, cz, data });
            }
        }
    }
    serde_json::json!({ "t": "chunks", "chunks": out }).to_string()
}

/// 廣播一則方塊更新（破壞/放置後）給所有連線。前端據此只重建受影響的 chunk mesh。
fn broadcast_block(x: i32, y: i32, z: i32, b: Block) {
    let msg = Arc::new(
        serde_json::json!({ "t": "block", "x": x, "y": y, "z": z, "b": b as u8 }).to_string(),
    );
    let _ = hub().tx.send(msg);
}

/// 讀目前玩家位置（reach 驗證用）。找不到回 None。
fn player_pos(id: Uuid) -> Option<(f32, f32, f32)> {
    let players = hub().players.read().unwrap();
    players.get(&id).map(|p| (p.x, p.y, p.z))
}

pub async fn voxel_ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    // 與主 ws 一致的安全硬化：訊息上限 64 KiB（任何合法 voxel 訊息都遠小於此；
    // chunk 是「伺服器送出」不受此限）。
    const WS_MAX_MSG_BYTES: usize = 64 * 1024;
    ws.max_message_size(WS_MAX_MSG_BYTES)
        .max_frame_size(WS_MAX_MSG_BYTES)
        .on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let my_id = Uuid::new_v4();

    // 出站訊息統一走 mpsc → 單一 writer task，避免「轉發任務」與「讀取迴圈」同時寫 socket。
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(64);
    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // 等第一則 Join（也容忍客戶端直接開動：逾時/壞訊息就用預設名）。
    let mut name = String::from("旅人");
    if let Some(Ok(Message::Text(txt))) = receiver.next().await {
        if let Ok(ClientMsg::Join { name: n }) = serde_json::from_str::<ClientMsg>(&txt) {
            if let Some(n) = n {
                let cleaned: String = n.trim().chars().take(24).collect();
                if !cleaned.is_empty() {
                    name = cleaned;
                }
            }
        }
    } else {
        // 連線一開始就斷/非文字 → 收攤。
        writer.abort();
        return;
    }

    // 建立權威玩家、登錄進 hub。
    let (sx, sy, sz) = spawn_pos();
    {
        let mut players = hub().players.write().unwrap();
        players.insert(
            my_id,
            VoxelPlayer {
                id: my_id,
                name: name.clone(),
                x: sx,
                y: sy,
                z: sz,
                yaw: 0.0,
            },
        );
    }

    // 送 welcome（出生點 + 世界常數，前端據此設碰撞/相機）。
    let welcome = serde_json::json!({
        "t": "welcome",
        "id": my_id.to_string(),
        "name": name,
        "spawn": { "x": sx, "y": sy, "z": sz },
        "sea": SEA_LEVEL,
        "base": BASE_HEIGHT,
        "chunk": CHUNK,
    })
    .to_string();
    if out_tx.send(Message::Text(welcome)).await.is_err() {
        cleanup(my_id, &writer);
        return;
    }

    // 送出生點周邊 chunk。
    let mut columns = Vec::new();
    for cx in -SPAWN_CHUNK_RADIUS..=SPAWN_CHUNK_RADIUS {
        for cz in -SPAWN_CHUNK_RADIUS..=SPAWN_CHUNK_RADIUS {
            columns.push((cx, cz));
        }
    }
    let _ = out_tx.send(Message::Text(pack_chunks_msg(&columns))).await;

    // 廣播「有人來了」，並讓新玩家立刻拿到全場快照。
    broadcast_players();

    // 轉發任務：把 hub 廣播（玩家快照）丟進出站 mpsc。
    let mut rx = hub().tx.subscribe();
    let fwd_tx = out_tx.clone();
    let forward = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if fwd_tx.send(Message::Text((*msg).clone())).await.is_err() {
                        break;
                    }
                }
                // 跟不上廣播（Lagged）就略過繼續；頻道關了才停。
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // 讀取迴圈：處理 move / req。
    while let Some(Ok(msg)) = receiver.next().await {
        let txt = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            // ping/pong/binary 一律忽略（切片①只用文字 JSON）。
            _ => continue,
        };
        match serde_json::from_str::<ClientMsg>(&txt) {
            Ok(ClientMsg::Move { x, y, z, yaw }) => {
                let changed = {
                    let mut players = hub().players.write().unwrap();
                    if let Some(p) = players.get_mut(&my_id) {
                        p.x = x;
                        p.y = y;
                        p.z = z;
                        p.yaw = yaw;
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    broadcast_players();
                }
            }
            Ok(ClientMsg::Req { cx, cz }) => {
                // 補送單一 column（含安全上限的相鄰格，讓走動時邊界平滑補上）。
                let mut cols = Vec::new();
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        cols.push((cx + dx, cz + dz));
                        if cols.len() >= MAX_REQ_CHUNKS {
                            break;
                        }
                    }
                }
                let _ = out_tx.send(Message::Text(pack_chunks_msg(&cols))).await;
            }
            Ok(ClientMsg::Break { x, y, z }) => {
                // 取玩家位置驗 reach，驗目標實心，套 delta（覆蓋成空氣），廣播。
                let Some((px, py, pz)) = player_pos(my_id) else {
                    continue;
                };
                let applied = {
                    let mut world = hub().deltas.write().unwrap();
                    if voxel::can_break(&world, px, py, pz, x, y, z) {
                        voxel::set_block(&mut world, x, y, z, Block::Air);
                        true
                    } else {
                        false
                    }
                };
                if applied {
                    broadcast_block(x, y, z, Block::Air);
                }
            }
            Ok(ClientMsg::Place { x, y, z, b }) => {
                // 解析方塊型別 → 驗 reach/可放/目標為空 → 套 delta，廣播。
                let Some(block) = Block::from_u8(b) else {
                    continue;
                };
                let Some((px, py, pz)) = player_pos(my_id) else {
                    continue;
                };
                let applied = {
                    let mut world = hub().deltas.write().unwrap();
                    if voxel::can_place(&world, px, py, pz, x, y, z, block) {
                        voxel::set_block(&mut world, x, y, z, block);
                        true
                    } else {
                        false
                    }
                };
                if applied {
                    broadcast_block(x, y, z, block);
                }
            }
            // 重複 Join 或壞訊息：忽略。
            _ => {}
        }
    }

    // 收攤：移除玩家、廣播、收掉任務。
    forward.abort();
    cleanup(my_id, &writer);
    broadcast_players();
}

/// 把玩家移出登錄並中止 writer task。
fn cleanup(id: Uuid, writer: &tokio::task::JoinHandle<()>) {
    {
        let mut players = hub().players.write().unwrap();
        players.remove(&id);
    }
    writer.abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_is_above_dry_land() {
        let (x, y, z) = spawn_pos();
        let h = voxel::height_at(x.floor() as i32, z.floor() as i32);
        // 出生點必須在「高於海平面的陸地」之上（不卡土裡、不泡水裡）。
        assert!(h > SEA_LEVEL + 1, "出生點該在陸地：h={h}");
        assert!(y > h as f32, "出生點 Y 應在地表之上：y={y} h={h}");
    }

    #[test]
    fn pack_chunks_msg_is_valid_json_with_ground() {
        let msg = pack_chunks_msg(&[(0, 0)]);
        let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["t"], "chunks");
        // (0,0) column 的地面 chunk 一定在，不會空陣列。
        assert!(
            v["chunks"].as_array().unwrap().iter().any(|c| c["cy"] == 0),
            "應含地面 chunk"
        );
    }

    #[test]
    fn join_parses() {
        let m: ClientMsg = serde_json::from_str(r#"{"t":"join","name":"諾娃"}"#).unwrap();
        match m {
            ClientMsg::Join { name } => assert_eq!(name.as_deref(), Some("諾娃")),
            _ => panic!("應解析成 Join"),
        }
        // name 可省略。
        let m2: ClientMsg = serde_json::from_str(r#"{"t":"join"}"#).unwrap();
        assert!(matches!(m2, ClientMsg::Join { name: None }));
    }

    #[test]
    fn move_and_req_parse() {
        let m: ClientMsg =
            serde_json::from_str(r#"{"t":"move","x":1.5,"y":10.0,"z":-3.0,"yaw":0.7}"#).unwrap();
        assert!(matches!(m, ClientMsg::Move { .. }));
        let r: ClientMsg = serde_json::from_str(r#"{"t":"req","cx":2,"cz":-1}"#).unwrap();
        match r {
            ClientMsg::Req { cx, cz } => {
                assert_eq!(cx, 2);
                assert_eq!(cz, -1);
            }
            _ => panic!("應解析成 Req"),
        }
    }

    #[test]
    fn break_and_place_parse() {
        let b: ClientMsg = serde_json::from_str(r#"{"t":"break","x":3,"y":9,"z":-4}"#).unwrap();
        match b {
            ClientMsg::Break { x, y, z } => assert_eq!((x, y, z), (3, 9, -4)),
            _ => panic!("應解析成 Break"),
        }
        let p: ClientMsg =
            serde_json::from_str(r#"{"t":"place","x":1,"y":10,"z":2,"b":3}"#).unwrap();
        match p {
            ClientMsg::Place { x, y, z, b } => assert_eq!((x, y, z, b), (1, 10, 2, 3)),
            _ => panic!("應解析成 Place"),
        }
    }

    #[test]
    fn place_block_id_validates() {
        // 合法 id → Some；越界 → None（伺服器據此忽略 place）。
        assert_eq!(Block::from_u8(3), Some(Block::Stone));
        assert!(Block::from_u8(200).is_none());
    }
}
