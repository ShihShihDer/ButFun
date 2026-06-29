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

use crate::npc_agent::{AgentAction, AgentDecision, NearbyPlayer, SenseInput};
use crate::npc_agent_wire::{self, AgentBus};
use crate::resident_npc::ResidentPersona;
use crate::voxel::{self, Block, ChunkCoord, WorldDelta, BASE_HEIGHT, CHUNK, SEA_LEVEL};
use crate::voxel_building::{self as vbuild, BuildStore};
use crate::voxel_desires::{self as vdes, DesireStore};
use crate::voxel_diary;
use crate::voxel_memory::{self as vmem, VoxelMemory};
use crate::voxel_relations::{self as vrel, SocialStore};
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

// ── 玩家↔居民對話（切片：點居民聊天）──────────────────────────────────────────
/// 玩家送來的單句對話長度上限（字元）。超過就截斷，擋惡意灌爆 prompt。
const TALK_MAX_CHARS: usize = 200;
/// 居民回覆長度上限（字元）：避免 LLM 偶爾長篇大論塞爆前端對話框。
const TALK_REPLY_MAX_CHARS: usize = 300;
/// 每條連線的對話冷卻（毫秒）：防單人狂送吃爆 LLM 額度（比照 npc_chat 的 per-player 冷卻）。
const TALK_COOLDOWN_MS: u64 = 4000;
/// Talk 路徑的 LLM 整體逾時（秒）：`llm_chat_fast` 每 tier 5-8 秒，四 tier 最差 ~23 秒；
/// 此值作為最後安全網，確保玩家不會永遠等不到回覆。
const TALK_LLM_TIMEOUT_SECS: u64 = 25;
/// 居民「主動招呼」觸發距離（方塊）：玩家靠到這麼近，居民偶爾冒一句招呼。
const GREET_DIST: f32 = 4.0;
/// 招呼冷卻（秒）：冒過一次招呼後要等這麼久才會再冒，避免洗版。
const GREET_COOLDOWN: f32 = 25.0;
/// 每個合格 tick 觸發招呼的機率（10Hz 下 0.04 ≈ 靠近後約 2.5 秒內冒一句）。
const GREET_CHANCE_PER_TICK: f32 = 0.04;
/// 居民建造頻率：每隔這麼多秒放一塊方塊（慢節奏，讓玩家能目睹過程）。
const BUILD_INTERVAL_SECS: f32 = 8.0;

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
    /// 主動招呼冷卻倒數（秒）：> 0 表示最近招呼過、暫不再冒，避免洗版。
    greet_timer: f32,
    /// 居民↔居民社交冷卻倒數（秒）：> 0 表示最近主動搭話過另一位居民，尚不可再發起。
    social_cooldown: f32,
    /// 另一位居民剛搭話，等這秒數到期後回應（id, 名字, 剩餘秒）。
    pending_response: Option<(String, String, f32)>,
    /// 建造 tick 倒數（秒）：降到 0 時嘗試放一塊或啟動新計畫；錯開避免同 tick 全員觸發。
    build_tick: f32,
}

/// 居民序列化視圖（廣播給客戶端渲染：位置/名字/朝向/說的話/當前心願）。
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
    /// 居民當前的心願（None / 省略 = 尚未有心願）。前端據此顯示夢想副標籤。
    #[serde(skip_serializing_if = "Option::is_none")]
    desire: Option<String>,
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

// ── 對話 / 招呼純邏輯（抽成可測函式，碰不到 hub / 鎖 / LLM）─────────────────────

/// 清洗玩家送來的對話文字：trim、空字串拒絕（回 None）、超長截斷到 `TALK_MAX_CHARS`。
/// 純函式：路由前的驗證，方便單元測試釘住。
fn sanitize_talk_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(TALK_MAX_CHARS).collect())
}

/// 對話冷卻判定：距離上次說話已過 `elapsed_ms` 毫秒，是否允許這次（≥ `TALK_COOLDOWN_MS`）。
fn talk_cooldown_ok(elapsed_ms: u64) -> bool {
    elapsed_ms >= TALK_COOLDOWN_MS
}

