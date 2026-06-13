//! 路人 / 居民 NPC 系統（ROADMAP 115 + 116）。
//!
//! 城鎮隨繁榮度自然成長一批「廉價」路人——純模板驅動，不呼叫 LLM。
//! 每位居民在城鎮範圍內緩慢閒晃，讓世界看起來有人氣。
//!
//! 分層架構（成本鐵律）：
//! - 少數深度 AI NPC（merchant / village_chief 等）→ 呼叫 LLM，有個性、記憶、生命週期。
//! - 多數路人居民（本模組）→ 模板行為，零 LLM 費用，只負責讓世界「看起來熱鬧」。
//!
//! 人口規則（湧現，非寫死）：
//! - 每位居民有自己的壽命計時器（ROADMAP 116）：
//!   90% 壽命 → 廣播告別倒數；100% → 回歸乙太，新居民遷入替補。
//! - 每 POPULATION_CHECK_SECS 秒依「全村平均繁榮感」增減人口：
//!   > GROW_THRESHOLD → 新移民遷入（含廣播）；< SHRINK_THRESHOLD → 靜靜離去（含廣播）。
//!
//! 完全記憶體模式，重啟清零，零 migration。

use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use crate::daynight::Phase;

// ── 居民生命週期常數（ROADMAP 116）──────────────────────────────────────────
/// 居民壽命預設（秒，真實時間）。約 2 天——讓居民像「長期老鄰居」般穩定存在（跨多次遊玩仍是同一批
/// 面孔），也成為穩定的人才庫（服務型 NPC 退休時提拔最年長居民接班，資歷才有意義）。比服務型 NPC
/// （約 1 天）長，確保隨時都有夠資深的居民可被提拔。可用 BUTFUN_RESIDENT_LIFESPAN_SECS 覆寫。
pub const RESIDENT_LIFESPAN_SECS_DEFAULT: f32 = 172800.0;
const RESIDENT_RETIREMENT_FRACTION: f32 = 0.90;

pub fn resident_lifespan_secs() -> f32 {
    std::env::var("BUTFUN_RESIDENT_LIFESPAN_SECS")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(RESIDENT_LIFESPAN_SECS_DEFAULT)
}

// ── 生命週期公告文字 ──────────────────────────────────────────────────────────
fn retirement_msg(name: &str) -> String {
    format!("🕯️ {} 感到乙太的呼喚，即將離開這片土地……", name)
}

fn farewell_msg(name: &str) -> String {
    format!("✨ {} 在乙太之光中安詳告別，感謝大家這段時間的陪伴。", name)
}

fn arrival_from_predecessor_msg(new_name: &str, old_name: &str) -> String {
    format!("🌱 {} 帶著對 {} 的思念遷入村落，展開全新生活。", new_name, old_name)
}

fn new_arrival_msg(name: &str) -> String {
    format!("🌱 {} 從遠方遷入，為村落帶來新氣象。", name)
}

fn departed_msg(name: &str) -> String {
    format!("🍂 {} 決定離開村落，踏上新的旅途。祝一路平安。", name)
}

// ── 時段切換廣播（ROADMAP 119）───────────────────────────────────────────────
fn phase_transition_msg(phase: Phase) -> &'static str {
    match phase {
        Phase::Dawn  => "🌅 黎明來臨，居民們紛紛出門，往市集方向走去。",
        Phase::Day   => "☀️ 白晝正盛，居民各歸崗位，各自忙碌起來。",
        Phase::Dusk  => "🌆 夕陽西下，居民陸陸續續走向廣場閒聊。",
        Phase::Night => "🌙 夜幕低垂，居民收起腳步，緩緩往住家方向走去。",
    }
}

// ── 生命週期事件（ROADMAP 116）───────────────────────────────────────────────
/// 居民思想泡泡事件（ROADMAP 118）：居民自發浮出思想泡泡，由 game.rs 廣播 NpcSpeech。
pub struct ResidentThoughtEvent {
    pub id: String,
    pub name: &'static str,
    pub persona: ResidentPersona,
    pub x: f32,
    pub y: f32,
    /// 供模板選取的種子。
    pub seed: usize,
    /// 凱旋餘韻談資（ROADMAP 186）：true 時改用「勝利談資」模板（居民在聊剛斬下的菁英首領），
    /// 由 game.rs 廣播時據此切換模板；`#[serde(default)]` 無關（純後端事件）。
    pub triumph: bool,
}

