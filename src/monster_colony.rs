//! ROADMAP 164：怪物巢穴=聚落——怪物從命名巢穴出生/回巢，
//! 族群可清剿衰退或放著壯大；與野生動物聚落同一設計哲學。
//!
//! ROADMAP 168：巢穴 Alpha 湧現——族群達峰值時湧現地區霸主，
//! 3 倍生命、守衛領地；擊殺後族群-2、全服廣播、殺手得乙太+晶核。
//!
//! ROADMAP 169：Alpha 咆哮指揮——Alpha 首領每 90 秒依血量/附近玩家數
//! 決定戰術（包圍/集火/撤退/集結），交由 game.rs 非同步生成 Groq 台詞並廣播。
//! 成本紀律：每隻 Alpha 最多每 90 秒呼叫一次 LLM；無玩家時仍使用罐頭台詞。
//!
//! ROADMAP 170：Alpha 領地爭奪——不同巢穴的 Alpha 進入 900px 內自動廝殺，
//! 敗者巢穴族群衰退、勝者稱霸，玩家趁亂收漁人之利；零 LLM、純算術。
//!
//! ROADMAP 173：傳說古 Alpha 降臨——3 個以上巢穴同時達滿員時，生態系頂點
//! 「傳說古 Alpha」降臨荒野；HP 600、攻擊距離 120px；Groq 生成降臨宣言廣播全服；
//! 擊倒掉落傳說晶核，合成傳說戰刃（ATK+55，全遊戲最強）；20 分鐘冷卻後可再湧現。
//! 成本紀律：降臨時才呼叫一次 LLM（利用 boss_ai_sem 限流）；零 migration。
//!
//! 效能：全純算術、零 migration；記憶體模式，重啟全重置。

use serde::Serialize;
use crate::combat::EnemyKind;

// ─── 常數 ────────────────────────────────────────────────────────────────────

/// 正常族群補充間隔（秒）：每 2 分鐘嘗試補充一隻。
const RESPAWN_SECS: f32 = 120.0;
/// 清剿後加長冷卻倍率：族群歸零後需等 3 倍才開始復生。
const WIPED_COOLDOWN_MULT: f32 = 3.0;
/// 玩家在此半徑（像素）內擊殺同類怪物，計入巢穴族群損失。
pub const COLONY_KILL_RADIUS: f32 = 420.0;

/// Alpha 冷卻（秒）：被擊殺後需等此時間才再次湧現。
const ALPHA_COOLDOWN_SECS: f32 = 300.0;
/// 挑戰 Alpha 的最大距離（像素）。
pub const ALPHA_ATTACK_REACH: f32 = 80.0;
/// Alpha 乙太獎勵：殺手個人獲得。
pub const ALPHA_KILLER_ETHER: u32 = 15;
/// Alpha 乙太獎勵：全服在線玩家各得。
pub const ALPHA_GLOBAL_ETHER: u32 = 3;
/// Alpha 晶核掉落數量。
pub const ALPHA_CRYSTAL_DROP: u32 = 1;

// ─── ROADMAP 169：Alpha 咆哮指揮常數 ────────────────────────────────────────

/// Alpha 首次湧現後，距第一次咆哮的等待時間（秒）。
const ALPHA_COMMAND_FIRST_WAIT_SECS: f32 = 60.0;
/// Alpha 咆哮指揮冷卻（秒）：每次下令後需等此時間才能再次發出指令。
pub const ALPHA_COMMAND_COOLDOWN_SECS: f32 = 90.0;
/// 當前指令顯示持續時間（秒）：前端顯示 active_tactic 氣泡的時長。
const ALPHA_TACTIC_DURATION_SECS: f32 = 30.0;

// ─── ROADMAP 170：Alpha 領地爭奪常數 ────────────────────────────────────────

/// 兩隻 Alpha 進入此半徑（像素）內且屬不同巢穴，觸發領地衝突廝殺。
const ALPHA_CLASH_RADIUS: f32 = 900.0;
/// 衝突中每秒互相造成的傷害（HP/秒）。
const ALPHA_CLASH_DAMAGE_PER_SEC: f32 = 8.0;
/// 敗者巢穴在 Alpha 被擊敗後的冷卻倍率（相對 ALPHA_COOLDOWN_SECS）。
const ALPHA_CLASH_DEFEAT_COOLDOWN_MULT: f32 = 2.0;

// ─── ROADMAP 174：Alpha 跨族結盟常數 ─────────────────────────────────────────

/// 兩隻不同巢穴的 Alpha 共存（未廝殺）多少秒後觸發跨族結盟。
const ALLIANCE_FORM_SECS: f32 = 180.0;
/// 結盟時每隻 Alpha 獲得的額外 HP（盟約加持）。
const ALLIANCE_HP_BONUS: u32 = 50;
/// 結盟期間擊殺盟約 Alpha 額外獲得的乙太獎勵。
pub const ALLIANCE_BREAK_BONUS_ETHER: u32 = 5;

// ─── ROADMAP 175：Alpha 覺醒危機常數 ─────────────────────────────────────────

/// 觸發 Alpha 覺醒的生態壓力閾值。
const ALPHA_AWAKENING_PRESSURE: f32 = 85.0;
/// 覺醒解除的生態壓力閾值（低於此值時解除覺醒）。
const ALPHA_DEAWAKEN_PRESSURE: f32 = 70.0;
/// 觸發覺醒所需的最少活躍 Alpha 數。
const ALPHA_AWAKENING_MIN_COUNT: usize = 2;
/// 覺醒時 HP 加成百分比（50%）。
const ALPHA_AWAKENING_HP_BONUS: f32 = 0.5;
/// 覺醒 Alpha 被擊殺時殺手額外獲得的乙太。
pub const AWAKENED_BONUS_ETHER: u32 = 5;

// ─── ROADMAP 173：傳說古 Alpha 常數 ──────────────────────────────────────────

/// 傳說古 Alpha 的生命值——遠超普通 Alpha（族群 Alpha 最多 24HP），需全服合力擊倒。
pub const ANCIENT_ALPHA_HP: u32 = 600;
/// 挑戰傳說古 Alpha 的最大距離（像素），比普通 Alpha 略大（體型更巨）。
pub const ANCIENT_ALPHA_ATTACK_REACH: f32 = 120.0;
/// 古 Alpha 被擊倒後的冷卻時間（秒）：20 分鐘後才能再次湧現。
const ANCIENT_ALPHA_COOLDOWN_SECS: f32 = 1200.0;
/// 觸發古 Alpha 湧現所需的「滿員巢穴」最低數量。
const ANCIENT_ALPHA_MIN_FULL_COLONIES: usize = 3;
/// 「滿員」的族群飽和度閾值（族群數 / 最大族群數 ≥ 此值）。
const ANCIENT_SATURATION_THRESHOLD: f32 = 0.80;
/// 古 Alpha 擊倒後殺手個人獲得的乙太。
pub const ANCIENT_ALPHA_KILLER_ETHER: u32 = 50;
/// 古 Alpha 擊倒後全服在線玩家各得的乙太。
pub const ANCIENT_ALPHA_GLOBAL_ETHER: u32 = 10;
/// 城鎮中心 X 座標（像素），確保古 Alpha 不在安全區內生成。
const TOWN_CENTER_X: f32 = 2336.0;
/// 城鎮中心 Y 座標（像素）。
const TOWN_CENTER_Y: f32 = 2272.0;
/// 古 Alpha 與城鎮中心的最小距離（像素）——確保在安全區外。
const ANCIENT_MIN_TOWN_DIST: f32 = 1600.0;

// ─── ROADMAP 176：物種霸主湧現常數 ───────────────────────────────────────────

/// 巢穴成為霸主所需的持續達標時間（秒），3 分鐘。
pub const DOMINANT_QUALIFY_SECS: f32 = 180.0;
/// 觸發霸主所需族群密度比例門檻（≥ 67% = 茂盛）。
const DOMINANT_MIN_POP_RATIO: f32 = 0.67;
/// 霸主存續期間額外生態壓力加成值。
pub const DOMINANT_PRESSURE_BONUS: f32 = 8.0;
/// 擊殺霸主巢穴普通怪物額外乙太獎勵。
pub const DOMINANT_KILL_BONUS_ETHER: u32 = 1;
/// 擊殺霸主巢穴 Alpha 額外乙太獎勵（疊加在 ALPHA_KILLER_ETHER 之上）。
pub const DOMINANT_ALPHA_BONUS_ETHER: u32 = 5;
/// 霸主解除後該巢穴的冷卻秒數（15 分鐘，避免連續稱霸）。
pub const DOMINANT_COOLDOWN_SECS: f32 = 15.0 * 60.0;

// ─── ROADMAP 179：怪物王號令援軍 ───────────────────────────────────────────────
/// 菁英 Alpha（覺醒或霸主）血量跌破此比例時，會號令巢穴援軍馳援。
/// 以「重傷」為觸發前提，等同要求玩家正在輸出——不會在無人交戰時憑空刷怪。
const ALPHA_SUMMON_HP_THRESHOLD: f32 = 0.5;
/// 兩次召喚之間的冷卻（秒），避免連續刷怪洗版。
const ALPHA_SUMMON_COOLDOWN_SECS: f32 = 45.0;
/// 每次召喚的援軍數量。
const ALPHA_SUMMON_COUNT: u32 = 3;
/// 援軍出生散佈半徑（像素，圍在王身邊）。
const ALPHA_SUMMON_RADIUS: f32 = 90.0;
/// 援軍兵力上限保護：該巢穴族群超過 max + 此值時暫停召喚（防病態洗怪）。
const ALPHA_SUMMON_MAX_EXTRA: u32 = 6;
/// 召喚指揮氣泡顯示文字（沿用 ROADMAP 169 前端指揮氣泡渲染）。
const SUMMON_TACTIC_NAME: &str = "號令援軍";

// ─── ROADMAP 183：族群潰逃 ─────────────────────────────────────────────────────
/// 族群人口（占上限比例）首次跌破此門檻時，殘兵士氣崩潰、潰逃回巢。
/// 與 colony_density 的「稀疏」界線（0.33）對齊：被打到稀疏即視為殘破。
pub const ROUT_FRACTION: f32 = 0.34;
/// 潰逃持續秒數：期間殘兵強制逃離玩家（沿用 retreat_timer 路徑），結束後自然回巢。
pub const ROUT_DURATION_SECS: f32 = 6.0;

/// 純函式：給定族群上限，回傳「潰逃門檻人口」——人口跌破此值即視為殘破。
/// 至少為 1，避免 max 很小時門檻歸零永不觸發。
pub fn rout_threshold(max_population: u32) -> u32 {
    ((max_population as f32 * ROUT_FRACTION).round() as u32).max(1)
}

// ─── 型別 ────────────────────────────────────────────────────────────────────

/// 單個怪物巢穴。
pub struct MonsterColony {
    pub id: u32,
    pub kind: EnemyKind,
    /// 巢穴顯示名稱（繁中）。
    pub name: &'static str,
    /// 巢穴中心世界座標 X（像素）。
    pub cx: f32,
    /// 巢穴中心世界座標 Y（像素）。
    pub cy: f32,
    /// 怪物出生散佈半徑（像素）。
    pub spawn_radius: f32,
    /// 目前活躍族群數（0 = 巢穴暫時廢棄）。
    pub population: u32,
    /// 最大族群容量。
    pub max_population: u32,
    /// 下次嘗試補充的倒數計時器（秒）。
    spawn_timer: f32,
    /// 累計生成次數，用作出生點散佈的鹽值（確保分佈不重疊）。
    spawn_count: u32,
    /// Alpha 被擊殺後的冷卻計時器（秒），>0 表示正在冷卻中。
    pub alpha_cooldown: f32,
}

/// 巢穴 Alpha 首領（ROADMAP 168 + 169 + 170）。
/// 單獨追蹤，不走 EnemyField。
pub struct ColonyAlpha {
    /// 全域唯一 ID（用於 AttackAlpha 訊息定位）。
    pub id: u32,
    /// 所屬巢穴 ID。
    pub colony_id: u32,
    /// 怪物種類（同巢穴）。
    pub kind: EnemyKind,
    /// 世界座標 X（在巢穴中心稍微偏移）。
    pub x: f32,
    /// 世界座標 Y。
    pub y: f32,
    /// 目前生命值。
    pub hp: u32,
    /// 最大生命值（= kind.max_hp() × 3）。
    pub max_hp: u32,
    /// 對應巢穴名稱（用於廣播）。
    pub colony_name: &'static str,
    // ROADMAP 169：咆哮指揮
    /// 距下次發出指揮指令倒數（秒）；首次設為 ALPHA_COMMAND_FIRST_WAIT_SECS。
    pub command_cooldown: f32,
    /// 當前指令名稱（繁中，如「包圍」），`None` 表示無指令。
    pub active_tactic: Option<String>,
    /// 當前指令剩餘顯示時間（秒）；到零後清除 active_tactic。
    tactic_remaining: f32,
    // ROADMAP 170：領地爭奪
    /// 正在與哪隻 Alpha 廝殺（對方的 alpha.id），`None` 表示無衝突。
    pub clash_target_id: Option<u32>,
    // ROADMAP 174：跨族結盟
    /// 盟約對象的 Alpha ID；`None` 表示未結盟。
    pub allied_to_id: Option<u32>,
    // ROADMAP 175：Alpha 覺醒危機
    /// 是否處於覺醒狀態（eco_pressure ≥ 85 且同場 Alpha ≥ 2 時激活）。
    pub awakened: bool,
}

/// 給協議層用的 Alpha 視圖（隨快照廣播）。
#[derive(Debug, Clone, Serialize)]
pub struct ColonyAlphaView {
    pub id: u32,
    pub colony_id: u32,
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub hp: u32,
    pub max_hp: u32,
    /// 當前指令名稱（繁中），無指令時為 `null`。前端用於顯示指揮氣泡。
    pub active_tactic: Option<String>,
    /// ROADMAP 170：正在廝殺的對方 Alpha ID；`null` 表示無衝突。前端顯示紅色衝突徽章。
    pub clash_target_id: Option<u32>,
    /// ROADMAP 174：盟約對象的 Alpha ID；`null` 表示未結盟。前端顯示金色連線與「盟」徽章。
    pub allied_to_id: Option<u32>,
    /// ROADMAP 175：是否處於覺醒狀態。前端顯示赤色外環 + 🔥👑 名牌。
    pub awakened: bool,
    /// ROADMAP 176：所屬巢穴是否為當前霸主。前端顯示 👑 徽章。
    pub is_dominant: bool,
}

/// 給協議層用的巢穴視圖（隨快照廣播，讓玩家在地圖/態度面板看到巢穴）。
#[derive(Debug, Clone, Serialize)]
pub struct MonsterColonyView {
    pub id: u32,
    pub kind: String,
    pub name: String,
    pub cx: f32,
    pub cy: f32,
    pub spawn_radius: f32,
    /// 族群密度：0=廢棄 1=稀疏 2=正常 3=茂盛（讓玩家有感而不顯示精確數字）。
    pub density: u32,
    /// 是否有 Alpha 首領活躍（前端顯示警示標記）。
    pub has_alpha: bool,
    /// ROADMAP 176：是否為當前霸主巢穴。前端顯示 👑 標記。
    pub is_dominant: bool,
}

