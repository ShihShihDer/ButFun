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
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::npc_agent::{AgentAction, NearbyPlayer, SenseInput};
use crate::npc_agent_wire::{self, AgentBus};
use crate::resident_npc::ResidentPersona;
use crate::voxel::{self, Block, ChunkCoord, WorldDelta, BASE_HEIGHT, CHUNK, SEA_LEVEL};
use crate::voxel_residents::{self as vr, Body};

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

// ── 乙太方界 AI 居民（切片③）────────────────────────────────────────────────
//
// 讓「靈魂」也活在 voxel 新世界：幾位 AI 居民站在地形上（重力＋逐軸碰撞＋踏階，純物理
// 在 voxel_residents.rs）、會閒晃、被既有 npc_agent「腦袋」低頻驅動偶爾冒一句心裡話/心願。
// 採集/蓋家留切片④（禱告蓋家）。

/// 乙太方界居民人數（刻意少：成本鐵律 + FPS 鐵律，渲染負擔可忽略）。
const RESIDENT_COUNT: usize = 4;
/// 居民閒晃半徑下/上限（方塊）：挑下一個目標時離當前位置的距離區間。
const WANDER_MIN_R: f32 = 4.0;
const WANDER_MAX_R: f32 = 12.0;
/// 「腦袋」決策 MoveTo 的本地夾制半徑：LLM 可能回很遠（甚至 2D 世界尺度）的座標，
/// 只取其「方向」當閒晃指引、把落點夾在本地，避免居民瞬間跑到天邊。
const BRAIN_MOVE_CAP: f32 = 12.0;
/// 一句話/心願冒泡的顯示秒數。
const SAY_SECS: f32 = 6.0;
/// 居民 tick 頻率（秒）。10Hz：移動平滑、頻寬/CPU 都極小。
const RESIDENT_DT: f32 = 0.1;

/// 一位乙太方界居民的權威運行狀態（位置/朝向 + 閒晃目標 + 思考排程 + 當前冒的話）。
struct VoxelResident {
    /// 系統 id（"vox_res_0"…），voxel 模組內專用，與 2D 居民 id 體系無交集。
    id: String,
    name: &'static str,
    persona: ResidentPersona,
    body: Body,
    yaw: f32,
    /// 當前水平閒晃目標。
    target_x: f32,
    target_z: f32,
    /// 抵達目標後的小歇秒數（> 0 = 在歇、原地落重力）。
    wait_timer: f32,
    /// 下次思考倒數（秒）。
    think_timer: f32,
    /// 此刻冒泡的話（空 = 不冒泡）。
    say: String,
    /// 冒泡剩餘秒數。
    say_timer: f32,
}

/// 居民序列化視圖（廣播給客戶端渲染：位置/名字/朝向/說的話）。
#[derive(Serialize)]
struct ResidentView {
    id: String,
    name: &'static str,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    /// 當前冒泡的話（空字串 = 不顯示泡泡）。
    say: String,
}

/// 居民名字池（取自 resident_npc 的近城居民風格名，柔和轉寫式、與主要 NPC 一致）。
const RESIDENT_NAMES: [&str; RESIDENT_COUNT] = ["露娜", "諾娃", "賽勒", "奧瑞"];

/// 依 index 配 persona（讓「人設」字串有變化，純供 LLM 口吻；voxel 不沿用 2D 的閒晃邊界）。
fn persona_for(i: usize) -> ResidentPersona {
    match i % 4 {
        0 => ResidentPersona::MarketBrowser,
        1 => ResidentPersona::FarmWorker,
        2 => ResidentPersona::TownSquare,
        _ => ResidentPersona::Wanderer,
    }
}

/// 初始化 N 位居民：環狀散在出生點周邊的乾地上，各自站穩。
fn init_residents() -> Vec<VoxelResident> {
    let mut out = Vec::with_capacity(RESIDENT_COUNT);
    for i in 0..RESIDENT_COUNT {
        // 環狀散開：每位一個方位 + 固定半徑，dry_ground_spawn 會就近找乾地站穩。
        let angle = (i as f32) / (RESIDENT_COUNT as f32) * std::f32::consts::TAU;
        let ox = (angle.cos() * 8.0).round() as i32;
        let oz = (angle.sin() * 8.0).round() as i32;
        let body = vr::dry_ground_spawn(ox, oz);
        out.push(VoxelResident {
            id: format!("vox_res_{i}"),
            name: RESIDENT_NAMES[i],
            persona: persona_for(i),
            target_x: body.x,
            target_z: body.z,
            yaw: 0.0,
            body,
            // 入場錯開首次思考，避免 N 位同一 tick 一起打 LLM。
            wait_timer: 0.5 + i as f32 * 0.5,
            think_timer: 3.0 + i as f32 * 2.0,
            say: String::new(),
            say_timer: 0.0,
        });
    }
    out
}

