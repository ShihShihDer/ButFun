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
use tokio::sync::{broadcast, mpsc, oneshot};
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
use crate::voxel_blueprint as vblueprint;
use crate::voxel_skills::{self as vskill, GatherSkill, GoalStore, NextActivity};
use crate::voxel_invent as vinvent;
use crate::voxel_desires::{self as vdes, DesireStore};
use crate::voxel_diary;
use crate::voxel_feed as vfeed;
use crate::voxel_village as vvillage;
use crate::voxel_discovery as vdisc;
use crate::voxel_landmark_note as vlmark;
use crate::voxel_craft as vcraft;
use crate::voxel_inventory::{self as vinv, InvStore};
use crate::voxel_memory::{self as vmem, VoxelMemory};
use crate::voxel_farm::{self as vfarm, FarmStore};
use crate::voxel_grove::{self as vgrove, GroveStore};
use crate::voxel_berry::{self as vberry, BerryStore};
use crate::voxel_coop::{self as vcoop, CoopStore};
use crate::voxel_gift as vgift;
use crate::voxel_keepsake as vkeep;
use crate::voxel_keepsake_recall as vkrecall;
use crate::voxel_humming as vhum;
use crate::voxel_bench as vbench;
use crate::voxel_bench_chat as vbenchchat;
use crate::voxel_bench_tiff as vbtiff;
use crate::voxel_anglerest as vangler;
use crate::voxel_raincover as vrain;
use crate::voxel_homegaze as vhome;
use crate::voxel_birthday as vbday;
use crate::voxel_campfire as vcamp;
use crate::voxel_campfire_tale as vtale;
use crate::voxel_bell as vbell;
use crate::voxel_seedgift as vseed;
use crate::voxel_giftgarden as vgg;
use crate::voxel_fishing as vfish;
use crate::voxel_player_stats as vstats;
use crate::voxel_smelt as vsmelt;
use crate::voxel_return_gift::{self as vret, ReturnGiftStore};
use crate::voxel_playercare as vcare;
use crate::voxel_admire as vadmire;
use crate::voxel_farm_admire as vfarmadmire;
use crate::voxel_structure_name as vstructname;
use crate::voxel_village_milestone as vvillms;
use crate::voxel_confide as vconfide;
use crate::voxel_request as vrequest;
use crate::voxel_witness as vwit;
use crate::voxel_friendtoken as vftoken;
use crate::voxel_preference as vpref;
use crate::voxel_overhear as vh;
use crate::voxel_relations::{self as vrel, SocialStore};
use crate::voxel_residents::{self as vr, Body};
use crate::voxel_roster as vroster;
use crate::voxel_time::{self as vt, WorldTime, TimePhase};
use crate::voxel_announce as vannounce;
use crate::voxel_bonds::{self as vbonds, ResidentBonds};
use crate::voxel_romance::{self as vromance, ResidentRomance};
use crate::voxel_lover_seek as vlover;
use crate::voxel_wildlife as vwild;
use crate::voxel_fish as vfishlife;
use crate::voxel_chicken as vchicken;
use crate::voxel_player_recipe as vprecipe;
use crate::voxel_diary_peek as vdiarypeek;
use crate::voxel_pet_admire as vpetadmire;
use crate::voxel_proximity_teach as vptteach;
use crate::voxel_treasure as vtreasure;
use crate::voxel_trade::{self as vtrade, TradeOffer};
use crate::voxel_visit as vvisit;
use crate::voxel_fond_greeting as vfond;
use crate::voxel_gossip as vgossip;
use crate::voxel_mood;
use crate::voxel_comfort as vcomfort;
use crate::voxel_hunger as vhunger;
use crate::voxel_share_meal as vsharemeal;
use crate::voxel_gratitude as vgrat;
use crate::voxel_ratelimit as vrl;
use crate::voxel_moderation as vmod;
use crate::voxel_cheer as vcheer;
use crate::voxel_chest as vchest;
use crate::voxel_chest_contribute as vchestgive;
use crate::voxel_envy as venvy;
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
use crate::voxel_player_home as vplayerhome;
use crate::voxel_weather as vweather;
use crate::voxel_season as vseason;
use crate::voxel_timely as vtimely;
use crate::voxel_bounty as vbounty;
use crate::voxel_stargaze as vstar;
use crate::voxel_firework as vfw;
use crate::voxel_compost as vcompost;
use crate::voxel_bucket as vbucket;
use crate::voxel_hoe as vhoe;
use crate::voxel_tool as vtool;
use crate::voxel_clique as vclique;
use crate::voxel_quarrel as vquarrel;
use crate::voxel_teach as vteach;
use crate::voxel_sleep as vsleep;
use crate::voxel_bedtime as vbedtime;
use crate::voxel_dream as vdream;
use crate::voxel_dreamshare as vdreamshare;
use crate::voxel_morning as vmorning;
use crate::voxel_daybreak as vdaybreak;
use crate::voxel_reunion as vreunion;
use crate::voxel_expedition as vexp;
use crate::voxel_frontier_visit as vfvisit;
use crate::voxel_illness as villness;
use crate::voxel_welcome as vwelcome;
use crate::voxel_resident_trade as vrtrade;
use crate::voxel_share as vshare;
use crate::voxel_milestones::{self as vmiles, MilestoneStore};
use crate::voxel_milestone_cheer as vmcheer;
use crate::voxel_player_pos as vpp;
use crate::voxel_bottle::{self as vbottle, BottleStore};
use crate::voxel_coop_gather as vcoop_gather;
use crate::voxel_dropitem::{self as vdrop, DropStore};
use crate::voxel_stall::{self as vstall, StallStore};
use crate::voxel_stall_notify as vstallnotify;
use crate::voxel_frontier_find as vffind;
use crate::voxel_mastery::{self as vmastery, MasteryKind, MasteryStore};
use crate::voxel_waypoint as vwaypoint;

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
    /// 後端 cookie→users 解出的登入帳號 email（權威，非客戶端自報）。
    /// **僅後端內部用**：同帳號去重（無痛重連幽靈分身修復）。訪客 = None。
    /// 不廣播（#[serde(skip)]），前端永遠看不到此值。
    #[serde(skip)]
    account: Option<String>,
    /// 手持工具可見 v1（自主提案切片）：前端隨 `Move` 自報目前熱鍵選中的物品 id。
    /// **純視覺 cosmetic**（不影響任何判定——挖礦加成走 790 `Break.tool` 各自驗證，
    /// 這裡只決定手上「看起來」拿什麼），信任等級比照 `say`：客戶端自報、伺服器照收廣播，
    /// 錯報頂多讓別人看到你手上的東西不準確，無任何利益可圖。空熱鍵格 = None，不序列化省頻寬。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    held: Option<u8>,
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
/// per-IP 對話限流被擋時，在玩家自己頭上冒的溫柔提示（治安三件套①）——
/// 讓超速的人知道「慢一點」，而非靜默吞掉；面向玩家字串、集中於此、i18n 友善。
const TALK_RATE_NOTICE: &str = "（說得太快啦，喘口氣、待會兒再聊～）";
/// 訪客（未登入）試圖與居民交談時，在自己頭上冒的溫柔提示（治安三件套③·對話需登入）——
/// 對話會觸發免費 LLM（居民的腦），匿名腳本可白嫖／燒爆額度；故要求登入才能聊，訪客可
/// 自由逛逛與觀看。面向玩家字串、集中於此、i18n 友善。
const TALK_GUEST_NOTICE: &str = "（登入之後就能和居民說話囉～先四處逛逛、看看這個世界吧！）";
/// 訪客（未登入）試圖丟漂流瓶時，在自己頭上冒的溫柔提示（漂流瓶 v1）——瓶中信是玩家
/// 留給玩家的自由文字，比照對話需登入的護欄，只有登入帳號才能寫、訪客可自由撿讀。
const BOTTLE_GUEST_NOTICE: &str = "（登入之後就能丟漂流瓶囉～現在可以自由撿別人的瓶子看看！）";
/// 訪客（未登入）試圖留地標留言時，在自己頭上冒的溫柔提示（地標旅人留言 v1）——留言簿
/// 署名給後來的旅人看，比照瓶中信同一套登入護欄，只有登入帳號才能寫、訪客可自由讀。
const LANDMARK_NOTE_GUEST_NOTICE: &str = "（登入之後就能留言給後來的旅人囉～）";
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
/// 每個合格 tick 觸發「主動聊心事」的機率（居民主動聊心事 v1，781）。設得比招呼低
/// （0.03）——掏心比打招呼更慎重；配合 240s 長冷卻＋好感門檻，整體稀有有份量。
const CONFIDE_CHANCE_PER_TICK: f32 = 0.03;
/// 每個合格 tick 觸發「主動向你討東西」的機率（居民拜託你幫個小忙 v1）。設得比掏心再低
/// （0.02）——開口拜託人比說心事更慎重；配合 300s 長冷卻＋好感門檻＋同時只掛一個請求，整體稀有有份量。
const REQUEST_CHANCE_PER_TICK: f32 = 0.02;
/// 每個合格 tick 觸發「教你一道獨門配方」的機率（居民教你一道獨門配方 v1，849）。設得比
/// 討東西更低（0.015）——這是一次性永久解鎖的大事件，配合 600s 長冷卻＋更高好感門檻，
/// 整體比掏心/討東西都更稀有有份量。
const TEACH_CHANCE_PER_TICK: f32 = 0.015;
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
    /// 主動掏心冷卻倒數（秒，居民主動聊心事 v1）：> 0 表示最近才跟某位玩家分享過心事，
    /// 暫不再掏心，讓「主動聊起自己的渴望」稀有有份量、天然防洗版。
    confide_timer: f32,
    /// 主動教學冷卻倒數（秒，居民教你一道獨門配方 v1）：> 0 表示最近才教過某位玩家一道獨門配方，
    /// 暫不再教，讓「教你一道能跟著你一輩子的本事」稀有有份量、天然防洗版。
    teach_timer: f32,
    /// 回饋糧倉冷卻倒數（秒，居民回饋糧倉 v1）：> 0 表示最近才把餘裕材料存進箱子過，
    /// 暫不再存，讓「回頭往箱子添東西」稀有有份量、天然防洗版。
    contribute_timer: f32,
    /// 主動討東西冷卻倒數（秒，居民拜託你幫個小忙 v1）：> 0 表示最近才向某位玩家開口討過材料，
    /// 暫不再開口，讓「反過來拜託你」稀有有份量、天然防洗版。
    request_timer: f32,
    /// 見賢思齊冷卻倒數（秒，居民見賢思齊 v1）：> 0 表示最近才因為路過一座已命名地標
    /// 心生嚮往過，暫不再觸發，讓「親眼所見萌生新心願」稀有有份量、天然防洗版。
    envy_timer: f32,
    /// 目前尚未了結的請求：`Some(item_id)` 表示居民正等著有人送這樣材料來（同時只掛一個）。
    /// 玩家把這樣材料當禮物送到即算幫上忙（走 `Gift` 送禮管線），送到後清成 `None`。
    /// 純記憶體、重啟歸零（與渴望 / 心事同款：未了的請求重啟後淡去，不落持久化）。
    open_request: Option<u8>,
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
    /// 發明採集·進行中的階梯礦井（記憶體、重啟歸零）：發明需要地下資源（石／泥）而地表無
    /// 天然源時，就地開一口 [`vskill::staircase_well`] 階梯井往下採（永不自困），挖到的實心
    /// 方塊誠實入背包。`finish_invent_run` 收尾時清空（每個 run 都經收尾，下個 run 起始必乾淨）。
    invent_quarry: Option<vdt::QuarryDig>,
    /// 本次發明已開的礦井數（守 [`vinvent::INVENT_MAX_WELLS`] 上限、並用來錯開每口井位置）。
    invent_quarry_wells: u32,
    /// 是否正在睡覺（日夜作息·睡覺 v1，ROADMAP 739）：深夜回到自家附近會躺下睡著，
    /// 睡著時停下一切閒晃／社交／採集／建造、名牌旁顯示 💤，天亮（離開夜間時段）才醒。
    /// 記憶體前置、不持久化、零 migration（重啟後大不了當晚重睡一次，無資料風險）。
    asleep: bool,
    /// 兩個夢之間的冷卻倒數（居民會做夢 v1，ROADMAP 805）：熟睡中做完一個夢後設為
    /// `vdream::DREAM_COOLDOWN_SECS`、每 tick 遞減，到期後才可能再做下一個夢，避免夢泡洗版。
    /// 純記憶體、重啟歸零（做夢是睡夢中的短暫過場，重啟大不了少做一次、無資料風險）。
    dream_cooldown: f32,
    /// 這位居民至今做過幾個夢（居民會做夢 v1，ROADMAP 805）：睡著時身體靜止、座標不變，
    /// 若只用座標當 `pick` 種子會整夜夢同一件事——摻入此計數讓每個夢輪替不同的往事／語氣。純記憶體。
    dream_seq: u32,
    /// 昨晚那個夢的核心，暫存等白天說給你聽（居民早上會把昨晚的夢說給你聽 v1，ROADMAP 807）：
    /// 做夢（805）時把夢核心存進來，白天遇到玩家分享一次後即清空 `None`；下次做夢再覆蓋成新的夢。
    /// 純記憶體、重啟歸零（分享夢是溫暖的偶發過場，重啟大不了少說一次、無資料風險）。
    last_dream: Option<String>,
    /// 主動分享夢的冷卻倒數（同上 807）：分享後設為 `vdreamshare::DREAMSHARE_COOLDOWN_SECS`、
    /// 每 tick 遞減，稀有才有份量、天然防洗版。純記憶體、各居民初始錯開。
    dreamshare_timer: f32,
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
    /// 心中地標其實是「哪位玩家的家」（居民認得你的家 v1，自主提案切片，ROADMAP 830）：Some(玩家名) =
    /// 上面 `cherished_sign` 那塊牌是該玩家親手署名、且牌面語氣被判成「家」的牌（`owner` 由伺服器權威
    /// 記下）；與 `cherished_neighbor` 互斥（一塊牌若是居民自建銘牌就走 750/751 那條路，不會同時是玩家的
    /// 家）。None = 不是任何玩家的家（訪客的牌／指路牌／舊資料等）。純記憶體、重啟歸零。
    cherished_player: Option<String>,
    /// 這趟朝聖走向的其實是「哪位玩家的家」（居民認得你的家 v1）：啟程時從 `cherished_player` 快照，
    /// 讓途中即使又讀到別的牌也不影響抵達判定。Some(玩家名) = 抵達時登門拜訪你；None = 走既有路徑。
    pilgrimage_player: Option<String>,
    /// 朝聖逾時倒數（秒，讀牌 v3）：啟程時設 [`vreadsign::PILGRIMAGE_TIMEOUT`]；未抵達時遞減，
    /// 歸零仍沒到（地形擋路等）即放棄，避免無限走。
    pilgrimage_timer: f32,
    /// 重返冷卻倒數（秒，讀牌 v3）：一次朝聖（抵達或放棄）後設為 [`vreadsign::PILGRIMAGE_COOLDOWN`]，
    /// 歸零前不再啟程——稀少才有感、不洗版。各居民初始錯開。
    pilgrimage_cooldown: f32,
    /// 望星冷卻倒數（秒，繁星夜空 v1，ROADMAP 783）：一次夜裡望星／邀你同賞後設為
    /// [`vstar::STARGAZE_COOLDOWN_SECS`]，歸零前不再觸發——星夜共賞是偶爾的浪漫一拍、不洗版。
    /// 各居民初始錯開。純記憶體、重啟歸零。
    stargaze_cooldown: f32,
    /// 這位居民擺在世界裡、你送的紀念物座標小佇列（睹物思人 v1，ROADMAP 784）：keepsake（732）
    /// 落地一件就記一筆（座標＋紀念物名＋送禮玩家名）；她日後閒晃恰好路過時偶爾駐足追憶。
    /// 上限 [`vkrecall::MAX_SPOTS`]、去重。純記憶體、重啟歸零（那塊方塊本身仍由 keepsake 持久化）。
    keepsake_spots: Vec<vkrecall::KeepsakeSpot>,
    /// 睹物思人冷卻倒數（秒，ROADMAP 784）：一次追憶後設 [`vkrecall::RECALL_COOLDOWN_SECS`]，
    /// 歸零前不再觸發——偶爾一拍才有感、不洗版。各居民初始錯開。純記憶體、重啟歸零。
    keepsake_recall_cooldown: f32,
    /// 哼歌冷卻倒數（秒，ROADMAP 788）：一次哼歌後設 [`vhum::HUM_COOLDOWN_SECS`]，歸零前不再哼——
    /// 心情正好時偶爾滿溢一段旋律、不洗版。各居民初始錯開。純記憶體、重啟歸零。
    humming_cooldown: f32,
    /// 營火取暖冷卻倒數（秒，乙太營火 v1）：一次圍暖後設 [`vcamp::WARM_COOLDOWN_SECS`]，歸零前不再取暖——
    /// 夜裡路過火邊偶爾駐足、不狂刷泡泡。各居民初始錯開。純記憶體、重啟歸零。
    campfire_warm_cooldown: f32,
    /// 圍著營火說故事冷卻倒數（秒，圍火講往事 v1）：一次開講後設 [`vtale::TALE_COOLDOWN_SECS`]，歸零前
    /// 不再開講——夜裡同在一座火邊偶爾講起一段往事、不連珠炮洗版。各居民初始錯開。純記憶體、重啟歸零。
    campfire_tale_cooldown: f32,
    /// 木長椅歇腳冷卻倒數（秒，木長椅 v1）：一次坐下歇腳後設 [`vbench::REST_COOLDOWN_SECS`]，歸零前
    /// 不再歇腳——白天路過椅邊偶爾坐下、不狂刷歇腳泡泡。各居民初始錯開。純記憶體、重啟歸零。
    bench_rest_cooldown: f32,
    /// 長椅並坐閒聊冷卻倒數（秒，長椅並坐閒聊 v1）：一次招呼並坐聊完後設
    /// [`vbenchchat::CHAT_COOLDOWN_SECS`]，歸零前不再起哄——白天同在一張長椅邊偶爾招呼熟人並肩閒聊、
    /// 不連珠炮洗版。各居民初始錯開。純記憶體、重啟歸零。
    bench_chat_cooldown: f32,
    /// 被熟人招呼在長椅上並肩坐下後、等這秒數到期再應和（發起者名字, 剩餘秒, 這次相遇的結果）
    /// （長椅並坐閒聊 v1 + 長椅拌嘴/和好 v1）。第三欄 `BenchOutcome` 決定應和該冒閒聊/拌嘴/和好
    /// 哪一種專屬台詞。與 `pending_tale_reply`／`pending_response` 分開，讓被招呼者冒的是「並坐」
    /// 味道的專屬應和。純記憶體。
    pending_bench_reply: Option<(String, f32, vbtiff::BenchOutcome)>,
    /// 臨水垂釣冷卻倒數（秒，居民臨水垂釣 v1）：一次坐下垂釣後設 [`vangler::REST_COOLDOWN_SECS`]，
    /// 歸零前不再垂釣——白天路過水邊偶爾坐下釣一竿、不狂刷垂釣泡泡。各居民初始錯開。純記憶體、重啟歸零。
    angler_cooldown: f32,
    /// 雨天躲雨冷卻倒數（秒，雨天葉傘避雨 v1）：一次停步躲雨後設 [`vrain::SHELTER_COOLDOWN_SECS`]，
    /// 歸零前不再躲——一場雨裡偶爾停步避一會兒、不狂刷避雨泡泡。各居民初始錯開。純記憶體、重啟歸零。
    rain_shelter_cooldown: f32,
    /// 顧家駐足冷卻倒數（秒，居民顧家駐足 v1）：一次在自家門前駐足後設 [`vhome::GAZE_COOLDOWN_SECS`]，
    /// 歸零前不再駐足——白天路過自家偶爾停下望一望、不狂刷顧家泡泡。各居民初始錯開。純記憶體、重啟歸零。
    homegaze_cooldown: f32,
    /// 正在應召循鐘聲趕來（集會鐘 v1）：玩家敲響集會鐘時，範圍內閒著的居民設此欄位，
    /// 移動鏈據此朝鐘走去、抵達即聚攏反應後清空；逾時（[`vbell::SUMMON_TIMEOUT_SECS`]）自動放棄。
    /// 純記憶體、重啟歸零。
    summon: Option<vbell::Summon>,
    /// 集會鐘應召冷卻倒數（秒，集會鐘 v1）：應召一次後設 [`vbell::SUMMON_COOLDOWN_SECS`]，歸零前
    /// 不再被鐘聲拉動——**濫用防護主閘**：狂敲鐘也拖不動同一位居民太頻繁。純記憶體、重啟歸零。
    summon_cooldown: f32,
    /// 被夥伴在營火邊講了故事後、等這秒數到期再應和（講述者名字, 剩餘秒）（圍火講往事 v1）。
    /// 與 `pending_response` 分開，讓聆聽者冒的是「聽故事」味道的專屬應和，而非通用社交回應。純記憶體。
    pending_tale_reply: Option<(String, f32)>,
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
    /// 居民察覺你翻過她的日記 v1（自主提案切片）：這位居民對哪些玩家記著「翻過我的日記、
    /// 我還沒發現」的待發現旗標——key＝玩家顯示名，存在＝待發現、不存在＝沒被那位玩家翻過或
    /// 已經揭穿完畢。`/voxel/diary?player=&resident=` 命中時鎖外寫鎖新增（見 `voxel_diary_peek`）；
    /// 打招呼那一刻依機率讀取／清除。有界成長（[`vdiarypeek::MAX_PEEK_ENTRIES_PER_RESIDENT`]），
    /// 防有心人拿亂數玩家名瘋狂洗版。純記憶體、重啟歸零（丟了大不了少一次驚喜，零資料風險）。
    diary_peeked: std::collections::HashSet<String>,
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
    /// 戀人牽掛（記憶驅動·戀人牽掛 v1，ROADMAP 852）：Some(戀人顯示名, 逾時剩餘秒) = 分開得夠遠、
    /// 放下手邊的事正走去找戀人；抵達（暖暖相見＋雙方各記一筆重逢記憶）或戀人睡了／逾時即清空。
    /// 純記憶體、重啟歸零。
    lover_seek: Option<(String, f32)>,
    /// 上次牽掛落幕（抵達或放棄）之後的靜置冷卻秒數，到期才會再次起念去找戀人（ROADMAP 852）。
    lover_seek_cooldown: f32,
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
    /// 邊陲巧遇冷卻倒數（秒，玩家追到邊陲找到我 v1）：> 0 表示最近才在邊陲被玩家巧遇過，
    /// 暫不再驚喜反應（防你賴在原地不動時每 tick 狂刷驚喜台詞）。純記憶體、重啟歸零。
    frontier_find_cooldown: f32,
    /// 手中捧著、還沒享用的食物餽贈（你送的食物她會細細享用 v1，ROADMAP 765）：
    /// `Some((食物 item_id, 送禮玩家名, 剩餘延遲秒))` = 玩家剛送了一份食物，居民收下但還沒吃，
    /// 倒數歸零後在一個閒下來的安靜片刻**真的享用**（冒暖泡泡＋動態牆＋重新點亮心情）；
    /// `None` = 手中沒有待享用的食物。同時只捧一份（再收到新食物就換成最新那份）。純記憶體、
    /// 重啟歸零（享用是數十秒內的短暫過場，重啟大不了少享用一次、無資料風險，零 migration）。
    savoring: Option<(u8, String, f32)>,
    /// 餓意（居民也會肚子餓 v1，ROADMAP 799）：0.0=剛吃飽、`vhunger::HUNGER_MAX`=餓極。
    /// 隨伺服器 tick 累積（`vhunger::tick_hunger`），越過 `HUNGRY_THRESHOLD` 就想找吃的。
    /// 純記憶體、重啟歸零（餓是數分鐘的過場狀態，零資料風險、零 migration）。
    hunger: f32,
    /// 冒餓／吃飽後的靜默冷卻（秒）：> 0 時不再喊餓，避免反覆碎念、稀少才有感。
    hunger_say_cd: f32,
    /// 正走回家找吃的（居民也會肚子餓 v1）：true = 已放下閒晃、目標設向家域中心，
    /// 到家即吃飽重置。純記憶體，鏡像 678 尋伴的「逐 tick 重設目標→抵達即結」機制。
    seeking_food: bool,
    /// 正為了吃而去收成／覓食（飢餓接農田 v2）：true = 餓了、家裡卻沒存糧，於是放下閒晃、走去
    /// 把附近熟了的作物收進小背包（收成→存糧→之後餓了吃它）。純記憶體，鏡像 `seeking_food` 的
    /// 「逐 tick 重設目標→抵達即收成」機制；沒糧的餓意由此驅動出「為了吃而去種田收成」的行為。
    foraging_food: bool,
    /// 目前鎖定要去收成的成熟作物座標（飢餓接農田 v2）：`Some((wx,wy,wz))`＝已找到一畦熟作物、
    /// 正朝它走去；`None`＝尚未鎖定（下個 agency tick 再找）。純記憶體。
    forage_target: Option<(i32, i32, i32)>,
    /// 共用糧倉 v1：附近找不到熟作物時，鎖定要去借一份存糧的箱子座標——`Some((wx,wy,wz))`＝
    /// 已找到一個有食物庫存的箱子、正朝它走去；`None`＝尚未鎖定或本輪未觸發。純記憶體，
    /// 鏡像 `forage_target` 的「逐 tick 走→抵達即收」機制，作物優先、箱子是找不到熟作物時的備援。
    larder_target: Option<(i32, i32, i32)>,
    /// 分食冷卻倒數（秒，飢餓時的守望相助 v1，ROADMAP 800）：一次分食後設
    /// [`vsharemeal::SHARE_COOLDOWN_SECS`]，歸零前不再對人分食，讓「分一口飯」稀少而有份量。純記憶體。
    share_meal_cooldown: f32,
    /// 被鄰居分食後、延遲道謝的待辦（飢餓時的守望相助 v1）：`Some((分食者名, 倒數, 是否回報))`，
    /// 倒數歸零時冒一句專屬道謝泡泡（比照 792 圍火講古的 `pending_tale_reply` 一來一往機制）。純記憶體。
    /// 第三欄 `is_repay`（知恩圖報 v1，ROADMAP 801）：true=這頓是「回報當年那口飯」，道謝改用專屬語氣。
    pending_meal_thanks: Option<(String, f32, bool)>,
    /// 出生 unix 秒（居民誕辰紀念 v1）：0＝沒有記錄在案的誕生時刻（初始四位居民），
    /// >0＝經世代傳承誕生、[`vbday::age_years`] 據此算出滿幾週歲。純記憶體（來源＝
    /// 名冊 `RosterEntry::birth_unix` 或誕生當下的 `now`，重啟由名冊/建構還原，非新持久化）。
    birth_unix: u64,
    /// 生下這位居民的父母顯示名（居民誕辰紀念 v1）：空字串＝沒有已知父母（初始四位居民）。
    /// 誕辰紀念泡泡點名感謝父母時使用。純記憶體，來源同 `birth_unix`。
    birth_parent_name: String,
    /// 上次已慶祝過的誕辰週歲數（居民誕辰紀念 v1）：0＝還沒慶祝過。純記憶體、重啟歸零——
    /// 重啟後若當下週歲已慶祝過，至多重觸發一次誕辰紀念，非資料風險（比照其他純記憶體冷卻慣例）。
    birthday_last_year: u64,
    /// 邊陲探友（居民千里跋涉去邊陲探望遠行的夥伴 v1，ROADMAP 821）：
    /// Some(朋友邊陲落點 x, z, 方位名, 朋友顯示名) = 正跋涉前往／已抵達朋友的邊陲營地小聚；
    /// None = 沒在探友（正常閒晃/採集/建造）。純記憶體、重啟歸零。
    frontier_visit: Option<(f32, f32, String, String)>,
    /// 抵達朋友邊陲落點後的小聚倒數（秒）：> 0 = 已找到朋友、正小聚，到 0 即啟程返家。
    frontier_visit_stay: f32,
    /// 邊陲探友去程逾時倒數（秒）：未抵達（朋友提前離開邊陲／地形擋路）就放棄，不無限跋涉。
    frontier_visit_timer: f32,
    /// 邊陲探友冷卻倒數（秒）：一次探友（尋得／放棄）後設定，稀少才有感、不洗版。各居民初始錯開。
    frontier_visit_cooldown: f32,
    /// 病況（居民也會生病 v1，ROADMAP 自主提案）：0.0=健康、[`villness::ILLNESS_MAX`]=剛病倒。
    /// 隨伺服器 tick 自然消退（[`villness::tick_recover`]）；鄰居陪伴／玩家送湯會加速消退。
    /// 純記憶體、重啟歸零（生病是數分鐘的過場狀態，零資料風險、零 migration）。
    illness_severity: f32,
    /// 發病冷卻倒數（秒）：> 0 時不會再次病倒，讓「生病」稀少而有份量。純記憶體，各居民初始錯開。
    illness_cooldown: f32,
    /// 陪伴照顧冷卻倒數（秒，居民也會生病 v1）：這位居民**當陪伴者**陪過人後設
    /// [`villness::CARE_COOLDOWN_SECS`]，歸零前不再對人陪伴，讓「停下來陪一會兒」稀少而有份量。
    care_cooldown: f32,
    /// 被鄰居陪伴後、延遲道謝的待辦：`Some((陪伴者名, 倒數))`，倒數歸零時冒一句專屬道謝泡泡
    /// （比照 800 飢餓時的守望相助 `pending_meal_thanks` 一來一往機制）。純記憶體。
    pending_care_thanks: Option<(String, f32)>,
    /// 關心玩家挨餓冷卻倒數（秒，居民關心你挨餓 v1）：一次上前遞麵包關心後設
    /// [`vcare::CARE_COOLDOWN_SECS`]，歸零前不再重複關心同一次挨餓——你若持續挨餓，過了這麼久
    /// 才會再被同一位居民注意到一次。各居民初始錯開。純記憶體、重啟歸零。
    hunger_care_cooldown: f32,
}