/// Alpha 擊殺結果（由 game.rs 用於廣播/發獎）。
pub struct AlphaKilledResult {
    pub colony_id: u32,
    pub colony_name: &'static str,
    pub kind: EnemyKind,
    /// ROADMAP 174：擊殺時 Alpha 是否正在盟約中（是 → 額外獎勵）。
    pub was_allied: bool,
    /// ROADMAP 175：擊殺時 Alpha 是否處於覺醒狀態（是 → 額外獎勵）。
    pub was_awakened: bool,
    /// ROADMAP 176：擊殺時 Alpha 是否為霸主巢穴的 Alpha（是 → 額外獎勵）。
    pub was_dominant: bool,
    /// ROADMAP 183：所屬巢穴中心座標——供 ws.rs 斬首後對殘部觸發潰逃。
    pub cx: f32,
    pub cy: f32,
    /// ROADMAP 183：潰逃影響半徑（巢穴 spawn_radius）。
    pub rout_radius: f32,
}

// ─── ROADMAP 173：傳說古 Alpha ────────────────────────────────────────────────

/// 傳說古 Alpha：生態系頂點，3 個以上巢穴同時滿員時湧現的世界頭目。
pub struct AncientAlpha {
    /// 世界座標 X（像素）。
    pub x: f32,
    /// 世界座標 Y（像素）。
    pub y: f32,
    /// 目前生命值。
    pub hp: u32,
    /// 最大生命值（固定 ANCIENT_ALPHA_HP）。
    pub max_hp: u32,
}

/// 給協議層用的古 Alpha 視圖（隨快照廣播，前端渲染特殊世界頭目圖示）。
#[derive(Debug, Clone, Serialize)]
pub struct AncientAlphaView {
    pub x: f32,
    pub y: f32,
    pub hp: u32,
    pub max_hp: u32,
}

/// 古 Alpha 擊殺結果（由 ws.rs 用於發獎）。
pub struct AncientAlphaKilledResult;