/// voxel 世界的多人 hub：玩家表 + 方塊改動 overlay + 廣播頻道 + AI 居民 + 決策匯流排。
/// 模組內全域單例（不污染 AppState）。
struct VoxelHub {
    players: RwLock<HashMap<Uuid, VoxelPlayer>>,
    /// 方塊改動 delta 層（疊在程序生成地形之上）。切片②先記憶體存，session 內正確套用+廣播。
    /// 之後切片可把它接 DB 持久化；AI 蓋家也會共用這層。
    deltas: RwLock<WorldDelta>,
    /// 乙太方界 AI 居民。
    residents: RwLock<Vec<VoxelResident>>,
    /// 居民決策匯流排（async 思考投入、tick 取走套用；嚴守無鎖 await 鐵律）。
    agent_bus: AgentBus,
    tx: broadcast::Sender<Arc<String>>,
}

static HUB: OnceLock<VoxelHub> = OnceLock::new();

fn hub() -> &'static VoxelHub {
    HUB.get_or_init(|| {
        let (tx, _rx) = broadcast::channel(256);
        VoxelHub {
            players: RwLock::new(HashMap::new()),
            deltas: RwLock::new(WorldDelta::new()),
            residents: RwLock::new(init_residents()),
            agent_bus: AgentBus::new(),
            tx,
        }
    })
}