/// 居民生命週期事件，由 game.rs 廣播至世界聊天。
pub enum ResidentLifecycleEvent {
    /// 居民即將回歸乙太（90% 壽命），廣播告別公告。
    RetirementSoon { name: &'static str, msg: String },
    /// 居民完成生命週期，新居民遷入替補。
    RetiredToEther { old_name: &'static str, new_name: &'static str, farewell_msg: String, arrival_msg: String },
    /// 繁榮帶來新移民遷入（不是替補，是真的人口增加）。
    NewArrival { name: &'static str, msg: String },
    /// 凋零造成人口外移（靜靜離去）。
    Departed { name: &'static str, msg: String },
    /// 時段切換（ROADMAP 119）：居民換到新時段對應的聚場。
    PhaseTransition { phase: Phase, msg: &'static str },
    /// 居民工作動態廣播（ROADMAP 120）：居民在工作時段定期廣播活動。
    WorkActivity { text: String },
    /// 兩位居民相遇打招呼（ROADMAP 121）：停下互道一聲，廣播雙方 NpcSpeech。
    NeighborChat {
        id_a: String, name_a: &'static str, text_a: String, x_a: f32, y_a: f32,
        id_b: String, name_b: &'static str, text_b: String, x_b: f32, y_b: f32,
    },
    /// 居民隨機小事件（ROADMAP 122）：偶爾廣播日常生活趣事至世界聊天。
    MiniEvent { text: String },
    /// 生態危機警戒開始（ROADMAP 180）：生態壓力衝頂，居民奔回城中心廣場避難。
    EcoAlarm { msg: String },
    /// 生態危機解除（ROADMAP 180）：壓力回落，居民鬆口氣散回日常。
    EcoCalm { msg: String },
    /// 居民主動向玩家打招呼（ROADMAP 123）：廣播 NpcSpeech 泡泡 + 世界聊天通知。
    PlayerGreeting {
        resident_id: String,
        resident_name: &'static str,
        x: f32,
        y: f32,
        player_name: String,
        text: String,
    },
    /// 居民發出互助請求（ROADMAP 125）：廣播世界聊天 + NpcSpeech 泡泡。
    HelpRequested {
        resident_id: String,
        resident_name: &'static str,
        x: f32,
        y: f32,
        text: String,
    },
    /// 居民快樂值首次突破 HAPPY_THRESHOLD（ROADMAP 126）：發出世界聊天廣播。
    HappinessBoost { name: &'static str, msg: String },
    /// 快樂居民主動招待附近玩家（ROADMAP 127）：給乙太小禮 + NpcSpeech 泡泡 + 世界聊天。
    PlayerGift {
        resident_id: String,
        resident_name: &'static str,
        x: f32,
        y: f32,
        player_name: String,
        text: String,
        gift_seed: usize,
    },
    /// 城鎮繁榮等級改變（ROADMAP 128）：由居民平均快樂值計算，等級升降時廣播。
    ProsperityChanged {
        /// 舊等級 u8（0-3）。
        old_level: u8,
        /// 新等級 u8（0-3）。
        new_level: u8,
        /// 廣播文字。
        msg: String,
    },
    /// 居民野外採集隊出發（ROADMAP 177）。
    ExpeditionStarted {
        names: Vec<&'static str>,
        target_name: String,
        target_x: f32,
        target_y: f32,
        msg: String,
    },
    /// 居民野外採集隊成功歸來（ROADMAP 177）。
    ExpeditionSuccess {
        names: Vec<&'static str>,
        msg: String,
    },
    /// 居民野外採集隊失敗（成員死亡）（ROADMAP 177）。
    ExpeditionFailed {
        names: Vec<&'static str>,
        msg: String,
    },
}

/// 故鄉城鎮閒晃邊界（像素）。
const WANDER_X_MIN: f32 = 1900.0;
const WANDER_X_MAX: f32 = 3100.0;
const WANDER_Y_MIN: f32 = 1850.0;
const WANDER_Y_MAX: f32 = 3050.0;

/// 居民移動速度（像素/秒）。比玩家慢、比有排程 NPC 慢，看起來悠閒。
const MOVE_SPEED: f32 = 40.0;

/// 到達目標附近後，再等幾秒才換新目標（在附近閒站）。
const WAIT_SECS_MIN: f32 = 3.0;
const WAIT_SECS_MAX: f32 = 12.0;

/// 抵達目標的判定半徑（像素）。
const ARRIVE_DIST: f32 = 8.0;

/// 城鎮中心座標（居民漫遊區正中央），採集隊出發/歸返的基準點（ROADMAP 177）。
const TOWN_CENTER_X: f32 = 2500.0;
const TOWN_CENTER_Y: f32 = 2500.0;

// ── 生態守望（ROADMAP 180）──────────────────────────────────────────────────
/// 生態壓力 ≥ 此值 → 城鎮進入避難警戒（居民奔回城中心廣場聚集）。
const ECO_ALARM_PRESSURE: f32 = 75.0;
/// 生態壓力 < 此值 → 解除警戒、回歸日常（與武裝值拉開遲滯，避免在閾值附近抖動）。
const ECO_CALM_PRESSURE: f32 = 45.0;
/// 避難時的急行軍速度倍率（比平時快，營造「趕緊跑回去」的張力）。
const ALARM_SPEED_MULT: f32 = 1.35;
/// 居民環繞城中心廣場聚集的基準半徑（px）。
const HUDDLE_RADIUS: f32 = 64.0;

/// 凱旋歡慶持續秒數（ROADMAP 185）：菁英 Alpha 被討伐後，城鎮居民原地歡慶的時長。
const CELEBRATE_DURATION_SECS: f32 = 8.0;

/// 凱旋餘韻持續秒數（ROADMAP 186）：歡慶（185）結束後，城鎮居民仍會在這段時間裡
/// 興奮地談論剛剛那場勝利——思想泡泡改冒「勝利談資」，把野外戰局接進居民的日常對話。
const TRIUMPH_AFTERGLOW_SECS: f32 = 40.0;

/// 依索引把居民確定性地散佈在城中心廣場周圍，避免全部疊在同一點。
/// 用黃金角分布 + 內外兩圈交錯，讓聚集看起來自然成團。
fn huddle_spot(index: usize) -> (f32, f32) {
    let ang = index as f32 * 2.399963; // 黃金角（弧度）≈ 137.5°
    let ring = if index % 2 == 0 { HUDDLE_RADIUS } else { HUDDLE_RADIUS * 0.55 };
    (TOWN_CENTER_X + ang.cos() * ring, TOWN_CENTER_Y + ang.sin() * ring)
}

// ── 思想泡泡計時器常數（ROADMAP 118）──────────────────────────────────────────
/// 思想泡泡最短間隔（秒）。
pub const THOUGHT_TIMER_MIN: f32 = 120.0;
/// 思想泡泡最長間隔（秒）。
pub const THOUGHT_TIMER_MAX: f32 = 300.0;
/// 居民互動距離（像素）：玩家在此範圍內才可搭話。
pub const RESIDENT_REACH: f32 = 80.0;

// ── 工作動態廣播計時器常數（ROADMAP 120）──────────────────────────────────────
/// 工作動態廣播最短間隔（秒）= 10 分鐘。
pub const WORK_TIMER_MIN: f32 = 600.0;
/// 工作動態廣播最長間隔（秒）= 20 分鐘。
pub const WORK_TIMER_MAX: f32 = 1200.0;

// ── 互助請求常數（ROADMAP 125）────────────────────────────────────────────────
/// 互助請求最短觸發間隔（秒）= 12 分鐘。
pub const HELP_REQUEST_TIMER_MIN: f32 = 720.0;
/// 互助請求最長觸發間隔（秒）= 20 分鐘。
pub const HELP_REQUEST_TIMER_MAX: f32 = 1200.0;
/// 互助請求持續時間（秒）= 8 分鐘，到時自動消除。
pub const HELP_REQUEST_DURATION_SECS: f32 = 480.0;
/// 玩家協助居民後獲得的乙太獎勵。
pub const HELP_REWARD_ETHER: u32 = 8;

// ── 快樂小回饋常數（ROADMAP 127）──────────────────────────────────────────────
/// 快樂居民招待計時器最短間隔（秒）= 15 分鐘。
pub const GIFT_TIMER_MIN: f32 = 900.0;
/// 快樂居民招待計時器最長間隔（秒）= 25 分鐘。
pub const GIFT_TIMER_MAX: f32 = 1500.0;
/// 快樂居民招待玩家的乙太小禮金額。
pub const GIFT_ETHER: u32 = 5;
/// 招待觸發距離（像素）：玩家在此範圍內才能收到禮。
pub const GIFT_DIST_PX: f32 = 80.0;

// ── 心情溫度常數（ROADMAP 126）────────────────────────────────────────────────
/// 快樂值初始值。
pub const HAPPINESS_INITIAL: u8 = 50;
/// 快樂值每次幫助後增加量。
pub const HAPPINESS_HELP_GAIN: u8 = 20;
/// 快樂值下限（自然衰減不低於此值）。
pub const HAPPINESS_MIN: u8 = 20;
/// 快樂值自然衰減間隔（秒）= 5 分鐘。
pub const HAPPINESS_DECAY_INTERVAL: f32 = 300.0;
/// 快樂值每次自然衰減量。
pub const HAPPINESS_DECAY_AMOUNT: u8 = 3;
/// 快樂態閾值：happiness >= 此值視為「快樂」，廣播更溫暖語氣。
pub const HAPPINESS_HAPPY_THRESHOLD: u8 = 70;

// ── 鄰里互動常數（ROADMAP 121）────────────────────────────────────────────────
/// 相遇觸發距離（像素）：兩位居民距離 ≤ 此值才能觸發打招呼。
pub const MEET_DIST: f32 = 60.0;
/// 互動期間停止移動的秒數。
pub const CHAT_DURATION: f32 = 8.0;
/// 每位居民每次互動後的冷卻時間（秒）= 3 分鐘。
pub const CHAT_COOLDOWN: f32 = 180.0;

// ── 居民隨機小事件計時器常數（ROADMAP 122）────────────────────────────────────
/// 小事件最短間隔（秒）= 20 分鐘。
pub const MINI_EVENT_TIMER_MIN: f32 = 1200.0;
/// 居民主動搭話冷卻計時（秒，ROADMAP 123）——每位居民 10 分鐘內只搭話一次。
pub const GREETING_COOLDOWN_SECS: f32 = 600.0;
/// 居民搭話觸發距離（像素）：玩家進入此範圍內才主動打招呼。
pub const GREETING_DIST_PX: f32 = 120.0;
/// 小事件最長間隔（秒）= 40 分鐘。
pub const MINI_EVENT_TIMER_MAX: f32 = 2400.0;

/// 人口下限：世界最冷清時至少這麼多人。
pub const MIN_POPULATION: usize = 4;

/// 人口上限：繁榮到頂時至多這麼多人。
pub const MAX_POPULATION: usize = 12;

/// 繁榮感觸發「新增居民」的閾值（所有 NPC 繁榮感平均 > 此值）。
const GROW_THRESHOLD: i32 = 60;

/// 繁榮感觸發「移除居民」的閾值（所有 NPC 繁榮感平均 < 此值）。
const SHRINK_THRESHOLD: i32 = 30;

/// 人口檢查週期（秒）。
pub const POPULATION_CHECK_SECS: f32 = 300.0;

/// 居民名字池——純在地化字串，不接 LLM。
/// 命名走「蒸汽龐克太空歌劇」世界觀：柔和的轉寫式名字，與主要 NPC（薇拉／萊拉／泰雅…）風格一致，
/// 帶一點星海／療癒感，不用鄉土俗名（舊池的狗蛋／二寶／大牛太跳戲，玩家回報）。
static NAME_POOL: &[&str] = &[
    "露娜", "諾娃", "賽勒", "奧瑞", "薇朵",
    "緹雅", "妮婭", "蕾娜", "米拉", "賽琳",
    "柯文", "葛瑞", "倫斯", "費歐", "凱倫",
    "黛西", "茉伊", "喬伊", "瑟拉", "薩米",
];

/// 居民行為類型（決定在哪一帶閒晃）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidentPersona {
    /// 四處遊走，整個城鎮都去。
    Wanderer,
    /// 在市場攤區附近逛。
    MarketBrowser,
    /// 在農田地帶附近勞動。
    FarmWorker,
    /// 主要停留在廣場 / 鎮中心。
    TownSquare,
}

impl ResidentPersona {
    /// 依 persona 決定閒晃的 x/y 邊界（白天各司其職，可部分重疊形成自然人流）。
    fn wander_bounds(&self) -> (f32, f32, f32, f32) {
        match self {
            // 市場攤區：npc_schedule 商人附近 (2000~2700, 1950~2450)
            ResidentPersona::MarketBrowser => (2000.0, 2700.0, 1950.0, 2450.0),
            // 農田區：公共農地偏南 (1900~2600, 2300~3050)
            ResidentPersona::FarmWorker    => (1900.0, 2600.0, 2300.0, 3050.0),
            // 廣場：鎮中心 (2200~2800, 2000~2600)
            ResidentPersona::TownSquare    => (2200.0, 2800.0, 2000.0, 2600.0),
            // 全城亂逛
            ResidentPersona::Wanderer      => (WANDER_X_MIN, WANDER_X_MAX, WANDER_Y_MIN, WANDER_Y_MAX),
        }
    }

    /// 依時段決定閒晃邊界（ROADMAP 119）。
    /// 黎明 → 市集聚攏；白天 → 各司其職（persona 本身邊界）；
    /// 黃昏 → 廣場聚集；夜晚 → 南邊住宅區休息。
    pub fn wander_bounds_for_phase(&self, phase: Phase) -> (f32, f32, f32, f32) {
        match phase {
            Phase::Dawn  => (2000.0, 2700.0, 1900.0, 2500.0), // 黎明：往市集方向聚攏
            Phase::Day   => self.wander_bounds(),               // 白天：各司其職
            Phase::Dusk  => (2200.0, 2800.0, 2000.0, 2600.0), // 黃昏：廣場聚集
            Phase::Night => (1950.0, 2650.0, 2400.0, 3050.0), // 夜晚：往南邊住宅區走去
        }
    }

    /// 依居民 index 分配 persona（讓城鎮人流分佈自然）。
    fn for_index(i: usize) -> Self {
        match i % 4 {
            0 => ResidentPersona::MarketBrowser,
            1 => ResidentPersona::FarmWorker,
            2 => ResidentPersona::TownSquare,
            _ => ResidentPersona::Wanderer,
        }
    }
}

/// 單一居民的完整運行狀態。
#[derive(Debug, Clone)]
pub struct ResidentNpc {
    /// 系統 id（"resident_0"、"resident_1"……）。
    pub id: String,
    /// 顯示名（從 NAME_POOL 取）。
    pub name: &'static str,
    pub persona: ResidentPersona,
    pub x: f32,
    pub y: f32,
    /// 當前行走目標。
    target_x: f32,
    target_y: f32,
    /// 到達目標後的等待秒數。> 0 = 在等、 <= 0 = 可選下個目標。
    wait_timer: f32,
    // ── 生命週期（ROADMAP 116）──────────────────────────
    /// 已活的秒數（真實時間）。
    pub age_secs: f32,
    /// 這一生的壽命（秒）。
    pub lifespan_secs: f32,
    /// 退休公告是否已發送（防重複廣播）。
    retirement_announced: bool,
    // ── 思想泡泡（ROADMAP 118）──────────────────────────
    /// 下次思想泡泡倒數計時（秒）。
    pub thought_timer: f32,
    /// 思想計數（用於模板種子，每次發射遞增）。
    pub thought_count: usize,
    // ── 作息時段（ROADMAP 119）──────────────────────────
    /// 目前所在時段；None 表示剛初始化，下一 tick 一定更新。
    current_phase: Option<Phase>,
    // ── 工作動態廣播（ROADMAP 120）──────────────────────
    /// 下次工作動態廣播倒數計時（秒）。
    work_timer: f32,
    /// 工作廣播計數（供模板種子輪替）。
    work_broadcast_count: usize,
    // ── 鄰里互動（ROADMAP 121）──────────────────────────
    /// 正在與鄰居打招呼的剩餘秒數（> 0 = 停止移動）。
    pub chat_remaining: f32,
    /// 上次互動結束後的冷卻秒數（> 0 = 不可再觸發）。
    pub chat_cooldown: f32,
    // ── 隨機小事件（ROADMAP 122）────────────────────────
    /// 下次小事件廣播倒數計時（秒）。
    mini_event_timer: f32,
    /// 小事件計數（供模板種子輪替）。
    mini_event_seed: usize,
    // ── 主動搭話（ROADMAP 123）──────────────────────────
    /// 主動搭話冷卻剩餘秒數（> 0 = 冷卻中，不觸發新搭話）。
    pub greeting_cooldown: f32,
    /// 搭話計數（供模板種子輪替）。
    greeting_seed: usize,
    // ── 互助請求（ROADMAP 125）──────────────────────────
    /// 距下次觸發互助請求的倒數計時（秒）。
    help_request_timer: f32,
    /// 目前是否有活躍的互助請求（等待玩家協助）。
    pub is_requesting_help: bool,
    /// 互助請求剩餘有效秒數（到 0 自動消除）。
    help_active_timer: f32,
    /// 互助請求模板種子（每次請求遞增）。
    help_request_seed: usize,
    // ── 心情溫度（ROADMAP 126）──────────────────────────
    /// 快樂值（0-100）；玩家幫助 +20、自然衰減 -3/5min，下限 20。
    pub happiness: u8,
    /// 快樂衰減倒數計時（秒）；到 0 扣 HAPPINESS_DECAY_AMOUNT，重設為 HAPPINESS_DECAY_INTERVAL。
    happiness_decay_timer: f32,
    // ── 快樂小回饋（ROADMAP 127）────────────────────────
    /// 招待計時器倒數（秒）；到 0 且快樂且有玩家在附近則觸發禮物。
    gift_timer: f32,
    /// 禮物種子（供模板輪替）。
    gift_seed: usize,
    // ── 野外採集隊（ROADMAP 177）────────────────────────
    /// 目前血量。
    pub hp: f32,
    /// 最大血量。
    pub max_hp: f32,
    /// 野外採集隊狀態。
    pub expedition: Option<ExpeditionState>,
    // ── 生態守望（ROADMAP 180）────────────────────────
    /// 是否正處於生態危機避難狀態（奔回廣場、頭頂 😰）。由 ResidentManager 每幀依城鎮整體警戒設定。
    pub alarmed: bool,
    // ── 凱旋歡慶（ROADMAP 185）────────────────────────
    /// 是否正處於菁英 Alpha 被討伐的歡慶狀態（原地雀躍、頭頂 🎉）。由 ResidentManager 每幀依城鎮歡慶計時設定。
    pub celebrating: bool,
}

/// 居民野外採集隊狀態（ROADMAP 177）。
#[derive(Debug, Clone, PartialEq)]
pub struct ExpeditionState {
    pub target_x: f32,
    pub target_y: f32,
    pub phase: ExpeditionPhase,
    /// 在目標點採樣的剩餘秒數。
    pub stay_timer: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExpeditionPhase {
    /// 前往目標。
    To,
    /// 抵達目標正在採樣。
    At,
    /// 採樣完成返回城鎮。
    From,
}

impl ResidentNpc {
    /// 用確定性 seed（依 index）初始化，保證每次重啟位置稍有不同但可預期。
    fn new(index: usize, rng: &mut impl Rng) -> Self {
        let persona = ResidentPersona::for_index(index);
        let (x0, x1, y0, y1) = persona.wander_bounds();
        let x = rng.gen_range(x0..=x1);
        let y = rng.gen_range(y0..=y1);
        let tx = rng.gen_range(x0..=x1);
        let ty = rng.gen_range(y0..=y1);
        let name = NAME_POOL[index % NAME_POOL.len()];
        // 錯開初始計時器，避免所有居民同時噴泡泡或廣播。
        let thought_offset = rng.gen_range(0.0..THOUGHT_TIMER_MAX);
        let work_offset = rng.gen_range(0.0..WORK_TIMER_MAX);
        let mini_offset = rng.gen_range(0.0..MINI_EVENT_TIMER_MAX);
        // 搭話冷卻錯開，避免多位居民在玩家登入瞬間同時打招呼。
        let greeting_offset = rng.gen_range(0.0..GREETING_COOLDOWN_SECS);
        // 互助請求計時錯開，避免所有居民同時廣播求助。
        let help_offset = rng.gen_range(0.0..HELP_REQUEST_TIMER_MAX);
        // 招待計時錯開，避免多位居民同時送禮。
        let gift_offset = rng.gen_range(0.0..GIFT_TIMER_MAX);
        Self {
            id: format!("resident_{}", index),
            name,
            persona,
            x,
            y,
            target_x: tx,
            target_y: ty,
            wait_timer: rng.gen_range(0.0..WAIT_SECS_MAX),
            age_secs: 0.0,
            lifespan_secs: resident_lifespan_secs(),
            retirement_announced: false,
            thought_timer: THOUGHT_TIMER_MIN + thought_offset,
            thought_count: index,
            current_phase: None,
            work_timer: WORK_TIMER_MIN + work_offset,
            work_broadcast_count: index,
            chat_remaining: 0.0,
            chat_cooldown: 0.0,
            mini_event_timer: MINI_EVENT_TIMER_MIN + mini_offset,
            mini_event_seed: index,
            greeting_cooldown: greeting_offset,
            greeting_seed: index,
            help_request_timer: HELP_REQUEST_TIMER_MIN + help_offset,
            is_requesting_help: false,
            help_active_timer: 0.0,
            help_request_seed: index,
            happiness: HAPPINESS_INITIAL,
            happiness_decay_timer: HAPPINESS_DECAY_INTERVAL,
            gift_timer: GIFT_TIMER_MIN + gift_offset,
            gift_seed: index,
            hp: 20.0,
            max_hp: 20.0,
            expedition: None,
            alarmed: false,
            celebrating: false,
        }
    }

    /// 壽命是否到了（應回歸乙太）。
    fn should_retire(&self) -> bool {
        self.age_secs >= self.lifespan_secs
    }

    /// 退休公告是否應發送（90% 壽命 且尚未發送）。
    fn should_announce_retirement(&self) -> bool {
        !self.retirement_announced
            && self.age_secs >= self.lifespan_secs * RESIDENT_RETIREMENT_FRACTION
    }

    /// 每幀推進：移動 + 等待計時。若時段切換則立即換新目標（ROADMAP 119）。
    fn tick(&mut self, dt: f32, rng: &mut impl Rng, phase: Phase, alarmed: bool, celebrating: bool, index: usize) {
        // ── 野外採集隊邏輯（ROADMAP 177）──
        // 採集隊已在野外執行任務，優先於避難（不會半途折返廣場），故不標記 alarmed。
        if let Some(ref mut ex) = self.expedition {
            match ex.phase {
                ExpeditionPhase::To => {
                    let dx = ex.target_x - self.x;
                    let dy = ex.target_y - self.y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist < ARRIVE_DIST {
                        ex.phase = ExpeditionPhase::At;
                        ex.stay_timer = 120.0; // 在目標點停留 2 分鐘採樣
                    } else {
                        let step = (MOVE_SPEED * 0.8 * dt).min(dist);
                        self.x += dx / dist * step;
                        self.y += dy / dist * step;
                    }
                }
                ExpeditionPhase::At => {
                    ex.stay_timer -= dt;
                    if ex.stay_timer <= 0.0 {
                        ex.phase = ExpeditionPhase::From;
                    }
                }
                ExpeditionPhase::From => {
                    let dx = TOWN_CENTER_X - self.x; // 城中心座標
                    let dy = TOWN_CENTER_Y - self.y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist >= ARRIVE_DIST {
                        let step = (MOVE_SPEED * 0.8 * dt).min(dist);
                        self.x += dx / dist * step;
                        self.y += dy / dist * step;
                    }
                    // 到達後由 ResidentManager::tick 判定成功並清除狀態
                }
            }
            return; // 採集隊期間不執行日常閒晃
        }

        // ── 生態守望避難（ROADMAP 180）──
        // 城鎮整體警戒時，居民放下手邊事、急行軍奔回城中心廣場確定性散佈點聚集，
        // 抵達後原地待避；危機期間不閒晃、不互動（互動類事件由 ResidentManager 統一抑制）。
        // 避難與歡慶旗標一律先更新（在任何提早 return 之前），避免狀態殘留導致前端泡泡顯示錯亂。
        // 互斥由 ResidentManager 保證：避難優先，故同幀不會同時為 true。
        self.alarmed = alarmed;
        self.celebrating = celebrating;
        if alarmed {
            let (hx, hy) = huddle_spot(index);
            let dx = hx - self.x;
            let dy = hy - self.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist >= ARRIVE_DIST {
                let step = (MOVE_SPEED * ALARM_SPEED_MULT * dt).min(dist);
                self.x += dx / dist * step;
                self.y += dy / dist * step;
            }
            return;
        }

        // ── 凱旋歡慶（ROADMAP 185）──
        // 菁英 Alpha 被討伐後，居民放下手邊事、原地雀躍歡呼（不移動），頭頂浮現 🎉。
        // 歡慶期間不閒晃、不互動（互動類事件由 ResidentManager 統一抑制），讓歡慶氣氛集中而鮮明。
        if celebrating {
            return;
        }

        // 鄰里互動冷卻計時（ROADMAP 121）
        if self.chat_cooldown > 0.0 {
            self.chat_cooldown -= dt;
        }
        // 主動搭話冷卻計時（ROADMAP 123）
        if self.greeting_cooldown > 0.0 {
            self.greeting_cooldown -= dt;
        }
        // 快樂衰減計時（ROADMAP 126）
        self.happiness_decay_timer -= dt;
        if self.happiness_decay_timer <= 0.0 {
            self.happiness = self.happiness.saturating_sub(HAPPINESS_DECAY_AMOUNT).max(HAPPINESS_MIN);
            self.happiness_decay_timer = HAPPINESS_DECAY_INTERVAL;
        }
        // 正在打招呼：停止移動，等計時結束
        if self.chat_remaining > 0.0 {
            self.chat_remaining -= dt;
            return;
        }
        // 時段切換：馬上給新目標、清除等待，居民立刻朝新區域走
        if self.current_phase != Some(phase) {
            self.current_phase = Some(phase);
            let (x0, x1, y0, y1) = self.persona.wander_bounds_for_phase(phase);
            self.target_x = rng.gen_range(x0..=x1);
            self.target_y = rng.gen_range(y0..=y1);
            self.wait_timer = 0.0;
        }
        if self.wait_timer > 0.0 {
            self.wait_timer -= dt;
            return;
        }
        let dx = self.target_x - self.x;
        let dy = self.target_y - self.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < ARRIVE_DIST {
            // 到了，等一下再換同時段內的下一個目標
            self.wait_timer = rng.gen_range(WAIT_SECS_MIN..=WAIT_SECS_MAX);
            let (x0, x1, y0, y1) = self.persona.wander_bounds_for_phase(phase);
            self.target_x = rng.gen_range(x0..=x1);
            self.target_y = rng.gen_range(y0..=y1);
        } else {
            let step = (MOVE_SPEED * dt).min(dist);
            self.x += dx / dist * step;
            self.y += dy / dist * step;
        }
    }
}

/// 居民群體管理器。
pub struct ResidentManager {
    pub residents: Vec<ResidentNpc>,
    /// 人口檢查計時器。
    population_timer: f32,
    /// 下一個新居民的 index（只增不減，保證 id 唯一）。
    next_index: usize,
    /// 隨機源（種子固定，重啟後走同一條路但不重要）。
    rng: StdRng,
    /// 上一次偵測到的時段（ROADMAP 119），用於偵測時段切換並廣播公告。
    current_phase: Option<Phase>,
    /// 上一次廣播時的繁榮等級（ROADMAP 128），避免重複廣播。初始 1（平靜）。
    last_prosperity_level: u8,
    /// 野外採集隊冷卻時間（秒）（ROADMAP 177）。
    pub expedition_cooldown: f32,
    /// 城鎮整體是否處於生態危機避難警戒（ROADMAP 180）；遲滯切換，避免在閾值附近抖動。
    town_alarmed: bool,
    /// 凱旋歡慶剩餘秒數（ROADMAP 185）；> 0 且未在避難時，城鎮居民原地歡慶（頭頂 🎉）。由 notify_hero_triumph 點燃。
    celebrate_timer: f32,
    /// 凱旋餘韻剩餘秒數（ROADMAP 186）；含歡慶期在內一併倒數，歡慶結束後仍 > 0 的這段即「餘韻期」，
    /// 居民思想泡泡改冒勝利談資。由 notify_hero_triumph 與 celebrate_timer 一同點燃。
    afterglow_timer: f32,
}

impl ResidentManager {
    /// 從最小人口出發建立管理器。
    pub fn new() -> Self {
        let mut rng = StdRng::seed_from_u64(42);
        let residents: Vec<ResidentNpc> = (0..MIN_POPULATION)
            .map(|i| ResidentNpc::new(i, &mut rng))
            .collect();
        Self {
            next_index: residents.len(),
            residents,
            population_timer: POPULATION_CHECK_SECS,
            rng,
            current_phase: None,
            last_prosperity_level: 1, // 初始視為平靜，避免啟動立即廣播
            expedition_cooldown: 300.0, // 初始 5 分鐘冷卻，給世界暖機時間
            town_alarmed: false,
            celebrate_timer: 0.0,
            afterglow_timer: 0.0,
        }
    }

    /// ROADMAP 185：菁英 Alpha（覺醒／霸主）被討伐時由 ws.rs 呼叫，點燃城鎮凱旋歡慶。
    /// 城鎮仍處生態避難警戒（180）時不歡慶——危機未解、避難優先，回傳 0（連捷報都不發）；
    /// 否則點亮歡慶計時並回傳將參與歡慶的在城居民數（採集隊在外者除外），供廣播判斷是否有人慶賀。
    pub fn notify_hero_triumph(&mut self) -> usize {
        if self.town_alarmed {
            return 0;
        }
        self.celebrate_timer = CELEBRATE_DURATION_SECS;
        // ROADMAP 186：餘韻含歡慶期一併倒數，歡慶結束後留下 TRIUMPH_AFTERGLOW_SECS 的勝利談資餘韻。
        self.afterglow_timer = CELEBRATE_DURATION_SECS + TRIUMPH_AFTERGLOW_SECS;
        self.residents.iter().filter(|r| r.expedition.is_none()).count()
    }

    /// 每幀推進：移動所有居民 + 生命週期 + 人口增減 + 思想泡泡計時。
    /// 回傳 (lifecycle_events, thought_events)，供 game.rs 廣播。
    pub fn tick(&mut self, dt: f32, avg_prosperity: i32, phase: Phase, player_positions: &[(String, f32, f32)], eco_pressure: f32) -> (Vec<ResidentLifecycleEvent>, Vec<ResidentThoughtEvent>) {
        let mut events = Vec::new();
        let mut thoughts = Vec::new();

        // ── 生態守望（ROADMAP 180）：城鎮對野外生態危機的整體反應 ──
        // 遲滯切換：壓力衝上 ECO_ALARM_PRESSURE 才進入警戒，跌破較低的 ECO_CALM_PRESSURE 才解除，
        // 避免在單一閾值附近反覆觸發/解除造成廣播洗版與居民來回奔波。
        let was_alarmed = self.town_alarmed;
        if !self.town_alarmed && eco_pressure >= ECO_ALARM_PRESSURE {
            self.town_alarmed = true;
        } else if self.town_alarmed && eco_pressure < ECO_CALM_PRESSURE {
            self.town_alarmed = false;
        }
        if self.town_alarmed && !was_alarmed {
            events.push(ResidentLifecycleEvent::EcoAlarm {
                msg: "😰【居民避難】野外生態驟然失衡，城鎮居民驚覺異動，紛紛趕回城中心廣場聚集互相依靠……".to_string(),
            });
        } else if !self.town_alarmed && was_alarmed {
            events.push(ResidentLifecycleEvent::EcoCalm {
                msg: "😌【居民安心】野外漸歸平靜，居民們鬆了一口氣，三三兩兩散回各自的營生。".to_string(),
            });
        }
        let alarmed = self.town_alarmed;

        // ── 凱旋歡慶（ROADMAP 185）：菁英 Alpha 被討伐後城鎮短暫歡慶 ──
        // 避難警戒優先（危機未解不慶祝）：歡慶計時雖在跑，只要城鎮仍在避難就不歡慶，
        // 兩者互斥，避免「一邊驚慌避難、一邊歡呼」的矛盾。
        if self.celebrate_timer > 0.0 {
            self.celebrate_timer = (self.celebrate_timer - dt).max(0.0);
        }
        let celebrating = self.celebrate_timer > 0.0 && !alarmed;

        // ── 凱旋餘韻談資（ROADMAP 186）：歡慶散場後，居民仍興奮地聊著剛斬下的菁英首領 ──
        // 餘韻計時含歡慶期一併倒數；歡慶結束（celebrating 為 false）後仍 > 0 的這段即「餘韻期」，
        // 期間冒出的思想泡泡改用勝利談資模板。避難優先（危機未解不閒聊），與歡慶同受 alarmed 壓制。
        if self.afterglow_timer > 0.0 {
            self.afterglow_timer = (self.afterglow_timer - dt).max(0.0);
        }
        let in_afterglow = self.afterglow_timer > 0.0 && !celebrating && !alarmed;

        // 0. 野外採集隊成敗判定（ROADMAP 177）
        let mut failed = false;
        let mut success = false;
        let mut expedition_names = Vec::new();
        let mut has_expedition = false;

        if self.expedition_cooldown > 0.0 {
            self.expedition_cooldown -= dt;
        }

        for r in &self.residents {
            if let Some(ref ex) = r.expedition {
                has_expedition = true;
                expedition_names.push(r.name);
                if r.hp <= 0.0 {
                    failed = true;
                } else if ex.phase == ExpeditionPhase::From {
                    let dx = TOWN_CENTER_X - r.x;
                    let dy = TOWN_CENTER_Y - r.y;
                    if (dx * dx + dy * dy).sqrt() < ARRIVE_DIST {
                        success = true;
                    }
                }
            }
        }

        if has_expedition {
            if failed {
                for r in &mut self.residents {
                    if r.expedition.is_some() {
                        r.expedition = None;
                        r.hp = r.max_hp;
                    }
                }
                events.push(ResidentLifecycleEvent::ExpeditionFailed {
                    names: expedition_names,
                    msg: "❌ 【採集隊撤退】成員體力耗盡，採集隊宣告失敗，物資全數遺失。".to_string(),
                });
            } else if success {
                // 簡化判定：所有隊員都回到城中心才算成功
                let all_back = self.residents.iter().filter(|r| r.expedition.is_some()).all(|r| {
                    if let Some(ref ex) = r.expedition {
                        if ex.phase != ExpeditionPhase::From { return false; }
                        let dx = TOWN_CENTER_X - r.x;
                        let dy = TOWN_CENTER_Y - r.y;
                        (dx * dx + dy * dy).sqrt() < ARRIVE_DIST
                    } else {
                        true
                    }
                });
                if all_back {
                    for r in &mut self.residents {
                        if r.expedition.is_some() {
                            r.expedition = None;
                            r.hp = r.max_hp;
                        }
                    }
                    events.push(ResidentLifecycleEvent::ExpeditionSuccess {
                        names: expedition_names,
                        msg: "🌿 【採集隊歸來】採集隊平安回到城鎮中心，帶回了珍貴的野外樣本！".to_string(),
                    });
                }
            }
        }

        // 時段切換偵測（ROADMAP 119）：廣播一條「居民換聚場」公告。
        if self.current_phase != Some(phase) {
            self.current_phase = Some(phase);
            events.push(ResidentLifecycleEvent::PhaseTransition {
                phase,
                msg: phase_transition_msg(phase),
            });
        }

        // 1. 推進每位居民的年齡、移動、思想計時
        for (i, r) in self.residents.iter_mut().enumerate() {
            r.age_secs += dt;
            r.tick(dt, &mut self.rng, phase, alarmed, celebrating, i);
            // 生態守望（ROADMAP 180）：避難期間居民只顧奔回廣場聚集；
            // 凱旋歡慶（ROADMAP 185）：歡慶期間居民只顧原地歡呼。
            // 兩者皆抑制日常演出（思想泡泡 / 工作動態 / 小事件 / 搭話 / 求助 / 送禮），讓當下氣氛集中鮮明。
            if alarmed || celebrating {
                continue;
            }
            // 思想泡泡計時（ROADMAP 118）
            r.thought_timer -= dt;
            if r.thought_timer <= 0.0 {
                thoughts.push(ResidentThoughtEvent {
                    id: r.id.clone(),
                    name: r.name,
                    persona: r.persona,
                    x: r.x,
                    y: r.y,
                    seed: r.thought_count,
                    // ROADMAP 186：餘韻期內冒出的思想泡泡聊的是剛斬下的菁英首領（勝利談資）。
                    triumph: in_afterglow,
                });
                r.thought_count += 1;
                r.thought_timer = self.rng.gen_range(THOUGHT_TIMER_MIN..=THOUGHT_TIMER_MAX);
            }
            // 工作動態廣播計時（ROADMAP 120 / 126）——居民在工作時段定期廣播活動，0 玩家也持續。
            // 快樂（ROADMAP 126）：happiness >= HAPPY_THRESHOLD 時使用更歡欣的模板。
            r.work_timer -= dt;
            if r.work_timer <= 0.0 {
                let text_opt = if r.happiness >= HAPPINESS_HAPPY_THRESHOLD {
                    Some(crate::resident_chat::get_happy_work_action(
                        r.persona, r.name, r.work_broadcast_count,
                    ))
                } else {
                    crate::resident_chat::get_work_action(
                        r.persona, phase, r.name, r.work_broadcast_count,
                    )
                };
                if let Some(text) = text_opt {
                    events.push(ResidentLifecycleEvent::WorkActivity { text });
                    r.work_broadcast_count += 1;
                }
                r.work_timer = self.rng.gen_range(WORK_TIMER_MIN..=WORK_TIMER_MAX);
            }
            // 隨機小事件計時（ROADMAP 122）——任何時段皆可觸發，廣播日常生活趣事。
            r.mini_event_timer -= dt;
            if r.mini_event_timer <= 0.0 {
                let text = crate::resident_chat::get_mini_event(
                    r.persona, r.name, r.mini_event_seed,
                );
                events.push(ResidentLifecycleEvent::MiniEvent { text });
                r.mini_event_seed += 1;
                r.mini_event_timer = self.rng.gen_range(MINI_EVENT_TIMER_MIN..=MINI_EVENT_TIMER_MAX);
            }
            // 主動向玩家打招呼（ROADMAP 123）——冷卻結束且玩家在附近時觸發。
            if r.greeting_cooldown <= 0.0 && r.chat_remaining <= 0.0 && !player_positions.is_empty() {
                if let Some((player_name, _, _)) = player_positions.iter().find(|(_, px, py)| {
                    let dx = r.x - px;
                    let dy = r.y - py;
                    (dx * dx + dy * dy).sqrt() <= GREETING_DIST_PX
                }) {
                    let text = crate::resident_chat::get_player_greeting(
                        r.persona, r.name, player_name, r.greeting_seed,
                    );
                    events.push(ResidentLifecycleEvent::PlayerGreeting {
                        resident_id: r.id.clone(),
                        resident_name: r.name,
                        x: r.x,
                        y: r.y,
                        player_name: player_name.clone(),
                        text,
                    });
                    r.greeting_cooldown = GREETING_COOLDOWN_SECS;
                    r.greeting_seed += 1;
                }
            }
            // 互助請求（ROADMAP 125）——計時到期後廣播求助，持續 HELP_REQUEST_DURATION_SECS 秒。
            if !r.is_requesting_help {
                r.help_request_timer -= dt;
                if r.help_request_timer <= 0.0 {
                    r.is_requesting_help = true;
                    r.help_active_timer = HELP_REQUEST_DURATION_SECS;
                    let text = crate::resident_chat::get_help_request(
                        r.persona, r.name, r.help_request_seed,
                    );
                    events.push(ResidentLifecycleEvent::HelpRequested {
                        resident_id: r.id.clone(),
                        resident_name: r.name,
                        x: r.x,
                        y: r.y,
                        text,
                    });
                }
            } else {
                r.help_active_timer -= dt;
                if r.help_active_timer <= 0.0 {
                    // 無人幫忙，請求自動過期
                    r.is_requesting_help = false;
                    r.help_request_timer = self.rng.gen_range(HELP_REQUEST_TIMER_MIN..=HELP_REQUEST_TIMER_MAX);
                }
            }
            // 快樂小回饋（ROADMAP 127）——快樂居民計時到期且有玩家在附近時，主動招待一位玩家。
            r.gift_timer -= dt;
            if r.gift_timer <= 0.0 {
                r.gift_timer = self.rng.gen_range(GIFT_TIMER_MIN..=GIFT_TIMER_MAX);
                // 只在快樂狀態下觸發
                if r.happiness >= HAPPINESS_HAPPY_THRESHOLD {
                    if let Some((player_name, _, _)) = player_positions.iter().find(|(_, px, py)| {
                        let dx = r.x - px;
                        let dy = r.y - py;
                        (dx * dx + dy * dy).sqrt() <= GIFT_DIST_PX
                    }) {
                        let text = crate::resident_chat::get_gift_message(
                            r.persona, r.name, player_name, r.gift_seed,
                        );
                        events.push(ResidentLifecycleEvent::PlayerGift {
                            resident_id: r.id.clone(),
                            resident_name: r.name,
                            x: r.x,
                            y: r.y,
                            player_name: player_name.clone(),
                            text,
                            gift_seed: r.gift_seed,
                        });
                        r.gift_seed += 1;
                    }
                }
            }
        }

        // 2. 鄰里互動檢查（ROADMAP 121）：兩位居民靠近時互相打招呼。
        // 生態守望（ROADMAP 180）：避難聚集時居民彼此緊鄰，若照常觸發鄰里寒暄會洗版，故暫停。
        if !alarmed {
            let neighbor_events = self.check_neighbor_interactions();
            events.extend(neighbor_events);
        }

        // 4. 退休公告（90% 壽命，防重複）
        for r in &mut self.residents {
            if r.should_announce_retirement() {
                r.retirement_announced = true;
                events.push(ResidentLifecycleEvent::RetirementSoon {
                    name: r.name,
                    msg: retirement_msg(r.name),
                });
            }
        }

        // 3. 壽命到期：回歸乙太 + 新居民遷入替補（每幀最多處理一位，防同時大量廣播）
        if let Some(pos) = self.residents.iter().position(|r| r.should_retire()) {
            let old = self.residents.remove(pos);
            let new_idx = self.next_index;
            self.next_index += 1;
            let new_r = ResidentNpc::new(new_idx, &mut self.rng);
            let farewell = farewell_msg(old.name);
            let arrival = arrival_from_predecessor_msg(new_r.name, old.name);
            events.push(ResidentLifecycleEvent::RetiredToEther {
                old_name: old.name,
                new_name: new_r.name,
                farewell_msg: farewell,
                arrival_msg: arrival,
            });
            self.residents.push(new_r);
        }

        // 5. 繁榮感驅動的人口增減（每 POPULATION_CHECK_SECS 秒一次）
        self.population_timer -= dt;
        if self.population_timer <= 0.0 {
            self.population_timer = POPULATION_CHECK_SECS;
            if avg_prosperity >= GROW_THRESHOLD && self.residents.len() < MAX_POPULATION {
                let idx = self.next_index;
                self.next_index += 1;
                let r = ResidentNpc::new(idx, &mut self.rng);
                events.push(ResidentLifecycleEvent::NewArrival {
                    name: r.name,
                    msg: new_arrival_msg(r.name),
                });
                self.residents.push(r);
            } else if avg_prosperity < SHRINK_THRESHOLD && self.residents.len() > MIN_POPULATION {
                if let Some(r) = self.residents.pop() {
                    events.push(ResidentLifecycleEvent::Departed {
                        name: r.name,
                        msg: departed_msg(r.name),
                    });
                }
            }
        }

        // 繁榮等級偵測（ROADMAP 128）：每幀算一次平均快樂值，等級改變才廣播。
        {
            use crate::town_prosperity::{prosperity_from_avg, prosperity_changed_msg};
            let avg = self.avg_happiness();
            let new_level = prosperity_from_avg(avg).as_u8();
            if new_level != self.last_prosperity_level {
                let old_level = self.last_prosperity_level;
                let old_lv = crate::town_prosperity::level_from_u8(old_level);
                let new_lv = crate::town_prosperity::level_from_u8(new_level);
                let msg = prosperity_changed_msg(old_lv, new_lv);
                self.last_prosperity_level = new_level;
                events.push(ResidentLifecycleEvent::ProsperityChanged { old_level, new_level, msg });
            }
        }

        (events, thoughts)
    }

    /// 回傳目前最年長居民的名字（供 AI NPC 收徒時點名使用）。
    pub fn oldest_resident_name(&self) -> Option<&'static str> {
        self.residents
            .iter()
            .max_by(|a, b| a.age_secs.partial_cmp(&b.age_secs).unwrap_or(std::cmp::Ordering::Equal))
            .map(|r| r.name)
    }

    /// 依 id 找居民，回傳 (persona, name, x, y)；找不到回 None。
    pub fn find_by_id(&self, resident_id: &str) -> Option<(ResidentPersona, &'static str, f32, f32)> {
        self.residents.iter()
            .find(|r| r.id == resident_id)
            .map(|r| (r.persona, r.name, r.x, r.y))
    }

    /// 掃描所有居民配對，對靠近且無冷卻的兩人觸發相遇打招呼（ROADMAP 121）。
    /// 每次最多觸發 2 組，避免瞬間大量廣播。
    fn check_neighbor_interactions(&mut self) -> Vec<ResidentLifecycleEvent> {
        let mut result = Vec::new();
        let n = self.residents.len();
        'outer: for i in 0..n {
            for j in (i + 1)..n {
                if self.residents[i].chat_remaining > 0.0
                    || self.residents[j].chat_remaining > 0.0
                    || self.residents[i].chat_cooldown > 0.0
                    || self.residents[j].chat_cooldown > 0.0
                {
                    continue;
                }
                let dx = self.residents[i].x - self.residents[j].x;
                let dy = self.residents[i].y - self.residents[j].y;
                if dx * dx + dy * dy > MEET_DIST * MEET_DIST {
                    continue;
                }
                // 提取所需資料（避免 borrow 衝突）
                let name_a = self.residents[i].name;
                let name_b = self.residents[j].name;
                let id_a   = self.residents[i].id.clone();
                let id_b   = self.residents[j].id.clone();
                let x_a    = self.residents[i].x;
                let y_a    = self.residents[i].y;
                let x_b    = self.residents[j].x;
                let y_b    = self.residents[j].y;
                let seed_a = self.residents[i].thought_count;
                let seed_b = self.residents[j].thought_count;
                let text_a = crate::resident_chat::get_neighbor_greet(name_b, seed_a);
                let text_b = crate::resident_chat::get_neighbor_reply(seed_b).to_string();
                // 設定互動狀態
                self.residents[i].chat_remaining = CHAT_DURATION;
                self.residents[i].chat_cooldown  = CHAT_COOLDOWN;
                self.residents[i].thought_count += 1;
                self.residents[j].chat_remaining = CHAT_DURATION;
                self.residents[j].chat_cooldown  = CHAT_COOLDOWN;
                self.residents[j].thought_count += 1;
                result.push(ResidentLifecycleEvent::NeighborChat {
                    id_a, name_a, text_a, x_a, y_a,
                    id_b, name_b, text_b, x_b, y_b,
                });
                if result.len() >= 2 { break 'outer; }
            }
        }
        result
    }

    /// 啟動野外採集隊（ROADMAP 177）。
    pub fn start_expedition(&mut self, target_name: String, tx: f32, ty: f32) -> Option<ResidentLifecycleEvent> {
        let mut candidates: Vec<usize> = self.residents.iter().enumerate()
            .filter(|(_, r)| r.expedition.is_none() && r.happiness >= 50)
            .map(|(i, _)| i)
            .collect();
        if candidates.is_empty() { return None; }

        candidates.shuffle(&mut self.rng);
        let count = if candidates.len() >= 2 { self.rng.gen_range(1..=2) } else { 1 };
        let selected = &candidates[..count];

        let mut names = Vec::new();
        for &idx in selected {
            let r = &mut self.residents[idx];
            r.expedition = Some(ExpeditionState {
                target_x: tx,
                target_y: ty,
                phase: ExpeditionPhase::To,
                stay_timer: 0.0,
            });
            r.hp = r.max_hp;
            names.push(r.name);
        }

        let msg = format!("🏹 【採集隊出發】{} 離開城鎮，前往 {} 進行野外採樣！", names.join("、"), target_name);
        Some(ResidentLifecycleEvent::ExpeditionStarted {
            names,
            target_name,
            target_x: tx,
            target_y: ty,
            msg,
        })
    }

    /// 目前採集隊的目標座標（ROADMAP 177）。
    pub fn expedition_target(&self) -> Option<(f32, f32)> {
        self.residents.iter()
            .find_map(|r| r.expedition.as_ref().map(|ex| (ex.target_x, ex.target_y)))
    }

    /// 回傳居民視圖清單：(id, name, x, y, is_expedition, hp_pct)。供快照廣播用。
    pub fn views(&self) -> impl Iterator<Item = (&str, &str, f32, f32, bool, Option<f32>, bool, bool)> {
        self.residents.iter().map(|r| (
            r.id.as_str(),
            r.name,
            r.x,
            r.y,
            r.expedition.is_some(),
            if r.expedition.is_some() { Some(r.hp / r.max_hp) } else { None },
            r.alarmed, // ROADMAP 180：生態危機避難中（前端顯示 😰）
            r.celebrating, // ROADMAP 185：菁英 Alpha 被討伐歡慶中（前端顯示 🎉）
        ))
    }

    /// 目前居民人數（供測試用）。
    pub fn population(&self) -> usize {
        self.residents.len()
    }

    /// 回傳目前有活躍互助請求的居民 id 清單（ROADMAP 125）。供快照廣播用。
    pub fn requesting_ids(&self) -> Vec<String> {
        self.residents.iter()
            .filter(|r| r.is_requesting_help)
            .map(|r| r.id.clone())
            .collect()
    }

    /// 查找正在求助的指定居民（ROADMAP 125），回傳 (persona, name, x, y)。
    /// 若居民不存在或未在求助中，回傳 None。
    pub fn find_requesting_by_id(&self, resident_id: &str) -> Option<(ResidentPersona, &'static str, f32, f32)> {
        self.residents.iter()
            .find(|r| r.id == resident_id && r.is_requesting_help)
            .map(|r| (r.persona, r.name, r.x, r.y))
    }

    /// 完成居民互助請求（ROADMAP 125 / 126）：清除 is_requesting_help，重設計時器；
    /// 同時為居民加快樂值，若首次突破 HAPPY_THRESHOLD 則回傳 HappinessBoost 事件。
    /// 回傳 (成功, Option<HappinessBoost事件>)。
    pub fn fulfill_help_request(&mut self, resident_id: &str) -> (bool, Option<ResidentLifecycleEvent>) {
        if let Some(r) = self.residents.iter_mut().find(|r| r.id == resident_id && r.is_requesting_help) {
            let old_happiness = r.happiness;
            r.is_requesting_help = false;
            r.help_active_timer = 0.0;
            r.help_request_timer = HELP_REQUEST_TIMER_MAX;
            r.help_request_seed += 1;
            r.happiness = (r.happiness as u16 + HAPPINESS_HELP_GAIN as u16).min(100) as u8;
            // 首次跨越快樂門檻時廣播（ROADMAP 126）
            let boost_event = if old_happiness < HAPPINESS_HAPPY_THRESHOLD
                && r.happiness >= HAPPINESS_HAPPY_THRESHOLD
            {
                let msg = crate::resident_chat::get_happiness_boost_chat(r.name);
                Some(ResidentLifecycleEvent::HappinessBoost { name: r.name, msg })
            } else {
                None
            };
            (true, boost_event)
        } else {
            (false, None)
        }
    }

    /// 回傳所有居民的 (id, happiness)，供快照廣播用（ROADMAP 126）。
    pub fn moods(&self) -> Vec<(String, u8)> {
        self.residents.iter().map(|r| (r.id.clone(), r.happiness)).collect()
    }

    /// 計算居民平均快樂值（無居民時回 50）。
    pub fn avg_happiness(&self) -> u8 {
        if self.residents.is_empty() {
            return 50;
        }
        let total: u32 = self.residents.iter().map(|r| r.happiness as u32).sum();
        (total / self.residents.len() as u32) as u8
    }

    /// 回傳目前快取的繁榮等級（0-3），用於快照廣播。
    pub fn prosperity_level(&self) -> u8 {
        self.last_prosperity_level
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_population_within_bounds() {
        let mgr = ResidentManager::new();
        assert!(mgr.population() >= MIN_POPULATION);
        assert!(mgr.population() <= MAX_POPULATION);
    }

    #[test]
    fn population_grows_when_prosperous() {
        let mut mgr = ResidentManager::new();
        let initial = mgr.population();
        // 直接觸發人口檢查（把計時器歸零）
        mgr.population_timer = 0.0;
        let (events, _) = mgr.tick(0.01, GROW_THRESHOLD + 1, Phase::Day, &[], 0.0);
        assert_eq!(mgr.population(), (initial + 1).min(MAX_POPULATION));
        // 繁榮帶來移民事件
        if initial < MAX_POPULATION {
            assert!(events.iter().any(|e| matches!(e, ResidentLifecycleEvent::NewArrival { .. })));
        }
    }

    #[test]
    fn population_shrinks_when_poor() {
        let mut mgr = ResidentManager::new();
        // 先讓人口超過最小值
        mgr.population_timer = 0.0;
        mgr.tick(0.01, GROW_THRESHOLD + 1, Phase::Day, &[], 0.0);
        mgr.population_timer = 0.0;
        mgr.tick(0.01, GROW_THRESHOLD + 1, Phase::Day, &[], 0.0);
        let before = mgr.population();
        mgr.population_timer = 0.0;
        let (events, _) = mgr.tick(0.01, SHRINK_THRESHOLD - 1, Phase::Day, &[], 0.0);
        assert_eq!(mgr.population(), (before - 1).max(MIN_POPULATION));
        if before > MIN_POPULATION {
            assert!(events.iter().any(|e| matches!(e, ResidentLifecycleEvent::Departed { .. })));
        }
    }

    #[test]
    fn population_never_below_min() {
        let mut mgr = ResidentManager::new();
        // 多次觸發衰退
        for _ in 0..20 {
            mgr.population_timer = 0.0;
            mgr.tick(0.01, 0, Phase::Day, &[], 0.0);
        }
        assert!(mgr.population() >= MIN_POPULATION);
    }

    #[test]
    fn population_never_above_max() {
        let mut mgr = ResidentManager::new();
        for _ in 0..20 {
            mgr.population_timer = 0.0;
            mgr.tick(0.01, 100, Phase::Day, &[], 0.0);
        }
        assert!(mgr.population() <= MAX_POPULATION);
    }

    #[test]
    fn residents_move_within_bounds() {
        let mut mgr = ResidentManager::new();
        // 跑 60 秒模擬
        for _ in 0..600 {
            mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        }
        for r in &mgr.residents {
            // 居民不該衝出全城大邊界
            assert!(r.x >= WANDER_X_MIN - 1.0 && r.x <= WANDER_X_MAX + 1.0,
                "x out of bounds: {}", r.x);
            assert!(r.y >= WANDER_Y_MIN - 1.0 && r.y <= WANDER_Y_MAX + 1.0,
                "y out of bounds: {}", r.y);
        }
    }

    #[test]
    fn retirement_announced_at_correct_fraction() {
        let mut mgr = ResidentManager::new();
        // 把第一位居民設到退休門檻前一步
        mgr.residents[0].lifespan_secs = 100.0;
        mgr.residents[0].age_secs = 88.0; // 89% < 90%
        // tick 一下，不應觸發
        let (ev, _) = mgr.tick(0.5, 50, Phase::Day, &[], 0.0);
        assert!(!ev.iter().any(|e| matches!(e, ResidentLifecycleEvent::RetirementSoon { .. })));
        // 再 tick 過 90%
        let (ev2, _) = mgr.tick(2.0, 50, Phase::Day, &[], 0.0);
        assert!(ev2.iter().any(|e| matches!(e, ResidentLifecycleEvent::RetirementSoon { .. })));
        // 已標記，再 tick 不重複
        let (ev3, _) = mgr.tick(0.5, 50, Phase::Day, &[], 0.0);
        assert!(!ev3.iter().any(|e| matches!(e, ResidentLifecycleEvent::RetirementSoon { .. })));
    }

    #[test]
    fn retired_to_ether_replaces_resident() {
        let mut mgr = ResidentManager::new();
        let before = mgr.population();
        let old_name = {
            mgr.residents[0].lifespan_secs = 100.0;
            mgr.residents[0].age_secs = 100.0; // 壽命到期
            mgr.residents[0].name
        };
        let (ev, _) = mgr.tick(0.01, 50, Phase::Day, &[], 0.0);
        // 人口不變（退休 + 新居民）
        assert_eq!(mgr.population(), before);
        // 事件應存在
        let retired = ev.iter().find(|e| matches!(e, ResidentLifecycleEvent::RetiredToEther { .. }));
        assert!(retired.is_some());
        if let Some(ResidentLifecycleEvent::RetiredToEther { old_name: on, new_name: nn, .. }) = retired {
            assert_eq!(*on, old_name);
            assert_ne!(*nn, old_name, "新居民應使用不同名字");
        }
    }

    #[test]
    fn oldest_resident_name_returns_most_aged() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].age_secs = 999.0;
        mgr.residents[1].age_secs = 1.0;
        let oldest = mgr.oldest_resident_name().unwrap();
        assert_eq!(oldest, mgr.residents[0].name);
    }

    // ── ROADMAP 119：作息時段測試 ─────────────────────────────────────────────

    #[test]
    fn dawn_zone_is_market_for_all_personas() {
        // 黎明：所有 persona 都往市集區（y 上半段）
        for p in [ResidentPersona::MarketBrowser, ResidentPersona::FarmWorker,
                  ResidentPersona::TownSquare, ResidentPersona::Wanderer] {
            let (_, _, y0, y1) = p.wander_bounds_for_phase(Phase::Dawn);
            assert!(y0 < 2500.0, "黎明 y0 應在市集上半段");
            assert!(y1 <= 2500.0, "黎明 y1 應在市集上半段");
        }
    }

    #[test]
    fn dusk_zone_is_square_for_all_personas() {
        // 黃昏：所有 persona 聚廣場（x 中段，y 中段）
        for p in [ResidentPersona::MarketBrowser, ResidentPersona::FarmWorker,
                  ResidentPersona::TownSquare, ResidentPersona::Wanderer] {
            let (x0, x1, y0, y1) = p.wander_bounds_for_phase(Phase::Dusk);
            assert!(x0 >= 2000.0 && x1 <= 3100.0, "黃昏 x 應在廣場範圍");
            assert!(y0 >= 1900.0 && y1 <= 2800.0, "黃昏 y 應在廣場範圍");
        }
    }

    #[test]
    fn night_zone_is_south_for_all_personas() {
        // 夜晚：所有 persona 往南邊住宅區（y 較大）
        for p in [ResidentPersona::MarketBrowser, ResidentPersona::FarmWorker,
                  ResidentPersona::TownSquare, ResidentPersona::Wanderer] {
            let (_, _, y0, _) = p.wander_bounds_for_phase(Phase::Night);
            assert!(y0 >= 2200.0, "夜晚 y0 應在南邊住宅區");
        }
    }

    #[test]
    fn day_zone_follows_persona() {
        // 白天：各 persona 回各自崗位
        let (_, _, _, market_y1) = ResidentPersona::MarketBrowser.wander_bounds_for_phase(Phase::Day);
        let (_, _, farm_y0, _)   = ResidentPersona::FarmWorker.wander_bounds_for_phase(Phase::Day);
        // 農田在市場南方
        assert!(farm_y0 > market_y1 - 200.0, "白天農田 y0 應比市場 y1 更南");
    }

    #[test]
    fn phase_transition_event_emitted_on_change() {
        let mut mgr = ResidentManager::new();
        // 初始 current_phase = None → 第一次 tick 任何時段都觸發轉換事件
        let (events, _) = mgr.tick(0.01, 50, Phase::Dawn, &[], 0.0);
        assert!(events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PhaseTransition { phase: Phase::Dawn, .. })));
        // 同一時段再 tick → 不重複觸發
        let (events2, _) = mgr.tick(0.01, 50, Phase::Dawn, &[], 0.0);
        assert!(!events2.iter().any(|e| matches!(e, ResidentLifecycleEvent::PhaseTransition { .. })));
    }

    #[test]
    fn phase_transition_changes_resident_targets() {
        let mut mgr = ResidentManager::new();
        // 先在白天跑幾秒讓居民穩定
        for _ in 0..10 { mgr.tick(0.5, 50, Phase::Day, &[], 0.0); }
        let old_targets: Vec<(f32, f32)> = mgr.residents.iter()
            .map(|r| (r.target_x, r.target_y))
            .collect();
        // 切換到黃昏
        mgr.tick(0.01, 50, Phase::Dusk, &[], 0.0);
        // 至少部分居民的目標應已改變（在新時段邊界內）
        let changed = mgr.residents.iter().zip(&old_targets)
            .filter(|(r, old)| (r.target_x - old.0).abs() > 1.0 || (r.target_y - old.1).abs() > 1.0)
            .count();
        assert!(changed > 0, "時段切換應讓至少一位居民更換目標");
    }

    // ── ROADMAP 120：工作動態廣播測試 ────────────────────────────────────────

    #[test]
    fn work_activity_fires_after_timer_expires() {
        let mut mgr = ResidentManager::new();
        // 把第一位居民的 work_timer 歸零，讓下一 tick 立刻觸發
        mgr.residents[0].work_timer = 0.0;
        let persona = mgr.residents[0].persona;
        // 確保是白天（所有 persona 白天都會廣播）
        let (events, _) = mgr.tick(0.01, 50, Phase::Day, &[], 0.0);
        let work_evs: Vec<_> = events.iter()
            .filter(|e| matches!(e, ResidentLifecycleEvent::WorkActivity { .. }))
            .collect();
        assert!(!work_evs.is_empty(), "work_timer=0 白天應觸發 WorkActivity（persona: {:?}）", persona);
    }

    #[test]
    fn work_activity_not_fired_at_night() {
        let mut mgr = ResidentManager::new();
        // 把所有居民 work_timer 清零
        for r in &mut mgr.residents { r.work_timer = 0.0; }
        let (events, _) = mgr.tick(0.01, 50, Phase::Night, &[], 0.0);
        let work_evs: usize = events.iter()
            .filter(|e| matches!(e, ResidentLifecycleEvent::WorkActivity { .. }))
            .count();
        assert_eq!(work_evs, 0, "夜間不應觸發工作動態廣播");
    }

    #[test]
    fn work_activity_text_contains_resident_name() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].work_timer = 0.0;
        let expected_name = mgr.residents[0].name;
        let (events, _) = mgr.tick(0.01, 50, Phase::Day, &[], 0.0);
        for ev in &events {
            if let ResidentLifecycleEvent::WorkActivity { text } = ev {
                assert!(
                    text.contains(expected_name),
                    "工作廣播文字應包含居民名字「{expected_name}」，實際：{text}"
                );
            }
        }
    }

    #[test]
    fn work_timer_resets_after_firing() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].work_timer = 0.0;
        mgr.tick(0.01, 50, Phase::Day, &[], 0.0);
        assert!(
            mgr.residents[0].work_timer >= WORK_TIMER_MIN,
            "觸發後 work_timer 應重置到最小間隔以上"
        );
    }

    // ── ROADMAP 121：鄰里互動測試 ─────────────────────────────────────────────

    /// 兩人靠近且無冷卻 → 應觸發 NeighborChat 事件。
    #[test]
    fn neighbor_chat_triggers_when_close() {
        let mut mgr = ResidentManager::new();
        // 確保有至少兩位居民
        while mgr.residents.len() < 2 {
            mgr.population_timer = 0.0;
            mgr.tick(0.01, 100, Phase::Day, &[], 0.0);
        }
        // 把兩人放在一起
        mgr.residents[0].x = 2400.0;
        mgr.residents[0].y = 2400.0;
        mgr.residents[0].chat_remaining = 0.0;
        mgr.residents[0].chat_cooldown  = 0.0;
        mgr.residents[1].x = 2400.0 + MEET_DIST * 0.5; // 距離 < MEET_DIST
        mgr.residents[1].y = 2400.0;
        mgr.residents[1].chat_remaining = 0.0;
        mgr.residents[1].chat_cooldown  = 0.0;
        let events = mgr.check_neighbor_interactions();
        assert!(
            events.iter().any(|e| matches!(e, ResidentLifecycleEvent::NeighborChat { .. })),
            "靠近且無冷卻應觸發 NeighborChat"
        );
    }

    /// 兩人距離 > MEET_DIST → 不應觸發。
    #[test]
    fn neighbor_chat_does_not_trigger_when_far() {
        let mut mgr = ResidentManager::new();
        while mgr.residents.len() < 2 {
            mgr.population_timer = 0.0;
            mgr.tick(0.01, 100, Phase::Day, &[], 0.0);
        }
        mgr.residents[0].x = 2400.0;
        mgr.residents[0].y = 2400.0;
        mgr.residents[0].chat_cooldown = 0.0;
        mgr.residents[1].x = 2400.0 + MEET_DIST * 2.0; // 超過觸發距離
        mgr.residents[1].y = 2400.0;
        mgr.residents[1].chat_cooldown = 0.0;
        let events = mgr.check_neighbor_interactions();
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::NeighborChat { .. })),
            "距離過遠不應觸發 NeighborChat"
        );
    }

    /// 任一方有冷卻 → 不應觸發。
    #[test]
    fn neighbor_chat_does_not_trigger_on_cooldown() {
        let mut mgr = ResidentManager::new();
        while mgr.residents.len() < 2 {
            mgr.population_timer = 0.0;
            mgr.tick(0.01, 100, Phase::Day, &[], 0.0);
        }
        mgr.residents[0].x = 2400.0; mgr.residents[0].y = 2400.0;
        mgr.residents[0].chat_cooldown = CHAT_COOLDOWN; // 有冷卻
        mgr.residents[1].x = 2400.0 + MEET_DIST * 0.5;
        mgr.residents[1].y = 2400.0;
        mgr.residents[1].chat_cooldown = 0.0;
        let events = mgr.check_neighbor_interactions();
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::NeighborChat { .. })),
            "有冷卻的居民不應觸發互動"
        );
    }

    /// 觸發後居民停止移動（chat_remaining = CHAT_DURATION）。
    #[test]
    fn neighbor_chat_sets_chat_remaining() {
        let mut mgr = ResidentManager::new();
        while mgr.residents.len() < 2 {
            mgr.population_timer = 0.0;
            mgr.tick(0.01, 100, Phase::Day, &[], 0.0);
        }
        mgr.residents[0].x = 2400.0; mgr.residents[0].y = 2400.0;
        mgr.residents[0].chat_cooldown = 0.0; mgr.residents[0].chat_remaining = 0.0;
        mgr.residents[1].x = 2400.0 + MEET_DIST * 0.5;
        mgr.residents[1].y = 2400.0;
        mgr.residents[1].chat_cooldown = 0.0; mgr.residents[1].chat_remaining = 0.0;
        mgr.check_neighbor_interactions();
        assert!(
            mgr.residents[0].chat_remaining > 0.0,
            "觸發後 chat_remaining 應 > 0（停止移動）"
        );
        assert!(
            mgr.residents[1].chat_remaining > 0.0,
            "另一居民 chat_remaining 也應 > 0"
        );
    }

    /// 觸發後冷卻應設為 CHAT_COOLDOWN。
    #[test]
    fn neighbor_chat_sets_cooldown() {
        let mut mgr = ResidentManager::new();
        while mgr.residents.len() < 2 {
            mgr.population_timer = 0.0;
            mgr.tick(0.01, 100, Phase::Day, &[], 0.0);
        }
        mgr.residents[0].x = 2400.0; mgr.residents[0].y = 2400.0;
        mgr.residents[0].chat_cooldown = 0.0; mgr.residents[0].chat_remaining = 0.0;
        mgr.residents[1].x = 2400.0 + MEET_DIST * 0.5;
        mgr.residents[1].y = 2400.0;
        mgr.residents[1].chat_cooldown = 0.0; mgr.residents[1].chat_remaining = 0.0;
        mgr.check_neighbor_interactions();
        assert!(
            (mgr.residents[0].chat_cooldown - CHAT_COOLDOWN).abs() < 0.1,
            "觸發後 chat_cooldown 應等於 CHAT_COOLDOWN"
        );
    }

    /// 觸發事件的 name_a/name_b 應與居民真實名字一致。
    #[test]
    fn neighbor_chat_event_has_correct_names() {
        let mut mgr = ResidentManager::new();
        while mgr.residents.len() < 2 {
            mgr.population_timer = 0.0;
            mgr.tick(0.01, 100, Phase::Day, &[], 0.0);
        }
        let expected_a = mgr.residents[0].name;
        let expected_b = mgr.residents[1].name;
        mgr.residents[0].x = 2400.0; mgr.residents[0].y = 2400.0;
        mgr.residents[0].chat_cooldown = 0.0; mgr.residents[0].chat_remaining = 0.0;
        mgr.residents[1].x = 2400.0 + MEET_DIST * 0.5;
        mgr.residents[1].y = 2400.0;
        mgr.residents[1].chat_cooldown = 0.0; mgr.residents[1].chat_remaining = 0.0;
        let events = mgr.check_neighbor_interactions();
        if let Some(ResidentLifecycleEvent::NeighborChat { name_a, name_b, text_a, .. }) = events.first() {
            assert_eq!(*name_a, expected_a, "event name_a 應與居民實際名字一致");
            assert_eq!(*name_b, expected_b, "event name_b 應與居民實際名字一致");
            assert!(text_a.contains(expected_b), "主動招呼文字應包含對方名字 {expected_b}");
        } else {
            panic!("應有 NeighborChat 事件");
        }
    }

    // ── ROADMAP 122：居民隨機小事件測試 ──────────────────────────────────────────

    /// mini_event_timer 歸零時應觸發 MiniEvent，且文字包含居民名字。
    #[test]
    fn mini_event_fires_when_timer_expires() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].mini_event_timer = 0.0;
        let (events, _) = mgr.tick(0.01, 50, Phase::Day, &[], 0.0);
        assert!(
            events.iter().any(|e| matches!(e, ResidentLifecycleEvent::MiniEvent { .. })),
            "mini_event_timer=0 應觸發 MiniEvent"
        );
    }

    /// 觸發後 mini_event_timer 應重置到最小間隔以上。
    #[test]
    fn mini_event_timer_resets_after_firing() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].mini_event_timer = 0.0;
        mgr.tick(0.01, 50, Phase::Day, &[], 0.0);
        assert!(
            mgr.residents[0].mini_event_timer >= MINI_EVENT_TIMER_MIN,
            "觸發後 mini_event_timer 應重置到最小間隔以上"
        );
    }

    /// MiniEvent 文字應包含居民名字。
    #[test]
    fn mini_event_text_contains_resident_name() {
        let mut mgr = ResidentManager::new();
        let expected_name = mgr.residents[0].name;
        mgr.residents[0].mini_event_timer = 0.0;
        let (events, _) = mgr.tick(0.01, 50, Phase::Day, &[], 0.0);
        if let Some(ResidentLifecycleEvent::MiniEvent { text }) = events.iter().find(|e| matches!(e, ResidentLifecycleEvent::MiniEvent { .. })) {
            assert!(text.contains(expected_name), "MiniEvent 文字應包含居民名字 '{expected_name}'，got: {text}");
        } else {
            panic!("應有 MiniEvent 事件");
        }
    }

    /// 計時器未到時不應觸發（避免洪水廣播）。
    #[test]
    fn mini_event_does_not_fire_before_timer() {
        let mut mgr = ResidentManager::new();
        // 確保所有居民計時器都遠未到期
        for r in &mut mgr.residents {
            r.mini_event_timer = MINI_EVENT_TIMER_MAX;
        }
        let (events, _) = mgr.tick(0.01, 50, Phase::Day, &[], 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::MiniEvent { .. })),
            "計時器未到不應觸發 MiniEvent"
        );
    }

    /// 不同居民觸發後 seed 各自獨立遞增（下次會輪到不同模板）。
    #[test]
    fn mini_event_seed_increments_independently() {
        let mut mgr = ResidentManager::new();
        let seed_before = mgr.residents[0].mini_event_seed;
        mgr.residents[0].mini_event_timer = 0.0;
        mgr.tick(0.01, 50, Phase::Day, &[], 0.0);
        assert_eq!(
            mgr.residents[0].mini_event_seed,
            seed_before + 1,
            "觸發後 mini_event_seed 應遞增 1"
        );
    }

    // ── ROADMAP 123 主動搭話測試 ──────────────────────────────────────────────

    /// 玩家進入範圍且冷卻結束時觸發 PlayerGreeting 事件。
    #[test]
    fn player_greeting_triggers_when_player_nearby() {
        let mut mgr = ResidentManager::new();
        // 強制冷卻結束
        for r in &mut mgr.residents { r.greeting_cooldown = 0.0; }
        let rx = mgr.residents[0].x;
        let ry = mgr.residents[0].y;
        // 玩家正好在居民旁邊（距離 50px，小於 GREETING_DIST_PX 120px）
        let players = vec![("冒險者".to_string(), rx + 50.0, ry)];
        let (events, _) = mgr.tick(0.01, 50, Phase::Day, &players, 0.0);
        assert!(
            events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PlayerGreeting { .. })),
            "玩家在範圍內且冷卻結束時應觸發 PlayerGreeting"
        );
    }

    /// 玩家在範圍外時不觸發搭話。
    #[test]
    fn player_greeting_no_trigger_when_player_far() {
        let mut mgr = ResidentManager::new();
        // 將所有居民集中在固定座標，避免隨機位置與玩家意外重疊。
        for r in &mut mgr.residents {
            r.greeting_cooldown = 0.0;
            r.x = 2400.0;
            r.y = 2400.0;
        }
        // 玩家距離 500px，超出 GREETING_DIST_PX 120px。
        let players = vec![("冒險者".to_string(), 2900.0, 2400.0)];
        let (events, _) = mgr.tick(0.01, 50, Phase::Day, &players, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PlayerGreeting { .. })),
            "玩家距離超出範圍時不應觸發 PlayerGreeting"
        );
    }

    /// 冷卻中不觸發搭話。
    #[test]
    fn player_greeting_no_trigger_on_cooldown() {
        let mut mgr = ResidentManager::new();
        // 所有居民都在冷卻中
        for r in &mut mgr.residents { r.greeting_cooldown = 60.0; }
        let rx = mgr.residents[0].x;
        let ry = mgr.residents[0].y;
        let players = vec![("冒險者".to_string(), rx + 50.0, ry)];
        let (events, _) = mgr.tick(0.01, 50, Phase::Day, &players, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PlayerGreeting { .. })),
            "冷卻中不應觸發 PlayerGreeting"
        );
    }

    /// 鄰里互動進行中時不觸發主動搭話。
    #[test]
    fn player_greeting_no_trigger_during_neighbor_chat() {
        let mut mgr = ResidentManager::new();
        for r in &mut mgr.residents {
            r.greeting_cooldown = 0.0;
            r.chat_remaining = 5.0; // 正在與鄰居聊天
        }
        let rx = mgr.residents[0].x;
        let ry = mgr.residents[0].y;
        let players = vec![("冒險者".to_string(), rx + 50.0, ry)];
        let (events, _) = mgr.tick(0.01, 50, Phase::Day, &players, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PlayerGreeting { .. })),
            "正在與鄰居打招呼時不應主動向玩家搭話"
        );
    }

    /// 觸發後 greeting_cooldown 應重置為 GREETING_COOLDOWN_SECS。
    #[test]
    fn player_greeting_sets_cooldown_after_trigger() {
        let mut mgr = ResidentManager::new();
        for r in &mut mgr.residents { r.greeting_cooldown = 0.0; }
        let rx = mgr.residents[0].x;
        let ry = mgr.residents[0].y;
        let players = vec![("路人甲".to_string(), rx + 30.0, ry)];
        mgr.tick(0.01, 50, Phase::Day, &players, 0.0);
        assert!(
            mgr.residents[0].greeting_cooldown > 0.0,
            "觸發搭話後 greeting_cooldown 應 > 0"
        );
    }

    /// 沒有玩家時不觸發（空列表安全）。
    #[test]
    fn player_greeting_no_trigger_with_empty_player_list() {
        let mut mgr = ResidentManager::new();
        for r in &mut mgr.residents { r.greeting_cooldown = 0.0; }
        let (events, _) = mgr.tick(0.01, 50, Phase::Day, &[], 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PlayerGreeting { .. })),
            "玩家列表為空時不應觸發 PlayerGreeting"
        );
    }

    // ── ROADMAP 125 互助請求測試 ──────────────────────────────────────────────

    /// 計時器歸零時應觸發 HelpRequested 事件。
    #[test]
    fn help_request_triggers_when_timer_expires() {
        let mut mgr = ResidentManager::new();
        // 把第一位居民的計時器設為幾乎到零
        mgr.residents[0].help_request_timer = 0.01;
        mgr.residents[0].is_requesting_help = false;
        let (events, _) = mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        assert!(
            events.iter().any(|e| matches!(e, ResidentLifecycleEvent::HelpRequested { .. })),
            "help_request_timer 到期應觸發 HelpRequested 事件"
        );
    }

    /// 觸發後 is_requesting_help 應為 true。
    #[test]
    fn help_request_sets_requesting_flag() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].help_request_timer = 0.01;
        mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        assert!(mgr.residents[0].is_requesting_help, "觸發後 is_requesting_help 應為 true");
    }

    /// 過期後 is_requesting_help 應清除。
    #[test]
    fn help_request_expires_after_duration() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].is_requesting_help = true;
        mgr.residents[0].help_active_timer = 0.01;
        mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        assert!(!mgr.residents[0].is_requesting_help, "時限到期後 is_requesting_help 應為 false");
    }

    /// requesting_ids 應正確回傳求助居民 id。
    #[test]
    fn requesting_ids_returns_correct_ids() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].is_requesting_help = true;
        let ids = mgr.requesting_ids();
        assert!(ids.contains(&mgr.residents[0].id), "requesting_ids 應包含求助居民的 id");
    }

    /// 無求助時 requesting_ids 回傳空清單。
    #[test]
    fn requesting_ids_empty_when_no_requests() {
        let mgr = ResidentManager::new();
        assert!(mgr.requesting_ids().is_empty(), "初始狀態應無求助請求");
    }

    /// fulfill_help_request 應清除請求並回傳 (true, ...)。
    #[test]
    fn fulfill_help_request_clears_flag() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].is_requesting_help = true;
        mgr.residents[0].help_active_timer = 100.0;
        let rid = mgr.residents[0].id.clone();
        let (ok, _) = mgr.fulfill_help_request(&rid);
        assert!(ok, "fulfill_help_request 應回傳 true");
        assert!(!mgr.residents[0].is_requesting_help, "完成後 is_requesting_help 應為 false");
    }

    /// 對非求助居民呼叫 fulfill_help_request 應回傳 (false, None)。
    #[test]
    fn fulfill_help_request_returns_false_if_not_requesting() {
        let mut mgr = ResidentManager::new();
        // 確保第一位居民沒在求助
        mgr.residents[0].is_requesting_help = false;
        let rid = mgr.residents[0].id.clone();
        let (ok, ev) = mgr.fulfill_help_request(&rid);
        assert!(!ok, "居民未求助時 fulfill_help_request 應回傳 false");
        assert!(ev.is_none(), "未求助時不應有快樂提升事件");
    }

    /// find_requesting_by_id 應找到求助居民。
    #[test]
    fn find_requesting_by_id_finds_requesting_resident() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].is_requesting_help = true;
        let rid = mgr.residents[0].id.clone();
        assert!(mgr.find_requesting_by_id(&rid).is_some(), "應找到正在求助的居民");
    }

    /// find_requesting_by_id 對未求助居民回傳 None。
    #[test]
    fn find_requesting_by_id_returns_none_if_not_requesting() {
        let mgr = ResidentManager::new();
        let rid = mgr.residents[0].id.clone();
        assert!(mgr.find_requesting_by_id(&rid).is_none(), "未求助時應回傳 None");
    }

    // ── 心情溫度測試（ROADMAP 126）─────────────────────────────────────────────

    /// 初始快樂值應為 HAPPINESS_INITIAL。
    #[test]
    fn resident_initial_happiness() {
        let mgr = ResidentManager::new();
        for r in &mgr.residents {
            assert_eq!(r.happiness, HAPPINESS_INITIAL, "初始快樂值應為 {HAPPINESS_INITIAL}");
        }
    }

    /// fulfill_help_request 應增加快樂值（上限 100）。
    #[test]
    fn fulfill_help_request_increases_happiness() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].is_requesting_help = true;
        mgr.residents[0].help_active_timer = 100.0;
        let initial = mgr.residents[0].happiness;
        let rid = mgr.residents[0].id.clone();
        let (ok, _) = mgr.fulfill_help_request(&rid);
        assert!(ok, "fulfill_help_request 應成功");
        let new_h = mgr.residents[0].happiness;
        assert!(new_h > initial || new_h == 100, "快樂值應增加或達上限：initial={initial}, new={new_h}");
    }

    /// 快樂值達上限後不超過 100。
    #[test]
    fn happiness_does_not_exceed_max() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = 95;
        mgr.residents[0].is_requesting_help = true;
        let rid = mgr.residents[0].id.clone();
        let _ = mgr.fulfill_help_request(&rid);
        assert!(mgr.residents[0].happiness <= 100, "快樂值不應超過 100");
    }

    /// 快樂值首次突破門檻時，fulfill_help_request 應回傳 HappinessBoost 事件。
    #[test]
    fn happiness_boost_event_emitted_on_threshold_cross() {
        let mut mgr = ResidentManager::new();
        // 把快樂值設在門檻剛好下方
        mgr.residents[0].happiness = HAPPINESS_HAPPY_THRESHOLD - 1;
        mgr.residents[0].is_requesting_help = true;
        let rid = mgr.residents[0].id.clone();
        let (ok, ev) = mgr.fulfill_help_request(&rid);
        assert!(ok, "fulfill_help_request 應成功");
        assert!(ev.is_some(), "跨越快樂門檻時應產生 HappinessBoost 事件");
    }

    /// 快樂值已在門檻以上時，fulfill_help_request 不應重複觸發 HappinessBoost。
    #[test]
    fn happiness_boost_event_not_emitted_if_already_happy() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = HAPPINESS_HAPPY_THRESHOLD + 5; // 已在門檻以上
        mgr.residents[0].is_requesting_help = true;
        let rid = mgr.residents[0].id.clone();
        let (ok, ev) = mgr.fulfill_help_request(&rid);
        assert!(ok, "fulfill_help_request 應成功");
        assert!(ev.is_none(), "快樂值已在門檻以上時不應重複觸發 HappinessBoost");
    }

    /// 快樂衰減：happiness_decay_timer 歸零後快樂值應減少，且不低於 HAPPINESS_MIN。
    #[test]
    fn happiness_decays_but_not_below_min() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = HAPPINESS_INITIAL;
        mgr.residents[0].happiness_decay_timer = 0.1; // 接近 0
        // tick 一幀讓計時器歸零
        let _ = mgr.tick(0.2, HAPPINESS_INITIAL as i32, Phase::Day, &[], 0.0);
        let h = mgr.residents[0].happiness;
        assert!(h < HAPPINESS_INITIAL, "衰減後快樂值應下降：got {h}");
        // 多次衰減也不低於 HAPPINESS_MIN
        for r in &mut mgr.residents { r.happiness = HAPPINESS_MIN; r.happiness_decay_timer = 0.0; }
        let _ = mgr.tick(1.0, HAPPINESS_MIN as i32, Phase::Day, &[], 0.0);
        for r in &mgr.residents {
            assert!(r.happiness >= HAPPINESS_MIN, "快樂值不應低於 HAPPINESS_MIN={HAPPINESS_MIN}：got {}", r.happiness);
        }
    }

    /// moods() 應回傳每位居民的 (id, happiness)。
    #[test]
    fn moods_returns_all_residents() {
        let mgr = ResidentManager::new();
        let moods = mgr.moods();
        assert_eq!(moods.len(), mgr.population(), "moods() 長度應等於居民人數");
        for (id, h) in &moods {
            assert!(id.starts_with("resident_"), "id 應以 'resident_' 開頭：{id}");
            assert!(*h >= HAPPINESS_MIN, "快樂值不應低於 HAPPINESS_MIN");
            assert!(*h <= 100, "快樂值不應超過 100");
        }
    }

    // ── ROADMAP 127 快樂小回饋測試 ──────────────────────────────────────────────

    /// 快樂居民 + gift_timer 歸零 + 玩家在範圍內 → 應觸發 PlayerGift 事件。
    #[test]
    fn gift_triggers_when_happy_and_player_nearby() {
        let mut mgr = ResidentManager::new();
        // 設第一位居民為快樂且計時器到期
        mgr.residents[0].happiness = HAPPINESS_HAPPY_THRESHOLD;
        mgr.residents[0].gift_timer = 0.01;
        let rx = mgr.residents[0].x;
        let ry = mgr.residents[0].y;
        // 玩家在 40px 以內（< GIFT_DIST_PX 80px）
        let players = vec![("英雄".to_string(), rx + 40.0, ry)];
        let (events, _) = mgr.tick(0.1, 50, Phase::Day, &players, 0.0);
        assert!(
            events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PlayerGift { .. })),
            "快樂且計時到期且玩家在附近時應觸發 PlayerGift"
        );
    }

    /// 不快樂的居民即使計時到期也不觸發禮物。
    #[test]
    fn gift_not_triggered_when_not_happy() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = HAPPINESS_HAPPY_THRESHOLD - 1; // 低於門檻
        mgr.residents[0].gift_timer = 0.01;
        let rx = mgr.residents[0].x;
        let ry = mgr.residents[0].y;
        let players = vec![("英雄".to_string(), rx + 40.0, ry)];
        let (events, _) = mgr.tick(0.1, 50, Phase::Day, &players, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PlayerGift { .. })),
            "不快樂的居民不應觸發 PlayerGift"
        );
    }

    /// 玩家距離超出 GIFT_DIST_PX 時不觸發禮物。
    #[test]
    fn gift_not_triggered_when_player_far() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = HAPPINESS_HAPPY_THRESHOLD;
        mgr.residents[0].gift_timer = 0.01;
        let rx = mgr.residents[0].x;
        let ry = mgr.residents[0].y;
        // 玩家距離超過 GIFT_DIST_PX
        let players = vec![("英雄".to_string(), rx + GIFT_DIST_PX + 10.0, ry)];
        let (events, _) = mgr.tick(0.1, 50, Phase::Day, &players, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PlayerGift { .. })),
            "玩家距離超出範圍不應觸發 PlayerGift"
        );
    }

    /// 無玩家在線時不觸發禮物。
    #[test]
    fn gift_not_triggered_when_no_players() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = HAPPINESS_HAPPY_THRESHOLD;
        mgr.residents[0].gift_timer = 0.01;
        let (events, _) = mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PlayerGift { .. })),
            "無玩家時不應觸發 PlayerGift"
        );
    }

    /// 觸發後 gift_timer 應重置為 ≥ GIFT_TIMER_MIN。
    #[test]
    fn gift_timer_resets_after_trigger() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = HAPPINESS_HAPPY_THRESHOLD;
        mgr.residents[0].gift_timer = 0.01;
        let rx = mgr.residents[0].x;
        let ry = mgr.residents[0].y;
        let players = vec![("英雄".to_string(), rx + 40.0, ry)];
        mgr.tick(0.1, 50, Phase::Day, &players, 0.0);
        assert!(
            mgr.residents[0].gift_timer >= GIFT_TIMER_MIN,
            "觸發後 gift_timer 應重置到最小間隔以上"
        );
    }

    /// PlayerGift 事件的 player_name 應與玩家真實名字一致。
    #[test]
    fn gift_event_has_correct_player_name() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = HAPPINESS_HAPPY_THRESHOLD;
        mgr.residents[0].gift_timer = 0.01;
        let rx = mgr.residents[0].x;
        let ry = mgr.residents[0].y;
        let players = vec![("小明".to_string(), rx + 30.0, ry)];
        let (events, _) = mgr.tick(0.1, 50, Phase::Day, &players, 0.0);
        if let Some(ResidentLifecycleEvent::PlayerGift { player_name, text, .. }) = events.iter().find(|e| matches!(e, ResidentLifecycleEvent::PlayerGift { .. })) {
            assert_eq!(player_name, "小明", "PlayerGift 的 player_name 應為 '小明'");
            assert!(text.contains("小明"), "招待訊息應含玩家名 '小明'：{text}");
        } else {
            panic!("應有 PlayerGift 事件");
        }
    }

    /// 計時器未到不觸發（防洪水廣播）。
    #[test]
    fn gift_not_triggered_before_timer() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = HAPPINESS_HAPPY_THRESHOLD;
        mgr.residents[0].gift_timer = GIFT_TIMER_MAX; // 遠未到期
        let rx = mgr.residents[0].x;
        let ry = mgr.residents[0].y;
        let players = vec![("英雄".to_string(), rx + 40.0, ry)];
        let (events, _) = mgr.tick(0.1, 50, Phase::Day, &players, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, ResidentLifecycleEvent::PlayerGift { .. })),
            "計時器未到不應觸發 PlayerGift"
        );
    }

    // ── ROADMAP 177：野外採集隊測試 ──────────────────────────────────────────

    #[test]
    fn expedition_starts_when_requested() {
        let mut mgr = ResidentManager::new();
        // 確保至少有一位快樂居民
        mgr.residents[0].happiness = 60;
        let ev = mgr.start_expedition("靈蛾巢".to_string(), 3000.0, 3000.0);
        assert!(ev.is_some(), "應能成功啟動採集隊");
        assert!(mgr.residents.iter().any(|r| r.expedition.is_some()), "應有居民進入採集狀態");
        if let Some(ResidentLifecycleEvent::ExpeditionStarted { target_name, .. }) = ev {
            assert_eq!(target_name, "靈蛾巢");
        }
    }

    #[test]
    fn expedition_npc_moves_toward_target() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = 60;
        mgr.residents[0].x = 2500.0;
        mgr.residents[0].y = 2500.0;
        mgr.start_expedition("目標".to_string(), 3000.0, 3000.0);
        
        // tick 推進
        mgr.tick(1.0, 50, Phase::Day, &[], 0.0);
        let r = &mgr.residents[0];
        assert!(r.x > 2500.0, "採集隊員應向目標移動 (x: {})", r.x);
        assert!(r.y > 2500.0, "採集隊員應向目標移動 (y: {})", r.y);
    }

    #[test]
    fn expedition_fails_on_npc_death() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].happiness = 60;
        mgr.start_expedition("危險地".to_string(), 3000.0, 3000.0);
        
        // 模擬受傷至死
        mgr.residents[0].hp = 0.0;
        let (events, _) = mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        
        assert!(events.iter().any(|e| matches!(e, ResidentLifecycleEvent::ExpeditionFailed { .. })), "死亡應導致採集失敗");
        assert!(mgr.residents[0].expedition.is_none(), "失敗後應清除狀態");
        assert!(mgr.residents[0].hp > 0.0, "失敗後應恢復生命以便日常生活");
    }

    #[test]
    fn expedition_succeeds_when_returning_to_center() {
        let mut mgr = ResidentManager::new();
        // 確保只有一位符合條件的快樂居民
        for r in &mut mgr.residents { r.happiness = 0; }
        mgr.residents[0].happiness = 60;

        mgr.start_expedition("目標".to_string(), 3000.0, 3000.0);
        assert_eq!(mgr.residents.iter().filter(|r| r.expedition.is_some()).count(), 1);
        
        // 1. 到達目標
        mgr.residents[0].x = 3000.0;
        mgr.residents[0].y = 3000.0;
        mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        assert!(matches!(mgr.residents[0].expedition.as_ref().unwrap().phase, ExpeditionPhase::At));
        
        // 2. 停留完成
        mgr.residents[0].expedition.as_mut().unwrap().stay_timer = 0.01;
        mgr.tick(0.2, 50, Phase::Day, &[], 0.0);
        assert!(matches!(mgr.residents[0].expedition.as_ref().unwrap().phase, ExpeditionPhase::From));
        
        // 3. 回到城中心
        mgr.residents[0].x = 2500.0;
        mgr.residents[0].y = 2500.0;
        let (events, _) = mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        
        assert!(events.iter().any(|e| matches!(e, ResidentLifecycleEvent::ExpeditionSuccess { .. })), "回到中心應觸發成功");
        assert!(mgr.residents[0].expedition.is_none(), "成功後應清除狀態");
    }

    // ── 生態守望（ROADMAP 180）測試 ──────────────────────────────────

    #[test]
    fn eco_alarm_triggers_above_threshold_once() {
        let mut mgr = ResidentManager::new();
        // 低壓不觸發
        let (ev, _) = mgr.tick(0.1, 50, Phase::Day, &[], ECO_ALARM_PRESSURE - 1.0);
        assert!(!ev.iter().any(|e| matches!(e, ResidentLifecycleEvent::EcoAlarm { .. })), "未達閾值不該警戒");
        assert!(!mgr.town_alarmed);
        // 衝上閾值觸發一次
        let (ev, _) = mgr.tick(0.1, 50, Phase::Day, &[], ECO_ALARM_PRESSURE);
        assert!(ev.iter().any(|e| matches!(e, ResidentLifecycleEvent::EcoAlarm { .. })), "達閾值應警戒");
        assert!(mgr.town_alarmed);
        // 持續高壓不重複廣播
        let (ev, _) = mgr.tick(0.1, 50, Phase::Day, &[], ECO_ALARM_PRESSURE + 5.0);
        assert!(!ev.iter().any(|e| matches!(e, ResidentLifecycleEvent::EcoAlarm { .. })), "持續高壓不該重複警戒廣播");
        assert!(mgr.town_alarmed);
    }

    #[test]
    fn eco_alarm_hysteresis_holds_between_thresholds() {
        let mut mgr = ResidentManager::new();
        mgr.tick(0.1, 50, Phase::Day, &[], ECO_ALARM_PRESSURE); // 進警戒
        assert!(mgr.town_alarmed);
        // 壓力落在 calm 與 alarm 之間：遲滯區，仍維持警戒、不解除
        let mid = (ECO_ALARM_PRESSURE + ECO_CALM_PRESSURE) / 2.0;
        let (ev, _) = mgr.tick(0.1, 50, Phase::Day, &[], mid);
        assert!(mgr.town_alarmed, "遲滯區間應維持警戒");
        assert!(!ev.iter().any(|e| matches!(e, ResidentLifecycleEvent::EcoCalm { .. })), "遲滯區間不該解除");
    }

    #[test]
    fn eco_calm_triggers_below_calm_threshold() {
        let mut mgr = ResidentManager::new();
        mgr.tick(0.1, 50, Phase::Day, &[], ECO_ALARM_PRESSURE);
        assert!(mgr.town_alarmed);
        let (ev, _) = mgr.tick(0.1, 50, Phase::Day, &[], ECO_CALM_PRESSURE - 1.0);
        assert!(ev.iter().any(|e| matches!(e, ResidentLifecycleEvent::EcoCalm { .. })), "跌破安寧值應解除警戒");
        assert!(!mgr.town_alarmed);
    }

    #[test]
    fn alarmed_residents_move_toward_town_center() {
        let mut mgr = ResidentManager::new();
        // 把一位居民放到遠離廣場的位置
        mgr.residents[0].x = WANDER_X_MIN;
        mgr.residents[0].y = WANDER_Y_MIN;
        mgr.residents[0].expedition = None;
        let before = {
            let r = &mgr.residents[0];
            let dx = TOWN_CENTER_X - r.x;
            let dy = TOWN_CENTER_Y - r.y;
            (dx * dx + dy * dy).sqrt()
        };
        // 推進數秒的避難移動
        for _ in 0..30 {
            mgr.tick(0.5, 50, Phase::Day, &[], ECO_ALARM_PRESSURE);
        }
        let after = {
            let r = &mgr.residents[0];
            let dx = TOWN_CENTER_X - r.x;
            let dy = TOWN_CENTER_Y - r.y;
            (dx * dx + dy * dy).sqrt()
        };
        assert!(after < before, "避難時居民應更靠近城中心（before={before}, after={after}）");
        assert!(mgr.residents[0].alarmed, "避難中居民 alarmed 應為 true");
    }

    // ── 菁英 Alpha 殞落凱旋（ROADMAP 185）測試 ──────────────────────────────────

    #[test]
    fn hero_triumph_ignites_celebration_and_counts_in_town_residents() {
        let mut mgr = ResidentManager::new();
        // 平時（未避難）討伐菁英 → 點燃歡慶，回傳在城居民數（無採集隊時 = 全員）。
        let cheering = mgr.notify_hero_triumph();
        assert_eq!(cheering, mgr.residents.len(), "未在避難時，全體在城居民都會歡慶");
        assert!(mgr.celebrate_timer > 0.0, "應點亮歡慶計時");
        // 一幀後居民 celebrating 應為 true
        mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        assert!(mgr.residents.iter().all(|r| r.celebrating), "歡慶幀內居民 celebrating 應為 true");
    }

    #[test]
    fn hero_triumph_suppressed_while_town_alarmed() {
        let mut mgr = ResidentManager::new();
        // 先進入生態避難警戒
        mgr.tick(0.1, 50, Phase::Day, &[], ECO_ALARM_PRESSURE);
        assert!(mgr.town_alarmed);
        // 危機未解時討伐菁英 → 不歡慶（避難優先），回傳 0
        let cheering = mgr.notify_hero_triumph();
        assert_eq!(cheering, 0, "城鎮避難中不該歡慶");
        assert_eq!(mgr.celebrate_timer, 0.0, "避難中不該點亮歡慶計時");
    }

    #[test]
    fn celebration_yields_to_alarm_when_crisis_strikes_mid_party() {
        let mut mgr = ResidentManager::new();
        mgr.notify_hero_triumph();
        // 歡慶中突然生態壓力衝頂 → 避難壓過歡慶，居民不再 celebrating（顯示 😰 而非 🎉）。
        mgr.tick(0.1, 50, Phase::Day, &[], ECO_ALARM_PRESSURE);
        assert!(mgr.town_alarmed, "壓力衝頂應進入避難");
        assert!(mgr.residents.iter().all(|r| !r.celebrating), "避難優先，歡慶旗標應被壓下");
        assert!(mgr.residents.iter().all(|r| r.alarmed), "居民應改為避難狀態");
    }

    #[test]
    fn celebration_expires_after_duration() {
        let mut mgr = ResidentManager::new();
        mgr.notify_hero_triumph();
        // 推進超過歡慶時長，歡慶自然結束、居民散回日常。
        let mut elapsed = 0.0;
        while elapsed <= CELEBRATE_DURATION_SECS + 1.0 {
            mgr.tick(0.5, 50, Phase::Day, &[], 0.0);
            elapsed += 0.5;
        }
        assert_eq!(mgr.celebrate_timer, 0.0, "超時後歡慶計時應歸零");
        assert!(mgr.residents.iter().all(|r| !r.celebrating), "超時後居民不應再歡慶");
    }

    #[test]
    fn celebration_views_expose_flag() {
        let mut mgr = ResidentManager::new();
        mgr.notify_hero_triumph();
        mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        // views() 第 8 元素（celebrating）應隨歡慶為 true，供快照傳給前端。
        assert!(mgr.views().all(|(_, _, _, _, _, _, _, celebrating)| celebrating),
            "歡慶中 views() 的 celebrating 應為 true");
    }

    #[test]
    fn huddle_spots_are_distinct_and_near_center() {
        // 不同索引散佈在不同點，且都落在城中心廣場附近（< 兩倍半徑）。
        let a = huddle_spot(0);
        let b = huddle_spot(1);
        let c = huddle_spot(2);
        assert!(a != b && b != c && a != c, "不同索引應散佈在不同點");
        for (x, y) in [a, b, c] {
            let d = ((TOWN_CENTER_X - x).powi(2) + (TOWN_CENTER_Y - y).powi(2)).sqrt();
            assert!(d <= HUDDLE_RADIUS * 2.0, "聚集點應靠近城中心廣場");
        }
    }

    #[test]
    fn alarmed_suppresses_flavor_thought_bubbles() {
        let mut mgr = ResidentManager::new();
        // 強制思想泡泡即刻就緒
        for r in &mut mgr.residents { r.thought_timer = 0.0; }
        let (_, thoughts) = mgr.tick(0.1, 50, Phase::Day, &[], ECO_ALARM_PRESSURE);
        assert!(thoughts.is_empty(), "避難期間應抑制思想泡泡（保持危機氣氛安靜）");
    }

    // ── 凱旋餘韻談資（ROADMAP 186）──────────────────────────────────────────

    #[test]
    fn triumph_celebration_still_suppresses_thoughts() {
        // 歡慶期（185）內仍應抑制思想泡泡，讓歡呼氣氛集中——餘韻談資只在歡慶散場後才冒。
        let mut mgr = ResidentManager::new();
        mgr.notify_hero_triumph();
        for r in &mut mgr.residents { r.thought_timer = 0.0; }
        let (_, thoughts) = mgr.tick(0.1, 50, Phase::Day, &[], 0.0);
        assert!(thoughts.is_empty(), "歡慶期間應抑制思想泡泡（集中歡慶氣氛）");
    }

    #[test]
    fn afterglow_emits_triumph_thoughts() {
        // 歡慶散場後進入餘韻期，思想泡泡應改冒勝利談資（triumph = true）。
        let mut mgr = ResidentManager::new();
        mgr.notify_hero_triumph();
        for r in &mut mgr.residents { r.thought_timer = 0.0; }
        // 一次推進跨過 8 秒歡慶期、落在餘韻期內。
        let (_, thoughts) = mgr.tick(CELEBRATE_DURATION_SECS + 0.1, 50, Phase::Day, &[], 0.0);
        assert!(!thoughts.is_empty(), "餘韻期應冒出思想泡泡");
        assert!(thoughts.iter().all(|t| t.triumph), "餘韻期思想泡泡應全為勝利談資");
    }

    #[test]
    fn afterglow_expires_back_to_normal_thoughts() {
        // 餘韻期過完，思想泡泡應回歸常態（triumph = false）。
        let mut mgr = ResidentManager::new();
        mgr.notify_hero_triumph();
        for r in &mut mgr.residents { r.thought_timer = 0.0; }
        // 一次推進跨過 歡慶 + 餘韻 總時長，餘韻已耗盡。
        let (_, thoughts) = mgr.tick(CELEBRATE_DURATION_SECS + TRIUMPH_AFTERGLOW_SECS + 0.1, 50, Phase::Day, &[], 0.0);
        assert!(!thoughts.is_empty(), "餘韻耗盡後思想泡泡應正常冒出");
        assert!(thoughts.iter().all(|t| !t.triumph), "餘韻耗盡後思想泡泡應回歸常態（非勝利談資）");
    }

    #[test]
    fn alarm_overrides_afterglow_chatter() {
        // 危機優先：餘韻期間若生態避難警戒響起，應壓過勝利談資、抑制思想泡泡。
        let mut mgr = ResidentManager::new();
        mgr.notify_hero_triumph();
        for r in &mut mgr.residents { r.thought_timer = 0.0; }
        // 推進跨過歡慶期落入餘韻，同時生態壓力衝頂進入避難。
        let (_, thoughts) = mgr.tick(CELEBRATE_DURATION_SECS + 0.1, 50, Phase::Day, &[], ECO_ALARM_PRESSURE);
        assert!(thoughts.is_empty(), "餘韻遇生態避難應讓位（危機優先，不閒聊勝利）");
    }
}
