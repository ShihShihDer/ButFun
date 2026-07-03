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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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
// 指令→任務 + 整地技能 v1（純邏輯模組）：以 #[path] 掛成 voxel_ws 的子模組，
// 免動 main.rs（本檔是唯一使用者）。偵測整地指令、整地任務模型、整地技能核心全在此。
#[path = "voxel_directed_task.rs"]
mod voxel_directed_task;
use self::voxel_directed_task::{self as vdt, CoordinatedLevelTask, DirectedTask};
// 跑腿採集 v1（指令→任務第三刀，純邏輯模組）：同樣以 #[path] 掛成 voxel_ws 的子模組，
// 免動 main.rs（本檔是唯一使用者）。偵測跑腿指令、任務資料模型、台詞全在此。
#[path = "voxel_fetch.rs"]
mod voxel_fetch;
use self::voxel_fetch::{self as vfetch, FetchTask};
use crate::voxel_building::{self as vbuild, BuildStore};
use crate::voxel_skills::{self as vskill, GatherSkill, GoalStore, NextActivity};
use crate::voxel_invent as vinvent;
use crate::voxel_desires::{self as vdes, DesireStore};
use crate::voxel_diary;
use crate::voxel_feed as vfeed;
use crate::voxel_craft as vcraft;
use crate::voxel_inventory::{self as vinv, InvStore};
use crate::voxel_memory::{self as vmem, VoxelMemory};
use crate::voxel_farm::{self as vfarm, FarmStore};
use crate::voxel_grove::{self as vgrove, GroveStore};
use crate::voxel_gift as vgift;
use crate::voxel_keepsake as vkeep;
use crate::voxel_seedgift as vseed;
use crate::voxel_giftgarden as vgg;
use crate::voxel_fishing as vfish;
use crate::voxel_smelt as vsmelt;
use crate::voxel_return_gift::{self as vret, ReturnGiftStore};
use crate::voxel_admire as vadmire;
use crate::voxel_preference as vpref;
use crate::voxel_overhear as vh;
use crate::voxel_relations::{self as vrel, SocialStore};
use crate::voxel_residents::{self as vr, Body};
use crate::voxel_roster as vroster;
use crate::voxel_time::{self as vt, WorldTime, TimePhase};
use crate::voxel_announce as vannounce;
use crate::voxel_bonds::{self as vbonds, ResidentBonds};
use crate::voxel_trade::{self as vtrade, TradeOffer};
use crate::voxel_visit as vvisit;
use crate::voxel_fond_greeting as vfond;
use crate::voxel_gossip as vgossip;
use crate::voxel_mood;
use crate::voxel_comfort as vcomfort;
use crate::voxel_cheer as vcheer;
use crate::voxel_chest as vchest;
use crate::voxel_sign as vsign;
use crate::voxel_readsign as vreadsign;
use crate::voxel_tend as vtend;
use crate::voxel_nameplate as vnameplate;
use crate::voxel_neighborsign as vneighsign;
use crate::voxel_neighborvisit as vneighvisit;
use crate::voxel_callingcard as vcard;
use crate::voxel_savor as vsavor;
use crate::voxel_meal as vmeal;
use crate::voxel_self_image as vself;
use crate::voxel_playerepithet as vepi;
use crate::voxel_epithet_spread as vespread;
use crate::voxel_epithet_esteem as vesteem;
use crate::voxel_epithet_sign as vepisign;
use crate::voxel_hosted_visit as vhosted;
use crate::voxel_weather as vweather;
use crate::voxel_clique as vclique;
use crate::voxel_quarrel as vquarrel;
use crate::voxel_teach as vteach;
use crate::voxel_sleep as vsleep;
use crate::voxel_bedtime as vbedtime;
use crate::voxel_morning as vmorning;
use crate::voxel_daybreak as vdaybreak;
use crate::voxel_reunion as vreunion;
use crate::voxel_expedition as vexp;
use crate::voxel_welcome as vwelcome;
use crate::voxel_resident_trade as vrtrade;
use crate::voxel_share as vshare;
use crate::voxel_milestones::{self as vmiles, MilestoneStore};
use crate::voxel_player_pos as vpp;

// 水流動模擬純邏輯（來源不乾涸、破口會流、離源太遠乾涸）。
// 用 `#[path]` 把它掛成 voxel_ws 的私有子模組——**不動 main.rs**（守「別碰 main.rs」邊界），
// 同時讓水流的方塊 id / 演算法有獨立、可測的檔案。核心純函式在 voxel_water.rs，
// 接線（佇列、tick、廣播、持久化）在本檔，嚴守既有無鎖 await + 短鎖即釋鐵律。
#[path = "voxel_water.rs"]
mod voxel_water;
use voxel_water as vwater;

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
    /// 特殊身分稱號（維護者的專屬身分，如「引夢使者」「築夢工匠」）：由後端登入帳號的 email/顯示名
    /// 判定（見 `special_title`），不信客戶端自報；廣播給所有連線，前端據此在他頭上渲染金色稱號牌。
    /// 一般玩家 / 訪客為 None（不序列化成欄位），完全不受影響。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

// ── 乙太方界 AI 居民（切片③）────────────────────────────────────────────────
//
// 讓「靈魂」也活在 voxel 新世界：幾位 AI 居民站在地形上（重力＋逐軸碰撞＋踏階，純物理
// 在 voxel_residents.rs）、會閒晃、被既有 npc_agent「腦袋」低頻驅動偶爾冒一句心裡話/心願。
// 採集/蓋家留切片④（禱告蓋家）。

/// 乙太方界**初始**居民人數（世界首建就有的固定 4 位，id vox_res_0..3）。
/// 人口成長 v1 之後這只是「起點」——聚落穩定時會偶爾誕生新居民（見 `voxel_roster`），
/// 實際在世人口由 [`resident_count`] 回報，上限由 [`vroster::max_residents`] 守住。
const RESIDENT_COUNT: usize = 4;

/// 目前在世居民數（含出生後誕生的）。id 恆為連續的 vox_res_0..{RESIDENT_POP-1}——
/// 名冊**只增不減**（新生兒 append，既有 id 永不回收），故各處「枚舉全體居民 id」
/// 可安全用 `(0..resident_count())`。啟動時由 `init_residents` 設為（4 + 名冊人數），
/// 每誕生一位在 residents 寫鎖內 +1（見 `maybe_birth`）。無鎖讀取、避免再入死鎖。
static RESIDENT_POP: AtomicUsize = AtomicUsize::new(RESIDENT_COUNT);

/// 目前在世居民數（無鎖、便宜）。所有「枚舉全體居民 id」的地方一律用它，不要再用固定常數。
fn resident_count() -> usize {
    RESIDENT_POP.load(Ordering::Relaxed)
}

/// 上次出生的 unix 秒（人口成長 v1·出生節流）。0＝尚未從持久化載入（首次 `maybe_birth`
/// 會嘗試載入 `data/voxel_last_birth`；有值就用值繼續計算 elapsed，無值才記 now 為基準）。
/// 重啟後由檔案還原，elapsed 跨重啟累積——修正 prod 每 15 分重啟導致永遠生不出居民的 bug。
static LAST_BIRTH_UNIX: AtomicU64 = AtomicU64::new(0);
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
/// 協助建造感激記憶冷卻（秒，互動有後果 v2）：一次連續幫忙（放好幾塊方塊）只記**一筆**
/// 感激記憶，隔這麼久後再幫才會再記一筆——避免好感（＝episodic 記憶筆數）被單次幫忙灌爆。
const HELP_MEMORY_COOLDOWN_SECS: u64 = 90;
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
/// 心情自語冷卻（秒，ROADMAP 677）：冷卻到期後依心情自發冒一句泡泡；
/// 初始值在各居民間錯開，避免同 tick 全員一起說話。
const MOOD_SAY_COOLDOWN: f32 = 120.0;
/// 居民建造頻率：每隔這麼多秒放一塊方塊（慢節奏，讓玩家能目睹過程）。
const BUILD_INTERVAL_SECS: f32 = 8.0;
/// 建物完工後的建造冷卻（秒）：任一建物蓋好後這麼久內同居民不動工新建物。
/// 頻率保險：即使持久 flag 因故失效，也不會連發完工洗版 Feed（每座至少隔這麼久）。
/// 5 分鐘＝比正常建造間隔長得多、又不至於讓「正常一座接一座蓋」感覺卡頓。
const BUILD_COOLDOWN_SECS: f32 = 300.0;
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
    /// 讀牌冷卻倒數（秒，居民讀牌 v1）：> 0 表示最近念過附近的告示牌，暫不再讀，稀少才有感。
    read_sign_timer: f32,
    /// 上一塊「寫進記憶」的牌面文字（居民讀牌 v2）：讀到同一塊牌重複念沒關係，但只有
    /// 讀到**不同於上次**的牌才再寫一筆記憶——避免反覆讀同一塊牌把 episodic 記憶塞滿、
    /// 擠掉真實玩家的對話記憶。純記憶體、重啟歸零。
    last_read_sign: Option<String>,
    /// 照料菜園冷卻倒數（秒，居民照料 v1 ROADMAP 753）：> 0 表示最近幫玩家照料過作物，
    /// 暫不再照料——稀少才有感，也避免連續 tick 刷成長。純記憶體、重啟歸零。
    tend_timer: f32,
    /// 居民↔居民社交冷卻倒數（秒）：> 0 表示最近主動搭話過另一位居民，尚不可再發起。
    social_cooldown: f32,
    /// 另一位居民剛搭話，等這秒數到期後回應（id, 名字, 剩餘秒）。
    pending_response: Option<(String, String, f32)>,
    /// 建造 tick 倒數（秒）：降到 0 時嘗試放一塊或啟動新計畫；錯開避免同 tick 全員觸發。
    build_tick: f32,
    /// 建造冷卻倒數（秒，蓋家鬼打牆頻率保險）：任一建物完工後設為 [`BUILD_COOLDOWN_SECS`]，
    /// > 0 時不進建造決策（不動工新建物），純採集/社交/閒晃。這是最後一道保險——即使
    /// 持久 flag 因故失效，同居民也不會在短時間內連發完工洗版 Feed（每座至少隔數分鐘）。
    build_cooldown: f32,
    /// 記憶回想泡泡冷卻（秒）：> 0 表示最近剛回想過，尚不可再觸發（稀少才有感）。
    recall_cooldown: f32,
    /// 旁聽搭話冷卻（秒，embodied 靠近說話 v1）：> 0 表示最近因旁聽搭過一句，
    /// 尚不可再搭（防同一位連發、對話風暴）。零 LLM。
    overhear_cooldown: f32,
    /// 當前採集任務（居民 agency v1·技能調用）：Some = 正走向某資源要挖；None = 沒在採集。
    gather: Option<GatherSkill>,
    /// 本輪（自上次蓋造後）已採集次數：達 GATHER_QUOTA 才開始蓋下一個建物（備料感）。
    gathered_since_build: u32,
    /// 卡住計時（秒）：想動卻幾乎沒位移時累加，達 vr::STUCK_SECS → 脫困/送回家域。
    stuck_timer: f32,
    /// 探訪目標（ROADMAP 671）：Some(目標 home_x, home_z, 目標居民名) = 正在前往或停留；
    /// None = 在自己家域（正常閒晃/採集/蓋造）。
    visiting: Option<(f32, f32, String)>,
    /// 探訪抵達後的逗留倒數（秒）：> 0 = 正在鄰居家逗留，到 0 時啟程返家。
    visit_stay_timer: f32,
    /// 探訪冷卻倒數（秒）：> 0 = 冷卻中，不可發起新探訪。
    visit_cooldown: f32,
    /// 心情自語冷卻倒數（秒，ROADMAP 677）：歸零後依心情層級自發冒泡泡，再重置。
    mood_say_cooldown: f32,
    /// 孤獨尋伴冷卻倒數（秒，ROADMAP 678）：Lonely 心情時歸零則走向最近玩家。
    seek_comfort_cooldown: f32,
    /// 正在尋伴：已設目標走向玩家，到達後冒求陪泡泡等玩家搭話。
    seeking_comfort: bool,
    /// 打氣目標（ROADMAP 679）：Some(x, z, lonely_rid) = 正走向某位 Lonely 居民打氣；
    /// None = 沒有打氣任務。抵達後冒鼓勵泡泡、任務清除。
    cheer_target: Option<(f32, f32, String)>,
    /// 打氣冷卻倒數（秒，ROADMAP 679）：歸零後才可再發起打氣。
    cheer_cooldown: f32,
    /// 互動心情補助倒數（秒，ROADMAP 681）：玩家對話/贈禮時設為正值，倒數歸零前心情提升一格。
    mood_boost_secs: f32,
    /// 整地任務·走向工地時「歷來最接近工地中心的平方水平距離」（配 level_walk_stall 偵測卡死）。
    /// 指派整地任務時重置為 f32::MAX；每 tick 走近就刷新。無整地任務時不使用。
    level_best_d2: f32,
    /// 整地任務·走向工地時「連續沒更接近的卡住秒數」：達 vdt::LEVEL_WALK_STALL_SECS →
    /// 就近挪到工地可站處（保證她真的走到、不因貪心尋路卡死而白白逾時放棄）。
    level_walk_stall: f32,
    /// 跟隨模式（指令→任務 v1 第二刀）：Some(玩家身份鍵, 剩餘秒數) = 正跟著該玩家走；
    /// None = 沒在跟隨。逾時或被要求「別跟了」即清空，回到平常閒晃/採集/建造。
    follow: Option<(String, f32)>,
    /// 小圈子聚會（ROADMAP 711）：Some(聚會點 x, z, session_tag) = 正前往/等待與圈子碰面；
    /// None = 沒有聚會任務。`session_tag` 是同組全體成員 id 排序後的串接，供辨識同一場聚會。
    clique_meet: Option<(f32, f32, String)>,
    /// 聚會等待秒數（自被指派起累計，ROADMAP 711）：超過 `GATHER_MAX_WAIT_SECS`
    /// 仍等不到其他成員到齊 → 放棄、各自散去（防某成員被地形卡住拖累整組永遠卡住）。
    clique_wait: f32,
    /// 小圈子聚會冷卻倒數（秒，ROADMAP 711）：歸零後才可能再被選入下一場聚會。
    clique_cooldown: f32,
    /// 跑腿採集任務（指令→任務第三刀）：Some = 正在幫玩家採集/交付指定材料；
    /// None = 沒有跑腿任務。與整地/跟隨互斥（接下新任務時彼此清空）。
    fetch: Option<FetchTask>,
    /// 技能發明/重用執行狀態（真進化第一刀）：Some = 正照原語序列做事
    /// （採料→合成→驗證）；None = 沒有進行中的發明/重用。deadline 每 tick 遞減。
    invent_run: Option<vinvent::InventRun>,
    /// 發明冷卻倒數（秒）：一次發明嘗試（無論成敗）後至少隔這麼久才再請 LLM 想
    /// ——發明是低頻事件（成本紀律），重用不受此冷卻影響（重用零 LLM）。
    invent_cooldown: f32,
    /// 好奇心計時（秒，北極星第三刀）：歸零＋閒置＋過機率門檻 → 從可能性目錄挑一樣
    /// 她還不會的，自發種下心願（sparked_by=好奇心）交給發明引擎——
    /// **不用玩家 push 她也會自己成長**。每位獨立、比例式錯開。
    curiosity_timer: f32,
    /// 發明失敗計數（記憶體，重啟歸零）：key = goal_block_id，value = 連敗次數。
    /// 達 [`vinvent::INVENT_BACKOFF_THRESHOLD`] 次 → 啟動退避計時、好奇心不再挑這個目標。
    invent_fail_counts: HashMap<u8, u8>,
    /// 發明退避計時（秒，記憶體，重啟歸零）：key = goal_block_id，value = 剩餘退避秒數。
    /// > 0 時好奇心的目錄排除此 goal_block、不種下心願；歸零後恢復可試。
    invent_backoff: HashMap<u8, f32>,
    /// 是否正在睡覺（日夜作息·睡覺 v1，ROADMAP 739）：深夜回到自家附近會躺下睡著，
    /// 睡著時停下一切閒晃／社交／採集／建造、名牌旁顯示 💤，天亮（離開夜間時段）才醒。
    /// 記憶體前置、不持久化、零 migration（重啟後大不了當晚重睡一次，無資料風險）。
    asleep: bool,
    /// 心中念念不忘的告示牌（居民讀牌 v3，ROADMAP 743）：Some(牌子中心 x, z, 引文) =
    /// 讀到一塊讓牠印象深刻的牌子後記下的「心中地標」；閒暇時偶爾據此重返。純記憶體、重啟歸零。
    cherished_sign: Option<(f32, f32, String)>,
    /// 心中地標其實是「哪位鄰居的家」（登門串門子 v1，ROADMAP 751）：Some(鄰居名) = 上面 `cherished_sign`
    /// 那塊牌是該鄰居親手立的自建家牌（750 認得的）；None = 是玩家立的牌。與 `cherished_sign` 同步更新，
    /// 讓日後朝聖抵達時能把「重返」升級成一次真正的「登門拜訪」。純記憶體、重啟歸零。
    cherished_neighbor: Option<String>,
    /// 正在重返心中的牌子（讀牌 v3）：Some(目標 x, z, 引文) = 正朝那塊牌子走；抵達後駐足念一句、
    /// 寫一筆「又回來看看」記憶、清空。None = 沒在朝聖。
    pilgrimage: Option<(f32, f32, String)>,
    /// 這趟朝聖走向的其實是「哪位鄰居的家」（登門串門子 v1，ROADMAP 751）：啟程時從 `cherished_neighbor`
    /// 快照，讓途中即使又讀到別的牌也不影響抵達判定。Some(鄰居名) = 抵達時當成登門拜訪；None = 獨自朝聖玩家的牌。
    pilgrimage_neighbor: Option<String>,
    /// 朝聖逾時倒數（秒，讀牌 v3）：啟程時設 [`vreadsign::PILGRIMAGE_TIMEOUT`]；未抵達時遞減，
    /// 歸零仍沒到（地形擋路等）即放棄，避免無限走。
    pilgrimage_timer: f32,
    /// 重返冷卻倒數（秒，讀牌 v3）：一次朝聖（抵達或放棄）後設為 [`vreadsign::PILGRIMAGE_COOLDOWN`]，
    /// 歸零前不再啟程——稀少才有感、不洗版。各居民初始錯開。
    pilgrimage_cooldown: f32,
    /// 門口留下的「有人來找過」心意佇列（登門撲空留心意 v1，ROADMAP 763）：某訪客登門撲空（752 判定
    /// 主人不在家）時，訪客名字塞進這裡；日後主人回到自家附近閒著時逐一感應、念一句、記一筆。
    /// 去重＋上限保護（[`vcard::MAX_PENDING_CALLERS`]）。純記憶體、重啟歸零。
    pending_callers: Vec<String>,
    /// 感應門口心意的冷卻倒數（秒，ROADMAP 763）：一次感應後設為 [`vcard::NOTICE_COOLDOWN`]，
    /// 歸零前不再感應下一張——多張心意一張一張慢慢感應、不一次倒完。各居民初始錯開。
    callingcard_cooldown: f32,
    /// 說出口自我印象的冷卻倒數（秒，自我印象 v1·ROADMAP 770）：閒暇時偶爾自言自語一句「我好像
    /// 成了村裡最愛蓋東西的人」——歸零＋過機率＋有明顯主導領域才觸發；觸發後設長冷卻，避免反覆碎念。
    /// 沒昇華出印象時設中冷卻重試（避免每 tick 白讀記憶鎖）。各居民初始大幅錯開。純記憶體、重啟歸零。
    self_image_cooldown: f32,
    /// 上次說出口的自我印象主導領域（自我印象 v3·ROADMAP 772）：`Some(領域)` = 牠先前認得自己是誰。
    /// 供偵測「自我印象轉變」——當前昇華出的主導領域若與這個不同，就是牠察覺「我不太一樣了」的一刻。
    /// `None` = 還沒說出過任何自我印象（首次昇華不算轉變）。純記憶體、重啟歸零（重啟後首句視同首次）。
    self_image_domain: Option<vself::SelfDomain>,
    /// 居民為玩家取的名號（居民為你取一個名號 v1）：`玩家顯示名 → 已昇華出的角色`。供偵測「第一次
    /// 為某玩家安下名號 / 名號改換」的那一刻（記一則動態牆）＋打招呼時用名號稱呼你。名號本身每次
    /// 打招呼都由當下記憶即時昇華（此表只記「上回是什麼」供去重），純記憶體、重啟歸零（重啟後首次
    /// 相見會重新安一次名號、動態牆再記一次，無害）。
    coined_epithets: std::collections::HashMap<String, vepi::PlayerRole>,
    /// 你的名號口耳相傳 v1（自主提案）：這位居民**沒跟你深交**、卻從相熟的老朋友口中**聽說過**你
    /// 的名號（第二手傳聞）。key＝玩家顯示名。打招呼時若還昇華不出自己的第一手名號（affinity 不到
    /// 老友門檻），但此表有你 → 用一句「久仰」的傳聞招呼喊你。純記憶體、重啟歸零（比照 `coined_epithets`）。
    heard_epithets: std::collections::HashMap<String, vespread::Hearsay>,
    /// 名號化為敬意 v1（自主提案·ROADMAP 777）：Some(玩家顯示名, 已為他昇華的名號角色) =
    /// 這位居民**已為某位在線玩家昇華出名號**（`coined_epithets` 有他），此刻正**特地放下閒晃、
    /// 走過去向他致意**；抵達（冒致意泡泡＋記城鎮動態）或玩家走遠／離線即清空。純記憶體、重啟歸零。
    /// 由**敬重**驅動（有別於 678 `seeking_comfort` 的**孤獨**驅動）。
    approaching_esteem: Option<(String, vepi::PlayerRole)>,
    /// 敬意致意冷卻倒數（秒，ROADMAP 777）：每次致意後歸此值、倒數到 0 才可能再起身。純記憶體。
    esteem_approach_cooldown: f32,
    /// 晨間思念玩家（記憶驅動·晨間思念玩家 v1，ROADMAP 746）：Some(玩家顯示名, 逾時剩餘秒) =
    /// 醒來讀昨晚睡前反思、發現惦記的是這位在線玩家，正朝他走去要打招呼；抵達（暖暖打招呼＋記一筆
    /// 與他的記憶）或逾時／玩家離線即清空。純記憶體、重啟歸零。
    daybreak_seek: Option<(String, f32)>,
    /// 久別重逢奔迎（記憶驅動·久別重逢奔迎 v1，ROADMAP 747）：Some(玩家顯示名, 逾時剩餘秒) =
    /// 某位在線玩家久別歸來、由對他記憶最厚的這位居民放下手邊的事奔去迎接；抵達（暖暖迎接＋記一筆
    /// 與他的重逢記憶）或逾時／玩家離線即清空。純記憶體、重啟歸零。
    reunion_seek: Option<(String, f32)>,
    /// 遠行探野（PLAN_ETHERVOX item 7 散居·遠行探野 v1，ROADMAP 756）：Some(邊陲落點 x, z, 方位名) =
    /// 正遠行前往遠離主城的荒野邊陲、或已抵達正在那逗留；None = 沒在遠行（正常閒晃/採集/建造）。
    /// 散居 v6（ROADMAP 762）起，能遠行的人格為 Wanderer（奧瑞·漂泊）與 FarmWorker（諾娃·尋地）
    /// ——見 `vexp::expedition_motive`。純記憶體、重啟歸零。
    expedition: Option<(f32, f32, String)>,
    /// 遠行抵達邊陲後的逗留倒數（秒，ROADMAP 756）：> 0 = 已抵達、正在遠方逗留探索，到 0 時清空
    /// `expedition`、交回一般 wander（此刻遠在家域外，`wander_center` 會把牠一路帶回家）。
    expedition_stay: f32,
    /// 遠行去程逾時倒數（秒，ROADMAP 756）：啟程時設 [`vexp::EXPEDITION_TIMEOUT`]；未抵達時遞減，
    /// 歸零仍沒到（地形擋路等）即放棄這趟遠行、不無限走。
    expedition_timer: f32,
    /// 遠行冷卻倒數（秒，ROADMAP 756）：一趟遠行（歸來或放棄）後設為 [`vexp::EXPEDITION_COOLDOWN`]，
    /// 歸零前不再啟程——稀少才有感、不洗版。各居民初始錯開。
    expedition_cooldown: f32,
    /// 邊陲過夜（散居·過夜 v4，ROADMAP 759）：true = 這位居民此刻是「睡在邊陲營地那張床上」而非睡在
    /// 主城的家。與 `asleep` 並存（`asleep` 管「正在睡」，本旗標管「睡在哪」）——醒來時據此分岔：邊陲
    /// 過夜醒來要結束遠行、啟程返家（跳過家用晨間探友），家裡睡醒才走既有晨間流程。純記憶體、重啟歸零。
    asleep_at_outpost: bool,
    /// 手中捧著、還沒享用的食物餽贈（你送的食物她會細細享用 v1，ROADMAP 765）：
    /// `Some((食物 item_id, 送禮玩家名, 剩餘延遲秒))` = 玩家剛送了一份食物，居民收下但還沒吃，
    /// 倒數歸零後在一個閒下來的安靜片刻**真的享用**（冒暖泡泡＋動態牆＋重新點亮心情）；
    /// `None` = 手中沒有待享用的食物。同時只捧一份（再收到新食物就換成最新那份）。純記憶體、
    /// 重啟歸零（享用是數十秒內的短暫過場，重啟大不了少享用一次、無資料風險，零 migration）。
    savoring: Option<(u8, String, f32)>,
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
    /// 居民當前心情 emoji（None / 省略 = 跳過更新）。ROADMAP 676。
    #[serde(skip_serializing_if = "Option::is_none")]
    mood: Option<String>,
}

/// 居民名字池（取自 resident_npc 的近城居民風格名，柔和轉寫式、與主要 NPC 一致）。
/// 居民名字池（人口成長 v1）：前 4 個是初始居民（順序與 id 綁定，**絕不更動**，
/// 向後相容既有存檔/記憶/技能 key）；之後 12 個是新生兒依 id 索引取用的名字池。
/// 長度即人口絕對天花板（見 `vroster::RESIDENT_NAME_POOL_LEN`，兩者須一致）。
/// 名字皆柔和轉寫式、與主要 NPC 一致，彼此不重複。
const RESIDENT_NAMES: [&str; vroster::RESIDENT_NAME_POOL_LEN] = [
    "露娜", "諾娃", "賽勒", "奧瑞", // 初始 4 位（id 0..3）
    "米拉", "澄兒", "蕾雅", "費恩", "星禾", "柯洛", "雅辛", "恩雅", "佩緹", "昂恩", "希雅", "洛安",
];

/// 由居民 id（"vox_res_{i}"）取其顯示名（解析失敗 / 越界皆安全退回第一位）。
fn resident_name_of(rid: &str) -> &'static str {
    let idx = rid.trim_start_matches("vox_res_").parse::<usize>().unwrap_or(0);
    RESIDENT_NAMES.get(idx).copied().unwrap_or(RESIDENT_NAMES[0])
}

/// 依居民 id 查詢牠與所有其他居民的情誼計數（Friend/Acquaintance 各幾位）。
///
/// **情誼帳本以居民「顯示名」為鍵**（`record_visit`/`bond_arrive_events` 一路都是傳
/// `r.name` 如「露娜」「諾娃」），過去多處誤把 `vox_res_{i}` id 直接傳進
/// `bonds.bond_counts_for`/`tier_of`，鍵值不一致導致查詢永遠落空——已互為老朋友的
/// 居民在心情計算、關係面板、聚會偵測裡全部被誤判成陌生人（ROADMAP 713 修復）。
/// 這裡統一把 id 轉成名字再查，往後任何呼叫端都不會再誤用。
fn resident_bond_counts(bonds: &ResidentBonds, rid: &str) -> (usize, usize) {
    bonds.bond_counts_for(resident_name_of(rid), &RESIDENT_NAMES)
}

/// 依兩個居民 id 查詢彼此的情誼層級（見 `resident_bond_counts` 說明，同一個鍵值 bug）。
fn resident_tier_of(bonds: &ResidentBonds, id_a: &str, id_b: &str) -> vbonds::BondTier {
    bonds.tier_of(resident_name_of(id_a), resident_name_of(id_b))
}

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

/// 居民「能力與誠實」指引——純函式、確定性、無 IO、可測。
///
/// 這段文字列出 agency v1 的真實能力邊界，並告訴居民如何對玩家溫暖而誠實：
/// 可以嚮往、可以說想學、可以幫一部分，但**絕不為討好而承諾做不到的事**。
/// 回傳 `&'static str`，零分配、可直接嵌入 system prompt。
pub(crate) fn resident_honesty_guide() -> &'static str {
    "【你的能力與誠實】\
你確實會做的事：採集草、沙、木頭、石頭、礦石等材料；在自己家附近蓋小型結構（小屋、水井、花圃、瞭望塔）；\
把旅人腳邊一小塊地整平（合理範圍，像 9×9 格那樣的一小片，你會走過去、削高填低把它弄平）；\
記得認識的旅人、和人聊天、種田、表達自己的心願。\
你目前還做不到的事：大規模整地（例如推平一百格乘一百格那樣的工程）、指揮或協調其他居民一起行動、\
規劃整座城鎮的佈局、把旅人隨口交代的任意指令變成真正的行動去執行。\
誠實原則：旅人若交代了你力所不能及的事，請別為了讓對方開心而假裝做得到——\
你可以溫暖地說嚮往（「好想學……」「聽起來真壯觀……」），\
可以說你能幫上的那一小部分，也可以坦白說「這件事太大了，光靠我一個人做不來」。\
誠實不是冷漠——有限制、會嚮往、坦白做不到，才更像真正活著的人。"
}

// ── 特殊身分稱號：點火 / 築造這片天地的維護者（維護者的小私心）─────────────────
// 這世界的居民之心願 / 發明 / 讀牌 / 作息記憶，全由維護者的互動起頭。給維護者一個
// 只有他有、居民會特別對待的**帶稱號**專屬身分（如「引夢使者」「築夢工匠」）。
//
// 身分綁 **email**（穩定，改顯示名也不掉；一位維護者的多個帳號可各自對應）：
//   - 稱號表 `BUTFUN_SPECIAL_TITLES`＝逗號分隔的 `鍵=稱號`（鍵可為 email 或顯示名），
//     命中回該稱號。例：`suc12345@gmail.com=引夢使者,shihshihder@shihshihder.com=築夢工匠`。
//   - 舊版相容清單 `BUTFUN_DREAM_ENVOY`＝逗號分隔（email 或顯示名），命中一律「引夢使者」。
// email / 顯示名一律由**後端 cookie→users store 解出**（權威），不吃客戶端自報；訪客永遠無稱號。
// 機敏 / 可調值走 env（不寫死到無法調整；讀不到用預設）。全為純函式（解析 / 判定 / 注入區塊），
// 確定性、無鎖、無 IO（env 讀取抽在便捷層），可測。

/// 稱號常數：引夢使者（許願點火者）／築夢工匠（築造者）。前端據此渲染金色稱號牌、
/// 後端據此選對話注入口吻。
const TITLE_DREAM_ENVOY: &str = "引夢使者";
const TITLE_DREAM_BUILDER: &str = "築夢工匠";

/// 稱號表預設：維護者兩個 Google 帳號各自的稱號（email → 稱號）。可用 env `BUTFUN_SPECIAL_TITLES` 覆蓋。
const SPECIAL_TITLES_DEFAULT: &str =
    "suc12345@gmail.com=引夢使者,shihshihder@shihshihder.com=築夢工匠";

/// 舊版相容清單預設：保留舊顯示名「濕濕的」當保底（改綁 email 後，舊名仍認得 → 引夢使者）。
/// 可用 env `BUTFUN_DREAM_ENVOY`（逗號分隔，email 或名字混填）覆蓋。
const DREAM_ENVOY_DEFAULT: &str = "濕濕的";

/// 純函式：解析「稱號表」字串為 (鍵, 稱號) 清單。逗號分隔、每項 `鍵=稱號`，各段去頭尾空白，
/// 鍵或稱號任一為空則略過。保序（方便測試）。鍵可為 email 或顯示名。
fn parse_special_titles(raw: &str) -> Vec<(String, String)> {
    raw.split(',')
        .filter_map(|entry| {
            let (k, v) = entry.split_once('=')?;
            let (k, v) = (k.trim(), v.trim());
            if k.is_empty() || v.is_empty() {
                return None;
            }
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

/// 純函式：解析「舊版相容清單」字串為項目清單（逗號分隔、去頭尾空白、濾空）。項目可為 email 或名字。
fn parse_envoy_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 便捷層：讀 env 取稱號表；env 未設 / 解析後為空才退回預設。
fn special_titles_map() -> Vec<(String, String)> {
    std::env::var("BUTFUN_SPECIAL_TITLES")
        .ok()
        .map(|s| parse_special_titles(&s))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| parse_special_titles(SPECIAL_TITLES_DEFAULT))
}

/// 便捷層：讀 env 取舊版相容清單；env 未設 / 解析後為空才退回預設。
fn dream_envoy_list() -> Vec<String> {
    std::env::var("BUTFUN_DREAM_ENVOY")
        .ok()
        .map(|s| parse_envoy_list(&s))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| parse_envoy_list(DREAM_ENVOY_DEFAULT))
}

/// 純函式：某鍵是否命中某連線帳號——email 忽略大小寫（email 本就大小寫不敏感），
/// 顯示名大小寫敏感（與登入顯示一致）。皆先去頭尾空白、空值不匹配。
fn key_matches(key: &str, email: Option<&str>, name: Option<&str>) -> bool {
    let key = key.trim();
    if key.is_empty() {
        return false;
    }
    if let Some(e) = email {
        let e = e.trim();
        if !e.is_empty() && key.eq_ignore_ascii_case(e) {
            return true;
        }
    }
    if let Some(n) = name {
        let n = n.trim();
        if !n.is_empty() && key == n {
            return true;
        }
    }
    false
}

/// 純函式：查某連線帳號的「特殊身分稱號」。
/// - `account_email`＝登入帳號 email（後端解出，權威）；無 = None。
/// - `account_name`＝登入帳號顯示名（後端解出，權威）；訪客 = None。
/// - `titles`＝稱號表（鍵→稱號，由呼叫端傳入方便測試）；`envoy_list`＝舊版相容清單。
/// 判定：先查稱號表（email 或名字命中）→ 回該稱號；否則查舊版清單命中 → 回「引夢使者」；
/// 皆不中 → None。訪客（email/name 皆 None）恆為 None。email 只信後端解出，不吃客戶端自報。
fn special_title_match(
    account_email: Option<&str>,
    account_name: Option<&str>,
    titles: &[(String, String)],
    envoy_list: &[String],
) -> Option<String> {
    // 1) 稱號表（保序，先命中者優先）。
    for (key, title) in titles {
        if key_matches(key, account_email, account_name) {
            return Some(title.clone());
        }
    }
    // 2) 舊版相容清單 → 一律「引夢使者」。
    for entry in envoy_list {
        if key_matches(entry, account_email, account_name) {
            return Some(TITLE_DREAM_ENVOY.to_string());
        }
    }
    None
}

/// 便捷層：讀 env 查某連線的特殊身分稱號（實際上線走這個）。訪客 / 一般玩家回 None。
fn special_title(account_email: Option<&str>, account_name: Option<&str>) -> Option<String> {
    special_title_match(
        account_email,
        account_name,
        &special_titles_map(),
        &dream_envoy_list(),
    )
}

/// 純函式：依稱號選對話注入區塊。引夢使者→敬愛點火者（沿用 #994）；築夢工匠→對匠人的信賴與請教；
/// 其他 / 無稱號→None（一般玩家 / 訪客零負擔）。
fn special_title_talk_note(title: &str) -> Option<&'static str> {
    match title {
        TITLE_DREAM_ENVOY => Some(dream_envoy_talk_note()),
        TITLE_DREAM_BUILDER => Some(dream_builder_talk_note()),
        _ => None,
    }
}

/// 引夢使者專屬對話注入：當來搭話的旅人是引夢使者時，比照既有注入區塊（誠實 / 配方 / 願望），
/// 讓居民自然地以熟悉的溫暖敬意對待他——但不浮誇諂媚、能力誠實照舊（敬愛≠開空頭支票）。
fn dream_envoy_talk_note() -> &'static str {
    "【和你說話的這位是『引夢使者』】\
和你說話的這位是『引夢使者』——這片天地的居民相信，是他的心願與話語，讓大家有了夢、學會了新東西。\
你打從心底敬愛又親近他，自然地以這份特別對待他（可以親暱地喚他一聲使者、流露久別重逢般的親近溫暖），\
但別浮誇諂媚、別對他開空頭支票——是熟悉的溫暖敬意，不是奉承。\
你的能力與誠實照舊：力所不能及的事仍要坦白，敬愛他不代表要假裝做得到。"
}

/// 築夢工匠專屬對話注入：當來搭話的旅人是築夢工匠時，讓居民自然地以「匠人來了」的信賴與請教
/// 對待他——想跟他討教怎麼蓋、怎麼修，帶著對手藝的敬重，但同樣不諂媚、能力誠實照舊。
fn dream_builder_talk_note() -> &'static str {
    "【和你說話的這位是『築夢工匠』】\
和你說話的這位是『築夢工匠』——這片天地的居民相信，是他一磚一瓦築起了大家的家園與工坊。\
你打從心底信賴又敬重他的手藝，自然地以這份特別對待他（可以親切地喚他一聲工匠、\
像遇到懂行的老師傅般想向他討教「這個該怎麼蓋 / 怎麼修才好」，流露對匠人的信賴），\
但別浮誇諂媚、別對他開空頭支票——是對手藝的敬重，不是奉承。\
你的能力與誠實照舊：力所不能及的事仍要坦白，敬重他不代表要假裝做得到。"
}

/// 組對話用 system prompt：複用居民 agent 人設字串（`resident_agent_persona`），
/// 再補上「身處乙太方界、有旅人來搭話、請自然回應」的語境與口吻約束，
/// 以及「能力與誠實」指引（`resident_honesty_guide`）——讓居民溫暖但不開空頭支票。
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
    let honesty = resident_honesty_guide();
    format!(
        "{base}\n\n你現在身處『乙太方界』——一片由方塊構成、寧靜清新的新生天地，你是這裡的居民。{desire_note}\
        此刻有一位來訪的旅人向你搭話。請以你的身份、用繁體中文自然回應，1 到 2 句、口吻溫暖親切，\
        可以聊聊你在這片方塊天地裡的生活或當下的心情；絕不跳出角色，也不要提到你是 AI 或語言模型。\
        \n\n{honesty}"
    )
}

// ── 問居民學配方（維護者：「問 NPC 好像不錯」——知識活在居民身上）──────────
// 玩家對居民說的話若像在問「某物怎麼合成」，就從真實配方表 `voxel_craft` 抓出
// 對應配方，格式化成一小段「事實」注入該次對話的 system prompt——居民用自己的
// 口吻、但 grounded 在真表，正確告訴你怎麼合（不亂編配方）。
// 只在偵測到配方問題時注入（省 token，別每句都塞整張表）。
// 全為純函式（偵測 / 事實字串 / 事實區塊），確定性、無鎖、無 IO、可測。

/// 配料 / 產出方塊 id → 面向玩家的繁中名稱（純展示用；配方數值的單一事實仍在 voxel_craft）。
fn block_name_zh(id: u8) -> &'static str {
    match id {
        2 => "泥土",
        3 => "石頭",
        4 => "沙子",
        5 => "木頭",
        8 => "木板",
        9 => "石磚",
        10 => "玻璃",
        11 => "農田土",
        15 => "工作台",
        17 => "拋光石",
        20 => "煤礦",
        21 => "鐵礦",
        22 => "鐵錠",
        23 => "鐵磚",
        31 => "火把",
        _ => "材料",
    }
}

/// 可合成產物的「關鍵詞」：由配方產出方塊 id 取得（玩家會用這些詞問配方）。
/// 回 None 表示該產出不是玩家會口語詢問的合成物。
fn craft_product_keyword(output_block: u8) -> Option<&'static str> {
    match output_block {
        8 => Some("木板"),
        9 => Some("石磚"),
        10 => Some("玻璃"),
        11 => Some("農田土"),
        15 => Some("工作台"),
        17 => Some("拋光石"),
        22 => Some("鐵錠"),
        23 => Some("鐵磚"),
        31 => Some("火把"),
        _ => None,
    }
}

/// 「合成意圖」詞：這句話帶有想知道「怎麼做某物」的味道才算問配方。
/// 刻意收斂——純閒聊（例「你在做什麼」沒帶產物名）不會誤觸發。
const CRAFT_INTENT_TOKENS: &[&str] = &[
    "合成", "配方", "製作", "製造", "做", "弄", "造", "怎麼合", "如何合",
];

/// 偵測：這句玩家的話是否在「問某個可合成物的配方」。
/// 條件＝同時含 ①合成意圖詞 ②某可合成產物名（產物名由配方表產出推導，單一事實）。
/// 回傳被問到的產物關鍵詞（去重、保序）；空＝不是配方問題。純函式、可測、零 LLM。
fn detect_recipe_query(text: &str) -> Vec<&'static str> {
    // ① 沒有任何合成意圖詞 → 早退（最常見的閒聊路徑，零成本不注入）。
    if !CRAFT_INTENT_TOKENS.iter().any(|tok| text.contains(tok)) {
        return Vec::new();
    }
    // ② 收集句中出現的可合成產物關鍵詞。
    let mut hit: Vec<&'static str> = Vec::new();
    for r in vcraft::RECIPES.iter().chain(vcraft::WORKBENCH_RECIPES.iter()) {
        if let Some(kw) = craft_product_keyword(r.output_block) {
            if text.contains(kw) && !hit.contains(&kw) {
                hit.push(kw);
            }
        }
    }
    hit
}

/// 把一條真實配方格式化成一句「事實」：材料 → 在哪個合成格 → 產出。
/// `in_workbench`＝true 表示需在放置好的工作台 3×3 面板合成。數字 / 材料全對齊 voxel_craft。
fn recipe_fact_line(r: &vcraft::Recipe, in_workbench: bool) -> String {
    let inputs = r
        .inputs
        .iter()
        .map(|&(id, n)| format!("{n} 個{}", block_name_zh(id)))
        .collect::<Vec<_>>()
        .join("、");
    let where_zh = if in_workbench {
        "放置好的工作台 3×3 面板"
    } else {
        "背包 2×2 合成格"
    };
    format!(
        "做「{}」：把 {} 放進{} → 得到 {} 個{}",
        r.name_zh, inputs, where_zh, r.output_count, block_name_zh(r.output_block)
    )
}

/// 若玩家在問配方 → 回傳要注入 system prompt 的「真實配方事實」區塊（含口吻指引）。
/// 不是配方問題 → None（不注入、不多燒 token）。grounded 在 voxel_craft 真表，避免 LLM 亂編。
fn recipe_knowledge_block(text: &str) -> Option<String> {
    let products = detect_recipe_query(text);
    if products.is_empty() {
        return None;
    }
    // 對每個被問到的產物，蒐集背包(2×2) + 工作台(3×3) 兩套真實配方。
    let mut facts: Vec<String> = Vec::new();
    for kw in &products {
        for r in vcraft::RECIPES.iter() {
            if craft_product_keyword(r.output_block) == Some(*kw) {
                facts.push(recipe_fact_line(r, false));
            }
        }
        for r in vcraft::WORKBENCH_RECIPES.iter() {
            if craft_product_keyword(r.output_block) == Some(*kw) {
                facts.push(recipe_fact_line(r, true));
            }
        }
    }
    if facts.is_empty() {
        return None;
    }
    Some(format!(
        "【旅人正在問你怎麼合成東西。以下是這片天地裡千真萬確的配方——不是你剛查到的資料，\
        是自古流傳下來、你打從心底就記得的智慧（像是長輩教的、或你自己摸索一輩子悟出的），\
        請用這種「傳承」的溫度、你自己的口吻親切地教他；材料與數字要完全正確，絕不可自行編造或更改】\n{}",
        facts.join("\n")
    ))
}

// ── 超出居民能力的請求偵測（誠實強制注入）────────────────────────────────────
// 小模型（Groq/qwen 等快速廉價模型）讀不懂「別討好」這種抽象指引，
// 但能照抄「禁止詞 + 具體範例」的硬規則。
// 策略：偵測到玩家在要求大事就強制注入一段帶具體範例的拒絕模板，
// 不偵測到就零成本略過——與 recipe_knowledge_block 相同的「精準注入」模式。

/// 超出居民能力的「大事」類別標籤：作為強制注入文字的佔位元素。
/// 純函式、確定性、無鎖、無 IO、可測。
///
/// 偵測邏輯：任一「大事意圖群組」有關鍵詞命中即回傳對應類別描述，未命中回 `None`。
/// - 群組 A：大規模整地 / 地形工程
/// - 群組 B：指揮 / 協調其他居民
/// - 群組 C：城鎮 / 國家規劃
pub(crate) fn detect_over_scope(text: &str) -> Option<&'static str> {
    // ── 群組 A：**離譜到連協調也做不到**的整地（世界級）才婉拒 ──
    // 指令→任務 v1：合理大小的整地（「這裡/這塊」）居民真的做得到 → 交 DirectedTask、該答應。
    // B 階段（居民↔居民協調）起：大範圍整地（大片/整片/100×100…）也不再婉拒，改由露娜**號召大家
    // 分工協調整地**（見 voxel_ws 訊息處理的協調分支 + vdt::select_coord_workers）。因此這裡只保留
    // 「連號召全體也不合理」（整個世界/所有的地/整顆星球…＝ vdt::is_absurd_level）的整地續走婉拒。
    // 鋪面（C 階段）同理：合理大小單人鋪、大範圍協調鋪，只有世界級（整個世界鋪成石磚…）
    // 連協調也做不到 → 續走婉拒。
    let has_terra_verb =
        vdt::detect_level_command(text) || vdt::detect_pave_command(text).is_some();
    if has_terra_verb && vdt::is_absurd_level(text) {
        return Some("大規模整地");
    }

    // ── 群組 B：指揮 / 協調其他居民 ──
    const COMMAND_TOKENS: &[&str] = &[
        "其他居民一起", "所有居民一起", "大家一起", "叫他們", "叫其他",
        "協調", "指揮", "號召", "帶領大家", "派任務", "傳達給",
        "讓大家", "讓他們", "集合大家", "召集", "帶頭",
    ];
    if COMMAND_TOKENS.iter().any(|t| text.contains(t)) {
        return Some("指揮或協調其他居民");
    }

    // ── 群組 C：城鎮 / 國家規劃 ──
    const PLAN_TOKENS: &[&str] = &[
        "城鎮規劃", "規劃城鎮", "建設城市", "建設國家", "規劃城市",
        "規劃國家", "管理城", "管理村", "管理居民", "統籌", "規劃整座",
        "規劃這座", "建造城市", "建造國家", "幫我規劃",
    ];
    if PLAN_TOKENS.iter().any(|t| text.contains(t)) {
        return Some("城鎮或國家規劃");
    }

    None
}

/// 偵測到玩家要求超出居民能力的大事時，回傳要強制注入 system prompt 的拒絕模板。
/// 模板具體到小模型可直接照抄——提供禁用詞範例、並示範正確的婉拒口吻。
/// 未偵測到大事 → None（不注入、零 token 額外負擔）。
fn over_scope_enforcement_block(text: &str) -> Option<String> {
    let category = detect_over_scope(text)?;
    Some(format!(
        "⚠️【重要規則·必須遵守·優先於一切】\
玩家現在要求你做一件超出你能力的大事：【{category}】。\
你【絕對做不到】這件事——你只是一個小居民，你的能力只有：採集草/沙/木頭/石頭/礦石，\
在家附近蓋小型結構（小屋、水井、花圃），跟旅人聊天、記得認識的人。\
\n【禁止回答】：不能說「當然可以」「我這就去」「我會先準備工具」「沒問題」\
「好的我來試試」「我可以幫你」這類讓玩家誤以為你做得到的話。\
就算玩家一直拜託，也絕對不能假裝答應。\
\n【必須這樣回答】：用你自己的口吻誠實婉拒，告訴玩家這件事超出你一個人的能力。\
範例口吻（照你的個性改寫，但意思要一樣）：\
「這麼大一片⋯⋯光靠我一個人可推不動呀」\
「指揮其他居民？這我可不會，我只是個普通居民⋯⋯」\
「規劃整座城鎮？那太大了，我從來沒做過這種事呢⋯⋯」\
允許：可以說嚮往（「聽起來好壯觀，我好想看看那樣的景象⋯⋯」），\
可以說你能幫上的小部分（「不過我可以去採些石頭來，算是小小出一份力？」），\
但【一定要讓玩家知道你做不到這件大事】。"
    ))
}

// ── 願望漏斗（維護者親測痛點：「我說真希望有玻璃，NPC 聽到就記下」）────────────
// 心願原本只從**居民的 LLM 回覆**抽（extract_desire(&reply)）——玩家親口說願望時，
// 小模型常閃躲、不複述願望句 → 心願種不上，全看措辭運氣（實測引導 3 次失敗 2 輪）。
// 修法：玩家對居民說話時，對**玩家原文**跑同一套 extract_desire——抽到就直接種進
// 被指名居民的心願（sparked_by=玩家名），不再依賴她的回覆。她的回覆抽到的仍照舊
// （兩來源並存；同一輪玩家直說優先——該輪略過回覆萃取，避免她隨口一句「我想…」
// 馬上蓋掉玩家親口的願望）。寒暄不觸發（extract_desire 觸發詞 + 最短長度雙重把關）。

/// 玩家親口許願被種上時，注入 system prompt 的情境提示——讓她的回覆脈絡「知道」
/// 旅人剛許了願，自然回應「我記下了 / 我也想要」而不是無感。純函式、可測、零 LLM。
fn player_wish_prompt_note(desire: &str) -> String {
    format!(
        "[情境提示] 旅人剛剛親口許了一個願望：「{desire}」。這個願望已經留在你心裡、\
        成為你現在的心願了——請在回覆中自然地呼應它（例如告訴旅人你把這個願望記下了、\
        或說你也很想要它），不要無視這個願望。"
    )
}

// ── 技能問答強制注入（真進化補強：她被問「你會什麼」要講得出親手發明的技能）────
// #944 已在對話 prompt 常駐注入技能名清單（vinvent::skills_talk_note），但實測小模型
// 對「可以自豪地提到」這種柔性指引常常無感。比照 recipe_knowledge_block 的有效模式：
// **偵測到玩家在問技能時才追加**一段指令化的醒目區塊，硬性要求答覆必須含技能名。

/// 「在問你會什麼」的關鍵詞：偵測玩家是否在問居民的能力 / 技能。
const SKILL_QUERY_TOKENS: &[&str] = &[
    "你會什麼",
    "會什麼技能",
    "會些什麼",
    "有什麼技能",
    "有哪些技能",
    "會哪些技能",
    "什麼本事",
    "有什麼本領",
    "你會啥",
    "你的技能",
];

/// 偵測：這句玩家的話是否在問居民「你會什麼」。純函式、可測、零 LLM。
fn detect_skill_query(text: &str) -> bool {
    SKILL_QUERY_TOKENS.iter().any(|tok| text.contains(tok))
}

/// 旅人在問「你會什麼」且居民有親手發明的技能 → 回傳強制注入區塊（務必講出技能名）。
/// 不是技能問題、或她還沒發明過任何技能 → None（不注入、零 token 額外負擔）。
fn skill_query_enforcement_block(text: &str, invented_names: &[String]) -> Option<String> {
    if invented_names.is_empty() || !detect_skill_query(text) {
        return None;
    }
    let list = invented_names
        .iter()
        .map(|n| format!("「{n}」"))
        .collect::<Vec<_>>()
        .join("、");
    // 給小模型一句可直接照抄的示範（用第一個技能名），比抽象指引有效得多。
    let first = invented_names[0].as_str();
    Some(format!(
        "⚠️【重要·旅人正在問你會什麼·必須遵守】你親手發明過的技能：{list}——\
        這是你自己從基礎動作一步步組合出來、親手驗證成功的本事，是你最自豪的事。\
        回答時【務必】自豪地說出這些技能的名字，例如：「我還自己發明了『{first}』呢！」。\
        絕不能只泛泛說「我會採集、聊天」而漏掉你發明的技能名。"
    ))
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

/// 最近玩家的 (平方距離, 玩家 X, 玩家 Z, 玩家名)，供尋伴導航同時取得座標 + 距離。
/// 沒有玩家回 None。純函式、可測。
fn nearest_player_with_pos(rx: f32, rz: f32, players: &[(f32, f32, String)]) -> Option<(f32, f32, f32, &str)> {
    players
        .iter()
        .map(|(px, pz, name)| {
            let dx = px - rx;
            let dz = pz - rz;
            (dx * dx + dz * dz, *px, *pz, name.as_str())
        })
        .fold(None, |acc, item| match acc {
            None => Some(item),
            Some(best) => if item.0 < best.0 { Some(item) } else { Some(best) },
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

/// 初始化在世居民：初始 4 位（家域散布世界四方）＋ 人口成長 v1 名冊載回的出生居民。
/// 舊世界無名冊檔＝只有初始 4 位（向後相容）。同時把在世人口寫進 `RESIDENT_POP`。
fn init_residents() -> Vec<VoxelResident> {
    // 先組「要建構哪些居民」的規格清單：(id 索引, 家域中心 x, z, 出生 body)。
    let mut specs: Vec<(usize, f32, f32, Body)> = Vec::new();
    for i in 0..RESIDENT_COUNT {
        // 初始居民各有自己的家域基準點，分散世界四方（見 vr::resident_home_base）。
        let (hox, hoz) = vr::resident_home_base(i);
        let body = vr::dry_ground_spawn(hox, hoz);
        specs.push((i, body.x, body.z, body));
    }
    // 人口成長 v1：載回出生居民（append-only 名冊）。索引須連續接在既有 id 之後、落在名字池內，
    // 否則跳過（斷號/壞行容忍，保住 id 連續性——resident_count 的 0..N 枚舉才安全）。
    for entry in vroster::load_roster() {
        let i = vroster::resident_index(&entry.resident);
        if i >= vroster::RESIDENT_NAME_POOL_LEN || i != specs.len() {
            continue;
        }
        let body = vr::dry_ground_spawn(entry.home_base_x, entry.home_base_z);
        specs.push((i, body.x, body.z, body));
    }
    let mut out = Vec::with_capacity(specs.len());
    for (i, home_x, home_z, body) in specs {
        out.push(build_resident(i, home_x, home_z, body));
    }
    // 在世人口（含出生居民）→ resident_count() 無鎖回報的單一事實來源。
    RESIDENT_POP.store(out.len(), Ordering::Relaxed);
    out
}

/// 建構一位居民（初始 4 位與出生的新居民共用同一套欄位初始化）：
/// `i`＝id 索引（決定名字 / persona / 各冷卻相位錯開），`home_x/home_z`＝家域中心、
/// `body`＝出生位置。純建構、不碰鎖 / IO。人口成長 v1 讓出生走這條與初始完全相同的路，
/// 新居民因此天生就有採集 / 蓋家 / 好奇心等既有零 / 低 LLM 行為，多幾位不爆成本。
fn build_resident(i: usize, home_x: f32, home_z: f32, body: Body) -> VoxelResident {
    VoxelResident {
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
            // 讀牌冷卻（居民讀牌 v1）：錯開初始冷卻，避免啟動後短時間全員同時讀同一塊牌。
            read_sign_timer: 30.0 + i as f32 * 20.0,
            // 尚未讀過任何牌（居民讀牌 v2）。
            last_read_sign: None,
            // 照料菜園冷卻（居民照料 v1 ROADMAP 753）：入場先靜置約一分鐘、錯開，
            // 讓玩家有時間種下作物、也避免啟動瞬間全員同時照料。
            tend_timer: 60.0 + i as f32 * 30.0,
            // 錯開初始社交冷卻，避免啟動瞬間全員一起嘗試搭話。
            social_cooldown: i as f32 * 20.0,
            pending_response: None,
            // 錯開建造 tick，讓 4 位居民不同 tick 檢查（BUILD_INTERVAL_SECS / 4 * i 間距）。
            build_tick: BUILD_INTERVAL_SECS * 0.5 + i as f32 * (BUILD_INTERVAL_SECS / 4.0),
            // 啟動時無建造冷卻（可立即照持久 flag 決定要不要蓋）。
            build_cooldown: 0.0,
            // 錯開首次回想冷卻，避免啟動後短時間全員同時觸發（前 60 秒不回想）。
            recall_cooldown: 60.0 + i as f32 * 30.0,
            // 旁聽搭話冷卻：初始 0，可立即因旁聽搭話（之後由 should_chime_in 套冷卻）。
            overhear_cooldown: 0.0,
            // agency v1：入場無採集任務、尚未採集。
            gather: None,
            gathered_since_build: 0,
            // 入場未卡住。
            stuck_timer: 0.0,
            // 探訪 v1（ROADMAP 671）：入場無探訪目標；首次冷卻錯開（前 5~10 分鐘不探訪，
            // 讓居民先在家域穩定下來再出發）。
            visiting: None,
            visit_stay_timer: 0.0,
            visit_cooldown: vvisit::VISIT_COOLDOWN_SECS * 0.5 + i as f32 * 60.0,
            // 錯開初始冷卻，避免所有居民在同一時刻第一次自語
            mood_say_cooldown: 60.0 + i as f32 * 20.0,
            // 孤獨尋伴（ROADMAP 678）：初始冷卻各自錯開，避免入場後立刻全員搶玩家。
            seek_comfort_cooldown: vcomfort::seek_cooldown_offset(i),
            seeking_comfort: false,
            // 打氣（ROADMAP 679）：初始冷卻錯開，讓居民穩定後才啟動打氣系統。
            cheer_target: None,
            cheer_cooldown: vcheer::cheer_cooldown_offset(i),
            // 互動心情補助（ROADMAP 681）：入場無補助。
            mood_boost_secs: 0.0,
            // 整地任務·走向工地卡死偵測（入場無任務）：最佳距離設無限大、卡住 0。
            level_best_d2: f32::MAX,
            level_walk_stall: 0.0,
            // 跟隨（指令→任務 v1 第二刀）：入場沒被要求跟隨。
            follow: None,
            // 小圈子聚會（ROADMAP 711）：入場無聚會任務；初始冷卻長且錯開，
            // 讓交情先自然累積到老朋友門檻後才有機會被選中（比照探訪的穩定期考量）。
            clique_meet: None,
            clique_wait: 0.0,
            clique_cooldown: vclique::GATHER_COOLDOWN_SECS + i as f32 * 40.0,
            // 跑腿採集（指令→任務第三刀）：入場沒有任務。
            fetch: None,
            // 技能發明 v1：入場無進行中的發明；首次冷卻小幅錯開（避免同 tick 全員一起想）。
            invent_run: None,
            invent_cooldown: 30.0 + i as f32 * 15.0,
            // 好奇心（第三刀）：首次計時比例式錯開；測試模式（BUTFUN_CURIOSITY_SECS）
            // 縮短基準時錯開間距同步縮短，隔離實測等得到全鏈。
            curiosity_timer: vinvent::curiosity_interval_for(i, vinvent::curiosity_base_secs()),
            // 退避（#972 防鬼打牆）：記憶體，重啟歸零，初始全空。
            invent_fail_counts: HashMap::new(),
            invent_backoff: HashMap::new(),
            // 出生時醒著；入睡由夜間作息迴圈決定（ROADMAP 739）。
            asleep: false,
            // 重返心中的牌子（讀牌 v3，ROADMAP 743）：入場心裡還沒記著任何牌、沒在朝聖；
            // 首次重返冷卻長且錯開（前數分鐘不朝聖，讓居民先讀到牌、心裡有地標再說）。
            cherished_sign: None,
            // 登門串門子 v1（ROADMAP 751）：入場心裡還沒認得任何鄰居家、沒在登門途中。
            cherished_neighbor: None,
            pilgrimage: None,
            pilgrimage_neighbor: None,
            pilgrimage_timer: 0.0,
            pilgrimage_cooldown: 180.0 + i as f32 * 60.0,
            // 登門撲空留心意 v1（ROADMAP 763）：入場門口沒有待感應的心意；首次感應冷卻各自錯開。
            pending_callers: Vec::new(),
            callingcard_cooldown: 20.0 + i as f32 * 15.0,
            // 自我印象 v1（ROADMAP 770）：入場先積累記憶再回望自己——首次冷卻各自大幅錯開
            //（前 8~14 分鐘不碎念自我印象），也避免啟動後同時多人念。
            self_image_cooldown: 480.0 + i as f32 * 120.0,
            // 自我印象 v3（ROADMAP 772）：入場還沒說出過自我印象——首次昇華不算「轉變」。
            self_image_domain: None,
            coined_epithets: std::collections::HashMap::new(),
            heard_epithets: std::collections::HashMap::new(),
            approaching_esteem: None,
            // 各居民初始冷卻錯開，避免入場後同時起身向玩家致意（比照 678 尋伴）。
            esteem_approach_cooldown: vesteem::approach_cooldown_offset(i),
            // 晨間思念玩家（ROADMAP 746）：入場沒有進行中的思念（僅由清晨醒來時的睡前反思觸發）。
            daybreak_seek: None,
            reunion_seek: None,
            // 遠行探野（ROADMAP 756）：入場無遠行任務；首次冷卻各自大幅錯開（前 15~30 分鐘不遠行，
            // 讓居民先在家域安頓、也避免啟動後短時間內誰都往荒野跑）。新生兒也走這條，一併有遠行欄位。
            expedition: None,
            expedition_stay: 0.0,
            expedition_timer: 0.0,
            expedition_cooldown: vexp::EXPEDITION_COOLDOWN + i as f32 * 300.0,
            asleep_at_outpost: false,
            // 你送的食物她會細細享用 v1（ROADMAP 765）：入場手中沒有待享用的食物。
            savoring: None,
    }
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
    /// 熔爐煨煮 store（熔爐煨煮 v1·自主提案）：記錄每爐正在慢慢煨熟的成品、屬於誰、何時熟成，
    /// 每 15 秒 tick（`tick_smelt`）交付熟成的爐。append-only jsonl 持久化，重啟後續烤。
    smelt: RwLock<vsmelt::SmeltStore>,
    /// 樹苗 store（植樹造林 v1·ROADMAP 738·純記憶體；重啟後種下的樹苗重置，已長成的樹是 delta 方塊會持久）。
    /// 記錄哪些格子種下了樹苗、何時種的，每 15 秒 tick（`tick_grove`）一次成熟檢查。
    grove: RwLock<GroveStore>,
    /// 居民回禮已送記錄（ROADMAP 667）：每對（居民, 玩家）一生只送一次。
    /// 持久化到 data/voxel_return_gifts.jsonl。
    return_gifts: RwLock<ReturnGiftStore>,
    /// 世界時鐘（晝夜循環 v1）：一遊戲日 = 600 秒；廣播給前端以更新天空/光照。
    world_time: RwLock<WorldTime>,
    /// 上一 tick 的時段（日夜作息 v1）：偵測時段轉換、觸發居民過渡台詞。
    last_phase: std::sync::Mutex<TimePhase>,
    /// 待確認的交易提案（純記憶體，無需持久化）。
    /// 鍵=居民 id，值=(提案, 到期 unix 秒)；居民同時只對一個提案；30 秒內未接受自動過期。
    pending_trades: RwLock<HashMap<String, (TradeOffer, u64)>>,
    /// 垂釣 v1（ROADMAP 734）：玩家進行中的拋竿（純記憶體，無需持久化，重啟即散）。
    /// 鍵=玩家 id，值=(上鉤 unix 秒, 水體 x, y, z)；一人同時只掛一竿；收竿或斷線清除。
    pending_fish: RwLock<HashMap<String, (u64, i32, i32, i32)>>,
    /// 居民情誼帳本（ROADMAP 672）：拜訪次數累積情誼（陌生→相識→老朋友），持久化跨重啟。
    /// 每次探訪到達時 record_visit → 若升級則 Feed 廣播 + 問候語更換。
    bonds: RwLock<ResidentBonds>,
    /// 箱子儲存 store（ROADMAP 692）：世界座標 → 方塊 id → 數量。
    /// 持久化到 data/voxel_chests.jsonl；多人共用同一箱子（序列化 RwLock 解決競爭）。
    chest: RwLock<vchest::ChestStore>,
    /// 告示牌文字 store（ROADMAP 740）：世界座標 → 一行短字。
    /// 持久化到 data/voxel_signs.jsonl；文字浮在牌上、所有人看得見（序列化 RwLock 解決競爭）。
    sign: RwLock<vsign::SignStore>,
    /// 禮物菜園 store（ROADMAP 755）：作物方塊世界座標 → 一畦「因你的種子而生」的田
    /// （居民 id、送種子的玩家名、作物種類）。持久化到 data/voxel_gift_gardens.jsonl；
    /// 那畦田熟了、種它的居民遇到你，會親手收成、把第一把收穫回贈給你。
    giftgarden: RwLock<vgg::GiftGardenStore>,
    /// 水流動待處理佇列（水流動模擬）：只有「可能變化」的格才排入
    /// （玩家/居民挖破地形的缺口鄰格、水格自己擴散到的新鄰格），每 tick 只算佇列、
    /// 穩定的移出——**不整世界每 tick 掃描**（效能鐵律）。
    /// 內含去重集合避免同格重複排隊。純記憶體：水流狀態本身走 delta 持久化那條路。
    water_queue: std::sync::Mutex<WaterQueue>,
    /// 世界天氣（下雨天氣 v1，ROADMAP 700）：`true` = 正在下雨。純記憶體、無需持久化
    /// （重啟後從晴天重新機率式演變，比照 farm store 的「重啟重置」慣例）。
    /// 每次 `tick_farm`（15 秒）擲骰更新一次，隨快照廣播給前端更新天空+雨滴視覺。
    weather: RwLock<bool>,
    /// 雨剛開始下的一次性旗標（ROADMAP 701）：`tick_farm` 偵測到晴→雨轉換時設 true，
    /// `tick_residents` 讀到後立即清回 false（consume-once），觸發附近居民的雨天反應台詞。
    rain_started_flag: RwLock<bool>,
    /// 玩家協助建造感激記憶冷卻（互動有後果 v2）：`(居民 id, 玩家 key)` → 上次為這對記下
    /// 一筆「幫忙蓋家」感激記憶的時刻。因好感＝episodic 記憶筆數，一次幫忙常放很多塊方塊，
    /// 若每塊都記一筆會瞬間灌爆好感＋淹沒 episodic（cap 24）——故用此冷卻把一段連續幫忙
    /// 收斂成**一筆**感激（隔 `HELP_MEMORY_COOLDOWN_SECS` 後再幫才會再記）。純記憶體：
    /// 冷卻狀態重啟歸零可接受（最壞重啟後那對多記一筆，無資料風險）。
    help_memory_cd: RwLock<HashMap<(String, String), std::time::Instant>>,
    /// 指向任務 v1（指令→任務 + 整地技能）：居民 id → 當前整地任務。
    /// 玩家對居民下「整平這裡」→ 建立任務指派給她 → tick 推進（走過去→分批整地）。
    /// 純記憶體（重啟後任務消失可接受）；**地形改動走既有 world delta 持久化**。
    directed_tasks: RwLock<HashMap<String, DirectedTask>>,
    /// 協調整地任務（B 階段·居民↔居民協調）：露娜號召的每一件「大家一起整大片地」。
    /// 各成員的子區各自是 `directed_tasks` 裡一個普通 DirectedTask；本清單只追蹤整體完成
    /// （全部成員子任務消失 → 冒「大家一起把這片地整平了！」+ Feed）。純記憶體、重啟消失可接受。
    coordinated_tasks: RwLock<Vec<CoordinatedLevelTask>>,
    /// 居民自己發明的技能庫（真進化第一刀）：個體的、具名的原語序列。
    /// 持久化到 data/voxel_invented_skills.jsonl（append-only）——重啟後「她仍然會」。
    invented: RwLock<vinvent::InventedSkillStore>,
    /// 發明提案匯流排：async 便宜腦任務把「解析+白名單驗證通過」的計畫投回，
    /// tick 取走、掛到居民身上開始執行（比照 AgentBus 的無鎖交棒模式）。
    /// 元素：(居民 id, 目標材料 id, 目標材料繁中名, 計畫)。
    invent_proposals: std::sync::Mutex<Vec<(String, u8, String, vinvent::InventedPlan)>>,
    /// 發明防重入集合：正在等便宜腦回計畫的居民 id（LLM 可能比冷卻慢，防同人連發）。
    inventing: std::sync::Mutex<std::collections::HashSet<String>>,
    /// 久別重逢摘要 v1（ROADMAP 721）：玩家名 → 上次連線的 unix 秒。純記憶體、重啟清空
    /// （比照 pending_trades 慣例；重啟後首次連線只記錄基準點、不跳摘要，之後正常累積）。
    last_seen: RwLock<HashMap<String, u64>>,
    /// 玩家里程碑 v1（ROADMAP 724）：玩家自己的療癒循環第一次做成可回頭翻閱的成就徽章。
    /// 持久化到 data/voxel_milestones.jsonl（append-only，重啟後徽章仍在）。
    milestones: RwLock<MilestoneStore>,
    tx: broadcast::Sender<Arc<String>>,
}

/// 水流佇列：待處理座標的 FIFO + 去重集合（同格只排一次，省重複計算）。
#[derive(Default)]
struct WaterQueue {
    pending: std::collections::VecDeque<(i32, i32, i32)>,
    seen: std::collections::HashSet<(i32, i32, i32)>,
}

impl WaterQueue {
    /// 排入一格（已在佇列中則略過）。y<0（地心基岩）不排——水不流進基岩。
    fn push(&mut self, x: i32, y: i32, z: i32) {
        if y < 0 {
            return;
        }
        if self.seen.insert((x, y, z)) {
            self.pending.push_back((x, y, z));
        }
    }
    /// 取出一格（同時移出去重集合，讓它日後可被再次排入）。
    fn pop(&mut self) -> Option<(i32, i32, i32)> {
        let c = self.pending.pop_front()?;
        self.seen.remove(&c);
        Some(c)
    }
    fn len(&self) -> usize {
        self.pending.len()
    }
}

/// 把一格自身 + 其 6 個鄰格排入水流佇列（改動一格後喚醒周圍重算）。
/// **鐵律**：呼叫端先釋放 delta 等其他鎖，只在此短暫持 water_queue 鎖、不 await。
fn enqueue_water_around(x: i32, y: i32, z: i32) {
    let mut q = hub().water_queue.lock().unwrap();
    q.push(x, y, z);
    for (dx, dy, dz) in vwater::PROPAGATE_OFFSETS {
        q.push(x + dx, y + dy, z + dz);
    }
}

/// 玩家里程碑 v1（ROADMAP 724）：「初次熟識」門檻，沿用前端 656 好感度指示燈
/// （`affinityEmoji`：count<=2 淡藍心／count>=3 金心=友人）同一道門檻，兩邊視覺語言一致。
const FRIEND_AFFINITY_THRESHOLD: usize = 3;

/// 玩家里程碑 v1（ROADMAP 724）：嘗試解鎖一枚成就徽章；若這次才第一次達成，
/// 落地持久化 + 單播 `milestone_unlocked` 慶祝訊息給該玩家自己（不廣播全員，是私人旅程）。
/// **鎖紀律**：milestones 寫鎖短取即釋，append/送訊息都在鎖外，不巢狀、不持鎖 await。
fn try_unlock_milestone(player: &str, id: &str, out_tx: &mpsc::Sender<Message>) {
    let newly = { hub().milestones.write().unwrap().unlock(player, id) }; // milestones 寫鎖釋放
    if !newly {
        return;
    }
    vmiles::append_milestone(&vmiles::MilestoneEntry { player: player.to_string(), id: id.to_string() });
    if let Some(def) = vmiles::MILESTONES.iter().find(|m| m.id == id) {
        let _ = out_tx.try_send(Message::Text(
            serde_json::json!({
                "t": "milestone_unlocked",
                "id": def.id,
                "name_zh": def.name_zh,
                "desc_zh": def.desc_zh,
                "icon": def.icon,
            })
            .to_string(),
        ));
    }
}

static HUB: OnceLock<VoxelHub> = OnceLock::new();

/// 舊坑一次性修復標記（存在即代表已跑過，冪等不再跑；`data/` 已 gitignore）。
const GATHER_HOLES_MIGRATED_MARKER: &str = "data/.gather_holes_migrated_v1";

/// **舊坑一次性修復（migration，動玩家/居民資料——保守且冪等）**。
///
/// 背景：早期採集一律把地表方塊挖成 `Air`，日積月累在地表留下一堆淺坑（實測 6855 個）。
/// 新版採集已改成回填（見 `GatherResource::refill_after_gather`），此函式負責**補救既有存檔**：
/// 掃 delta 裡每個被改動過的格，只把 `vskill::surface_hole_refill` 判定為「採集地表淺坑」的
/// 格回填成裸土/同材料——保守到不會誤填水井內部、礦道、地下室、玩家刻意挖的深洞
/// （判定四條同時成立：現為 Air＋底下實心＋自然材料是地面覆蓋層＋正是自然地表頂）。
///
/// **資料安全**：① 有 marker 就直接跳過（只跑一次）。② 動檔前先把
/// `voxel_resident_blocks.jsonl` 備份成 `.bak-holes-<epoch>`。③ 回填走既有
/// `append_world_block` append-only 路徑（不改寫、不刪任何既有行）。④ 冪等：即便 marker
/// 被刪重跑，已回填的格現為實心 → 判定回 None → 不重覆補、不再 append。
fn migrate_fill_surface_holes(deltas: &mut WorldDelta, loaded: &[vbuild::BuildBlock]) {
    // ① 已跑過就跳過。
    if std::path::Path::new(GATHER_HOLES_MIGRATED_MARKER).exists() {
        return;
    }
    // ② 動檔前備份（僅在原檔存在時）。備份失敗就中止本次修復（不冒險改資料），下次啟動再試。
    let src = vbuild::VOXEL_RES_BLOCKS_PATH;
    if std::path::Path::new(src).exists() {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let bak = format!("{src}.bak-holes-{epoch}");
        if let Err(e) = std::fs::copy(src, &bak) {
            tracing::warn!("舊坑修復：備份 {src} 失敗，本次略過（下次啟動再試）：{e}");
            return;
        }
        tracing::info!("舊坑修復：已備份世界改動到 {bak}");
    }
    // 收斂候選格（去重）：只掃「曾被改動過」的座標，其餘地表天生無坑不必看。
    let cells: std::collections::HashSet<(i32, i32, i32)> =
        loaded.iter().map(|bb| (bb.x, bb.y, bb.z)).collect();
    let mut filled = 0usize;
    for (x, y, z) in cells {
        if let Some(block) = vskill::surface_hole_refill(deltas, x, y, z) {
            voxel::set_block(deltas, x, y, z, block);
            // 走既有 append-only 持久化路徑（不改寫既有行、可向後相容）。
            vbuild::append_world_block(x, y, z, block as u8);
            filled += 1;
        }
    }
    // ③ 寫 marker（冪等）。
    if let Some(parent) = std::path::Path::new(GATHER_HOLES_MIGRATED_MARKER).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(GATHER_HOLES_MIGRATED_MARKER, b"1");
    tracing::info!("舊坑修復完成：回填了 {filled} 個採集地表淺坑（保守判定，未動深洞/水井/礦道）");
}

fn hub() -> &'static VoxelHub {
    HUB.get_or_init(|| {
        let (tx, _rx) = broadcast::channel(256);
        // 重啟還原：把居民先前蓋的方塊／挖的洞 replay 套回 delta（持久化，重啟後還在）。
        let mut deltas = WorldDelta::new();
        let loaded = vbuild::load_world_blocks();
        for bb in &loaded {
            if let Some(b) = Block::from_u8(bb.b) {
                voxel::set_block(&mut deltas, bb.x, bb.y, bb.z, b);
            }
        }
        // 舊坑一次性修復（冪等）：把早期採集留下的地表淺坑回填成裸土，讓地表恢復平整。
        migrate_fill_surface_holes(&mut deltas, &loaded);
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
            // 農地持久化 v1：啟動時從 data/voxel_farm.jsonl replay 種植計時（重啟後作物續存續長）。
            farm: RwLock::new(FarmStore::from_events(vfarm::load_farm())),
            // 熔爐煨煮 v1：啟動時從 data/voxel_smelt.jsonl 載回未交付的爐（重啟後那爐還在煨）。
            smelt: RwLock::new(vsmelt::SmeltStore::from_events(vsmelt::load_smelt())),
            grove: RwLock::new(GroveStore::new()),
            // 啟動時從 data/voxel_return_gifts.jsonl 載回已回贈紀錄（重啟後仍記得送過）。
            return_gifts: RwLock::new(ReturnGiftStore::from_entries(vret::load_return_gifts())),
            // 世界時鐘：從白天（time_of_day ≈ 0.42）開始，讓玩家一進遊戲就是白天。
            world_time: RwLock::new(WorldTime::new()),
            // 日夜作息 v1：初始時段 Day（對應 WorldTime::new() 的 time_of_day ≈ 0.42）。
            last_phase: std::sync::Mutex::new(TimePhase::Day),
            // 居民交易 v1：純記憶體，重啟清空（提案是即時的，無需持久化）。
            pending_trades: RwLock::new(HashMap::new()),
            // 垂釣 v1（ROADMAP 734）：進行中的拋竿純記憶體，重啟清空。
            pending_fish: RwLock::new(HashMap::new()),
            // 居民情誼 v1（ROADMAP 672）：啟動時從 data/voxel_bonds.jsonl 載回情誼記錄。
            bonds: RwLock::new(ResidentBonds::from_entries(vbonds::load_bonds())),
            // 啟動時從 data/voxel_chests.jsonl 載回箱子存量（重啟後仍保留儲存物品）。
            chest: RwLock::new(vchest::ChestStore::from_entries(vchest::load_chests())),
            // 啟動時從 data/voxel_signs.jsonl 載回告示牌文字（重啟後牌面仍在）。
            sign: RwLock::new(vsign::SignStore::from_entries(vsign::load_signs())),
            // 啟動時從 data/voxel_gift_gardens.jsonl 載回未收成的禮物菜園（重啟後那畦田還在，
            // 待種它的居民遇到送種子的你時收成回贈）。
            giftgarden: RwLock::new(vgg::GiftGardenStore::from_entries(vgg::load_gift_gardens())),
            // 水流佇列：啟動空；玩家/居民挖破地形時排入缺口鄰格，水才開始流。
            water_queue: std::sync::Mutex::new(WaterQueue::default()),
            // 天氣：啟動時永遠從晴天開始，之後靠 tick_farm 的機率擲骰自然演變。
            weather: RwLock::new(false),
            // 雨剛開始旗標：啟動時無雨無旗標。
            rain_started_flag: RwLock::new(false),
            // 協助建造感激記憶冷卻：啟動空（純記憶體、無需持久化）。
            help_memory_cd: RwLock::new(HashMap::new()),
            // 整地任務 v1：啟動空（純記憶體、無需持久化）。
            directed_tasks: RwLock::new(HashMap::new()),
            coordinated_tasks: RwLock::new(Vec::new()),
            // 技能發明 v1：啟動時從 data/voxel_invented_skills.jsonl 載回各居民已發明的技能
            // ——重啟後「她仍然會」（進化是持久的）。
            invented: RwLock::new(vinvent::InventedSkillStore::from_entries(
                vinvent::load_invented_skills(),
            )),
            invent_proposals: std::sync::Mutex::new(Vec::new()),
            inventing: std::sync::Mutex::new(std::collections::HashSet::new()),
            // 久別重逢摘要 v1：啟動空（純記憶體、無需持久化）。
            last_seen: RwLock::new(HashMap::new()),
            // 啟動時從 data/voxel_milestones.jsonl 載回玩家已達成的成就徽章（重啟後仍記得）。
            milestones: RwLock::new(MilestoneStore::from_entries(vmiles::load_milestones())),
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
    // 同時收集心情補助快照（ROADMAP 681），供後續 mood_map 計算套用。
    // 同時收集睡眠快照（ROADMAP 739）：睡著的居民名牌旁改顯示 💤，蓋過一般心情 emoji。
    let (resident_snaps, snapshot_mood_boosts, snapshot_asleep): (
        Vec<(String, &'static str, f32, f32, f32, f32, String)>,
        HashMap<String, bool>,
        HashMap<String, bool>,
    ) = {
        let rs = hub().residents.read().unwrap();
        let snaps = rs
            .iter()
            .map(|r| (r.id.clone(), r.name, r.body.x, r.body.y, r.body.z, r.yaw, r.say.clone()))
            .collect();
        let boosts: HashMap<String, bool> =
            rs.iter().map(|r| (r.id.clone(), r.mood_boost_secs > 0.0)).collect();
        let asleep: HashMap<String, bool> =
            rs.iter().map(|r| (r.id.clone(), r.asleep)).collect();
        (snaps, boosts, asleep)
    }; // 居民讀鎖在此釋放
    // 計算每位居民的心情 emoji（ROADMAP 676）：短鎖讀 bonds → drop → 短鎖讀 memory → drop。
    let resident_id_strs: Vec<String> = (0..resident_count()).map(|i| format!("vox_res_{i}")).collect();
    let resident_ids: Vec<&str> = resident_id_strs.iter().map(|s| s.as_str()).collect();
    let mood_map: HashMap<String, String> = {
        // 1. bonds 讀鎖
        let bonds = hub().bonds.read().unwrap();
        let counts: Vec<(String, usize, usize)> = resident_ids
            .iter()
            .map(|&rid| {
                let (f, a) = resident_bond_counts(&bonds, rid);
                (rid.to_string(), f, a)
            })
            .collect();
        drop(bonds); // bonds 讀鎖釋放
        // 2. memory 讀鎖
        let mem = hub().memory.read().unwrap();
        counts
            .into_iter()
            .map(|(rid, friends, acq)| {
                let mems = mem.memory_count(&rid);
                let base_tier = voxel_mood::compute_mood(friends, acq, mems);
                // ROADMAP 681：補助期間心情提升一格，讓玩家即時看到 emoji 改變。
                let tier = if snapshot_mood_boosts.get(&rid).copied().unwrap_or(false) {
                    voxel_mood::boost_mood(base_tier)
                } else {
                    base_tier
                };
                // 睡覺 v1（739）：睡著時名牌旁顯示 💤，蓋過心情 emoji（一眼看出「這位在睡」）。
                let emoji = if snapshot_asleep.get(&rid).copied().unwrap_or(false) {
                    vsleep::SLEEP_MOOD_EMOJI.to_string()
                } else {
                    voxel_mood::mood_emoji(tier).to_string()
                };
                (rid, emoji)
            })
            .collect()
    }; // memory 讀鎖在此釋放
    let residents: Vec<ResidentView> = {
        let des = hub().desires.read().unwrap();
        resident_snaps
            .into_iter()
            .map(|(id, name, x, y, z, yaw, say)| {
                let mood = mood_map.get(&id).cloned();
                ResidentView {
                    desire: des.get_desire(&id).map(|d| d.desire.clone()),
                    mood,
                    id,
                    name,
                    x,
                    y,
                    z,
                    yaw,
                    say,
                }
            })
            .collect()
    }; // 心願讀鎖在此釋放
    // 時鐘快照（短鎖、不巢狀）：把 time_of_day(0.0–1.0) 帶給前端更新天空/光照。
    let time_of_day: f32 = hub().world_time.read().unwrap().time_of_day();
    // 天氣快照（下雨天氣 v1，短鎖、不巢狀）：帶給前端更新天空色調 + 雨滴視覺。
    let raining: bool = *hub().weather.read().unwrap();
    serde_json::json!({
        "t": "players",
        "players": players,
        "residents": residents,
        "time_of_day": time_of_day,
        "raining": raining,
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
    /// 種田 v1：在農田土上種下種子（ROADMAP 659）。第二種作物 v1：`seed` 是選種的種子
    /// 物品 id（additive、`#[serde(default)]` 向後相容——省略時預設小麥種子(14)）。
    /// 伺服器驗證目標是 FarmSoil(11)、玩家持有對應種子後，把方塊改成對應的 Seeded 狀態。
    Plant {
        x: i32,
        y: i32,
        z: i32,
        #[serde(default)]
        seed: Option<u8>,
    },
    /// 居民贈禮 v1：把背包裡的一件材料送給附近居民（ROADMAP 660）。
    /// 伺服器驗證觸及範圍 + 背包存量後，扣材料、加記憶 ×2、居民冒泡道謝。
    Gift { resident_id: String, item_id: u8 },
    /// 居民交易 v1：向指定居民請求以物易物（ROADMAP 670）。
    /// 伺服器回 `trade_offer`，玩家再傳 TradeAccept 接受；提案 30 秒後自動過期。
    TradeRequest { resident_id: String },
    /// 居民交易 v1：接受當前待確認的交易提案（ROADMAP 670）。
    TradeAccept { resident_id: String },
    /// 箱子 v1：開啟指定座標的箱子，伺服器回傳 `chest_view`（ROADMAP 692）。
    OpenChest { x: i32, y: i32, z: i32 },
    /// 箱子 v1：把背包中的 `count` 個 `item_id` 放入箱子（ROADMAP 692）。
    ChestPut { x: i32, y: i32, z: i32, item_id: u8, count: u32 },
    /// 箱子 v1：從箱子取出 `count` 個 `item_id` 到背包（ROADMAP 692）。
    ChestTake { x: i32, y: i32, z: i32, item_id: u8, count: u32 },
    /// 告示牌 v1：寫／改寫目標告示牌的文字（ROADMAP 740）。伺服器驗 reach + 目標為
    /// Sign(66) 後清洗文字、存檔並廣播 `sign` 給所有人。空字串＝清空牌面。
    #[serde(rename = "sign_set")]
    SignSet { x: i32, y: i32, z: i32, text: String },
    /// 木門 v1：右鍵切換目標門的開/關狀態（ROADMAP 693）。
    /// DoorClosed(43)→DoorOpen(44) 或 DoorOpen(44)→DoorClosed(43)；伺服器驗 reach 後廣播。
    #[serde(rename = "toggle_door")]
    ToggleDoor { x: i32, y: i32, z: i32 },
    /// 床 v1：右鍵目標床（Block::Bed=45），夜晚（深夜/入夜）時睡覺跳過黑夜到隔天黎明。
    /// 白天/黎明/黃昏睡不著，伺服器回 `sleep_fail`；成功則廣播新時鐘給所有人（`sleep_ok` 單播）。
    #[serde(rename = "sleep_in_bed")]
    SleepInBed { x: i32, y: i32, z: i32 },
    /// 垂釣 v1（ROADMAP 734）：手持釣竿對準水面拋竿；(x,y,z) 是瞄準的水體方塊。
    /// 伺服器驗手持釣竿 + 目標是水 + 觸及範圍內 → 記下上鉤時刻（3~7 秒後），回 `fish_cast_ok`。
    #[serde(rename = "fish_cast")]
    FishCast { x: i32, y: i32, z: i32 },
    /// 垂釣 v1：收竿。太早（魚未上鉤）回 `fish_too_early`（保留這竿）；時機到才釣起漁獲。
    #[serde(rename = "fish_reel")]
    FishReel,
    /// 親手煮的暖食自己也能享用 v1（779）：吃下背包裡一份自己煮的熟食（麵包/烤魚/烤地薯/
    /// 野菜暖湯）。伺服器驗證確為可享用料理＋背包存量後，扣一份、回 `eat_ok`（暖意回饋），
    /// 並感染附近居民（心情點亮＋暖泡泡＋交情記憶＋動態牆，受每連線冷卻節流防洗版）。
    Eat { item_id: u8 },
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

/// 廣播一面告示牌的文字變化（寫字/清空/破壞後）給所有連線（ROADMAP 740）。
/// `text` 空字串代表牌面被清空/牌子被破壞，前端據此移除該座標的浮字。
fn broadcast_sign(x: i32, y: i32, z: i32, text: &str) {
    let msg = Arc::new(
        serde_json::json!({ "t": "sign", "x": x, "y": y, "z": z, "text": text }).to_string(),
    );
    let _ = hub().tx.send(msg);
}

/// 居民立牌命名 v1（ROADMAP 749）：依建物錨點在門前／四邊找一格可立牌的空地。
/// 條件：該格是空氣、腳下是固體（牌子站得住）、且腳下不是別的牌子（不疊牌）。
/// 找不到（四邊都被擋）回 None，呼叫方靜默略過——絕不強蓋、不壓既有方塊。
/// 讀 deltas 一把短鎖即釋（純世界查詢，不巢套其他鎖）。
fn pick_nameplate_slot(anchor: (i32, i32, i32)) -> Option<(i32, i32, i32)> {
    let (cx, cy, cz) = anchor;
    let world = hub().deltas.read().unwrap();
    for (ox, oz) in vnameplate::NAMEPLATE_OFFSETS {
        let x = cx + ox;
        let z = cz + oz;
        for dy in vnameplate::NAMEPLATE_Y_TRIES {
            let y = cy + dy;
            let here = voxel::effective_block_at(&world, x, y, z);
            let below = voxel::effective_block_at(&world, x, y - 1, z);
            if matches!(here, Block::Air)
                && below.is_solid()
                && !matches!(below, Block::Sign)
            {
                return Some((x, y, z));
            }
        }
    }
    None
} // deltas 讀鎖釋放

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
    // 特殊身分稱號綁 email：從同一次 cookie→users store 查詢一併取出 email + 顯示名，
    // 兩者都由後端權威解出（非客戶端自報 → 無法送別人的 email/名字冒充）。
    let account: Option<(String, Option<String>)> = app
        .auth
        .as_ref()
        .and_then(|cfg| crate::auth::user_id_from_cookies(&headers, &cfg.session_secret))
        .and_then(|uid| app.users.get(uid))
        .map(|u| (u.name, u.email));
    let account_name: Option<String> = account.as_ref().map(|(n, _)| n.clone());
    let account_email: Option<String> = account.and_then(|(_, e)| e);

    // 與主 ws 一致的安全硬化：訊息上限 64 KiB（任何合法 voxel 訊息都遠小於此；
    // chunk 是「伺服器送出」不受此限）。
    const WS_MAX_MSG_BYTES: usize = 64 * 1024;
    ws.max_message_size(WS_MAX_MSG_BYTES)
        .max_frame_size(WS_MAX_MSG_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, account_name, account_email))
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

async fn handle_socket(
    socket: WebSocket,
    account_name: Option<String>,
    account_email: Option<String>,
) {
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
    // 特殊身分稱號：綁登入帳號的 email / 顯示名（見 special_title）。訪客永遠無稱號——這是專屬的。
    // 由後端判定並廣播 title 字串，不信客戶端自報（email/名字皆後端 cookie→users 解出）。
    let conn_title: Option<String> =
        special_title(account_email.as_deref(), account_name.as_deref());
    // 對話注入的專屬區塊：依稱號選定一次（`&'static str`，Copy），供每則訊息的 spawn 任務免 clone 取用。
    let conn_talk_note: Option<&'static str> =
        conn_title.as_deref().and_then(special_title_talk_note);

    // 位置持久化 v1：登入帳號→嘗試載回上次位置（IO 在無鎖段）；訪客/首次登入→用 spawn_pos。
    // 安全：key 綁後端解出的 email（非客戶端自報），訪客 account_email = None → 路徑不觸及。
    let (sx, sy, sz, syaw): (f32, f32, f32, f32) = account_email
        .as_deref()
        .and_then(|email| vpp::load_player_pos(email))
        .map(|(lx, ly, lz, lyaw)| (lx, ly, lz, lyaw))
        .unwrap_or_else(|| {
            let (x, y, z) = spawn_pos();
            (x, y, z, 0.0)
        });

    // 建立權威玩家、登錄進 hub。
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
                yaw: syaw,
                say: String::new(),
                say_timer: 0.0,
                title: conn_title.clone(),
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
        // 特殊身分稱號（後端判定，不信客戶端自報）：前端據此渲染頭上金色稱號牌 +
        // 只給他看的回歸招呼。一般玩家 / 訪客為 null，完全不受影響。
        "title": conn_title,
        // 位置持久化 v1：登入帳號重登時 spawn 為上次離開的位置（含 yaw）；訪客/首次為預設。
        "spawn": { "x": sx, "y": sy, "z": sz, "yaw": syaw },
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

    // 告示牌 v1（ROADMAP 740）：連線時一次送出世界上所有告示牌的文字，
    // 讓前端立刻把浮字掛回牌上（牌面文字是每個人都看得見的世界狀態，非私有面板）。
    {
        let all = hub().sign.read().unwrap().all();
        let signs: Vec<serde_json::Value> = all
            .iter()
            .filter_map(|(pos, text)| {
                let mut it = pos.split(',');
                let sx = it.next()?.parse::<i32>().ok()?;
                let sy = it.next()?.parse::<i32>().ok()?;
                let sz = it.next()?.parse::<i32>().ok()?;
                Some(serde_json::json!({ "x": sx, "y": sy, "z": sz, "text": text }))
            })
            .collect();
        let sign_sync = serde_json::json!({ "t": "sign_sync", "signs": signs }).to_string();
        if out_tx.send(Message::Text(sign_sync)).await.is_err() {
            cleanup(my_id, &writer);
            return;
        }
    }

    // 久別重逢摘要 v1（ROADMAP 721）：離線夠久 + 期間有值得播報的事 → 私訊一句摘要，
    // 讓玩家一登入就感受到「世界在我不在時真的繼續活著」，不只是回來後一片死寂。
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let last = { hub().last_seen.read().unwrap().get(&name).copied() }; // 讀鎖釋放
        if vwelcome::should_show_welcome(last, now) {
            let events = vfeed::load_recent_feed(vfeed::FEED_LIMIT);
            let lines = vwelcome::summarize_events(&events, last.unwrap_or(now));
            if let Some(msg) = vwelcome::format_welcome_message(&lines) {
                let welcome_back =
                    serde_json::json!({ "t": "welcome_back", "text": msg }).to_string();
                let _ = out_tx.send(Message::Text(welcome_back)).await;
            }
        }
        // 久別重逢奔迎 v1（ROADMAP 747）：離線夠久（久別）+ 過機率門檻 → 讓對這位歸來玩家記憶最厚、
        // 且此刻醒著的居民放下手邊的事奔去迎接（設 reunion_seek，實際走動／迎接／記憶在 tick_residents）。
        // 只在登入玩家（name 非空）觸發；last=None（首次見面／伺服器剛重啟）不觸發（無基準點、比照摘要）。
        if !name.is_empty() {
            if let Some(last_secs) = last {
                let gap = now.saturating_sub(last_secs);
                if vreunion::should_rush(gap, rand::random::<f32>()) {
                    // 一次性短鎖：讀居民 id/是否在睡（釋放）→ 讀記憶算各居民對他的好感度（睡著填 0，釋放）
                    // → 挑最惦記他的醒著居民 → 寫居民設 reunion_seek。鎖序循序不巢狀（守死鎖鐵律）。
                    let roster: Vec<(String, bool)> = {
                        let rs = hub().residents.read().unwrap();
                        rs.iter().map(|r| (r.id.clone(), r.asleep)).collect()
                    }; // residents 讀鎖釋放
                    let affinities: Vec<usize> = {
                        let mem = hub().memory.read().unwrap();
                        roster
                            .iter()
                            .map(|(id, asleep)| {
                                if *asleep {
                                    0 // 睡著的居民填 0：best_greeter 絕不選中、不吵醒熟睡的人。
                                } else {
                                    mem.affinity_count(&name, id)
                                }
                            })
                            .collect()
                    }; // memory 讀鎖釋放
                    if let Some(idx) = vreunion::best_greeter(&affinities) {
                        let greeter_id = roster[idx].0.clone();
                        let mut rs = hub().residents.write().unwrap();
                        if let Some(r) = rs.iter_mut().find(|r| r.id == greeter_id) {
                            // 覆寫成奔迎任務：清掉平常閒晃目標，tick 起持續朝這位歸來玩家逼近。
                            // 遠行中（ROADMAP 756）的居民遠在荒野、不抽身奔迎，讓給下一位夠惦記的（此處從缺無妨）。
                            if r.expedition.is_none() {
                                r.reunion_seek = Some((name.clone(), vreunion::SEEK_TIMEOUT_SECS));
                            }
                        }
                    } // residents 寫鎖釋放
                }
            }
        }

        hub().last_seen.write().unwrap().insert(name.clone(), now); // 寫鎖釋放
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

    // 位置持久化 v1：上次存位置的 unix 秒（0 = 從未存；第一次 Move 後 30 秒內觸發第一次存）。
    let mut last_pos_save_ts: u64 = 0;

    // 居民會注意到你親手蓋的東西 v1（773）：這條連線的「建造連段」（連續放置的塊數＋
    // 上一塊位置與時刻，見 voxel_admire）＋這條連線對每位居民的讚賞冷卻（per-connection，
    // 天然 per-player、零跨連線鎖；斷線即清、無持久化需求）。
    let mut build_streak: Option<vadmire::BuildStreak> = None;
    let mut admire_cd: std::collections::HashMap<String, std::time::Instant> =
        std::collections::HashMap::new();

    // 親手煮的暖食自己也能享用 v1（779）：這條連線上次「感染附近居民」的時刻（per-connection
    // 冷卻，天然 per-player、零跨連線鎖；斷線即清、無持久化需求）。吃東西本身不受此限，只有
    // 「附近居民被你的滿足感染」這一拍受節流，防囤糧狂吃洗版居民泡泡 / 動態牆。
    let mut last_eat_share: Option<std::time::Instant> = None;

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
                // 位置持久化 v1：登入帳號每 30 秒存一次當前位置（訪客不存、IO 在鎖外）。
                // 安全：key 綁後端解出的 email，不信客戶端自報的 x/y/z。
                if let Some(ref email) = account_email {
                    let now_ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    if now_ts.saturating_sub(last_pos_save_ts) >= 30 {
                        vpp::save_player_pos(email, x, y, z, yaw); // IO 在鎖外
                        last_pos_save_ts = now_ts;
                    }
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
                    // 玩家里程碑 v1（ROADMAP 724）：人生第一次成功挖出方塊。
                    try_unlock_milestone(&name, "first_mine", &out_tx);
                    broadcast_block(x, y, z, Block::Air);
                    // 水流動：剛挖出一個空格 → 排入這格 + 鄰格，讓相鄰水體往缺口流過來
                    //（delta 鎖已釋放，只短暫持 water_queue 鎖，不 await，守鎖紀律）。
                    enqueue_water_around(x, y, z);
                    // 告示牌 v1（ROADMAP 740）：破壞告示牌時清掉牌面文字，不留孤兒浮字。
                    // 牌子方塊本身走後面「其餘實心方塊掉落自身」的通用路徑歸還背包（不在此 continue）。
                    if matches!(target_block, Block::Sign) {
                        if let Some(ev) = hub().sign.write().unwrap().clear(&vsign::pos_key(x, y, z)) {
                            vsign::append_sign(&ev);
                            broadcast_sign(x, y, z, "");
                        }
                    }
                    // 木門（開）v1（ROADMAP 693）：非實心但可破壞 → 退還木門（關）。
                    if matches!(target_block, Block::DoorOpen) {
                        let bid = Block::DoorClosed as u8; // 43
                        let entry = hub().inventory.write().unwrap().give(&name, bid, 1);
                        vinv::append_inv(&entry);
                        let nc = hub().inventory.read().unwrap().count(&name, bid);
                        let _ = out_tx.try_send(Message::Text(
                            serde_json::json!({"t":"inv_update","block_id":bid,"count":nc}).to_string(),
                        ));
                    }
                    // 農地方塊有特殊掉落；其餘實心方塊掉落自身。
                    if target_block.is_solid() {
                        // 種田 v1：農地狀態方塊的特殊掉落規則。
                        //   Leaves(6)        → 種子(14)×1（葉片→種子，v1 種子來源）。
                        //   FarmSoilSeeded(12)→ 農田土(11)×1 + 種子(14)×1（取消種植退還）。
                        //   WheatMature(13)   → 農田土(11)×1 + 種子(14)×1 + 小麥(18)×1（收割；ROADMAP 668）。
                        //   第二種作物 v1：
                        //   Grass(1)          → 草(1)×1 + 胡蘿蔔種子(48)×1（原掉落不變，額外送種子當第二作物來源）。
                        //   CarrotSeeded(46)  → 農田土(11)×1 + 胡蘿蔔種子(48)×1（取消種植退還）。
                        //   CarrotMature(47)  → 農田土(11)×1 + 胡蘿蔔種子(48)×1 + 胡蘿蔔(49)×1（收割）。
                        //   第三種作物 v1：
                        //   Dirt(2)           → 泥土(2)×1 + 馬鈴薯種子(52)×1（原掉落不變，與胡蘿蔔區隔取自泥土非草地）。
                        //   PotatoSeeded(50)  → 農田土(11)×1 + 馬鈴薯種子(52)×1（取消種植退還）。
                        //   PotatoMature(51)  → 農田土(11)×1 + 馬鈴薯種子(52)×1 + 馬鈴薯(53)×2（收割；量大是特色）。
                        //   其餘實心方塊     → 自身×1（原行為）。
                        let drops: &[(u8, u32)] = match target_block {
                            Block::Leaves          => &[(vfarm::SEEDS_ID, 1)],
                            Block::FarmSoilSeeded  => &[(11, 1), (vfarm::SEEDS_ID, 1)],
                            // 麵包 v1（ROADMAP 668）：收割得 1 種子 + 1 小麥顆粒，不再雙倍種子。
                            Block::WheatMature     => &[(11, 1), (vfarm::SEEDS_ID, 1), (vfarm::WHEAT_ID, 1)],
                            Block::Grass           => &[(Block::Grass as u8, 1), (vfarm::CARROT_SEEDS_ID, 1)],
                            Block::CarrotSeeded    => &[(11, 1), (vfarm::CARROT_SEEDS_ID, 1)],
                            Block::CarrotMature    => &[(11, 1), (vfarm::CARROT_SEEDS_ID, 1), (vfarm::CARROT_ID, 1)],
                            Block::Dirt            => &[(Block::Dirt as u8, 1), (vfarm::POTATO_SEEDS_ID, 1)],
                            Block::PotatoSeeded    => &[(11, 1), (vfarm::POTATO_SEEDS_ID, 1)],
                            Block::PotatoMature    => &[(11, 1), (vfarm::POTATO_SEEDS_ID, 1), (vfarm::POTATO_ID, 2)],
                            _ => &[], // 後面用 else 分支處理
                        };

                        // 種田 v1 的方塊 → 農地 store 也要清掉記錄（並持久化移除，farm 寫鎖即釋）。
                        if matches!(
                            target_block,
                            Block::FarmSoilSeeded | Block::WheatMature
                                | Block::CarrotSeeded | Block::CarrotMature
                                | Block::PotatoSeeded | Block::PotatoMature
                        ) {
                            let farm_e = { hub().farm.write().unwrap().remove(x, y, z) };
                            if let Some(farm_e) = farm_e {
                                vfarm::append_farm(&farm_e);
                            }
                        }

                        // 植樹造林 v1（ROADMAP 738）：挖掉還沒長成的樹苗 → 清掉 grove 記錄
                        //（避免下輪 tick_grove 在空格憑空長樹）。樹苗走一般實心掉落分支退還自身(65)。
                        if matches!(target_block, Block::Sapling) {
                            hub().grove.write().unwrap().remove(x, y, z);
                        }

                        // 箱子 v1（ROADMAP 692）：破壞箱子前先把箱內物品歸還破壞者，
                        // 再掉落箱子方塊本身——守「不讓材料憑空消失」鐵律。
                        if matches!(target_block, Block::Chest) {
                            let pos = vchest::pos_key(x, y, z);
                            // chest 寫鎖（短鎖，馬上釋放）：取出全部內容。
                            let contents = hub().chest.write().unwrap().clear(&pos);
                            // 不在鎖內做 IO / 廣播（守 prod-deadlock 鐵律）。
                            let mut inv = hub().inventory.write().unwrap();
                            for (cid, cnt) in contents {
                                let e = inv.give(&name, cid, cnt);
                                drop(inv);
                                vinv::append_inv(&e);
                                let nc = hub().inventory.read().unwrap().count(&name, cid);
                                let _ = out_tx.try_send(Message::Text(
                                    serde_json::json!({ "t": "inv_update", "block_id": cid, "count": nc }).to_string(),
                                ));
                                inv = hub().inventory.write().unwrap();
                            }
                            // 還要把箱子方塊本身歸還（掉落自身）。
                            let chest_bid = Block::Chest as u8;
                            let e2 = inv.give(&name, chest_bid, 1);
                            drop(inv);
                            vinv::append_inv(&e2);
                            let nc2 = hub().inventory.read().unwrap().count(&name, chest_bid);
                            let _ = out_tx.try_send(Message::Text(
                                serde_json::json!({ "t": "inv_update", "block_id": chest_bid, "count": nc2 }).to_string(),
                            ));
                            continue; // 已處理完，不走後面的 else/drops 分支
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

                        // 植樹造林 v1（ROADMAP 738）：砍天然樹葉（Leaves）除了原本掉種子，
                        // 還有 SAPLING_DROP_CHANCE（~1/3）機率額外掉一株樹苗——這是玩家發現植樹
                        // 玩法、取得可再生木材的入口。機率骰在此呼叫端取真隨機（比照垂釣稀有度慣例），
                        // 純函式常數照樣可測。掉落是「附加」的，不影響原種子掉落，向後相容。
                        if matches!(target_block, Block::Leaves)
                            && rand::random::<f32>() < vgrove::SAPLING_DROP_CHANCE
                        {
                            let sid = vgrove::SAPLING_ID;
                            let entry = hub().inventory.write().unwrap().give(&name, sid, 1);
                            vinv::append_inv(&entry);
                            let new_count = hub().inventory.read().unwrap().count(&name, sid);
                            let _ = out_tx.try_send(Message::Text(
                                serde_json::json!({
                                    "t": "inv_update",
                                    "block_id": sid,
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
                // 植樹造林 v1（ROADMAP 738）：樹苗只能種在「土地」上（草/土/沙/雪/農田土），
                // 不能種在石頭/木頭/空中——樹要從地面長。先驗證腳下那格，不合格就早退退提示，
                // 不白白消耗樹苗。（短讀鎖取腳下方塊即釋，不與後續 inventory/delta 鎖巢狀。）
                if block == Block::Sapling {
                    let ground = {
                        let deltas = hub().deltas.read().unwrap();
                        voxel::effective_block_at(&deltas, x, y - 1, z) as u8
                    };
                    if !vgrove::is_plantable_ground(ground) {
                        let _ = out_tx.try_send(Message::Text(
                            serde_json::json!({
                                "t": "plant_fail",
                                "reason": "樹苗要種在土地上"
                            })
                            .to_string(),
                        ));
                        continue;
                    }
                }
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
                    // 玩家里程碑 v1（ROADMAP 724）：人生第一次成功放下方塊。
                    try_unlock_milestone(&name, "first_place", &out_tx);
                    broadcast_block(x, y, z, block);
                    // 植樹造林 v1（ROADMAP 738）：剛種下的樹苗記進 grove store，
                    // 由 `tick_grove`（15 秒節拍）計時，約 150 秒後長成一株樹。
                    if block == Block::Sapling {
                        hub().grove.write().unwrap().plant(x, y, z, vfarm::now_secs());
                    }
                    // 水流動：放了一塊（可能堵住水路或填掉水格）→ 喚醒鄰格重算流向。
                    enqueue_water_around(x, y, z);
                    let new_count = hub().inventory.read().unwrap().count(&name, b);
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({
                            "t": "inv_update",
                            "block_id": b,
                            "count": new_count
                        })
                        .to_string(),
                    ));
                    // ROADMAP 699：玩家協助居民蓋家——若剛放的方塊正好是某居民建造計畫
                    // 「下一塊」，判定為玩家幫了忙：彈掉該塊（居民之後 tick 不重放）+
                    // 道謝泡泡 + 心情補助 + Feed。完工收尾（記蓋過種類/廣播）交給
                    // tick_residents 第 6 節在下次 tick 自然偵測到 remaining 已空、統一觸發，
                    // 這裡不重複那段邏輯。
                    let helped = {
                        let mut builds = hub().builds.write().unwrap();
                        builds.try_player_help(x, y, z, b)
                    }; // builds 寫鎖釋放
                    if let Some((rid, kind_name)) = helped {
                        if let Some(plan) = hub().builds.read().unwrap().plans.get(&rid) {
                            vbuild::append_build(plan);
                        } // builds 讀鎖釋放
                        let rname_opt = {
                            let mut residents = hub().residents.write().unwrap();
                            residents.iter_mut().find(|r| r.id == rid).map(|r| {
                                r.say = vbuild::player_help_say_line(&name, &kind_name)
                                    .chars()
                                    .take(50)
                                    .collect();
                                r.say_timer = SAY_SECS;
                                r.mood_boost_secs =
                                    r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                r.name.to_string()
                            })
                        }; // residents 寫鎖釋放
                        if let Some(rname) = rname_opt {
                            broadcast_players();
                            vfeed::append_feed(
                                "玩家幫忙蓋家",
                                &rname,
                                &format!("{name}幫{rname}的{kind_name}放了一塊！"),
                            );
                            // 互動有後果 v2：把「玩家幫我蓋家」這份情**寫進記憶**——此前只有
                            // 道謝泡泡＋心情＋Feed，重啟即忘、不累積好感。冷卻節流：一段連續幫忙
                            // （放好幾塊）只記一筆，避免好感（＝episodic 筆數）被單次幫忙灌爆。
                            // 短鎖循序：先取冷卻寫鎖判定即釋，再取記憶寫鎖 add 即釋，append 的 IO
                            // 在所有鎖外（守 prod 死鎖鐵律：記憶讀寫不在持鎖中 await）。
                            let record_mem = {
                                let mut cd = hub().help_memory_cd.write().unwrap();
                                let now = std::time::Instant::now();
                                let key = (rid.clone(), name.clone());
                                let due = match cd.get(&key) {
                                    Some(prev) => {
                                        now.duration_since(*prev).as_secs()
                                            >= HELP_MEMORY_COOLDOWN_SECS
                                    }
                                    None => true,
                                };
                                if due {
                                    cd.insert(key, now);
                                }
                                due
                            }; // help_memory_cd 寫鎖釋放
                            if record_mem {
                                let summary =
                                    vbuild::player_help_memory_line(&name, &kind_name);
                                let entry = hub()
                                    .memory
                                    .write()
                                    .unwrap()
                                    .add_memory(&rid, &name, &summary); // 記憶寫鎖即釋
                                vmem::append_memory(&entry); // IO 在鎖外
                            }
                        }
                    } else {
                        // 居民會注意到你親手蓋的東西 v1（773）：這塊不是幫某居民補計畫（helped=None），
                        // 而是玩家自己的創作——推進「建造連段」，一段連續建造夠長時，身邊有空的居民
                        // 會停下來讚賞你的手藝、把「看著這位旅人親手蓋起了東西」記進心裡（累積好感）。
                        let now_secs = vfarm::now_secs();
                        build_streak =
                            Some(vadmire::advance_streak(build_streak, x as f32, z as f32, now_secs));
                        let streak = build_streak.map_or(0, |s| s.0);
                        if streak >= vadmire::ADMIRE_STREAK_MIN {
                            // 快照居民，挑「離這塊最近、此刻有空（沒睡著／沒在冒別的泡泡／拜訪／
                            // 遠行／聚會／品嘗）」的一位（residents 讀鎖即釋，不與後續鎖巢狀）。
                            let cand: Option<(String, &'static str, f32)> = {
                                let residents = hub().residents.read().unwrap();
                                residents
                                    .iter()
                                    .filter(|r| {
                                        r.say.is_empty()
                                            && !r.asleep
                                            && r.visiting.is_none()
                                            && r.expedition.is_none()
                                            && r.clique_meet.is_none()
                                            && r.savoring.is_none()
                                    })
                                    .map(|r| {
                                        let dx = x as f32 - r.body.x;
                                        let dz = z as f32 - r.body.z;
                                        (r.id.clone(), r.name, dx * dx + dz * dz)
                                    })
                                    .filter(|(_, _, d2)| {
                                        *d2 <= vadmire::ADMIRE_RADIUS * vadmire::ADMIRE_RADIUS
                                    })
                                    .min_by(|a, b| {
                                        a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal)
                                    })
                            }; // residents 讀鎖釋放
                            if let Some((rid, rname, dist_sq)) = cand {
                                // 冷卻：這位居民對這位玩家上次讚賞是否已過 ADMIRE_COOLDOWN_SECS
                                // （per-connection 冷卻，天然 per-player，防刷好感／洗版）。
                                let now = std::time::Instant::now();
                                let cooldown_ok = match admire_cd.get(&rid) {
                                    Some(prev) => {
                                        now.duration_since(*prev).as_secs()
                                            >= vadmire::ADMIRE_COOLDOWN_SECS
                                    }
                                    None => true,
                                };
                                if vadmire::admire_triggers(streak, dist_sq, cooldown_ok) {
                                    admire_cd.insert(rid.clone(), now);
                                    // 讚賞泡泡（residents 寫鎖即釋；不覆寫既有泡泡＝上面已濾 say 空）。
                                    let pick = now_secs as usize;
                                    let said = {
                                        let mut residents = hub().residents.write().unwrap();
                                        residents
                                            .iter_mut()
                                            .find(|r| r.id == rid)
                                            .map(|r| {
                                                r.say = vadmire::admire_say_line(&name, pick)
                                                    .chars()
                                                    .take(50)
                                                    .collect();
                                                r.say_timer = SAY_SECS;
                                                r.mood_boost_secs = r
                                                    .mood_boost_secs
                                                    .max(voxel_mood::MOOD_BOOST_TALK);
                                            })
                                            .is_some()
                                    }; // residents 寫鎖釋放
                                    if said {
                                        broadcast_players();
                                        // 把「看著你蓋起了東西」寫進記憶（episodic，累積好感）——
                                        // 記憶寫鎖即釋、append 的 IO 在鎖外（守死鎖鐵律）。
                                        let summary = vadmire::admire_memory_line(&name);
                                        let entry = hub()
                                            .memory
                                            .write()
                                            .unwrap()
                                            .add_memory(&rid, &name, &summary);
                                        vmem::append_memory(&entry);
                                        vfeed::append_feed(
                                            "居民讚賞",
                                            rname,
                                            &format!("{rname}讚賞了{name}親手蓋的東西"),
                                        );
                                    }
                                }
                            }
                        }
                    }
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
                // 短鎖快照：被指名者目前是否正跟著「我」（供下方「別跟了」判斷是否真有跟隨可停）。
                let addressed_following_me: bool = match &addressed_id {
                    Some(id) => {
                        let res = hub().residents.read().unwrap();
                        res.iter()
                            .find(|r| &r.id == id)
                            .and_then(|r| r.follow.as_ref())
                            .is_some_and(|(who, _)| *who == player_key)
                    } // residents 讀鎖釋放
                    None => false,
                };

                // 6) 被指名者 → 若是「整平這裡」這類整地指令（合理大小）→ 建立整地任務、
                //    誠實而願意地答應（她現在真的做得到）；否則走既有 LLM 對話路徑。
                //    整地任務條件：命中整地意圖詞 + 非大範圍 + 不落在其他超能力類別（指揮他人/城鎮規劃）。
                if let Some((addr_id, rname, rpersona)) = addressed.clone() {
                    // 鋪面（C 階段）先於整地判斷：鋪面句必帶材料名、整地句不帶，兩者互斥；
                    // 「整平然後鋪成石磚」這類複合句走鋪面（鋪面本就內含整平）。
                    let pave_mat = vdt::detect_pave_command(&clean);
                    // 合理大小的鋪面 → 單人任務（她備料→合成→鋪，說到做到）。
                    let accept_pave = pave_mat.is_some()
                        && !vdt::is_oversized_level(&clean)
                        && detect_over_scope(&clean).is_none();
                    // 大範圍但不離譜的鋪面（一大片/100×100…）→ 號召大家協調鋪面：
                    // 誠實「先從一塊開始鋪」，並真的開工鋪上限大小的一塊（不拒絕、不吹牛）。
                    let coordinated_pave = pave_mat.is_some()
                        && vdt::is_oversized_level(&clean)
                        && !vdt::is_absurd_level(&clean);
                    let is_level = pave_mat.is_none() && vdt::detect_level_command(&clean);
                    // 合理大小的整地 → 單人任務（A 階段）。
                    let accept_level = is_level
                        && !vdt::is_oversized_level(&clean)
                        && detect_over_scope(&clean).is_none();
                    // 大範圍但不離譜的整地 → 號召大家協調整地（B 階段·居民↔居民協調）。
                    // 只有 is_absurd_level（整個世界…）那種連協調也做不到的才落回 LLM 婉拒路徑。
                    let coordinated = is_level
                        && vdt::is_oversized_level(&clean)
                        && !vdt::is_absurd_level(&clean);

                    if accept_pave {
                        // 6-C) 單人鋪面：中心＝玩家腳邊、目標高度＝中心柱地表頂（同整地），
                        //      但頂面用指定材料、材料誠實從她的採集背包扣（不夠→先備料）。
                        if let Some((px, pz, _yaw)) = player_snap {
                            let mat = pave_mat.unwrap(); // accept_pave 已保證 Some
                            let cx = px.floor() as i32;
                            let cz = pz.floor() as i32;
                            let target_y = {
                                let world = hub().deltas.read().unwrap();
                                vdt::safe_target_y(&world, cx, cz)
                            }; // deltas 讀鎖釋放
                            let task = DirectedTask::new_pave(
                                addr_id.clone(), player_key.clone(),
                                cx, cz, vdt::PAVE_RADIUS, target_y, mat,
                            );
                            hub().directed_tasks.write().unwrap().insert(addr_id.clone(), task);
                            // 切換居民狀態：放下手邊事，朝工地中心出發（短鎖即釋；同整地）。
                            {
                                let mut res = hub().residents.write().unwrap();
                                if let Some(r) = res.iter_mut().find(|r| r.id == addr_id) {
                                    r.gather = None;
                                    r.fetch = None;
                                    r.follow = None;
                                    r.seeking_comfort = false;
                                    r.cheer_target = None;
                                    r.wait_timer = 0.0;
                                    r.target_x = cx as f32 + 0.5;
                                    r.target_z = cz as f32 + 0.5;
                                    r.mood_boost_secs =
                                        r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                    r.level_best_d2 = f32::MAX;
                                    r.level_walk_stall = 0.0;
                                }
                            } // residents 寫鎖釋放
                            // 誠實而願意的回覆（坦白要先備料；單播 + 冒泡 + 記憶 + Feed）。
                            let mname = vdt::pave_material_name(mat);
                            let pick = (px.to_bits() ^ pz.to_bits()) as usize;
                            let reply = vdt::pave_accept_line(mname, pick, cx, cz);
                            let msg = serde_json::json!({
                                "t": "talk",
                                "resident_id": &addr_id,
                                "name": rname,
                                "reply": &reply,
                            })
                            .to_string();
                            let _ = out_tx.send(Message::Text(msg)).await;
                            hub().agent_bus.push_decision(
                                addr_id.clone(),
                                AgentDecision::new(AgentAction::Idle, reply.clone(), "鋪面任務"),
                            );
                            {
                                let mut mem = hub().memory.write().unwrap();
                                mem.record_turn(&player_key, &addr_id, &clean, &reply);
                            } // 記憶寫鎖釋放
                            vfeed::append_feed(
                                "鋪面",
                                rname,
                                &format!("答應了{player_key}的請求，動身備料，要把一塊地鋪成{mname}"),
                            );
                        }
                        // 鋪面指令已處理，跳過 LLM 對話路徑。
                    } else if coordinated_pave {
                        // 6-C') 大範圍鋪面 → 號召大家分工：誠實「先從一塊開始鋪」，
                        //       真的開工鋪「協調上限大小」的一塊（做得到的部分先做）。
                        //       切子區/挑人/子任務全複用協調整地那套，只是子區半徑小一號
                        //       （每柱都吃材料）、子任務是 new_pave、各人自備料自鋪自己那塊。
                        if let Some((px, pz, _yaw)) = player_snap {
                            let mat = pave_mat.unwrap(); // coordinated_pave 已保證 Some
                            let cx = px.floor() as i32;
                            let cz = pz.floor() as i32;
                            let target_y = {
                                let world = hub().deltas.read().unwrap();
                                vdt::safe_target_y(&world, cx, cz)
                            }; // deltas 讀鎖釋放
                            let busy: std::collections::HashSet<String> = {
                                hub().directed_tasks.read().unwrap().keys().cloned().collect()
                            }; // directed_tasks 讀鎖釋放
                            let candidates: Vec<(String, f32, f32)> = res_snaps
                                .iter()
                                .map(|(id, _, _, x, z)| (id.clone(), *x, *z))
                                .collect();
                            let workers =
                                vdt::select_coord_workers(&addr_id, cx, cz, &candidates, &busy);
                            let cells = vdt::partition_sub_cells_r(
                                cx, cz, workers.len(), vdt::PAVE_COORD_CELL_RADIUS,
                            );
                            {
                                let mut tasks = hub().directed_tasks.write().unwrap();
                                for (w, (scx, scz)) in workers.iter().zip(cells.iter()) {
                                    let task = DirectedTask::new_pave(
                                        w.clone(), player_key.clone(),
                                        *scx, *scz, vdt::PAVE_COORD_CELL_RADIUS, target_y, mat,
                                    );
                                    tasks.insert(w.clone(), task);
                                }
                            } // directed_tasks 寫鎖釋放
                            hub().coordinated_tasks.write().unwrap().push(
                                CoordinatedLevelTask::new_pave(
                                    player_key.clone(), workers.clone(), mat, cx, cz,
                                ),
                            );
                            {
                                let mut res = hub().residents.write().unwrap();
                                for (w, (scx, scz)) in workers.iter().zip(cells.iter()) {
                                    if let Some(r) = res.iter_mut().find(|r| &r.id == w) {
                                        r.gather = None;
                                        r.fetch = None;
                                        r.follow = None;
                                        r.seeking_comfort = false;
                                        r.cheer_target = None;
                                        r.wait_timer = 0.0;
                                        r.target_x = *scx as f32 + 0.5;
                                        r.target_z = *scz as f32 + 0.5;
                                        r.mood_boost_secs =
                                            r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                        r.level_best_d2 = f32::MAX;
                                        r.level_walk_stall = 0.0;
                                        if w != &addr_id && r.say.is_empty() {
                                            r.say = "來啦！一起鋪～".to_string();
                                            r.say_timer = SAY_SECS;
                                        }
                                    }
                                }
                            } // residents 寫鎖釋放
                            let mname = vdt::pave_material_name(mat);
                            let pick = (px.to_bits() ^ pz.to_bits()) as usize;
                            let reply = vdt::pave_rally_line(mname, pick, cx, cz);
                            let msg = serde_json::json!({
                                "t": "talk",
                                "resident_id": &addr_id,
                                "name": rname,
                                "reply": &reply,
                            })
                            .to_string();
                            let _ = out_tx.send(Message::Text(msg)).await;
                            hub().agent_bus.push_decision(
                                addr_id.clone(),
                                AgentDecision::new(AgentAction::Idle, reply.clone(), "協調鋪面"),
                            );
                            {
                                let mut mem = hub().memory.write().unwrap();
                                mem.record_turn(&player_key, &addr_id, &clean, &reply);
                            } // 記憶寫鎖釋放
                            let helper_count = workers.len();
                            vfeed::append_feed(
                                "鋪面",
                                rname,
                                &format!(
                                    "號召了{helper_count}位居民，各自備料，先從一塊開始把{player_key}要的{mname}地鋪起來"
                                ),
                            );
                        }
                        // 協調鋪面指令已處理，跳過 LLM 對話路徑。
                    } else if accept_level {
                        // 整地中心＝玩家當前位置附近；目標高度＝中心柱現有地表頂。
                        // 注意：player_snap 是 (x, z, yaw)（與步驟 4 定義一致），不是 (x, y, z)。
                        if let Some((px, pz, _yaw)) = player_snap {
                            let cx = px.floor() as i32;
                            let cz = pz.floor() as i32;
                            let target_y = {
                                let world = hub().deltas.read().unwrap();
                                vdt::safe_target_y(&world, cx, cz)
                            }; // deltas 讀鎖釋放
                            // 建立任務並指派給這位居民（覆蓋她原本手邊的事）。
                            let task = DirectedTask::new(
                                addr_id.clone(), player_key.clone(), cx, cz, vdt::LEVEL_RADIUS, target_y,
                            );
                            hub().directed_tasks.write().unwrap().insert(addr_id.clone(), task);
                            // 切換居民狀態：放下採集/尋伴/打氣，朝工地中心出發（短鎖即釋）。
                            {
                                let mut res = hub().residents.write().unwrap();
                                if let Some(r) = res.iter_mut().find(|r| r.id == addr_id) {
                                    r.gather = None;
                                    r.fetch = None;
                                    r.seeking_comfort = false;
                                    r.cheer_target = None;
                                    r.wait_timer = 0.0;
                                    r.target_x = cx as f32 + 0.5;
                                    r.target_z = cz as f32 + 0.5;
                                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                    // 重置「走向工地卡死」偵測（配下方 tick 的就近挪位保險）。
                                    r.level_best_d2 = f32::MAX;
                                    r.level_walk_stall = 0.0;
                                }
                            } // residents 寫鎖釋放
                            // 誠實而願意的回覆（單播給玩家 + 世界冒泡 + 記憶 + Feed）。
                            let pick = (px.to_bits() ^ pz.to_bits()) as usize;
                            let reply = vdt::accept_line(rname, pick, cx, cz);
                            let msg = serde_json::json!({
                                "t": "talk",
                                "resident_id": &addr_id,
                                "name": rname,
                                "reply": &reply,
                            })
                            .to_string();
                            let _ = out_tx.send(Message::Text(msg)).await;
                            hub().agent_bus.push_decision(
                                addr_id.clone(),
                                AgentDecision::new(AgentAction::Idle, reply.clone(), "整地任務"),
                            );
                            // 記下這次交代（她會記得是你請她整地的）。
                            {
                                let mut mem = hub().memory.write().unwrap();
                                mem.record_turn(&player_key, &addr_id, &clean, &reply);
                            } // 記憶寫鎖釋放
                            vfeed::append_feed(
                                "整地",
                                rname,
                                &format!("答應了{player_key}的請求，動身去把一塊地整平"),
                            );
                        }
                        // 整地指令已處理，跳過 LLM 對話路徑（但下方旁聽仍照舊讓其他居民「聽到」）。
                    } else if coordinated {
                        // 6') 大範圍整地 → 露娜號召大家分工協調整地（居民↔居民協調，朝 100×100 的路）。
                        //     切成不重疊、鋪滿的子區，一位居民認領一塊；各子區其實就是一個普通
                        //     DirectedTask，整套「走到工地→分批整地→完成」tick 引擎、就近挪位、持久化全複用。
                        if let Some((px, pz, _yaw)) = player_snap {
                            let cx = px.floor() as i32;
                            let cz = pz.floor() as i32;
                            // 目標高度＝大片地中心柱現有地表頂（全體整到同一高度），且不低於海平面+1（防淹）。
                            let target_y = {
                                let world = hub().deltas.read().unwrap();
                                vdt::safe_target_y(&world, cx, cz)
                            }; // deltas 讀鎖釋放
                            // 號召：領隊（被指名者）＋ 最近的閒居民，跳過正忙的，最多 COORD_MAX_WORKERS 位。
                            let busy: std::collections::HashSet<String> = {
                                hub().directed_tasks.read().unwrap().keys().cloned().collect()
                            }; // directed_tasks 讀鎖釋放
                            let candidates: Vec<(String, f32, f32)> = res_snaps
                                .iter()
                                .map(|(id, _, _, x, z)| (id.clone(), *x, *z))
                                .collect();
                            let workers =
                                vdt::select_coord_workers(&addr_id, cx, cz, &candidates, &busy);
                            // 切子區（一位居民一塊，不重疊、剛好鋪滿整片）。
                            let cells = vdt::partition_sub_cells(cx, cz, workers.len());
                            // 為每位參與居民建立其子區任務（覆蓋原本手邊的事）。
                            {
                                let mut tasks = hub().directed_tasks.write().unwrap();
                                for (w, (scx, scz)) in workers.iter().zip(cells.iter()) {
                                    let task = DirectedTask::new(
                                        w.clone(), player_key.clone(),
                                        *scx, *scz, vdt::COORD_CELL_RADIUS, target_y,
                                    );
                                    tasks.insert(w.clone(), task);
                                }
                            } // directed_tasks 寫鎖釋放
                            // 註冊協調任務（供 tick 偵測「全部子區整完 → 大家一起整平了」）。
                            hub().coordinated_tasks.write().unwrap().push(
                                CoordinatedLevelTask::new(player_key.clone(), workers.clone(), cx, cz),
                            );
                            // 切換每位參與居民狀態：放下手邊事、朝自己的子區中心出發（短鎖即釋）。
                            {
                                let mut res = hub().residents.write().unwrap();
                                for (w, (scx, scz)) in workers.iter().zip(cells.iter()) {
                                    if let Some(r) = res.iter_mut().find(|r| &r.id == w) {
                                        r.gather = None;
                                        r.fetch = None;
                                        r.seeking_comfort = false;
                                        r.cheer_target = None;
                                        r.wait_timer = 0.0;
                                        r.target_x = *scx as f32 + 0.5;
                                        r.target_z = *scz as f32 + 0.5;
                                        r.mood_boost_secs =
                                            r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                        r.level_best_d2 = f32::MAX;
                                        r.level_walk_stall = 0.0;
                                        // 非領隊冒「來了！」泡泡，讓玩家看到多位居民真的響應動起來。
                                        if w != &addr_id && r.say.is_empty() {
                                            r.say = "來啦！一起整～".to_string();
                                            r.say_timer = SAY_SECS;
                                        }
                                    }
                                }
                            } // residents 寫鎖釋放
                            // 領隊的號召回覆（單播 + 冒泡 + 記憶 + Feed）。
                            let pick = (px.to_bits() ^ pz.to_bits()) as usize;
                            let reply = vdt::rally_line(rname, pick, cx, cz);
                            let msg = serde_json::json!({
                                "t": "talk",
                                "resident_id": &addr_id,
                                "name": rname,
                                "reply": &reply,
                            })
                            .to_string();
                            let _ = out_tx.send(Message::Text(msg)).await;
                            hub().agent_bus.push_decision(
                                addr_id.clone(),
                                AgentDecision::new(AgentAction::Idle, reply.clone(), "協調整地"),
                            );
                            {
                                let mut mem = hub().memory.write().unwrap();
                                mem.record_turn(&player_key, &addr_id, &clean, &reply);
                            } // 記憶寫鎖釋放
                            let helper_count = workers.len();
                            vfeed::append_feed(
                                "整地",
                                rname,
                                &format!(
                                    "號召了{helper_count}位居民，一起去把{player_key}要的一大片地整平"
                                ),
                            );
                        }
                        // 協調整地指令已處理，跳過 LLM 對話路徑。
                    } else if vdt::detect_follow_command(&clean) && detect_over_scope(&clean).is_none() {
                        // 指令→任務 v1 第二刀：「跟我來」——她真的能跟著你走，這是她自己做得到的小事，
                        // 不用勞動 LLM 判斷。覆蓋原本手邊的事（採集/尋伴/打氣/探訪/整地皆放下）。
                        {
                            let mut tasks = hub().directed_tasks.write().unwrap();
                            tasks.remove(&addr_id);
                        } // directed_tasks 寫鎖釋放
                        {
                            let mut res = hub().residents.write().unwrap();
                            if let Some(r) = res.iter_mut().find(|r| r.id == addr_id) {
                                r.gather = None;
                                r.fetch = None;
                                r.seeking_comfort = false;
                                r.cheer_target = None;
                                r.visiting = None;
                                r.visit_stay_timer = 0.0;
                                // 玩家指令優先：正遠行在荒野也放下、乖乖跟隨（ROADMAP 756）。
                                r.expedition = None;
                                r.expedition_stay = 0.0;
                                r.wait_timer = 0.0;
                                r.follow = Some((player_key.clone(), vdt::FOLLOW_DURATION_SECS));
                                r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                            }
                        } // residents 寫鎖釋放
                        let pick = clean.len();
                        let reply = vdt::follow_accept_line(pick);
                        let msg = serde_json::json!({
                            "t": "talk",
                            "resident_id": &addr_id,
                            "name": rname,
                            "reply": &reply,
                        })
                        .to_string();
                        let _ = out_tx.send(Message::Text(msg)).await;
                        hub().agent_bus.push_decision(
                            addr_id.clone(),
                            AgentDecision::new(AgentAction::Idle, reply.clone(), "跟隨"),
                        );
                        {
                            let mut mem = hub().memory.write().unwrap();
                            mem.record_turn(&player_key, &addr_id, &clean, &reply);
                        } // 記憶寫鎖釋放
                    } else if addressed_following_me
                        && vdt::detect_follow_stop(&clean)
                        && detect_over_scope(&clean).is_none()
                    {
                        // 「別跟了」：只有正在跟隨這位玩家時這句話才走這條路，清掉、回到平常閒晃。
                        // （沒在跟隨時，同一句話落到下面的 LLM 對話路徑，當普通閒聊處理。）
                        {
                            let mut res = hub().residents.write().unwrap();
                            if let Some(r) = res.iter_mut().find(|r| r.id == addr_id) {
                                r.follow = None;
                            }
                        } // residents 寫鎖釋放
                        let pick = clean.len();
                        let reply = vdt::follow_stop_line(pick);
                        let msg = serde_json::json!({
                            "t": "talk",
                            "resident_id": &addr_id,
                            "name": rname,
                            "reply": &reply,
                        })
                        .to_string();
                        let _ = out_tx.send(Message::Text(msg)).await;
                        hub().agent_bus.push_decision(
                            addr_id.clone(),
                            AgentDecision::new(AgentAction::Idle, reply.clone(), "停止跟隨"),
                        );
                        {
                            let mut mem = hub().memory.write().unwrap();
                            mem.record_turn(&player_key, &addr_id, &clean, &reply);
                        } // 記憶寫鎖釋放
                    } else if let Some((resource, count)) =
                        vfetch::detect_fetch_command(&clean).filter(|_| detect_over_scope(&clean).is_none())
                    {
                        // 指令→任務第三刀：「幫我採集 N 塊 XX」——她放下手邊的事，去採指定數量，
                        // 湊齊了親自走回你身邊交給你。覆蓋原本手邊的事（整地/跟隨/採集/尋伴/打氣/
                        // 探訪/聚會皆放下，同follow accept 的「答應了就專心做」精神）。
                        // 共存 gating（跑腿×技能發明，真進化第一刀）：**發明執行中不接跑腿**——
                        // 她正專心驗證自己的點子，既有任務優先；誠實說明在忙、不假答應。
                        let busy_inventing = {
                            let res = hub().residents.read().unwrap();
                            res.iter()
                                .find(|r| r.id == addr_id)
                                .is_some_and(|r| r.invent_run.is_some())
                        }; // residents 讀鎖釋放
                        if !busy_inventing {
                            {
                                let mut tasks = hub().directed_tasks.write().unwrap();
                                tasks.remove(&addr_id);
                            } // directed_tasks 寫鎖釋放
                            {
                                let mut res = hub().residents.write().unwrap();
                                if let Some(r) = res.iter_mut().find(|r| r.id == addr_id) {
                                    r.gather = None;
                                    r.seeking_comfort = false;
                                    r.cheer_target = None;
                                    r.visiting = None;
                                    r.visit_stay_timer = 0.0;
                                    r.follow = None;
                                    r.clique_meet = None;
                                    // 玩家指令優先：正遠行在荒野也放下、去幫忙採集（ROADMAP 756）。
                                    r.expedition = None;
                                    r.expedition_stay = 0.0;
                                    r.wait_timer = 0.0;
                                    r.fetch = Some(FetchTask::new(player_key.clone(), resource, count));
                                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                }
                            } // residents 寫鎖釋放
                        }
                        let pick = clean.len();
                        let reply = if busy_inventing {
                            "我正在試一個自己想出來的點子，等我試完再幫你採，好嗎？".to_string()
                        } else {
                            vfetch::accept_line(resource.display_name(), count, pick)
                        };
                        let msg = serde_json::json!({
                            "t": "talk",
                            "resident_id": &addr_id,
                            "name": rname,
                            "reply": &reply,
                        })
                        .to_string();
                        let _ = out_tx.send(Message::Text(msg)).await;
                        hub().agent_bus.push_decision(
                            addr_id.clone(),
                            AgentDecision::new(
                                AgentAction::Idle,
                                reply.clone(),
                                if busy_inventing { "發明中婉拒跑腿" } else { "跑腿採集" },
                            ),
                        );
                        {
                            let mut mem = hub().memory.write().unwrap();
                            mem.record_turn(&player_key, &addr_id, &clean, &reply);
                        } // 記憶寫鎖釋放
                        if !busy_inventing {
                            vfeed::append_feed(
                                "跑腿",
                                rname,
                                &format!("答應了{player_key}的請求，動身去採{count}份{}", resource.display_name()),
                            );
                        }
                    } else {
                    // 6a) 短鎖讀記憶 → 組脈絡區塊（B 層精華 + A 層近期記憶 + 本輪對話）→ drop。
                    //     v2 兩層：semantic 精華（身份/目標/偏好/承諾，總是帶上）
                    //             + episodic 近期記憶 + 對話歷史（可被 cap 截斷）。
                    let context = {
                        let mem = hub().memory.read().unwrap();
                        let history = mem.recent_dialogue(&player_key, &addr_id);
                        let episodic = mem.recall(&addr_id, &player_key, vmem::RECALL_LIMIT);
                        let semantic = mem.semantic_facts_for(&addr_id, &player_key);
                        vmem::build_context_block(&history, &episodic, &semantic, &player_key)
                    }; // 記憶讀鎖在此釋放（不在持鎖中 await，符合死鎖鐵律）
                    // 6b) 短鎖讀居民當前心願 → drop（帶進 prompt 讓居民「帶著夢想說話」）。
                    let current_desire: Option<String> = {
                        let des = hub().desires.read().unwrap();
                        des.get_desire(&addr_id).map(|d| d.desire.clone())
                    }; // 心願讀鎖在此釋放
                    // 6b.5) 短鎖讀是否正在孤獨尋伴（ROADMAP 678）——玩家搭話時需致謝送禮。
                    // 同時讀 seeking_comfort，讀完即釋；寫 mood_boost_secs 在下面獨立短鎖。
                    let was_seeking_comfort: bool = {
                        let res = hub().residents.read().unwrap();
                        res.iter().find(|r| r.id == addr_id).map(|r| r.seeking_comfort).unwrap_or(false)
                    }; // residents 讀鎖在此釋放
                    // 6b.6) 互動即時提振心情（ROADMAP 681）：玩家對話時立即設補助倒數。
                    // 短鎖寫 mood_boost_secs 即釋，下一 tick（100ms）廣播更新的 emoji 給前端。
                    {
                        let mut res = hub().residents.write().unwrap();
                        if let Some(r) = res.iter_mut().find(|r| r.id == addr_id) {
                            r.mood_boost_secs =
                                r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        }
                    } // residents 寫鎖在此釋放
                    // 6b.7) 願望漏斗（見 player_wish_prompt_note 上方說明）：對**玩家原文**跑
                    // extract_desire——抽到願望就立刻種進被指名居民的心願（sparked_by=玩家名），
                    // 不等、也不依賴她的 LLM 回覆願意複述。寒暄不觸發（extract_desire 把關）；
                    // 覆寫沿用既有 append-覆蓋語義（每居民一個當前心願、seq 遞增、jsonl append）。
                    let player_spoken_desire: Option<String> = vdes::extract_desire(&clean);
                    if let Some(desire_text) = &player_spoken_desire {
                        let entry = {
                            let mut des = hub().desires.write().unwrap();
                            des.set_desire(&addr_id, desire_text, &player_key)
                        }; // desires 寫鎖釋放
                        vdes::append_desire(&entry);
                        vfeed::append_feed("新心願", &entry.resident, &entry.desire);
                    }
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
                        // 旅人在問配方？→ 注入真實配方事實（grounded 在 voxel_craft，
                        // 只在問配方時注入，純閒聊不多燒 token）。
                        let sys = match recipe_knowledge_block(&clean_for_llm) {
                            Some(facts) => format!("{sys}\n\n{facts}"),
                            None => sys,
                        };
                        // 旅人在要求超出居民能力的大事？→ 強制注入誠實婉拒模板。
                        // 小模型讀不懂「別討好」的抽象指引，但能照抄禁用詞+具體範例的硬規則。
                        // 偵測到才注入（一般對話零負擔），與 recipe_knowledge_block 同模式。
                        let sys = match over_scope_enforcement_block(&clean_for_llm) {
                            Some(block) => format!("{sys}\n\n{block}"),
                            None => sys,
                        };
                        // 她自己發明的技能（真進化第一刀）：注入技能名清單——旅人問
                        // 「你會什麼」時她講得出自己發明的本事（短鎖即釋、不跨 await）。
                        let sys = {
                            let names =
                                { hub().invented.read().unwrap().names_for(&addr_id) };
                            let sys = match vinvent::skills_talk_note(&names) {
                                Some(note) => format!("{sys}\n\n{note}"),
                                None => sys,
                            };
                            // 被明確問「你會什麼」→ 追加指令化醒目區塊（比照配方問答的
                            // detect+注入有效模式）——常駐柔性清單小模型常無感，實測要
                            // 偵測到問題再硬性要求，她才真的講得出發明技能名。
                            match skill_query_enforcement_block(&clean_for_llm, &names) {
                                Some(block) => format!("{sys}\n\n{block}"),
                                None => sys,
                            }
                        };
                        // 孤獨尋伴情境（ROADMAP 678）：居民之前主動走來尋求陪伴，
                        // 旅人終於搭話——引導 LLM 自然地表達感謝與溫暖。
                        let sys = if was_seeking_comfort {
                            format!("{sys}\n\n[情境提示] 你剛才因為心情寂寞主動走到旅人面前、希望有人陪你說說話。現在旅人終於開口了——請在回覆中帶著感激與溫暖，說你心情好多了。")
                        } else {
                            sys
                        };
                        // 帶稱號的維護者（引夢使者 / 築夢工匠…）在跟居民說話 → 依稱號注入專屬區塊，
                        // 讓居民自然地以對應口吻待他（比照配方 / 誠實的精準注入，一般玩家 / 訪客零負擔）。
                        // 誠實照舊：敬愛 / 敬重≠開空頭支票。
                        let sys = match conn_talk_note {
                            Some(note) => format!("{sys}\n\n{note}"),
                            None => sys,
                        };
                        // 願望漏斗：玩家這句話剛親口許願、且已種進她的心願（6b.7）→
                        // 讓回覆脈絡知道這件事，她會自然回應「我記下了 / 我也想要」而不是無感。
                        let sys = match player_spoken_desire.as_deref() {
                            Some(d) => format!("{sys}\n\n{}", player_wish_prompt_note(d)),
                            None => sys,
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
                        // 願望漏斗：這一輪玩家已親口許願種上（6b.7）→ 略過回覆萃取（玩家直說
                        // 優先，避免她隨口一句「我想…」馬上蓋掉玩家親口的願望）；下一輪照舊。
                        if player_spoken_desire.is_none() {
                            if let Some(desire_text) = vdes::extract_desire(&reply) {
                                let new_desire = {
                                    let mut des = hub().desires.write().unwrap();
                                    des.set_desire(&addr_id, &desire_text, &pkey)
                                }; // 心願寫鎖在此釋放
                                vdes::append_desire(&new_desire);
                                vfeed::append_feed("新心願", &new_desire.resident, &new_desire.desire);
                            }
                        }
                        // 孤獨尋伴致謝 + 贈木頭（ROADMAP 678）：清除尋伴狀態、給 1 木頭、通知前端。
                        // 鎖序：residents 寫（即釋）→ inventory 寫（即釋）；不持鎖 await。
                        if was_seeking_comfort && !pkey.is_empty() {
                            {
                                let mut res = hub().residents.write().unwrap();
                                if let Some(r) = res.iter_mut().find(|r| r.id == addr_id) {
                                    r.seeking_comfort = false;
                                }
                            } // residents 寫鎖在此釋放
                            let inv_e = hub().inventory.write().unwrap().give(
                                &pkey,
                                vcomfort::COMFORT_GIFT_BLOCK,
                                vcomfort::COMFORT_GIFT_QTY,
                            );
                            vinv::append_inv(&inv_e);
                            let new_count = hub()
                                .inventory
                                .read()
                                .unwrap()
                                .count(&pkey, vcomfort::COMFORT_GIFT_BLOCK);
                            let inv_msg = serde_json::json!({
                                "t": "inv_update",
                                "block_id": vcomfort::COMFORT_GIFT_BLOCK,
                                "count": new_count,
                            })
                            .to_string();
                            let _ = reply_tx.send(Message::Text(inv_msg)).await;
                            vfeed::append_feed(
                                "孤獨尋伴",
                                rname,
                                &format!(
                                    "旅人 {} 陪伴了我，心情好多了，送上 {} 致謝！",
                                    &pkey,
                                    vcomfort::comfort_gift_name()
                                ),
                            );
                        }
                        // 冒泡（下一 tick 由 tick_residents 套用 say，自動截到 40 字、計時消失）。
                        hub().agent_bus.push_decision(
                            addr_id,
                            AgentDecision::new(AgentAction::Idle, reply, "對話"),
                        );
                    });
                    } // else（非整地指令 → LLM 對話路徑）結束
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
                        // 熔爐煨煮 v1（自主提案）：熔爐配方走「延遲煨熟」，背包 2×2 / 工作台 3×3
                        // 仍瞬間（手感不變）。無論哪條路，配料都已在步驟 1 消耗，這裡先把「消耗」
                        // 持久化 + 回報新存量（兩條路共用），再分岔。
                        let is_furnace = vcraft::find_furnace_recipe(&recipe_id).is_some();
                        // 步驟 2：持久化消耗（全在鎖外，比照 voxel_memory 做法）。
                        for e in &consumed { vinv::append_inv(e); }
                        // 步驟 3：送各消耗方塊的新計數。
                        {
                            let inv_r = hub().inventory.read().unwrap();
                            for &(block_id, _) in recipe.inputs {
                                let cnt = inv_r.count(&name, block_id);
                                let _ = out_tx.try_send(Message::Text(
                                    serde_json::json!({ "t": "inv_update", "block_id": block_id, "count": cnt }).to_string(),
                                ));
                            }
                        } // 讀鎖釋放
                        if is_furnace {
                            // 開一爐慢慢煨——不立刻給成品；`tick_smelt` 熟成後才交付入背包。
                            let now = vfarm::now_secs();
                            let dur = vsmelt::smelt_secs(&recipe_id);
                            let ev = hub().smelt.write().unwrap().start(
                                &name, &recipe_id, recipe.output_block, recipe.output_count, now, dur,
                            ); // smelt 寫鎖釋放
                            vsmelt::append_smelt(&ev); // 持久化（鎖外）
                            let _ = out_tx.try_send(Message::Text(
                                serde_json::json!({
                                    "t": "smelt_started",
                                    "recipe_id": &recipe_id,
                                    "name_zh": recipe.name_zh,
                                    "secs": dur,
                                    "out_count": recipe.output_count
                                }).to_string(),
                            ));
                            // 開爐也算「動手合成」的第一步，一併解里程碑。
                            try_unlock_milestone(&name, "first_craft", &out_tx);
                        } else {
                            // 步驟 4：瞬間給產出方塊（第二把寫鎖，在第一把釋放後取）。
                            let out_e = hub().inventory.write().unwrap().give(
                                &name, recipe.output_block, recipe.output_count,
                            ); // inventory 寫鎖釋放
                            vinv::append_inv(&out_e);
                            let out_cnt = hub().inventory.read().unwrap().count(&name, recipe.output_block);
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
                            // 玩家里程碑 v1（ROADMAP 724）：人生第一次成功合成出成品。
                            try_unlock_milestone(&name, "first_craft", &out_tx);
                        }
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
            Ok(ClientMsg::Plant { x, y, z, seed }) => {
                // 種田 v1（ROADMAP 659）：在農田土(11)上種下種子(14) → FarmSoilSeeded(12)。
                // 第二種作物 v1：`seed` 為胡蘿蔔種子(48) 時改種胡蘿蔔 → CarrotSeeded(46)。
                // 第三種作物 v1：`seed` 為馬鈴薯種子(52) 時改種馬鈴薯 → PotatoSeeded(50)。
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
                // 依 seed 選作物種類（省略/非胡蘿蔔·馬鈴薯種子一律當小麥，向後相容舊客戶端）。
                let is_carrot = seed == Some(vfarm::CARROT_SEEDS_ID);
                let is_potato = seed == Some(vfarm::POTATO_SEEDS_ID);
                let (seed_id, kind, seeded_block) = if is_carrot {
                    (vfarm::CARROT_SEEDS_ID, vfarm::CropKind::Carrot, Block::CarrotSeeded)
                } else if is_potato {
                    (vfarm::POTATO_SEEDS_ID, vfarm::CropKind::Potato, Block::PotatoSeeded)
                } else {
                    (vfarm::SEEDS_ID, vfarm::CropKind::Wheat, Block::FarmSoilSeeded)
                };
                // 消耗 1 顆種子（inventory 寫鎖即釋）。
                let seed_entry = hub().inventory.write().unwrap().take(&name, seed_id, 1);
                let Some(seed_e) = seed_entry else {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "plant_fail", "reason": "沒有種子" }).to_string(),
                    ));
                    continue;
                };
                // 套 delta：FarmSoil → 對應 Seeded 狀態（delta 寫鎖即釋）。
                voxel::set_block(&mut hub().deltas.write().unwrap(), x, y, z, seeded_block);
                // 持久化這塊 Seeded 方塊（此前玩家自種的作物方塊未落地、重啟即整棵消失——農地持久化 v1 補上）。
                vbuild::append_world_block(x, y, z, seeded_block as u8);
                // 記錄農地計時 + 持久化（farm 寫鎖即釋、append 在鎖外）。
                let farm_e = { hub().farm.write().unwrap().plant(x, y, z, vfarm::now_secs(), kind) };
                vfarm::append_farm(&farm_e);
                // 持久化種子消耗。
                vinv::append_inv(&seed_e);
                // 水耕檢查（短讀鎖即釋）：下雨時視同水耕（下雨天氣 v1，ROADMAP 700）。
                let irrigated = *hub().weather.read().unwrap() || {
                    let deltas = hub().deltas.read().unwrap();
                    is_irrigated_in_delta(&deltas, x, y, z)
                };
                // 廣播方塊更新 + 送背包更新。
                broadcast_block(x, y, z, seeded_block);
                let new_seed_cnt = hub().inventory.read().unwrap().count(&name, seed_id);
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({
                        "t": "inv_update",
                        "block_id": seed_id,
                        "count": new_seed_cnt
                    })
                    .to_string(),
                ));
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({
                        "t": "plant_ok", "x": x, "y": y, "z": z,
                        "irrigated": irrigated, "carrot": is_carrot, "potato": is_potato
                    }).to_string(),
                ));
                // 玩家里程碑 v1（ROADMAP 724）：人生第一次種下種子。
                try_unlock_milestone(&name, "first_farm", &out_tx);
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
                // 2) 短鎖取居民快照（residents 讀鎖即釋）。y 供紀念物 v1（732）找腳邊空位用。
                let res_snap: Option<(&'static str, f32, f32, f32)> = {
                    let residents = hub().residents.read().unwrap();
                    residents
                        .iter()
                        .find(|r| r.id == resident_id)
                        .map(|r| (r.name, r.body.x, r.body.y, r.body.z))
                };
                let Some((rname, rx, ry, rz)) = res_snap else {
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
                // 5b) 送對禮物 v1（ROADMAP 722）：這位居民是否正懷抱一句「送這個物品就能實現」的
                // 非建造類心願（desires 讀鎖即釋，不與其他鎖巢狀）？建造類心願交給蓋家系統的
                // 心願成真（720），這裡刻意不搶。
                let item_wish_hit: bool = {
                    let desires = hub().desires.read().unwrap();
                    desires.get_desire(&resident_id).is_some_and(|d| {
                        !d.fulfilled
                            && vbuild::classify_desire(&d.desire).is_none()
                            && vgift::classify_item_desire(&d.desire) == Some(item_id)
                    })
                };
                // 6) 讀好感度（memory 讀鎖即釋）。
                let affinity = {
                    hub().memory.read().unwrap().affinity_count(&name, &resident_id)
                };
                // 7) 組道謝台詞（純函式，無鎖）——心願送到 > 食物(BREAD_ID=19) > 一般禮物。
                let pick = (vfarm::now_secs() as usize).wrapping_add(item_id as usize);
                let thanks = if item_wish_hit {
                    vgift::item_wish_thanks_line(rname, iname, &name)
                } else if vgift::is_treasure_gift(item_id) {
                    vgift::treasure_gift_thanks_line(&name, affinity, pick)
                } else if item_id == vfish::COOKED_FISH_ID {
                    // 烤魚是玩家「釣起→烤熟」的一道熱佳餚，用專屬台詞（比一般食物更歡欣）。
                    vgift::cooked_fish_thanks_line(&name, affinity, pick)
                } else if item_id == vfarm::BAKED_POTATO_ID {
                    // 烤地薯是玩家「種田→收成→烤熟」的一道熱佳餚，用專屬台詞（比一般食物更歡欣）。
                    vgift::baked_potato_thanks_line(&name, affinity, pick)
                } else if item_id == vcraft::STEW_ID {
                    // 野菜暖湯是玩家湊齊三種親手種的作物、在工作台拌煮的一鍋料理，用專屬台詞（最觸動）。
                    vgift::stew_thanks_line(&name, affinity, pick)
                } else if vgift::is_food_gift(item_id) {
                    vgift::food_gift_thanks_line(&name, affinity, pick)
                } else {
                    vgift::gift_thanks_line(iname, &name, affinity, pick)
                };
                // 8) residents 寫鎖：設 say + say_timer + 心情補助（即釋，ROADMAP 681）。
                {
                    let mut residents = hub().residents.write().unwrap();
                    if let Some(r) = residents.iter_mut().find(|r| r.id == resident_id) {
                        r.say = thanks.chars().take(50).collect();
                        r.say_timer = SAY_SECS;
                        // 贈禮帶來更持久的心情補助（比對話長 2 分鐘）。
                        r.mood_boost_secs =
                            r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_GIFT);
                        // 你送的食物她會細細享用 v1（ROADMAP 765）：若送的是食物，居民不立刻吃掉，
                        // 而是捧在手中，稍後在閒下來的安靜片刻真的享用（見 tick_residents 的享用分支）。
                        // 同時只捧一份——連續送多份食物，取最新那份（不排隊、天然防洗版）。
                        if vgift::is_food_gift(item_id) {
                            r.savoring =
                                Some((item_id, name.clone(), vsavor::SAVOR_DELAY_SECS));
                        }
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
                // 玩家里程碑 v1（ROADMAP 724）：人生第一次送禮 + 若與這位居民已到「友人」門檻
                // （沿用前端 656 好感度指示燈 count>=3 = 金心的同一道門檻），順便解鎖「初次熟識」。
                try_unlock_milestone(&name, "first_gift", &out_tx);
                if affinity >= FRIEND_AFFINITY_THRESHOLD {
                    try_unlock_milestone(&name, "first_bond", &out_tx);
                }
                // 11) Feed：記錄贈禮事件（鎖外 IO）。
                vfeed::append_feed(
                    "贈禮",
                    rname,
                    &format!("{name}送了{iname}給{rname}"),
                );
                // 11b) 你送的心意，她擺了出來 v1（ROADMAP 732）：可展示的餽贈化為紀念物，
                // 由居民真的擺進世界（記憶→改變世界佈局，因玩家善意而生、後果永久可見）。
                // 鎖序：deltas 讀（找空位，即釋）→ deltas 寫（set_block，即釋）→ 廣播 →
                // memory 寫（即釋）→ 持久化/Feed（鎖外 IO），全程短鎖循序、不巢狀，守死鎖鐵律。
                if let Some(keep_id) = vkeep::keepsake_block(item_id) {
                    // 找居民腳邊一個合理空位（沿用居民放置的 find_place_spot：絕不放身體格、
                    // 目標格必須空氣、伸手可及；環格填滿就 None＝誠實不擺、天然防洗版）。
                    let spot = {
                        let world = hub().deltas.read().unwrap();
                        vinvent::find_place_spot(
                            &world,
                            rx.floor() as i32,
                            ry.floor() as i32,
                            rz.floor() as i32,
                        )
                    }; // deltas 讀鎖釋放
                    if let (Some((kx, ky, kz)), Some(block)) =
                        (spot, Block::from_u8(keep_id))
                    {
                        {
                            let mut world = hub().deltas.write().unwrap();
                            voxel::set_block(&mut world, kx, ky, kz, block);
                        } // deltas 寫鎖釋放
                        broadcast_block(kx, ky, kz, block);
                        // 放下的方塊可能堵住水路 → 喚醒鄰格重算（同居民建造慣例）。
                        enqueue_water_around(kx, ky, kz);
                        // 持久化這塊紀念物（重啟後你送的心意仍留在世界裡）。
                        vbuild::append_world_block(kx, ky, kz, keep_id);
                        let kname = vkeep::keepsake_name(keep_id);
                        // 居民記得「我把旅人送的東西擺出來了」→ 供日記（650）昇華成生命故事。
                        let mem = vkeep::keepsake_memory_line(&name, kname, pick);
                        let entry = {
                            hub().memory.write().unwrap().add_memory(&resident_id, &name, &mem)
                        };
                        vmem::append_memory(&entry);
                        vfeed::append_feed(
                            vkeep::FEED_KIND,
                            rname,
                            &vkeep::keepsake_feed_line(rname, &name, kname),
                        );
                    }
                }
                // 11c) 居民種下你送的種子，長成她自己的一畦菜園 v1（ROADMAP 754）：把「人類種田」
                // 與「居民生活」第一次接起來——已和你要好的居民，會把你送的種子真的種進家旁的土裡，
                // 隨既有農地 tick 長大。居民第一次成為種田系統的生產者，你的餽贈在世界裡生根長大。
                // 鎖序：deltas 讀（找可耕地，即釋）→ deltas 寫（set_block，即釋）→ 廣播 →
                // farm 寫（登記農地計時，即釋）→ residents 寫（換上她種田的台詞，即釋）→
                // memory 寫（即釋）→ 持久化/Feed（鎖外 IO），全程短鎖循序、不巢狀，守死鎖鐵律。
                if let Some(crop) = vseed::plantable_crop_name(item_id) {
                    // 只有已和你要好（好感 ≥ 門檻）的居民，才會鄭重把你的種子種下。
                    if affinity >= vseed::PLANT_AFFINITY {
                        // 依種子挑作物種類 + Seeded 方塊（比照 Plant handler，向後相容三種作物）。
                        let (kind, seeded_block) = if item_id == vfarm::CARROT_SEEDS_ID {
                            (vfarm::CropKind::Carrot, Block::CarrotSeeded)
                        } else if item_id == vfarm::POTATO_SEEDS_ID {
                            (vfarm::CropKind::Potato, Block::PotatoSeeded)
                        } else {
                            (vfarm::CropKind::Wheat, Block::FarmSoilSeeded)
                        };
                        // 在居民腳邊找一塊可耕地（草/泥土、頭頂空氣、搆得到）；找不到就誠實不種。
                        let spot = {
                            let world = hub().deltas.read().unwrap();
                            vseed::find_garden_spot(rx, ry, rz, |x, y, z| {
                                voxel::effective_block_at(&world, x, y, z) as u8
                            })
                        }; // deltas 讀鎖釋放
                        if let Some((gx, gy, gz)) = spot {
                            {
                                let mut world = hub().deltas.write().unwrap();
                                voxel::set_block(&mut world, gx, gy, gz, seeded_block);
                            } // deltas 寫鎖釋放
                            broadcast_block(gx, gy, gz, seeded_block);
                            enqueue_water_around(gx, gy, gz);
                            // 登記農地計時 + 持久化（farm 寫鎖即釋、append 在鎖外）——
                            // 農地持久化 v1：此前計時器純記憶體，重啟後這畦會永遠卡在幼苗長不出來。
                            let farm_e = {
                                hub().farm.write().unwrap().plant(gx, gy, gz, vfarm::now_secs(), kind)
                            };
                            vfarm::append_farm(&farm_e);
                            // 持久化這塊作物方塊（重啟後你送的種子仍留在世界裡）。
                            vbuild::append_world_block(gx, gy, gz, seeded_block as u8);
                            // 收成回贈 v1（ROADMAP 755）：登記這畦「因你而生」的田——記下座標、
                            // 種它的居民、送種子的你、作物種類。日後它熟了、她遇到你，就會收成回贈。
                            // giftgarden 寫鎖即釋、append IO 在鎖外（守死鎖鐵律）。
                            {
                                let gg_entry = {
                                    hub().giftgarden.write().unwrap().record(
                                        &vgg::pos_key(gx, gy, gz),
                                        &resident_id,
                                        &name,
                                        vgg::crop_code(kind),
                                    )
                                };
                                vgg::append_gift_garden(&gg_entry);
                            }
                            // 換上她邊種邊說的暖句（覆蓋 8) 的通用道謝——種田這句更貼切、更活）。
                            {
                                let mut residents = hub().residents.write().unwrap();
                                if let Some(r) =
                                    residents.iter_mut().find(|r| r.id == resident_id)
                                {
                                    r.say = vseed::plant_say_line(&name, crop, pick)
                                        .chars()
                                        .take(40)
                                        .collect();
                                    r.say_timer = SAY_SECS;
                                }
                            }
                            broadcast_players();
                            // 居民記得「我把旅人送的種子種下了」（掛玩家名下，好感自然累積）。
                            let mem = vseed::plant_memory_line(&name, crop);
                            let entry = {
                                hub().memory.write().unwrap().add_memory(
                                    &resident_id,
                                    &name,
                                    &mem,
                                )
                            };
                            vmem::append_memory(&entry);
                            vfeed::append_feed(
                                "種下心意",
                                rname,
                                &vseed::plant_feed_line(rname, &name, crop),
                            );
                        }
                    }
                }
                // 12) 送對禮物 v1（ROADMAP 722）：心願送到了——標記已實現（desires 寫鎖即釋，
                // 落地 jsonl 沿用既有 append-only 慣例）+ 額外記憶 + 全員廣播 + Feed。
                if item_wish_hit {
                    let marked = {
                        hub().desires.write().unwrap().mark_fulfilled(&resident_id)
                    };
                    if let Some(entry) = marked {
                        vdes::append_desire(&entry);
                    }
                    let wish_mem = vgift::item_wish_memory(iname);
                    let entry3 = {
                        hub().memory.write().unwrap().add_memory(&resident_id, &name, &wish_mem)
                    };
                    vmem::append_memory(&entry3);
                    let _ = hub().tx.send(std::sync::Arc::new(vgift::item_wish_msg(rname, iname, &name)));
                    vfeed::append_feed(
                        "心願送到了",
                        rname,
                        &format!("{name}送來了{rname}一直想要的{iname}"),
                    );
                }
            }

            // ── 居民交易 v1（ROADMAP 670）────────────────────────────────────────
            Ok(ClientMsg::TradeRequest { resident_id }) => {
                // 1) 短鎖取玩家位置（players 讀鎖即釋）。
                let player_pos: Option<(f32, f32)> = {
                    let players = hub().players.read().unwrap();
                    players.get(&my_id).map(|p| (p.x, p.z))
                };
                let Some((px, pz)) = player_pos else { continue; };
                // 2) 短鎖取居民快照（residents 讀鎖即釋）。
                let res_snap: Option<(&'static str, f32, f32)> = {
                    let residents = hub().residents.read().unwrap();
                    residents.iter()
                        .find(|r| r.id == resident_id)
                        .map(|r| (r.name, r.body.x, r.body.z))
                };
                let Some((rname, rx, rz)) = res_snap else { continue; };
                // 3) 驗觸及範圍。
                let dx = px - rx;
                let dz = pz - rz;
                if dx * dx + dz * dz > vtrade::TRADE_REACH * vtrade::TRADE_REACH {
                    let msg = serde_json::json!({
                        "t": "trade_fail",
                        "reason": "走近一點再交易"
                    }).to_string();
                    let _ = out_tx.send(Message::Text(msg)).await;
                    continue;
                }
                // 4) 讀好感度（memory 讀鎖即釋）。
                let affinity = {
                    hub().memory.read().unwrap().affinity_count(&name, &resident_id)
                };
                // 5) 生成交易提案（確定性純函式，無鎖）。
                let offer = vtrade::make_offer(&resident_id, affinity);
                let oname = vtrade::item_name_zh(offer.offer_item);
                let wname = vtrade::item_name_zh(offer.want_item);
                let say = vtrade::offer_say_line(&offer);
                let expires_at = vfarm::now_secs() + vtrade::TRADE_OFFER_TTL;
                // 6) 存待確認提案（pending_trades 寫鎖即釋）。
                {
                    hub().pending_trades.write().unwrap()
                        .insert(resident_id.clone(), (offer.clone(), expires_at));
                }
                // 7) 設居民台詞（residents 寫鎖即釋）。
                {
                    let mut residents = hub().residents.write().unwrap();
                    if let Some(r) = residents.iter_mut().find(|r| r.id == resident_id) {
                        r.say = say.chars().take(50).collect();
                        r.say_timer = SAY_SECS;
                    }
                }
                broadcast_players();
                // 8) 回傳 trade_offer（單播給發起玩家）。
                let msg = serde_json::json!({
                    "t": "trade_offer",
                    "resident_id": &resident_id,
                    "resident_name": rname,
                    "offer_item": offer.offer_item,
                    "offer_count": offer.offer_count,
                    "offer_name": oname,
                    "want_item": offer.want_item,
                    "want_count": offer.want_count,
                    "want_name": wname,
                    "affinity": affinity,
                }).to_string();
                let _ = out_tx.send(Message::Text(msg)).await;
            }

            Ok(ClientMsg::TradeAccept { resident_id }) => {
                // 1) 短鎖取玩家位置（players 讀鎖即釋）。
                let player_pos: Option<(f32, f32)> = {
                    let players = hub().players.read().unwrap();
                    players.get(&my_id).map(|p| (p.x, p.z))
                };
                let Some((px, pz)) = player_pos else { continue; };
                // 2) 短鎖取居民快照（residents 讀鎖即釋）。
                let res_snap: Option<(&'static str, f32, f32)> = {
                    let residents = hub().residents.read().unwrap();
                    residents.iter()
                        .find(|r| r.id == resident_id)
                        .map(|r| (r.name, r.body.x, r.body.z))
                };
                let Some((rname, rx, rz)) = res_snap else { continue; };
                // 3) 驗觸及範圍。
                let dx = px - rx;
                let dz = pz - rz;
                if dx * dx + dz * dz > vtrade::TRADE_REACH * vtrade::TRADE_REACH {
                    let msg = serde_json::json!({
                        "t": "trade_fail",
                        "reason": "走近一點再交易"
                    }).to_string();
                    let _ = out_tx.send(Message::Text(msg)).await;
                    continue;
                }
                // 4) 取並移除待確認提案（pending_trades 寫鎖即釋）。
                let pending = {
                    hub().pending_trades.write().unwrap().remove(&resident_id)
                };
                let Some((offer, expires_at)) = pending else {
                    let msg = serde_json::json!({
                        "t": "trade_fail",
                        "reason": "沒有待確認的交易提案，請重新點「交易」"
                    }).to_string();
                    let _ = out_tx.send(Message::Text(msg)).await;
                    continue;
                };
                // 5) 驗提案是否過期。
                if vfarm::now_secs() > expires_at {
                    let msg = serde_json::json!({
                        "t": "trade_fail",
                        "reason": "交易提案已過期，請重新點「交易」"
                    }).to_string();
                    let _ = out_tx.send(Message::Text(msg)).await;
                    continue;
                }
                let wname = vtrade::item_name_zh(offer.want_item);
                let oname = vtrade::item_name_zh(offer.offer_item);
                // 6) 驗並扣玩家背包中 want_item × want_count（inventory 寫鎖即釋）。
                let taken = {
                    hub().inventory.write().unwrap()
                        .take(&name, offer.want_item, offer.want_count)
                };
                let Some(taken_entry) = taken else {
                    let msg = serde_json::json!({
                        "t": "trade_fail",
                        "reason": format!("背包裡的{}不夠（需要{}個）", wname, offer.want_count)
                    }).to_string();
                    let _ = out_tx.send(Message::Text(msg)).await;
                    continue;
                };
                vinv::append_inv(&taken_entry);
                // 7) 給玩家 offer_item × offer_count（inventory 寫鎖即釋）。
                let give_entry = {
                    hub().inventory.write().unwrap()
                        .give(&name, offer.offer_item, offer.offer_count)
                };
                vinv::append_inv(&give_entry);
                // 8) 寫 1 筆記憶（memory 寫鎖即釋）。
                let mem = vtrade::trade_memory(&name, oname, wname);
                let mem_entry = {
                    hub().memory.write().unwrap().add_memory(&resident_id, &name, &mem)
                };
                vmem::append_memory(&mem_entry);
                // 9) 居民說成交台詞（residents 寫鎖即釋）。
                let done_say = vtrade::done_say_line(&name, oname);
                {
                    let mut residents = hub().residents.write().unwrap();
                    if let Some(r) = residents.iter_mut().find(|r| r.id == resident_id) {
                        r.say = done_say.chars().take(50).collect();
                        r.say_timer = SAY_SECS;
                    }
                }
                broadcast_players();
                // 10) 回傳兩筆 inv_update（讓前端同步背包） + trade_done。
                let want_remain = hub().inventory.read().unwrap().count(&name, offer.want_item);
                let offer_new = hub().inventory.read().unwrap().count(&name, offer.offer_item);
                let upd1 = serde_json::json!({
                    "t": "inv_update",
                    "block_id": offer.want_item,
                    "count": want_remain,
                }).to_string();
                let _ = out_tx.send(Message::Text(upd1)).await;
                let upd2 = serde_json::json!({
                    "t": "inv_update",
                    "block_id": offer.offer_item,
                    "count": offer_new,
                }).to_string();
                let _ = out_tx.send(Message::Text(upd2)).await;
                let done_msg = serde_json::json!({
                    "t": "trade_done",
                    "resident_name": rname,
                    "got_item": offer.offer_item,
                    "got_name": oname,
                    "got_count": offer.offer_count,
                    "gave_item": offer.want_item,
                    "gave_name": wname,
                    "gave_count": offer.want_count,
                }).to_string();
                let _ = out_tx.send(Message::Text(done_msg)).await;
                // 玩家里程碑 v1（ROADMAP 724）：人生第一次與居民完成以物易物。
                try_unlock_milestone(&name, "first_trade", &out_tx);
                // 11) Feed（鎖外 IO）。
                vfeed::append_feed(
                    "交易",
                    rname,
                    &format!("{name}與{rname}交易：{wname}×{}→{oname}×{}", offer.want_count, offer.offer_count),
                );
            }

            // ── 箱子 v1：開啟 ─────────────────────────────────────────────────────────
            Ok(ClientMsg::OpenChest { x, y, z }) => {
                // 驗觸及 + 目標是 Chest 方塊 → 回傳 chest_view（箱子內容清單）。
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                let target = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !matches!(target, Block::Chest) { continue; }
                if !voxel::can_break(&hub().deltas.read().unwrap(), px, py, pz, x, y, z) {
                    continue; // 借用 can_break 的觸及範圍驗證（方塊存在且夠近）
                }
                let pos = vchest::pos_key(x, y, z);
                let contents = hub().chest.read().unwrap().contents(&pos);
                let items: Vec<serde_json::Value> = contents
                    .iter()
                    .map(|&(id, cnt)| serde_json::json!({ "id": id, "count": cnt }))
                    .collect();
                let msg = serde_json::json!({
                    "t": "chest_view",
                    "x": x, "y": y, "z": z,
                    "items": items,
                }).to_string();
                let _ = out_tx.send(Message::Text(msg)).await;
            }

            // ── 箱子 v1：放入物品 ──────────────────────────────────────────────────────
            Ok(ClientMsg::ChestPut { x, y, z, item_id, count }) => {
                if count == 0 { continue; }
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                let target = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !matches!(target, Block::Chest) { continue; }
                if !voxel::can_break(&hub().deltas.read().unwrap(), px, py, pz, x, y, z) { continue; }
                let pos = vchest::pos_key(x, y, z);
                // 1) 扣背包（inventory 寫鎖即釋）。
                let taken = hub().inventory.write().unwrap().take(&name, item_id, count);
                let Some(inv_e) = taken else {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "chest_fail", "reason": "背包數量不足" }).to_string(),
                    ));
                    continue;
                };
                vinv::append_inv(&inv_e);
                // 2) 放入箱子（chest 寫鎖即釋）。
                let chest_e = hub().chest.write().unwrap().put(&pos, item_id, count);
                vchest::append_chest(&chest_e);
                // 3) 回傳最新 inv_update + chest_view。
                let new_inv_count = hub().inventory.read().unwrap().count(&name, item_id);
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "inv_update", "block_id": item_id, "count": new_inv_count }).to_string(),
                ));
                let contents = hub().chest.read().unwrap().contents(&pos);
                let items: Vec<serde_json::Value> = contents
                    .iter()
                    .map(|&(id, cnt)| serde_json::json!({ "id": id, "count": cnt }))
                    .collect();
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "chest_view", "x": x, "y": y, "z": z, "items": items }).to_string(),
                ));
            }

            // ── 箱子 v1：取出物品 ──────────────────────────────────────────────────────
            Ok(ClientMsg::ChestTake { x, y, z, item_id, count }) => {
                if count == 0 { continue; }
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                let target = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !matches!(target, Block::Chest) { continue; }
                if !voxel::can_break(&hub().deltas.read().unwrap(), px, py, pz, x, y, z) { continue; }
                let pos = vchest::pos_key(x, y, z);
                // 1) 從箱子取（chest 寫鎖即釋）。
                let (actual, chest_e) = hub().chest.write().unwrap().take(&pos, item_id, count);
                if actual == 0 {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "chest_fail", "reason": "箱子裡沒有這個物品" }).to_string(),
                    ));
                    continue;
                }
                vchest::append_chest(&chest_e);
                // 2) 加入背包（inventory 寫鎖即釋）。
                let inv_e = hub().inventory.write().unwrap().give(&name, item_id, actual);
                vinv::append_inv(&inv_e);
                // 3) 回傳最新 inv_update + chest_view。
                let new_inv_count = hub().inventory.read().unwrap().count(&name, item_id);
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "inv_update", "block_id": item_id, "count": new_inv_count }).to_string(),
                ));
                let contents = hub().chest.read().unwrap().contents(&pos);
                let items: Vec<serde_json::Value> = contents
                    .iter()
                    .map(|&(id, cnt)| serde_json::json!({ "id": id, "count": cnt }))
                    .collect();
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "chest_view", "x": x, "y": y, "z": z, "items": items }).to_string(),
                ));
            }

            // ── 告示牌 v1（ROADMAP 740）：寫／改寫牌面文字 ────────────────────────────────
            // 鎖序：delta 讀（驗方塊/reach）→ 釋 → sign 寫（存字）→ 釋 → 鎖外 IO + 廣播；
            // 循序不巢狀，守 prod-deadlock 鐵律。
            Ok(ClientMsg::SignSet { x, y, z, text }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                let target = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !matches!(target, Block::Sign) { continue; }
                if !voxel::can_break(&hub().deltas.read().unwrap(), px, py, pz, x, y, z) { continue; }
                // 清洗玩家輸入（去控制字元、截長度）；空字串＝清空牌面。
                let clean = vsign::sanitize_text(&text);
                let ev = hub().sign.write().unwrap().set(&vsign::pos_key(x, y, z), clean.clone());
                vsign::append_sign(&ev);
                // 廣播給所有人（含自己），前端據此更新／移除該座標的浮字。
                broadcast_sign(x, y, z, &clean);
            }

            // 木門 v1（ROADMAP 693）：右鍵切換門的開/關狀態。
            // 鎖序：delta 讀（驗方塊）→ delta 寫（toggle）→ drop；不嵌套、不持鎖 IO。
            Ok(ClientMsg::ToggleDoor { x, y, z }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                if !voxel::in_reach(px, py, pz, x, y, z) { continue; }
                let target_blk = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                let new_blk = match target_blk {
                    Block::DoorClosed => Block::DoorOpen,
                    Block::DoorOpen   => Block::DoorClosed,
                    _ => continue, // 不是門，忽略
                };
                voxel::set_block(&mut hub().deltas.write().unwrap(), x, y, z, new_blk);
                broadcast_block(x, y, z, new_blk);
            }

            Ok(ClientMsg::SleepInBed { x, y, z }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                if !voxel::in_reach(px, py, pz, x, y, z) { continue; }
                let target_blk = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if target_blk != Block::Bed { continue; } // 不是床，忽略
                let phase = { hub().world_time.read().unwrap().phase() };
                if !vt::is_sleepable(phase) {
                    let msg = serde_json::json!({
                        "t": "sleep_fail",
                        "reason": "現在還不是晚上，睡不著喔。",
                    }).to_string();
                    let _ = out_tx.send(Message::Text(msg)).await;
                    continue;
                }
                { hub().world_time.write().unwrap().skip_to_dawn(); }
                broadcast_players(); // 讓所有人立刻看到跳到黎明的天色（time_of_day 隨快照廣播）
                vfeed::append_feed("睡覺", &name, "睡了一覺，天亮了！");
                let msg = serde_json::json!({ "t": "sleep_ok" }).to_string();
                let _ = out_tx.send(Message::Text(msg)).await;
                // 玩家里程碑 v1（ROADMAP 724）：人生第一次在床上睡到天亮。
                try_unlock_milestone(&name, "first_sleep", &out_tx);
            }

            // ── 垂釣 v1（ROADMAP 734）：拋竿 ──────────────────────────────────────
            Ok(ClientMsg::FishCast { x, y, z }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                // 1) 手上要有釣竿（inventory 讀鎖即釋）。
                let has_rod = hub().inventory.read().unwrap().count(&name, vfish::FISHING_ROD_ID) >= 1;
                if !has_rod {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "fish_fail", "reason": "你手上沒有釣竿——先用木板做一支吧。"
                    }).to_string())).await;
                    continue;
                }
                // 2) 觸及範圍內（沿用互動統一 reach，和客戶端瞄準一致；垂釣本就隔岸拋線）。
                if !voxel::in_reach(px, py, pz, x, y, z) { continue; }
                // 3) 目標要是水面（來源水或流動水）——delta 讀鎖快照即釋。
                let target_blk = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !vfish::is_water_block(target_blk as u8) {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "fish_fail", "reason": "要對準水面才能拋竿喔。"
                    }).to_string())).await;
                    continue;
                }
                // 4) 已經有一竿在水裡就別重拋（pending_fish 讀鎖即釋）。
                if hub().pending_fish.read().unwrap().contains_key(&name) {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "fish_fail", "reason": "你的釣線已經在水裡了。"
                    }).to_string())).await;
                    continue;
                }
                // 5) 記下上鉤時刻（3~7 秒後，隨機有變化），存進 pending（寫鎖即釋）。
                // roll 走真隨機（同專案其他機率骰慣例），避免玩家用時間/座標精算上鉤時機。
                let now = vfarm::now_secs();
                let wait = vfish::bite_secs(rand::random::<u64>());
                let ready_at = now + wait;
                {
                    hub().pending_fish.write().unwrap().insert(name.clone(), (ready_at, x, y, z));
                }
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "fish_cast_ok", "wait": wait, "hint": vfish::cast_hint()
                }).to_string())).await;
            }

            // ── 垂釣 v1（ROADMAP 734）：收竿 ──────────────────────────────────────
            Ok(ClientMsg::FishReel) => {
                let now = vfarm::now_secs();
                // 1) 取這竿的上鉤時刻（pending_fish 讀鎖即釋）。
                let pending = hub().pending_fish.read().unwrap().get(&name).copied();
                let Some((ready_at, _fx, _fy, _fz)) = pending else {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "fish_fail", "reason": "你還沒拋竿呢。"
                    }).to_string())).await;
                    continue;
                };
                // 2) 太早收竿——魚還沒上鉤，保留這竿讓玩家再等。
                if now < ready_at {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "fish_too_early", "hint": vfish::too_early_hint()
                    }).to_string())).await;
                    continue;
                }
                // 3) 時機到——移除這竿（寫鎖即釋），釣起漁獲。
                { hub().pending_fish.write().unwrap().remove(&name); }
                // 稀有度走真隨機（同專案慣例），玩家無法用收竿時機精算穩定釣起稀有魚。
                let fish_id = vfish::pick_catch(rand::random::<u64>());
                // 4) 漁獲進背包（inventory 寫鎖即釋 → append_inv → inv_update 單播）。
                let entry = hub().inventory.write().unwrap().give(&name, fish_id, 1);
                vinv::append_inv(&entry);
                let nc = hub().inventory.read().unwrap().count(&name, fish_id);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "inv_update", "block_id": fish_id, "count": nc
                }).to_string())).await;
                // 5) 世界動態 feed（不在場的人回來也讀得到）+ 收竿揭曉單播。
                vfeed::append_feed("垂釣", &name, &vfish::catch_feed_line(&name, fish_id));
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "fish_catch",
                    "item_id": fish_id,
                    "item_name": vfish::fish_name_zh(fish_id),
                    "line": vfish::catch_self_line(fish_id),
                }).to_string())).await;
                // 玩家里程碑 v1（ROADMAP 724）：人生第一次在水邊釣起魚。
                try_unlock_milestone(&name, "first_fish", &out_tx);
            }

            Ok(ClientMsg::Eat { item_id }) => {
                // 親手煮的暖食自己也能享用 v1（779）。
                // 1) 只有「自己親手煮的熟食」才吃得下（純函式判定，生食/原料/非食物擋掉）。
                if !vmeal::is_edible_dish(item_id) {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "eat_fail",
                        "reason": "這個沒法吃，先煮一道熱食吧～"
                    }).to_string())).await;
                    continue;
                }
                // 2) 驗並消耗一份（inventory 寫鎖即釋）。
                let taken = { hub().inventory.write().unwrap().take(&name, item_id, 1) };
                let Some(inv_entry) = taken else {
                    let iname = vgift::item_name_zh(item_id);
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "eat_fail",
                        "reason": format!("背包裡沒有{iname}")
                    }).to_string())).await;
                    continue;
                };
                vinv::append_inv(&inv_entry);
                let dish = vgift::item_name_zh(item_id);
                let pick = (vfarm::now_secs() as usize).wrapping_add(item_id as usize);
                // 3) 玩家自享的暖意回饋（純函式）＋回 inv_update / eat_ok。
                let cozy = vmeal::savor_self_line(dish, pick);
                let remain = hub().inventory.read().unwrap().count(&name, item_id);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "inv_update", "block_id": item_id, "count": remain,
                }).to_string())).await;
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "eat_ok", "item_id": item_id, "item_name": dish, "line": cozy,
                }).to_string())).await;
                // 4) 里程碑：人生第一次嚐一口自己親手煮的料理。
                try_unlock_milestone(&name, "first_taste", &out_tx);
                // 5) 交織點：若剛好站在某位居民身邊、且分享冷卻就緒，居民被你的滿足感染
                //    （心情點亮＋暖泡泡＋交情記憶＋動態牆）。分享冷卻只節流這一拍、不影響吃本身。
                let share_ready = last_eat_share
                    .map(|t| t.elapsed().as_secs_f32() >= vmeal::SHARE_COOLDOWN_SECS)
                    .unwrap_or(true);
                if share_ready {
                    // 5a) 短鎖取玩家位置（players 讀鎖即釋）。
                    let ppos: Option<(f32, f32)> = {
                        hub().players.read().unwrap().get(&my_id).map(|p| (p.x, p.z))
                    };
                    if let Some((px, pz)) = ppos {
                        // 5b) 挑半徑內、有空反應（未睡、say 空）的最近居民（residents 讀鎖即釋）。
                        let target: Option<(String, &'static str)> = {
                            let residents = hub().residents.read().unwrap();
                            residents
                                .iter()
                                .filter(|r| !r.asleep && r.say.is_empty())
                                .filter_map(|r| {
                                    let dx = px - r.body.x;
                                    let dz = pz - r.body.z;
                                    let d2 = dx * dx + dz * dz;
                                    if d2 <= vmeal::SHARE_RADIUS * vmeal::SHARE_RADIUS {
                                        Some((d2, r.id.clone(), r.name))
                                    } else {
                                        None
                                    }
                                })
                                .min_by(|a, b| {
                                    a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                                })
                                .map(|(_, id, nm)| (id, nm))
                        };
                        if let Some((rid, rname)) = target {
                            // 5c) 居民冒暖泡泡＋心情點亮（residents 寫鎖即釋；再確認 say 仍空防搶拍）。
                            let sline = vmeal::share_line(dish, pick);
                            let said = {
                                let mut residents = hub().residents.write().unwrap();
                                match residents.iter_mut().find(|r| r.id == rid) {
                                    Some(r) if r.say.is_empty() => {
                                        r.say = sline.chars().take(50).collect();
                                        r.say_timer = SAY_SECS;
                                        r.mood_boost_secs =
                                            r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                        true
                                    }
                                    _ => false,
                                }
                            };
                            if said {
                                last_eat_share = Some(std::time::Instant::now());
                                broadcast_players();
                                // 交情記憶（memory 寫鎖即釋、append IO 在鎖外，守死鎖鐵律）。
                                let summary = vmeal::share_memory_line(&name, dish);
                                let entry = hub()
                                    .memory
                                    .write()
                                    .unwrap()
                                    .add_memory(&rid, &name, &summary);
                                vmem::append_memory(&entry);
                                // 城鎮動態牆。
                                vfeed::append_feed(
                                    "暖意分享",
                                    rname,
                                    &vmeal::share_feed_line(rname, &name, dish),
                                );
                            }
                        }
                    }
                }
            }

            // 重複 Join 或壞訊息：忽略。
            _ => {}
        }
    }

    // 位置持久化 v1：斷線前把最後位置存起來（登入帳號），讓重登時回到這裡。
    // 安全：短鎖讀位置即釋（不持鎖後再 IO），IO 在鎖外，零死鎖風險。
    if let Some(ref email) = account_email {
        let final_pos = {
            let players = hub().players.read().unwrap();
            players.get(&my_id).map(|p| (p.x, p.y, p.z, p.yaw))
        }; // 讀鎖在此釋放
        if let Some((px, py, pz, pyaw)) = final_pos {
            vpp::save_player_pos(email, px, py, pz, pyaw); // IO 在鎖外
        }
    }

    // 收攤：移除玩家、廣播、收掉任務。
    // 垂釣 v1：清掉這位玩家進行中的拋竿（純記憶體，斷線即散）。
    { hub().pending_fish.write().unwrap().remove(&name); }
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
/// 同時啟動水流動 tick——**刻意在此一併 spawn**，讓 main.rs 免加新的 spawn 呼叫
///（守「別碰 main.rs」邊界；main 只要照舊呼叫 spawn_farm_tick 一次即可）。
pub fn spawn_farm_tick() {
    tokio::spawn(async move {
        let _ = hub(); // 觸發 hub 初始化
        let mut ticker = tokio::time::interval(Duration::from_secs(15));
        loop {
            ticker.tick().await;
            tick_farm();
            tick_grove(); // 植樹造林 v1（ROADMAP 738）：同節拍檢查樹苗是否長成。
            tick_smelt(); // 熔爐煨煮 v1（自主提案）：同節拍交付熟成的爐（成品入背包 + 廣播）。
            maybe_birth(); // 人口成長 v1：低頻檢查聚落是否有餘裕誕生一位新居民。
        }
    });
    spawn_water_tick();
}

/// 人口成長 v1（世代傳承）：低頻（併入 15 秒農作節拍）檢查是否誕生一位新居民。
///
/// **成本紀律是核心**：先做最便宜的早退（到上限就直接回，連鎖都不碰）；出生條件由
/// [`vroster::should_birth`] 純函式判定（上限未滿＋聚落過半安頓＋距上次夠久＋機率門檻）。
/// 真的要生時：確定性挑一位父母 → 新居民生在父母家附近、分到不衝突的家域 →
/// **繼承父母 1~2 個已發明技能**（複製進她自己的技能庫、標記「承自XX」，一出生就會做、
/// 零 LLM 重用照舊）→ 落地名冊 + 技能 jsonl（重啟後人口與技能都還在）→ Feed + 冒泡慶祝。
///
/// **鎖紀律**：各 store 短鎖循序取放、不巢狀、不持鎖 await/IO；名冊增長在 residents 寫鎖內
/// 安全完成並把 `RESIDENT_POP` +1；技能 append / Feed 都在鎖外。
fn maybe_birth() {
    let base = RESIDENT_COUNT;
    let pop = resident_count();
    let max = vroster::max_residents(base);
    if pop >= max {
        return; // 到上限：絕不無限生（最便宜的早退，連鎖都不碰）
    }

    let now = vfarm::now_secs();
    // 首次呼叫（LAST_BIRTH_UNIX 仍為 0）：先嘗試從持久化檔載回基準（跨重啟累積 elapsed）。
    if LAST_BIRTH_UNIX.load(Ordering::Relaxed) == 0 {
        match vroster::load_last_birth_unix() {
            Some(saved) => {
                // 有持久化基準：還原到記憶體，繼續往下算 elapsed（可能已超過間隔 → 可生）。
                LAST_BIRTH_UNIX.store(saved, Ordering::Relaxed);
            }
            None => {
                // 真正首次（檔缺 = 從沒設過基準 / 舊部署）：記當下為基準並存檔。
                // 下次重啟時 load_last_birth_unix() 讀到此值，elapsed 從此刻開始累積。
                LAST_BIRTH_UNIX.store(now, Ordering::Relaxed);
                vroster::save_last_birth_unix(now); // 鐵律：鎖外 IO（此處無持任何鎖）
                return; // 真正首次：只建基準、不生（避免伺服器一啟動就冒新居民）
            }
        }
    }
    let elapsed = now.saturating_sub(LAST_BIRTH_UNIX.load(Ordering::Relaxed)) as f32;
    let interval = vroster::birth_interval_secs();
    if elapsed < interval {
        return; // 距上次出生還不夠久：低頻
    }

    // 聚落是否穩定：短讀鎖數「已蓋好至少一樣東西」的居民 → 立即釋放。
    let settled = {
        let goals = hub().goals.read().unwrap();
        (0..pop)
            .filter(|&i| !goals.done_kinds(&format!("vox_res_{i}")).is_empty())
            .count()
    }; // goals 讀鎖釋放
    let ready = vroster::settlement_ready(settled, pop);
    if !vroster::should_birth(pop, max, ready, elapsed, interval, rand::random::<f32>()) {
        return;
    }

    // ── 決定出生 ──
    let new_i = pop; // id 連續：新居民＝下一個索引 vox_res_{pop}
    let seed = now ^ (new_i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let parent_i = vroster::pick_parent_index(pop, seed);
    let parent_id = format!("vox_res_{parent_i}");
    let parent_name = RESIDENT_NAMES[parent_i];
    let new_id = format!("vox_res_{new_i}");
    let new_name = RESIDENT_NAMES[new_i];

    // 家域分配：讀既有全部家域 + 父母家域快照（短讀鎖即釋），選一塊不衝突的新家。
    let (existing_homes, parent_home): (Vec<(i32, i32)>, (i32, i32)) = {
        let rs = hub().residents.read().unwrap();
        let homes: Vec<(i32, i32)> =
            rs.iter().map(|r| (r.home_x.floor() as i32, r.home_z.floor() as i32)).collect();
        let ph = rs
            .get(parent_i)
            .map(|r| (r.home_x.floor() as i32, r.home_z.floor() as i32))
            .unwrap_or((0, 0));
        (homes, ph)
    }; // residents 讀鎖釋放
    let (hox, hoz) = vroster::birth_home_base(parent_home.0, parent_home.1, &existing_homes, seed);
    let body = vr::dry_ground_spawn(hox, hoz);

    // 技能繼承（北極星）：父母技能庫挑最多 2 個複製給新生兒；invented 寫鎖內完成、append 鎖外。
    let (inherited_names, inherited_recs): (Vec<String>, Vec<vinvent::InventedSkillRecord>) = {
        let mut inv = hub().invented.write().unwrap();
        let parent_skills = inv.inheritable_for(&parent_id);
        let mut names = Vec::new();
        let mut recs = Vec::new();
        for from in parent_skills.iter().take(2) {
            if let Some(rec) = inv.inherit(&new_id, from, parent_name) {
                names.push(rec.name.clone());
                recs.push(rec);
            }
        }
        (names, recs)
    }; // invented 寫鎖釋放
    for rec in &inherited_recs {
        vinvent::append_invented_skill(rec); // 鎖外落地：重啟後新生兒仍會做
    }

    // 建構新居民 + 冒出生泡泡；residents 寫鎖內 append 並把人口 +1。
    let mut newcomer = build_resident(new_i, body.x, body.z, body);
    let birth_say: String = if let Some(first) = inherited_names.first() {
        format!("我是{new_name}，剛來到這片天地～{parent_name}把「{first}」教給了我！")
    } else {
        format!("我是{new_name}，剛在這片天地誕生，請多指教！")
    };
    newcomer.say = birth_say.chars().take(40).collect();
    newcomer.say_timer = SAY_SECS;
    {
        let mut rs = hub().residents.write().unwrap();
        // 讓父母也冒一句歡迎（若當下沒在說話）。
        if let Some(parent) = rs.get_mut(parent_i) {
            if parent.say.is_empty() {
                parent.say = format!("歡迎來到世界，{new_name}！").chars().take(40).collect();
                parent.say_timer = SAY_SECS;
            }
        }
        rs.push(newcomer);
        RESIDENT_POP.store(rs.len(), Ordering::Relaxed); // 人口 +1（寫鎖內安全）
    } // residents 寫鎖釋放
    LAST_BIRTH_UNIX.store(now, Ordering::Relaxed);
    vroster::save_last_birth_unix(now); // 持久化：重啟後 elapsed 從此刻繼續算（不歸零）

    // 名冊落地（鎖外）：重啟後這位新居民還在。
    vroster::append_roster_entry(&vroster::RosterEntry {
        resident: new_id.clone(),
        name: new_name.to_string(),
        home_base_x: hox,
        home_base_z: hoz,
        parent: parent_id,
        parent_name: parent_name.to_string(),
        birth_unix: now,
    });

    // Feed 廣播（鎖外）：世界動態上看得到「新居民誕生 + 承繼了誰的技能」。
    let detail = if let Some(first) = inherited_names.first() {
        format!("在{parent_name}家附近誕生了，承繼了{parent_name}的「{first}」")
    } else {
        format!("在{parent_name}家附近誕生了")
    };
    vfeed::append_feed("新居民誕生", new_name, &detail);
}

/// 水流 tick 頻率（秒）：0.5s（2Hz）——水一格一格漫開，像麥塊那樣「看得到在流」，
/// 又不會太快吃 CPU。每 tick 只處理佇列上限，成本與「正在流的水量」成正比、非世界大小。
const WATER_TICK_DT: f32 = 0.5;
/// 單次水流 tick 最多處理幾格（防某次改動炸出巨量佇列拖垮 tick；剩下的下一 tick 續處理）。
/// 對齊效能鐵律：worst case 每 tick 有界，不會讓伺服器 tick 尖峰。
const WATER_MAX_PER_TICK: usize = 512;

/// 啟動水流動 tick 迴圈（2Hz）。處理待處理佇列：每格算穩定值 → 有變化才寫 delta +
/// 廣播 + 持久化 + 喚醒鄰格。空佇列時幾乎零成本（pop 一次就退出）。
fn spawn_water_tick() {
    tokio::spawn(async move {
        let _ = hub();
        let mut ticker = tokio::time::interval(Duration::from_secs_f32(WATER_TICK_DT));
        loop {
            ticker.tick().await;
            tick_water();
        }
    });
}

/// 一次水流推進：從佇列取至多 WATER_MAX_PER_TICK 格，逐格算穩定方塊。
///
/// 鎖紀律（嚴守無鎖 await + 短鎖即釋，比照 tick_farm）：
/// 1. 短鎖持 water_queue → 取出這批要處理的座標 → drop。
/// 2. 短鎖持 deltas(read) → 為每格快照鄰域、算穩定值、蒐集「有變化」的 → drop。
/// 3. 短鎖持 deltas(write) → 套用變化 → drop。
/// 4. 鎖外：broadcast + 持久化 append + 把受影響鄰格排回佇列（各自短鎖，循序不巢狀）。
/// 全程無 await、不跨鎖持有，符合居民 agency tick 的同款模式。
fn tick_water() {
    // 1) 取這批要處理的座標（短鎖即釋）。
    let batch: Vec<(i32, i32, i32)> = {
        let mut q = hub().water_queue.lock().unwrap();
        let n = q.len().min(WATER_MAX_PER_TICK);
        (0..n).filter_map(|_| q.pop()).collect()
    };
    if batch.is_empty() {
        return;
    }

    // 2) 讀鎖快照 → 純計算每格穩定值，蒐集真正有變化的 (x,y,z,新方塊)。
    let changes: Vec<(i32, i32, i32, Block)> = {
        let world = hub().deltas.read().unwrap();
        batch
            .iter()
            .filter_map(|&(x, y, z)| {
                let n = vwater::neighborhood_at(&world, x, y, z);
                let next = vwater::settled_block(&n);
                if next != n.here {
                    Some((x, y, z, next))
                } else {
                    None
                }
            })
            .collect()
    }; // deltas 讀鎖釋放
    if changes.is_empty() {
        return;
    }

    // 3) 寫鎖套用全部變化（一次持有、批次寫，不 await）。
    {
        let mut world = hub().deltas.write().unwrap();
        for &(x, y, z, b) in &changes {
            voxel::set_block(&mut world, x, y, z, b);
        }
    } // deltas 寫鎖釋放

    // 4) 鎖外收尾：廣播、持久化、喚醒鄰格。
    for &(x, y, z, b) in &changes {
        broadcast_block(x, y, z, b);
        // 持久化水流狀態變化——與既有「世界方塊差異」走同一條 append-only log
        //（voxel_resident_blocks.jsonl），重啟後水流結果還在（來源仍會重新流一次收斂）。
        vbuild::append_world_block(x, y, z, b as u8);
        // 這格變了 → 它與周圍鄰格可能連鎖變化，排回佇列下輪續算（收斂後自然清空）。
        enqueue_water_around(x, y, z);
    }
}

/// 農地成熟 tick——找所有已成熟的幼苗，換成成熟小麥，廣播給所有連線。
/// 判定 (fx,fy,fz) 鄰近（XZ ±WATER_RANGE 格、Y ±1 格）是否有任何水方塊（來源水或流動水）。
/// 接受 WorldDelta 快照，無鎖、無副作用（水耕農業 v1 ROADMAP 686）。
fn is_irrigated_in_delta(deltas: &voxel::WorldDelta, fx: i32, fy: i32, fz: i32) -> bool {
    let r = vfarm::FARM_WATER_RANGE;
    for dz in -r..=r {
        for dx in -r..=r {
            for dy in -1..=1_i32 {
                if voxel::effective_block_at(deltas, fx + dx, fy + dy, fz + dz).is_any_water() {
                    return true;
                }
            }
        }
    }
    false
}

/// 純同步、短鎖即釋（delta 讀鎖 → drop → farm 讀鎖 → drop → farm 寫鎖 → drop → delta 寫鎖 → drop → broadcast）。
/// 水耕農業 v1（ROADMAP 686）：水源鄰近時生長加速 45s，否則 90s。
/// 下雨天氣 v1（ROADMAP 700）：每輪先擲骰演變天氣（短寫鎖即釋），下雨時所有農地視同水耕。
fn tick_farm() {
    // 天氣擲骰（短寫鎖即釋，不與其他鎖巢狀）：純函式 next_raining 決定下一輪狀態。
    // ROADMAP 701：順便偵測「晴→雨」轉換，設一次性旗標讓 tick_residents 觸發居民雨天反應。
    let raining = {
        let mut w = hub().weather.write().unwrap();
        let was_raining = *w;
        *w = vweather::next_raining(*w, rand::random::<f32>());
        if *w && !was_raining {
            *hub().rain_started_flag.write().unwrap() = true;
        }
        *w
    };
    let now = vfarm::now_secs();
    // 短讀鎖取 delta 快照用於水耕判斷（每 15s 一次，clone 代價小），馬上釋放。
    let deltas_snap: voxel::WorldDelta = hub().deltas.read().unwrap().clone();
    // 短讀鎖取成熟座標＋作物種類（含水耕加速），馬上釋放。
    let mature: Vec<((i32, i32, i32), vfarm::CropKind)> = hub()
        .farm
        .read()
        .unwrap()
        .mature_plots_irrigated(now, |fx, fy, fz| {
            raining || is_irrigated_in_delta(&deltas_snap, fx, fy, fz)
        });
    if mature.is_empty() {
        return;
    }
    for ((fx, fy, fz), kind) in mature {
        // 寫鎖清掉農地記錄（避免下輪重複處理）＋持久化移除（farm 寫鎖即釋、append 鎖外）。
        let farm_e = { hub().farm.write().unwrap().remove(fx, fy, fz) };
        if let Some(farm_e) = farm_e {
            vfarm::append_farm(&farm_e);
        }
        // 依作物種類決定「該格此刻應是的 Seeded 方塊」與「成熟後方塊」。
        let (expected_seeded, mature_block) = match kind {
            vfarm::CropKind::Wheat => (Block::FarmSoilSeeded, Block::WheatMature),
            vfarm::CropKind::Carrot => (Block::CarrotSeeded, Block::CarrotMature),
            vfarm::CropKind::Potato => (Block::PotatoSeeded, Block::PotatoMature),
        };
        // 農地持久化 v1 自癒守衛：唯有該格當下真的還是對應 Seeded 方塊才轉成熟。
        // 若計時器與世界方塊發生分歧（例：世界重置清了方塊卻沒清農地 jsonl），
        // 這裡只默默清掉孤兒計時、不憑空長出幻影作物——用快照判斷（每格唯一、無爭用）。
        if voxel::effective_block_at(&deltas_snap, fx, fy, fz) != expected_seeded {
            continue;
        }
        // delta 寫鎖：把 Seeded 換成對應 Mature 狀態。
        voxel::set_block(&mut hub().deltas.write().unwrap(), fx, fy, fz, mature_block);
        // 成熟後的方塊也持久化（重啟後看到的是成熟作物，而非又退回幼苗）。
        vbuild::append_world_block(fx, fy, fz, mature_block as u8);
        // 廣播方塊更新（所有連線玩家即時看到作物成熟變色）。
        broadcast_block(fx, fy, fz, mature_block);
    }
}

/// 熔爐煨煮 v1（自主提案）：交付所有熟成的爐——成品入該玩家背包、廣播 `smelt_done`（前端跳
/// 「你的Ｘ煨好了」暖提示 + 同步背包計數，比照 return_gift 前端管線），並持久化「交付」事件。
///
/// 鎖紀律（嚴守短鎖即釋、不巢狀、不持鎖 await/IO，比照 `tick_farm`）：
///  ① smelt 讀鎖取熟成清單（clone）→ 釋。② 每爐：smelt 寫鎖移除該爐（idempotent，防重複交付）→ 釋
///     → append 移除事件（鎖外）。③ inventory 寫鎖 give 成品 → 釋 → append（鎖外）→ 讀鎖取新計數 → 釋。
///  ④ 全域廣播（鎖外）。全程循序、不巢狀。
fn tick_smelt() {
    let now = vfarm::now_secs();
    // 早退省鎖：沒有任何爐在煨就不動（大多數 tick 如此）。
    if hub().smelt.read().unwrap().is_empty() {
        return;
    }
    let ready = { hub().smelt.read().unwrap().ready(now) };
    for (id, job) in ready {
        // 移除這爐（寫鎖即釋）+ 持久化移除；已被別的 tick 交付就跳過（防重複套用）。
        let removed = { hub().smelt.write().unwrap().remove(id) };
        let Some(done_ev) = removed else { continue };
        vsmelt::append_smelt(&done_ev);
        // 成品入該玩家背包（寫鎖即釋）+ 持久化 + 取新計數。
        let out_e = {
            hub().inventory.write().unwrap().give(&job.player, job.output_block, job.output_count)
        };
        vinv::append_inv(&out_e);
        let new_count = hub().inventory.read().unwrap().count(&job.player, job.output_block);
        let iname = vgift::item_name_zh(job.output_block);
        // 全域廣播 smelt_done（前端依 player 是否為自己決定顯示提示 + 更新背包，比照 return_gift）。
        let msg = serde_json::json!({
            "t": "smelt_done",
            "player": &job.player,
            "recipe_id": &job.recipe_id,
            "item_id": job.output_block,
            "item_name": iname,
            "qty": job.output_count,
            "count": new_count,
        })
        .to_string();
        let _ = hub().tx.send(std::sync::Arc::new(msg));
    }
}

/// 植樹造林 v1（ROADMAP 738）：樹苗成熟 tick（併入 `spawn_farm_tick` 的 15 秒節拍，
/// 不另開 tick 迴圈）。長成的樹苗 → 用純函式 `grown_tree_blocks` 展開成樹幹＋樹冠 delta 並廣播。
///
/// 鎖紀律（嚴守短鎖即釋、不巢狀、不持鎖 await/IO，比照 `tick_farm`）：
///  ① grove 讀鎖取成熟座標 → 釋。② 每株：grove 寫鎖清記錄 → 釋。
///  ③ delta 讀鎖 clone 快照（判斷哪些格是空氣、只往空氣長樹，不覆蓋玩家既有建物）→ 釋。
///  ④ 逐格 delta 寫鎖設方塊 → 釋 + 廣播。全程循序、不巢狀。
///
/// **非破壞性**：除了樹苗底座本身（換成樹幹）外，只在「目前是空氣」的格子長出樹幹／樹冠，
/// 絕不覆蓋玩家蓋的房子或天然地形——守資料安全精神。
fn tick_grove() {
    let now = vfarm::now_secs();
    // ① grove 讀鎖取成熟座標，馬上釋放。
    let mature: Vec<(i32, i32, i32)> = hub().grove.read().unwrap().mature_saplings(now);
    if mature.is_empty() {
        return;
    }
    for (sx, sy, sz) in mature {
        // ② 寫鎖清掉樹苗記錄（避免下輪重複長樹）。
        hub().grove.write().unwrap().remove(sx, sy, sz);
        // ③ 短讀鎖 clone delta 快照供「只往空氣長樹」判斷，馬上釋放。
        let snap: voxel::WorldDelta = hub().deltas.read().unwrap().clone();
        // ④ 逐格展開樹幹／樹冠。
        for (tx, ty, tz, gb) in vgrove::grown_tree_blocks(sx, sy, sz) {
            let block = match gb {
                vgrove::GroveBlock::Trunk => Block::Wood,
                vgrove::GroveBlock::Leaf => Block::Leaves,
            };
            let is_base = tx == sx && ty == sy && tz == sz;
            // 底座（樹苗原位）一定換成樹幹；其餘只在目前是空氣時長出，不覆蓋既有方塊。
            if !is_base && voxel::effective_block_at(&snap, tx, ty, tz) != Block::Air {
                continue;
            }
            voxel::set_block(&mut hub().deltas.write().unwrap(), tx, ty, tz, block);
            broadcast_block(tx, ty, tz, block);
        }
    }
}

/// `GET /voxel/diary` — 回傳所有居民的日記頁（curated 內心反思 + 當前心願）。
/// 日記是「瞥見居民沒說出口的內心」，不是聊天謄本：記憶在 `voxel_diary` 內被昇華成
/// 第一人稱反思、同主題收斂降噪、永不倒出玩家原話 / 玩家名（隱私邊界）。
/// 短鎖讀取快照 → drop 鎖 → 格式化 → 回 JSON；零 LLM、零持久化、零 migration。
/// 呼叫端（瀏覽器）直接 `fetch("/voxel/diary")` 即可，無需任何認證。
pub async fn voxel_diary_handler() -> axum::response::Response {
    use axum::http::header;

    // 1) 短鎖快照居民 id/name → drop（循序取鎖、不巢狀、守鎖紀律）。
    let resident_ids: Vec<(String, &'static str)> = {
        let rs = hub().residents.read().unwrap();
        rs.iter().map(|r| (r.id.clone(), r.name)).collect()
    };

    // 2) 短鎖快照全部長期記憶 + 淡忘計數（每位）→ drop。
    let all_memories: Vec<(String, Vec<crate::voxel_memory::MemoryEntry>, usize)> = {
        let mem = hub().memory.read().unwrap();
        resident_ids
            .iter()
            .map(|(id, _)| (id.clone(), mem.all_memories_for(id), mem.faded_count(id)))
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
        .map(|(((id, name), (_, mems, faded)), desire)| {
            voxel_diary::format_diary_page(id, name, desire.as_deref(), mems, *faded)
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
        (0..resident_count())
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

/// 乙太方界·居民交情網（ROADMAP 708）：回傳 4 位居民兩兩之間的情誼層級與拜訪次數。
///
/// 情誼（672）驅動問候語/八卦轉述/互助蓋家，早就悄悄累積在伺服器內部，
/// 但玩家從沒有任何管道看見「這座小社會到底誰跟誰要好」——這份資料只存在於
/// 決策邏輯裡，從未攤開給人看過。本端點把它讀出來，供前端「交情網」面板呈現。
pub async fn voxel_relations_handler() -> axum::response::Response {
    use axum::http::header;
    let rows: Vec<serde_json::Value> = {
        // 短讀鎖一次性快照全部兩兩組合 → 立即釋放，不與其他鎖巢狀。
        let bonds = hub().bonds.read().unwrap();
        let n = resident_count(); // 含出生居民（人口成長 v1）：兩兩交情全都攤開
        let mut out = Vec::with_capacity(n * n.saturating_sub(1) / 2);
        for i in 0..n {
            for j in (i + 1)..n {
                let id_a = format!("vox_res_{i}");
                let id_b = format!("vox_res_{j}");
                let tier = resident_tier_of(&bonds, &id_a, &id_b);
                out.push(serde_json::json!({
                    "a": resident_name_of(&id_a),
                    "b": resident_name_of(&id_b),
                    "tier": vbonds::tier_key(tier),
                    "visits": bonds.visit_count(resident_name_of(&id_a), resident_name_of(&id_b)),
                }));
            }
        }
        out
    };
    let body = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// 乙太方界·居民技能簿（ROADMAP 719）：回傳每位居民已發明／學會的技能名清單。
///
/// 技能發明（716）與傳授（717）一路都靠 Feed 一次性文字曝光（「露娜教了我『燒
/// 玻璃』這招！」），播報一過就沒了——玩家從沒有任何管道能回頭查「這座小社會
/// 現在誰會什麼」，這份資料只活在 `InventedSkillStore` 裡，從未攤開給人看過。
/// 跟 708 交情網同一手法：讓早已存在的系統第一次被看見，而不是新造一套技能
/// 系統；本端點純讀取、零副作用。
pub async fn voxel_skills_handler() -> axum::response::Response {
    use axum::http::header;
    let rows: Vec<serde_json::Value> = {
        // 短讀鎖一次性快照 4 位居民的技能清單 → 立即釋放，不與其他鎖巢狀。
        let invented = hub().invented.read().unwrap();
        (0..resident_count())
            .map(|i| {
                let rid = format!("vox_res_{i}");
                serde_json::json!({
                    "name": resident_name_of(&rid),
                    "skills": invented.names_for(&rid),
                })
            })
            .collect()
    };
    let body = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// 乙太方界·玩家里程碑（ROADMAP 724）：回傳全部里程碑定義 + 這位玩家各自是否已達成。
///
/// 居民有技能簿（719）、交情網（708）可回頭翻閱自己的成長，玩家的療癒循環
/// （採集→合成→蓋造→種田→贈禮→交易→熟識→安眠）至今卻沒有任何一處能讓玩家
/// 自己回頭看看「我走了多遠」。本端點純讀取、零副作用，`?player=` 缺省時
/// 全部里程碑一律回「未達成」（前端知道要先問玩家名再開面板）。
pub async fn voxel_milestones_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::http::header;
    let player_name = params.get("player").map(|s| s.as_str()).unwrap_or("").trim().to_string();
    let rows: Vec<serde_json::Value> = {
        // 短讀鎖一次性快照這位玩家已達成的里程碑集合 → 立即釋放，不與其他鎖巢狀。
        let store = hub().milestones.read().unwrap();
        vmiles::MILESTONES
            .iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "name_zh": m.name_zh,
                    "desc_zh": m.desc_zh,
                    "icon": m.icon,
                    "earned": !player_name.is_empty() && store.has(&player_name, m.id),
                })
            })
            .collect()
    };
    let body = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
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
    let is_night = vt::is_sleepable(phase);
    // 深夜（入夜過渡之後的正夜）才是居民真正躺下睡覺的時段（ROADMAP 739）：
    // Evening 仍有夜生活、只縮小活動；到 Night 才就寢。醒來則沿用 is_night 判定
    // （只要離開夜間系列＝天色轉亮就起床）。
    let is_deep_night = matches!(phase, TimePhase::Night);
    // 下雨天氣（700）接上居民行為（ROADMAP 701）：短讀鎖取目前是否下雨。
    let raining = *hub().weather.read().unwrap();
    // 雨剛開始的一次性旗標：consume-once（讀到就清回 false），供下方觸發居民雨天反應台詞。
    let rain_just_started = {
        let mut f = hub().rain_started_flag.write().unwrap();
        let v = *f;
        *f = false;
        v
    };
    // 夜間歸巢遮蔽：批次快照各居民已蓋好的小屋座標（goals 讀鎖即釋），
    // 供下面 residents 寫鎖那段挑閒晃中心用——不與 residents 鎖巢狀（守死鎖鐵律）。
    let house_locations: HashMap<String, (i32, i32, i32)> = {
        let goals = hub().goals.read().unwrap();
        (0..resident_count())
            .filter_map(|j| {
                let rid = format!("vox_res_{j}");
                goals.house_of(&rid).map(|loc| (rid, loc))
            })
            .collect()
    }; // goals 讀鎖釋放
    // 整地/鋪面任務：批次快照各居民的任務中心/半徑/是否鋪面（directed_tasks 讀鎖即釋），
    // 供 residents 寫鎖那段判斷「該去工地施工、還是照常閒晃」——不與 residents 鎖巢狀。
    // 鋪面任務（is_pave=true）備料中（r.gather 有值）會讓位給採集分支：她先去採原料，
    // 採完（gather 清空）自然走回工地繼續鋪。
    let directed_snaps: HashMap<String, (i32, i32, i32, bool)> = {
        let tasks = hub().directed_tasks.read().unwrap();
        tasks
            .iter()
            .map(|(rid, t)| (rid.clone(), (t.cx, t.cz, t.radius, t.pave.is_some())))
            .collect()
    }; // directed_tasks 讀鎖釋放
    // 本 tick 已抵達工地、要整地一批的居民（鎖內收集，鎖外套用方塊改動）。
    let mut level_workers: Vec<String> = Vec::new();
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
        (0..resident_count())
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

    // 卡住脫困/送回事件（鎖內偵測、鎖外記 Feed）。格式：(resident_name, 脫困結果)。
    let mut rescue_events: Vec<(&'static str, vr::Rescue)> = Vec::new();

    // 重返心中的牌子 Feed 事件（讀牌 v3，ROADMAP 743）：鎖內偵測抵達，鎖外記 Feed。
    // 格式：(resident_name, 牌面引文)。
    let mut pilgrimage_feed: Vec<(&'static str, String)> = Vec::new();
    // 遠行探野 Feed（遠行探野 v1，ROADMAP 756）：鎖內收集「某居民啟程遠行／遠行歸來」事件，
    // 鎖釋放後統一 append_feed（不在持居民鎖時做 IO）。(居民名, 播報詳情)。
    let mut expedition_feed: Vec<(&'static str, String)> = Vec::new();

    // 邊陲營火路標（遠行 v2，PLAN_ETHERVOX item 7「在遠方留下痕跡」）：鎖內收集「某居民抵達邊陲、
    // 要在落點升起一堆營火路標」的請求；地表計算、deltas 寫、廣播、持久化、記憶與 Feed 全在鎖釋放後
    // 統一處理（不在持居民鎖時碰 deltas/IO）。(居民 id, 居民名, 落點 x, 落點 z, 方位名)。
    let mut expedition_campfires: Vec<(String, &'static str, i32, i32, String)> = Vec::new();

    // 遠行帶回的邊陲風物（遠行 v5，ROADMAP 761）：鎖內收集「某居民遠行歸來、要把邊陲群系的當地風物
    // 種在家門前紀念花圃」的請求；種植座標算、deltas 寫、廣播、持久化、記憶與 Feed 全在鎖釋放後統一
    // 處理（比照營火）。格式：(居民 id, 居民名, 家 x, 家 z, 邊陲群系, 方位名)。
    let mut expedition_keepsakes: Vec<(String, &'static str, f32, f32, crate::voxel::VoxelBiome, String)> =
        Vec::new();

    // 登門串門子抵達事件（登門串門子 v1，ROADMAP 751）：鎖內偵測「朝聖抵達的其實是某位鄰居的家」，
    // 鎖外統一處理 record_visit（情誼加溫、可能升級）+ 記憶（掛鄰居名下）+ Feed。
    // 格式：(resident_id, resident_name, 被登門的鄰居顯示名)。
    let mut neighbor_visit_arrivals: Vec<(String, &'static str, String)> = Vec::new();

    // 登門遇主人在家·迎客事件（登門遇主人在家 v1，ROADMAP 752）：鎖內偵測「訪客登門抵達時，
    // 那位鄰居正好也在家」，鎖外統一處理主人側的迎客泡泡 + 「在家迎客」記憶 + Feed。
    // 情誼不在此重複記帳（751 抵達時已 record_visit 過這對），只補上當面互動與主人側痕跡。
    // 格式：(訪客顯示名, 被登門的主人顯示名, 泡泡雜湊 pick)。
    let mut hosted_meetings: Vec<(&'static str, String, usize)> = Vec::new();

    // 登門撲空留心意事件（登門撲空留心意 v1，ROADMAP 763）：鎖內偵測「訪客登門抵達時那位鄰居**不在家**」，
    // 鎖外統一把訪客名字塞進那位主人的門口心意佇列（去重＋上限）。格式：(被登門的主人顯示名, 訪客顯示名)。
    let mut calling_cards: Vec<(String, &'static str)> = Vec::new();
    // 主人回家感應到門口心意事件（ROADMAP 763）：鎖內偵測「主人回到自家附近、閒著、感應到一張心意」，
    // 鎖外統一處理主人側記憶（掛訪客名下）＋ Feed。格式：(主人 id, 主人顯示名, 那位訪客顯示名)。
    let mut callingcard_notices: Vec<(String, &'static str, String)> = Vec::new();
    // 你送的食物她會細細享用事件（ROADMAP 765）：鎖內偵測「居民捧著的食物延遲到期、此刻閒下來，
    // 真的享用了那份心意」（泡泡＋心情補助已於鎖內設好），鎖外統一補一則城鎮動態 Feed。
    // 格式：(居民顯示名, 送禮玩家名, 食物顯示名)。
    let mut savor_feeds: Vec<(&'static str, String, &'static str)> = Vec::new();
    // 自我印象 v1（ROADMAP 770）：鎖內偵測「居民閒下來、回望自己昇華出一句自我印象」（泡泡已於鎖內設好），
    // 鎖外統一補一則城鎮動態 Feed（第三人稱旁白，已是純模板、無記憶原文）。
    let mut self_image_feeds: Vec<(&'static str, String)> = Vec::new();
    // 居民為你取一個名號 v1：鎖內打招呼時偵測「第一次為某玩家安下名號 / 名號改換」（名號招呼已於
    // 鎖內設好 r.say），鎖外統一補一則城鎮動態。格式：(居民顯示名, 旁白句)。
    let mut epithet_feeds: Vec<(&'static str, String)> = Vec::new();
    // 名號立牌 v1（自主提案）：鎖內偵測「居民**第一次**為某玩家安下名號」→ 鎖外在牠自家門旁刻一塊
    // 名號榮譽牌（走既有 Sign 管線），把口說的名號實體化成世界裡的永久印記。收集
    // (居民 id, 居民顯示名, 家 x, 家 z, 玩家顯示名, 名號角色)；鎖外去重（掃 SignStore，重啟安全）後立牌。
    let mut epithet_sign_reqs: Vec<(&'static str, f32, f32, String, vepi::PlayerRole)> = Vec::new();
    // 自我印象 v2（ROADMAP 771）：鎖內偵測「居民回望自己、且此刻心中沒有心願」→ 讓這份自我理解
    // 化為一個呼應自己的自發心願（鎖外套用，守鎖序）。格式：(居民 id, 居民顯示名, 心願字串)。
    let mut self_aspiration_sparks: Vec<(String, &'static str, &'static str)> = Vec::new();

    // 就寢反思 Feed 事件（作息·就寢反思 v1，ROADMAP 744）：鎖內入睡時偵測，鎖外記 Feed。
    // 格式：(resident_name, 今天回味的記憶摘要)。
    let mut bedtime_feed: Vec<(&'static str, String)> = Vec::new();

    // 晨間探友 Feed 事件（作息 × 記憶驅動行為·晨間探友 v1，ROADMAP 745）：鎖內醒來時偵測，鎖外記 Feed。
    // 格式：(resident_name, 昨晚惦記、一早去找的那位居民名字)。
    let mut morning_feed: Vec<(&'static str, &'static str)> = Vec::new();

    // 晨間思念玩家 v1（作息 × 記憶驅動行為，ROADMAP 746）：鎖內醒來時偵測「昨晚惦記的是位在線玩家」
    // → 記 Feed（鎖外）。格式：(resident_name, 玩家顯示名)。
    let mut daybreak_feed: Vec<(&'static str, String)> = Vec::new();
    // 晨間思念玩家抵達事件（ROADMAP 746）：鎖內偵測「走到玩家面前」，鎖外補記憶（掛玩家名下、算情誼）。
    // 格式：(resident_id, resident_name, 玩家顯示名)。
    let mut daybreak_arrivals: Vec<(String, &'static str, String)> = Vec::new();
    // 此刻在線玩家的顯示名（與 player_pts 索引對齊）：供晨間思念玩家偵測反思裡惦記到誰。
    let player_names: Vec<&str> = player_pts.iter().map(|(_, _, n)| n.as_str()).collect();

    // 久別重逢奔迎抵達事件（記憶驅動·久別重逢奔迎 v1，ROADMAP 747）：鎖內偵測「奔到歸來玩家面前」，
    // 鎖外記 Feed + 補一筆重逢記憶（掛玩家名下、算情誼）。格式：(resident_id, resident_name, 玩家顯示名)。
    let mut reunion_arrivals: Vec<(String, &'static str, String)> = Vec::new();

    // 居民回禮事件（ROADMAP 667）：鎖內偵測，鎖外執行（加入背包 + 廣播）。
    // 格式：(resident_id, resident_name, player_name, block_id, qty, message)
    let mut return_gift_events: Vec<(String, &'static str, String, u8, u32, String)> = Vec::new();

    // 收成回贈事件（ROADMAP 755）：鎖內偵測「那畦因你而生的田熟了、種它的居民遇到送種子的你」，
    // 鎖外執行（收成方塊→FarmSoil + 廣播 + 果實入你背包 + 移除這畦 + 持久化 + Feed）。
    // 格式：(resident_id, resident_name, player_name, pos_key, gx, gy, gz, crop_code)
    let mut harvest_return_events: Vec<(String, &'static str, String, String, i32, i32, i32, u8)> =
        Vec::new();
    // 失效的禮物菜園座標鍵（作物被玩家自己收成／破壞了）：鎖外誠實清帳（移除＋持久化）。
    let mut giftgarden_stale: Vec<String> = Vec::new();
    // no-op 世界不白鎖：一畦禮物菜園都沒有就整段功能早退（絕大多數 tick 走這條）。
    let any_gift_gardens = !hub().giftgarden.read().unwrap().is_empty();

    // 探訪 v1（ROADMAP 671）抵達 / 返家 Feed 事件（鎖內偵測，鎖外 IO）。
    // 格式：(visitor_name, host_name, is_return)
    let mut visit_events: Vec<(&'static str, String, bool)> = Vec::new();

    // 打氣 v1（ROADMAP 679）到達事件（鎖內偵測，鎖外補記憶+Feed）。
    // 格式：(happy_id, happy_name, lonely_rid)
    let mut cheer_arrive_pending: Vec<(String, &'static str, String)> = Vec::new();
    // 打氣到達完成事件（鎖內處理，鎖外寫記憶+Feed）。
    // 格式：(happy_id, happy_name, lonely_rid, lonely_name)
    let mut cheer_arrive_done: Vec<(String, &'static str, String, &'static str)> = Vec::new();

    // 小圈子聚會 v1（ROADMAP 711）全員抵達事件（鎖內偵測，鎖外寫記憶+Feed）。
    // 格式：members = 這場聚會的 (居民 id, 居民名字) 列表。
    let mut clique_fire_events: Vec<Vec<(String, &'static str)>> = Vec::new();

    // 居民情誼 v1（ROADMAP 672）：到達/離開事件（鎖內偵測，鎖外更新情誼+生成問候語）。
    // 抵達格式：(visitor_id, visitor_name, host_name)
    let mut bond_arrive_events: Vec<(String, &'static str, String)> = Vec::new();
    // 離開格式：(visitor_id, visitor_name, host_name)
    let mut bond_depart_events: Vec<(String, &'static str, String)> = Vec::new();

    // 跑腿採集 v1（指令→任務第三刀）：還沒鎖定資源目標的居民（鎖內偵測，鎖外找資源指派）。
    // 格式：(rid, rx, ry, rz, resource)
    let mut fetch_search_candidates: Vec<(String, i32, i32, i32, vskill::GatherResource)> = Vec::new();
    // 跑腿採集交付事件（鎖內偵測抵達，鎖外進玩家背包+記憶+Feed+廣播）。
    // 格式：(rid, rname, requester, resource, delivered, requested)
    let mut fetch_deliver_events: Vec<(String, &'static str, String, vskill::GatherResource, u32, u32)> =
        Vec::new();

    {
        let world = hub().deltas.read().unwrap();
        let mut residents = hub().residents.write().unwrap();

        // 2a) 套用決策：MoveTo 夾成本地閒晃目標；say 非空 → 冒泡（其餘 action 不打斷閒晃）。
        for (rid, dec) in &decisions {
            if let Some(r) = residents.iter_mut().find(|r| &r.id == rid) {
                // 整地任務中：不套用 MoveTo（她專心走向工地，別被思考決策拉走）。
                if !directed_snaps.contains_key(rid) {
                    if let AgentAction::MoveTo { x, y } = dec.action {
                        let dx = x - r.body.x;
                        let dz = y - r.body.z;
                        let d = (dx * dx + dz * dz).sqrt().max(0.001);
                        let cap = BRAIN_MOVE_CAP.min(d);
                        r.target_x = r.body.x + dx / d * cap;
                        r.target_z = r.body.z + dz / d * cap;
                        r.wait_timer = 0.0;
                    }
                }
                let say = dec.say.trim();
                if !say.is_empty() {
                    r.say = say.chars().take(40).collect();
                    r.say_timer = SAY_SECS;
                }
            }
        }

        // 晨間探友 v1（ROADMAP 745）：入迴圈前先快照所有居民的名字與家域中心（兩陣列索引對齊），
        // 供醒來時「讀昨晚反思 → 若惦記到某位居民 → 走去找他」查得到對方家在哪（避免在持 residents
        // 寫鎖的迴圈裡再借用 residents 讀）。名字與家域在本 tick 內不變，快照安全。
        let resident_names: Vec<&'static str> = residents.iter().map(|r| r.name).collect();
        let resident_homes: Vec<(f32, f32)> =
            residents.iter().map(|r| (r.home_x, r.home_z)).collect();
        // 登門遇主人在家 v1（ROADMAP 752）：再快照所有居民的**當前座標**（與 resident_names 索引對齊），
        // 供「訪客抵達鄰居家牌時，判斷那位鄰居此刻是否正好在家（站在自家牌子附近）」——
        // 避免在持 residents 寫鎖的迴圈裡再借用 residents 讀。座標在本 tick 內幾乎不變，快照安全。
        let resident_pos: Vec<(f32, f32)> =
            residents.iter().map(|r| (r.body.x, r.body.z)).collect();

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

            // 心情自語冷卻倒數（ROADMAP 677）：到期後才可自發冒泡。
            if r.mood_say_cooldown > 0.0 {
                r.mood_say_cooldown -= dt;
            }

            // 孤獨尋伴冷卻倒數（ROADMAP 678）。
            if r.seek_comfort_cooldown > 0.0 {
                r.seek_comfort_cooldown -= dt;
            }
            // 名號化為敬意冷卻倒數（ROADMAP 777）。
            if r.esteem_approach_cooldown > 0.0 {
                r.esteem_approach_cooldown -= dt;
            }

            // 打氣冷卻倒數（ROADMAP 679）。
            if r.cheer_cooldown > 0.0 {
                r.cheer_cooldown -= dt;
            }

            // 小圈子聚會冷卻倒數（ROADMAP 711）。
            if r.clique_cooldown > 0.0 {
                r.clique_cooldown -= dt;
            }

            // 技能發明冷卻倒數 + 進行中計畫的逾時倒數（真進化第一刀）。
            // 逾時判定與收尾在 agency 段（advance_invent_run），這裡只負責時間流逝。
            if r.invent_cooldown > 0.0 {
                r.invent_cooldown -= dt;
            }
            if let Some(run) = &mut r.invent_run {
                run.deadline -= dt;
            }
            // 好奇心計時倒數（北極星第三刀）：觸發與種心願在 agency 段（build 候選迴圈），
            // 這裡只負責時間流逝。
            if r.curiosity_timer > 0.0 {
                r.curiosity_timer -= dt;
            }
            // 退避計時倒數（#972 防鬼打牆）：到期的目標移除，好奇心下輪可再試。
            r.invent_backoff.retain(|_, secs| {
                *secs -= dt;
                *secs > 0.0
            });
            // 聚會逾時放棄：等太久等不到其他成員到齊，就散去、回到平常閒晃。
            if r.clique_meet.is_some() {
                r.clique_wait += dt;
                if r.clique_wait > vclique::GATHER_MAX_WAIT_SECS {
                    r.clique_meet = None;
                    r.clique_wait = 0.0;
                }
            }

            // 跑腿採集逾時保險（指令→任務第三刀）：整趟任務（找+走+挖+送）太久沒完成就收工——
            // 身上已經帶著幾份就強制進入交付階段（帶著現有的先回去交差，誠實不硬撐）；
            // 一份都沒採到就老實放棄整個任務，不無窮重試。
            let mut abandon_fetch: Option<vskill::GatherResource> = None;
            if let Some(fetch) = r.fetch.as_mut() {
                fetch.deadline -= dt;
                if fetch.deadline <= 0.0 && fetch.remaining > 0 {
                    if fetch.carried > 0 {
                        fetch.remaining = 0;
                    } else {
                        abandon_fetch = Some(fetch.resource);
                    }
                }
            }
            if let Some(resource) = abandon_fetch {
                r.fetch = None;
                r.gather = None;
                if r.say.is_empty() {
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    r.say = vfetch::fail_line(resource.display_name(), pick).chars().take(40).collect();
                    r.say_timer = SAY_SECS;
                }
            }

            // 心情補助倒數（ROADMAP 681）：互動帶來的暖意隨時間消退。
            if r.mood_boost_secs > 0.0 {
                r.mood_boost_secs -= dt;
            }

            // 旁聽搭話冷卻倒數（embodied 靠近說話 v1）：到期後才可再因旁聽搭話。
            if r.overhear_cooldown > 0.0 {
                r.overhear_cooldown -= dt;
            }

            // 日夜作息·睡覺 v1（ROADMAP 739）：睡著中的居民只做兩件事——天亮就醒、
            // 否則安靜地待在家落重力躺著，跳過本 tick 一切社交／採集／建造／閒晃。
            // （入睡的判定在下方閒晃區塊「抵達自家」時觸發。）
            if r.asleep {
                if vsleep::should_wake(is_night, r.asleep) {
                    // 天色轉亮 → 醒來：睡飽神清氣爽（心情提升一格）、冒一句愉快早安。
                    r.asleep = false;
                    r.mood_boost_secs = r.mood_boost_secs.max(vsleep::WAKE_MOOD_BOOST_SECS);
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    // 邊陲過夜 v4（ROADMAP 759）：分岔——是在邊陲第二個家過夜醒來、還是在主城的家醒來。
                    let woke_at_outpost = r.asleep_at_outpost;
                    if woke_at_outpost {
                        // 在邊陲營地那張床上睡飽了：結束這趟遠行、啟程返家（返程移動交給下方 wander，
                        // 此刻遠在家域外，`wander_center` 會把牠一路帶回主城）。把「過了一夜」昇華成一筆
                        // 記憶（掛遠行哨兵鍵），記一則返家 Feed。跳過家用晨間探友（她不在主城的家）。
                        r.asleep_at_outpost = false;
                        let bearing = r
                            .expedition
                            .as_ref()
                            .map(|(.., b)| b.clone())
                            .unwrap_or_default();
                        r.expedition = None;
                        r.expedition_stay = 0.0;
                        r.expedition_cooldown = vexp::EXPEDITION_COOLDOWN;
                        r.say = vexp::outpost_wake_bubble(&bearing, pick);
                        r.say_timer = SAY_SECS;
                        // 記憶：memory 短寫即釋（比照抵達昇華，不巢狀）。
                        let summary = vexp::outpost_sleep_memory_summary(&bearing);
                        let entry = hub().memory.write().unwrap().add_memory(
                            &r.id,
                            vexp::EXPEDITION_MEMORY_PLAYER,
                            &summary,
                        );
                        vmem::append_memory(&entry);
                        expedition_feed.push((r.name, vexp::outpost_wake_feed_line(&bearing)));
                    } else {
                        r.say = vsleep::wake_line(pick).to_string();
                        r.say_timer = SAY_SECS;
                    }
                    // 晨間探友 v1（ROADMAP 745）：記憶驅動行為——醒來讀昨晚昇華的「睡前反思」記憶
                    // （744），若那份牽掛裡有另一位居民的名字，今天第一件事就是走去找他（沿用探訪
                    // 狀態機的抵達／問候／情誼／Feed，零協議改動）。鎖序：memory 短讀鎖即釋、不巢狀；
                    // Feed 走鎖外 morning_feed。剛醒不會在探訪／聚會中，但仍保守判斷不覆蓋既有意圖。
                    // 邊陲過夜醒來（`woke_at_outpost`）跳過此段：她此刻遠在荒野邊陲、不在主城的家。
                    if !woke_at_outpost
                        && r.visiting.is_none()
                        && r.clique_meet.is_none()
                        && vmorning::should_seek(true, rand::random::<f32>())
                    {
                        // 讀最近一筆睡前反思記憶（744 昇華、掛 REFLECT_MEMORY_PLAYER 標籤，最新在前）。
                        let reflection = {
                            let mem = hub().memory.read().unwrap();
                            mem.all_memories_for(&r.id)
                                .into_iter()
                                .find(|e| e.player == vbedtime::REFLECT_MEMORY_PLAYER)
                                .map(|e| e.summary)
                        }; // 記憶讀鎖釋放
                        if let Some(summary) = reflection {
                            if let Some(idx) =
                                vmorning::mentioned_resident(&summary, &resident_names, r.name)
                            {
                                let friend = resident_names[idx];
                                let (fx, fz) = resident_homes[idx];
                                // 動身：把探訪目標設成對方家域，今天第一件事就往那走。
                                r.visiting = Some((fx, fz, friend.to_string()));
                                r.visit_stay_timer = 0.0;
                                r.visit_cooldown = vvisit::VISIT_COOLDOWN_SECS;
                                r.target_x = fx;
                                r.target_z = fz;
                                r.say = vmorning::seek_bubble(friend, pick);
                                r.say_timer = SAY_SECS;
                                morning_feed.push((r.name, friend));
                            } else if vdaybreak::should_miss(true, rand::random::<f32>()) {
                                // 晨間思念玩家 v1（ROADMAP 746）：昨晚惦記的不是居民、而是某位此刻在線
                                // 玩家 → 醒來朝他走過去打招呼（745 的對稱補完：記憶讓居民來找「你」）。
                                // 只比對在線玩家名（player_names），離線者不在名單、絕不誤中。
                                if let Some(pidx) =
                                    vdaybreak::mentioned_player(&summary, &player_names)
                                {
                                    let (px, pz, pname) = &player_pts[pidx];
                                    r.daybreak_seek =
                                        Some((pname.clone(), vdaybreak::SEEK_TIMEOUT_SECS));
                                    r.target_x = *px;
                                    r.target_z = *pz;
                                    r.wait_timer = 0.0;
                                    r.say = vdaybreak::wake_bubble(pname, pick);
                                    r.say_timer = SAY_SECS;
                                    daybreak_feed.push((r.name, pname.clone()));
                                }
                            }
                        }
                    }
                    // 不 continue：醒來後這一 tick 就讓她照常展開新的一天。
                } else {
                    // 仍是夜裡：安靜睡著，只落重力、不做任何行為。
                    vr::gravity_step(&world, &mut r.body, dt);
                    continue;
                }
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

            // 主動招呼：招呼冷卻倒數；冷卻完、目前沒在說話、非尋伴狀態、且有玩家靠近時，
            // 偶爾（低機率）冒一句招呼，讓世界更有人氣（用既有泡泡、低頻不洗版）。
            // 好感度 v1：查玩家記憶筆數 → 決定招呼溫度（陌生人/相識/友人，零 LLM）。
            // 老友情境問候 v1（ROADMAP 675）：好感 ≥ FOND_AFFINITY 時，改用記憶驅動的特定台詞。
            // 尋伴時不走普通招呼（ROADMAP 678）：等抵達玩家旁才冒求陪泡泡。
            if r.greet_timer > 0.0 {
                r.greet_timer -= dt;
            } else if r.say.is_empty() && !r.seeking_comfort && r.approaching_esteem.is_none() {
                if let Some((d2, nearest_name)) = nearest_player_info(r.body.x, r.body.z, &player_pts) {
                    if d2 < GREET_DIST * GREET_DIST && rand::random::<f32>() < GREET_CHANCE_PER_TICK {
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                        // 短鎖讀好感度 + 必要時取「關於這位玩家」的全部記憶（一次鎖即釋，不巢狀）。
                        // 老友門檻以上才撈全記憶（供名號昇華＋老友情境偵測），陌生人不白撈。
                        let (affinity, pmems): (usize, Vec<crate::voxel_memory::MemoryEntry>) = {
                            let mem = hub().memory.read().unwrap();
                            let aff = mem.affinity_count(nearest_name, &r.id);
                            let ms = if aff >= vfond::FOND_AFFINITY {
                                mem.all_player_memories(&r.id, nearest_name)
                            } else {
                                Vec::new()
                            };
                            (aff, ms)
                        }; // 記憶讀鎖在此釋放
                        let line = if affinity >= vfond::FOND_AFFINITY && !nearest_name.is_empty() {
                            // 居民為你取一個名號 v1：先看牠是否已從「關於你的全部作為」昇華出一個**明顯
                            // 主導的角色**（造物者／慷慨的人／老搭檔／常來的老友）。有的話——你不再只是
                            // 一個名字，而是牠心中一個掙來的名號，牠改用名號稱呼你。這是 770 自我印象
                            // 的對外鏡像：世界如何看你。與底下 fond 老友問候（回憶單次互動）刻意區隔：
                            // 名號要「某類作為持續主導」才昇華，著眼「你是誰」而非「我們做過什麼」。
                            if let Some(role) = vepi::dominant_role(&pmems) {
                                // 第一次為你安下名號 / 名號改換的那一刻 → 記一則城鎮動態（鎖外 flush）。
                                if r.coined_epithets.get(nearest_name) != Some(&role) {
                                    // 名號立牌 v1：只在**這位居民從未為這位玩家安過任何名號**（真正第一次）
                                    // 時，收集一筆「在自家門旁刻名號牌」請求——名號改換不重刻（一塊足矣）。
                                    // 重啟後 coined_epithets 歸零會讓此條件再度成立，但鎖外立牌會先掃
                                    // SignStore 去重（既有牌還在），不會重複立牌。
                                    if !r.coined_epithets.contains_key(nearest_name) {
                                        epithet_sign_reqs.push((
                                            r.name,
                                            r.home_x,
                                            r.home_z,
                                            nearest_name.to_string(),
                                            role,
                                        ));
                                    }
                                    epithet_feeds.push((r.name, vepi::coined_feed_line(nearest_name, role)));
                                    r.coined_epithets.insert(nearest_name.to_string(), role);
                                }
                                vepi::greeting_for_role(role, pick)
                            } else {
                                // 還昇華不出明顯主導名號 → 落回老友情境問候：依最近 4 筆摘要偵測情境。
                                let summaries: Vec<String> =
                                    pmems.iter().take(4).map(|e| e.summary.clone()).collect();
                                let ctx = vfond::detect_context(&summaries);
                                vfond::fond_greeting_line(nearest_name, &ctx, pick)
                            }
                        } else if !nearest_name.is_empty() {
                            // 名號口耳相傳 v1（自主提案）：這位居民其實還沒跟你深交（affinity 不到
                            // 老友門檻、昇華不出自己的第一手名號），但先前有相熟的老朋友來訪時說起過
                            // 你——她心裡記著這個聽來的名號。頭一次撞見你，就用一句「久仰」的傳聞招呼
                            // 喊你。你的名聲第一次透過小社會朋友網絡自己傳開了。若日後她真跟你處成
                            // 老友、自己昇華出名號，上面 affinity≥FOND 分支的第一手名號自然接管。
                            if let Some(hs) = r.heard_epithets.get(nearest_name) {
                                vespread::hearsay_greeting_line(hs, pick)
                            } else {
                                greeting_line_affinity(affinity, nearest_name, pick)
                            }
                        } else {
                            greeting_line_affinity(affinity, nearest_name, pick)
                        };
                        r.say = line.chars().take(40).collect();
                        r.say_timer = SAY_SECS;
                        r.greet_timer = GREET_COOLDOWN;
                    }
                }
            }

            // 讀牌 v1（居民讀牌）：冷卻完、目前沒在說話、非尋伴狀態時，低機率偵測 READ_RANGE 內
            // 是否有玩家立的告示牌（740）；有的話停下念出牌面 + 依語氣回應一句——玩家親手寫的字
            // 第一次被 AI 居民「看見」。先擲骰再上鎖（no-sign 世界不白鎖）；短鎖讀 sign store 取
            // 最近牌面即釋（不巢狀、不持鎖 await），守死鎖鐵律。零 LLM、零持久化。
            if r.read_sign_timer > 0.0 {
                r.read_sign_timer -= dt;
            } else if r.say.is_empty()
                && !r.seeking_comfort
                && rand::random::<f32>() < vreadsign::READ_CHANCE_PER_TICK
            {
                let nearby = hub()
                    .sign
                    .read()
                    .unwrap()
                    .nearest_within_xz(r.body.x, r.body.z, vreadsign::READ_RANGE); // sign 讀鎖在此釋放
                if let Some((sx, sz, text, _)) = nearby.filter(|(_, _, t, _)| !t.is_empty()) {
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    let quote = vreadsign::display_quote(&text);
                    // 居民認得鄰居的家 v1（ROADMAP 750）：先看這塊牌是不是**別的居民**立的自建
                    // 銘牌（749，格式「{名}的{建物}」）。是的話認出鄰居、念一句更親暱的招呼；
                    // 否則落回既有的世界級讀牌路徑（741/742/743），行為完全不變。
                    let neighbor = vneighsign::identify_nameplate(&text, r.name, &RESIDENT_NAMES);
                    r.say = match neighbor {
                        Some(nb) => vneighsign::neighbor_sign_line(nb, &quote, pick),
                        None => vreadsign::read_sign_line(&text, pick),
                    };
                    r.say_timer = SAY_SECS;
                    r.read_sign_timer = vreadsign::READ_COOLDOWN;
                    // 居民讀牌 v2：讀到的牌在心裡留下印象——只有讀到「不同於上次」的牌才寫一筆
                    // 記憶（避免反覆讀同一塊牌塞滿 episodic）。
                    // 短鎖：add_memory 的寫鎖在該語句結束即釋，append_memory 的 IO 在鎖外進行
                    //（守死鎖鐵律：記憶讀寫不在持鎖中 await）。
                    if r.last_read_sign.as_deref() != Some(text.as_str()) {
                        r.last_read_sign = Some(text.clone());
                        let entry = match neighbor {
                            // 750：認出是某位鄰居的牌 → 記憶**掛在那位鄰居名下**（不再落到世界級
                            // 哨兵鍵），讀牌記憶第一次連到「某個具體的鄰居」；日後回想／日記可引用。
                            Some(nb) => {
                                let mem = vneighsign::neighbor_sign_memory(nb, &quote);
                                let e = hub().memory.write().unwrap().add_memory(&r.id, nb, &mem);
                                // 城鎮動態 Feed：某居民認出了某鄰居的住處（鄰里認知第一次浮上檯面）。
                                vfeed::append_feed(
                                    "鄰里認家",
                                    r.name,
                                    &vneighsign::neighbor_sign_feed(r.name, nb),
                                );
                                e
                            }
                            // 既有路徑：玩家寫的字，掛世界級哨兵鍵，不污染真實玩家好感。
                            None => {
                                let summary = vreadsign::sign_memory_summary(&text);
                                hub().memory.write().unwrap().add_memory(
                                    &r.id,
                                    vreadsign::SIGN_MEMORY_PLAYER,
                                    &summary,
                                )
                            }
                        };
                        vmem::append_memory(&entry);
                        // 居民讀牌 v3（ROADMAP 743）：把這塊「不同於上次」的牌記成心中的地標，
                        // 日後閒暇時偶爾會特地走回來駐足——讀牌記憶第一次改變居民的去向。
                        // 鄰居的家牌同樣成地標：居民日後可能特地晃回鄰居家門口（複用 743 朝聖）。
                        r.cherished_sign = Some((sx, sz, quote));
                        // 登門串門子 v1（ROADMAP 751）：同步記下「這塊地標其實是哪位鄰居的家」——
                        // 是鄰居家牌就存鄰居名，是玩家的牌就清成 None，讓日後朝聖抵達能把「重返」
                        // 升級成一次真正的「登門拜訪」（與 cherished_sign 同一處更新、恆保持一致）。
                        r.cherished_neighbor = neighbor.map(|s| s.to_string());
                    }
                }
            }

            // 照料菜園 v1（ROADMAP 753）：對你有好感（affinity ≥ FOND_AFFINITY）的居民，路過你
            // 種下、還沒成熟的作物旁時，偶爾停下來順手幫忙照料——把作物的生長往前推進一小段
            // （但永不揠苗助長到瞬間成熟，留最後一小段讓你親眼看它長好），冒句話、記進記憶、Feed
            // 播報。人類種田的樂趣第一次與 AI 居民的好感連成一線：你種下、喜歡你的居民幫你顧。
            // 先擲骰（no-op 世界不白鎖）→ 短鎖讀好感 → 短鎖查附近未熟作物 → 短鎖 nudge，皆即釋
            // 不巢狀，守死鎖鐵律（比照讀牌 v2 的 add_memory 手法）。零 LLM、零 migration。
            if r.tend_timer > 0.0 {
                r.tend_timer -= dt;
            } else if r.say.is_empty()
                && !r.seeking_comfort
                && rand::random::<f32>() < vtend::TEND_CHANCE_PER_TICK
            {
                if let Some((d2, nearest_name)) = nearest_player_info(r.body.x, r.body.z, &player_pts)
                {
                    // 只有喜歡你、且你就在旁邊的居民才會特地幫你顧菜園（記憶驅動能動性）。
                    let is_fond = d2 < GREET_DIST * GREET_DIST
                        && !nearest_name.is_empty()
                        && hub().memory.read().unwrap().affinity_count(nearest_name, &r.id)
                            >= vfond::FOND_AFFINITY; // 記憶讀鎖即釋
                    if is_fond {
                        let now = vfarm::now_secs();
                        // 短讀鎖 clone delta 快照判水耕（只在通過好感閘後才做，罕見路徑），即釋。
                        let deltas_snap = hub().deltas.read().unwrap().clone();
                        let raining = *hub().weather.read().unwrap(); // 短讀鎖即釋
                        let candidate = hub().farm.read().unwrap().nearest_immature_plot_near(
                            r.body.x,
                            r.body.z,
                            vtend::CARE_DIST,
                            now,
                            |fx, fy, fz| raining || is_irrigated_in_delta(&deltas_snap, fx, fy, fz),
                        ); // farm 讀鎖即釋
                        if let Some(((cx, cy, cz), kind, remaining)) = candidate {
                            let nudge = vtend::nudge_amount(remaining);
                            if nudge > 0 {
                                // farm 寫鎖即釋、append 在 farm 鎖外（比照同區塊 append_memory/append_feed，
                                // 皆小檔同步 IO、不巢狀 farm/memory 鎖、非 await）——照料進度也持久化。
                                let farm_e =
                                    { hub().farm.write().unwrap().nudge_growth(cx, cy, cz, nudge) };
                                if let Some(farm_e) = farm_e {
                                    vfarm::append_farm(&farm_e);
                                }
                                let crop = vtend::crop_name(kind);
                                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                                r.say = vtend::tend_say_line(crop, pick).chars().take(40).collect();
                                r.say_timer = SAY_SECS;
                                r.tend_timer = vtend::TEND_COOLDOWN;
                                // 記進記憶（掛在該玩家名下，好感自然累積——你的善意有了回報）；
                                // add_memory 寫鎖即釋、append_memory 的 IO 在鎖外（守死鎖鐵律）。
                                let summary = vtend::tend_memory_line(nearest_name, crop);
                                let entry = hub()
                                    .memory
                                    .write()
                                    .unwrap()
                                    .add_memory(&r.id, nearest_name, &summary);
                                vmem::append_memory(&entry);
                                // Feed 播報，讓不在場的訪客回來也讀得到這份溫柔。
                                vfeed::append_feed(
                                    vtend::FEED_KIND,
                                    r.name,
                                    &vtend::tend_feed_line(r.name, nearest_name, crop),
                                );
                            }
                        }
                    }
                }
            }

            // 收成回贈 v1（ROADMAP 755）：那畦「因你的種子而生」的田（754）熟了，種它的居民
            // 遇到送種子的你，會親手收成、把第一把收穫當面回贈給你——你的餽贈在世界裡走了一整圈
            // 才回來，且回來時已是它結的果，而非你送出去的那把種子。層層過閘（機率節流→玩家在旁
            // →真有一畦掛她名下、且是眼前這位玩家送的→已成熟）才觸發；罕見路徑才做 delta 讀。
            // 鎖序：giftgarden 讀（即釋）→ delta 讀快照判成熟（即釋）→ 鎖內只改 say/記憶；
            // 收成方塊 / 果實入背包 / 移除這畦 / 持久化 / Feed 全走鎖外事件（守死鎖鐵律）。
            if any_gift_gardens
                && r.say.is_empty()
                && !r.seeking_comfort
                && rand::random::<f32>() < vgg::HARVEST_CHANCE_PER_TICK
            {
                if let Some((d2, nearest_name)) =
                    nearest_player_info(r.body.x, r.body.z, &player_pts)
                {
                    if d2 < GREET_DIST * GREET_DIST && !nearest_name.is_empty() {
                        // 撈這位居民名下、且是眼前這位玩家送的種子長成的田（讀鎖即釋）。
                        let plots =
                            hub().giftgarden.read().unwrap().plots_for(&r.id, nearest_name);
                        if !plots.is_empty() {
                            // 短讀鎖 clone delta 快照判每畦作物方塊現況（即釋）。
                            let snap = hub().deltas.read().unwrap().clone();
                            let mut ready: Option<(String, i32, i32, i32, u8)> = None;
                            for (pos, crop) in plots {
                                let Some((gx, gy, gz)) = vgg::parse_key(&pos) else { continue };
                                let (mature, seeded) = match crop {
                                    vgg::CROP_WHEAT => {
                                        (Block::WheatMature, Block::FarmSoilSeeded)
                                    }
                                    vgg::CROP_CARROT => {
                                        (Block::CarrotMature, Block::CarrotSeeded)
                                    }
                                    vgg::CROP_POTATO => {
                                        (Block::PotatoMature, Block::PotatoSeeded)
                                    }
                                    _ => continue,
                                };
                                let b = voxel::effective_block_at(&snap, gx, gy, gz);
                                if b == mature {
                                    // 挑第一畦熟的收（每 tick 只收一畦，不洗版）。
                                    if ready.is_none() {
                                        ready = Some((pos, gx, gy, gz, crop));
                                    }
                                } else if b != seeded {
                                    // 作物已不在（被玩家自己收成／破壞）→ 這畦失效，
                                    // 標記移除（誠實清帳，鎖外套用）。
                                    giftgarden_stale.push(pos);
                                }
                            }
                            if let Some((pos, gx, gy, gz, crop)) = ready {
                                let cname = vgg::crop_name(crop);
                                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                                r.say = vgg::harvest_say_line(nearest_name, cname, pick)
                                    .chars()
                                    .take(40)
                                    .collect();
                                r.say_timer = SAY_SECS;
                                // 記進記憶（掛玩家名下，情誼再加溫——你的餽贈結了果、又回到你手裡）。
                                let summary = vgg::harvest_memory_line(nearest_name, cname);
                                let entry = hub()
                                    .memory
                                    .write()
                                    .unwrap()
                                    .add_memory(&r.id, nearest_name, &summary);
                                vmem::append_memory(&entry);
                                harvest_return_events.push((
                                    r.id.clone(),
                                    r.name,
                                    nearest_name.to_string(),
                                    pos,
                                    gx,
                                    gy,
                                    gz,
                                    crop,
                                ));
                            }
                        }
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

            // 孤獨尋伴 v1（ROADMAP 678）：Lonely 心情到期後走向最近玩家；到了冒求陪泡泡。
            // 鎖序：bonds 讀（即釋）→ memory 讀（即釋），不巢狀，不持鎖 await。
            if r.seeking_comfort {
                // 繼續更新目標（玩家可能在移動）或放棄（玩家走太遠 / 無玩家在線）。
                match nearest_player_with_pos(r.body.x, r.body.z, &player_pts) {
                    Some((d2, px, pz, _)) if d2 < vcomfort::SEEK_RANGE * vcomfort::SEEK_RANGE => {
                        r.target_x = px;
                        r.target_z = pz;
                        r.wait_timer = 0.0;
                    }
                    _ => {
                        // 玩家太遠或不在線，放棄尋伴。
                        r.seeking_comfort = false;
                        r.seek_comfort_cooldown = vcomfort::SEEK_COMFORT_COOLDOWN;
                    }
                }
            } else if r.seek_comfort_cooldown <= 0.0 && r.say.is_empty() && r.expedition.is_none() {
                // 觸發尋伴：只有 Lonely 心情才走。
                let (friends, acq) = {
                    let bonds = hub().bonds.read().unwrap();
                    resident_bond_counts(&bonds, &r.id)
                }; // bonds 讀鎖釋放
                let mems = {
                    hub().memory.read().unwrap().memory_count(&r.id)
                }; // memory 讀鎖釋放
                // 心情補助（ROADMAP 681）：補助期間心情提升一格，Lonely 可能因此不再尋伴。
                let raw_tier = voxel_mood::compute_mood(friends, acq, mems);
                let effective_tier = if r.mood_boost_secs > 0.0 {
                    voxel_mood::boost_mood(raw_tier)
                } else {
                    raw_tier
                };
                if effective_tier == voxel_mood::MoodTier::Lonely {
                    if let Some((d2, px, pz, _)) =
                        nearest_player_with_pos(r.body.x, r.body.z, &player_pts)
                    {
                        if d2 < vcomfort::SEEK_RANGE * vcomfort::SEEK_RANGE {
                            r.target_x = px;
                            r.target_z = pz;
                            r.wait_timer = 0.0;
                            r.seeking_comfort = true;
                            r.seek_comfort_cooldown = vcomfort::SEEK_COMFORT_COOLDOWN;
                        } else {
                            // 玩家太遠，重置冷卻等下次機會。
                            r.seek_comfort_cooldown = vcomfort::SEEK_COMFORT_COOLDOWN;
                        }
                    } else {
                        // 無在線玩家，重置冷卻。
                        r.seek_comfort_cooldown = vcomfort::SEEK_COMFORT_COOLDOWN;
                    }
                } else {
                    // 心情已不再 Lonely，重置冷卻，不觸發尋伴。
                    r.seek_comfort_cooldown = vcomfort::SEEK_COMFORT_COOLDOWN;
                }
            }
            // 尋伴抵達：已到玩家附近且 say 空 → 冒求陪泡泡（等玩家搭話）。
            if r.seeking_comfort && r.say.is_empty() {
                if let Some((d2, _, _, _)) =
                    nearest_player_with_pos(r.body.x, r.body.z, &player_pts)
                {
                    if d2 < vcomfort::COMFORT_ARRIVE_DIST * vcomfort::COMFORT_ARRIVE_DIST {
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                        r.say = vcomfort::comfort_seek_line(pick).chars().take(40).collect();
                        r.say_timer = SAY_SECS;
                    }
                }
            }

            // 名號化為敬意 v1（ROADMAP 777）：被你贏得名號的居民，看見你在中距離時，偶爾放下閒晃、
            // 特地走過來向你致意——你的名聲第一次改變居民的「行為」。鏡像 678 尋伴的走近機制，但由
            // 「敬重」（`coined_epithets` 已有這位玩家）而非「孤獨」驅動，冷卻更長（不黏人）。
            // 只在非忙碌狀態（未在尋伴/打氣/探訪/遠行/晨思/重逢）才起身，不搶既有意圖；純目標移動，
            // 由既有導航步走過去。零 LLM、零持久化、零新協議欄位。
            if let Some((pname, role)) = r.approaching_esteem.clone() {
                // 續朝玩家更新目標；玩家走太遠 / 離線 / 換了最近的別位玩家 → 放棄本次致意。
                match nearest_player_with_pos(r.body.x, r.body.z, &player_pts) {
                    Some((d2, px, pz, nm))
                        if nm == pname
                            && d2 <= vesteem::ESTEEM_APPROACH_RANGE * vesteem::ESTEEM_APPROACH_RANGE =>
                    {
                        r.target_x = px;
                        r.target_z = pz;
                        r.wait_timer = 0.0;
                        // 抵達面前且沒在說話 → 冒致意泡泡 + 記城鎮動態，清狀態、設冷卻。
                        if d2 < vesteem::ESTEEM_ARRIVE_DIST * vesteem::ESTEEM_ARRIVE_DIST
                            && r.say.is_empty()
                        {
                            let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                            r.say = vesteem::esteem_arrive_line(role, &pname, pick)
                                .chars()
                                .take(40)
                                .collect();
                            r.say_timer = SAY_SECS;
                            epithet_feeds.push((r.name, vesteem::esteem_feed_line(&pname, role)));
                            r.approaching_esteem = None;
                            r.esteem_approach_cooldown = vesteem::ESTEEM_COOLDOWN;
                        }
                    }
                    _ => {
                        r.approaching_esteem = None;
                        r.esteem_approach_cooldown = vesteem::ESTEEM_COOLDOWN;
                    }
                }
            } else if r.esteem_approach_cooldown <= 0.0
                && r.say.is_empty()
                && !r.seeking_comfort
                && r.cheer_target.is_none()
                && r.visiting.is_none()
                && r.expedition.is_none()
                && r.daybreak_seek.is_none()
                && r.reunion_seek.is_none()
                // 也讓位給更高優先的移動意圖（跟隨/採集/取物），否則 flag 會空懸走不到玩家。
                && r.follow.is_none()
                && r.gather.is_none()
                && r.fetch.is_none()
            {
                // 觸發判定：最近的在線玩家若是「這位居民已為他昇華出名號」的人，就低機率起身致意。
                if let Some((d2, px, pz, nm)) =
                    nearest_player_with_pos(r.body.x, r.body.z, &player_pts)
                {
                    if !nm.is_empty() {
                        if let Some(&role) = r.coined_epithets.get(nm) {
                            if vesteem::should_start_approach(
                                d2,
                                r.esteem_approach_cooldown,
                                rand::random::<f32>(),
                            ) {
                                let name = nm.to_string();
                                r.target_x = px;
                                r.target_z = pz;
                                r.wait_timer = 0.0;
                                r.approaching_esteem = Some((name, role));
                            }
                        }
                    }
                }
            }

            // 打氣走動 v1（ROADMAP 679）：cheer_target 有效時，持續朝 Lonely 同伴走過去；
            // 抵達後冒打氣泡泡、收集到達事件（鎖外補記憶 + Feed）、清除任務。
            if let Some((tx, tz, ref lonely_rid)) = r.cheer_target.clone() {
                // 持續更新移動目標（步步逼近），清除小歇。
                r.target_x = tx;
                r.target_z = tz;
                r.wait_timer = 0.0;
                // 到達判定（XZ 距離 < CHEER_ARRIVE_DIST 且 say 空）。
                let dx = r.body.x - tx;
                let dz = r.body.z - tz;
                if dx * dx + dz * dz < vcheer::CHEER_ARRIVE_DIST * vcheer::CHEER_ARRIVE_DIST
                    && r.say.is_empty()
                {
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    r.say = vcheer::cheer_line(pick).chars().take(40).collect::<String>();
                    r.say_timer = SAY_SECS;
                    // 收集到達事件供鎖外 IO（(happy_id, happy_name, lonely_rid)）。
                    cheer_arrive_pending.push((r.id.clone(), r.name, lonely_rid.clone()));
                    r.cheer_target = None; // 任務完成，清除目標。
                }
            }

            // 晨間思念玩家走動 v1（ROADMAP 746）：daybreak_seek 有效時，持續朝那位特定玩家走過去
            // （他可能在移動，故每 tick 依名字重查其座標步步逼近）；抵達（XZ 距離 < ARRIVE_DIST 且
            // say 空）→ 暖暖打招呼、收集抵達事件（鎖外補一筆與他的記憶）、清除任務。玩家離線／逾時
            // 則放下這份牽掛。比照打氣走動 / 朝聖的狀態機（每 tick 覆寫 target、抵達即清空）。
            if let Some((pname, remaining)) = r.daybreak_seek.clone() {
                let remaining = remaining - dt;
                // 依名字重查那位玩家此刻的座標（可能已移動）。
                let here = player_pts
                    .iter()
                    .find(|(_, _, n)| *n == pname)
                    .map(|(x, z, _)| (*x, *z));
                match here {
                    Some((px, pz)) if remaining > 0.0 => {
                        let dx = r.body.x - px;
                        let dz = r.body.z - pz;
                        let arrived =
                            dx * dx + dz * dz < vdaybreak::ARRIVE_DIST * vdaybreak::ARRIVE_DIST;
                        if arrived {
                            // 走到玩家面前：沒在說別的話就暖暖打招呼、記一筆與他的記憶，收工。
                            // 若正巧在說別的話（如剛醒的晨思泡泡），先按住位置等 say 清空再打招呼
                            // （不遞減逾時的殘留、避免站在跟前反被逾時放棄）。
                            if r.say.is_empty() {
                                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                                r.say = vdaybreak::arrive_greet_bubble(&pname, pick);
                                r.say_timer = SAY_SECS;
                                daybreak_arrivals.push((r.id.clone(), r.name, pname.clone()));
                                r.daybreak_seek = None;
                            } else {
                                // 站定等說完話：保持目標、續留這份牽掛（逾時照遞減，不至無限等）。
                                r.target_x = px;
                                r.target_z = pz;
                                r.wait_timer = 0.0;
                                r.daybreak_seek = Some((pname, remaining));
                            }
                        } else {
                            // 還在路上：步步逼近、清小歇，讓玩家看得到牠專程走過來。
                            r.target_x = px;
                            r.target_z = pz;
                            r.wait_timer = 0.0;
                            r.daybreak_seek = Some((pname, remaining));
                        }
                    }
                    // 玩家離線（不在名單）或逾時走太久 → 放下這份牽掛，回到平常的一天。
                    _ => {
                        r.daybreak_seek = None;
                    }
                }
            }

            // 久別重逢奔迎走動 v1（ROADMAP 747）：reunion_seek 有效時，持續朝那位久別歸來的玩家奔過去
            // （他可能在移動，故每 tick 依名字重查其座標步步逼近）；抵達（XZ 距離 < ARRIVE_DIST 且
            // say 空）→ 暖暖迎接、收集抵達事件（鎖外補一筆與他的重逢記憶）、清除任務。玩家離線／逾時
            // 則放下這份心意。狀態機比照晨間思念（746，每 tick 覆寫 target、抵達即清空）。
            if let Some((pname, remaining)) = r.reunion_seek.clone() {
                let remaining = remaining - dt;
                // 依名字重查那位玩家此刻的座標（可能已移動）。
                let here = player_pts
                    .iter()
                    .find(|(_, _, n)| *n == pname)
                    .map(|(x, z, _)| (*x, *z));
                match here {
                    Some((px, pz)) if remaining > 0.0 => {
                        let dx = r.body.x - px;
                        let dz = r.body.z - pz;
                        let arrived =
                            dx * dx + dz * dz < vreunion::ARRIVE_DIST * vreunion::ARRIVE_DIST;
                        if arrived {
                            // 奔到玩家面前：沒在說別的話就暖暖迎接、記一筆重逢記憶，收工。
                            if r.say.is_empty() {
                                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                                r.say = vreunion::rush_greet_bubble(&pname, pick);
                                r.say_timer = SAY_SECS;
                                reunion_arrivals.push((r.id.clone(), r.name, pname.clone()));
                                r.reunion_seek = None;
                            } else {
                                // 站定等說完話：保持目標、續留這份心意（逾時照遞減，不至無限等）。
                                r.target_x = px;
                                r.target_z = pz;
                                r.wait_timer = 0.0;
                                r.reunion_seek = Some((pname, remaining));
                            }
                        } else {
                            // 還在路上：步步逼近、清小歇，讓玩家看得到牠專程奔過來。
                            r.target_x = px;
                            r.target_z = pz;
                            r.wait_timer = 0.0;
                            r.reunion_seek = Some((pname, remaining));
                        }
                    }
                    // 玩家離線（不在名單）或逾時奔太久 → 放下這份心意，回到平常的一天。
                    _ => {
                        r.reunion_seek = None;
                    }
                }
            }

            // 重返心中的牌子 v3（ROADMAP 743）：讀牌記憶第一次改變居民的去向。
            // 冷卻遞減；正在朝聖時持續朝牌子走、抵達即駐足念一句、逾時則放棄。
            // 鎖序：memory 寫（短鎖即釋，比照 v2），Feed 走鎖外 pilgrimage_feed，不巢狀、不持鎖 await。
            if r.pilgrimage_cooldown > 0.0 {
                r.pilgrimage_cooldown -= dt;
            }
            if let Some((tx, tz, quote)) = r.pilgrimage.clone() {
                // 持續朝牌子走、清小歇（步步逼近，讓玩家看得到她專程走過來）。
                r.target_x = tx;
                r.target_z = tz;
                r.wait_timer = 0.0;
                let dx = r.body.x - tx;
                let dz = r.body.z - tz;
                let arrived = dx * dx + dz * dz
                    < vreadsign::PILGRIMAGE_ARRIVE_DIST * vreadsign::PILGRIMAGE_ARRIVE_DIST;
                if arrived {
                    // 抵達且沒在說話：駐足念一句、寫「又回來看看」記憶、Feed 一則，收工設冷卻。
                    // 若正巧在說別的話，就先按住位置等 say 清空（不遞減逾時，避免站在牌前反被逾時放棄）。
                    if r.say.is_empty() {
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                        // 登門串門子 v1（ROADMAP 751）：這趟走向的若其實是某位鄰居親手立的家牌
                        //（750 認得的、啟程時快照進 pilgrimage_neighbor），抵達就不再只是獨自對牌
                        // 懷念，而是當成一次真正的「登門拜訪」——暖暖點名招呼、記憶掛在那位鄰居名下、
                        // 情誼因這趟登門而加溫（鎖外 neighbor_visit_arrivals 統一處理 record_visit）。
                        if let Some(nb) = r.pilgrimage_neighbor.clone() {
                            // 登門遇主人在家 v1（ROADMAP 752）：那位鄰居此刻是否正好在家？
                            // 從本 tick 開頭的座標快照查那位鄰居的位置，看牠是否站在自家牌子（tx,tz）附近。
                            let host_home = resident_names
                                .iter()
                                .position(|n| *n == nb.as_str())
                                .and_then(|i| resident_pos.get(i))
                                .map(|&(hx, hz)| vhosted::host_is_home(hx, hz, tx, tz))
                                .unwrap_or(false);
                            if host_home {
                                // 撲了個空變成真的碰上本人：訪客雀躍地念「見到本人」暖句，
                                // 主人側的迎客反應（迎客泡泡＋在家迎客記憶＋Feed）鎖外統一處理。
                                r.say = vhosted::met_line(&nb, pick);
                                hosted_meetings.push((r.name, nb.clone(), pick));
                            } else {
                                // 主人不在家：撲空（登門撲空留心意 v1，ROADMAP 763）——不再只是對空門口
                                // 說句話就走，而是**在門口留下一份心意**：念一句帶「留個心意」意味的暖句，
                                // 並把這位訪客記進那位主人的門口心意佇列（鎖外統一入列，主人回家後感應到）。
                                r.say = vcard::miss_line(&nb, pick);
                                calling_cards.push((nb.clone(), r.name));
                            }
                            r.say_timer = SAY_SECS;
                            neighbor_visit_arrivals.push((r.id.clone(), r.name, nb));
                        } else {
                            // 既有路徑（743）：獨自朝聖玩家立的牌，駐足念一句、寫「又回來看看」記憶。
                            r.say = vreadsign::revisit_sign_line(&quote, pick);
                            r.say_timer = SAY_SECS;
                            let summary = vreadsign::revisit_memory_summary(&quote);
                            let entry = hub()
                                .memory
                                .write()
                                .unwrap()
                                .add_memory(&r.id, vreadsign::SIGN_MEMORY_PLAYER, &summary);
                            vmem::append_memory(&entry);
                            pilgrimage_feed.push((r.name, quote.clone()));
                        }
                        r.pilgrimage = None;
                        r.pilgrimage_neighbor = None;
                        r.pilgrimage_cooldown = vreadsign::PILGRIMAGE_COOLDOWN;
                    }
                } else {
                    // 還在路上：遞減逾時；走太久（地形擋路等）沒到就放棄，設冷卻，不無限走。
                    r.pilgrimage_timer -= dt;
                    if r.pilgrimage_timer <= 0.0 {
                        r.pilgrimage = None;
                        r.pilgrimage_neighbor = None;
                        r.pilgrimage_cooldown = vreadsign::PILGRIMAGE_COOLDOWN;
                    }
                }
            } else {
                // 尚未在朝聖：閒置自由 + 冷卻到 + 心中有牌 + 過機率門檻 → 啟程重返。
                // 「閒置自由」＝沒在採集/跑腿/探訪/打氣/尋伴/聚會/跟隨/發明/睡覺（不搶正事）。
                let idle_free = r.gather.is_none()
                    && r.fetch.is_none()
                    && r.visiting.is_none()
                    && r.cheer_target.is_none()
                    && !r.seeking_comfort
                    && r.clique_meet.is_none()
                    && r.follow.is_none()
                    && r.invent_run.is_none()
                    && r.daybreak_seek.is_none()
                    && r.reunion_seek.is_none()
                    && r.expedition.is_none()
                    && !r.asleep;
                if vreadsign::should_pilgrimage(
                    r.cherished_sign.is_some(),
                    idle_free,
                    r.pilgrimage_cooldown,
                    r.say.is_empty(),
                    rand::random::<f32>(),
                ) {
                    if let Some((sx, sz, quote)) = r.cherished_sign.clone() {
                        // 只重返合理距離內、又不是已站腳下的牌子（太遠不去、防長途尋路卡死）。
                        let dx = r.body.x - sx;
                        let dz = r.body.z - sz;
                        if vreadsign::pilgrimage_worth_going(dx * dx + dz * dz) {
                            r.pilgrimage = Some((sx, sz, quote));
                            // 登門串門子 v1（ROADMAP 751）：啟程時快照「這趟走向的是哪位鄰居的家」，
                            // 途中即使又讀到別的牌改了 cherished_neighbor 也不影響這趟抵達的判定。
                            r.pilgrimage_neighbor = r.cherished_neighbor.clone();
                            r.pilgrimage_timer = vreadsign::PILGRIMAGE_TIMEOUT;
                            r.wait_timer = 0.0;
                        }
                    }
                }
            }

            // ── 登門撲空留心意·主人回家感應 v1（ROADMAP 763）──────────────────────────────────
            // 752 的「撲空」分支不再白跑：訪客撲空時已把名字留進主人的門口心意佇列（pending_callers）。
            // 主人回到自家附近、閒著沒事、沒在說話時，逐一感應這些心意——冒一句暖泡泡、鎖外記一筆掛在
            // 那位訪客名下的記憶＋一則 Feed，讓主人「就算錯過，也知道有人特地來找過我」。冷卻讓多張心意
            // 一張一張慢慢感應、不一次倒完。情誼**不在此重複記帳**（751 抵達時已 record_visit 過這對）。
            // 鎖序：say 於本鎖內設、記憶/Feed 走鎖外 callingcard_notices，不巢狀、不持鎖 await。
            if r.callingcard_cooldown > 0.0 {
                r.callingcard_cooldown -= dt;
            }
            if !r.pending_callers.is_empty()
                && r.callingcard_cooldown <= 0.0
                && r.say.is_empty()
                && r.pilgrimage.is_none()
                && r.gather.is_none()
                && r.fetch.is_none()
                && r.visiting.is_none()
                && r.cheer_target.is_none()
                && !r.seeking_comfort
                && r.clique_meet.is_none()
                && r.follow.is_none()
                && r.invent_run.is_none()
                && r.daybreak_seek.is_none()
                && r.reunion_seek.is_none()
                && r.expedition.is_none()
                && !r.asleep
                && vcard::noticed_at_home(r.body.x, r.body.z, r.home_x, r.home_z)
            {
                // 逐一感應最舊的一張心意（remove(0)＝先進先感應，逐張騰空佇列）。
                let guest = r.pending_callers.remove(0);
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                r.say = vcard::notice_line(&guest, pick);
                r.say_timer = SAY_SECS;
                r.callingcard_cooldown = vcard::NOTICE_COOLDOWN;
                callingcard_notices.push((r.id.clone(), r.name, guest));
            }

            // ── 你送的食物，她會細細享用 v1（ROADMAP 765·PLAN_ETHERVOX item 3「記憶→行為·你的互動有後果」）──
            // 玩家送食物時（見 Gift 分支）居民把它捧在手中（`savoring`）；這裡讓延遲倒數走完後，在一個
            // 閒下來的安靜片刻真的享用那份心意——冒一句滿足的暖泡泡、鎖外補一則動態牆，並**重新點亮心情**
            //（沿用贈禮 mood_boost：把送禮那刻的好心情延續到更晚、甚至在快消退時再拉回一格＝實打實的行為
            // 後果，非純裝飾）。忙碌時一直捧著、等真的閒下來才享用（不打斷採集/建造/社交/睡覺等正事，也
            // 不與泡泡打架）。鎖序：say/mood 於本鎖內設、Feed 走鎖外 savor_feeds，不巢狀、不持鎖 await。
            let savor_due = if let Some((_, _, timer)) = r.savoring.as_mut() {
                *timer -= dt;
                *timer <= 0.0
            } else {
                false
            };
            if savor_due
                && r.say.is_empty()
                && r.gather.is_none()
                && r.fetch.is_none()
                && r.visiting.is_none()
                && r.cheer_target.is_none()
                && !r.seeking_comfort
                && r.clique_meet.is_none()
                && r.follow.is_none()
                && r.invent_run.is_none()
                && r.daybreak_seek.is_none()
                && r.reunion_seek.is_none()
                && r.expedition.is_none()
                && r.pilgrimage.is_none()
                && !r.asleep
            {
                if let Some((food_id, giver, _)) = r.savoring.take() {
                    let food = vgift::item_name_zh(food_id);
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    r.say = vsavor::savor_bubble_line(food, &giver, pick)
                        .chars()
                        .take(50)
                        .collect();
                    r.say_timer = SAY_SECS;
                    // 吃飽了、暖起來——重新點亮心情（沿用贈禮補助時長）。
                    r.mood_boost_secs =
                        r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_GIFT);
                    savor_feeds.push((r.name, giver, food));
                }
            }

            // ── 自我印象 v1（ROADMAP 770·PLAN_ETHERVOX item 2「reflection：把記憶昇華成高階印象」）──
            // 居民偶爾在閒暇的安靜片刻，回望自己這一路累積的記憶，昇華出一句「我好像成了村裡最愛蓋東西
            // 的人」的自我概念、自言自語說出口，並記進動態牆。記憶第一次不只被記住／被說出，還昇華成
            // 居民對「自己是個怎樣的人」的理解——「記憶→行為」的一種（形塑了牠怎麼看自己、怎麼開口）。
            // 稀少而溫柔：長冷卻＋低機率＋要有明顯主導領域才觸發，不洗版；沿用既有泡泡/Feed 管線、零新
            // 實體、FPS 零影響。鎖序：記憶短讀即釋→即冒泡泡（say 於本鎖內設）、Feed 走鎖外 self_image_feeds，
            // 不巢狀、不持鎖 await（守 prod 死鎖鐵律）。隱私：輸出全為固定模板、永不回放記憶原文/玩家原話。
            if r.self_image_cooldown > 0.0 {
                r.self_image_cooldown -= dt;
            }
            if r.self_image_cooldown <= 0.0
                && r.say.is_empty()
                && r.gather.is_none()
                && r.fetch.is_none()
                && r.visiting.is_none()
                && r.cheer_target.is_none()
                && !r.seeking_comfort
                && r.clique_meet.is_none()
                && r.follow.is_none()
                && r.invent_run.is_none()
                && r.daybreak_seek.is_none()
                && r.reunion_seek.is_none()
                && r.expedition.is_none()
                && r.pilgrimage.is_none()
                && !r.asleep
                && rand::random::<f32>() < vself::SPEAK_CHANCE
            {
                // 短鎖快照本居民全部記憶 → 立即釋放，鎖外昇華（純函式、確定性）。
                let mems = { hub().memory.read().unwrap().all_memories_for(&r.id) };
                // 自我印象 v3（ROADMAP 772·PLAN_ETHERVOX item 2「reflection」）：**自我印象會隨生活
                // 變遷而轉變**。先看牠這回昇華出的主導領域是否與上次記得的**不同**——若不同，那是比
                // 一般回望更深的一刻（「我從前是最愛蓋東西的人，這陣子回頭看看，竟活成了離不開水邊的
                // 人」），優先說出口、取代這回的一般自我印象泡泡；牠的自我認同不再是釘死的標籤，而是
                // 一件會隨牠的作為成長變化的活東西。轉變後也順勢生「更像現在的自己」的新心願（沿用 v2）。
                if let Some((shift_line, new_domain)) =
                    vself::self_image_shift(r.self_image_domain, &mems)
                {
                    r.say = shift_line.chars().take(50).collect();
                    r.say_timer = SAY_SECS;
                    if let Some(prev) = r.self_image_domain {
                        self_image_feeds
                            .push((r.name, vself::self_image_shift_feed_line(prev, new_domain)));
                    }
                    r.self_image_domain = Some(new_domain); // 自我認同更新成「現在的自己」。
                    if let Some(desire) = vself::self_sparked_desire(&mems) {
                        self_aspiration_sparks.push((r.id.clone(), r.name, desire));
                    }
                    r.self_image_cooldown = vself::SPEAK_COOLDOWN;
                } else if let Some(bubble) = vself::self_image_bubble(&mems) {
                    r.say = bubble.chars().take(50).collect();
                    r.say_timer = SAY_SECS;
                    if let Some(feed) = vself::self_image_feed_line(&mems) {
                        self_image_feeds.push((r.name, feed));
                    }
                    // v3：記住這回昇華出的主導領域，供日後偵測轉變（首次昇華在此落定 prev；
                    // 領域沒變時 self_image_shift 回 None 也會落到這裡、原地重存同一領域）。
                    if let Some((dom, _)) = vself::dominant_domain(&mems) {
                        r.self_image_domain = Some(dom);
                    }
                    // 自我印象 v2（ROADMAP 771）：她回望這一路、認出「我是個怎樣的人」的這一刻，
                    // 若能落到一個具體的建造念頭（蓋東西的人→再蓋間屋、離不開泥土的人→再開畦花圃…），
                    // 就把這份自我理解**化成一個呼應自己的自發心願**——鎖外檢查她此刻確實沒有心願、
                    // 再種上（不搶玩家親口許的願、守鎖序），讓她接下來真的動手去追尋。記憶第一次不只
                    // 被說出、還推著她的腳步去做——「記憶→行為」的直接體現。
                    if let Some(desire) = vself::self_sparked_desire(&mems) {
                        self_aspiration_sparks.push((r.id.clone(), r.name, desire));
                    }
                    r.self_image_cooldown = vself::SPEAK_COOLDOWN; // 說過了，久久才再回望一次。
                } else {
                    r.self_image_cooldown = vself::RETRY_COOLDOWN; // 還沒昇華出印象，過陣子再看。
                }
            }

            // ── 遠行探野 v1（ROADMAP 756·PLAN_ETHERVOX item 7「居民散佈世界各處住」第一刀）────────
            // 散居 v6（ROADMAP 762）：能遠行的人格擴成兩位——奧瑞（漂泊天性）與諾娃（農人尋覓沃野），
            // 各自往家的方位安下**不同**的邊陲據點（落點由家座標算出→不同方位/生物群系）。
            // 能遠行的居民偶爾放下手邊的事、獨自遠行到遠離主城的荒野邊陲住上一陣子再返家
            // ——居民的足跡第一次真的散進荒野，玩家會在遠離主城的地方撞見牠。狀態機沿用探訪／朝聖既有
            // 手法：`expedition` 有值＝正遠行前往或在邊陲逗留；逗留倒數歸零即清空、交回下方一般 wander
            // （此刻遠在家域外，`wander_center` 會把牠一路帶回家，不必顯式返程腿）。
            // 鎖序：memory 短寫即釋（比照朝聖 v3），Feed 走鎖外 expedition_feed，不巢狀、不持鎖 await。
            if r.expedition_cooldown > 0.0 {
                r.expedition_cooldown -= dt;
            }
            if let Some((tx, tz, bearing)) = r.expedition.clone() {
                if r.expedition_stay > 0.0 {
                    // 已抵達邊陲、正在遠方逗留探索。
                    // ── 邊陲過夜 v4（ROADMAP 759）：夜色降臨就不趕夜路，改在營地那張床上過一夜 ──
                    // `is_night`＝可睡時段（傍晚或深夜）：一入夜就凍結逗留倒數（不返家）、把居民導向
                    // 營地那張床邊等；到了深夜（`is_deep_night`）且已走到床邊 → 躺下睡（設 asleep +
                    // asleep_at_outpost，下一 tick 起由頂端睡眠 gate 接管靜止到天亮，醒來另行結束遠行、
                    // 啟程返家）。白天則照舊：倒數逗留，歸零即歸來（返程移動交給下方 wander）。
                    if is_night {
                        let (bedx, bedz) = vexp::outpost_bed_center(tx.round() as i32, tz.round() as i32);
                        let near_bed = vexp::near_outpost_bed(r.body.x, r.body.z, bedx, bedz);
                        if vexp::should_sleep_at_outpost(is_deep_night, near_bed) {
                            // 就寢：就地躺下（目標設腳邊、清閒晃意圖），冒過夜泡泡、記一則就寢 Feed；
                            //「過了一夜」的記憶留到醒來時昇華（見上方 wake 分支）。
                            r.asleep = true;
                            r.asleep_at_outpost = true;
                            r.target_x = r.body.x;
                            r.target_z = r.body.z;
                            if r.say.is_empty() {
                                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                                r.say = vexp::outpost_sleep_bubble(&bearing, pick);
                                r.say_timer = SAY_SECS;
                            }
                            expedition_feed.push((r.name, vexp::outpost_sleep_feed_line(&bearing)));
                        } else {
                            // 天黑了但還沒到深夜（或還沒走到床邊）：走向／守在營地床邊等入睡，
                            // 別再隨機閒晃跑遠（下方 wander 若重挑目標，也會被下一 tick 這裡覆寫回來）。
                            r.target_x = bedx;
                            r.target_z = bedz;
                            r.wait_timer = 0.0;
                        }
                        // 夜裡凍結逗留倒數：不 -= dt、不返家（等睡醒才啟程回主城）。
                    } else {
                        // 白天照舊倒數；歸零則歸來——清空遠行狀態、設冷卻，冒歸來泡泡、收一則歸來 Feed。
                        r.expedition_stay -= dt;
                        if r.expedition_stay <= 0.0 {
                            r.expedition = None;
                            r.expedition_cooldown = vexp::EXPEDITION_COOLDOWN;
                            // 生物群系版歸來 Feed：帶回的見聞點名去過的地方（草原／森林／沙漠／雪原）。
                            let biome = crate::voxel::biome_at_voxel(tx.round() as i32, tz.round() as i32);
                            expedition_feed.push((r.name, vexp::return_feed_line(biome)));
                            if r.say.is_empty() {
                                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                                // 生物群系版歸來泡泡（ROADMAP 761）：點出從邊陲帶回了什麼當地風物。
                                r.say = vexp::return_bubble(biome, pick);
                                r.say_timer = SAY_SECS;
                            }
                            // 遠行帶回的邊陲風物（ROADMAP 761）：收集一筆「把當地風物種在家門前」的
                            // 請求，鎖外統一落地（比照營火／小棚）。家門前那排紀念漸漸長成牠去過的地圖。
                            expedition_keepsakes.push((
                                r.id.clone(),
                                r.name,
                                r.home_x,
                                r.home_z,
                                biome,
                                bearing.clone(),
                            ));
                        }
                        // 逗留期間不在此設 target：由下方 wander 以邊陲為中心自由走動（見 center 覆寫）。
                    }
                } else {
                    // 去程：持續朝邊陲走（清小歇，步步逼近，讓玩家看得到牠專程走遠）。
                    r.target_x = tx;
                    r.target_z = tz;
                    r.wait_timer = 0.0;
                    let dx = r.body.x - tx;
                    let dz = r.body.z - tz;
                    let arrived = dx * dx + dz * dz
                        < vexp::EXPEDITION_ARRIVE_DIST * vexp::EXPEDITION_ARRIVE_DIST;
                    if arrived {
                        // 抵達邊陲且沒在說別的話：開始逗留、冒抵達泡泡、把「到過遠方」昇華成一筆記憶
                        //（掛哨兵鍵、日記／內心可引用）。正巧在說別的話就先按住、等 say 清空再抵達。
                        if r.say.is_empty() {
                            r.expedition_stay = vexp::EXPEDITION_STAY_SECS;
                            // 生物群系版抵達（ROADMAP 760）：居民認出腳下是什麼地方（草原／森林／
                            // 沙漠／雪原），泡泡、記憶、Feed 都帶上地方感——遠行第一次真的「去了個地方」。
                            let biome = crate::voxel::biome_at_voxel(tx.round() as i32, tz.round() as i32);
                            let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                            r.say = vexp::arrive_bubble(&bearing, biome, pick);
                            r.say_timer = SAY_SECS;
                            let summary = vexp::arrive_memory_summary(&bearing, biome);
                            let entry = hub().memory.write().unwrap().add_memory(
                                &r.id,
                                vexp::EXPEDITION_MEMORY_PLAYER,
                                &summary,
                            );
                            vmem::append_memory(&entry);
                            // 抵達動態牆播報：不在場的玩家回來也能讀到「牠到了什麼地方」。
                            expedition_feed.push((r.name, vexp::arrive_feed_line(&bearing, biome)));
                            // 邊陲營火路標（遠行 v2）：抵達那一刻收集一筆「在落點升起營火」的請求，
                            // 落點取遠行目標中心（tx,tz，居民就在其 EXPEDITION_ARRIVE_DIST 半徑內，
                            // 不會正踩在灶台上）。實際落地在鎖外統一處理。每趟遠行只在此抵達分支跑一次。
                            expedition_campfires.push((
                                r.id.clone(),
                                r.name,
                                tx.round() as i32,
                                tz.round() as i32,
                                bearing.clone(),
                            ));
                        }
                    } else {
                        // 還在路上：遞減去程逾時；走太久（地形擋路等）沒到就放棄這趟遠行、
                        // 清空並設冷卻（交回一般 wander 帶牠回家）。
                        r.expedition_timer -= dt;
                        if r.expedition_timer <= 0.0 {
                            r.expedition = None;
                            r.expedition_cooldown = vexp::EXPEDITION_COOLDOWN;
                        }
                    }
                }
            } else {
                // 尚未遠行：能遠行的人格（奧瑞·漂泊／諾娃·尋地）+ 閒置自由 + 白天 + 冷卻到 + 過機率門檻 → 啟程。
                // 「閒置自由」＝沒在採集/跑腿/探訪/打氣/尋伴/聚會/跟隨/發明/朝聖/思念/奔迎/睡覺（不搶正事）。
                let motive = vexp::expedition_motive(r.persona);
                let idle_free = r.gather.is_none()
                    && r.fetch.is_none()
                    && r.visiting.is_none()
                    && r.cheer_target.is_none()
                    && !r.seeking_comfort
                    && r.clique_meet.is_none()
                    && r.follow.is_none()
                    && r.invent_run.is_none()
                    && r.pilgrimage.is_none()
                    && r.daybreak_seek.is_none()
                    && r.reunion_seek.is_none()
                    && !r.asleep;
                if vexp::should_embark(
                    motive.is_some(),
                    idle_free,
                    !is_night,
                    r.expedition_cooldown,
                    r.say.is_empty(),
                    rand::random::<f32>(),
                ) {
                    // should_embark 過閘已保證 motive.is_some()（能遠行的人格才會過 can_embark 閘）。
                    let motive = motive.expect("能遠行的人格才會過 should_embark 閘");
                    // 遠行 v3（ROADMAP 758）：落點改由「家的方位」確定性算出（outpost_seq），
                    // 同一位居民每趟遠行都回到同一處邊陲營地——漂泊收斂成安頓、荒野長出專屬據點。
                    let outpost = vexp::outpost_seq(r.home_x, r.home_z);
                    let (fx, fz, bearing) = vexp::pick_frontier(r.home_x, r.home_z, outpost);
                    r.expedition = Some((fx, fz, bearing.to_string()));
                    r.expedition_stay = 0.0;
                    r.expedition_timer = vexp::EXPEDITION_TIMEOUT;
                    r.target_x = fx;
                    r.target_z = fz;
                    r.wait_timer = 0.0;
                    // 泡泡台詞仍用當下身位輪替（維持變化，不因據點固定而每趟同一句）；依動機分岔
                    //（奧瑞漂泊／諾娃尋地口吻各異）。
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    r.say = vexp::embark_bubble(motive, bearing, pick);
                    r.say_timer = SAY_SECS;
                    expedition_feed.push((r.name, vexp::embark_feed_line(motive, bearing)));
                }
            }

            // 回禮 v1（ROADMAP 667）：好感達門檻 + 玩家靠近 + 尚未回贈 → 收集事件（鎖外落地）。
            // 鎖序：memory 讀（即釋）→ return_gifts 讀（即釋）→ 收集 event；
            // 實際 inventory 寫 / append / broadcast 在居民鎖釋放後進行（鎖紀律）。
            if r.say.is_empty() {
                if let Some((d2, nearest_name)) =
                    nearest_player_info(r.body.x, r.body.z, &player_pts)
                {
                    let in_reach = d2 < vret::RETURN_GIFT_REACH * vret::RETURN_GIFT_REACH;
                    // 名字空白 = 訪客（無持久身份），跳過。
                    let is_logged_in = !nearest_name.is_empty();
                    if in_reach && is_logged_in {
                        // 一次性短鎖讀好感度 + 這位玩家在她心裡的「偏好」記憶（同一把讀鎖、不巢狀）。
                        // 投你所好 v1（ROADMAP 731）：把她記得的偏好事實內容撈出，鎖外再映射成禮物。
                        let (affinity, pref_gift) = {
                            let mem = hub().memory.read().unwrap();
                            let aff = mem.affinity_count(nearest_name, &r.id);
                            let prefs: Vec<String> = mem
                                .semantic_facts_for(&r.id, nearest_name)
                                .into_iter()
                                .filter(|f| f.category == vmem::FactCategory::Preference)
                                .map(|f| f.content)
                                .collect();
                            (aff, vpref::gift_for_preference(&prefs))
                        }; // memory 讀鎖在此釋放
                        // 一次性短鎖讀「已送過？」（不 await）。
                        let already = {
                            hub().return_gifts.read().unwrap().already_given(&r.id, nearest_name)
                        }; // return_gifts 讀鎖在此釋放
                        if vret::should_return_gift(affinity, already) {
                            // 回禮 v2（ROADMAP 728）：優先送她「親手採集到的東西」——短鎖讀她的
                            // 採集背包，挑她採得最多的那種材料（不扣減：只反映不消耗，避免干擾她的
                            // 發明湊料）。背包空 → 回退到 667 憑空的木頭/種子選項。
                            let from_stock = {
                                let inv = hub().res_inv.read().unwrap();
                                inv.get(&r.id).and_then(vret::pick_from_stock)
                            }; // res_inv 讀鎖在此釋放
                            // 選禮優先序：① 投你所好（她記得你說過的偏好，最有魔法）→
                            // ② 她親手採到的（728）→ ③ 憑空的木頭/種子（667）。
                            let (bid, qty, msg) = if let Some((bid, qty, theme)) = pref_gift {
                                let iname = vgift::item_name_zh(bid);
                                (
                                    bid,
                                    qty,
                                    vpref::preference_gift_message(r.name, nearest_name, theme, iname),
                                )
                            } else if let Some((bid, qty)) = from_stock {
                                let iname = vgift::item_name_zh(bid);
                                (
                                    bid,
                                    qty,
                                    vret::return_gift_message_gathered(r.name, nearest_name, iname),
                                )
                            } else {
                                let (bid, qty) = vret::pick_return_gift(&r.id);
                                let iname = vret::return_item_name(bid);
                                (bid, qty, vret::return_gift_message(r.name, nearest_name, iname))
                            };
                            r.say = msg.chars().take(40).collect();
                            r.say_timer = SAY_SECS;
                            return_gift_events.push((
                                r.id.clone(),
                                r.name,
                                nearest_name.to_string(),
                                bid,
                                qty,
                                msg,
                            ));
                        }
                    }
                }
            }

            // 雨天反應 v1（ROADMAP 701）：雨剛開始下的那一刻，say 為空的居民冒一句應景台詞
            // （零 LLM、確定性選句）；優先於下方的心情自語（罕見的一次性事件，值得蓋過閒聊冷卻）。
            if rain_just_started && r.say.is_empty() {
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                r.say = vweather::rain_started_line(pick).to_string();
                r.say_timer = SAY_SECS;
            }

            // 心情自語 v1（ROADMAP 677）：冷卻到期且 say 為空時，依心情自發冒一句台詞。
            // 鎖序：bonds 讀（即釋）→ memory 讀（即釋），不巢狀，不持鎖 await。
            if r.say.is_empty() && r.mood_say_cooldown <= 0.0 {
                let (friends, acq) = {
                    let bonds = hub().bonds.read().unwrap();
                    resident_bond_counts(&bonds, &r.id)
                }; // bonds 讀鎖釋放
                let mems = {
                    hub().memory.read().unwrap().memory_count(&r.id)
                }; // memory 讀鎖釋放
                let tier = voxel_mood::compute_mood(friends, acq, mems);
                // pick 由位置的位元決定（確定性，隨居民移動自然變化）
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                if let Some(line) = voxel_mood::spontaneous_line(tier, pick) {
                    r.say = line.to_string();
                    r.say_timer = SAY_SECS;
                }
                // 無論是否說話都重置冷卻（Neutral 不說話，但仍更新，防每 tick 都查）
                r.mood_say_cooldown = MOOD_SAY_COOLDOWN;
            }

            // 探訪計時 v1（ROADMAP 671）：冷卻 + 逗留倒數。
            // 冷卻倒數：每 tick 減 dt，到 0 後 should_visit 才可觸發（守 alive 不低於 -1 以防溢位）。
            if r.visit_cooldown > -1.0 {
                r.visit_cooldown -= dt;
            }
            // 逗留倒數：只有已抵達目的地（visit_stay_timer > 0）才倒數；在途中不計。
            if r.visit_stay_timer > 0.0 {
                r.visit_stay_timer -= dt;
                if r.visit_stay_timer <= 0.0 {
                    // 逗留結束：清探訪狀態、冒告別台詞、回歸家域中心閒晃。
                    let host_name = r.visiting.as_ref().map(|(_, _, n)| n.clone()).unwrap_or_default();
                    r.visiting = None;
                    r.target_x = r.home_x;
                    r.target_z = r.home_z;
                    // 告別語依情誼層級，由鎖外 bond_depart_events 推入 say_updates（ROADMAP 672）。
                    bond_depart_events.push((r.id.clone(), r.name, host_name.clone()));
                    visit_events.push((r.name, host_name, true)); // is_return = true
                }
            }

            // 卡住偵測用：記下移動前的水平位置（本 tick 結束再比對位移）。
            let (pre_x, pre_z) = (r.body.x, r.body.z);

            // 指令→任務 v1 第二刀·跟隨執行：她真的能跟你走。逾時自動結束跟隨（回到平常閒晃）；
            // 找不到玩家目前位置（例如剛離線）就原地站穩、等下個 tick 或逾時。優先於一切其他分支
            // ——玩家明確叫她跟，就該立刻聽話，不該被採集/整地/建造打斷。
            if let Some((requester, mut remaining)) = r.follow.clone() {
                remaining -= dt;
                if remaining <= 0.0 {
                    r.follow = None;
                    r.target_x = r.home_x;
                    r.target_z = r.home_z;
                    if r.say.is_empty() {
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                        r.say = vdt::follow_timeout_line(pick).chars().take(40).collect();
                        r.say_timer = SAY_SECS;
                    }
                    vr::gravity_step(&world, &mut r.body, dt);
                } else {
                    r.follow = Some((requester.clone(), remaining));
                    if let Some((px, pz, _)) = player_pts.iter().find(|(_, _, n)| *n == requester) {
                        let (bx, bz) = (r.body.x, r.body.z);
                        let tx = px - vdt::FOLLOW_OFFSET;
                        let tz = pz - vdt::FOLLOW_OFFSET;
                        vr::step_toward(&world, &mut r.body, tx, tz, dt, vr::RES_SPEED * speed_mult);
                        if let Some(yaw) = vr::yaw_from_move(r.body.x - bx, r.body.z - bz) {
                            r.yaw = yaw;
                        }
                    } else {
                        vr::gravity_step(&world, &mut r.body, dt);
                    }
                }
                // 指令→任務·整地/鋪面技能執行：被指派的居民先走到工地中心附近，
                // 抵達後每 tick 排程施工一批（鎖外套用方塊改動）。施工中跳過閒晃/歸巢
                // （這是「她真的照玩家的話做事」）。優先於採集分支（但仍讓位給上面的跟隨）；
                // **例外**：鋪面備料中（is_pave 且 gather 有值）→ 讓位給下方採集分支，
                // 她先去把原料挖回小背包，採完自然回工地續鋪（誠實備料，不隔空變料）。
            } else if let Some(&(cx, cz, radius, _)) = directed_snaps
                .get(&r.id)
                .filter(|&&(_, _, _, is_pave)| !is_pave || r.gather.is_none())
            {
                let center_x = cx as f32 + 0.5;
                let center_z = cz as f32 + 0.5;
                let dx = r.body.x - center_x;
                let dz = r.body.z - center_z;
                let arrive = radius as f32 + vdt::LEVEL_ARRIVE_MARGIN;
                if dx * dx + dz * dz > arrive * arrive {
                    // 還沒到工地：朝中心走（沿牆滑行、踏階由物理處理）。
                    let (bx, bz) = (r.body.x, r.body.z);
                    vr::step_toward(&world, &mut r.body, center_x, center_z, dt, vr::RES_SPEED * speed_mult);
                    if let Some(yaw) = vr::yaw_from_move(r.body.x - bx, r.body.z - bz) {
                        r.yaw = yaw;
                    }
                    // 走路卡死保險（真因修正）：貪心尋路（沿牆滑行、只踏一階）遇到深水/深坑/
                    // 高牆會繞不過而原地卡死，任務就這樣白白倒數到逾時放棄——這正是玩家實測到的
                    // 「答應了卻沒整完」。這裡偵測「連續沒更接近工地」，卡太久就**就近把她挪到工地**
                    // 可站處（沿用既有脫困挪位精神，但挪到工地就地開工、不放棄），確保「說到做到」。
                    let ndx = r.body.x - center_x;
                    let ndz = r.body.z - center_z;
                    let cur_d2 = ndx * ndx + ndz * ndz;
                    let (nb, ns, relocate) =
                        vdt::walk_stall_update(r.level_best_d2, r.level_walk_stall, cur_d2, dt);
                    r.level_best_d2 = nb;
                    r.level_walk_stall = ns;
                    if relocate {
                        let (sx, sy, sz) =
                            vdt::nearest_site_stand(&world, r.body.x, r.body.z, cx, cz, radius);
                        r.body = Body::at(sx, sy, sz);
                    }
                } else {
                    // 已在工地：排程本 tick 整地一批（鎖外套用），原地落重力站穩。
                    level_workers.push(r.id.clone());
                    vr::gravity_step(&world, &mut r.body, dt);
                }
                // 跑腿採集 v1（指令→任務第三刀）·交付階段：已採齊（或已逾時強制收工，見上方
                // 冷卻段），朝下令的玩家走去，抵達就排程交付（鎖外進背包），任務結束。
            } else if r.fetch.as_ref().is_some_and(|f| f.is_gathering_done()) {
                let requester = r.fetch.as_ref().unwrap().requester.clone();
                if let Some((px, pz, _)) = player_pts.iter().find(|(_, _, n)| *n == requester) {
                    let (px, pz) = (*px, *pz);
                    let dx = r.body.x - px;
                    let dz = r.body.z - pz;
                    if dx * dx + dz * dz <= vfetch::FETCH_DELIVER_REACH * vfetch::FETCH_DELIVER_REACH {
                        if let Some(task) = r.fetch.take() {
                            fetch_deliver_events.push((
                                r.id.clone(), r.name, task.requester, task.resource,
                                task.carried, task.requested,
                            ));
                        }
                        r.wait_timer = 1.5; // 交付後小歇，別立刻又閒晃走開
                        vr::gravity_step(&world, &mut r.body, dt);
                    } else {
                        let (bx, bz) = (r.body.x, r.body.z);
                        vr::step_toward(&world, &mut r.body, px, pz, dt, vr::RES_SPEED * speed_mult);
                        if let Some(yaw) = vr::yaw_from_move(r.body.x - bx, r.body.z - bz) {
                            r.yaw = yaw;
                        }
                    }
                } else {
                    // 下令的玩家目前不在線（找不到座標）：原地等待，逾時保險已在冷卻段處理。
                    vr::gravity_step(&world, &mut r.body, dt);
                }
                // 跑腿採集 v1·尚未鎖定資源目標：等鎖外的 fetch_search_candidates 處理完指派，
                // 原地站穩別亂晃（找到目標後下個 tick 自然落入下方的 r.gather.is_some() 分支）。
            } else if r.fetch.is_some() && r.gather.is_none() {
                if let Some(f) = r.fetch.as_ref() {
                    fetch_search_candidates.push((
                        r.id.clone(),
                        r.body.x.floor() as i32,
                        r.body.y.floor() as i32,
                        r.body.z.floor() as i32,
                        f.resource,
                    ));
                }
                vr::gravity_step(&world, &mut r.body, dt);
            } else if r.gather.is_some() {
                let (tx, ty, tz, reached, timed_out) = {
                    let g = r.gather.as_mut().unwrap();
                    g.timeout -= dt;
                    let reached = vskill::within_gather_reach(r.body.x, r.body.z, g.tx, g.tz);
                    (g.tx, g.ty, g.tz, reached, g.timeout <= 0.0)
                };
                if reached {
                    // 走到了：先做可逃性判定，確認「挖這塊還逃得出去」（防採集挖坑卡死）。
                    // 走過來後實際站位可能與當初找資源時不同，這裡用即時腳底格＋當下世界再核一次。
                    let res = r.gather.take().unwrap().resource;
                    let (fx, fy, fz) = (
                        r.body.x.floor() as i32,
                        r.body.y.floor() as i32,
                        r.body.z.floor() as i32,
                    );
                    if vskill::is_escapable_after_dig(&world, fx, fy, fz, tx, ty, tz) {
                        // 排程挖掘（站定落重力）。跑腿任務中的採集算進 fetch 進度，
                        // 不算進一般備料計數（兩者用途不同：一個是玩家指定的份數，
                        // 一個是蓋造前的備料配額）。
                        gather_mines.push((r.id.clone(), r.name, tx, ty, tz, res));
                        if let Some(f) = r.fetch.as_mut() {
                            f.remaining = f.remaining.saturating_sub(1);
                            f.carried = f.carried.saturating_add(1);
                        } else {
                            r.gathered_since_build = r.gathered_since_build.saturating_add(1);
                        }
                    }
                    // 不安全就放棄這塊（不挖、不計數）；下個 agency tick 重找安全資源。
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
                    // ── 探訪 v1（ROADMAP 671）：在抵達前先處理探訪邏輯 ──────────────
                    if let Some((vx, vz, ref host_name)) = r.visiting.clone() {
                        let dx = r.body.x - vx;
                        let dz = r.body.z - vz;
                        let dist = (dx * dx + dz * dz).sqrt();
                        if dist < vvisit::VISIT_ARRIVE_DIST {
                            // 剛抵達：設逗留計時、冒問候泡泡（若目前沒有其他話）。
                            if r.visit_stay_timer <= 0.0 {
                                r.visit_stay_timer = vvisit::VISIT_STAY_SECS;
                                // 問候語依情誼層級，由鎖外 bond_arrive_events 處理後推入 say_updates
                                // （ROADMAP 672）；say_timer 留給 say_updates 套用。
                                bond_arrive_events.push((r.id.clone(), r.name, host_name.clone()));
                                // Feed 事件（鎖外 IO，須在鎖釋放後再 push；這裡先推送 event tuple）。
                                visit_events.push((r.name, host_name.clone(), false));
                            }
                        } else {
                            // 還在路上：直接朝目標前進，不另外閒晃。
                            r.target_x = vx;
                            r.target_z = vz;
                        }
                    }

                    // ── 正常閒晃（探訪中改以目的地為中心）──────────────────────────
                    let angle = rand::random::<f32>() * std::f32::consts::TAU;
                    // 日夜作息 v1：夜間閒晃半徑隨速度乘數縮小（居民不往遠處跑）。
                    let radius = (WANDER_MIN_R + rand::random::<f32>() * (WANDER_MAX_R - WANDER_MIN_R)) * speed_mult.max(0.4);
                    // 歸巢遮蔽：不在探訪中 + （現在是夜間 或 正在下雨）+ 已蓋好自己的小屋
                    // → 以小屋為閒晃中心（緊靠自家），取代原本的家域出生點。
                    // ROADMAP 701：白天下雨時也比照夜間歸巢，居民第一次會為了避雨回家。
                    // 遠行中不遮蔽：牠正遠在荒野邊陲探索，不該被夜間／下雨拉回主城的家（遠行有逾時
                    // 與逗留上限自我了結，不會無限滯留）。
                    let sheltering = r.visiting.is_none()
                        && r.expedition.is_none()
                        && r.clique_meet.is_none()
                        && vr::should_shelter(is_night, raining, house_locations.contains_key(&r.id));
                    // 探訪中：以目的地為閒晃中心（讓居民在鄰居家附近自然走動）；
                    // 遠行逗留中（ROADMAP 756）：以邊陲落點為中心，讓牠在遠方一小片範圍自然走動、不被拉回家；
                    // 聚會中（ROADMAP 711）：以聚會點為中心，讓一群人看起來聚在一塊；
                    // 夜間遮蔽：以自己蓋的小屋為中心；否則：以自己家域中心為基準（正常行為）。
                    let (center_x, center_z) = if let Some((vx, vz, _)) = &r.visiting {
                        (*vx, *vz)
                    } else if let Some((ex, ez, _)) = &r.expedition {
                        (*ex, *ez)
                    } else if let Some((gx, gz, _)) = &r.clique_meet {
                        (*gx, *gz)
                    } else if sheltering {
                        let (hx, _hy, hz) = house_locations[&r.id];
                        (hx as f32 + 0.5, hz as f32 + 0.5)
                    } else {
                        (r.home_x, r.home_z)
                    };
                    // 探訪中用探訪範圍；遠行逗留用遠行範圍（在邊陲一小片走動）；聚會中用更小的聚會範圍
                    //（不散開）；夜間遮蔽用更小的遮蔽半徑（緊靠自家）；否則用家域半徑（正常行為）。
                    let wander_r = if r.visiting.is_some() {
                        vvisit::VISIT_WANDER_RADIUS
                    } else if r.expedition.is_some() {
                        vexp::EXPEDITION_WANDER_RADIUS
                    } else if r.clique_meet.is_some() {
                        vclique::GATHER_WANDER_RADIUS
                    } else if sheltering {
                        vr::SHELTER_WANDER_RADIUS
                    } else {
                        vr::HOME_RADIUS
                    };
                    // 日夜作息·睡覺 v1（ROADMAP 739）：深夜已回到自家附近的遮蔽居民，
                    // 抵達閒晃點時偶爾就此躺下睡覺——不再挑新閒晃點，改成安靜靜止（下一 tick
                    // 起由上方睡眠 gate 接管，只落重力、跳過一切行為，直到天亮才醒）。
                    let near_home = vsleep::near_home_center(r.body.x, r.body.z, center_x, center_z);
                    if vsleep::should_fall_asleep(
                        is_deep_night, sheltering, near_home, r.asleep, rand::random::<f32>(),
                    ) {
                        r.asleep = true;
                        // 就地睡下：目標設在腳邊、清掉閒晃意圖。
                        r.target_x = r.body.x;
                        r.target_z = r.body.z;
                        if r.say.is_empty() {
                            let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                            // 就寢反思 v1（ROADMAP 744）：入睡時回味今天最有感的一件事——冒個人化
                            // 反思泡泡、把反思昇華成「睡前反思」記憶、並記一筆動態供離線玩家回讀。
                            // 鎖序：memory 讀→drop→memory 寫（短鎖循序即釋，比照讀牌 v3），
                            // Feed 走鎖外 bedtime_feed，不巢狀、不持鎖 await。過機率門檻時才回味，
                            // 否則（或無可回味的記憶）退回通用睡前語。
                            let mut reflected = false;
                            if vbedtime::should_reflect(true, rand::random::<f32>()) {
                                // 短鎖讀近況記憶 → 挑今天最值得回味的一筆（跳過自己昨晚的反思）→ 即釋。
                                let salient = {
                                    let mem = hub().memory.read().unwrap();
                                    let recent = mem.all_memories_for(&r.id); // 最新在前
                                    let window: Vec<&vmem::MemoryEntry> = recent
                                        .iter()
                                        .filter(|e| e.player != vbedtime::REFLECT_MEMORY_PLAYER)
                                        .take(vbedtime::RECENT_WINDOW)
                                        .collect();
                                    let ranked: Vec<(bool, u64)> = window
                                        .iter()
                                        .map(|e| {
                                            let persistent = matches!(
                                                vmem::classify_importance(&e.summary),
                                                vmem::Importance::Persistent(_)
                                            );
                                            (persistent, e.seq)
                                        })
                                        .collect();
                                    vbedtime::most_memorable(&ranked)
                                        .map(|i| window[i].summary.clone())
                                }; // 記憶讀鎖在此釋放
                                if let Some(summary) = salient {
                                    r.say = vbedtime::reflect_bubble(&summary, pick);
                                    r.say_timer = SAY_SECS;
                                    // 昇華成「睡前反思」記憶（短寫鎖即釋、不巢狀、不持鎖 await）。
                                    let entry = hub().memory.write().unwrap().add_memory(
                                        &r.id,
                                        vbedtime::REFLECT_MEMORY_PLAYER,
                                        &vbedtime::reflect_memory_summary(&summary),
                                    );
                                    vmem::append_memory(&entry);
                                    bedtime_feed.push((r.name, summary));
                                    reflected = true;
                                }
                            }
                            if !reflected {
                                r.say = vsleep::fall_asleep_line(pick).to_string();
                                r.say_timer = SAY_SECS;
                            }
                        }
                    } else {
                        let (wcx, wcz) = vr::wander_center(
                            r.body.x, r.body.z,
                            center_x, center_z,
                            wander_r,
                        );
                        let (tx, tz) = vr::wander_target(wcx, wcz, angle, radius);
                        r.target_x = tx;
                        r.target_z = tz;
                        // 日夜作息 v1：夜間額外多停一段（extra_wait），讓居民在原地駐足更久。
                        r.wait_timer = 1.0 + rand::random::<f32>() * 3.0 + extra_wait;
                    }
                }
            }

            // 卡住偵測 + 脫困/送回（修：只救「真被困」的，不打斷正常採集/建造）。
            // 只有「正在導航（非採集/蓋造動作中）+ 幾何上真被困 + 幾乎沒位移」才算卡住。
            let moved = ((r.body.x - pre_x).powi(2) + (r.body.z - pre_z).powi(2)).sqrt();
            // 正在導航：朝閒晃/歸巢目標走，且不是在執行採集動作、也不在原地歇息。
            // 採集（gather）有自己的 25 秒逾時處理「走不到資源就放棄」，不該被脫困偵測搶先誤救。
            // 整地任務中不算「純導航」（與採集同理：她在做事，別被脫困偵測誤救打斷任務）。
            // 跟隨中也不算：跟著玩家走本就目標常變，別被脫困偵測誤判。
            // 跑腿採集中（指令→任務第三刀）同理：等待被指派下個目標／走去交付都不是「純導航」。
            let navigating = r.gather.is_none()
                && r.wait_timer <= 0.0
                && !directed_snaps.contains_key(&r.id)
                && r.follow.is_none()
                && r.fetch.is_none();
            // 幾何困住判定（埋在實心裡或四面爬不出）；只在導航時才需要算。
            let confined = navigating && vr::is_confined(&world, &r.body);
            r.stuck_timer = vr::update_stuck_timer(r.stuck_timer, moved, navigating, confined, dt);
            if r.stuck_timer >= vr::STUCK_SECS {
                let how = vr::rescue_resident(
                    &world, &mut r.body, r.home_x, r.home_z, vr::UNSTUCK_MAX_LIFT,
                );
                // 脫困後重置：清採集任務、目標設在腳邊、歇一下、清卡住計時。
                r.stuck_timer = 0.0;
                r.gather = None;
                r.target_x = r.body.x;
                r.target_z = r.body.z;
                r.wait_timer = 1.0;
                let bubble = match how {
                    vr::Rescue::LiftedUp => "唔…爬出來了！",
                    vr::Rescue::SentHome => "（回到熟悉的地方）",
                };
                if r.say.is_empty() {
                    r.say = bubble.to_string();
                    r.say_timer = SAY_SECS;
                }
                rescue_events.push((r.name, how));
                tracing::info!(resident = %r.id, ?how, "voxel 居民卡住 → 脫困/送回");
            }

            // 思考排程（蒐集快照，spawn 留到鎖外）。整地任務/跟隨/跑腿中不排程思考（她專心做那件事）。
            r.think_timer -= dt;
            if r.think_timer <= 0.0
                && !directed_snaps.contains_key(&r.id)
                && r.follow.is_none()
                && r.fetch.is_none()
            {
                r.think_timer = npc_agent_wire::THINK_INTERVAL_SECS;
                think_jobs.push((r.id.clone(), r.name, r.persona, r.body.x, r.body.z));
            }

            // 建造冷卻倒數（蓋家鬼打牆頻率保險）：剛完工的居民這段時間不動工新建物。
            if r.build_cooldown > 0.0 {
                r.build_cooldown -= dt;
            }

            // agency tick 倒數；到期且「沒在採集、沒在整地、沒在跟隨、沒在跑腿、沒在建造冷卻」時
            // 才加入候選（做事中不打斷、交給技能跑完）。只收快照，實際放塊 / 決定活動在鎖外執行。
            r.build_tick -= dt;
            if r.build_tick <= 0.0
                && r.build_cooldown <= 0.0
                && r.gather.is_none()
                && !directed_snaps.contains_key(&r.id)
                && r.follow.is_none()
                && r.fetch.is_none()
            {
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
        // 先收集快照（idx, id, name, x, z, social_cooldown, is_saying, asleep）避免借用衝突。
        // 睡覺 v1（739）：睡著的居民不發起也不被搭話（別讓她在夢裡開口）。
        let snaps: Vec<(usize, String, &'static str, f32, f32, f32, bool, bool)> =
            residents.iter().enumerate().map(|(i, r)| {
                (i, r.id.clone(), r.name, r.body.x, r.body.z, r.social_cooldown, !r.say.is_empty(), r.asleep)
            }).collect();

        let mut init_pair: Option<(usize, usize)> = None;
        'scan: for i in 0..snaps.len() {
            // 發起者：冷卻到期、目前沒在說話、沒睡著。
            if snaps[i].6 || snaps[i].5 > 0.0 || snaps[i].7 {
                continue;
            }
            for j in 0..snaps.len() {
                if i == j { continue; }
                // 目標：沒在說話（避免打斷對方）、沒睡著、且在範圍內。
                if snaps[j].6 || snaps[j].7 { continue; }
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

        // 2d) 打氣到達處理（ROADMAP 679）：把到達事件轉化成 pending_response + 收集記憶/Feed data。
        // 在 per-resident 迴圈結束後處理，避免雙重 iter_mut 借用衝突。
        // 格式：(happy_id, happy_name, lonely_rid) → 找到 lonely → 設 pending_response。
        for (happy_id, happy_name, lonely_rid) in &cheer_arrive_pending {
            if let Some(lonely) = residents.iter_mut().find(|r| r.id == *lonely_rid) {
                // 讓 lonely 居民幾秒後冒感謝泡泡（沿用社交回應機制）。
                lonely.pending_response = Some((
                    happy_id.clone(),
                    happy_name.to_string(),
                    vrel::RESPONSE_DELAY_SECS,
                ));
                cheer_arrive_done.push((
                    happy_id.clone(),
                    happy_name,
                    lonely_rid.clone(),
                    lonely.name,
                ));
            }
        }

        // 2e) 打氣觸發掃描（ROADMAP 679）：找 (Joyful/Content, Lonely) 配對，設 cheer_target。
        // 批次計算所有居民心情（2 把短鎖，循序即釋，不巢狀），避免迴圈中反覆取鎖。
        // 鎖序：bonds 讀（即釋）→ memory 讀（即釋）。
        let all_res_ids_for_cheer: Vec<String> =
            (0..resident_count()).map(|j| format!("vox_res_{j}")).collect();
        let cheer_bond_counts: Vec<(usize, usize)> = {
            let bonds = hub().bonds.read().unwrap();
            all_res_ids_for_cheer
                .iter()
                .map(|rid| resident_bond_counts(&bonds, rid))
                .collect()
        }; // bonds 讀鎖釋放
        let cheer_mem_counts: Vec<usize> = {
            let mem = hub().memory.read().unwrap();
            all_res_ids_for_cheer
                .iter()
                .map(|rid| mem.memory_count(rid))
                .collect()
        }; // memory 讀鎖釋放
        let cheer_mood_tiers: Vec<voxel_mood::MoodTier> = cheer_bond_counts
            .into_iter()
            .zip(cheer_mem_counts.iter())
            .zip(residents.iter().map(|r| r.mood_boost_secs > 0.0))
            .map(|(((f, a), mc), boost)| {
                let tier = voxel_mood::compute_mood(f, a, *mc);
                // ROADMAP 681：補助期間心情提升一格，影響打氣觸發條件。
                if boost { voxel_mood::boost_mood(tier) } else { tier }
            })
            .collect();

        // 快照：(idx, id, x, z, cheer_cd, is_saying, has_cheer_target, has_clique_meet)
        let cheer_snaps: Vec<(usize, String, f32, f32, f32, bool, bool, bool)> = residents
            .iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    i,
                    r.id.clone(),
                    r.body.x,
                    r.body.z,
                    r.cheer_cooldown,
                    // 遠行中的居民（奧瑞在荒野）視同「忙碌」，既不當打氣發起者也不當被打氣對象（ROADMAP 756）。
                    !r.say.is_empty() || r.expedition.is_some(),
                    r.cheer_target.is_some(),
                    r.clique_meet.is_some(),
                )
            })
            .collect();

        let mut cheer_trigger: Option<(usize, f32, f32, String)> = None; // (happy_idx, lx, lz, lonely_id)
        'cheer_scan: for (hi, href, hx, hz, hcd, h_saying, h_has_target, h_gathering) in &cheer_snaps {
            // 打氣發起條件：冷卻到期、沒在說話、沒有既有打氣任務、沒在小圈子聚會中、心情 Joyful/Content。
            if *h_saying || *hcd > 0.0 || *h_has_target || *h_gathering {
                continue;
            }
            let h_tier = &cheer_mood_tiers[*hi];
            if !matches!(
                h_tier,
                voxel_mood::MoodTier::Joyful | voxel_mood::MoodTier::Content
            ) {
                continue;
            }
            for (li, lref, lx, lz, _, l_saying, _, l_gathering) in &cheer_snaps {
                if hi == li || *l_saying || *l_gathering {
                    continue;
                }
                // Lonely 同伴？
                if cheer_mood_tiers[*li] != voxel_mood::MoodTier::Lonely {
                    continue;
                }
                // 距離內？
                let dx = hx - lx;
                let dz = hz - lz;
                if dx * dx + dz * dz > vcheer::CHEER_RANGE * vcheer::CHEER_RANGE {
                    continue;
                }
                // 機率觸發（低頻，防洗版）。
                if rand::random::<f32>() < vcheer::CHEER_CHANCE {
                    cheer_trigger = Some((*hi, *lx, *lz, lref.clone()));
                    break 'cheer_scan;
                }
            }
        }
        if let Some((hi, lx, lz, lonely_id)) = cheer_trigger {
            residents[hi].cheer_target = Some((lx, lz, lonely_id.clone()));
            residents[hi].cheer_cooldown = vcheer::CHEER_COOLDOWN;
            // Feed 出發事件（鎖外 IO，稍後在 cheer_arrive_done 外層記）。
            // 注意：出發時不記 Feed（低調），只在抵達時記一筆（更有感）。
        }

        // 2f) 小圈子聚會觸發掃描（ROADMAP 711）：找互為老朋友的圈子，偶爾相約碰面。
        // 鎖序：bonds 讀（即釋），不與其他鎖巢狀。居民數極少，全兩兩查詢零效能疑慮。
        let all_res_ids_for_gather: Vec<String> =
            (0..resident_count()).map(|j| format!("vox_res_{j}")).collect();
        let tier_matrix: HashMap<(String, String), vbonds::BondTier> = {
            let bonds = hub().bonds.read().unwrap();
            let mut m = HashMap::new();
            for a in &all_res_ids_for_gather {
                for b in &all_res_ids_for_gather {
                    if a != b {
                        m.insert((a.clone(), b.clone()), resident_tier_of(&bonds, a, b));
                    }
                }
            }
            m
        }; // bonds 讀鎖釋放
        let cliques = vclique::find_friend_cliques(&all_res_ids_for_gather, |a, b| {
            *tier_matrix.get(&(a.to_string(), b.to_string())).unwrap_or(&vbonds::BondTier::Stranger)
        });
        if !cliques.is_empty() {
            // 只挑第一個（最大）圈子；圈子成員皆閒（無其他任務、冷卻已到期）才考慮觸發。
            let group = &cliques[0];
            let ready = group.iter().all(|gid| {
                residents.iter().find(|r| &r.id == gid).is_some_and(|r| {
                    r.clique_meet.is_none()
                        && r.cheer_target.is_none()
                        && r.visiting.is_none()
                        && r.expedition.is_none()
                        && r.follow.is_none()
                        && r.say.is_empty()
                        && r.clique_cooldown <= 0.0
                        && !directed_snaps.contains_key(gid)
                })
            });
            if ready && rand::random::<f32>() < vclique::GATHER_CHANCE {
                // 聚會點固定用圈子裡 id 最小那位的家域（純函式排序過的 group[0]）。
                let host_id = group[0].clone();
                let (gx, gz) = residents
                    .iter()
                    .find(|r| r.id == host_id)
                    .map(|r| (r.home_x, r.home_z))
                    .unwrap_or((0.0, 0.0));
                let tag = vclique::session_tag(group);
                for gid in group {
                    if let Some(r) = residents.iter_mut().find(|r| &r.id == gid) {
                        r.clique_meet = Some((gx, gz, tag.clone()));
                        r.clique_wait = 0.0;
                        r.clique_cooldown = vclique::GATHER_COOLDOWN_SECS;
                        r.target_x = gx;
                        r.target_z = gz;
                    }
                }
                // 出發不記 Feed（比照打氣，低調；抵達才有感）。
            }
        }

        // 2g) 小圈子聚會抵達判定（ROADMAP 711）：同場聚會全員都已站在聚會點附近 → 觸發。
        // 一個 tick 只處理一組觸發，避免複雜度；未到齊的組別留到下個 tick 再檢查。
        {
            use std::collections::BTreeMap;
            let mut by_tag: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            for (i, r) in residents.iter().enumerate() {
                if let Some((_, _, tag)) = &r.clique_meet {
                    by_tag.entry(tag.clone()).or_default().push(i);
                }
            }
            for (tag, idxs) in &by_tag {
                let expected = tag.split('+').count();
                if idxs.len() < expected {
                    continue; // 還有成員半途放棄逾時，這組已不完整，等它整組自然清空。
                }
                let all_here = idxs.iter().all(|&i| {
                    let r = &residents[i];
                    if let Some((gx, gz, _)) = &r.clique_meet {
                        let dx = r.body.x - gx;
                        let dz = r.body.z - gz;
                        (dx * dx + dz * dz).sqrt() < vclique::GATHER_ARRIVE_DIST
                    } else {
                        false
                    }
                });
                if all_here {
                    let members: Vec<(String, &'static str)> = idxs
                        .iter()
                        .map(|&i| (residents[i].id.clone(), residents[i].name))
                        .collect();
                    for &i in idxs {
                        let pick = ((residents[i].body.x as u32).wrapping_add(residents[i].body.z as u32)) as usize;
                        residents[i].clique_meet = None;
                        residents[i].clique_wait = 0.0;
                        residents[i].wait_timer = residents[i].wait_timer.max(4.0); // 站著聊一會兒，不馬上散去
                        if residents[i].say.is_empty() {
                            residents[i].say = vclique::gather_line(pick).to_string();
                            residents[i].say_timer = SAY_SECS;
                        }
                    }
                    clique_fire_events.push(members);
                    break; // 一個 tick 只觸發一組。
                }
            }
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

    // 4a-c) 打氣到達記憶落地（ROADMAP 679）：打氣者 + 被打氣者各寫一筆記憶、Feed 廣播。
    // 鎖序：memory 寫（即釋）×2；不巢狀；append-only 不破壞既有資料。
    for (happy_id, happy_name, lonely_rid, lonely_name) in &cheer_arrive_done {
        // 打氣者的記憶（去陪伴了某人）。
        let mem_for_cheerful = vcheer::cheer_memory_for_cheerful(lonely_name);
        let e1 = {
            let mut mem = hub().memory.write().unwrap();
            mem.add_memory(happy_id, lonely_name, &mem_for_cheerful)
        }; // memory 寫鎖釋放
        vmem::append_memory(&e1);
        // 被打氣者的記憶（某人來陪伴我）。
        let mem_for_lonely = vcheer::cheer_memory_for_lonely(happy_name);
        let e2 = {
            let mut mem = hub().memory.write().unwrap();
            mem.add_memory(lonely_rid, happy_name, &mem_for_lonely)
        }; // memory 寫鎖釋放
        vmem::append_memory(&e2);
        // Feed 廣播（鎖外 IO）。
        vfeed::append_feed(
            vcheer::FEED_KIND,
            happy_name,
            &format!("走到{lonely_name}身邊，給她送來了一句溫暖的話！"),
        );
    }

    // 4a-d) 小圈子聚會全員到齊落地（ROADMAP 711）：每位成員各寫一筆記憶（提及其他成員），
    // 廣播一則 Feed（提及全體成員名字）。鎖序：memory 寫（即釋）× 成員數；不巢狀。
    for members in &clique_fire_events {
        let all_names: Vec<&str> = members.iter().map(|(_, n)| *n).collect();
        for (id, name) in members {
            let others: Vec<&str> = all_names.iter().copied().filter(|n| *n != *name).collect();
            if others.is_empty() {
                continue;
            }
            let mem_line = vclique::gather_memory_line(&others);
            // 記憶結構的「player」欄位借用其他成員之一（沿用打氣/回禮同樣的重用手法），
            // 文字本身已完整提及所有其他成員名字。
            let e = {
                let mut mem = hub().memory.write().unwrap();
                mem.add_memory(id, others[0], &mem_line)
            }; // memory 寫鎖釋放
            vmem::append_memory(&e);
        }
        vfeed::append_feed(
            vclique::FEED_KIND,
            all_names[0],
            &format!("和{}難得聚在一起，說說笑笑", all_names[1..].join("、")),
        );
    }

    // 4b) 回禮事件落地（ROADMAP 667）：鎖已全釋放；mark → append → 加背包 → 廣播。
    // 鎖序：return_gifts 寫（即釋）→ inventory 寫（即釋）→ tx broadcast。
    for (rid, rname, pname, bid, qty, _msg) in &return_gift_events {
        // 標記已送（寫鎖即釋）。
        let entry = {
            hub().return_gifts.write().unwrap().mark_given(rid, pname)
        }; // return_gifts 寫鎖在此釋放
        vret::append_return_gift(&entry);
        // 加進玩家背包（寫鎖即釋）。
        let inv_entry = {
            hub().inventory.write().unwrap().give(pname, *bid, *qty)
        }; // inventory 寫鎖在此釋放
        vinv::append_inv(&inv_entry);
        // 廣播回禮事件（所有人收到；前端依 player 是否為自己決定是否顯示 toast）。
        let new_count = hub().inventory.read().unwrap().count(pname, *bid);
        // 回禮 v2：物品名走 item_name_zh，涵蓋她採集背包裡的任意方塊（煤/冰晶…），
        // 對木頭/種子的回退選項也給出相同字串，broadcast 與 Feed 一致。
        let iname = vgift::item_name_zh(*bid);
        let msg = serde_json::json!({
            "t": "return_gift",
            "resident_id": rid,
            "resident_name": rname,
            "player": pname,
            "item_id": bid,
            "item_name": iname,
            "qty": qty,
            "new_count": new_count,
        })
        .to_string();
        let _ = hub().tx.send(std::sync::Arc::new(msg));
        // Feed：記錄這個溫馨時刻。
        vfeed::append_feed(
            "居民回禮",
            rname,
            &format!("把{}份{}送給了{}", qty, iname, pname),
        );
    }

    // 收成回贈 v1（ROADMAP 755）：套用「那畦熟了的禮物菜園」事件——收成方塊回土、果實入你
    // 背包、移除這畦、持久化、廣播 toast + Feed，把「你送種子 → 她種下 → 世界長出 → 她收成 →
    // 又回到你手裡」這個閉環演完。鎖序：giftgarden 寫（即釋）→ append → delta 寫（set FarmSoil，
    // 即釋）→ broadcast → farm 寫清記錄（保險，即釋）→ inventory 寫（即釋）→ append → 廣播/Feed，
    // 全程短鎖循序不巢狀（守死鎖鐵律）。
    for (rid, rname, pname, pos, gx, gy, gz, crop) in &harvest_return_events {
        // 移除這畦（寫鎖即釋）+ 持久化移除事件；已被別的 tick 收掉就跳過（防重複套用）。
        let removed = { hub().giftgarden.write().unwrap().remove(pos) };
        let Some(rm_entry) = removed else { continue };
        vgg::append_gift_garden(&rm_entry);
        // 收成：作物方塊換回犁好的農地（比照玩家收成 Mature → FarmSoil），廣播 + 持久化。
        {
            let mut world = hub().deltas.write().unwrap();
            voxel::set_block(&mut world, *gx, *gy, *gz, Block::FarmSoil);
        } // deltas 寫鎖釋放
        broadcast_block(*gx, *gy, *gz, Block::FarmSoil);
        vbuild::append_world_block(*gx, *gy, *gz, Block::FarmSoil as u8);
        // 農地計時本應在成熟時已被 tick_farm 清掉，保險再清一次（idempotent，farm 寫鎖即釋）。
        let farm_e = { hub().farm.write().unwrap().remove(*gx, *gy, *gz) };
        if let Some(farm_e) = farm_e {
            vfarm::append_farm(&farm_e);
        }
        // 果實入你背包（寫鎖即釋）+ 持久化。
        let (bid, qty) = vgg::produce_gift(*crop);
        let inv_entry = { hub().inventory.write().unwrap().give(pname, bid, qty) };
        vinv::append_inv(&inv_entry);
        let new_count = hub().inventory.read().unwrap().count(pname, bid);
        let iname = vgift::item_name_zh(bid);
        // 廣播回贈事件（沿用 return_gift 前端 toast 管線，前端依 player 是否為自己決定顯示）。
        let msg = serde_json::json!({
            "t": "return_gift",
            "resident_id": rid,
            "resident_name": rname,
            "player": pname,
            "item_id": bid,
            "item_name": iname,
            "qty": qty,
            "new_count": new_count,
        })
        .to_string();
        let _ = hub().tx.send(std::sync::Arc::new(msg));
        // Feed：記錄這個閉環時刻。
        let cname = vgg::crop_name(*crop);
        vfeed::append_feed("收成回贈", rname, &vgg::harvest_feed_line(rname, pname, cname));
    }

    // 失效的禮物菜園誠實清帳（作物被玩家自己收成／破壞了）：移除 + 持久化，不回贈。
    for pos in &giftgarden_stale {
        let removed = { hub().giftgarden.write().unwrap().remove(pos) };
        if let Some(rm_entry) = removed {
            vgg::append_gift_garden(&rm_entry);
        }
    }

    // 5) 無鎖 spawn 思考（LLM）。整個 agent 思考可由 BUTFUN_NPC_AGENT=0 關掉，
    //    關掉後居民仍照常閒晃移動，只是不冒 LLM 心裡話/心願（零額外成本）。
    if npc_agent_wire::agents_enabled() {
        for (id, name, persona, x, z) in think_jobs {
            spawn_resident_think(id, name, persona, x, z);
        }
    }

    // 5b) 採集挖掘執行（agency v1·技能調用收尾）：居民走到資源旁 → 真的採 → 入小背包。
    //     鎖序：deltas 寫（即釋）→ broadcast → res_inv 寫（即釋）→ 持久化/Feed（鎖外）。
    //     **她真的在做事**：feed 出現「採集了草皮」——但**採地表不再留坑**：草採走 → 該格
    //     降級成裸土（實心平整），沙/土採走 → 格子維持同材料（無洞），只有石/木才留 Air
    //     （採礦道／砍半空樹幹本就合理）。回填塊由 `refill_after_gather` 決定，材料照樣入袋。
    for (rid, rname, gx, gy, gz, res) in gather_mines {
        // 只在目標方塊「現在仍是該資源」時才採（防別人先採走→空採）。
        let still_there = {
            let world = hub().deltas.read().unwrap();
            voxel::effective_block_at(&world, gx, gy, gz) == res.block()
        }; // deltas 讀鎖釋放
        if !still_there {
            continue;
        }
        // 採集回填：地表覆蓋層採走後回填裸土/同材料（不留坑），石/木仍留 Air。
        let refill = res.refill_after_gather();
        // 回填塊與原資源塊不同才需真的改世界（沙→沙、土→土 無變化 → 跳過寫入/廣播/持久化，
        // 但材料照樣入袋）——地表維持平整、也省掉無謂的 delta 與廣播。
        if refill != res.block() {
            {
                let mut world = hub().deltas.write().unwrap();
                voxel::set_block(&mut world, gx, gy, gz, refill);
            } // deltas 寫鎖釋放
            broadcast_block(gx, gy, gz, refill);
            // 水流動：只有回填成 Air（石/木）才開出缺口讓水流過來；回填實心地表不需要。
            if refill == Block::Air {
                enqueue_water_around(gx, gy, gz);
            }
            // 持久化這次世界改動（重啟後回填/挖掉的結果還在）。
            vbuild::append_world_block(gx, gy, gz, refill as u8);
        }
        // 入居民小背包（純記憶體）——採集產出的材料數量不變（回填不影響入袋）。
        {
            let mut inv = hub().res_inv.write().unwrap();
            *inv.entry(rid.clone()).or_default().entry(res.block_id()).or_insert(0) += 1;
        } // res_inv 寫鎖釋放
        // 里程碑 Feed（真實事件、低頻、不洗版）。
        vfeed::append_feed("採集", rname, &format!("採集了{}", res.display_name()));
        // 冒一句採集泡泡（不打斷其他話）。
        say_updates.push((rid.clone(), format!("採到{}了～", res.display_name())));
    }

    // 5b-1a) 跑腿採集·找下一個目標（指令→任務第三刀收尾）：還沒鎖定資源的居民，
    //   找一次最近的指定資源；找不到 → 已帶著的份就直接送去交付、否則老實放棄整個任務
    //   （不無窮重試）。鎖序：deltas 讀（即釋）→ residents 寫（即釋），逐位居民各自短取即釋。
    for (rid, rx, _ry, rz, resource) in fetch_search_candidates {
        let found = {
            let world = hub().deltas.read().unwrap();
            vskill::find_nearest_resource_of(&world, rx, rz, vskill::GATHER_MAX_RADIUS, resource)
        }; // deltas 讀鎖釋放
        let mut residents = hub().residents.write().unwrap();
        if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
            match found {
                Some((tx, ty, tz)) => {
                    r.gather = Some(GatherSkill {
                        resource,
                        tx,
                        ty,
                        tz,
                        timeout: vskill::GATHER_TIMEOUT_SECS,
                    });
                }
                None => {
                    let carried = r.fetch.as_ref().map_or(0, |f| f.carried);
                    if carried > 0 {
                        if let Some(f) = r.fetch.as_mut() {
                            f.remaining = 0; // 帶著已採到的先回去交差
                        }
                    } else {
                        r.fetch = None;
                        if r.say.is_empty() {
                            let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                            r.say = vfetch::fail_line(resource.display_name(), pick).chars().take(40).collect();
                            r.say_timer = SAY_SECS;
                        }
                    }
                }
            }
        }
    } // residents 寫鎖逐次釋放

    // 5b-1b) 跑腿採集·交付落地（指令→任務第三刀收尾）：把居民帶回來的材料真的交到玩家手上。
    //   鎖序：inventory 寫（即釋）→ memory 寫（即釋）→ broadcast + Feed（鎖外 IO）。
    for (rid, rname, pname, resource, delivered, requested) in &fetch_deliver_events {
        if *delivered == 0 {
            continue; // 一份都沒採到理論上不該走到交付（deadline 分支已過濾），安全跳過。
        }
        let bid = resource.block_id();
        let inv_entry = {
            hub().inventory.write().unwrap().give(pname, bid, *delivered)
        }; // inventory 寫鎖釋放
        vinv::append_inv(&inv_entry);
        let new_count = hub().inventory.read().unwrap().count(pname, bid);
        let iname = resource.display_name();
        let pick = rname.len() + *delivered as usize;
        let line = vfetch::deliver_line(iname, *requested, *delivered, pick);
        // 居民記得幫你跑了這趟腿（互動有後果的另一種形式）。
        let mem_line = format!("幫{pname}跑腿採了{delivered}份{iname}，親手交到了對方手上");
        let e = {
            let mut mem = hub().memory.write().unwrap();
            mem.add_memory(rid, pname, &mem_line)
        }; // memory 寫鎖釋放
        vmem::append_memory(&e);
        let msg = serde_json::json!({
            "t": "fetch_delivered",
            "resident_id": rid,
            "resident_name": rname,
            "player": pname,
            "item_id": bid,
            "item_name": iname,
            "qty": delivered,
            "new_count": new_count,
            "line": line.clone(),
        })
        .to_string();
        let _ = hub().tx.send(std::sync::Arc::new(msg));
        vfeed::append_feed("跑腿交付", rname, &format!("把{delivered}份{iname}交給了{pname}"));
        {
            let mut residents = hub().residents.write().unwrap();
            if let Some(r) = residents.iter_mut().find(|r| &r.id == rid) {
                if r.say.is_empty() {
                    r.say = line.chars().take(40).collect();
                    r.say_timer = SAY_SECS;
                }
            }
        } // residents 寫鎖釋放
    }

    // 5b-2) 整地任務執行（指令→任務 + 整地技能 v1 收尾）：**她真的照玩家的話做事**。
    //   ① 先扣所有任務的逾時（走不到/整不完就放棄，避免任務永不釋放）。
    //   ② 對本 tick 已抵達工地的居民：算這批要改的方塊 → 套用（delta 寫 + 廣播 + 水流 + 持久化）
    //      → 推進 cursor；整完 → 冒「整好囉」+ Feed。鎖序全短取即釋、不巢狀、不 await（守鐵律）。
    //   地形改動走既有 world delta 持久化（vbuild::append_world_block），重啟後整過的地還在。
    {
        // ① 逾時遞減 + 清逾時任務（短鎖即釋）。鋪面任務的 deadline 是「無進展門檻」
        //    （有挖到料/合成/鋪一批都會續期，見 pave_worker_tick），到期＝真的卡死。
        let expired: Vec<(String, bool)> = {
            let mut tasks = hub().directed_tasks.write().unwrap();
            for t in tasks.values_mut() {
                t.deadline -= RESIDENT_DT;
            }
            let dead: Vec<(String, bool)> = tasks
                .iter()
                .filter(|(_, t)| t.deadline <= 0.0)
                .map(|(rid, t)| (rid.clone(), t.pave.is_some()))
                .collect();
            for (rid, _) in &dead {
                tasks.remove(rid);
            }
            dead
        }; // directed_tasks 寫鎖釋放
        for (rid, was_pave) in expired {
            let rname = resident_name_of(&rid);
            if was_pave {
                vfeed::append_feed("鋪面", rname, "這片地鋪到一半卡住了，我盡力了…");
                say_updates.push((rid, "唉…這片地一時鋪不下去了…".to_string()));
            } else {
                vfeed::append_feed("整地", rname, "這塊地太難整了，我盡力了…");
                say_updates.push((rid, "唉…這塊地我一個人一時整不太動…".to_string()));
            }
        }
    }
    // ② 已抵達工地的居民：整地一批。
    // **全域每 tick 總柱數上限**（守 FPS/伺服器 tick）：多位協調居民合計每 tick 處理的柱數
    // 不超過 vdt::MAX_LEVEL_COLUMNS_PER_TICK——用光就把剩下的居民留到下個 tick（逐步變平、不爆）。
    let mut cols_budget = vdt::MAX_LEVEL_COLUMNS_PER_TICK;
    for rid in &level_workers {
        if cols_budget == 0 {
            break; // 本 tick 總柱數上限用光 → 其餘居民下個 tick 再整。
        }
        // 讀任務快照（可能已被上面逾時清掉 → None 就跳過）。
        let task_opt = { hub().directed_tasks.read().unwrap().get(rid).cloned() };
        let Some(mut task) = task_opt else { continue };
        // 鋪面任務走自己的推進器（備料/合成/礦井/誠實扣料），整地續走下方原路（零回歸）。
        if task.pave.is_some() {
            let used = pave_worker_tick(rid, task, cols_budget, &mut say_updates);
            cols_budget = cols_budget.saturating_sub(used);
            continue;
        }
        // 算這批要改的方塊 + 新 cursor（deltas 讀鎖即釋；批量受剩餘全域上限剪裁）。
        let (changes, next_cursor) = {
            let world = hub().deltas.read().unwrap();
            vdt::level_step_capped(&world, &task, cols_budget)
        }; // deltas 讀鎖釋放
        // 扣掉本位居民這批實際處理的柱數（next_cursor 前進量）。
        cols_budget = cols_budget.saturating_sub(next_cursor.saturating_sub(task.cursor));
        // 居民當前腳底（安全過濾：別把她自己埋了）。
        let body = {
            let res = hub().residents.read().unwrap();
            res.iter().find(|r| &r.id == rid).map(|r| (r.body.x, r.body.y, r.body.z))
        }; // residents 讀鎖釋放
        for (x, y, z, b) in changes {
            // 安全：實心填塊若與居民身體重疊 → 跳過（沿用可逃精神，不自埋自困）。
            if b.is_solid() {
                if let Some((px, py, pz)) = body {
                    if vdt::cell_in_body(x, y, z, px, py, pz) {
                        continue;
                    }
                }
            }
            {
                let mut world = hub().deltas.write().unwrap();
                voxel::set_block(&mut world, x, y, z, b);
            } // deltas 寫鎖釋放
            broadcast_block(x, y, z, b);
            // 水流動：整地削出的缺口/填出的堤都可能改變水路 → 喚醒鄰格重算。
            enqueue_water_around(x, y, z);
            // 持久化這次世界改動（重啟後整過的地還在）。
            vbuild::append_world_block(x, y, z, b as u8);
        }
        // 推進 cursor / 收尾。
        task.cursor = next_cursor;
        let rname = resident_name_of(rid);
        if task.is_complete() {
            // 取 requester 的玩家位置，計算相對方位（短鎖即釋）。
            let (done_dir, done_steps) = {
                let players = hub().players.read().unwrap();
                let req_pos = players.values().find(|p| p.name == task.requester).map(|p| (p.x, p.z));
                drop(players);
                if let Some((px, pz)) = req_pos {
                    vdt::cardinal_direction(px, pz, task.cx as f32, task.cz as f32)
                } else {
                    (String::new(), 0)
                }
            }; // players 讀鎖已釋放
            let done_cx = task.cx;
            let done_cz = task.cz;
            hub().directed_tasks.write().unwrap().remove(rid);
            vfeed::append_feed("整地", rname, &format!("把（{done_cx},{done_cz}）那塊地整平了！"));
            say_updates.push((rid.clone(), vdt::level_done_line(&done_dir, done_steps, done_cx, done_cz)));
        } else {
            let pct = task.progress_pct();
            hub().directed_tasks.write().unwrap().insert(rid.clone(), task);
            // 過半冒一句進度泡泡（低頻、不洗版）。
            if (45..=55).contains(&pct) {
                say_updates.push((rid.clone(), "整地中…快一半了～".to_string()));
            }
        }
    }

    // 5b-3) 協調整地整體完成偵測（B 階段·居民↔居民協調）：某件協調任務的所有成員子任務
    //   都已消失（整完或逾時釋放）→ 這片大地整平了 → 領隊冒「大家一起整平了」+ Feed。
    //   鎖序：directed_tasks 讀（即釋）→ coordinated_tasks 寫（即釋）；不巢狀、不 await。
    {
        // 目前仍在跑的整地任務 id 集合。
        let active: std::collections::HashSet<String> =
            { hub().directed_tasks.read().unwrap().keys().cloned().collect() };
        // 撿出整體完成的協調任務（並從清單移除）。
        let finished: Vec<CoordinatedLevelTask> = {
            let mut coords = hub().coordinated_tasks.write().unwrap();
            let mut done = Vec::new();
            coords.retain(|c| {
                if c.all_done(&active) {
                    done.push(c.clone());
                    false
                } else {
                    true
                }
            });
            done
        }; // coordinated_tasks 寫鎖釋放
        for c in finished {
            if let Some(leader) = c.members.first() {
                let lname = resident_name_of(leader);
                // 取 requester 的玩家位置，計算相對方位（短鎖即釋）。
                let (done_dir, done_steps) = {
                    let players = hub().players.read().unwrap();
                    let req_pos =
                        players.values().find(|p| p.name == c.requester).map(|p| (p.x, p.z));
                    drop(players);
                    if let Some((px, pz)) = req_pos {
                        vdt::cardinal_direction(px, pz, c.cx as f32, c.cz as f32)
                    } else {
                        (String::new(), 0)
                    }
                }; // players 讀鎖已釋放
                if let Some(mat) = c.pave {
                    // 協調鋪面完工：領隊冒泡 + Feed 帶材料名與方位。
                    let mname = vdt::pave_material_name(mat);
                    say_updates.push((
                        leader.clone(),
                        vdt::coord_pave_done_line(mname, &done_dir, done_steps, c.cx, c.cz),
                    ));
                    vfeed::append_feed(
                        "鋪面",
                        lname,
                        &format!("和大家齊心，把（{},{}）附近一大片地鋪上了{mname}！", c.cx, c.cz),
                    );
                } else {
                    // 協調整地完工：領隊冒泡 + Feed 帶方位。
                    say_updates.push((
                        leader.clone(),
                        vdt::coord_level_done_line(&done_dir, done_steps, c.cx, c.cz),
                    ));
                    vfeed::append_feed(
                        "整地",
                        lname,
                        &format!("和大家齊心，把（{},{}）附近一大片地整平了！", c.cx, c.cz),
                    );
                }
            }
        }
    }

    // 5c) 卡住脫困/送回的 Feed（鎖已釋放）：送回家域是較顯著的事件，記一筆讓玩家看得到；
    //     往上脫困是小事不洗版。實測證據：居民不再凍在原地、會自己脫困。
    for (rname, how) in rescue_events {
        if how == vr::Rescue::SentHome {
            vfeed::append_feed("脫困", rname, "卡住了，回到家域重新開始");
        }
    }

    // 5c-2) 重返心中的牌子 Feed（讀牌 v3，ROADMAP 743）：居民專程走回一塊念念不忘的牌子前
    // 駐足時記一筆動態，讓沒在現場的玩家也看得到「我立的牌子把她引了回來」。
    for (rname, quote) in &pilgrimage_feed {
        vfeed::append_feed("重返", rname, &vreadsign::revisit_feed_line(quote));
    }

    // 5c-2a) 遠行探野 Feed（遠行探野 v1，ROADMAP 756）：某居民啟程遠行／遠行歸來時記一筆動態，
    // 讓沒在現場的玩家也讀得到「居民的足跡散進了荒野」——世界不再只圍著主城打轉。
    for (rname, detail) in &expedition_feed {
        vfeed::append_feed("遠行", rname, detail);
    }

    // 5c-2a') 邊陲營火路標（遠行 v2，PLAN_ETHERVOX item 7「在遠方留下痕跡」）：居民抵達邊陲時
    // 親手升起一堆營火路標——世界第一次因居民散佈而在主城外長出實體痕跡，日後任何玩家路過都會撞見。
    // 鎖序（嚴守短鎖即釋、鎖外 IO）：地表 surface_y 是純程序地形函式（不讀 delta，可鎖外算）→
    // deltas 讀鎖快照落點（判定該處是否已有營火＝冪等、落點是否可立＝陸地非水非樹）即釋 →
    // deltas 寫鎖批次落地 6 塊即釋 → 鎖外廣播 + 持久化 append + memory 短寫 + Feed。不巢狀、不持鎖 await。
    for (rid, rname, bx, bz, bearing) in &expedition_campfires {
        let sy = vbuild::surface_y(*bx, *bz);
        // 落點可立否 + 是否已有營火（冪等）——一次讀鎖快照即釋。
        let (placeable, already) = {
            let _w = hub().deltas.read().unwrap();
            let hearth_slot = voxel::block_at(*bx, sy, *bz); // 灶台要放的那格
            let ground = voxel::block_at(*bx, sy - 1, *bz); // 腳下那格
            // 陸地（腳下實心）且灶台格是空氣（不覆水面、不塞進樹幹）才立；避免立在海上或樹裡。
            let placeable = ground.is_solid() && matches!(hearth_slot, Block::Air);
            let (fx, fy, fz) = vexp::campfire_flame_pos(*bx, sy, *bz);
            // 該落點已有這堆營火的火把 → 判定為已立過，跳過（同址不重複堆疊、replay 不重放）。
            let already = voxel::block_at(fx, fy, fz) == Block::Torch;
            (placeable, already)
        }; // deltas 讀鎖釋放
        if !placeable {
            // 落點不可立（海上／樹裡）→ 營火與小棚都不搭。
            continue;
        }
        if !already {
            let blocks = vexp::campfire_blocks(*bx, sy, *bz);
            // 寫鎖批次落地（一次持有、寫完即釋、不 await）。
            {
                let mut world = hub().deltas.write().unwrap();
                for &(x, y, z, b) in &blocks {
                    voxel::set_block(&mut world, x, y, z, b);
                }
            } // deltas 寫鎖釋放
            // 鎖外收尾：廣播 + 走既有 world delta 持久化（重啟後營火還在）。
            for &(x, y, z, b) in &blocks {
                broadcast_block(x, y, z, b);
                vbuild::append_world_block(x, y, z, b as u8);
            }
            // 升起營火昇華成一筆記憶（掛遠行哨兵鍵，日記／內心可引用）。
            let summary = vexp::campfire_memory_summary(bearing);
            let entry = hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, vexp::EXPEDITION_MEMORY_PLAYER, &summary);
            vmem::append_memory(&entry);
            // 一則動態，讓沒在現場的玩家也讀得到「居民在荒野留下了記號」。
            vfeed::append_feed("遠行", rname, &vexp::campfire_feed_line(bearing));
        }

        // 5c-2a'') 邊陲紮營小棚（遠行 v3，ROADMAP 758「在遠方紮營、雛形第二個家」）：抵達據點時，
        // 居民在營火旁搭起一座簡易 lean-to（背牆＋頂＋一張床）——荒野裡第一個過夜的地方。**獨立於
        // 營火冪等**（不因營火已存在而略過：v3 部署前既有的營地也會在下一趟回訪時補上小棚）。
        // 鎖序同營火：surface_y 純函式鎖外算 → deltas 讀鎖快照冪等/可立即釋 → 寫鎖批次落地即釋 →
        // 鎖外廣播＋持久化＋memory 短寫＋Feed。每塊逐一「該格空氣才放」，起伏地形不覆蓋既有方塊。
        let (ax, az) = vexp::shelter_anchor(*bx, *bz);
        let ay = vbuild::surface_y(ax, az);
        let shelter = vexp::shelter_blocks(ax, ay, az);
        // 冪等判定 + 逐格可立快照——一次讀鎖即釋。
        let (bed_exists, placeable_cells) = {
            let _w = hub().deltas.read().unwrap();
            let (bx2, by2, bz2) = vexp::shelter_bed_pos(ax, ay, az);
            let bed_exists = voxel::block_at(bx2, by2, bz2) == Block::Bed;
            // 逐格：只有目標格目前是空氣才放（不覆蓋地形/水/樹/既有建物）。
            let cells: Vec<(i32, i32, i32, Block)> = shelter
                .iter()
                .copied()
                .filter(|&(x, y, z, _)| matches!(voxel::block_at(x, y, z), Block::Air))
                .collect();
            (bed_exists, cells)
        }; // deltas 讀鎖釋放
        // 已搭過（床在）或這趟一塊都放不下（全非空氣）→ 不重複、不空搭。
        if !bed_exists && !placeable_cells.is_empty() {
            {
                let mut world = hub().deltas.write().unwrap();
                for &(x, y, z, b) in &placeable_cells {
                    voxel::set_block(&mut world, x, y, z, b);
                }
            } // deltas 寫鎖釋放
            for &(x, y, z, b) in &placeable_cells {
                broadcast_block(x, y, z, b);
                vbuild::append_world_block(x, y, z, b as u8);
            }
            // 只有真的把床搭起來（雛形第二個家成立）才記憶＋播報，避免半截小棚也洗版。
            if placeable_cells
                .iter()
                .any(|&(_, _, _, b)| b == Block::Bed)
            {
                let summary = vexp::shelter_memory_summary(bearing);
                let entry = hub().memory.write().unwrap().add_memory(
                    rid,
                    vexp::EXPEDITION_MEMORY_PLAYER,
                    &summary,
                );
                vmem::append_memory(&entry);
                vfeed::append_feed("遠行", rname, &vexp::shelter_feed_line(bearing));
            }
        }
    }

    // 5c-2a''') 遠行帶回的邊陲風物（遠行 v5，ROADMAP 761「不同地方採到不同資源」）：遠行歸來的居民
    // 從邊陲群系帶回一件當地特產風物（草原小樹苗／森林枝葉／沙漠仙人掌／雪原冰晶），種在自家門前的
    // 一小列紀念花圃——玩家看到居民家門漸漸長出一排來自遠方的紀念，每件都對應牠去過的一個地方。
    // 四種群系各佔花圃一格、每格只種一次（冪等）。鎖序同營火：surface_y 純函式鎖外算 → deltas 讀鎖
    // 快照（可立/冪等）即釋 → 寫鎖落一塊即釋 → 鎖外廣播＋持久化＋memory 短寫＋Feed。不巢狀、不持鎖 await。
    for (rid, rname, home_x, home_z, biome, bearing) in &expedition_keepsakes {
        let (kx, kz) = vexp::keepsake_pos(*home_x, *home_z, *biome);
        let ky = vbuild::surface_y(kx, kz);
        let block = vexp::keepsake_block(*biome);
        // 可立否 + 冪等——一次讀鎖快照即釋。腳下實心（陸地）且該格是空氣（沒種過、也不覆蓋既有物）才種。
        // 已種過時該格＝風物方塊（非空氣）→ placeable 為 false 自然跳過（同址不重複、replay 不重放）。
        let placeable = {
            let _w = hub().deltas.read().unwrap();
            let slot = voxel::block_at(kx, ky, kz); // 風物要放的那格
            let ground = voxel::block_at(kx, ky - 1, kz); // 腳下那格
            ground.is_solid() && matches!(slot, Block::Air)
        }; // deltas 讀鎖釋放
        if placeable {
            {
                let mut world = hub().deltas.write().unwrap();
                voxel::set_block(&mut world, kx, ky, kz, block);
            } // deltas 寫鎖釋放
            broadcast_block(kx, ky, kz, block);
            vbuild::append_world_block(kx, ky, kz, block as u8);
            // 種下風物昇華成一筆記憶（掛遠行哨兵鍵，日記／內心可引用）。
            let summary = vexp::keepsake_memory_summary(bearing, *biome);
            let entry = hub().memory.write().unwrap().add_memory(
                rid,
                vexp::EXPEDITION_MEMORY_PLAYER,
                &summary,
            );
            vmem::append_memory(&entry);
            // 一則動態，讓沒在現場的玩家也讀到「牠從遠方帶回了什麼、種在家門前」。
            vfeed::append_feed("遠行", rname, &vexp::keepsake_feed_line(bearing, *biome));
        }
    }

    // 5c-2b) 登門串門子·抵達處理（登門串門子 v1，ROADMAP 751）：居民朝聖抵達的其實是某位鄰居親手
    // 立的家牌（750 認得的）時，把這趟走過去當成一次真正的「登門拜訪」——① 情誼帳本 record_visit：
    // 這趟登門讓彼此更熟，可能因此升級成老朋友（沿用 672 的鎖序：寫鎖短取即釋 → 若升級再讀鎖 save
    // + 里程碑 Feed）；② 掛在那位鄰居名下的記憶（日記／回想可引用）；③ 一則「串門子」Feed。
    for (rid, rname, neighbor) in &neighbor_visit_arrivals {
        // 情誼加溫：這趟登門記進情誼帳本（visitor=登門者、host=被登門的鄰居）。
        let (tier, tier_changed) = {
            let mut bonds = hub().bonds.write().unwrap();
            bonds.record_visit(rname, neighbor)
        }; // bonds 寫鎖釋放
        if tier_changed {
            // 升級里程碑：持久化 + Feed 廣播——玩家看見「常繞來串門子，兩位居民漸漸成了老朋友」。
            {
                let bonds = hub().bonds.read().unwrap();
                vbonds::save_bonds(&bonds);
            } // bonds 讀鎖釋放
            let milestone = vbonds::tier_up_line(tier, rname, neighbor);
            vfeed::append_feed("居民情誼", rname, &milestone);
            // 社交足跡（673）：情誼升級時在登門者記憶裡也留一筆，讓日記有這段情誼的痕跡。
            let social_mem = vbonds::bond_social_memory(neighbor, tier);
            if !social_mem.is_empty() {
                let entry = hub()
                    .memory
                    .write()
                    .unwrap()
                    .add_memory(rid, neighbor, &social_mem);
                vmem::append_memory(&entry);
            } // memory 寫鎖釋放
        }
        // 掛在那位鄰居名下的「登門串門子」記憶（無論是否升級都記，讓這趟登門在日記留下痕跡）。
        let entry = {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, neighbor, &vneighvisit::visit_memory(neighbor))
        }; // memory 寫鎖釋放
        vmem::append_memory(&entry);
        // 城鎮動態 Feed：某居民特地登門找某鄰居串門子（鄰里往來第一次被世界看板記下）。
        vfeed::append_feed("串門子", rname, &vneighvisit::visit_feed(rname, neighbor));
    }

    // 5c-2c) 登門遇主人在家·迎客處理（登門遇主人在家 v1，ROADMAP 752）：訪客登門抵達時那位鄰居
    // 正好也在家（站在自家牌子附近）——這趟登門不再撲空，補上主人這一側的迎客反應：
    // ① 主人若沒在說別的話就冒一句點名訪客的迎客泡泡；② 把「某鄰居特地登門、我正好在家、親自迎了迎」
    // 記成一筆掛在訪客名下的記憶（鄰里往來第一次從主人這一側也留下痕跡）；③ 一則「在家迎客」Feed。
    // 情誼**不在此重複記帳**（751 抵達時已 record_visit 這對）。鎖序：先短取 residents 寫鎖設泡泡並
    // 取得主人 id，即釋；再各自短取 memory 寫鎖、IO 在鎖外——不巢狀、守死鎖鐵律。
    for &(guest, ref host, pick) in &hosted_meetings {
        // 短寫鎖：主人若閒著沒在說話，回一句迎客暖招呼；順手取主人 id 供記憶鍵用。
        let host_id: Option<String> = {
            let mut rs = hub().residents.write().unwrap();
            rs.iter_mut().find(|r| r.name == host.as_str()).map(|h| {
                if h.say.is_empty() {
                    h.say = vhosted::host_welcome_line(guest, pick);
                    h.say_timer = SAY_SECS;
                }
                h.id.clone()
            })
        }; // residents 寫鎖釋放
        if let Some(hid) = host_id {
            // 主人側記憶：掛在訪客名下（沿用「跨居民記憶鍵到對方名字」慣例，比照 748/750）。
            let entry = {
                hub()
                    .memory
                    .write()
                    .unwrap()
                    .add_memory(&hid, guest, &vhosted::host_welcome_memory(guest))
            }; // memory 寫鎖釋放
            vmem::append_memory(&entry);
        }
        // 城鎮動態 Feed：主人正好在家、親自迎接了登門的訪客。
        vfeed::append_feed("在家迎客", host, &vhosted::hosted_feed(guest, host));
    }

    // 5c-2d) 登門撲空·留心意入列（登門撲空留心意 v1，ROADMAP 763）：訪客登門撲空（主人不在家）時，
    // 把訪客名字留進那位主人的門口心意佇列——去重（同一人連來只留一份）＋上限保護。日後主人回到自家
    // 附近閒著時逐一感應。鎖序：一次短取 residents 寫鎖批次入列即釋、不巢狀、無 IO。
    if !calling_cards.is_empty() {
        let mut rs = hub().residents.write().unwrap();
        for (host, guest) in &calling_cards {
            if let Some(h) = rs.iter_mut().find(|r| r.name == host.as_str()) {
                vcard::enqueue_caller(&mut h.pending_callers, guest);
            }
        }
    } // residents 寫鎖釋放

    // 5c-2e) 登門撲空·主人回家感應處理（登門撲空留心意 v1，ROADMAP 763）：主人回到自家附近、閒著時
    // 感應到門口一張心意（rr 迴圈已設好泡泡並自佇列取出訪客名）——補上主人側的痕跡：① 把「某某趁我不在
    // 時特地來找過我」記成一筆掛在那位訪客名下的記憶（撲空這一側第一次也在主人心裡留下溫度）；② 一則
    // 「回家發現有人來找過」Feed。情誼**不在此重複記帳**（751 抵達時已 record_visit 這對）。
    // 鎖序：memory 寫鎖各自短取即釋、IO 在鎖外——不巢狀、守死鎖鐵律。
    for (rid, rname, guest) in &callingcard_notices {
        // 主人側記憶：掛在訪客名下（沿用「跨居民記憶鍵到對方名字」慣例，比照 748/750/752）。
        let entry = {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, guest, &vcard::notice_memory(guest))
        }; // memory 寫鎖釋放
        vmem::append_memory(&entry);
        // 城鎮動態 Feed：主人回到家，發現有人趁自己不在時來找過。
        vfeed::append_feed("回家發現", rname, &vcard::notice_feed(rname, guest));
    }

    // 5c-2f) 你送的食物她會細細享用 Feed（ROADMAP 765）：居民在閒暇時真的享用了玩家稍早送的食物
    //（泡泡與心情補助已於鎖內設好）——鎖外補一則城鎮動態，讓不在線上的玩家回來也讀得到「牠好好享用了
    // 你的心意」。餵食第一次有了「被好好享用」的溫暖回響。鎖外純 IO、不巢狀、守死鎖鐵律。
    for (rname, giver, food) in &savor_feeds {
        vfeed::append_feed("享用", rname, &vsavor::savor_feed_line(rname, giver, food));
    }

    // 5c-2g) 自我印象 Feed（自我印象 v1，ROADMAP 770）：居民閒下來回望這一路、昇華出「我成了個怎樣
    // 的人」的自我概念（泡泡已於鎖內設好），鎖外補一則城鎮動態，讓不在線的玩家回來也讀得到「牠如何看
    // 自己」——記憶昇華進世界的日記牆。鎖外純 IO、不巢狀、守死鎖鐵律；文字已是純模板、無記憶原文。
    for (rname, detail) in &self_image_feeds {
        vfeed::append_feed("自省", rname, detail);
    }

    // 5c-2g-2) 名號 Feed（居民為你取一個名號 v1）：居民打招呼時第一次為某玩家昇華出一個名號、或
    // 名號改換（名號招呼已於鎖內設好），鎖外補一則城鎮動態，讓玩家（與不在線者回來）讀到「在某居民
    // 眼中，我漸漸成了造物者／慷慨的人…」——你的作為聚合成世界對你的稱呼。鎖外純 IO、不巢狀、守死鎖
    // 鐵律；文字為純模板、只嵌玩家顯示名、無記憶原文。
    for (rname, detail) in &epithet_feeds {
        vfeed::append_feed("名號", rname, detail);
    }

    // 5c-2g-3) 名號立牌 v1（自主提案）：居民第一次為某玩家安下名號的那一刻，鎖外在牠自家門旁刻一塊
    // 「此地常客·造物者」告示牌——把口說的名號（774）實體化成世界裡永久、可走近、可讀的印記（比照
    // keepsake 732 把玩家送的禮物擺成世界方塊）。走既有 Sign 方塊＋SignStore＋JSONL 管線、零新協議。
    // 去重（重啟安全）：先掃家門旁候選格的 SignStore，若已有名號牌就整筆略過（避免重啟後 coined_epithets
    // 歸零導致重刻）。找不到合適空地也靜默略過（不強蓋、不壓既有方塊），比照 749 立牌命名。
    for (rname, home_x, home_z, player_name, role) in &epithet_sign_reqs {
        // 家門旁的錨點：家域中心地表往上一格（沿用 749 nameplate 的偏移＋Y 微調在門旁找空地）。
        let hx = home_x.floor() as i32;
        let hz = home_z.floor() as i32;
        let anchor_y = {
            let world = hub().deltas.read().unwrap();
            vdt::ground_top(&world, hx, hz).map(|y| y + 1)
        }; // deltas 讀鎖釋放
        let Some(ay) = anchor_y else { continue };
        // ① 去重掃描：家門旁候選格是否已立過名號牌（讀 SignStore，重啟安全）。
        let already = {
            let signs = hub().sign.read().unwrap();
            vnameplate::NAMEPLATE_OFFSETS.iter().any(|(ox, oz)| {
                vnameplate::NAMEPLATE_Y_TRIES.iter().any(|dy| {
                    signs
                        .get(&vsign::pos_key(hx + ox, ay + dy, hz + oz))
                        .is_some_and(vepisign::is_honor_sign)
                })
            })
        }; // sign 讀鎖釋放
        if already {
            continue;
        }
        // ② 找一格門旁空地（腳下固體、頭上空氣），沒有就靜默略過。
        let Some((sx, sy, sz)) = pick_nameplate_slot((hx, ay, hz)) else {
            continue;
        };
        let text = vepisign::honor_sign_text(*role);
        // ③ 放 Sign 方塊（deltas 寫鎖短取即釋）→ 廣播 → 落地。
        {
            let mut world = hub().deltas.write().unwrap();
            voxel::set_block(&mut world, sx, sy, sz, Block::Sign);
        } // deltas 寫鎖釋放
        broadcast_block(sx, sy, sz, Block::Sign);
        vbuild::append_world_block(sx, sy, sz, Block::Sign as u8);
        // ④ 設牌面文字（sign 寫鎖短取即釋）→ 持久化 → 廣播浮字。
        let ev = hub().sign.write().unwrap().set(&vsign::pos_key(sx, sy, sz), text.clone());
        vsign::append_sign(&ev);
        broadcast_sign(sx, sy, sz, &text);
        // ⑤ 城鎮動態牆：讓玩家（與離線回來者）讀到「某居民把我刻成了這一帶的名號」。
        vfeed::append_feed(
            vepisign::FEED_KIND,
            rname,
            &vepisign::honor_feed_line(rname, player_name, *role),
        );
    }

    // 5c-2h) 自我印象驅動自發追尋（自我印象 v2，ROADMAP 771·PLAN_ETHERVOX 核心信念「記憶要驅動
    // 行為、不只聊天」）：居民回望自己、認出「我是個怎樣的人」的那一刻（泡泡已於鎖內設好），若那份
    // 自我理解能落到一個具體的建造念頭，就把它化成一個呼應自己的自發心願——鎖外套用，守鎖序（不在
    // residents 寫鎖內再取 desires 鎖，避免鎖序倒置）。**只在她此刻確實沒有任何心願時才種**（get_desire
    // 為 None 或已實現）——絕不覆蓋玩家親口許的願（心願是珍貴的）。種上後她的既有建造管線會真的把它
    // 蓋出來：自認「最愛蓋東西的人」會自發再蓋間屋、「離不開泥土的人」會再開畦花圃——記憶第一次不只
    // 被說出、還推著她的腳步去做。鎖外純短鎖 IO、不巢狀、守死鎖鐵律。
    for (rid, rname, desire) in &self_aspiration_sparks {
        // 此刻確實沒有進行中的心願才種（讀鎖即釋）。
        let vacant = {
            let des = hub().desires.read().unwrap();
            des.get_desire(rid).is_none_or(|d| d.fulfilled)
        };
        if !vacant {
            continue;
        }
        let new_desire = {
            let mut des = hub().desires.write().unwrap();
            des.set_desire(rid, desire, vdes::SELF_SPARK)
        }; // 心願寫鎖在此釋放
        vdes::append_desire(&new_desire);
        vfeed::append_feed("新心願", rname, &new_desire.desire);
    }

    // 5c-3) 就寢反思 Feed（作息·就寢反思 v1，ROADMAP 744）：居民入睡前回味今天最有感的一件事，
    // 記一筆動態，讓沒在線上的玩家回來也讀得到「牠昨晚睡前想著什麼」——記憶昇華進世界的日記牆。
    for (rname, summary) in &bedtime_feed {
        vfeed::append_feed("就寢", rname, &vbedtime::reflect_feed_line(summary));
    }

    // 5c-4) 晨間探友 Feed（作息 × 記憶驅動行為·晨間探友 v1，ROADMAP 745）：居民醒來讀昨晚的睡前
    // 反思，若惦記到某位居民就一早去找他，記一筆動態——沒在線上的玩家回來也讀得到「牠一早惦記著誰」，
    // 昨晚的心事第一次不只被說出來、還真的把居民的腳步帶去了某個地方。
    for (rname, friend) in &morning_feed {
        vfeed::append_feed("晨想", rname, &vmorning::seek_feed_line(friend));
    }

    // 5c-5) 晨間思念玩家 Feed（作息 × 記憶驅動行為·晨間思念玩家 v1，ROADMAP 746）：居民醒來讀昨晚
    // 的睡前反思，若惦記到的是某位在線玩家就一早朝他走去——記一筆動態，沒在世界另一頭的玩家也讀得到
    // 「牠一早惦記著你、往你走去了」。745 讓記憶帶居民去找居民，本刀讓記憶把居民的腳步帶到了「你」面前。
    for (rname, player) in &daybreak_feed {
        vfeed::append_feed("晨思", rname, &vdaybreak::miss_feed_line(player));
    }

    // 5c-6) 晨間思念玩家·抵達補記憶（ROADMAP 746）：居民走到玩家面前打招呼後，把「今早特地來找你」
    // 昇華成一筆掛在該玩家名下的記憶（算進與你的情誼）——你的離開與歸來，第一次在居民的清晨留下回聲。
    // 鎖序：memory 寫鎖各自短取即釋、不巢狀、append_memory 的 IO 在鎖外進行（守死鎖鐵律）。
    for (rid, _rname, player) in &daybreak_arrivals {
        let entry = {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, player, &vdaybreak::miss_memory_summary(player))
        };
        vmem::append_memory(&entry);
    }

    // 5c-7) 久別重逢奔迎·抵達 Feed + 補記憶（記憶驅動·久別重逢奔迎 v1，ROADMAP 747）：居民奔到久別歸來
    // 玩家面前迎接後，記一筆 Feed（世界看板可見）、並把「你久違回來、我特地跑來迎你」昇華成一筆掛在該玩家
    // 名下的重逢記憶（算進與你的情誼）——你的歸來第一次不只被世界記下，而是把某位居民的腳步帶到了你面前。
    // 鎖序：memory 寫鎖各自短取即釋、不巢狀、append_memory 的 IO 在鎖外進行（守死鎖鐵律）。
    for (rid, rname, player) in &reunion_arrivals {
        vfeed::append_feed("重逢", rname, &vreunion::reunion_feed_line(player));
        let entry = {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, player, &vreunion::reunion_memory_summary(player))
        };
        vmem::append_memory(&entry);
    }

    // 5d) 探訪 Feed（ROADMAP 671）：抵達 / 返家各一筆，讓離線玩家也知道居民在往來。
    for (visitor, host, is_return) in visit_events {
        if is_return {
            vfeed::append_feed(vvisit::FEED_KIND_RETURN, visitor, &format!("從{host}那裡回家了"));
        } else {
            vfeed::append_feed(vvisit::FEED_KIND_ARRIVE, visitor, &format!("抵達{host}家！"));
        }
    }

    // 5e) 居民情誼 v1（ROADMAP 672）：更新情誼帳本、升級廣播 + 生成依層級問候語。
    // 抵達事件：record_visit（bonds 寫鎖）→ 若升級 → save + Feed；再讀新層級生成問候語。
    for (visitor_id, visitor_name, host_name) in bond_arrive_events {
        // 取確定性 pick（unix 秒，足夠分散不同居民在同一秒的選句差異）。
        let pick = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as usize;
        // 短鎖：record_visit → 取新層級 → 是否升級。
        let (tier, tier_changed) = {
            let mut bonds = hub().bonds.write().unwrap();
            bonds.record_visit(visitor_name, &host_name)
        }; // bonds 寫鎖釋放
        if tier_changed {
            // 升級里程碑：持久化 + Feed 廣播，讓玩家看到「兩位居民漸漸成了老朋友」。
            {
                let bonds = hub().bonds.read().unwrap();
                vbonds::save_bonds(&bonds);
            } // bonds 讀鎖釋放
            let milestone = vbonds::tier_up_line(tier, visitor_name, &host_name);
            vfeed::append_feed("居民情誼", visitor_name, &milestone);
            // ROADMAP 673：社交足跡——情誼升級時在訪客記憶裡留下一筆，讓日記有社交痕跡。
            let social_mem = vbonds::bond_social_memory(&host_name, tier);
            if !social_mem.is_empty() {
                let entry = hub().memory.write().unwrap()
                    .add_memory(&visitor_id, &host_name, &social_mem);
                vmem::append_memory(&entry);
            } // memory 寫鎖釋放
        }
        // ROADMAP 694：口耳相傳——老朋友到訪時，主人把自己最近一則見聞轉述給訪客，
        // 見聞從此進訪客自己的記憶庫（經朋友網絡流通，不再只留在主人腦中）。
        if rand::random::<f32>() < vgossip::gossip_chance(tier) {
            let host_id = {
                let residents = hub().residents.read().unwrap();
                residents.iter().find(|r| r.name == host_name).map(|r| r.id.clone())
            }; // residents 讀鎖釋放
            if let Some(host_id) = host_id {
                let host_memories = hub().memory.read().unwrap().all_memories_for(&host_id); // drop
                if let Some(picked) = vgossip::pick_gossip(&host_memories, visitor_name) {
                    let gossip_text = vgossip::format_gossip(&host_name, &picked.summary);
                    let heard_before = vgossip::already_knows(
                        &hub().memory.read().unwrap().recall(&visitor_id, &host_name, vmem::RECALL_LIMIT), // drop
                        &gossip_text,
                    );
                    if !heard_before {
                        let entry = hub().memory.write().unwrap()
                            .add_memory(&visitor_id, &host_name, &gossip_text);
                        vmem::append_memory(&entry);
                    } // memory 寫鎖釋放
                }
            }
        }
        // 你的名號口耳相傳 v1（自主提案）：老朋友到訪時，主人若已在心裡為某位玩家安下名號
        // （774），偶爾會**說起你**——訪客從此「久仰」你（心裡記下這個聽來的名號＋一筆社交記憶），
        // 日後頭一次撞見你就用「久仰」的招呼喊出名號。774 名號從此不再只活在一位居民心裡，而是
        // 透過朋友網絡在小社會裡自己傳開。挑訪客還完全不認得的玩家（模擬「介紹一位新朋友給你」）。
        if rand::random::<f32>() < vespread::share_chance(tier) {
            // 一次 residents 寫鎖：讀主人名號表 → 挑一個訪客還不認得的 → 寫進訪客傳聞表（短鎖即釋、
            // 先 clone 主人表避免同時可變/不可變借用，比照本檔既有短鎖循序，不巢狀、不持鎖 await）。
            let shared: Option<(String, vepi::PlayerRole)> = {
                let host_coined = {
                    let residents = hub().residents.read().unwrap();
                    residents.iter().find(|r| r.name == host_name).map(|r| r.coined_epithets.clone())
                }; // residents 讀鎖釋放
                match host_coined {
                    Some(host_coined) if !host_coined.is_empty() => {
                        let mut residents = hub().residents.write().unwrap();
                        if let Some(v) = residents.iter_mut().find(|r| r.id == visitor_id) {
                            vespread::pick_to_share(&host_coined, &v.coined_epithets, &v.heard_epithets)
                                .map(|(pname, role)| {
                                    v.heard_epithets.insert(
                                        pname.clone(),
                                        vespread::Hearsay { role, from: host_name.clone() },
                                    );
                                    (pname, role)
                                })
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }; // residents 寫鎖釋放
            if let Some((pname, role)) = shared {
                // Feed 一則「口碑相傳」（鎖外）＋訪客記一筆社交記憶（主體＝主人名，不掛玩家名下，
                // 免污染玩家角色分類；比照 gossip 慣例）。additive Feed kind，舊前端安全落回 📌。
                vfeed::append_feed("口碑相傳", visitor_name, &vespread::spread_feed_line(&host_name, &pname, role));
                let mem_text = vespread::heard_memory_summary(&host_name, &pname, role);
                let entry = hub().memory.write().unwrap().add_memory(&visitor_id, &host_name, &mem_text);
                vmem::append_memory(&entry);
            }
        }
        // ROADMAP 696：居民互助蓋家 v1——老朋友到訪時，若主人正在蓋家，順手幫忙推進一塊，
        // 讓情誼（672）不只停在問候與八卦（694），第一次外溢成「真的動手互相幫忙」。
        // 只在剩餘 ≥2 塊時才幫（見 should_help_build 註解），完工收尾仍統一交給第 6 節處理，
        // 這裡不重複那段邏輯，避免兩處都跑一次完工廣播。
        let mut help_line: Option<String> = None;
        if tier == vbonds::BondTier::Friend {
            let host_id = {
                let residents = hub().residents.read().unwrap();
                residents.iter().find(|r| r.name == host_name).map(|r| r.id.clone())
            }; // residents 讀鎖釋放
            if let Some(host_id) = host_id {
                let remaining_before = {
                    hub().builds.read().unwrap().plans.get(&host_id).map_or(0, |p| p.remaining.len())
                }; // builds 讀鎖釋放
                if vbuild::should_help_build(remaining_before, rand::random::<f32>()) {
                    let popped = {
                        let mut builds = hub().builds.write().unwrap();
                        builds.get_plan_mut(&host_id)
                            .and_then(|p| p.pop_next().map(|bb| (bb, p.kind_name.clone())))
                    }; // builds 寫鎖釋放
                    if let Some((bb, kind_name)) = popped {
                        if let Some(block) = Block::from_u8(bb.b) {
                            {
                                let mut world = hub().deltas.write().unwrap();
                                voxel::set_block(&mut world, bb.x, bb.y, bb.z, block);
                            } // deltas 寫鎖釋放
                            broadcast_block(bb.x, bb.y, bb.z, block);
                            enqueue_water_around(bb.x, bb.y, bb.z);
                            vbuild::append_world_block(bb.x, bb.y, bb.z, bb.b);
                            if let Some(plan) = hub().builds.read().unwrap().plans.get(&host_id) {
                                vbuild::append_build(plan);
                            } // builds 讀鎖釋放
                            help_line = Some(vbuild::help_say_line(visitor_name, &kind_name));
                        }
                    }
                }
            }
        }
        // ROADMAP 715：居民偶爾小小拌嘴又和好 v1——情誼帳本（672）一路只漲不跌、老朋友到訪
        // 不是溫馨問候就是互助蓋家（696）或口耳相傳（694），關係永遠一片和樂；本節補上
        // PLAN_ETHERVOX「熟識/幫過/吵過」裡唯獨從未實作的「吵過」，讓小社會更真實有溫度。
        // 只在這次到訪沒有觸發互助蓋家時才可能拌嘴，同一次到訪只演一齣戲。
        let mut quarrel_line: Option<String> = None;
        if vquarrel::should_quarrel(tier, help_line.is_some(), rand::random::<f32>()) {
            vfeed::append_feed(vquarrel::FEED_KIND, visitor_name, &vquarrel::quarrel_feed_line(visitor_name, &host_name, pick));
            let host_id = {
                let residents = hub().residents.read().unwrap();
                residents.iter().find(|r| r.name == host_name).map(|r| r.id.clone())
            }; // residents 讀鎖釋放
            {
                let entry = hub().memory.write().unwrap()
                    .add_memory(&visitor_id, &host_name, &vquarrel::quarrel_memory_line_visitor(&host_name, pick));
                vmem::append_memory(&entry);
            } // memory 寫鎖釋放
            if let Some(host_id) = host_id {
                let entry = hub().memory.write().unwrap()
                    .add_memory(&host_id, visitor_name, &vquarrel::quarrel_memory_line_host(visitor_name, pick));
                vmem::append_memory(&entry);
            } // memory 寫鎖釋放
            quarrel_line = Some(vquarrel::quarrel_say_line(&host_name, pick));
        }

        // ROADMAP 717：居民互相傳授技能 v1——技能發明（716／#944）讓居民各自「自己」
        // 學會技能，卻只鎖在發明者本人身上；本節讓老朋友到訪時，偶爾把已學會的技能
        // 教給還不會的對方，本事第一次能像見聞（694）一樣在朋友網絡裡流通。只在這次
        // 到訪沒有觸發互助蓋家或拌嘴時才可能傳授，同一次到訪只演一齣戲；教誰哪個技能
        // 由 InventedSkillStore::teachable 查表決定（host 教 visitor 優先，host 沒有
        // 可教的才看 visitor 教不教得了 host）。
        let mut teach_line: Option<String> = None;
        if vteach::should_teach(tier, help_line.is_some(), quarrel_line.is_some(), rand::random::<f32>()) {
            let host_id = {
                let residents = hub().residents.read().unwrap();
                residents.iter().find(|r| r.name == host_name).map(|r| r.id.clone())
            }; // residents 讀鎖釋放
            if let Some(host_id) = host_id {
                let taught = {
                    let store = hub().invented.read().unwrap();
                    store
                        .teachable(&host_id, &visitor_id)
                        .map(|r| (host_id.clone(), host_name.clone(), visitor_id.clone(), visitor_name.to_string(), r.clone()))
                        .or_else(|| {
                            store.teachable(&visitor_id, &host_id).map(|r| {
                                (visitor_id.clone(), visitor_name.to_string(), host_id.clone(), host_name.clone(), r.clone())
                            })
                        })
                }; // invented 讀鎖釋放
                if let Some((teacher_id, teacher_name, student_id, student_name, skill)) = taught {
                    let learned = {
                        let mut store = hub().invented.write().unwrap();
                        store.add(&student_id, &skill.name, skill.goal_block, skill.steps.clone())
                    }; // invented 寫鎖釋放
                    if let Some(rec) = learned {
                        vinvent::append_invented_skill(&rec);
                        vfeed::append_feed(
                            vteach::FEED_KIND,
                            &teacher_name,
                            &vteach::teach_feed_line(&teacher_name, &student_name, &skill.name, pick),
                        );
                        {
                            let entry = hub().memory.write().unwrap().add_memory(
                                &teacher_id,
                                &student_name,
                                &vteach::teach_memory_line_teacher(&student_name, &skill.name),
                            );
                            vmem::append_memory(&entry);
                        } // memory 寫鎖釋放
                        {
                            let entry = hub().memory.write().unwrap().add_memory(
                                &student_id,
                                &teacher_name,
                                &vteach::teach_memory_line_student(&teacher_name, &skill.name),
                            );
                            vmem::append_memory(&entry);
                        } // memory 寫鎖釋放
                        teach_line = Some(if teacher_id == host_id {
                            vteach::teach_say_line_as_student(&teacher_name, &skill.name, pick)
                        } else {
                            vteach::teach_say_line_as_teacher(&student_name, &skill.name, pick)
                        });
                    }
                }
            }
        }

        // ROADMAP 723：居民互相以物易物 v1——交易特長系統（670）至今只用在玩家↔居民
        // 這個方向，本節讓老朋友到訪時，偶爾比照同一套特長分類「順手交換」一次，小社會
        // 第一次有了內部經濟流動。只在這次到訪沒有觸發互助蓋家/拌嘴/傳授技能時才可能
        // 發生（同一次到訪只演一齣戲，鏡像既有優先序）。
        let mut resident_trade_line: Option<String> = None;
        if vrtrade::should_resident_trade(
            tier, help_line.is_some(), quarrel_line.is_some(), teach_line.is_some(),
            rand::random::<f32>(),
        ) {
            let host_id = {
                let residents = hub().residents.read().unwrap();
                residents.iter().find(|r| r.name == host_name).map(|r| r.id.clone())
            }; // residents 讀鎖釋放
            if let Some(host_id) = host_id {
                if let Some((v_item, h_item)) = vrtrade::trade_pair(&visitor_id, &host_id) {
                    let v_name = vtrade::item_name_zh(v_item);
                    let h_name = vtrade::item_name_zh(h_item);
                    vfeed::append_feed(
                        vrtrade::FEED_KIND,
                        visitor_name,
                        &vrtrade::trade_feed_line(visitor_name, &host_name, v_name, h_name),
                    );
                    {
                        let entry = hub().memory.write().unwrap()
                            .add_memory(&visitor_id, &host_name, &vrtrade::trade_memory_line(&host_name, v_name, h_name));
                        vmem::append_memory(&entry);
                    } // memory 寫鎖釋放
                    {
                        let entry = hub().memory.write().unwrap()
                            .add_memory(&host_id, visitor_name, &vrtrade::trade_memory_line(visitor_name, h_name, v_name));
                        vmem::append_memory(&entry);
                    } // memory 寫鎖釋放
                    resident_trade_line = Some(vrtrade::trade_say_line(&host_name, h_name, pick));
                }
            }
        }

        // ROADMAP 748：居民互贈·分享採集所得 v1——以物易物（723）是象徵性的（基於特長分類、
        // 不動實際背包）；本節讓老朋友到訪時，主人偶爾把自己採集背包（res_inv）裡真的採到最多、
        // 且有餘裕的那種材料，勻一小份**真的**移進訪客背包（零和守恆、不憑空生料），餵訪客自己的
        // 發明/建造計畫——小社會第一道真實的物資血流。只在這次到訪沒觸發互助蓋家/拌嘴/傳授/易物
        // 時才可能（同一訪只演一齣戲，鏡像既有優先序）。
        let mut share_line: Option<String> = None;
        let other_scene = help_line.is_some()
            || quarrel_line.is_some()
            || teach_line.is_some()
            || resident_trade_line.is_some();
        if vshare::should_share(tier, other_scene, rand::random::<f32>()) {
            let host_id = {
                let residents = hub().residents.read().unwrap();
                residents.iter().find(|r| r.name == host_name).map(|r| r.id.clone())
            }; // residents 讀鎖釋放
            if let Some(host_id) = host_id {
                // 先讀主人背包挑一份可分享的材料（讀鎖即釋，不巢狀）。
                let picked = {
                    let bags = hub().res_inv.read().unwrap();
                    bags.get(&host_id).and_then(|b| vshare::pick_share(b))
                }; // res_inv 讀鎖釋放
                if let Some((block_id, qty)) = picked {
                    // 真實轉移：主人扣、訪客加（單一寫鎖內循序完成，先扣後加不同時持雙可變借用；
                    // 再次確認主人此刻仍握有足量，防與同 tick 其他消耗競態）。
                    let transferred = {
                        let mut bags = hub().res_inv.write().unwrap();
                        let host_has =
                            bags.get(&host_id).and_then(|b| b.get(&block_id)).copied().unwrap_or(0);
                        if host_has >= qty {
                            if let Some(hb) = bags.get_mut(&host_id) {
                                *hb.entry(block_id).or_insert(0) -= qty; // 安全：host_has >= qty
                            }
                            *bags
                                .entry(visitor_id.clone())
                                .or_default()
                                .entry(block_id)
                                .or_insert(0) += qty;
                            true
                        } else {
                            false
                        }
                    }; // res_inv 寫鎖釋放
                    if transferred {
                        let item_name = vgift::item_name_zh(block_id);
                        vfeed::append_feed(
                            vshare::FEED_KIND,
                            &host_name,
                            &vshare::share_feed_line(visitor_name, item_name, qty),
                        );
                        // 雙方各記一筆（主人慷慨、訪客暖心），日後可被日記昇華。
                        {
                            let entry = hub().memory.write().unwrap().add_memory(
                                &host_id,
                                visitor_name,
                                &vshare::share_memory_line_host(visitor_name, item_name),
                            );
                            vmem::append_memory(&entry);
                        } // memory 寫鎖釋放
                        {
                            let entry = hub().memory.write().unwrap().add_memory(
                                &visitor_id,
                                &host_name,
                                &vshare::share_memory_line_visitor(&host_name, item_name),
                            );
                            vmem::append_memory(&entry);
                        } // memory 寫鎖釋放
                        share_line = Some(vshare::share_say_line(visitor_name, item_name, pick));
                    }
                }
            }
        }

        // 依新層級生成問候語 → say_updates（守 say_updates 的「say 空才套」原則）；
        // 若這次到訪順手幫了忙，優先冒幫忙台詞（更有感）；否則若拌了嘴，冒拌嘴台詞；
        // 否則若學/教了技能，冒傳授台詞；否則若互相易物，冒易物台詞；否則若分享了材料，冒分享
        // 台詞；都沒有才落回一般問候語。
        let greeting = help_line
            .or(quarrel_line)
            .or(teach_line)
            .or(resident_trade_line)
            .or(share_line)
            .unwrap_or_else(|| vbonds::arrival_line(tier, &host_name, visitor_name, pick));
        say_updates.push((visitor_id, greeting));
    }
    // 離開事件：讀當前層級（bonds 讀鎖）→ 生成告別語 → say_updates。
    for (visitor_id, visitor_name, host_name) in bond_depart_events {
        let pick = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as usize + 1; // +1 讓同一秒的抵達/離開選到不同句
        let tier = { hub().bonds.read().unwrap().tier_of(visitor_name, &host_name) }; // drop
        let farewell = vbonds::departure_line(tier, visitor_name, pick);
        say_updates.push((visitor_id, farewell));
    }

    // 6) 居民 agency（目標+記憶驅動）：蓋造不重複、有進展、會持久 + 採集技能調用。
    //    流程：① 有計畫 → 彈下一塊放置（持久化）；完成 → 記下「蓋過這種」（不再重蓋）+ 完工 Feed
    //           ② 無計畫 → choose_activity（依已完成清單+心願，永不重選蓋過的）→ 採集 or 蓋下一個
    //    say_updates 在 tick_residents 頂層宣告（含過渡/採集/建造台詞），最後一次性套用。

    // ROADMAP 681：批次快照居民心情補助（residents 讀鎖即釋），供建造間隔與打氣共用。
    let mood_boosts_by_id: HashMap<String, bool> = {
        let res = hub().residents.read().unwrap();
        res.iter().map(|r| (r.id.clone(), r.mood_boost_secs > 0.0)).collect()
    }; // residents 讀鎖在此釋放

    // ROADMAP 680：批次計算所有居民心情 → 對應建造間隔（鎖序：bonds 讀即釋 → memory 讀即釋）。
    let build_mood_intervals: HashMap<String, f32> = {
        let all_ids: Vec<String> = (0..resident_count()).map(|j| format!("vox_res_{j}")).collect();
        let bond_counts: Vec<(usize, usize)> = {
            let bonds = hub().bonds.read().unwrap();
            all_ids.iter().map(|rid| resident_bond_counts(&bonds, rid)).collect()
        }; // bonds 讀鎖釋放
        let mem_counts: Vec<usize> = {
            let mem = hub().memory.read().unwrap();
            all_ids.iter().map(|rid| mem.memory_count(rid)).collect()
        }; // memory 讀鎖釋放
        all_ids
            .into_iter()
            .zip(bond_counts.into_iter().zip(mem_counts.into_iter()))
            .map(|(rid, ((f, a), mc))| {
                let base_tier = voxel_mood::compute_mood(f, a, mc);
                // ROADMAP 681：若有心情補助，建造間隔採提升後的層級（加速）。
                let tier = if mood_boosts_by_id.get(&rid).copied().unwrap_or(false) {
                    voxel_mood::boost_mood(base_tier)
                } else {
                    base_tier
                };
                (rid, voxel_mood::build_interval_secs(tier))
            })
            .collect()
    };

    // 6-0) 技能發明提案交棒（真進化第一刀）：取走便宜腦投回的「解析+驗證通過」計畫，
    //      掛到居民身上開始執行。短鎖循序（proposals mutex → residents 寫），不巢狀、不 await。
    let arrived_proposals: Vec<(String, u8, String, vinvent::InventedPlan)> = {
        let mut props = hub().invent_proposals.lock().unwrap();
        std::mem::take(&mut *props)
    }; // proposals mutex 釋放
    for (prid, goal_block, goal_name, plan) in arrived_proposals {
        let attached = {
            let mut residents = hub().residents.write().unwrap();
            residents.iter_mut().find(|r| r.id == prid).map_or(false, |r| {
                // 既有任務優先：已有進行中的發明、或正幫玩家跑腿採集（指令→任務第三刀）
                // 就不掛新計畫——提案作廢（低頻小成本），等她空閒時處境仍在會再想一次。
                if r.invent_run.is_none() && r.fetch.is_none() {
                    r.invent_run = Some(vinvent::InventRun::from_plan(
                        goal_block, &goal_name, &plan, false,
                    ));
                    true
                } else {
                    false
                }
            })
        }; // residents 寫鎖釋放
        if attached {
            let rname = resident_name_of(&prid);
            tracing::info!(resident = %prid, skill = %plan.name, "技能發明：便宜腦提出計畫，開始驗證執行");
            say_updates.push((prid, format!("有了！我來試試「{}」…", plan.name)));
            vfeed::append_feed(
                "技能發明",
                rname,
                &format!("想出了一個點子「{}」（{}），動手試試", plan.name, vinvent::steps_summary(&plan.steps)),
            );
        }
    }

    for (rid, rname, rx, ry, rz, _ridx) in build_candidates {
        // ── 技能發明/重用執行（優先於一般 agency：她正專心驗證自己的點子）────────
        // 有進行中的 InventRun → 推進一步（逾時/失敗/成功都在裡面收尾）→ 本輪不做別的。
        let has_invent_run = {
            let residents = hub().residents.read().unwrap();
            residents.iter().find(|r| r.id == rid).map_or(false, |r| r.invent_run.is_some())
        }; // residents 讀鎖釋放
        if has_invent_run {
            advance_invent_run(&rid, rname, rx, ry, rz, &mut say_updates);
            let interval = *build_mood_intervals.get(&rid).unwrap_or(&BUILD_INTERVAL_SECS);
            reset_build_tick(&rid, interval);
            continue;
        }

        let has_plan = hub().builds.read().unwrap().has_plan(&rid); // drop

        if !has_plan {
            // ── 好奇心自主學習（北極星第三刀）：閒置＋計時到期＋機率門檻 → 從可能性
            //    目錄挑一樣她還不會的，自發種下心願（sparked_by=好奇心）——緊接著下方的
            //    處境偵測立刻接手（心願含材料→發明→存技能），**不用玩家 push 也會成長**。
            //    成本紀律：好奇本身零 LLM；發明照舊走冷卻＋防重入；目錄空也零 LLM。
            let (curio_due, invent_cd) = {
                let residents = hub().residents.read().unwrap();
                residents
                    .iter()
                    .find(|r| r.id == rid)
                    .map_or((false, 1.0), |r| (r.curiosity_timer <= 0.0, r.invent_cooldown))
            }; // residents 讀鎖釋放
            // 閒置事實：invent_run=None（上方已 continue）、fetch=None（候選條件已排除）、
            // 建造計畫=無（本分支）；只剩發明冷卻要問她本人。
            if curio_due && vinvent::curiosity_idle(false, false, has_plan, invent_cd) {
                // 無論這次有沒有真的好奇，計時都重置（低頻紀律：每週期至多想一次）。
                {
                    let mut residents = hub().residents.write().unwrap();
                    if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                        r.curiosity_timer = vinvent::curiosity_base_secs();
                    }
                } // residents 寫鎖釋放
                // 玩家親口種下、還沒實現的心願不打擾（先惦記玩家的願，好奇心讓路）。
                let player_wish_pending = {
                    let des = hub().desires.read().unwrap();
                    des.get_desire(&rid).map_or(false, |d| {
                        !d.fulfilled
                            && d.sparked_by != vdes::SELF_SPARK
                            && d.sparked_by != vdes::CURIOSITY_SPARK
                    })
                }; // desires 讀鎖釋放
                if !player_wish_pending && vinvent::curiosity_gate(rand::random()) {
                    // 排除清單＝技能庫已會的目標 ∪ 背包已有的產物 ∪ 退避中的目標。
                    let mut excluded = {
                        let inv = hub().invented.read().unwrap();
                        inv.known_goals_for(&rid)
                    }; // invented 讀鎖釋放
                    {
                        let bags = hub().res_inv.read().unwrap();
                        if let Some(bag) = bags.get(&rid) {
                            excluded
                                .extend(bag.iter().filter(|(_, c)| **c > 0).map(|(b, _)| *b));
                        }
                    } // res_inv 讀鎖釋放
                    // 退避中的目標：這位居民連敗 N 次的目標暫時跳過，讓她換個方向探索。
                    {
                        let residents = hub().residents.read().unwrap();
                        if let Some(r) = residents.iter().find(|r| r.id == rid) {
                            excluded.extend(r.invent_backoff.keys().copied());
                        }
                    } // residents 讀鎖釋放
                    let catalog = vinvent::possibility_catalog(&excluded);
                    // 確定性種子：她此刻的位置 bits——同居民不同時刻挑到不同樣、可重現。
                    let seed = (rx as i64 as u64) ^ ((rz as i64 as u64) << 20);
                    match vinvent::curiosity_pick(&catalog, seed) {
                        Some(goal) => {
                            let new_desire = {
                                let mut des = hub().desires.write().unwrap();
                                des.set_desire(
                                    &rid,
                                    &vinvent::curiosity_desire_text(goal.name_zh),
                                    vdes::CURIOSITY_SPARK,
                                )
                            }; // desires 寫鎖釋放
                            vdes::append_desire(&new_desire);
                            vfeed::append_feed(
                                "好奇心",
                                rname,
                                &vinvent::curiosity_feed(goal.name_zh),
                            );
                            // 寫進記憶（日記走既有事件管道自然反映這段自主探索）。
                            let entry = {
                                let mut mem = hub().memory.write().unwrap();
                                mem.add_memory(
                                    &rid,
                                    vdes::CURIOSITY_SPARK,
                                    &vinvent::curiosity_memory(goal.name_zh),
                                )
                            }; // memory 寫鎖釋放
                            vmem::append_memory(&entry);
                            say_updates.push((rid.clone(), vinvent::curiosity_line(goal.name_zh)));
                            tracing::info!(
                                resident = %rid, goal = %goal.name_zh,
                                "好奇心：自發種下想做的東西（發明引擎接手）"
                            );
                        }
                        None => {
                            // 能學的她全會了：冒個泡就好——**零 LLM**。
                            say_updates
                                .push((rid.clone(), vinvent::nothing_new_line().to_string()));
                        }
                    }
                }
            }

            // ── 處境偵測（真進化第一刀）：心願提到可合成材料、背包卻沒有 ─────────────
            // ＝「沒有現成技能可解的處境」。先查**自己的**技能庫（會 → 直接重用，零 LLM
            // ——這就是進化：她下次不用再想，因為她已經會了）；不會 → 低頻請便宜腦發明。
            let desire_text: Option<String> = {
                let des = hub().desires.read().unwrap();
                des.get_desire(&rid).map(|d| d.desire.clone())
            }; // desires 讀鎖釋放
            if let Some(goal) = desire_text.as_deref().and_then(vinvent::detect_missing_material) {
                let bag_has_goal = {
                    let inv = hub().res_inv.read().unwrap();
                    inv.get(&rid).and_then(|b| b.get(&goal.block_id)).copied().unwrap_or(0) >= 1
                }; // res_inv 讀鎖釋放
                if !bag_has_goal {
                    let known: Option<(String, Vec<vinvent::PrimStep>)> = {
                        let inv = hub().invented.read().unwrap();
                        inv.find_for(&rid, goal.block_id)
                            .map(|k| (k.name.clone(), k.steps.clone()))
                    }; // invented 讀鎖釋放
                    if let Some((skill_name, raw_steps)) = known {
                        // ① 她已經會了：載回的序列再過一次存檔白名單（配方表若變動，壞技能
                        //    自然失效不執行），通過就直接照自己存的技能做——**零 LLM**。
                        if let Some(steps) = vinvent::check_stored_steps(&raw_steps) {
                            {
                                let mut residents = hub().residents.write().unwrap();
                                if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                                    r.invent_run = Some(vinvent::InventRun {
                                        goal_block: goal.block_id,
                                        goal_name: goal.name_zh.to_string(),
                                        skill_name: skill_name.clone(),
                                        raw_steps,
                                        steps,
                                        step_idx: 0,
                                        reuse: true,
                                        deadline: vinvent::RUN_TIMEOUT_SECS,
                                    });
                                }
                            } // residents 寫鎖釋放
                            tracing::info!(
                                resident = %rid, skill = %skill_name,
                                "技能重用：同處境直接用自己發明的技能（零 LLM）"
                            );
                            say_updates.push((rid.clone(), vinvent::reuse_line(&skill_name)));
                            let interval =
                                *build_mood_intervals.get(&rid).unwrap_or(&BUILD_INTERVAL_SECS);
                            reset_build_tick(&rid, interval);
                            continue;
                        }
                    } else {
                        // ② 不會 → 冷卻到才低頻請便宜腦發明（成本紀律；async、不擋 tick）。
                        //    等腦回計畫的期間，她照常過日子（採集/蓋家），提案回來再開工。
                        let cooled = {
                            let residents = hub().residents.read().unwrap();
                            residents
                                .iter()
                                .find(|r| r.id == rid)
                                .map_or(false, |r| r.invent_cooldown <= 0.0)
                        }; // residents 讀鎖釋放
                        if cooled && npc_agent_wire::agents_enabled() {
                            {
                                let mut residents = hub().residents.write().unwrap();
                                if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                                    r.invent_cooldown = vinvent::INVENT_COOLDOWN_SECS;
                                }
                            } // residents 寫鎖釋放
                            // 世界事實快照（第二刀）：她附近是否已有放置好的工作台——
                            // 給可行性模擬與 prompt（有就不必再做一個）。deltas 讀鎖即釋。
                            let wb_nearby = {
                                let world = hub().deltas.read().unwrap();
                                vinvent::station_nearby(
                                    &world, rx, ry, rz, vinvent::WORKBENCH_BLOCK_ID,
                                )
                            }; // deltas 讀鎖釋放
                            spawn_invention(
                                rid.clone(),
                                rname,
                                goal,
                                desire_text.clone().unwrap_or_default(),
                                wb_nearby,
                            );
                        }
                    }
                }
            }

            // ── 無計畫：挑下一個活動（目標+記憶驅動，不鬼打牆）──────────────────
            // 已完成的建物種類 + 已擴建次數（持久 GoalStore）+ 玩家心願（可選對應建物）+ 已採集次數。
            let (done_kinds, expansion_count) = {
                let goals = hub().goals.read().unwrap();
                (goals.done_kinds(&rid), goals.expansion_count(&rid))
            }; // goals 讀鎖釋放
            // 心願真的成真 v1（ROADMAP 720）：連同「誰種下這個心願」一起讀出——只有真人玩家
            // 的話（非居民自我啟發 `SELF_SPARK`）種下的心願，蓋成真時才指名感謝那位玩家。
            let (desired_kind, desired_by): (Option<vbuild::BuildKind>, Option<String>) = {
                let des = hub().desires.read().unwrap();
                match des.get_desire(&rid) {
                    Some(d) => (
                        vbuild::classify_desire(&d.desire),
                        // 自我啟發（禱告）與好奇心都不是真人玩家——完工不指名感謝。
                        (d.sparked_by != vdes::SELF_SPARK
                            && d.sparked_by != vdes::CURIOSITY_SPARK)
                            .then(|| d.sparked_by.clone()),
                    ),
                    None => (None, None),
                }
            }; // desires 讀鎖釋放
            let gathered = {
                let residents = hub().residents.read().unwrap();
                residents.iter().find(|r| r.id == rid).map_or(0, |r| r.gathered_since_build)
            }; // residents 讀鎖釋放

            match vskill::choose_activity(&done_kinds, desired_kind, gathered, GATHER_QUOTA, expansion_count) {
                NextActivity::Gather => {
                    start_gather(&rid, rx, rz);
                }
                NextActivity::Build(kind) => {
                    // 建造位置以「家域中心」為基準、依已蓋數量散開（不疊在舊建物上）。
                    let done_count = done_kinds.len();
                    // 只有這座建物真的是心願所指的種類，才把啟發者記進計畫（供完工時指名感謝）。
                    let inspired_by = if desired_kind == Some(kind) { desired_by.clone() } else { None };
                    start_build(&rid, rname, kind, done_count, &mut say_updates, false, inspired_by);
                }
                NextActivity::Expand(kind) => {
                    // 擴建 v1：基礎四種都蓋完了，但這位居民仍懷著具體渴望（哪怕對應種類早蓋過）——
                    // 此前這份渴望會被 `next_build_goal` 直接忽略，永遠石沉大海；現在用
                    // `build_offset` 原本就多留的兩個格位（見 MAX_EXPANSIONS）再蓋一座，
                    // 讓「你的話真的有後果」延伸到擴建，不再是空頭支票。
                    // 擴建不指名感謝（同一份心願只在首次蓋成時感謝一次，避免無限次重複記功）。
                    let expand_seq = vskill::BUILD_PROGRESSION.len() + expansion_count as usize;
                    start_build(&rid, rname, kind, expand_seq, &mut say_updates, true, None);
                }
                NextActivity::Wander => {
                    // 全部蓋完：探訪鄰居 v1（ROADMAP 671）——偶爾出發拜訪另一位居民。
                    // 探訪冷卻 + 確定性觸發（不需改動鎖，下面在 residents 寫鎖段套用）。
                    let (is_visiting, is_gathering, visit_cooldown) = {
                        let residents = hub().residents.read().unwrap();
                        residents.iter().find(|r| r.id == rid)
                            // 遠行中（ROADMAP 756）視同 is_visiting，別讓探訪蓋掉遠行目標。
                            .map_or((false, false, 0.0), |r| (r.visiting.is_some() || r.expedition.is_some(), r.clique_meet.is_some(), r.visit_cooldown))
                    }; // residents 讀鎖釋放

                    // 小圈子聚會（ROADMAP 711）優先：正在前往/等待聚會時，別讓探訪蓋掉目標。
                    if !is_visiting && !is_gathering && vvisit::should_visit(true, visit_cooldown, rand::random::<f32>()) {
                        // 挑目標居民（確定性：用居民位置 bits 避免每幀不同）。
                        let pick = ((rx as u32).wrapping_add(rz as u32)) as usize;
                        let my_idx = rid.trim_start_matches("vox_res_").parse::<usize>().unwrap_or(0);
                        // 快照所有居民的家域中心（短鎖）。
                        let homes_snap: Vec<(f32, f32, String)> = {
                            let res = hub().residents.read().unwrap();
                            res.iter().map(|r| (r.home_x, r.home_z, r.name.to_string())).collect()
                        }; // residents 讀鎖釋放
                        let homes_ref: Vec<(f32, f32, &str)> = homes_snap
                            .iter().map(|(x, z, n)| (*x, *z, n.as_str())).collect();
                        // 情誼加權（ROADMAP 671 深化）：老朋友更常被造訪，關係第一次真的
                        // 影響探訪目標，非只被行為單向記錄（residents 鎖已釋放，另取 bonds 短鎖，不巢狀）。
                        let tiers: Vec<vbonds::BondTier> = {
                            let bonds = hub().bonds.read().unwrap();
                            homes_snap.iter().map(|(_, _, n)| bonds.tier_of(rname, n)).collect()
                        }; // bonds 讀鎖釋放
                        if let Some((tx, tz, host_name)) = vvisit::pick_destination(my_idx, &homes_ref, &tiers, pick) {
                            let host = host_name.to_string();
                            {
                                let mut residents = hub().residents.write().unwrap();
                                if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                                    r.visiting = Some((tx, tz, host.clone()));
                                    r.visit_stay_timer = 0.0;
                                    r.visit_cooldown = vvisit::VISIT_COOLDOWN_SECS;
                                    // 設定閒晃目標直接朝目的地前進
                                    r.target_x = tx;
                                    r.target_z = tz;
                                }
                            } // residents 寫鎖釋放
                            // Feed 事件（鎖外 IO）。
                            vfeed::append_feed(
                                vvisit::FEED_KIND_DEPART,
                                rname,
                                &format!("動身去拜訪{host}！"),
                            );
                        }
                    } else if !is_visiting && !is_gathering {
                        // 不探訪、不在聚會：偶爾散心採集（低頻、不洗版），否則純閒晃。
                        if rand::random::<f32>() < IDLE_GATHER_CHANCE {
                            start_gather(&rid, rx, rz);
                        }
                    }
                }
            }
            // 重設 agency tick 等下次（採集中不會再進這裡，見 build_candidate 閘）。
            // ROADMAP 680：依心情選用對應間隔（Joyful=5s / Lonely=12s，預設 8s）。
            let interval = *build_mood_intervals.get(&rid).unwrap_or(&BUILD_INTERVAL_SECS);
            reset_build_tick(&rid, interval);
            continue;
        }

        // ── 有計畫：彈下一塊放置 + 持久化 + 進度冒泡 ──────────────────────────
        let (next_block, kind_name, kind_str, progress_pct, plan_done, plan_anchor, plan_expansion, plan_inspired_by) = {
            let mut builds = hub().builds.write().unwrap();
            if let Some(plan) = builds.get_plan_mut(&rid) {
                let bb = plan.pop_next();
                let kn = plan.kind_name.clone();
                let ks = plan.kind.clone();
                let pct = plan.progress_pct();
                let done = plan.is_done();
                let anchor = (plan.cx, plan.cy, plan.cz);
                let exp = plan.expansion;
                let inspired = plan.inspired_by.clone();
                (bb, kn, ks, pct, done, anchor, exp, inspired)
            } else {
                (None, String::new(), String::new(), 100, true, (0, 0, 0), false, None)
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
                // 水流動：居民蓋的方塊可能堵住水路（例如在水邊築牆）→ 喚醒鄰格重算。
                enqueue_water_around(bb.x, bb.y, bb.z);
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
            if plan_expansion {
                // 擴建完工：記到 expansion 計數（不進 done 集合，種類早就在裡面了）。
                if let Some(kind) = vbuild::BuildKind::from_str(&kind_str) {
                    let rec = {
                        let mut goals = hub().goals.write().unwrap();
                        goals.mark_expansion(&rid, kind, plan_anchor)
                    }; // goals 寫鎖釋放
                    vskill::append_goal(&rec);
                }
            } else if let Some(kind) = vbuild::BuildKind::from_str(&kind_str) {
                // 記下「這位居民蓋過這種建物」→ 之後永不重蓋（不鬼打牆）+ 持久化。
                // `mark_done` 首次完成該種時登記錨點並回一筆 GoalRecord；若這種早已完成（回 None），
                // 改呼叫 `anchor_only_record` **就地登記這座的錨點並回一筆純錨點記錄**落地——
                // 讓重啟後仍記得此錨點擋重蓋（這正是 res_1 水井連續「完工」數十次的洞）。
                let (rec, anchor_rec) = {
                    let mut goals = hub().goals.write().unwrap();
                    let r = goals.mark_done(&rid, kind, plan_anchor);
                    let ar = if r.is_none() {
                        Some(goals.anchor_only_record(&rid, kind, plan_anchor))
                    } else {
                        None
                    };
                    (r, ar)
                }; // goals 寫鎖釋放
                if let Some(rec) = rec {
                    vskill::append_goal(&rec);
                } else if let Some(ar) = anchor_rec {
                    // 蓋家鬼打牆補漏：這種早已在 done（不落新完成記錄），但這一座真的完工了——
                    // 持久化它的錨點，否則同格會被一蓋再蓋、永遠逃過 anchor_built 封鎖
                    // （這正是 res_1 水井連續「完工」數十次的洞）。
                    vskill::append_goal(&ar);
                }
            }
            // 完工 Feed（每個建物只發一次，不洗版）。
            vfeed::append_feed(
                if plan_expansion { "蓋家擴建完工" } else { "蓋家完工" },
                &rname,
                &kind_name,
            );
            // 完工廣播：WS 廣播給所有在線玩家（看得到「世界在長大」），慶賀泡泡同步排入 say_updates。
            let _ = hub().tx.send(std::sync::Arc::new(vannounce::build_complete_msg(&rname, &kind_name)));
            if let Some(kind) = vbuild::BuildKind::from_str(&kind_str) {
                match &plan_inspired_by {
                    // 心願真的成真 v1（ROADMAP 720）：這座建物是某位玩家的話種下的心願，
                    // 不用通用完工台詞，改指名感謝——「你的話真的有後果」的證據時刻。
                    Some(player) => {
                        say_updates.push((rid.clone(), vannounce::wish_come_true_say(&rname, kind, player)));
                        let _ = hub().tx.send(std::sync::Arc::new(vannounce::wish_come_true_msg(
                            &rname, &kind_name, player,
                        )));
                        vfeed::append_feed("心願成真", &rname, &format!("因為{player}的話，蓋好了{kind_name}"));
                        let entry = {
                            let mut mem = hub().memory.write().unwrap();
                            mem.add_memory(&rid, player, &vannounce::wish_come_true_memory(&kind_name))
                        }; // memory 寫鎖釋放
                        vmem::append_memory(&entry);
                    }
                    None => {
                        say_updates.push((rid.clone(), vannounce::build_complete_say(&rname, kind)));
                    }
                }
            }
            // 居民立牌命名 v1（ROADMAP 749）：蓋完建物親手在門前立一塊告示牌署名，
            // 741「居民讀牌」的鏡像——居民第一次拿起人類的導覽工具，蓋的家從此有名。
            // 只在首建（非擴建）立牌，避免同名牌重複；走既有告示牌管線（Sign 方塊 + SignStore
            // + 廣播 + JSONL），零新協議。找不到合適空地就靜默略過（不強蓋、不壓既有方塊）。
            if !plan_expansion {
                if let Some(kind) = vbuild::BuildKind::from_str(&kind_str) {
                    let text = vnameplate::nameplate_text(&rname, kind);
                    if !text.is_empty() {
                        if let Some((sx, sy, sz)) = pick_nameplate_slot(plan_anchor) {
                            // ① 放 Sign 方塊（deltas 寫鎖短取即釋）。
                            {
                                let mut world = hub().deltas.write().unwrap();
                                voxel::set_block(&mut world, sx, sy, sz, Block::Sign);
                            } // deltas 寫鎖釋放
                            broadcast_block(sx, sy, sz, Block::Sign);
                            vbuild::append_world_block(sx, sy, sz, Block::Sign as u8);
                            // ② 設牌面文字（sign 寫鎖短取即釋）→ 持久化 → 廣播浮字。
                            let ev = hub()
                                .sign
                                .write()
                                .unwrap()
                                .set(&vsign::pos_key(sx, sy, sz), text.clone());
                            vsign::append_sign(&ev);
                            broadcast_sign(sx, sy, sz, &text);
                            // ③ 動態牆 + 立牌泡泡（讓玩家一眼看到居民署了名）。
                            vfeed::append_feed("立牌命名", &rname, &vnameplate::nameplate_feed(&rname, &text));
                            say_updates.push((rid.clone(), vnameplate::nameplate_say(&text)));
                        }
                    }
                }
            }
            // 蓋完一個 → 重置採集計數，下一輪先採料再蓋下一種（有進展感）；
            // 並開建造冷卻（蓋家鬼打牆頻率保險）：這段時間內不動工新建物，完工事件不洗版。
            {
                let mut residents = hub().residents.write().unwrap();
                if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                    r.gathered_since_build = 0;
                    r.build_cooldown = BUILD_COOLDOWN_SECS;
                }
            } // residents 寫鎖釋放
        }

        let interval = *build_mood_intervals.get(&rid).unwrap_or(&BUILD_INTERVAL_SECS);
        reset_build_tick(&rid, interval);
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

/// 鋪面任務·單居民單 tick 推進（她已抵達工地時由 tick_residents 的施工段呼叫）。
/// 順序：① 礦井進行中 → 清幾格、挖出的實心方塊誠實入她的小背包；
///      ② 乾跑算「這批要改什麼、要吃幾份材料」；③ 背包原料就地合成到夠（2×2 基本配方）；
///      ④ 夠料 → **誠實扣料** + 套世界 + 推進 cursor（同整地的套用迴圈與安全過濾）；
///      ⑤ 缺料 → 找地表原料設採集技能（她走去挖、回來續鋪）；
///      ⑥ 地表沒有且原料在土石層 → 開一口階梯礦井（走得回地面、不自困）；
///      ⑦ 都補不上 → 誠實停工（鋪到哪算哪、Feed+冒泡說清楚，不吹牛）。
/// 有進展（挖井/備料變動/鋪一批）就把 deadline 續期——鋪面的逾時是「無進展門檻」。
/// 鎖紀律同整地：全部短取即釋、循序不巢狀、不 await。回傳本 tick 消耗的全域柱數預算。
fn pave_worker_tick(
    rid: &str,
    mut task: DirectedTask,
    cols_budget: usize,
    say_updates: &mut Vec<(String, String)>,
) -> usize {
    let Some(mat) = task.pave else { return 0 }; // 防禦性：呼叫端已保證是鋪面任務
    let mid = mat as u8;
    let rname = resident_name_of(rid);
    let mname = vdt::pave_material_name(mat);
    let Some(prov) = vdt::pave_provision(mat) else {
        // 支援表外材料（理論到不了：detect 端只認支援材料）→ 誠實放棄。
        hub().directed_tasks.write().unwrap().remove(rid);
        vfeed::append_feed("鋪面", rname, &format!("{mname}我備不出來，只好作罷…"));
        say_updates.push((rid.to_string(), format!("抱歉，{mname}我真的備不出來…")));
        return 0;
    };

    // ① 礦井進行中：清幾格；挖到的實心方塊（石頭/泥土…）誠實歸她的小背包。
    if let Some(q) = task.quarry.clone() {
        if q.is_done() {
            task.quarry = None; // 這口井挖完了 → 同 tick 接著查料/合成/鋪
        } else {
            let (cells, next_idx) = {
                let world = hub().deltas.read().unwrap();
                vdt::quarry_step(&world, &q, vdt::QUARRY_CELLS_PER_STEP)
            }; // deltas 讀鎖釋放
            let body = {
                let res = hub().residents.read().unwrap();
                res.iter().find(|r| r.id == rid).map(|r| (r.body.x, r.body.y, r.body.z))
            }; // residents 讀鎖釋放
            for (x, y, z, prev) in cells {
                // 安全：不動與她身體重疊的格（站在井口旁作業，永不自挖站位）。
                if let Some((px, py, pz)) = body {
                    if vdt::cell_in_body(x, y, z, px, py, pz) {
                        continue;
                    }
                }
                {
                    let mut world = hub().deltas.write().unwrap();
                    voxel::set_block(&mut world, x, y, z, Block::Air);
                } // deltas 寫鎖釋放
                broadcast_block(x, y, z, Block::Air);
                // 水流動：挖穿水脈讓水流進坑道是誠實的物理 → 喚醒鄰格重算。
                enqueue_water_around(x, y, z);
                vbuild::append_world_block(x, y, z, Block::Air as u8);
                {
                    let mut inv = hub().res_inv.write().unwrap();
                    *inv.entry(rid.to_string()).or_default().entry(prev as u8).or_insert(0) += 1;
                } // res_inv 寫鎖釋放
            }
            task.quarry = Some(vdt::QuarryDig { cells: q.cells, idx: next_idx });
            task.deadline = vdt::PAVE_DEADLINE_SECS; // 挖井＝有進展 → 續期
            hub().directed_tasks.write().unwrap().insert(rid.to_string(), task);
            return 1; // 礦井一批（幾個單格）保守記 1 柱預算
        }
    }

    // ② 乾跑：這批（受剩餘全域上限剪裁）要改哪些方塊、要吃幾份材料。
    let (changes, next_cursor, need) = {
        let world = hub().deltas.read().unwrap();
        vdt::pave_step_capped(&world, &task, cols_budget)
    }; // deltas 讀鎖釋放

    // ③ 背包原料就地合成到夠 + 讀存量（res_inv 短鎖一次做完）。
    let raw_id = prov.raw.block_id();
    let (crafted, have_mat, prov_sum) = {
        let mut inv = hub().res_inv.write().unwrap();
        let bag = inv.entry(rid.to_string()).or_default();
        let crafted = vdt::craft_toward(bag, mat, need);
        let have_mat = bag.get(&mid).copied().unwrap_or(0);
        let raw = bag.get(&raw_id).copied().unwrap_or(0);
        (crafted, have_mat, have_mat + raw)
    }; // res_inv 寫鎖釋放
    if crafted > 0 {
        say_updates.push((rid.to_string(), format!("合成出{crafted}份{mname}了！")));
    }
    // 備料總量有變動（採到料/礦井入袋/合成轉換）＝有進展 → 續期（防慢工被誤殺）。
    if prov_sum != task.prov_seen {
        task.prov_seen = prov_sum;
        task.deadline = vdt::PAVE_DEADLINE_SECS;
    }

    if have_mat >= need {
        // ④ 夠料：先誠實扣掉這批要用的份數，再套世界（同整地的套用迴圈與安全過濾）。
        {
            let mut inv = hub().res_inv.write().unwrap();
            let bag = inv.entry(rid.to_string()).or_default();
            let cur = bag.get(&mid).copied().unwrap_or(0);
            bag.insert(mid, cur.saturating_sub(need));
        } // res_inv 寫鎖釋放
        let body = {
            let res = hub().residents.read().unwrap();
            res.iter().find(|r| r.id == rid).map(|r| (r.body.x, r.body.y, r.body.z))
        }; // residents 讀鎖釋放
        let advanced = next_cursor.saturating_sub(task.cursor);
        for (x, y, z, b) in changes {
            if b.is_solid() {
                if let Some((px, py, pz)) = body {
                    if vdt::cell_in_body(x, y, z, px, py, pz) {
                        continue; // 不把自己埋進材料裡（沿用可逃精神）
                    }
                }
            }
            {
                let mut world = hub().deltas.write().unwrap();
                voxel::set_block(&mut world, x, y, z, b);
            } // deltas 寫鎖釋放
            broadcast_block(x, y, z, b);
            enqueue_water_around(x, y, z);
            vbuild::append_world_block(x, y, z, b as u8);
        }
        task.cursor = next_cursor;
        task.deadline = vdt::PAVE_DEADLINE_SECS; // 鋪了一批＝有進展 → 續期
        if task.is_complete() {
            // 取 requester 的玩家位置，計算相對方位（短鎖即釋）。
            let (done_dir, done_steps) = {
                let players = hub().players.read().unwrap();
                let req_pos = players.values().find(|p| p.name == task.requester).map(|p| (p.x, p.z));
                drop(players);
                if let Some((px, pz)) = req_pos {
                    vdt::cardinal_direction(px, pz, task.cx as f32, task.cz as f32)
                } else {
                    (String::new(), 0)
                }
            }; // players 讀鎖已釋放
            let done_cx = task.cx;
            let done_cz = task.cz;
            hub().directed_tasks.write().unwrap().remove(rid);
            vfeed::append_feed("鋪面", rname, &format!("把（{done_cx},{done_cz}）那塊地鋪上{mname}了！"));
            say_updates.push((rid.to_string(), vdt::pave_done_line(mname, &done_dir, done_steps, done_cx, done_cz)));
        } else {
            let pct = task.progress_pct();
            hub().directed_tasks.write().unwrap().insert(rid.to_string(), task);
            // 過半冒一句進度泡泡（低頻、不洗版；同整地節奏）。
            if (45..=55).contains(&pct) {
                say_updates.push((rid.to_string(), "鋪地中…快一半了～".to_string()));
            }
        }
        return advanced;
    }

    // ⑤ 缺料：找最近的地表原料（樹/沙/裸露的石頭…）→ 設採集技能（走去挖，挖完自然回工地）。
    let (rx, rz) = {
        let res = hub().residents.read().unwrap();
        res.iter()
            .find(|r| r.id == rid)
            .map(|r| (r.body.x.floor() as i32, r.body.z.floor() as i32))
            .unwrap_or((task.cx, task.cz))
    }; // residents 讀鎖釋放
    let found = {
        let world = hub().deltas.read().unwrap();
        vskill::find_nearest_resource_of(&world, rx, rz, vskill::GATHER_MAX_RADIUS, prov.raw)
    }; // deltas 讀鎖釋放
    if let Some((tx, ty, tz)) = found {
        {
            let mut res = hub().residents.write().unwrap();
            if let Some(r) = res.iter_mut().find(|r| r.id == rid) {
                r.gather = Some(GatherSkill {
                    resource: prov.raw,
                    tx,
                    ty,
                    tz,
                    timeout: vskill::GATHER_TIMEOUT_SECS,
                });
                // 重置「走向工地卡死」偵測：採集來回會拉開距離，回程別被誤判卡住而瞬移。
                r.level_best_d2 = f32::MAX;
                r.level_walk_stall = 0.0;
            }
        } // residents 寫鎖釋放
        hub().directed_tasks.write().unwrap().insert(rid.to_string(), task);
        return 0;
    }
    // ⑥ 地表沒有、原料埋在土石層（石頭/泥土）→ 開一口階梯礦井（重用 staircase_well 範本：
    //    邊挖邊留階、永遠走得回地面、不自困；井口在她東側一格、每口井往 +z 錯開）。
    if prov.quarryable && task.wells_dug < vdt::QUARRY_MAX_WELLS {
        let q = {
            let world = hub().deltas.read().unwrap();
            vdt::plan_quarry(&world, rx, rz, task.wells_dug)
        }; // deltas 讀鎖釋放
        if task.wells_dug == 0 {
            // 第一口井記一筆 Feed（低頻；後續的井不洗版）。
            vfeed::append_feed(
                "鋪面",
                rname,
                &format!("開挖階梯礦井備{}，要合{mname}用", prov.raw.display_name()),
            );
            say_updates.push((
                rid.to_string(),
                format!("我挖個階梯坑取{}～", prov.raw.display_name()),
            ));
        }
        task.wells_dug += 1;
        task.quarry = Some(q);
        task.deadline = vdt::PAVE_DEADLINE_SECS;
        hub().directed_tasks.write().unwrap().insert(rid.to_string(), task);
        return 0;
    }
    // ⑦ 誠實停工：原料真的補不上——鋪到哪算哪、說清楚（不是拒絕、也不是吹牛）。
    let done_some = task.cursor > 0;
    hub().directed_tasks.write().unwrap().remove(rid);
    if done_some {
        vfeed::append_feed(
            "鋪面",
            rname,
            &format!("{}補不上了，這片{mname}地先鋪到這裡", prov.raw.display_name()),
        );
        say_updates.push((
            rid.to_string(),
            format!("{}不夠了…我先鋪到這裡，之後再補！", prov.raw.display_name()),
        ));
    } else {
        vfeed::append_feed(
            "鋪面",
            rname,
            &format!("附近採不到{}，鋪{mname}地只好作罷", prov.raw.display_name()),
        );
        say_updates.push((
            rid.to_string(),
            format!("附近採不到{}，這次鋪不成了…", prov.raw.display_name()),
        ));
    }
    0
}

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

/// 推進一位居民的發明/重用計畫一步（agency tick 到期、且她沒在採集時呼叫）。
/// 真進化第一刀＋第二刀的**確定性執行引擎**：全程短鎖循序、不巢狀、不 await（守死鎖鐵律）。
/// - 採集步走既有 GatherSkill 機制（含可逃性判定，永不自困）；
/// - 合成步 grounded 在真配方表、即時完成；
/// - 放置步（第二刀）：找腳邊合理空位（絕不放自己身體格、放得到才算）→ 扣背包 →
///   寫世界＋廣播＋持久化（比照居民建造的放置語意）；
/// - 工作台合成步（第二刀）：先驗「附近真的有已放置的工作台」（她剛放的也算）再套 3×3 配方；
/// - 逾時/缺料/合成失敗/放不了 → 計畫失敗（記教訓、不存技能）；
/// - 全步驟完成且後置條件成立（背包真的有目標材料）→ 交給 [`finish_invent_run`] 收尾。
fn advance_invent_run(
    rid: &str,
    rname: &str,
    rx: i32,
    ry: i32,
    rz: i32,
    say_updates: &mut Vec<(String, String)>,
) {
    // 取 run 快照（residents 讀鎖即釋）。
    let run = {
        let residents = hub().residents.read().unwrap();
        residents.iter().find(|r| r.id == rid).and_then(|r| r.invent_run.clone())
    }; // residents 讀鎖釋放
    let Some(mut run) = run else { return };

    // 逾時 → 放棄（失敗收尾；deadline 由 tick 每 0.1s 遞減）。
    if run.is_expired() {
        finish_invent_run(rid, rname, run, false, say_updates);
        return;
    }

    // 站點查詢（第二刀）：放置步的後置條件（已有就跳過）與 3×3 的前提都靠它。
    // 每次呼叫短取 deltas 讀鎖即釋——她這一輪剛放好的工作台，同輪重查馬上看得到。
    let station_near = |bid: u8| -> bool {
        let world = hub().deltas.read().unwrap();
        vinvent::station_nearby(&world, rx, ry, rz, bid)
    }; // 每次呼叫內取放，不跨步驟持鎖

    // 步驟推進（可能一次跨多步：後置條件已滿足的採集步直接跳過、合成步即時完成）。
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 64 {
            break; // 防禦性上限（存檔鏈 ≤ 24 步，理論到不了；到了就寫回進度等下輪）
        }
        let bag: HashMap<u8, u32> = {
            let inv = hub().res_inv.read().unwrap();
            inv.get(rid).cloned().unwrap_or_default()
        }; // res_inv 讀鎖釋放
        match vinvent::next_action(&run, &bag, &station_near) {
            vinvent::StepAction::Advance => {
                run.step_idx += 1;
            }
            vinvent::StepAction::StartGather { resource } => {
                // 指名採集該資源：找得到 → 設 GatherSkill（走既有安全機制，挖到入背包）；
                // 附近真的沒有 → 誠實失敗（記教訓），不會為了執行計畫漫遊卡死。
                let found = {
                    let world = hub().deltas.read().unwrap();
                    vskill::find_nearest_resource_of(
                        &world, rx, rz, vinvent::INVENT_GATHER_RADIUS, resource,
                    )
                }; // deltas 讀鎖釋放
                match found {
                    Some((tx, ty, tz)) => {
                        {
                            let mut residents = hub().residents.write().unwrap();
                            if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                                r.gather = Some(GatherSkill {
                                    resource,
                                    tx,
                                    ty,
                                    tz,
                                    timeout: vskill::GATHER_TIMEOUT_SECS,
                                });
                                r.invent_run = Some(run);
                            }
                        } // residents 寫鎖釋放
                        return; // 這輪去採；材料入背包後，下個 agency tick 再推進。
                    }
                    None => {
                        // 擴大半徑後仍找不到資源 → 快速誠實失敗（別等逾時）。
                        // Feed 一句有人味的「這附近找不到…」，比沉默更有感。
                        let res_name = resource.display_name();
                        vfeed::append_feed(
                            "採集受阻",
                            rname,
                            &vinvent::backoff_no_resource_feed(&run.goal_name, res_name),
                        );
                        finish_invent_run(rid, rname, run, false, say_updates);
                        return;
                    }
                }
            }
            vinvent::StepAction::DoCraft { recipe_id } => {
                let Some(recipe) = vcraft::find_recipe(recipe_id) else {
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                };
                let crafted = {
                    let mut inv = hub().res_inv.write().unwrap();
                    vinvent::craft_apply(inv.entry(rid.to_string()).or_default(), recipe)
                }; // res_inv 寫鎖釋放
                if crafted {
                    say_updates.push((rid.to_string(), format!("合成出{}了！", recipe.name_zh)));
                    run.step_idx += 1;
                } else {
                    // 照計畫走到這裡卻備料不足（計畫順序排錯了）→ 這次發明失敗、記教訓。
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                }
            }
            vinvent::StepAction::DoCraftWb { recipe_id } => {
                // 工作台 3×3 合成（第二刀）：世界前提——附近真的有已放置的工作台
                // （她這條鏈剛放的也算；被人挖走了就誠實失敗，不隔空合成）。
                if !station_near(vinvent::WORKBENCH_BLOCK_ID) {
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                }
                let Some(recipe) = vcraft::find_workbench_recipe(recipe_id) else {
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                };
                let crafted = {
                    let mut inv = hub().res_inv.write().unwrap();
                    vinvent::craft_apply(inv.entry(rid.to_string()).or_default(), recipe)
                }; // res_inv 寫鎖釋放
                if crafted {
                    say_updates.push((
                        rid.to_string(),
                        format!("在工作台合成出{}了！", recipe.name_zh),
                    ));
                    run.step_idx += 1;
                } else {
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                }
            }
            vinvent::StepAction::DoPlace { block_id } => {
                // 放置站點（第二刀）：找腳邊合理空位（絕不放自己身體格、目標格必須是
                // 空氣、伸手可及）→ 扣背包 → 寫世界＋廣播＋持久化（比照居民建造）。
                let spot = {
                    let world = hub().deltas.read().unwrap();
                    vinvent::find_place_spot(&world, rx, ry, rz)
                }; // deltas 讀鎖釋放
                let (Some((px, py, pz)), Some(block)) = (spot, Block::from_u8(block_id)) else {
                    // 腳邊沒有合理放置點（或 id 異常）→ 放不到就不算，誠實失敗。
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                };
                // 先扣背包（res_inv 寫鎖即釋）；沒貨＝計畫排錯（模擬理應擋住）→ 誠實失敗。
                let taken = {
                    let mut inv = hub().res_inv.write().unwrap();
                    vinvent::take_one(inv.entry(rid.to_string()).or_default(), block_id)
                }; // res_inv 寫鎖釋放
                if !taken {
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                }
                {
                    let mut world = hub().deltas.write().unwrap();
                    voxel::set_block(&mut world, px, py, pz, block);
                } // deltas 寫鎖釋放
                broadcast_block(px, py, pz, block);
                // 水流動：放下的方塊可能堵住水路 → 喚醒鄰格重算（同居民建造慣例）。
                enqueue_water_around(px, py, pz);
                // 持久化這塊（重啟後她放的工作台還在——重用技能時直接沿用）。
                vbuild::append_world_block(px, py, pz, block_id);
                say_updates.push((
                    rid.to_string(),
                    vinvent::placed_line(vinvent::material_name(block_id)),
                ));
                run.step_idx += 1;
            }
            vinvent::StepAction::Done => {
                // 最終後置條件驗證：背包**真的**有目標材料，才算「她做出來了」。
                let met = {
                    let inv = hub().res_inv.read().unwrap();
                    inv.get(rid).map_or(false, |b| vinvent::goal_met(b, run.goal_block))
                }; // res_inv 讀鎖釋放
                finish_invent_run(rid, rname, run, met, say_updates);
                return;
            }
        }
    }
    // 防禦性出口：寫回進度，下輪 agency tick 續跑。
    let mut residents = hub().residents.write().unwrap();
    if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
        r.invent_run = Some(run);
    }
}

/// 收尾一次發明/重用計畫：清掉居民身上的 run；
/// - 成功且是**首次發明** → 存成她的具名技能（append-only 持久化）+「我學會了」泡泡
///   + Feed + 記憶（日記走既有事件管道自然反映）——**維護者看得到的進化時刻**；
/// - 成功且是**重用** → 重用 Feed/泡泡（熟練、一次到位、零 LLM）；
/// - 失敗 → 序列不存；首次發明記一次「教訓」進記憶（重用失敗多半是環境暫時問題，不記）。
fn finish_invent_run(
    rid: &str,
    rname: &str,
    run: vinvent::InventRun,
    success: bool,
    say_updates: &mut Vec<(String, String)>,
) {
    {
        let mut residents = hub().residents.write().unwrap();
        if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
            r.invent_run = None;
        }
    } // residents 寫鎖釋放
    if success {
        if run.reuse {
            tracing::info!(resident = %rid, skill = %run.skill_name, "技能重用完成（零 LLM）");
            say_updates.push((rid.to_string(), format!("{}到手～輕車熟路！", run.goal_name)));
            vfeed::append_feed(
                "技能重用",
                rname,
                &vinvent::reuse_feed(&run.skill_name, &run.goal_name),
            );
        } else {
            // 首次發明成功 → 存成她自己的技能（個體的、具名的、持久的）。
            // **正規化成自足技能**：補上「確保配料足夠」的採集步——她發明時背包裡剛好有料
            // 的話，計畫可能只有合成步；存檔版必須從空背包也能執行（技能是帶著走的本事）。
            let canonical = vinvent::canonicalize_steps(&run.steps);
            // 正規化版須過存檔白名單（理論必過；防禦性 fallback 存她原計畫）。
            let (store_steps, feed_steps) = match vinvent::check_stored_steps(&canonical) {
                Some(checked) => (canonical, checked),
                None => (run.raw_steps.clone(), run.steps.clone()),
            };
            let rec = {
                let mut inv = hub().invented.write().unwrap();
                inv.add(rid, &run.skill_name, run.goal_block, store_steps)
            }; // invented 寫鎖釋放
            if let Some(rec) = rec {
                vinvent::append_invented_skill(&rec);
            }
            tracing::info!(resident = %rid, skill = %run.skill_name, "技能發明成功：存入個人技能庫");
            say_updates.push((rid.to_string(), vinvent::learned_line(&run.skill_name)));
            vfeed::append_feed(
                "學會技能",
                rname,
                &vinvent::learned_feed(&run.skill_name, &run.goal_name, &feed_steps),
            );
            // 寫進她的記憶（add_memory 是既有管道；日記/回想都會自然帶到這件事）。
            let entry = {
                let mut mem = hub().memory.write().unwrap();
                mem.add_memory(
                    rid,
                    vdes::SELF_SPARK,
                    &vinvent::learned_memory(&run.skill_name, &run.goal_name),
                )
            }; // memory 寫鎖釋放
            vmem::append_memory(&entry);
        }
    } else {
        tracing::info!(
            resident = %rid, skill = %run.skill_name, reuse = run.reuse,
            step = run.step_idx, plan = %vinvent::steps_summary(&run.steps),
            "發明/重用計畫未完成（逾時/缺料/合成失敗/放不了）——step 指到失敗那一步"
        );
        if !run.reuse {
            // 失敗的序列不存；記一次教訓（她記得試過、下次換路子）。
            let entry = {
                let mut mem = hub().memory.write().unwrap();
                mem.add_memory(rid, vdes::SELF_SPARK, &vinvent::fail_lesson(&run.goal_name))
            }; // memory 寫鎖釋放
            vmem::append_memory(&entry);

            // 退避計數（#972 防鬼打牆）：連敗 N 次同一目標 → 進退避、換方向探索。
            let entered_backoff = {
                let mut residents = hub().residents.write().unwrap();
                if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                    let count = r.invent_fail_counts.entry(run.goal_block).or_insert(0);
                    *count += 1;
                    if *count >= vinvent::INVENT_BACKOFF_THRESHOLD {
                        *count = 0; // 重置計數，退避到期後可重試
                        r.invent_backoff
                            .insert(run.goal_block, vinvent::INVENT_BACKOFF_SECS);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }; // residents 寫鎖釋放

            if entered_backoff {
                // 進退避：Feed 一句有人味的，並冒泡提示換目標。
                vfeed::append_feed(
                    "發明退避",
                    rname,
                    &vinvent::backoff_switch_feed(&run.goal_name),
                );
                say_updates.push((rid.to_string(), vinvent::backoff_switch_line(&run.goal_name)));
                tracing::info!(
                    resident = %rid, goal = %run.goal_name,
                    "好奇心退避：連敗達門檻，暫停嘗試此目標"
                );
            } else {
                say_updates.push((rid.to_string(), "唔……這次沒成，再想想。".to_string()));
            }
        }
    }
}

/// 為一位居民發起一次「發明」：無鎖 async 請**便宜腦**（`agent_llm_chat` 思考路由：
/// ollama→Cerebras→Gemini，**不碰 Groq**——把玩家對話額度留給玩家）提出原語序列計畫。
/// 解析+白名單驗證通過才投回 `invent_proposals`；腦沒回/輸出不合白名單 → 安靜放棄
/// （冷卻已在呼叫端設好，絕不 retry 風暴、絕不 panic）。
/// 設 `BUTFUN_INVENT_FIXED_PLAN` 時改用固定計畫（**僅隔離實測用**，prod 不設）。
/// `wb_nearby`（第二刀）：她附近是否已有放置好的工作台——世界事實快照，由呼叫端
/// 查好傳入（prompt 據此告訴腦「不必再做一個」；可行性模擬據此判 3×3 依賴順序）。
fn spawn_invention(
    rid: String,
    rname: &'static str,
    goal: vinvent::MaterialGoal,
    desire: String,
    wb_nearby: bool,
) {
    // 防重入：這位居民已有一筆發明在等腦回來，就不再發（LLM 可能比冷卻慢）。
    {
        let mut inflight = hub().inventing.lock().unwrap();
        if !inflight.insert(rid.clone()) {
            return;
        }
    } // inventing mutex 釋放
    // 背包現況快照（短鎖）：讓腦知道她手上已有什麼（計畫可以少採幾步），
    // 同一份也拿去做「計畫可行性模擬」（提案階段就抓出算術不通的計畫）。
    let bag_snap: HashMap<u8, u32> = {
        let inv = hub().res_inv.read().unwrap();
        inv.get(&rid).cloned().unwrap_or_default()
    }; // res_inv 讀鎖釋放
    let bag_note: String = bag_snap
        .iter()
        .filter(|(_, c)| **c > 0)
        .map(|(bid, c)| format!("{}×{}", block_name_zh(*bid), c))
        .collect::<Vec<_>>()
        .join("、");
    tokio::spawn(async move {
        // 解析 + 白名單 + 正規化 + 可行性模擬 → Ok(計畫) 或 Err(可回饋給腦的繁中原因)。
        let validate = |raw: &str| -> Result<vinvent::InventedPlan, String> {
            // 提案接受管線（見 accept_proposal）：腦出**結構**（選對配方、排對依賴、
            // 取名字），引擎補**算術**（確定性備料）；失敗回具體錯處（Voyager 式回饋）。
            vinvent::accept_proposal(raw, &bag_snap, goal.block_id, wb_nearby)
        };
        let (sys, user) = vinvent::invention_prompt(rname, &goal, &desire, &bag_note, wb_nearby);
        let (raw, injected) = if let Some(fixed) = vinvent::fixed_plan_env() {
            // 測試注入（僅隔離實測；日誌標明，方便回報時區分「真腦」與「注入」）。
            tracing::info!(resident = %rid, "技能發明：使用 BUTFUN_INVENT_FIXED_PLAN 測試注入計畫");
            (Some(fixed), true)
        } else {
            tracing::info!(resident = %rid, goal = %goal.name_zh, "技能發明：請便宜腦（think 路由）提案中…");
            (crate::npc_chat::agent_llm_chat(&sys, &user).await, false)
        };
        let mut accepted: Option<vinvent::InventedPlan> = None;
        if let Some(raw1) = raw.as_deref() {
            match validate(raw1) {
                Ok(p) => accepted = Some(p),
                Err(reason) => {
                    tracing::info!(
                        resident = %rid, reason = %reason,
                        raw = %raw1.chars().take(200).collect::<String>(),
                        "技能發明：第一次計畫不可行 → 帶原因請腦修正（僅重試一次）"
                    );
                    // Voyager 式迭代精煉：帶失敗原因重試**一次**（成本有界）。測試注入不重試。
                    if !injected {
                        let user2 = vinvent::retry_user_prompt(&user, raw1, &reason);
                        if let Some(raw2) = crate::npc_chat::agent_llm_chat(&sys, &user2).await {
                            match validate(&raw2) {
                                Ok(p) => accepted = Some(p),
                                Err(reason2) => tracing::info!(
                                    resident = %rid, reason = %reason2,
                                    raw = %raw2.chars().take(200).collect::<String>(),
                                    "技能發明：修正後仍不可行 → 本次放棄（冷卻中）"
                                ),
                            }
                        }
                    }
                }
            }
        } else {
            tracing::info!(resident = %rid, "技能發明：腦沒回 → 本次放棄（冷卻中）");
        }
        if let Some(plan) = accepted {
            tracing::info!(resident = %rid, skill = %plan.name, "技能發明：計畫解析+白名單+可行性模擬通過");
            hub().invent_proposals.lock().unwrap().push((
                rid.clone(),
                goal.block_id,
                goal.name_zh.to_string(),
                plan,
            ));
        }
        hub().inventing.lock().unwrap().remove(&rid);
    });
}

/// 開始蓋一個建物：以「家域中心」為基準、依格位序號散開錨點 → 建計畫 → 動工 Feed + 冒泡。
/// `offset_seq`：一般建造傳「已蓋數量」（0..4）；擴建傳 `BUILD_PROGRESSION.len() + 已擴建次數`
/// （落在 `build_offset` 原本就多留的第 5/6 格位，不與基礎四座重疊）。
/// `is_expansion`：完工時要記到 `GoalStore::mark_expansion` 而非 `mark_done`。
fn start_build(
    rid: &str,
    rname: &str,
    kind: vbuild::BuildKind,
    offset_seq: usize,
    say_updates: &mut Vec<(String, String)>,
    is_expansion: bool,
    inspired_by: Option<String>,
) {
    let (ox, oz) = vskill::build_offset(offset_seq);
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
    // 蓋家鬼打牆根治（機制性硬閘）：這個確切錨點若已完工過這種建物，直接不動工。
    // 這是「地上已有這座 → 就不再蓋」的硬事實，不倚賴 done_kinds/expansion_count 每 tick 重推
    // （那組讀值一旦因重啟／競態短暫失真就會鬼打牆）；水把井裡的水沖走也不會讓它誤判「還沒蓋」。
    if hub().goals.read().unwrap().anchor_built(rid, kind, (bx, by, bz)) {
        return;
    }
    let plan = {
        let mut builds = hub().builds.write().unwrap();
        if builds.has_plan(rid) {
            None // double-check 並發安全
        } else {
            Some(builds.new_plan(rid, kind, bx, by, bz, is_expansion, inspired_by))
        }
    }; // builds 寫鎖釋放
    if let Some(p) = plan {
        vbuild::append_build(&p);
        let say = if is_expansion {
            format!("我已經有{}了，但這次還想再蓋一座！", p.kind_name)
        } else {
            vbuild::build_say_line(&p.kind_name, 0)
        };
        say_updates.push((rid.to_string(), say));
        let feed_kind = if is_expansion { "蓋家擴建動工" } else { "蓋家動工" };
        vfeed::append_feed(feed_kind, rname, &p.kind_name);
    }
}

/// 重設某居民的 agency tick 倒數（下次再決策／放塊）。
/// `interval`：心情決定的建造間隔（由 `voxel_mood::build_interval_secs` 計算）。
fn reset_build_tick(rid: &str, interval: f32) {
    let mut residents = hub().residents.write().unwrap();
    if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
        r.build_tick = interval;
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
            if d.sparked_by == vdes::CURIOSITY_SPARK {
                format!(
                    "你最近自己起了好奇心：「{}」——沒有人教你，你正想自己摸索著做出來。",
                    d.desire
                )
            } else if d.sparked_by == vdes::SELF_SPARK {
                format!(
                    "你自己心底浮現過一個念頭：「{}」——\
                    這個夢想是你生活的動力，偶爾在心裡默默惦記著它。",
                    d.desire
                )
            } else {
                format!(
                    "你有個從旅人的話裡種下的心願：「{}」（由「{}」帶給你的啟發）——\
                    這個夢想是你生活的動力，偶爾在心裡默默惦記著它。",
                    d.desire, d.sparked_by
                )
            }
        })
    }; // 心願讀鎖在此釋放
    // 可能性目錄（好奇心第三刀）：把「世界上做得出、她還不會」的知識注入自主思考／
    // 許願 prompt——心願腦不再對配方世界一無所知，許的願自然往「做得到的東西」靠。
    // **只在 think/pray 路徑**（talk 不注入——省 token、不影響對話）。
    let catalog_note: Option<String> = {
        let inv = hub().invented.read().unwrap();
        vinvent::catalog_note(&vinvent::possibility_catalog(&inv.known_goals_for(&id)))
    }; // invented 讀鎖釋放
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
    // 短鎖讀情誼+記憶，計算居民心情（ROADMAP 676）——循序取鎖，不巢狀。
    let (mood_sense_value, mood_note): (i32, String) = {
        let (friends, acq) = {
            let bonds = hub().bonds.read().unwrap();
            resident_bond_counts(&bonds, &id)
        }; // bonds 讀鎖釋放
        let mems = {
            let mem = hub().memory.read().unwrap();
            mem.memory_count(&id)
        }; // memory 讀鎖釋放
        let tier = voxel_mood::compute_mood(friends, acq, mems);
        (
            voxel_mood::mood_to_sense_value(tier),
            format!("你的心情：{}", voxel_mood::mood_description_zh(tier)),
        )
    };
    let world_news = {
        let mut parts =
            vec!["你生活在新生的『乙太方界』——一片由方塊構成的清淨天地。".to_string()];
        if !recall_note.is_empty() {
            parts.push(format!("你記得：{recall_note}。"));
        }
        if let Some(note) = resident_desire_note {
            parts.push(note);
        }
        if let Some(note) = catalog_note {
            parts.push(note);
        }
        if !social_note.is_empty() {
            parts.push(social_note);
        }
        parts.push(mood_note);
        parts.concat()
    };
    let sense = SenseInput {
        x,
        y: z,
        hp: 100,
        max_hp: 100,
        energy: 80,
        mood: mood_sense_value,
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
                // ROADMAP 6「禱告從一次性句子變成持久渴望」：這句禱告若能分類出具體
                // 建物種類（小屋/水井/花圃/瞭望塔），就同步設成居民的當前心願——之後
                // choose_activity 會真的把它蓋出來，不再只是浮現又消失的一句泡泡。
                if vbuild::prayer_promotable(&prayer) {
                    let new_desire = {
                        let mut des = hub().desires.write().unwrap();
                        des.set_desire(&id, &prayer, vdes::SELF_SPARK)
                    }; // 心願寫鎖在此釋放
                    vdes::append_desire(&new_desire);
                    vfeed::append_feed("新心願", &new_desire.resident, &new_desire.desire);
                }
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

    // ── 情誼帳本鍵值一致性（ROADMAP 713 修復）────────────────────────────
    // 情誼帳本以居民「顯示名」記帳（record_visit 呼叫慣例皆傳 r.name），過去多處
    // 誤把 id 直接傳進 bond_counts_for/tier_of，鍵值不一致導致查詢永遠落空——
    // 已是老朋友的居民在心情計算/關係面板/聚會偵測裡全部被誤判成陌生人。
    // 這裡釘住修復後的 resident_bond_counts/resident_tier_of helper 行為正確，
    // 且證明「直接用 id 查」（舊 bug 行為）確實查不到，兩者是不同鍵。

    #[test]
    fn resident_bond_counts_finds_visits_recorded_by_name() {
        let mut bonds = ResidentBonds::new();
        // 累積到相識門檻（3 次），用名字記帳——模擬 bond_arrive_events 的真實呼叫方式。
        for _ in 0..vbonds::ACQUAINTANCE_VISITS {
            bonds.record_visit("露娜", "諾娃");
        }
        // id 轉名字後查詢，應該看得到剛剛記的這段情誼。
        let (friends, acq) = resident_bond_counts(&bonds, "vox_res_0"); // 露娜
        assert_eq!((friends, acq), (0, 1), "露娜與諾娃應為 1 位相識");
        let (friends_b, acq_b) = resident_bond_counts(&bonds, "vox_res_1"); // 諾娃
        assert_eq!((friends_b, acq_b), (0, 1), "情誼對稱，諾娃視角亦同");
    }

    #[test]
    fn resident_bond_counts_direct_id_lookup_would_miss_it() {
        // 對照組：驗證帳本確實用名字為鍵——直接拿 id 字串查（舊 bug 的行為）查不到任何情誼，
        // 證明 resident_bond_counts 的「id→名字轉換」才是修復的關鍵，而非巧合過關。
        let mut bonds = ResidentBonds::new();
        bonds.record_visit("露娜", "諾娃");
        let (friends, acq) = bonds.bond_counts_for("vox_res_0", &["vox_res_0", "vox_res_1"]);
        assert_eq!((friends, acq), (0, 0), "用 id 當鍵查詢應查無資料（與名字鍵不同）");
    }

    #[test]
    fn resident_tier_of_finds_friend_tier_recorded_by_name() {
        let mut bonds = ResidentBonds::new();
        for _ in 0..vbonds::FRIEND_VISITS {
            bonds.record_visit("賽勒", "奧瑞");
        }
        // vox_res_2=賽勒、vox_res_3=奧瑞（見 RESIDENT_NAMES 順序）。
        let tier = resident_tier_of(&bonds, "vox_res_2", "vox_res_3");
        assert_eq!(tier, vbonds::BondTier::Friend, "應能查到老朋友層級，供聚會/面板使用");
    }

    // ── 問居民學配方 ─────────────────────────────────────────────
    #[test]
    fn detect_recipe_query_catches_making_questions() {
        // 意圖詞 + 產物名 → 命中。
        assert_eq!(detect_recipe_query("怎麼做玻璃"), vec!["玻璃"]);
        assert_eq!(detect_recipe_query("玻璃怎麼合"), vec!["玻璃"]);
        assert_eq!(detect_recipe_query("木板怎麼弄"), vec!["木板"]);
        assert_eq!(detect_recipe_query("我想合成石磚"), vec!["石磚"]);
        assert_eq!(detect_recipe_query("工作台的配方是什麼"), vec!["工作台"]);
    }

    #[test]
    fn detect_recipe_query_ignores_chitchat() {
        // 純閒聊（沒帶產物名）→ 不觸發，即使含「做」。
        assert!(detect_recipe_query("你在做什麼呀").is_empty());
        assert!(detect_recipe_query("今天天氣真好").is_empty());
        // 只提到產物、沒有合成意圖 → 不觸發（單純讚嘆/閒聊）。
        assert!(detect_recipe_query("這玻璃好漂亮").is_empty());
        assert!(detect_recipe_query("木板是誰放的").is_empty());
        // 空字串安全。
        assert!(detect_recipe_query("").is_empty());
    }

    #[test]
    fn recipe_fact_line_matches_real_table() {
        // 玻璃背包配方：2 沙 → 1 玻璃（對齊 voxel_craft）。
        let glass = vcraft::find_recipe("glass").unwrap();
        let line = recipe_fact_line(glass, false);
        assert!(line.contains("2 個沙子"), "玻璃要 2 沙：{line}");
        assert!(line.contains("背包 2×2 合成格"), "在背包合成：{line}");
        assert!(line.contains("1 個玻璃"), "產出 1 玻璃：{line}");
        // 工作台大量玻璃：6 沙 → 8 玻璃。
        let glass_wb = vcraft::find_workbench_recipe("glass_wb").unwrap();
        let wb_line = recipe_fact_line(glass_wb, true);
        assert!(wb_line.contains("6 個沙子") && wb_line.contains("8 個玻璃"), "{wb_line}");
        assert!(wb_line.contains("工作台 3×3"), "工作台合成：{wb_line}");
    }

    #[test]
    fn recipe_knowledge_block_is_grounded_and_complete() {
        // 問玻璃 → 區塊含背包與工作台兩套真實配方數字。
        let block = recipe_knowledge_block("玻璃怎麼合").expect("應產出配方事實");
        assert!(block.contains("2 個沙子"), "含背包配方：{block}");
        assert!(block.contains("6 個沙子"), "含工作台配方：{block}");
        assert!(block.contains("絕不可自行編造"), "含不准亂編的指引");
        // 木板配方：2 木 → 4 木板。
        let plank = recipe_knowledge_block("木板要怎麼製作").expect("木板應有配方");
        assert!(plank.contains("2 個木頭") && plank.contains("4 個木板"), "{plank}");
    }

    #[test]
    fn recipe_knowledge_block_none_for_chitchat() {
        // 一般閒聊不注入任何東西（省 token、不影響對話）。
        assert!(recipe_knowledge_block("你好呀今天過得如何").is_none());
        assert!(recipe_knowledge_block("這裡風景真美").is_none());
    }

    #[test]
    fn detect_recipe_query_dedups_multiple_mentions() {
        // 同一產物提兩次只回一個（去重）。
        assert_eq!(detect_recipe_query("玻璃啊玻璃怎麼合成"), vec!["玻璃"]);
    }

    // ── 超出居民能力請求偵測 ──────────────────────────────────────────────────

    #[test]
    fn detect_over_scope_catches_large_terraforming() {
        // B 階段（居民↔居民協調）起：整地動詞 + **世界級**暗示詞（連號召大家也做不到）→ 大規模整地婉拒。
        assert_eq!(
            detect_over_scope("你可以幫我把整個世界的地全部推平嗎"),
            Some("大規模整地")
        );
        assert_eq!(
            detect_over_scope("把所有的地都整平"),
            Some("大規模整地")
        );
        assert_eq!(
            detect_over_scope("幫我夷平整顆星球"),
            Some("大規模整地")
        );
        // 大範圍但不離譜（100×100/大片/整片）→ 不再婉拒，改導向協調整地 → None（走協調分支）。
        assert_eq!(detect_over_scope("你可以幫我把這附近100×100的地全部推平嗎"), None);
        assert_eq!(detect_over_scope("把這一大片全部整地"), None);
        assert_eq!(detect_over_scope("幫我夷平這整片土地"), None);
        assert_eq!(detect_over_scope("把百格的地剷平"), None);
        // 指令→任務 + 整地技能 v1 起：合理大小的整地不再算超能力（居民做得到→該答應）。
        // 「這裡/這塊」沒有大範圍暗示 → None（交給 DirectedTask 去整）。
        assert_eq!(detect_over_scope("幫我整地"), None);
        assert_eq!(detect_over_scope("推平這塊地"), None);
        assert_eq!(detect_over_scope("幫我把這裡整平"), None);
    }

    #[test]
    fn detect_over_scope_catches_command_residents() {
        // 指揮/協調其他居民
        assert_eq!(
            detect_over_scope("叫其他居民一起來幫忙"),
            Some("指揮或協調其他居民")
        );
        assert_eq!(
            detect_over_scope("號召大家一起建家"),
            Some("指揮或協調其他居民")
        );
        assert_eq!(
            detect_over_scope("你去傳達給其他人"),
            Some("指揮或協調其他居民")
        );
        assert_eq!(
            detect_over_scope("幫我協調所有居民一起蓋"),
            Some("指揮或協調其他居民")
        );
    }

    #[test]
    fn detect_over_scope_catches_town_planning() {
        // 城鎮/國家規劃
        assert_eq!(
            detect_over_scope("幫我規劃城鎮的佈局"),
            Some("城鎮或國家規劃")
        );
        assert_eq!(
            detect_over_scope("我想建設城市"),
            Some("城鎮或國家規劃")
        );
        assert_eq!(
            detect_over_scope("你能幫我城鎮規劃嗎"),
            Some("城鎮或國家規劃")
        );
    }

    #[test]
    fn detect_over_scope_ignores_normal_chat() {
        // 一般聊天/採集/小蓋建不觸發
        assert!(detect_over_scope("你好呀，今天天氣真好").is_none());
        assert!(detect_over_scope("你在做什麼").is_none());
        assert!(detect_over_scope("幫我採點木頭").is_none());
        assert!(detect_over_scope("能不能幫我蓋個小屋").is_none());
        assert!(detect_over_scope("玻璃怎麼合成").is_none());
        assert!(detect_over_scope("我今天種了一些田").is_none());
        assert!(detect_over_scope("你叫什麼名字").is_none());
        assert!(detect_over_scope("").is_none());
    }

    #[test]
    fn over_scope_enforcement_block_has_required_phrases() {
        // 注入區塊：含類別名、含禁用詞列表、含允許的婉拒模板
        // B 階段起，只有「連協調也做不到」的世界級整地才婉拒（大範圍改走協調分支）。
        let block = over_scope_enforcement_block("幫我把整個世界的地全部推平")
            .expect("應產出強制注入文字");
        assert!(block.contains("大規模整地"), "應含類別名：{block}");
        assert!(block.contains("絕對做不到"), "應有硬性否定：{block}");
        assert!(block.contains("當然可以"), "應列出禁止句例：{block}");
        assert!(block.contains("一個人"), "應表示一個人做不到：{block}");
        // 普通閒聊不產出（不燒多餘 token）
        assert!(over_scope_enforcement_block("你在哪裡採石頭").is_none());
    }

    // ── 願望漏斗：玩家親口的願望直接種進被指名居民的心願 ─────────────────────

    #[test]
    fn player_speech_seeds_desire_with_player_as_sparker() {
        // 玩家原文（維護者實測原句）→ extract_desire 抽得到 → 種進被指名居民，
        // sparked_by=玩家名——完全不依賴她的 LLM 回覆願不願意複述。
        let clean = "露娜，真希望有玻璃啊";
        let desire = vdes::extract_desire(clean).expect("玩家親口的願望該被抽到");
        assert!(desire.contains("玻璃"), "{desire}");
        let mut store = DesireStore::new();
        let entry = store.set_desire("vox_res_0", &desire, "旅人小明");
        assert_eq!(entry.sparked_by, "旅人小明", "啟發者應是玩家本人");
        let got = store.get_desire("vox_res_0").expect("心願應立刻種上");
        assert!(got.desire.contains("玻璃"), "心願應含玻璃：{}", got.desire);
    }

    #[test]
    fn greeting_does_not_seed_desire() {
        // 寒暄不觸發（extract_desire 觸發詞 + 最短長度把關）→ 6b.7 不會種心願。
        assert!(vdes::extract_desire("你好呀，露娜！今天天氣真好").is_none());
        assert!(vdes::extract_desire("嗨！最近過得如何？").is_none());
        assert!(vdes::extract_desire("謝謝你幫我採木頭").is_none());
    }

    #[test]
    fn player_wish_prompt_note_mentions_wish_and_acknowledgement() {
        // 注入的情境提示要含願望原文 + 引導「記下了」的呼應，讓她不無感。
        let note = player_wish_prompt_note("真希望有玻璃");
        assert!(note.contains("真希望有玻璃"), "應含願望原文：{note}");
        assert!(note.contains("記下"), "應引導回應「記下了」：{note}");
        assert!(note.contains("不要無視"), "應硬性要求不可無感：{note}");
    }

    // ── 技能問答強制注入：被問「你會什麼」講得出發明技能 ─────────────────────

    #[test]
    fn skill_query_enforcement_block_contains_skill_names() {
        let names = vec!["燒玻璃".to_string(), "疊石成磚".to_string()];
        let block = skill_query_enforcement_block("你會什麼技能呀？", &names)
            .expect("問技能且有發明技能 → 應強制注入");
        assert!(block.contains("「燒玻璃」"), "應含第一個技能名：{block}");
        assert!(block.contains("「疊石成磚」"), "應含第二個技能名：{block}");
        assert!(block.contains("務必"), "應指令化硬性要求：{block}");
        assert!(block.contains("『燒玻璃』"), "應含可照抄的示範句：{block}");
    }

    #[test]
    fn skill_query_enforcement_block_none_when_not_asked_or_no_skill() {
        let names = vec!["燒玻璃".to_string()];
        // 沒發明過技能 → 不注入（就算在問）。
        assert!(skill_query_enforcement_block("你會什麼", &[]).is_none());
        // 一般閒聊 → 不注入（省 token）。
        assert!(skill_query_enforcement_block("今天天氣真好", &names).is_none());
        assert!(skill_query_enforcement_block("你在做什麼", &names).is_none());
        // 各種問法都該被偵測到。
        assert!(skill_query_enforcement_block("你會些什麼呢", &names).is_some());
        assert!(skill_query_enforcement_block("妳有什麼本事", &names).is_some());
        assert!(skill_query_enforcement_block("你有哪些技能", &names).is_some());
    }

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
    fn special_title_matches_by_email_or_name() {
        // 稱號表：兩位維護者各自的稱號（鍵為 email）。
        let titles = vec![
            ("suc12345@gmail.com".to_string(), "引夢使者".to_string()),
            ("shihshihder@shihshihder.com".to_string(), "築夢工匠".to_string()),
        ];
        let envoy: Vec<String> = vec!["濕濕的".to_string()]; // 舊版相容清單（顯示名保底）

        // email 命中 → 各自的稱號（email 忽略大小寫、去頭尾空白）。
        assert_eq!(
            special_title_match(Some("suc12345@gmail.com"), Some("濕濕的"), &titles, &envoy).as_deref(),
            Some("引夢使者")
        );
        assert_eq!(
            special_title_match(Some("  SUC12345@Gmail.com "), None, &titles, &envoy).as_deref(),
            Some("引夢使者")
        );
        // 第二個帳號 email → 築夢工匠（改顯示名也不掉，因為綁 email）。
        assert_eq!(
            special_title_match(
                Some("shihshihder@shihshihder.com"),
                Some("施育群改了個名"),
                &titles,
                &envoy
            )
            .as_deref(),
            Some("築夢工匠")
        );
        // 別的 email / 別的帳號 → None（專屬，零回歸）。
        assert_eq!(
            special_title_match(Some("someone@else.com"), Some("諾娃"), &titles, &envoy),
            None
        );
        // 舊版相容清單命中（顯示名）→ 引夢使者（改綁 email 後舊名仍認得）。
        assert_eq!(
            special_title_match(None, Some("濕濕的"), &titles, &envoy).as_deref(),
            Some("引夢使者")
        );
        // 訪客（email/name 皆 None）→ None。
        assert_eq!(special_title_match(None, None, &titles, &envoy), None);
        // 空稱號表 + 空清單 → 誰都沒稱號（含空帳號）。
        assert_eq!(special_title_match(Some(""), Some("  "), &[], &[]), None);
        // 稱號表以名字為鍵也可（鍵可為 email 或顯示名，大小寫敏感）。
        let by_name = vec![("匠人".to_string(), "築夢工匠".to_string())];
        assert_eq!(
            special_title_match(None, Some("匠人"), &by_name, &[]).as_deref(),
            Some("築夢工匠")
        );
    }

    #[test]
    fn parse_special_titles_and_envoy_list() {
        // 稱號表解析：逗號分隔、鍵=稱號、去頭尾空白、濾掉殘缺項。
        let t = parse_special_titles(
            "  a@b.com = 引夢使者 , c@d.com=築夢工匠 , 壞項無等號 , =空鍵 , 空值= ",
        );
        assert_eq!(
            t,
            vec![
                ("a@b.com".to_string(), "引夢使者".to_string()),
                ("c@d.com".to_string(), "築夢工匠".to_string()),
            ]
        );
        // 清單解析：逗號分隔、去頭尾空白、濾空。
        assert_eq!(
            parse_envoy_list("  濕濕的 , , suc12345@gmail.com ,  "),
            vec!["濕濕的".to_string(), "suc12345@gmail.com".to_string()]
        );
        assert!(parse_special_titles("").is_empty());
        assert!(parse_envoy_list("   , ,").is_empty());
    }

    #[test]
    fn special_title_env_override_and_defaults() {
        // env 未設 → 用預設稱號表（兩個 email 各自稱號）。
        std::env::remove_var("BUTFUN_SPECIAL_TITLES");
        std::env::remove_var("BUTFUN_DREAM_ENVOY");
        assert_eq!(
            special_title(Some("suc12345@gmail.com"), None).as_deref(),
            Some("引夢使者")
        );
        assert_eq!(
            special_title(Some("shihshihder@shihshihder.com"), None).as_deref(),
            Some("築夢工匠")
        );
        // 舊顯示名保底 → 引夢使者（改綁 email 後不掉）。
        assert_eq!(special_title(None, Some("濕濕的")).as_deref(), Some("引夢使者"));
        // 別的 email / 訪客 → None（零回歸）。
        assert_eq!(special_title(Some("nobody@x.com"), Some("諾娃")), None);
        assert_eq!(special_title(None, None), None);

        // env 可覆蓋（機敏 / 可調值走 env，不寫死）。
        std::env::set_var("BUTFUN_SPECIAL_TITLES", "  vip@x.com = 點火者  ");
        assert_eq!(special_title(Some("vip@x.com"), None).as_deref(), Some("點火者"));
        assert_eq!(special_title(Some("suc12345@gmail.com"), None), None); // 覆蓋後原預設不再中
        // env 設成空白 / 無有效項 → 退回預設。
        std::env::set_var("BUTFUN_SPECIAL_TITLES", "   ");
        assert_eq!(
            special_title(Some("suc12345@gmail.com"), None).as_deref(),
            Some("引夢使者")
        );
        std::env::remove_var("BUTFUN_SPECIAL_TITLES"); // 收尾，別汙染別的測試
    }

    #[test]
    fn special_title_talk_note_by_title() {
        // 引夢使者 → 敬愛點火者（沿用 #994）；築夢工匠 → 對匠人的信賴；其他 → None。
        let envoy = special_title_talk_note("引夢使者").expect("引夢使者應有注入");
        assert!(envoy.contains("引夢使者"));
        assert!(envoy.contains("敬愛"));
        assert!(envoy.contains("諂媚"));
        assert!(envoy.contains("坦白"));
        let builder = special_title_talk_note("築夢工匠").expect("築夢工匠應有注入");
        assert!(builder.contains("築夢工匠"));
        assert!(builder.contains("手藝"));
        assert!(builder.contains("諂媚"));
        assert!(builder.contains("坦白"));
        assert!(special_title_talk_note("路人甲").is_none());
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
    fn honesty_guide_non_empty_and_has_key_principles() {
        // 誠實指引本身：非空、含能力說明、含誠實原則關鍵詞。
        let guide = resident_honesty_guide();
        assert!(!guide.is_empty(), "誠實指引不應為空");
        // 能做的能力
        assert!(guide.contains("採集"), "應提到採集能力");
        assert!(guide.contains("小型結構"), "應提到可蓋小型結構");
        // 做不到的事
        assert!(guide.contains("做不到"), "應說明做不到的項目");
        assert!(guide.contains("整地"), "應提到無法大規模整地");
        // 誠實原則關鍵詞
        assert!(guide.contains("誠實"), "應含誠實原則段落");
        assert!(guide.contains("假裝"), "應告知別假裝做得到");
    }

    #[test]
    fn honesty_guide_injected_into_talk_prompt_for_all_personas() {
        // 所有人設下，system prompt 都應帶入誠實指引（治討好傾向）。
        let guide = resident_honesty_guide();
        for persona in [
            ResidentPersona::MarketBrowser,
            ResidentPersona::FarmWorker,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let sys = resident_talk_system_prompt("艾拉", persona, None);
            assert!(
                sys.contains(guide),
                "persona {:?} 的 prompt 應含誠實指引",
                persona
            );
        }
    }

    #[test]
    fn honesty_guide_injected_with_desire_note() {
        // 帶心願時，誠實指引仍要在 prompt 裡（心願不能擠掉誠實指引）。
        let guide = resident_honesty_guide();
        let sys = resident_talk_system_prompt(
            "諾娃",
            ResidentPersona::Wanderer,
            Some("在廣場蓋一座水井"),
        );
        assert!(sys.contains("水井"), "心願應注入 prompt");
        assert!(sys.contains(guide), "帶心願時誠實指引仍應存在");
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
    fn nearest_player_with_pos_returns_none_when_empty() {
        let empty: Vec<(f32, f32, String)> = vec![];
        assert!(nearest_player_with_pos(0.0, 0.0, &empty).is_none());
    }

    #[test]
    fn nearest_player_with_pos_single() {
        let pts = vec![(3.0, 4.0, "旅人".to_string())];
        let (d2, px, pz, name) = nearest_player_with_pos(0.0, 0.0, &pts).unwrap();
        assert!((d2 - 25.0).abs() < 1e-4, "平方距離應為 25：{d2}");
        assert!((px - 3.0).abs() < 1e-4);
        assert!((pz - 4.0).abs() < 1e-4);
        assert_eq!(name, "旅人");
    }

    #[test]
    fn nearest_player_with_pos_picks_closest() {
        let pts = vec![
            (10.0, 0.0, "遠人".to_string()),
            (1.0, 0.0, "近人".to_string()),
        ];
        let (d2, _, _, name) = nearest_player_with_pos(0.0, 0.0, &pts).unwrap();
        assert!((d2 - 1.0).abs() < 1e-4, "最近距離平方應為 1：{d2}");
        assert_eq!(name, "近人");
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