/// 目前所有玩家 + 居民序列化成 `players` 訊息字串（廣播用）。
/// 兩把鎖**循序**取放（讀完玩家 drop 再讀居民），不巢狀、不跨 await，守鎖紀律。
fn players_snapshot_json() -> String {
    let players: Vec<VoxelPlayer> = {
        let p = hub().players.read().unwrap();
        p.values().cloned().collect()
    }; // 玩家讀鎖在此釋放
    let residents: Vec<ResidentView> = {
        let rs = hub().residents.read().unwrap();
        rs.iter()
            .map(|r| ResidentView {
                id: r.id.clone(),
                name: r.name,
                x: r.body.x,
                y: r.body.y,
                z: r.body.z,
                yaw: r.yaw,
                say: r.say.clone(),
            })
            .collect()
    }; // 居民讀鎖在此釋放
    serde_json::json!({ "t": "players", "players": players, "residents": residents }).to_string()
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

// ── 居民 tick 迴圈（切片③）──────────────────────────────────────────────────
//
// 嚴守 prod 死鎖鐵律：物理/套用決策全在**同步、短鎖、不 await**的段落；思考一律
// 「短鎖快照 → drop → spawn async → 下一 tick 用 AgentBus 套用」，絕不持鎖 await。

/// 啟動乙太方界居民 tick 迴圈（main.rs 啟動時呼叫一次）。10Hz。
pub fn spawn_residents() {
    tokio::spawn(async move {
        // 觸發 hub 初始化（建出居民），並開一個 10Hz 節拍。
        let _ = hub();
        let mut ticker = tokio::time::interval(Duration::from_secs_f32(RESIDENT_DT));
        loop {
            ticker.tick().await;
            tick_residents(RESIDENT_DT);
        }
    });
}

/// 一次居民世界推進：套用上輪思考的決策 → 物理/閒晃 → 廣播 → 排程新一輪思考（無鎖 spawn）。
fn tick_residents(dt: f32) {
    // 1) 先取走上輪 async 思考投回的決策（短鎖、不 await）。
    let decisions = hub().agent_bus.drain();

    // 2) 同步推進：套決策 + 物理/閒晃。deltas(read) + residents(write) 都只在這段持有、不 await。
    //    需要思考的居民這裡只蒐集「快照」，spawn 留到鎖釋放後。
    let mut think_jobs: Vec<(String, &'static str, ResidentPersona, f32, f32)> = Vec::new();
    {
        let world = hub().deltas.read().unwrap();
        let mut residents = hub().residents.write().unwrap();

        // 2a) 套用決策：MoveTo 夾成本地閒晃目標；say 非空 → 冒泡（其餘 action 不打斷閒晃）。
        for (rid, dec) in &decisions {
            if let Some(r) = residents.iter_mut().find(|r| &r.id == rid) {
                if let AgentAction::MoveTo { x, y } = dec.action {
                    let dx = x - r.body.x;
                    let dz = y - r.body.z;
                    let d = (dx * dx + dz * dz).sqrt().max(0.001);
                    let cap = BRAIN_MOVE_CAP.min(d);
                    r.target_x = r.body.x + dx / d * cap;
                    r.target_z = r.body.z + dz / d * cap;
                    r.wait_timer = 0.0;
                }
                let say = dec.say.trim();
                if !say.is_empty() {
                    r.say = say.chars().take(40).collect();
                    r.say_timer = SAY_SECS;
                }
            }
        }

        // 2b) 物理 + 閒晃 + 思考排程。
        for r in residents.iter_mut() {
            // 冒泡倒數。
            if r.say_timer > 0.0 {
                r.say_timer -= dt;
                if r.say_timer <= 0.0 {
                    r.say.clear();
                }
            }

            if r.wait_timer > 0.0 {
                // 小歇：原地落重力（站穩、不亂飄）。
                r.wait_timer -= dt;
                vr::gravity_step(&world, &mut r.body, dt);
            } else {
                let (bx, bz) = (r.body.x, r.body.z);
                let reached = vr::step_toward(&world, &mut r.body, r.target_x, r.target_z, dt);
                if let Some(yaw) = vr::yaw_from_move(r.body.x - bx, r.body.z - bz) {
                    r.yaw = yaw;
                }
                if reached {
                    // 挑下一個閒晃目標 + 小歇片刻。
                    let angle = rand::random::<f32>() * std::f32::consts::TAU;
                    let radius = WANDER_MIN_R + rand::random::<f32>() * (WANDER_MAX_R - WANDER_MIN_R);
                    let (tx, tz) = vr::wander_target(r.body.x, r.body.z, angle, radius);
                    r.target_x = tx;
                    r.target_z = tz;
                    r.wait_timer = 1.0 + rand::random::<f32>() * 3.0;
                }
            }

            // 思考排程（蒐集快照，spawn 留到鎖外）。
            r.think_timer -= dt;
            if r.think_timer <= 0.0 {
                r.think_timer = npc_agent_wire::THINK_INTERVAL_SECS;
                think_jobs.push((r.id.clone(), r.name, r.persona, r.body.x, r.body.z));
            }
        }
    } // deltas/residents 鎖在此一併釋放

    // 3) 廣播最新快照（含居民位置/名字/說的話）。
    broadcast_players();

    // 4) 無鎖 spawn 思考（LLM）。整個 agent 思考可由 BUTFUN_NPC_AGENT=0 關掉，
    //    關掉後居民仍照常閒晃移動，只是不冒 LLM 心裡話/心願（零額外成本）。
    if npc_agent_wire::agents_enabled() {
        for (id, name, persona, x, z) in think_jobs {
            spawn_resident_think(id, name, persona, x, z);
        }
    }
}

/// 為一位居民發起一次無鎖 async 思考：短鎖讀附近玩家 → drop → spawn → npc_think/npc_pray
/// → 把決策投進 AgentBus（下一 tick 套用）。比照 game.rs npc_agent_wire 的做法，全程不持遊戲狀態鎖。
fn spawn_resident_think(id: String, name: &'static str, persona: ResidentPersona, x: f32, z: f32) {
    // 防重入：上一輪思考還沒回來就先不發新的（LLM 逾時可能 > 思考間隔）。
    if !hub().agent_bus.try_begin_thinking(&id) {
        return;
    }
    // 短鎖讀附近玩家快照（把 voxel 的 z 當成 SenseInput 的 y——prompt 只用座標當情境）。
    let nearby_players: Vec<NearbyPlayer> = {
        let players = hub().players.read().unwrap();
        players
            .values()
            .map(|p| NearbyPlayer { name: p.name.clone(), x: p.x, y: p.z })
            .collect()
    }; // 讀鎖在此釋放
    let sense = SenseInput {
        x,
        y: z,
        hp: 100,
        max_hp: 100,
        energy: 80,
        mood: 70,
        needs_summary: String::new(),
        nearby_players,
        nearby_nodes: Vec::new(),
        world_news: "你生活在新生的『乙太方界』——一片由方塊構成的清淨天地。".to_string(),
    };
    let persona_str = npc_agent_wire::resident_agent_persona(name, persona);
    let resident_name = name.to_string();
    tokio::spawn(async move {
        // npc_think 內部：有 LLM 走 LLM、沒有就走罐頭規則，永遠回得出決策、不 panic。
        let decision = crate::npc_agent::npc_think(&sense, &persona_str).await;
        // 向後相容：模型偶爾在決策 JSON 主動給心願就當 bonus 落地。
        if let Some(prayer) = &decision.prayer {
            crate::npc_agent::append_prayer(&resident_name, prayer);
        }
        hub().agent_bus.push_decision(id.clone(), decision);
        hub().agent_bus.end_thinking(&id);

        // 居民禱告（獨立生成、機率節流）：成功就落地 data/prayers.jsonl，並冒一句心願泡泡。
        let pray_roll: f64 = rand::random();
        if crate::npc_agent::should_pray(pray_roll) {
            if let Some(prayer) = crate::npc_agent::npc_pray(&sense, &persona_str).await {
                crate::npc_agent::append_prayer(&resident_name, &prayer);
                // 把心願當「說的話」冒泡（💭 前綴與一般對白區隔）。Idle action 不會打斷閒晃。
                hub().agent_bus.push_decision(
                    id.clone(),
                    crate::npc_agent::AgentDecision::new(
                        AgentAction::Idle,
                        format!("💭 {prayer}"),
                        "心願",
                    ),
                );
            }
        }
    });
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