/// 環境生物的種類（水中游魚 v1，ROADMAP 848 起 wildlife 系統擴充為可延伸的多種類；
/// 放養雞 v1，ROADMAP 870 再添第二種可馴服的陸地生物）。
/// 序列化成字串給前端據以挑選對應模型（"rabbit"/"fish"/"chicken"）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum WildlifeKind {
    Rabbit,
    Fish,
    Chicken,
}

impl WildlifeKind {
    fn wire(self) -> &'static str {
        match self {
            WildlifeKind::Rabbit => "rabbit",
            WildlifeKind::Fish => "fish",
            WildlifeKind::Chicken => "chicken",
        }
    }
}

/// 野兔 v1（自主提案切片，ROADMAP 847）＋水中游魚 v1（ROADMAP 848）：世界環境生物。
/// 純點綴、無 AI 大腦、無戰鬥、無記憶——陸地上的野兔會閒晃＋見到玩家靠近就受驚逃開；
/// 水裡的魚只悠游（見 `voxel_fish` 模組說明：魚不怕人，行為樹刻意與野兔不同）。
/// 純記憶體，重啟於固定家域點重新生成（比照既有 `drops`/`stalls` 世界暫態慣例，零 migration）。
struct WildlifeAnimal {
    /// 系統 id（"vox_wld_0"…），與居民 id 體系無交集。
    id: String,
    kind: WildlifeKind,
    body: Body,
    yaw: f32,
    /// 家域中心（世界座標）：平靜時的閒晃圍繞這一點打轉，範圍遠比居民家域小。
    home_x: f32,
    home_z: f32,
    /// 當前水平移動目標（閒晃目的地，或受驚時的逃跑落點）。
    target_x: f32,
    target_z: f32,
    /// 抵達閒晃目標後的小歇秒數（> 0 = 在歇、原地落重力/靜止）。野兔受驚時忽略（不歇息）。
    wait_timer: f32,
    /// 此刻是否受驚逃跑中（遲滯判定見 `voxel_wildlife::should_flee`）。只有野兔會用到；
    /// 魚恆為 `false`（魚不怕人，見 `voxel_fish` 模組說明）。
    fleeing: bool,
    /// 是否已被玩家餵食馴服（餵野兔馴服 v1，自主提案切片）。一次性、永久生效——
    /// 馴服後永遠不再受驚逃跑。只有野兔會用到；魚恆為 `false`（無馴服機制）。
    tamed: bool,
    /// 已馴服的兔子此刻是否正在跟隨附近的玩家（馴服兔子跟隨你 v1，自主提案切片）；
    /// 已馴服的雞也共用這個欄位跟隨（放養雞 v1）。魚恆為 `false`（無跟隨機制）。
    following: bool,
    /// 已馴服的雞距離下一次下蛋還要多久（秒，放養雞 v1，自主提案切片 ROADMAP 870）。
    /// 只有 `tamed` 的雞會遞減這個欄位；兔子／魚／未馴服的雞恆為 `0.0`（不使用）。
    lay_cd: f32,
}

