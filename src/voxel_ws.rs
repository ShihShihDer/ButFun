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
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::npc_agent::{AgentAction, AgentDecision, NearbyPlayer, SenseInput};
use crate::npc_agent_wire::{self, AgentBus};
use crate::resident_npc::ResidentPersona;
use crate::state::AppState;
use crate::voxel::{self, Block, ChunkCoord, WorldDelta, BASE_HEIGHT, CHUNK, SEA_LEVEL};
use crate::voxel_building::{self as vbuild, BuildStore};
use crate::voxel_skills::{self as vskill, GatherSkill, GoalStore, NextActivity};
use crate::voxel_desires::{self as vdes, DesireStore};
use crate::voxel_diary;
use crate::voxel_feed as vfeed;
use crate::voxel_craft as vcraft;
use crate::voxel_inventory::{self as vinv, InvStore};
use crate::voxel_memory::{self as vmem, VoxelMemory};
use crate::voxel_farm::{self as vfarm, FarmStore};
use crate::voxel_gift as vgift;
use crate::voxel_overhear as vh;
use crate::voxel_relations::{self as vrel, SocialStore};
use crate::voxel_residents::{self as vr, Body};
use crate::voxel_time::{self as vt, WorldTime, TimePhase};

/// 入場時串給玩家的 chunk 半徑（以 chunk 為單位，水平）。3 → 7×7 column。
const SPAWN_CHUNK_RADIUS: i32 = 3;
/// 垂直 chunk 範圍（cy）。0..=1 覆蓋世界 Y 0..31，含所有地形高度。
const CY_MIN: i32 = 0;
const CY_MAX: i32 = 1;
/// 安全上限：單次 req 最多回幾個 chunk（擋惡意客戶端狂要）。
const MAX_REQ_CHUNKS: usize = 8;