/// 組對話用 system prompt：複用居民 agent 人設字串（`resident_agent_persona`），
/// 再補上「身處乙太方界、有旅人來搭話、請自然回應」的語境與口吻約束。
/// `desire` 非空時把心願注入——居民帶著夢想和玩家對話，玩家能感受到「你的話種下了什麼」。
fn resident_talk_system_prompt(name: &str, persona: ResidentPersona, desire: Option<&str>) -> String {
    let base = npc_agent_wire::resident_agent_persona(name, persona);
    let desire_note = desire
        .map(|d| {
            format!(
                "\n你目前有個心願：「{d}」——這是旅人的話在你心裡種下的夢想，對話中可以自然地提到它、\
                流露你對它的期待，但不要刻意強調，自然融入即可。"
            )
        })
        .unwrap_or_default();
    format!(
        "{base}\n\n你現在身處『乙太方界』——一片由方塊構成、寧靜清新的新生天地，你是這裡的居民。{desire_note}\
        此刻有一位來訪的旅人向你搭話。請以你的身份、用繁體中文自然回應，1 到 2 句、口吻溫暖親切，\
        可以聊聊你在這片方塊天地裡的生活或當下的心情；絕不跳出角色，也不要提到你是 AI 或語言模型。"
    )
}

/// 居民對話罐頭回覆（LLM 未啟用 / 連不到時的降級，永遠回得出一句）。依名字雜湊選句、增加變化。
fn resident_canned_reply(name: &str) -> String {
    const POOL: [&str; 4] = [
        "你好呀，旅人！在這片方塊天地裡走走，感覺很不一樣吧？",
        "嗨，歡迎來到乙太方界。我正四處晃晃，你也是來看看的嗎？",
        "見到你真好。這裡很安靜，但住久了會慢慢喜歡上的。",
        "你好！要不要一起在這片新生的天地裡逛逛？",
    ];
    let idx = name.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize)) % POOL.len();
    POOL[idx].to_string()
}

/// 居民主動招呼句池：靠近時冒一句（用既有泡泡，低頻、簡短）。依索引選句。
fn greeting_line(n: usize) -> &'static str {
    const GREETINGS: [&str; 4] = ["你來啦！", "嗨，旅人～", "哦，有客人。", "你好呀！"];
    GREETINGS[n % GREETINGS.len()]
}