/// 野兔家域點（世界座標偏移，散布在村莊周圍，玩家出生後很快就有機會撞見）。
/// 沿用居民 `dry_ground_spawn` 找最近陸地，故偏移不需精確落在草地，找到最近乾地即可。
const WILDLIFE_HOMES: [(i32, i32); 6] = [
    (12, 6), (-10, 14), (18, -8), (-14, -12), (6, -20), (-20, 4),
];

/// 魚家域點（世界座標偏移）：沿用 `voxel_fish::wet_spot_spawn` 向外螺旋找最近的深水域，
/// 偏移不需精確落在水裡，找到附近夠深的水塘即可（地形起伏必然生出窪地/湖泊，見模組說明）。
const FISH_HOMES: [(i32, i32); 4] = [(30, 30), (-30, 30), (30, -30), (-30, -30)];

/// 雞的家域點（世界座標偏移，放養雞 v1，自主提案切片 ROADMAP 870）：與野兔家域錯開，
/// 散布在村莊周圍，沿用同一套 `dry_ground_spawn` 找最近陸地。
const CHICKEN_HOMES: [(i32, i32); 4] = [(8, -6), (-8, 8), (16, 12), (-16, -8)];

/// 建出初始環境生物群（hub 初始化時呼叫一次）：野兔（陸地）+ 魚（水域）+ 雞（陸地）。
fn init_wildlife() -> Vec<WildlifeAnimal> {
    let rabbits = WILDLIFE_HOMES.iter().enumerate().map(|(i, (ox, oz))| {
        let body = vr::dry_ground_spawn(*ox, *oz);
        WildlifeAnimal {
            id: format!("vox_wld_{i}"),
            kind: WildlifeKind::Rabbit,
            home_x: body.x,
            home_z: body.z,
            target_x: body.x,
            target_z: body.z,
            body,
            yaw: 0.0,
            wait_timer: 0.0,
            fleeing: false,
            tamed: false,
            following: false,
            lay_cd: 0.0,
        }
    });
    let fish = FISH_HOMES.iter().enumerate().map(|(i, (ox, oz))| {
        let (x, y, z) = vfishlife::wet_spot_spawn(*ox, *oz);
        WildlifeAnimal {
            id: format!("vox_fsh_{i}"),
            kind: WildlifeKind::Fish,
            home_x: x,
            home_z: z,
            target_x: x,
            target_z: z,
            body: Body::at(x, y, z),
            yaw: 0.0,
            wait_timer: 0.0,
            fleeing: false,
            tamed: false,
            following: false,
            lay_cd: 0.0,
        }
    });
    let chickens = CHICKEN_HOMES.iter().enumerate().map(|(i, (ox, oz))| {
        let body = vr::dry_ground_spawn(*ox, *oz);
        WildlifeAnimal {
            id: format!("vox_chk_{i}"),
            kind: WildlifeKind::Chicken,
            home_x: body.x,
            home_z: body.z,
            target_x: body.x,
            target_z: body.z,
            body,
            yaw: 0.0,
            wait_timer: 0.0,
            fleeing: false,
            tamed: false,
            following: false,
            lay_cd: 0.0,
        }
    });
    rabbits.chain(fish).chain(chickens).collect()
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

/// 環境生物序列化視圖（廣播給客戶端渲染：位置/朝向/種類。野兔 v1 ROADMAP 847
/// ＋水中游魚 v1 ROADMAP 848）。`kind` 讓前端據以挑選對應模型（"rabbit"/"fish"）。
#[derive(Serialize)]
struct WildlifeView {
    id: String,
    kind: &'static str,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
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

/// 依兩個居民 id 查詢彼此是不是「彆扭中」（長椅拌嘴/和好 v1，同一套 id→名字轉換慣例）。
fn resident_is_sulking(bonds: &ResidentBonds, id_a: &str, id_b: &str) -> bool {
    bonds.is_sulking(resident_name_of(id_a), resident_name_of(id_b))
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

/// 對話需登入判定（治安三件套③）：與居民交談會觸發免費 LLM（居民的腦），匿名腳本
/// 可白嫖／燒爆額度。故只有登入帳號（`is_account==true`，後端 cookie→users 權威解出）
/// 才能發起對話；訪客回 `false`＝擋下（可自由逛逛與觀看，只是不能聊）。純函式、單一
/// 政策點，日後若要改成「訪客給極嚴格額度」只需改這裡與呼叫端。
fn talk_allowed_for_identity(is_account: bool) -> bool {
    is_account
}

/// 全域 per-IP 對話速率限制器（治安三件套①）：跨所有連線共用一份，以真實 IP 為鍵，
/// 給對話（觸發 LLM）的路徑設一道 per-connection 冷卻擋不住的跨連線天花板。
fn ip_talk_limiter() -> &'static std::sync::Mutex<vrl::IpTalkLimiter> {
    static L: std::sync::OnceLock<std::sync::Mutex<vrl::IpTalkLimiter>> = std::sync::OnceLock::new();
    L.get_or_init(|| std::sync::Mutex::new(vrl::IpTalkLimiter::new()))
}

/// 全域「作品命名」儲存（居民為你的建造作品取名字 v1；860 擴充納入居民自蓋的家）：
/// 以分格座標為鍵記已取過的名字＋擁有者（`None`＝玩家蓋的，見 773/854；
/// `Some(resident_id)`＝居民自己蓋的家，見 860，讓 858 見賢思齊能排除「羨慕自己的家」）。
/// 純記憶體、重啟歸零（作品本身不受影響，只是重啟後可再被重新命名一次，v1 刻意收斂）。
fn structure_names(
) -> &'static std::sync::Mutex<std::collections::HashMap<(i32, i32), (String, Option<String>)>> {
    static N: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(i32, i32), (String, Option<String>)>>,
    > = std::sync::OnceLock::new();
    N.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 全域 per-IP 連線數計數器（治安三件套②·連線數上限）：跨所有連線共用一份，以真實 IP 為鍵，
/// 擋「一個腳本開幾十條連線並行灌 talk（繞過 per-connection 冷卻）」。連線建立時 try_acquire、
/// 結束時 release。
fn ip_conn_limiter() -> &'static std::sync::Mutex<vrl::IpConnLimiter> {
    static L: std::sync::OnceLock<std::sync::Mutex<vrl::IpConnLimiter>> = std::sync::OnceLock::new();
    L.get_or_init(|| std::sync::Mutex::new(vrl::IpConnLimiter::new()))
}

/// 全域 per-IP 內容違規計數器（治安三件套①·累犯加長冷卻）：以真實 IP 為鍵記「累積命中審查幾次」。
/// 玩家每被內容審查（注入／NSFW／辱罵）攔一次就 +1；違規越多、下一次對話的額外冷卻越長，
/// 讓反覆試探的人自然被拖慢，正常人零感知（永遠不會命中）。純記憶體、單調累加、無界性由
/// 「命中審查者本就稀少」天然約束（不像每 IP 都有的 rate 桶會長大）。
fn ip_violations() -> &'static std::sync::Mutex<std::collections::HashMap<String, u32>> {
    static V: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u32>>> =
        std::sync::OnceLock::new();
    V.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 記一次某 IP 的內容違規並回傳「累積違規次數」（含這次）。短鎖即釋、不 await（守鎖紀律）。
fn record_violation(ip: &str) -> u32 {
    let mut v = ip_violations().lock().unwrap();
    let c = v.entry(ip.to_string()).or_insert(0);
    *c = c.saturating_add(1);
    *c
}

/// 累犯額外冷卻（毫秒）：純函式。違規 n 次 → 額外 min(n, CAP) × STEP 毫秒，封頂避免無限長。
/// 第一次違規（n=1）就給一點冷卻，反覆試探者遞增到上限；正常玩家永不命中審查、恆 0。
fn violation_cooldown_ms(violations: u32) -> u64 {
    const STEP_MS: u64 = 2000; // 每次違規多 2 秒
    const CAP: u64 = 30; // 封頂 30 次 → 最多額外 60 秒
    (violations as u64).min(CAP) * STEP_MS
}

/// 同一 IP 的同時連線數上限（治安三件套②）：讀 `BUTFUN_MAX_CONN_PER_IP`，未設／壞值／0 →
/// 退預設 `vrl::MAX_CONN_PER_IP`（5）。維護者可視需要調整；下限 clamp 到 1（永不歸零把人全鎖死）。
fn max_conn_per_ip() -> usize {
    std::env::var("BUTFUN_MAX_CONN_PER_IP")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(vrl::MAX_CONN_PER_IP)
}

/// 純函式：某 IP 是否在「連線數／速率」限流白名單內（QA／localhost 豁免）。
/// - localhost（`127.0.0.1`、`::1`、解不出 IP 的保底桶 `unknown`）恆豁免——本機冒煙／隔離
///   測試不受上限干擾（prod 的真流量一律經 CF tunnel，帶真實 `cf-connecting-ip`，永遠不是這些）。
/// - 另可用 `whitelist`（由呼叫端傳入，來自 env `BUTFUN_RL_WHITELIST` 逗號分隔）補充。
fn ip_limit_exempt(ip: &str, whitelist: &[String]) -> bool {
    matches!(ip, "127.0.0.1" | "::1" | "unknown") || whitelist.iter().any(|w| w == ip)
}

/// 便捷層：讀 env 白名單清單（逗號分隔、去空白、濾空）。未設 → 空 Vec（只剩內建 localhost 豁免）。
fn rl_whitelist() -> Vec<String> {
    std::env::var("BUTFUN_RL_WHITELIST")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 便捷層：這條連線的 IP 是否豁免限流（讀 env 白名單 + 內建 localhost）。
fn ip_is_exempt(ip: &str) -> bool {
    ip_limit_exempt(ip, &rl_whitelist())