/// 一名 voxel 玩家的權威狀態（位置 + 朝向 + 當前冒泡的話）。
#[derive(Clone, Debug, Serialize)]
struct VoxelPlayer {
    id: Uuid,
    name: String,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    /// 此刻玩家頭上冒泡的話（embodied 靠近說話 v1）：空 = 不冒泡。廣播給別人看到你在說話。
    #[serde(default)]
    say: String,
    /// 冒泡剩餘秒數（不廣播；伺服器 tick 倒數，歸零清空 say）。
    #[serde(skip)]
    say_timer: f32,
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

// ── 玩家↔居民對話（切片：點居民聊天／embodied 靠近說話）────────────────────────
/// 玩家送來的單句對話長度上限（字元）。超過就截斷，擋惡意灌爆 prompt。
const TALK_MAX_CHARS: usize = 200;
/// 玩家頭上對話泡泡顯示秒數（embodied 靠近說話 v1）：和居民冒泡同節奏。
const PLAYER_SAY_SECS: f32 = 6.0;
/// 玩家對話泡泡的最大字元數（截短避免撐爆世界裡的泡泡貼圖）。
const PLAYER_SAY_MAX_CHARS: usize = 60;
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
/// 記憶回想泡泡觸發距離（方塊）：比招呼稍近，表示「走到面前才說起回憶」。
const RECALL_DIST: f32 = 5.0;
/// 回想泡泡冷卻（秒）：觸發一次後要等這麼久——稀少才有感，不能跟招呼一樣頻繁。
const RECALL_COOLDOWN_SECS: f32 = 180.0;
/// 每個合格 tick 觸發回想的機率（10Hz 下 0.002 ≈ 在範圍內平均 50 秒才偶發一次）。
const RECALL_CHANCE_PER_TICK: f32 = 0.002;
/// 居民建造頻率：每隔這麼多秒放一塊方塊（慢節奏，讓玩家能目睹過程）。
const BUILD_INTERVAL_SECS: f32 = 8.0;
/// 居民每蓋一個建物前要先採集幾次（備料感、「她真的在做事」）。
const GATHER_QUOTA: u32 = 2;
/// 全部建物蓋完後，閒置時每個 agency tick 觸發一次「散心採集」的機率（低頻、不洗版）。
const IDLE_GATHER_CHANCE: f32 = 0.15;

/// 一位乙太方界居民的權威運行狀態（位置/朝向 + 閒晃目標 + 思考排程 + 當前冒的話）。
struct VoxelResident {
    /// 系統 id（"vox_res_0"…），voxel 模組內專用，與 2D 居民 id 體系無交集。
    id: String,
    name: &'static str,
    persona: ResidentPersona,
    body: Body,
    yaw: f32,
    /// 此居民的家域中心（世界座標）：閒晃時若超出 HOME_RADIUS 就歸巢。
    home_x: f32,
    home_z: f32,
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
    /// 記憶回想泡泡冷卻（秒）：> 0 表示最近剛回想過，尚不可再觸發（稀少才有感）。
    recall_cooldown: f32,
    /// 旁聽搭話冷卻（秒，embodied 靠近說話 v1）：> 0 表示最近因旁聽搭過一句，
    /// 尚不可再搭（防同一位連發、對話風暴）。零 LLM。
    overhear_cooldown: f32,
    /// 當前採集任務（居民 agency v1·技能調用）：Some = 正走向某資源要挖；None = 沒在採集。
    gather: Option<GatherSkill>,
    /// 本輪（自上次蓋造後）已採集次數：達 GATHER_QUOTA 才開始蓋下一個建物（備料感）。
    gathered_since_build: u32,
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

/// 居民主動招呼——依好感等級選不同句池（好感等級由 `affinity_count` 派生）。
/// - affinity 0   → 陌生人：通用招呼
/// - affinity 1–2 → 相識：帶玩家名字的親切招呼
/// - affinity 3+  → 友人：帶名字、更溫暖、暗示記得對方
/// `pick` 用居民位置雜湊決定（確定性，避免每幀不同）；`player_name` 空字串安全退回通用句。
/// 純函式、可測、零 LLM 成本。
fn greeting_line_affinity(affinity: usize, player_name: &str, pick: usize) -> String {
    if affinity == 0 || player_name.is_empty() {
        // 陌生人：沿用原來4句罐頭
        const STRANGER: [&str; 4] = ["你來啦！", "嗨，旅人～", "哦，有客人。", "你好呀！"];
        return STRANGER[pick % STRANGER.len()].to_string();
    }
    // 截斷名字：避免超長名字撐爆泡泡（最多 6 字）
    let name: String = player_name.chars().take(6).collect();
    if affinity <= 2 {
        // 相識：用名字打招呼
        const ACQUAINT: [&str; 3] = ["又見面啦，{name}～", "嗨，{name}！", "{name}，歡迎來玩！"];
        ACQUAINT[pick % ACQUAINT.len()].replace("{name}", &name)
    } else {
        // 友人：更溫暖，暗示記得你
        const FRIEND: [&str; 3] = ["{name}！你回來了！", "哇，{name}，好久不見。", "嗨{name}，我有想你呢！"];
        FRIEND[pick % FRIEND.len()].replace("{name}", &name)
    }
}

/// 一組玩家（水平座標 + 顯示名）中，離 (rx,rz) 最近者的 (平方距離, 玩家名)。
/// 沒有玩家回 None。純函式、可測。
fn nearest_player_info(rx: f32, rz: f32, players: &[(f32, f32, String)]) -> Option<(f32, &str)> {
    players
        .iter()
        .map(|(px, pz, name)| {
            let dx = px - rx;
            let dz = pz - rz;
            (dx * dx + dz * dz, name.as_str())
        })
        .fold(None, |acc, (d, name)| match acc {
            None => Some((d, name)),
            Some((bd, bn)) => if d < bd { Some((d, name)) } else { Some((bd, bn)) },
        })
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
/// 初始化 N 位居民：各自散佈到家域中心，各自站穩在陸地上。
fn init_residents() -> Vec<VoxelResident> {
    let mut out = Vec::with_capacity(RESIDENT_COUNT);
    for i in 0..RESIDENT_COUNT {
        // 各居民有自己的家域基準點，分散世界四方（見 vr::resident_home_base）。
        let (hox, hoz) = vr::resident_home_base(i);
        let body = vr::dry_ground_spawn(hox, hoz);
        let home_x = body.x;
        let home_z = body.z;
        out.push(VoxelResident {
            id: format!("vox_res_{i}"),
            name: RESIDENT_NAMES[i],
            persona: persona_for(i),
            home_x,
            home_z,
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
            // 錯開首次回想冷卻，避免啟動後短時間全員同時觸發（前 60 秒不回想）。
            recall_cooldown: 60.0 + i as f32 * 30.0,
            // 旁聽搭話冷卻：初始 0，可立即因旁聽搭話（之後由 should_chime_in 套冷卻）。
            overhear_cooldown: 0.0,
            // agency v1：入場無採集任務、尚未採集。
            gather: None,
            gathered_since_build: 0,
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
    /// 居民已完成目標 store（agency v1）：每居民「蓋過哪些建物」。持久化到 data/voxel_goals.jsonl。
    /// 讓挑目標永不重選蓋過的種類（不鬼打牆）、蓋完自然生出下一個（進展）。
    goals: RwLock<GoalStore>,
    /// 居民小背包（agency v1·純記憶體）：採集挖到的材料進這裡（rid → block_id → 數量）。
    /// 「她真的在做事」的成果；與玩家背包（inventory）分開，互不干涉。
    res_inv: RwLock<HashMap<String, HashMap<u8, u32>>>,
    /// 玩家背包（採集 v1）：挖方塊得材料、放置消耗存量。持久化到 data/voxel_inventory.jsonl。
    inventory: RwLock<InvStore>,
    /// 農地 store（種田 v1·純記憶體；重啟後農地重置，與世界 delta 行為一致）。
    /// 記錄哪些格子種下了幼苗、何時種的，每 15 秒 tick 一次成熟檢查。
    farm: RwLock<FarmStore>,
    /// 世界時鐘（晝夜循環 v1）：一遊戲日 = 600 秒；廣播給前端以更新天空/光照。
    world_time: RwLock<WorldTime>,
    /// 上一 tick 的時段（日夜作息 v1）：偵測時段轉換、觸發居民過渡台詞。
    last_phase: std::sync::Mutex<TimePhase>,
    tx: broadcast::Sender<Arc<String>>,
}

static HUB: OnceLock<VoxelHub> = OnceLock::new();

fn hub() -> &'static VoxelHub {
    HUB.get_or_init(|| {
        let (tx, _rx) = broadcast::channel(256);
        // 重啟還原：把居民先前蓋的方塊／挖的洞 replay 套回 delta（持久化，重啟後還在）。
        let mut deltas = WorldDelta::new();
        for bb in vbuild::load_world_blocks() {
            if let Some(b) = Block::from_u8(bb.b) {
                voxel::set_block(&mut deltas, bb.x, bb.y, bb.z, b);
            }
        }
        VoxelHub {
            players: RwLock::new(HashMap::new()),
            deltas: RwLock::new(deltas),
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
            // 啟動時從 data/voxel_goals.jsonl 載回已完成目標（重啟後不重蓋蓋過的）。
            goals: RwLock::new(GoalStore::from_entries(vskill::load_goals())),
            // 居民小背包純記憶體（採集成果；重啟重置，與農地一致）。
            res_inv: RwLock::new(HashMap::new()),
            // 啟動時從 data/voxel_inventory.jsonl 載回玩家背包（重啟後存量還在）。
            inventory: RwLock::new(InvStore::from_entries(vinv::load_inventory())),
            // 農地 store 純記憶體（與世界 delta 一致：重啟後農地重置，玩家重新種即可）。
            farm: RwLock::new(FarmStore::new()),
            // 世界時鐘：從白天（time_of_day ≈ 0.42）開始，讓玩家一進遊戲就是白天。
            world_time: RwLock::new(WorldTime::new()),
            // 日夜作息 v1：初始時段 Day（對應 WorldTime::new() 的 time_of_day ≈ 0.42）。
            last_phase: std::sync::Mutex::new(TimePhase::Day),
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
    // 時鐘快照（短鎖、不巢狀）：把 time_of_day(0.0–1.0) 帶給前端更新天空/光照。
    let time_of_day: f32 = hub().world_time.read().unwrap().time_of_day();
    serde_json::json!({
        "t": "players",
        "players": players,
        "residents": residents,
        "time_of_day": time_of_day,
    }).to_string()
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
    /// 跟居民對話（embodied 靠近說話 v1）：
    /// - `resident_id = Some(id)`：指定對象（點居民 / 走近面對）——舊行為，向後相容。
    /// - `resident_id = None`：範圍「說話」——伺服器挑半徑內最近/面對者當被指名者回話，
    ///   其餘範圍內的居民旁聽（進記憶、零 LLM，偶爾依個性搭一句）。
    /// 回 `talk`（單播）給玩家、並由被指名者頭上冒泡讓附近人看到。
    Talk {
        #[serde(default)]
        resident_id: Option<String>,
        text: String,
    },
    /// 合成台 v1：用配料合成新型方塊（ROADMAP 658）。`recipe_id` 對齊 voxel_craft::Recipe.id。
    Craft { recipe_id: String },
    /// 種田 v1：在農田土上種下種子（ROADMAP 659）。
    /// 伺服器驗證目標是 FarmSoil(11)、玩家有種子(14)後，把方塊改成 FarmSoilSeeded(12)。
    Plant { x: i32, y: i32, z: i32 },
    /// 居民贈禮 v1：把背包裡的一件材料送給附近居民（ROADMAP 660）。
    /// 伺服器驗證觸及範圍 + 背包存量後，扣材料、加記憶 ×2、居民冒泡道謝。
    Gift { resident_id: String, item_id: u8 },
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

pub async fn voxel_ws_handler(
    ws: WebSocketUpgrade,
    State(app): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // 登入綁定：WS 升級握手會夾帶同源 cookie。若帶有效 session → 解出帳號顯示名，
    // 當作這條連線的「穩定身份」（記憶/好感度/背包都綁帳號，換訪客名也認得你）。
    // 安全：身份只認 cookie，不認客戶端 join 自報名 → 無法靠送別人的名字冒充帳號。
    // 訪客（無 cookie / OAuth 未設）回 None，照舊以 join 顯示名進場。
    let account_name: Option<String> = app
        .auth
        .as_ref()
        .and_then(|cfg| crate::auth::user_id_from_cookies(&headers, &cfg.session_secret))
        .and_then(|uid| app.users.get(uid))
        .map(|u| u.name);

    // 與主 ws 一致的安全硬化：訊息上限 64 KiB（任何合法 voxel 訊息都遠小於此；
    // chunk 是「伺服器送出」不受此限）。
    const WS_MAX_MSG_BYTES: usize = 64 * 1024;
    ws.max_message_size(WS_MAX_MSG_BYTES)
        .max_frame_size(WS_MAX_MSG_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, account_name))
}

/// 解析連線身份鍵：登入帳號名優先（穩定、跨 session），其次 join 自報顯示名，皆無則「旅人」。
/// 純函式：身份綁定的單一真相，方便測試「登入覆蓋訪客名」。皆去頭尾空白並截斷 24 字。
fn resolve_identity(account_name: Option<&str>, join_name: Option<&str>) -> String {
    for candidate in [account_name, join_name].into_iter().flatten() {
        let cleaned: String = candidate.trim().chars().take(24).collect();
        if !cleaned.is_empty() {
            return cleaned;
        }
    }
    String::from("旅人")
}

async fn handle_socket(socket: WebSocket, account_name: Option<String>) {
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
    // 身份鍵：登入帳號優先（覆蓋 join 自報名），訪客才用 join 顯示名。
    let mut join_name: Option<String> = None;
    if let Some(Ok(Message::Text(txt))) = receiver.next().await {
        if let Ok(ClientMsg::Join { name: n }) = serde_json::from_str::<ClientMsg>(&txt) {
            join_name = n;
        }
    } else {
        // 連線一開始就斷/非文字 → 收攤。
        writer.abort();
        return;
    }
    let name = resolve_identity(account_name.as_deref(), join_name.as_deref());
    let is_account = account_name.is_some();

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
                say: String::new(),
                say_timer: 0.0,
            },
        );
    }

    // 送 welcome（出生點 + 世界常數，前端據此設碰撞/相機）。
    let welcome = serde_json::json!({
        "t": "welcome",
        "id": my_id.to_string(),
        "name": name,
        // 登入綁定：前端據此知道目前是「帳號身分」還是訪客（帳號名一律由 cookie 解出，
        // 非客戶端自報；換訪客名也認得你）。
        "account": is_account,
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

    // 送目前背包存量（讓前端熱鍵欄立即顯示正確數量）。
    {
        let pairs = hub().inventory.read().unwrap().pairs(&name);
        let inv_sync =
            serde_json::json!({ "t": "inv_sync", "items": pairs }).to_string();
        if out_tx.send(Message::Text(inv_sync)).await.is_err() {
            cleanup(my_id, &writer);
            return;
        }
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
                // 採集 v1：先讀目標方塊型別（讀鎖即釋），破壞後給予對應材料。
                let Some((px, py, pz)) = player_pos(my_id) else {
                    continue;
                };
                // 讀鎖快照目標方塊型別（delta 讀鎖，馬上釋放）。
                let target_block =
                    voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                // delta 寫鎖：驗證 + 設為空氣（循序取放，不嵌套）。
                let broken = {
                    let mut world = hub().deltas.write().unwrap();
                    if voxel::can_break(&world, px, py, pz, x, y, z) {
                        voxel::set_block(&mut world, x, y, z, Block::Air);
                        true
                    } else {
                        false
                    }
                }; // delta 寫鎖在此釋放
                if broken {
                    broadcast_block(x, y, z, Block::Air);
                    // 農地方塊有特殊掉落；其餘實心方塊掉落自身。
                    if target_block.is_solid() {
                        // 種田 v1：農地狀態方塊的特殊掉落規則。
                        //   Leaves(6)        → 種子(14)×1（葉片→種子，v1 種子來源）。
                        //   FarmSoilSeeded(12)→ 農田土(11)×1 + 種子(14)×1（取消種植退還）。
                        //   WheatMature(13)   → 農田土(11)×1 + 種子(14)×2（收割，淨賺+1）。
                        //   其餘實心方塊     → 自身×1（原行為）。
                        let drops: &[(u8, u32)] = match target_block {
                            Block::Leaves          => &[(vfarm::SEEDS_ID, 1)],
                            Block::FarmSoilSeeded  => &[(11, 1), (vfarm::SEEDS_ID, 1)],
                            Block::WheatMature     => &[(11, 1), (vfarm::SEEDS_ID, 2)],
                            _ => &[], // 後面用 else 分支處理
                        };

                        // 種田 v1 的方塊 → 農地 store 也要清掉記錄。
                        if matches!(target_block, Block::FarmSoilSeeded | Block::WheatMature) {
                            hub().farm.write().unwrap().remove(x, y, z);
                        }

                        if !drops.is_empty() {
                            // inventory 寫鎖（delta 已釋放，循序不巢狀，守死鎖鐵律）。
                            let mut inv = hub().inventory.write().unwrap();
                            for &(did, cnt) in drops {
                                let entry = inv.give(&name, did, cnt);
                                drop(inv); // 先釋放再 append（不持鎖 IO）
                                vinv::append_inv(&entry);
                                let new_count =
                                    hub().inventory.read().unwrap().count(&name, did);
                                let _ = out_tx.try_send(Message::Text(
                                    serde_json::json!({
                                        "t": "inv_update",
                                        "block_id": did,
                                        "count": new_count
                                    })
                                    .to_string(),
                                ));
                                // 重新借鎖繼續下一個 drop（若有）。
                                inv = hub().inventory.write().unwrap();
                            }
                        } else {
                            // 一般實心方塊：掉落自身。
                            let bid = target_block as u8;
                            let entry = hub().inventory.write().unwrap().give(&name, bid, 1);
                            vinv::append_inv(&entry);
                            let new_count =
                                hub().inventory.read().unwrap().count(&name, bid);
                            let _ = out_tx.try_send(Message::Text(
                                serde_json::json!({
                                    "t": "inv_update",
                                    "block_id": bid,
                                    "count": new_count
                                })
                                .to_string(),
                            ));
                        }
                    }
                }
            }
            Ok(ClientMsg::Place { x, y, z, b }) => {
                // 採集 v1：先消耗庫存材料，再套 delta 放置；若放置失敗則退還材料。
                // 鎖序：inventory 先取再釋 → delta 後取再釋（循序不巢狀，守死鎖鐵律）。
                let Some(block) = Block::from_u8(b) else {
                    continue;
                };
                if !block.is_placeable() {
                    continue;
                }
                let Some((px, py, pz)) = player_pos(my_id) else {
                    continue;
                };
                // 步驟1：嘗試消耗材料（inventory 寫鎖，立即釋放）。
                let inv_entry = {
                    hub().inventory.write().unwrap().take(&name, b, 1)
                }; // inventory 寫鎖在此釋放
                let Some(inv_e) = inv_entry else {
                    // 材料不足 → 通知客戶端，不更動世界。
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "inv_denied", "block_id": b }).to_string(),
                    ));
                    continue;
                };
                // 步驟2：套 delta（delta 寫鎖，在 inventory 鎖已釋放後取，循序不巢狀）。
                let placed = {
                    let mut world = hub().deltas.write().unwrap();
                    if voxel::can_place(&world, px, py, pz, x, y, z, block) {
                        voxel::set_block(&mut world, x, y, z, block);
                        true
                    } else {
                        false
                    }
                }; // delta 寫鎖在此釋放
                if placed {
                    vinv::append_inv(&inv_e); // 放置成功才持久化消耗記錄
                    broadcast_block(x, y, z, block);
                    let new_count = hub().inventory.read().unwrap().count(&name, b);
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({
                            "t": "inv_update",
                            "block_id": b,
                            "count": new_count
                        })
                        .to_string(),
                    ));
                } else {
                    // 放置位置不合法（被搶佔等），退還材料。
                    hub().inventory.write().unwrap().give(&name, b, 1);
                    // 不持久化（退還等於沒發生）
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
                last_talk = Some(now);
                // 身份鍵：登入者為帳號名（穩定、跨 session、換訪客名也認得你），訪客為 join 顯示名。
                // `name` 已在入場時由 resolve_identity 綁定。
                let player_key = name.clone();

                // 3) embodied 靠近說話 v1：玩家自己頭上先冒泡（不論有沒有人被指名）→ 話活在世界裡。
                //    短鎖 players 寫，設 say + 計時；步驟 8 的 broadcast_players 立即推給所有人。
                {
                    let mut players = hub().players.write().unwrap();
                    if let Some(p) = players.get_mut(&my_id) {
                        p.say = clean.chars().take(PLAYER_SAY_MAX_CHARS).collect();
                        p.say_timer = PLAYER_SAY_SECS;
                    }
                } // players 寫鎖釋放

                // 4) 短鎖快照玩家位置 + 朝向（指名選擇與旁聽範圍都要）→ drop。
                let player_snap: Option<(f32, f32, f32)> = {
                    let players = hub().players.read().unwrap();
                    players.get(&my_id).map(|p| (p.x, p.z, p.yaw))
                }; // players 讀鎖釋放
                // 短鎖快照所有居民（id, name, persona, x, z）→ drop（絕不持鎖 await）。
                let res_snaps: Vec<(String, &'static str, ResidentPersona, f32, f32)> = {
                    let res = hub().residents.read().unwrap();
                    res.iter()
                        .map(|r| (r.id.clone(), r.name, r.persona, r.body.x, r.body.z))
                        .collect()
                }; // residents 讀鎖釋放

                // 5) 決定「被指名」回話的居民（純函式、零鎖）：
                //    - 顯式 resident_id（點居民 / 走近面對）→ 找該居民（保留舊行為，不受半徑限制）。
                //    - 範圍說話（None）→ 半徑內最近/面對者（vh::pick_addressed），半徑內無人 → 無人回話。
                let addressed: Option<(String, &'static str, ResidentPersona)> = match &resident_id {
                    Some(rid) => res_snaps
                        .iter()
                        .find(|(id, ..)| id == rid)
                        .map(|(id, n, p, ..)| (id.clone(), *n, *p)),
                    None => match player_snap {
                        Some((px, pz, yaw)) => {
                            let positions: Vec<(f32, f32)> =
                                res_snaps.iter().map(|(_, _, _, x, z)| (*x, *z)).collect();
                            vh::pick_addressed(px, pz, yaw, &positions, vh::OVERHEAR_RADIUS).map(|i| {
                                let (id, n, p, ..) = &res_snaps[i];
                                (id.clone(), *n, *p)
                            })
                        }
                        None => None,
                    },
                };
                let addressed_id: Option<String> = addressed.as_ref().map(|(id, ..)| id.clone());

                // 6) 被指名者 → 走既有 LLM 對話路徑（記憶/心願/思考中佔位/罐頭後備、世界冒泡）。
                if let Some((addr_id, rname, rpersona)) = addressed.clone() {
                    // 6a) 短鎖讀記憶 → 組脈絡區塊（近期對話 + 關於這位玩家的長期記憶）→ drop。
                    let context = {
                        let mem = hub().memory.read().unwrap();
                        let history = mem.recent_dialogue(&player_key, &addr_id);
                        let memories = mem.recall(&addr_id, &player_key, vmem::RECALL_LIMIT);
                        vmem::build_context_block(&history, &memories, &player_key)
                    }; // 記憶讀鎖在此釋放
                    // 6b) 短鎖讀居民當前心願 → drop（帶進 prompt 讓居民「帶著夢想說話」）。
                    let current_desire: Option<String> = {
                        let des = hub().desires.read().unwrap();
                        des.get_desire(&addr_id).map(|d| d.desire.clone())
                    }; // 心願讀鎖在此釋放
                    // 6c) 立即送「思考中」佔位（私聊單播，不走 AgentBus 冒泡）。
                    let ack = serde_json::json!({
                        "t": "talk",
                        "resident_id": &addr_id,
                        "name": rname,
                        "reply": "…",
                        "thinking": true,
                    })
                    .to_string();
                    let _ = out_tx.send(Message::Text(ack)).await;

                    // 6d) 無鎖 async task：快速 LLM 路徑 → 回覆單播 + AgentBus 冒泡 + 記憶/心願寫入。
                    let reply_tx = out_tx.clone();
                    let clean_for_llm = clean.clone();
                    let pkey = player_key.clone();
                    tokio::spawn(async move {
                        let base_sys =
                            resident_talk_system_prompt(rname, rpersona, current_desire.as_deref());
                        let sys = if context.is_empty() {
                            base_sys
                        } else {
                            format!("{base_sys}\n\n{context}")
                        };
                        let reply: String = match tokio::time::timeout(
                            Duration::from_secs(TALK_LLM_TIMEOUT_SECS),
                            crate::npc_chat::raw_llm_call_fast(&sys, &clean_for_llm),
                        )
                        .await
                        {
                            Ok(Some(t)) => t.chars().take(TALK_REPLY_MAX_CHARS).collect(),
                            _ => resident_canned_reply(rname), // 逾時 / LLM 未啟用 → 罐頭後備
                        };
                        let msg = serde_json::json!({
                            "t": "talk",
                            "resident_id": addr_id,
                            "name": rname,
                            "reply": reply,
                        })
                        .to_string();
                        let _ = reply_tx.send(Message::Text(msg)).await;

                        // 記憶寫入（短鎖、不 await）：對話歷史 + 規則摘要進長期記憶並落地。
                        let new_memory = {
                            let mut mem = hub().memory.write().unwrap();
                            mem.record_turn(&pkey, &addr_id, &clean_for_llm, &reply);
                            vmem::summarize_exchange(&pkey, &clean_for_llm)
                                .map(|summary| mem.add_memory(&addr_id, &pkey, &summary))
                        }; // 記憶寫鎖在此釋放
                        if let Some(entry) = new_memory {
                            vmem::append_memory(&entry);
                        }
                        // 心願萃取（記憶驅動行為 v1）：回覆浮現「我想…」→ 更新心願並落地。
                        if let Some(desire_text) = vdes::extract_desire(&reply) {
                            let new_desire = {
                                let mut des = hub().desires.write().unwrap();
                                des.set_desire(&addr_id, &desire_text, &pkey)
                            }; // 心願寫鎖在此釋放
                            vdes::append_desire(&new_desire);
                            vfeed::append_feed("新心願", &new_desire.resident, &new_desire.desire);
                        }
                        // 冒泡（下一 tick 由 tick_residents 套用 say，自動截到 40 字、計時消失）。
                        hub().agent_bus.push_decision(
                            addr_id,
                            AgentDecision::new(AgentAction::Idle, reply, "對話"),
                        );
                    });
                }

                // 7) 旁聽（embodied）：半徑內、非被指名的居民「聽到」。
                //    a) 進記憶（零 LLM）：每位旁聽者記下「聽到旅人說…」（餵養念頭起念）。
                //    b) 搭話閘：should_chime_in（戳中度×外向度×冷卻×機率）→ 偶爾冒 canned 泡泡，
                //       多半只聽不講，防對話風暴。
                //    c) 念頭播種：戳中個性 + 尚無心願 → 種下心願（保留既有閉環）。
                if let Some((px, pz, _)) = player_snap {
                    let r2 = vh::OVERHEAR_RADIUS * vh::OVERHEAR_RADIUS;
                    let in_range: Vec<(String, &'static str, ResidentPersona)> = res_snaps
                        .iter()
                        .filter(|(id, _, _, rx, rz)| {
                            Some(id) != addressed_id.as_ref() && {
                                let dx = rx - px;
                                let dz = rz - pz;
                                dx * dx + dz * dz <= r2
                            }
                        })
                        .map(|(id, n, p, _, _)| (id.clone(), *n, *p))
                        .collect();

                    if !in_range.is_empty() {
                        // a) 旁聽進記憶（社交記憶 store，append-only；鎖外落地、絕不持鎖 await）。
                        if let Some(summary) = vrel::overhear_summary(&player_key, &clean) {
                            let entries: Vec<_> = {
                                let mut soc = hub().social.write().unwrap();
                                in_range
                                    .iter()
                                    .map(|(id, _, _)| soc.record_overheard(id, &player_key, &summary))
                                    .collect()
                            }; // social 寫鎖釋放
                            for e in &entries {
                                vrel::append_social(e);
                            }
                        }

                        // b) 搭話閘 + canned 泡泡（residents 寫鎖，短取即釋）。
                        {
                            let mut res = hub().residents.write().unwrap();
                            for (id, n, p) in &in_range {
                                if let Some(r) = res.iter_mut().find(|r| &r.id == id) {
                                    if !r.say.is_empty() {
                                        continue; // 正在說話 → 不打斷
                                    }
                                    let fits = vh::speech_fits_persona(&clean, *p);
                                    let extro = vh::persona_extroversion(*p);
                                    let cd_ok = r.overhear_cooldown <= 0.0;
                                    if vh::should_chime_in(fits, extro, cd_ok, rand::random::<f32>()) {
                                        r.say = vh::canned_overhear_reaction(*p, n);
                                        r.say_timer = vh::REACTION_SAY_SECS;
                                        r.overhear_cooldown = vh::OVERHEAR_CHIME_COOLDOWN_SECS;
                                    }
                                }
                            }
                        } // residents 寫鎖釋放

                        // c) 念頭播種：戳中個性 + 尚無心願 → 種心願（desires 寫鎖，每筆分開）。
                        if let Some(desire_text) = vdes::extract_desire(&clean) {
                            let has_desire_ids: Vec<String> = {
                                let des = hub().desires.read().unwrap();
                                in_range
                                    .iter()
                                    .filter(|(id, _, _)| des.get_desire(id).is_some())
                                    .map(|(id, _, _)| id.clone())
                                    .collect()
                            }; // desires 讀鎖釋放
                            for (id, _, p) in &in_range {
                                if has_desire_ids.contains(id)
                                    || !vh::speech_fits_persona(&clean, *p)
                                {
                                    continue;
                                }
                                let entry = {
                                    let mut des = hub().desires.write().unwrap();
                                    des.set_desire(id, &desire_text, &player_key)
                                }; // desires 寫鎖釋放
                                vdes::append_desire(&entry);
                                vfeed::append_feed("念頭種下", &entry.resident, &entry.desire);
                            }
                        }
                    }
                }

                // 8) 廣播：玩家自己的對話泡泡 + 居民旁聽反應泡泡，一次推給所有人。
                broadcast_players();
            }
            Ok(ClientMsg::Craft { recipe_id }) => {
                // 合成台 v1（ROADMAP 658）：消耗配料 → 給產出方塊 → 送 inv_update + craft_ok/fail。
                // 鎖紀律：一次 inventory 寫鎖內完成「確認 + 消耗」再釋放；give 在第二把寫鎖；
                //         兩把皆短鎖即釋、循序不巢狀，守 prod 死鎖鐵律。
                // find_any_recipe 統一查 2×2（RECIPES）和工作台（WORKBENCH_RECIPES）兩表。
                if let Some(recipe) = vcraft::find_any_recipe(&recipe_id) {
                    // 步驟 1：單把寫鎖內完成「確認足夠材料 + 消耗所有配料」（原子，防 TOCTOU）。
                    let (ok, consumed) = {
                        let mut inv = hub().inventory.write().unwrap();
                        if vcraft::can_craft(recipe, &inv, &name) {
                            let mut entries = Vec::new();
                            for &(block_id, count) in recipe.inputs {
                                // can_craft 已確認足夠，take 必成功；失敗不影響已改的（逐項消耗）。
                                if let Some(e) = inv.take(&name, block_id, count) {
                                    entries.push(e);
                                }
                            }
                            (true, entries)
                        } else {
                            (false, Vec::new())
                        }
                    }; // inventory 寫鎖釋放
                    if ok {
                        // 步驟 2：給產出方塊（第二把寫鎖，在第一把釋放後取）。
                        let out_e = hub().inventory.write().unwrap().give(
                            &name, recipe.output_block, recipe.output_count,
                        ); // inventory 寫鎖釋放
                        // 步驟 3：持久化（全在鎖外，比照 voxel_memory 做法）。
                        for e in &consumed { vinv::append_inv(e); }
                        vinv::append_inv(&out_e);
                        // 步驟 4：送 inv_update（各消耗方塊 + 產出方塊的新計數）。
                        let inv_r = hub().inventory.read().unwrap();
                        for &(block_id, _) in recipe.inputs {
                            let cnt = inv_r.count(&name, block_id);
                            let _ = out_tx.try_send(Message::Text(
                                serde_json::json!({ "t": "inv_update", "block_id": block_id, "count": cnt }).to_string(),
                            ));
                        }
                        let out_cnt = inv_r.count(&name, recipe.output_block);
                        drop(inv_r); // 讀鎖釋放後再送 craft_ok（守循序取放）
                        let _ = out_tx.try_send(Message::Text(
                            serde_json::json!({
                                "t": "inv_update",
                                "block_id": recipe.output_block,
                                "count": out_cnt
                            }).to_string(),
                        ));
                        let _ = out_tx.try_send(Message::Text(
                            serde_json::json!({
                                "t": "craft_ok",
                                "recipe_id": &recipe_id,
                                "name_zh": recipe.name_zh,
                                "out_count": recipe.output_count
                            }).to_string(),
                        ));
                    } else {
                        let _ = out_tx.try_send(Message::Text(
                            serde_json::json!({
                                "t": "craft_fail",
                                "recipe_id": &recipe_id,
                                "reason": "材料不足"
                            }).to_string(),
                        ));
                    }
                }
            }
            Ok(ClientMsg::Plant { x, y, z }) => {
                // 種田 v1（ROADMAP 659）：在農田土(11)上種下種子(14) → FarmSoilSeeded(12)。
                // 鎖序：inventory → delta → farm（循序取放，不巢狀，守死鎖鐵律）。
                let Some((px, py, pz)) = player_pos(my_id) else {
                    continue;
                };
                // 觸及範圍驗證（種植在方塊本身，不是面外側，距離比放置更寬鬆）。
                if !voxel::in_reach(px, py, pz, x, y, z) {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "plant_fail", "reason": "太遠了" }).to_string(),
                    ));
                    continue;
                }
                // 確認目標方塊是農田土(11)。
                let target = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if target != Block::FarmSoil {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "plant_fail", "reason": "需要農田土" }).to_string(),
                    ));
                    continue;
                }
                // 消耗 1 顆種子（inventory 寫鎖即釋）。
                let seed_entry = hub().inventory.write().unwrap().take(&name, vfarm::SEEDS_ID, 1);
                let Some(seed_e) = seed_entry else {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "plant_fail", "reason": "沒有種子" }).to_string(),
                    ));
                    continue;
                };
                // 套 delta：FarmSoil → FarmSoilSeeded（delta 寫鎖即釋）。
                voxel::set_block(&mut hub().deltas.write().unwrap(), x, y, z, Block::FarmSoilSeeded);
                // 記錄農地 + 持久化種子消耗（兩者都在鎖外）。
                hub().farm.write().unwrap().plant(x, y, z, vfarm::now_secs());
                vinv::append_inv(&seed_e);
                // 廣播方塊更新 + 送背包更新。
                broadcast_block(x, y, z, Block::FarmSoilSeeded);
                let new_seed_cnt = hub().inventory.read().unwrap().count(&name, vfarm::SEEDS_ID);
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({
                        "t": "inv_update",
                        "block_id": vfarm::SEEDS_ID,
                        "count": new_seed_cnt
                    })
                    .to_string(),
                ));
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "plant_ok", "x": x, "y": y, "z": z }).to_string(),
                ));
            }
            // ── 居民贈禮 v1（ROADMAP 660）────────────────────────────────────────
            Ok(ClientMsg::Gift { resident_id, item_id }) => {
                // 1) 短鎖取玩家位置（players 讀鎖即釋）。
                let player_pos: Option<(f32, f32)> = {
                    let players = hub().players.read().unwrap();
                    players.get(&my_id).map(|p| (p.x, p.z))
                };
                let Some((px, pz)) = player_pos else {
                    continue;
                };
                // 2) 短鎖取居民快照（residents 讀鎖即釋）。
                let res_snap: Option<(&'static str, f32, f32)> = {
                    let residents = hub().residents.read().unwrap();
                    residents
                        .iter()
                        .find(|r| r.id == resident_id)
                        .map(|r| (r.name, r.body.x, r.body.z))
                };
                let Some((rname, rx, rz)) = res_snap else {
                    continue; // 找不到居民
                };
                // 3) 驗觸及範圍（水平 XZ）。
                let dx = px - rx;
                let dz = pz - rz;
                if dx * dx + dz * dz > vgift::GIFT_REACH * vgift::GIFT_REACH {
                    let msg = serde_json::json!({
                        "t": "gift_fail",
                        "reason": "走近一點再送禮"
                    }).to_string();
                    let _ = out_tx.send(Message::Text(msg)).await;
                    continue;
                }
                // 4) 驗並消耗背包材料（inventory 寫鎖即釋）。
                let taken_entry = {
                    hub().inventory.write().unwrap().take(&name, item_id, 1)
                };
                let Some(inv_entry) = taken_entry else {
                    let iname = vgift::item_name_zh(item_id);
                    let msg = serde_json::json!({
                        "t": "gift_fail",
                        "reason": format!("背包裡沒有{iname}")
                    }).to_string();
                    let _ = out_tx.send(Message::Text(msg)).await;
                    continue;
                };
                vinv::append_inv(&inv_entry);
                // 5) 加兩筆記憶（memory 寫鎖各自短取即釋，循序不巢狀）。
                let iname = vgift::item_name_zh(item_id);
                let mem1 = vgift::gift_memory_event(&name, iname);
                let mem2 = vgift::gift_memory_feeling(&name, iname);
                let entry1 = {
                    hub().memory.write().unwrap().add_memory(&resident_id, &name, &mem1)
                };
                vmem::append_memory(&entry1);
                let entry2 = {
                    hub().memory.write().unwrap().add_memory(&resident_id, &name, &mem2)
                };
                vmem::append_memory(&entry2);
                // 6) 讀好感度（memory 讀鎖即釋）。
                let affinity = {
                    hub().memory.read().unwrap().affinity_count(&name, &resident_id)
                };
                // 7) 組道謝台詞（純函式，無鎖）。
                let pick = (vfarm::now_secs() as usize).wrapping_add(item_id as usize);
                let thanks = vgift::gift_thanks_line(iname, &name, affinity, pick);
                // 8) residents 寫鎖：設 say + say_timer（即釋）。
                {
                    let mut residents = hub().residents.write().unwrap();
                    if let Some(r) = residents.iter_mut().find(|r| r.id == resident_id) {
                        r.say = thanks.chars().take(50).collect();
                        r.say_timer = SAY_SECS;
                    }
                }
                // 9) 廣播讓所有人看到居民道謝泡泡。
                broadcast_players();
                // 10) 回送 inv_update（扣材料後存量）+ gift_ok（通知玩家成功）。
                let remain = hub().inventory.read().unwrap().count(&name, item_id);
                let inv_msg = serde_json::json!({
                    "t": "inv_update",
                    "block_id": item_id,
                    "count": remain,
                }).to_string();
                let _ = out_tx.send(Message::Text(inv_msg)).await;
                let ok_msg = serde_json::json!({
                    "t": "gift_ok",
                    "resident_id": &resident_id,
                    "resident_name": rname,
                    "item_id": item_id,
                    "item_name": iname,
                    "affinity": affinity,
                }).to_string();
                let _ = out_tx.send(Message::Text(ok_msg)).await;
                // 11) Feed：記錄贈禮事件（鎖外 IO）。
                vfeed::append_feed(
                    "贈禮",
                    rname,
                    &format!("{name}送了{iname}給{rname}"),
                );
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

/// 啟動農地成熟 tick（每 15 秒檢查一次，成熟的幼苗換成成熟小麥並廣播）。
pub fn spawn_farm_tick() {
    tokio::spawn(async move {
        let _ = hub(); // 觸發 hub 初始化
        let mut ticker = tokio::time::interval(Duration::from_secs(15));
        loop {
            ticker.tick().await;
            tick_farm();
        }
    });
}

/// 農地成熟 tick——找所有已成熟的幼苗，換成成熟小麥，廣播給所有連線。
/// 純同步、短鎖即釋（farm 讀鎖 → drop → farm 寫鎖 → drop → delta 寫鎖 → drop → broadcast）。
fn tick_farm() {
    let now = vfarm::now_secs();
    // 先讀鎖取成熟座標清單，馬上釋放。
    let mature: Vec<(i32, i32, i32)> = hub().farm.read().unwrap().mature_plots(now);
    if mature.is_empty() {
        return;
    }
    for (fx, fy, fz) in mature {
        // 寫鎖清掉農地記錄（避免下輪重複處理）。
        hub().farm.write().unwrap().remove(fx, fy, fz);
        // delta 寫鎖：把方塊從 FarmSoilSeeded(12) 換成 WheatMature(13)。
        voxel::set_block(&mut hub().deltas.write().unwrap(), fx, fy, fz, Block::WheatMature);
        // 廣播方塊更新（所有連線玩家即時看到小麥變金黃）。
        broadcast_block(fx, fy, fz, Block::WheatMature);
    }
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

/// `GET /voxel/feed` — 回傳最新 30 筆世界動態事件（最新在前）。
///
/// 純讀 jsonl 檔案、無鎖、零 LLM、向後相容（檔案不存在回空陣列）。
pub async fn voxel_feed_handler() -> axum::response::Response {
    use axum::http::header;
    let events = vfeed::load_recent_feed(vfeed::FEED_LIMIT);
    let body = serde_json::to_string(&events).unwrap_or_else(|_| "[]".into());
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// `GET /voxel/affinity?player=<顯示名>` — 回傳此玩家與各居民的好感度計數。
///
/// JSON 格式：`{ "vox_res_0": 2, "vox_res_1": 0, ... }`
/// 純讀 memory store、無 LLM、無 migration、向後相容（新路由 additive）。
pub async fn voxel_affinity_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::http::header;
    let player_name = params.get("player").map(|s| s.as_str()).unwrap_or("").trim().to_string();
    // 短鎖快照各居民的好感度計數 → 立即釋放。
    let counts: std::collections::HashMap<String, usize> = {
        let mem = hub().memory.read().unwrap();
        (0..RESIDENT_COUNT)
            .map(|i| {
                let rid = format!("vox_res_{i}");
                let count = if player_name.is_empty() { 0 } else { mem.affinity_count(&player_name, &rid) };
                (rid, count)
            })
            .collect()
    };
    let body = serde_json::to_string(&counts).unwrap_or_else(|_| "{}".into());
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// 一次居民世界推進：套用上輪思考的決策 → 物理/閒晃 → 社交互動 → 廣播 → 排程新一輪思考。
fn tick_residents(dt: f32) {
    // 0) 推進世界時鐘（短鎖即釋，不巢狀）。晝夜循環 v1。
    { hub().world_time.write().unwrap().tick(dt); }

    // 0b) 讀取目前時段 + 偵測時段轉換（日夜作息 v1）。
    //     短鎖讀 time → drop；短鎖寫 last_phase → drop，不與其他鎖巢狀。
    let phase = { hub().world_time.read().unwrap().phase() };
    let speed_mult = vt::wander_mult(phase);
    let extra_wait = vt::rest_wait_extra(phase);
    // say_updates 提前宣告，過渡台詞與建造台詞共用同一張 Vec，在末尾一次套用。
    let mut say_updates: Vec<(String, String)> = Vec::new();
    {
        let mut last = hub().last_phase.lock().unwrap();
        if *last != phase {
            // 時段切換：挑一句過渡台詞（seed 用 phase 值合一個確定性數，可測）。
            let seed = rand::random::<u32>();
            if let Some(text) = vt::transition_phrase(phase, seed) {
                // 讓距玩家最近的居民冒台詞（露娜 vox_res_0 在原點，最常被看到）。
                say_updates.push(("vox_res_0".to_string(), text.to_string()));
                // 動態 Feed：記錄時段切換事件（夜間/黎明各一筆，讓離線玩家也知道）。
                let feed_kind = match phase {
                    TimePhase::Night | TimePhase::Evening => "入夜",
                    TimePhase::Dawn => "黎明",
                    _ => "",
                };
                if !feed_kind.is_empty() {
                    vfeed::append_feed(feed_kind, "露娜", text);
                }
            }
            *last = phase;
        }
    } // last_phase mutex 在此釋放

    // 1) 先取走上輪 async 思考投回的決策（短鎖、不 await）。
    let decisions = hub().agent_bus.drain();

    // 2) 同步推進：套決策 + 物理/閒晃。deltas(read) + residents(write) 都只在這段持有、不 await。
    //    需要思考的居民這裡只蒐集「快照」，spawn 留到鎖釋放後。
    let mut think_jobs: Vec<(String, &'static str, ResidentPersona, f32, f32)> = Vec::new();

    // 主動招呼用：先短鎖快照所有玩家水平座標＋顯示名 → drop（循序取放、不與居民鎖巢狀，守鎖紀律）。
    // 好感度招呼需要知道「最近的是誰」，故多快照一份 name。
    let player_pts: Vec<(f32, f32, String)> = {
        let players = hub().players.read().unwrap();
        players.values().map(|p| (p.x, p.z, p.name.clone())).collect()
    }; // 玩家讀鎖在此釋放

    // embodied 靠近說話 v1：玩家對話泡泡倒數（短鎖、不巢狀）。say_timer 歸零就清空 say，
    // 下方 broadcast_players 自然把「泡泡消失」推給所有人。
    {
        let mut players = hub().players.write().unwrap();
        for p in players.values_mut() {
            if p.say_timer > 0.0 {
                p.say_timer -= dt;
                if p.say_timer <= 0.0 {
                    p.say.clear();
                }
            }
        }
    } // 玩家寫鎖在此釋放

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

    // 建造候選（鎖內收集位置快照，鎖外執行放塊 / 啟動計畫 / 決定活動）。
    // 格式：(resident_id, resident_name, wx, wy, wz, resident_idx)
    let mut build_candidates: Vec<(String, &'static str, i32, i32, i32, usize)> = Vec::new();

    // 採集挖掘動作（agency v1·技能調用）：居民走到資源旁時收集，鎖外執行 set_block + 入袋 + feed。
    // 格式：(resident_id, resident_name, x, y, z, 資源)
    let mut gather_mines: Vec<(String, &'static str, i32, i32, i32, vskill::GatherResource)> =
        Vec::new();

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

            // 旁聽搭話冷卻倒數（embodied 靠近說話 v1）：到期後才可再因旁聽搭話。
            if r.overhear_cooldown > 0.0 {
                r.overhear_cooldown -= dt;
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
            // 好感度 v1：查玩家記憶筆數 → 決定招呼溫度（陌生人/相識/友人，零 LLM）。
            if r.greet_timer > 0.0 {
                r.greet_timer -= dt;
            } else if r.say.is_empty() {
                if let Some((d2, nearest_name)) = nearest_player_info(r.body.x, r.body.z, &player_pts) {
                    if d2 < GREET_DIST * GREET_DIST && rand::random::<f32>() < GREET_CHANCE_PER_TICK {
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                        // 短鎖讀好感度（僅計數，不 await）→ 立即釋放記憶鎖。
                        let affinity = {
                            let mem = hub().memory.read().unwrap();
                            mem.affinity_count(nearest_name, &r.id)
                        };
                        let line = greeting_line_affinity(affinity, nearest_name, pick);
                        r.say = line.chars().take(40).collect();
                        r.say_timer = SAY_SECS;
                        r.greet_timer = GREET_COOLDOWN;
                    }
                }
            }

            // 記憶回想泡泡 v1：友人等級（好感 ≥ RECALL_AFFINITY_THRESHOLD）時，
            // 居民偶爾主動說出「我記得你說過…」——記憶第一次驅動主動社交行為。
            // 冷卻期到 + 沒在說話 + 玩家靠近 + 隨機觸發 → 短鎖讀記憶 → 生成泡泡。
            if r.recall_cooldown > 0.0 {
                r.recall_cooldown -= dt;
            } else if r.say.is_empty() {
                if let Some((d2, nearest_name)) = nearest_player_info(r.body.x, r.body.z, &player_pts) {
                    if d2 < RECALL_DIST * RECALL_DIST && rand::random::<f32>() < RECALL_CHANCE_PER_TICK {
                        // 一次性短鎖：先查好感，夠了再取最近一筆記憶（不巢狀、不持鎖 await）。
                        let top = {
                            let mem = hub().memory.read().unwrap();
                            if mem.affinity_count(nearest_name, &r.id) >= vmem::RECALL_AFFINITY_THRESHOLD {
                                mem.recall(&r.id, nearest_name, 1)
                            } else {
                                Vec::new()
                            }
                        }; // 記憶讀鎖在此釋放
                        if let Some(entry) = top.into_iter().next() {
                            let bubble = vmem::recall_bubble(&entry.summary);
                            r.say = bubble.chars().take(40).collect();
                            r.say_timer = SAY_SECS;
                            r.recall_cooldown = RECALL_COOLDOWN_SECS;
                        }
                    }
                }
            }

            // agency v1·採集技能執行（技能調用骨架：找目標→走過去→動作）。
            // 若正在採集：朝鎖定的資源走；走到旁邊→排程挖掘（鎖外 set_block）；逾時→放棄。
            // 採集中時跳過閒晃/歸巢邏輯（這一刀是「她真的在做事」）。
            if r.gather.is_some() {
                let (tx, ty, tz, reached, timed_out) = {
                    let g = r.gather.as_mut().unwrap();
                    g.timeout -= dt;
                    let reached = vskill::within_gather_reach(r.body.x, r.body.z, g.tx, g.tz);
                    (g.tx, g.ty, g.tz, reached, g.timeout <= 0.0)
                };
                if reached {
                    // 走到了：排程挖掘 + 採集次數 +1，清掉任務（站定落重力）。
                    let res = r.gather.take().unwrap().resource;
                    gather_mines.push((r.id.clone(), r.name, tx, ty, tz, res));
                    r.gathered_since_build = r.gathered_since_build.saturating_add(1);
                    vr::gravity_step(&world, &mut r.body, dt);
                } else if timed_out {
                    // 走不到（地形擋路等）→ 放棄這次採集，下個 agency tick 再決定。
                    r.gather = None;
                    vr::gravity_step(&world, &mut r.body, dt);
                } else {
                    // 朝資源方塊中心走（沿牆滑行、踏階由物理處理）。
                    let (bx, bz) = (r.body.x, r.body.z);
                    vr::step_toward(
                        &world, &mut r.body,
                        tx as f32 + 0.5, tz as f32 + 0.5,
                        dt, vr::RES_SPEED * speed_mult,
                    );
                    if let Some(yaw) = vr::yaw_from_move(r.body.x - bx, r.body.z - bz) {
                        r.yaw = yaw;
                    }
                }
            } else if r.wait_timer > 0.0 {
                // 小歇：原地落重力（站穩、不亂飄）。
                r.wait_timer -= dt;
                vr::gravity_step(&world, &mut r.body, dt);
            } else {
                let (bx, bz) = (r.body.x, r.body.z);
                // 日夜作息 v1：夜間/入夜以 speed_mult 降速（重力不受影響）。
                let reached = vr::step_toward(&world, &mut r.body, r.target_x, r.target_z, dt, vr::RES_SPEED * speed_mult);
                if let Some(yaw) = vr::yaw_from_move(r.body.x - bx, r.body.z - bz) {
                    r.yaw = yaw;
                }
                if reached {
                    // 挑下一個閒晃目標 + 小歇片刻。
                    // 以「家域感知中心」為基準：超出家域則歸巢，否則原地自由閒晃。
                    let angle = rand::random::<f32>() * std::f32::consts::TAU;
                    // 日夜作息 v1：夜間閒晃半徑隨速度乘數縮小（居民不往遠處跑）。
                    let radius = (WANDER_MIN_R + rand::random::<f32>() * (WANDER_MAX_R - WANDER_MIN_R)) * speed_mult.max(0.4);
                    let (wcx, wcz) = vr::wander_center(
                        r.body.x, r.body.z,
                        r.home_x, r.home_z,
                        vr::HOME_RADIUS,
                    );
                    let (tx, tz) = vr::wander_target(wcx, wcz, angle, radius);
                    r.target_x = tx;
                    r.target_z = tz;
                    // 日夜作息 v1：夜間額外多停一段（extra_wait），讓居民在原地駐足更久。
                    r.wait_timer = 1.0 + rand::random::<f32>() * 3.0 + extra_wait;
                }
            }

            // 思考排程（蒐集快照，spawn 留到鎖外）。
            r.think_timer -= dt;
            if r.think_timer <= 0.0 {
                r.think_timer = npc_agent_wire::THINK_INTERVAL_SECS;
                think_jobs.push((r.id.clone(), r.name, r.persona, r.body.x, r.body.z));
            }

            // agency tick 倒數；到期且「沒在採集」時才加入候選（採集中不打斷、交給技能跑完）。
            // 只收快照，實際放塊 / 決定活動在鎖外執行。
            r.build_tick -= dt;
            if r.build_tick <= 0.0 && r.gather.is_none() {
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
    for (speaker_id, speaker_name, listener_id, listener_name, line, is_response) in &social_events {
        if let Some(summary) = vrel::overhear_summary(speaker_name, line) {
            let entry = {
                let mut soc = hub().social.write().unwrap();
                soc.record_overheard(listener_id, speaker_id, &summary)
            }; // social 寫鎖在此釋放
            vrel::append_social(&entry);
        }
        // 動態 Feed：只記發起對話那筆（避免對話重複），後端在鎖外呼叫。
        if !is_response {
            let detail = format!("對{}說：「{}」", listener_name, line.chars().take(30).collect::<String>());
            vfeed::append_feed("鄰里閒聊", speaker_name, &detail);
        }
    }

    // 5) 無鎖 spawn 思考（LLM）。整個 agent 思考可由 BUTFUN_NPC_AGENT=0 關掉，
    //    關掉後居民仍照常閒晃移動，只是不冒 LLM 心裡話/心願（零額外成本）。
    if npc_agent_wire::agents_enabled() {
        for (id, name, persona, x, z) in think_jobs {
            spawn_resident_think(id, name, persona, x, z);
        }
    }

    // 5b) 採集挖掘執行（agency v1·技能調用收尾）：居民走到資源旁 → 真的挖掉 → 入小背包。
    //     鎖序：deltas 寫（即釋）→ broadcast → res_inv 寫（即釋）→ 持久化/Feed（鎖外）。
    //     **她真的在做事**：玩家會看到地表被挖出一個洞、feed 出現「採集了草皮」。
    for (rid, rname, gx, gy, gz, res) in gather_mines {
        // 只在目標方塊「現在仍是該資源」時才挖（防別人先挖走→空挖）。
        let still_there = {
            let world = hub().deltas.read().unwrap();
            voxel::effective_block_at(&world, gx, gy, gz) == res.block()
        }; // deltas 讀鎖釋放
        if !still_there {
            continue;
        }
        // 挖掉（設成空氣）。
        {
            let mut world = hub().deltas.write().unwrap();
            voxel::set_block(&mut world, gx, gy, gz, Block::Air);
        } // deltas 寫鎖釋放
        broadcast_block(gx, gy, gz, Block::Air);
        // 持久化這次世界改動（重啟後挖的洞還在）。
        vbuild::append_world_block(gx, gy, gz, Block::Air as u8);
        // 入居民小背包（純記憶體）。
        {
            let mut inv = hub().res_inv.write().unwrap();
            *inv.entry(rid.clone()).or_default().entry(res.block_id()).or_insert(0) += 1;
        } // res_inv 寫鎖釋放
        // 里程碑 Feed（真實事件、低頻、不洗版）。
        vfeed::append_feed("採集", rname, &format!("採集了{}", res.display_name()));
        // 冒一句採集泡泡（不打斷其他話）。
        say_updates.push((rid.clone(), format!("採到{}了～", res.display_name())));
    }

    // 6) 居民 agency（目標+記憶驅動）：蓋造不重複、有進展、會持久 + 採集技能調用。
    //    流程：① 有計畫 → 彈下一塊放置（持久化）；完成 → 記下「蓋過這種」（不再重蓋）+ 完工 Feed
    //           ② 無計畫 → choose_activity（依已完成清單+心願，永不重選蓋過的）→ 採集 or 蓋下一個
    //    say_updates 在 tick_residents 頂層宣告（含過渡/採集/建造台詞），最後一次性套用。

    for (rid, rname, rx, _ry, rz, _ridx) in build_candidates {
        let has_plan = hub().builds.read().unwrap().has_plan(&rid); // drop

        if !has_plan {
            // ── 無計畫：挑下一個活動（目標+記憶驅動，不鬼打牆）──────────────────
            // 已完成的建物種類（持久 GoalStore）+ 玩家心願（可選對應建物）+ 已採集次數。
            let done_kinds = hub().goals.read().unwrap().done_kinds(&rid); // drop
            let desired_kind: Option<vbuild::BuildKind> = {
                let des = hub().desires.read().unwrap();
                des.get_desire(&rid).and_then(|d| vbuild::classify_desire(&d.desire))
            }; // desires 讀鎖釋放
            let gathered = {
                let residents = hub().residents.read().unwrap();
                residents.iter().find(|r| r.id == rid).map_or(0, |r| r.gathered_since_build)
            }; // residents 讀鎖釋放

            match vskill::choose_activity(&done_kinds, desired_kind, gathered, GATHER_QUOTA) {
                NextActivity::Gather => {
                    start_gather(&rid, rx, rz);
                }
                NextActivity::Build(kind) => {
                    // 建造位置以「家域中心」為基準、依已蓋數量散開（不疊在舊建物上）。
                    let done_count = done_kinds.len();
                    start_build(&rid, rname, kind, done_count, &mut say_updates);
                }
                NextActivity::Wander => {
                    // 全部蓋完：偶爾散心採集（低頻、不洗版），否則純閒晃。
                    if rand::random::<f32>() < IDLE_GATHER_CHANCE {
                        start_gather(&rid, rx, rz);
                    }
                }
            }
            // 重設 agency tick 等下次（採集中不會再進這裡，見 build_candidate 閘）。
            reset_build_tick(&rid);
            continue;
        }

        // ── 有計畫：彈下一塊放置 + 持久化 + 進度冒泡 ──────────────────────────
        let (next_block, kind_name, kind_str, progress_pct, plan_done) = {
            let mut builds = hub().builds.write().unwrap();
            if let Some(plan) = builds.get_plan_mut(&rid) {
                let bb = plan.pop_next();
                let kn = plan.kind_name.clone();
                let ks = plan.kind.clone();
                let pct = plan.progress_pct();
                let done = plan.is_done();
                (bb, kn, ks, pct, done)
            } else {
                (None, String::new(), String::new(), 100, true)
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
                // 持久化這塊（重啟後蓋的東西還在）。
                vbuild::append_world_block(bb.x, bb.y, bb.z, bb.b);

                // 持久化更新後的計畫（remaining 已縮短，重啟後接著蓋）
                if let Some(plan) = hub().builds.read().unwrap().plans.get(&rid) {
                    vbuild::append_build(plan);
                } // builds 讀鎖釋放

                // 進度冒泡：50% / 95% 各冒一次（完工 Feed 改由 plan_done 統一發、不重複）
                if progress_pct == 50 || progress_pct >= 95 {
                    let say = vbuild::build_say_line(&kind_name, progress_pct);
                    say_updates.push((rid.clone(), say));
                }
            }
        }

        if plan_done {
            {
                let mut builds = hub().builds.write().unwrap();
                builds.remove_if_done(&rid);
            } // builds 寫鎖釋放
            // 記下「這位居民蓋過這種建物」→ 之後永不重蓋（不鬼打牆）+ 持久化。
            if let Some(kind) = vbuild::BuildKind::from_str(&kind_str) {
                let rec = {
                    let mut goals = hub().goals.write().unwrap();
                    goals.mark_done(&rid, kind)
                }; // goals 寫鎖釋放
                if let Some(rec) = rec {
                    vskill::append_goal(&rec);
                }
            }
            // 完工 Feed（每個建物只發一次，不洗版）。
            vfeed::append_feed("蓋家完工", &rname, &kind_name);
            // 蓋完一個 → 重置採集計數，下一輪先採料再蓋下一種（有進展感）。
            {
                let mut residents = hub().residents.write().unwrap();
                if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                    r.gathered_since_build = 0;
                }
            } // residents 寫鎖釋放
        }

        reset_build_tick(&rid);
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

// ── agency v1 輔助（全在 tick_residents 鎖釋放後呼叫；各短鎖即釋、不巢狀、不 await）──────

/// 開始一次採集任務：以 (rx,rz) 為原點找最近資源 → 設居民的 gather 技能狀態。
/// 找不到資源（罕見）→ 視為已備料（gathered=配額），下個 agency tick 直接蓋，避免卡死。
fn start_gather(rid: &str, rx: i32, rz: i32) {
    let found = {
        let world = hub().deltas.read().unwrap();
        vskill::find_nearest_resource(&world, rx, rz, vskill::GATHER_MAX_RADIUS)
    }; // deltas 讀鎖釋放
    let mut residents = hub().residents.write().unwrap();
    if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
        match found {
            Some((tx, ty, tz, res)) => {
                r.gather = Some(GatherSkill {
                    resource: res,
                    tx,
                    ty,
                    tz,
                    timeout: vskill::GATHER_TIMEOUT_SECS,
                });
            }
            None => {
                // 附近沒可採資源 → 當作備料完成（不卡在採集前置）。
                r.gathered_since_build = GATHER_QUOTA;
            }
        }
    }
}

/// 開始蓋一個建物：以「家域中心」為基準、依已蓋數量散開錨點 → 建計畫 → 動工 Feed + 冒泡。
fn start_build(
    rid: &str,
    rname: &str,
    kind: vbuild::BuildKind,
    done_count: usize,
    say_updates: &mut Vec<(String, String)>,
) {
    let (ox, oz) = vskill::build_offset(done_count);
    let (hx, hz) = {
        let residents = hub().residents.read().unwrap();
        residents
            .iter()
            .find(|r| r.id == rid)
            .map(|r| (r.home_x, r.home_z))
            .unwrap_or((0.0, 0.0))
    }; // residents 讀鎖釋放
    let bx = hx.floor() as i32 + ox;
    let bz = hz.floor() as i32 + oz;
    let by = vbuild::surface_y(bx, bz);
    let plan = {
        let mut builds = hub().builds.write().unwrap();
        if builds.has_plan(rid) {
            None // double-check 並發安全
        } else {
            Some(builds.new_plan(rid, kind, bx, by, bz))
        }
    }; // builds 寫鎖釋放
    if let Some(p) = plan {
        vbuild::append_build(&p);
        say_updates.push((rid.to_string(), vbuild::build_say_line(&p.kind_name, 0)));
        vfeed::append_feed("蓋家動工", rname, &p.kind_name);
    }
}

/// 重設某居民的 agency tick 倒數（下次再決策／放塊）。
fn reset_build_tick(rid: &str) {
    let mut residents = hub().residents.write().unwrap();
    if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
        r.build_tick = BUILD_INTERVAL_SECS;
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
    fn resolve_identity_prefers_account_over_join_name() {
        // 登入帳號優先：即使 join 自報別的名字（甚至想冒充別人），也以帳號名為準。
        assert_eq!(
            resolve_identity(Some("諾娃"), Some("冒充者")),
            "諾娃"
        );
        // 訪客（無帳號）→ 用 join 顯示名。
        assert_eq!(resolve_identity(None, Some("旅行者")), "旅行者");
        // 帳號名空白／全空白 → 退回 join 名（不會綁到空字串鍵）。
        assert_eq!(resolve_identity(Some("   "), Some("阿一")), "阿一");
        // 兩者皆無 → 預設「旅人」。
        assert_eq!(resolve_identity(None, None), "旅人");
        assert_eq!(resolve_identity(Some(""), Some("")), "旅人");
        // 去頭尾空白 + 截斷 24 字（與入場清洗一致）。
        assert_eq!(resolve_identity(None, Some("  邊緣  ")), "邊緣");
        let long: String = "字".repeat(30);
        assert_eq!(resolve_identity(None, Some(&long)).chars().count(), 24);
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
        // 顯式對象（點居民 / 走近面對）：resident_id = Some。
        let m: ClientMsg =
            serde_json::from_str(r#"{"t":"talk","resident_id":"vox_res_0","text":"你好"}"#).unwrap();
        match m {
            ClientMsg::Talk { resident_id, text } => {
                assert_eq!(resident_id.as_deref(), Some("vox_res_0"));
                assert_eq!(text, "你好");
            }
            _ => panic!("應解析成 Talk"),
        }
    }

    #[test]
    fn range_talk_parses_without_resident_id() {
        // embodied 範圍說話：不帶 resident_id（常駐輸入列）→ resident_id = None。
        let m: ClientMsg = serde_json::from_str(r#"{"t":"talk","text":"嗨大家"}"#).unwrap();
        match m {
            ClientMsg::Talk { resident_id, text } => {
                assert_eq!(resident_id, None);
                assert_eq!(text, "嗨大家");
            }
            _ => panic!("應解析成 Talk"),
        }
        // 顯式 null 也視為範圍說話。
        let m2: ClientMsg =
            serde_json::from_str(r#"{"t":"talk","resident_id":null,"text":"哈囉"}"#).unwrap();
        match m2 {
            ClientMsg::Talk { resident_id, .. } => assert_eq!(resident_id, None),
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
    fn greeting_line_affinity_wraps_and_non_empty() {
        // affinity=0（陌生人），索引取模、永遠回得出非空招呼句。
        for i in 0..20 {
            assert!(!greeting_line_affinity(0, "", i).is_empty());
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
    fn nearest_player_info_works() {
        // 沒有玩家 → None。
        let empty: Vec<(f32, f32, String)> = vec![];
        assert!(nearest_player_info(0.0, 0.0, &empty).is_none());
        // 多名玩家：取最近者（距離 + 名字）。
        let pts = vec![
            (3.0, 4.0, "遠人".to_string()),
            (1.0, 0.0, "近人".to_string()),
            (10.0, 10.0, "最遠".to_string()),
        ];
        let (d2, name) = nearest_player_info(0.0, 0.0, &pts).unwrap();
        assert!((d2 - 1.0).abs() < 1e-4, "最近者平方距離應為 1：{d2}");
        assert_eq!(name, "近人", "最近者名字應為 '近人'");
    }

    #[test]
    fn greeting_line_affinity_stranger_is_generic() {
        // affinity=0 → 陌生人招呼，不帶名字。
        let g = greeting_line_affinity(0, "小明", 0);
        assert!(!g.contains("小明"), "陌生人招呼不應含玩家名：{g}");
    }

    #[test]
    fn greeting_line_affinity_acquaintance_contains_name() {
        // affinity=1–2 → 相識招呼，應帶名字。
        for aff in [1usize, 2] {
            let g = greeting_line_affinity(aff, "小明", 0);
            assert!(g.contains("小明"), "相識招呼應含玩家名 (aff={aff})：{g}");
        }
    }

    #[test]
    fn greeting_line_affinity_friend_contains_name() {
        // affinity>=3 → 友人招呼，應帶名字且更親密。
        let g = greeting_line_affinity(3, "小明", 0);
        assert!(g.contains("小明"), "友人招呼應含玩家名：{g}");
    }

    #[test]
    fn greeting_line_affinity_empty_name_is_safe() {
        // 名字空字串時 → 安全退回通用招呼，不 panic。
        let g = greeting_line_affinity(5, "", 0);
        assert!(!g.is_empty(), "空名字時應仍有招呼句");
    }

    #[test]
    fn greeting_line_affinity_long_name_truncated() {
        // 超長名字：招呼長度不應超過一定範圍（不塞爆泡泡）。
        let long_name = "超級無敵長名字玩家甲乙丙丁戊";
        let g = greeting_line_affinity(2, long_name, 0);
        assert!(g.chars().count() <= 30, "招呼不應超長：{g}");
    }

    #[test]
    fn place_block_id_validates() {
        // 合法 id → Some；越界 → None（伺服器據此忽略 place）。
        assert_eq!(Block::from_u8(3), Some(Block::Stone));
        assert!(Block::from_u8(200).is_none());
    }
}