/// 巢穴管理器發出的事件，由 game.rs 消化。
pub enum MonsterColonyEvent {
    /// 應在此座標注入一隻怪物（由 EnemyField::inject_event_enemy 執行）。
    SpawnAt { colony_id: u32, kind: EnemyKind, x: f32, y: f32 },
    /// 巢穴族群被清空（可廣播全服聊天）。
    ColonyCleared { name: &'static str, cx: f32, cy: f32 },
    /// 廢棄巢穴族群復生（可廣播全服聊天）。
    ColonyRevived { name: &'static str },
    /// 巢穴 Alpha 湧現（廣播全服通知玩家）。
    AlphaAppeared { colony_name: &'static str, kind: EnemyKind },
    /// Alpha 發出咆哮指揮（ROADMAP 169）：
    /// game.rs 計算附近玩家數→決定戰術→非同步呼叫 Groq 台詞→廣播。
    AlphaCommandReady {
        alpha_id: u32,
        colony_name: &'static str,
        kind: EnemyKind,
        /// 當前血量百分比（0.0~1.0），供 canned_tactic 判斷。
        hp_pct: f32,
        alpha_x: f32,
        alpha_y: f32,
    },
    /// ROADMAP 170：兩隻不同巢穴的 Alpha 首次進入衝突半徑，廣播開戰通知。
    AlphaClashStart {
        colony_a_name: &'static str,
        colony_b_name: &'static str,
    },
    /// ROADMAP 170：Alpha 領地衝突結束，敗者 Alpha 倒下。
    /// game.rs 負責更新敗者巢穴族群 + 廣播勝利訊息。
    AlphaClashVictory {
        winner_colony_name: &'static str,
        loser_colony_name: &'static str,
        /// 敗者巢穴 ID，game.rs 用於更新族群計數與冷卻。
        loser_colony_id: u32,
    },
    /// ROADMAP 172：玩家在某個巢穴附近擊殺一隻怪物，族群 -1。
    /// ws.rs 將此事件轉給 EcoBountyState::on_colony_kill()。
    MonsterKilledInColony { colony_id: u32 },
    /// ROADMAP 173：傳說古 Alpha 降臨——3 個以上巢穴同時滿員時湧現。
    /// game.rs 負責廣播降臨訊息並非同步呼叫 Groq 生成宣言台詞。
    AncientAlphaEmerged { x: f32, y: f32 },
    /// ROADMAP 173：傳說古 Alpha 被擊倒。game.rs 負責廣播勝利訊息。
    AncientAlphaSlain,
    /// ROADMAP 174：跨族結盟達成——兩隻不同巢穴的 Alpha 共存 3 分鐘後締結盟約。
    /// game.rs 負責廣播警告 + 生態壓力加成。
    AllianceFormed {
        alpha_a_name: &'static str,
        alpha_b_name: &'static str,
    },
    /// ROADMAP 174：跨族結盟瓦解——某隻盟約 Alpha 被玩家擊殺。
    /// game.rs 負責廣播破盟訊息 + 發送額外乙太。
    AllianceBroken {
        survivor_name: &'static str,
    },
    /// ROADMAP 175：Alpha 覺醒危機——壓力衝頂且多隻 Alpha 同場，全員進入覺醒狀態。
    /// game.rs 負責廣播全服警報。
    AlphaAwakened { count: usize },
    /// ROADMAP 176：巢穴稱霸宣告——持續維持高密度族群 + Alpha 達 3 分鐘。
    /// game.rs 負責廣播全服警示；ws.rs 收到後對該巢穴殺怪加乙太獎勵。
    DominanceDeclaration { colony_id: u32, colony_name: &'static str },
    /// ROADMAP 176：霸主落幕——族群跌落或霸主 Alpha 被擊殺（由 attack_alpha 清除後事件）。
    /// game.rs 負責廣播全服消息。
    DominanceBroken { colony_id: u32, colony_name: &'static str },
    /// ROADMAP 176：玩家在霸主巢穴附近擊殺普通怪物——ws.rs 給擊殺者 +1 乙太。
    MonsterKilledInDominantColony,
    /// ROADMAP 179：菁英 Alpha（覺醒或霸主）受重傷時號令巢穴援軍馳援。
    /// game.rs 對每個 position 呼叫 inject_event_enemy 注入援軍，並廣播全服警示。
    AlphaSummonedReinforcements {
        colony_name: &'static str,
        kind: EnemyKind,
        count: u32,
        positions: Vec<(f32, f32)>,
    },
    /// ROADMAP 183：族群被打殘（人口首次跌破 ROUT_FRACTION）→ 殘兵潰逃。
    /// ws.rs 收到後對 (cx,cy) radius 內同種怪呼叫 EnemyField::rout_region 並廣播。
    ColonyRouted {
        name: &'static str,
        kind: EnemyKind,
        cx: f32,
        cy: f32,
        /// 潰逃影響半徑（採巢穴 spawn_radius）。
        radius: f32,
    },
}

/// 管理所有怪物巢穴。
pub struct MonsterColonyManager {
    pub colonies: Vec<MonsterColony>,
    /// 當前活躍的 Alpha 首領（同一巢穴至多 1 隻）。
    pub alphas: Vec<ColonyAlpha>,
    /// 下一個 Alpha 的全域唯一 ID 計數器。
    next_alpha_id: u32,
    // ROADMAP 173：傳說古 Alpha
    /// 當前活躍的傳說古 Alpha（全局唯一，`None` = 已死亡或尚未湧現）。
    pub ancient: Option<AncientAlpha>,
    /// 古 Alpha 被擊倒後的冷卻計時器（秒），>0 = 冷卻中，不觸發湧現判斷。
    pub ancient_cooldown: f32,
    // ROADMAP 174：跨族結盟
    /// 兩隻以上不同巢穴 Alpha 共存（未廝殺）的累計秒數；達 ALLIANCE_FORM_SECS 觸發結盟。
    coexistence_timer: f32,
    /// 當前是否有跨族結盟活躍。
    alliance_active: bool,
    /// 結盟的兩個 Alpha ID（僅 alliance_active = true 時有效）。
    alliance_pair: [u32; 2],
    // ROADMAP 176：物種霸主
    /// 當前稱霸的巢穴 ID；None = 無霸主。
    dominant_colony_id: Option<u32>,
    /// 候選巢穴 ID（持續達標但計時未到）；None = 無候選。
    candidate_colony_id: Option<u32>,
    /// 候選巢穴已連續達標的秒數。
    dominant_qualify_timer: f32,
    /// 各巢穴霸主冷卻剩餘秒數（剛失去霸主後需冷卻，避免連續稱霸）。
    dominant_cooldowns: std::collections::HashMap<u32, f32>,
    // ROADMAP 179：怪物王號令援軍
    /// 各 Alpha 的召喚冷卻剩餘秒數（key = alpha.id）；>0 表示冷卻中不再召喚。
    alpha_summon_cd: std::collections::HashMap<u32, f32>,
}

impl MonsterColonyManager {
    pub fn new() -> Self {
        Self {
            colonies: build_colonies(),
            alphas: Vec::new(),
            next_alpha_id: 1,
            ancient: None,
            ancient_cooldown: 0.0,
            coexistence_timer: 0.0,
            alliance_active: false,
            alliance_pair: [0, 0],
            dominant_colony_id: None,
            candidate_colony_id: None,
            dominant_qualify_timer: 0.0,
            dominant_cooldowns: std::collections::HashMap::new(),
            alpha_summon_cd: std::collections::HashMap::new(),
        }
    }

    /// 每幀推進：族群補充 + Alpha 冷卻倒數 + Alpha 湧現 + Alpha 指揮計時 + Alpha 領地爭奪。
    pub fn tick(&mut self, dt: f32, eco_pressure: f32) -> Vec<MonsterColonyEvent> {
        let mut events = Vec::new();

        // 族群補充 + Alpha 冷卻倒數
        for col in &mut self.colonies {
            if col.alpha_cooldown > 0.0 {
                col.alpha_cooldown = (col.alpha_cooldown - dt).max(0.0);
            }
            if col.population >= col.max_population {
                continue;
            }
            col.spawn_timer -= dt;
            if col.spawn_timer > 0.0 {
                continue;
            }
            col.spawn_timer = RESPAWN_SECS;
            let was_empty = col.population == 0;
            col.population += 1;
            col.spawn_count += 1;
            let (sx, sy) = colony_spawn_pos(col);
            events.push(MonsterColonyEvent::SpawnAt { colony_id: col.id, kind: col.kind, x: sx, y: sy });
            if was_empty {
                events.push(MonsterColonyEvent::ColonyRevived { name: col.name });
            }
        }

        // ROADMAP 169：Alpha 咆哮指揮計時——倒數並在歸零時發出 AlphaCommandReady
        for alpha in &mut self.alphas {
            // 指令顯示計時：到期則清除前端氣泡
            if alpha.tactic_remaining > 0.0 {
                alpha.tactic_remaining -= dt;
                if alpha.tactic_remaining <= 0.0 {
                    alpha.active_tactic = None;
                }
            }
            // 指揮冷卻：歸零則觸發一次指令
            alpha.command_cooldown -= dt;
            if alpha.command_cooldown <= 0.0 {
                let hp_pct = alpha.hp as f32 / alpha.max_hp.max(1) as f32;
                events.push(MonsterColonyEvent::AlphaCommandReady {
                    alpha_id: alpha.id,
                    colony_name: alpha.colony_name,
                    kind: alpha.kind,
                    hp_pct,
                    alpha_x: alpha.x,
                    alpha_y: alpha.y,
                });
                // 重置冷卻（game.rs 呼叫 set_alpha_tactic 不再重複重置）
                alpha.command_cooldown = ALPHA_COMMAND_COOLDOWN_SECS;
            }
        }

        // Alpha 湧現：族群已滿 + 無活躍 Alpha + 冷卻結束（兩遍避免借用衝突）
        let to_spawn: Vec<(u32, EnemyKind, f32, f32, u32, &'static str)> = self.colonies.iter()
            .filter(|col| {
                col.population >= col.max_population
                    && col.alpha_cooldown <= 0.0
                    && !self.alphas.iter().any(|a| a.colony_id == col.id)
            })
            .map(|col| {
                let (ax, ay) = alpha_spawn_pos(col);
                (col.id, col.kind, ax, ay, col.kind.max_hp() * 3, col.name)
            })
            .collect();

        for (colony_id, kind, ax, ay, max_hp, colony_name) in to_spawn {
            events.push(MonsterColonyEvent::AlphaAppeared { colony_name, kind });
            let alpha_id = self.next_alpha_id;
            self.next_alpha_id += 1;
            self.alphas.push(ColonyAlpha {
                id: alpha_id, colony_id, kind, x: ax, y: ay,
                hp: max_hp, max_hp, colony_name,
                command_cooldown: ALPHA_COMMAND_FIRST_WAIT_SECS,
                active_tactic: None,
                tactic_remaining: 0.0,
                clash_target_id: None,
                allied_to_id: None,
                awakened: false,
            });
        }

        // ROADMAP 170：Alpha 領地爭奪——兩兩偵測 + 互相施傷 + 結算
        self.tick_alpha_clash(dt, &mut events);

        // ROADMAP 174：跨族結盟——共存計時 + 結盟觸發 + 盟約失效偵測
        self.tick_alliance(dt, &mut events);

        // ROADMAP 173：傳說古 Alpha 湧現判斷
        self.tick_ancient_alpha(dt, &mut events);

        // ROADMAP 175：Alpha 覺醒危機——壓力衝頂時覺醒所有 Alpha
        self.tick_awakening(eco_pressure, &mut events);

        // ROADMAP 176：物種霸主——族群茂盛 + Alpha 持續 3 分鐘則稱霸
        self.tick_dominance(dt, &mut events);

        // ROADMAP 179：怪物王號令援軍——菁英 Alpha 受重傷召喚巢穴援軍
        self.tick_alpha_summon(dt, &mut events);

        events
    }

    /// ROADMAP 170：偵測所有不同巢穴 Alpha 對的領地衝突，施加傷害並結算。
    fn tick_alpha_clash(&mut self, dt: f32, events: &mut Vec<MonsterColonyEvent>) {
        let n = self.alphas.len();
        if n < 2 {
            return;
        }

        // 第一遍：收集衝突對，決定傷害量與新衝突通知
        // 分離收集以避免可變借用衝突
        let mut damage_vec: Vec<(usize, u32)> = Vec::new();
        let mut clash_starts: Vec<(&'static str, &'static str)> = Vec::new();

        for i in 0..n {
            for j in (i + 1)..n {
                if self.alphas[i].colony_id == self.alphas[j].colony_id {
                    continue; // 同巢穴不相殺
                }
                let dx = self.alphas[i].x - self.alphas[j].x;
                let dy = self.alphas[i].y - self.alphas[j].y;
                if dx * dx + dy * dy > ALPHA_CLASH_RADIUS * ALPHA_CLASH_RADIUS {
                    continue; // 超出衝突半徑
                }
                let aj_id = self.alphas[j].id;
                let ai_id = self.alphas[i].id;
                // 首次進入範圍才發廣播
                if self.alphas[i].clash_target_id != Some(aj_id) {
                    clash_starts.push((self.alphas[i].colony_name, self.alphas[j].colony_name));
                }
                self.alphas[i].clash_target_id = Some(aj_id);
                self.alphas[j].clash_target_id = Some(ai_id);
                let dmg = ((ALPHA_CLASH_DAMAGE_PER_SEC * dt) as u32).max(1);
                damage_vec.push((i, dmg));
                damage_vec.push((j, dmg));
            }
        }

        for (a_name, b_name) in clash_starts {
            events.push(MonsterColonyEvent::AlphaClashStart {
                colony_a_name: a_name,
                colony_b_name: b_name,
            });
        }

        // 第二遍：套用傷害
        for (idx, dmg) in damage_vec {
            self.alphas[idx].hp = self.alphas[idx].hp.saturating_sub(dmg);
        }

        // 第三遍：找出因衝突歸零的 Alpha，結算勝負
        let dead_ids: Vec<u32> = self.alphas.iter()
            .filter(|a| a.hp == 0 && a.clash_target_id.is_some())
            .map(|a| a.id)
            .collect();

        for &dead_id in &dead_ids {
            let (loser_name, loser_colony_id) = match self.alphas.iter().find(|a| a.id == dead_id) {
                Some(a) => (a.colony_name, a.colony_id),
                None => continue,
            };
            let winner_name: &'static str = self.alphas.iter()
                .find(|a| a.clash_target_id == Some(dead_id) && a.id != dead_id)
                .map(|a| a.colony_name)
                .unwrap_or("未知");
            events.push(MonsterColonyEvent::AlphaClashVictory {
                winner_colony_name: winner_name,
                loser_colony_name: loser_name,
                loser_colony_id,
            });
            // 敗者巢穴族群衰退 + 加長冷卻
            if let Some(col) = self.colonies.iter_mut().find(|c| c.id == loser_colony_id) {
                col.population = col.population.saturating_sub(2);
                col.alpha_cooldown = ALPHA_COOLDOWN_SECS * ALPHA_CLASH_DEFEAT_COOLDOWN_MULT;
            }
        }

        // 移除陣亡的 Alpha，並清除存活者對已消失 Alpha 的引用
        if !dead_ids.is_empty() {
            self.alphas.retain(|a| !dead_ids.contains(&a.id));
            let alive_ids: std::collections::HashSet<u32> =
                self.alphas.iter().map(|a| a.id).collect();
            for alpha in &mut self.alphas {
                if let Some(tid) = alpha.clash_target_id {
                    if !alive_ids.contains(&tid) {
                        alpha.clash_target_id = None;
                    }
                }
            }
        }
    }

    /// ROADMAP 179：怪物王號令援軍——菁英 Alpha（覺醒或霸主）受重傷時召喚巢穴援軍。
    ///
    /// 設計重點：
    /// - **只有菁英級**（`awakened` 或所屬巢穴稱霸）才召喚——守成本紀律，大眾 Alpha 不刷怪。
    /// - **重傷才召喚**（HP < 50%）——血量只在玩家攻擊時下降，故等同「玩家正在交戰」的前提，
    ///   不會在無人荒野憑空生怪。
    /// - **冷卻 + 兵力上限**雙重保護，避免病態洗版。
    /// - 援軍走既有 `inject_event_enemy` 管線（由 game.rs 消化事件），零戰鬥/移動架構改動。
    fn tick_alpha_summon(&mut self, dt: f32, events: &mut Vec<MonsterColonyEvent>) {
        // 冷卻倒數 + 清除已不存在 Alpha 的殘留計時（避免 map 無限增長）。
        let alive: std::collections::HashSet<u32> = self.alphas.iter().map(|a| a.id).collect();
        self.alpha_summon_cd.retain(|id, _| alive.contains(id));
        for cd in self.alpha_summon_cd.values_mut() {
            *cd = (*cd - dt).max(0.0);
        }

        let dominant = self.dominant_colony_id;
        // 第一遍：挑出符合召喚條件的 Alpha（先收集避免可變借用衝突）。
        let mut to_summon: Vec<(u32, u32)> = Vec::new(); // (alpha_id, colony_id)
        for a in &self.alphas {
            let is_elite = a.awakened || dominant == Some(a.colony_id);
            let hp_pct = a.hp as f32 / a.max_hp.max(1) as f32;
            let on_cd = self.alpha_summon_cd.get(&a.id).copied().unwrap_or(0.0) > 0.0;
            if is_elite && a.hp > 0 && hp_pct < ALPHA_SUMMON_HP_THRESHOLD && !on_cd {
                to_summon.push((a.id, a.colony_id));
            }
        }

        // 第二遍：執行召喚。
        for (alpha_id, colony_id) in to_summon {
            // 取得王身座標作為援軍散佈圓心。
            let Some((ax, ay, kind, colony_name)) = self.alphas.iter()
                .find(|a| a.id == alpha_id)
                .map(|a| (a.x, a.y, a.kind, a.colony_name))
            else { continue };

            // 兵力上限保護：族群已遠超容量則暫停召喚（仍進冷卻，避免每幀重判）。
            let over_cap = self.colonies.iter()
                .find(|c| c.id == colony_id)
                .map(|c| c.population >= c.max_population + ALPHA_SUMMON_MAX_EXTRA)
                .unwrap_or(true);
            if over_cap {
                self.alpha_summon_cd.insert(alpha_id, ALPHA_SUMMON_COOLDOWN_SECS);
                continue;
            }

            // 散佈援軍出生點（圍在王身邊）。
            let mut positions = Vec::with_capacity(ALPHA_SUMMON_COUNT as usize);
            for k in 0..ALPHA_SUMMON_COUNT {
                let (ox, oy) = summon_offset(alpha_id, k);
                positions.push((ax + ox, ay + oy));
            }
            let count = positions.len() as u32;

            // 族群計數同步增加（援軍是「額外兵力」，允許暫時超過 max；死亡後自然回落，
            // 期間自然暫停一般補充——符合「王把族群全叫出來護駕」的直覺）。
            if let Some(col) = self.colonies.iter_mut().find(|c| c.id == colony_id) {
                col.population += count;
            }
            // 設定指揮氣泡（沿用 ROADMAP 169 前端渲染，無需新前端碼）。
            if let Some(a) = self.alphas.iter_mut().find(|a| a.id == alpha_id) {
                a.active_tactic = Some(SUMMON_TACTIC_NAME.to_string());
                a.tactic_remaining = ALPHA_TACTIC_DURATION_SECS;
            }
            self.alpha_summon_cd.insert(alpha_id, ALPHA_SUMMON_COOLDOWN_SECS);

            events.push(MonsterColonyEvent::AlphaSummonedReinforcements {
                colony_name, kind, count, positions,
            });
        }
    }

    /// 設定指定 Alpha 的當前指令（game.rs 在 AlphaCommandReady 後同步呼叫）。
    /// 前端在 `active_tactic` 非 None 期間顯示指揮氣泡（ROADMAP 169）。
    pub fn set_alpha_tactic(&mut self, alpha_id: u32, tactic_name: String) {
        if let Some(a) = self.alphas.iter_mut().find(|a| a.id == alpha_id) {
            a.active_tactic = Some(tactic_name);
            a.tactic_remaining = ALPHA_TACTIC_DURATION_SECS;
        }
    }

    /// 玩家攻擊指定 Alpha（id = alpha.id，在距離 reach 內有效）。
    /// 傳回 `Some(AlphaKilledResult)` 代表 Alpha 被擊殺，否則為 `None`（未死或找不到）。
    pub fn attack_alpha(
        &mut self,
        alpha_id: u32,
        px: f32,
        py: f32,
        power: u32,
        reach: f32,
    ) -> Option<AlphaKilledResult> {
        let reach_sq = reach * reach;
        let idx = self.alphas.iter().position(|a| {
            a.id == alpha_id
                && (a.x - px).powi(2) + (a.y - py).powi(2) <= reach_sq
        })?;

        let alpha = &mut self.alphas[idx];
        alpha.hp = alpha.hp.saturating_sub(power);
        if alpha.hp > 0 {
            return None; // 未死
        }

        // Alpha 死亡：記錄盟約狀態、覺醒狀態、霸主狀態
        let was_allied = self.alphas[idx].allied_to_id.is_some();
        let was_awakened = self.alphas[idx].awakened;
        let was_dominant = self.dominant_colony_id == Some(self.alphas[idx].colony_id);
        // ROADMAP 183：取所屬巢穴幾何，供斬首後對殘部觸發潰逃（巢穴可能已被清空 → 退回 Alpha 自身位置）。
        let colony_id = self.alphas[idx].colony_id;
        let (rout_cx, rout_cy, rout_radius) = self.colonies.iter()
            .find(|c| c.id == colony_id)
            .map(|c| (c.cx, c.cy, c.spawn_radius))
            .unwrap_or((self.alphas[idx].x, self.alphas[idx].y, COLONY_KILL_RADIUS));
        let result = AlphaKilledResult {
            colony_id: self.alphas[idx].colony_id,
            colony_name: self.alphas[idx].colony_name,
            kind: self.alphas[idx].kind,
            was_allied,
            was_awakened,
            was_dominant,
            cx: rout_cx,
            cy: rout_cy,
            rout_radius,
        };
        self.alphas.swap_remove(idx);

        // 對應巢穴族群 -2（最少 0）並設冷卻
        if let Some(col) = self.colonies.iter_mut().find(|c| c.id == result.colony_id) {
            col.population = col.population.saturating_sub(2);
            col.alpha_cooldown = ALPHA_COOLDOWN_SECS;
        }

        // ROADMAP 176：若擊殺的是霸主 Alpha，立即解除霸主並設冷卻
        if was_dominant {
            self.dominant_colony_id = None;
            self.candidate_colony_id = None;
            self.dominant_qualify_timer = 0.0;
            self.dominant_cooldowns.insert(result.colony_id, DOMINANT_COOLDOWN_SECS);
        }

        Some(result)
    }

    // ── ROADMAP 173：傳說古 Alpha 相關方法 ──────────────────────────────────

    /// ROADMAP 174：推進跨族結盟邏輯：
    /// 1. 若已結盟 → 偵測盟約是否因 Alpha 死亡而失效。
    /// 2. 若未結盟 → 計算正在和平共存（無廝殺）的異巢穴 Alpha 對；達 ALLIANCE_FORM_SECS 觸發結盟。
    fn tick_alliance(&mut self, dt: f32, events: &mut Vec<MonsterColonyEvent>) {
        if self.alliance_active {
            // 確認兩隻盟約 Alpha 是否仍存活
            let [id_a, id_b] = self.alliance_pair;
            let a_alive = self.alphas.iter().any(|a| a.id == id_a);
            let b_alive = self.alphas.iter().any(|a| a.id == id_b);
            if !a_alive || !b_alive {
                // 盟約 Alpha 死亡 → 結盟瓦解
                let survivor_name: &'static str = if a_alive {
                    self.alphas.iter().find(|a| a.id == id_a)
                        .map(|a| a.colony_name).unwrap_or("未知")
                } else if b_alive {
                    self.alphas.iter().find(|a| a.id == id_b)
                        .map(|a| a.colony_name).unwrap_or("未知")
                } else {
                    "未知"
                };
                self.alliance_active = false;
                self.alliance_pair = [0, 0];
                // 清除所有 Alpha 的 allied_to_id
                for alpha in &mut self.alphas {
                    alpha.allied_to_id = None;
                }
                events.push(MonsterColonyEvent::AllianceBroken { survivor_name });
            }
            return;
        }

        // 未結盟：計算可結盟的 Alpha 對（不同巢穴、互無廝殺）
        let peaceful: Vec<usize> = (0..self.alphas.len())
            .filter(|&i| self.alphas[i].clash_target_id.is_none())
            .collect();

        // 找出第一對不同巢穴的 peaceful Alpha
        let mut candidate_pair: Option<(usize, usize)> = None;
        'outer: for i in 0..peaceful.len() {
            for j in (i + 1)..peaceful.len() {
                let ai = peaceful[i];
                let aj = peaceful[j];
                if self.alphas[ai].colony_id != self.alphas[aj].colony_id {
                    candidate_pair = Some((ai, aj));
                    break 'outer;
                }
            }
        }

        if candidate_pair.is_some() {
            self.coexistence_timer += dt;
            if self.coexistence_timer >= ALLIANCE_FORM_SECS {
                let (ai, aj) = candidate_pair.unwrap();
                let id_a = self.alphas[ai].id;
                let id_b = self.alphas[aj].id;
                let name_a = self.alphas[ai].colony_name;
                let name_b = self.alphas[aj].colony_name;
                // 盟約加持：各補 ALLIANCE_HP_BONUS 點生命（不超過最大值的 2 倍為上限）
                self.alphas[ai].hp = (self.alphas[ai].hp + ALLIANCE_HP_BONUS)
                    .min(self.alphas[ai].max_hp * 2);
                self.alphas[aj].hp = (self.alphas[aj].hp + ALLIANCE_HP_BONUS)
                    .min(self.alphas[aj].max_hp * 2);
                self.alphas[ai].allied_to_id = Some(id_b);
                self.alphas[aj].allied_to_id = Some(id_a);
                self.alliance_active = true;
                self.alliance_pair = [id_a, id_b];
                self.coexistence_timer = 0.0;
                events.push(MonsterColonyEvent::AllianceFormed {
                    alpha_a_name: name_a,
                    alpha_b_name: name_b,
                });
            }
        } else {
            // 沒有可結盟對 → 重置計時
            self.coexistence_timer = 0.0;
        }
    }

    /// 回傳跨族結盟是否活躍（供 game.rs 計算額外生態壓力加成）。
    pub fn alliance_active(&self) -> bool {
        self.alliance_active
    }

    /// ROADMAP 175：覺醒危機——壓力衝頂且 Alpha 達數量門檻時全員覺醒；壓力回落後解除。
    fn tick_awakening(&mut self, eco_pressure: f32, events: &mut Vec<MonsterColonyEvent>) {
        if self.alphas.is_empty() {
            return;
        }
        if eco_pressure >= ALPHA_AWAKENING_PRESSURE && self.alphas.len() >= ALPHA_AWAKENING_MIN_COUNT {
            // 找出尚未覺醒的 Alpha，進行覺醒加持
            let newly_awakened: Vec<usize> = self.alphas.iter()
                .enumerate()
                .filter(|(_, a)| !a.awakened)
                .map(|(i, _)| i)
                .collect();
            if !newly_awakened.is_empty() {
                for i in newly_awakened {
                    let a = &mut self.alphas[i];
                    a.awakened = true;
                    // HP 加成 50%（上限 1.5× max_hp）
                    let cap = a.max_hp + a.max_hp / 2;
                    let bonus = (a.max_hp as f32 * ALPHA_AWAKENING_HP_BONUS) as u32;
                    a.hp = (a.hp + bonus).min(cap);
                }
                let count = self.alphas.len();
                events.push(MonsterColonyEvent::AlphaAwakened { count });
            }
        } else if eco_pressure < ALPHA_DEAWAKEN_PRESSURE {
            // 壓力回落，解除覺醒（HP 不恢復）
            for a in &mut self.alphas {
                a.awakened = false;
            }
        }
    }

    /// ROADMAP 176：推進物種霸主邏輯。
    /// 條件：有 Alpha + 族群比例 ≥ 67%（密度茂盛）持續 3 分鐘 → 稱霸廣播。
    /// 霸主 Alpha 被擊殺由 attack_alpha() 直接清除；族群衰退則在此偵測。
    fn tick_dominance(&mut self, dt: f32, events: &mut Vec<MonsterColonyEvent>) {
        // 更新各巢穴霸主冷卻倒數
        for cd in self.dominant_cooldowns.values_mut() {
            *cd = (*cd - dt).max(0.0);
        }

        // 若已有霸主：確認是否仍符合條件（族群比例 ≥ 閾值 且 Alpha 存活）
        if let Some(dom_id) = self.dominant_colony_id {
            let still_ok = self.colonies.iter()
                .find(|c| c.id == dom_id)
                .map(|c| {
                    c.max_population > 0
                        && c.population as f32 / c.max_population as f32 >= DOMINANT_MIN_POP_RATIO
                        && self.alphas.iter().any(|a| a.colony_id == dom_id)
                })
                .unwrap_or(false);
            if !still_ok {
                // 族群衰退或 Alpha 消失（廝殺等非擊殺路徑）→ 解除霸主
                let name = self.colonies.iter()
                    .find(|c| c.id == dom_id)
                    .map(|c| c.name)
                    .unwrap_or("未知");
                self.dominant_colony_id = None;
                self.candidate_colony_id = None;
                self.dominant_qualify_timer = 0.0;
                self.dominant_cooldowns.insert(dom_id, DOMINANT_COOLDOWN_SECS);
                events.push(MonsterColonyEvent::DominanceBroken { colony_id: dom_id, colony_name: name });
            }
            // 有霸主時不尋找新候選
            return;
        }

        // 尋找候選：族群比例 ≥ 閾值 + 有 Alpha + 無冷卻
        let candidate = self.colonies.iter()
            .filter(|c| {
                c.max_population > 0
                    && c.population as f32 / c.max_population as f32 >= DOMINANT_MIN_POP_RATIO
                    && self.alphas.iter().any(|a| a.colony_id == c.id)
                    && self.dominant_cooldowns.get(&c.id).copied().unwrap_or(0.0) == 0.0
            })
            .map(|c| c.id)
            .next();

        match candidate {
            None => {
                // 無候選，重置
                self.candidate_colony_id = None;
                self.dominant_qualify_timer = 0.0;
            }
            Some(cid) => {
                if self.candidate_colony_id != Some(cid) {
                    // 候選換了，重置計時
                    self.candidate_colony_id = Some(cid);
                    self.dominant_qualify_timer = 0.0;
                }
                self.dominant_qualify_timer += dt;
                if self.dominant_qualify_timer >= DOMINANT_QUALIFY_SECS {
                    let name = self.colonies.iter()
                        .find(|c| c.id == cid)
                        .map(|c| c.name)
                        .unwrap_or("未知");
                    self.dominant_colony_id = Some(cid);
                    self.candidate_colony_id = None;
                    self.dominant_qualify_timer = 0.0;
                    events.push(MonsterColonyEvent::DominanceDeclaration { colony_id: cid, colony_name: name });
                }
            }
        }
    }

    /// 每幀推進古 Alpha 冷卻 + 湧現判斷。
    /// 當 `ancient` 為 `None` 且冷卻歸零、且 3+ 巢穴飽和度 ≥ 80% 時，湧現古 Alpha。
    fn tick_ancient_alpha(&mut self, dt: f32, events: &mut Vec<MonsterColonyEvent>) {
        // 1. 古 Alpha 存活中：不做任何事（玩家攻擊負責減 HP）。
        if self.ancient.is_some() {
            return;
        }
        // 2. 冷卻倒數。
        if self.ancient_cooldown > 0.0 {
            self.ancient_cooldown = (self.ancient_cooldown - dt).max(0.0);
            return;
        }
        // 3. 冷卻歸零：檢查是否有足夠多的「滿員」巢穴。
        let full_colonies: Vec<(f32, f32)> = self.colonies.iter()
            .filter(|c| {
                c.max_population > 0
                    && c.population as f32 / c.max_population as f32 >= ANCIENT_SATURATION_THRESHOLD
            })
            .map(|c| (c.cx, c.cy))
            .collect();

        if full_colonies.len() < ANCIENT_ALPHA_MIN_FULL_COLONIES {
            return;
        }

        // 4. 計算滿員巢穴的幾何中心，並確保不在城鎮安全區內。
        let n = full_colonies.len() as f32;
        let cx = full_colonies.iter().map(|(x, _)| x).sum::<f32>() / n;
        let cy = full_colonies.iter().map(|(_, y)| y).sum::<f32>() / n;
        let dx = cx - TOWN_CENTER_X;
        let dy = cy - TOWN_CENTER_Y;
        let dist = (dx * dx + dy * dy).sqrt().max(1.0);
        let (ax, ay) = if dist < ANCIENT_MIN_TOWN_DIST {
            let scale = ANCIENT_MIN_TOWN_DIST / dist;
            (TOWN_CENTER_X + dx * scale, TOWN_CENTER_Y + dy * scale)
        } else {
            (cx, cy)
        };

        // 5. 湧現！
        self.ancient = Some(AncientAlpha { x: ax, y: ay, hp: ANCIENT_ALPHA_HP, max_hp: ANCIENT_ALPHA_HP });
        events.push(MonsterColonyEvent::AncientAlphaEmerged { x: ax, y: ay });
    }

    /// 玩家對傳說古 Alpha 發動一次攻擊。
    /// 需：未倒地、距離 ≤ ANCIENT_ALPHA_ATTACK_REACH、古 Alpha 存活。
    /// 回傳 `Some(AncientAlphaKilledResult)` 代表擊倒，`None` 代表未死或條件不符。
    pub fn attack_ancient_alpha(
        &mut self,
        px: f32,
        py: f32,
        power: u32,
    ) -> Option<AncientAlphaKilledResult> {
        let ancient = self.ancient.as_mut()?;
        let reach_sq = ANCIENT_ALPHA_ATTACK_REACH * ANCIENT_ALPHA_ATTACK_REACH;
        let dist_sq = (ancient.x - px).powi(2) + (ancient.y - py).powi(2);
        if dist_sq > reach_sq {
            return None; // 距離不足
        }
        ancient.hp = ancient.hp.saturating_sub(power);
        if ancient.hp > 0 {
            return None; // 未死
        }
        // 古 Alpha 死亡：清除實體、設冷卻。
        self.ancient = None;
        self.ancient_cooldown = ANCIENT_ALPHA_COOLDOWN_SECS;
        Some(AncientAlphaKilledResult)
    }

    /// 回傳供快照廣播的古 Alpha 視圖（`None` 代表目前無古 Alpha 活躍）。
    pub fn ancient_alpha_view(&self) -> Option<AncientAlphaView> {
        self.ancient.as_ref().map(|a| AncientAlphaView {
            x: a.x, y: a.y, hp: a.hp, max_hp: a.max_hp,
        })
    }

    /// 玩家在 (kill_x, kill_y) 擊殺了 kill_kind 種類的怪 →
    /// 找最近的同類巢穴（在 COLONY_KILL_RADIUS 內）並扣族群數。
    pub fn on_monster_killed_near(
        &mut self,
        kill_x: f32,
        kill_y: f32,
        kill_kind: EnemyKind,
    ) -> Vec<MonsterColonyEvent> {
        let mut events = Vec::new();
        let radius_sq = COLONY_KILL_RADIUS * COLONY_KILL_RADIUS;
        let mut best: Option<usize> = None;
        let mut best_dist_sq = radius_sq;
        for (idx, col) in self.colonies.iter().enumerate() {
            if col.kind != kill_kind || col.population == 0 {
                continue;
            }
            let dx = col.cx - kill_x;
            let dy = col.cy - kill_y;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best = Some(idx);
            }
        }
        if let Some(idx) = best {
            let col = &mut self.colonies[idx];
            let pop_before = col.population;
            col.population -= 1;
            let colony_id = col.id;
            // ROADMAP 172：通知生態清剿委託此巢穴被擊殺了一隻怪物。
            events.push(MonsterColonyEvent::MonsterKilledInColony { colony_id });
            // ROADMAP 183：族群人口首次跌破潰逃門檻（且尚未歸零）→ 殘兵士氣崩潰、潰逃回巢。
            // 只在「跨過門檻的那一刀」觸發一次，避免之後每刀洗版；歸零走下方 ColonyCleared、不潰逃。
            let thresh = rout_threshold(col.max_population);
            if col.population > 0 && col.population < thresh && pop_before >= thresh {
                events.push(MonsterColonyEvent::ColonyRouted {
                    name: col.name,
                    kind: col.kind,
                    cx: col.cx,
                    cy: col.cy,
                    radius: col.spawn_radius,
                });
            }
            // ROADMAP 176：若為霸主巢穴，額外發出霸主擊殺事件供 ws.rs 給乙太獎勵。
            if self.dominant_colony_id == Some(colony_id) {
                events.push(MonsterColonyEvent::MonsterKilledInDominantColony);
            }
            if col.population == 0 {
                // 巢穴清空：加長冷卻再復生；同時移除 Alpha（族群歸零 Alpha 也消失）
                col.spawn_timer = RESPAWN_SECS * WIPED_COOLDOWN_MULT;
                self.alphas.retain(|a| a.colony_id != col.id);
                events.push(MonsterColonyEvent::ColonyCleared { name: col.name, cx: col.cx, cy: col.cy });
            }
        }
        events
    }

    /// 回傳供快照廣播的 Alpha 視圖清單。
    pub fn alpha_views(&self) -> Vec<ColonyAlphaView> {
        self.alphas.iter().map(|a| ColonyAlphaView {
            id: a.id,
            colony_id: a.colony_id,
            kind: a.kind.as_str().to_string(),
            x: a.x,
            y: a.y,
            hp: a.hp,
            max_hp: a.max_hp,
            active_tactic: a.active_tactic.clone(),
            clash_target_id: a.clash_target_id,
            allied_to_id: a.allied_to_id,
            awakened: a.awakened,
            is_dominant: self.dominant_colony_id == Some(a.colony_id),
        }).collect()
    }

    /// 回傳供快照廣播的視圖清單。
    pub fn colony_views(&self) -> Vec<MonsterColonyView> {
        self.colonies.iter().map(|col| MonsterColonyView {
            id:           col.id,
            kind:         col.kind.as_str().to_string(),
            name:         col.name.to_string(),
            cx:           col.cx,
            cy:           col.cy,
            spawn_radius: col.spawn_radius,
            density:      colony_density(col.population, col.max_population),
            has_alpha:    self.alphas.iter().any(|a| a.colony_id == col.id),
            is_dominant:  self.dominant_colony_id == Some(col.id),
        }).collect()
    }

    /// ROADMAP 176：霸主存續期間額外的生態壓力加成值（供 game.rs 疊加）。
    pub fn dominant_pressure_bonus(&self) -> f32 {
        if self.dominant_colony_id.is_some() { DOMINANT_PRESSURE_BONUS } else { 0.0 }
    }
}

impl Default for MonsterColonyManager {
    fn default() -> Self { Self::new() }
}

// ─── 輔助函式 ─────────────────────────────────────────────────────────────────

/// 族群密度等級：0=廢棄 1=稀疏 2=正常 3=茂盛。
fn colony_density(pop: u32, max: u32) -> u32 {
    if pop == 0 || max == 0 { return 0; }
    let ratio = pop as f32 / max as f32;
    if ratio <= 0.33 { 1 } else if ratio <= 0.66 { 2 } else { 3 }
}

/// 依巢穴 id + spawn_count 決定性散佈出生位置（純函式，不隨機）。
fn colony_spawn_pos(col: &MonsterColony) -> (f32, f32) {
    let mut s = (col.id as u64).wrapping_mul(0x9E3779B97F4A7C15);
    s = s.wrapping_add((col.spawn_count as u64).wrapping_mul(0xBF58476D1CE4E5B9));
    s ^= s >> 30;
    s = s.wrapping_mul(0x94D049BB133111EB);
    s ^= s >> 27;
    // 角度均勻分佈，半徑 [0.2, 1.0] × spawn_radius
    let angle = (s & 0xFFFF) as f32 / 65535.0 * std::f32::consts::TAU;
    let r_frac = 0.2 + 0.8 * ((s >> 16 & 0xFFFF) as f32 / 65535.0);
    let r = col.spawn_radius * r_frac;
    (col.cx + r * angle.cos(), col.cy + r * angle.sin())
}

/// Alpha 固定在巢穴中心正北方 40px 處（確保明顯、不與普通怪重疊）。
fn alpha_spawn_pos(col: &MonsterColony) -> (f32, f32) {
    (col.cx, col.cy - 40.0)
}

/// ROADMAP 179：援軍相對王身的散佈偏移（以 alpha_id + 序號為鹽值，確定性不重疊）。
fn summon_offset(alpha_id: u32, k: u32) -> (f32, f32) {
    let mut s = (alpha_id as u64).wrapping_mul(0x9E3779B97F4A7C15);
    s = s.wrapping_add((k as u64).wrapping_mul(0xBF58476D1CE4E5B9));
    s ^= s >> 30;
    s = s.wrapping_mul(0x94D049BB133111EB);
    s ^= s >> 27;
    let angle = (s & 0xFFFF) as f32 / 65535.0 * std::f32::consts::TAU;
    // 半徑 [0.4, 1.0] × 召喚半徑，確保援軍環繞而非疊在王身上。
    let r_frac = 0.4 + 0.6 * ((s >> 16 & 0xFFFF) as f32 / 65535.0);
    let r = ALPHA_SUMMON_RADIUS * r_frac;
    (r * angle.cos(), r * angle.sin())
}

/// 世界座標巢穴列表（城外安全區外，分散四方供玩家探索）。
///
/// 城鎮中心像素 ≈ (2336, 2272)，安全區半徑 ≈ 1344px（42 格）。
/// 各巢穴均距城鎮中心 > 1500px，確保在安全區外。
fn build_colonies() -> Vec<MonsterColony> {
    vec![
        MonsterColony {
            id: 0, kind: EnemyKind::FlutterSprite,
            name: "靈蛾巢（東北荒野）",
            cx: 4000.0, cy: 1800.0, spawn_radius: 220.0,
            population: 5, max_population: 8,
            spawn_timer: RESPAWN_SECS, spawn_count: 0,
            alpha_cooldown: 0.0,
        },
        MonsterColony {
            id: 1, kind: EnemyKind::MushroomStalker,
            name: "蘑菇潛行窟（東南澤地）",
            cx: 3900.0, cy: 3200.0, spawn_radius: 240.0,
            population: 5, max_population: 7,
            spawn_timer: RESPAWN_SECS, spawn_count: 0,
            alpha_cooldown: 0.0,
        },
        MonsterColony {
            id: 2, kind: EnemyKind::ScrapDrone,
            name: "廢料無人機陣（南方廢墟）",
            cx: 2200.0, cy: 3900.0, spawn_radius: 200.0,
            population: 4, max_population: 6,
            spawn_timer: RESPAWN_SECS, spawn_count: 0,
            alpha_cooldown: 0.0,
        },
        MonsterColony {
            id: 3, kind: EnemyKind::CrystalGolem,
            name: "水晶魔像坑（西岸礦脈）",
            cx: 700.0, cy: 2400.0, spawn_radius: 260.0,
            population: 3, max_population: 5,
            spawn_timer: RESPAWN_SECS, spawn_count: 0,
            alpha_cooldown: 0.0,
        },
        MonsterColony {
            id: 4, kind: EnemyKind::EtherWisp,
            name: "乙太幽靈霧潭（西北霧區）",
            cx: 1100.0, cy: 800.0, spawn_radius: 210.0,
            population: 5, max_population: 7,
            spawn_timer: RESPAWN_SECS, spawn_count: 0,
            alpha_cooldown: 0.0,
        },
    ]
}

// ─── 測試 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colony_ids_unique() {
        let mgr = MonsterColonyManager::new();
        let mut ids: Vec<u32> = mgr.colonies.iter().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), mgr.colonies.len(), "巢穴 ID 必須唯一");
    }

    #[test]
    fn colonies_start_with_positive_population() {
        let mgr = MonsterColonyManager::new();
        for col in &mgr.colonies {
            assert!(col.population > 0, "巢穴 {} 初始族群應 > 0", col.name);
            assert!(col.population <= col.max_population, "族群不應超過上限");
        }
    }

    #[test]
    fn tick_spawns_when_below_max() {
        let mut mgr = MonsterColonyManager::new();
        // 清空第一個巢穴並讓計時歸零
        mgr.colonies[0].population = 0;
        mgr.colonies[0].spawn_timer = 0.0;
        let events = mgr.tick(0.1, 0.0);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::SpawnAt { .. })),
            "族群未滿且計時歸零應觸發 SpawnAt"
        );
    }

    #[test]
    fn tick_no_spawn_when_full() {
        let mut mgr = MonsterColonyManager::new();
        let col = &mut mgr.colonies[0];
        col.population = col.max_population;
        let events = mgr.tick(RESPAWN_SECS + 1.0, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::SpawnAt { colony_id: 0, .. })),
            "族群已滿不應觸發 SpawnAt"
        );
    }

    #[test]
    fn tick_emits_revived_when_empty_colony_respawns() {
        let mut mgr = MonsterColonyManager::new();
        mgr.colonies[0].population = 0;
        mgr.colonies[0].spawn_timer = 0.0;
        let events = mgr.tick(0.1, 0.0);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::ColonyRevived { .. })),
            "廢棄巢穴復生應發出 ColonyRevived"
        );
    }

    #[test]
    fn kill_near_reduces_population() {
        let mut mgr = MonsterColonyManager::new();
        let col = &mgr.colonies[0];
        let (cx, cy, kind, initial) = (col.cx, col.cy, col.kind, col.population);
        mgr.on_monster_killed_near(cx, cy, kind);
        assert_eq!(mgr.colonies[0].population, initial - 1);
    }

    #[test]
    fn kill_different_kind_does_not_affect_colony() {
        let mut mgr = MonsterColonyManager::new();
        let col = &mgr.colonies[0]; // FlutterSprite
        let (cx, cy) = (col.cx, col.cy);
        let initial = col.population;
        // 在巢穴中心殺其他種怪，不應影響
        mgr.on_monster_killed_near(cx, cy, EnemyKind::CrystalGolem);
        assert_eq!(mgr.colonies[0].population, initial, "殺不同種怪不影響此巢穴");
    }

    #[test]
    fn kill_far_does_not_reduce_population() {
        let mut mgr = MonsterColonyManager::new();
        let kind = mgr.colonies[0].kind;
        let initial = mgr.colonies[0].population;
        // (0, 0) 距所有巢穴均遠超 COLONY_KILL_RADIUS
        mgr.on_monster_killed_near(0.0, 0.0, kind);
        assert_eq!(mgr.colonies[0].population, initial, "距離超出半徑不應扣族群");
    }

    #[test]
    fn wiping_colony_emits_cleared_event() {
        let mut mgr = MonsterColonyManager::new();
        mgr.colonies[0].population = 1;
        let (cx, cy, kind) = (mgr.colonies[0].cx, mgr.colonies[0].cy, mgr.colonies[0].kind);
        let events = mgr.on_monster_killed_near(cx, cy, kind);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::ColonyCleared { .. })),
            "族群清空應發出 ColonyCleared"
        );
        assert_eq!(mgr.colonies[0].population, 0);
    }

    #[test]
    fn wiped_colony_has_longer_respawn_timer() {
        let mut mgr = MonsterColonyManager::new();
        mgr.colonies[0].population = 1;
        let (cx, cy, kind) = (mgr.colonies[0].cx, mgr.colonies[0].cy, mgr.colonies[0].kind);
        mgr.on_monster_killed_near(cx, cy, kind);
        assert!(
            mgr.colonies[0].spawn_timer > RESPAWN_SECS,
            "清剿後補充計時器應比正常更長"
        );
    }

    #[test]
    fn colony_views_count_matches() {
        let mgr = MonsterColonyManager::new();
        assert_eq!(mgr.colony_views().len(), mgr.colonies.len());
    }

    // ── ROADMAP 183：族群潰逃 ──────────────────────────────────────────────────

    #[test]
    fn rout_threshold_basics() {
        // 34% 門檻：取四捨五入、至少 1。
        assert_eq!(rout_threshold(10), 3, "10*0.34=3.4→3");
        assert_eq!(rout_threshold(8), 3, "8*0.34=2.72→3");
        assert_eq!(rout_threshold(1), 1, "上限 1 時門檻至少 1");
        assert_eq!(rout_threshold(0), 1, "上限 0 也保底為 1，避免永不觸發");
    }

    #[test]
    fn kill_crossing_rout_threshold_emits_routed_once() {
        let mut mgr = MonsterColonyManager::new();
        let (cx, cy, kind) = (mgr.colonies[0].cx, mgr.colonies[0].cy, mgr.colonies[0].kind);
        let max = mgr.colonies[0].max_population;
        let thresh = rout_threshold(max);
        // 把人口設在門檻（尚未潰逃），下一刀跌破門檻 → 觸發一次潰逃。
        mgr.colonies[0].population = thresh;
        let events = mgr.on_monster_killed_near(cx, cy, kind);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::ColonyRouted { .. })),
            "人口首次跌破潰逃門檻應發出 ColonyRouted"
        );
        // 再殺一刀（已在門檻下）不應再次潰逃，避免洗版。
        let events2 = mgr.on_monster_killed_near(cx, cy, kind);
        assert!(
            !events2.iter().any(|e| matches!(e, MonsterColonyEvent::ColonyRouted { .. })),
            "已在門檻下不應重複發出 ColonyRouted"
        );
    }

    #[test]
    fn wiping_to_zero_does_not_rout() {
        // 打到歸零走 ColonyCleared，不發潰逃（沒有殘兵可逃）。
        let mut mgr = MonsterColonyManager::new();
        mgr.colonies[0].population = 1;
        let (cx, cy, kind) = (mgr.colonies[0].cx, mgr.colonies[0].cy, mgr.colonies[0].kind);
        let events = mgr.on_monster_killed_near(cx, cy, kind);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::ColonyRouted { .. })),
            "族群歸零不應發出 ColonyRouted"
        );
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::ColonyCleared { .. })),
            "族群歸零應發出 ColonyCleared"
        );
    }

    #[test]
    fn alpha_kill_result_carries_colony_geometry() {
        // 斬首路：attack_alpha 回傳應帶巢穴中心與半徑，供 ws.rs 觸發潰逃。
        let mut mgr = MonsterColonyManager::new();
        let col0 = &mgr.colonies[0];
        let (col_id, cx, cy, radius, kind) =
            (col0.id, col0.cx, col0.cy, col0.spawn_radius, col0.kind);
        // 直接植入一隻位於巢穴 0 中心的 Alpha，再用足量傷害斬殺。
        let aid = mgr.next_alpha_id;
        mgr.next_alpha_id += 1;
        mgr.alphas.push(ColonyAlpha {
            id: aid,
            colony_id: col_id,
            kind,
            x: cx,
            y: cy,
            hp: 100,
            max_hp: 100,
            colony_name: "測試巢",
            command_cooldown: 9999.0,
            active_tactic: None,
            tactic_remaining: 0.0,
            clash_target_id: None,
            allied_to_id: None,
            awakened: false,
        });
        let result = mgr.attack_alpha(aid, cx, cy, 99999, ALPHA_ATTACK_REACH)
            .expect("足量傷害應斬殺 Alpha");
        assert_eq!(result.cx, cx);
        assert_eq!(result.cy, cy);
        assert_eq!(result.rout_radius, radius);
    }

    #[test]
    fn density_levels() {
        assert_eq!(colony_density(0, 8), 0, "族群 0 = 廢棄");
        assert_eq!(colony_density(1, 8), 1, "1/8 ≤ 33% → 稀疏");
        assert_eq!(colony_density(4, 8), 2, "4/8 = 50% → 正常");
        assert_eq!(colony_density(7, 8), 3, "7/8 > 66% → 茂盛");
        assert_eq!(colony_density(8, 8), 3, "8/8 = 100% → 茂盛");
    }

    #[test]
    fn spawn_pos_within_radius() {
        let mgr = MonsterColonyManager::new();
        for col in &mgr.colonies {
            let (sx, sy) = colony_spawn_pos(col);
            let dx = sx - col.cx;
            let dy = sy - col.cy;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(
                dist <= col.spawn_radius,
                "巢穴 {} 出生點距離 {} > spawn_radius {}",
                col.name, dist, col.spawn_radius
            );
        }
    }

    #[test]
    fn colony_views_density_reflects_population() {
        let mgr = MonsterColonyManager::new();
        for view in mgr.colony_views() {
            assert!(view.density <= 3, "密度等級應在 0~3 之間");
        }
    }

    // ─── ROADMAP 168 Alpha 測試 ─────────────────────────────────────────────

    #[test]
    fn alpha_spawns_when_colony_full() {
        let mut mgr = MonsterColonyManager::new();
        // 讓第一個巢穴族群達上限
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        let events = mgr.tick(0.1, 0.0);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaAppeared { .. })),
            "族群達峰值應湧現 Alpha"
        );
        assert_eq!(mgr.alphas.len(), 1, "應有 1 個 Alpha 活躍");
    }

    #[test]
    fn alpha_does_not_spawn_when_on_cooldown() {
        let mut mgr = MonsterColonyManager::new();
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 100.0; // 冷卻中
        let events = mgr.tick(0.1, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaAppeared { .. })),
            "冷卻中不應湧現 Alpha"
        );
        assert_eq!(mgr.alphas.len(), 0);
    }

    #[test]
    fn alpha_not_duplicate_per_colony() {
        let mut mgr = MonsterColonyManager::new();
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0); // 第一次：Alpha 湧現
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        let events = mgr.tick(0.1, 0.0); // 第二次：Alpha 已存在，不應再湧現
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaAppeared { .. })),
            "已有 Alpha 時不應重複湧現"
        );
        assert_eq!(mgr.alphas.len(), 1, "同一巢穴只有 1 個 Alpha");
    }

    #[test]
    fn alpha_has_triple_hp() {
        let mut mgr = MonsterColonyManager::new();
        let col = &mgr.colonies[0];
        let kind = col.kind;
        let max = col.max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0);
        let alpha = &mgr.alphas[0];
        assert_eq!(alpha.kind, kind);
        assert_eq!(alpha.max_hp, kind.max_hp() * 3, "Alpha 生命應為基礎值的 3 倍");
        assert_eq!(alpha.hp, alpha.max_hp, "Alpha 湧現時應滿血");
    }

    #[test]
    fn attack_alpha_reduces_hp() {
        let mut mgr = MonsterColonyManager::new();
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0);
        let alpha = &mgr.alphas[0];
        let (id, ax, ay) = (alpha.id, alpha.x, alpha.y);
        let result = mgr.attack_alpha(id, ax, ay, 1, ALPHA_ATTACK_REACH);
        assert!(result.is_none(), "一點傷害不應擊殺 Alpha");
        assert!(mgr.alphas[0].hp < mgr.alphas[0].max_hp, "HP 應已減少");
    }

    #[test]
    fn attack_alpha_kill_removes_alpha_and_reduces_colony() {
        let mut mgr = MonsterColonyManager::new();
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0);
        let alpha = &mgr.alphas[0];
        let (id, ax, ay, big_power) = (alpha.id, alpha.x, alpha.y, 999999);
        let result = mgr.attack_alpha(id, ax, ay, big_power, ALPHA_ATTACK_REACH);
        assert!(result.is_some(), "超大傷害應擊殺 Alpha");
        assert_eq!(mgr.alphas.len(), 0, "Alpha 應被移除");
        assert_eq!(mgr.colonies[0].population, max.saturating_sub(2), "族群應減 2");
        assert!(mgr.colonies[0].alpha_cooldown > 0.0, "擊殺後應進入冷卻");
    }

    #[test]
    fn attack_alpha_out_of_reach_fails() {
        let mut mgr = MonsterColonyManager::new();
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0);
        let alpha = &mgr.alphas[0];
        let (id, ax, ay) = (alpha.id, alpha.x, alpha.y);
        // 玩家距 Alpha 超過 ALPHA_ATTACK_REACH
        let result = mgr.attack_alpha(id, ax + ALPHA_ATTACK_REACH * 2.0, ay, 999999, ALPHA_ATTACK_REACH);
        assert!(result.is_none(), "超出距離不應命中");
        assert_eq!(mgr.alphas[0].hp, mgr.alphas[0].max_hp, "HP 應無損");
    }

    #[test]
    fn cooldown_decrements_over_time() {
        let mut mgr = MonsterColonyManager::new();
        mgr.colonies[0].alpha_cooldown = 100.0;
        mgr.tick(10.0, 0.0);
        assert!(
            mgr.colonies[0].alpha_cooldown < 100.0,
            "冷卻計時器應隨時間減少"
        );
    }

    #[test]
    fn alpha_views_reflect_active_alphas() {
        let mut mgr = MonsterColonyManager::new();
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0);
        let views = mgr.alpha_views();
        assert_eq!(views.len(), 1, "應有 1 個 Alpha 視圖");
        assert_eq!(views[0].max_hp, mgr.colonies[0].kind.max_hp() * 3);
    }

    #[test]
    fn colony_view_has_alpha_flag() {
        let mut mgr = MonsterColonyManager::new();
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0);
        let views = mgr.colony_views();
        assert!(views[0].has_alpha, "Alpha 活躍時巢穴視圖應標示 has_alpha");
    }

    #[test]
    fn wiping_colony_also_removes_alpha() {
        let mut mgr = MonsterColonyManager::new();
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0);
        assert_eq!(mgr.alphas.len(), 1, "Alpha 應存在");
        // 清空族群
        mgr.colonies[0].population = 1;
        mgr.on_monster_killed_near(mgr.colonies[0].cx, mgr.colonies[0].cy, mgr.colonies[0].kind);
        assert_eq!(mgr.colonies[0].population, 0);
        assert_eq!(mgr.alphas.len(), 0, "族群清空時 Alpha 應一併移除");
    }

    #[test]
    fn alpha_cooldown_prevents_immediate_respawn() {
        let mut mgr = MonsterColonyManager::new();
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0);
        // 擊殺 Alpha
        let (id, ax, ay) = {
            let a = &mgr.alphas[0];
            (a.id, a.x, a.y)
        };
        mgr.attack_alpha(id, ax, ay, 999999, ALPHA_ATTACK_REACH);
        // 立刻設族群為滿 + tick
        mgr.colonies[0].population = max;
        let events = mgr.tick(0.1, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaAppeared { .. })),
            "Alpha 剛被擊殺後冷卻中不應立刻湧現"
        );
    }

    // ─── ROADMAP 169 Alpha 咆哮指揮測試 ────────────────────────────────────────

    fn spawn_alpha(mgr: &mut MonsterColonyManager) -> u32 {
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0);
        assert_eq!(mgr.alphas.len(), 1, "輔助函式：應已湧現 Alpha");
        mgr.alphas[0].id
    }

    #[test]
    fn new_alpha_starts_with_first_wait() {
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        assert!(
            mgr.alphas[0].command_cooldown > 0.0,
            "新湧現的 Alpha 應有初始指令冷卻"
        );
        assert!(
            mgr.alphas[0].active_tactic.is_none(),
            "新湧現的 Alpha 無指令"
        );
    }

    #[test]
    fn alpha_command_not_ready_before_first_wait() {
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        // 僅推進一小段（遠小於 first wait）
        let events = mgr.tick(1.0, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaCommandReady { .. })),
            "第一次等待時間未到不應觸發 AlphaCommandReady"
        );
    }

    #[test]
    fn alpha_command_fires_after_first_wait() {
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        let events = mgr.tick(ALPHA_COMMAND_FIRST_WAIT_SECS + 1.0, 0.0);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaCommandReady { .. })),
            "第一次等待結束後應觸發 AlphaCommandReady"
        );
    }

    #[test]
    fn alpha_command_resets_cooldown_after_fire() {
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        // 推進到觸發
        mgr.tick(ALPHA_COMMAND_FIRST_WAIT_SECS + 1.0, 0.0);
        // 觸發後冷卻應重置
        assert!(
            mgr.alphas[0].command_cooldown > 0.0,
            "AlphaCommandReady 觸發後應重置指令冷卻"
        );
    }

    #[test]
    fn alpha_command_does_not_fire_twice_in_one_tick() {
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        // 一次 tick 推進遠超兩個冷卻週期
        let events = mgr.tick(ALPHA_COMMAND_FIRST_WAIT_SECS + ALPHA_COMMAND_COOLDOWN_SECS * 3.0, 0.0);
        let count = events.iter()
            .filter(|e| matches!(e, MonsterColonyEvent::AlphaCommandReady { .. }))
            .count();
        assert_eq!(count, 1, "單次 tick 最多只觸發一次 AlphaCommandReady（冷卻已重置）");
    }

    #[test]
    fn set_alpha_tactic_updates_view() {
        let mut mgr = MonsterColonyManager::new();
        let id = spawn_alpha(&mut mgr);
        mgr.set_alpha_tactic(id, "包圍".to_string());
        let views = mgr.alpha_views();
        assert_eq!(views[0].active_tactic.as_deref(), Some("包圍"), "視圖應反映剛設定的指令");
    }

    #[test]
    fn active_tactic_clears_after_duration() {
        let mut mgr = MonsterColonyManager::new();
        let id = spawn_alpha(&mut mgr);
        mgr.set_alpha_tactic(id, "集結".to_string());
        // 推進超過指令持續時間
        mgr.tick(ALPHA_TACTIC_DURATION_SECS + 1.0, 0.0);
        assert!(
            mgr.alphas[0].active_tactic.is_none(),
            "指令持續時間到期後應清除"
        );
    }

    #[test]
    fn alpha_command_ready_carries_correct_info() {
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        // 手動設冷卻歸零以確定觸發
        mgr.alphas[0].command_cooldown = 0.0;
        let events = mgr.tick(0.01, 0.0);
        let ev = events.iter().find(|e| matches!(e, MonsterColonyEvent::AlphaCommandReady { .. }));
        assert!(ev.is_some(), "應找到 AlphaCommandReady");
        if let Some(MonsterColonyEvent::AlphaCommandReady { alpha_id, hp_pct, .. }) = ev {
            assert_eq!(*alpha_id, mgr.alphas[0].id, "alpha_id 應匹配");
            assert!(*hp_pct > 0.0 && *hp_pct <= 1.0, "hp_pct 應在合理範圍");
        }
    }

    #[test]
    fn multiple_alphas_have_independent_cooldowns() {
        let mut mgr = MonsterColonyManager::new();
        // 讓前兩個巢穴都達滿族群 + Alpha 冷卻歸零
        for i in 0..2 {
            let max = mgr.colonies[i].max_population;
            mgr.colonies[i].population = max;
            mgr.colonies[i].alpha_cooldown = 0.0;
        }
        mgr.tick(0.1, 0.0); // 湧現兩個 Alpha
        // 手動讓第一個 Alpha 指令冷卻歸零、第二個保留
        mgr.alphas[0].command_cooldown = 0.0;
        mgr.alphas[1].command_cooldown = 999.0;
        let events = mgr.tick(0.01, 0.0);
        let count = events.iter()
            .filter(|e| matches!(e, MonsterColonyEvent::AlphaCommandReady { .. }))
            .count();
        assert_eq!(count, 1, "只有冷卻歸零的那個 Alpha 應觸發指令");
    }

    #[test]
    fn killed_alpha_no_longer_emits_commands() {
        let mut mgr = MonsterColonyManager::new();
        let id = spawn_alpha(&mut mgr);
        let (ax, ay) = (mgr.alphas[0].x, mgr.alphas[0].y);
        mgr.attack_alpha(id, ax, ay, 999999, ALPHA_ATTACK_REACH);
        assert_eq!(mgr.alphas.len(), 0, "Alpha 應已消失");
        // 推進足夠時間，不應有任何 AlphaCommandReady
        let events = mgr.tick(ALPHA_COMMAND_COOLDOWN_SECS * 2.0, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaCommandReady { .. })),
            "Alpha 死亡後不應再發出指令"
        );
    }

    // ─── ROADMAP 170：Alpha 領地爭奪測試 ─────────────────────────────────────

    /// 直接植入兩個 Alpha（在衝突半徑內），驗輔函式，免重複建巢穴。
    fn spawn_two_nearby_alphas(mgr: &mut MonsterColonyManager) -> (u32, u32) {
        let a_id = mgr.next_alpha_id;
        mgr.next_alpha_id += 1;
        mgr.alphas.push(ColonyAlpha {
            id: a_id,
            colony_id: 1,
            kind: crate::combat::EnemyKind::ScrapDrone,
            x: 0.0,
            y: 0.0,
            hp: 100,
            max_hp: 100,
            colony_name: "巢穴甲",
            command_cooldown: 9999.0,
            active_tactic: None,
            tactic_remaining: 0.0,
            clash_target_id: None,
            allied_to_id: None,
                awakened: false,
        });
        let b_id = mgr.next_alpha_id;
        mgr.next_alpha_id += 1;
        mgr.alphas.push(ColonyAlpha {
            id: b_id,
            colony_id: 2,
            kind: crate::combat::EnemyKind::CrystalGolem,
            x: 500.0, // 在 ALPHA_CLASH_RADIUS(900) 內
            y: 0.0,
            hp: 100,
            max_hp: 100,
            colony_name: "巢穴乙",
            command_cooldown: 9999.0,
            active_tactic: None,
            tactic_remaining: 0.0,
            clash_target_id: None,
            allied_to_id: None,
                awakened: false,
        });
        (a_id, b_id)
    }

    #[test]
    fn two_nearby_alphas_start_clashing() {
        let mut mgr = MonsterColonyManager::new();
        spawn_two_nearby_alphas(&mut mgr);
        let events = mgr.tick(0.1, 0.0);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaClashStart { .. })),
            "兩隻不同巢穴的 Alpha 進入範圍應觸發 AlphaClashStart"
        );
    }

    #[test]
    fn clash_start_fires_only_once() {
        let mut mgr = MonsterColonyManager::new();
        spawn_two_nearby_alphas(&mut mgr);
        mgr.tick(0.1, 0.0); // 第一幀：觸發 ClashStart
        let events2 = mgr.tick(0.1, 0.0); // 第二幀：已有 clash_target_id，不再廣播
        assert!(
            !events2.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaClashStart { .. })),
            "AlphaClashStart 只應在衝突首次偵測到時發出一次"
        );
    }

    #[test]
    fn clash_damages_both_alphas() {
        let mut mgr = MonsterColonyManager::new();
        spawn_two_nearby_alphas(&mut mgr);
        let initial_hp = (mgr.alphas[0].hp, mgr.alphas[1].hp);
        mgr.tick(1.0, 0.0); // 1 秒鐘傷害
        assert!(mgr.alphas[0].hp < initial_hp.0, "衝突 Alpha A 應受到傷害");
        // Alpha B 可能已死（若勝負很快決定），不過至少要有過傷害
        let b_still_alive = mgr.alphas.iter().any(|a| a.colony_id == 2);
        if b_still_alive {
            let b_hp = mgr.alphas.iter().find(|a| a.colony_id == 2).unwrap().hp;
            assert!(b_hp < initial_hp.1, "衝突 Alpha B 也應受到傷害");
        }
    }

    #[test]
    fn weaker_alpha_dies_in_clash() {
        let mut mgr = MonsterColonyManager::new();
        // 讓其中一隻血很少，確保會在幾秒內死亡
        let (a_id, _b_id) = spawn_two_nearby_alphas(&mut mgr);
        mgr.alphas.iter_mut().find(|a| a.id == a_id).unwrap().hp = 5; // A 快死了
        // 持續推進，直到 A 死亡（最多 10 秒 = 足夠）
        for _ in 0..100 {
            mgr.tick(0.1, 0.0);
            if !mgr.alphas.iter().any(|a| a.id == a_id) { break; }
        }
        assert!(
            !mgr.alphas.iter().any(|a| a.id == a_id),
            "低血量 Alpha A 應在衝突中死亡"
        );
    }

    #[test]
    fn clash_victory_event_emitted() {
        let mut mgr = MonsterColonyManager::new();
        let (a_id, _) = spawn_two_nearby_alphas(&mut mgr);
        mgr.alphas.iter_mut().find(|a| a.id == a_id).unwrap().hp = 1;
        // 推進到死亡
        let mut victory_found = false;
        for _ in 0..20 {
            let events = mgr.tick(0.5, 0.0);
            if events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaClashVictory { .. })) {
                victory_found = true;
                break;
            }
        }
        assert!(victory_found, "衝突應在 Alpha 死亡時發出 AlphaClashVictory 事件");
    }

    #[test]
    fn loser_colony_alpha_cooldown_extended() {
        let mut mgr = MonsterColonyManager::new();
        // 植入兩個假巢穴供 Alpha 使用
        while mgr.colonies.len() < 2 { mgr.colonies.clear(); break; }
        mgr.colonies = vec![
            MonsterColony {
                id: 1, kind: crate::combat::EnemyKind::ScrapDrone, name: "甲",
                cx: 0.0, cy: 0.0, spawn_radius: 100.0,
                population: 3, max_population: 3,
                spawn_timer: 999.0, spawn_count: 0, alpha_cooldown: 0.0,
            },
            MonsterColony {
                id: 2, kind: crate::combat::EnemyKind::CrystalGolem, name: "乙",
                cx: 500.0, cy: 0.0, spawn_radius: 100.0,
                population: 3, max_population: 3,
                spawn_timer: 999.0, spawn_count: 0, alpha_cooldown: 0.0,
            },
        ];
        let (a_id, _) = spawn_two_nearby_alphas(&mut mgr);
        mgr.alphas.iter_mut().find(|a| a.id == a_id).unwrap().hp = 1;
        for _ in 0..20 { mgr.tick(0.5, 0.0); }
        let loser_col = mgr.colonies.iter().find(|c| c.id == 1);
        if let Some(col) = loser_col {
            assert!(col.alpha_cooldown > ALPHA_COOLDOWN_SECS,
                "敗者巢穴冷卻應大於正常冷卻（{ALPHA_COOLDOWN_SECS}s）");
        }
    }

    #[test]
    fn same_colony_alphas_do_not_clash() {
        let mut mgr = MonsterColonyManager::new();
        // 植入兩隻同巢穴 Alpha
        for i in 0..2u32 {
            let id = mgr.next_alpha_id;
            mgr.next_alpha_id += 1;
            mgr.alphas.push(ColonyAlpha {
                id,
                colony_id: 99, // 同巢穴
                kind: crate::combat::EnemyKind::ScrapDrone,
                x: (i as f32) * 100.0,
                y: 0.0,
                hp: 100, max_hp: 100,
                colony_name: "共同巢穴",
                command_cooldown: 9999.0,
                active_tactic: None,
                tactic_remaining: 0.0,
                clash_target_id: None,
                allied_to_id: None,
                awakened: false,
            });
        }
        let events = mgr.tick(10.0, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaClashStart { .. })),
            "同一巢穴的 Alpha 不應互相衝突"
        );
        assert_eq!(mgr.alphas.len(), 2, "同巢穴 Alpha 不應因衝突消失");
    }

    #[test]
    fn out_of_range_alphas_do_not_clash() {
        let mut mgr = MonsterColonyManager::new();
        let id1 = mgr.next_alpha_id;
        mgr.next_alpha_id += 1;
        mgr.alphas.push(ColonyAlpha {
            id: id1, colony_id: 1,
            kind: crate::combat::EnemyKind::ScrapDrone,
            x: 0.0, y: 0.0,
            hp: 100, max_hp: 100, colony_name: "遠端巢穴甲",
            command_cooldown: 9999.0, active_tactic: None, tactic_remaining: 0.0,
            clash_target_id: None,
            allied_to_id: None,
                awakened: false,
        });
        let id2 = mgr.next_alpha_id;
        mgr.next_alpha_id += 1;
        mgr.alphas.push(ColonyAlpha {
            id: id2, colony_id: 2,
            kind: crate::combat::EnemyKind::CrystalGolem,
            x: 2000.0, y: 0.0, // 超出 900px 衝突半徑
            hp: 100, max_hp: 100, colony_name: "遠端巢穴乙",
            command_cooldown: 9999.0, active_tactic: None, tactic_remaining: 0.0,
            clash_target_id: None,
            allied_to_id: None,
                awakened: false,
        });
        let events = mgr.tick(1.0, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaClashStart { .. })),
            "距離超過衝突半徑的 Alpha 不應開始衝突"
        );
        assert_eq!(mgr.alphas[0].hp, 100, "超出範圍的 Alpha 不應受到傷害");
    }

    // ── ROADMAP 173：傳說古 Alpha 測試 ──────────────────────────────────────

    fn make_full_colony_mgr_n(n: usize) -> MonsterColonyManager {
        let mut mgr = MonsterColonyManager::new();
        mgr.colonies.clear();
        for i in 0..n {
            mgr.colonies.push(MonsterColony {
                id: i as u32,
                kind: crate::combat::EnemyKind::ScrapDrone,
                name: "測試巢穴",
                cx: (i as f32) * 500.0 + 4000.0, // 距城鎮中心 > 1600px
                cy: 4000.0,
                spawn_radius: 100.0,
                population: 10, max_population: 10, // 100% 飽和
                spawn_timer: 9999.0, spawn_count: 0, alpha_cooldown: 0.0,
            });
        }
        mgr
    }

    #[test]
    fn ancient_alpha_not_spawned_with_less_than_3_full_colonies() {
        let mut mgr = make_full_colony_mgr_n(2);
        let events = mgr.tick(1.0, 0.0);
        assert!(mgr.ancient.is_none(), "2 個滿員巢穴不應觸發古 Alpha");
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AncientAlphaEmerged { .. })),
            "不應發出 AncientAlphaEmerged 事件"
        );
    }

    #[test]
    fn ancient_alpha_spawns_with_3_full_colonies() {
        let mut mgr = make_full_colony_mgr_n(3);
        let events = mgr.tick(1.0, 0.0);
        assert!(mgr.ancient.is_some(), "3 個滿員巢穴應觸發古 Alpha");
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::AncientAlphaEmerged { .. })),
            "應發出 AncientAlphaEmerged 事件"
        );
    }

    #[test]
    fn ancient_alpha_has_correct_hp() {
        let mut mgr = make_full_colony_mgr_n(3);
        mgr.tick(1.0, 0.0);
        let ancient = mgr.ancient.as_ref().unwrap();
        assert_eq!(ancient.hp, ANCIENT_ALPHA_HP);
        assert_eq!(ancient.max_hp, ANCIENT_ALPHA_HP);
    }

    #[test]
    fn ancient_alpha_not_in_safe_zone() {
        let mut mgr = make_full_colony_mgr_n(5);
        mgr.tick(1.0, 0.0);
        let ancient = mgr.ancient.as_ref().unwrap();
        let dx = ancient.x - TOWN_CENTER_X;
        let dy = ancient.y - TOWN_CENTER_Y;
        let dist = (dx * dx + dy * dy).sqrt();
        assert!(dist >= ANCIENT_MIN_TOWN_DIST,
            "古 Alpha 應在城鎮安全區外（dist={dist:.0}px）");
    }

    #[test]
    fn ancient_alpha_attack_reduces_hp() {
        let mut mgr = make_full_colony_mgr_n(3);
        mgr.tick(1.0, 0.0);
        let (ax, ay) = { let a = mgr.ancient.as_ref().unwrap(); (a.x, a.y) };
        let result = mgr.attack_ancient_alpha(ax, ay, 10);
        assert!(result.is_none(), "單次攻擊不應擊倒");
        assert_eq!(mgr.ancient.as_ref().unwrap().hp, ANCIENT_ALPHA_HP - 10);
    }

    #[test]
    fn ancient_alpha_attack_out_of_range_fails() {
        let mut mgr = make_full_colony_mgr_n(3);
        mgr.tick(1.0, 0.0);
        let result = mgr.attack_ancient_alpha(0.0, 0.0, 9999);
        assert!(result.is_none(), "超出攻擊範圍應回傳 None");
        assert!(mgr.ancient.is_some(), "超出範圍攻擊不應移除古 Alpha");
    }

    #[test]
    fn ancient_alpha_kill_removes_and_sets_cooldown() {
        let mut mgr = make_full_colony_mgr_n(3);
        mgr.tick(1.0, 0.0);
        let (ax, ay) = { let a = mgr.ancient.as_ref().unwrap(); (a.x, a.y) };
        let result = mgr.attack_ancient_alpha(ax, ay, ANCIENT_ALPHA_HP);
        assert!(result.is_some(), "HP 歸零應回傳擊殺結果");
        assert!(mgr.ancient.is_none(), "擊殺後古 Alpha 應消失");
        assert!(mgr.ancient_cooldown > 0.0, "擊殺後應進入冷卻");
    }

    #[test]
    fn ancient_alpha_cooldown_prevents_reemergence() {
        let mut mgr = make_full_colony_mgr_n(3);
        mgr.tick(1.0, 0.0);
        let (ax, ay) = { let a = mgr.ancient.as_ref().unwrap(); (a.x, a.y) };
        mgr.attack_ancient_alpha(ax, ay, ANCIENT_ALPHA_HP);
        // 冷卻中不應再度湧現
        let events = mgr.tick(1.0, 0.0);
        assert!(mgr.ancient.is_none(), "冷卻中不應再度湧現");
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AncientAlphaEmerged { .. })),
            "冷卻中不應發出 AncientAlphaEmerged"
        );
    }

    #[test]
    fn ancient_alpha_view_reflects_state() {
        let mut mgr = make_full_colony_mgr_n(3);
        assert!(mgr.ancient_alpha_view().is_none(), "無古 Alpha 時視圖應為 None");
        mgr.tick(1.0, 0.0);
        let view = mgr.ancient_alpha_view();
        assert!(view.is_some(), "古 Alpha 存活時視圖應為 Some");
        assert_eq!(view.unwrap().hp, ANCIENT_ALPHA_HP);
    }

    // ─── ROADMAP 174 跨族結盟測試 ─────────────────────────────────────────────

    /// 建立兩個不同巢穴各一隻 Alpha，方便結盟測試。
    fn spawn_two_different_colony_alphas(mgr: &mut MonsterColonyManager) -> (u32, u32) {
        assert!(mgr.colonies.len() >= 2, "至少需要 2 個巢穴");
        // 巢穴 0
        mgr.colonies[0].population = mgr.colonies[0].max_population;
        mgr.colonies[0].alpha_cooldown = 0.0;
        // 巢穴 1
        mgr.colonies[1].population = mgr.colonies[1].max_population;
        mgr.colonies[1].alpha_cooldown = 0.0;
        mgr.tick(0.1, 0.0);
        assert_eq!(mgr.alphas.len(), 2, "應湧現 2 隻 Alpha");
        (mgr.alphas[0].id, mgr.alphas[1].id)
    }

    #[test]
    fn alliance_not_formed_with_single_alpha() {
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        // 跑超過閾值時間
        let events = mgr.tick(ALLIANCE_FORM_SECS + 1.0, 0.0);
        assert!(
            !mgr.alliance_active(),
            "單隻 Alpha 不應觸發結盟"
        );
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AllianceFormed { .. })),
            "不應有 AllianceFormed 事件"
        );
    }

    #[test]
    fn alliance_coexistence_timer_increments() {
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        // 移除衝突（確保都在遠離的位置，不會進衝突半徑）
        mgr.alphas[0].x = 0.0; mgr.alphas[0].y = 0.0;
        mgr.alphas[1].x = 5000.0; mgr.alphas[1].y = 5000.0;
        mgr.tick(10.0, 0.0);
        assert!(
            mgr.coexistence_timer > 0.0,
            "兩隻 Alpha 共存應累積 coexistence_timer"
        );
    }

    #[test]
    fn alliance_forms_after_threshold() {
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        // 放置在遠離位置（不觸發衝突）
        mgr.alphas[0].x = 0.0; mgr.alphas[0].y = 0.0;
        mgr.alphas[1].x = 5000.0; mgr.alphas[1].y = 5000.0;
        // 跑到剛好超過閾值
        let events = mgr.tick(ALLIANCE_FORM_SECS + 0.1, 0.0);
        assert!(
            mgr.alliance_active(),
            "共存 {} 秒後應觸發結盟",
            ALLIANCE_FORM_SECS
        );
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::AllianceFormed { .. })),
            "應發出 AllianceFormed 事件"
        );
    }

    #[test]
    fn alliance_gives_hp_bonus() {
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        mgr.alphas[0].x = 0.0; mgr.alphas[0].y = 0.0;
        mgr.alphas[1].x = 5000.0; mgr.alphas[1].y = 5000.0;
        let hp_before_a = mgr.alphas[0].hp;
        let hp_before_b = mgr.alphas[1].hp;
        mgr.tick(ALLIANCE_FORM_SECS + 0.1, 0.0);
        assert!(
            mgr.alphas[0].hp > hp_before_a || mgr.alphas[1].hp > hp_before_b,
            "結盟後至少一隻 Alpha 血量應增加"
        );
    }

    #[test]
    fn alliance_broken_when_alpha_killed() {
        let mut mgr = MonsterColonyManager::new();
        let (id_a, _id_b) = spawn_two_different_colony_alphas(&mut mgr);
        mgr.alphas[0].x = 0.0; mgr.alphas[0].y = 0.0;
        mgr.alphas[1].x = 5000.0; mgr.alphas[1].y = 5000.0;
        // 觸發結盟
        mgr.tick(ALLIANCE_FORM_SECS + 0.1, 0.0);
        assert!(mgr.alliance_active(), "前置條件：應已結盟");
        // 擊殺盟約 Alpha A
        let alpha_a = mgr.alphas.iter().find(|a| a.id == id_a).unwrap();
        let (ax, ay, max_hp) = (alpha_a.x, alpha_a.y, alpha_a.max_hp * 2 + 100);
        let result = mgr.attack_alpha(id_a, ax, ay, max_hp, ALPHA_ATTACK_REACH);
        assert!(result.is_some(), "應擊殺 Alpha A");
        assert!(result.unwrap().was_allied, "擊殺盟約 Alpha 應設 was_allied=true");
        // 下一幀偵測到結盟已失效
        let events = mgr.tick(0.1, 0.0);
        assert!(
            !mgr.alliance_active(),
            "盟約 Alpha 被擊殺後應解除結盟"
        );
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::AllianceBroken { .. })),
            "應發出 AllianceBroken 事件"
        );
    }

    #[test]
    fn non_allied_alpha_kill_has_was_allied_false() {
        let mut mgr = MonsterColonyManager::new();
        let id_a = spawn_alpha(&mut mgr);
        let alpha_a = mgr.alphas.iter().find(|a| a.id == id_a).unwrap();
        let (ax, ay, max_hp) = (alpha_a.x, alpha_a.y, alpha_a.max_hp + 100);
        let result = mgr.attack_alpha(id_a, ax, ay, max_hp, ALPHA_ATTACK_REACH);
        assert!(result.is_some(), "應擊殺 Alpha");
        assert!(
            !result.unwrap().was_allied,
            "非盟約 Alpha 擊殺不應設 was_allied=true"
        );
    }

    #[test]
    fn alliance_not_formed_when_clashing() {
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        // 讓兩隻 Alpha 貼近觸發廝殺
        mgr.alphas[0].x = 100.0; mgr.alphas[0].y = 100.0;
        mgr.alphas[1].x = 200.0; mgr.alphas[1].y = 100.0; // 距離 < ALPHA_CLASH_RADIUS
        // 跑超過結盟閾值
        let events = mgr.tick(ALLIANCE_FORM_SECS + 1.0, 0.0);
        assert!(
            !mgr.alliance_active(),
            "廝殺中的 Alpha 不應觸發結盟"
        );
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AllianceFormed { .. })),
            "廝殺中不應有 AllianceFormed 事件"
        );
    }

    #[test]
    fn alliance_active_method_returns_correct_state() {
        let mut mgr = MonsterColonyManager::new();
        assert!(!mgr.alliance_active(), "初始無結盟");
        spawn_two_different_colony_alphas(&mut mgr);
        mgr.alphas[0].x = 0.0; mgr.alphas[0].y = 0.0;
        mgr.alphas[1].x = 5000.0; mgr.alphas[1].y = 5000.0;
        mgr.tick(ALLIANCE_FORM_SECS + 0.1, 0.0);
        assert!(mgr.alliance_active(), "結盟觸發後 alliance_active() 應為 true");
    }

    // ── ROADMAP 175：Alpha 覺醒危機測試 ──────────────────────────────────────

    #[test]
    fn awakening_not_triggered_single_alpha() {
        // 單隻 Alpha + 高壓力：不達覺醒最低數量門檻，不觸發。
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        let events = mgr.tick(0.1, ALPHA_AWAKENING_PRESSURE + 1.0);
        assert!(!mgr.alphas[0].awakened, "單隻 Alpha 不應觸發覺醒");
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaAwakened { .. })),
            "不應有 AlphaAwakened 事件"
        );
    }

    #[test]
    fn awakening_not_triggered_low_pressure() {
        // 兩隻 Alpha + 低壓力：壓力未達閾值，不觸發。
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        let events = mgr.tick(0.1, ALPHA_AWAKENING_PRESSURE - 1.0);
        assert!(!mgr.alphas[0].awakened, "壓力不足不應觸發覺醒");
        assert!(!mgr.alphas[1].awakened, "壓力不足不應觸發覺醒");
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaAwakened { .. })),
            "不應有 AlphaAwakened 事件"
        );
    }

    #[test]
    fn awakening_triggered_two_alphas_high_pressure() {
        // 兩隻 Alpha + 高壓力：觸發覺醒，發出 AlphaAwakened 事件。
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        let events = mgr.tick(0.1, ALPHA_AWAKENING_PRESSURE + 1.0);
        assert!(mgr.alphas[0].awakened, "Alpha[0] 應處於覺醒狀態");
        assert!(mgr.alphas[1].awakened, "Alpha[1] 應處於覺醒狀態");
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaAwakened { count: 2 })),
            "應有 AlphaAwakened {{ count: 2 }} 事件"
        );
    }

    #[test]
    fn awakening_gives_hp_bonus() {
        // 覺醒時 HP 應增加 50%。
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        let hp_before = mgr.alphas[0].hp;
        let max_hp = mgr.alphas[0].max_hp;
        mgr.tick(0.1, ALPHA_AWAKENING_PRESSURE + 1.0);
        let hp_after = mgr.alphas[0].hp;
        let expected_bonus = (max_hp as f32 * ALPHA_AWAKENING_HP_BONUS) as u32;
        assert_eq!(hp_after, (hp_before + expected_bonus).min(max_hp + max_hp / 2),
            "覺醒 HP 加成應為 50%");
    }

    #[test]
    fn awakening_hp_capped_at_150pct_max() {
        // HP 加成不超過 1.5× max_hp。
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        let max_hp = mgr.alphas[0].max_hp;
        mgr.alphas[0].hp = max_hp; // 滿血
        mgr.tick(0.1, ALPHA_AWAKENING_PRESSURE + 1.0);
        let cap = max_hp + max_hp / 2;
        assert!(mgr.alphas[0].hp <= cap, "覺醒 HP 不應超過 1.5× max_hp");
    }

    #[test]
    fn awakening_no_double_event() {
        // 已覺醒的 Alpha 再次觸發不應重複發事件。
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        mgr.tick(0.1, ALPHA_AWAKENING_PRESSURE + 1.0); // 第一次覺醒
        let events2 = mgr.tick(0.1, ALPHA_AWAKENING_PRESSURE + 1.0); // 第二次同壓力
        assert!(
            !events2.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaAwakened { .. })),
            "已覺醒的 Alpha 不應再次發出 AlphaAwakened 事件"
        );
    }

    #[test]
    fn deawakening_when_pressure_drops() {
        // 壓力回落到 DEAWAKEN 以下時，覺醒應解除。
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        mgr.tick(0.1, ALPHA_AWAKENING_PRESSURE + 1.0); // 覺醒
        assert!(mgr.alphas[0].awakened, "應已覺醒");
        mgr.tick(0.1, ALPHA_DEAWAKEN_PRESSURE - 1.0); // 壓力回落
        assert!(!mgr.alphas[0].awakened, "壓力回落後應解除覺醒");
        assert!(!mgr.alphas[1].awakened, "壓力回落後應解除覺醒");
    }

    #[test]
    fn kill_alpha_sets_was_awakened_true() {
        // 覺醒狀態下擊殺 Alpha，was_awakened 應為 true。
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        mgr.tick(0.1, ALPHA_AWAKENING_PRESSURE + 1.0);
        let alpha_id = mgr.alphas[0].id;
        let alpha_x = mgr.alphas[0].x;
        let alpha_y = mgr.alphas[0].y;
        let max_hp = mgr.alphas[0].max_hp;
        let result = mgr.attack_alpha(alpha_id, alpha_x, alpha_y, max_hp * 10, ALPHA_ATTACK_REACH);
        assert!(result.is_some(), "應成功擊殺");
        assert!(result.unwrap().was_awakened, "was_awakened 應為 true");
    }

    #[test]
    fn kill_alpha_sets_was_awakened_false_when_normal() {
        // 非覺醒狀態下擊殺 Alpha，was_awakened 應為 false。
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        let alpha_id = mgr.alphas[0].id;
        let alpha_x = mgr.alphas[0].x;
        let alpha_y = mgr.alphas[0].y;
        let max_hp = mgr.alphas[0].max_hp;
        let result = mgr.attack_alpha(alpha_id, alpha_x, alpha_y, max_hp * 10, ALPHA_ATTACK_REACH);
        assert!(result.is_some(), "應成功擊殺");
        assert!(!result.unwrap().was_awakened, "非覺醒狀態 was_awakened 應為 false");
    }

    #[test]
    fn alpha_view_includes_awakened_field() {
        // alpha_views() 回傳的視圖應含 awakened 欄位。
        let mut mgr = MonsterColonyManager::new();
        spawn_two_different_colony_alphas(&mut mgr);
        let views_before = mgr.alpha_views();
        assert!(!views_before[0].awakened, "覺醒前 awakened 應為 false");
        mgr.tick(0.1, ALPHA_AWAKENING_PRESSURE + 1.0);
        let views_after = mgr.alpha_views();
        assert!(views_after[0].awakened, "覺醒後 awakened 應為 true");
    }

    // ── ROADMAP 176：物種霸主湧現測試 ────────────────────────────────────────

    /// 建立一個族群滿員且有 Alpha 的場景，回傳 mgr 和 colony_id。
    fn setup_dominant_candidate() -> (MonsterColonyManager, u32) {
        let mut mgr = MonsterColonyManager::new();
        let col_id = mgr.colonies[0].id;
        // 將族群拉到 max（確保比例 = 1.0 ≥ DOMINANT_MIN_POP_RATIO）
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        mgr.colonies[0].alpha_cooldown = 0.0;
        // 直接植入 Alpha（繞過湧現計時）
        let alpha_id = mgr.next_alpha_id;
        mgr.next_alpha_id += 1;
        mgr.alphas.push(ColonyAlpha {
            id: alpha_id,
            colony_id: col_id,
            kind: mgr.colonies[0].kind,
            x: mgr.colonies[0].cx,
            y: mgr.colonies[0].cy,
            hp: 100,
            max_hp: 100,
            colony_name: mgr.colonies[0].name,
            command_cooldown: 9999.0,
            active_tactic: None,
            tactic_remaining: 0.0,
            clash_target_id: None,
            allied_to_id: None,
            awakened: false,
        });
        (mgr, col_id)
    }

    #[test]
    fn dominance_not_triggered_before_qualify_time() {
        // 條件達成但計時未到，不發 DominanceDeclaration。
        let (mut mgr, _) = setup_dominant_candidate();
        let events = mgr.tick(DOMINANT_QUALIFY_SECS - 1.0, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::DominanceDeclaration { .. })),
            "計時未到不應觸發 DominanceDeclaration"
        );
        assert!(mgr.dominant_colony_id.is_none(), "尚未稱霸");
    }

    #[test]
    fn dominance_triggered_after_qualify_time() {
        // 計時達到 DOMINANT_QUALIFY_SECS 後發 DominanceDeclaration。
        let (mut mgr, col_id) = setup_dominant_candidate();
        let events = mgr.tick(DOMINANT_QUALIFY_SECS + 0.1, 0.0);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::DominanceDeclaration { colony_id, .. } if *colony_id == col_id)),
            "計時到達後應發 DominanceDeclaration"
        );
        assert_eq!(mgr.dominant_colony_id, Some(col_id), "稱霸 colony_id 應正確");
    }

    #[test]
    fn dominance_not_triggered_without_alpha() {
        // 族群滿員但無 Alpha（且 alpha_cooldown 阻止自動湧現），不應稱霸。
        let mut mgr = MonsterColonyManager::new();
        let col = &mut mgr.colonies[0];
        let max = col.max_population;
        col.population = max;
        // 高冷卻防止 tick 內自動湧現 Alpha
        col.alpha_cooldown = DOMINANT_QUALIFY_SECS * 2.0;
        let events = mgr.tick(DOMINANT_QUALIFY_SECS + 1.0, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::DominanceDeclaration { .. })),
            "無 Alpha 不應稱霸"
        );
    }

    #[test]
    fn dominance_not_triggered_low_population() {
        // Alpha 存在但族群比例 < DOMINANT_MIN_POP_RATIO，不應稱霸。
        let mut mgr = MonsterColonyManager::new();
        let col_id = mgr.colonies[0].id;
        // 族群設為 max 的 50%（< 67% 閾值）
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max / 2;
        let alpha_id = mgr.next_alpha_id;
        mgr.next_alpha_id += 1;
        mgr.alphas.push(ColonyAlpha {
            id: alpha_id, colony_id: col_id, kind: mgr.colonies[0].kind,
            x: mgr.colonies[0].cx, y: mgr.colonies[0].cy,
            hp: 100, max_hp: 100, colony_name: mgr.colonies[0].name,
            command_cooldown: 9999.0, active_tactic: None, tactic_remaining: 0.0,
            clash_target_id: None, allied_to_id: None, awakened: false,
        });
        let events = mgr.tick(DOMINANT_QUALIFY_SECS + 1.0, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::DominanceDeclaration { .. })),
            "族群不足不應稱霸"
        );
    }

    #[test]
    fn dominance_broken_on_population_drop() {
        // 稱霸後族群下滑至閾值以下，觸發 DominanceBroken。
        let (mut mgr, col_id) = setup_dominant_candidate();
        mgr.tick(DOMINANT_QUALIFY_SECS + 0.1, 0.0); // 觸發稱霸
        assert_eq!(mgr.dominant_colony_id, Some(col_id));
        // 族群跌到 0（被清空）
        mgr.colonies[0].population = 0;
        let events = mgr.tick(0.1, 0.0);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::DominanceBroken { colony_id, .. } if *colony_id == col_id)),
            "族群跌落後應發 DominanceBroken"
        );
        assert!(mgr.dominant_colony_id.is_none(), "霸主應被清除");
    }

    #[test]
    fn dominance_broken_on_alpha_kill() {
        // 霸主 Alpha 被擊殺時 attack_alpha 應清除霸主並設 was_dominant = true。
        let (mut mgr, col_id) = setup_dominant_candidate();
        mgr.tick(DOMINANT_QUALIFY_SECS + 0.1, 0.0);
        assert_eq!(mgr.dominant_colony_id, Some(col_id));
        let alpha_id = mgr.alphas[0].id;
        let ax = mgr.alphas[0].x;
        let ay = mgr.alphas[0].y;
        let result = mgr.attack_alpha(alpha_id, ax, ay, 99999, ALPHA_ATTACK_REACH);
        assert!(result.is_some(), "應成功擊殺");
        let r = result.unwrap();
        assert!(r.was_dominant, "was_dominant 應為 true");
        assert!(mgr.dominant_colony_id.is_none(), "霸主應被清除");
    }

    #[test]
    fn kill_normal_alpha_sets_was_dominant_false() {
        // 非霸主 Alpha 被擊殺時 was_dominant 應為 false。
        let (mut mgr, _) = setup_dominant_candidate();
        // 不推進計時，Alpha 尚未稱霸
        let alpha_id = mgr.alphas[0].id;
        let ax = mgr.alphas[0].x;
        let ay = mgr.alphas[0].y;
        let result = mgr.attack_alpha(alpha_id, ax, ay, 99999, ALPHA_ATTACK_REACH);
        assert!(result.is_some());
        assert!(!result.unwrap().was_dominant, "非霸主 was_dominant 應為 false");
    }

    #[test]
    fn dominant_pressure_bonus_correct() {
        // 無霸主時加成為 0；稱霸後加成為 DOMINANT_PRESSURE_BONUS。
        let (mut mgr, _) = setup_dominant_candidate();
        assert_eq!(mgr.dominant_pressure_bonus(), 0.0, "初始無霸主，加成應為 0");
        mgr.tick(DOMINANT_QUALIFY_SECS + 0.1, 0.0);
        assert_eq!(mgr.dominant_pressure_bonus(), DOMINANT_PRESSURE_BONUS, "稱霸後加成應為正確值");
    }

    #[test]
    fn dominance_cooldown_prevents_immediate_re_dominance() {
        // 霸主解除後同一巢穴立刻無法再度稱霸（需等冷卻）。
        let (mut mgr, col_id) = setup_dominant_candidate();
        mgr.tick(DOMINANT_QUALIFY_SECS + 0.1, 0.0);
        mgr.colonies[0].population = 0; // 族群跌落
        mgr.tick(0.1, 0.0); // DominanceBroken，設冷卻
        // 恢復族群 + Alpha
        let max = mgr.colonies[0].max_population;
        mgr.colonies[0].population = max;
        let events = mgr.tick(DOMINANT_QUALIFY_SECS + 1.0, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::DominanceDeclaration { colony_id: id, .. } if *id == col_id)),
            "冷卻期間不應再度觸發 DominanceDeclaration"
        );
    }

    #[test]
    fn colony_view_shows_is_dominant() {
        // colony_views() 中霸主巢穴的 is_dominant 應為 true。
        let (mut mgr, col_id) = setup_dominant_candidate();
        let views_before = mgr.colony_views();
        let v = views_before.iter().find(|c| c.id == col_id).unwrap();
        assert!(!v.is_dominant, "稱霸前 is_dominant 應為 false");
        mgr.tick(DOMINANT_QUALIFY_SECS + 0.1, 0.0);
        let views_after = mgr.colony_views();
        let v2 = views_after.iter().find(|c| c.id == col_id).unwrap();
        assert!(v2.is_dominant, "稱霸後 is_dominant 應為 true");
    }

    #[test]
    fn alpha_view_shows_is_dominant() {
        // alpha_views() 中霸主巢穴 Alpha 的 is_dominant 應為 true。
        let (mut mgr, _) = setup_dominant_candidate();
        let views_before = mgr.alpha_views();
        assert!(!views_before[0].is_dominant, "稱霸前 Alpha is_dominant 應為 false");
        mgr.tick(DOMINANT_QUALIFY_SECS + 0.1, 0.0);
        let views_after = mgr.alpha_views();
        assert!(views_after[0].is_dominant, "稱霸後 Alpha is_dominant 應為 true");
    }

    #[test]
    fn no_double_dominance_declaration() {
        // 已稱霸的巢穴再次 tick 不應重複發 DominanceDeclaration。
        let (mut mgr, _) = setup_dominant_candidate();
        mgr.tick(DOMINANT_QUALIFY_SECS + 0.1, 0.0); // 第一次稱霸
        let events2 = mgr.tick(DOMINANT_QUALIFY_SECS + 0.1, 0.0);
        assert!(
            !events2.iter().any(|e| matches!(e, MonsterColonyEvent::DominanceDeclaration { .. })),
            "已稱霸不應重複發 DominanceDeclaration"
        );
    }

    // ─── ROADMAP 179：怪物王號令援軍 ───────────────────────────────────────────

    /// 輔助：建立一隻「覺醒（菁英）+ 重傷」的 Alpha。
    /// 注意：須以 eco_pressure 落在 [70, 85) 區間的值 tick，避免 tick_awakening 重置覺醒旗標。
    fn wounded_awakened_alpha() -> MonsterColonyManager {
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        mgr.alphas[0].awakened = true;
        mgr.alphas[0].hp = 1; // 重傷（遠低於 50%）
        mgr
    }

    /// 維持覺醒不被重置的安全壓力值（70 ≤ x < 85）。
    const KEEP_AWAKE_PRESSURE: f32 = 75.0;

    #[test]
    fn summon_not_triggered_when_not_elite() {
        // 重傷但非菁英（未覺醒、非霸主）→ 不召喚。
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        mgr.alphas[0].hp = 1;
        let events = mgr.tick(0.1, 0.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaSummonedReinforcements { .. })),
            "非菁英 Alpha 不應召喚援軍"
        );
    }

    #[test]
    fn summon_not_triggered_at_full_hp() {
        // 菁英但滿血 → 不召喚（重傷才召喚）。
        let mut mgr = MonsterColonyManager::new();
        spawn_alpha(&mut mgr);
        mgr.alphas[0].awakened = true;
        let events = mgr.tick(0.1, KEEP_AWAKE_PRESSURE);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaSummonedReinforcements { .. })),
            "滿血菁英 Alpha 不應召喚援軍"
        );
    }

    #[test]
    fn summon_triggered_for_wounded_awakened_alpha() {
        // 覺醒 + 重傷 → 召喚指定數量援軍，族群增加，氣泡與冷卻就位。
        let mut mgr = wounded_awakened_alpha();
        let col_id = mgr.alphas[0].colony_id;
        let alpha_id = mgr.alphas[0].id;
        let pop_before = mgr.colonies.iter().find(|c| c.id == col_id).unwrap().population;
        let events = mgr.tick(0.1, KEEP_AWAKE_PRESSURE);
        let summon = events.iter().find_map(|e| match e {
            MonsterColonyEvent::AlphaSummonedReinforcements { count, positions, .. } =>
                Some((*count, positions.len())),
            _ => None,
        });
        let (count, npos) = summon.expect("重傷菁英 Alpha 應召喚援軍");
        assert_eq!(count, ALPHA_SUMMON_COUNT, "召喚數量應為常數值");
        assert_eq!(npos as u32, ALPHA_SUMMON_COUNT, "position 數應等於召喚數");
        let pop_after = mgr.colonies.iter().find(|c| c.id == col_id).unwrap().population;
        assert_eq!(pop_after, pop_before + ALPHA_SUMMON_COUNT, "族群應增加援軍數");
        assert_eq!(
            mgr.alphas[0].active_tactic.as_deref(), Some(SUMMON_TACTIC_NAME),
            "應設定召喚指揮氣泡"
        );
        assert!(
            mgr.alpha_summon_cd.get(&alpha_id).copied().unwrap_or(0.0) > 0.0,
            "召喚後應進入冷卻"
        );
    }

    #[test]
    fn summon_respects_cooldown() {
        // 連續兩幀都重傷，冷卻期間不應再次召喚。
        let mut mgr = wounded_awakened_alpha();
        let _ = mgr.tick(0.1, KEEP_AWAKE_PRESSURE); // 第一次召喚
        mgr.alphas[0].hp = 1; // 維持重傷
        let events2 = mgr.tick(0.1, KEEP_AWAKE_PRESSURE);
        assert!(
            !events2.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaSummonedReinforcements { .. })),
            "冷卻期間不應再召喚"
        );
    }

    #[test]
    fn summon_triggered_for_wounded_dominant_alpha() {
        // 霸主資格（非覺醒）+ 重傷 → 召喚援軍。
        let (mut mgr, _) = setup_dominant_candidate();
        mgr.tick(DOMINANT_QUALIFY_SECS + 0.1, 0.0); // 稱霸
        assert!(mgr.dominant_colony_id.is_some(), "前置：應已稱霸");
        mgr.alphas[0].hp = 1;
        mgr.alphas[0].awakened = false; // 確認靠霸主資格而非覺醒成為菁英
        let events = mgr.tick(0.1, 0.0);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaSummonedReinforcements { .. })),
            "重傷霸主 Alpha 應召喚援軍"
        );
    }

    #[test]
    fn summon_skipped_when_over_cap() {
        // 族群兵力已超過上限保護值 → 不召喚，但仍進冷卻避免每幀重判。
        let mut mgr = wounded_awakened_alpha();
        let col_id = mgr.alphas[0].colony_id;
        let alpha_id = mgr.alphas[0].id;
        if let Some(col) = mgr.colonies.iter_mut().find(|c| c.id == col_id) {
            col.population = col.max_population + ALPHA_SUMMON_MAX_EXTRA + 1;
        }
        let events = mgr.tick(0.1, KEEP_AWAKE_PRESSURE);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::AlphaSummonedReinforcements { .. })),
            "兵力超過上限時不應再召喚"
        );
        assert!(
            mgr.alpha_summon_cd.get(&alpha_id).copied().unwrap_or(0.0) > 0.0,
            "超上限仍應設冷卻"
        );
    }

    #[test]
    fn summon_cooldown_pruned_when_alpha_gone() {
        // Alpha 消失後其冷卻紀錄應被清除，避免 map 無限增長。
        let mut mgr = wounded_awakened_alpha();
        let aid = mgr.alphas[0].id;
        let _ = mgr.tick(0.1, KEEP_AWAKE_PRESSURE); // 召喚 → 建立冷卻
        assert!(mgr.alpha_summon_cd.contains_key(&aid), "前置：應有冷卻紀錄");
        mgr.alphas.clear(); // Alpha 消失
        let _ = mgr.tick(0.1, 0.0);
        assert!(
            !mgr.alpha_summon_cd.contains_key(&aid),
            "Alpha 消失後冷卻紀錄應被清除"
        );
    }
}