/// 一組玩家水平座標中，離 (rx,rz) 最近者的平方距離。沒有玩家回 None。純函式、可測。
fn nearest_player_dist_sq(rx: f32, rz: f32, players: &[(f32, f32)]) -> Option<f32> {
    players
        .iter()
        .map(|&(px, pz)| {
            let dx = px - rx;
            let dz = pz - rz;
            dx * dx + dz * dz
        })
        .fold(None, |acc, d| Some(acc.map_or(d, |a: f32| a.min(d))))
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
            greet_timer: 0.0,
            // 錯開初始社交冷卻，避免啟動瞬間全員一起嘗試搭話。
            social_cooldown: i as f32 * 20.0,
            pending_response: None,
            // 錯開建造 tick，讓 4 位居民不同 tick 檢查（BUILD_INTERVAL_SECS / 4 * i 間距）。
            build_tick: BUILD_INTERVAL_SECS * 0.5 + i as f32 * (BUILD_INTERVAL_SECS / 4.0),
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
    /// 居民記憶（短期對話歷史 + 長期累積記憶）。短鎖讀寫、絕不持鎖 await，
    /// 摘要/LLM 一律在無鎖 async task；長期記憶持久化到 data/voxel_memory.jsonl。
    memory: RwLock<VoxelMemory>,
    /// 居民渴望（每居民一個「當前心願」）。短鎖讀寫、持久化到 data/voxel_desires.jsonl。
    /// 玩家對話讓居民萌生心願 → 驅動後續思考與對話（記憶驅動行為 v1）。
    desires: RwLock<DesireStore>,
    /// 居民社交記憶（誰聽到誰說了什麼）。持久化到 data/voxel_social.jsonl。
    /// 居民↔居民偶爾對話 → 雙方存入社交記憶 → think 時帶入 world_news → 自然提及彼此。
    social: RwLock<SocialStore>,
    /// 居民建造計畫（每人至多一份 active plan）。持久化到 data/voxel_builds.jsonl。
    /// 居民有心願後 → 分類 → 生成方塊清單 → 每 8 秒放一塊（渴望化為方塊 v1）。
    builds: RwLock<BuildStore>,
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
            // 啟動時從 data/voxel_memory.jsonl 載回長期記憶（檔缺 = 首次啟動，回空），
            // 重啟後居民仍記得跟誰聊過、聊到什麼。
            memory: RwLock::new(VoxelMemory::from_entries(vmem::load_memories())),
            // 啟動時從 data/voxel_desires.jsonl 載回居民心願（重啟後仍記得心願）。
            desires: RwLock::new(DesireStore::from_entries(vdes::load_desires())),
            // 啟動時從 data/voxel_social.jsonl 載回居民社交記憶（重啟後仍記得聽到過什麼）。
            social: RwLock::new(SocialStore::from_entries(vrel::load_social())),
            // 啟動時從 data/voxel_builds.jsonl 載回未完成的建造計畫（重啟後繼續蓋）。
            builds: RwLock::new(BuildStore::from_entries(vbuild::load_builds())),
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
    // 先讀居民快照（drop）→ 再讀心願（drop）→ 組合成 ResidentView，嚴守循序取鎖、不巢狀。
    let resident_snaps: Vec<(String, &'static str, f32, f32, f32, f32, String)> = {
        let rs = hub().residents.read().unwrap();
        rs.iter()
            .map(|r| (r.id.clone(), r.name, r.body.x, r.body.y, r.body.z, r.yaw, r.say.clone()))
            .collect()
    }; // 居民讀鎖在此釋放
    let residents: Vec<ResidentView> = {
        let des = hub().desires.read().unwrap();
        resident_snaps
            .into_iter()
            .map(|(id, name, x, y, z, yaw, say)| ResidentView {
                desire: des.get_desire(&id).map(|d| d.desire.clone()),
                id,
                name,
                x,
                y,
                z,
                yaw,
                say,
            })
            .collect()
    }; // 心願讀鎖在此釋放
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
    /// 跟居民對話：指定居民 id + 玩家說的話。伺服器以該居民人設呼叫 LLM 對話路徑，
    /// 回 `talk`（單播）給玩家、並冒泡讓附近人看到。
    Talk { resident_id: String, text: String },
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

    // 對話冷卻：記這條連線上次跟居民說話的時刻（per-connection 節流，防灌爆 LLM）。
    let mut last_talk: Option<std::time::Instant> = None;

    // 讀取迴圈：處理 move / req / break / place / talk。
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
            Ok(ClientMsg::Talk { resident_id, text }) => {
                // 1) 驗證 + 清洗文字（空 / 純空白 → 忽略；超長截斷）。
                let Some(clean) = sanitize_talk_text(&text) else {
                    continue;
                };
                // 2) per-connection 冷卻：太頻繁就忽略（保護免費 LLM 額度）。
                let now = std::time::Instant::now();
                if let Some(prev) = last_talk {
                    if !talk_cooldown_ok(now.duration_since(prev).as_millis() as u64) {
                        continue;
                    }
                }
                // 3) 短鎖取居民人設快照 → drop（ResidentPersona 是 Copy，name 是 'static）。
                //    絕不持鎖 await，比照 spawn_resident_think / TalkToNpc 的鎖紀律。
                let snap: Option<(&'static str, ResidentPersona)> = {
                    let residents = hub().residents.read().unwrap();
                    residents
                        .iter()
                        .find(|r| r.id == resident_id)
                        .map(|r| (r.name, r.persona))
                }; // 讀鎖在此釋放
                let Some((rname, rpersona)) = snap else {
                    continue; // 沒這位居民 → 忽略
                };
                last_talk = Some(now);
                // 3b) 玩家身份鍵：用顯示名辨識（登入帳號未接前，訪客以顯示名跨 session 記憶）。
                let player_key = name.clone();
                // 3c) 短鎖讀記憶 → 組「脈絡區塊」（近期對話 + 關於這位玩家的長期記憶），
                //     立刻 drop 鎖、產出 owned String 帶進無鎖 async task → 居民「記得你」。
                let context = {
                    let mem = hub().memory.read().unwrap();
                    let history = mem.recent_dialogue(&player_key, &resident_id);
                    let memories = mem.recall(&resident_id, &player_key, vmem::RECALL_LIMIT);
                    vmem::build_context_block(&history, &memories, &player_key)
                }; // 記憶讀鎖在此釋放（絕不持鎖 await）
                // 3d) 短鎖讀居民當前心願 → drop（帶進 prompt 讓居民「帶著夢想說話」）。
                let current_desire: Option<String> = {
                    let des = hub().desires.read().unwrap();
                    des.get_desire(&resident_id).map(|d| d.desire.clone())
                }; // 心願讀鎖在此釋放
                // 4a) 立即送「思考中」佔位 → 前端用 `thinking:true` 顯示動畫指示器，
                //     不當成一般回覆氣泡顯示，避免「居民只回 …」的誤解。
                //     此訊息為私聊（只送這位玩家），不走 AgentBus 冒泡。
                let ack = serde_json::json!({
                    "t": "talk",
                    "resident_id": &resident_id,
                    "name": rname,
                    "reply": "…",
                    "thinking": true,  // 前端判斷旗標：顯示動畫指示器而非一般氣泡
                })
                .to_string();
                let _ = out_tx.send(Message::Text(ack)).await;

                // 4b) 無鎖 async task：呼叫快速 LLM 路徑（每 tier 5-8 秒逾時，最差 ~23 秒）
                //     取代舊的 raw_llm_call（每 tier 最多 15-20 秒，Cerebras+Gemini 掛著時合計 35 秒）。
                //     取得回覆後單播給這位玩家，並投 AgentBus 讓居民冒泡（附近人看得到）。
                let reply_tx = out_tx.clone();
                tokio::spawn(async move {
                    // 把「脈絡區塊」附到人設後 → 居民帶著記憶/對話歷史/心願回話。
                    let base_sys = resident_talk_system_prompt(rname, rpersona, current_desire.as_deref());
                    let sys = if context.is_empty() {
                        base_sys
                    } else {
                        format!("{base_sys}\n\n{context}")
                    };
                    // raw_llm_call_fast：每 tier 縮短逾時；外層再加 TALK_LLM_TIMEOUT_SECS 安全網。
                    let reply: String = match tokio::time::timeout(
                        Duration::from_secs(TALK_LLM_TIMEOUT_SECS),
                        crate::npc_chat::raw_llm_call_fast(&sys, &clean),
                    )
                    .await
                    {
                        Ok(Some(t)) => t.chars().take(TALK_REPLY_MAX_CHARS).collect(),
                        _ => resident_canned_reply(rname), // 逾時 / LLM 未啟用 / 全失敗 → 罐頭後備
                    };
                    let msg = serde_json::json!({
                        "t": "talk",
                        "resident_id": resident_id,
                        "name": rname,
                        "reply": reply,
                    })
                    .to_string();
                    let _ = reply_tx.send(Message::Text(msg)).await;

                    // 記憶寫入（短鎖、不 await）：
                    // 1) 短期對話歷史記一輪 → 下一句對話帶得上脈絡（同段對話連貫）。
                    // 2) 規則摘要這次互動 → 存進該居民的長期記憶 + 落地 jsonl（重啟後仍記得）。
                    let new_memory = {
                        let mut mem = hub().memory.write().unwrap();
                        mem.record_turn(&player_key, &resident_id, &clean, &reply);
                        vmem::summarize_exchange(&player_key, &clean)
                            .map(|summary| mem.add_memory(&resident_id, &player_key, &summary))
                    }; // 記憶寫鎖在此釋放
                    if let Some(entry) = new_memory {
                        // append-only 小檔寫（同步、不持鎖）：絕不覆寫/刪除既有記憶。
                        vmem::append_memory(&entry);
                    }
                    // 心願萃取（記憶驅動行為 v1）：若居民的 LLM 回覆中浮現「我想…」等心願模式，
                    // 更新居民的當前心願 → 後續思考與對話都帶著這個夢想（記憶驅動行為）。
                    // 規則擷取：零 LLM 成本、確定性、可測。
                    if let Some(desire_text) = vdes::extract_desire(&reply) {
                        let new_desire = {
                            let mut des = hub().desires.write().unwrap();
                            des.set_desire(&resident_id, &desire_text, &player_key)
                        }; // 心願寫鎖在此釋放
                        // append-only 小檔寫（同步、不持鎖）：重啟後仍記得心願。
                        vdes::append_desire(&new_desire);
                    }
                    // 冒泡（下一 tick 由 tick_residents 套用 say，自動截到 40 字、計時消失）。
                    hub().agent_bus.push_decision(
                        resident_id,
                        AgentDecision::new(AgentAction::Idle, reply, "對話"),
                    );
                });
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

/// `GET /voxel/diary` — 回傳所有居民的日記頁（記憶摘要 + 當前心願）。
/// 短鎖讀取快照 → drop 鎖 → 格式化 → 回 JSON；零 LLM、零持久化、零 migration。
/// 呼叫端（瀏覽器）直接 `fetch("/voxel/diary")` 即可，無需任何認證。
pub async fn voxel_diary_handler() -> axum::response::Response {
    use axum::http::header;

    // 1) 短鎖快照居民 id/name → drop（循序取鎖、不巢狀、守鎖紀律）。
    let resident_ids: Vec<(String, &'static str)> = {
        let rs = hub().residents.read().unwrap();
        rs.iter().map(|r| (r.id.clone(), r.name)).collect()
    };

    // 2) 短鎖快照全部長期記憶（每位）→ drop。
    let all_memories: Vec<(String, Vec<crate::voxel_memory::MemoryEntry>)> = {
        let mem = hub().memory.read().unwrap();
        resident_ids
            .iter()
            .map(|(id, _)| (id.clone(), mem.all_memories_for(id)))
            .collect()
    };

    // 3) 短鎖快照心願 → drop。
    let desires: Vec<Option<String>> = {
        let des = hub().desires.read().unwrap();
        resident_ids.iter().map(|(id, _)| des.get_desire(id).map(|d| d.desire.clone())).collect()
    };

    // 4) 純函式格式化（無鎖、確定性）。
    let pages: Vec<voxel_diary::DiaryPage> = resident_ids
        .iter()
        .zip(all_memories.iter())
        .zip(desires.iter())
        .map(|(((id, name), (_, mems)), desire)| {
            voxel_diary::format_diary_page(id, name, desire.as_deref(), mems)
        })
        .collect();

    let body = serde_json::to_string(&pages).unwrap_or_else(|_| "[]".into());
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// 一次居民世界推進：套用上輪思考的決策 → 物理/閒晃 → 社交互動 → 廣播 → 排程新一輪思考。
fn tick_residents(dt: f32) {
    // 1) 先取走上輪 async 思考投回的決策（短鎖、不 await）。
    let decisions = hub().agent_bus.drain();

    // 2) 同步推進：套決策 + 物理/閒晃。deltas(read) + residents(write) 都只在這段持有、不 await。
    //    需要思考的居民這裡只蒐集「快照」，spawn 留到鎖釋放後。
    let mut think_jobs: Vec<(String, &'static str, ResidentPersona, f32, f32)> = Vec::new();

    // 主動招呼用：先短鎖快照所有玩家水平座標 → drop（循序取放、不與居民鎖巢狀，守鎖紀律）。
    let player_pts: Vec<(f32, f32)> = {
        let players = hub().players.read().unwrap();
        players.values().map(|p| (p.x, p.z)).collect()
    }; // 玩家讀鎖在此釋放

    // 社交對話生成用：快照所有居民心願（先 drop desires 鎖，再取居民寫鎖，守循序不巢狀鐵律）。
    // 居民 id 格式固定為 "vox_res_{i}"，直接枚舉取（不需先讀居民清單）。
    let desire_snaps: HashMap<String, String> = {
        let des = hub().desires.read().unwrap();
        (0..RESIDENT_COUNT)
            .filter_map(|i| {
                let id = format!("vox_res_{i}");
                des.get_desire(&id).map(|d| (id, d.desire.clone()))
            })
            .collect()
    }; // desires 讀鎖在此釋放

    // 社交事件（鎖內收集，鎖外落地記憶）。
    // 格式：(initiator_id, initiator_name, target_id, target_name, line, is_response)
    // is_response=false → 發起對話；is_response=true → 回應對話。
    let mut social_events: Vec<(String, String, String, String, String, bool)> = Vec::new();

    // 建造候選（鎖內收集位置快照，鎖外執行放塊 / 啟動計畫）。
    // 格式：(resident_id, resident_name, wx, wy, wz, resident_idx)
    let mut build_candidates: Vec<(String, &'static str, i32, i32, i32, usize)> = Vec::new();

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

        // 2b) 物理 + 閒晃 + 社交冷卻 + 思考排程。
        for r in residents.iter_mut() {
            // 冒泡倒數。
            if r.say_timer > 0.0 {
                r.say_timer -= dt;
                if r.say_timer <= 0.0 {
                    r.say.clear();
                }
            }

            // 社交冷卻倒數。
            if r.social_cooldown > 0.0 {
                r.social_cooldown -= dt;
            }

            // 待回應倒數：另一位居民搭話後，延遲幾秒再自然回應（零 LLM、程式化台詞）。
            let resp_ready = match &mut r.pending_response {
                Some((_, _, cd)) => {
                    *cd -= dt;
                    *cd <= 0.0
                }
                None => false,
            };
            if resp_ready && r.say.is_empty() {
                if let Some((init_id, init_name, _)) = r.pending_response.take() {
                    let resp = vrel::resident_social_response(r.name, &init_name);
                    let safe: String = resp.chars().take(vrel::SOCIAL_SAY_CHARS).collect();
                    social_events.push((
                        r.id.clone(), r.name.to_string(),
                        init_id, init_name,
                        safe.clone(), true, // is_response
                    ));
                    r.say = safe;
                    r.say_timer = SAY_SECS;
                }
            }

            // 主動招呼：招呼冷卻倒數；冷卻完、目前沒在說話、且有玩家靠很近時，
            // 偶爾（低機率）冒一句招呼，讓世界更有人氣（用既有泡泡、低頻不洗版）。
            if r.greet_timer > 0.0 {
                r.greet_timer -= dt;
            } else if r.say.is_empty() {
                if let Some(d2) = nearest_player_dist_sq(r.body.x, r.body.z, &player_pts) {
                    if d2 < GREET_DIST * GREET_DIST && rand::random::<f32>() < GREET_CHANCE_PER_TICK {
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                        r.say = greeting_line(pick).to_string();
                        r.say_timer = SAY_SECS;
                        r.greet_timer = GREET_COOLDOWN;
                    }
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

            // 建造 tick 倒數；到期則加入候選（只收快照，實際放塊在鎖外執行）。
            r.build_tick -= dt;
            if r.build_tick <= 0.0 {
                // 居民 id 格式固定 "vox_res_{i}"，取末位數字當 index。
                let idx = r.id.trim_start_matches("vox_res_").parse::<usize>().unwrap_or(0);
                build_candidates.push((
                    r.id.clone(),
                    r.name,
                    r.body.x.floor() as i32,
                    r.body.y.floor() as i32,
                    r.body.z.floor() as i32,
                    idx,
                ));
            }
        }

        // 2c) 社交發起掃描：每 tick 最多一對居民發起對話（低頻、有冷卻、不干擾物理主迴圈）。
        // 先收集快照（idx, id, name, x, z, social_cooldown, is_saying）避免借用衝突。
        let snaps: Vec<(usize, String, &'static str, f32, f32, f32, bool)> =
            residents.iter().enumerate().map(|(i, r)| {
                (i, r.id.clone(), r.name, r.body.x, r.body.z, r.social_cooldown, !r.say.is_empty())
            }).collect();

        let mut init_pair: Option<(usize, usize)> = None;
        'scan: for i in 0..snaps.len() {
            // 發起者：冷卻到期、目前沒在說話。
            if snaps[i].6 || snaps[i].5 > 0.0 {
                continue;
            }
            for j in 0..snaps.len() {
                if i == j { continue; }
                // 目標：沒在說話（避免打斷對方）、且在範圍內。
                if snaps[j].6 { continue; }
                if !vrel::pair_within_range(snaps[i].3, snaps[i].4, snaps[j].3, snaps[j].4, vrel::SOCIAL_RANGE) {
                    continue;
                }
                if vrel::should_initiate_social(rand::random::<f32>()) {
                    init_pair = Some((i, j));
                    break 'scan;
                }
            }
        }
        if let Some((i, j)) = init_pair {
            let ini_id = snaps[i].1.clone();
            let ini_name = snaps[i].2;
            let tar_id = snaps[j].1.clone();
            let tar_name = snaps[j].2;
            let desire_opt = desire_snaps.get(&ini_id).map(|s| s.as_str());
            let line = vrel::resident_social_initiation(ini_name, tar_name, desire_opt);
            let safe_line: String = line.chars().take(vrel::SOCIAL_SAY_CHARS).collect();
            residents[i].say = safe_line.clone();
            residents[i].say_timer = SAY_SECS;
            residents[i].social_cooldown = vrel::SOCIAL_COOLDOWN_SECS;
            // 目標居民幾秒後回應（pending_response 存 initiator id + name + 倒數）。
            residents[j].pending_response = Some((ini_id.clone(), ini_name.to_string(), vrel::RESPONSE_DELAY_SECS));
            social_events.push((ini_id, ini_name.to_string(), tar_id, tar_name.to_string(), safe_line, false));
        }
    } // deltas/residents 鎖在此一併釋放

    // 3) 廣播最新快照（含居民位置/名字/說的話）。
    broadcast_players();

    // 4) 落地社交記憶（鎖已釋放；一律 append-only，不破壞既有）。
    // 說話者：speaker；聽到的那方：listener（發起時=目標；回應時=原發起者）。
    for (speaker_id, speaker_name, listener_id, _listener_name, line, _is_response) in &social_events {
        if let Some(summary) = vrel::overhear_summary(speaker_name, line) {
            let entry = {
                let mut soc = hub().social.write().unwrap();
                soc.record_overheard(listener_id, speaker_id, &summary)
            }; // social 寫鎖在此釋放
            vrel::append_social(&entry);
        }
    }

    // 5) 無鎖 spawn 思考（LLM）。整個 agent 思考可由 BUTFUN_NPC_AGENT=0 關掉，
    //    關掉後居民仍照常閒晃移動，只是不冒 LLM 心裡話/心願（零額外成本）。
    if npc_agent_wire::agents_enabled() {
        for (id, name, persona, x, z) in think_jobs {
            spawn_resident_think(id, name, persona, x, z);
        }
    }

    // 6) 居民建造（builds/desires/deltas/residents 的鎖全在上面釋放後才取，守循序不巢狀鐵律）。
    //    流程：① 若無計畫 → 讀心願 → 分類 → 啟動計畫 → 冒「開始蓋」台詞
    //           ② 若有計畫 → 彈下一塊 → set_block → broadcast → 持久化 → 重設 build_tick
    //    所有說話更新收集到 say_updates，最後一次性套用（只一把 residents 寫鎖）。
    let mut say_updates: Vec<(String, String)> = Vec::new(); // (resident_id, say_text)

    for (rid, rname, rx, _ry, rz, ridx) in build_candidates {
        let has_plan = hub().builds.read().unwrap().has_plan(&rid); // drop

        if !has_plan {
            // 無計畫：讀心願 → 分類 → 啟動
            let desire_opt: Option<String> = {
                let des = hub().desires.read().unwrap();
                des.get_desire(&rid).map(|d| d.desire.clone())
            }; // desires 讀鎖釋放

            if let Some(desire_text) = desire_opt {
                if let Some(kind) = vbuild::classify_desire(&desire_text) {
                    // 找建造位置：居民當前 x/z + 固定方位偏移
                    let (ox, oz) = vbuild::build_anchor_offset(ridx);
                    let bx = rx + ox;
                    let bz = rz + oz;
                    let by = vbuild::surface_y(bx, bz);

                    let plan = {
                        let mut builds = hub().builds.write().unwrap();
                        if builds.has_plan(&rid) {
                            // double-check（並發安全）
                            None
                        } else {
                            Some(builds.new_plan(&rid, kind, bx, by, bz))
                        }
                    }; // builds 寫鎖釋放

                    if let Some(p) = plan {
                        vbuild::append_build(&p);
                        let say = vbuild::build_say_line(&p.kind_name, 0);
                        say_updates.push((rid.clone(), say));
                    }
                }
            }
            // 本 tick 不放塊，重設 build_tick 等下次
            {
                let mut residents = hub().residents.write().unwrap();
                if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                    r.build_tick = BUILD_INTERVAL_SECS;
                }
            } // residents 寫鎖釋放
            continue;
        }

        // 有計畫：彈下一塊
        let (next_block, kind_name, progress_pct, plan_done) = {
            let mut builds = hub().builds.write().unwrap();
            if let Some(plan) = builds.get_plan_mut(&rid) {
                let bb = plan.pop_next();
                let kn = plan.kind_name.clone();
                let pct = plan.progress_pct();
                let done = plan.is_done();
                (bb, kn, pct, done)
            } else {
                (None, String::new(), 100, true)
            }
        }; // builds 寫鎖釋放

        if let Some(bb) = next_block {
            if let Some(block) = Block::from_u8(bb.b) {
                // 寫入 delta layer
                {
                    let mut world = hub().deltas.write().unwrap();
                    voxel::set_block(&mut world, bb.x, bb.y, bb.z, block);
                } // deltas 寫鎖釋放
                broadcast_block(bb.x, bb.y, bb.z, block);

                // 持久化更新後的計畫（remaining 已縮短）
                if let Some(plan) = hub().builds.read().unwrap().plans.get(&rid) {
                    vbuild::append_build(plan);
                } // builds 讀鎖釋放

                // 關鍵進度冒泡（開始蓋時已冒過；之後在 50%、95%、完成時各冒一次）
                if progress_pct == 50 || progress_pct >= 95 {
                    let say = vbuild::build_say_line(&kind_name, progress_pct);
                    say_updates.push((rid.clone(), say));
                }
            }
        }

        if plan_done {
            let mut builds = hub().builds.write().unwrap();
            builds.remove_if_done(&rid);
        } // builds 寫鎖釋放（若未取則無鎖）

        // 重設 build_tick
        {
            let mut residents = hub().residents.write().unwrap();
            if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                r.build_tick = BUILD_INTERVAL_SECS;
            }
        } // residents 寫鎖釋放
    }

    // 一次性套用說話更新（單獨一把 residents 寫鎖；say_updates 可能為空）。
    if !say_updates.is_empty() {
        let mut residents = hub().residents.write().unwrap();
        for (rid, say_text) in say_updates {
            if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                if r.say.is_empty() {
                    // 只在居民沒有其他話時冒建造台詞，不打斷社交對話
                    let safe: String = say_text.chars().take(40).collect();
                    r.say = safe;
                    r.say_timer = SAY_SECS;
                }
            }
        }
    } // residents 寫鎖釋放
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
    // 回想（思考用）：短鎖撈這位居民對「附近每位玩家」的長期記憶 → 拼成一句脈絡，
    // 讓居民思考時也記得在場的人是誰、之前聊過什麼（drop 鎖後才 spawn，絕不持鎖 await）。
    let recall_note: String = {
        let mem = hub().memory.read().unwrap();
        let mut lines: Vec<String> = Vec::new();
        for p in &nearby_players {
            for e in mem.recall(&id, &p.name, vmem::RECALL_LIMIT) {
                lines.push(format!("（關於 {}）{}", p.name, e.summary));
            }
        }
        lines.join("；")
    }; // 記憶讀鎖在此釋放
    // 短鎖讀居民的當前心願（落在 recall_note 快照之後、drop 後 spawn，不持鎖 await）。
    let resident_desire_note: Option<String> = {
        let des = hub().desires.read().unwrap();
        des.get_desire(&id).map(|d| {
            format!(
                "你有個從旅人的話裡種下的心願：「{}」（由「{}」帶給你的啟發）——\
                這個夢想是你生活的動力，偶爾在心裡默默惦記著它。",
                d.desire, d.sparked_by
            )
        })
    }; // 心願讀鎖在此釋放
    // 短鎖讀社交記憶（最近聽到其他居民說了什麼）→ 帶入 world_news 讓居民思考時知道彼此近況。
    let social_note: String = {
        let soc = hub().social.read().unwrap();
        let snaps = soc.recall_for(&id, vrel::SOCIAL_RECALL_LIMIT);
        if snaps.is_empty() {
            String::new()
        } else {
            let notes: Vec<String> = snaps.iter().map(|s| s.summary.clone()).collect();
            format!("你最近聽到的鄰居近況：{}", notes.join("；"))
        }
    }; // social 讀鎖在此釋放
    let world_news = {
        let mut parts =
            vec!["你生活在新生的『乙太方界』——一片由方塊構成的清淨天地。".to_string()];
        if !recall_note.is_empty() {
            parts.push(format!("你記得：{recall_note}。"));
        }
        if let Some(note) = resident_desire_note {
            parts.push(note);
        }
        if !social_note.is_empty() {
            parts.push(social_note);
        }
        parts.concat()
    };
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
        world_news,
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
    fn talk_parses() {
        let m: ClientMsg =
            serde_json::from_str(r#"{"t":"talk","resident_id":"vox_res_0","text":"你好"}"#).unwrap();
        match m {
            ClientMsg::Talk { resident_id, text } => {
                assert_eq!(resident_id, "vox_res_0");
                assert_eq!(text, "你好");
            }
            _ => panic!("應解析成 Talk"),
        }
    }

    #[test]
    fn sanitize_talk_text_rules() {
        // 空 / 純空白 → None（忽略）。
        assert!(sanitize_talk_text("").is_none());
        assert!(sanitize_talk_text("   ").is_none());
        // 正常 → trim 後保留。
        assert_eq!(sanitize_talk_text("  哈囉  ").as_deref(), Some("哈囉"));
        // 超長 → 截斷到上限字元數（用多位元組中文字驗證是按「字元」非位元組截）。
        let long: String = "字".repeat(TALK_MAX_CHARS + 50);
        let out = sanitize_talk_text(&long).unwrap();
        assert_eq!(out.chars().count(), TALK_MAX_CHARS);
    }

    #[test]
    fn talk_cooldown_boundary() {
        assert!(!talk_cooldown_ok(0));
        assert!(!talk_cooldown_ok(TALK_COOLDOWN_MS - 1));
        // 剛好到門檻就放行。
        assert!(talk_cooldown_ok(TALK_COOLDOWN_MS));
        assert!(talk_cooldown_ok(TALK_COOLDOWN_MS + 1000));
    }

    #[test]
    fn talk_prompt_and_canned_non_empty() {
        // 對話 system prompt 含居民名字、且非空。
        let sys = resident_talk_system_prompt("露娜", ResidentPersona::Wanderer, None);
        assert!(sys.contains("露娜"));
        assert!(sys.contains("乙太方界"));
        // 罐頭回覆永遠非空（降級時也回得出一句）。
        for n in RESIDENT_NAMES {
            assert!(!resident_canned_reply(n).is_empty());
        }
    }

    #[test]
    fn greeting_line_wraps_and_non_empty() {
        // 索引取模、永遠回得出非空招呼句。
        for i in 0..20 {
            assert!(!greeting_line(i).is_empty());
        }
    }

    #[test]
    fn nearest_player_dist_sq_works() {
        // 沒有玩家 → None。
        assert!(nearest_player_dist_sq(0.0, 0.0, &[]).is_none());
        // 多名玩家取最近者的平方距離。
        let pts = [(3.0, 4.0), (1.0, 0.0), (10.0, 10.0)];
        let d = nearest_player_dist_sq(0.0, 0.0, &pts).unwrap();
        assert!((d - 1.0).abs() < 1e-4, "最近者 (1,0) 平方距離應為 1：{d}");
    }

    #[test]
    fn place_block_id_validates() {
        // 合法 id → Some；越界 → None（伺服器據此忽略 place）。
        assert_eq!(Block::from_u8(3), Some(Block::Stone));
        assert!(Block::from_u8(200).is_none());
    }
}
