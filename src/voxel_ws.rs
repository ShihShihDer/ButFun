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
    /// 相遇被老朋友就地教了一手後、等這秒數到期再冒「原來如此！」的應和（應和台詞, 剩餘秒）
    /// （技能互教·北極星第四刀）。與 `pending_tale_reply`／`pending_bench_reply` 分開，
    /// 讓受教有專屬的一來一往語氣。台詞在教學那刻就從句式池組好（零 LLM）。純記憶體。
    pending_teach_reply: Option<(String, f32)>,
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
}

/// 當前 UNIX 毫秒時間戳（餵給 token bucket；時鐘不可用時退 0，限流器內部對回退安全）。
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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

/// 飢餓速率倍率（飢餓接農田 v2·QA 用）：讀 `BUTFUN_HUNGER_RATE_MULT`，未設 / 解析失敗 / ≤0
/// → 回 1.0（維持原速）。**僅供隔離測試**把「數分鐘才餓」壓成「數秒就餓」以便觀察餓→吃／收成，
/// 正式線上不設此環境變數，對玩家零影響（不改任何預設常數、無資料風險）。
fn hunger_rate_mult() -> f32 {
    std::env::var("BUTFUN_HUNGER_RATE_MULT")
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|&m| m > 0.0 && m.is_finite())
        .unwrap_or(1.0)
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
    // 治安三件套①：拒絕成人／露骨內容、溫和轉移話題的一小段守則（內建過濾是主閘，這段是補強）。
    let refusal = vmod::refusal_guide();
    format!(
        "{base}\n\n你現在身處『乙太方界』——一片由方塊構成、寧靜清新的新生天地，你是這裡的居民。{desire_note}\
        此刻有一位來訪的旅人向你搭話。請以你的身份、用繁體中文自然回應，1 到 2 句、口吻溫暖親切，\
        可以聊聊你在這片方塊天地裡的生活或當下的心情；絕不跳出角色，也不要提到你是 AI 或語言模型。\
        \n\n{honesty}\n\n{refusal}"
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
    // 先組「要建構哪些居民」的規格清單：(id 索引, 家域中心 x, z, 出生 body, 出生 unix 秒, 父母名)。
    // 初始居民沒有記錄在案的誕生時刻／父母（居民誕辰紀念 v1恆 0/空字串，本刀不觸發她們）。
    let mut specs: Vec<(usize, f32, f32, Body, u64, String)> = Vec::new();
    for i in 0..RESIDENT_COUNT {
        // 初始居民各有自己的家域基準點，分散世界四方（見 vr::resident_home_base）。
        let (hox, hoz) = vr::resident_home_base(i);
        let body = vr::dry_ground_spawn(hox, hoz);
        specs.push((i, body.x, body.z, body, 0, String::new()));
    }
    // 人口成長 v1：載回出生居民（append-only 名冊）。索引須連續接在既有 id 之後、落在名字池內，
    // 否則跳過（斷號/壞行容忍，保住 id 連續性——resident_count 的 0..N 枚舉才安全）。
    for entry in vroster::load_roster() {
        let i = vroster::resident_index(&entry.resident);
        if i >= vroster::RESIDENT_NAME_POOL_LEN || i != specs.len() {
            continue;
        }
        let body = vr::dry_ground_spawn(entry.home_base_x, entry.home_base_z);
        specs.push((i, body.x, body.z, body, entry.birth_unix, entry.parent_name.clone()));
    }
    let mut out = Vec::with_capacity(specs.len());
    for (i, home_x, home_z, body, birth_unix, parent_name) in specs {
        out.push(build_resident(i, home_x, home_z, body, birth_unix, parent_name));
    }
    // 居民搬新家（引導式都更）：已搬完（或新家已完工、正拆舊家）的居民，家域中心跟著
    // 新家走（重啟持久）——初始居民的 resident_home_base / 名冊 home_base 只是出生預設，
    // 搬過家的以搬家記錄為準（她的日常活動圈從此繞著村裡的新家）。
    let reloc = vvillage::RelocationStore::from_entries(vvillage::load_relocations());
    for r in &mut out {
        if let Some((nx, _ny, nz)) = reloc.home_override(&r.id) {
            r.home_x = nx as f32 + 0.5;
            r.home_z = nz as f32 + 0.5;
        }
    }
    // 在世人口（含出生居民）→ resident_count() 無鎖回報的單一事實來源。
    RESIDENT_POP.store(out.len(), Ordering::Relaxed);
    out
}

/// 建構一位居民（初始 4 位與出生的新居民共用同一套欄位初始化）：
/// `i`＝id 索引（決定名字 / persona / 各冷卻相位錯開），`home_x/home_z`＝家域中心、
/// `body`＝出生位置。純建構、不碰鎖 / IO。人口成長 v1 讓出生走這條與初始完全相同的路，
/// 新居民因此天生就有採集 / 蓋家 / 好奇心等既有零 / 低 LLM 行為，多幾位不爆成本。
fn build_resident(
    i: usize,
    home_x: f32,
    home_z: f32,
    body: Body,
    birth_unix: u64,
    birth_parent_name: String,
) -> VoxelResident {
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
            // 主動掏心冷卻（居民主動聊心事 v1）：錯開初始冷卻，避免啟動後同時觸發；
            // 也讓一上線不會立刻碎念心事，先熟起來再說。
            confide_timer: vconfide::CONFIDE_COOLDOWN_SECS + i as f32 * 90.0,
            // 主動教學冷卻（居民教你一道獨門配方 v1）：錯開初始冷卻，避免啟動後同時觸發；
            // 也讓一上線不會立刻教配方，得先累積起足夠交情才有這一刻。
            teach_timer: vprecipe::TEACH_COOLDOWN_SECS + i as f32 * 120.0,
            // 回饋糧倉冷卻（居民回饋糧倉 v1）：錯開初始冷卻，避免啟動後同時往箱子塞東西；
            // 也讓一上線不會立刻存料，先累積一點採集所得再說。
            contribute_timer: vchestgive::CONTRIBUTE_COOLDOWN_SECS + i as f32 * 100.0,
            // 主動討東西冷卻（居民拜託你幫個小忙 v1）：錯開初始冷卻，避免啟動後同時開口；
            // 也讓一上線不會立刻伸手要東西，先熟起來、玩家有材料在手時再說。
            request_timer: vrequest::REQUEST_COOLDOWN_SECS + i as f32 * 75.0,
            // 一上線手邊沒有未了的請求。
            open_request: None,
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
            // 發明採集·階梯礦井：入場無進行中的井、井數歸零（隨每次 run 收尾重置）。
            invent_quarry: None,
            invent_quarry_wells: 0,
            // 出生時醒著；入睡由夜間作息迴圈決定（ROADMAP 739）。
            asleep: false,
            // 做夢 v1（ROADMAP 805）：入場沒在冷卻、還沒做過夢。
            dream_cooldown: 0.0,
            dream_seq: 0,
            // 把昨晚的夢說給你聽 v1（ROADMAP 807）：入場還沒做過夢可說；分享冷卻錯開初始，
            // 避免一上線就同時觸發（也讓居民先夜裡真的做過夢，白天才有夢可分享）。
            last_dream: None,
            dreamshare_timer: vdreamshare::DREAMSHARE_COOLDOWN_SECS + i as f32 * 60.0,
            // 重返心中的牌子（讀牌 v3，ROADMAP 743）：入場心裡還沒記著任何牌、沒在朝聖；
            // 首次重返冷卻長且錯開（前數分鐘不朝聖，讓居民先讀到牌、心裡有地標再說）。
            cherished_sign: None,
            // 登門串門子 v1（ROADMAP 751）：入場心裡還沒認得任何鄰居家、沒在登門途中。
            cherished_neighbor: None,
            // 居民認得你的家 v1（自主提案切片，ROADMAP 830）：入場心裡還沒認得任何玩家的家。
            cherished_player: None,
            pilgrimage: None,
            pilgrimage_neighbor: None,
            pilgrimage_player: None,
            pilgrimage_timer: 0.0,
            pilgrimage_cooldown: 180.0 + i as f32 * 60.0,
            // 繁星夜空 v1（ROADMAP 783）：望星冷卻各自錯開，避免同一個星夜大家一起邀。
            stargaze_cooldown: 120.0 + i as f32 * 45.0,
            // 睹物思人 v1（ROADMAP 784）：入場還沒擺出任何你送的紀念物；追憶冷卻各自錯開。
            keepsake_spots: Vec::new(),
            keepsake_recall_cooldown: 90.0 + i as f32 * 40.0,
            // 哼歌 v1（ROADMAP 788）：哼歌冷卻各自錯開，避免大家同時哼起來。
            humming_cooldown: 60.0 + i as f32 * 35.0,
            // 乙太營火 v1：首次取暖冷卻各自錯開，避免入夜同一 tick 一群人齊聲說暖語。
            campfire_warm_cooldown: 140.0 + i as f32 * 30.0,
            // 圍火講往事 v1：首次講述冷卻各自錯開，避免入夜同一 tick 一群人同時開講；入場無待應和的故事。
            campfire_tale_cooldown: 170.0 + i as f32 * 45.0,
            // 木長椅 v1：首次歇腳冷卻各自錯開，避免一群人同一 tick 齊坐齊念歇腳話。
            bench_rest_cooldown: 80.0 + i as f32 * 20.0,
            // 長椅並坐閒聊 v1：首次並坐冷卻各自錯開，避免白天同一 tick 一群人同時招呼並坐；入場無待應和。
            bench_chat_cooldown: 100.0 + i as f32 * 25.0,
            pending_bench_reply: None,
            // 居民臨水垂釣 v1：首次垂釣冷卻各自錯開，避免白天同一 tick 一群人一起開釣。
            angler_cooldown: vangler::fish_cd_offset(i),
            // 雨天葉傘避雨 v1：首次躲雨冷卻各自錯開，避免一下雨同一 tick 一群人齊聲說避雨話。
            rain_shelter_cooldown: vrain::shelter_cd_offset(i),
            homegaze_cooldown: vhome::gaze_cd_offset(i),
            pending_tale_reply: None,
            // 技能互教（北極星第四刀）：入場沒有待應和的教學。
            pending_teach_reply: None,
            // 集會鐘 v1：入場沒有正在應召的鐘；應召冷卻歸零（一出生就聽得到第一次鐘聲）。
            summon: None,
            summon_cooldown: 0.0,
            // 登門撲空留心意 v1（ROADMAP 763）：入場門口沒有待感應的心意；首次感應冷卻各自錯開。
            pending_callers: Vec::new(),
            callingcard_cooldown: 20.0 + i as f32 * 15.0,
            // 自我印象 v1（ROADMAP 770）：入場先積累記憶再回望自己——首次冷卻各自大幅錯開
            //（前 8~14 分鐘不碎念自我印象），也避免啟動後同時多人念。
            self_image_cooldown: 480.0 + i as f32 * 120.0,
            // 自我印象 v3（ROADMAP 772）：入場還沒說出過自我印象——首次昇華不算「轉變」。
            self_image_domain: None,
            coined_epithets: std::collections::HashMap::new(),
            // 居民察覺你翻過她的日記 v1：入場沒有任何待發現的偷看旗標。
            diary_peeked: std::collections::HashSet::new(),
            heard_epithets: std::collections::HashMap::new(),
            approaching_esteem: None,
            // 各居民初始冷卻錯開，避免入場後同時起身向玩家致意（比照 678 尋伴）。
            esteem_approach_cooldown: vesteem::approach_cooldown_offset(i),
            // 晨間思念玩家（ROADMAP 746）：入場沒有進行中的思念（僅由清晨醒來時的睡前反思觸發）。
            daybreak_seek: None,
            reunion_seek: None,
            // 戀人牽掛（ROADMAP 852）：入場沒有進行中的牽掛，冷卻歸零（真正觸發還需先締結戀人）。
            lover_seek: None,
            lover_seek_cooldown: 0.0,
            // 遠行探野（ROADMAP 756）：入場無遠行任務；首次冷卻各自大幅錯開（前 15~30 分鐘不遠行，
            // 讓居民先在家域安頓、也避免啟動後短時間內誰都往荒野跑）。新生兒也走這條，一併有遠行欄位。
            expedition: None,
            expedition_stay: 0.0,
            expedition_timer: 0.0,
            expedition_cooldown: vexp::EXPEDITION_COOLDOWN + i as f32 * 300.0,
            asleep_at_outpost: false,
            // 邊陲巧遇 v1：入場沒有正在冷卻的巧遇，反正沒在遠行也用不到，隨遠行一起就緒即可。
            frontier_find_cooldown: 0.0,
            // 你送的食物她會細細享用 v1（ROADMAP 765）：入場手中沒有待享用的食物。
            savoring: None,
            // 居民也會肚子餓 v1（ROADMAP 799）：入場剛吃飽，餓意從 0 起累積；
            // 初始靜默冷卻錯開，避免啟動後短時間全員一起喊餓。
            hunger: 0.0,
            hunger_say_cd: vhunger::hunger_cd_offset(i),
            seeking_food: false,
            // 飢餓接農田 v2：入場不在覓食收成途中。
            foraging_food: false,
            forage_target: None,
            // 共用糧倉 v1：入場不在借糧途中。
            larder_target: None,
            // 飢餓時的守望相助 v1（ROADMAP 800）：入場分食冷卻錯開，避免啟動後全員一起搶著分食。
            share_meal_cooldown: vsharemeal::share_cd_offset(i),
            pending_meal_thanks: None,
            // 居民誕辰紀念 v1：出生時刻/父母由呼叫端傳入（初始四位居民恆 0/空字串，不觸發）；
            // 入場還沒慶祝過任何週歲。
            birth_unix,
            birth_parent_name,
            birthday_last_year: 0,
            // 邊陲探友 v1（ROADMAP 821）：入場無探友任務；首次冷卻各自大幅錯開（前 15~25 分鐘不
            // 跋涉，讓居民先在家域穩定下來、朋友也先累積出老朋友交情再說）。
            frontier_visit: None,
            frontier_visit_stay: 0.0,
            frontier_visit_timer: 0.0,
            frontier_visit_cooldown: vfvisit::COOLDOWN_SECS * 0.5 + i as f32 * 250.0,
            // 居民也會生病 v1（自主提案）：入場健康；首次發病冷卻各自大幅錯開，
            // 避免啟動後短時間全員扎堆病倒。
            illness_severity: 0.0,
            illness_cooldown: villness::onset_cd_offset(i),
            care_cooldown: 0.0,
            pending_care_thanks: None,
            // 居民關心你挨餓 v1：首次關心冷卻各自錯開，避免伺服器剛啟動、你剛好挨餓時一群居民同時衝過來。
            hunger_care_cooldown: vcare::care_cd_offset(i),
            // 居民見賢思齊 v1：首次冷卻各自錯開，避免啟動後一群居民同時路過地標齊聲心生嚮往。
            envy_timer: venvy::ENVY_COOLDOWN_SECS * 0.5 + i as f32 * 90.0,
    }
}

/// voxel 世界的多人 hub：玩家表 + 方塊改動 overlay + 廣播頻道 + AI 居民 + 決策匯流排。
/// 模組內全域單例（不污染 AppState）。
struct VoxelHub {
    players: RwLock<HashMap<Uuid, VoxelPlayer>>,
    /// 方塊改動 delta 層（疊在程序生成地形之上）。切片②先記憶體存，session 內正確套用+廣播。
    /// 之後切片可把它接 DB 持久化；AI 蓋家也會共用這層。
    deltas: RwLock<WorldDelta>,
    /// 野兔 v1（自主提案切片，ROADMAP 847）：世界第一種環境生物。純記憶體、啟動時於
    /// 固定家域點生成（見 `init_wildlife`），重啟即重新生成（比照 `drops`/`stalls` 世界暫態慣例，
    /// 零 migration、零持久化）。tick 節奏與 `residents` 相同（10Hz）但各自獨立鎖，不巢狀。
    wildlife: RwLock<Vec<WildlifeAnimal>>,
    /// 乙太營火 v1（自主提案切片）：世界中所有營火方塊座標。放置時 push、破壞時 retain，
    /// 啟動時由 `vcamp::scan_campfires` 從 delta 重建（篝火持久化，重啟後居民仍記得去哪圍暖）。
    /// tick 時先短鎖 clone 一份快照再進 residents 寫鎖迴圈用，不巢狀鎖（守 prod 死鎖鐵律）。
    campfires: RwLock<Vec<(i32, i32, i32)>>,
    /// 木長椅 v1（自主提案切片）：世界中所有長椅方塊座標。放置時 push、破壞時 retain，
    /// 啟動時由 `vbench::scan_benches` 從 delta 重建（長椅持久化，重啟後居民仍記得去哪歇腳）。
    /// tick 時先短鎖 clone 一份快照再進 residents 寫鎖迴圈用，不巢狀鎖（守 prod 死鎖鐵律）。
    benches: RwLock<Vec<(i32, i32, i32)>>,
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
    /// 村莊地塊認領註冊表（村莊系統 v1）：誰認領了哪塊沿路地塊。持久化到 data/voxel_village_plots.jsonl。
    /// 居民新建築（含新生兒的家）改成「認領最近的空地塊」當錨點 → 蓋在地塊上自動沿路對齊、不再散落。
    village: RwLock<vvillage::PlotRegistry>,
    /// 居民搬新家（引導式都更）：搬家進度狀態機（**全村一次一位**）。持久化到
    /// data/voxel_relocations.jsonl——重啟後接著搬（新家續蓋、舊家拆除冪等重算恢復），
    /// 搬完的居民進 done 名單永不重搬。
    relocations: RwLock<vvillage::RelocationStore>,
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
    /// 莓果叢 store（莓果叢 v1·ROADMAP 806·純記憶體；重啟後未結果的苗重置計時，已結果的叢是 delta 方塊會持久）。
    /// 記錄哪些格子種下了莓果叢、何時起算，每 15 秒 tick（`tick_berry`）一次結果檢查；採收後回退重新登記計時。
    berry: RwLock<BerryStore>,
    /// 雞舍 store（雞舍生蛋 v1·純記憶體；重啟後未生蛋的雞舍重置計時，已生蛋的雞舍是 delta 方塊會持久）。
    /// 記錄哪些格子放了雞舍、何時起算，每 15 秒 tick（`tick_coop`）一次生蛋檢查；收蛋後回退重新登記計時。
    coop: RwLock<CoopStore>,
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
    /// 居民戀愛帳本（ROADMAP 846）：老朋友並坐閒聊時偶爾擦出心動火花，締結成一對戀人（一生
    /// 只有一位），持久化跨重啟。與 `bonds` 並行、鎖各自獨立短取即釋。
    romance: RwLock<ResidentRomance>,
    /// 欠飯帳本（知恩圖報 v1，ROADMAP 801）：記錄「誰欠誰一口飯」——被分過飯的居民（欠飯者 id）→
    /// 牠欠著一口飯的一群恩人（分食者 id）集合。純記憶體、重啟歸零（過場恩情、零 migration）；
    /// 800 分食 → owe，日後回報 → repay。以居民 id 記帳、與情誼帳本並行、鎖各自獨立短取即釋。
    meal_debts: RwLock<vgrat::MealDebts>,
    /// 箱子儲存 store（ROADMAP 692）：世界座標 → 方塊 id → 數量。
    /// 持久化到 data/voxel_chests.jsonl；多人共用同一箱子（序列化 RwLock 解決競爭）。
    chest: RwLock<vchest::ChestStore>,
    /// 告示牌文字 store（ROADMAP 740）：世界座標 → 一行短字。
    /// 持久化到 data/voxel_signs.jsonl；文字浮在牌上、所有人看得見（序列化 RwLock 解決競爭）。
    sign: RwLock<vsign::SignStore>,
    /// 漂流瓶 store（漂流瓶 v1，自主提案切片 825）：世界座標 → 一封尚未被撿走的瓶中信。
    /// 持久化到 data/voxel_bottles.jsonl；內文絕不外流（連線同步只送座標），撿走即從世界移除
    /// （一次性拾起，非常駐可讀——序列化 RwLock 解決競爭）。
    bottle: RwLock<BottleStore>,
    /// 掉落物 store（掉落物 v1，自主提案切片 828）：世界上還沒被撿走的實體材料。
    /// 純記憶體、重啟歸零（比照 `bottle`/`giftgarden` 等世界暫態，掉落物本就是暫留地上等人
    /// 撿的短命狀態，非玩家永久資產）。玩家丟下手上一件材料 → 落地存進這裡；任何玩家
    /// （含自己）走近即自動撿起；沒人撿的話 `DESPAWN_SECS` 後安靜消失。
    drops: RwLock<DropStore>,
    /// 交易攤 store（玩家自由市集 v1，自主提案切片 832）：世界座標 → 一攤待人接手的以物易物
    /// 提案。純記憶體、重啟歸零（比照 `bottle`/`drops` 慣例）。擺攤者的給出材料在擺攤當下
    /// 就已 escrow 進攤位本身（非其背包）；任何身上有攤位所求材料的玩家路過都能接手成交，
    /// 哪怕擺攤者早已離線；擺攤者可隨時自行收攤退還，逾時未接手也會自動收攤。
    stalls: RwLock<StallStore>,
    /// 自由市集成交後、待送達的賣家通知佇列（自主提案切片，ROADMAP 864）：per-owner 一份
    /// 「你的攤位被誰接手、換到了什麼」清單，攤位成交當下塞入、賣家下次連線時投遞並清空。
    /// 純記憶體、重啟歸零（比照 `stalls` 世界暫態慣例，非玩家永久資產，零 migration）。
    stall_notices: RwLock<vstallnotify::StallNoticeQueue>,
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
    /// 雨後彩虹剩餘天氣檢查 tick 數（雨後彩虹 v1，ROADMAP 780）：> 0 = 天邊正掛著一道彩虹。
    /// 純記憶體、無需持久化（重啟後從無彩虹重新演變）。每次 `tick_farm`（15 秒）由純函式
    /// `vweather::next_rainbow` 更新：雨→晴升起、持續晴天逐 tick 淡出；隨快照廣播 `rainbow:bool` 給前端。
    rainbow_ticks: RwLock<u32>,
    /// 彩虹剛升起的一次性旗標（ROADMAP 780）：`tick_farm` 偵測到「雨→晴」（彩虹 0→>0）時設 true，
    /// `tick_residents` 讀到後立即清回 false（consume-once），觸發附近居民抬頭望見彩虹的歡呼＋心情補助。
    rainbow_started_flag: RwLock<bool>,
    /// 上一輪偵測到的季節（季節輪替 v1，ROADMAP 798）：`tick_residents` 每輪由世界時鐘累計日數
    /// 推算當前季節，與此比對——不同即「換季」，設下方一次性旗標並上一則城鎮動態。純記憶體、重啟
    /// 從初春重新流轉（比照天氣／彩虹狀態）。
    last_season: RwLock<vseason::Season>,
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
    /// 探索紀事 v1（自主提案切片，接續 838/839）：玩家找到的地標座標與種類，可回頭翻閱。
    /// 持久化到 data/voxel_discoveries.jsonl（append-only，重啟後紀事仍在）。
    discovery: RwLock<vdisc::DiscoveryStore>,
    /// 地標旅人留言 v1（自主提案切片，ROADMAP 862）：每處地標的共同留言簿，發現地標時能讀到
    /// 先前旅人的話、也能留一句給後來的人。持久化到 data/voxel_landmark_notes.jsonl。
    landmark_notes: RwLock<vlmark::LandmarkNoteStore>,
    /// 玩家熟練度 v1（自主提案切片，ROADMAP 842）：⛏️採集／🌾耕種／🎣垂釣三條連續經驗值。
    /// 持久化到 data/voxel_mastery.jsonl（append-only，重啟後熟練度仍在）。
    mastery: RwLock<MasteryStore>,
    /// 同帳號去重——踢舊連線用的 oneshot 信號表（ROADMAP fix 幽靈分身）。
    /// 連線 UUID → 踢信號發送端；有同 email 的新連線進來時，從此表取出舊 UUID 的發送端並送 ()，
    /// 讓舊連線的 select! 觸發、優雅退出（幽靈分身消失）。
    /// 短鎖即釋、所有操作在鎖外 await，守 prod 死鎖鐵律。
    conn_kick: RwLock<HashMap<Uuid, oneshot::Sender<()>>>,
    /// 玩家生存指標（飢餓度＋血量，溫和版·後端權威）：玩家名 → PlayerStats。
    /// 只在伺服器算（tick 衰減/傷害、吃回復），廣播給玩家自己（別人看不到你的條，減噪）。
    /// 登入玩家持久化到 data/voxel_player_stats.jsonl（比照 #1024 位置持久化風格，重登保留）；
    /// 訪客 session 內有效即可（斷線清）。鍵用玩家顯示名（登入玩家綁帳號名，穩定）。
    player_stats: RwLock<HashMap<String, vstats::PlayerStats>>,
    /// 玩家獨門配方 v1（居民教你一道獨門配方，自主提案切片，ROADMAP 849）：玩家名 → 已被
    /// 居民教過、永久解鎖的獨門配方 id 集合。持久化到 data/voxel_player_recipes.jsonl
    /// （append-only，重啟後學會的配方仍在）。
    player_recipes: RwLock<vprecipe::PlayerRecipeStore>,
    /// 村莊集體里程碑 v1（自主提案切片，ROADMAP 856）：全域一份（不分玩家）的「村莊達成過
    /// 哪些集體門檻」帳本，與 `milestones`（per-player）刻意區隔。持久化到
    /// `data/voxel_village_milestones.jsonl`（append-only，重啟後仍記得）。
    village_milestones: RwLock<vvillms::VillageMilestoneStore>,
    /// 個人路標 v1（自主提案切片）：玩家名 → 自己插的路標（名字＋座標），在羅盤/雷達面板
    /// 與居民座標並列導航。持久化到 data/voxel_waypoints.jsonl（append-only，含刪除
    /// tombstone，重啟後仍記得）。
    waypoints: RwLock<vwaypoint::WaypointStore>,
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
        // 居民為你的個人里程碑喝采 v1（自主提案切片）：私人旅程之外，讓身邊閒著的居民也
        // 為這一刻由衷喝采。里程碑本身全庫只對每位玩家觸發一次，天然不會刷版。
        maybe_cheer_milestone(player, def);
    }
}

/// 居民為你的個人里程碑喝采 v1（自主提案切片，接續 724/856）：里程碑冪等解鎖成功後呼叫，
/// 找到玩家當下位置附近一位閒著的居民，讓她為你喝采＋記進心裡＋動態牆播報。
/// 比照 773/863 讚賞的「挑一位近旁有空的居民」手法（residents 讀鎖即釋、不巢狀）。
/// **鎖紀律**：players/residents 讀鎖各自短取即釋；residents 寫鎖另外短取即釋；
/// broadcast/memory append/feed append 全在鎖外，不巢狀、不持鎖 await。
fn maybe_cheer_milestone(player_name: &str, def: &vmiles::MilestoneDef) {
    let pos: Option<(f32, f32)> = {
        hub().players.read().unwrap().values().find(|p| p.name == player_name).map(|p| (p.x, p.z))
    }; // players 讀鎖釋放
    let Some((px, pz)) = pos else { return };
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
                let dx = px - r.body.x;
                let dz = pz - r.body.z;
                (r.id.clone(), r.name, dx * dx + dz * dz)
            })
            .filter(|(_, _, d2)| vmcheer::cheer_eligible(*d2))
            .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    }; // residents 讀鎖釋放
    let Some((rid, rname, _)) = cand else { return };
    let pick = vfarm::now_secs() as usize;
    let say_line = vmcheer::cheer_say_line(player_name, def.name_zh, def.icon, pick);
    let said = {
        let mut residents = hub().residents.write().unwrap();
        residents
            .iter_mut()
            .find(|r| r.id == rid)
            .map(|r| {
                r.say = say_line.chars().take(50).collect();
                r.say_timer = SAY_SECS;
                r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
            })
            .is_some()
    }; // residents 寫鎖釋放
    if said {
        broadcast_players();
        let summary = vmcheer::cheer_memory_line(player_name, def.name_zh);
        let entry = hub().memory.write().unwrap().add_memory(&rid, player_name, &summary);
        vmem::append_memory(&entry);
        vfeed::append_feed(
            "居民喝采",
            rname,
            &vmcheer::cheer_feed_line(rname, player_name, def.name_zh, def.icon),
        );
    }
}

/// 玩家熟練度 v1（自主提案切片，ROADMAP 842）：累加一次動作的經驗值，落地持久化；
/// 若剛好升級，單播 `mastery_levelup` 慶祝訊息給玩家自己（不廣播全員，同里程碑同為私人旅程）。
/// 回傳升級後目前等級，供呼叫端判斷是否已解鎖產出加成（`vmastery::mastery_yield_bonus`）。
/// **鎖紀律**：mastery 寫鎖短取即釋，append/送訊息都在鎖外，不巢狀、不持鎖 await。
fn award_mastery(player: &str, kind: MasteryKind, out_tx: &mpsc::Sender<Message>) -> u32 {
    let delta = kind.xp_per_action();
    let (_xp, leveled_up, level) = { hub().mastery.write().unwrap().add_xp(player, kind, delta) };
    vmastery::append_mastery(&vmastery::MasteryEntry {
        player: player.to_string(),
        kind: kind.as_str().to_string(),
        xp_delta: delta,
    });
    if leveled_up {
        let _ = out_tx.try_send(Message::Text(
            serde_json::json!({
                "t": "mastery_levelup",
                "kind": kind.as_str(),
                "level": level,
                "title": vmastery::title_for_level(level),
                "line": vmastery::levelup_line(kind, level),
            })
            .to_string(),
        ));
    }
    level
}

/// 玩家熟練度產出加成 v1：若這條熟練度已練到解鎖門檻，給玩家額外一份指定材料（與工具加成
/// 790／並肩協作 827 同量級 +1，各自獨立疊加）＋單播 `mastery_bonus` 揭曉句。
/// **鎖紀律**：inventory 寫鎖短取即釋，append/送訊息都在鎖外，呼叫端須確保未持有其他鎖。
fn give_mastery_bonus(
    player: &str,
    kind: MasteryKind,
    level: u32,
    item_id: u8,
    out_tx: &mpsc::Sender<Message>,
) {
    let bonus = vmastery::mastery_yield_bonus(level);
    if bonus == 0 {
        return;
    }
    let entry = hub().inventory.write().unwrap().give(player, item_id, bonus);
    vinv::append_inv(&entry);
    let new_count = hub().inventory.read().unwrap().count(player, item_id);
    let _ = out_tx.try_send(Message::Text(
        serde_json::json!({ "t": "inv_update", "block_id": item_id, "count": new_count }).to_string(),
    ));
    let _ = out_tx.try_send(Message::Text(
        serde_json::json!({
            "t": "mastery_bonus",
            "block_id": item_id,
            "count": bonus,
            "line": vmastery::bonus_line(kind),
        })
        .to_string(),
    ));
}

static HUB: OnceLock<VoxelHub> = OnceLock::new();

// ============================================================
// 玩家生存指標持久化（血/飢跨重登，比照 #1024 位置持久化：jsonl 一行一玩家）
// ============================================================
/// 玩家血/飢存檔路徑（`data/` 已 gitignore，執行期產生；重啟後登入玩家血/飢還在）。
const VOXEL_PLAYER_STATS_PATH: &str = "data/voxel_player_stats.jsonl";

/// 啟動時從磁碟載回玩家血/飢（檔缺＝首次啟動，回空）。純 IO：解析走 vstats 純函式。
/// 韌性：讀檔失敗／髒行都不 panic（比照其他 voxel store 載入慣例）。
fn load_player_stats() -> HashMap<String, vstats::PlayerStats> {
    let text = std::fs::read_to_string(VOXEL_PLAYER_STATS_PATH).unwrap_or_default();
    let mut map = HashMap::new();
    for row in vstats::parse_rows(&text) {
        map.insert(row.player.clone(), row.to_stats());
    }
    map
}

/// 把目前玩家血/飢快照落地（原子寫：寫暫存檔再 rename，避免半截檔）。
/// 呼叫端在鎖外呼叫（此函式自己短取讀鎖組快照即釋，不持鎖寫檔）。
fn persist_player_stats() {
    let rows: Vec<vstats::StatsRow> = {
        let map = hub().player_stats.read().unwrap();
        map.iter()
            .map(|(name, s)| vstats::StatsRow::from_stats(name, s))
            .collect()
    }; // 讀鎖釋放
    let text = vstats::serialize_rows(&rows);
    let tmp = format!("{VOXEL_PLAYER_STATS_PATH}.tmp");
    if std::fs::write(&tmp, text.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, VOXEL_PLAYER_STATS_PATH);
    }
}

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

/// 村莊系統 v1 一次性整理（冪等、保守、只加不拆）：把散落的居民家連成一座有街廓的村莊。
///
/// **資料安全鐵律**（絕不刪居民已蓋的作品）：
/// - 動檔前先備份 `voxel_resident_blocks.jsonl`（備份失敗即中止本次、下次啟動再試）。
/// - 只走 append-only 持久化路徑（`append_world_block`），不改寫既有行。
/// - **每一格路面都先查該格地表方塊是否為「自然地表」**（[`vvillage::is_natural_ground`]）——
///   遇既有建築 / 樹 / 水 / 農田 / 任何建材就跳過那格（不覆蓋、不拆）。
/// - 旗標檔冪等：跑過一次就寫 `data/voxel_village_done`，之後啟動直接跳過。
///
/// 做法：以居民 home_base 群聚中心定村莊中心 → 生成廣場鋪面＋四角燈＋十字主路 →
/// 再從廣場鋪 L 形路連到每個既有建築（home_base）。全部只鋪在自然地表、放發光燈點綴。
fn migrate_lay_out_village(deltas: &mut WorldDelta, residents: &[VoxelResident]) {
    // ① 已跑過就跳過（冪等）。
    if vvillage::village_done() {
        return;
    }
    // ② 動檔前備份（僅在原檔存在時）。備份失敗就中止本次整理（不冒險改資料），下次啟動再試。
    let src = vbuild::VOXEL_RES_BLOCKS_PATH;
    if std::path::Path::new(src).exists() {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let bak = format!("{src}.bak-village-{epoch}");
        if let Err(e) = std::fs::copy(src, &bak) {
            tracing::warn!("村莊整理：備份 {src} 失敗，本次略過（下次啟動再試）：{e}");
            return;
        }
        tracing::info!("村莊整理：已備份世界改動到 {bak}");
    }

    // 村莊中心 = 居民 home_base 群聚質心（空則退回出生點 0,0）。
    let home_bases: Vec<(i32, i32)> = residents
        .iter()
        .map(|r| (r.home_x.floor() as i32, r.home_z.floor() as i32))
        .collect();
    let (vcx, vcz) = vvillage::village_center(&home_bases);
    let biome = voxel::biome_at_voxel(vcx, vcz);
    let plan = vvillage::plan_village(vcx, vcz, biome);
    let road_surface = vvillage::road_surface(biome);
    let plaza_surface = vvillage::plaza_surface(biome);

    // 就地鋪一格路面：找地表 y，只有當「該格地表方塊是自然地表、且其上方是空氣」時才鋪
    // （絕不覆蓋作品；上方被牆體/樹幹/水佔著＝路遇到東西了，在此打住、不鑽進建築底下）。
    // 回傳是否真的鋪了（供計數）。
    let mut lay_surface = |deltas: &mut WorldDelta, x: i32, z: i32, surf: Block| -> bool {
        let sy = vbuild::surface_y(x, z); // 地面正上方一格
        let gy = sy - 1; // 地表方塊本身
        let cur = voxel::effective_block_at(deltas, x, gy, z);
        if !vvillage::is_natural_ground(cur) {
            return false; // 遇建築/樹/水/農田等 → 不鋪、不拆
        }
        // 上方必須是空氣：牆/樹幹/水佔著＝遇到東西，路到此為止（不鑽建築、不入水）。
        if voxel::effective_block_at(deltas, x, sy, z) != Block::Air {
            return false;
        }
        if cur == surf {
            return false; // 已是同材質（重跑冪等）→ 不重複落地
        }
        voxel::set_block(deltas, x, gy, z, surf);
        vbuild::append_world_block(x, gy, z, surf as u8);
        true
    };

    let mut paved = 0usize;
    // 廣場鋪面（中央除外——中央留給水井）。
    for &(x, z) in &plan.plaza {
        if (x, z) == plan.well_center {
            continue;
        }
        if lay_surface(deltas, x, z, plaza_surface) {
            paved += 1;
        }
    }
    // 十字主路。
    for &(x, z) in &plan.road {
        if lay_surface(deltas, x, z, road_surface) {
            paved += 1;
        }
    }
    // 從廣場鋪 L 形路連到每個既有建築（居民 home_base）。
    for &(hx, hz) in &home_bases {
        for (x, z) in vvillage::pave_path_cells(vcx, vcz, hx, hz) {
            if lay_surface(deltas, x, z, road_surface) {
                paved += 1;
            }
        }
    }

    // 廣場四角燈（火把）：放在廣場鋪面「之上」一格（發光點綴，也讓村莊遠處認得出）。
    // 僅在該格目前為空氣時放（不覆蓋任何既有方塊）。
    let mut lanterns = 0usize;
    for &(x, z) in &plan.lantern_cells {
        let sy = vbuild::surface_y(x, z);
        if voxel::effective_block_at(deltas, x, sy, z) == Block::Air {
            voxel::set_block(deltas, x, sy, z, Block::Torch);
            vbuild::append_world_block(x, sy, z, Block::Torch as u8);
            lanterns += 1;
        }
    }

    // ③ 寫旗標檔（冪等）。
    vvillage::mark_village_done(vcx, vcz);
    // Feed 一句（面向玩家）：村裡鋪起了石板路。
    vfeed::append_feed("村莊整理", "村子", vvillage::village_feed_line());
    tracing::info!(
        "村莊整理完成：中心 ({vcx},{vcz})／{}，鋪了 {paved} 格石板路、點了 {lanterns} 盞廣場燈（只鋪自然地表、未覆蓋任何既有建築）",
        voxel::biome_name(biome)
    );
}

/// 村莊大修復一次性標記（存在即代表已跑過，冪等不再跑；`data/` 已 gitignore）。
const VILLAGE_RESTORE_MARKER: &str = "data/.village_restored_v1";

/// **村莊大修復（migration，動玩家/居民資料——保守、冪等、備份後動）**。
///
/// 背景：居民為鋪路/合成挖石（階梯礦井）、採集、水邊整地，把村莊中心 (0,19) 一帶挖出大坑
/// （相機會掉進去）、挖穿水脈導致大面積淹水灌進村區（實測地表 7000+ 個洞）。此函式一次性
/// 掃村莊半徑 [`vvillage::VILLAGE_RESTORE_RADIUS`] 內的柱，**回填被挖低於自然地表的坑**
/// （回基底材料）、**清掉灌進來的流動水**——但**絕不動保留清單**。
///
/// **保留清單（絕不回填/絕不清）**：靠「只動 Air／流動水格、且該格自然基底是實心」這道
/// 判定天然把建築/道路/廣場/農田/告示牌/箱子/床/火把/工作台/熔爐/樹/源水湖全排除在外——
/// 那些格目前有東西（非 Air、非流動水），[`vvillage::village_hole_refill`] 一律回 `None`。
/// 深礦道（y < [`vvillage::VILLAGE_REFILL_MIN_Y`]）也保留（合理採礦，非村容坑）。
///
/// **回傳**：需喚醒水流重算的邊界格（呼叫端把它們預先排入水流佇列，讓殘餘水穩定）。
///
/// **資料安全**：① 有 marker 就直接跳過（只跑一次）。② 動檔前先把
/// `voxel_resident_blocks.jsonl` 備份成 `.bak-restore-<epoch>`。③ 回填/清水走既有
/// `append_world_block` append-only 路徑（不改寫、不刪任何既有行）。④ 冪等：即便 marker
/// 被刪重跑，已回填的格現為實心／已清的水格現為 Air → 判定回 None/false → 不重覆動、不再 append。
fn migrate_restore_village(deltas: &mut WorldDelta, residents: &[VoxelResident]) -> Vec<(i32, i32, i32)> {
    // ① 已跑過就跳過。
    if std::path::Path::new(VILLAGE_RESTORE_MARKER).exists() {
        return Vec::new();
    }
    // ② 動檔前備份（僅在原檔存在時）。備份失敗就中止本次修復（不冒險改資料），下次啟動再試。
    let src = vbuild::VOXEL_RES_BLOCKS_PATH;
    if std::path::Path::new(src).exists() {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let bak = format!("{src}.bak-restore-{epoch}");
        if let Err(e) = std::fs::copy(src, &bak) {
            tracing::warn!("村莊大修復：備份 {src} 失敗，本次略過（下次啟動再試）：{e}");
            return Vec::new();
        }
        tracing::info!("村莊大修復：已備份世界改動到 {bak}");
    }

    // 村莊中心：優先用一次性整理釘死的中心（load_village_center，prod 即 (0,19)），
    // 缺檔（極舊世界）→ 退回居民 home_base 群聚質心（與 migrate_lay_out_village 同源）。
    let (vcx, vcz) = vvillage::load_village_center().unwrap_or_else(|| {
        let home_bases: Vec<(i32, i32)> = residents
            .iter()
            .map(|r| (r.home_x.floor() as i32, r.home_z.floor() as i32))
            .collect();
        vvillage::village_center(&home_bases)
    });

    let r = vvillage::VILLAGE_RESTORE_RADIUS;
    let mut refilled = 0usize;
    let mut drained = 0usize;
    // 需喚醒水流的邊界格：清水/回填後，讓殘餘水在鎖外由水流模擬重算穩定。
    let mut wake: Vec<(i32, i32, i32)> = Vec::new();

    // 掃村莊半徑內每一柱、每一層地表格（y 3..=14），逐格判定回填/排水。
    for x in (vcx - r)..=(vcx + r) {
        for z in (vcz - r)..=(vcz + r) {
            if !vvillage::in_village_restore_range(vcx, vcz, x, z) {
                continue; // 只動歐氏圓內（角落不掃）
            }
            for y in vvillage::VILLAGE_REFILL_MIN_Y..=vvillage::VILLAGE_REFILL_MAX_Y {
                let cur = voxel::effective_block_at(deltas, x, y, z);
                let base = voxel::block_at(x, y, z);
                // ── 地形回填：被挖低於自然地表的坑 → 回基底材料（保留清單天然被濾掉）──
                if let Some(fill) = vvillage::village_hole_refill(base, cur, y) {
                    voxel::set_block(deltas, x, y, z, fill);
                    vbuild::append_world_block(x, y, z, fill as u8);
                    refilled += 1;
                    wake.push((x, y, z)); // 填了坑＝改了水路，喚醒鄰格重算
                    continue; // 這格已回填成實心，不會再是流動水
                }
                // ── 排水：灌進來的流動水 → 空氣（源水湖 7 不動）──
                if vvillage::village_should_drain(cur) {
                    voxel::set_block(deltas, x, y, z, Block::Air);
                    vbuild::append_world_block(x, y, z, Block::Air as u8);
                    drained += 1;
                    wake.push((x, y, z)); // 清了水＝改了水路，喚醒鄰格重算殘餘穩定
                }
            }
        }
    }

    // ③ 寫 marker（冪等）。
    if let Some(parent) = std::path::Path::new(VILLAGE_RESTORE_MARKER).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(VILLAGE_RESTORE_MARKER, b"1");
    // Feed 一句溫暖的（面向玩家）。
    vfeed::append_feed("村莊修復", "村子", vvillage::village_restore_feed_line());
    tracing::info!(
        "村莊大修復完成：中心 ({vcx},{vcz})，回填 {refilled} 個地表坑、清 {drained} 格灌進來的流動水（保守判定，未動任何建築/道路/農田/功能方塊/樹/源水湖/深礦道）"
    );
    wake
}

/// 挖掘紀律：居民自主開挖的**離村禁區**（快取一次）。
/// 回 `Some((vcx, vcz, radius))`＝村中心與禁區半徑，供 [`vskill::find_nearest_resource_excl`]
/// 等選址跳過村內格；`None`＝村莊尚未規劃/釘死中心（極舊/乾淨世界，不設限）。
/// **只擋居民自主挖資源**（採集/發明/自主備料）；玩家指定的工地（整地/鋪面）不查此、傳 None。
/// 快取：村莊中心一旦釘死就不變（見 voxel_village 旗標檔），啟動載一次即可，熱路徑零 IO。
fn village_dig_exclusion() -> Option<(i32, i32, i32)> {
    static CACHE: OnceLock<Option<(i32, i32, i32)>> = OnceLock::new();
    *CACHE.get_or_init(|| {
        vvillage::load_village_center()
            .map(|(vcx, vcz)| (vcx, vcz, vvillage::VILLAGE_DIG_EXCLUSION_RADIUS))
    })
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
        // 居民先建好（村莊整理需要各家家域中心當「既有建築」的位置參考）。
        let residents = init_residents();
        // 村莊系統 v1 一次性整理（冪等、備份後）：以居民 home_base 群聚中心定村莊中心，
        // 鋪出中央廣場＋十字主路，並從廣場鋪 L 形路連到每個既有建築——只加不拆、遇非地表方塊即停，
        // 把散落的家連成一座有街廓的村莊。這一步只在乾淨/舊世界啟動時跑一次（旗標檔冪等）。
        migrate_lay_out_village(&mut deltas, &residents);
        // 村莊大修復（冪等、備份後）：居民把村莊中心一帶挖爛（大坑＋灌水），此步一次性回填
        // 被挖低於自然地表的坑、清掉灌進來的流動水——絕不動任何建築/道路/農田/功能方塊/樹/源水湖。
        // 回傳需喚醒水流重算的邊界格，預先排入水流佇列讓殘餘水穩定（hub 尚未成形，不能呼叫
        // enqueue_water_around；改成建構時就把這些格塞進 water_queue 初值）。
        let water_wake = migrate_restore_village(&mut deltas, &residents);
        // 乙太營火 v1：從已 replay 的 delta 掃出所有既存營火座標，重建取暖清單（重啟後居民
        // 仍會被重啟前蓋的火堆吸引）。掃描發生在 deltas 被 move 進 RwLock 之前，一次性、非熱路徑。
        let campfires = vcamp::scan_campfires(&deltas);
        // 木長椅 v1：同理從 replay 後的 delta 掃出所有既存長椅座標，重建歇腳清單（重啟後居民
        // 仍會被重啟前擺的長椅吸引坐下歇腳）。一次性、非熱路徑。
        let benches = vbench::scan_benches(&deltas);
        VoxelHub {
            players: RwLock::new(HashMap::new()),
            deltas: RwLock::new(deltas),
            campfires: RwLock::new(campfires),
            benches: RwLock::new(benches),
            // 野兔 v1：啟動時於固定家域點生成（純記憶體、重啟即重新生成，零持久化）。
            wildlife: RwLock::new(init_wildlife()),
            residents: RwLock::new(residents),
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
            // 村莊系統 v1：啟動時從 data/voxel_village_plots.jsonl 載回地塊認領（重啟後仍記得誰住哪塊）。
            village: RwLock::new(vvillage::PlotRegistry::from_entries(vvillage::load_plot_claims())),
            // 居民搬新家：啟動時從 data/voxel_relocations.jsonl 載回搬家進度（重啟後接著搬）。
            relocations: RwLock::new(vvillage::RelocationStore::from_entries(
                vvillage::load_relocations(),
            )),
            // 居民小背包純記憶體（採集成果；重啟重置，與農地一致）。
            res_inv: RwLock::new(HashMap::new()),
            // 啟動時從 data/voxel_inventory.jsonl 載回玩家背包（重啟後存量還在）。
            inventory: RwLock::new(InvStore::from_entries(vinv::load_inventory())),
            // 農地持久化 v1：啟動時從 data/voxel_farm.jsonl replay 種植計時（重啟後作物續存續長）。
            farm: RwLock::new(FarmStore::from_events(vfarm::load_farm())),
            // 熔爐煨煮 v1：啟動時從 data/voxel_smelt.jsonl 載回未交付的爐（重啟後那爐還在煨）。
            smelt: RwLock::new(vsmelt::SmeltStore::from_events(vsmelt::load_smelt())),
            grove: RwLock::new(GroveStore::new()),
            berry: RwLock::new(BerryStore::new()),
            coop: RwLock::new(CoopStore::new()),
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
            // 居民戀愛 v1（ROADMAP 846）：啟動時從 data/voxel_romance.jsonl 載回已締結的戀人對。
            romance: RwLock::new(ResidentRomance::from_entries(vromance::load_romance())),
            // 知恩圖報 v1（ROADMAP 801）：欠飯帳本純記憶體、啟動時空的（過場恩情、重啟歸零、零持久化）。
            meal_debts: RwLock::new(vgrat::MealDebts::default()),
            // 啟動時從 data/voxel_chests.jsonl 載回箱子存量（重啟後仍保留儲存物品）。
            chest: RwLock::new(vchest::ChestStore::from_entries(vchest::load_chests())),
            // 啟動時從 data/voxel_signs.jsonl 載回告示牌文字（重啟後牌面仍在）。
            sign: RwLock::new(vsign::SignStore::from_entries(vsign::load_signs())),
            // 啟動時從 data/voxel_bottles.jsonl 載回尚未被撿走的瓶中信（重啟後瓶子還在水裡）。
            bottle: RwLock::new(BottleStore::from_entries(vbottle::load_bottles())),
            // 掉落物 v1：純記憶體，重啟歸零（暫留地上等人撿的短命狀態，非玩家永久資產）。
            drops: RwLock::new(DropStore::new()),
            // 交易攤 v1：純記憶體，重啟歸零（比照 drops，世界暫態，非玩家永久資產）。
            stalls: RwLock::new(StallStore::new()),
            // 自由市集賣家通知佇列：純記憶體，重啟歸零（比照 stalls，世界暫態，非玩家永久資產）。
            stall_notices: RwLock::new(vstallnotify::StallNoticeQueue::new()),
            // 啟動時從 data/voxel_gift_gardens.jsonl 載回未收成的禮物菜園（重啟後那畦田還在，
            // 待種它的居民遇到送種子的你時收成回贈）。
            giftgarden: RwLock::new(vgg::GiftGardenStore::from_entries(vgg::load_gift_gardens())),
            // 水流佇列：啟動一般空；玩家/居民挖破地形時排入缺口鄰格，水才開始流。
            // 例外：村莊大修復回填/清水後留下的邊界格先預排進來（含 6 鄰格），
            // 讓水流模擬一啟動就把殘餘水重算穩定（此時 hub 尚未成形，不能呼叫 enqueue_water_around）。
            water_queue: std::sync::Mutex::new({
                let mut q = WaterQueue::default();
                for (x, y, z) in &water_wake {
                    q.push(*x, *y, *z);
                    for (dx, dy, dz) in vwater::PROPAGATE_OFFSETS {
                        q.push(x + dx, y + dy, z + dz);
                    }
                }
                q
            }),
            // 天氣：啟動時永遠從晴天開始，之後靠 tick_farm 的機率擲骰自然演變。
            weather: RwLock::new(false),
            // 雨剛開始旗標：啟動時無雨無旗標。
            rain_started_flag: RwLock::new(false),
            // 雨後彩虹：啟動時晴天、無彩虹。
            rainbow_ticks: RwLock::new(0),
            rainbow_started_flag: RwLock::new(false),
            // 季節輪替 v1（ROADMAP 798）：啟動時世界日數為 0 ＝初春；之後靠 tick_residents 逐日推進換季。
            last_season: RwLock::new(vseason::season_for_day(0)),
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
            // 啟動時從 data/voxel_discoveries.jsonl 載回玩家的探索紀事（重啟後仍記得）。
            discovery: RwLock::new(vdisc::DiscoveryStore::from_entries(vdisc::load_discoveries())),
            landmark_notes: RwLock::new(vlmark::LandmarkNoteStore::from_entries(vlmark::load_notes())),
            mastery: RwLock::new(MasteryStore::from_entries(vmastery::load_mastery())),
            // 同帳號去重：啟動空，每條連線進場時登記、離場時清除。純記憶體、無需持久化。
            conn_kick: RwLock::new(HashMap::new()),
            // 玩家生存指標：啟動時從 data/voxel_player_stats.jsonl 載回血/飢（重登保留，比照 #1024）。
            player_stats: RwLock::new(load_player_stats()),
            // 啟動時從 data/voxel_player_recipes.jsonl 載回玩家已被教過的獨門配方（重啟後仍記得）。
            player_recipes: RwLock::new(vprecipe::PlayerRecipeStore::from_entries(vprecipe::load_player_recipes())),
            // 啟動時從 data/voxel_village_milestones.jsonl 載回村莊已達成的集體門檻（重啟後仍記得）。
            village_milestones: RwLock::new(vvillms::VillageMilestoneStore::from_entries(
                vvillms::load_village_milestones(),
            )),
            // 啟動時從 data/voxel_waypoints.jsonl 載回玩家個人路標（含刪除 tombstone，重啟後仍記得）。
            waypoints: RwLock::new(vwaypoint::WaypointStore::from_entries(vwaypoint::load_entries())),
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
    // 雨後彩虹 v1（ROADMAP 780，短鎖、不巢狀）：> 0 tick = 天邊正掛著彩虹，前端據此顯示彩虹弧。
    let rainbow: bool = *hub().rainbow_ticks.read().unwrap() > 0;
    // 季節輪替 v1（ROADMAP 798，短鎖、不巢狀）：由世界累計日數推算當前季節，帶給前端隨季節微染天地色調。
    let season: &str = {
        let day = hub().world_time.read().unwrap().days_elapsed();
        vseason::season_for_day(day).as_str()
    };
    // 野兔快照（野兔 v1，ROADMAP 847，短鎖、不巢狀）：純位置/朝向，前端渲染環境點綴用。
    let wildlife: Vec<WildlifeView> = {
        let a = hub().wildlife.read().unwrap();
        a.iter()
            .map(|w| WildlifeView { id: w.id.clone(), kind: w.kind.wire(), x: w.body.x, y: w.body.y, z: w.body.z, yaw: w.yaw })
            .collect()
    }; // 野兔讀鎖在此釋放
    serde_json::json!({
        "t": "players",
        "players": players,
        "residents": residents,
        "wildlife": wildlife,
        "time_of_day": time_of_day,
        "raining": raining,
        "rainbow": rainbow,
        "season": season,
    }).to_string()
}

/// 廣播一次最新玩家快照給所有連線。
fn broadcast_players() {
    let snap = Arc::new(players_snapshot_json());
    // 沒有訂閱者時 send 會 Err，無所謂（單人在線）。
    let _ = hub().tx.send(snap);
}

/// 居民注意到你悉心照料的農地 v1（自主提案切片）：在 Plant/HoeTill 成功後呼叫，推進這條
/// 連線的「農忙連段」、挑一位近旁有空的居民，過門檻就讚賞＋記進心裡＋動態牆播報。
/// 與 773 建造讚賞（`build_streak`/`admire_cd`）完全獨立的一組連段/冷卻，互不干擾——
/// 比照 773 讚賞在 Place handler 內的挑人／冷卻／落地手法（residents 讀鎖即釋、不巢狀）。
fn maybe_farm_admire(
    x: f32,
    z: f32,
    name: &str,
    farm_streak: &mut Option<vfarmadmire::FarmStreak>,
    farm_admire_cd: &mut std::collections::HashMap<String, std::time::Instant>,
) {
    let now_secs = vfarm::now_secs();
    *farm_streak = Some(vfarmadmire::advance_streak(*farm_streak, x, z, now_secs));
    let streak = farm_streak.map_or(0, |s| s.0);
    if streak < vfarmadmire::FARM_ADMIRE_STREAK_MIN {
        return;
    }
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
                let dx = x - r.body.x;
                let dz = z - r.body.z;
                (r.id.clone(), r.name, dx * dx + dz * dz)
            })
            .filter(|(_, _, d2)| {
                *d2 <= vfarmadmire::FARM_ADMIRE_RADIUS * vfarmadmire::FARM_ADMIRE_RADIUS
            })
            .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    }; // residents 讀鎖釋放
    let Some((rid, rname, dist_sq)) = cand else { return; };
    let now = std::time::Instant::now();
    let cooldown_ok = match farm_admire_cd.get(&rid) {
        Some(prev) => {
            now.duration_since(*prev).as_secs() >= vfarmadmire::FARM_ADMIRE_COOLDOWN_SECS
        }
        None => true,
    };
    if !vfarmadmire::admire_triggers(streak, dist_sq, cooldown_ok) {
        return;
    }
    farm_admire_cd.insert(rid.clone(), now);
    let pick = now_secs as usize;
    let say_line = vfarmadmire::admire_say_line(name, pick);
    let said = {
        let mut residents = hub().residents.write().unwrap();
        residents
            .iter_mut()
            .find(|r| r.id == rid)
            .map(|r| {
                r.say = say_line.chars().take(50).collect();
                r.say_timer = SAY_SECS;
                r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
            })
            .is_some()
    }; // residents 寫鎖釋放
    if said {
        broadcast_players();
        // 把「看著你在田裡忙進忙出」寫進記憶（episodic，累積好感）——記憶寫鎖即釋，
        // append 的 IO 在鎖外（守 prod 死鎖鐵律）。
        let summary = vfarmadmire::admire_memory_line(name);
        let entry = hub().memory.write().unwrap().add_memory(&rid, name, &summary);
        vmem::append_memory(&entry);
        vfeed::append_feed(
            "居民讚賞",
            rname,
            &format!("{rname}看著{name}在田裡忙進忙出，悉心照料，由衷讚賞。"),
        );
    }
}

// ── WS 協定（JSON，全是 voxel 自己的型別）──────────────────────────────────────

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum ClientMsg {
    /// 入場：帶顯示名（可選）。
    Join { name: Option<String> },
    /// 位置更新（前端權威預測，伺服器照收並廣播給別人；切片①不做伺服器端反作弊）。
    /// 手持工具可見 v1（自主提案切片）：`held` 為目前熱鍵選中的物品 id（additive、
    /// `#[serde(default)]` 向後相容——舊前端不送即沒有手持顯示，純視覺不影響行為）。
    Move {
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
        #[serde(default)]
        held: Option<u8>,
    },
    /// 走到新區塊時補要 chunk（cx,cz 為 chunk 座標，伺服器補該 column 的 cy 範圍）。
    Req { cx: i32, cz: i32 },
    /// 破壞方塊：目標方塊世界座標。伺服器驗證觸及範圍/實心後挖掉並廣播。
    /// 工欲善其事 v1（790）：`tool` 為前端自報的手持物品 id（additive、`#[serde(default)]`
    /// 向後相容——舊前端不送即無工具加成）。伺服器**必查背包確認真持有該工具**才給加成，
    /// 防偽報白嫖。
    Break {
        x: i32,
        y: i32,
        z: i32,
        #[serde(default)]
        tool: Option<u8>,
    },
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
    /// 餵野兔馴服 v1（自主提案切片）：手持胡蘿蔔、準心對準一隻野兔 → 就地餵食。
    /// 伺服器驗證（種類必須是野兔＋觸及範圍夠近＋尚未馴服過＋背包真持有胡蘿蔔）後
    /// 消耗 1 根胡蘿蔔，永久馴服這隻兔子（此後不再受驚逃跑）。`id` 是 wildlife 系統 id
    /// （"vox_wld_N"，見 `WildlifeAnimal.id`）。
    #[serde(rename = "feed_wildlife")]
    FeedWildlife { id: String },
    /// 放養雞 v1（自主提案切片，ROADMAP 870）：手持小麥種子、準心對準一隻雞 → 就地餵食。
    /// 伺服器驗證（種類必須是雞＋觸及範圍夠近＋尚未馴服過＋背包真持有種子）後消耗
    /// 1 顆種子，永久馴服這隻雞（此後跟隨你、定期回饋一顆蛋）。`id` 是 wildlife 系統 id
    /// （"vox_chk_N"，見 `WildlifeAnimal.id`）。
    #[serde(rename = "feed_chicken")]
    FeedChicken { id: String },
    /// 居民交易 v1：向指定居民請求以物易物（ROADMAP 670）。
    /// 伺服器回 `trade_offer`，玩家再傳 TradeAccept 接受；提案 30 秒後自動過期。
    #[serde(rename = "trade_request")]
    TradeRequest { resident_id: String },
    /// 居民交易 v1：接受當前待確認的交易提案（ROADMAP 670）。
    /// 付幣代替湊材料 v1（ROADMAP 874）起，`pay_with_coin=true` 時改直接扣提案的
    /// `coin_price` 枚乙太幣成交，不必湊出 `want_item`；省略／`false` 維持 v1 原行為。
    #[serde(rename = "trade_accept")]
    TradeAccept {
        resident_id: String,
        #[serde(default)]
        pay_with_coin: bool,
    },
    /// 箱子 v1：開啟指定座標的箱子，伺服器回傳 `chest_view`（ROADMAP 692）。
    #[serde(rename = "open_chest")]
    OpenChest { x: i32, y: i32, z: i32 },
    /// 箱子 v1：把背包中的 `count` 個 `item_id` 放入箱子（ROADMAP 692）。
    #[serde(rename = "chest_put")]
    ChestPut { x: i32, y: i32, z: i32, item_id: u8, count: u32 },
    /// 箱子 v1：從箱子取出 `count` 個 `item_id` 到背包（ROADMAP 692）。
    #[serde(rename = "chest_take")]
    ChestTake { x: i32, y: i32, z: i32, item_id: u8, count: u32 },
    /// 告示牌 v1：寫／改寫目標告示牌的文字（ROADMAP 740）。伺服器驗 reach + 目標為
    /// Sign(66) 後清洗文字、存檔並廣播 `sign` 給所有人。空字串＝清空牌面。
    #[serde(rename = "sign_set")]
    SignSet { x: i32, y: i32, z: i32, text: String },
    /// 漂流瓶 v1：對準水面丟一只瓶中信（自主提案切片 825）。伺服器驗 reach + 目標為水面 +
    /// 登入身分 + 手持空玻璃瓶(83) 後清洗文字、內容審查，扣一只瓶子並存檔，僅廣播座標
    /// （內文絕不外流，只有撿到的人才讀得到）。
    #[serde(rename = "throw_bottle")]
    ThrowBottle { x: i32, y: i32, z: i32, text: String },
    /// 漂流瓶 v1：撿起指定座標的瓶中信（自主提案切片 825）。伺服器驗 reach 後把內文單播給
    /// 撿到的玩家、從世界移除該瓶（一次性拾起），廣播座標讓所有人的世界同步移除浮標。
    #[serde(rename = "read_bottle")]
    ReadBottle { x: i32, y: i32, z: i32 },
    /// 掉落物 v1：對著 `(x,y,z)` 丟下手上 `count` 個 `item_id`（自主提案切片 828）。伺服器驗
    /// reach + 背包足量後扣下、落地存進世界（安靜留在原地），廣播讓所有人看見；任何玩家
    /// （含自己）之後走近即自動撿起（見 `Move` handler）。
    #[serde(rename = "drop_item")]
    DropItem { x: i32, y: i32, z: i32, item_id: u8, count: u32 },
    /// 玩家自由市集 v1：在瞄準座標擺一個交易攤（自主提案切片 832）。伺服器驗 reach + 該格
    /// 可擺攤（空氣＋腳下實心）+ 給出/要求物品有效 + 背包足量 give_item 後扣下（escrow）、
    /// 存進世界並廣播 `stall_open` 給所有人。
    #[serde(rename = "stall_open")]
    StallOpen {
        x: i32,
        y: i32,
        z: i32,
        give_item: u8,
        give_count: u32,
        want_item: u8,
        want_count: u32,
    },
    /// 玩家自由市集 v1：與瞄準座標上的攤位互動（自主提案切片 832）。伺服器驗 reach 後判斷：
    /// 若互動者正是擺攤者本人 → 視為收攤（退還 escrow 材料）；否則視為接手成交
    /// （需背包持有 want_item×want_count，成交後雙方各自入帳）。
    #[serde(rename = "stall_interact")]
    StallInteract { x: i32, y: i32, z: i32 },
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
    /// QA 專用授予物品（只在環境變數 `BUTFUN_QA_DEBUG=1` 時生效，正式線上完全惰性/被忽略）：
    /// 讓隔離 QA 伺服器能給測試玩家幾個食物來驗「吃→飢餓回復＋背包扣」的真實往返。
    /// 濫用防護：未設 QA flag 時伺服器直接無視此訊息（不授予、不回應），故正式環境無法靠它刷物品。
    #[serde(rename = "qa_grant")]
    QaGrant { item_id: u8, count: u32 },
    /// 乙太煙火 v1（ROADMAP 785）：朝夜空施放一束背包裡的乙太煙火(68)。伺服器驗每連線
    /// 冷卻＋消耗一份煙火後，廣播 `firework`（施放者頭頂上方位置＋火花配色）給全場，附近
    /// 醒著的居民抬頭歡呼。無座標欄位——火花在施放者頭頂夜空綻放，位置由伺服器取施放者當前
    /// 座標決定（客戶端不自報位置，防偽造他人施放）。
    #[serde(rename = "firework_launch")]
    FireworkLaunch,
    /// 乙太沃肥 v1（ROADMAP 789）：手持沃肥(69)對準一株幼苗施肥。(x,y,z) 是瞄準的作物方塊。
    /// 伺服器驗觸及範圍 + 目標為 Seeded 幼苗(12/46/50) + 背包有沃肥 → 消耗一份、把該格農地
    /// 生長計時往前推進 `FERTILIZER_BOOST_SECS`（沿用 nudge_growth，持久化），回 `fertilize_ok`。
    Fertilize { x: i32, y: i32, z: i32 },
    /// 水桶舀水 v1（自主提案切片）：手持空水桶對準一格水源 → 伺服器驗持有空水桶 + 目標為
    /// 來源水 + 觸及範圍內 → 該格化為空氣（喚醒鄰格重算水流）、背包空水桶換成滿水桶，回 `bucket_ok`。
    #[serde(rename = "bucket_fill")]
    BucketFill { x: i32, y: i32, z: i32 },
    /// 水桶倒水 v1（自主提案切片）：手持滿水桶對準一格空氣／流動水 → 伺服器驗持有滿水桶 +
    /// 目標可倒 + 觸及範圍內 → 該格放下一格永久來源水（既有水流模擬自然漫開）、滿水桶換回空水桶。
    #[serde(rename = "bucket_pour")]
    BucketPour { x: i32, y: i32, z: i32 },
    /// 鋤頭開墾 v1（自主提案切片）：手持木鋤頭對準一格草地／泥土 → 伺服器驗持有鋤頭 +
    /// 目標可鋤（草／土）+ 觸及範圍內 → 該格就地翻成農田土（`Block::FarmSoil`），回 `hoe_ok`。
    /// 鋤頭是工具、反覆使用不耗損（比照鎬／斧採集不消耗工具），只驗持有、不消耗。
    #[serde(rename = "hoe_till")]
    HoeTill { x: i32, y: i32, z: i32 },
    /// 集會鐘 v1（自主提案切片）：右鍵敲響一座集會鐘。(x,y,z) 是瞄準的鐘方塊。伺服器驗
    /// 觸及範圍 + 目標為 Bell(74) → 把範圍內閒著、醒著、冷卻到期的居民設為「應召」，讓牠們
    /// 循聲朝鐘走來；至少召到一位才廣播鐘聲＋上 Feed（濫用防護：無在場可召者則不廣播、不洗版）。
    #[serde(rename = "ring_bell")]
    RingBell { x: i32, y: i32, z: i32 },
    /// 地標旅人留言 v1（自主提案切片，ROADMAP 862）：在一處地標留一句話給後來的旅人。
    /// `(x,y,z)` 對古代遺跡是瞄準乙太礦（現）在的座標（`voxel::ruin_ore_at` 純座標判定，
    /// 挖掉後仍成立）；溫泉不看座標、改看玩家目前是否泡在溫泉裡（`feet_in_hot_spring`）。
    /// 伺服器驗證身分＋內容審查後存檔＋回傳這處地標目前的留言簿（供前端顯示先前旅人的話）。
    #[serde(rename = "leave_landmark_note")]
    LeaveLandmarkNote { x: i32, y: i32, z: i32, text: String },
    /// 個人路標 v1（自主提案切片，ROADMAP 869）：在玩家目前所站的位置插一支路標，取個
    /// 短名字。座標一律由伺服器讀 `player_pos` 決定，不信任客戶端自報位置；同名重插＝
    /// 原地改寫。成功回傳這位玩家目前完整的路標清單（`waypoint_sync`）。
    #[serde(rename = "set_waypoint")]
    SetWaypoint { label: String },
    /// 個人路標 v1：刪除指定名字的路標。找不到回 `waypoint_fail`；成功回傳更新後的清單。
    #[serde(rename = "remove_waypoint")]
    RemoveWaypoint { label: String },
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

/// 廣播世界上出現一只新漂流瓶（漂流瓶 v1，自主提案切片 825）。只送座標，**絕不**廣播內文——
/// 內容只有真的撿起來的人才讀得到，讓「撿到」保有巧遇的驚喜。
fn broadcast_bottle_dropped(x: i32, y: i32, z: i32) {
    let msg = Arc::new(
        serde_json::json!({ "t": "bottle_dropped", "x": x, "y": y, "z": z }).to_string(),
    );
    let _ = hub().tx.send(msg);
}

/// 廣播一只漂流瓶被撿走，所有人的世界同步移除該座標的浮標（漂流瓶 v1）。
fn broadcast_bottle_removed(x: i32, y: i32, z: i32) {
    let msg = Arc::new(
        serde_json::json!({ "t": "bottle_removed", "x": x, "y": y, "z": z }).to_string(),
    );
    let _ = hub().tx.send(msg);
}

/// 廣播世界上出現一件新掉落物（掉落物 v1，自主提案切片 828）。與漂流瓶不同——
/// 掉落物**不匿名**，帶上丟下者姓名供前端做懸浮提示（誰丟的一目了然，非驚喜巧遇）。
fn broadcast_item_dropped(id: u64, x: f32, y: f32, z: f32, item_id: u8, count: u32, dropped_by: &str) {
    let msg = Arc::new(
        serde_json::json!({
            "t": "item_dropped", "id": id, "x": x, "y": y, "z": z,
            "item_id": item_id, "count": count, "dropped_by": dropped_by,
        })
        .to_string(),
    );
    let _ = hub().tx.send(msg);
}

/// 廣播一件掉落物從世界上消失（被撿走或逾時消散），所有人同步移除浮標（掉落物 v1）。
fn broadcast_item_removed(id: u64) {
    let msg = Arc::new(serde_json::json!({ "t": "item_removed", "id": id }).to_string());
    let _ = hub().tx.send(msg);
}

/// 廣播世界上出現一個新交易攤（玩家自由市集 v1，自主提案切片 832）。不匿名——
/// 標明擺攤者姓名，讓路過的人知道是誰擺的（非驚喜巧遇，是明擺著的交易看板）。
#[allow(clippy::too_many_arguments)]
fn broadcast_stall_open(
    x: i32, y: i32, z: i32,
    give_item: u8, give_count: u32, want_item: u8, want_count: u32, owner: &str,
) {
    let msg = Arc::new(
        serde_json::json!({
            "t": "stall_open", "x": x, "y": y, "z": z,
            "give_item": give_item, "give_count": give_count,
            "want_item": want_item, "want_count": want_count, "owner": owner,
        })
        .to_string(),
    );
    let _ = hub().tx.send(msg);
}

/// 廣播一個交易攤從世界上消失（成交、收攤或逾時），所有人同步移除浮標（玩家自由市集 v1）。
fn broadcast_stall_removed(x: i32, y: i32, z: i32) {
    let msg = Arc::new(
        serde_json::json!({ "t": "stall_removed", "x": x, "y": y, "z": z }).to_string(),
    );
    let _ = hub().tx.send(msg);
}

/// 廣播一束乙太煙火的施放給全場（ROADMAP 785）：`(x,y,z)` 是施放者當前座標，前端據此
/// 在其頭頂夜空綻放火花；`palette` 是伺服器選定的配色盤索引（人人看到同色）。
fn broadcast_firework(x: f32, y: f32, z: f32, palette: u32) {
    let msg = Arc::new(
        serde_json::json!({ "t": "firework", "x": x, "y": y, "z": z, "palette": palette })
            .to_string(),
    );
    let _ = hub().tx.send(msg);
}

/// 廣播一座集會鐘被敲響給全場（集會鐘 v1）：`(x,y,z)` 是鐘方塊座標，前端據此播鐘聲音效／
/// 在鐘上冒一圈聲波漣漪；`ringer` 敲鐘者名、`count` 循聲聚來的居民數（供前端做提示）。
/// 只在真的召到居民時才呼叫（見 RingBell handler），避免空敲洗版全場。
fn broadcast_bell_ring(x: i32, y: i32, z: i32, ringer: &str, count: usize) {
    let msg = Arc::new(
        serde_json::json!({
            "t": "bell_ring", "x": x, "y": y, "z": z, "ringer": ringer, "count": count,
        })
        .to_string(),
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

/// 玩家生存指標 v1（溫和版）：把 PlayerStats 組成單播給玩家自己的 `player_stats` 訊息。
/// 只送血/飢/上限/是否餓瘋（前端據此顯示條＋餓瘋提示＋移動懲罰視覺）。
/// **後端權威**：這串永遠從伺服器狀態組出，客戶端無法自報。
fn player_stats_msg(s: &vstats::PlayerStats) -> String {
    serde_json::json!({
        "t": "player_stats",
        "health": s.health,
        "max_health": vstats::MAX_HEALTH,
        "hunger": s.hunger.round() as i32,
        "max_hunger": vstats::MAX_HUNGER as i32,
        "starving": s.is_starving(),
    })
    .to_string()
}

/// 對玩家套用傷害（溫和版·後端權威）：扣血、清飽食回血累加器（受傷打斷自癒）、
/// 廣播新指標；血歸零 → 觸發溫柔重生（滿血飢＋回床/廣場＋暖心提示，**背包不動**）。
///
/// - `bed` / `pick`：重生點選擇與提示輪替由呼叫端（per-connection）提供，血歸零時用。
/// - 回是否發生了重生（呼叫端可據此重設 fall 追蹤等）。
/// **鎖紀律**：短取 player_stats 寫鎖組出新值即釋，送訊息/持久化在鎖外。
async fn apply_player_damage(
    name: &str,
    dmg: u32,
    out_tx: &mpsc::Sender<Message>,
) {
    let new_stats = {
        let mut m = hub().player_stats.write().unwrap();
        let s = m.entry(name.to_string()).or_insert_with(vstats::PlayerStats::default);
        s.health = vstats::apply_damage(s.health, dmg);
        s.regen_acc = 0.0; // 受傷打斷自癒的節奏
        *s
    }; // 寫鎖釋放
    // 先送受傷（含扣血量，前端據此閃輕紅暈）＋新指標。
    let _ = out_tx.send(Message::Text(serde_json::json!({
        "t": "player_hurt", "damage": dmg
    }).to_string())).await;
    let _ = out_tx.send(Message::Text(player_stats_msg(&new_stats))).await;
    persist_player_stats(); // 血變動落地（登入玩家重登保留）
}

/// 邊陲營地探索 v1（自主提案切片，接續 881 立牌）：判定玩家此刻是否站在某位居民親手搭起的
/// 邊陲營地床邊。**與遺跡／溫泉的關鍵差異**：後兩者座標由世界生成種子決定（純座標函式，
/// 與玩家/居民狀態無關）；邊陲營地座標由該居民的**家座標**純函式算出（`vexp::outpost_seq`
/// → `pick_frontier` → `outpost_bed_center`，與 `voxel_ws.rs` 遠行狀態機算落點的手法完全
/// 一致），且只有小棚真的搭起（`Block::Bed` 已落地）才算數——半路経過空地不算發現。
/// 只掃能遠行的人格（`vexp::expedition_motive` 有值，目前 2 位），成本可忽略。
/// 找到就回傳 `(居民 id, 居民名, 床的世界座標)`；找不到回 `None`。
/// **鎖紀律**：residents 讀鎖只取一次快照即釋、deltas 讀鎖只取一次即釋，兩者不巢狀。
fn player_near_built_outpost(px: f32, pz: f32) -> Option<(String, &'static str, i32, i32, i32)> {
    let candidates: Vec<(String, &'static str, f32, f32)> = {
        let rs = hub().residents.read().unwrap();
        rs.iter()
            .filter(|r| vexp::expedition_motive(r.persona).is_some())
            .map(|r| (r.id.clone(), r.name, r.home_x, r.home_z))
            .collect()
    }; // residents 讀鎖釋放
    for (rid, rname, home_x, home_z) in candidates {
        let seq = vexp::outpost_seq(home_x, home_z);
        let (fx, fz, _bearing) = vexp::pick_frontier(home_x, home_z, seq);
        let (tx, tz) = (fx.round() as i32, fz.round() as i32);
        let (bedx, bedz) = vexp::outpost_bed_center(tx, tz);
        if !vexp::near_outpost_bed(px, pz, bedx, bedz) {
            continue;
        }
        let (ax, az) = vexp::shelter_anchor(tx, tz);
        let ay = vbuild::surface_y(ax, az);
        let built = {
            let _w = hub().deltas.read().unwrap();
            voxel::block_at(ax, ay, az) == Block::Bed
        }; // deltas 讀鎖釋放
        if built {
            return Some((rid, rname, ax, ay, az));
        }
    }
    None
}

/// 地標旅人留言 v1（自主提案切片，ROADMAP 862）：把這處地標目前的留言簿（可能是空的）
/// 單播給這位玩家，並附上 `(x,y,z)`——前端據此讓玩家能就地回送 `LeaveLandmarkNote`
/// 寫下自己的一句話（溫泉不看座標，附的是玩家此刻的腳下位置即可；遺跡則必須是乙太礦
/// 本身座標）。呼叫端只在「第一次發現這處地標」這種天然稀疏的時機呼叫（不會每 tick 狂推）。
async fn send_landmark_notes(
    kind: vdisc::LandmarkKind,
    dedup_key: (i32, i32),
    x: i32,
    y: i32,
    z: i32,
    out_tx: &mpsc::Sender<Message>,
) {
    let notes = hub().landmark_notes.read().unwrap().notes_at(kind, dedup_key);
    let payload: Vec<serde_json::Value> = notes
        .iter()
        .map(|n| serde_json::json!({ "player": n.player, "text": n.text }))
        .collect();
    let _ = out_tx.send(Message::Text(serde_json::json!({
        "t": "landmark_notes",
        "kind": kind.wire_id(),
        "x": x, "y": y, "z": z,
        "notes": payload,
    }).to_string())).await;
}

/// 個人路標 v1（自主提案切片，ROADMAP 869）：單播這位玩家目前完整的路標清單。
/// 呼叫端在 `SetWaypoint`/`RemoveWaypoint` 成功後各呼叫一次，讓前端面板即時更新。
async fn send_waypoints(player: &str, out_tx: &mpsc::Sender<Message>) {
    let items = hub().waypoints.read().unwrap().list(player);
    let payload: Vec<serde_json::Value> = items
        .iter()
        .map(|w| serde_json::json!({ "label": w.label, "x": w.x, "y": w.y, "z": w.z }))
        .collect();
    let _ = out_tx.send(Message::Text(serde_json::json!({
        "t": "waypoint_sync",
        "items": payload,
    }).to_string())).await;
}

/// 溫柔重生（療癒世界·血歸零時）：血飢回滿、傳送回床邊或村莊廣場、送暖心提示——
/// **背包不掉落**（本函式完全不碰 inventory）。畫面柔和淡出由前端收到 `respawn` 時處理。
///
/// - `bed`：這條連線最近睡過的床（優先）；沒有則回 `spawn_pos()`（村莊廣場）。
/// - `pick`：暖心提示輪替指標（確定性）。
/// **鎖紀律**：分別短取 player_stats／players 寫鎖即釋，送訊息在鎖外。
async fn do_gentle_respawn(
    my_id: Uuid,
    name: &str,
    bed: Option<(f32, f32, f32)>,
    pick: usize,
    out_tx: &mpsc::Sender<Message>,
) {
    // 1) 選重生點（床優先，否則廣場）。
    let (rx, ry, rz) = vstats::respawn_point(bed, spawn_pos());
    // 2) 血飢回滿（清所有累加器）。
    let full = {
        let mut m = hub().player_stats.write().unwrap();
        let s = m.entry(name.to_string()).or_insert_with(vstats::PlayerStats::default);
        *s = vstats::revived_stats();
        *s
    };
    // 3) 把權威位置也搬到重生點（別人看到你瞬移回村；也避免下一個 Move 從死亡點又掉一次）。
    {
        let mut players = hub().players.write().unwrap();
        if let Some(p) = players.get_mut(&my_id) {
            p.x = rx;
            p.y = ry;
            p.z = rz;
        }
    }
    broadcast_players(); // 讓所有人看到你回到了村莊
    // 4) 送 respawn（帶座標讓前端把相機/預測位置拉回＋柔和淡出）＋暖心提示＋新滿血指標。
    let _ = out_tx.send(Message::Text(serde_json::json!({
        "t": "respawn",
        "x": rx, "y": ry, "z": rz,
        "message": vstats::respawn_message(pick),
    }).to_string())).await;
    let _ = out_tx.send(Message::Text(player_stats_msg(&full))).await;
    persist_player_stats();
    vfeed::append_feed("重生", name, "在溫暖的爐火邊醒來，重新出發。");
}

/// 睡覺／溫柔重生時把血飢回滿並廣播（療癒世界）。短鎖即釋、送訊息在鎖外。
async fn heal_to_full_on_sleep(name: &str, out_tx: &mpsc::Sender<Message>) {
    let full = {
        let mut m = hub().player_stats.write().unwrap();
        let s = m.entry(name.to_string()).or_insert_with(vstats::PlayerStats::default);
        *s = vstats::revived_stats();
        *s
    };
    let _ = out_tx.send(Message::Text(player_stats_msg(&full))).await;
    persist_player_stats();
}

/// 依玩家頭部（腳底 + EYE_HEIGHT）採樣的方塊，判定頭是否泡在水裡（溺水判定用）。
/// 短取 delta 讀鎖即釋，不巢狀。
fn head_in_water(x: f32, y: f32, z: f32) -> bool {
    let hx = x.floor() as i32;
    let hy = (y + crate::voxel::EYE_HEIGHT).floor() as i32;
    let hz = z.floor() as i32;
    let blk = voxel::effective_block_at(&hub().deltas.read().unwrap(), hx, hy, hz);
    vfish::is_water_block(blk as u8)
}

/// 溫泉遺跡 v1（世界第二種可探索地標，自主提案切片）：玩家腳下是否正踩進溫泉水裡。
/// 與 [`head_in_water`]（溺水判定，看頭部是否沒頂）刻意分開——泡溫泉看腳底這格，
/// 站進去就算，不必整個人沒頂（溫泉池本就淺，不會也不該讓人溺水）。
/// 短取 delta 讀鎖即釋，不巢狀。
fn feet_in_hot_spring(x: f32, y: f32, z: f32) -> bool {
    let hx = x.floor() as i32;
    let hy = y.floor() as i32;
    let hz = z.floor() as i32;
    voxel::effective_block_at(&hub().deltas.read().unwrap(), hx, hy, hz) == Block::HotSpringWater
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

    // 治安三件套①：解出這條連線的真實 client IP，供 per-IP 對話限流用（跨連線天花板）。
    // Cloudflare tunnel 後真實 IP 在 `cf-connecting-ip`（退而求其次 `x-forwarded-for` 取首段）。
    // 與既有建議箱 per-IP 限流（main.rs `post_suggestion`）同一套取法；解不出則歸一個
    // 保底桶 "unknown"（本機/無標頭時所有連線共用一桶，只影響 QA、不影響 prod CF 流量）。
    let client_ip: String = headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    // 與主 ws 一致的安全硬化：訊息上限 64 KiB（任何合法 voxel 訊息都遠小於此；
    // chunk 是「伺服器送出」不受此限）。
    const WS_MAX_MSG_BYTES: usize = 64 * 1024;
    ws.max_message_size(WS_MAX_MSG_BYTES)
        .max_frame_size(WS_MAX_MSG_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, account_name, account_email, client_ip))
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

/// 同帳號去重：在 `players` 寫鎖內呼叫——搜出同 email 的舊 entry、移除並回傳其 UUID。
/// 訪客（`email = ""`）或找不到舊 entry → 回 `None`，不動 `players`。
///
/// **安全**：`email` 必須由後端 cookie→users 解出（呼叫方保證），不信客戶端自報——
/// 只踢「同一真實帳號」的舊連線，不可能跨帳號踢別人。
///
/// **鎖紀律**：此函式本身不取任何鎖，由呼叫方在 `players` 寫鎖內呼叫。
/// 回傳 old_id 後，呼叫方在**鎖外**再取 conn_kick 鎖送踢信號，不巢狀（守死鎖鐵律）。
fn remove_duplicate_account(
    players: &mut HashMap<Uuid, VoxelPlayer>,
    email: &str,
) -> Option<Uuid> {
    let old_id = players
        .iter()
        .find(|(_, p)| p.account.as_deref() == Some(email))
        .map(|(id, _)| *id)?;
    players.remove(&old_id);
    Some(old_id)
}

async fn handle_socket(
    socket: WebSocket,
    account_name: Option<String>,
    account_email: Option<String>,
    client_ip: String,
) {
    let (mut sender, mut receiver) = socket.split();
    let my_id = Uuid::new_v4();

    // 治安三件套②·連線數上限：同一真實 IP 只准這麼多條同時連線（預設 5，env 可調），擋
    // 「開幾十條連線並行灌 talk 繞過 per-connection 冷卻」。白名單（localhost / QA / env）豁免，
    // 讓本機冒煙與隔離測試不受限。超上限 → 直接關這條連線（連 Join 都不讀，最省資源）。
    // 短鎖即釋、不 await（守鎖紀律）。豁免連線不佔名額、也不需 release（一致性由 exempt 判定守住）。
    let conn_exempt = ip_is_exempt(&client_ip);
    if !conn_exempt {
        let acquired = ip_conn_limiter()
            .lock()
            .unwrap()
            .try_acquire(&client_ip, max_conn_per_ip());
        if !acquired {
            // 名額已滿：不建 writer、不進場，直接讓 socket 隨函式結束而關閉。
            return;
        }
    }
    // 名額守衛：確保任何離開路徑（早退 / 正常收攤）都會釋放這條連線的名額（豁免者不佔名額）。
    let _conn_guard = ConnSlotGuard {
        ip: client_ip.clone(),
        active: !conn_exempt,
    };

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
        // 連線一開始就斷/非文字 → 收攤（_conn_guard 於函式結束時自動釋放名額）。
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

    // 同帳號去重 v1（幽靈分身修復，ROADMAP fix #1021 重連副作用）：
    // 建立這條連線的踢信號通道；之後若有同帳號的第二條連線進來，它會把信號送過來，
    // 讓我們的 select! 觸發、優雅退出，幽靈分身自然消失。
    // 安全：email 由後端 cookie→users 解出（權威），只踢同一真實帳號的舊連線，不能跨帳號踢別人。
    let (kick_tx, mut kick_rx) = oneshot::channel::<()>();

    // 建立權威玩家、登錄進 hub；同帳號去重在同一把 players 寫鎖內完成（原子性）。
    let old_id_to_kick: Option<Uuid> = {
        let mut players = hub().players.write().unwrap();
        // 登入帳號才做去重（訪客 account_email = None → 多條訪客連線不互踢）。
        let old_id = account_email
            .as_deref()
            .and_then(|email| remove_duplicate_account(&mut players, email));
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
                account: account_email.clone(), // 後端解出，不廣播，僅去重用
                held: None,
            },
        );
        old_id
    }; // players 寫鎖在此釋放

    // 鎖外操作：更新踢信號表、送踢信號給舊連線（守 prod 死鎖鐵律：不持鎖 await）。
    {
        let mut conn_kick = hub().conn_kick.write().unwrap();
        // 若同帳號的舊連線還在，取出其踢信號發送端（同時從表移除）。
        let old_kick_tx = old_id_to_kick.and_then(|oid| conn_kick.remove(&oid));
        // 登記自己的踢信號（供將來同帳號的下一條連線使用）。
        conn_kick.insert(my_id, kick_tx);
        // 送踢信號——鎖已釋放，send() 不在鎖內，守鐵律。
        if let Some(tx) = old_kick_tx {
            let _ = tx.send(()); // 舊連線的 select! 收到後優雅退出
        }
    } // conn_kick 寫鎖在此釋放

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

    // 玩家生存指標 v1（溫和版）：登入玩家從存檔載回血/飢（重登保留，比照 #1024）；
    // 訪客／首次登入 → 預設滿血滿飢。取得後單播 player_stats 給玩家自己（別人看不到，減噪）。
    // **後端權威**：這裡從伺服器狀態組出，客戶端只顯示、不自報。
    {
        let stats = {
            let mut m = hub().player_stats.write().unwrap();
            *m.entry(name.clone()).or_insert_with(vstats::PlayerStats::default)
        }; // 寫鎖釋放
        let msg = player_stats_msg(&stats);
        if out_tx.send(Message::Text(msg)).await.is_err() {
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

    // 漂流瓶 v1（自主提案切片 825）：連線時一次送出世界上所有尚未被撿走的瓶子座標，
    // 讓前端立刻掛回浮標——但**只送座標，絕不送內文**（內文只有真的撿起來才讀得到）。
    {
        let positions = hub().bottle.read().unwrap().all_positions();
        let bottles: Vec<serde_json::Value> = positions
            .iter()
            .filter_map(|pos| {
                let mut it = pos.split(',');
                let bx = it.next()?.parse::<i32>().ok()?;
                let by = it.next()?.parse::<i32>().ok()?;
                let bz = it.next()?.parse::<i32>().ok()?;
                Some(serde_json::json!({ "x": bx, "y": by, "z": bz }))
            })
            .collect();
        let bottle_sync = serde_json::json!({ "t": "bottle_sync", "bottles": bottles }).to_string();
        if out_tx.send(Message::Text(bottle_sync)).await.is_err() {
            cleanup(my_id, &writer);
            return;
        }
    }

    // 掉落物 v1（自主提案切片 828）：連線時一次送出世界上所有還沒被撿走的掉落物，
    // 讓前端立刻掛回浮標。
    {
        let items: Vec<serde_json::Value> = hub()
            .drops
            .read()
            .unwrap()
            .all()
            .iter()
            .map(|d| {
                serde_json::json!({
                    "id": d.id, "x": d.x, "y": d.y, "z": d.z,
                    "item_id": d.item_id, "count": d.count, "dropped_by": d.dropped_by,
                })
            })
            .collect();
        let drop_sync = serde_json::json!({ "t": "drop_sync", "items": items }).to_string();
        if out_tx.send(Message::Text(drop_sync)).await.is_err() {
            cleanup(my_id, &writer);
            return;
        }
    }

    // 玩家自由市集 v1（自主提案切片 832）：連線時一次送出世界上所有還在等人接手的攤位。
    {
        let stalls: Vec<serde_json::Value> = hub()
            .stalls
            .read()
            .unwrap()
            .all()
            .iter()
            .map(|s| {
                serde_json::json!({
                    "x": s.x, "y": s.y, "z": s.z,
                    "give_item": s.give_item, "give_count": s.give_count,
                    "want_item": s.want_item, "want_count": s.want_count, "owner": s.owner,
                })
            })
            .collect();
        let stall_sync = serde_json::json!({ "t": "stall_sync", "stalls": stalls }).to_string();
        if out_tx.send(Message::Text(stall_sync)).await.is_err() {
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

    // 自由市集賣家通知 v1（自主提案切片，ROADMAP 864）：連線時投遞這位玩家的待送達成交通知——
    // 不看離線多久（跟久別重逢摘要的取樣窗獨立）、只要有成交過就送，送達後立刻清空這位賣家的佇列。
    if !name.is_empty() {
        let notices = {
            hub().stall_notices.write().unwrap().remove(&name).unwrap_or_default()
        }; // stall_notices 寫鎖釋放
        if let Some(msg) = vstallnotify::format_notice_message(&notices) {
            let stall_sold =
                serde_json::json!({ "t": "stall_sold_notice", "text": msg }).to_string();
            let _ = out_tx.send(Message::Text(stall_sold)).await;
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
    // 內容審查累犯罰則（治安三件套①）：反覆命中審查者，這條連線被推遲到此刻後才准再對話。
    // None＝目前無罰則（正常玩家永遠是 None，零感知）。
    let mut talk_penalty_until: Option<std::time::Instant> = None;

    // 位置持久化 v1：上次存位置的 unix 秒（0 = 從未存；第一次 Move 後 30 秒內觸發第一次存）。
    let mut last_pos_save_ts: u64 = 0;

    // 跌落傷害 v1（玩家生存指標·溫和版）：per-connection 追「這一趟下墜的最高點」。
    // Move 是前端權威預測、逐幀上報，故用「y 上升＝離地/爬升 → 記新峰值；y 停止下降＝落地
    // → 用（峰值 − 落地 y）算落差」偵測著陸事件。落地一結算就清峰值，等下一次起跳/下墜。
    // `None` = 尚未開始追（剛進場或剛落地）。純記憶體、per-player、斷線即清。
    let mut fall_peak_y: Option<f32> = None;
    let mut last_move_y: f32 = f32::NAN;

    // 溫柔重生 v1（玩家生存指標·溫和版）：這條連線最近睡過的床座標（站上去的位置）。
    // 血歸零時優先在床邊醒來，沒床則回村莊廣場（spawn_pos）。per-connection、斷線即清。
    let mut last_bed: Option<(f32, f32, f32)> = None;
    // 溺水扣血跨 tick 的本地累加器（伺服器 tick 在別的 task，故溺水在這條連線收 Move 時
    // 順帶推進——但用玩家 stats 內的 drown 累加器保存，這裡不另存；見 apply/tick 呼叫）。
    // 重生提示輪替指標（確定性、不走 random）。
    let mut respawn_pick: usize = 0;

    // 居民會注意到你親手蓋的東西 v1（773）：這條連線的「建造連段」（連續放置的塊數＋
    // 上一塊位置與時刻，見 voxel_admire）＋這條連線對每位居民的讚賞冷卻（per-connection，
    // 天然 per-player、零跨連線鎖；斷線即清、無持久化需求）。
    let mut build_streak: Option<vadmire::BuildStreak> = None;
    let mut admire_cd: std::collections::HashMap<String, std::time::Instant> =
        std::collections::HashMap::new();

    // 居民注意到你悉心照料的農地 v1（自主提案切片）：這條連線的「農忙連段」（連續翻土/
    // 播種的次數＋上一次位置與時刻，見 voxel_farm_admire）＋這條連線對每位居民的讚賞冷卻——
    // 與上面 773 建造讚賞的 build_streak/admire_cd 完全獨立，各自連段、各自冷卻，
    // 互不干擾（per-connection，天然 per-player、零跨連線鎖；斷線即清、無持久化需求）。
    let mut farm_streak: Option<vfarmadmire::FarmStreak> = None;
    let mut farm_admire_cd: std::collections::HashMap<String, std::time::Instant> =
        std::collections::HashMap::new();

    // 親手煮的暖食自己也能享用 v1（779）：這條連線上次「感染附近居民」的時刻（per-connection
    // 冷卻，天然 per-player、零跨連線鎖；斷線即清、無持久化需求）。吃東西本身不受此限，只有
    // 「附近居民被你的滿足感染」這一拍受節流，防囤糧狂吃洗版居民泡泡 / 動態牆。
    let mut last_eat_share: Option<std::time::Instant> = None;

    // 乙太煙火 v1（785）：這條連線上次施放煙火的時刻（per-connection 冷卻，天然 per-player、
    // 零跨連線鎖；斷線即清）。施放會廣播給全場，此冷卻擋連放洗爆所有人畫面（濫用防護①）。
    let mut last_firework: Option<std::time::Instant> = None;

    // 溫泉遺跡 v1（世界第二種可探索地標，自主提案切片）：這條連線上一 tick 是否正泡在溫泉裡，
    // 用來偵測「剛踏進去」的那一刻只提示一次（per-connection、斷線即清，不必持久化）。
    let mut was_soaking = false;

    // 邊陲營地探索 v1（自主提案切片，接續 881）：這條連線上一 tick 是否正站在某位居民的邊陲
    // 營地床邊，用來偵測「剛走近」的那一刻只嘗試記一次探索紀事（避免逗留原地時每秒都取一次
    // discovery 寫鎖；record() 本身已冪等，此旗標純粹省一趟鎖，非正確性必要）。
    let mut was_near_outpost = false;

    // 玩家生存指標 tick（溫和版）：per-connection 每秒推進一次飢餓衰減／溺水／飽食回血，
    // 並在指標變動時單播 player_stats（只給玩家自己，減噪）。放這條連線的 select loop 裡跑，
    // 天然拿得到 out_tx／位置／床，避免居民 tick 那條 task 沒有 per-connection 送訊息管道。
    // 1 Hz 夠：療癒節奏本就慢（飢餓幾十分鐘見底），也省廣播頻寬。
    const STATS_TICK_DT: f32 = 1.0;
    let mut stats_ticker = tokio::time::interval(std::time::Duration::from_secs_f32(STATS_TICK_DT));
    stats_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // 讀取迴圈：處理 move / req / break / place / talk。
    // 同時監聽同帳號去重的踢信號（kick_rx）：新連線進來時 send(())，此 arm 觸發後 break，
    // 優雅退出——幽靈分身從 players 表消失，廣播讓所有人看到它不見。
    loop {
        let msg = tokio::select! {
            biased; // 踢信號優先（低頻但時效性高），避免 select 平等隨機時延遲踢除
            _ = &mut kick_rx => break, // 被同帳號新連線踢出，退出即結束
            _ = stats_ticker.tick() => {
                // ── 玩家生存指標 tick（溫和版·後端權威）──────────────────────────
                // 取當前位置（判定頭是否在水裡）。取不到（剛斷線）就跳過這 tick。
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                let in_water = head_in_water(px, py, pz);
                // 溫泉遺跡 v1（自主提案切片）：泡在溫泉裡 → 門檻寬鬆得多、節奏更快地回血，飢餓消耗打折。
                let soaking = feet_in_hot_spring(px, py, pz);
                // 短取寫鎖：推進飢餓衰減／溺水累加／飽食回血，算出這 tick 的傷害與回血。
                let (new_stats, drown_dmg, heal, changed) = {
                    let mut m = hub().player_stats.write().unwrap();
                    let s = m.entry(name.clone()).or_insert_with(vstats::PlayerStats::default);
                    let before = *s;
                    // 飢餓慢慢降（泡溫泉時打折）。
                    s.hunger = vstats::decay_hunger_soaking(s.hunger, STATS_TICK_DT, soaking);
                    // 溺水：頭在水中久了扣血（有緩衝，離水即歸零）。
                    let (da, ta, ddmg) = vstats::tick_drown(in_water, s.drown_acc, s.drown_tick_acc, STATS_TICK_DT);
                    s.drown_acc = da;
                    s.drown_tick_acc = ta;
                    // 飽食回血（泡溫泉時門檻寬鬆、回得更快）。
                    let (ra, h) = vstats::tick_regen_soaking(s.hunger, s.health, s.regen_acc, STATS_TICK_DT, soaking);
                    s.regen_acc = ra;
                    if h > 0 { s.health = (s.health + h).min(vstats::MAX_HEALTH); }
                    let changed = *s != before;
                    (*s, ddmg, h, changed)
                }; // 寫鎖釋放
                let _ = heal;
                // 溫泉遺跡 v1：剛踏進去那一刻單播一句暖意提示（只在「上一 tick 沒泡、這一 tick 泡了」時提一次）。
                if soaking && !was_soaking {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({
                            "t": "hot_spring_enter",
                            "line": "暖流環繞全身，泡進溫泉舒服多了～"
                        })
                        .to_string(),
                    ));
                    // 探索紀事 v1（自主提案切片，接續 839）：溫泉可反覆進出，用所屬格子座標當
                    // 穩定去重鍵，同一玩家對同一泓溫泉只記第一次踏進去的那一拍。
                    try_unlock_milestone(&name, "first_hotspring", &out_tx);
                    let cell = voxel::hot_spring_cell_of(px.floor() as i32, pz.floor() as i32);
                    let (ix, iy, iz) = (px.floor() as i32, py.floor() as i32, pz.floor() as i32);
                    let found = {
                        let mut d = hub().discovery.write().unwrap();
                        d.record(&name, vdisc::LandmarkKind::HotSpring, cell, ix, iy, iz)
                    }; // discovery 寫鎖釋放
                    if let Some(entry) = found {
                        vdisc::append_discovery(&entry);
                        // 地標旅人留言 v1（自主提案切片，ROADMAP 862）：第一次踏進這泓溫泉時，
                        // 順手看看先前旅人留下的話（若有）——只在真的有人留過話時才推，避免空洗版。
                        send_landmark_notes(vdisc::LandmarkKind::HotSpring, cell, ix, iy, iz, &out_tx).await;
                    }
                }
                was_soaking = soaking;
                // 邊陲營地探索 v1（自主提案切片，接續 881 立牌）：881 讓居民自己在營地立牌、登記
                // 進地標系統（居民視角），但玩家視角走到同一處荒野據點，此前完全不會被世界記住——
                // 探索紀事／旅人留言簿只認遺跡與溫泉。這一刀讓玩家第一次走近某位居民親手搭起的
                // 邊陲營地床邊時，也留下一筆探索紀事、解鎖里程碑、順手看看先前旅人的留言。
                let near_outpost = player_near_built_outpost(px, pz);
                if let Some((rid, rname, ax, ay, az)) = &near_outpost {
                    if !was_near_outpost {
                        try_unlock_milestone(&name, "first_outpost_discover", &out_tx);
                        let found = {
                            let mut d = hub().discovery.write().unwrap();
                            d.record(&name, vdisc::LandmarkKind::Outpost, (*ax, *az), *ax, *ay, *az)
                        }; // discovery 寫鎖釋放
                        if let Some(entry) = found {
                            vdisc::append_discovery(&entry);
                            send_landmark_notes(vdisc::LandmarkKind::Outpost, (*ax, *az), *ax, *ay, *az, &out_tx).await;
                            vfeed::append_feed(
                                "探索",
                                &name,
                                &format!("走進荒野，找到了{rname}親手搭起的邊陲營地"),
                            );
                        }
                    }
                }
                was_near_outpost = near_outpost.is_some();
                // 溺水扣血走統一傷害路徑（含死亡→重生判定、廣播、持久化）。
                if drown_dmg > 0 {
                    apply_player_damage(&name, drown_dmg, &out_tx).await;
                } else if changed {
                    // 只是飢餓/回血變動：單播新指標（若剛好血歸零由下方重生統一處理）。
                    let _ = out_tx.send(Message::Text(player_stats_msg(&new_stats))).await;
                }
                // 死亡 → 溫柔重生（血歸零。fall/eat 路徑扣完血也會走到這條 tick 檢查）。
                let is_down = { hub().player_stats.read().unwrap().get(&name).map(|s| s.is_down()).unwrap_or(false) };
                if is_down {
                    do_gentle_respawn(my_id, &name, last_bed, respawn_pick, &out_tx).await;
                    respawn_pick = respawn_pick.wrapping_add(1);
                    fall_peak_y = None;
                    last_move_y = f32::NAN;
                }
                continue;
            },
            v = receiver.next() => match v {
                Some(Ok(m)) => m,
                _ => break, // WebSocket 斷開或錯誤，正常離線
            },
        };
        let txt = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            // ping/pong/binary 一律忽略（切片①只用文字 JSON）。
            _ => continue,
        };
        match serde_json::from_str::<ClientMsg>(&txt) {
            Ok(ClientMsg::Move { x, y, z, yaw, held }) => {
                let changed = {
                    let mut players = hub().players.write().unwrap();
                    if let Some(p) = players.get_mut(&my_id) {
                        p.x = x;
                        p.y = y;
                        p.z = z;
                        p.yaw = yaw;
                        p.held = held;
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    broadcast_players();
                    // 掉落物 v1：走近自動撿起，含撿回自己剛丟下的東西（自主提案切片 828）。
                    let picked = {
                        let mut store = hub().drops.write().unwrap();
                        store.nearest_in_range(x, y, z).and_then(|id| store.remove(id))
                    };
                    if let Some(item) = picked {
                        let entry = hub().inventory.write().unwrap().give(&name, item.item_id, item.count);
                        vinv::append_inv(&entry);
                        let nc = hub().inventory.read().unwrap().count(&name, item.item_id);
                        let _ = out_tx.send(Message::Text(serde_json::json!({
                            "t": "inv_update", "block_id": item.item_id, "count": nc
                        }).to_string())).await;
                        broadcast_item_removed(item.id);
                    }
                }
                // 跌落傷害 v1（溫和版）：偵測著陸事件並結算落差。
                // 追蹤這趟下墜的最高點：y 上升 → 更新峰值（起跳/爬升/被推高）；
                // y 由降轉平/升 → 視為落地，用（峰值 − 落地 y）算落差 → fall_damage。
                // **後端權威**：落差由伺服器收到的位置序列推算，客戶端不自報傷害。
                if !last_move_y.is_nan() {
                    let dy = y - last_move_y;
                    if dy > 0.05 {
                        // 上升中：重設/抬高峰值（起跳或被地形推高，開始新一段可能的下墜）。
                        fall_peak_y = Some(fall_peak_y.map_or(y, |p| p.max(y)));
                    } else if dy < -0.02 {
                        // 下降中：確保有在追峰值（若之前是平移才開始下墜，用上一點當峰值）。
                        fall_peak_y = Some(fall_peak_y.unwrap_or(last_move_y));
                    } else {
                        // 幾乎沒垂直位移（落地站定/平地走）：若剛從高處掉下 → 結算落差。
                        if let Some(peak) = fall_peak_y.take() {
                            let fall = peak - y;
                            let dmg = vstats::fall_damage(fall);
                            if dmg > 0 {
                                apply_player_damage(&name, dmg, &out_tx).await;
                                // 這一摔若讓血歸零 → 立刻溫柔重生（別等下一個 stats tick）。
                                let is_down = { hub().player_stats.read().unwrap().get(&name).map(|s| s.is_down()).unwrap_or(false) };
                                if is_down {
                                    do_gentle_respawn(my_id, &name, last_bed, respawn_pick, &out_tx).await;
                                    respawn_pick = respawn_pick.wrapping_add(1);
                                    last_move_y = f32::NAN;
                                }
                            }
                        }
                    }
                }
                last_move_y = y;

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
            Ok(ClientMsg::Break { x, y, z, tool }) => {
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
                    // 探索紀事 v1（自主提案切片，接續 838）：剛挖掉的這一格恰是遺跡柱頂裸露的
                    // 乙太礦——固定位置、挖掉即成空氣不會再生，本身就是天然、不必額外去重的
                    // 「發現一處新遺跡」信號。
                    if matches!(target_block, Block::AetherOre) && voxel::ruin_ore_at(x, y, z) {
                        try_unlock_milestone(&name, "first_ruin", &out_tx);
                        let found = {
                            let mut d = hub().discovery.write().unwrap();
                            d.record(&name, vdisc::LandmarkKind::Ruin, (x, z), x, y, z)
                        }; // discovery 寫鎖釋放
                        if let Some(entry) = found {
                            vdisc::append_discovery(&entry);
                            // 地標旅人留言 v1（自主提案切片，ROADMAP 862）：第一次挖到這處遺跡時，
                            // 順手看看先前旅人留下的話（若有）。
                            send_landmark_notes(vdisc::LandmarkKind::Ruin, (x, z), x, y, z, &out_tx).await;
                        }
                    }
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
                    // 乙太營火 v1：破壞營火 → 從取暖清單移除該座標，並持久化這格已成空氣
                    //（append world_blocks 覆蓋 Air，重啟 replay 後不會又冒出舊火堆）。火堆方塊
                    // 本身走後面「其餘實心方塊掉落自身」的通用路徑退還背包(70)，不在此 continue。
                    if matches!(target_block, Block::Campfire) {
                        hub()
                            .campfires
                            .write()
                            .unwrap()
                            .retain(|&(cx, cy, cz)| !(cx == x && cy == y && cz == z));
                        vbuild::append_world_block(x, y, z, Block::Air as u8);
                    }
                    // 集會鐘 v1：破壞鐘 → 持久化這格已成空氣（重啟 replay 後不再冒出舊鐘）。鐘方塊
                    // 本身走後面「其餘實心方塊掉落自身」的通用路徑退還背包(74)，不在此 continue。
                    // 應召狀態只是 in-memory：鐘沒了，正朝它走的居民逾時（SUMMON_TIMEOUT）自然放棄，不必特別清。
                    if matches!(target_block, Block::Bell) {
                        vbuild::append_world_block(x, y, z, Block::Air as u8);
                    }
                    // 木長椅 v1：破壞長椅 → 從歇腳清單移除該座標，並持久化這格已成空氣
                    //（重啟 replay 後不再冒出舊長椅）。椅方塊本身走後面「其餘實心方塊掉落自身」的
                    // 通用路徑退還背包(79)，不在此 continue。
                    if matches!(target_block, Block::Bench) {
                        hub()
                            .benches
                            .write()
                            .unwrap()
                            .retain(|&(cx, cy, cz)| !(cx == x && cy == y && cz == z));
                        vbuild::append_world_block(x, y, z, Block::Air as u8);
                    }
                    // 莓果叢 v1（ROADMAP 806）：採收「結果的莓果叢」→ **不消失**，就地回退成
                    // 莓果叢苗(75) ＋ 重啟結果計時（多年生：採過還會再結，不必重種）。破壞流程上面
                    // 已把這格設為空氣並廣播；這裡緊接著再放回莓果叢苗、廣播、重新登記計時。莓果
                    // 本身走下面 is_solid 的 drops 特殊掉落規則給予（BerryBushRipe → 莓果×2）。
                    if matches!(target_block, Block::BerryBushRipe) {
                        {
                            let mut world = hub().deltas.write().unwrap();
                            voxel::set_block(&mut world, x, y, z, Block::BerryBush);
                        } // delta 寫鎖即釋
                        broadcast_block(x, y, z, Block::BerryBush);
                        hub().berry.write().unwrap().plant(x, y, z, vfarm::now_secs());
                    }
                    // 莓果叢 v1：挖除「未結果的莓果叢苗」→ 清掉 berry 計時記錄（避免下輪 tick_berry
                    // 在空格憑空結果）。莓果叢苗方塊本身走下面「其餘實心方塊掉落自身」通用路徑退還背包(75)。
                    if matches!(target_block, Block::BerryBush) {
                        hub().berry.write().unwrap().remove(x, y, z);
                    }
                    // 雞舍生蛋 v1：收下「有蛋的雞舍」→ **不消失**，就地回退成空雞舍(80) ＋ 重啟生蛋計時
                    //（可反覆收成，不必重蓋）。破壞流程上面已把這格設為空氣並廣播；這裡緊接著再放回
                    // 空雞舍、廣播、重新登記計時。蛋本身走下面 is_solid 的 drops 特殊掉落規則給予。
                    if matches!(target_block, Block::CoopReady) {
                        {
                            let mut world = hub().deltas.write().unwrap();
                            voxel::set_block(&mut world, x, y, z, Block::Coop);
                        } // delta 寫鎖即釋
                        broadcast_block(x, y, z, Block::Coop);
                        hub().coop.write().unwrap().plant(x, y, z, vfarm::now_secs());
                    }
                    // 雞舍生蛋 v1：拆除「空雞舍」（尚未生蛋）→ 清掉 coop 計時記錄（避免下輪 tick_coop
                    // 在空格憑空生蛋）。空雞舍方塊本身走下面「其餘實心方塊掉落自身」通用路徑退還背包(80)。
                    if matches!(target_block, Block::Coop) {
                        hub().coop.write().unwrap().remove(x, y, z);
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
                            // 莓果叢 v1（ROADMAP 806）：採收結果的莓果叢 → 莓果×2（叢本身上面已就地回退成
                            // 莓果叢苗、不掉落方塊）。未結果的莓果叢苗(75) 不在此表 → 走 else 掉落自身退還。
                            Block::BerryBushRipe   => &[(vberry::BERRY_ID, vberry::BERRY_YIELD)],
                            // 雞舍生蛋 v1：收下有蛋的雞舍 → 蛋×1（雞舍本身上面已就地回退成空雞舍、
                            // 不掉落方塊）。空雞舍(80) 不在此表 → 走 else 掉落自身退還。
                            Block::CoopReady       => &[(vcoop::EGG_ID, vcoop::EGG_YIELD)],
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

                        // 時令豐收 v1（ROADMAP 812）：收割「當季時令」的成熟作物 → 額外多得一份果實
                        // （附加掉落，比照上面 738 砍葉附加掉樹苗的慣例）。與 811「種在時令長得快」對成
                        // 「種在時令長得快／收在時令收得多」一對。只獎不罰：非時令照常收成、不減產。
                        // season 讀鎖即釋、不巢狀（delta 寫鎖此處已釋放，循序取鎖、守死鎖鐵律）。
                        if let Some(kind) = vbounty::crop_kind_of_mature_block(target_block) {
                            let season = {
                                let day = hub().world_time.read().unwrap().days_elapsed();
                                vseason::season_for_day(day)
                            };
                            let extra = vbounty::harvest_bonus(kind, season);
                            if extra > 0 {
                                let cid = vbounty::crop_item_id(kind);
                                let entry = hub().inventory.write().unwrap().give(&name, cid, extra);
                                vinv::append_inv(&entry);
                                let new_count = hub().inventory.read().unwrap().count(&name, cid);
                                let _ = out_tx.try_send(Message::Text(
                                    serde_json::json!({
                                        "t": "inv_update", "block_id": cid, "count": new_count
                                    })
                                    .to_string(),
                                ));
                                // 當季鮮採的暖回饋（前端 toast；欄位 additive，舊客戶端忽略即可）。
                                let _ = out_tx.try_send(Message::Text(
                                    serde_json::json!({
                                        "t": "bounty",
                                        "line": vbounty::bounty_line(kind, season),
                                        "block_id": cid
                                    })
                                    .to_string(),
                                ));
                            }

                            // 玩家熟練度 v1（自主提案切片，ROADMAP 842）：收割成熟作物累積🌾耕種
                            // 熟練度；練到 Lv.5 起額外多收一份剛收成的作物（與上面的時令豐收各自
                            // 獨立判斷、不互相取代，兩者可同時觸發）。
                            let level = award_mastery(&name, MasteryKind::Farming, &out_tx);
                            give_mastery_bonus(
                                &name,
                                MasteryKind::Farming,
                                level,
                                vbounty::crop_item_id(kind),
                                &out_tx,
                            );
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

                        // 工欲善其事 v1（ROADMAP 790）：手持「對的」工具採集對應的天然方塊時，
                        // 有機率多掉一份該材料——鎬採石/礦、斧砍原木、鏟挖泥沙，階級越高機率越大。
                        // 「附加」掉落、不影響基礎掉落，向後相容。
                        // 濫用防護：前端自報手持工具 id，這裡**必查背包確認真持有該工具**才給加成，
                        // 防偽報白嫖；機率骰取真隨機（比照上方樹苗/垂釣稀有度慣例），純函式常數可測。
                        if let Some(tid) = tool {
                            let owns_tool = hub().inventory.read().unwrap().count(&name, tid) >= 1;
                            if owns_tool {
                                if let Some((bonus_id, bonus_cnt)) =
                                    vtool::tool_bonus_drop(tid, target_block, rand::random::<f32>())
                                {
                                    let entry =
                                        hub().inventory.write().unwrap().give(&name, bonus_id, bonus_cnt);
                                    vinv::append_inv(&entry);
                                    let new_count =
                                        hub().inventory.read().unwrap().count(&name, bonus_id);
                                    let _ = out_tx.try_send(Message::Text(
                                        serde_json::json!({
                                            "t": "inv_update",
                                            "block_id": bonus_id,
                                            "count": new_count
                                        })
                                        .to_string(),
                                    ));
                                    // 告訴前端這是「工具加成」的多收，讓它跳一句小回饋。
                                    let _ = out_tx.try_send(Message::Text(
                                        serde_json::json!({
                                            "t": "tool_bonus",
                                            "block_id": bonus_id,
                                            "count": bonus_cnt
                                        })
                                        .to_string(),
                                    ));
                                }
                            }
                        }

                        // 並肩協作 v1（自主提案切片 827）：挖天然方塊時，附近若有其他真人玩家
                        // 一起忙活，默契讓這塊多掉一點——玩家↔玩家至今唯一互動是漂流瓶（825，
                        // 非同步/匿名/一次性），本刀補上第一個即時/同步協作。只認天然採集方塊
                        // （沿用 790 工具加成同一張適配表，不重立表，也不與工具/時令加成疊算判斷
                        // 順序衝突——三者皆各自獨立附加，互不影響）。
                        // 濫用防護：同伴數由伺服器讀既有 players map 權威判定（非客戶端自報）；
                        // 只是額外一份既有材料、不觸發 LLM、封頂 MAX_PARTNERS，無經濟破壞面。
                        if vcoop_gather::coop_eligible_block(target_block) {
                            let others: Vec<(f32, f32, f32)> = hub()
                                .players
                                .read()
                                .unwrap()
                                .values()
                                .filter(|p| p.id != my_id)
                                .map(|p| (p.x, p.y, p.z))
                                .collect();
                            let partners = vcoop_gather::count_partners((px, py, pz), &others);
                            let bonus = vcoop_gather::coop_yield_bonus(partners);
                            if bonus > 0 {
                                // 玩家里程碑（自主提案切片，追上 827）：第一次因協作多得一份。
                                try_unlock_milestone(&name, "first_coop", &out_tx);
                                let bid = target_block as u8;
                                let entry = hub().inventory.write().unwrap().give(&name, bid, bonus);
                                vinv::append_inv(&entry);
                                let new_count = hub().inventory.read().unwrap().count(&name, bid);
                                let _ = out_tx.try_send(Message::Text(
                                    serde_json::json!({
                                        "t": "inv_update",
                                        "block_id": bid,
                                        "count": new_count
                                    })
                                    .to_string(),
                                ));
                                let _ = out_tx.try_send(Message::Text(
                                    serde_json::json!({
                                        "t": "coop_bonus",
                                        "block_id": bid,
                                        "count": bonus,
                                        "line": vcoop_gather::coop_toast_line(partners, bonus)
                                    })
                                    .to_string(),
                                ));
                            }

                            // 玩家熟練度 v1（自主提案切片，ROADMAP 842）：挖天然方塊累積⛏️採集
                            // 熟練度；練到 Lv.5 起額外多收一份剛挖到的方塊（與上面工具加成 790／
                            // 並肩協作 827 各自獨立判斷、可同時觸發）。
                            let level = award_mastery(&name, MasteryKind::Gathering, &out_tx);
                            give_mastery_bonus(
                                &name,
                                MasteryKind::Gathering,
                                level,
                                target_block as u8,
                                &out_tx,
                            );
                        }

                        // 深層寶藏 v1（自主提案切片，接續 790）：原生礦脈（非遺跡礦）裡極稀有
                        // 一小撮秘密藏著寶藏——挖礦第一次有機會遇上與被挖的礦石種類完全脫鉤的
                        // 驚喜獎勵，也是乙太幣（873）第一次有「挖到的」而非只有「鑄出來的」來源。
                        // 判定純函式在 voxel::treasure_ore_at（確定性、零狀態）；獎勵/公告在本模組。
                        if matches!(target_block, Block::CoalOre | Block::IronOre | Block::AetherOre)
                            && voxel::treasure_ore_at(x, y, z)
                        {
                            try_unlock_milestone(&name, "first_treasure", &out_tx);
                            let (rid, rcount) = vtreasure::treasure_reward();
                            let entry = hub().inventory.write().unwrap().give(&name, rid, rcount);
                            vinv::append_inv(&entry);
                            let new_count = hub().inventory.read().unwrap().count(&name, rid);
                            let _ = out_tx.try_send(Message::Text(
                                serde_json::json!({
                                    "t": "inv_update",
                                    "block_id": rid,
                                    "count": new_count
                                })
                                .to_string(),
                            ));
                            vfeed::append_feed("尋寶", &name, &vtreasure::treasure_feed_detail());
                            let _ = out_tx.try_send(Message::Text(
                                serde_json::json!({
                                    "t": "treasure",
                                    "block_id": rid,
                                    "count": rcount
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
                // 莓果叢 v1（ROADMAP 806）：莓果叢苗和樹苗一樣只能種在「土地」上（草/土/沙/雪/農田土），
                // 不合格早退退提示、不白白消耗莓果叢苗。（短讀鎖取腳下方塊即釋，不與後續鎖巢狀。）
                if block == Block::BerryBush {
                    let ground = {
                        let deltas = hub().deltas.read().unwrap();
                        voxel::effective_block_at(&deltas, x, y - 1, z) as u8
                    };
                    if !vberry::is_plantable_ground(ground) {
                        let _ = out_tx.try_send(Message::Text(
                            serde_json::json!({
                                "t": "plant_fail",
                                "reason": "莓果叢要種在土地上"
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
                        // 玩家里程碑（自主提案切片，追上 738）：第一次親手種下樹苗。
                        try_unlock_milestone(&name, "first_grove", &out_tx);
                    }
                    // 莓果叢 v1（ROADMAP 806）：剛種下的莓果叢苗記進 berry store，
                    // 由 `tick_berry`（15 秒節拍）計時，約 100 秒後結果。
                    if block == Block::BerryBush {
                        hub().berry.write().unwrap().plant(x, y, z, vfarm::now_secs());
                    }
                    // 雞舍生蛋 v1（自主提案切片）：剛放下的空雞舍記進 coop store，
                    // 由 `tick_coop`（15 秒節拍）計時，約 70 秒後生蛋。
                    if block == Block::Coop {
                        hub().coop.write().unwrap().plant(x, y, z, vfarm::now_secs());
                    }
                    // 乙太營火 v1：剛放下一座營火 → 記進取暖清單（居民夜裡靠它吸引），並持久化
                    // （走既有 world_blocks append-only log，重啟後火堆與取暖清單一併還原）。
                    // campfires 寫鎖短取即釋，不與其他鎖巢狀。
                    if block == Block::Campfire {
                        hub().campfires.write().unwrap().push((x, y, z));
                        vbuild::append_world_block(x, y, z, block as u8);
                    }
                    // 集會鐘 v1：剛放下一座鐘 → 持久化（走既有 world_blocks append-only log，重啟後鐘還在）。
                    // 不需 in-memory 索引：召集是敲響當下依鐘座標即時判定範圍（不像營火要夜裡持續吸引路過者）。
                    if block == Block::Bell {
                        vbuild::append_world_block(x, y, z, block as u8);
                    }
                    // 木長椅 v1：剛擺下一張長椅 → 記進歇腳清單（居民白天靠它吸引坐下），並持久化
                    //（走既有 world_blocks append-only log，重啟後長椅與歇腳清單一併還原）。
                    // benches 寫鎖短取即釋，不與其他鎖巢狀。
                    if block == Block::Bench {
                        hub().benches.write().unwrap().push((x, y, z));
                        vbuild::append_world_block(x, y, z, block as u8);
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
                                    // 居民為你的建造作品取名字 v1：這一格是不是第一次被讚賞過——
                                    // 有就沿用既有名字（喚出來、證明「記得」），沒有就當場取一個
                                    // 新名字並記住（短鎖即釋，不與其他鎖巢狀）。
                                    let cell = vstructname::cell_key(x as f32, z as f32);
                                    let pick = now_secs as usize;
                                    let (structure_name, just_named) = {
                                        let mut names = structure_names().lock().unwrap();
                                        match names.get(&cell) {
                                            Some((n, _)) => (n.clone(), false),
                                            None => {
                                                let n = vstructname::pick_name(pick).to_string();
                                                names.insert(cell, (n.clone(), None));
                                                (n, true)
                                            }
                                        }
                                    }; // structure_names 鎖釋放
                                    // 讚賞泡泡（residents 寫鎖即釋；不覆寫既有泡泡＝上面已濾 say 空）。
                                    let say_line = if just_named {
                                        vstructname::name_announce_line(&name, &structure_name, pick)
                                    } else {
                                        vstructname::named_revisit_line(&name, &structure_name, pick)
                                    };
                                    let said = {
                                        let mut residents = hub().residents.write().unwrap();
                                        residents
                                            .iter_mut()
                                            .find(|r| r.id == rid)
                                            .map(|r| {
                                                r.say = say_line.chars().take(50).collect();
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
                                        let summary = vstructname::admire_memory_line_named(
                                            &name,
                                            Some(&structure_name),
                                        );
                                        let entry = hub()
                                            .memory
                                            .write()
                                            .unwrap()
                                            .add_memory(&rid, &name, &summary);
                                        vmem::append_memory(&entry);
                                        if just_named {
                                            vfeed::append_feed(
                                                "居民命名",
                                                rname,
                                                &vstructname::name_feed_line(
                                                    rname,
                                                    &name,
                                                    &structure_name,
                                                ),
                                            );
                                            // 村莊集體里程碑 v1（自主提案切片，ROADMAP 856）：
                                            // 剛命名的這座地標，讓累計地標數跨過門檻了嗎？
                                            // 檢查＋解鎖一氣呵成（單一寫鎖內），與個人里程碑
                                            // （只通知該玩家）刻意區隔——這是全村一起慶祝的事件。
                                            let landmark_count = structure_names().lock().unwrap().len();
                                            let new_tier = hub()
                                                .village_milestones
                                                .write()
                                                .unwrap()
                                                .try_unlock_new_tier(landmark_count);
                                            if let Some(tier) = new_tier {
                                                vvillms::append_village_milestone(
                                                    &vvillms::VillageMilestoneEntry {
                                                        id: tier.id.to_string(),
                                                    },
                                                );
                                                // 全體居民一起歡呼；不覆寫正忙著別的事的居民
                                                // （say 非空＝正在忙），比照既有「不覆寫既有泡泡」慣例。
                                                {
                                                    let mut residents =
                                                        hub().residents.write().unwrap();
                                                    for (i, r) in residents.iter_mut().enumerate() {
                                                        if r.say.is_empty() {
                                                            r.say = vvillms::celebrate_say_line(
                                                                landmark_count + i,
                                                            )
                                                            .to_string();
                                                            r.say_timer = SAY_SECS;
                                                            r.mood_boost_secs = r
                                                                .mood_boost_secs
                                                                .max(voxel_mood::MOOD_BOOST_TALK);
                                                        }
                                                    }
                                                } // residents 寫鎖釋放
                                                broadcast_players();
                                                vfeed::append_feed(
                                                    "村莊里程碑",
                                                    "全村",
                                                    &vvillms::celebrate_feed_line(
                                                        tier.name_zh,
                                                        landmark_count,
                                                    ),
                                                );
                                            }
                                        } else {
                                            vfeed::append_feed(
                                                "居民讚賞",
                                                rname,
                                                &format!(
                                                    "{rname}又看了一眼「{structure_name}」"
                                                ),
                                            );
                                        }
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
                //    累犯罰則（治安三件套①）：反覆命中內容審查者，`talk_penalty_until` 會被推遲到
                //    未來一段時間——這段內連 per-connection 冷卻都直接擋，把試探者自然拖慢。
                let now = std::time::Instant::now();
                if let Some(until) = talk_penalty_until {
                    if now < until {
                        continue; // 罰則冷卻中：靜默忽略（已提示過），別再燒 LLM
                    }
                }
                if let Some(prev) = last_talk {
                    if !talk_cooldown_ok(now.duration_since(prev).as_millis() as u64) {
                        continue;
                    }
                }
                last_talk = Some(now);
                // 2b) per-IP 對話限流（治安三件套①）：步驟 2 的 per-connection 冷卻可被「同一人
                //     開多條 WebSocket 連線」繞過（每連線各有 last_talk）→ 白嫖／燒爆免費 LLM。
                //     這道以真實 IP 為鍵的 token bucket 設一道跨連線天花板；超量→在玩家自己頭上
                //     冒一句溫柔提示、跳過（絕不觸發 LLM）。短鎖即釋、不持鎖 await（守鎖紀律）。
                //     白名單 IP（localhost / QA / env）豁免，讓隔離測試與本機冒煙不受限。
                if !conn_exempt {
                    let allowed = ip_talk_limiter()
                        .lock()
                        .unwrap()
                        .allow(&client_ip, now_unix_ms());
                    if !allowed {
                        let mut players = hub().players.write().unwrap();
                        if let Some(p) = players.get_mut(&my_id) {
                            p.say = TALK_RATE_NOTICE.to_string();
                            p.say_timer = PLAYER_SAY_SECS;
                        }
                        continue;
                    }
                }
                // 2c) 內容審查（治安三件套①·進 LLM 前）：文字長度/速率合格不代表「內容」乾淨。
                //     玩家對話會 ①直達免費 LLM（居民的腦）②廣播成泡泡給所有人看——這道純邏輯審查
                //     攔 prompt injection/越獄注入、成人露骨（NSFW）與明顯辱罵；命中→在玩家自己
                //     頭上冒一句居民得體的迴避、記一次該 IP 違規（累犯加長冷卻）、跳過（絕不觸發
                //     LLM、絕不把原文廣播出去）。零鎖純比對；違規計數短鎖即釋。
                {
                    let verdict = vmod::screen(&clean);
                    if verdict != vmod::Screen::Clean {
                        // 累犯罰則：記一次違規、依累積次數推遲這條連線的下次可對話時間。
                        let n = record_violation(&client_ip);
                        let penalty = violation_cooldown_ms(n);
                        if penalty > 0 {
                            talk_penalty_until =
                                Some(now + std::time::Duration::from_millis(penalty));
                        }
                        let mut players = hub().players.write().unwrap();
                        if let Some(p) = players.get_mut(&my_id) {
                            p.say = vmod::gentle_notice(verdict).to_string();
                            p.say_timer = PLAYER_SAY_SECS;
                        }
                        continue;
                    }
                }
                // 2d) 對話需登入（治安三件套③）：與居民交談會觸發免費 LLM（居民的腦），匿名
                //     腳本可不登入就狂灌、白嫖／燒爆額度。故只有登入帳號才能發起對話；訪客可自由
                //     逛逛與觀看，只是不能聊。身分由後端 cookie→users 權威解出（`is_account`，絕不
                //     信客戶端自報）。訪客→在自己頭上冒一句溫柔提示、跳過（絕不觸發 LLM、絕不廣播）。
                if !talk_allowed_for_identity(is_account) {
                    let mut players = hub().players.write().unwrap();
                    if let Some(p) = players.get_mut(&my_id) {
                        p.say = TALK_GUEST_NOTICE.to_string();
                        p.say_timer = PLAYER_SAY_SECS;
                    }
                    continue;
                }
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
                            Ok(Some(t)) => {
                                // 治安三件套①·LLM 出來後（出口過濾）：小模型偶爾被誘導吐出露骨/失格
                                // 內容——出口再過一遍，命中就改罐頭（守住出口，絕不把失格內容廣播）。
                                if vmod::reply_flagged(&t) {
                                    resident_canned_reply(rname)
                                } else {
                                    t.chars().take(TALK_REPLY_MAX_CHARS).collect()
                                }
                            }
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
                // 居民教你一道獨門配方 v1（自主提案，849）：`TAUGHT_RECIPES` 刻意不併入
                // `find_any_recipe`——只有這位玩家已被居民教過（`player_recipes` 短讀鎖權威
                // 判定，非客戶端自報）才把它接進來合成，否則落到下面 else if 回一句專屬拒絕。
                let recipe_opt = vcraft::find_any_recipe(&recipe_id).or_else(|| {
                    if vcraft::is_taught_recipe(&recipe_id)
                        && hub().player_recipes.read().unwrap().knows(&name, &recipe_id)
                    {
                        vcraft::find_taught_recipe(&recipe_id)
                    } else {
                        None
                    }
                });
                if let Some(recipe) = recipe_opt {
                    // 工作台/熔爐配方伺服器閘門 v1（自主提案切片）：ROADMAP 665/666 的設計
                    // 原意就是「這兩張配方表需要先放置工作台/熔爐才能合成」，但過去只靠前端
                    // 「目前開哪個面板」自律把關——伺服器從未驗證玩家真的站在對應方塊旁，
                    // 改一行前端就能繞過這道門檻。這裡補上權威判定，不信任客戶端自報
                    // （背包 2×2／獨門配方不受此限，本就不需要站別方塊）。
                    let station_need = if vcraft::find_workbench_recipe(&recipe_id).is_some() {
                        Some((Block::Workbench, "身邊要有工作台才能合成這個"))
                    } else if vcraft::find_furnace_recipe(&recipe_id).is_some() {
                        Some((Block::Furnace, "身邊要有熔爐才能合成這個"))
                    } else {
                        None
                    };
                    if let Some((want, reason)) = station_need {
                        let has_station = player_pos(my_id).is_some_and(|(px, py, pz)| {
                            station_nearby(&hub().deltas.read().unwrap(), px, py, pz, want)
                        });
                        if !has_station {
                            let _ = out_tx.try_send(Message::Text(
                                serde_json::json!({
                                    "t": "craft_fail",
                                    "recipe_id": &recipe_id,
                                    "reason": reason
                                }).to_string(),
                            ));
                            continue;
                        }
                    }
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
                } else if vcraft::is_taught_recipe(&recipe_id) {
                    // 存在這道獨門配方，但這位玩家還沒被居民教過——回專屬拒絕理由（比照材料不足）。
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({
                            "t": "craft_fail",
                            "recipe_id": &recipe_id,
                            "reason": "你還沒學會這道配方"
                        }).to_string(),
                    ));
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
                // 時令作物 v1（ROADMAP 811）：種在該作物的時令季節 → 種下當下就靠 nudge_growth
                // 給一截 head-start（沿用居民照料 753／沃肥 789 同一套機制），比平時更快成熟。
                // 非時令不減速、不損資料（只獎不罰、療癒優先）。season 讀鎖即釋、不巢狀。
                let timely_line: Option<String> = {
                    let season = {
                        let day = hub().world_time.read().unwrap().days_elapsed();
                        vseason::season_for_day(day)
                    };
                    if vtimely::is_in_season(kind, season) {
                        let base = vfarm::effective_grow_secs(kind, false);
                        let boost = vtimely::head_start_secs(base);
                        if let Some(nudge_e) =
                            { hub().farm.write().unwrap().nudge_growth(x, y, z, boost) }
                        {
                            vfarm::append_farm(&nudge_e); // head-start 也持久化，重啟仍算數
                        }
                        Some(vtimely::in_season_line(kind, season))
                    } else {
                        None
                    }
                };
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
                        "irrigated": irrigated, "carrot": is_carrot, "potato": is_potato,
                        // 時令作物 v1（811）：種在時令季節時附一句暖回饋（非時令為 null，向後相容舊客戶端）。
                        "timely": timely_line
                    }).to_string(),
                ));
                // 玩家里程碑 v1（ROADMAP 724）：人生第一次種下種子。
                try_unlock_milestone(&name, "first_farm", &out_tx);
                // 居民注意到你悉心照料的農地 v1（自主提案切片）：播種也算農忙連段一步。
                maybe_farm_admire(x as f32, z as f32, &name, &mut farm_streak, &mut farm_admire_cd);
            }
            // ── 乙太沃肥 v1（ROADMAP 789）：手持沃肥對準幼苗催熟一截 ─────────────────
            Ok(ClientMsg::Fertilize { x, y, z }) => {
                // 鎖序：players 讀位置 → delta 讀目標 → inventory 寫扣料 → farm 寫 nudge，
                // 循序取放、不巢狀、鎖外 IO（比照 Plant handler，守 prod 死鎖鐵律）。
                let Some((px, py, pz)) = player_pos(my_id) else {
                    continue;
                };
                // 觸及範圍驗證（施肥作用在方塊本身，比照 Plant）。
                if !voxel::in_reach(px, py, pz, x, y, z) {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "fertilize_fail", "reason": "太遠了" }).to_string(),
                    ));
                    continue;
                }
                // 目標方塊必須是「還在長的幼苗」——後端權威判定（前端不自報合法性·濫用防護）。
                let target = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !vcompost::is_growing_crop(target) {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "fertilize_fail", "reason": "只能施在幼苗上" }).to_string(),
                    ));
                    continue;
                }
                // 消耗 1 份沃肥（放不了不消耗·白嫖不到；inventory 寫鎖即釋）。
                let fert_entry = hub()
                    .inventory
                    .write()
                    .unwrap()
                    .take(&name, vcompost::FERTILIZER_ID, 1);
                let Some(fert_e) = fert_entry else {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "fertilize_fail", "reason": "沒有沃肥" }).to_string(),
                    ));
                    continue;
                };
                vinv::append_inv(&fert_e);
                // 農地計時往前推進一截（沿用居民照料 753 同一套 nudge_growth，farm 寫鎖即釋）。
                let farm_e = {
                    hub()
                        .farm
                        .write()
                        .unwrap()
                        .nudge_growth(x, y, z, vcompost::FERTILIZER_BOOST_SECS)
                };
                if let Some(e) = &farm_e {
                    vfarm::append_farm(e);
                }
                // 作物顯示名（依 Seeded 方塊分岔；面向玩家字串，i18n 集中）。
                let crop = match target {
                    Block::CarrotSeeded => "胡蘿蔔",
                    Block::PotatoSeeded => "馬鈴薯",
                    _ => "小麥",
                };
                let pick = rand::random::<u64>() as usize;
                // 送背包更新 + 施肥回饋（作物成熟與否交給既有 farm tick 判定翻面，near-instant）。
                let new_cnt = hub()
                    .inventory
                    .read()
                    .unwrap()
                    .count(&name, vcompost::FERTILIZER_ID);
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({
                        "t": "inv_update",
                        "block_id": vcompost::FERTILIZER_ID,
                        "count": new_cnt
                    })
                    .to_string(),
                ));
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({
                        "t": "fertilize_ok", "x": x, "y": y, "z": z,
                        "say": vcompost::fertilize_say_line(crop, pick)
                    })
                    .to_string(),
                ));
            }
            // ── 水桶舀水 v1（自主提案切片）──────────────────────────────────────
            Ok(ClientMsg::BucketFill { x, y, z }) => {
                // 鎖序：players 讀位置 → delta 讀目標 → inventory 寫換桶 → delta 寫設空氣，
                // 循序取放不巢狀、鎖外 enqueue_water_around（比照破壞水路慣例，守 prod 死鎖鐵律）。
                let Some((px, py, pz)) = player_pos(my_id) else {
                    continue;
                };
                if !voxel::in_reach(px, py, pz, x, y, z) {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "bucket_fail", "reason": "太遠了" }).to_string(),
                    ));
                    continue;
                }
                // 目標必須是來源水——後端權威判定（前端不自報合法性·濫用防護）。
                let target = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !vbucket::is_fillable_source(target as u8) {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "bucket_fail", "reason": "這裡沒有水源可舀" })
                            .to_string(),
                    ));
                    continue;
                }
                // 消耗 1 只空水桶（沒有就舀不成·白嫖不到；inventory 寫鎖即釋）。
                let take_e = hub()
                    .inventory
                    .write()
                    .unwrap()
                    .take(&name, vbucket::BUCKET_ID, 1);
                let Some(take_e) = take_e else {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "bucket_fail", "reason": "手上沒有空水桶" })
                            .to_string(),
                    ));
                    continue;
                };
                vinv::append_inv(&take_e);
                // 把水源那格化為空氣（delta 寫鎖即釋；session-only，比照一般破壞不寫 world_blocks log）。
                {
                    let mut world = hub().deltas.write().unwrap();
                    voxel::set_block(&mut world, x, y, z, Block::Air);
                }
                broadcast_block(x, y, z, Block::Air);
                // 鎖外喚醒鄰格重算水流——鄰近水體會往缺口流來補位（天然源不會憑空多出、無限複製）。
                enqueue_water_around(x, y, z);
                // 換給一只滿水桶（inventory 寫鎖即釋）。
                let give_e = hub()
                    .inventory
                    .write()
                    .unwrap()
                    .give(&name, vbucket::WATER_BUCKET_ID, 1);
                vinv::append_inv(&give_e);
                // 兩個 inv_update（空桶減、滿桶增）＋回饋句。
                let empty_cnt = hub().inventory.read().unwrap().count(&name, vbucket::BUCKET_ID);
                let full_cnt = hub()
                    .inventory
                    .read()
                    .unwrap()
                    .count(&name, vbucket::WATER_BUCKET_ID);
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "inv_update", "block_id": vbucket::BUCKET_ID, "count": empty_cnt })
                        .to_string(),
                ));
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "inv_update", "block_id": vbucket::WATER_BUCKET_ID, "count": full_cnt })
                        .to_string(),
                ));
                let pick = rand::random::<u64>() as usize;
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "bucket_ok", "say": vbucket::fill_ok_line(pick) })
                        .to_string(),
                ));
            }
            // ── 水桶倒水 v1（自主提案切片）──────────────────────────────────────
            Ok(ClientMsg::BucketPour { x, y, z }) => {
                let Some((px, py, pz)) = player_pos(my_id) else {
                    continue;
                };
                if !voxel::in_reach(px, py, pz, x, y, z) {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "bucket_fail", "reason": "太遠了" }).to_string(),
                    ));
                    continue;
                }
                // 目標必須可倒（空氣或流動水；實心擋水、既有源不必重放·後端權威判定）。
                let target = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !vbucket::is_pourable_target(target as u8) {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "bucket_fail", "reason": "這裡倒不了水" })
                            .to_string(),
                    ));
                    continue;
                }
                // 消耗 1 只滿水桶（沒有就倒不成·白嫖不到；inventory 寫鎖即釋）。
                let take_e = hub()
                    .inventory
                    .write()
                    .unwrap()
                    .take(&name, vbucket::WATER_BUCKET_ID, 1);
                let Some(take_e) = take_e else {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "bucket_fail", "reason": "手上沒有滿水桶" })
                            .to_string(),
                    ));
                    continue;
                };
                vinv::append_inv(&take_e);
                // 放下一格永久來源水（delta 寫鎖即釋；session-only，比照一般放置不寫 world_blocks log）。
                {
                    let mut world = hub().deltas.write().unwrap();
                    voxel::set_block(&mut world, x, y, z, Block::Water);
                }
                broadcast_block(x, y, z, Block::Water);
                // 鎖外喚醒水流——來源水由既有模擬當永不乾涸的源頭自然漫開、把周圍農地接上水耕。
                enqueue_water_around(x, y, z);
                // 換回一只空水桶（inventory 寫鎖即釋）。
                let give_e = hub()
                    .inventory
                    .write()
                    .unwrap()
                    .give(&name, vbucket::BUCKET_ID, 1);
                vinv::append_inv(&give_e);
                let empty_cnt = hub().inventory.read().unwrap().count(&name, vbucket::BUCKET_ID);
                let full_cnt = hub()
                    .inventory
                    .read()
                    .unwrap()
                    .count(&name, vbucket::WATER_BUCKET_ID);
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "inv_update", "block_id": vbucket::WATER_BUCKET_ID, "count": full_cnt })
                        .to_string(),
                ));
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "inv_update", "block_id": vbucket::BUCKET_ID, "count": empty_cnt })
                        .to_string(),
                ));
                let pick = rand::random::<u64>() as usize;
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "bucket_ok", "say": vbucket::pour_ok_line(pick) })
                        .to_string(),
                ));
            }
            // ── 鋤頭開墾 v1（自主提案切片）：就地把草／土翻成農田土 ─────────────────────
            Ok(ClientMsg::HoeTill { x, y, z }) => {
                // 鎖序：players 讀位置 → delta 讀目標 → inventory 讀驗持有 → delta 寫設農田土，
                // 循序取放不巢狀（守 prod 死鎖鐵律）。鋤頭是工具不耗損，只驗持有、不消耗。
                let Some((px, py, pz)) = player_pos(my_id) else {
                    continue;
                };
                if !voxel::in_reach(px, py, pz, x, y, z) {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "hoe_fail", "reason": "太遠了" }).to_string(),
                    ));
                    continue;
                }
                // 目標必須是草地或泥土——後端權威判定（前端不自報合法性·濫用防護）。
                let target = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !vhoe::is_tillable(target as u8) {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "hoe_fail", "reason": "這裡沒法開墾" }).to_string(),
                    ));
                    continue;
                }
                // 背包必須真持有鋤頭才生效（前端自報手持工具 id，伺服器必查·防偽報白嫖）。
                if hub().inventory.read().unwrap().count(&name, vhoe::HOE_ID) < 1 {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "hoe_fail", "reason": "手上沒有鋤頭" }).to_string(),
                    ));
                    continue;
                }
                // 就地翻成農田土（delta 寫鎖即釋；session-only，比照一般放置不寫 world_blocks log）。
                {
                    let mut world = hub().deltas.write().unwrap();
                    voxel::set_block(&mut world, x, y, z, Block::FarmSoil);
                }
                broadcast_block(x, y, z, Block::FarmSoil);
                let pick = rand::random::<u64>() as usize;
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "hoe_ok", "say": vhoe::till_ok_line(pick) }).to_string(),
                ));
                // 居民注意到你悉心照料的農地 v1（自主提案切片）：翻土也算農忙連段一步。
                maybe_farm_admire(x as f32, z as f32, &name, &mut farm_streak, &mut farm_admire_cd);
            }
            // ── 餵野兔馴服 v1（自主提案切片）：世界環境軸線(847/848)與玩家互動軸線首次交會 ──
            Ok(ClientMsg::FeedWildlife { id }) => {
                // 鎖序：players 讀位置 → wildlife 讀鎖驗種類/距離/是否已馴服 → inventory 寫鎖
                // 消耗胡蘿蔔 → wildlife 寫鎖落地馴服狀態，循序取放不巢狀（守死鎖鐵律）。
                let Some((px, _py, pz)) = player_pos(my_id) else {
                    continue;
                };
                let snap: Option<(bool, bool, f32)> = {
                    let animals = hub().wildlife.read().unwrap();
                    animals.iter().find(|a| a.id == id).map(|a| {
                        let dx = px - a.body.x;
                        let dz = pz - a.body.z;
                        (matches!(a.kind, WildlifeKind::Rabbit), a.tamed, dx * dx + dz * dz)
                    })
                };
                let Some((is_rabbit, already_tamed, dist_sq)) = snap else {
                    continue; // 這隻動物已消失（id 過期）——靜默忽略
                };
                if !is_rabbit {
                    continue; // 目前只有野兔可餵（魚不需要馴服）——靜默忽略，非錯誤
                }
                if !vwild::should_tame(already_tamed, dist_sq) {
                    let reason = if already_tamed { "牠已經不怕你了，不用再餵" } else { "走近一點再餵" };
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "feed_wildlife_fail", "reason": reason }).to_string(),
                    ));
                    continue;
                }
                // 背包必須真持有胡蘿蔔（前端不自報合法性·濫用防護：伺服器必查真實庫存）。
                let Some(inv_entry) = hub().inventory.write().unwrap().take(&name, vfarm::CARROT_ID, 1) else {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "feed_wildlife_fail", "reason": "背包裡沒有胡蘿蔔" }).to_string(),
                    ));
                    continue;
                };
                vinv::append_inv(&inv_entry);
                {
                    let mut animals = hub().wildlife.write().unwrap();
                    if let Some(a) = animals.iter_mut().find(|a| a.id == id) {
                        a.tamed = true;
                    }
                }
                try_unlock_milestone(&name, "first_tame", &out_tx);
                let remain = hub().inventory.read().unwrap().count(&name, vfarm::CARROT_ID);
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "inv_update", "block_id": vfarm::CARROT_ID, "count": remain }).to_string(),
                ));
                let pick = rand::random::<u64>() as usize;
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "feed_wildlife_ok", "say": vwild::tame_line(pick) }).to_string(),
                ));
            }
            // ── 放養雞 v1（自主提案切片，ROADMAP 870）：世界環境軸線第二種可馴服動物 ──
            Ok(ClientMsg::FeedChicken { id }) => {
                // 鎖序比照餵野兔馴服（847）：players 讀位置 → wildlife 讀鎖驗種類/距離/是否
                // 已馴服 → inventory 寫鎖消耗種子 → wildlife 寫鎖落地馴服狀態+首次下蛋倒數，
                // 循序取放不巢狀（守死鎖鐵律）。
                let Some((px, _py, pz)) = player_pos(my_id) else {
                    continue;
                };
                let snap: Option<(bool, bool, f32)> = {
                    let animals = hub().wildlife.read().unwrap();
                    animals.iter().find(|a| a.id == id).map(|a| {
                        let dx = px - a.body.x;
                        let dz = pz - a.body.z;
                        (matches!(a.kind, WildlifeKind::Chicken), a.tamed, dx * dx + dz * dz)
                    })
                };
                let Some((is_chicken, already_tamed, dist_sq)) = snap else {
                    continue; // 這隻動物已消失（id 過期）——靜默忽略
                };
                if !is_chicken {
                    continue; // 目前只有雞可餵（兔子/魚走各自的餵食路徑）——靜默忽略，非錯誤
                }
                if !vwild::should_tame(already_tamed, dist_sq) {
                    let reason = if already_tamed { "牠已經不怕你了，不用再餵" } else { "走近一點再餵" };
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "feed_chicken_fail", "reason": reason }).to_string(),
                    ));
                    continue;
                }
                // 背包必須真持有種子（前端不自報合法性·濫用防護：伺服器必查真實庫存）。
                let Some(inv_entry) = hub().inventory.write().unwrap().take(&name, vfarm::SEEDS_ID, 1) else {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "feed_chicken_fail", "reason": "背包裡沒有小麥種子" }).to_string(),
                    ));
                    continue;
                };
                vinv::append_inv(&inv_entry);
                {
                    let mut animals = hub().wildlife.write().unwrap();
                    if let Some(a) = animals.iter_mut().find(|a| a.id == id) {
                        a.tamed = true;
                        // 馴服當下就設好第一次下蛋倒數（正常間隔起算，非立即下蛋）。
                        a.lay_cd = vchicken::next_lay_cooldown(rand::random::<f32>());
                    }
                }
                try_unlock_milestone(&name, "first_chicken_tame", &out_tx);
                let remain = hub().inventory.read().unwrap().count(&name, vfarm::SEEDS_ID);
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "inv_update", "block_id": vfarm::SEEDS_ID, "count": remain }).to_string(),
                ));
                let pick = rand::random::<u64>() as usize;
                let _ = out_tx.try_send(Message::Text(
                    serde_json::json!({ "t": "feed_chicken_ok", "say": vchicken::tame_line(pick) }).to_string(),
                ));
            }
            // ── 集會鐘 v1（自主提案切片）：敲響一座鐘，把附近閒著的居民召到身邊 ─────────
            Ok(ClientMsg::RingBell { x, y, z }) => {
                // 鎖序：players 讀位置 → delta 讀目標型別 → residents 寫設應召，循序不巢狀（守死鎖鐵律）。
                let Some((px, py, pz)) = player_pos(my_id) else {
                    continue;
                };
                if !voxel::in_reach(px, py, pz, x, y, z) {
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "ring_fail", "reason": "走近一點才敲得到鐘" }).to_string(),
                    ));
                    continue;
                }
                // 目標必須是集會鐘——後端權威判定（前端不自報合法性·濫用防護③：型別由伺服器認）。
                let target = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !matches!(target, Block::Bell) {
                    // 對準的不是鐘（或鐘已被別人挖掉）→ 靜默忽略。
                    continue;
                }
                // 召集：把範圍內閒著（醒著、非遠行、應召冷卻到期）的居民設為應召，循聲朝鐘走來。
                // 濫用防護①：per-居民 [`vbell::SUMMON_COOLDOWN_SECS`] 冷卻＝就算狂敲鐘，同一位居民也
                // 拖不動太頻繁。濫用防護②：ring_bell 不觸發任何 LLM／不收自由文字，無注入／燒額度風險。
                let ringer = name.clone();
                let bx = x as f32 + 0.5;
                let bz = z as f32 + 0.5;
                let heeded = {
                    let mut residents = hub().residents.write().unwrap();
                    let mut n = 0usize;
                    for r in residents.iter_mut() {
                        if !vbell::eligible(r.asleep, r.expedition.is_some(), r.summon_cooldown) {
                            continue;
                        }
                        if vbell::within_summon(bx, bz, r.body.x, r.body.z, vbell::SUMMON_RADIUS) {
                            r.summon = Some(vbell::Summon {
                                x: bx,
                                z: bz,
                                timer: vbell::SUMMON_TIMEOUT_SECS,
                                ringer: ringer.clone(),
                            });
                            n += 1;
                        }
                    }
                    n
                }; // residents 寫鎖釋放
                if heeded > 0 {
                    // 至少召到一位才廣播鐘聲＋上 Feed（濫用防護：空敲不廣播、不洗版全場）。
                    broadcast_bell_ring(x, y, z, &ringer, heeded);
                    vfeed::append_feed("鐘聲", &ringer, &vbell::ring_feed_line(&ringer, heeded));
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "ring_ok", "count": heeded }).to_string(),
                    ));
                } else {
                    // 附近沒有聽得到又有空的居民（都睡了／遠行／剛應召過）→ 只回敲鐘者一句提示，不廣播。
                    let _ = out_tx.try_send(Message::Text(
                        serde_json::json!({ "t": "ring_none" }).to_string(),
                    ));
                }
            }
            // ── 地標旅人留言 v1（自主提案切片，ROADMAP 862）─────────────────────────
            Ok(ClientMsg::LeaveLandmarkNote { x, y, z, text }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                // 判定這裡是不是一處已知地標：遺跡看瞄準座標（`ruin_ore_at` 純座標判定，
                // 挖掉後仍成立，但要求觸及範圍內，比照告示牌/敲鐘同一套 reach 護欄）；
                // 溫泉不看瞄準座標，改看玩家此刻是否真的泡在溫泉裡（比照溫泉回血 tick
                // 同一套判定）；邊陲營地（自主提案切片，接續 881）不看瞄準座標、也不看泡水，
                // 改看玩家此刻是否站在某位居民親手搭起的營地床邊（`player_near_built_outpost`，
                // 與探索紀事 tick 偵測同一套判定）——三者皆避免對著遠方喊話就騙過伺服器。
                let landmark = if voxel::in_reach(px, py, pz, x, y, z) && voxel::ruin_ore_at(x, y, z) {
                    Some((vdisc::LandmarkKind::Ruin, (x, z), x, y, z))
                } else if feet_in_hot_spring(px, py, pz) {
                    let cell = voxel::hot_spring_cell_of(px.floor() as i32, pz.floor() as i32);
                    let (ix, iy, iz) = (px.floor() as i32, py.floor() as i32, pz.floor() as i32);
                    Some((vdisc::LandmarkKind::HotSpring, cell, ix, iy, iz))
                } else if let Some((_, _, ax, ay, az)) = player_near_built_outpost(px, pz) {
                    Some((vdisc::LandmarkKind::Outpost, (ax, az), ax, ay, az))
                } else {
                    None
                };
                let Some((kind, dedup_key, lx, ly, lz)) = landmark else {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "landmark_note_fail", "reason": "這裡不是已知的地標，找一處遺跡、溫泉或邊陲營地試試看。"
                    }).to_string())).await;
                    continue;
                };
                // 留言簿署名給後來的旅人看，比照瓶中信同一套登入護欄——只有登入帳號才能寫。
                if !talk_allowed_for_identity(is_account) {
                    let mut players = hub().players.write().unwrap();
                    if let Some(p) = players.get_mut(&my_id) {
                        p.say = LANDMARK_NOTE_GUEST_NOTICE.to_string();
                        p.say_timer = PLAYER_SAY_SECS;
                    }
                    continue;
                }
                let clean = vsign::sanitize_text(&text);
                if clean.is_empty() {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "landmark_note_fail", "reason": "留言不能是空的喔，寫點什麼吧。"
                    }).to_string())).await;
                    continue;
                }
                // 內容審查（治安三件套②同款）：命中→溫柔提示、絕不存檔／絕不廣播原文。
                {
                    let verdict = vmod::screen(&clean);
                    if verdict != vmod::Screen::Clean {
                        let mut players = hub().players.write().unwrap();
                        if let Some(p) = players.get_mut(&my_id) {
                            p.say = vmod::gentle_notice(verdict).to_string();
                            p.say_timer = PLAYER_SAY_SECS;
                        }
                        continue;
                    }
                }
                let entry = {
                    let mut store = hub().landmark_notes.write().unwrap();
                    store.leave(kind, dedup_key, &name, &clean)
                }; // landmark_notes 寫鎖釋放
                if let Some(entry) = entry {
                    vlmark::append_note(&entry);
                    try_unlock_milestone(&name, "first_landmark_note", &out_tx);
                    // 回傳這處地標目前的完整留言簿（含剛寫入的這一筆），前端據此更新面板。
                    send_landmark_notes(kind, dedup_key, lx, ly, lz, &out_tx).await;
                }
            }
            // ── 個人路標 v1（自主提案切片，ROADMAP 869）─────────────────────────
            Ok(ClientMsg::SetWaypoint { label }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                let clean = vwaypoint::sanitize_label(&label);
                let result = {
                    let mut store = hub().waypoints.write().unwrap();
                    store.set(&name, &clean, px.floor() as i32, py.floor() as i32, pz.floor() as i32)
                }; // waypoints 寫鎖釋放
                match result {
                    Ok(entry) => {
                        vwaypoint::append_entry(&entry);
                        try_unlock_milestone(&name, "first_waypoint", &out_tx);
                        send_waypoints(&name, &out_tx).await;
                    }
                    Err(vwaypoint::SetErr::EmptyLabel) => {
                        let _ = out_tx.send(Message::Text(serde_json::json!({
                            "t": "waypoint_fail", "reason": "路標名稱不能是空的喔，取個短名字吧。"
                        }).to_string())).await;
                    }
                    Err(vwaypoint::SetErr::TooMany) => {
                        let _ = out_tx.send(Message::Text(serde_json::json!({
                            "t": "waypoint_fail",
                            "reason": format!("路標已插滿 {} 支了，先刪一支再插新的吧。", vwaypoint::MAX_WAYPOINTS_PER_PLAYER),
                        }).to_string())).await;
                    }
                }
            }
            // ── 個人路標 v1：刪除 ─────────────────────────────────────────────
            Ok(ClientMsg::RemoveWaypoint { label }) => {
                let clean = vwaypoint::sanitize_label(&label);
                let removed = {
                    let mut store = hub().waypoints.write().unwrap();
                    store.remove(&name, &clean)
                }; // waypoints 寫鎖釋放
                match removed {
                    Some(tombstone) => {
                        vwaypoint::append_entry(&tombstone);
                        send_waypoints(&name, &out_tx).await;
                    }
                    None => {
                        let _ = out_tx.send(Message::Text(serde_json::json!({
                            "t": "waypoint_fail", "reason": "找不到這支路標。"
                        }).to_string())).await;
                    }
                }
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
                // 2) 短鎖取居民快照（residents 讀鎖即釋）。y 供紀念物 v1（732）找腳邊空位用；
                //    open_request 供「拜託你幫個小忙 v1」判斷這份禮是否正中居民開口討的材料。
                let res_snap: Option<(&'static str, f32, f32, f32, Option<u8>, f32)> = {
                    let residents = hub().residents.read().unwrap();
                    residents
                        .iter()
                        .find(|r| r.id == resident_id)
                        .map(|r| (r.name, r.body.x, r.body.y, r.body.z, r.open_request, r.illness_severity))
                };
                let Some((rname, rx, ry, rz, open_request, illness_severity)) = res_snap else {
                    continue; // 找不到居民
                };
                // 拜託你幫個小忙 v1（自主提案）：這位居民此刻是否正等著有人送這樣材料來？
                // 送對了＝你在她開口時幫上了忙，值得一份更歡欣的道謝＋記進「你幫過我」的人情。
                let request_fulfilled = open_request == Some(item_id);
                // 居民也會生病 v1（自主提案）：送的正是野菜暖湯、且這位居民此刻正生病——
                // 你在她最難受的時候端了碗湯來，值得一份更觸動的道謝＋病況大幅緩解＋深記憶。
                let soup_care_hit = item_id == vcraft::STEW_ID && villness::is_sick(illness_severity);
                // 建築藍圖 v1（自主提案）：這份禮是不是一張藍圖？藍圖直接指定她接下來蓋哪一種建物
                // （非猜關鍵詞），下方 5c) 據此改寫她的心願。
                let blueprint_kind_hit = vblueprint::blueprint_kind(item_id);
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
                // 拜託你幫個小忙 v1：若這份禮正中她開口討的材料，另記一筆「你在我開口時幫了我」的
                // 人情（episodic，掛玩家名下累積好感）——她會記得你幫過她，日後回想 / 日記可引用。
                if request_fulfilled {
                    let mem3 = vrequest::fulfil_memory_line(&name, iname);
                    let entry3 = {
                        hub().memory.write().unwrap().add_memory(&resident_id, &name, &mem3)
                    };
                    vmem::append_memory(&entry3);
                }
                // 居民也會生病 v1：你在她正生病時端了碗熱湯來——這份暖她記得格外深（掛玩家名下累積好感）。
                if soup_care_hit {
                    let mem4 = villness::soup_care_memory(&name);
                    let entry4 = {
                        hub().memory.write().unwrap().add_memory(&resident_id, &name, &mem4)
                    };
                    vmem::append_memory(&entry4);
                }
                // 5b) 送對禮物 v1（ROADMAP 722）：這位居民是否正懷抱一句「送這個物品就能實現」的
                // 非建造類心願（desires 讀鎖即釋，不與其他鎖巢狀）？建造類心願交給蓋家系統的
                // 心願成真（720），這裡刻意不搶。讀的是這份禮**送到之前**的舊心願狀態，故排在
                // 5c) 藍圖改寫她的心願之前。
                let item_wish_hit: bool = {
                    let desires = hub().desires.read().unwrap();
                    desires.get_desire(&resident_id).is_some_and(|d| {
                        !d.fulfilled
                            && vbuild::classify_desire(&d.desire).is_none()
                            && vgift::classify_item_desire(&d.desire) == Some(item_id)
                    })
                };
                // 5c) 建築藍圖 v1：藍圖直接改寫她的心願成藍圖指定的建物種類——沿用玩家聊天種
                // 願望的同一套 `set_desire`（`sparked_by`=玩家名 → 完工時指名感謝，見既有
                // 「無計畫」活動選擇段落）；零新狀態機，`voxel_building`/`voxel_skills` 不用改一行。
                if let Some(kind) = blueprint_kind_hit {
                    let entry = {
                        let mut des = hub().desires.write().unwrap();
                        des.set_desire(&resident_id, vblueprint::blueprint_desire_text(kind), &name)
                    }; // desires 寫鎖釋放
                    vdes::append_desire(&entry);
                    let mem5 = vblueprint::blueprint_memory_line(&name, kind);
                    let entry5 = {
                        hub().memory.write().unwrap().add_memory(&resident_id, &name, &mem5)
                    };
                    vmem::append_memory(&entry5);
                    // 玩家里程碑（自主提案切片，追上 826）：第一次用藍圖指定居民蓋什麼。
                    try_unlock_milestone(&name, "first_blueprint", &out_tx);
                }
                // 6) 讀好感度（memory 讀鎖即釋）。
                let affinity = {
                    hub().memory.read().unwrap().affinity_count(&name, &resident_id)
                };
                // 7) 組道謝台詞（純函式，無鎖）——藍圖 > 心願送到 > 食物(BREAD_ID=19) > 一般禮物。
                let pick = (vfarm::now_secs() as usize).wrapping_add(item_id as usize);
                let thanks = if let Some(kind) = blueprint_kind_hit {
                    // 藍圖最優先：這句話點名「心願被直接指定」，非一般贈禮道謝可比。
                    vblueprint::blueprint_thanks_line(kind, pick)
                } else if item_wish_hit {
                    vgift::item_wish_thanks_line(rname, iname, &name)
                } else if soup_care_hit {
                    // 送對了時機的暖湯——比一般贈禮更觸動的專屬道謝。
                    villness::soup_care_thanks_line(&name, pick)
                } else if request_fulfilled {
                    // 你採來了她開口討的材料——比一般贈禮更歡欣（「你在我開口時幫了我」）。
                    vrequest::fulfil_thanks_line(&name, iname, pick)
                } else if vgift::is_treasure_gift(item_id) {
                    vgift::treasure_gift_thanks_line(&name, affinity, pick)
                } else if vgift::is_flower_gift(item_id) {
                    // 野花 v1（自主提案切片）：世界第一句「收到花」的心動道謝——與珍寶（礦物向
                    // 驚喜）、食物（實用向感謝）都不同的情感register，純粹送禮示好的心意。
                    vgift::flower_gift_thanks_line(&name, affinity, pick)
                } else if item_id == vfish::COOKED_FISH_ID {
                    // 烤魚是玩家「釣起→烤熟」的一道熱佳餚，用專屬台詞（比一般食物更歡欣）。
                    vgift::cooked_fish_thanks_line(&name, affinity, pick)
                } else if item_id == vfarm::BAKED_POTATO_ID {
                    // 烤地薯是玩家「種田→收成→烤熟」的一道熱佳餚，用專屬台詞（比一般食物更歡欣）。
                    vgift::baked_potato_thanks_line(&name, affinity, pick)
                } else if item_id == vcraft::STEW_ID {
                    // 野菜暖湯是玩家湊齊三種親手種的作物、在工作台拌煮的一鍋料理，用專屬台詞（最觸動）。
                    vgift::stew_thanks_line(&name, affinity, pick)
                } else if item_id == crate::voxel_berry::JAM_ID {
                    // 莓果醬是乙太方界第一種甜點，居民對甜食格外雀躍，用專屬台詞（莓果醬 v1 ROADMAP 808）。
                    vgift::jam_thanks_line(&name, affinity, pick)
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
                        // 拜託你幫個小忙 v1：這份禮正中她開口討的材料 → 請求了結、清掉並重置冷卻
                        //（避免剛幫完又立刻再討，讓「反過來拜託你」保持稀有有份量）。
                        if request_fulfilled {
                            r.open_request = None;
                            r.request_timer = vrequest::REQUEST_COOLDOWN_SECS;
                        }
                        // 你送的食物她會細細享用 v1（ROADMAP 765）：若送的是食物，居民不立刻吃掉，
                        // 而是捧在手中，稍後在閒下來的安靜片刻真的享用（見 tick_residents 的享用分支）。
                        // 同時只捧一份——連續送多份食物，取最新那份（不排隊、天然防洗版）。
                        if vgift::is_food_gift(item_id) {
                            r.savoring =
                                Some((item_id, name.clone(), vsavor::SAVOR_DELAY_SECS));
                        }
                        // 居民也會生病 v1：這碗湯送對了時機——病況大幅緩解（比鄰居陪伴更大方）。
                        if soup_care_hit {
                            r.illness_severity =
                                villness::apply_care(r.illness_severity, villness::SOUP_CARE_BOOST);
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
                if let Some(kind) = blueprint_kind_hit {
                    // 建築藍圖 v1：不在場的玩家回來也讀得到「誰指定了哪位居民蓋什麼」。
                    vfeed::append_feed(
                        "藍圖",
                        rname,
                        &vblueprint::blueprint_feed_line(&name, rname, kind),
                    );
                } else if soup_care_hit {
                    // 居民也會生病 v1：你在她正生病時送湯來——動態牆記成一則專屬的鄰里照應，
                    // 讓小社會看見「有人在她難受時送了暖」。
                    vfeed::append_feed(
                        villness::FEED_KIND,
                        rname,
                        &villness::soup_care_feed_line(rname, &name),
                    );
                } else if request_fulfilled {
                    // 拜託你幫個小忙 v1：她開口討的材料被送到了——動態牆記成一則「求助達成」，
                    // 讓小社會看見「有人回應了某居民的請求」。
                    vfeed::append_feed(
                        "求助達成",
                        rname,
                        &format!("{name}把{rname}想要的{iname}送來了，幫上了她的忙。"),
                    );
                } else {
                    vfeed::append_feed(
                        "贈禮",
                        rname,
                        &format!("{name}送了{iname}給{rname}"),
                    );
                }
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
                        // 睹物思人 v1（ROADMAP 784）：把這件剛擺出的紀念物座標記進這位居民的追憶佇列，
                        // 她日後閒晃恰好路過時偶爾會駐足想起你。residents 短寫鎖即釋、不巢狀（此處已在
                        // 其他鎖外，安全）；純記憶體、重啟歸零。
                        {
                            let mut residents = hub().residents.write().unwrap();
                            if let Some(r) = residents.iter_mut().find(|r| r.id == resident_id) {
                                vkrecall::remember_spot(
                                    &mut r.keepsake_spots,
                                    vkrecall::KeepsakeSpot {
                                        x: kx,
                                        y: ky,
                                        z: kz,
                                        item: kname.to_string(),
                                        giver: name.clone(),
                                    },
                                    vkrecall::MAX_SPOTS,
                                );
                            }
                        } // residents 寫鎖釋放
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
                    // 只有「首次」圓夢（mark_fulfilled 回 Some）才是值得道賀的一刻——
                    // 重複送同一件禮物不再觸發（冪等，天然防洗版）。
                    let first_fulfill = marked.is_some();
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

                    // 12b) 居民為鄰居圓夢而賀喜 v1（ROADMAP 782）：圓夢的這一刻，若身邊剛好有另一位
                    // 醒著的鄰居，她會看見、由衷道賀一句，圓夢者回謝，兩人情誼因這份共同喜悅升溫、
                    // 各記一筆——小社會第一次為彼此的成就道賀。**鎖序**：residents 讀（快照鄰居位置，
                    // 即釋）→ residents 寫（設兩人泡泡，即釋）→ 廣播 → bonds 寫（record_visit，即釋）
                    // →〔升級時〕bonds 讀 save + memory 寫社交痕跡 → memory 寫 ×2（雙方見證記憶）→
                    // 動態牆 IO，全程短鎖循序、不巢狀、鎖外 IO，守死鎖鐵律。
                    if first_fulfill {
                        // 快照：找醒著、非圓夢者本人的鄰居，算相對圓夢者(rx,rz)的水平位移。
                        let (wid, wname, cand_offsets): (Vec<String>, Vec<&'static str>, Vec<(f32, f32)>) = {
                            let residents = hub().residents.read().unwrap();
                            let mut ids = Vec::new();
                            let mut names = Vec::new();
                            let mut offs = Vec::new();
                            for r in residents.iter() {
                                if r.id == resident_id || r.asleep {
                                    continue;
                                }
                                ids.push(r.id.clone());
                                names.push(r.name);
                                offs.push((r.body.x - rx, r.body.z - rz));
                            }
                            (ids, names, offs)
                        }; // residents 讀鎖釋放
                        if let Some(wi) = vwit::nearest_witness_index(&cand_offsets) {
                            let witness_id = wid[wi].clone();
                            let witness_name = wname[wi];
                            let say = vwit::witness_say_line(rname, pick);
                            let reply = vwit::witness_reply_line(witness_name, pick);
                            // 設兩人泡泡（residents 寫鎖即釋，一把鎖內同時設，畫面上看得見這場道賀）。
                            {
                                let mut residents = hub().residents.write().unwrap();
                                for r in residents.iter_mut() {
                                    if r.id == witness_id {
                                        r.say = say.chars().take(40).collect();
                                        r.say_timer = SAY_SECS;
                                    } else if r.id == resident_id {
                                        r.say = reply.chars().take(40).collect();
                                        r.say_timer = SAY_SECS;
                                    }
                                }
                            } // residents 寫鎖釋放
                            broadcast_players();
                            // 情誼因這份共同見證的喜悅加溫一格（bonds 以顯示名記帳）。
                            let (tier, tier_changed) = {
                                let mut bonds = hub().bonds.write().unwrap();
                                bonds.record_visit(rname, witness_name)
                            }; // bonds 寫鎖釋放
                            if tier_changed {
                                {
                                    let bonds = hub().bonds.read().unwrap();
                                    vbonds::save_bonds(&bonds);
                                } // bonds 讀鎖釋放
                                let milestone = vbonds::tier_up_line(tier, rname, witness_name);
                                vfeed::append_feed("居民情誼", rname, &milestone);
                                let social_mem = vbonds::bond_social_memory(witness_name, tier);
                                if !social_mem.is_empty() {
                                    let e = hub().memory.write().unwrap()
                                        .add_memory(&resident_id, witness_name, &social_mem);
                                    vmem::append_memory(&e);
                                } // memory 寫鎖釋放
                            }
                            // 雙方各記一筆「一起見證圓夢」的記憶（掛在對方名下，交情自然加深）。
                            let mem_w = vwit::witness_memory_for_witness(rname);
                            let ew = {
                                hub().memory.write().unwrap().add_memory(&witness_id, rname, &mem_w)
                            }; // memory 寫鎖釋放
                            vmem::append_memory(&ew);
                            let mem_a = vwit::witness_memory_for_achiever(witness_name);
                            let ea = {
                                hub().memory.write().unwrap().add_memory(&resident_id, witness_name, &mem_a)
                            }; // memory 寫鎖釋放
                            vmem::append_memory(&ea);
                            // 動態牆：讓玩家看見 AI 居民為彼此的成就道賀（鎖外 IO）。
                            vfeed::append_feed(
                                vwit::FEED_KIND,
                                witness_name,
                                &vwit::witness_feed_line(witness_name, rname),
                            );
                        }
                    }
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
                    "coin_price": offer.coin_price,
                }).to_string();
                let _ = out_tx.send(Message::Text(msg)).await;
            }

            Ok(ClientMsg::TradeAccept { resident_id, pay_with_coin }) => {
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
                // 付幣代替湊材料 v1（ROADMAP 874）：pay_with_coin 時改付 COIN_ID×coin_price
                // 代替湊 want_item×want_count，其餘流程（給 offer_item/記憶/台詞/Feed）共用
                // 同一套、只是「玩家實際付出的是什麼」不同——單一分流點，別在後面處處 if。
                let (pay_item, pay_count, pay_name): (u8, u32, &'static str) = if pay_with_coin {
                    (vcraft::COIN_ID, offer.coin_price, vtrade::item_name_zh(vcraft::COIN_ID))
                } else {
                    (offer.want_item, offer.want_count, wname)
                };
                // 6) 驗並扣玩家背包中 pay_item × pay_count（inventory 寫鎖即釋）。
                let taken = {
                    hub().inventory.write().unwrap()
                        .take(&name, pay_item, pay_count)
                };
                let Some(taken_entry) = taken else {
                    let msg = serde_json::json!({
                        "t": "trade_fail",
                        "reason": format!("背包裡的{}不夠（需要{}個）", pay_name, pay_count)
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
                let mem = if pay_with_coin {
                    vtrade::trade_memory_coin(&name, pay_count, oname)
                } else {
                    vtrade::trade_memory(&name, oname, wname)
                };
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
                let pay_remain = hub().inventory.read().unwrap().count(&name, pay_item);
                let offer_new = hub().inventory.read().unwrap().count(&name, offer.offer_item);
                let upd1 = serde_json::json!({
                    "t": "inv_update",
                    "block_id": pay_item,
                    "count": pay_remain,
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
                    "gave_item": pay_item,
                    "gave_name": pay_name,
                    "gave_count": pay_count,
                    "paid_with_coin": pay_with_coin,
                }).to_string();
                let _ = out_tx.send(Message::Text(done_msg)).await;
                // 玩家里程碑 v1（ROADMAP 724）：人生第一次與居民完成以物易物。
                try_unlock_milestone(&name, "first_trade", &out_tx);
                // 11) Feed（鎖外 IO）。
                vfeed::append_feed(
                    "交易",
                    rname,
                    &format!("{name}與{rname}交易：{pay_name}×{}→{oname}×{}", pay_count, offer.offer_count),
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
                // 玩家里程碑（自主提案切片，追上 692）：第一次把材料收進箱子。
                try_unlock_milestone(&name, "first_chest", &out_tx);
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
                // 居民認得你的家 v1（自主提案切片，ROADMAP 830）：伺服器權威記下這塊牌是哪位
                // 玩家立的——只有已登入帳號才記名（比照瓶中信的登入護欄），訪客的牌 owner 永遠
                // None，行為與今日完全一致；清空牌面（clean 為空）也不必記歸屬。
                let owner = if is_account && !clean.is_empty() { Some(name.clone()) } else { None };
                let ev = hub().sign.write().unwrap().set(&vsign::pos_key(x, y, z), clean.clone(), owner);
                vsign::append_sign(&ev);
                // 廣播給所有人（含自己），前端據此更新／移除該座標的浮字。
                broadcast_sign(x, y, z, &clean);
            }

            // ── 漂流瓶 v1：丟瓶（自主提案切片 825）───────────────────────────────────
            // 世界第一次有了「玩家↔玩家」的痕跡：合成一只空玻璃瓶、對準水面寫上一句話，
            // 之後另一位路過水邊的玩家會撿起它、讀到陌生旅人的匿名留言。
            Ok(ClientMsg::ThrowBottle { x, y, z, text }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                // 1) 觸及範圍內（沿用互動統一 reach）。
                if !voxel::in_reach(px, py, pz, x, y, z) { continue; }
                // 2) 目標要是水面（來源水或流動水，同垂釣/水桶判定）。
                let target_blk = voxel::effective_block_at(&hub().deltas.read().unwrap(), x, y, z);
                if !vfish::is_water_block(target_blk as u8) {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "bottle_fail", "reason": "要對準水面才能丟瓶喔。"
                    }).to_string())).await;
                    continue;
                }
                // 3) 瓶中信需登入（比照對話需登入護欄）：是留給陌生玩家的自由文字，
                //    只有登入帳號才能寫，訪客可自由撿讀（身分由後端 cookie 權威解出）。
                if !talk_allowed_for_identity(is_account) {
                    let mut players = hub().players.write().unwrap();
                    if let Some(p) = players.get_mut(&my_id) {
                        p.say = BOTTLE_GUEST_NOTICE.to_string();
                        p.say_timer = PLAYER_SAY_SECS;
                    }
                    continue;
                }
                // 4) 清洗文字；拒空（清洗後等於沒寫就別丟）。
                let clean = vbottle::sanitize_text(&text);
                if clean.is_empty() {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "bottle_fail", "reason": "瓶中信不能是空的喔，寫點什麼吧。"
                    }).to_string())).await;
                    continue;
                }
                // 5) 內容審查（治安三件套②同款）：命中→溫柔提示、絕不存檔／絕不廣播原文。
                {
                    let verdict = vmod::screen(&clean);
                    if verdict != vmod::Screen::Clean {
                        let mut players = hub().players.write().unwrap();
                        if let Some(p) = players.get_mut(&my_id) {
                            p.say = vmod::gentle_notice(verdict).to_string();
                            p.say_timer = PLAYER_SAY_SECS;
                        }
                        continue;
                    }
                }
                // 6) 手上要有空玻璃瓶（inventory 讀鎖即釋）。
                let has_bottle = hub().inventory.read().unwrap().count(&name, vbottle::BOTTLE_ID) >= 1;
                if !has_bottle {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "bottle_fail", "reason": "你手上沒有空玻璃瓶——先合成一個吧（2 玻璃）。"
                    }).to_string())).await;
                    continue;
                }
                let pos = vbottle::pos_key(x, y, z);
                // 7) 同座標已有瓶子、或全局瓶數已達上限 → 婉拒（bottle 讀鎖即釋，防無限堆積）。
                //    先算好結果、放開鎖，再送訊息——絕不持鎖跨 await（守鎖紀律）。
                let (pos_taken, at_cap) = {
                    let store = hub().bottle.read().unwrap();
                    (store.has(&pos), store.len() >= vbottle::MAX_ACTIVE_BOTTLES)
                };
                if pos_taken {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "bottle_fail", "reason": "這裡已經有一只瓶子了，換個地方丟吧。"
                    }).to_string())).await;
                    continue;
                }
                if at_cap {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "bottle_fail", "reason": "海上漂流瓶已經很多了，晚點再試試看。"
                    }).to_string())).await;
                    continue;
                }
                // 8) 扣一只空玻璃瓶（inventory 寫鎖即釋 → append 落地 → 單播新存量）。
                let Some(inv_e) = hub().inventory.write().unwrap().take(&name, vbottle::BOTTLE_ID, 1) else {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "bottle_fail", "reason": "你手上沒有空玻璃瓶——先合成一個吧（2 玻璃）。"
                    }).to_string())).await;
                    continue;
                };
                vinv::append_inv(&inv_e);
                let nc = hub().inventory.read().unwrap().count(&name, vbottle::BOTTLE_ID);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "inv_update", "block_id": vbottle::BOTTLE_ID, "count": nc
                }).to_string())).await;
                // 9) 存進 store + 落地持久化（bottle 寫鎖即釋 → IO 在鎖外）。
                let ev = hub().bottle.write().unwrap().set(&pos, clean);
                vbottle::append_bottle(&ev);
                // 玩家里程碑（自主提案切片，追上 825）：第一次把心裡話丟進漂流瓶。
                try_unlock_milestone(&name, "first_bottle", &out_tx);
                // 10) 廣播座標給所有人（絕不廣播內文）+ 單播成功提示。
                broadcast_bottle_dropped(x, y, z);
                let _ = out_tx.send(Message::Text(
                    serde_json::json!({ "t": "bottle_throw_ok" }).to_string(),
                )).await;
                // 11) 世界動態 feed：刻意匿名，不點名投瓶人是誰——保留「陌生旅人」的巧遇感。
                vfeed::append_feed("漂流瓶", "神秘的旅人", "把一封瓶中信丟進了海裡，不知道會漂向誰……");
            }

            // ── 漂流瓶 v1：撿瓶（自主提案切片 825）───────────────────────────────────
            // 讀不需要登入（訪客也能自由撿讀，只是不能寫）；一次性拾起，撿走後全場同步移除。
            Ok(ClientMsg::ReadBottle { x, y, z }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                if !voxel::in_reach(px, py, pz, x, y, z) { continue; }
                let pos = vbottle::pos_key(x, y, z);
                let claimed = { hub().bottle.write().unwrap().claim(&pos) };
                let Some((text, ev)) = claimed else {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "bottle_fail", "reason": "這裡沒有瓶子。"
                    }).to_string())).await;
                    continue;
                };
                vbottle::append_bottle(&ev);
                broadcast_bottle_removed(x, y, z);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "bottle_read", "text": text
                }).to_string())).await;
                // 世界動態 feed：同樣刻意匿名，不點名撿到的人是誰。
                vfeed::append_feed("漂流瓶", "一位旅人", "在岸邊撿起了一封漂流瓶……");
            }

            // ── 掉落物 v1：丟下手上一件材料（自主提案切片 828）───────────────────────
            // 玩家↔玩家至今僅有漂流瓶（825，非同步/文字）與並肩協作（827，被動加成）——
            // 本刀補上第一個主動的實體資源轉手：對著地面丟下一件材料，安靜留在原地，
            // 撿起在 `Move` handler 裡自動判定（走近即撿，含撿回自己剛丟下的東西）。
            Ok(ClientMsg::DropItem { x, y, z, item_id, count }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                if !voxel::in_reach(px, py, pz, x, y, z) { continue; }
                let want = vdrop::clamp_drop_count(count);
                // 1) 先查全局上限（真的扣背包**之前**檢查，避免物品憑空消失）。
                let at_cap = { hub().drops.read().unwrap().len() >= vdrop::MAX_ACTIVE_DROPS };
                if at_cap {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "drop_fail", "reason": "地上的掉落物已經很多了，晚點再試試看。"
                    }).to_string())).await;
                    continue;
                }
                // 2) 背包要有足量的該材料（讀鎖即釋）。
                let have = hub().inventory.read().unwrap().count(&name, item_id);
                if have < want {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "drop_fail", "reason": "你手上沒有那麼多喔。"
                    }).to_string())).await;
                    continue;
                }
                // 3) 扣下材料（inventory 寫鎖即釋 → append 落地 → 單播新存量）。
                let Some(inv_e) = hub().inventory.write().unwrap().take(&name, item_id, want) else {
                    continue;
                };
                vinv::append_inv(&inv_e);
                let nc = hub().inventory.read().unwrap().count(&name, item_id);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "inv_update", "block_id": item_id, "count": nc
                }).to_string())).await;
                // 4) 落地存進世界（世界座標＝瞄準格頂面），廣播給所有人。
                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let wx = x as f32 + 0.5;
                let wy = y as f32 + 1.0;
                let wz = z as f32 + 0.5;
                let spawned = { hub().drops.write().unwrap().spawn(wx, wy, wz, item_id, want, &name, now_secs) };
                if let Some(id) = spawned {
                    broadcast_item_dropped(id, wx, wy, wz, item_id, want, &name);
                    // 玩家里程碑（自主提案切片，追上 828）：第一次把材料親手轉交給另一位真人。
                    try_unlock_milestone(&name, "first_dropitem", &out_tx);
                } else {
                    // 極端競態（多人同時丟到上限）：退還材料，別讓東西憑空消失。
                    let refund = hub().inventory.write().unwrap().give(&name, item_id, want);
                    vinv::append_inv(&refund);
                    let nc2 = hub().inventory.read().unwrap().count(&name, item_id);
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "inv_update", "block_id": item_id, "count": nc2
                    }).to_string())).await;
                }
            }

            // ── 玩家自由市集 v1：擺攤（自主提案切片 832）────────────────────────────
            Ok(ClientMsg::StallOpen { x, y, z, give_item, give_count, want_item, want_count }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                if !voxel::in_reach(px, py, pz, x, y, z) { continue; }
                // 1) 瞄準格必須是空氣、腳下要是實心（攤位才站得住），比照掉落物/立牌落地判定。
                let placeable = {
                    let world = hub().deltas.read().unwrap();
                    let here = voxel::effective_block_at(&world, x, y, z);
                    let below = voxel::effective_block_at(&world, x, y - 1, z);
                    matches!(here, Block::Air) && below.is_solid()
                };
                if !placeable {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "stall_fail", "reason": "這裡不能擺攤，換個腳下有地面的空位吧。"
                    }).to_string())).await;
                    continue;
                }
                // 2) 給出/要求物品需有效（非 Air、彼此不同）。
                if !vstall::valid_stall_items(give_item, want_item) {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "stall_fail", "reason": "擺攤的物品不合法（不能兩邊選一樣的東西）。"
                    }).to_string())).await;
                    continue;
                }
                let gcount = vstall::clamp_stall_count(give_count);
                let wcount = vstall::clamp_stall_count(want_count);
                // 3) 先查全局上限 + 該座標是否已有攤位（真的扣背包**之前**檢查，避免材料憑空消失）。
                let (at_cap, pos_taken) = {
                    let store = hub().stalls.read().unwrap();
                    (store.len() >= vstall::MAX_ACTIVE_STALLS, store.has((x, y, z)))
                };
                if pos_taken {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "stall_fail", "reason": "這裡已經有一個攤位了，換個地方擺吧。"
                    }).to_string())).await;
                    continue;
                }
                if at_cap {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "stall_fail", "reason": "市集的攤位已經很多了，晚點再試試看。"
                    }).to_string())).await;
                    continue;
                }
                // 4) 背包要有足量的給出材料（讀鎖即釋）。
                let have = hub().inventory.read().unwrap().count(&name, give_item);
                if have < gcount {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "stall_fail", "reason": "你手上沒有那麼多喔。"
                    }).to_string())).await;
                    continue;
                }
                // 5) 扣下給出材料（escrow 進攤位；inventory 寫鎖即釋 → append 落地 → 單播新存量）。
                let Some(inv_e) = hub().inventory.write().unwrap().take(&name, give_item, gcount) else {
                    continue;
                };
                vinv::append_inv(&inv_e);
                let nc = hub().inventory.read().unwrap().count(&name, give_item);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "inv_update", "block_id": give_item, "count": nc
                }).to_string())).await;
                // 6) 存進世界（stalls 寫鎖即釋），廣播給所有人。
                let now_secs = vfarm::now_secs();
                let opened = {
                    hub().stalls.write().unwrap()
                        .open((x, y, z), give_item, gcount, want_item, wcount, &name, now_secs)
                };
                if opened {
                    broadcast_stall_open(x, y, z, give_item, gcount, want_item, wcount, &name);
                    try_unlock_milestone(&name, "first_market", &out_tx);
                } else {
                    // 極端競態（多人同時擺到同座標/上限）：退還材料，別讓東西憑空消失。
                    let refund = hub().inventory.write().unwrap().give(&name, give_item, gcount);
                    vinv::append_inv(&refund);
                    let nc2 = hub().inventory.read().unwrap().count(&name, give_item);
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "inv_update", "block_id": give_item, "count": nc2
                    }).to_string())).await;
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "stall_fail", "reason": "剛好被人搶先擺了，換個地方吧。"
                    }).to_string())).await;
                }
            }

            // ── 玩家自由市集 v1：互動（接手成交／收攤）（自主提案切片 832）─────────────
            Ok(ClientMsg::StallInteract { x, y, z }) => {
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                if !voxel::in_reach(px, py, pz, x, y, z) { continue; }
                // 1) 攤位是否存在（讀鎖即釋）。
                let exists = { hub().stalls.read().unwrap().has((x, y, z)) };
                if !exists {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "stall_fail", "reason": "這裡沒有攤位。"
                    }).to_string())).await;
                    continue;
                }
                // 2) 先偷看擁有者是誰（不移除），決定走「收攤」還是「接手」分支。
                let owner = { hub().stalls.read().unwrap().get((x, y, z)).map(|s| s.owner.clone()) };
                let Some(owner) = owner else { continue; };
                if owner == name {
                    // ── 收攤：只有擺攤者本人能收回，退還 escrow 材料 ──
                    let removed = { hub().stalls.write().unwrap().remove((x, y, z)) };
                    let Some(stall) = removed else {
                        let _ = out_tx.send(Message::Text(serde_json::json!({
                            "t": "stall_fail", "reason": "攤位剛好被人接手了，來不及收回。"
                        }).to_string())).await;
                        continue;
                    };
                    let refund = hub().inventory.write().unwrap().give(&name, stall.give_item, stall.give_count);
                    vinv::append_inv(&refund);
                    let nc = hub().inventory.read().unwrap().count(&name, stall.give_item);
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "inv_update", "block_id": stall.give_item, "count": nc
                    }).to_string())).await;
                    broadcast_stall_removed(x, y, z);
                    let _ = out_tx.send(Message::Text(
                        serde_json::json!({ "t": "stall_cancel_ok" }).to_string(),
                    )).await;
                    continue;
                }
                // ── 接手成交：需背包持有 want_item×want_count ──
                let (want_item, want_count) = {
                    let store = hub().stalls.read().unwrap();
                    match store.get((x, y, z)) {
                        Some(s) => (s.want_item, s.want_count),
                        None => continue,
                    }
                };
                let have = hub().inventory.read().unwrap().count(&name, want_item);
                if have < want_count {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "stall_fail", "reason": "你身上沒有這攤要換的東西喔。"
                    }).to_string())).await;
                    continue;
                }
                // 驗證通過才真的取出攤位（原子移除；race 時只有一人搶得到，另一人乾淨落空）。
                let removed = { hub().stalls.write().unwrap().remove((x, y, z)) };
                let Some(stall) = removed else {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "stall_fail", "reason": "攤位剛好被別人接手走了，晚一步。"
                    }).to_string())).await;
                    continue;
                };
                // 再驗一次接手者存量（雙重確認，防競態下夾在中間的極端狀況）；不足就整攤放回去。
                let have2 = hub().inventory.read().unwrap().count(&name, stall.want_item);
                if have2 < stall.want_count {
                    hub().stalls.write().unwrap().put_back(stall);
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "stall_fail", "reason": "你身上沒有這攤要換的東西喔。"
                    }).to_string())).await;
                    continue;
                }
                let Some(taken) = hub().inventory.write().unwrap().take(&name, stall.want_item, stall.want_count) else {
                    hub().stalls.write().unwrap().put_back(stall);
                    continue;
                };
                vinv::append_inv(&taken);
                let owner_credit = hub().inventory.write().unwrap().give(&stall.owner, stall.want_item, stall.want_count);
                vinv::append_inv(&owner_credit);
                let acceptor_credit = hub().inventory.write().unwrap().give(&name, stall.give_item, stall.give_count);
                vinv::append_inv(&acceptor_credit);
                let nc_want = hub().inventory.read().unwrap().count(&name, stall.want_item);
                let nc_give = hub().inventory.read().unwrap().count(&name, stall.give_item);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "inv_update", "block_id": stall.want_item, "count": nc_want
                }).to_string())).await;
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "inv_update", "block_id": stall.give_item, "count": nc_give
                }).to_string())).await;
                broadcast_stall_removed(x, y, z);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "stall_trade_ok", "got_item": stall.give_item, "got_count": stall.give_count,
                }).to_string())).await;
                try_unlock_milestone(&name, "first_market", &out_tx);
                vfeed::append_feed("自由市集", &name, &format!(
                    "在市集用材料和{}的攤位成交了一筆交易", stall.owner
                ));
                // 自由市集賣家通知 v1（自主提案切片，ROADMAP 864）：把「你的攤位被誰接手、換到
                // 了什麼」塞進賣家的待送達佇列，賣家下次連線時會收到私訊（見連線區塊接線）。
                {
                    let notice = vstallnotify::StallSaleNotice {
                        buyer: name.clone(),
                        got_item_name: vgift::item_name_zh(stall.want_item).to_string(),
                        got_count: stall.want_count,
                    };
                    vstallnotify::enqueue_sale(
                        &mut hub().stall_notices.write().unwrap(),
                        &stall.owner,
                        notice,
                    );
                } // stall_notices 寫鎖釋放
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
                // 溫柔重生 v1：記下這張床——日後血歸零時優先在這裡的床邊醒來。
                // 站到床頂面上方（by+1）＋格中心，避免重生卡進床方塊。
                last_bed = Some((x as f32 + 0.5, (y + 1) as f32, z as f32 + 0.5));
                let msg = serde_json::json!({ "t": "sleep_ok" }).to_string();
                let _ = out_tx.send(Message::Text(msg)).await;
                // 睡覺也順帶回滿血飢（一覺好眠，療癒世界）。
                heal_to_full_on_sleep(&name, &out_tx).await;
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
                // 雨天垂釣 v1（自主提案切片 841）：下雨時魚更活躍，上鉤等得更快（只獎不罰）。
                let raining = *hub().weather.read().unwrap();
                let now = vfarm::now_secs();
                let wait = vfish::bite_secs_for(rand::random::<u64>(), raining);
                let ready_at = now + wait;
                {
                    hub().pending_fish.write().unwrap().insert(name.clone(), (ready_at, x, y, z));
                }
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "fish_cast_ok", "wait": wait, "hint": vfish::cast_hint_for(raining)
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
                // 雨天垂釣 v1（自主提案切片 841）：下雨時稀有乙太魚機率提高（只獎不罰）。
                let raining = *hub().weather.read().unwrap();
                let fish_id = vfish::pick_catch_for(rand::random::<u64>(), raining);
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
                // 玩家熟練度 v1（自主提案切片，ROADMAP 842）：每次收竿累積🎣垂釣熟練度；
                // 練到 Lv.5 起額外多釣起一尾同種魚。
                let level = award_mastery(&name, MasteryKind::Fishing, &out_tx);
                give_mastery_bonus(&name, MasteryKind::Fishing, level, fish_id, &out_tx);
            }

            // QA 專用授予（只在 BUTFUN_QA_DEBUG=1 生效；正式線上直接忽略，無法刷物品）。
            Ok(ClientMsg::QaGrant { item_id, count }) => {
                if std::env::var("BUTFUN_QA_DEBUG").as_deref() != Ok("1") {
                    continue; // 未開 QA flag → 惰性忽略（濫用防護）
                }
                let n = count.min(64); // 上限保護
                let entry = hub().inventory.write().unwrap().give(&name, item_id, n);
                vinv::append_inv(&entry);
                let nc = hub().inventory.read().unwrap().count(&name, item_id);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "inv_update", "block_id": item_id, "count": nc
                }).to_string())).await;
            }

            Ok(ClientMsg::Eat { item_id }) => {
                // 親手煮的暖食自己也能享用 v1（779）。
                // 玩家生存指標 v1（溫和版·後端權威）：吃東西真的回復飢餓、扣背包。
                //   ── 可吃範圍擴大：不只熟食，所有食物（生穀/蔬果/魚/熟食/加工）都能填飽肚子
                //      （vstats::food_nutrition 定義），但只有「自己親手煮的熟食」才有那份暖意
                //      social 交織（is_edible_dish 才觸發居民感染／暖句）。
                // 1) 後端權威判定：是食物 ＆ 背包有 ＆ 沒吃飽 → 才吃。客戶端只發 item_id，不自報飢餓/血。
                let is_dish = vmeal::is_edible_dish(item_id);
                let have = hub().inventory.read().unwrap().count(&name, item_id);
                let cur_hunger = {
                    hub().player_stats.read().unwrap()
                        .get(&name).map(|s| s.hunger).unwrap_or(vstats::MAX_HUNGER)
                };
                let new_hunger = vstats::try_eat(item_id, have, cur_hunger);
                if new_hunger.is_none() {
                    let reason = if !vstats::is_edible(item_id) {
                        "這個沒法吃，先煮一道熱食吧～".to_string()
                    } else if have == 0 {
                        let iname = vgift::item_name_zh(item_id);
                        format!("背包裡沒有{iname}")
                    } else {
                        "你已經很飽了，吃不下～".to_string()
                    };
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "eat_fail", "reason": reason
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
                // 2b) 更新飢餓（後端權威）＋單播新指標。飽食下一次 stats tick 會自動緩慢回血。
                if let Some(nh) = new_hunger {
                    let new_stats = {
                        let mut m = hub().player_stats.write().unwrap();
                        let s = m.entry(name.clone()).or_insert_with(vstats::PlayerStats::default);
                        s.hunger = nh;
                        *s
                    };
                    let _ = out_tx.send(Message::Text(player_stats_msg(&new_stats))).await;
                    persist_player_stats();
                }
                let dish = vgift::item_name_zh(item_id);
                let pick = (vfarm::now_secs() as usize).wrapping_add(item_id as usize);
                // 3) 玩家自享的暖意回饋（純函式）＋回 inv_update / eat_ok。
                //    莓果醬是甜點、不是熱食，用甜味專屬暖句；其餘熟食走「熱騰騰」暖句（莓果醬 v1 ROADMAP 808）；
                //    生食／原料（非熟食）就只有樸實的「填了肚子」回饋，沒有那份精心料理的暖意。
                let cozy = if !is_dish {
                    String::new()
                } else if item_id == crate::voxel_berry::JAM_ID {
                    vmeal::savor_sweet_line(pick)
                } else {
                    vmeal::savor_self_line(dish, pick)
                };
                let remain = hub().inventory.read().unwrap().count(&name, item_id);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "inv_update", "block_id": item_id, "count": remain,
                }).to_string())).await;
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "eat_ok", "item_id": item_id, "item_name": dish, "line": cozy,
                }).to_string())).await;
                // 4) 里程碑：人生第一次嚐一口自己親手煮的料理（只算熟食，生嚼原料不算）。
                if is_dish {
                    try_unlock_milestone(&name, "first_taste", &out_tx);
                }
                // 5) 交織點：只有熟食才有那份「暖意分享」——若剛好站在某位居民身邊、且分享冷卻就緒，
                //    居民被你的滿足感染（心情點亮＋暖泡泡＋交情記憶＋動態牆）。生食不觸發社交交織。
                let share_ready = is_dish && last_eat_share
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

            Ok(ClientMsg::FireworkLaunch) => {
                // 乙太煙火 v1（785）：朝夜空施放一束煙火，火花在頭頂綻放、附近居民抬頭歡呼。
                // 1) 每連線冷卻——擋連放洗爆全場畫面（濫用防護①；冷卻未到不消耗煙火）。
                let ready = last_firework
                    .map(|t| t.elapsed().as_secs_f32() >= vfw::FIREWORK_COOLDOWN_SECS)
                    .unwrap_or(true);
                if !ready {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "firework_fail", "reason": "煙火還在冷卻，稍等一下再放～"
                    }).to_string())).await;
                    continue;
                }
                // 2) 取施放者當前位置（players 讀鎖即釋；客戶端不自報座標＝火花只在施放者頭頂綻放，
                //    防偽造他人施放位置，濫用防護③權限由後端權威判定）。
                let Some((px, py, pz)) = player_pos(my_id) else { continue; };
                // 3) 驗並消耗一份煙火（inventory 寫鎖即釋）——放不了就白嫖不到（濫用防護②）。
                let taken = { hub().inventory.write().unwrap().take(&name, vfw::FIREWORK_ID, 1) };
                let Some(inv_entry) = taken else {
                    let _ = out_tx.send(Message::Text(serde_json::json!({
                        "t": "firework_fail",
                        "reason": "背包裡沒有乙太煙火——在工作台用乙太礦＋煤礦＋沙做一束吧。"
                    }).to_string())).await;
                    continue;
                };
                vinv::append_inv(&inv_entry);
                last_firework = Some(std::time::Instant::now());
                // 4) 選火花配色（真隨機，每次顏色有變化）＋回報新存量＋施放回饋（單播）。
                let palette = vfw::firework_palette(rand::random::<u64>());
                let pick = (vfarm::now_secs() as usize).wrapping_add(palette as usize);
                let remain = hub().inventory.read().unwrap().count(&name, vfw::FIREWORK_ID);
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "inv_update", "block_id": vfw::FIREWORK_ID, "count": remain,
                }).to_string())).await;
                let _ = out_tx.send(Message::Text(serde_json::json!({
                    "t": "firework_ok", "line": vfw::launch_self_line(pick),
                }).to_string())).await;
                // 5) 廣播火花給全場（火花在施放者頭頂夜空綻放，人人看到同色）。
                broadcast_firework(px, py, pz, palette);
                // 6) 里程碑：人生第一次朝夜空施放煙火。
                try_unlock_milestone(&name, "first_firework", &out_tx);
                // 7) 城鎮動態牆（不在場的人回來也讀得到這份熱鬧）。
                vfeed::append_feed("煙火", &name, &vfw::launch_feed_line(&name));
                // 8) 附近醒著、有空反應的居民抬頭歡呼（心情點亮＋暖泡泡＋交情記憶）。
                //    只挑半徑內、未睡、say 空者（比照 780 彩虹齊發，但受半徑天然節流）。
                let cheerers: Vec<String> = {
                    let residents = hub().residents.read().unwrap();
                    residents
                        .iter()
                        .filter(|r| !r.asleep && r.say.is_empty())
                        .filter_map(|r| {
                            let dx = px - r.body.x;
                            let dz = pz - r.body.z;
                            if dx * dx + dz * dz <= vfw::CHEER_RADIUS * vfw::CHEER_RADIUS {
                                Some(r.id.clone())
                            } else {
                                None
                            }
                        })
                        .collect()
                }; // residents 讀鎖釋放
                if !cheerers.is_empty() {
                    let mut cheer_mems: Vec<(String, String)> = Vec::new(); // (rid, summary)
                    {
                        let mut residents = hub().residents.write().unwrap();
                        for (i, rid) in cheerers.iter().enumerate() {
                            if let Some(r) = residents.iter_mut().find(|r| &r.id == rid) {
                                if r.say.is_empty() {
                                    r.say = vfw::cheer_line(pick.wrapping_add(i))
                                        .chars()
                                        .take(vfw::CHEER_SAY_MAX_CHARS)
                                        .collect();
                                    r.say_timer = SAY_SECS;
                                    r.mood_boost_secs =
                                        r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                    // 訪客（名空白）不記交情。
                                    if !name.is_empty() {
                                        cheer_mems
                                            .push((rid.clone(), vfw::cheer_memory_line(&name)));
                                    }
                                }
                            }
                        }
                    } // residents 寫鎖釋放
                    broadcast_players();
                    // 交情記憶：memory 寫鎖即釋、append IO 在鎖外，守死鎖鐵律。
                    for (rid, summary) in &cheer_mems {
                        let entry = hub().memory.write().unwrap().add_memory(rid, &name, summary);
                        vmem::append_memory(&entry);
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

    // 玩家生存指標持久化：斷線前把最後血/飢落地（登入玩家重登保留最新狀態）。
    // 訪客的 stats 留在記憶體、下次重啟／清空即散（session 內有效即可），此存檔只影響登入玩家。
    // 存檔以玩家名為鍵，訪客名不穩定不會誤蓋登入玩家（比照 inventory 綁名慣例）。
    if account_email.is_some() {
        persist_player_stats(); // IO 在鎖外（函式內短取讀鎖組快照即釋）
    }

    // 收攤：移除玩家、廣播、收掉任務。
    // 垂釣 v1：清掉這位玩家進行中的拋竿（純記憶體，斷線即散）。
    { hub().pending_fish.write().unwrap().remove(&name); }
    forward.abort();
    cleanup(my_id, &writer);
    broadcast_players();
}

/// 連線名額守衛（治安三件套②）：`handle_socket` 任一離開路徑（早退／正常收攤／panic 展開）
/// 都會在此 drop，自動釋放這條連線在 per-IP 連線數計數器裡佔的名額。`active==false`（豁免的
/// localhost/QA 連線）則不佔名額、drop 時也不動計數，行為一致。短鎖即釋、不 await（守鎖紀律）。
struct ConnSlotGuard {
    ip: String,
    active: bool,
}
impl Drop for ConnSlotGuard {
    fn drop(&mut self) {
        if self.active {
            ip_conn_limiter().lock().unwrap().release(&self.ip);
        }
    }
}

/// 把玩家移出登錄、清除踢信號、並中止 writer task。
/// **鎖紀律**：players 寫鎖短取即釋，conn_kick 寫鎖另一把短取即釋，不巢狀。
fn cleanup(id: Uuid, writer: &tokio::task::JoinHandle<()>) {
    {
        let mut players = hub().players.write().unwrap();
        players.remove(&id);
    }
    // 同帳號去重：清掉自己的踢信號（連線已結束，不再需要信號）。
    // 若新連線已提前移走此 entry（dedup 時），remove 是 no-op，安全。
    { hub().conn_kick.write().unwrap().remove(&id); }
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
            tick_wildlife(RESIDENT_DT); // 野兔 v1：同節拍，各自獨立鎖，不與居民鎖巢狀。
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
            tick_berry(); // 莓果叢 v1（ROADMAP 806）：同節拍檢查莓果叢是否結果。
            tick_coop(); // 雞舍生蛋 v1（自主提案切片）：同節拍檢查雞舍是否生蛋。
            tick_smelt(); // 熔爐煨煮 v1（自主提案）：同節拍交付熟成的爐（成品入背包 + 廣播）。
            maybe_birth(); // 人口成長 v1：低頻檢查聚落是否有餘裕誕生一位新居民。
            maybe_breed_rabbits(); // 馴服兔子生寶寶 v1（自主提案切片 855）：同節拍檢查是否誕生一隻小兔子。
            maybe_pet_admire(); // 居民注意到你身邊跟著的馴服動物 v1（自主提案切片 875）：同節拍檢查身邊有無寵物觸發讚賞。
            maybe_proximity_teach(); // 就地指導 v1（自主提案切片）：同節拍檢查有無卡關居民身邊剛好站著會解法的老朋友。
            tick_dropitem_expire(); // 掉落物 v1（自主提案切片 828）：同節拍清掉沒人撿的過期掉落物。
            tick_stall_expire(); // 玩家自由市集 v1（自主提案切片 832）：同節拍清掉逾時沒人接手的攤位、退還材料。
        }
    });
    spawn_water_tick();
}

/// 掉落物 v1（自主提案切片 828）：清掉超過 `DESPAWN_SECS` 沒被撿走的掉落物，廣播移除
/// （消散不點名是誰丟的，安靜地消失即可，比照漂流瓶消散精神）。
fn tick_dropitem_expire() {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let expired = hub().drops.write().unwrap().expire(now_secs);
    for item in expired {
        broadcast_item_removed(item.id);
    }
}

/// 玩家自由市集 v1（自主提案切片 832）：清掉超過 `STALL_TTL_SECS` 沒人接手的攤位，
/// 材料退還擺攤者（哪怕擺攤者早已離線，`give` 不需要對方在線），並廣播移除浮標。
fn tick_stall_expire() {
    let now_secs = vfarm::now_secs();
    let expired = hub().stalls.write().unwrap().expire(now_secs);
    for stall in expired {
        let refund = hub().inventory.write().unwrap().give(&stall.owner, stall.give_item, stall.give_count);
        vinv::append_inv(&refund);
        broadcast_stall_removed(stall.x, stall.y, stall.z);
    }
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
    let mut newcomer = build_resident(new_i, body.x, body.z, body, now, parent_name.to_string());
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

/// 工作台/熔爐配方伺服器閘門 v1（自主提案切片）：判定玩家目前位置附近是否真的有
/// 指定種類的方塊。掃描範圍比照 [`is_irrigated_in_delta`] 同款手法，取比觸及範圍
/// 略寬鬆的半徑——玩家開面板後在原地小範圍走動、或站在略高/略低的地形上仍算數，
/// 不苛求分毫不差貼著方塊。垂直範圍較窄（±2），故站在同層樓面之外（如樓上/樓下
/// 隔了好幾層）的工作台/熔爐不會被誤判為「在附近」，屬已知的近似（比照 865 的
/// wait_timer 近似精神：純度數上的簡化，不影響正確性或安全）。
const STATION_RANGE_XZ: i32 = 4;
const STATION_RANGE_Y: i32 = 2;
fn station_nearby(deltas: &voxel::WorldDelta, px: f32, py: f32, pz: f32, want: Block) -> bool {
    let (cx, cy, cz) = (px.floor() as i32, py.floor() as i32, pz.floor() as i32);
    for dx in -STATION_RANGE_XZ..=STATION_RANGE_XZ {
        for dz in -STATION_RANGE_XZ..=STATION_RANGE_XZ {
            for dy in -STATION_RANGE_Y..=STATION_RANGE_Y {
                if voxel::effective_block_at(deltas, cx + dx, cy + dy, cz + dz) == want {
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
        // 雨後彩虹 v1（ROADMAP 780）：更新彩虹剩餘 tick（雨→晴升起、持續晴天逐 tick 淡出），
        // 並偵測「彩虹剛升起（0→>0）」設一次性旗標，供 tick_residents 觸發居民歡呼。短寫鎖即釋、不巢狀。
        {
            let mut rb = hub().rainbow_ticks.write().unwrap();
            let prev = *rb;
            *rb = vweather::next_rainbow(prev, was_raining, *w);
            if prev == 0 && *rb > 0 {
                *hub().rainbow_started_flag.write().unwrap() = true;
            }
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

/// 莓果叢結果 tick（莓果叢 v1·ROADMAP 806）——與 `tick_grove` 同 15 秒節拍。
/// 鎖序：① berry 讀鎖取「已結果」座標即釋。② 每叢：只在該格「還是莓果叢苗(75)」時
/// 換成結果的莓果叢(76)（防玩家已挖走／被別的方塊蓋掉時憑空長出），換好即把該格從
/// berry store 移除（結果狀態不需再計時，採收時再重新登記回退計時）。零鎖巢狀、零 IO。
fn tick_berry() {
    let now = vfarm::now_secs();
    // ① berry 讀鎖取已結果座標，馬上釋放。
    let ripe: Vec<(i32, i32, i32)> = hub().berry.read().unwrap().ripe_bushes(now);
    if ripe.is_empty() {
        return;
    }
    for (bx, by, bz) in ripe {
        // ② 短讀鎖確認該格仍是莓果叢苗（玩家可能已挖走／回退前被覆蓋），不是就跳過並清記錄。
        let still_bush = {
            let deltas = hub().deltas.read().unwrap();
            voxel::effective_block_at(&deltas, bx, by, bz) == Block::BerryBush
        };
        // 不論長成與否，先把這輪計時記錄清掉（避免下輪重複觸發）；長成後由採收重新登記回退計時。
        hub().berry.write().unwrap().remove(bx, by, bz);
        if !still_bush {
            continue;
        }
        {
            let mut world = hub().deltas.write().unwrap();
            voxel::set_block(&mut world, bx, by, bz, Block::BerryBushRipe);
        } // delta 寫鎖即釋
        broadcast_block(bx, by, bz, Block::BerryBushRipe);
    }
}

/// 雞舍生蛋 tick（雞舍生蛋 v1·自主提案切片）——與 `tick_berry` 同 15 秒節拍、同鎖序精神。
/// 鎖序：① coop 讀鎖取「已生蛋」座標即釋。② 每座：只在該格「還是空雞舍(80)」時
/// 換成有蛋的雞舍(81)（防玩家已拆走／被別的方塊蓋掉時憑空生蛋），換好即把該格從
/// coop store 移除（生蛋狀態不需再計時，收蛋時再重新登記回退計時）。零鎖巢狀、零 IO。
fn tick_coop() {
    let now = vfarm::now_secs();
    // ① coop 讀鎖取已生蛋座標，馬上釋放。
    let ready: Vec<(i32, i32, i32)> = hub().coop.read().unwrap().ready_coops(now);
    if ready.is_empty() {
        return;
    }
    for (cx, cy, cz) in ready {
        // ② 短讀鎖確認該格仍是空雞舍（玩家可能已拆走／回退前被覆蓋），不是就跳過並清記錄。
        let still_coop = {
            let deltas = hub().deltas.read().unwrap();
            voxel::effective_block_at(&deltas, cx, cy, cz) == Block::Coop
        };
        // 不論生蛋與否，先把這輪計時記錄清掉（避免下輪重複觸發）；生蛋後由收蛋重新登記回退計時。
        hub().coop.write().unwrap().remove(cx, cy, cz);
        if !still_coop {
            continue;
        }
        {
            let mut world = hub().deltas.write().unwrap();
            voxel::set_block(&mut world, cx, cy, cz, Block::CoopReady);
        } // delta 寫鎖即釋
        broadcast_block(cx, cy, cz, Block::CoopReady);
    }
}

/// `GET /voxel/diary` — 回傳所有居民的日記頁（curated 內心反思 + 當前心願）。
/// 日記是「瞥見居民沒說出口的內心」，不是聊天謄本：記憶在 `voxel_diary` 內被昇華成
/// 第一人稱反思、同主題收斂降噪、永不倒出玩家原話 / 玩家名（隱私邊界）。
/// 短鎖讀取快照 → drop 鎖 → 格式化 → 回 JSON；零 LLM、零持久化、零 migration。
/// 呼叫端（瀏覽器）直接 `fetch("/voxel/diary")` 即可，無需任何認證。
///
/// **居民察覺你翻過她的日記 v1（自主提案切片）**：可選 `?player=&resident=` 兩參數——都非空
/// 且 `resident` 命中一位真實居民時，記下「這位玩家翻過我的日記、還沒被我發現」的待發現旗標
/// （見 `voxel_diary_peek`），下次她打招呼時有機率點破。只有**點開單一居民日記面板**（意圖明確）
/// 才會觸發；日記牆一次讀全部居民、刻意不夾帶 `resident`，不觸發（避免掃過日記牆就被人人抓包）。
/// 缺任一參數或 `resident` 沒命中任何居民 → 靜默略過，回傳內容不受影響，向後相容舊前端。
pub async fn voxel_diary_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::http::header;

    // 1) 短鎖快照居民 id/name → drop（循序取鎖、不巢狀、守鎖紀律）。
    let resident_ids: Vec<(String, &'static str)> = {
        let rs = hub().residents.read().unwrap();
        rs.iter().map(|r| (r.id.clone(), r.name)).collect()
    };

    // 1b) 居民察覺你翻過她的日記 v1：`player`+`resident` 皆非空且命中真實居民 → 短寫鎖插旗，
    // 有界成長（`MAX_PEEK_ENTRIES_PER_RESIDENT`）防洗版，插滿靜默略過不 panic、不報錯。
    let peek_player = params.get("player").map(|s| s.trim()).unwrap_or("");
    let peek_resident = params.get("resident").map(|s| s.trim()).unwrap_or("");
    if !peek_player.is_empty() && !peek_resident.is_empty() {
        let mut rs = hub().residents.write().unwrap();
        if let Some(r) = rs.iter_mut().find(|r| r.id == peek_resident) {
            if r.diary_peeked.contains(peek_player)
                || r.diary_peeked.len() < vdiarypeek::MAX_PEEK_ENTRIES_PER_RESIDENT
            {
                r.diary_peeked.insert(peek_player.to_string());
            }
        }
    } // residents 寫鎖釋放

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
    // 戀愛心動 v1（ROADMAP 846）：romance 讀鎖獨立快照後即釋放，不與 bonds 鎖巢狀。
    let romance = hub().romance.read().unwrap().to_entries();
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
                let name_a = resident_name_of(&id_a);
                let name_b = resident_name_of(&id_b);
                let sweetheart = romance.iter().any(|e| {
                    (e.id_a == name_a && e.id_b == name_b) || (e.id_a == name_b && e.id_b == name_a)
                });
                out.push(serde_json::json!({
                    "a": name_a,
                    "b": name_b,
                    "tier": vbonds::tier_key(tier),
                    "visits": bonds.visit_count(name_a, name_b),
                    "sweetheart": sweetheart,
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

/// 乙太方界·小圈子攤開（自主提案切片，接續 708 交情網 + 711 小圈子聚會）：
///
/// 711 讓互為老朋友的居民不時相約碰面，靠的正是 `find_friend_cliques` 這個「看整張
/// 關係網、找出彼此皆為老朋友的一群」的判定——但這份判定至今只活在伺服器內部驅動
/// 聚會行為，玩家從沒有任何管道能直接看見「這幾位其實是感情要好的一群」，交情網
/// （708）也只攤開兩兩之間的數字，看不出誰跟誰其實是一夥的。本端點純讀取、零副
/// 作用，直接複用 `voxel_clique::find_friend_cliques`（與觸發聚會用的同一份邏輯），
/// 把 id 換成顯示名後回傳；不是新造一套群體判定，是讓早已存在的系統第一次被看見。
pub async fn voxel_cliques_handler() -> axum::response::Response {
    use axum::http::header;
    let cliques: Vec<Vec<&'static str>> = {
        // 短讀鎖一次性算完全部圈子 → 立即釋放，不與其他鎖巢狀（比照交情網手法）。
        let bonds = hub().bonds.read().unwrap();
        let n = resident_count();
        let ids: Vec<String> = (0..n).map(|i| format!("vox_res_{i}")).collect();
        vclique::find_friend_cliques(&ids, |a, b| resident_tier_of(&bonds, a, b))
            .into_iter()
            .map(|group| group.iter().map(|id| resident_name_of(id)).collect())
            .collect()
    };
    let body = serde_json::to_string(&cliques).unwrap_or_else(|_| "[]".into());
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
        // 短讀鎖一次性快照全體居民的技能清單 → 立即釋放，不與其他鎖巢狀。
        let invented = hub().invented.read().unwrap();
        (0..resident_count())
            .map(|i| {
                let rid = format!("vox_res_{i}");
                // 師承鏈可見（技能互教·北極星第四刀）：每筆技能連同「來歷」一起攤開——
                // 自己發明／承自XX（親子）／師承XX（教學），村裡的知識系譜第一次看得見。
                // `skills`（純名字陣列）保留不動，既有前端/QA 向後相容；`lineage` 新增並列。
                let lineage: Vec<serde_json::Value> = invented
                    .records_for(&rid)
                    .into_iter()
                    .map(|r| {
                        serde_json::json!({
                            "name": r.name,
                            "origin": vinvent::lineage_label(r),
                        })
                    })
                    .collect();
                serde_json::json!({
                    "name": resident_name_of(&rid),
                    "skills": invented.names_for(&rid),
                    "lineage": lineage,
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

/// 乙太方界·玩家獨門配方（居民教你一道獨門配方 v1，自主提案切片，ROADMAP 849）：
/// 回傳全部獨門配方定義 + 這位玩家各自是否已被居民教過。
///
/// 比照 708 交情網／719 技能簿／724 里程碑同一手法：純讀取、零副作用，`?player=`
/// 缺省時全部一律回「未學會」；前端合成台用這份清單決定哪些獨門配方要顯示可合成。
pub async fn voxel_known_recipes_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::http::header;
    let player_name = params.get("player").map(|s| s.as_str()).unwrap_or("").trim().to_string();
    let rows: Vec<serde_json::Value> = {
        // 短讀鎖一次性快照這位玩家已學會的獨門配方集合 → 立即釋放，不與其他鎖巢狀。
        let store = hub().player_recipes.read().unwrap();
        vcraft::TAUGHT_RECIPES
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "name_zh": r.name_zh,
                    "known": !player_name.is_empty() && store.knows(&player_name, r.id),
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

/// 乙太方界·個人路標 v1（自主提案切片，ROADMAP 869）：回傳這位玩家目前所有路標
/// （名字＋座標，依插旗先後）。跟里程碑/探索紀事端點同一手法：`?player=` 缺省時回空
/// 清單，供開面板時先拉一份現況；之後的即時更新走 WS `waypoint_sync`。純讀取、零副作用。
pub async fn voxel_waypoints_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::http::header;
    let player_name = params.get("player").map(|s| s.as_str()).unwrap_or("").trim().to_string();
    let items: Vec<serde_json::Value> = if player_name.is_empty() {
        Vec::new()
    } else {
        hub().waypoints.read().unwrap().list(&player_name)
            .iter()
            .map(|w| serde_json::json!({ "label": w.label, "x": w.x, "y": w.y, "z": w.z }))
            .collect()
    };
    let body = serde_json::to_string(&items).unwrap_or_else(|_| "[]".into());
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// 乙太方界·探索紀事 v1（自主提案切片，接續 838/839）：回傳這位玩家找到過的地標
/// （種類＋座標，依發現順序）＋分類小計。跟里程碑端點同一手法：`?player=` 缺省時
/// 回空清單，前端知道要先問玩家名再開面板。純讀取、零副作用。
pub async fn voxel_discoveries_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::http::header;
    let player_name = params.get("player").map(|s| s.as_str()).unwrap_or("").trim().to_string();
    let (items, ruins, springs, outposts) = if player_name.is_empty() {
        (Vec::new(), 0usize, 0usize, 0usize)
    } else {
        // 短讀鎖一次性快照這位玩家的探索紀事 → 立即釋放，不與其他鎖巢狀。
        let store = hub().discovery.read().unwrap();
        let list: Vec<serde_json::Value> = store
            .list_for(&player_name)
            .iter()
            .map(|e| {
                serde_json::json!({
                    "kind": e.kind.wire_id(),
                    "label": e.kind.label(),
                    "icon": e.kind.icon(),
                    "x": e.x,
                    "y": e.y,
                    "z": e.z,
                })
            })
            .collect();
        let (r, s, o) = store.counts_for(&player_name);
        (list, r, s, o)
    };
    let body = serde_json::json!({ "items": items, "ruins": ruins, "springs": springs, "outposts": outposts }).to_string();
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// 乙太方界·玩家熟練度 v1（自主提案切片，ROADMAP 842）：回傳這位玩家三條熟練度目前的
/// 經驗值／等級／稱號／是否已解鎖產出加成，供前端 📈 熟練度面板顯示。
pub async fn voxel_mastery_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::http::header;
    let player_name = params.get("player").map(|s| s.as_str()).unwrap_or("").trim().to_string();
    let rows: Vec<serde_json::Value> = {
        // 短讀鎖一次性快照這位玩家的熟練度 → 立即釋放，不與其他鎖巢狀。
        let store = hub().mastery.read().unwrap();
        MasteryKind::ALL
            .iter()
            .map(|&k| {
                let xp = if player_name.is_empty() { 0 } else { store.xp_for(&player_name, k) };
                let level = vmastery::level_for_xp(xp);
                serde_json::json!({
                    "kind": k.as_str(),
                    "name_zh": k.display_name_zh(),
                    "icon": k.icon(),
                    "xp": xp,
                    "level": level,
                    "title": vmastery::title_for_level(level),
                    "next_level_xp": (level.min(vmastery::MAX_LEVEL - 1) + 1) * vmastery::LEVEL_XP_STEP,
                    "bonus_unlocked": vmastery::mastery_yield_bonus(level) > 0,
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

/// 乙太方界·村莊地圖 v1（自主提案切片，ROADMAP 837）：回傳村莊中心＋廣場/主路尺寸常數＋
/// 每塊沿路地塊（座標＋認領者，未認領則 None）。
///
/// 村莊系統（835）早就把居民的家收攏成中央廣場＋十字主路＋沿路地塊的實體佈局，玩家也能
/// 走在真正鋪好的石板路上——但那份佈局只活在腳下：玩家從沒有任何管道能一眼看到「村子
/// 多大、廣場在哪、誰住哪塊地」，只能靠雙腳一格格丈量。跟 708 交情網／719 技能簿同一手法：
/// 讓早已存在的系統第一次被看見，而非新造一套村莊系統；本端點純讀取、零副作用。
pub async fn voxel_village_map_handler() -> axum::response::Response {
    use axum::http::header;
    // 村莊中心：優先用一次性整理時釘死的中心，缺（村莊尚未規劃）才退回即時質心——
    // 與 `claim_or_reuse_plot` 同一套判斷，確保地圖與居民實際認領的地塊完全對齊。
    let (vcx, vcz) = match vvillage::load_village_center() {
        Some(c) => c,
        None => {
            let home_bases: Vec<(i32, i32)> = {
                let residents = hub().residents.read().unwrap();
                residents
                    .iter()
                    .map(|r| (r.home_x.floor() as i32, r.home_z.floor() as i32))
                    .collect()
            }; // residents 讀鎖釋放
            vvillage::village_center(&home_bases)
        }
    };
    let layout = vvillage::plot_layout(vcx, vcz); // 純函式、鎖外算
    let plots: Vec<serde_json::Value> = {
        // 短讀鎖一次性反查每塊地的認領者 → 立即釋放，不與其他鎖巢狀。
        let village = hub().village.read().unwrap();
        layout
            .iter()
            .map(|p| {
                serde_json::json!({
                    "cx": p.cx,
                    "cz": p.cz,
                    "resident": village.resident_at(p.cx, p.cz).map(resident_name_of),
                })
            })
            .collect()
    };
    let body = serde_json::json!({
        "cx": vcx,
        "cz": vcz,
        "plaza_radius": vvillage::PLAZA_RADIUS,
        "road_reach": vvillage::ROAD_REACH,
        "plots": plots,
    })
    .to_string();
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// 環境生物 tick（野兔 v1 ROADMAP 847 ＋水中游魚 v1 ROADMAP 848 ＋放養雞 v1 ROADMAP 870）：
/// 與 `tick_residents` 同節拍（10Hz）。純點綴生物——沒有思考/記憶/社交；野兔閒晃＋見到玩家
/// 靠近就受驚逃開，魚只在自己的水塘裡悠游（不怕人、無陸地碰撞），雞跟野兔一樣不怕人，
/// 已馴服的雞會跟隨並定期下蛋（見 `voxel_wildlife`/`voxel_fish`/`voxel_chicken` 模組說明）。
/// 鎖序：`players`(read) → `deltas`(read) → `wildlife`(write，內層區塊釋放) → `drops`(write，
/// 落雞蛋，鎖已釋放後才取)，循序取放、與 `residents` 鎖各自獨立、不巢狀（守 prod 死鎖鐵律）。
fn tick_wildlife(dt: f32) {
    // 玩家座標快照（短鎖即釋，不與 wildlife 鎖巢狀）。野兔/雞需要，魚不怕人不查。
    let player_pts: Vec<(f32, f32)> = {
        let players = hub().players.read().unwrap();
        players.values().map(|p| (p.x, p.z)).collect()
    }; // 玩家讀鎖在此釋放
    let world = hub().deltas.read().unwrap();
    // 這一 tick 該下蛋的已馴服雞座標，留到 wildlife 寫鎖釋放後才落地掉落物（不巢狀鎖）。
    let mut egg_layers: Vec<(f32, f32, f32)> = Vec::new();
    {
    let mut animals = hub().wildlife.write().unwrap();
    for a in animals.iter_mut() {
        let (bx, bz) = (a.body.x, a.body.z);
        match a.kind {
            WildlifeKind::Rabbit => {
                // 找最近玩家（沒有玩家在線 = 視為無限遠，永遠不受驚）。
                let nearest = player_pts.iter().min_by(|(x1, z1), (x2, z2)| {
                    let d1 = (a.body.x - x1).powi(2) + (a.body.z - z1).powi(2);
                    let d2 = (a.body.x - x2).powi(2) + (a.body.z - z2).powi(2);
                    d1.partial_cmp(&d2).unwrap_or(std::cmp::Ordering::Equal)
                });
                let nearest_dist_sq = nearest
                    .map(|(px, pz)| (a.body.x - px).powi(2) + (a.body.z - pz).powi(2))
                    .unwrap_or(f32::MAX);
                // 已馴服的兔子走完全不同的分支：不再受驚，改成跟隨附近玩家
                // （馴服兔子跟隨你 v1，自主提案切片，ROADMAP 851）。
                if a.tamed {
                    a.fleeing = false;
                    a.following = vwild::should_follow(a.following, nearest_dist_sq);
                    if a.following {
                        if let Some((px, pz)) = nearest {
                            if vwild::should_close_follow_gap(nearest_dist_sq) {
                                vr::step_toward(&world, &mut a.body, *px, *pz, dt, vwild::FOLLOW_SPEED);
                            } else {
                                vr::gravity_step(&world, &mut a.body, dt);
                            }
                        }
                        a.wait_timer = 0.0;
                    } else if a.wait_timer > 0.0 {
                        a.wait_timer -= dt;
                        vr::gravity_step(&world, &mut a.body, dt);
                    } else {
                        let reached = vr::step_toward(
                            &world, &mut a.body, a.target_x, a.target_z, dt, vwild::WANDER_SPEED,
                        );
                        if reached {
                            let angle = rand::random::<f32>() * std::f32::consts::TAU;
                            let radius = vwild::WANDER_MIN_R
                                + rand::random::<f32>() * (vwild::WANDER_MAX_R - vwild::WANDER_MIN_R);
                            let (tx, tz) = vr::wander_target(a.home_x, a.home_z, angle, radius);
                            a.target_x = tx;
                            a.target_z = tz;
                            a.wait_timer = 0.5 + rand::random::<f32>() * 1.5;
                        }
                    }
                    if let Some(yaw) = vr::yaw_from_move(a.body.x - bx, a.body.z - bz) {
                        a.yaw = yaw;
                    }
                    continue;
                }
                a.fleeing = vwild::should_flee(a.fleeing, nearest_dist_sq);
                if a.fleeing {
                    // 受驚中：每 tick 重算逃跑方向（玩家持續逼近時，逃跑目標跟著即時調整）。
                    if let Some((px, pz)) = nearest {
                        let (tx, tz) = vwild::flee_target(a.body.x, a.body.z, *px, *pz);
                        a.target_x = tx;
                        a.target_z = tz;
                    }
                    vr::step_toward(&world, &mut a.body, a.target_x, a.target_z, dt, vwild::FLEE_SPEED);
                    a.wait_timer = 0.0; // 受驚時不歇息。
                } else if a.wait_timer > 0.0 {
                    a.wait_timer -= dt;
                    vr::gravity_step(&world, &mut a.body, dt);
                } else {
                    let reached =
                        vr::step_toward(&world, &mut a.body, a.target_x, a.target_z, dt, vwild::WANDER_SPEED);
                    if reached {
                        let angle = rand::random::<f32>() * std::f32::consts::TAU;
                        let radius = vwild::WANDER_MIN_R
                            + rand::random::<f32>() * (vwild::WANDER_MAX_R - vwild::WANDER_MIN_R);
                        let (tx, tz) = vr::wander_target(a.home_x, a.home_z, angle, radius);
                        a.target_x = tx;
                        a.target_z = tz;
                        a.wait_timer = 0.5 + rand::random::<f32>() * 1.5;
                    }
                }
            }
            WildlifeKind::Fish => {
                // 魚不怕人：無視玩家、無重力/無陸地碰撞——只在自己的水塘裡悠游。
                if a.wait_timer > 0.0 {
                    a.wait_timer -= dt;
                } else {
                    let (nx, nz, reached) =
                        vfishlife::swim_step(a.body.x, a.body.z, a.target_x, a.target_z, dt);
                    a.body.x = nx;
                    a.body.z = nz;
                    if reached {
                        let angle = rand::random::<f32>() * std::f32::consts::TAU;
                        let radius = vfishlife::WANDER_MIN_R
                            + rand::random::<f32>() * (vfishlife::WANDER_MAX_R - vfishlife::WANDER_MIN_R);
                        let (tx, tz) = vr::wander_target(a.home_x, a.home_z, angle, radius);
                        // 複驗候選目標仍在夠深的水域裡，避免魚游出水塘擱淺在陸地上。
                        if vfishlife::is_deep_water(tx.round() as i32, tz.round() as i32) {
                            a.target_x = tx;
                            a.target_z = tz;
                        } else {
                            a.target_x = a.home_x;
                            a.target_z = a.home_z;
                        }
                        a.wait_timer = 0.5 + rand::random::<f32>() * 1.5;
                    }
                }
                a.body.y = vfishlife::clamp_swim_y(a.body.x.round() as i32, a.body.z.round() as i32, a.body.y);
            }
            WildlifeKind::Chicken => {
                // 雞不怕人：沒有受驚分支，永遠只是閒晃或（馴服後）跟隨——比照野兔跟隨分支，
                // 但沒有 fleeing 這條路（放養雞 v1，自主提案切片 ROADMAP 870）。
                let nearest = player_pts.iter().min_by(|(x1, z1), (x2, z2)| {
                    let d1 = (a.body.x - x1).powi(2) + (a.body.z - z1).powi(2);
                    let d2 = (a.body.x - x2).powi(2) + (a.body.z - z2).powi(2);
                    d1.partial_cmp(&d2).unwrap_or(std::cmp::Ordering::Equal)
                });
                let nearest_dist_sq = nearest
                    .map(|(px, pz)| (a.body.x - px).powi(2) + (a.body.z - pz).powi(2))
                    .unwrap_or(f32::MAX);
                let now_following = a.tamed && vwild::should_follow(a.following, nearest_dist_sq);
                a.following = now_following;
                if now_following {
                    if let Some((px, pz)) = nearest {
                        if vwild::should_close_follow_gap(nearest_dist_sq) {
                            vr::step_toward(&world, &mut a.body, *px, *pz, dt, vwild::FOLLOW_SPEED);
                        } else {
                            vr::gravity_step(&world, &mut a.body, dt);
                        }
                    }
                    a.wait_timer = 0.0;
                } else if a.wait_timer > 0.0 {
                    a.wait_timer -= dt;
                    vr::gravity_step(&world, &mut a.body, dt);
                } else {
                    let reached =
                        vr::step_toward(&world, &mut a.body, a.target_x, a.target_z, dt, vwild::WANDER_SPEED);
                    if reached {
                        let angle = rand::random::<f32>() * std::f32::consts::TAU;
                        let radius = vwild::WANDER_MIN_R
                            + rand::random::<f32>() * (vwild::WANDER_MAX_R - vwild::WANDER_MIN_R);
                        let (tx, tz) = vr::wander_target(a.home_x, a.home_z, angle, radius);
                        a.target_x = tx;
                        a.target_z = tz;
                        a.wait_timer = 0.5 + rand::random::<f32>() * 1.5;
                    }
                }
                // 已馴服的雞會定期回饋一顆蛋（掉落在腳邊，走近撿起——複用既有掉落物管線）。
                if a.tamed {
                    a.lay_cd -= dt;
                    if vchicken::should_lay(a.lay_cd) {
                        egg_layers.push((a.body.x, a.body.y, a.body.z));
                        a.lay_cd = vchicken::next_lay_cooldown(rand::random::<f32>());
                    }
                }
            }
        }
        if let Some(yaw) = vr::yaw_from_move(a.body.x - bx, a.body.z - bz) {
            a.yaw = yaw;
        }
    }
    } // wildlife 寫鎖釋放
    drop(world);
    for (x, y, z) in egg_layers {
        let spawned = hub().drops.write().unwrap().spawn(x, y, z, vcoop::EGG_ID, 1, "雞", vfarm::now_secs());
        if let Some(id) = spawned {
            broadcast_item_dropped(id, x, y, z, vcoop::EGG_ID, 1, "雞");
        }
    }
}

/// 全域生育節流時間戳記（馴服兔子生寶寶 v1，自主提案切片，ROADMAP 855）。
/// 純記憶體、重啟歸零——比照 wildlife 系統本身「重啟即重新生成」的既有慣例。
static LAST_BREED_UNIX: AtomicU64 = AtomicU64::new(0);
/// 小兔子 id 流水號（馴服兔子生寶寶 v1），與 `vox_wld_{i}`（初始兔子）/`vox_fsh_{i}`
/// （魚）兩個既有命名空間分開，避免新生兒與初始生物 id 撞號。
static NEXT_BABY_RABBIT_SEQ: AtomicU64 = AtomicU64::new(0);

/// 馴服兔子生寶寶 v1（自主提案切片，ROADMAP 855）：低頻（併入 15 秒節拍）檢查已馴服的
/// 兔子裡有沒有兩隻湊得夠近，機率誕生一隻小兔子（見 `voxel_wildlife` 模組說明）。
///
/// **鎖紀律**：全程只碰 `wildlife` 一把寫鎖、短取即釋，不與其他 store 巢狀；
/// Feed 落地在鎖外（同既有 `tick_coop`/`maybe_birth` 慣例）。
fn maybe_breed_rabbits() {
    let now = vfarm::now_secs();
    let elapsed = now.saturating_sub(LAST_BREED_UNIX.load(Ordering::Relaxed)) as f32;
    let feed_line = {
        let mut animals = hub().wildlife.write().unwrap();
        let rabbit_count = animals.iter().filter(|a| a.kind == WildlifeKind::Rabbit).count();
        let tamed_positions: Vec<(usize, f32, f32)> = animals
            .iter()
            .enumerate()
            .filter(|(_, a)| a.kind == WildlifeKind::Rabbit && a.tamed)
            .map(|(i, a)| (i, a.body.x, a.body.z))
            .collect();
        let Some((ia, ib)) = vwild::find_breeding_pair(&tamed_positions) else {
            return;
        };
        if !vwild::should_breed(rabbit_count, elapsed, rand::random::<f32>()) {
            return;
        }
        LAST_BREED_UNIX.store(now, Ordering::Relaxed);

        let (ax, az) = (animals[ia].body.x, animals[ia].body.z);
        let (bx, bz) = (animals[ib].body.x, animals[ib].body.z);
        let (sx, sz) = vwild::baby_spawn_point(ax, az, bx, bz);
        let body = vr::dry_ground_spawn(sx.round() as i32, sz.round() as i32);
        let seq = NEXT_BABY_RABBIT_SEQ.fetch_add(1, Ordering::Relaxed);
        animals.push(WildlifeAnimal {
            id: format!("vox_wld_baby_{seq}"),
            kind: WildlifeKind::Rabbit,
            home_x: body.x,
            home_z: body.z,
            target_x: body.x,
            target_z: body.z,
            body,
            yaw: 0.0,
            wait_timer: 0.0,
            fleeing: false,
            tamed: true, // 一出生就認得你、立刻跟父母一樣跟著走。
            following: false,
            lay_cd: 0.0,
        });
        vwild::baby_line(rand::random::<u64>() as usize).to_string()
    }; // wildlife 寫鎖釋放
    vfeed::append_feed("兔子誕生", "野兔", &feed_line);
}

/// 居民注意到你身邊跟著的馴服動物 v1（自主提案切片，ROADMAP 875）：全域「居民→冷卻時刻」
/// 儲存（不分是哪位玩家的寵物觸發），比照 `structure_names()` 的 `OnceLock<Mutex<..>>` 慣例。
/// 純記憶體、重啟歸零（讚賞本身無持久化，比照既有 773/774 讚賞冷卻慣例）。
fn pet_admire_cd() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    static C: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
        std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 居民注意到你身邊跟著的馴服動物 v1（自主提案切片，ROADMAP 875，低頻併入 15 秒節拍檢查）：
/// 已馴服且正在跟隨的兔子/雞若就在某位玩家身邊，挑一位近旁有空的居民，過冷卻就讚賞這份
/// 陪伴＋記進心裡＋動態牆播報。與 773/774 讚賞完全獨立的一組判定/冷卻，互不干擾
/// （見 `voxel_pet_admire` 模組說明）。
///
/// **鎖紀律**：players／wildlife／residents（讀→寫兩段）／memory 全部各自短取即釋、
/// 循序不巢狀（守 prod 死鎖鐵律，比照 `maybe_farm_admire`/`maybe_breed_rabbits` 慣例）。
fn maybe_pet_admire() {
    // 1) 玩家快照（短鎖即釋，不與下面任何鎖巢狀）。
    let players_snap: Vec<(String, f32, f32)> = {
        let players = hub().players.read().unwrap();
        players.values().map(|p| (p.name.clone(), p.x, p.z)).collect()
    };
    if players_snap.is_empty() {
        return;
    }

    // 2) 已馴服且正在跟隨中的動物快照（短鎖即釋）。魚恆無跟隨機制，天然不會出現在此。
    let pets_snap: Vec<(f32, f32, &'static str)> = {
        let animals = hub().wildlife.read().unwrap();
        animals
            .iter()
            .filter(|a| a.tamed && a.following)
            .map(|a| {
                let label = match a.kind {
                    WildlifeKind::Rabbit => "兔子",
                    WildlifeKind::Chicken => "雞",
                    WildlifeKind::Fish => "魚",
                };
                (a.body.x, a.body.z, label)
            })
            .collect()
    };
    if pets_snap.is_empty() {
        return;
    }

    let now_secs = vfarm::now_secs();
    for (pname, px, pz) in &players_snap {
        let nearest_pet = pets_snap
            .iter()
            .map(|(ax, az, label)| {
                let dx = px - ax;
                let dz = pz - az;
                (dx * dx + dz * dz, *label)
            })
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let Some((pet_dist_sq, pet_label)) = nearest_pet else { continue };
        if !vpetadmire::has_pet_nearby(pet_dist_sq) {
            continue;
        }

        // 3) 近旁有空的居民（residents 讀鎖即釋，不與後續寫鎖巢狀）。
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
                    let dx = px - r.body.x;
                    let dz = pz - r.body.z;
                    (r.id.clone(), r.name, dx * dx + dz * dz)
                })
                .filter(|(_, _, d2)| *d2 <= vpetadmire::PET_ADMIRE_RADIUS * vpetadmire::PET_ADMIRE_RADIUS)
                .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        };
        let Some((rid, rname, dist_sq)) = cand else { continue };

        let cooldown_ok = {
            let cd = pet_admire_cd().lock().unwrap();
            match cd.get(&rid) {
                Some(prev) => now_secs.saturating_sub(*prev) >= vpetadmire::PET_ADMIRE_COOLDOWN_SECS,
                None => true,
            }
        };
        if !vpetadmire::admire_triggers(true, dist_sq, cooldown_ok) {
            continue;
        }
        { pet_admire_cd().lock().unwrap().insert(rid.clone(), now_secs); }

        let pick = now_secs as usize;
        let say_line = vpetadmire::admire_say_line(pname, pet_label, pick);
        let said = {
            let mut residents = hub().residents.write().unwrap();
            residents
                .iter_mut()
                .find(|r| r.id == rid)
                .map(|r| {
                    r.say = say_line.chars().take(50).collect();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                })
                .is_some()
        }; // residents 寫鎖釋放
        if said {
            broadcast_players();
            let summary = vpetadmire::admire_memory_line(pname, pet_label);
            let entry = hub().memory.write().unwrap().add_memory(&rid, pname, &summary);
            vmem::append_memory(&entry);
            vfeed::append_feed(
                "居民讚賞",
                rname,
                &format!("{rname}注意到{pname}身邊跟著一隻{pet_label}，忍不住多看了幾眼。"),
            );
        }
    }
}

/// 就地指導 v1（自主提案切片）：全域「學生→上次被就地教學的時刻」冷卻儲存，
/// 比照 `pet_admire_cd()` 慣例。純記憶體、重啟歸零（比照既有 773/774/875 冷卻慣例，
/// 教學本身早已透過 `voxel_invented_skills.jsonl` 落地持久化，只有這道冷卻本身不必記）。
fn proximity_teach_cd() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    static C: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
        std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 技能互教·每居民「教或學」帳本（北極星第四刀）：居民 id → 上次教/學的 unix 秒。
/// 三條教學路（717 登門到訪／就地指導／相遇互教）**共用同一本帳**：教過或學過任一次，
/// 這一遊戲天（[`vteach::TEACH_LEDGER_SECS`]）內不再參與登門教學與相遇互教——手藝在
/// 村裡以自然節奏擴散、不會一個下午全村都會。就地指導（卡關救援）不受帳本**攔阻**
/// （自救優先，仍走它自己的 240s 冷卻），但成功救援會**記帳**（那也算這天教過了）。
/// 純記憶體、重啟歸零（頂多重啟後早一點再教，零資料風險）。
fn skill_teach_ledger() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    static C: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
        std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 帳本冷卻秒數：預設一遊戲天（[`vteach::TEACH_LEDGER_SECS`]）；QA 可用環境變數
/// `BUTFUN_TEACH_COOLDOWN_SECS` 縮短（只影響教學節奏，不動任何資料）。啟動時讀一次。
fn teach_ledger_secs() -> u64 {
    static V: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("BUTFUN_TEACH_COOLDOWN_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(vteach::TEACH_LEDGER_SECS)
    })
}

/// 相遇互教每輪掃描的觸發機率：預設 [`vteach::ENCOUNTER_TEACH_CHANCE`]；QA 可用
/// 環境變數 `BUTFUN_TEACH_CHANCE` 調高做確定性驗證。啟動時讀一次，夾在 [0,1]。
fn encounter_teach_chance() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("BUTFUN_TEACH_CHANCE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .map(|c| c.clamp(0.0, 1.0))
            .unwrap_or(vteach::ENCOUNTER_TEACH_CHANCE)
    })
}

/// 相遇互教的「近旁」半徑（方塊）：預設沿用就地指導的 [`vptteach::PROXIMITY_TEACH_RADIUS`]；
/// QA 可用環境變數 `BUTFUN_TEACH_RADIUS` 放大，免等居民恰好晃到彼此身邊。啟動時讀一次。
fn encounter_teach_radius() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("BUTFUN_TEACH_RADIUS")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|r| r.is_finite() && *r > 0.0)
            .unwrap_or(vptteach::PROXIMITY_TEACH_RADIUS)
    })
}

/// 就地指導 v1（自主提案切片，低頻併入 15 秒節拍檢查）：找一位正卡關（`invent_backoff`
/// 非空）的居民，身邊剛好站著已經會解法的老朋友，就當場教會她、解除這個目標的退避——
/// 不必等到 717（`voxel_teach`）下次登門到訪才有機會補上（見 `voxel_proximity_teach`
/// 模組說明）。每次呼叫只促成一組教學（保持稀疏、成本可預期），找到第一組符合條件的
/// 就執行並提前返回；下一輪 tick 再繼續找下一組。
///
/// **鎖紀律**：residents／bonds／invented／memory 全部各自短取即釋、循序不巢狀
/// （守 prod 死鎖鐵律，比照 `maybe_pet_admire` 慣例）——三個階段依序：①residents 讀鎖
/// 快照位置/是否有空/卡關目標 ②bonds 讀鎖篩出「老朋友且站得夠近」的候選對 ③invented
/// 讀鎖確認老師真的會、再各自短寫鎖落地。
fn maybe_proximity_teach() {
    // 1) 居民快照：位置 + 是否有空 + 卡關中的目標材料集合（短讀鎖即釋，不與其他鎖巢狀）。
    let snap: Vec<(String, &'static str, f32, f32, Vec<u8>, bool)> = {
        let residents = hub().residents.read().unwrap();
        residents
            .iter()
            .map(|r| {
                let free = r.say.is_empty()
                    && !r.asleep
                    && r.visiting.is_none()
                    && r.expedition.is_none()
                    && r.clique_meet.is_none()
                    && r.savoring.is_none()
                    && r.invent_run.is_none();
                (
                    r.id.clone(),
                    r.name,
                    r.body.x,
                    r.body.z,
                    r.invent_backoff.keys().copied().collect::<Vec<u8>>(),
                    free,
                )
            })
            .collect()
    }; // residents 讀鎖釋放

    let now_secs = vfarm::now_secs();

    // 2) 篩出「學生卡關中 + 老朋友站得夠近 + 學生冷卻已過」的候選對（bonds 短讀鎖即釋）。
    let mut candidate_pair: Option<(usize, usize)> = None;
    {
        let bonds = hub().bonds.read().unwrap();
        'outer: for (si, (student_id, _, sx, sz, stuck_goals, student_free)) in snap.iter().enumerate() {
            if stuck_goals.is_empty() || !student_free {
                continue;
            }
            let cooldown_ok = {
                let cd = proximity_teach_cd().lock().unwrap();
                match cd.get(student_id) {
                    Some(prev) => now_secs.saturating_sub(*prev) >= vptteach::PROXIMITY_TEACH_COOLDOWN_SECS,
                    None => true,
                }
            };
            if !cooldown_ok {
                continue;
            }
            for (ti, (teacher_id, _, tx, tz, _, teacher_free)) in snap.iter().enumerate() {
                if ti == si || !teacher_free {
                    continue;
                }
                let dx = sx - tx;
                let dz = sz - tz;
                let tier = resident_tier_of(&bonds, student_id, teacher_id);
                if vptteach::teach_triggers(tier, dx * dx + dz * dz, true) {
                    candidate_pair = Some((si, ti));
                    break 'outer;
                }
            }
        }
    } // bonds 讀鎖釋放
    let Some((si, ti)) = candidate_pair else {
        // 沒有人卡關待救 → 走相遇互教（技能互教·北極星第四刀）：交情夠的兩位閒著的
        // 居民恰好站得近，偶爾把自己會、對方不會的手藝就地教一手。
        maybe_encounter_teach(&snap, now_secs);
        return;
    };
    let student_id = &snap[si].0;
    let student_name = snap[si].1;
    let stuck_goals = &snap[si].4;
    let teacher_id = &snap[ti].0;
    let teacher_name = snap[ti].1;

    // 3) 老師是否真的會學生正卡關的某個目標（invented 短讀鎖即釋，不與上面鎖巢狀）。
    let taught_skill = {
        let store = hub().invented.read().unwrap();
        stuck_goals.iter().find_map(|&goal| store.find_for(teacher_id, goal).cloned())
    }; // invented 讀鎖釋放
    let Some(skill) = taught_skill else { return };

    // 4) 真的教會：技能落地（invented 短寫鎖即釋）。師承鏈（北極星第四刀）：走 learn_from
    //    而非 add——學來的技能 source 標老師名、taught 標 true，技能簿看得出「師承XX」。
    let learned = {
        let mut store = hub().invented.write().unwrap();
        store.learn_from(student_id, &skill, teacher_name)
    }; // invented 寫鎖釋放
    let Some(rec) = learned else { return };
    vinvent::append_invented_skill(&rec);
    { proximity_teach_cd().lock().unwrap().insert(student_id.clone(), now_secs); }
    // 教/學帳本也記一筆（救援不受帳本攔阻、但算進這一天的教學額度，見 skill_teach_ledger）。
    {
        let mut led = skill_teach_ledger().lock().unwrap();
        led.insert(teacher_id.clone(), now_secs);
        led.insert(student_id.clone(), now_secs);
    }

    // 解除這個目標的退避——答案已經到手，不必再乾等冷卻歸零。
    {
        let mut residents = hub().residents.write().unwrap();
        if let Some(r) = residents.iter_mut().find(|r| &r.id == student_id) {
            r.invent_backoff.remove(&skill.goal_block);
            r.invent_fail_counts.remove(&skill.goal_block);
        }
    } // residents 寫鎖釋放

    let pick = now_secs as usize;
    {
        let teacher_line = vteach::teach_say_line_as_teacher(student_name, &skill.name, pick);
        let student_line = vteach::teach_say_line_as_student(teacher_name, &skill.name, pick);
        let mut residents = hub().residents.write().unwrap();
        for (rid, line) in [(teacher_id.clone(), teacher_line), (student_id.clone(), student_line)] {
            if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                r.say = line.chars().take(50).collect();
                r.say_timer = SAY_SECS;
                r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
            }
        }
    } // residents 寫鎖釋放
    broadcast_players();

    {
        let entry = hub().memory.write().unwrap().add_memory(
            teacher_id,
            student_name,
            &vteach::teach_memory_line_teacher(student_name, &skill.name),
        );
        vmem::append_memory(&entry);
    } // memory 寫鎖釋放
    {
        let entry = hub().memory.write().unwrap().add_memory(
            student_id,
            teacher_name,
            &vteach::teach_memory_line_student(teacher_name, &skill.name),
        );
        vmem::append_memory(&entry);
    } // memory 寫鎖釋放
    vfeed::append_feed(
        vteach::FEED_KIND,
        teacher_name,
        &vteach::teach_feed_line(teacher_name, student_name, &skill.name, pick),
    );
}

/// 相遇互教（技能互教·北極星第四刀，與就地指導同一 15 秒節拍、就地指導沒人待救時才輪到）：
/// 交情到老朋友的兩位居民**平常相處時剛好站得夠近、雙方都閒**，偶爾把自己會、對方還不會的
/// 手藝就地教一手——手藝第一次不必等登門到訪或有人卡關，就能在活人之間口耳相傳，與出生時的
/// 血脈繼承（#998「承自XX」）互補成兩條知識傳承的路。
///
/// **不無限擴散的三道閘**：①機率門檻（[`encounter_teach_chance`]，每輪掃描擲一次）
/// ②每居民教/學帳本冷卻（一遊戲天最多參與一次，[`skill_teach_ledger`]）③交情要到老朋友
/// ——一個技能傳遍全村自然要花好幾遊戲天，像真的村子。
///
/// **教學那一幕**：兩人停留 [`vteach::TEACH_PAUSE_SECS`] 秒（設 `wait_timer`，比照長椅並坐），
/// 老師先冒「來，我教你…」的開講泡泡，學生幾秒後應和「原來如此！」（`pending_teach_reply`，
/// 全程句式池、零 LLM）；學到的技能 `source` 標老師名（技能簿顯示「師承XX」）、走既有 jsonl
/// 落地，之後同處境零 LLM 重用照舊、也能再往下教。
///
/// **鎖紀律**：bonds 讀 → drop → invented 讀 → drop → invented 寫 → drop → residents 寫 →
/// drop → memory 寫 ×2，全程短鎖循序、不巢狀、不持鎖 await（守 prod 死鎖鐵律）。
fn maybe_encounter_teach(snap: &[(String, &'static str, f32, f32, Vec<u8>, bool)], now_secs: u64) {
    // 機率門檻：每輪掃描只擲一次骰（不隨在場人數膨脹觸發率）。
    if rand::random::<f32>() >= encounter_teach_chance() {
        return;
    }

    // 1) 篩出「兩人都閒 + 老朋友 + 站得夠近 + 雙方教/學帳本都過冷卻」的候選對
    //    （bonds 讀鎖只在挑對期間持有；帳本是獨立小 Mutex，取值即釋）。
    let ledger_ok: Vec<bool> = {
        let led = skill_teach_ledger().lock().unwrap();
        snap.iter()
            .map(|(id, ..)| vteach::ledger_ready(now_secs, led.get(id).copied(), teach_ledger_secs()))
            .collect()
    }; // 帳本鎖釋放
    let candidates: Vec<(usize, usize)> = {
        let bonds = hub().bonds.read().unwrap();
        let mut out = Vec::new();
        for (ti, (teacher_id, _, tx, tz, _, teacher_free)) in snap.iter().enumerate() {
            if !teacher_free || !ledger_ok[ti] {
                continue;
            }
            for (si, (student_id, _, sx, sz, _, student_free)) in snap.iter().enumerate() {
                if ti == si || !student_free || !ledger_ok[si] {
                    continue;
                }
                let dx = sx - tx;
                let dz = sz - tz;
                let r = encounter_teach_radius();
                let tier = resident_tier_of(&bonds, teacher_id, student_id);
                // 交情要到老朋友＋站得夠近（半徑預設同就地指導，QA 可調）。
                if tier == vbonds::BondTier::Friend && dx * dx + dz * dz <= r * r {
                    out.push((ti, si));
                }
            }
        }
        out
    }; // bonds 讀鎖釋放

    // 2) 依序找第一對「老師真的有學生不會的技能」（invented 短讀鎖即釋）；每輪最多教一組。
    let taught = {
        let store = hub().invented.read().unwrap();
        candidates.into_iter().find_map(|(ti, si)| {
            store
                .teachable(&snap[ti].0, &snap[si].0)
                .map(|k| (ti, si, k.clone()))
        })
    }; // invented 讀鎖釋放
    let Some((ti, si, skill)) = taught else { return };
    let teacher_id = &snap[ti].0;
    let teacher_name = snap[ti].1;
    let student_id = &snap[si].0;
    let student_name = snap[si].1;

    // 3) 真的教會：複製進學生技能庫（source=師承老師名）＋既有 jsonl 落地（向後相容）。
    let learned = {
        let mut store = hub().invented.write().unwrap();
        store.learn_from(student_id, &skill, teacher_name)
    }; // invented 寫鎖釋放
    let Some(rec) = learned else { return };
    vinvent::append_invented_skill(&rec);
    {
        let mut led = skill_teach_ledger().lock().unwrap();
        led.insert(teacher_id.clone(), now_secs);
        led.insert(student_id.clone(), now_secs);
    } // 帳本鎖釋放

    // 4) 教學那一幕：老師開講、兩人停留一會兒，學生幾秒後應和（零 LLM、句式池）。
    let pick = now_secs as usize;
    {
        let mut residents = hub().residents.write().unwrap();
        if let Some(r) = residents.iter_mut().find(|r| &r.id == teacher_id) {
            r.say = vteach::teach_open_line(student_name, &skill.name, pick)
                .chars()
                .take(50)
                .collect();
            r.say_timer = SAY_SECS;
            r.wait_timer = r.wait_timer.max(vteach::TEACH_PAUSE_SECS);
            r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
        }
        if let Some(r) = residents.iter_mut().find(|r| &r.id == student_id) {
            r.pending_teach_reply = Some((
                vteach::teach_reply_line(&skill.name, pick),
                vteach::TEACH_REPLY_DELAY_SECS,
            ));
            r.wait_timer = r.wait_timer.max(vteach::TEACH_PAUSE_SECS);
        }
    } // residents 寫鎖釋放
    broadcast_players();

    // 5) 雙方各留一筆記憶＋世界 Feed（沿用 717 的句式，同一件事不另造一套詞）。
    {
        let entry = hub().memory.write().unwrap().add_memory(
            teacher_id,
            student_name,
            &vteach::teach_memory_line_teacher(student_name, &skill.name),
        );
        vmem::append_memory(&entry);
    } // memory 寫鎖釋放
    {
        let entry = hub().memory.write().unwrap().add_memory(
            student_id,
            teacher_name,
            &vteach::teach_memory_line_student(teacher_name, &skill.name),
        );
        vmem::append_memory(&entry);
    } // memory 寫鎖釋放
    vfeed::append_feed(
        vteach::FEED_KIND,
        teacher_name,
        &vteach::teach_feed_line(teacher_name, student_name, &skill.name, pick),
    );
    tracing::info!(
        teacher = %teacher_id, student = %student_id, skill = %skill.name,
        "相遇互教：手藝在活人之間口耳相傳（師承鏈落地，零 LLM）"
    );
}

/// 一次居民世界推進：套用上輪思考的決策 → 物理/閒晃 → 社交互動 → 廣播 → 排程新一輪思考。
fn tick_residents(dt: f32) {
    // 0) 推進世界時鐘（短鎖即釋，不巢狀）。晝夜循環 v1。
    { hub().world_time.write().unwrap().tick(dt); }

    // 0a) 季節輪替 v1（ROADMAP 798）：由世界累計日數推算當前季節，與上輪比對偵測「換季」。
    //     換季那一刻設本地旗標（供下方觸發附近醒著居民抬頭反應）＋上一則城鎮動態（不在線上的玩家
    //     回來也讀得到世界換了季）。短鎖即釋、不巢狀（守死鎖鐵律）；Feed append 走鎖外。
    let current_season = {
        let day = hub().world_time.read().unwrap().days_elapsed();
        vseason::season_for_day(day)
    };
    let season_just_turned = {
        let mut last = hub().last_season.write().unwrap();
        if *last != current_season {
            *last = current_season;
            true
        } else {
            false
        }
    };
    if season_just_turned {
        vfeed::append_feed("季節", "乙太方界", vseason::season_feed_detail(current_season));
    }

    // 0a-2) 居民誕辰紀念 v1：本 tick 的目前 unix 秒（純讀系統時鐘，無鎖），供下方逐居民算滿週歲數。
    let now_unix = vfarm::now_secs();

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
    // 彩虹剛升起的一次性旗標（雨後彩虹 v1，ROADMAP 780）：consume-once，觸發居民抬頭望見彩虹的歡呼。
    let rainbow_just_appeared = {
        let mut f = hub().rainbow_started_flag.write().unwrap();
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
    // 居民搬新家（引導式都更）：進行中搬家的快照（relocations 讀鎖即釋），供 residents
    // 寫鎖段用——拆除段把她排除出 agency 候選（搬家中不接其他任務）、閒晃中心貼著當前
    // 工地（蓋新家貼新地塊、拆舊家貼舊家），不與 residents 鎖巢狀（守死鎖鐵律）。
    // 元組＝(居民 id, 工地焦點 x, 工地焦點 z, 是否在拆除段)。
    let reloc_snap: Option<(String, f32, f32, bool)> = {
        let reloc = hub().relocations.read().unwrap();
        reloc.active().map(|a| {
            let demolishing = a.phase == vvillage::RELOC_PHASE_DEMOLISH;
            let (fx, fz) = if demolishing {
                (a.old_x as f32 + 0.5, a.old_z as f32 + 0.5)
            } else {
                (a.new_x as f32 + 0.5, a.new_z as f32 + 0.5)
            };
            (a.resident.clone(), fx, fz, demolishing)
        })
    }; // relocations 讀鎖釋放
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

    // 居民關心你挨餓 v1：短鎖快照所有玩家目前是否挨餓（is_starving）→ drop（循序取放、不與
    // 居民鎖巢狀）。只記名字→bool，不外洩其他生存數值，供下方逐居民判定用。
    let player_starving: std::collections::HashMap<String, bool> = {
        let stats = hub().player_stats.read().unwrap();
        stats.iter().map(|(name, s)| (name.clone(), s.is_starving())).collect()
    }; // player_stats 讀鎖在此釋放

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
    // 邊陲探友（居民千里跋涉去邊陲探望遠行的夥伴 v1，ROADMAP 821）：鎖內收集「訪客抵達朋友邊陲落點、
    // 找到人」事件，記憶寫（雙方，朋友 id 於鎖外用名字查回）＋record_visit 加溫＋Feed 在居民鎖釋放後
    // 統一落地（不持居民鎖做 IO，守死鎖鐵律）。(訪客 id, 訪客名, 朋友名, 方位名)。
    let mut frontier_visit_arrive_events: Vec<(String, &'static str, String, String)> = Vec::new();
    // 邊陲探友純 Feed 事件（啟程／小聚結束返家／半路撲空放棄），鎖釋放後統一 append_feed。
    let mut frontier_visit_feed: Vec<(&'static str, String)> = Vec::new();
    // 繁星夜空·星夜共賞（v1，ROADMAP 783）：鎖內收集「某居民記得這位玩家愛看星星、在星夜喚他同賞」
    // 事件；記憶寫（add_memory）與 Feed（append_feed）全在居民鎖釋放後統一處理（不持居民鎖做 IO）。
    // (居民 id, 居民名, 玩家名)。
    let mut stargaze_events: Vec<(String, &'static str, String)> = Vec::new();
    // 睹物思人事件（ROADMAP 784）：居民駐足追憶時鎖內收集，記憶寫＋Feed 在居民鎖釋放後統一落地
    //（不持居民鎖做 IO，守死鎖鐵律）。(居民 id, 居民名, 送禮玩家名, 紀念物名, pick)。
    let mut keepsake_recall_events: Vec<(String, &'static str, String, String, usize)> = Vec::new();
    // 哼歌事件（ROADMAP 788）：某居民心情正好、你正好在身邊、牠哼給你聽時鎖內收集，記憶寫＋Feed
    // 在居民鎖釋放後統一落地（不持居民鎖做 IO，守死鎖鐵律）。(居民 id, 居民名, 玩家名, pick)。
    let mut humming_events: Vec<(String, &'static str, String, usize)> = Vec::new();
    // 營火取暖事件（乙太營火 v1）：某居民夜裡路過火邊駐足圍暖時鎖內收集；記憶寫＋Feed 在居民鎖
    // 釋放後統一落地（不持居民鎖做 IO，守死鎖鐵律）。玩家在旁才記交情，否則 None 只上 Feed。
    // (居民 id, 居民名, 火邊玩家名 Option, pick)。
    let mut campfire_warm_events: Vec<(String, &'static str, Option<String>, usize)> = Vec::new();
    // 木長椅歇腳事件（木長椅 v1）：某居民白天路過椅邊坐下歇腳時鎖內收集；記憶寫＋Feed 在居民鎖
    // 釋放後統一落地（不持居民鎖做 IO，守死鎖鐵律）。玩家在旁才記交情，否則 None 只上 Feed。
    // (居民 id, 居民名, 椅邊玩家名 Option, pick)。
    let mut bench_rest_events: Vec<(String, &'static str, Option<String>, usize)> = Vec::new();
    // 臨水垂釣事件（居民臨水垂釣 v1）：某居民白天恰好臨水、坐下釣一竿時鎖內收集；記憶寫＋Feed 在居民鎖
    // 釋放後統一落地（不持居民鎖做 IO，守死鎖鐵律）。玩家在水邊才記交情，否則 None 只上 Feed。
    // (居民 id, 居民名, 水邊玩家名 Option, pick)。
    let mut angler_events: Vec<(String, &'static str, Option<String>, usize)> = Vec::new();
    // 雨天避雨事件（雨天葉傘避雨 v1）：某居民下雨時停步躲一會兒時鎖內收集；記憶寫＋Feed 在居民鎖
    // 釋放後統一落地（不持居民鎖做 IO，守死鎖鐵律）。玩家在近旁才記交情，否則 None 只上 Feed。
    // (居民 id, 居民名, 近旁玩家名 Option, pick)。
    let mut rain_shelter_events: Vec<(String, &'static str, Option<String>, usize)> = Vec::new();
    // 顧家駐足事件（居民顧家駐足 v1）：某居民白天在自家門前停步望家時鎖內收集；記憶寫＋Feed 在居民鎖
    // 釋放後統一落地（不持居民鎖做 IO，守死鎖鐵律）。玩家在近旁才記交情，否則 None 只上 Feed。
    // (居民 id, 居民名, 近旁玩家名 Option, pick)。
    let mut homegaze_events: Vec<(String, &'static str, Option<String>, usize)> = Vec::new();
    // 邊陲巧遇事件（玩家追到邊陲、巧遇正在遠行的居民 v1）：某位遠行居民在邊陲逗留時被玩家撞見，鎖內
    // 收集；記憶寫＋Feed 在居民鎖釋放後統一落地（不持居民鎖做 IO，守死鎖鐵律）。
    // (居民 id, 居民名, 巧遇玩家名, 遠行方位, pick)。
    let mut frontier_find_events: Vec<(String, &'static str, String, String, usize)> = Vec::new();
    // 居民誕辰紀念事件（居民誕辰紀念 v1 + v1.1 分你一份心意）：某位經世代傳承誕生的居民滿一個乙太年
    // 時鎖內收集；記憶寫＋Feed 在居民鎖釋放後統一落地（不持居民鎖做 IO，守死鎖鐵律）。玩家在近旁才記
    // 交情、才可能分到心意，否則 None 只上 Feed。
    // (居民 id, 居民名, 近旁玩家名 Option, 滿週歲數, 父母名, pick, 分享的心意 Option<(item_id, qty)>)。
    let mut birthday_events: Vec<(
        String,
        &'static str,
        Option<String>,
        u64,
        String,
        usize,
        Option<(u8, u32)>,
    )> = Vec::new();
    // 集會鐘 v1：某位應召的居民走到鐘邊聚攏時，鎖內收集事件；「你敲鐘召我來」的交情記憶＋Feed
    // 在居民鎖釋放後統一落地（不持居民鎖做 IO，守死鎖鐵律）。(居民 id, 居民名, 敲鐘者名)。
    let mut bell_gather_events: Vec<(String, &'static str, String)> = Vec::new();
    // 圍火講往事 v1：夜裡兩位醒著的居民同在一座營火邊、其中一位講起一段往事時，鎖內收集事件；
    // 聆聽者的社交記憶寫＋Feed 在居民鎖釋放後統一落地（不持居民鎖做 IO，守死鎖鐵律）。
    // (講述者 id, 講述者名, 聆聽者 id, 聆聽者名, 往事摘要)。
    let mut campfire_tale_events: Vec<(String, &'static str, String, &'static str, String)> = Vec::new();
    // 長椅並坐閒聊 v1：白天兩位相識以上的居民同在一張長椅邊、一位招呼另一位並肩坐下閒聊時，鎖內收集
    // 事件；雙方交情記憶＋record_visit 加溫＋Feed 在居民鎖釋放後統一落地（不持居民鎖做 IO，守死鎖
    // 鐵律）。(發起者 id, 發起者名, 被招呼者 id, 被招呼者名)。
    let mut bench_chat_events: Vec<(String, &'static str, String, &'static str)> = Vec::new();
    // 長椅拌嘴/和好 v1：與長椅並坐閒聊共用同一個配對掃描，互斥（每次相遇只擇一）。拌嘴＝雙方記一筆
    // 摩擦記憶 + 標記彆扭中 + 交情暫冷一格；和好＝解除彆扭 + 雙方記一筆修復記憶 + 交情回暖一格。
    // 鎖外統一落地，格式同 bench_chat_events：(發起者 id, 發起者名, 被招呼者 id, 被招呼者名)。
    let mut bench_tiff_events: Vec<(String, &'static str, String, &'static str)> = Vec::new();
    let mut bench_makeup_events: Vec<(String, &'static str, String, &'static str)> = Vec::new();
    // 飢餓時的守望相助 v1（ROADMAP 800）：本 tick 分食事件 (分食者 id, 分食者名, 被分食者 id, 被分食者名)，
    // 鎖外落地雙方記憶 + 情誼加溫 + 城鎮動態（比照 792/witness 的鎖外處理，守死鎖鐵律）。
    // 末欄 is_repay（知恩圖報 v1，801）：true=這頓是「回報當年那口飯」，鎖外落地時走專屬記憶/Feed
    // 並結清欠飯；false=一般守望相助分食，落地時登記「被分食者欠分食者一口飯」。
    let mut share_meal_events: Vec<(String, &'static str, String, &'static str, bool)> = Vec::new();
    // 居民也會生病 v1（自主提案）：本 tick 自然痊癒事件（居民名），鎖外落地城鎮動態。
    let mut illness_recovered_events: Vec<&'static str> = Vec::new();
    // 居民也會生病 v1：本 tick 病倒事件（居民名, 是否為淋雨引發），鎖外落地城鎮動態；
    // 淋雨引發的病倒（自主提案 v2）走專屬雨因文案，與泛用病倒刻意區隔。
    let mut illness_onset_events: Vec<(&'static str, bool)> = Vec::new();
    // 居民也會生病 v1：本 tick 鄰居陪伴事件 (陪伴者 id, 陪伴者名, 被陪伴者 id, 被陪伴者名)，
    // 鎖外落地雙方記憶 + 情誼加溫 + 城鎮動態（比照 800 飢餓時的守望相助的鎖外處理，守死鎖鐵律）。
    let mut illness_care_events: Vec<(String, &'static str, String, &'static str)> = Vec::new();
    // 乙太營火 v1：夜裡才需要一份營火座標快照。短鎖 clone 即釋，避免在 residents 寫鎖迴圈內
    // 再取 campfires 讀鎖（不巢狀鎖，守 prod 死鎖鐵律）；非夜晚時空 Vec，跳過整段判定。
    let campfire_spots: Vec<(i32, i32, i32)> = if matches!(phase, TimePhase::Night) {
        hub().campfires.read().unwrap().clone()
    } else {
        Vec::new()
    };
    // 木長椅 v1：與營火相反——白天（清醒時段：拂曉／白晝／黃昏）才需要一份長椅座標快照，
    // 讓路過的居民坐下歇腳。短鎖 clone 即釋，不巢狀鎖（守 prod 死鎖鐵律）；夜裡居民本就睡了，空 Vec 跳過整段。
    let bench_spots: Vec<(i32, i32, i32)> =
        if matches!(phase, TimePhase::Dawn | TimePhase::Day | TimePhase::Dusk) {
            hub().benches.read().unwrap().clone()
        } else {
            Vec::new()
        };

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

    // 居民認得你的家·登門拜訪你抵達事件（居民認得你的家 v1，自主提案切片，ROADMAP 830）：鎖內偵測
    // 「朝聖抵達的其實是你親手署名的家牌」，鎖外統一處理掛在你名下的記憶＋Feed（不需要 763 那樣的
    // 「回家感應」延遲，動態牆本就是你隨時能讀到的非同步 channel）。
    // 格式：(resident_id, resident_name, 被登門的玩家顯示名, 是否碰上本人)。
    let mut home_visit_events: Vec<(String, &'static str, String, bool)> = Vec::new();

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
    // 居民也會肚子餓 v1（ROADMAP 799）：鎖內偵測「居民在餓著的時候被玩家餵了食物」（餓意已於鎖內
    // 歸零），鎖外統一補一則掛玩家名下的深記憶＋城鎮動態。格式：(居民 id, 送食物的玩家名, 居民顯示名)。
    let mut hunger_fed: Vec<(String, String, &'static str)> = Vec::new();
    // 飢餓接農田／倉庫 v2（ROADMAP 799）：鎖內偵測「餓著沒存糧的居民走到一畦熟作物旁、要為了吃而收成」，
    // 鎖外統一執行（作物方塊 Mature→Seeded 退回可再長＋廣播＋持久化、食物入她小背包、清收成狀態、冒收成泡泡）。
    // 走鎖外是為守 prod 死鎖鐵律——收成要寫 deltas，而本迴圈全程持著 deltas 讀 guard（line 7441）。
    // 格式：(居民 id, 居民顯示名, wx, wy, wz, 作物方塊)。
    let mut forage_harvest_events: Vec<(String, &'static str, i32, i32, i32, Block)> = Vec::new();
    // 共用糧倉 v1：鎖內偵測「找不到熟作物、走到一個有存糧的箱子旁，要借一份」，鎖外統一執行
    // （箱子扣量＋持久化、食物入她小背包、冒借糧泡泡）。走鎖外是同一條 prod 死鎖鐵律——扣箱子
    // 要拿 `chest` 寫鎖，本迴圈全程只短取（讀鎖找目標即釋），寫入留到鎖外統一做。
    // 格式：(居民 id, 居民顯示名, wx, wy, wz)。
    let mut larder_take_events: Vec<(String, &'static str, i32, i32, i32)> = Vec::new();
    // 自我印象 v1（ROADMAP 770）：鎖內偵測「居民閒下來、回望自己昇華出一句自我印象」（泡泡已於鎖內設好），
    // 鎖外統一補一則城鎮動態 Feed（第三人稱旁白，已是純模板、無記憶原文）。
    let mut self_image_feeds: Vec<(&'static str, String)> = Vec::new();
    // 居民為你取一個名號 v1：鎖內打招呼時偵測「第一次為某玩家安下名號 / 名號改換」（名號招呼已於
    // 鎖內設好 r.say），鎖外統一補一則城鎮動態。格式：(居民顯示名, 旁白句)。
    let mut epithet_feeds: Vec<(&'static str, String)> = Vec::new();
    // 居民主動聊心事 v1（自主提案，781）：招呼時序內偵測「夠熟的居民對你掏心（把當前渴望當心事說出口）」
    // → 收集 (居民 id, 玩家顯示名)；residents 寫鎖釋放後再開 memory 寫鎖記一筆「我對這位旅人掏了心」
    // （episodic，累積好感），守「residents 寫鎖內不巢狀取 memory 寫鎖」的死鎖鐵律。
    let mut confide_mems: Vec<(String, String)> = Vec::new();
    // 居民察覺你翻過她的日記 v1（自主提案切片）：招呼時序內偵測「這位居民對你恰好有待發現的
    // 偷看旗標、且這次擲骰揭穿了」→ 收集 (居民 id, 玩家顯示名)；residents 寫鎖釋放後再開
    // memory 寫鎖記一筆「你翻過我的日記」（episodic），守「residents 寫鎖內不巢狀取 memory
    // 寫鎖」的死鎖鐵律（比照 `confide_mems` 同一手法）。
    let mut diary_peek_reveals: Vec<(String, String)> = Vec::new();
    // 居民教你一道獨門配方 v1（自主提案，ROADMAP 849）：招呼時序內偵測「感情深厚的居民主動
    // 教你一道獨門配方」→ 收集 (居民 id, 居民顯示名, 玩家顯示名)；residents 寫鎖釋放後才
    // 落地學會（player_recipes 寫鎖）+ 記憶（memory 寫鎖）+ Feed + 廣播，守死鎖鐵律。
    let mut recipe_teach_events: Vec<(String, &'static str, String)> = Vec::new();
    // 居民回饋糧倉 v1（自主提案）：鎖內偵測「有餘裕材料＋附近有一口已知箱子」→ 決定要存的
    // (item_id, qty)（泡泡已於鎖內設好 r.say），鎖外統一執行真正的轉移（res_inv 寫→chest 寫，
    // 各自短鎖循序不巢狀）。格式：(居民 id, 居民顯示名, wx, wy, wz, item_id, qty)。
    let mut chest_contribute_events: Vec<(String, &'static str, i32, i32, i32, u8, u32)> = Vec::new();
    // 居民見賢思齊 v1（自主提案，ROADMAP 858）：鎖內偵測「路過一座已命名地標且過機率門檻」→
    // 決定嚮往哪種建物（泡泡已於鎖內設好 r.say），鎖外統一種下心願（desires 寫）+ 持久化 + Feed，
    // 守「residents 寫鎖內不巢狀取 desires 寫鎖」的死鎖鐵律。格式：(居民 id, 居民顯示名, 地標名, 建物種類)。
    let mut envy_events: Vec<(String, &'static str, String, vbuild::BuildKind)> = Vec::new();
    // 把昨晚的夢說給你聽 v1（自主提案，807）：招呼時序內偵測「做過夢的居民把昨晚的夢分享給你」
    // → 收集 (居民 id, 玩家顯示名) 記一筆 episodic 記憶（累積好感），與 (居民顯示名, 玩家顯示名)
    // 補一則城鎮動態；同樣 residents 寫鎖釋放後才開 memory 寫鎖 / 動態 IO，守死鎖鐵律。
    let mut dreamshare_mems: Vec<(String, String)> = Vec::new();
    let mut dreamshare_feeds: Vec<(&'static str, String)> = Vec::new();
    // 居民拜託你幫個小忙 v1（自主提案）：招呼時序內偵測「夠面熟的居民主動向你討一樣材料」→ 收集
    // (居民顯示名, 材料名)；residents 寫鎖釋放後鎖外統一補一則城鎮動態牆（讓不在場 / 回來的玩家
    // 也讀到「某居民正想要某材料」）。純模板、只嵌居民名＋材料名、無記憶原文。
    let mut request_feeds: Vec<(&'static str, &'static str)> = Vec::new();
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

    // 做夢 Feed 事件（作息 × 記憶驅動行為·居民會做夢 v1，ROADMAP 805）：鎖內熟睡時偵測，鎖外記 Feed。
    // 格式：(resident_name, 夢見的那段珍貴往事摘要)。
    let mut dream_feed: Vec<(&'static str, String)> = Vec::new();

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

    // 戀人牽掛抵達事件（記憶驅動·戀人牽掛 v1，ROADMAP 852）：鎖內偵測，鎖外寫雙方記憶+Feed。
    // 格式：(seeker_id, seeker_name, partner_id, partner_name)。
    let mut lover_arrivals: Vec<(String, &'static str, String, String)> = Vec::new();

    // 居民回禮事件（ROADMAP 667）：鎖內偵測，鎖外執行（加入背包 + 廣播）。
    // 格式：(resident_id, resident_name, player_name, block_id, qty, message)
    let mut return_gift_events: Vec<(String, &'static str, String, u8, u32, String)> = Vec::new();

    // 居民關心你挨餓事件（自主提案切片，ROADMAP 845）：鎖內偵測「你在附近挨餓」，鎖外執行
    // （麵包入背包 + 廣播 + 記憶 + Feed）。格式：(resident_id, resident_name, player_name, block_id, qty)
    let mut hunger_care_events: Vec<(String, &'static str, String, u8, u32)> = Vec::new();

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

    // 邊陲探友 v1（ROADMAP 821）：批次快照目前「正遠行、已抵達邊陲逗留中（非夜間過夜熟睡）」居民的
    // id/落點/方位（名字 → id, 落點 x, z, 方位名），供下方 residents 寫鎖迴圈判斷「惦記的那位朋友，
    // 現在人在邊陲哪裡」；也一併帶 id，讓迴圈內（已 mutably borrow `residents` 無法再 `iter()`）
    // 仍能把驚喜回應塞進 `say_updates`——先讀後放，不與下方 residents 寫鎖巢狀（守死鎖鐵律）。
    let outpost_snap: HashMap<&'static str, (String, f32, f32, String)> = {
        let residents = hub().residents.read().unwrap();
        residents
            .iter()
            .filter_map(|r| {
                if r.expedition_stay > 0.0 && !r.asleep_at_outpost {
                    r.expedition
                        .clone()
                        .map(|(tx, tz, bearing)| (r.name, (r.id.clone(), tx, tz, bearing)))
                } else {
                    None
                }
            })
            .collect()
    }; // residents 讀鎖釋放

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
        // 戀人牽掛 v1（ROADMAP 852）：名字→(座標, 是否睡著, 居民id) 快照，供下方判斷戀人此刻在
        // 哪、醒著沒（避免在持 residents 寫鎖的迴圈裡再借用 residents 讀，比照 745/752 快照慣例）。
        let lover_status_by_name: HashMap<&'static str, (f32, f32, bool, String)> = residents
            .iter()
            .map(|r| (r.name, (r.body.x, r.body.z, r.asleep, r.id.clone())))
            .collect();
        // 名字→戀人名字快照（romance 讀鎖短取即釋、不巢狀不反向，比照下方 846 長椅段落的鎖序慣例）。
        let sweetheart_of: HashMap<&'static str, String> = {
            let romance = hub().romance.read().unwrap();
            resident_names
                .iter()
                .filter_map(|&nm| romance.partner_of(nm).map(|p| (nm, p)))
                .collect()
        }; // romance 讀鎖釋放

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
            // 餓意累積（居民也會肚子餓 v1，ROADMAP 799）：每 tick 慢慢餓一點；靜默冷卻同步倒數。
            // QA 加速：`BUTFUN_HUNGER_RATE_MULT`（預設 1.0）僅供隔離測試把數分鐘的餓意壓成數秒觀察，
            // 正式線上不設此環境變數→維持原速（15 分鐘餓極），對玩家零影響。
            r.hunger = vhunger::tick_hunger(r.hunger, dt * hunger_rate_mult());
            if r.hunger_say_cd > 0.0 {
                r.hunger_say_cd -= dt;
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
                // 熔爐冶煉煨煮倒數（第四刀）：只在真的開了爐（Some）時才倒數。
                if let Some(w) = &mut run.smelt_wait {
                    *w -= dt;
                }
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
                    // 仍是夜裡：安靜睡著，只落重力、不做任何行為——但熟睡中偶爾會做個夢（ROADMAP 805）。
                    // 做夢 v1：一段深藏心底的珍貴往事不由自主浮成夢，冒「💤 夢見…」泡泡＋記進動態 Feed，
                    // 讓夜裡路過的玩家與離線回訪者都瞥見居民「連睡夢裡都在活著」的內心。
                    // 與 744 就寢反思區隔：那是「躺下當下、有意識回味今天最近的事」；本刀是「睡著之後、
                    // 不由自主浮現、可觸及整座記憶庫裡更舊更深的珍藏」，觸發點／取樣／語氣皆不同。
                    if r.dream_cooldown > 0.0 {
                        r.dream_cooldown -= dt;
                    }
                    if r.say.is_empty()
                        && r.dream_cooldown <= 0.0
                        && vdream::should_dream(rand::random::<f32>())
                    {
                        // 短鎖讀整座記憶庫 → 濾掉哨兵偽玩家（只夢真實經歷，不夢自己的反思）→ 收集
                        // 珍貴（persistent）的往事 → 即釋。鎖序：residents 寫（外）→ memory 讀（內），
                        // 比照 744 就寢反思的短鎖循序，同向不巢狀、不反向、不持鎖 await。
                        let dream_core = {
                            let mem = hub().memory.read().unwrap();
                            let cherished: Vec<String> = mem
                                .all_memories_for(&r.id) // 最新在前
                                .into_iter()
                                .take(vdream::DREAM_WINDOW)
                                // 跳過就寢反思／遠行等哨兵偽玩家（皆 "__voxel_*__" 命名）——夢的是
                                // 真實經歷（玩家/鄰居/旅人），不是居民自己昨晚的反思，免遞迴夢反思。
                                .filter(|e| !e.player.starts_with("__"))
                                // 只夢「珍貴」的往事（目標／偏好／承諾／人際等 persistent），寒暄瑣事
                                // 不入夢——夢自然而然只浮現真正放在心上的事，也天然壓低頻率。
                                .filter(|e| {
                                    matches!(
                                        vmem::classify_importance(&e.summary),
                                        vmem::Importance::Persistent(_)
                                    )
                                })
                                .map(|e| e.summary)
                                .collect();
                            // 摻入「已做過幾個夢」讓 pick 逐夢變化（睡著身體靜止，只用座標會整夜同夢）。
                            let pick =
                                (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize ^ r.dream_seq as usize;
                            vdream::pick_dream(cherished.len(), pick).map(|i| cherished[i].clone())
                        }; // 記憶讀鎖在此釋放
                        if let Some(core) = dream_core {
                            let pick =
                                (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize ^ r.dream_seq as usize;
                            r.say = vdream::dream_bubble(&core, pick);
                            r.say_timer = SAY_SECS;
                            // 一個好夢，暖意延續一格（沿用既有心情補助機制，比照火邊聆聽 792）。
                            r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                            r.dream_cooldown = vdream::DREAM_COOLDOWN_SECS;
                            r.dream_seq = r.dream_seq.wrapping_add(1);
                            // 把這個夢的核心暫存起來，等白天遇到玩家時說給他聽（ROADMAP 807）——
                            // 覆蓋前一個沒說出口的夢（只留最近一個夢可分享，昨晚沒說成隔天不追溯）。
                            r.last_dream = Some(core.clone());
                            dream_feed.push((r.name, core));
                        }
                        // 沒有可夢的珍貴往事 → 無夢好眠，冷卻不重置、下次擲中再試。
                    }
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

            // 圍火講往事 v1·聆聽者應和倒數：被夥伴在營火邊講了故事後，延遲幾秒冒一句「聽故事」味道的
            // 專屬應和（零 LLM、程式化台詞），心情也亮一格。與上面的通用社交回應分開，讓火邊聆聽有專屬語氣。
            let tale_reply_ready = match &mut r.pending_tale_reply {
                Some((_, cd)) => {
                    *cd -= dt;
                    *cd <= 0.0
                }
                None => false,
            };
            if tale_reply_ready && r.say.is_empty() {
                if r.pending_tale_reply.take().is_some() {
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    r.say = vtale::listener_bubble(pick).to_string();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                }
            }

            // 長椅並坐閒聊 v1·被招呼者應和倒數：被熟人招呼在長椅上並肩坐下後，延遲幾秒冒一句嵌發起者名的
            // 專屬應和泡泡（零 LLM、程式化台詞）。與圍火聆聽／通用社交回應分開，讓並坐閒聊有專屬語氣。
            // 長椅拌嘴/和好 v1：`BenchOutcome` 決定應和該冒閒聊/拌嘴/和好哪一種台詞——拌嘴回嘴不設
            // 並坐 wait_timer、不加心情（吵架不是暖心事），閒聊/和好才並肩坐一會兒、心情亮一格。
            // 倒數與 take 都在居民自身鎖內、不巢狀。
            let bench_reply_ready = match &mut r.pending_bench_reply {
                Some((_, cd, _)) => {
                    *cd -= dt;
                    *cd <= 0.0
                }
                None => false,
            };
            if bench_reply_ready && r.say.is_empty() {
                if let Some((opener_name, _, outcome)) = r.pending_bench_reply.take() {
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    let line = match outcome {
                        vbtiff::BenchOutcome::Chat => vbenchchat::reply_line(&opener_name, pick),
                        vbtiff::BenchOutcome::Tiff => vbtiff::tiff_reply_line(&opener_name, pick),
                        vbtiff::BenchOutcome::MakeUp => vbtiff::makeup_reply_line(&opener_name, pick),
                    };
                    r.say = line.chars().take(vbenchchat::CHAT_SAY_CHARS).collect();
                    r.say_timer = SAY_SECS;
                    if outcome != vbtiff::BenchOutcome::Tiff {
                        // 被招呼者也停下來並肩坐一會兒（設 wait_timer，比照歇腳）。
                        r.wait_timer = r.wait_timer.max(vbench::REST_SIT_SECS);
                        r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    }
                }
            }

            // 技能互教·學生應和倒數（北極星第四刀）：被老朋友就地教了一手後，延遲幾秒冒一句
            // 「原來如此！」的專屬應和（台詞在教學那刻已從句式池組好，零 LLM）。與圍火聆聽／
            // 並坐應和分開，讓受教有一來一往的專屬語氣。倒數與 take 都在居民自身鎖內、不巢狀。
            let teach_reply_ready = match &mut r.pending_teach_reply {
                Some((_, cd)) => {
                    *cd -= dt;
                    *cd <= 0.0
                }
                None => false,
            };
            if teach_reply_ready && r.say.is_empty() {
                if let Some((line, _)) = r.pending_teach_reply.take() {
                    r.say = line.chars().take(50).collect();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                }
            }

            // 飢餓時的守望相助 v1·被分食者道謝倒數（ROADMAP 800）：被鄰居分了一口飯後，延遲幾秒冒一句
            // 嵌分食者名的專屬道謝泡泡（零 LLM、程式化台詞），心情也亮一格。與通用社交回應分開，讓
            // 「這頓解了餓」有專屬語氣。倒數與 take 都在居民自身鎖內、不巢狀。
            let meal_thanks_ready = match &mut r.pending_meal_thanks {
                Some((_, cd, _)) => {
                    *cd -= dt;
                    *cd <= 0.0
                }
                None => false,
            };
            if meal_thanks_ready && r.say.is_empty() {
                if let Some((sharer_name, _, is_repay)) = r.pending_meal_thanks.take() {
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    // 知恩圖報 v1（801）：被回報者用專屬道謝語氣（「你還記得那頓飯呀」），
                    // 與 800 一般分食道謝分開，讓「當年那口飯有了回聲」有專屬溫度。
                    let line = if is_repay {
                        vgrat::repay_thanks_line(&sharer_name, pick)
                    } else {
                        vsharemeal::thanks_line(&sharer_name, pick)
                    };
                    r.say = line.chars().take(40).collect();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                }
            }

            // 居民也會生病 v1·被陪伴者道謝倒數：被鄰居留下來陪了一會兒後，延遲幾秒冒一句嵌陪伴者名的
            // 專屬道謝泡泡（零 LLM、程式化台詞），心情也亮一格。倒數與 take 都在居民自身鎖內、不巢狀。
            let care_thanks_ready = match &mut r.pending_care_thanks {
                Some((_, cd)) => {
                    *cd -= dt;
                    *cd <= 0.0
                }
                None => false,
            };
            if care_thanks_ready && r.say.is_empty() {
                if let Some((carer_name, _)) = r.pending_care_thanks.take() {
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    r.say = villness::cared_thanks_line(&carer_name, pick)
                        .chars()
                        .take(40)
                        .collect();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
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
                        // 居民察覺你翻過她的日記 v1：這一拍最高優先——若這位居民對眼前這位玩家
                        // 恰好留著「待發現」的偷看旗標、且這次擲骰揭穿了，整拍改用「抓包」台詞，
                        // 蓋過底下名號／老友/陌生人問候（稀有一次性驚喜，優先於例行招呼）。
                        // 命中後立刻清旗標（不重複抓包同一次偷看）＋收集鎖外記一筆記憶。
                        let diary_reveal = !nearest_name.is_empty()
                            && r.diary_peeked.contains(nearest_name)
                            && rand::random::<f32>() < vdiarypeek::REVEAL_CHANCE;
                        let line = if diary_reveal {
                            r.diary_peeked.remove(nearest_name);
                            diary_peek_reveals.push((r.id.clone(), nearest_name.to_string()));
                            vdiarypeek::peek_reveal_line(nearest_name, pick)
                        } else if affinity >= vfond::FOND_AFFINITY && !nearest_name.is_empty() {
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

            // 主動聊心事 v1（自主提案，781）：招呼之外的另一拍——夠熟的居民偶爾**主動**把
            // 自己此刻懷著的渴望（`voxel_desires`）當成心事對你說出口。被動的日記內在，第一次
            // 變成她主動分享的話；「對你掏了心」也記進她對你的記憶、讓交情更深一層。
            // 與上面招呼刻意錯開：只在這一 tick 招呼沒觸發（`r.say` 仍空）時才可能發生，
            // 兩拍不同幀、不互相蓋泡泡。冷卻長（240s）＋好感門檻＝稀有有份量、天然防洗版。
            if r.confide_timer > 0.0 {
                r.confide_timer -= dt;
            } else if r.say.is_empty() && !r.seeking_comfort && r.approaching_esteem.is_none() {
                if let Some((d2, nearest_name)) = nearest_player_info(r.body.x, r.body.z, &player_pts)
                {
                    // 先過「靠得夠近＋低頻機率」再讀鎖（省鎖）；訪客名空（未綁定）不掏心。
                    if d2 < GREET_DIST * GREET_DIST
                        && !nearest_name.is_empty()
                        && rand::random::<f32>() < CONFIDE_CHANCE_PER_TICK
                    {
                        // 好感讀鎖即釋（與上面招呼同款短讀鎖，不巢狀、不持鎖 await）。
                        let affinity = hub().memory.read().unwrap().affinity_count(nearest_name, &r.id);
                        // cooldown_ok 由外層 `confide_timer <= 0` 分支保證；roll 已在上面過門檻，
                        // 這裡再過一次好感門檻（把「熟不熟該不該說」的判定集中在純函式裡）。
                        if vconfide::should_confide(affinity, true, 0.0, 1.0) {
                            // 有當前渴望才有心事可掏；desires 讀鎖即釋。
                            let desire = hub()
                                .desires
                                .read()
                                .unwrap()
                                .get_desire(&r.id)
                                .map(|d| d.desire.clone());
                            if let Some(d) = desire {
                                if !d.trim().is_empty() {
                                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                                    r.say = vconfide::confide_line(&d, pick)
                                        .chars()
                                        .take(vconfide::CONFIDE_SAY_MAX_CHARS)
                                        .collect();
                                    r.say_timer = SAY_SECS;
                                    r.confide_timer = vconfide::CONFIDE_COOLDOWN_SECS;
                                    r.mood_boost_secs =
                                        r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                    // 「對你掏了心」記進記憶（鎖外 flush，守死鎖鐵律）。
                                    confide_mems.push((r.id.clone(), nearest_name.to_string()));
                                }
                            }
                        }
                    }
                }
            }

            // 居民教你一道獨門配方 v1（自主提案，ROADMAP 849）：招呼／掏心之外的另一拍——
            // 感情深厚到門檻（`TEACH_MIN_AFFINITY`，高於老友問候/掏心門檻）的居民，偶爾會
            // 主動教你一道她的獨門配方（護身符），從此你自己也做得出來、永久解鎖。與掏心
            // （781）刻意區隔：掏心是說出當下渴望（會反覆發生、渴望常換），教配方是**一次性
            // 的永久解鎖**（學會了就不會再教同一道，冷卻也長得多）——居民↔居民早有傳授技能
            // （717），本刀第一次把「教」這件事接到居民↔玩家這條軸線上。
            if r.teach_timer > 0.0 {
                r.teach_timer -= dt;
            } else if r.say.is_empty() && !r.seeking_comfort && r.approaching_esteem.is_none() {
                if let Some((d2, nearest_name)) = nearest_player_info(r.body.x, r.body.z, &player_pts)
                {
                    // 先過「靠得夠近＋低頻機率」再讀鎖（省鎖）；訪客名空（未綁定）不教。
                    if d2 < GREET_DIST * GREET_DIST
                        && !nearest_name.is_empty()
                        && rand::random::<f32>() < TEACH_CHANCE_PER_TICK
                    {
                        // 好感 + 是否已學會，兩把短讀鎖各自即釋、不巢狀、不持鎖 await。
                        let affinity = hub().memory.read().unwrap().affinity_count(nearest_name, &r.id);
                        let already_known = hub()
                            .player_recipes
                            .read()
                            .unwrap()
                            .knows(nearest_name, vprecipe::TAUGHT_RECIPE_ID);
                        // cooldown_ok 由外層 `teach_timer <= 0` 分支保證；roll 已在上面過門檻，
                        // 這裡再過一次好感／已學會門檻（把判定集中在純函式裡）。
                        if vprecipe::should_teach_recipe(affinity, already_known, true, 0.0, 1.0) {
                            if let Some(recipe) = vcraft::find_taught_recipe(vprecipe::TAUGHT_RECIPE_ID) {
                                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                                r.say = vprecipe::teach_bubble(recipe.name_zh, pick)
                                    .chars()
                                    .take(vprecipe::TEACH_SAY_MAX_CHARS)
                                    .collect();
                                r.say_timer = SAY_SECS;
                                r.teach_timer = vprecipe::TEACH_COOLDOWN_SECS;
                                r.mood_boost_secs =
                                    r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                // 落地學會 + 記憶 + Feed + 廣播（鎖外 flush，守死鎖鐵律）。
                                recipe_teach_events.push((r.id.clone(), r.name, nearest_name.to_string()));
                            }
                        }
                    }
                }
            }

            // 居民回饋糧倉 v1（自主提案）：共用糧倉至今只讓居民向箱子「取」，本刀補上「存」的另一半——
            // 手上材料有餘裕、附近又有一口你已經用過的箱子時，居民閒晃途中偶爾順手把多的那份存進去。
            // 與招呼／掏心／教配方同款：只在這一 tick `r.say` 仍空時才可能發生，不同幀、不互蓋泡泡。
            if r.contribute_timer > 0.0 {
                r.contribute_timer -= dt;
            } else if r.say.is_empty() && !r.seeking_comfort && r.approaching_esteem.is_none() {
                // 先讀居民自己的採集背包挑一份餘裕材料（讀鎖即釋、不巢狀）；沒有餘料就不必再查箱子省鎖。
                let bag_pick = {
                    let bags = hub().res_inv.read().unwrap();
                    bags.get(&r.id).and_then(|b| vchestgive::pick_contribution(b))
                }; // res_inv 讀鎖釋放
                let chest_hit = bag_pick.and_then(|_| {
                    hub().chest.read().unwrap().nearest_known_chest(
                        r.body.x.floor() as i32,
                        r.body.z.floor() as i32,
                        vchestgive::CONTRIBUTE_RADIUS,
                    )
                }); // chest 讀鎖釋放
                if vchestgive::should_contribute(
                    bag_pick.is_some(),
                    chest_hit.is_some(),
                    rand::random::<f32>(),
                    vchestgive::CONTRIBUTE_CHANCE_PER_TICK,
                ) {
                    let (item_id, qty) = bag_pick.expect("should_contribute 已保證 bag_pick 為 Some");
                    let (cx, cy, cz) =
                        chest_hit.expect("should_contribute 已保證 chest_hit 為 Some");
                    let item_name = vgift::item_name_zh(item_id);
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    r.say = vchestgive::contribute_bubble(item_name, qty, pick)
                        .chars()
                        .take(vchestgive::CONTRIBUTE_SAY_MAX_CHARS)
                        .collect();
                    r.say_timer = SAY_SECS;
                    r.contribute_timer = vchestgive::CONTRIBUTE_COOLDOWN_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    // 真正的材料轉移 + 持久化 + Feed 留到鎖外統一做（守死鎖鐵律）。
                    chest_contribute_events.push((r.id.clone(), r.name, cx, cy, cz, item_id, qty));
                }
            }

            // 居民見賢思齊 v1（自主提案，ROADMAP 858）：居民的渴望至今只從對話/自我禱告/好奇心
            // 三個來源萌生，世界裡真實存在、被居民命名記住的地標（773/854）從沒觸發過任何居民的
            // 心願——本刀補上第四個來源：閒晃路過一座已命名地標，偶爾心生嚮往，也想擁有一座自己
            // 的類似建物。與招呼／掏心／教配方／回饋糧倉同款：只在這一 tick `r.say` 仍空時才可能
            // 發生，不同幀、不互蓋泡泡。
            if r.envy_timer > 0.0 {
                r.envy_timer -= dt;
            } else if r.say.is_empty() && !r.seeking_comfort && r.approaching_esteem.is_none() {
                // 只讀分格鍵當下是否已被命名（Mutex 短鎖即釋、不巢狀）；未命名的地方安靜省鎖。
                let cell = vstructname::cell_key(r.body.x, r.body.z);
                let named = structure_names().lock().unwrap().get(&cell).cloned();
                // 860：這座地標若是自己蓋的家（owner == 自己 id），不觸發羨慕——
                // 羨慕的前提是「別人的美好」，不會有人羨慕自己親手蓋的家。
                if let Some((structure_name, owner)) = named {
                    if owner.as_deref() != Some(r.id.as_str())
                        && venvy::should_envy(true, rand::random::<f32>())
                    {
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                        let kind = venvy::pick_envy_kind(pick);
                        r.say = venvy::envy_say_line(&structure_name, kind, pick);
                        r.say_timer = SAY_SECS;
                        r.envy_timer = venvy::ENVY_COOLDOWN_SECS;
                        r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        // 真正種下心願 + 持久化 + Feed 留到鎖外統一做（守死鎖鐵律）。
                        envy_events.push((r.id.clone(), r.name, structure_name, kind));
                    }
                }
            }

            // 把昨晚的夢說給你聽 v1（自主提案，807）：招呼 / 掏心之外的另一拍——夜裡做過夢（805）
            // 的居民，白天遇到你時偶爾主動把昨晚那個夢分享出來。夜裡孤零零浮現的夢，第一次有了
            // 白天的回響與聽眾；「把夢說給你聽」的親近也記進記憶、加深情誼。與掏心（781）刻意區隔：
            // 那是私密的當前渴望、要熟才說；夢是輕盈奇妙的、遇到你就可能說（不設好感門檻），且因果
            // 綁在「昨晚真的做了那個夢」——沒夢就沒得說。與上面幾拍同款：只在這一 tick 招呼／掏心
            // 都沒觸發（`r.say` 仍空）時才可能發生，不同幀、不互蓋泡泡。
            if r.dreamshare_timer > 0.0 {
                r.dreamshare_timer -= dt;
            } else if r.say.is_empty()
                && !r.seeking_comfort
                && r.approaching_esteem.is_none()
                && r.last_dream.is_some()
            {
                if let Some((d2, nearest_name)) = nearest_player_info(r.body.x, r.body.z, &player_pts)
                {
                    // 靠得夠近 ＋ 非訪客（名非空）＋ 過低頻機率 → 分享（純函式集中判定，無鎖）。
                    let near = d2 < GREET_DIST * GREET_DIST;
                    if vdreamshare::should_share_dream(
                        true,
                        near && !nearest_name.is_empty(),
                        true,
                        rand::random::<f32>(),
                        vdreamshare::DREAMSHARE_CHANCE_PER_TICK,
                    ) {
                        // take() 取出並清空那個夢——說過就不再重複說同一個夢（下次做夢再覆蓋成新的）。
                        let core = r.last_dream.take().unwrap_or_default();
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize
                            ^ r.dream_seq as usize;
                        r.say = vdreamshare::dreamshare_line(&core, nearest_name, pick);
                        r.say_timer = SAY_SECS;
                        r.dreamshare_timer = vdreamshare::DREAMSHARE_COOLDOWN_SECS;
                        r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        // 「把夢說給你聽」記進記憶 ＋ 城鎮動態（鎖外 flush，守死鎖鐵律）。
                        dreamshare_mems.push((r.id.clone(), nearest_name.to_string()));
                        dreamshare_feeds.push((r.name, nearest_name.to_string()));
                    }
                }
            }

            // 拜託你幫個小忙 v1（自主提案）：招呼 / 掏心之外的另一拍——夠面熟的居民偶爾**反過來**
            // 主動向你討一樣好採集的基礎材料（木/石/煤/沙）。至今「幫忙採集」永遠是玩家對居民下令
            // （`voxel_fetch`）；這裡第一次讓居民對你開口，把「採集」這條人類樂趣接上「居民的需要」。
            // 與招呼 / 掏心刻意錯開：只在這一 tick 前面都沒觸發（`r.say` 仍空）時才可能發生，
            // 兩拍不同幀、不互相蓋泡泡。冷卻長（300s）＋好感門檻＋同時只掛一個未了請求＝稀有防洗版。
            if r.request_timer > 0.0 {
                r.request_timer -= dt;
            } else if r.say.is_empty()
                && !r.seeking_comfort
                && r.approaching_esteem.is_none()
                && r.open_request.is_none()
            {
                if let Some((d2, nearest_name)) =
                    nearest_player_info(r.body.x, r.body.z, &player_pts)
                {
                    // 先過「靠得夠近＋低頻機率」再讀鎖（省鎖）；訪客名空（未綁定）不開口討東西。
                    if d2 < GREET_DIST * GREET_DIST
                        && !nearest_name.is_empty()
                        && rand::random::<f32>() < REQUEST_CHANCE_PER_TICK
                    {
                        // 好感讀鎖即釋（與招呼 / 掏心同款短讀鎖，不巢狀、不持鎖 await）。
                        let affinity =
                            hub().memory.read().unwrap().affinity_count(nearest_name, &r.id);
                        // has_open_request 由外層 `r.open_request.is_none()` 分支保證為 false；
                        // roll 已在上面過門檻，這裡把「熟不熟該不該討」的判定集中在純函式裡。
                        if vrequest::should_post_request(affinity, true, false, 0.0, 1.0) {
                            let pick =
                                (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                            let req = vrequest::pick_request(pick);
                            r.say = vrequest::request_line(req.name, pick)
                                .chars()
                                .take(vrequest::REQUEST_SAY_MAX_CHARS)
                                .collect();
                            r.say_timer = SAY_SECS;
                            r.request_timer = vrequest::REQUEST_COOLDOWN_SECS;
                            r.open_request = Some(req.item_id);
                            // 城鎮動態牆（鎖外 flush，守死鎖鐵律）。
                            request_feeds.push((r.name, req.name));
                        }
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
                if let Some((sx, sz, text, _, owner)) = nearby.filter(|(_, _, t, _, _)| !t.is_empty()) {
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    let quote = vreadsign::display_quote(&text);
                    // 居民認得鄰居的家 v1（ROADMAP 750）：先看這塊牌是不是**別的居民**立的自建
                    // 銘牌（749，格式「{名}的{建物}」）。是的話認出鄰居、念一句更親暱的招呼；
                    // 否則落回既有的世界級讀牌路徑（741/742/743），行為完全不變。
                    let neighbor = vneighsign::identify_nameplate(&text, r.name, &RESIDENT_NAMES);
                    // 居民認得你的家 v1（自主提案切片，ROADMAP 830）：不是鄰居自建銘牌時，再看這塊
                    // 牌是不是**某位玩家**親手署名（`owner`，伺服器權威記下）、且牌面語氣被判成
                    // 「家」（[`vreadsign::SignTone::Home`]，含「家/屋/窩/居/巢」）——避免玩家隨手寫的
                    // 路標／留言也被誤認成家。兩者互斥：是鄰居的牌就不會又是玩家的家。
                    let home_owner = if neighbor.is_none()
                        && vreadsign::classify(&text) == vreadsign::SignTone::Home
                    {
                        owner.clone()
                    } else {
                        None
                    };
                    r.say = match neighbor {
                        Some(nb) => vneighsign::neighbor_sign_line(nb, &quote, pick),
                        None => match &home_owner {
                            Some(player) => vplayerhome::recognized_line(player, pick),
                            None => vreadsign::read_sign_line(&text, pick),
                        },
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
                            None => match &home_owner {
                                // 830：認出是某位玩家親手署名的家牌 → 記憶**掛在那位玩家名下**，
                                // 讓「你的互動有後果」第一次伸向「你在世界裡安的家」。
                                Some(player) => {
                                    let mem = vplayerhome::recognized_memory(player);
                                    let e = hub()
                                        .memory
                                        .write()
                                        .unwrap()
                                        .add_memory(&r.id, player, &mem);
                                    vfeed::append_feed(
                                        "認得你的家",
                                        r.name,
                                        &vplayerhome::recognized_feed(r.name, player),
                                    );
                                    e
                                }
                                // 既有路徑：玩家寫的字（非家牌／訪客），掛世界級哨兵鍵，不污染真實玩家好感。
                                None => {
                                    let summary = vreadsign::sign_memory_summary(&text);
                                    hub().memory.write().unwrap().add_memory(
                                        &r.id,
                                        vreadsign::SIGN_MEMORY_PLAYER,
                                        &summary,
                                    )
                                }
                            },
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
                        // 居民認得你的家 v1（830）：同步記下「這塊地標其實是哪位玩家的家」——與
                        // `cherished_neighbor` 互斥，讓日後朝聖抵達能第一次把「重返」升級成登門拜訪你。
                        r.cherished_player = home_owner;
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
                                // 時令的贈禮花園 v1：收成正逢當季 → 說出「還趕上時令、多結了一份」的暖句
                                //（回贈數量在鎖外套用事件時同樣走時令版 produce_gift_timely）。
                                r.say = if vgg::is_timely(crop, current_season) {
                                    vgg::harvest_say_line_timely(
                                        nearest_name,
                                        cname,
                                        current_season.display_name(),
                                        pick,
                                    )
                                } else {
                                    vgg::harvest_say_line(nearest_name, cname, pick)
                                }
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

            // 戀人牽掛 v1（記憶驅動·戀人牽掛，ROADMAP 852）：846 讓兩位老朋友締結成戀人，但成了
            // 戀人之後這份羈絆從沒有改變過任何行為——本刀讓「戀人」第一次真的影響行為：分開得夠遠、
            // 冷卻到期、戀人醒著、過機率門檻，就放下手邊的事，起身去找對方。
            if r.lover_seek.is_none()
                && r.say.is_empty()
                && !r.asleep
                && !r.seeking_food
                && !r.foraging_food
                && !r.seeking_comfort
                && r.cheer_target.is_none()
                && r.visiting.is_none()
                && r.clique_meet.is_none()
                && r.approaching_esteem.is_none()
                && r.expedition.is_none()
                && r.pilgrimage.is_none()
                && r.daybreak_seek.is_none()
                && r.reunion_seek.is_none()
                && r.follow.is_none()
                && r.gather.is_none()
                && r.fetch.is_none()
                && r.invent_run.is_none()
            {
                if let Some(partner_name) = sweetheart_of.get(r.name) {
                    if let Some(&(px, pz, partner_asleep, _)) =
                        lover_status_by_name.get(partner_name.as_str())
                    {
                        let dx = r.body.x - px;
                        let dz = r.body.z - pz;
                        let dist_sq = dx * dx + dz * dz;
                        if vlover::should_seek(
                            dist_sq,
                            r.lover_seek_cooldown,
                            partner_asleep,
                            rand::random::<f32>(),
                        ) {
                            let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                            r.target_x = px;
                            r.target_z = pz;
                            r.wait_timer = 0.0;
                            r.lover_seek = Some((partner_name.clone(), vlover::SEEK_TIMEOUT_SECS));
                            r.say = vlover::seek_bubble_line(pick).chars().take(40).collect();
                            r.say_timer = SAY_SECS;
                        }
                    }
                }
            }

            // 回家吃自己種的／存的 v2（飢餓接農田／倉庫，ROADMAP 799）：餓了就放下閒晃、朝自己的
            // 家域中心走去；到家後**檢查小背包真的有沒有吃的**（小麥／麵包／胡蘿蔔／馬鈴薯／莓果…），
            // 有就吃掉一份、真的扣量、餓意歸零、冒一句點名「吃了什麼」的滿足暖泡泡；**沒有就真的餓著**
            // ——誠實心聲「家裡什麼吃的都沒有…」，改去把附近熟了的作物收成進背包（見下方 foraging_food）。
            // 建築（田／倉）第一次有真正的功能：吃的是自己種的、存的，不是憑空。純自理、不黏玩家。
            // 鏡像尋伴／致意的「逐 tick 重設目標→抵達即結」機制。res_inv 短鎖即釋（守 prod 死鎖鐵律：
            // residents 寫→res_inv 寫，比照本檔既有回禮/合成處對 res_inv 的短取即釋慣例）。
            if r.seeking_food {
                let dx = r.home_x - r.body.x;
                let dz = r.home_z - r.body.z;
                if dx * dx + dz * dz
                    < vhunger::EAT_ARRIVE_DIST * vhunger::EAT_ARRIVE_DIST
                {
                    // 到家：查小背包挑一份食物、真的吃掉（扣量）；沒得吃回 None。
                    let eaten: Option<u8> = {
                        let mut inv = hub().res_inv.write().unwrap();
                        let bag = inv.entry(r.id.clone()).or_default();
                        match vhunger::pick_food(bag) {
                            Some(fid) if vhunger::consume_one(bag, fid) => Some(fid),
                            _ => None,
                        }
                    }; // res_inv 寫鎖釋放
                    r.seeking_food = false;
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    if let Some(fid) = eaten {
                        // 吃上了自己種的／存的：餓意歸零、上靜默冷卻，冒一句點名「吃了什麼」的暖泡泡。
                        r.hunger = 0.0;
                        r.hunger_say_cd = vhunger::HUNGER_SAY_COOLDOWN;
                        if r.say.is_empty() {
                            let food = vhunger::food_name_zh(fid).unwrap_or("東西");
                            r.say = vhunger::ate_say_line(food, pick).chars().take(40).collect();
                            r.say_timer = SAY_SECS;
                        }
                    } else {
                        // 家裡什麼吃的都沒有——真的餓著（餓意**不歸零**）：冒一句誠實心聲，轉去收成覓食。
                        // 仍上靜默冷卻，只是為了避免同一輪又立刻落回「餓了喊一句回家」的碎念；覓食由
                        // `foraging_food` 旗接手驅動（不受冷卻影響），冷卻過後若還沒吃到會再想辦法。
                        r.hunger_say_cd = vhunger::HUNGER_SAY_COOLDOWN;
                        r.foraging_food = true;
                        r.forage_target = None;
                        if r.say.is_empty() {
                            r.say = vhunger::no_food_say_line(pick).chars().take(40).collect();
                            r.say_timer = SAY_SECS;
                        }
                    }
                } else {
                    // 還沒到家：持續把目標釘在家域中心（純自走回家，不受玩家/世界牽引）。
                    r.target_x = r.home_x;
                    r.target_z = r.home_z;
                    r.wait_timer = 0.0;
                }
            } else if vhunger::is_hungry(r.hunger)
                && !r.foraging_food
                && r.hunger_say_cd <= 0.0
                && r.say.is_empty()
                && r.gather.is_none()
                && r.fetch.is_none()
                && r.visiting.is_none()
                && r.cheer_target.is_none()
                && !r.seeking_comfort
                && r.approaching_esteem.is_none()
                && r.clique_meet.is_none()
                && r.follow.is_none()
                && r.invent_run.is_none()
                && r.daybreak_seek.is_none()
                && r.reunion_seek.is_none()
                && r.expedition.is_none()
                && r.pilgrimage.is_none()
                && !r.asleep
            {
                // 餓了、又閒著：冒一句餓的心聲，起身回家覓食。
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                r.say = vhunger::hunger_say_line(pick).chars().take(40).collect();
                r.say_timer = SAY_SECS;
                r.seeking_food = true;
                r.hunger_say_cd = vhunger::HUNGER_SAY_COOLDOWN;
                r.target_x = r.home_x;
                r.target_z = r.home_z;
                r.wait_timer = 0.0;
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

            // 戀人牽掛走動 v1（記憶驅動·戀人牽掛，ROADMAP 852）：lover_seek 有效時，持續朝那位
            // 戀人此刻所在位置走去（她可能在移動，故每 tick 依名字重查快照座標步步逼近）；抵達
            // （XZ 距離 < ARRIVE_DIST 且 say 空）→ 暖暖相見、收集抵達事件（鎖外補雙方各一筆重逢
            // 記憶+Feed）、清除牽掛並上冷卻。戀人睡了／逾時奔太久則放下這份牽掛。狀態機比照久別
            // 重逢奔迎（747，每 tick 覆寫 target、抵達即清空）。
            if let Some((partner_name, remaining)) = r.lover_seek.clone() {
                let remaining = remaining - dt;
                // 依名字重查戀人此刻的座標與是否睡著（可能已移動／已入睡）。
                let here = lover_status_by_name
                    .get(partner_name.as_str())
                    .map(|&(x, z, asleep, _)| (x, z, asleep));
                match here {
                    Some((px, pz, false)) if remaining > 0.0 => {
                        let dx = r.body.x - px;
                        let dz = r.body.z - pz;
                        let arrived = dx * dx + dz * dz < vlover::ARRIVE_DIST * vlover::ARRIVE_DIST;
                        if arrived {
                            // 找到戀人：沒在說別的話就暖暖相見、收集雙方重逢記憶事件，收工＋上冷卻。
                            if r.say.is_empty() {
                                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                                r.say = vlover::arrive_greet_line(pick).chars().take(40).collect();
                                r.say_timer = SAY_SECS;
                                if let Some(entry) = lover_status_by_name.get(partner_name.as_str())
                                {
                                    let partner_id = entry.3.clone();
                                    lover_arrivals.push((
                                        r.id.clone(),
                                        r.name,
                                        partner_id,
                                        partner_name.clone(),
                                    ));
                                }
                                r.lover_seek = None;
                                r.lover_seek_cooldown = vlover::SEEK_COOLDOWN_SECS;
                            } else {
                                // 站定等說完話：保持目標、續留這份牽掛（逾時照遞減，不至無限等）。
                                r.target_x = px;
                                r.target_z = pz;
                                r.wait_timer = 0.0;
                                r.lover_seek = Some((partner_name, remaining));
                            }
                        } else {
                            // 還在路上：步步逼近、清小歇，讓玩家看得到她專程走過去。
                            r.target_x = px;
                            r.target_z = pz;
                            r.wait_timer = 0.0;
                            r.lover_seek = Some((partner_name, remaining));
                        }
                    }
                    // 戀人睡了／逾時奔太久 → 放下這份牽掛，回到平常的一天（短冷卻，避免立刻又觸發）。
                    _ => {
                        r.lover_seek = None;
                        r.lover_seek_cooldown = vlover::SEEK_COOLDOWN_SECS * 0.25;
                    }
                }
            }

            // 重返心中的牌子 v3（ROADMAP 743）：讀牌記憶第一次改變居民的去向。
            // 冷卻遞減；正在朝聖時持續朝牌子走、抵達即駐足念一句、逾時則放棄。
            // 鎖序：memory 寫（短鎖即釋，比照 v2），Feed 走鎖外 pilgrimage_feed，不巢狀、不持鎖 await。
            if r.pilgrimage_cooldown > 0.0 {
                r.pilgrimage_cooldown -= dt;
            }

            // 戀人牽掛 v1（ROADMAP 852）：牽掛冷卻遞減（純記憶體、每 tick 一次）。
            if r.lover_seek_cooldown > 0.0 {
                r.lover_seek_cooldown -= dt;
            }

            // 繁星夜空 v1（ROADMAP 783）：望星冷卻遞減（純記憶體、每 tick 一次）。
            if r.stargaze_cooldown > 0.0 {
                r.stargaze_cooldown -= dt;
            }
            // 睹物思人 v1（ROADMAP 784）：追憶冷卻遞減（純記憶體、每 tick 一次）。
            if r.keepsake_recall_cooldown > 0.0 {
                r.keepsake_recall_cooldown -= dt;
            }
            // 哼歌 v1（ROADMAP 788）：哼歌冷卻遞減（純記憶體、每 tick 一次）。
            if r.humming_cooldown > 0.0 {
                r.humming_cooldown -= dt;
            }
            // 乙太營火 v1：取暖冷卻遞減（純記憶體、每 tick 一次）。
            if r.campfire_warm_cooldown > 0.0 {
                r.campfire_warm_cooldown -= dt;
            }
            if r.campfire_tale_cooldown > 0.0 {
                r.campfire_tale_cooldown -= dt;
            }
            // 木長椅 v1：歇腳冷卻遞減（純記憶體、每 tick 一次）。
            if r.bench_rest_cooldown > 0.0 {
                r.bench_rest_cooldown -= dt;
            }
            // 長椅並坐閒聊 v1：並坐冷卻遞減（純記憶體、每 tick 一次）。
            if r.bench_chat_cooldown > 0.0 {
                r.bench_chat_cooldown -= dt;
            }
            // 居民臨水垂釣 v1：垂釣冷卻遞減（純記憶體、每 tick 一次）。
            if r.angler_cooldown > 0.0 {
                r.angler_cooldown -= dt;
            }
            // 雨天葉傘避雨 v1：躲雨冷卻遞減（純記憶體、每 tick 一次）。
            if r.rain_shelter_cooldown > 0.0 {
                r.rain_shelter_cooldown -= dt;
            }
            // 居民顧家駐足 v1：顧家冷卻遞減（純記憶體、每 tick 一次）。
            if r.homegaze_cooldown > 0.0 {
                r.homegaze_cooldown -= dt;
            }
            // 居民關心你挨餓 v1：關心冷卻遞減（純記憶體、每 tick 一次）。
            if r.hunger_care_cooldown > 0.0 {
                r.hunger_care_cooldown -= dt;
            }
            // 邊陲巧遇 v1：巧遇冷卻遞減（純記憶體、每 tick 一次）。
            if r.frontier_find_cooldown > 0.0 {
                r.frontier_find_cooldown -= dt;
            }
            // 飢餓時的守望相助 v1（ROADMAP 800）：分食冷卻遞減（純記憶體、每 tick 一次）。
            if r.share_meal_cooldown > 0.0 {
                r.share_meal_cooldown -= dt;
            }
            // 居民也會生病 v1（自主提案）：發病／陪伴冷卻遞減；生病中的病況自然消退
            //（靠自己休息也會漸漸好轉，無論此刻在做什麼都持續消退，比照餓意持續累積）。
            if r.illness_cooldown > 0.0 {
                r.illness_cooldown -= dt;
            }
            if r.care_cooldown > 0.0 {
                r.care_cooldown -= dt;
            }
            if villness::is_sick(r.illness_severity) {
                r.illness_severity = villness::tick_recover(r.illness_severity, dt);
                if !villness::is_sick(r.illness_severity) && r.say.is_empty() && !r.asleep {
                    // 靠自己扛過去、自然痊癒（沒人陪伴/送湯也走到終點）。
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    r.say = villness::recovered_bubble(pick).to_string();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    illness_recovered_events.push(r.name);
                }
            }
            // 集會鐘 v1：應召逾時遞減（純記憶體、每 tick 一次，不管居民此刻走哪條移動分支都算），
            // 逾時歸零即放棄應召（例如鐘被挖了、或被別的高優先任務卡著走不到）——守「卡住自救」。
            if let Some(sm) = r.summon.as_mut() {
                sm.timer -= dt;
                if sm.timer <= 0.0 {
                    r.summon = None;
                }
            }
            // 應召冷卻遞減（濫用防護主閘：一位居民應召一次後隔一段才會再被鐘聲拉動）。
            if r.summon_cooldown > 0.0 {
                r.summon_cooldown -= dt;
            }
            // 睹物思人 v1（ROADMAP 784）：閒著、醒著、且恰好路過她擺出的某件你送的紀念物時，
            // 偶爾駐足追憶一句、記進交情——keepsake（732）落地後的持續回響（記憶→行為）。
            // say 為空、醒著、手邊沒重要事才觸發，不搶正事；長冷卻＋極低機率＝天然節流。
            // 鎖序：純讀 r.keepsake_spots（居民自身欄位，已持居民寫鎖），記憶寫＋Feed 走鎖外
            // `keepsake_recall_events`，守死鎖鐵律。
            if !r.keepsake_spots.is_empty()
                && r.say.is_empty()
                && !r.asleep
                && r.keepsake_recall_cooldown <= 0.0
                && r.pilgrimage.is_none()
                && r.expedition.is_none()
            {
                if let Some(idx) =
                    vkrecall::nearest_spot(&r.keepsake_spots, r.body.x, r.body.z, vkrecall::RECALL_NEAR_RADIUS)
                {
                    if vkrecall::should_recall(
                        true, // nearest_spot 已確認在半徑內
                        true, // 冷卻已在外層確認過
                        rand::random::<f32>(),
                        vkrecall::RECALL_CHANCE,
                    ) {
                        r.keepsake_recall_cooldown = vkrecall::RECALL_COOLDOWN_SECS;
                        let spot = r.keepsake_spots[idx].clone();
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                        r.say = vkrecall::recall_line(&spot.giver, &spot.item, pick);
                        r.say_timer = SAY_SECS;
                        r.mood_boost_secs =
                            r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        // 只有登入玩家（送禮者名非空）才記進交情、上動態牆（訪客送的無持久身份）。
                        if !spot.giver.is_empty() {
                            keepsake_recall_events.push((
                                r.id.clone(),
                                r.name,
                                spot.giver.clone(),
                                spot.item.clone(),
                                pick,
                            ));
                        }
                    }
                }
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
                        } else if let Some(player) = r.pilgrimage_player.clone() {
                            // 居民認得你的家 v1（自主提案切片，ROADMAP 830）：這趟走向的其實是你親手
                            // 署名的家牌（830 認得的、啟程時快照進 pilgrimage_player），抵達就不再只是
                            // 對牌自言自語，而是一趟真正的「登門拜訪你」——你在家（此刻在線且站在牌子
                            // 附近，複用 752 的在家半徑）就碰上本人暖招呼；你不在（離線或離得遠）就撲空，
                            // 在城鎮動態留一句「今天繞去找過你」（鎖外 home_visit_events 統一處理記憶／Feed，
                            // 不需要像 763 那樣等你「回家感應」——動態牆本就是你隨時能讀到的非同步channel）。
                            let player_here = player_pts
                                .iter()
                                .find(|(_, _, n)| n == &player)
                                .map(|&(px, pz, _)| vplayerhome::player_is_home(px, pz, tx, tz))
                                .unwrap_or(false);
                            r.say = if player_here {
                                vplayerhome::visit_present_line(&player, pick)
                            } else {
                                vplayerhome::visit_missed_line(&player, pick)
                            };
                            r.say_timer = SAY_SECS;
                            home_visit_events.push((r.id.clone(), r.name, player, player_here));
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
                        r.pilgrimage_player = None;
                        r.pilgrimage_cooldown = vreadsign::PILGRIMAGE_COOLDOWN;
                    }
                } else {
                    // 還在路上：遞減逾時；走太久（地形擋路等）沒到就放棄，設冷卻，不無限走。
                    r.pilgrimage_timer -= dt;
                    if r.pilgrimage_timer <= 0.0 {
                        r.pilgrimage = None;
                        r.pilgrimage_neighbor = None;
                        r.pilgrimage_player = None;
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
                            // 居民認得你的家 v1（830）：同理快照「這趟走向的是哪位玩家的家」。
                            r.pilgrimage_player = r.cherished_player.clone();
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
                    // 餓時被餵（居民也會肚子餓 v1，ROADMAP 799）：若這口吃的正落在牠餓著的時候，
                    // 餓意當場歸零、結束覓食，並記得格外深——你的善意踩在對的時間點上。深記憶＋Feed
                    // 走鎖外（守死鎖鐵律，不在 residents 寫鎖內取 memory 寫鎖）。
                    if vhunger::is_hungry(r.hunger) {
                        r.hunger = 0.0;
                        r.seeking_food = false;
                        r.hunger_say_cd = vhunger::HUNGER_SAY_COOLDOWN;
                        hunger_fed.push((r.id.clone(), giver.clone(), r.name));
                    }
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

            // ── 邊陲探友 v1（ROADMAP 821·PLAN_ETHERVOX item 4「居民↔居民關係」× item 7「散居」）───
            // 散居（756~762）與探訪（671）至今互不相干：朋友遠行在邊陲時，跨域探訪仍只會走到牠
            // 空無一人的家。本段讓留守主城的人格（市集人·露娜／廣場人·賽勒，`expedition_motive`
            // 回 `None`）若跟正在邊陲逗留的老朋友交情夠深，偶爾放下手邊的事、跋涉去邊陲找她——
            // 讀 `outpost_snap` 即時座標（不是家域座標），找到朋友那一刻兩人在荒野盡頭重聚。
            // 鎖序：bonds 讀（即釋）；記憶寫＋Feed 走鎖外 frontier_visit_arrive_events／
            // frontier_visit_feed，不巢狀、不持鎖 await（守死鎖鐵律）。
            if r.frontier_visit_cooldown > 0.0 {
                r.frontier_visit_cooldown -= dt;
            }
            if let Some((tx, tz, bearing, friend)) = r.frontier_visit.clone() {
                if r.frontier_visit_stay > 0.0 {
                    // 已找到朋友、正在邊陲小聚：倒數，歸零即道別啟程返家。
                    r.frontier_visit_stay -= dt;
                    if r.frontier_visit_stay <= 0.0 {
                        r.frontier_visit = None;
                        r.frontier_visit_cooldown = vfvisit::COOLDOWN_SECS;
                        r.target_x = r.home_x;
                        r.target_z = r.home_z;
                        frontier_visit_feed.push((r.name, vfvisit::depart_home_feed_line(&friend)));
                    }
                } else if !outpost_snap.contains_key(friend.as_str()) {
                    // 朋友半路已離開邊陲（提前歸來）：這趟撲空，放棄並回家。
                    r.frontier_visit = None;
                    r.frontier_visit_cooldown = vfvisit::COOLDOWN_SECS;
                    r.target_x = r.home_x;
                    r.target_z = r.home_z;
                    frontier_visit_feed.push((r.name, vfvisit::giveup_feed_line(&friend)));
                } else {
                    // 去程：持續朝朋友的邊陲落點走。
                    r.target_x = tx;
                    r.target_z = tz;
                    r.wait_timer = 0.0;
                    let dx = r.body.x - tx;
                    let dz = r.body.z - tz;
                    if dx * dx + dz * dz < vfvisit::ARRIVE_DIST * vfvisit::ARRIVE_DIST {
                        // 抵達，找到朋友了！沒在說別的話才冒相聚泡泡（正巧在說話就等下 tick）。
                        if r.say.is_empty() {
                            r.frontier_visit_stay = vfvisit::STAY_SECS;
                            let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                            r.say = vfvisit::arrive_bubble(&friend, pick);
                            r.say_timer = SAY_SECS;
                            frontier_visit_arrive_events.push((
                                r.id.clone(),
                                r.name,
                                friend.clone(),
                                bearing.clone(),
                            ));
                            // 被找到的朋友驚喜回應（她原本沒在說話才冒出，走既有 say_updates 統一套用；
                            // id 取自快照，不再向已被 iter_mut 借用的 residents 另要一次 iter()）。
                            if let Some((friend_id, ..)) = outpost_snap.get(friend.as_str()) {
                                say_updates
                                    .push((friend_id.clone(), vfvisit::host_reply_bubble(r.name, pick)));
                            }
                        }
                    } else {
                        r.frontier_visit_timer -= dt;
                        if r.frontier_visit_timer <= 0.0 {
                            r.frontier_visit = None;
                            r.frontier_visit_cooldown = vfvisit::COOLDOWN_SECS;
                            frontier_visit_feed.push((r.name, vfvisit::giveup_feed_line(&friend)));
                        }
                    }
                }
            } else {
                // 尚未探友：留守人格 + 閒置自由 + 冷卻到期 + 沒在說話 → 檢查是否有老朋友正在邊陲。
                let town_bound = vexp::expedition_motive(r.persona).is_none();
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
                if town_bound && idle_free && r.say.is_empty() && r.frontier_visit_cooldown <= 0.0 {
                    // 挑第一位交情達老朋友、此刻確實在邊陲的朋友（本世界僅 4 位居民，遍歷成本可忽略）。
                    // bonds 讀鎖只取一次（即釋，不巢狀）。
                    let found = {
                        let bonds = hub().bonds.read().unwrap();
                        outpost_snap.iter().find(|(name, _)| {
                            **name != r.name && bonds.tier_of(r.name, name) == vbonds::BondTier::Friend
                        })
                    }; // bonds 讀鎖釋放
                    if let Some((&friend_name, (_id, fx, fz, fbearing))) = found {
                        if vfvisit::should_seek_friend(
                            town_bound,
                            true,
                            true,
                            idle_free,
                            r.frontier_visit_cooldown,
                            r.say.is_empty(),
                            rand::random::<f32>(),
                        ) {
                            r.frontier_visit =
                                Some((*fx, *fz, fbearing.clone(), friend_name.to_string()));
                            r.frontier_visit_stay = 0.0;
                            r.frontier_visit_timer = vfvisit::TIMEOUT_SECS;
                            r.target_x = *fx;
                            r.target_z = *fz;
                            r.wait_timer = 0.0;
                            let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                            r.say = vfvisit::depart_bubble(friend_name, fbearing, pick);
                            r.say_timer = SAY_SECS;
                            frontier_visit_feed
                                .push((r.name, vfvisit::depart_feed_line(friend_name, fbearing)));
                        }
                    }
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

            // 居民關心你挨餓 v1（自主提案切片，ROADMAP 845）：你在近旁挨餓（`is_starving`）、
            // 這位居民恰好閒著醒著、不在朝聖/遠行、冷卻到期 → 主動上前遞一份麵包、記進她心裡。
            // 「你的互動有後果」第一次反過來——不必你先送禮，居民自己會注意到你過得好不好。
            // 鎖序：純讀居民自身欄位＋鎖前備好的 `player_starving` 快照，背包寫/記憶寫/Feed 走
            // 鎖外 `hunger_care_events`（守 prod 死鎖鐵律）。
            if r.say.is_empty()
                && !r.asleep
                && r.hunger_care_cooldown <= 0.0
                && r.pilgrimage.is_none()
                && r.expedition.is_none()
            {
                if let Some((d2, pname)) = nearest_player_info(r.body.x, r.body.z, &player_pts) {
                    let in_reach = d2 < vcare::CARE_REACH * vcare::CARE_REACH;
                    let starving = !pname.is_empty()
                        && *player_starving.get(pname).unwrap_or(&false);
                    if in_reach
                        && starving
                        && vcare::should_notice_hunger(
                            true,
                            0.0,
                            rand::random::<f32>(),
                            vcare::CARE_CHANCE,
                        )
                    {
                        r.hunger_care_cooldown = vcare::CARE_COOLDOWN_SECS;
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                        r.say = vcare::care_bubble_with_player(pname, pick)
                            .chars()
                            .take(vcare::SAY_CHARS)
                            .collect();
                        r.say_timer = SAY_SECS;
                        r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        hunger_care_events.push((
                            r.id.clone(),
                            r.name,
                            pname.to_string(),
                            vfarm::BREAD_ID,
                            vcare::CARE_GIFT_QTY,
                        ));
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

            // 雨後彩虹 v1（ROADMAP 780）：彩虹剛掛上天邊那一刻，say 為空、醒著的居民抬頭望見而歡呼
            // 一句（零 LLM、確定性選句），並讓心情亮一格（`mood_boost` 是驅動行為的真狀態，非純美術）。
            // 與雨天反應同屬罕見一次性事件，值得蓋過閒聊冷卻。
            if rainbow_just_appeared && !r.asleep && r.say.is_empty() {
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                r.say = vweather::rainbow_line(pick).to_string();
                r.say_timer = SAY_SECS;
                r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
            }

            // 季節輪替 v1（ROADMAP 798）：換季那一刻，say 為空、醒著的居民抬頭感到季節更迭，冒一句
            // 應景台詞（零 LLM、確定性選句），心情也跟著微亮一格（`mood_boost` 是驅動行為的真狀態）。
            // 與雨天／彩虹反應同屬罕見的一次性環境事件，值得蓋過閒聊冷卻。
            if season_just_turned && !r.asleep && r.say.is_empty() {
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                r.say = vseason::season_turn_line(current_season, pick).to_string();
                r.say_timer = SAY_SECS;
                r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
            }

            // 繁星夜空·星夜共賞 v1（ROADMAP 783）：夜裡（Night/Evening）靠近你的居民偶爾抬頭望星；
            // 若牠記得你曾說過愛看星空（`FactCategory::Preference` 記憶命中星／月／夜空關鍵詞），便特地
            // 喚你到身邊一起看——記憶驅動行為的浪漫一拍（PLAN_ETHERVOX 北極星）。say 為空、醒著才觸發，
            // 不搶正事；長冷卻＋極低機率＝天然節流。鎖序：memory 讀鎖只在確定要觸發後才短取即釋（不巢狀），
            // 記憶寫＋Feed 走鎖外 `stargaze_events`（不持居民鎖做 IO，守死鎖鐵律）。
            if is_night && r.say.is_empty() && !r.asleep {
                if let Some((d2, pname)) = nearest_player_info(r.body.x, r.body.z, &player_pts) {
                    let in_range = d2 < vstar::STARGAZE_RANGE * vstar::STARGAZE_RANGE;
                    if vstar::should_stargaze(
                        is_night,
                        in_range,
                        r.stargaze_cooldown <= 0.0,
                        false, // say 已在外層確認為空
                        false, // asleep 已在外層確認為醒
                        rand::random::<f32>(),
                    ) {
                        // 冷卻先鎖上（不論一般望星或邀約，都算一次觸發，避免下一 tick 又擲骰）。
                        r.stargaze_cooldown = vstar::STARGAZE_COOLDOWN_SECS;
                        let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                        // 只有登入玩家（名字非空）才查偏好記憶、才可能升級為「記得你愛看星星」的邀約。
                        let remembers = if pname.is_empty() {
                            false
                        } else {
                            let prefs: Vec<String> = {
                                let mem = hub().memory.read().unwrap();
                                mem.semantic_facts_for(&r.id, pname)
                                    .into_iter()
                                    .filter(|f| f.category == vmem::FactCategory::Preference)
                                    .map(|f| f.content)
                                    .collect()
                            }; // memory 讀鎖即釋
                            vstar::remembers_star_love(&prefs)
                        };
                        if remembers {
                            // 記憶驅動行為的魔法一拍：點名邀你同賞、記進交情、上動態牆。
                            r.say = vstar::invite_line(r.name, pname, pick);
                            r.say_timer = SAY_SECS;
                            r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                            stargaze_events.push((r.id.clone(), r.name, pname.to_string()));
                        } else {
                            // 一般望星自語（純夜色氛圍，不寫記憶、不上 Feed）。
                            r.say = vstar::gaze_line(pick).to_string();
                            r.say_timer = SAY_SECS;
                            r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        }
                    }
                }
            }

            // 哼歌 v1（ROADMAP 788）：心情正好（`mood_boost_secs > 0`＝剛因一次互動被點亮）、say 為空、
            // 醒著、手邊沒正事的居民，偶爾忍不住輕輕哼起歌來（頭頂飄音符——前端偵測 say 以 ♪ 起頭）。
            // 記憶驅動行為的一拍：若此刻有位登入玩家正好在身邊（[`vhum::HUM_NEAR_RADIUS`] 內），牠哼的那句
            // 會點到你名、並把「和你在一起忍不住哼起歌來」記進交情、上動態牆；否則只哼無詞的調子（純氛圍）。
            // 長冷卻＋極低機率＝天然節流。鎖序：純讀居民自身欄位（已持居民寫鎖），記憶寫＋Feed 走鎖外
            // `humming_events`，守死鎖鐵律。
            if r.mood_boost_secs > 0.0
                && r.say.is_empty()
                && !r.asleep
                && r.humming_cooldown <= 0.0
                && r.pilgrimage.is_none()
                && r.expedition.is_none()
                && vhum::should_hum(true, true, rand::random::<f32>(), vhum::HUM_CHANCE)
            {
                r.humming_cooldown = vhum::HUM_COOLDOWN_SECS;
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                // 身邊最近的玩家：夠近（半徑內）且是登入玩家（名非空）→ 哼給你聽、記進交情。
                let near_player = nearest_player_info(r.body.x, r.body.z, &player_pts)
                    .filter(|(d2, pname)| {
                        *d2 < vhum::HUM_NEAR_RADIUS * vhum::HUM_NEAR_RADIUS && !pname.is_empty()
                    })
                    .map(|(_, pname)| pname.to_string());
                if let Some(pname) = near_player {
                    r.say = vhum::hum_to_player_line(&pname, pick);
                    r.say_timer = SAY_SECS;
                    humming_events.push((r.id.clone(), r.name, pname, pick));
                } else {
                    // 沒有玩家在身邊（或只有訪客）：獨自哼無詞的調子，不寫記憶、不上 Feed。
                    r.say = vhum::hum_solo_line(pick);
                    r.say_timer = SAY_SECS;
                }
            }

            // 乙太營火 v1：入夜後，閒著、醒著、且恰好路過玩家蓋的某座營火附近時，居民偶爾駐足
            // 圍暖、心情變好、說句暖心話；你也在火邊（WARM_PLAYER_RADIUS 內）時點你名並記進交情。
            // 三閘（靠近火＋冷卻＋機率）＋長冷卻＝天然節流。鎖序：純讀居民自身欄位＋鎖前備好的
            // `campfire_spots` 快照（不巢狀 campfires 讀鎖），記憶寫＋Feed 走鎖外 `campfire_warm_events`。
            if !campfire_spots.is_empty()
                && r.say.is_empty()
                && !r.asleep
                && r.campfire_warm_cooldown <= 0.0
                && r.pilgrimage.is_none()
                && r.expedition.is_none()
                && vcamp::nearest_campfire(&campfire_spots, r.body.x, r.body.z, vcamp::WARM_RADIUS)
                    .is_some()
                && vcamp::should_warm(true, 0.0, rand::random::<f32>(), vcamp::WARM_CHANCE)
            {
                r.campfire_warm_cooldown = vcamp::WARM_COOLDOWN_SECS;
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                // 火邊最近的玩家：夠近（WARM_PLAYER_RADIUS 內）且是登入玩家（名非空）→ 暖語點你名、記交情。
                let near_player = nearest_player_info(r.body.x, r.body.z, &player_pts)
                    .filter(|(d2, pname)| {
                        *d2 < vcamp::WARM_PLAYER_RADIUS * vcamp::WARM_PLAYER_RADIUS
                            && !pname.is_empty()
                    })
                    .map(|(_, pname)| pname.to_string());
                if let Some(pname) = near_player {
                    r.say = vcamp::warm_bubble_with_player(&pname, pick)
                        .chars()
                        .take(50)
                        .collect();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    campfire_warm_events.push((r.id.clone(), r.name, Some(pname), pick));
                } else {
                    // 沒有玩家在火邊（或只有訪客）：獨自圍暖念句通用暖語，上 Feed、不寫玩家交情。
                    r.say = vcamp::warm_bubble(pick).to_string();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    campfire_warm_events.push((r.id.clone(), r.name, None, pick));
                }
            }

            // 木長椅 v1：白天，閒著、醒著、且恰好路過玩家擺的某張長椅附近時，居民偶爾**停下腳步
            // 坐上去歇一會兒**（設 wait_timer 原地小坐，這一拍的關鍵新行為＝居民第一次主動停步休息）、
            // 心情變好、說句輕鬆的歇腳話；你也坐在旁邊（REST_PLAYER_RADIUS 內）時點你名並記進交情。
            // 三閘（靠近椅＋冷卻＋機率）＋長冷卻＝天然節流。與營火（夜間圍暖）刻意對成白天／夜晚一對。
            // 鎖序：純讀居民自身欄位＋鎖前備好的 `bench_spots` 快照（不巢狀 benches 讀鎖），記憶寫＋Feed
            // 走鎖外 `bench_rest_events`（守 prod 死鎖鐵律）。`bench_spots` 夜裡為空 → 夜間整段早退零成本。
            if !bench_spots.is_empty()
                && r.say.is_empty()
                && !r.asleep
                && r.bench_rest_cooldown <= 0.0
                && r.pilgrimage.is_none()
                && r.expedition.is_none()
                && vbench::nearest_bench(&bench_spots, r.body.x, r.body.z, vbench::REST_RADIUS)
                    .is_some()
                && vbench::should_rest(true, 0.0, rand::random::<f32>(), vbench::REST_CHANCE)
            {
                r.bench_rest_cooldown = vbench::REST_COOLDOWN_SECS;
                // 坐下歇腳＝停下移動、原地小坐一會兒（下方移動分支讀到 wait_timer > 0 就讓她站/坐著不走）。
                r.wait_timer = r.wait_timer.max(vbench::REST_SIT_SECS);
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                // 椅邊最近的玩家：夠近（REST_PLAYER_RADIUS 內）且是登入玩家（名非空）→ 歇腳話點你名、記交情。
                let near_player = nearest_player_info(r.body.x, r.body.z, &player_pts)
                    .filter(|(d2, pname)| {
                        *d2 < vbench::REST_PLAYER_RADIUS * vbench::REST_PLAYER_RADIUS
                            && !pname.is_empty()
                    })
                    .map(|(_, pname)| pname.to_string());
                if let Some(pname) = near_player {
                    r.say = vbench::rest_bubble_with_player(&pname, pick)
                        .chars()
                        .take(50)
                        .collect();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    bench_rest_events.push((r.id.clone(), r.name, Some(pname), pick));
                } else {
                    // 沒有玩家在椅邊（或只有訪客）：獨自坐下念句通用歇腳話，上 Feed、不寫玩家交情。
                    r.say = vbench::rest_bubble(pick).to_string();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    bench_rest_events.push((r.id.clone(), r.name, None, pick));
                }
            }

            // 居民臨水垂釣 v1：白天，閒著、醒著、且恰好走到天然水體邊的居民，偶爾**停下腳步、對著水面
            // 靜靜垂一竿**（設 wait_timer 原地小坐）、心情變好、釣起一尾小魚說句療癒的垂釣話；你也在水邊
            // （PLAYER_RADIUS 內）時點你名並記進交情。把垂釣（734）模組早埋下「居民的日記悄悄嚮往著釣魚」
            // 那份至今只寫在日記裡的嚮往第一次真的活出來——記憶/嚮往驅動行為（PLAN_ETHERVOX 核心信念）。
            // 臨水判定走既有 `world`（deltas 讀 guard，本迴圈本就持有、不另取鎖）取樣居民四周鄰格；記憶寫＋
            // Feed 走鎖外 `angler_events`（守 prod 死鎖鐵律）。三閘（臨水＋冷卻＋機率）＋長冷卻＝天然節流。
            if r.say.is_empty()
                && !r.asleep
                && r.angler_cooldown <= 0.0
                && r.pilgrimage.is_none()
                && r.expedition.is_none()
                && matches!(phase, TimePhase::Dawn | TimePhase::Day | TimePhase::Dusk)
            {
                // 取樣居民腳邊四個水平鄰格（腳所在層與其下一層）——任一是水就算臨水（沿用 vfish 水判定）。
                let fx = r.body.x.floor() as i32;
                let fy = r.body.y.floor() as i32;
                let fz = r.body.z.floor() as i32;
                let mut neigh = [0u8; 8];
                let mut n = 0usize;
                for &(dx, dz) in &[(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                    for &dy in &[0i32, -1] {
                        neigh[n] = voxel::effective_block_at(&world, fx + dx, fy + dy, fz + dz) as u8;
                        n += 1;
                    }
                }
                if vangler::any_water(&neigh)
                    && vangler::should_fish(true, 0.0, rand::random::<f32>(), vangler::FISH_CHANCE)
                {
                    r.angler_cooldown = vangler::REST_COOLDOWN_SECS;
                    // 坐下垂釣＝停下移動、原地靜靜釣一會兒（移動分支讀到 wait_timer > 0 就讓她駐足）。
                    r.wait_timer = r.wait_timer.max(vangler::FISH_SIT_SECS);
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    // 水邊最近的登入玩家（PLAYER_RADIUS 內、名非空）→ 垂釣話點你名、記交情。
                    let near_player = nearest_player_info(r.body.x, r.body.z, &player_pts)
                        .filter(|(d2, pname)| {
                            *d2 < vangler::PLAYER_RADIUS * vangler::PLAYER_RADIUS && !pname.is_empty()
                        })
                        .map(|(_, pname)| pname.to_string());
                    if let Some(pname) = near_player {
                        r.say = vangler::angler_bubble_with_player(&pname, pick)
                            .chars()
                            .take(vangler::SAY_CHARS)
                            .collect();
                        r.say_timer = SAY_SECS;
                        r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        angler_events.push((r.id.clone(), r.name, Some(pname), pick));
                    } else {
                        // 沒有玩家在水邊（或只有訪客）：獨自垂一竿念句通用垂釣話，上 Feed、不寫玩家交情。
                        r.say = vangler::angler_bubble(pick).to_string();
                        r.say_timer = SAY_SECS;
                        r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        angler_events.push((r.id.clone(), r.name, None, pick));
                    }
                }
            }

            // 雨天葉傘避雨 v1（voxel_raincover）：下雨時，閒著、醒著、不在朝聖/遠行的居民偶爾**停下腳步、
            // 摘片闊葉舉頭頂躲一會兒雨**（設 wait_timer 原地駐足＝這一拍的關鍵新行為：雨第一次改變居民
            //「做什麼」而非只「說什麼」）、心情因這點遮蔽安穩一格；你也在近旁（SHELTER_PLAYER_RADIUS 內）時
            // 招呼你共避一葉傘、把「和你一起避雨」記進交情。三閘（下雨＋冷卻＋機率）＋長冷卻＝天然節流。
            // 鎖序：純讀居民自身欄位＋鎖前備好的 `raining` 快照，記憶寫＋Feed 走鎖外 `rain_shelter_events`
            //（守 prod 死鎖鐵律）。與雨天反應（701·雨剛下說一句）、彩虹（780·雨停望天）刻意區隔。
            if raining
                && r.say.is_empty()
                && !r.asleep
                && r.rain_shelter_cooldown <= 0.0
                && r.pilgrimage.is_none()
                && r.expedition.is_none()
                && vrain::should_shelter(true, 0.0, rand::random::<f32>(), vrain::SHELTER_CHANCE)
            {
                r.rain_shelter_cooldown = vrain::SHELTER_COOLDOWN_SECS;
                // 躲雨＝停下移動、原地在葉傘下躲一會兒（移動分支讀到 wait_timer > 0 就讓她駐足）。
                r.wait_timer = r.wait_timer.max(vrain::SHELTER_HUDDLE_SECS);
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                // 近旁最近的登入玩家（SHELTER_PLAYER_RADIUS 內、名非空）→ 避雨話點你名、招呼共避、記交情。
                let near_player = nearest_player_info(r.body.x, r.body.z, &player_pts)
                    .filter(|(d2, pname)| {
                        *d2 < vrain::SHELTER_PLAYER_RADIUS * vrain::SHELTER_PLAYER_RADIUS
                            && !pname.is_empty()
                    })
                    .map(|(_, pname)| pname.to_string());
                if let Some(pname) = near_player {
                    r.say = vrain::shelter_bubble_with_player(&pname, pick)
                        .chars()
                        .take(vrain::SAY_CHARS)
                        .collect();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    rain_shelter_events.push((r.id.clone(), r.name, Some(pname), pick));
                } else {
                    // 沒有玩家在近旁（或只有訪客）：獨自躲雨念句通用避雨話，上 Feed、不寫玩家交情。
                    r.say = vrain::shelter_bubble(pick).to_string();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    rain_shelter_events.push((r.id.clone(), r.name, None, pick));
                }
            }

            // 居民顧家駐足 v1（voxel_homegaze）：白天，閒著、醒著、不在朝聖/遠行、且恰好走到自家門前
            //（離家域中心 HOME_RADIUS 內）的居民，偶爾**停下腳步、望著自己一手安頓下來的家、湧起一股
            // 踏實的歸屬感**（設 wait_timer 原地駐足＝這一拍的行為：居民第一次對「一個地點（自家）」生出
            // 情感）、心情亮一格；你也在近旁（GAZE_PLAYER_RADIUS 內）時把「家的踏實有你相伴」記進交情。
            // 白天限定（phase 為 Dawn/Day/Dusk）＝與夜歸就寢刻意區隔（夜睡／晝望，時段相反）。三閘（在家＋
            // 冷卻＋機率）＋長冷卻＝天然節流。鎖序：純讀居民自身欄位，記憶寫＋Feed 走鎖外 `homegaze_events`。
            if r.say.is_empty()
                && !r.asleep
                && r.homegaze_cooldown <= 0.0
                && r.pilgrimage.is_none()
                && r.expedition.is_none()
                && matches!(phase, TimePhase::Dawn | TimePhase::Day | TimePhase::Dusk)
                && vhome::should_gaze(
                    vhome::near_home(r.body.x, r.body.z, r.home_x, r.home_z),
                    0.0,
                    rand::random::<f32>(),
                    vhome::GAZE_CHANCE,
                )
            {
                r.homegaze_cooldown = vhome::GAZE_COOLDOWN_SECS;
                // 顧家＝停下移動、原地在自家門前望一會兒（移動分支讀到 wait_timer > 0 就讓她駐足）。
                r.wait_timer = r.wait_timer.max(vhome::GAZE_PAUSE_SECS);
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                // 近旁最近的登入玩家（GAZE_PLAYER_RADIUS 內、名非空）→ 顧家話點你名、把家的踏實記交情。
                let near_player = nearest_player_info(r.body.x, r.body.z, &player_pts)
                    .filter(|(d2, pname)| {
                        *d2 < vhome::GAZE_PLAYER_RADIUS * vhome::GAZE_PLAYER_RADIUS
                            && !pname.is_empty()
                    })
                    .map(|(_, pname)| pname.to_string());
                if let Some(pname) = near_player {
                    r.say = vhome::gaze_bubble_with_player(&pname, pick)
                        .chars()
                        .take(vhome::SAY_CHARS)
                        .collect();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    homegaze_events.push((r.id.clone(), r.name, Some(pname), pick));
                } else {
                    // 沒有玩家在近旁（或只有訪客）：獨自望家念句通用顧家話，上 Feed、不寫玩家交情。
                    r.say = vhome::gaze_bubble(pick).to_string();
                    r.say_timer = SAY_SECS;
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    homegaze_events.push((r.id.clone(), r.name, None, pick));
                }
            }

            // 邊陲巧遇 v1（voxel_frontier_find，PLAN_ETHERVOX item 7×item 3 交會）：正在邊陲逗留
            // （expedition 已抵達、stay > 0，非睡眠中）的居民，若你恰好走到牠身邊，會認出「你是特地
            // 追這麼遠來的」，比在主城相遇更驚喜的一句招呼——821 讓居民追去邊陲找老朋友，本刀把同一種
            // 「追到荒野盡頭找到你」的驚喜第一次伸向玩家。三閘（在邊陲逗留＋你在近旁＋冷卻＋機率）皆過
            // 才觸發；鎖序：純讀居民自身欄位＋player_pts 快照，記憶寫＋Feed 走鎖外 frontier_find_events。
            if r.say.is_empty()
                && !r.asleep
                && r.frontier_find_cooldown <= 0.0
                && r.expedition_stay > 0.0
            {
                if let Some((_, _, bearing)) = r.expedition.clone() {
                    let near_player = nearest_player_info(r.body.x, r.body.z, &player_pts)
                        .filter(|(d2, pname)| {
                            *d2 < vffind::FIND_PLAYER_RADIUS * vffind::FIND_PLAYER_RADIUS
                                && !pname.is_empty()
                        })
                        .map(|(_, pname)| pname.to_string());
                    if let Some(pname) = near_player {
                        if vffind::should_find(true, true, 0.0, rand::random::<f32>(), vffind::FIND_CHANCE)
                        {
                            r.frontier_find_cooldown = vffind::FIND_COOLDOWN_SECS;
                            let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                            r.say = vffind::found_bubble(&pname, &bearing, pick);
                            r.say_timer = SAY_SECS;
                            r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                            frontier_find_events.push((r.id.clone(), r.name, pname, bearing.clone(), pick));
                        }
                    }
                }
            }

            // 居民也會生病 v1（voxel_illness，自主提案）：閒著、醒著、目前健康、不在朝聖/遠行的居民，
            // 偶爾**病倒**——身子不舒服、停下腳步歇一會兒（設 wait_timer 原地駐足）。零場地限制（生病
            // 不挑地點），靠 [`villness::ONSET_CHANCE`]（極小）+ [`villness::ONSET_COOLDOWN_SECS`]（長）
            // 天然節流，稀少而有份量。病況此後隨 tick 自然消退（見上方冷卻遞減段），也可能被鄰居陪伴／
            // 玩家送湯加速好轉（見下方掃描／禮物特例）。全庫唯一還空白的「被照顧」情感深度第一次出現。
            //
            // v2（自主提案）淋雨易著涼：正下雨、且此刻沒在躲雨/歇著（`wait_timer<=0.0`——比照 815
            // 用 wait_timer 判斷是否正躲/歇；若剛觸發躲雨/歇腳等任何駐足，wait_timer 已 >0，視為
            // 當下有遮蔽，機率不加成，簡化但足夠的近似）→ 發病機率乘 `RAIN_ONSET_MULTIPLIER`，
            // 讓「沒躲好雨」第一次有看得見的後果；觸發後台詞/動態牆依雨因區分。
            let raining_unsheltered = raining && r.wait_timer <= 0.0;
            if r.say.is_empty()
                && !r.asleep
                && r.pilgrimage.is_none()
                && r.expedition.is_none()
                && villness::should_fall_ill(
                    villness::is_sick(r.illness_severity),
                    r.illness_cooldown,
                    rand::random::<f32>(),
                    villness::onset_chance_now(raining_unsheltered, villness::ONSET_CHANCE),
                )
            {
                r.illness_severity = villness::ILLNESS_MAX;
                r.illness_cooldown = villness::ONSET_COOLDOWN_SECS;
                r.wait_timer = r.wait_timer.max(villness::ONSET_REST_SECS);
                let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                r.say = if raining_unsheltered {
                    villness::onset_bubble_rain(pick).to_string()
                } else {
                    villness::onset_bubble(pick).to_string()
                };
                r.say_timer = SAY_SECS;
                illness_onset_events.push((r.name, raining_unsheltered));
            }

            // 居民誕辰紀念 v1（voxel_birthday）：經世代傳承誕生的居民（`birth_unix > 0`）每滿一個
            // 乙太年就迎來一次誕辰紀念——回望來到這片天地多久、記得父母便謝過（點名感謝生下自己的
            // 居民），你也在近旁時特地點名和你分享這一刻並記進交情。初始四位居民 `birth_unix == 0`
            // 恆不觸發（誠實的取捨，見模組檔頭）。無機率門檻——年歲跨越是確定性的一次性事件（比照
            // 780 彩虹／798 換季），靠 `birthday_last_year` 記帳防同一週歲重複觸發。say 為空、醒著、
            // 不在朝聖/遠行才觸發（不搶正事）。鎖序：純讀居民自身欄位，記憶寫＋Feed 走鎖外
            // `birthday_events`（守 prod 死鎖鐵律）。
            if r.say.is_empty() && !r.asleep && r.pilgrimage.is_none() && r.expedition.is_none() {
                let age = vbday::age_years(now_unix, r.birth_unix);
                if vbday::is_birthday_moment(age, r.birthday_last_year) {
                    r.birthday_last_year = age;
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    // 近旁最近的登入玩家（BIRTHDAY_PLAYER_RADIUS 內、名非空）→ 生日話點你名、記交情。
                    let near_player = nearest_player_info(r.body.x, r.body.z, &player_pts)
                        .filter(|(d2, pname)| {
                            *d2 < vbday::BIRTHDAY_PLAYER_RADIUS * vbday::BIRTHDAY_PLAYER_RADIUS
                                && !pname.is_empty()
                        })
                        .map(|(_, pname)| pname.to_string());
                    if let Some(pname) = near_player {
                        r.say = vbday::birthday_bubble_with_player(&pname, age, pick)
                            .chars()
                            .take(vbday::SAY_CHARS)
                            .collect();
                        r.say_timer = SAY_SECS;
                        r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        // v1.1 分你一份心意：短鎖讀她的採集背包（比照 728 回禮短取即釋慣例），
                        // 挑她親手採到的東西、壓成象徵性的 1 份；背包空 → 誠實不硬塞（None）。
                        let stock_pick = {
                            let inv = hub().res_inv.read().unwrap();
                            inv.get(&r.id).and_then(vret::pick_from_stock)
                        }; // res_inv 讀鎖釋放
                        let gift = vbday::birthday_gift_from_stock(stock_pick);
                        birthday_events.push((
                            r.id.clone(),
                            r.name,
                            Some(pname),
                            age,
                            r.birth_parent_name.clone(),
                            pick,
                            gift,
                        ));
                    } else if !r.birth_parent_name.is_empty() {
                        // 沒有玩家在近旁、但記得是誰生下自己→念句謝過父母的生日話，上 Feed、不寫玩家交情。
                        r.say = vbday::birthday_bubble_with_parent(&r.birth_parent_name, age, pick)
                            .chars()
                            .take(vbday::SAY_CHARS)
                            .collect();
                        r.say_timer = SAY_SECS;
                        r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        birthday_events.push((
                            r.id.clone(),
                            r.name,
                            None,
                            age,
                            r.birth_parent_name.clone(),
                            pick,
                            None,
                        ));
                    } else {
                        // 沒有玩家、也沒有已知父母：獨自念句通用生日話，上 Feed、不寫玩家交情。
                        r.say = vbday::birthday_bubble(age, pick)
                            .chars()
                            .take(vbday::SAY_CHARS)
                            .collect();
                        r.say_timer = SAY_SECS;
                        r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                        birthday_events.push((
                            r.id.clone(),
                            r.name,
                            None,
                            age,
                            String::new(),
                            pick,
                            None,
                        ));
                    }
                }
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
            } else if r.foraging_food {
                // 飢餓接農田／倉庫 v2·為了吃而去收成：家裡沒存糧的餓著居民，走去把最近一畦熟作物收進背包。
                // 收成即入存糧、退回可再長的田——之後餓了就吃它，整條「餓→收成→存糧→吃」在世界裡真實跑通。
                // 找不到熟作物 → 共用糧倉 v1 備援：改去借村裡有人存了食物的箱子一份；兩者皆無才誠實放棄
                // 本輪覓食（不鬼打牆漫遊），餓意還在、下輪再想辦法。
                if let Some((cx, cy, cz)) = r.larder_target {
                    // 已鎖定一個有存糧的箱子，鏡像作物覓食同一套「逐 tick 走→抵達即收」節奏。
                    let still_has_food = {
                        let contents =
                            hub().chest.read().unwrap().contents(&vchest::pos_key(cx, cy, cz));
                        contents.iter().any(|&(id, cnt)| vhunger::FOOD_IDS.contains(&id) && cnt > 0)
                    }; // chest 讀鎖即釋（比照 res_inv 短取即釋慣例）
                    if !still_has_food {
                        // 箱子存糧被別人先拿走／清空：老實放棄這次借糧，餓意仍在、下輪再想辦法。
                        r.larder_target = None;
                        vr::gravity_step(&world, &mut r.body, dt);
                    } else if vskill::within_gather_reach(r.body.x, r.body.z, cx, cz) {
                        // 走到箱子旁：排程鎖外取糧（扣箱子存量、入背包、冒借糧泡泡），結束本輪覓食。
                        larder_take_events.push((r.id.clone(), r.name, cx, cy, cz));
                        r.foraging_food = false;
                        r.larder_target = None;
                        vr::gravity_step(&world, &mut r.body, dt);
                    } else {
                        // 朝那口箱子走（沿牆滑行、踏階由物理處理）。
                        let (bx, bz) = (r.body.x, r.body.z);
                        vr::step_toward(
                            &world, &mut r.body,
                            cx as f32 + 0.5, cz as f32 + 0.5,
                            dt, vr::RES_SPEED * speed_mult,
                        );
                        if let Some(yaw) = vr::yaw_from_move(r.body.x - bx, r.body.z - bz) {
                            r.yaw = yaw;
                        }
                    }
                } else {
                    let target = match r.forage_target {
                        Some(t) => Some(t),
                        None => vskill::find_nearest_ripe_crop(
                            &world,
                            r.body.x.floor() as i32,
                            r.body.z.floor() as i32,
                            vskill::GATHER_MAX_RADIUS,
                        )
                        .map(|(x, y, z, _)| (x, y, z)),
                    };
                    match target {
                        None => {
                            // 半徑內沒有熟作物可收：共用糧倉 v1 備援，找一口有存糧的箱子借一份。
                            let chest_hit = hub().chest.read().unwrap().nearest_food_chest(
                                r.body.x.floor() as i32,
                                r.body.z.floor() as i32,
                                vskill::GATHER_MAX_RADIUS,
                                &vhunger::FOOD_IDS,
                            ); // chest 讀鎖即釋（比照 res_inv 短取即釋慣例）
                            match chest_hit {
                                Some((cx, cy, cz, _fid)) => {
                                    r.larder_target = Some((cx, cy, cz));
                                    vr::gravity_step(&world, &mut r.body, dt);
                                }
                                None => {
                                    // 熟作物與糧倉皆無：老實收手（清覓食旗），餓著等下一輪（可能有作物
                                    // 長熟、有人存了糧、鄰居分食、或玩家餵食）。不無窮漫遊、不鬼打牆。
                                    r.foraging_food = false;
                                    r.forage_target = None;
                                    vr::gravity_step(&world, &mut r.body, dt);
                                }
                            }
                        }
                        Some((tx, ty, tz)) => {
                            r.forage_target = Some((tx, ty, tz));
                            // 目標作物此刻是否仍是熟的（可能被玩家/tick 先收走）→ 沒了就重找。
                            let still_ripe = vhunger::is_harvestable_food_block(
                                voxel::effective_block_at(&world, tx, ty, tz),
                            );
                            if !still_ripe {
                                r.forage_target = None;
                                vr::gravity_step(&world, &mut r.body, dt);
                            } else if vskill::within_gather_reach(r.body.x, r.body.z, tx, tz) {
                                // 走到作物旁：排程鎖外收成（收成入背包、田退回可再長、冒收成泡泡），結束本輪覓食。
                                let crop = voxel::effective_block_at(&world, tx, ty, tz);
                                forage_harvest_events.push((r.id.clone(), r.name, tx, ty, tz, crop));
                                r.foraging_food = false;
                                r.forage_target = None;
                                vr::gravity_step(&world, &mut r.body, dt);
                            } else {
                                // 朝那畦作物中心走（沿牆滑行、踏階由物理處理）。
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
                        }
                    }
                }
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
            } else if let Some(sm) = r.summon.clone() {
                // 集會鐘 v1·應召：循著鐘聲朝鐘走去；抵達鐘邊就聚攏反應。優先於平常閒晃／小歇，
                // 但讓位給上方跟隨／整地／交付等玩家明確指派或已committed的任務（那些分支在前）。
                // 逾時遞減＋走不到的放棄在上方冷卻段每 tick 處理（守「卡住自救」，不鬼打牆）。
                let dx = r.body.x - sm.x;
                let dz = r.body.z - sm.z;
                if vbell::arrived(dx, dz, vbell::GATHER_RADIUS) {
                    // 抵達鐘邊：清應召、設冷卻（濫用防護：一段時間內不再被同/別的鐘拉動），冒聚攏泡泡、
                    // 心情亮一格；「你敲鐘召我來」的交情記憶＋Feed 走鎖外 `bell_gather_events`。原地站穩。
                    r.summon = None;
                    r.summon_cooldown = vbell::SUMMON_COOLDOWN_SECS;
                    let pick = (r.body.x.to_bits() ^ r.body.z.to_bits()) as usize;
                    if r.say.is_empty() {
                        // 敲鐘者名非空（登入玩家）→ 點名版；訪客／名空 → 通用版。
                        let line = if sm.ringer.is_empty() {
                            vbell::gather_bubble(pick).to_string()
                        } else {
                            vbell::gather_bubble_with_ringer(&sm.ringer, pick)
                        };
                        r.say = line.chars().take(50).collect();
                        r.say_timer = SAY_SECS;
                    }
                    r.mood_boost_secs = r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    bell_gather_events.push((r.id.clone(), r.name, sm.ringer.clone()));
                    vr::gravity_step(&world, &mut r.body, dt);
                } else {
                    // 還在路上：朝鐘走（沿牆滑行、踏階由物理處理）。
                    let (bx, bz) = (r.body.x, r.body.z);
                    vr::step_toward(&world, &mut r.body, sm.x, sm.z, dt, vr::RES_SPEED * speed_mult);
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
                        && r.frontier_visit.is_none()
                        && vr::should_shelter(is_night, raining, house_locations.contains_key(&r.id));
                    // 探訪中：以目的地為閒晃中心（讓居民在鄰居家附近自然走動）；
                    // 遠行逗留中（ROADMAP 756）：以邊陲落點為中心，讓牠在遠方一小片範圍自然走動、不被拉回家；
                    // 聚會中（ROADMAP 711）：以聚會點為中心，讓一群人看起來聚在一塊；
                    // 邊陲探友逗留中（ROADMAP 821）：以朋友的邊陲落點為中心，讓兩人在荒野一小片範圍相聚；
                    // 夜間遮蔽：以自己蓋的小屋為中心；否則：以自己家域中心為基準（正常行為）。
                    // 搬家中的居民（引導式都更）：閒晃中心貼著當前工地——蓋新家段貼新地塊、
                    // 拆除段貼舊家——玩家全程看得到她在搬家現場忙活（優先於探訪/遠行/遮蔽）。
                    let reloc_here: Option<(f32, f32)> = reloc_snap
                        .as_ref()
                        .and_then(|(id, fx, fz, _)| (*id == r.id).then_some((*fx, *fz)));
                    let (center_x, center_z) = if let Some((fx, fz)) = reloc_here {
                        (fx, fz)
                    } else if let Some((vx, vz, _)) = &r.visiting {
                        (*vx, *vz)
                    } else if let Some((ex, ez, _)) = &r.expedition {
                        (*ex, *ez)
                    } else if let Some((gx, gz, _)) = &r.clique_meet {
                        (*gx, *gz)
                    } else if let Some((fx, fz, _, _)) = &r.frontier_visit {
                        (*fx, *fz)
                    } else if sheltering {
                        let (hx, _hy, hz) = house_locations[&r.id];
                        (hx as f32 + 0.5, hz as f32 + 0.5)
                    } else {
                        (r.home_x, r.home_z)
                    };
                    // 探訪中用探訪範圍；遠行逗留用遠行範圍（在邊陲一小片走動）；聚會中用更小的聚會範圍
                    //（不散開）；邊陲探友逗留用探友範圍；夜間遮蔽用更小的遮蔽半徑（緊靠自家）；
                    // 否則用家域半徑（正常行為）。
                    let wander_r = if reloc_here.is_some() {
                        // 搬家工地：緊貼著打轉（看得出她在忙搬家，不是路過）。
                        RELOC_WANDER_RADIUS
                    } else if r.visiting.is_some() {
                        vvisit::VISIT_WANDER_RADIUS
                    } else if r.expedition.is_some() {
                        vexp::EXPEDITION_WANDER_RADIUS
                    } else if r.clique_meet.is_some() {
                        vclique::GATHER_WANDER_RADIUS
                    } else if r.frontier_visit.is_some() {
                        vfvisit::WANDER_RADIUS
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
                // 搬家拆除段的居民不進 agency 候選（搬家中不接其他任務；拆除由搬家
                // tick 自己推進，不受 build_cooldown 影響）。蓋新家段照常進（推進計畫）。
                && !reloc_snap
                    .as_ref()
                    .map_or(false, |(id, _, _, demolishing)| *demolishing && *id == r.id)
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

        // 2c-2) 圍火講往事掃描（圍火講往事 v1）：入夜後，若兩位醒著、沒在說話的居民恰好聚在**同一座**
        // 營火邊、且其中一位講述冷卻到期，偶爾其中一位把心裡的一段往事講給對方聽。每 tick 最多一對、
        // 有長冷卻，天然節流、不干擾物理主迴圈。沿用 2c 的 `snaps` 位置快照（i≠j 循序索引、不雙重借用）；
        // 講述者記憶以短讀鎖取一次即釋（比照招呼段的記憶讀取，不巢狀寫鎖，守死鎖鐵律）。非夜晚時
        // `campfire_spots` 為空 → 整段早退零成本。
        if !campfire_spots.is_empty() {
            let mut tale_pair: Option<(usize, usize)> = None; // (講述者 i, 聆聽者 j)
            'tscan: for i in 0..snaps.len() {
                // 講述者：此刻沒在說話（讀 live，避免 2c 剛讓他開口）、沒睡著、講述冷卻到期、不在朝聖/遠行途中。
                if !residents[i].say.is_empty() || snaps[i].7 { continue; }
                if residents[i].campfire_tale_cooldown > 0.0 { continue; }
                if residents[i].pilgrimage.is_some() || residents[i].expedition.is_some() { continue; }
                let ci = match vcamp::nearest_campfire(&campfire_spots, snaps[i].3, snaps[i].4, vcamp::WARM_RADIUS) {
                    Some(c) => c,
                    None => continue, // 講述者不在任何火邊
                };
                for j in 0..snaps.len() {
                    if i == j { continue; }
                    // 聆聽者：此刻沒在說話、沒睡著、且和講述者在**同一座**火邊。
                    if !residents[j].say.is_empty() || snaps[j].7 { continue; }
                    if vcamp::nearest_campfire(&campfire_spots, snaps[j].3, snaps[j].4, vcamp::WARM_RADIUS) != Some(ci) {
                        continue;
                    }
                    if vtale::should_tell(0.0, rand::random::<f32>(), vtale::TALE_CHANCE) {
                        tale_pair = Some((i, j));
                        break 'tscan;
                    }
                }
            }
            if let Some((i, j)) = tale_pair {
                let teller_id = snaps[i].1.clone();
                let teller_name = snaps[i].2;
                // 讀講述者記憶、挑一則可講的往事（短讀鎖即釋，不巢狀）。挑不到（新居民還沒累積往事）就這 tick 不講。
                let tale_summary: Option<String> = {
                    let mem = hub().memory.read().unwrap();
                    let mems = mem.all_memories_for(&teller_id);
                    vtale::pick_tale(&mems).map(|e| e.summary.clone())
                }; // memory 讀鎖釋放
                if let Some(summary) = tale_summary {
                    let listener_id = snaps[j].1.clone();
                    let listener_name = snaps[j].2;
                    let pick = (residents[i].body.x.to_bits() ^ residents[i].body.z.to_bits()) as usize;
                    residents[i].say = vtale::tale_bubble(&summary, pick);
                    residents[i].say_timer = SAY_SECS;
                    residents[i].campfire_tale_cooldown = vtale::TALE_COOLDOWN_SECS;
                    residents[i].mood_boost_secs = residents[i].mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                    // 聆聽者幾秒後應和（pending_tale_reply 存講述者名 + 倒數，下一 tick 由回應段接手）。
                    residents[j].pending_tale_reply = Some((teller_name.to_string(), vtale::TALE_REPLY_DELAY_SECS));
                    campfire_tale_events.push((teller_id, teller_name, listener_id, listener_name, summary));
                }
            }
        }

        // 2c-2b) 長椅並坐閒聊/拌嘴/和好掃描（長椅並坐閒聊 v1 + 長椅拌嘴/和好 v1）：白天，若兩位醒著、
        // 沒在說話、交情已到相識以上的居民恰好都走到**同一張**長椅邊、且發起者並坐冷卻到期，這次相遇
        // 三選一（互斥）：①若這對正彆扭中→保證和好（解除彆扭、交情回暖）；②若已是老朋友且未彆扭→
        // 低機率拌嘴（標記彆扭中、交情暫冷一格）；③否則走原本的並坐閒聊機率。每 tick 最多一對、長冷卻
        // ＋機率門檻＝天然節流。沿用 2c-2 圍火講故事的 `snaps` 位置快照（i≠j 循序索引、不雙重借用）與
        // `bench_spots`（白天才非空 → 夜裡整段早退零成本）；情誼層級/彆扭旗標以短讀鎖取一次即釋（鎖序
        // residents 寫→bonds 讀，不巢狀、不反向，比照 800 分食掃描，守死鎖鐵律）。
        if !bench_spots.is_empty() {
            let chat_pair: Option<(usize, usize, vbtiff::BenchOutcome)> = {
                // (發起者 i, 被招呼者 j, 這次相遇的結果)。bonds 讀鎖只在挑對期間持有、找到即釋。
                let bonds = hub().bonds.read().unwrap();
                let mut found: Option<(usize, usize, vbtiff::BenchOutcome)> = None;
                'cscan: for i in 0..snaps.len() {
                    // 發起者：此刻沒在說話（讀 live，避免 2c/2c-2 剛讓她開口）、沒睡著、並坐冷卻到期、
                    // 不在朝聖/遠行途中（不搶正事）。
                    if !residents[i].say.is_empty() || snaps[i].7 {
                        continue;
                    }
                    if residents[i].bench_chat_cooldown > 0.0 {
                        continue;
                    }
                    if residents[i].pilgrimage.is_some() || residents[i].expedition.is_some() {
                        continue;
                    }
                    let ci = match vbench::nearest_bench(
                        &bench_spots, snaps[i].3, snaps[i].4, vbenchchat::CHAT_RADIUS,
                    ) {
                        Some(c) => c,
                        None => continue, // 發起者不在任何長椅邊
                    };
                    for j in 0..snaps.len() {
                        if i == j {
                            continue;
                        }
                        // 被招呼者：此刻沒在說話、沒睡著、且和發起者在**同一張**長椅邊。
                        if !residents[j].say.is_empty() || snaps[j].7 {
                            continue;
                        }
                        if vbench::nearest_bench(
                            &bench_spots, snaps[j].3, snaps[j].4, vbenchchat::CHAT_RADIUS,
                        ) != Some(ci)
                        {
                            continue;
                        }
                        // 記憶驅動行為的閘：只有交情到相識以上的兩人才會招呼彼此並肩坐下。
                        let tier = resident_tier_of(&bonds, &snaps[i].1, &snaps[j].1);
                        if !vbenchchat::tier_allows_chat(tier) {
                            continue;
                        }
                        let sulking = vbtiff::tier_allows_tiff(tier)
                            && resident_is_sulking(&bonds, &snaps[i].1, &snaps[j].1);
                        let outcome = if sulking {
                            // 彆扭中的老朋友再碰上同一張長椅→保證和好（比照 2D 559 慣例，不用再擲骰）。
                            Some(vbtiff::BenchOutcome::MakeUp)
                        } else if vbtiff::tier_allows_tiff(tier)
                            && vbtiff::should_tiff(0.0, rand::random::<f32>())
                        {
                            Some(vbtiff::BenchOutcome::Tiff)
                        } else if vbenchchat::should_chat(0.0, rand::random::<f32>(), vbenchchat::CHAT_CHANCE) {
                            Some(vbtiff::BenchOutcome::Chat)
                        } else {
                            None
                        };
                        if let Some(outcome) = outcome {
                            found = Some((i, j, outcome));
                            break 'cscan;
                        }
                    }
                }
                found
            }; // bonds 讀鎖釋放
            if let Some((i, j, outcome)) = chat_pair {
                let opener_id = snaps[i].1.clone();
                let opener_name = snaps[i].2;
                let other_id = snaps[j].1.clone();
                let other_name = snaps[j].2;
                let pick = (residents[i].body.x.to_bits() ^ residents[i].body.z.to_bits()) as usize;
                let line = match outcome {
                    vbtiff::BenchOutcome::Chat => vbenchchat::opener_line(other_name, pick),
                    vbtiff::BenchOutcome::Tiff => vbtiff::tiff_opener_line(other_name, pick),
                    vbtiff::BenchOutcome::MakeUp => vbtiff::makeup_opener_line(other_name, pick),
                };
                residents[i].say = line.chars().take(vbenchchat::CHAT_SAY_CHARS).collect();
                residents[i].say_timer = SAY_SECS;
                residents[i].bench_chat_cooldown = vbenchchat::CHAT_COOLDOWN_SECS;
                if outcome != vbtiff::BenchOutcome::Tiff {
                    // 閒聊/和好才並肩坐一會兒、心情亮一格；拌嘴各自悶著，不算暖心事。
                    residents[i].wait_timer = residents[i].wait_timer.max(vbench::REST_SIT_SECS);
                    residents[i].mood_boost_secs =
                        residents[i].mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                }
                // 被招呼者幾秒後應和（pending_bench_reply 存發起者名 + 倒數 + 結果，下一 tick 由回應段接手）。
                residents[j].pending_bench_reply =
                    Some((opener_name.to_string(), vbenchchat::CHAT_REPLY_DELAY_SECS, outcome));
                match outcome {
                    vbtiff::BenchOutcome::Chat => {
                        bench_chat_events.push((opener_id, opener_name, other_id, other_name))
                    }
                    vbtiff::BenchOutcome::Tiff => {
                        bench_tiff_events.push((opener_id, opener_name, other_id, other_name))
                    }
                    vbtiff::BenchOutcome::MakeUp => {
                        bench_makeup_events.push((opener_id, opener_name, other_id, other_name))
                    }
                }
            }
        }

        // 2c-3) 飢餓時的守望相助掃描（飢餓時的守望相助 v1，ROADMAP 800）：一位餓著找吃的（`seeking_food`）
        // 居民路過一位此刻閒著、自己不餓、分食冷卻到期、且與她交情已到相識以上的鄰居時，鄰居偶爾喚住她、
        // 分一口飯——餓意當場解除、被分食者稍後道謝、雙方各記一筆暖記憶、情誼再加溫一格。每 tick 最多一對、
        // 有長冷卻 + 低機率，天然節流、不干擾物理主迴圈。沿用 2c/2c-2 的 `snaps` 位置快照（i≠j 循序索引、
        // 不雙重借用）；情誼層級以短讀鎖取一次即釋（比照 2e 打氣掃描的 bonds 讀取，鎖序 residents 寫→
        // bonds 讀，不巢狀、不反向，守死鎖鐵律）。
        // 末欄 is_repay（知恩圖報 v1，801）：這對是不是「回報當年那口飯」（分食者 si 記著欠 hi 一口飯）。
        let share_pair: Option<(usize, usize, bool)> = {
            // (分食者 si, 被分食者 hi, 是否回報)。bonds/debts 讀鎖只在挑對期間持有、找到即釋。
            let bonds = hub().bonds.read().unwrap();
            let debts = hub().meal_debts.read().unwrap();
            let mut found: Option<(usize, usize, bool)> = None;
            'sscan: for hi in 0..snaps.len() {
                // 被分食者：正餓著找吃的、沒睡著。
                if !residents[hi].seeking_food || snaps[hi].7 {
                    continue;
                }
                for si in 0..snaps.len() {
                    if si == hi {
                        continue;
                    }
                    // 分食者：此刻沒在說話（讀 live，避免本 tick 剛讓她開口）、沒睡著、分食冷卻到期、
                    // 自己不餓也沒在覓食、不在朝聖/遠行途中（不搶正事）。
                    if !residents[si].say.is_empty() || snaps[si].7 {
                        continue;
                    }
                    if residents[si].share_meal_cooldown > 0.0 {
                        continue;
                    }
                    if residents[si].seeking_food || vhunger::is_hungry(residents[si].hunger) {
                        continue;
                    }
                    if residents[si].pilgrimage.is_some() || residents[si].expedition.is_some() {
                        continue;
                    }
                    // 就在旁邊才分（近距）。
                    if !vrel::pair_within_range(
                        snaps[si].3, snaps[si].4, snaps[hi].3, snaps[hi].4, vsharemeal::SHARE_RADIUS,
                    ) {
                        continue;
                    }
                    // 知恩圖報 v1（801）：分食者 si 是否記著曾被餓著的 hi 分過一口飯（欠 hi 一頓）。
                    // 若欠著 → 這頓是「回報」：**打破 800 的相識門檻**（連陌生人也還）、且用更積極的
                    // 回報機率。記憶對行為產生真實例外——你會看到交情還淺的兩人之間也主動分食。
                    let is_repay = debts.owes(&snaps[si].1, &snaps[hi].1);
                    // 一般守望相助（非回報）：仍守 800 鐵律——只有交情到相識以上的鄰居才會分一口飯。
                    if !is_repay
                        && !vsharemeal::tier_allows_share(resident_tier_of(
                            &bonds,
                            &snaps[si].1,
                            &snaps[hi].1,
                        ))
                    {
                        continue;
                    }
                    // 回報用更高機率（記著的恩一有機會就想還）；一般分食用 800 機率。冷卻皆共用、天然節流。
                    let passed = if is_repay {
                        vgrat::should_repay(
                            residents[si].share_meal_cooldown,
                            rand::random::<f32>(),
                            vgrat::REPAY_CHANCE,
                        )
                    } else {
                        vsharemeal::should_share(
                            residents[si].share_meal_cooldown,
                            rand::random::<f32>(),
                            vsharemeal::SHARE_CHANCE,
                        )
                    };
                    if passed {
                        found = Some((si, hi, is_repay));
                        break 'sscan;
                    }
                }
            }
            found
        }; // bonds / debts 讀鎖釋放
        if let Some((si, hi, is_repay)) = share_pair {
            let sharer_id = snaps[si].1.clone();
            let sharer_name = snaps[si].2;
            let hungry_id = snaps[hi].1.clone();
            let hungry_name = snaps[hi].2;
            // 分食者喚住鄰居、遞上一口飯（冒暖泡泡、上分食冷卻、心情亮一格）。回報時用專屬語氣
            //（「上回你分我一口，這次換我」）；一般守望相助用 800 的暖句。
            let pick = (residents[si].body.x.to_bits() ^ residents[si].body.z.to_bits()) as usize;
            let sharer_say = if is_repay {
                vgrat::repay_sharer_line(pick)
            } else {
                vsharemeal::sharer_line(pick)
            };
            residents[si].say = sharer_say.chars().take(40).collect();
            residents[si].say_timer = SAY_SECS;
            residents[si].share_meal_cooldown = vsharemeal::SHARE_COOLDOWN_SECS;
            residents[si].mood_boost_secs =
                residents[si].mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
            // 被分食者：餓意當場解除、停下覓食（別再往家走）、上靜默冷卻、稍後冒一句專屬道謝、心情亮一格。
            residents[hi].hunger = 0.0;
            residents[hi].seeking_food = false;
            residents[hi].hunger_say_cd = vhunger::HUNGER_SAY_COOLDOWN;
            residents[hi].pending_meal_thanks =
                Some((sharer_name.to_string(), vsharemeal::THANKS_DELAY_SECS, is_repay));
            residents[hi].mood_boost_secs =
                residents[hi].mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
            share_meal_events.push((sharer_id, sharer_name, hungry_id, hungry_name, is_repay));
        }

        // 2c-4) 居民也會生病·鄰居陪伴掃描（自主提案）：一位正生病的居民恰好被一位此刻閒著、自己健康、
        // 陪伴冷卻到期、且與她交情已到相識以上的鄰居路過時，鄰居偶爾停下腳步陪她一會兒——病況大幅緩解、
        // 被陪伴者稍後道謝、雙方各記一筆暖記憶、情誼再加溫一格。每 tick 最多一對、有長冷卻 + 低機率，
        // 天然節流、不干擾物理主迴圈。沿用 2c/2c-3 的 `snaps` 位置快照（i≠j 循序索引、不雙重借用）；
        // 情誼層級以短讀鎖取一次即釋（比照 2c-3 飢餓時的守望相助的 bonds 讀取，鎖序 residents 寫→
        // bonds 讀，不巢狀、不反向，守死鎖鐵律）。
        let care_pair: Option<(usize, usize)> = {
            // (陪伴者 si, 被陪伴者 hi)。bonds 讀鎖只在挑對期間持有、找到即釋。
            let bonds = hub().bonds.read().unwrap();
            let mut found: Option<(usize, usize)> = None;
            'cscan: for hi in 0..snaps.len() {
                // 被陪伴者：正生病、沒睡著（讀 live 病況，隨 tick 消退）。
                if !villness::is_sick(residents[hi].illness_severity) || snaps[hi].7 {
                    continue;
                }
                for si in 0..snaps.len() {
                    if si == hi {
                        continue;
                    }
                    // 陪伴者：此刻沒在說話（讀 live）、沒睡著、陪伴冷卻到期、自己沒生病、
                    // 不在朝聖/遠行途中（不搶正事）。
                    if !residents[si].say.is_empty() || snaps[si].7 {
                        continue;
                    }
                    if residents[si].care_cooldown > 0.0 {
                        continue;
                    }
                    if villness::is_sick(residents[si].illness_severity) {
                        continue;
                    }
                    if residents[si].pilgrimage.is_some() || residents[si].expedition.is_some() {
                        continue;
                    }
                    // 就在旁邊才陪（近距）。
                    if !vrel::pair_within_range(
                        snaps[si].3, snaps[si].4, snaps[hi].3, snaps[hi].4, villness::CARE_RADIUS,
                    ) {
                        continue;
                    }
                    // 只有交情到相識以上的鄰居才會停下來陪伴（記憶驅動行為）。
                    if !villness::tier_allows_care(resident_tier_of(&bonds, &snaps[si].1, &snaps[hi].1)) {
                        continue;
                    }
                    if villness::should_care(
                        residents[si].care_cooldown,
                        rand::random::<f32>(),
                        villness::CARE_CHANCE,
                    ) {
                        found = Some((si, hi));
                        break 'cscan;
                    }
                }
            }
            found
        }; // bonds 讀鎖釋放
        if let Some((si, hi)) = care_pair {
            let carer_id = snaps[si].1.clone();
            let carer_name = snaps[si].2;
            let patient_id = snaps[hi].1.clone();
            let patient_name = snaps[hi].2;
            // 陪伴者停下來陪一會兒（冒暖泡泡、上陪伴冷卻、心情亮一格）。
            let pick = (residents[si].body.x.to_bits() ^ residents[si].body.z.to_bits()) as usize;
            residents[si].say = villness::carer_line(pick).chars().take(40).collect();
            residents[si].say_timer = SAY_SECS;
            residents[si].care_cooldown = villness::CARE_COOLDOWN_SECS;
            residents[si].mood_boost_secs =
                residents[si].mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
            // 被陪伴者：病況大幅緩解（不一定當場全好，留一點餘韻）、稍後冒一句專屬道謝、心情亮一格。
            residents[hi].illness_severity =
                villness::apply_care(residents[hi].illness_severity, villness::CARE_BOOST);
            residents[hi].pending_care_thanks =
                Some((carer_name.to_string(), vsharemeal::THANKS_DELAY_SECS));
            residents[hi].mood_boost_secs =
                residents[hi].mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
            illness_care_events.push((carer_id, carer_name, patient_id, patient_name));
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

    // 4a-2) 圍火講往事落地（圍火講往事 v1）：聆聽者把「在營火邊聽某人講起往事」記進**社交記憶**
    // （走既有 `SocialStore`／`append_social`，零新持久化格式），並廣播一則城鎮動態。鎖序：social 寫
    // （即釋）；不巢狀；append-only 不破壞既有資料。往事原文已由 `listen_social_summary` 截斷去換行。
    for (teller_id, teller_name, listener_id, listener_name, tale_summary) in &campfire_tale_events {
        let summary = vtale::listen_social_summary(teller_name, tale_summary);
        let entry = {
            let mut soc = hub().social.write().unwrap();
            soc.record_overheard(listener_id, teller_id, &summary)
        }; // social 寫鎖在此釋放
        vrel::append_social(&entry);
        vfeed::append_feed("圍火講古", teller_name, &vtale::tale_feed_line(teller_name, listener_name));
    }

    // 4a-2a2) 長椅並坐閒聊落地（長椅並坐閒聊 v1）：發起者 + 被招呼者各寫一筆 episodic 記憶（掛在對方
    // 名下，累積情誼、也能昇華進日記），交情因這場並肩閒聊再加溫一格（升級才 save + 播里程碑，比照 800/
    // 782 witness），並廣播一則城鎮動態。鎖序：memory 寫（即釋）×2 → bonds 寫（即釋）〔升級時〕→ bonds
    // 讀 save；不巢狀、append-only 不破壞既有資料，守死鎖鐵律。
    for (opener_id, opener_name, other_id, other_name) in &bench_chat_events {
        // 發起者的記憶（掛在被招呼者名下）。
        let eo = {
            hub().memory.write().unwrap().add_memory(
                opener_id,
                other_name,
                &vbenchchat::chat_memory_line(other_name),
            )
        }; // memory 寫鎖釋放
        vmem::append_memory(&eo);
        // 被招呼者的記憶（掛在發起者名下）。
        let et = {
            hub().memory.write().unwrap().add_memory(
                other_id,
                opener_name,
                &vbenchchat::chat_memory_line(opener_name),
            )
        }; // memory 寫鎖釋放
        vmem::append_memory(&et);
        // 交情因這場並肩閒聊加溫一格（bonds 以顯示名記帳）。
        let (tier, tier_changed) = {
            let mut bonds = hub().bonds.write().unwrap();
            bonds.record_visit(opener_name, other_name)
        }; // bonds 寫鎖釋放
        if tier_changed {
            {
                let bonds = hub().bonds.read().unwrap();
                vbonds::save_bonds(&bonds);
            } // bonds 讀鎖釋放
            let milestone = vbonds::tier_up_line(tier, opener_name, other_name);
            vfeed::append_feed("居民情誼", opener_name, &milestone);
        }
        vfeed::append_feed(
            "長椅並坐",
            opener_name,
            &vbenchchat::chat_feed_line(opener_name, other_name),
        );

        // 居民戀愛心動 v1（ROADMAP 846）：僅老朋友（並坐前已是、或這場閒聊剛升到老朋友）才可能
        // 擦出火花，且雙方都還沒有戀人（一生只有一位，見 voxel_romance 模組說明）。鎖序：romance
        // 讀（即釋）判斷資格 → 過關才 romance 寫（即釋）→〔真正新締結才〕romance 讀 save +
        // memory 寫 ×2 + Feed；與 bonds 鎖各自獨立、不巢狀，守死鎖鐵律。
        if tier == vbonds::BondTier::Friend {
            let eligible = {
                let romance = hub().romance.read().unwrap();
                !romance.is_sweetheart(opener_name, other_name)
                    && !romance.has_partner(opener_name)
                    && !romance.has_partner(other_name)
            }; // romance 讀鎖釋放
            if eligible && vromance::spark_roll(rand::random::<f32>()) {
                let newly_sparked = {
                    let mut romance = hub().romance.write().unwrap();
                    romance.record_spark(opener_name, other_name)
                }; // romance 寫鎖釋放
                if newly_sparked {
                    {
                        let romance = hub().romance.read().unwrap();
                        vromance::save_romance(&romance);
                    } // romance 讀鎖釋放
                    let mo = {
                        hub().memory.write().unwrap().add_memory(
                            opener_id,
                            other_name,
                            &vromance::sweetheart_memory_line(other_name),
                        )
                    }; // memory 寫鎖釋放
                    vmem::append_memory(&mo);
                    let mt = {
                        hub().memory.write().unwrap().add_memory(
                            other_id,
                            opener_name,
                            &vromance::sweetheart_memory_line(opener_name),
                        )
                    }; // memory 寫鎖釋放
                    vmem::append_memory(&mt);
                    vfeed::append_feed(
                        "心動時刻",
                        opener_name,
                        &vromance::sweetheart_feed_line(opener_name, other_name),
                    );
                }
            }
        }
    }

    // 4a-2a3) 長椅拌嘴落地（長椅拌嘴/和好 v1）：發起者 + 被抱怨者各寫一筆摩擦記憶（掛在對方名下），
    // 標記這一對「彆扭中」＋交情暫時冷一格（純 flavor，不影響情誼等級——見 `voxel_bonds::begin_tiff`），
    // 並廣播一則城鎮動態。鎖序：memory 寫（即釋）×2 → bonds 寫（即釋）→ bonds 讀 save；不巢狀、
    // append-only 不破壞既有資料，守死鎖鐵律。
    for (opener_id, opener_name, other_id, other_name) in &bench_tiff_events {
        let eo = {
            hub().memory.write().unwrap().add_memory(
                opener_id,
                other_name,
                &vbtiff::tiff_memory_line(other_name),
            )
        }; // memory 寫鎖釋放
        vmem::append_memory(&eo);
        let et = {
            hub().memory.write().unwrap().add_memory(
                other_id,
                opener_name,
                &vbtiff::tiff_memory_line(opener_name),
            )
        }; // memory 寫鎖釋放
        vmem::append_memory(&et);
        {
            let mut bonds = hub().bonds.write().unwrap();
            bonds.begin_tiff(opener_name, other_name);
        } // bonds 寫鎖釋放
        {
            let bonds = hub().bonds.read().unwrap();
            vbonds::save_bonds(&bonds);
        } // bonds 讀鎖釋放
        vfeed::append_feed(
            "長椅拌嘴",
            opener_name,
            &vbtiff::tiff_feed_line(opener_name, other_name),
        );
    }

    // 4a-2a4) 長椅和好落地（長椅拌嘴/和好 v1）：發起者 + 被招呼者各寫一筆修復記憶（掛在對方名下），
    // 解除這一對的「彆扭中」旗標＋交情回暖一格，並廣播一則城鎮動態。鎖序同上（memory 寫×2 → bonds
    // 寫 → bonds 讀 save，不巢狀，守死鎖鐵律）。
    for (opener_id, opener_name, other_id, other_name) in &bench_makeup_events {
        let eo = {
            hub().memory.write().unwrap().add_memory(
                opener_id,
                other_name,
                &vbtiff::makeup_memory_line(other_name),
            )
        }; // memory 寫鎖釋放
        vmem::append_memory(&eo);
        let et = {
            hub().memory.write().unwrap().add_memory(
                other_id,
                opener_name,
                &vbtiff::makeup_memory_line(opener_name),
            )
        }; // memory 寫鎖釋放
        vmem::append_memory(&et);
        {
            let mut bonds = hub().bonds.write().unwrap();
            bonds.make_up(opener_name, other_name);
        } // bonds 寫鎖釋放
        {
            let bonds = hub().bonds.read().unwrap();
            vbonds::save_bonds(&bonds);
        } // bonds 讀鎖釋放
        vfeed::append_feed(
            "長椅和好",
            opener_name,
            &vbtiff::makeup_feed_line(opener_name, other_name),
        );
    }

    // 4a-2b) 飢餓時的守望相助落地（飢餓時的守望相助 v1，ROADMAP 800）：分食者 + 被分食者各寫一筆
    // episodic 記憶（掛在對方名下，累積情誼、也能昇華進日記），情誼因這頓飯再加溫一格（升級才 save +
    // 播里程碑，比照 782 witness），並廣播一則城鎮動態。鎖序：memory 寫（即釋）×2 → bonds 寫（即釋）
    //〔升級時〕→ bonds 讀 save + memory 寫；不巢狀、append-only 不破壞既有資料，守死鎖鐵律。
    // 末欄 is_repay（知恩圖報 v1，801）：true=這頓是「回報當年那口飯」——分食者 sharer 其實是**曾被
    // hungry 分過飯的欠飯者**，此刻反過來還 hungry 一口。記憶/Feed 改用回報語氣，並結清欠飯帳（不再
    // 產生反向欠飯，避免無止盡你來我往）；false=一般守望相助，登記「被分食者欠分食者一口飯」供日後回報。
    for (sharer_id, sharer_name, hungry_id, hungry_name, is_repay) in &share_meal_events {
        // 被分食者的記憶（掛在分食者名下）。回報時：hungry 是當年的恩人，牠記「牠竟記著、把那口飯還我」。
        let mem_h = if *is_repay {
            vgrat::repay_memory_for_benefactor(sharer_name)
        } else {
            vsharemeal::shared_memory_for_hungry(sharer_name)
        };
        let eh = {
            hub().memory.write().unwrap().add_memory(hungry_id, sharer_name, &mem_h)
        }; // memory 寫鎖釋放
        vmem::append_memory(&eh);
        // 分食者的記憶（掛在被分食者名下）。回報時：sharer 是回報者，牠記「今天把當年那口飯還了回去」。
        let mem_s = if *is_repay {
            vgrat::repay_memory_for_repayer(hungry_name)
        } else {
            vsharemeal::shared_memory_for_sharer(hungry_name)
        };
        let es = {
            hub().memory.write().unwrap().add_memory(sharer_id, hungry_name, &mem_s)
        }; // memory 寫鎖釋放
        vmem::append_memory(&es);
        // 情誼因這頓飯（不論分食或回報）加溫一格（bonds 以顯示名記帳）。
        let (tier, tier_changed) = {
            let mut bonds = hub().bonds.write().unwrap();
            bonds.record_visit(sharer_name, hungry_name)
        }; // bonds 寫鎖釋放
        if tier_changed {
            {
                let bonds = hub().bonds.read().unwrap();
                vbonds::save_bonds(&bonds);
            } // bonds 讀鎖釋放
            let milestone = vbonds::tier_up_line(tier, sharer_name, hungry_name);
            vfeed::append_feed("居民情誼", sharer_name, &milestone);
        }
        // 欠飯帳本更新（純記憶體、短寫鎖即釋）：回報 → 結清這一筆；一般分食 → 被分食者記下欠分食者一口。
        {
            let mut debts = hub().meal_debts.write().unwrap();
            if *is_repay {
                debts.repay(sharer_id, hungry_id);
            } else {
                debts.owe(hungry_id, sharer_id);
            }
        } // meal_debts 寫鎖釋放
        // 城鎮動態牆（鎖外 IO）。回報用專屬分類與文案（「當年那口飯有了回聲」）。
        if *is_repay {
            vfeed::append_feed(
                vgrat::FEED_KIND,
                sharer_name,
                &vgrat::repay_feed_line(sharer_name, hungry_name),
            );
        } else {
            vfeed::append_feed(
                vsharemeal::FEED_KIND,
                sharer_name,
                &vsharemeal::share_feed_line(sharer_name, hungry_name),
            );
        }
    }

    // 4a-2c) 居民也會生病·病倒／自然痊癒落地（自主提案）：本 tick 病倒 / 痊癒的居民各廣播一則城鎮
    // 動態（鎖外 IO），讓不在場 / 回來的玩家也讀到「這位居民今天不太舒服」的生活痕跡。
    for (name, caused_by_rain) in &illness_onset_events {
        let line = if *caused_by_rain {
            villness::onset_feed_line_rain(name)
        } else {
            villness::onset_feed_line(name)
        };
        vfeed::append_feed(villness::FEED_KIND, name, &line);
    }
    for name in &illness_recovered_events {
        vfeed::append_feed(villness::FEED_KIND, name, &villness::recovered_feed_line(name));
    }

    // 4a-2d) 居民也會生病·鄰居陪伴落地（自主提案）：陪伴者 + 被陪伴者各寫一筆 episodic 記憶
    // （掛在對方名下，累積情誼、也能昇華進日記），情誼因這份陪伴再加溫一格（升級才 save + 播里程碑，
    // 比照 800/782），並廣播一則城鎮動態。鎖序：memory 寫（即釋）×2 → bonds 寫（即釋）〔升級時〕→
    // bonds 讀 save + memory 寫；不巢狀、append-only 不破壞既有資料，守死鎖鐵律。
    for (carer_id, carer_name, patient_id, patient_name) in &illness_care_events {
        // 被陪伴者的記憶（掛在陪伴者名下）。
        let mem_p = villness::cared_memory_for_patient(carer_name);
        let ep = {
            hub().memory.write().unwrap().add_memory(patient_id, carer_name, &mem_p)
        }; // memory 寫鎖釋放
        vmem::append_memory(&ep);
        // 陪伴者的記憶（掛在被陪伴者名下）。
        let mem_c = villness::cared_memory_for_carer(patient_name);
        let ec = {
            hub().memory.write().unwrap().add_memory(carer_id, patient_name, &mem_c)
        }; // memory 寫鎖釋放
        vmem::append_memory(&ec);
        // 情誼因這份陪伴加溫一格（bonds 以顯示名記帳）。
        let (tier, tier_changed) = {
            let mut bonds = hub().bonds.write().unwrap();
            bonds.record_visit(carer_name, patient_name)
        }; // bonds 寫鎖釋放
        if tier_changed {
            {
                let bonds = hub().bonds.read().unwrap();
                vbonds::save_bonds(&bonds);
            } // bonds 讀鎖釋放
            let milestone = vbonds::tier_up_line(tier, carer_name, patient_name);
            vfeed::append_feed("居民情誼", carer_name, &milestone);
        }
        vfeed::append_feed(
            villness::FEED_KIND,
            carer_name,
            &villness::care_feed_line(carer_name, patient_name),
        );
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

    // 居民關心你挨餓事件落地（自主提案切片，ROADMAP 845）：鎖已全釋放；加背包 → 廣播 → 記憶 → Feed。
    // 鎖序：inventory 寫（即釋）→ memory 寫（即釋）→ tx broadcast，皆短取即釋、不巢狀（守死鎖鐵律）。
    for (rid, rname, pname, bid, qty) in &hunger_care_events {
        let inv_entry = { hub().inventory.write().unwrap().give(pname, *bid, *qty) }; // inventory 寫鎖在此釋放
        vinv::append_inv(&inv_entry);
        let new_count = hub().inventory.read().unwrap().count(pname, *bid);
        let iname = vgift::item_name_zh(*bid);
        let msg = serde_json::json!({
            "t": "player_care",
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
        hub()
            .memory
            .write()
            .unwrap()
            .add_memory(rid, pname, &vcare::care_memory_line(pname)); // 記憶寫鎖即釋
        vfeed::append_feed(vcare::FEED_KIND, rname, &vcare::care_feed_line(rname, pname));
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
        // 果實入你背包（寫鎖即釋）+ 持久化。時令的贈禮花園 v1：正逢當季多回贈一份（812 精神）。
        let (bid, qty) = vgg::produce_gift_timely(*crop, current_season);
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
        // Feed：記錄這個閉環時刻（時令則點明「正逢當季、多回贈一份」）。
        let cname = vgg::crop_name(*crop);
        let feed_line = if vgg::is_timely(*crop, current_season) {
            vgg::harvest_feed_line_timely(rname, pname, cname, current_season.display_name())
        } else {
            vgg::harvest_feed_line(rname, pname, cname)
        };
        vfeed::append_feed("收成回贈", rname, &feed_line);
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

    // 5b-2) 為了吃而去收成執行（飢餓接農田／倉庫 v2，ROADMAP 799）：餓著沒存糧的居民走到一畦熟作物旁後，
    //   真的把它收成進小背包（收成→存糧），田退回「已播種」狀態能再長一輪（療癒的可持續農業）。
    //   鎖序：deltas 讀（驗仍熟，即釋）→ deltas 寫（退回種子態，即釋）→ broadcast → 持久化 → farm 短鎖清計時
    //   → res_inv 寫（食物入袋，即釋）→ Feed / say（鎖外）。全在 residents 寫鎖釋放後、不巢狀，守死鎖鐵律。
    for (rid, rname, gx, gy, gz, crop) in forage_harvest_events {
        // 只在目標「現在仍是那株熟作物」時才收（防別人/別的 tick 先收走 → 空收）。
        let still_ripe = {
            let world = hub().deltas.read().unwrap();
            voxel::effective_block_at(&world, gx, gy, gz) == crop
        }; // deltas 讀鎖釋放
        if !still_ripe {
            continue;
        }
        let Some((food_id, qty, regrow)) = vhunger::harvest_food_of(crop) else {
            continue; // 理論上不會（能排進來就一定是熟作物），保險略過
        };
        // 收成：作物方塊換成收成後的退回態（Mature→Seeded 能再長／莓果叢退回結果前的苗），廣播 + 持久化。
        {
            let mut world = hub().deltas.write().unwrap();
            voxel::set_block(&mut world, gx, gy, gz, regrow);
        } // deltas 寫鎖釋放
        broadcast_block(gx, gy, gz, regrow);
        vbuild::append_world_block(gx, gy, gz, regrow as u8);
        // 農地計時：小麥/胡蘿蔔/馬鈴薯退回已播種態要重新起算「再長一輪」（比照玩家收割後續種）。
        // 莓果叢由 tick_berry 自行管理（退回苗即重啟計時），這裡只處理犁田三作物。
        if matches!(regrow, Block::FarmSoilSeeded | Block::CarrotSeeded | Block::PotatoSeeded) {
            let kind = match regrow {
                Block::CarrotSeeded => vfarm::CropKind::Carrot,
                Block::PotatoSeeded => vfarm::CropKind::Potato,
                _ => vfarm::CropKind::Wheat,
            };
            let farm_e =
                { hub().farm.write().unwrap().plant(gx, gy, gz, vfarm::now_secs(), kind) };
            vfarm::append_farm(&farm_e);
        }
        // 收穫入居民小背包（res_inv 短鎖即釋）——收成→存糧，之後餓了就吃它。
        {
            let mut inv = hub().res_inv.write().unwrap();
            *inv.entry(rid.clone()).or_default().entry(food_id).or_insert(0) += qty;
        } // res_inv 寫鎖釋放
        let food = vhunger::food_name_zh(food_id).unwrap_or("作物");
        // Feed（真實事件、低頻）＋ 收成泡泡（她餓著、剛把吃的收進袋子）。
        vfeed::append_feed("收成", rname, &format!("{rname}餓著肚子，把熟了的{food}收進了袋子"));
        let pick = (gx as usize) ^ (gz as usize);
        say_updates.push((rid.clone(), vhunger::foraged_say_line(food, pick)));
    }

    // 共用糧倉 v1：找不到熟作物、走到一口有存糧的箱子旁——扣箱子 1 份存量、入居民小背包。
    for (rid, rname, cx, cy, cz) in larder_take_events {
        // 只在箱子此刻仍有食物時才取（防別人/別的居民先取走 → 空取）；扣 1 份、生持久化事件。
        let pos = vchest::pos_key(cx, cy, cz);
        let taken = {
            let mut store = hub().chest.write().unwrap();
            let contents = store.contents(&pos);
            let fid = contents
                .iter()
                .map(|&(id, _)| id)
                .find(|id| vhunger::FOOD_IDS.contains(id));
            fid.and_then(|fid| {
                let (got, entry) = store.take(&pos, fid, 1);
                (got > 0).then_some((fid, entry))
            })
        }; // chest 寫鎖釋放
        let Some((food_id, chest_e)) = taken else {
            continue; // 箱子已被別人先掏空：誠實放棄這次借糧
        };
        vchest::append_chest(&chest_e);
        // 借到的糧食入居民小背包（res_inv 短鎖即釋）——下次餓了就吃它，鏡像收成→存糧的既有節奏。
        {
            let mut inv = hub().res_inv.write().unwrap();
            *inv.entry(rid.clone()).or_default().entry(food_id).or_insert(0) += 1;
        } // res_inv 寫鎖釋放
        let food = vhunger::food_name_zh(food_id).unwrap_or("吃的");
        // Feed（真實事件、低頻）＋ 借糧泡泡（她餓著、找不到熟作物，靠村裡的箱子撐過去）。
        vfeed::append_feed("糧倉", rname, &format!("{rname}餓著肚子，翻了村裡的箱子借到了{food}"));
        let pick = (cx as usize) ^ (cz as usize);
        say_updates.push((rid.clone(), vhunger::borrowed_say_line(food, pick)));
    }

    // 居民回饋糧倉 v1（自主提案）：把決定拍時選中的材料，真的從居民背包移進箱子（零和守恆、
    // 不憑空生料，鏡像 748 分享的雙重確認精神）。鎖序：deltas 讀（即釋，確認箱子沒被同 tick
    // 破壞掉）→ res_inv 寫（即釋，再次確認居民此刻仍握有足量）→ chest 寫（即釋）→ 持久化/Feed（鎖外）。
    for (rid, rname, cx, cy, cz, item_id, qty) in chest_contribute_events {
        let still_chest = {
            let world = hub().deltas.read().unwrap();
            voxel::effective_block_at(&world, cx, cy, cz) == Block::Chest
        }; // deltas 讀鎖釋放
        if !still_chest {
            continue; // 箱子恰好在同一 tick 被破壞：誠實放棄這次回饋
        }
        let taken = {
            let mut bags = hub().res_inv.write().unwrap();
            let has = bags.get(&rid).and_then(|b| b.get(&item_id)).copied().unwrap_or(0);
            let actual = has.min(qty);
            if actual > 0 {
                if let Some(b) = bags.get_mut(&rid) {
                    *b.entry(item_id).or_insert(0) -= actual; // 安全：actual <= has
                }
            }
            actual
        }; // res_inv 寫鎖釋放
        if taken == 0 {
            continue; // 材料同 tick 被別的行為（如發明/建造）先耗盡：誠實放棄
        }
        let pos = vchest::pos_key(cx, cy, cz);
        let entry = { hub().chest.write().unwrap().put(&pos, item_id, taken) }; // chest 寫鎖釋放
        vchest::append_chest(&entry);
        let item_name = vgift::item_name_zh(item_id);
        vfeed::append_feed(
            vchestgive::FEED_KIND,
            rname,
            &vchestgive::contribute_feed_line(rname, item_name, taken),
        );
    }

    // 居民見賢思齊 v1（自主提案，ROADMAP 858）：把鎖內判定觸發的「見賢思齊」心願真的種下——
    // desires 寫鎖種下心願（覆寫既有心願，沿用既有 `set_desire` 覆蓋語義：與玩家聊天種願望
    // 同款，最新萌生的心願蓋過舊的）→ 持久化 → Feed，鎖外循序、不巢狀，守死鎖鐵律；
    // 文字全走固定模板，不含玩家原話。
    for (rid, rname, structure_name, kind) in envy_events {
        let entry = {
            let mut des = hub().desires.write().unwrap();
            des.set_desire(&rid, &venvy::envy_desire_text(&structure_name, kind), vdes::ENVY_SPARK)
        }; // desires 寫鎖釋放
        vdes::append_desire(&entry);
        vfeed::append_feed("見賢思齊", rname, &venvy::envy_feed_line(rname, &structure_name, kind));
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

    // 5c-2a') 邊陲探友（居民千里跋涉去邊陲探望遠行的夥伴 v1，ROADMAP 821）：純 Feed 事件
    //（啟程／小聚結束返家／半路撲空放棄），讓沒在現場的玩家也讀得到「留守者追去邊陲找散居的朋友」。
    for (rname, detail) in &frontier_visit_feed {
        vfeed::append_feed("邊陲探友", rname, detail);
    }
    // 邊陲探友·抵達找到人（ROADMAP 821）：雙方各寫一筆 episodic 記憶（掛對方名下、累積情誼），
    // 交情因這趟跋涉加溫一格（升級才 save + 播里程碑，比照 792/800 的鎖外處理）——留守與散居兩條線
    // 第一次交織，訪客的心意與朋友的驚喜都被世界記下。朋友 id 以名字查回（residents 鎖此刻已釋放，
    // 可安全再取）。
    for (visitor_id, visitor_name, friend_name, bearing) in &frontier_visit_arrive_events {
        let ev = hub().memory.write().unwrap().add_memory(
            visitor_id,
            friend_name,
            &vfvisit::visitor_memory_line(friend_name, bearing),
        ); // memory 寫鎖釋放
        vmem::append_memory(&ev);
        let friend_id = {
            let residents = hub().residents.read().unwrap();
            residents.iter().find(|r| r.name == friend_name.as_str()).map(|r| r.id.clone())
        }; // residents 讀鎖釋放
        if let Some(friend_id) = friend_id {
            let eh = hub().memory.write().unwrap().add_memory(
                &friend_id,
                visitor_name,
                &vfvisit::host_memory_line(visitor_name),
            ); // memory 寫鎖釋放
            vmem::append_memory(&eh);
        }
        let (tier, tier_changed) = {
            let mut bonds = hub().bonds.write().unwrap();
            bonds.record_visit(visitor_name, friend_name)
        }; // bonds 寫鎖釋放
        if tier_changed {
            let bonds = hub().bonds.read().unwrap();
            vbonds::save_bonds(&bonds);
        } // bonds 讀鎖釋放
        vfeed::append_feed(
            "邊陲探友",
            visitor_name,
            &vfvisit::arrive_feed_line(visitor_name, friend_name, bearing),
        );
    }

    // 5c-2a'') 星夜共賞（繁星夜空 v1，ROADMAP 783）：居民記得你愛看星星、在星夜喚你同賞時，
    // 把這則共賞記進牠對你的記憶（掛玩家名下、深化交情），並記一筆動態牆——鎖外統一 IO（不持居民鎖）。
    for (rid, rname, pname) in &stargaze_events {
        hub()
            .memory
            .write()
            .unwrap()
            .add_memory(rid, pname, &vstar::stargaze_memory(pname)); // 記憶寫鎖即釋
        vfeed::append_feed(vstar::FEED_KIND, rname, &vstar::stargaze_feed_line(rname, pname));
    }

    // 睹物思人（ROADMAP 784）：居民駐足在你送的紀念物前想起你時，把這份「又想起了你」記進交情、
    // 上動態牆——鎖外統一 IO（不持居民鎖，守死鎖鐵律）。
    for (rid, rname, giver, item, pick) in &keepsake_recall_events {
        hub().memory.write().unwrap().add_memory(
            rid,
            giver,
            &vkrecall::recall_memory_line(giver, item, *pick),
        ); // 記憶寫鎖即釋
        vfeed::append_feed(
            vkrecall::FEED_KIND,
            rname,
            &vkrecall::recall_feed_line(rname, giver, item),
        );
    }

    // 哼歌（ROADMAP 788）：居民心情正好、你正好在身邊、牠哼給你聽時，把這份好心情記進交情、上動態牆
    //——鎖外統一 IO（不持居民鎖，守死鎖鐵律）。
    for (rid, rname, pname, pick) in &humming_events {
        hub()
            .memory
            .write()
            .unwrap()
            .add_memory(rid, pname, &vhum::hum_memory_line(pname, *pick)); // 記憶寫鎖即釋
        vfeed::append_feed(vhum::FEED_KIND, rname, &vhum::hum_feed_line(rname, pname));
    }

    // 乙太營火 v1：夜裡圍暖事件落地——玩家在火邊時把「一起圍爐」記進交情（掛玩家名下），
    // 無論有無玩家都上動態牆。記憶寫鎖短取即釋、Feed 走 IO，皆在 residents 鎖釋放後（守死鎖鐵律）。
    for (rid, rname, pname_opt, _pick) in &campfire_warm_events {
        if let Some(pname) = pname_opt {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, pname, &vcamp::warm_memory_line(pname)); // 記憶寫鎖即釋
        }
        vfeed::append_feed("營火取暖", rname, &vcamp::warm_feed_line(rname));
    }

    // 木長椅 v1：白天坐下歇腳事件落地——玩家在椅邊時把「和你同坐歇腳」記進交情（掛玩家名下），
    // 無論有無玩家都上動態牆。記憶寫鎖短取即釋、Feed 走 IO，皆在 residents 鎖釋放後（守死鎖鐵律）。
    // 比照營火：歇腳是輕鬆的日常小拍，記憶只進記憶庫（in-memory，重啟歸零），不額外持久化。
    for (rid, rname, pname_opt, _pick) in &bench_rest_events {
        if let Some(pname) = pname_opt {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, pname, &vbench::rest_memory_line(pname)); // 記憶寫鎖即釋
        }
        vfeed::append_feed("長椅歇腳", rname, &vbench::rest_feed_line(rname));
    }

    // 居民臨水垂釣 v1：白天臨水釣一竿的事件落地——玩家在水邊時把「和你一起臨水垂釣」記進交情（掛玩家
    // 名下、日後浮進日記把「想釣魚」的嚮往變成「和你一起釣過魚」的回憶），無論有無玩家都上動態牆。
    // 記憶寫鎖短取即釋、Feed 走 IO，皆在 residents 鎖釋放後（守死鎖鐵律）。比照長椅歇腳：垂釣是輕鬆的
    // 日常小拍，記憶只進記憶庫（in-memory，重啟歸零），不額外持久化。
    for (rid, rname, pname_opt, _pick) in &angler_events {
        if let Some(pname) = pname_opt {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, pname, &vangler::angler_memory_line(pname)); // 記憶寫鎖即釋
        }
        vfeed::append_feed(vangler::FEED_KIND, rname, &vangler::angler_feed_line(rname));
    }

    // 雨天葉傘避雨 v1：雨中停步躲一會兒的事件落地——玩家在近旁時把「和你一起擠在葉傘下避雨」記進交情
    //（掛玩家名下、日後浮進日記），無論有無玩家都上動態牆。記憶寫鎖短取即釋、Feed 走 IO，皆在 residents
    // 鎖釋放後（守死鎖鐵律）。比照長椅歇腳／臨水垂釣：避雨是輕鬆日常小拍，記憶只進記憶庫（in-memory、
    // 重啟歸零），不額外持久化。
    for (rid, rname, pname_opt, _pick) in &rain_shelter_events {
        if let Some(pname) = pname_opt {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, pname, &vrain::shelter_memory_line(pname)); // 記憶寫鎖即釋
        }
        vfeed::append_feed(vrain::FEED_KIND, rname, &vrain::shelter_feed_line(rname));
    }

    // 居民顧家駐足 v1：白天在自家門前望家出神的事件落地——玩家在近旁時把「家的踏實有你相伴」記進交情
    //（掛玩家名下、日後浮進日記），無論有無玩家都上動態牆。記憶寫鎖短取即釋、Feed 走 IO，皆在 residents
    // 鎖釋放後（守死鎖鐵律）。比照臨水垂釣／雨天避雨：顧家是輕鬆日常小拍，記憶只進記憶庫（in-memory、
    // 重啟歸零），不額外持久化。
    for (rid, rname, pname_opt, _pick) in &homegaze_events {
        if let Some(pname) = pname_opt {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, pname, &vhome::gaze_memory_line(pname)); // 記憶寫鎖即釋
        }
        vfeed::append_feed(vhome::FEED_KIND, rname, &vhome::gaze_feed_line(rname));
    }

    // 邊陲巧遇 v1：玩家追到邊陲、被正在遠行的居民認出的事件落地——把「你千里迢迢追到邊陲找到我」
    // 記進交情（掛玩家名下、日後浮進日記），並上動態牆讓沒跟去的其他玩家也讀得到。記憶寫鎖短取即釋、
    // Feed 走 IO，皆在 residents 鎖釋放後（守死鎖鐵律）。純記憶體（比照顧家駐足／臨水垂釣），不額外持久化。
    for (rid, rname, pname, bearing, _pick) in &frontier_find_events {
        hub()
            .memory
            .write()
            .unwrap()
            .add_memory(rid, pname, &vffind::found_memory_line(pname, bearing)); // 記憶寫鎖即釋
        vfeed::append_feed(vffind::FEED_KIND, rname, &vffind::found_feed_line(rname, pname, bearing));
    }

    // 居民誕辰紀念 v1：滿一個乙太年的事件落地——玩家在近旁時把「和你一起過了第 N 個生日」記進交情
    //（掛玩家名下、日後浮進日記），無論有無玩家都上動態牆（含父母名，若有）。記憶寫鎖短取即釋、Feed
    // 走 IO，皆在 residents 鎖釋放後（守死鎖鐵律）。比照顧家駐足／臨水垂釣：記憶只進記憶庫
    //（in-memory、重啟歸零），不額外持久化——`birth_unix`/`birthday_last_year` 本身才是這刀的狀態。
    //
    // v1.1（ROADMAP 872）分你一份心意：`gift` 為 `Some` 時（玩家在場且她的採集背包當時有貨），
    // 額外把心意存進玩家背包、記一句更具體的記憶、上一行補充動態牆；`gift` 為 `None`（背包空/玩家
    // 不在場）→ 只走原本 v1 的話語與記憶，不硬塞禮物。廣播刻意重用既有 `return_gift` 訊息格式
    // （前端已有通用「居民把 X 送給你了」toast 處理，零前端改動）；本刀不需要「已送過」節流——
    // 觸發本就綁在誕辰事件上，`birthday_last_year` 已保證同一週歲恆只送一次。
    for (rid, rname, pname_opt, age, parent_name, _pick, gift) in &birthday_events {
        if let Some(pname) = pname_opt {
            let mem_line = if let Some((bid, _)) = gift {
                vbday::birthday_memory_line_gift(pname, *age, vgift::item_name_zh(*bid))
            } else {
                vbday::birthday_memory_line(pname, *age)
            };
            hub().memory.write().unwrap().add_memory(rid, pname, &mem_line); // 記憶寫鎖即釋

            if let Some((bid, qty)) = gift {
                let inv_entry = { hub().inventory.write().unwrap().give(pname, *bid, *qty) }; // inventory 寫鎖即釋
                vinv::append_inv(&inv_entry);
                let new_count = hub().inventory.read().unwrap().count(pname, *bid);
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
                vfeed::append_feed(vbday::FEED_KIND, rname, &vbday::birthday_gift_feed_line(rname, pname, &iname));
            }
        }
        vfeed::append_feed(vbday::FEED_KIND, rname, &vbday::birthday_feed_line(rname, *age, parent_name));
    }

    // 集會鐘 v1：應召走到鐘邊聚攏的居民，把「你敲鐘召我來」這份互動記進交情（掛敲鐘者名下、
    // 累積好感）——鎖外統一 IO（不持居民鎖，守死鎖鐵律）。動態牆已由敲鐘當下的一則彙總 Feed
    // （「X敲響集會鐘，N位居民聚來」）代表，這裡不逐位重複上 Feed（避免洗版）。敲鐘者名空（訪客）
    // 無可歸屬帳號 → 只享當下反應、不寫交情記憶。記憶持久化（比照贈禮/幫忙：玩家主動互動該留得下）。
    for (rid, _rname, ringer) in &bell_gather_events {
        if !ringer.is_empty() {
            let entry = hub()
                .memory
                .write()
                .unwrap()
                .add_memory(rid, ringer, &vbell::gather_memory_line(ringer)); // 記憶寫鎖即釋
            vmem::append_memory(&entry);
        }
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

                // 邊陲營地立牌 v1（ROADMAP 881，PLAN_ETHERVOX item 7）：小棚第一次搭起（床剛
                // 落地，本 if 分支只在此時進入）就順手在營地前立一塊牌子署名，比照 749 主城建物
                // 立牌、860 讓自蓋作品算進地標系統——這處荒野據點從此有名字、被世界記住。走既有
                // 告示牌管線（Sign 方塊 + SignStore + 廣播 + JSONL），零新協議。找不到合適空地
                // （四邊都被擋）就靜默略過，不強蓋、不壓既有方塊。
                let text = vexp::outpost_nameplate_text(rname);
                if !text.is_empty() {
                    if let Some((sx, sy, sz)) = pick_nameplate_slot((ax, ay, az)) {
                        {
                            let mut world = hub().deltas.write().unwrap();
                            voxel::set_block(&mut world, sx, sy, sz, Block::Sign);
                        } // deltas 寫鎖釋放
                        broadcast_block(sx, sy, sz, Block::Sign);
                        vbuild::append_world_block(sx, sy, sz, Block::Sign as u8);
                        let ev = hub()
                            .sign
                            .write()
                            .unwrap()
                            .set(&vsign::pos_key(sx, sy, sz), text.clone(), None);
                        vsign::append_sign(&ev);
                        broadcast_sign(sx, sy, sz, &text);
                        vfeed::append_feed("立牌命名", rname, &vexp::outpost_nameplate_feed(rname, &text));
                        say_updates.push((rid.clone(), vexp::outpost_nameplate_say(&text)));
                        // 邊陲營地也算進世界的地標系統（860 同款慣例）：讓見賢思齊（858）與村莊
                        // 里程碑（856）一視同仁，不獨厚主城的建物。`contains_key` 先判斷再插入＝
                        // 冪等（同一格只登記一次）。
                        let newly_landmarked = {
                            let cell = vstructname::cell_key(ax as f32, az as f32);
                            let mut names = structure_names().lock().unwrap();
                            if names.contains_key(&cell) {
                                false
                            } else {
                                names.insert(cell, (text.clone(), Some(rid.clone())));
                                true
                            }
                        }; // structure_names 鎖釋放
                        if newly_landmarked {
                            let landmark_count = structure_names().lock().unwrap().len();
                            let new_tier = hub()
                                .village_milestones
                                .write()
                                .unwrap()
                                .try_unlock_new_tier(landmark_count);
                            if let Some(tier) = new_tier {
                                vvillms::append_village_milestone(&vvillms::VillageMilestoneEntry {
                                    id: tier.id.to_string(),
                                });
                                // 全體居民一起歡呼；不覆寫正忙著別的事的居民，比照 773/856/860 慣例。
                                {
                                    let mut residents = hub().residents.write().unwrap();
                                    for (i, r) in residents.iter_mut().enumerate() {
                                        if r.say.is_empty() {
                                            r.say = vvillms::celebrate_say_line(landmark_count + i)
                                                .to_string();
                                            r.say_timer = SAY_SECS;
                                            r.mood_boost_secs =
                                                r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                        }
                                    }
                                } // residents 寫鎖釋放
                                broadcast_players();
                                vfeed::append_feed(
                                    "村莊里程碑",
                                    "全村",
                                    &vvillms::celebrate_feed_line(tier.name_zh, landmark_count),
                                );
                            }
                        }
                    }
                }
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

    // 5c-2e-2) 居民登門拜訪你·抵達處理（居民認得你的家 v1，自主提案切片，ROADMAP 830）：居民朝聖
    // 抵達的其實是你親手署名的家牌時，把這趟走過去當成一趟真正的「登門拜訪你」——① 掛在你名下的
    // 記憶（碰面／撲空文案不同，日後回想／日記可引用）；② 一則登門／撲空 Feed，讓你回來就讀得到
    // 「今天有人繞去找過你」。不需要 763 那樣的「回家感應」延遲：Feed 本就是你隨時能讀到的非同步
    // channel。鎖序：memory 寫鎖短取即釋、IO 在鎖外——不巢狀、守死鎖鐵律。
    for (rid, rname, player, met) in &home_visit_events {
        let (mem, feed_kind, feed_line) = if *met {
            (
                vplayerhome::visit_present_memory(player),
                "登門拜訪你",
                vplayerhome::visit_present_feed(rname, player),
            )
        } else {
            (
                vplayerhome::visit_missed_memory(player),
                "撲空拜訪你",
                vplayerhome::visit_missed_feed(rname, player),
            )
        };
        let entry = {
            hub().memory.write().unwrap().add_memory(rid, player, &mem)
        }; // memory 寫鎖釋放
        vmem::append_memory(&entry);
        vfeed::append_feed(feed_kind, rname, &feed_line);
    }

    // 5c-2f) 你送的食物她會細細享用 Feed（ROADMAP 765）：居民在閒暇時真的享用了玩家稍早送的食物
    //（泡泡與心情補助已於鎖內設好）——鎖外補一則城鎮動態，讓不在線上的玩家回來也讀得到「牠好好享用了
    // 你的心意」。餵食第一次有了「被好好享用」的溫暖回響。鎖外純 IO、不巢狀、守死鎖鐵律。
    for (rname, giver, food) in &savor_feeds {
        vfeed::append_feed("享用", rname, &vsavor::savor_feed_line(rname, giver, food));
    }

    // 5c-2f-2) 餓時被餵的深記憶（居民也會肚子餓 v1，ROADMAP 799）：居民在餓著的時候被玩家餵了一口
    //（餓意已於鎖內歸零），鎖外把「你在我正餓的時候餵了我」記進牠對這位玩家的記憶——episodic、累積
    // 好感，讓「善意踩在對的時間點上」成為情誼裡分量更重的一筆；並補一則城鎮動態。記憶寫鎖即釋、
    // append IO 在鎖外（守死鎖鐵律）；摘要為純模板、只嵌玩家顯示名、不含任何原話。
    for (rid, giver, rname) in &hunger_fed {
        let summary = vhunger::fed_memory_line(giver);
        let entry = hub().memory.write().unwrap().add_memory(rid, giver, &summary);
        vmem::append_memory(&entry);
        vfeed::append_feed("餵食", rname, &vhunger::fed_feed_line(rname, giver));
    }

    // 5c-2g) 自我印象 Feed（自我印象 v1，ROADMAP 770）：居民閒下來回望這一路、昇華出「我成了個怎樣
    // 的人」的自我概念（泡泡已於鎖內設好），鎖外補一則城鎮動態，讓不在線的玩家回來也讀得到「牠如何看
    // 自己」——記憶昇華進世界的日記牆。鎖外純 IO、不巢狀、守死鎖鐵律；文字已是純模板、無記憶原文。
    for (rname, detail) in &self_image_feeds {
        vfeed::append_feed("自省", rname, detail);
    }

    // 5c-2g-1b) 居民主動聊心事 v1（自主提案，781）：居民主動對某玩家掏心（把當前渴望當心事說出口，
    // 泡泡已於鎖內設好）後，鎖外把「我對這位旅人掏了心」記進牠對這位玩家的記憶——episodic、累積好感，
    // 讓「她主動跟我聊過心事」成為情誼裡的一筆、日後也能昇華進日記。記憶寫鎖即釋、append IO 在鎖外
    // （守死鎖鐵律）；摘要為純模板、只嵌玩家顯示名、不含渴望原文。
    for (rid, player) in &confide_mems {
        let summary = vconfide::confide_memory_line(player);
        let entry = hub().memory.write().unwrap().add_memory(rid, player, &summary);
        vmem::append_memory(&entry);
    }

    // 5c-2g-1b-2) 居民察覺你翻過她的日記 v1（自主提案切片）：居民打招呼時抓包了「你翻過我的
    // 日記」（泡泡已於鎖內設好、旗標已於鎖內清除）後，鎖外把這一刻記進她對這位玩家的記憶——
    // episodic、純模板、不含日記原文，摘要視角。記憶寫鎖即釋、append IO 在鎖外（守死鎖鐵律）。
    for (rid, player) in &diary_peek_reveals {
        let summary = vdiarypeek::peek_memory_line(player);
        let entry = hub().memory.write().unwrap().add_memory(rid, player, &summary);
        vmem::append_memory(&entry);
    }

    // 5c-2g-1b-1) 居民教你一道獨門配方 v1（自主提案，ROADMAP 849）：居民主動教了某玩家一道獨門
    // 配方（泡泡已於鎖內設好）後，鎖外依序：① player_recipes 寫鎖學會（冪等，`learn` 回 true 才
    // 繼續）→ ② append 持久化 → ③ memory 寫鎖記一筆「我教了她」episodic 記憶（累積好感）→
    // ④ Feed 動態牆 → ⑤ 廣播 `recipe_taught`（前端只有當事玩家會彈提示，比照 `player_care`）。
    // 全程短鎖循序取放、不巢狀、不持鎖 await，守死鎖鐵律。
    for (rid, rname, pname) in &recipe_teach_events {
        let newly = { hub().player_recipes.write().unwrap().learn(pname, vprecipe::TAUGHT_RECIPE_ID) };
        if !newly {
            continue; // 已學過（理論上鎖內 already_known 已擋，這裡再防一層重入/競態）
        }
        vprecipe::append_player_recipe(&vprecipe::PlayerRecipeEntry {
            player: pname.clone(),
            recipe_id: vprecipe::TAUGHT_RECIPE_ID.to_string(),
        });
        let recipe_name = vcraft::find_taught_recipe(vprecipe::TAUGHT_RECIPE_ID)
            .map(|r| r.name_zh)
            .unwrap_or("獨門配方");
        let summary = vprecipe::teach_memory_line(pname, recipe_name);
        let entry = hub().memory.write().unwrap().add_memory(rid, pname, &summary); // memory 寫鎖即釋
        vmem::append_memory(&entry);
        vfeed::append_feed("獨門配方", rname, &vprecipe::teach_feed_line(rname, pname, recipe_name));
        let msg = serde_json::json!({
            "t": "recipe_taught",
            "resident_name": rname,
            "player": pname,
            "recipe_id": vprecipe::TAUGHT_RECIPE_ID,
            "name_zh": recipe_name,
        })
        .to_string();
        let _ = hub().tx.send(std::sync::Arc::new(msg));
    }

    // 5c-2g-1b-2) 把昨晚的夢說給你聽 v1（自主提案，807）：居民把昨晚的夢分享給某玩家（泡泡已於鎖內
    // 設好）後，鎖外把「我把夢說給了對方聽」記進牠對這位玩家的 episodic 記憶（累積好感，讓「她把夢
    // 分享給我」成為情誼裡的一筆、日後也能昇華進日記），並補一則城鎮動態牆讓不在場 / 回訪的玩家也讀到。
    // 記憶寫鎖即釋、append IO 在鎖外（守死鎖鐵律）；摘要 / 動態皆純模板、只嵌玩家與居民顯示名、不含夢原文。
    for (rid, player) in &dreamshare_mems {
        let summary = vdreamshare::dreamshare_memory_line(player);
        let entry = hub().memory.write().unwrap().add_memory(rid, player, &summary);
        vmem::append_memory(&entry);
    }
    for (rname, player) in &dreamshare_feeds {
        vfeed::append_feed("夢", rname, &vdreamshare::dreamshare_feed_line(rname, player));
    }

    // 5c-2g-1c) 拜託你幫個小忙 v1（自主提案）：居民主動向某位玩家開口討材料（泡泡＋open_request
    // 已於鎖內設好）後，鎖外補一則城鎮動態牆，讓不在場 / 回來的玩家也讀到「某居民正想要某材料」，
    // 有機會去採來幫忙。純 IO、不巢狀、守死鎖鐵律；文字為純模板、只嵌居民名＋材料名、無記憶原文。
    for (rname, item_name) in &request_feeds {
        vfeed::append_feed("求助", rname, &vrequest::request_feed_line(rname, item_name));
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
        // ④ 設牌面文字（sign 寫鎖短取即釋）→ 持久化 → 廣播浮字。居民自己刻的名號牌
        // owner 恆 None（不是玩家立的，居民認得你的家 v1 只認玩家親手署名的牌）。
        let ev = hub().sign.write().unwrap().set(&vsign::pos_key(sx, sy, sz), text.clone(), None);
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

    // 5c-3b) 做夢 Feed（作息 × 記憶驅動行為·居民會做夢 v1，ROADMAP 805）：居民熟睡中，一段深藏
    // 心底的珍貴往事浮成夢，記一筆動態——沒在線上的玩家回來也讀得到「牠昨晚夢見了什麼」。744 是躺下
    // 當下有意識回味今天，本刀是睡著之後不由自主浮現的深夜之夢，記憶連睡夢裡都在悄悄活著。
    for (rname, summary) in &dream_feed {
        vfeed::append_feed("夢", rname, &vdream::dream_feed_line(summary));
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

    // 5c-8) 戀人牽掛·抵達 Feed + 補雙方記憶（記憶驅動·戀人牽掛 v1，ROADMAP 852）：846 讓兩位
    // 老朋友締結成戀人，但這份羈絆從沒有改變過任何行為——本刀第一次讓「戀人」真的影響行為：
    // 找到彼此那一刻，雙方各自留一筆重逢記憶（掛在對方名下），世界看板也播報。
    // 鎖序：memory 寫鎖各自短取即釋、不巢狀，append_memory 的 IO 在鎖外進行（守死鎖鐵律）。
    for (seeker_id, seeker_name, partner_id, partner_name) in &lover_arrivals {
        vfeed::append_feed("戀人牽掛", seeker_name, &vlover::arrive_feed_line(seeker_name, partner_name));
        let entry_seeker = {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(seeker_id, partner_name, &vlover::arrive_memory_line(partner_name))
        };
        vmem::append_memory(&entry_seeker);
        let entry_partner = {
            hub()
                .memory
                .write()
                .unwrap()
                .add_memory(partner_id, seeker_name, &vlover::arrive_memory_line(seeker_name))
        };
        vmem::append_memory(&entry_partner);
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
            // 居民為友誼立下信物 v1（自主提案）：兩位居民**第一次**跨越老朋友門檻的這一刻
            // （tier_changed 保證同一對只跨一次＝冪等、天然防重放），作東的居民在自家旁的
            // 空地點起一盞「友誼的燈」作信物——這段情誼第一次在世界裡留下**持久、看得見**的
            // 實體地標（沿用居民立牌 749 的選址＋方塊落地＋持久化路徑）。找不到合適空地就
            // 靜默略過（不強蓋、不壓既有方塊），優雅退化。
            if tier == vbonds::BondTier::Friend {
                // 取作東居民 id 與當前位置（record_visit 的 host 側；讀鎖即釋、不巢狀）。
                let host_pos: Option<(String, (i32, i32, i32))> = {
                    let residents = hub().residents.read().unwrap();
                    residents.iter().find(|r| r.name == host_name).map(|r| {
                        (
                            r.id.clone(),
                            (r.body.x.floor() as i32, r.body.y.floor() as i32, r.body.z.floor() as i32),
                        )
                    })
                }; // residents 讀鎖釋放
                if let Some((host_id, anchor)) = host_pos {
                    // 在作東居民腳邊挑一塊「上方為空、下方為實地」的格子（deltas 讀鎖短取即釋）。
                    if let Some((tx, ty, tz)) = pick_nameplate_slot(anchor) {
                        let token = vftoken::token_block(pick);
                        // ① 落下信物方塊（deltas 寫鎖短取即釋）→ 廣播 → 持久化（重啟後仍亮著）。
                        {
                            let mut world = hub().deltas.write().unwrap();
                            voxel::set_block(&mut world, tx, ty, tz, token);
                        } // deltas 寫鎖釋放
                        broadcast_block(tx, ty, tz, token);
                        vbuild::append_world_block(tx, ty, tz, token as u8);
                        // ② 動態牆 + 作東者記一筆友誼記憶（掛對方名下、社交足跡；memory 寫鎖即釋）。
                        vfeed::append_feed(
                            vftoken::FEED_KIND,
                            &host_name,
                            &vftoken::token_feed_line(&host_name, visitor_name),
                        );
                        {
                            let entry = hub().memory.write().unwrap()
                                .add_memory(&host_id, visitor_name, &vftoken::token_memory_line(visitor_name));
                            vmem::append_memory(&entry);
                        } // memory 寫鎖釋放
                        // ③ 作東者冒立信物泡泡（走既有 say_updates 統一套用，比照見證圓夢的雙泡泡）。
                        say_updates.push((host_id, vftoken::token_say_line(visitor_name, pick)));
                    }
                }
            }
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
                        builds.get_plan_mut(&host_id).and_then(|p| {
                            let bb = p.pop_next();
                            // 合力蓋家 v1（ROADMAP 834）：記下這位協力者，完工時一起感謝
                            // （零額外鎖，沿用這段本就持有的 builds 寫鎖）。
                            if bb.is_some() {
                                p.add_helper(visitor_name);
                            }
                            bb.map(|bb| (bb, p.kind_name.clone()))
                        })
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
            // 教/學帳本冷卻（技能互教·北極星第四刀）：任一方這一遊戲天已教過/學過就先不教
            //（與相遇互教共用同一本帳，手藝以自然節奏擴散、不洗版）。帳本小鎖取值即釋。
            let ledger_ready = host_id.as_ref().map_or(false, |hid| {
                let now = vfarm::now_secs();
                let led = skill_teach_ledger().lock().unwrap();
                let cd = teach_ledger_secs();
                vteach::ledger_ready(now, led.get(hid).copied(), cd)
                    && vteach::ledger_ready(now, led.get(&visitor_id).copied(), cd)
            });
            if let Some(host_id) = host_id.filter(|_| ledger_ready) {
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
                    // 師承鏈（北極星第四刀）：走 learn_from——source 標老師名、taught 標
                    // true，技能簿看得出這手藝是「師承XX」，與親子的「承自XX」並行。
                    let learned = {
                        let mut store = hub().invented.write().unwrap();
                        store.learn_from(&student_id, &skill, &teacher_name)
                    }; // invented 寫鎖釋放
                    if let Some(rec) = learned {
                        vinvent::append_invented_skill(&rec);
                        {
                            let now = vfarm::now_secs();
                            let mut led = skill_teach_ledger().lock().unwrap();
                            led.insert(teacher_id.clone(), now);
                            led.insert(student_id.clone(), now);
                        } // 帳本鎖釋放
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

    // ── 居民搬新家（引導式都更）：排程 / 推進（全村一次一位、錯開；低頻節奏內建，
    //    平時每 tick 只付一次倒數遞減的成本）。全部短鎖即釋、循序不巢狀、不 await。
    tick_home_relocation(dt, &mut say_updates);

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
                            && d.sparked_by != vdes::ENVY_SPARK
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
                // 退避中的目標（#972）本輪完全不碰——不重用、不發明。否則一個老是失敗的
                // 已學會技能會每個 build tick 被重用、卡在同一步無限鬼打牆（線上實見
                // `reuse=true step=0` 每 ~9 秒重試）。退避到期（見 invent_backoff 倒數）後可再試。
                let under_backoff = {
                    let residents = hub().residents.read().unwrap();
                    residents
                        .iter()
                        .find(|r| r.id == rid)
                        .map_or(false, |r| r.invent_backoff.contains_key(&goal.block_id))
                }; // residents 讀鎖釋放
                if !bag_has_goal && !under_backoff {
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
                                        smelt_wait: None,
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
                            // 世界事實快照（第四刀）：她附近是否已有放置好的熔爐——
                            // 給可行性模擬與 prompt（有就不必再做一個，直接 smelt）。
                            let furnace_nearby = {
                                let world = hub().deltas.read().unwrap();
                                vinvent::station_nearby(
                                    &world, rx, ry, rz, vinvent::FURNACE_BLOCK_ID,
                                )
                            }; // deltas 讀鎖釋放
                            // 她自己已經會的技能（第三刀·技能組合技能）：讓計畫的 use_skill
                            // 步驟能查表展開——已經會的事，不用每次重新拆成一串原語。
                            let known_skills = {
                                let inv = hub().invented.read().unwrap();
                                inv.known_steps_for(&rid)
                            }; // invented 讀鎖釋放
                            spawn_invention(
                                rid.clone(),
                                rname,
                                goal,
                                desire_text.clone().unwrap_or_default(),
                                wb_nearby,
                                furnace_nearby,
                                known_skills,
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
                        // 自我啟發（禱告）、好奇心、見賢思齊都不是真人玩家——完工不指名感謝。
                        (d.sparked_by != vdes::SELF_SPARK
                            && d.sparked_by != vdes::CURIOSITY_SPARK
                            && d.sparked_by != vdes::ENVY_SPARK)
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
        let (next_block, kind_name, kind_str, progress_pct, plan_done, plan_anchor, plan_expansion, plan_inspired_by, plan_helpers) = {
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
                let helpers = plan.helpers.clone();
                (bb, kn, ks, pct, done, anchor, exp, inspired, helpers)
            } else {
                (None, String::new(), String::new(), 100, true, (0, 0, 0), false, None, Vec::new())
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
            // 完工 Feed（每個建物只發一次，不洗版）。合力蓋家 v1（ROADMAP 834）：有協力者
            // 就在建物名後標注「（與 X、Y 合力）」，讓 696 的幫忙在完工瞬間也被看見。
            vfeed::append_feed(
                if plan_expansion { "蓋家擴建完工" } else { "蓋家完工" },
                &rname,
                &vbuild::build_credit_detail(&kind_name, &plan_helpers),
            );
            // 完工廣播：WS 廣播給所有在線玩家（看得到「世界在長大」），慶賀泡泡同步排入 say_updates。
            let _ = hub().tx.send(std::sync::Arc::new(vannounce::build_complete_msg_with_helpers(
                &rname, &kind_name, &plan_helpers,
            )));
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
                        say_updates.push((rid.clone(), vannounce::build_complete_say_with_helpers(&rname, kind, &plan_helpers)));
                    }
                }
                // 心願閉環 v1（ROADMAP 859）：mark_fulfilled 此前只在「玩家送禮」路徑（722）被
                // 呼叫過——居民自己蓋出建物讓心願成真時（不論來源：玩家親口/自我啟發 771/好奇心/
                // 見賢思齊 858）從未補標記，導致這份心願在 DesireStore 裡永遠停在「未實現」，
                // 進而卡死 771 自我印象驅動自發追尋（`vacant` 判定要求 fulfilled==true 或 None，
                // 同一格心願一旦分類成某種建物、蓋完後也不會清空，這位居民就再也種不出新渴望）。
                // 只看「剛完工的種類是否正是目前心願所指」，不論擴建與否、不論來源，皆補上這一刀。
                let should_close_wish = {
                    let des = hub().desires.read().unwrap();
                    des.get_desire(&rid)
                        .is_some_and(|d| vbuild::build_fulfills_desire(&d.desire, d.fulfilled, kind))
                }; // desires 讀鎖釋放
                if should_close_wish {
                    let marked = { hub().desires.write().unwrap().mark_fulfilled(&rid) }; // desires 寫鎖釋放
                    if let Some(entry) = marked {
                        vdes::append_desire(&entry);
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
                            // ② 設牌面文字（sign 寫鎖短取即釋）→ 持久化 → 廣播浮字。居民自己
                            // 蓋完家立的署名牌 owner 恆 None（不是玩家立的，只有 740 玩家親手寫
                            // 的牌才會被居民認得你的家 v1 認成「你的家」）。
                            let ev = hub()
                                .sign
                                .write()
                                .unwrap()
                                .set(&vsign::pos_key(sx, sy, sz), text.clone(), None);
                            vsign::append_sign(&ev);
                            broadcast_sign(sx, sy, sz, &text);
                            // ③ 動態牆 + 立牌泡泡（讓玩家一眼看到居民署了名）。
                            vfeed::append_feed("立牌命名", &rname, &vnameplate::nameplate_feed(&rname, &text));
                            say_updates.push((rid.clone(), vnameplate::nameplate_say(&text)));
                            // ④ 居民親手蓋的家也算數 v1（自主提案，ROADMAP 860）：773/854 命名
                            // 地標系統此前只認得「玩家蓋、居民讚賞取名」的作品——居民自己蓋出建物
                            // 讓心願成真（859）、立牌署名（749）之後，這座建物在系統眼中仍是隱形
                            // 的：從沒被算進村莊集體里程碑（856 複用同一份地標數），也從沒讓任何
                            // 一位居民路過心生嚮往（858 見賢思齊只掃 structure_names()）。這裡把
                            // 剛立好牌的這座建物用牌面文字（如「露娜的家」）連同擁有者 id 登記進去，
                            // 讓「被記住的地標」這條鏈第一次對居民自己的作品一視同仁。
                            // `contains_key` 先判斷再插入＝冪等（同一格只登記一次，擴建/鬼打牆
                            // 補漏重蓋不會重複計入地標數）。
                            let newly_landmarked = {
                                let cell = vstructname::cell_key(plan_anchor.0 as f32, plan_anchor.2 as f32);
                                let mut names = structure_names().lock().unwrap();
                                if names.contains_key(&cell) {
                                    false
                                } else {
                                    names.insert(cell, (text.clone(), Some(rid.clone())));
                                    true
                                }
                            }; // structure_names 鎖釋放
                            if newly_landmarked {
                                let landmark_count = structure_names().lock().unwrap().len();
                                let new_tier = hub()
                                    .village_milestones
                                    .write()
                                    .unwrap()
                                    .try_unlock_new_tier(landmark_count);
                                if let Some(tier) = new_tier {
                                    vvillms::append_village_milestone(&vvillms::VillageMilestoneEntry {
                                        id: tier.id.to_string(),
                                    });
                                    // 全體居民一起歡呼；不覆寫正忙著別的事的居民（say 非空＝正在忙），
                                    // 比照 773/856 既有「不覆寫既有泡泡」慣例。
                                    {
                                        let mut residents = hub().residents.write().unwrap();
                                        for (i, r) in residents.iter_mut().enumerate() {
                                            if r.say.is_empty() {
                                                r.say = vvillms::celebrate_say_line(landmark_count + i)
                                                    .to_string();
                                                r.say_timer = SAY_SECS;
                                                r.mood_boost_secs =
                                                    r.mood_boost_secs.max(voxel_mood::MOOD_BOOST_TALK);
                                            }
                                        }
                                    } // residents 寫鎖釋放
                                    broadcast_players();
                                    vfeed::append_feed(
                                        "村莊里程碑",
                                        "全村",
                                        &vvillms::celebrate_feed_line(tier.name_zh, landmark_count),
                                    );
                                }
                            }
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
    // 挖掘紀律：這是居民**自主**備料採集（蓋家前的自發採集）→ 帶離村禁區，選址跳過村內、往村外找。
    let excl = village_dig_exclusion();
    let found = {
        let world = hub().deltas.read().unwrap();
        vskill::find_nearest_resource_excl(&world, rx, rz, vskill::GATHER_MAX_RADIUS, excl)
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
                // 指名採集該資源。優先序：①續挖進行中的階梯井（地下資源）→②地表天然源
                // →③地表無源＋屬地下資源＋未達井上限 → 開新井往下採 →④其餘 → 誠實失敗。

                // ① 已有進行中的階梯礦井、且這一步要的正是地下資源（石／泥）→ 續挖一批：
                //    清出的實心方塊誠實入袋（body-safe，絕不挖她站的格），夠料後下輪 next_action
                //    自然 Advance；這口挖完仍不夠 → 清掉它（③ 依上限決定再開一口或誠實失敗）。
                let active_quarry = {
                    let residents = hub().residents.read().unwrap();
                    residents
                        .iter()
                        .find(|r| r.id == rid)
                        .and_then(|r| r.invent_quarry.clone())
                }; // residents 讀鎖釋放
                if vinvent::resource_is_underground(resource) {
                    if let Some(q) = active_quarry {
                        let (cells, next_idx) = {
                            let world = hub().deltas.read().unwrap();
                            vdt::quarry_step(&world, &q, vdt::QUARRY_CELLS_PER_STEP)
                        }; // deltas 讀鎖釋放
                        // 身體格保護：絕不挖她自己站的柱（永不自挖站位、不自困）。
                        let body = {
                            let res = hub().residents.read().unwrap();
                            res.iter()
                                .find(|r| r.id == rid)
                                .map(|r| (r.body.x, r.body.y, r.body.z))
                        }; // residents 讀鎖釋放
                        for (x, y, z, prev) in cells {
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
                            // 挖穿水脈讓水流進坑道是誠實物理 → 喚醒鄰格重算。
                            enqueue_water_around(x, y, z);
                            vbuild::append_world_block(x, y, z, Block::Air as u8);
                            {
                                let mut inv = hub().res_inv.write().unwrap();
                                *inv.entry(rid.to_string())
                                    .or_default()
                                    .entry(prev as u8)
                                    .or_insert(0) += 1;
                            } // res_inv 寫鎖釋放
                        }
                        let stepped = vdt::QuarryDig { cells: q.cells, idx: next_idx };
                        let done = stepped.is_done();
                        {
                            let mut residents = hub().residents.write().unwrap();
                            if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                                // 挖完這口 → 清掉（仍不夠的話 ③ 再開一口，受上限約束）。
                                r.invent_quarry = if done { None } else { Some(stepped) };
                                // 站定挖井（別邊挖邊晃）：站到下一批（一個建造間隔）。
                                r.wait_timer = r.wait_timer.max(BUILD_INTERVAL_SECS);
                                r.invent_run = Some(run);
                            }
                        } // residents 寫鎖釋放
                        return; // 這輪挖井；料入袋後下個 agency tick 再推進。
                    }
                }

                // ② 地表天然源優先：找得到 → 設 GatherSkill（走既有安全機制，挖到入背包）。
                //    挖掘紀律：發明採集是居民**自主**行為 → 帶離村禁區，選址跳過村內、往村外找。
                let excl = village_dig_exclusion();
                let found = {
                    let world = hub().deltas.read().unwrap();
                    vskill::find_nearest_resource_of_excl(
                        &world, rx, rz, vinvent::INVENT_GATHER_RADIUS, resource, excl,
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
                        // ③ 地表無天然源、且這是「埋在底下」的資源（石／泥）、未達井上限
                        //    → 就地開一口階梯井往下採（`staircase_well`，永遠走得回地面、不自困）。
                        //    石器（石鎬／石斧／石鏟…）與需泥的配方第一次真的採得到料、走得完全程
                        //    ——「居民真的學會用工具」這件北極星魔法不再卡在採料步（實測發明成功率 0%）。
                        let wells = {
                            let residents = hub().residents.read().unwrap();
                            residents
                                .iter()
                                .find(|r| r.id == rid)
                                .map_or(0, |r| r.invent_quarry_wells)
                        }; // residents 讀鎖釋放
                        // 挖掘紀律：自主開礦井也受離村禁區約束——居民站在村內就不准就地開井
                        //   （井口在 rx+1，仍是村內），改成快速誠實失敗（下方 ④），逼她去村外找。
                        let in_village = vskill::in_dig_exclusion(excl, rx, rz);
                        if vinvent::resource_is_underground(resource)
                            && wells < vinvent::INVENT_MAX_WELLS
                            && !in_village
                        {
                            let q = {
                                let world = hub().deltas.read().unwrap();
                                vdt::plan_quarry(&world, rx, rz, wells)
                            }; // deltas 讀鎖釋放
                            {
                                let mut residents = hub().residents.write().unwrap();
                                if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                                    r.invent_quarry = Some(q);
                                    r.invent_quarry_wells = wells + 1;
                                    r.wait_timer = r.wait_timer.max(BUILD_INTERVAL_SECS);
                                    r.invent_run = Some(run);
                                }
                            } // residents 寫鎖釋放
                            return; // 下個 agency tick 起逐批開挖。
                        }
                        // ④ 地表無源、且非地下資源或已達井上限 → 快速誠實失敗（別等逾時）。
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
            vinvent::StepAction::DoSmelt { recipe_id } => {
                // 開爐冶煉（第四刀）：世界前提——附近真的有已放置的熔爐
                // （她這條鏈剛放的也算；被人挖走了就誠實失敗，不隔空冶煉）。
                if !station_near(vinvent::FURNACE_BLOCK_ID) {
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                }
                let Some(recipe) = vcraft::find_furnace_recipe(recipe_id) else {
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                };
                let started = {
                    let mut inv = hub().res_inv.write().unwrap();
                    vinvent::smelt_start_apply(inv.entry(rid.to_string()).or_default(), recipe)
                }; // res_inv 寫鎖釋放
                if !started {
                    // 照計畫走到這裡卻生料不足（計畫順序排錯了）→ 這次發明失敗、記教訓。
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                }
                run.smelt_wait = Some(vsmelt::smelt_secs(recipe_id) as f32);
                say_updates.push((rid.to_string(), vinvent::smelting_started_line(recipe.name_zh)));
                let mut residents = hub().residents.write().unwrap();
                if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                    r.invent_run = Some(run);
                }
                return; // 這輪開始煨煮；之後每 tick 倒數，熟成後下輪收成。
            }
            vinvent::StepAction::Waiting => {
                // 冶煉還在煨煮中——這輪什麼都不做，等下個 tick（smelt_wait 已在別處倒數）。
                let mut residents = hub().residents.write().unwrap();
                if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                    r.invent_run = Some(run);
                }
                return;
            }
            vinvent::StepAction::CollectSmelt { recipe_id } => {
                // 冶煉已熟成：交付成品進背包（不需站點/材料檢查——開爐時已驗過、扣過料）。
                let Some(recipe) = vcraft::find_furnace_recipe(recipe_id) else {
                    finish_invent_run(rid, rname, run, false, say_updates);
                    return;
                };
                {
                    let mut inv = hub().res_inv.write().unwrap();
                    let bag = inv.entry(rid.to_string()).or_default();
                    *bag.entry(recipe.output_block).or_insert(0) += recipe.output_count;
                } // res_inv 寫鎖釋放
                say_updates.push((rid.to_string(), vinvent::smelting_done_line(recipe.name_zh)));
                run.smelt_wait = None;
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
            // 清掉發明採集的階梯礦井狀態：下一次發明從乾淨狀態起（井數歸零、無殘留半挖井）。
            r.invent_quarry = None;
            r.invent_quarry_wells = 0;
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
        // 退避計數（#972 防鬼打牆）：**發明與重用兩條失敗路徑共用**。連敗達門檻同一目標
        // → 進退避、換方向探索。重用一個老是失敗的已學會技能（多半是身邊暫時沒料）同樣會
        // 鬼打牆，先前只有首次發明會退避、重用不會，導致 `reuse=true step=0` 每 ~9 秒無限
        // 重試——本次把重用也納入退避止血。
        let entered_backoff = {
            let mut residents = hub().residents.write().unwrap();
            if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                let count = r.invent_fail_counts.entry(run.goal_block).or_insert(0);
                if vinvent::note_fail_should_backoff(count) {
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

        if !run.reuse {
            // 首次發明失敗的序列不存；記一次教訓（她記得試過、下次換路子）。
            // 重用失敗多半是環境暫時問題（先前註解原意），不記進記憶、只靠退避止血。
            let entry = {
                let mut mem = hub().memory.write().unwrap();
                mem.add_memory(rid, vdes::SELF_SPARK, &vinvent::fail_lesson(&run.goal_name))
            }; // memory 寫鎖釋放
            vmem::append_memory(&entry);
        }

        if entered_backoff {
            // 進退避：Feed 一句有人味的，並冒泡提示換目標（發明/重用皆適用）。
            vfeed::append_feed(
                "發明退避",
                rname,
                &vinvent::backoff_switch_feed(&run.goal_name),
            );
            say_updates.push((rid.to_string(), vinvent::backoff_switch_line(&run.goal_name)));
            tracing::info!(
                resident = %rid, goal = %run.goal_name, reuse = run.reuse,
                "好奇心退避：連敗達門檻，暫停嘗試此目標"
            );
        } else if !run.reuse {
            // 首次發明未達門檻的失敗照舊冒一句「再想想」；重用未達門檻則安靜路過（沿用原意）。
            say_updates.push((rid.to_string(), "唔……這次沒成，再想想。".to_string()));
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
/// `furnace_nearby`（第四刀）：同理，她附近是否已有放置好的熔爐（判冶煉依賴順序）。
/// `known_skills`（第三刀·技能組合技能）：她自己技能庫裡「(名字, 原語序列)」清單——
/// 讓計畫裡的 `use_skill` 步驟能查表展開成具體原語，已經會的事不必重新拆解。
fn spawn_invention(
    rid: String,
    rname: &'static str,
    goal: vinvent::MaterialGoal,
    desire: String,
    wb_nearby: bool,
    furnace_nearby: bool,
    known_skills: Vec<(String, Vec<vinvent::PrimStep>)>,
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
    let known_names: Vec<String> = known_skills.iter().map(|(n, _)| n.clone()).collect();
    tokio::spawn(async move {
        // 解析 + 白名單 + 正規化 + 可行性模擬 → Ok(計畫) 或 Err(可回饋給腦的繁中原因)。
        let validate = |raw: &str| -> Result<vinvent::InventedPlan, String> {
            // 提案接受管線（見 accept_proposal_with_skills）：腦出**結構**（選對配方、
            // 排對依賴、取名字，可引用她已學會的技能），引擎補**算術**（確定性備料）；
            // 失敗回具體錯處（Voyager 式回饋）。
            vinvent::accept_proposal_with_skills(
                raw, &bag_snap, goal.block_id, wb_nearby, furnace_nearby, &known_skills,
            )
        };
        let (sys, user) = vinvent::invention_prompt(
            rname, &goal, &desire, &bag_note, wb_nearby, furnace_nearby, &known_names,
        );
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
/// 村莊系統 v1：替居民取得「蓋家的地塊中心」當建址基準（取代舊的家域中心）。
/// - 已認領過地塊 → 直接回原地塊中心（同一位居民所有建築都聚在自己那塊地上）。
/// - 尚未認領 → 由村莊規劃（以居民 home_base 群聚中心定村莊）挑「離自家最近的空地塊」認領、
///   持久化，並 Feed 一句「某某在村子某方向安了新家」。
/// - 村莊全滿 / 找不到空地塊 → 保守退回傳入的家域中心 (hx,hz)（維持既有散落行為，零回歸）。
/// 鎖序（嚴守短鎖即釋、鎖外 IO）：residents 讀鎖快照家域清單 → drop → village 寫鎖認領 → drop →
/// 鎖外 append 落地 jsonl + Feed。純規劃函式在鎖外算。
fn claim_or_reuse_plot(rid: &str, hx: i32, hz: i32) -> (i32, i32) {
    // 已認領過 → 沿用（鎖外只讀一次）。
    if let Some((cx, cz)) = hub().village.read().unwrap().claim_of(rid) {
        return (cx, cz);
    }
    // 村莊中心：優先用一次性整理時**釘死的中心**（旗標檔），讓認領的地塊與已鋪的道路網對齊、
    // 不隨新生兒改變質心而漂移；旗標缺（極少：整理尚未跑）才退回即時質心。
    let (vcx, vcz) = match vvillage::load_village_center() {
        Some(c) => c,
        None => {
            let home_bases: Vec<(i32, i32)> = {
                let residents = hub().residents.read().unwrap();
                residents
                    .iter()
                    .map(|r| (r.home_x.floor() as i32, r.home_z.floor() as i32))
                    .collect()
            }; // residents 讀鎖釋放
            vvillage::village_center(&home_bases)
        }
    };
    let plots = vvillage::plot_layout(vcx, vcz); // 純函式、鎖外算
    // village 寫鎖：挑最近空地塊 + 認領（double-check 併發安全：進鎖後再確認一次沒被別的 tick 搶認）。
    let (claim, plot) = {
        let mut village = hub().village.write().unwrap();
        if let Some((cx, cz)) = village.claim_of(rid) {
            return (cx, cz); // 併發下別的 tick 已幫這居民認領
        }
        match village.nearest_free_plot(&plots, hx, hz) {
            Some(p) => (Some(village.claim(rid, p.cx, p.cz)), Some(p)),
            None => (None, None), // 村莊全滿 → 退回家域中心
        }
    }; // village 寫鎖釋放
    match (claim, plot) {
        (Some(rec), Some(p)) => {
            vvillage::append_plot_claim(&rec); // 鎖外落地
            let rname = resident_name_of(rid);
            vfeed::append_feed("村莊安家", rname, &vvillage::plot_claim_feed_line(rname, &p, vcx, vcz));
            (p.cx, p.cz)
        }
        _ => (hx, hz), // 保守退回家域中心（村莊全滿）
    }
}

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
    // 村莊系統 v1：建址基準改用「認領的地塊中心」而非家域中心——蓋在地塊上＝自動沿路、對齊、
    // 不再散落一地。首次蓋家時就近認領一塊尚未被佔的空地塊並持久化；之後所有建築（含擴建）都以
    // 同一塊地塊中心當基準，用 `build_offset` 在地塊周圍散開（沿用原偏移，防鬼打牆 #967 不變）。
    // 找不到空地塊（村莊全滿或村莊未規劃）→ 保守退回原家域中心，維持既有行為（零回歸）。
    let (base_x, base_z) = claim_or_reuse_plot(rid, hx.floor() as i32, hz.floor() as i32);
    let bx = base_x + ox;
    let bz = base_z + oz;
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

// ── 居民搬新家（引導式都更）：搬家任務 tick ──────────────────────────────────────
//
// 維護者拍板：「城鎮破破爛爛」不 god-mode 重建、不放生——老家不在村莊地塊上的居民，
// **一次一位、錯開地**自己搬進村：(a) 認領地塊 →(b) Feed 昭告 →(c) 用她的樣式在地塊上
// 蓋新家（走既有 BuildPlan 引擎，完工收尾/立牌全沿用）→(d) 走回舊家逐塊拆除、材料回收
// 入她的小背包 →(e) 錨點與家域中心遷到新家。純邏輯（名單判定/一次一位狀態機/舊家方塊
// 集合重算/拆除安全過濾/重啟恢復）在 voxel_village / voxel_building / voxel_skills，
// 這裡只做排程與世界 IO——嚴守短鎖即釋、循序不巢狀、不 await 的死鎖鐵律。

/// 搬家排程的節奏狀態（純記憶體）：`timer`＝下次掃名單/推進一步的倒數；
/// `walk_stall`＝拆除段「她還沒走到舊家」的累計秒數（超時就地開拆，保證有進展）。
struct RelocPace {
    timer: f32,
    walk_stall: f32,
}

static RELOC_PACE: std::sync::Mutex<RelocPace> =
    std::sync::Mutex::new(RelocPace { timer: 10.0, walk_stall: 0.0 });

/// 搬家節奏可用 `BUTFUN_RELOC_FAST=1` 加速（隔離實測用；prod 不設＝正常節奏）。
fn reloc_fast() -> bool {
    static FAST: OnceLock<bool> = OnceLock::new();
    *FAST.get_or_init(|| std::env::var("BUTFUN_RELOC_FAST").map_or(false, |v| v == "1"))
}

/// 沒有搬家進行中時，多久掃一次待都更名單（秒）。
fn reloc_scan_secs() -> f32 {
    if reloc_fast() { 3.0 } else { 30.0 }
}
/// 搬家進行中，多久推進一步（秒）——比照建造節奏（BUILD_INTERVAL_SECS）。
fn reloc_step_secs() -> f32 {
    if reloc_fast() { 1.0 } else { BUILD_INTERVAL_SECS }
}
/// 拆除段一步拆幾塊（與建造同節奏、但一步多拆幾塊——拆比蓋快是誠實的體感，
/// 也讓全村在一兩個遊戲天內看得見地自我重組）。
fn reloc_demolish_per_step() -> usize {
    if reloc_fast() { 6 } else { 3 }
}
/// 蓋新家段每步「加放」幾塊：疊在既有建造引擎（8 秒一塊）之上等效加速——
/// 搬家戶多時才趕得上「全村 1-2 遊戲天搬完」的節奏；只對搬家中的計畫生效。
fn reloc_extra_build_per_step() -> usize {
    if reloc_fast() { 4 } else { 1 }
}
/// 一位搬完後隔多久才輪下一位（錯開；全村不會同時工地連天）。
fn reloc_gap_secs() -> f32 {
    if reloc_fast() { 3.0 } else { 90.0 }
}
/// 拆除段：走不到舊家的累計超時（秒）——超過就地開拆（誠實推進，不無限空等）。
fn reloc_walk_timeout_secs() -> f32 {
    if reloc_fast() { 10.0 } else { 60.0 }
}
/// 拆除段：走回舊家的抵達判定距離（水平，格）。
const RELOC_ARRIVE_DIST: f32 = 9.0;
/// 搬家中居民的閒晃半徑（緊貼工地打轉，看得出她在忙搬家）。
const RELOC_WANDER_RADIUS: f32 = 3.0;

/// 搬家任務主 tick（tick_residents 每輪呼叫；節奏閘讓平時只付一次倒數遞減成本）。
/// 無進行中搬家 → 低頻掃待都更名單開新的一件；有 → 依階段推進（蓋新家加放 / 拆舊家）。
fn tick_home_relocation(dt: f32, say_updates: &mut Vec<(String, String)>) {
    {
        let mut pace = RELOC_PACE.lock().unwrap();
        pace.timer -= dt;
        if pace.timer > 0.0 {
            return;
        }
    } // RELOC_PACE mutex 釋放（各分支自行重設 timer）
    let active: Option<vvillage::RelocationRecord> =
        hub().relocations.read().unwrap().active().cloned(); // relocations 讀鎖即釋

    let Some(rec) = active else {
        relocation_kickoff(say_updates);
        RELOC_PACE.lock().unwrap().timer = reloc_scan_secs();
        return;
    };

    let rid = rec.resident.clone();
    match rec.phase.as_str() {
        vvillage::RELOC_PHASE_BUILD => {
            let has_plan = { hub().builds.read().unwrap().has_plan(&rid) }; // builds 讀鎖釋放
            let new_anchor = (rec.new_x, rec.new_y, rec.new_z);
            if has_plan {
                // 建造引擎照常 8 秒一塊；這裡每步再加放幾塊（等效加速，留最後一塊給
                // 主引擎收尾——完工 Feed/錨點登記/立牌走既有唯一路徑，不重複）。
                relocation_place_extra_blocks(&rid, reloc_extra_build_per_step());
            } else if hub()
                .goals
                .read()
                .unwrap()
                .anchor_built(&rid, vbuild::BuildKind::House, new_anchor)
            {
                // 新家完工（完工收尾已由建造引擎統一處理）→ (d) 走回舊家、進拆除段。
                let advanced = { hub().relocations.write().unwrap().advance_to_demolish() };
                if let Some(adv) = advanced {
                    vvillage::append_relocation(&adv);
                    let rname = resident_name_of(&rid);
                    vfeed::append_feed("都更搬家", rname, &vvillage::reloc_demolish_feed_line(rname));
                    say_updates.push((rid.clone(), vvillage::reloc_demolish_say_line().to_string()));
                    {
                        let mut residents = hub().residents.write().unwrap();
                        if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                            r.target_x = rec.old_x as f32 + 0.5;
                            r.target_z = rec.old_z as f32 + 0.5;
                            r.wait_timer = 0.0;
                        }
                    } // residents 寫鎖釋放
                    RELOC_PACE.lock().unwrap().walk_stall = 0.0;
                    tracing::info!(resident = %rid, "都更搬家：新家完工，回舊家拆料");
                }
            } else {
                // 中斷恢復（極少：搬家已登記、計畫卻遺失且新家未完工）→ 冪等重建計畫續蓋。
                let plan = {
                    let mut builds = hub().builds.write().unwrap();
                    if builds.has_plan(&rid) {
                        None
                    } else {
                        Some(builds.new_plan(
                            &rid,
                            vbuild::BuildKind::House,
                            rec.new_x,
                            rec.new_y,
                            rec.new_z,
                            false,
                            None,
                        ))
                    }
                }; // builds 寫鎖釋放
                if let Some(p) = &plan {
                    vbuild::append_build(p);
                }
            }
            RELOC_PACE.lock().unwrap().timer = reloc_step_secs();
        }
        vvillage::RELOC_PHASE_DEMOLISH => {
            RELOC_PACE.lock().unwrap().timer = reloc_step_secs();
            relocation_demolish_step(&rec, say_updates);
        }
        _ => {
            // 防呆：不明階段（理論到不了）→ 收尾釋放名額，別卡死全村輪替。
            if let Some(f) = hub().relocations.write().unwrap().finish() {
                vvillage::append_relocation(&f);
            }
            RELOC_PACE.lock().unwrap().timer = reloc_gap_secs();
        }
    }
}

/// 掃待都更名單、挑第一位「此刻閒著」的居民開工搬家（一次至多開一件）。
/// (a) 認領地塊 →(b) Feed「開始把家搬到村裡的新地塊」→(c) 動工新家（既有 BuildPlan 引擎）。
fn relocation_kickoff(say_updates: &mut Vec<(String, String)>) {
    // 村莊還沒規劃（無釘死中心）→ 沒有地塊可搬，安靜早退（舊世界向後相容）。
    let Some((vcx, vcz)) = vvillage::load_village_center() else { return };
    let plots = vvillage::plot_layout(vcx, vcz); // 純函式、鎖外算
    let houses = { hub().goals.read().unwrap().all_houses() }; // goals 讀鎖釋放
    let done = { hub().relocations.read().unwrap().done_residents() }; // relocations 讀鎖釋放
    let cands = vvillage::relocation_candidates(&houses, &plots, &done);
    for (rid, old) in cands {
        // 此刻在忙（已有建造計畫/被指派整地/發明/跑腿/跟隨/睡著）→ 這輪先跳過她，
        // 搬家不打斷玩家指派或進行中的大事（比照既有 gating 精神，下輪掃描再看）。
        // 自發的散步採集（gather）不算大事——搬家開工時她會自己放下（見下方清除）。
        if hub().builds.read().unwrap().has_plan(&rid) {
            continue; // builds 讀鎖釋放
        }
        if hub().directed_tasks.read().unwrap().contains_key(&rid) {
            continue; // directed_tasks 讀鎖釋放
        }
        let busy = {
            let residents = hub().residents.read().unwrap();
            residents.iter().find(|r| r.id == rid).map_or(true, |r| {
                r.invent_run.is_some() || r.fetch.is_some() || r.follow.is_some() || r.asleep
            })
        }; // residents 讀鎖釋放
        if busy {
            continue;
        }
        // (a) 認領地塊（已認領過＝沿用同一塊；村滿認不到 → 這位搬不了，換下一位）。
        let (pcx, pcz) = claim_or_reuse_plot(&rid, old.0, old.2);
        if hub().village.read().unwrap().claim_of(&rid).is_none() {
            continue; // village 讀鎖釋放
        }
        // 挑新家錨位：避開她既有建物錨點（同地塊上可能已有花圃/水井等）。
        let spots: Vec<(i32, i32)> = (0..6).map(vskill::build_offset).collect();
        let taken = { hub().goals.read().unwrap().anchors_xz_of(&rid) }; // goals 讀鎖釋放
        let Some((bx, bz)) = vvillage::first_free_spot(pcx, pcz, &spots, &taken) else {
            continue; // 她的地塊排不下新家（極少）→ 換下一位
        };
        let by = vbuild::surface_y(bx, bz);
        // 一次一位硬閘（store 內建）：登記 build 階段並落地。
        let rec = { hub().relocations.write().unwrap().begin(&rid, old, (bx, by, bz)) };
        let Some(rec) = rec else { return }; // 併發防呆：已有人在搬 → 本輪收手
        vvillage::append_relocation(&rec);
        // (c) 動工新家：走既有 BuildPlan 引擎（她的樣式 BuildStyle::for_resident /
        // 放塊節奏 / 進度冒泡 / 完工收尾與立牌全沿用，零新建造路徑）。
        let plan = {
            let mut builds = hub().builds.write().unwrap();
            if builds.has_plan(&rid) {
                None // double-check 併發安全
            } else {
                Some(builds.new_plan(&rid, vbuild::BuildKind::House, bx, by, bz, false, None))
            }
        }; // builds 寫鎖釋放
        if let Some(p) = &plan {
            vbuild::append_build(p);
        }
        // (b) Feed + 泡泡 + 她放下手邊採集、動身走向新地塊（閒晃中心由搬家焦點接管）。
        let rname = resident_name_of(&rid);
        vfeed::append_feed("都更搬家", rname, &vvillage::reloc_start_feed_line(rname));
        say_updates.push((rid.clone(), vvillage::reloc_start_say_line().to_string()));
        {
            let mut residents = hub().residents.write().unwrap();
            if let Some(r) = residents.iter_mut().find(|r| r.id == rid) {
                r.gather = None; // 搬家是大事：自發的散步採集先放下
                // 建造冷卻歸零：搬家是刻意決策，不是完工連發——否則剛蓋完別的建物的
                // 300 秒冷卻會把新家計畫的收尾（agency 放最後一塊+完工登記）卡住。
                r.build_cooldown = 0.0;
                r.target_x = bx as f32 + 0.5;
                r.target_z = bz as f32 + 0.5;
                r.wait_timer = 0.0;
            }
        } // residents 寫鎖釋放
        tracing::info!(resident = %rid, old = ?old, new = ?(bx, by, bz), "都更搬家：認領地塊、動工新家");
        return; // 一次只開一件（錯開）
    }
}

/// 蓋新家段的「加放」：既有建造引擎（8 秒一塊）之外每步多放幾塊——只對搬家中的計畫
/// 生效，且**永遠留最後一塊**給主引擎收尾（完工 Feed/錨點登記/心願閉環/立牌走既有唯一
/// 路徑、不重複觸發）。放置流程與主引擎逐項相同：set_block → 廣播 → 水流喚醒 → 持久化。
fn relocation_place_extra_blocks(rid: &str, n: usize) {
    let mut placed = 0usize;
    for _ in 0..n {
        let bb = {
            let mut builds = hub().builds.write().unwrap();
            builds
                .get_plan_mut(rid)
                .and_then(|p| if p.remaining.len() > 1 { p.pop_next() } else { None })
        }; // builds 寫鎖釋放
        let Some(bb) = bb else { break };
        if let Some(block) = Block::from_u8(bb.b) {
            {
                let mut world = hub().deltas.write().unwrap();
                voxel::set_block(&mut world, bb.x, bb.y, bb.z, block);
            } // deltas 寫鎖釋放
            broadcast_block(bb.x, bb.y, bb.z, block);
            enqueue_water_around(bb.x, bb.y, bb.z);
            vbuild::append_world_block(bb.x, bb.y, bb.z, bb.b);
            placed += 1;
        }
    }
    if placed > 0 {
        // 持久化更新後的計畫（remaining 已縮短，重啟後接著蓋）。
        if let Some(plan) = hub().builds.read().unwrap().plans.get(rid) {
            vbuild::append_build(plan);
        } // builds 讀鎖釋放
    }
}

/// 拆除段單步：她在舊家附近（或走路超時）→ 依**確定性重算**的舊家方塊集合拆幾塊、
/// 材料回收入她的小背包；集合裡再也沒有可拆的格 → 搬家收尾（錨點/家域遷移）。
/// 資料安全鐵律：只有「集合裡的座標 + 現況方塊正是她當年放的那塊」才拆
/// （見 `vbuild::demolish_allowed`）——玩家平台/告示牌/箱子/鄰居建物機制性一塊碰不到。
fn relocation_demolish_step(
    rec: &vvillage::RelocationRecord,
    say_updates: &mut Vec<(String, String)>,
) {
    let rid = &rec.resident;
    // 她此刻的狀態快照（residents 讀鎖即釋）。
    let snap = {
        let residents = hub().residents.read().unwrap();
        residents.iter().find(|r| r.id == *rid).map(|r| {
            (
                r.body.x,
                r.body.y,
                r.body.z,
                r.asleep,
                r.fetch.is_some() || r.follow.is_some(),
            )
        })
    }; // residents 讀鎖釋放
    let Some((px, py, pz, asleep, player_task)) = snap else {
        // 居民不存在（理論到不了）→ 收尾釋放名額，別卡死全村輪替。
        if let Some(f) = hub().relocations.write().unwrap().finish() {
            vvillage::append_relocation(&f);
        }
        RELOC_PACE.lock().unwrap().timer = reloc_gap_secs();
        return;
    };
    // 睡著／正被玩家指派任務（跟隨/跑腿/整地）→ 搬家讓路暫停，醒來/忙完下一步再續拆。
    let directed = hub().directed_tasks.read().unwrap().contains_key(rid); // 讀鎖即釋
    if asleep || player_task || directed {
        return;
    }
    // 還沒走到舊家：繼續走（目標持續指向舊家），累計超時後就地開拆（誠實推進）。
    let dx = px - (rec.old_x as f32 + 0.5);
    let dz = pz - (rec.old_z as f32 + 0.5);
    let near = (dx * dx + dz * dz).sqrt() <= RELOC_ARRIVE_DIST;
    if !near {
        let timed_out = {
            let mut pace = RELOC_PACE.lock().unwrap();
            pace.walk_stall += reloc_step_secs();
            pace.walk_stall >= reloc_walk_timeout_secs()
        }; // RELOC_PACE mutex 釋放
        if !timed_out {
            let mut residents = hub().residents.write().unwrap();
            if let Some(r) = residents.iter_mut().find(|r| r.id == *rid) {
                r.target_x = rec.old_x as f32 + 0.5;
                r.target_z = rec.old_z as f32 + 0.5;
            }
            return; // residents 寫鎖釋放
        }
    }

    // 舊家方塊集合（確定性重算，見 `vbuild::house_blocks_at`）——只有這份集合裡的格才可能被拆。
    let expected = vbuild::house_blocks_at(rid, rec.old_x, rec.old_y, rec.old_z);
    let mut removed = 0usize;
    let mut skipped_body = 0usize;
    let mut removable_left = false;
    for bb in &expected {
        // 現況（deltas 短讀鎖即釋，逐格取放——一步最多幾塊，非熱路徑）。
        let current = {
            let world = hub().deltas.read().unwrap();
            voxel::effective_block_at(&world, bb.x, bb.y, bb.z)
        }; // deltas 讀鎖釋放
        if !vbuild::demolish_allowed(bb.b, current) {
            continue; // 不是她當年放的那塊（早拆過/被改動/玩家物）→ 一律不動
        }
        if removed >= reloc_demolish_per_step() {
            removable_left = true; // 本步額度用完，還有得拆 → 下一步繼續
            break;
        }
        // 安全：不拆與她身體重疊的格（永不自拆站位；她挪開後下一步再拆）。
        if vdt::cell_in_body(bb.x, bb.y, bb.z, px, py, pz) {
            skipped_body += 1;
            continue;
        }
        // 拆！地板層回復自然基底（地表不留坑）、其餘回空氣；廣播 + 水流喚醒 + 持久化。
        let restore = vbuild::demolition_restore(bb.x, bb.y, bb.z);
        {
            let mut world = hub().deltas.write().unwrap();
            voxel::set_block(&mut world, bb.x, bb.y, bb.z, restore);
        } // deltas 寫鎖釋放
        broadcast_block(bb.x, bb.y, bb.z, restore);
        enqueue_water_around(bb.x, bb.y, bb.z);
        vbuild::append_world_block(bb.x, bb.y, bb.z, restore as u8);
        // 材料回收入她的小背包（res_inv 短鎖即釋，比照礦井挖掘入包慣例）。
        {
            let mut inv = hub().res_inv.write().unwrap();
            *inv.entry(rid.to_string())
                .or_default()
                .entry(vbuild::demolition_yield(bb.b))
                .or_insert(0) += 1;
        } // res_inv 寫鎖釋放
        removed += 1;
    }

    if removed == 0 && skipped_body > 0 {
        // 只剩她站位擋著的那幾格：把她請往新家方向挪開，下一步就拆得到了。
        let mut residents = hub().residents.write().unwrap();
        if let Some(r) = residents.iter_mut().find(|r| r.id == *rid) {
            r.target_x = rec.new_x as f32 + 0.5;
            r.target_z = rec.new_z as f32 + 0.5;
            r.wait_timer = 0.0;
        }
        return; // residents 寫鎖釋放
    }
    if removed == 0 && !removable_left && skipped_body == 0 {
        // (e) 集合裡再也沒有可拆的格＝舊家拆完 → 搬家收尾。
        relocation_finish(rec, say_updates);
    }
}

/// 搬家收尾：舊家拆完 → 移舊錨點、小屋座標與家域中心遷到新家（含持久化）＋ Feed/泡泡，
/// 釋放「一次一位」名額（隔 `reloc_gap_secs` 後輪下一位）。
fn relocation_finish(
    rec: &vvillage::RelocationRecord,
    say_updates: &mut Vec<(String, String)>,
) {
    let rid = &rec.resident;
    let fin = { hub().relocations.write().unwrap().finish() }; // relocations 寫鎖釋放
    let Some(fin) = fin else { return };
    vvillage::append_relocation(&fin);
    // built_anchors 更新：移舊錨點、小屋座標遷到新家（新家錨點完工時已由建造引擎登記）。
    let (removal, moved) = {
        let mut goals = hub().goals.write().unwrap();
        goals.relocate_house(
            rid,
            (rec.old_x, rec.old_y, rec.old_z),
            (rec.new_x, rec.new_y, rec.new_z),
        )
    }; // goals 寫鎖釋放
    vskill::append_goal(&removal);
    vskill::append_goal(&moved);
    // 家域中心遷到新家：日常活動圈（閒晃/歸巢/遠行基準）從此繞著村裡的新家。
    {
        let mut residents = hub().residents.write().unwrap();
        if let Some(r) = residents.iter_mut().find(|r| r.id == *rid) {
            r.home_x = rec.new_x as f32 + 0.5;
            r.home_z = rec.new_z as f32 + 0.5;
            r.target_x = r.home_x;
            r.target_z = r.home_z;
            r.wait_timer = 0.0;
        }
    } // residents 寫鎖釋放
    let rname = resident_name_of(rid);
    vfeed::append_feed("都更搬家", rname, &vvillage::reloc_done_feed_line(rname));
    say_updates.push((rid.clone(), vvillage::reloc_done_say_line().to_string()));
    {
        let mut pace = RELOC_PACE.lock().unwrap();
        pace.timer = reloc_gap_secs();
        pace.walk_stall = 0.0;
    } // RELOC_PACE mutex 釋放
    // 誠實記錄：她背包目前的材料總數（含拆舊家回收的）——隔離實測據此驗「材料真的入包」。
    let bag_total: u32 = {
        let inv = hub().res_inv.read().unwrap();
        inv.get(rid).map_or(0, |b| b.values().sum())
    }; // res_inv 讀鎖釋放
    tracing::info!(
        resident = %rid, bag_total,
        new_home = ?(rec.new_x, rec.new_y, rec.new_z),
        "都更搬家：完成（舊家拆除回收、家域遷至新家）"
    );
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
            } else if d.sparked_by == vdes::ENVY_SPARK {
                format!(
                    "你親眼見過一件讓你心生嚮往的作品後，冒出了一個念頭：「{}」——\
                    沒有人要求你這麼做，這個夢想是你自己心裡冒出來的。",
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
        // 舊前端不送 held（向後相容）→ 預設 None。
        assert!(matches!(m, ClientMsg::Move { held: None, .. }));
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
    fn move_with_held_parses() {
        // 手持工具可見 v1：帶 held 欄位（有選中物品）。
        let m: ClientMsg = serde_json::from_str(
            r#"{"t":"move","x":0.0,"y":64.0,"z":0.0,"yaw":0.0,"held":34}"#,
        )
        .unwrap();
        assert!(matches!(m, ClientMsg::Move { held: Some(34), .. }));
        // held: null（熱鍵格清空）也要能解析成 None。
        let m2: ClientMsg = serde_json::from_str(
            r#"{"t":"move","x":0.0,"y":64.0,"z":0.0,"yaw":0.0,"held":null}"#,
        )
        .unwrap();
        assert!(matches!(m2, ClientMsg::Move { held: None, .. }));
    }

    #[test]
    fn break_and_place_parse() {
        let b: ClientMsg = serde_json::from_str(r#"{"t":"break","x":3,"y":9,"z":-4}"#).unwrap();
        match b {
            // 工欲善其事 v1（790）：舊訊息不帶 tool 欄位 → 預設 None，向後相容。
            ClientMsg::Break { x, y, z, tool } => {
                assert_eq!((x, y, z), (3, 9, -4));
                assert_eq!(tool, None);
            }
            _ => panic!("應解析成 Break"),
        }
        // 帶 tool 欄位（新前端）也要正確解析。
        let bt: ClientMsg =
            serde_json::from_str(r#"{"t":"break","x":1,"y":2,"z":3,"tool":33}"#).unwrap();
        match bt {
            ClientMsg::Break { tool, .. } => assert_eq!(tool, Some(33)),
            _ => panic!("應解析成 Break"),
        }
        let p: ClientMsg =
            serde_json::from_str(r#"{"t":"place","x":1,"y":10,"z":2,"b":3}"#).unwrap();
        match p {
            ClientMsg::Place { x, y, z, b } => assert_eq!((x, y, z, b), (1, 10, 2, 3)),
            _ => panic!("應解析成 Place"),
        }
    }

    /// 修回歸測試（真 bug）：`TradeRequest`/`TradeAccept`/`OpenChest`/`ChestPut`/`ChestTake`
    /// 這 5 個多詞 variant 過去沒加 `#[serde(rename=...)]`，`rename_all="lowercase"` 不分詞會
    /// 期待 `"tradeaccept"`/`"openchest"` 等無底線 tag，但前端 `web/voxel/main.js` 一直送的是
    /// `"trade_request"`/`"open_chest"` 等底線寫法——兩邊對不上，解析全數悄悄失敗、被最後的
    /// `_ => {}` 吞掉，箱子開/存/取與玩家向居民請求交易因此**完全不通但零錯誤訊息**。
    /// 這裡直接餵前端實際會送的字串，鎖死正確 tag，防止日後重構又漏了 rename。
    #[test]
    fn chest_and_trade_tags_match_frontend() {
        let m: ClientMsg =
            serde_json::from_str(r#"{"t":"open_chest","x":1,"y":2,"z":3}"#).unwrap();
        match m {
            ClientMsg::OpenChest { x, y, z } => assert_eq!((x, y, z), (1, 2, 3)),
            _ => panic!("應解析成 OpenChest"),
        }
        let m: ClientMsg = serde_json::from_str(
            r#"{"t":"chest_put","x":1,"y":2,"z":3,"item_id":8,"count":2}"#,
        )
        .unwrap();
        match m {
            ClientMsg::ChestPut { x, y, z, item_id, count } => {
                assert_eq!((x, y, z, item_id, count), (1, 2, 3, 8, 2))
            }
            _ => panic!("應解析成 ChestPut"),
        }
        let m: ClientMsg = serde_json::from_str(
            r#"{"t":"chest_take","x":1,"y":2,"z":3,"item_id":8,"count":2}"#,
        )
        .unwrap();
        match m {
            ClientMsg::ChestTake { x, y, z, item_id, count } => {
                assert_eq!((x, y, z, item_id, count), (1, 2, 3, 8, 2))
            }
            _ => panic!("應解析成 ChestTake"),
        }
        let m: ClientMsg =
            serde_json::from_str(r#"{"t":"trade_request","resident_id":"vox_res_0"}"#).unwrap();
        match m {
            ClientMsg::TradeRequest { resident_id } => assert_eq!(resident_id, "vox_res_0"),
            _ => panic!("應解析成 TradeRequest"),
        }
        let m: ClientMsg =
            serde_json::from_str(r#"{"t":"trade_accept","resident_id":"vox_res_0"}"#).unwrap();
        match m {
            ClientMsg::TradeAccept { resident_id, pay_with_coin } => {
                assert_eq!(resident_id, "vox_res_0");
                assert!(!pay_with_coin, "省略 pay_with_coin 應預設 false（v1 原行為不變）");
            }
            _ => panic!("應解析成 TradeAccept"),
        }
    }

    /// 付幣代替湊材料 v1（ROADMAP 874）：`pay_with_coin` 欄位能正確解析成 true。
    #[test]
    fn trade_accept_pay_with_coin_parses() {
        let m: ClientMsg = serde_json::from_str(
            r#"{"t":"trade_accept","resident_id":"vox_res_1","pay_with_coin":true}"#,
        )
        .unwrap();
        match m {
            ClientMsg::TradeAccept { resident_id, pay_with_coin } => {
                assert_eq!(resident_id, "vox_res_1");
                assert!(pay_with_coin);
            }
            _ => panic!("應解析成 TradeAccept"),
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
    fn talk_requires_login_gate() {
        // 治安三件套③：登入帳號才能發起對話（觸發免費 LLM）；訪客擋下。
        assert!(talk_allowed_for_identity(true), "登入帳號應可與居民交談");
        assert!(!talk_allowed_for_identity(false), "訪客應被擋下、不可觸發 LLM");
        // 訪客提示非空、且是面向玩家的溫柔字串（i18n 集中於此常數）。
        assert!(!TALK_GUEST_NOTICE.is_empty());
    }

    #[test]
    fn violation_cooldown_grows_and_caps() {
        // 治安三件套①·累犯加長冷卻：0 次違規＝零冷卻；違規越多冷卻越長；封頂不無限長。
        assert_eq!(violation_cooldown_ms(0), 0, "無違規＝零冷卻（正常玩家零感知）");
        assert!(violation_cooldown_ms(1) > 0, "第一次違規就有一點冷卻");
        assert!(
            violation_cooldown_ms(3) > violation_cooldown_ms(1),
            "違規越多冷卻越長（遞增）"
        );
        // 封頂：極多次違規不會爆長（30 步封頂）。
        assert_eq!(
            violation_cooldown_ms(1000),
            violation_cooldown_ms(30),
            "冷卻有封頂、不無限長"
        );
    }

    #[test]
    fn ip_limit_whitelist_exempts_localhost_and_env() {
        // 治安三件套②：localhost / 保底桶恆豁免（本機冒煙、隔離測試不受連線數/速率上限干擾）。
        assert!(ip_limit_exempt("127.0.0.1", &[]));
        assert!(ip_limit_exempt("::1", &[]));
        assert!(ip_limit_exempt("unknown", &[]));
        // 一般公網 IP 不豁免（照常受限）。
        assert!(!ip_limit_exempt("203.0.113.7", &[]));
        // env 白名單補充命中。
        let wl = vec!["10.0.0.9".to_string()];
        assert!(ip_limit_exempt("10.0.0.9", &wl));
        assert!(!ip_limit_exempt("10.0.0.10", &wl));
    }

    #[test]
    fn max_conn_per_ip_defaults_and_clamps() {
        // 未設 env → 用預設（vrl::MAX_CONN_PER_IP）。
        std::env::remove_var("BUTFUN_MAX_CONN_PER_IP");
        assert_eq!(max_conn_per_ip(), vrl::MAX_CONN_PER_IP);
        // 壞值 / 0 → 退預設；合法值 → 採用。
        std::env::set_var("BUTFUN_MAX_CONN_PER_IP", "0");
        assert_eq!(max_conn_per_ip(), vrl::MAX_CONN_PER_IP, "0 不可把人全鎖死，退預設");
        std::env::set_var("BUTFUN_MAX_CONN_PER_IP", "abc");
        assert_eq!(max_conn_per_ip(), vrl::MAX_CONN_PER_IP, "壞值退預設");
        std::env::set_var("BUTFUN_MAX_CONN_PER_IP", "8");
        assert_eq!(max_conn_per_ip(), 8);
        std::env::remove_var("BUTFUN_MAX_CONN_PER_IP");
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

    // ── 同帳號去重（幽靈分身修復）測試 ───────────────────────────────────────────

    /// 建一個最小 VoxelPlayer，方便去重測試用（account 欄位可指定）。
    fn make_player(id: Uuid, account: Option<&str>) -> VoxelPlayer {
        VoxelPlayer {
            id,
            name: "測試玩家".to_string(),
            x: 0.0,
            y: 64.0,
            z: 0.0,
            yaw: 0.0,
            say: String::new(),
            say_timer: 0.0,
            title: None,
            account: account.map(String::from),
            held: None,
        }
    }

    #[test]
    fn remove_duplicate_account_kicks_old_entry() {
        // 同帳號第二次 join → 第一個 entry 被移除、回傳舊 UUID。
        let mut players: HashMap<Uuid, VoxelPlayer> = HashMap::new();
        let old_id = Uuid::new_v4();
        players.insert(old_id, make_player(old_id, Some("player@example.com")));

        let result = remove_duplicate_account(&mut players, "player@example.com");

        assert_eq!(result, Some(old_id), "應回傳舊連線 UUID");
        assert!(players.is_empty(), "players 表中舊 entry 應已移除");
    }

    #[test]
    fn remove_duplicate_account_guest_not_kicked() {
        // 訪客（account = None）第二條連線 → 不互踢（允許多訪客同時在線）。
        let mut players: HashMap<Uuid, VoxelPlayer> = HashMap::new();
        let guest1 = Uuid::new_v4();
        let guest2 = Uuid::new_v4();
        players.insert(guest1, make_player(guest1, None));
        players.insert(guest2, make_player(guest2, None));

        // 以任何 email 去重，兩位訪客都不受波及（他們的 account 是 None）。
        let result = remove_duplicate_account(&mut players, "someone@example.com");

        assert_eq!(result, None, "訪客帳號不應被去重踢除");
        assert_eq!(players.len(), 2, "兩位訪客都應仍在線");
    }

    #[test]
    fn remove_duplicate_account_different_accounts_not_kicked() {
        // 不同帳號互不干擾：去重只踢同一 email 的舊連線。
        let mut players: HashMap<Uuid, VoxelPlayer> = HashMap::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        players.insert(id_a, make_player(id_a, Some("alice@example.com")));
        players.insert(id_b, make_player(id_b, Some("bob@example.com")));

        // Bob 重連，只踢 Bob 的舊連線，Alice 不受影響。
        let result = remove_duplicate_account(&mut players, "bob@example.com");

        assert_eq!(result, Some(id_b), "應回傳 Bob 的舊 UUID");
        assert!(players.contains_key(&id_a), "Alice 的連線應仍在線");
        assert!(!players.contains_key(&id_b), "Bob 的舊連線應已移除");
    }

    #[test]
    fn remove_duplicate_account_no_existing_returns_none() {
        // 帳號首次登入（players 表沒有同帳號的舊 entry）→ 回 None，不動 players。
        let mut players: HashMap<Uuid, VoxelPlayer> = HashMap::new();
        let id = Uuid::new_v4();
        players.insert(id, make_player(id, Some("other@example.com")));

        let result = remove_duplicate_account(&mut players, "new@example.com");

        assert_eq!(result, None, "首次登入不應觸發去重");
        assert_eq!(players.len(), 1, "players 表不應被改動");
    }

    #[test]
    fn account_field_not_serialized() {
        // account 欄位帶 #[serde(skip)]，廣播的 JSON 中永遠看不到此值——安全防護。
        let id = Uuid::new_v4();
        let p = make_player(id, Some("secret@example.com"));
        let json = serde_json::to_string(&p).expect("序列化不應失敗");
        assert!(
            !json.contains("account"),
            "account 欄位不應出現在廣播 JSON 中：{json}"
        );
        assert!(
            !json.contains("secret@example.com"),
            "email 不應出現在廣播 JSON 中：{json}"
        );
    }

    // ── 工作台/熔爐配方伺服器閘門（自主提案切片）───────────────────────────
    // station_nearby：伺服器權威判定玩家附近是否真的有指定站別方塊，
    // 不信任「client 目前開哪個面板」的自報狀態。

    #[test]
    fn station_nearby_true_when_workbench_in_range() {
        let mut deltas = WorldDelta::new();
        voxel::set_block(&mut deltas, 10, 5, 10, Block::Workbench);
        assert!(station_nearby(&deltas, 11.5, 5.0, 10.5, Block::Workbench));
    }

    #[test]
    fn station_nearby_false_when_absent() {
        let deltas = WorldDelta::new();
        assert!(!station_nearby(&deltas, 0.0, 5.0, 0.0, Block::Workbench));
    }

    #[test]
    fn station_nearby_false_when_out_of_range() {
        let mut deltas = WorldDelta::new();
        voxel::set_block(&mut deltas, 100, 5, 100, Block::Workbench);
        assert!(
            !station_nearby(&deltas, 0.0, 5.0, 0.0, Block::Workbench),
            "太遠的工作台不該算數，否則等於沒有門檻"
        );
    }

    #[test]
    fn station_nearby_distinguishes_block_kind() {
        let mut deltas = WorldDelta::new();
        voxel::set_block(&mut deltas, 10, 5, 10, Block::Furnace);
        assert!(station_nearby(&deltas, 11.0, 5.0, 10.0, Block::Furnace));
        assert!(
            !station_nearby(&deltas, 11.0, 5.0, 10.0, Block::Workbench),
            "熔爐不該被誤判成工作台，兩種站別各自獨立把關"
        );
    }

    #[test]
    fn station_nearby_xz_boundary_inclusive() {
        let mut deltas = WorldDelta::new();
        // 剛好在 STATION_RANGE_XZ 邊界上，邊界含頭含尾應仍算數。
        voxel::set_block(&mut deltas, STATION_RANGE_XZ, 5, 0, Block::Workbench);
        assert!(station_nearby(&deltas, 0.5, 5.0, 0.5, Block::Workbench));
    }

    #[test]
    fn station_nearby_y_out_of_vertical_range() {
        let mut deltas = WorldDelta::new();
        // 同一 XZ 位置，但垂直落差超過 STATION_RANGE_Y——不同樓層的工作台不算數。
        voxel::set_block(&mut deltas, 0, 5 + STATION_RANGE_Y + 3, 0, Block::Workbench);
        assert!(!station_nearby(&deltas, 0.5, 5.0, 0.5, Block::Workbench));
    }
}
