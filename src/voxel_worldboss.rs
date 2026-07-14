//! 乙太方界·遠征首領 v1（World Boss）——世界第一次有了一個需要**遠行討伐**的目標
//! （自主提案切片）。
//!
//! **真缺口**：戰鬥/抵禦這條軸線至今已疊了三層——暗影（怪物/抵禦第一刀）給了夜的張力、
//! 驅影之劍（887）給了武裝、暗潮之夜（893）把兩者串成一場**全村在家防守**的集體事件。
//! 但三層全部發生在**村莊周邊**、全部只在**夜間**存在、全部是**多隻小怪**——世界始終缺一個
//! 「值得放下手邊的事、往遠方走一趟」的**單一目標**：一個不會自己送上門、你得主動出發去找、
//! 找到後也不會一擊而潰、需要撐過一段時間（甚至跨越好幾個日夜）才能打倒的**首領**。
//! 舊 2D/3D 世界曾有「宇宙裂縫守護者」「獸潮攻城」證明這類「遠方的、有血量的、值得召集
//! 大家一起打」的目標玩家會買單，但乙太方界（voxel 系列）至今完全沒有這一類系統。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **平時暗影**＝零散、朝**你**漂近、緩慢觸碰扣血、村莊庇護圈**外**才生成——被動的夜間點綴。
//! - **暗潮之夜**＝突破庇護圈、湧向**村莊中心**的一夜戰役——**在家防守**，天亮必退、絕不留到白天。
//! - **遠征首領（本刀）**＝生在遠離村莊的一個**固定點**（庇護圈外更遠一圈的環帶）、**原地不動**
//!   （不追人、不主動攻擊，一改暗影「威脅感」為「等待被挑戰的巨獸」）、**不分晝夜、不隨黎明
//!   消散**——你得主動找到它、打到它倒下為止，可能得跨好幾個日夜才湊齊足夠的人手/裝備。
//!   三者是「近/夜/被動防守」與「遠/恆常/主動遠征」的根本分野，不是同一件事重做。
//!
//! **設計調性**：仍守療癒底線——首領本身不主動攻擊玩家（不設仇恨/傷害判定，v1 刻意有界），
//! 張力來自「找得到、打得倒嗎」的遠征感，而非戰鬥危險；擊倒後全服一起慶祝、掉一大筆溫柔獎勵。
//!
//! 純函式層：確定性、零 LLM、零鎖、零 IO，可單元測試。真正的擲骰/tick 驅動/廣播/傷害套用
//! 都在 `voxel_ws.rs`（首領血量走 `RwLock<Option<WorldBoss>>`，被玩家連線併發挖擊時靠寫鎖
//! 序列化傷害套用，不用裸原子——避免併發扣血互相蓋掉；出生擲骰仍走 tick 迴圈既有的單執行緒
//! 原子旗標慣例，嚴守 prod 死鎖鐵律）。
//!
//! ## v2：居民聞訊馳援（自主提案切片，接續 v1，ROADMAP 983）
//!
//! **真缺口**：v1 上線後，玩家孤軍走遠路找到首領、獨自一鎬一鎬把 36 血打下來——但即使首領
//! 就在附近咆哮，AI 居民依然故我採集/閒晃，對這場世界級遠征毫無反應。這直接牴觸乙太方界的
//! 核心信念（`docs/PLAN_ETHERVOX.md`）：「AI 居民真的活著、記憶要驅動行為」——世界發生大事時，
//! 居民理應有所反應，而不是背景板。
//!
//! **做法**：首領在世期間，少數（**全世界至多 [`ASSIST_MAX_RESIDENTS`] 位，不分玩家**）當下
//! 真正**完全閒置**（沒有任何進行中的探訪/遠行/聚會/跟隨/發明/採集等既有任務——見
//! `voxel_ws.rs` 的守門判定）的居民，會週期性小機率擲骰決定要不要啟程前往首領所在地陪你；
//! 選中後走到首領旁邊，每隔遠比玩家慢的 [`ASSIST_HIT_INTERVAL_SECS`] 秒才輕輕削一次
//! [`ASSIST_HIT_POWER`] 點血；首領倒下時，有到場的居民會各自寫一筆「我也在場」的記憶，
//! 動態牆額外點名感謝；首領撤退（逾期沒被打倒）則安靜清空馳援旗標，不留記憶、不播報。
//!
//! **與 v1 razor-sharp 區隔（陪伴，不是代打）**：
//! - v1＝玩家獨力承擔全部戰鬥；v2＝**加一層**陪伴反應，玩家仍是唯一有意義的傷害輸出來源
//!   （兩位居民全程陪到底，撐死也只削掉個位數血量，相對 36 血杯水車薪）。**這不只是口號**：
//!   單看擊擊間隔／傷害本身不足以保證——首領存續上限長達 [`BOSS_LIFETIME_SECS`]（1800 秒），
//!   若無其他約束，兩位居民理論上能在首領存續期間內單獨磨死牠，直接牴觸「陪伴不是代打」。
//!   因此另設世界層級的 [`ASSIST_TOTAL_DAMAGE_CAP`]——居民對**同一位**首領累計貢獻的傷害
//!   有真正的個位數硬上限，用完即止（居民仍會貼著打轉、冒挖擊動作，只是不再真的扣血），
//!   這才是「玩家永遠是主力」唯一被程式碼真正保證、而非僅靠參數巧合湊出來的地方。
//! - 居民挖擊間隔（4 秒）遠慢於玩家的 [`HIT_MIN_INTERVAL_SECS`]（0.25 秒）十幾倍，
//!   絕不可能比玩家更快削血、也不會讓「居民幫忙打」變成「玩家躺著等居民代打」。
//! - 只從**完全閒置**的居民裡挑，不搶占/中斷任何既有任務，也不需要在其他觸發點加排除條件
//!   （反正被選中的居民本來就沒有別的任務在跑）。
//!
//! **成本紀律／濫用防護**：世界層級人數硬上限 2（`active_count` 由呼叫端每輪 tick 前掃一次現有
//! 居民算出，同一輪 tick 不會超發）；週期檢查間隔數十秒才擲一次骰、且擲中機率僅三成上下，
//! 不是首領一出現就秒收到支援；每人一次馳援後有 [`ASSIST_COOLDOWN_SECS`] 冷卻，不會連續被選中；
//! 零 LLM、零額外 IO（決定啟程時只冒一句泡泡，不寫記憶不發 Feed，稀少事件才上 Feed 是本專案
//! 一貫慣例，避免居民自主行為的高頻率洗版動態牆）。
//!
//! **護欄**：傷害套用走既有 [`register_hit`]（與玩家共用同一份純函式，不會重造一份不一致的
//! 扣血邏輯）；居民座標到首領座標的移動優先權，接進既有「閒晃中心 if/else if 鏈」，不新開一套
//! 移動系統；鎖序上，世界快照（哪些居民要動身、首領座標）一律在進入 `residents` 寫鎖**之前**
//! 準備好，傷害套用則是 `residents` 迴圈裡先收集「誰打中了首領、扣多少血」成一份清單，
//! 迴圈跑完、`residents` 寫鎖釋放後才統一對 `world_boss` 拿一次寫鎖套用——`world_boss` 與
//! `residents` 兩把鎖任何時候都不互相巢狀持有，嚴守 prod 死鎖鐵律。

// ── 調性參數（集中一處，日後平衡好調）───────────────────────────────────────────

/// 首領顯示名。
pub const BOSS_NAME: &str = "巨蝕者";

/// 首領血量上限：遠高於一隻暗影（3）——一人徒手要打上數十下，鼓勵召集夥伴、鼓勵武裝鐵劍。
pub const BOSS_MAX_HP: u32 = 36;

/// 白天檢查一次「今天要不要出現一位遠征首領」的機率：僅在無首領在世時才擲，
/// 稀有到值得召集夥伴，又不會多天不見蹤影（一遊戲日 = 10 分鐘真實時間，換算約每 70 分鐘一位）。
pub const DAWN_SPAWN_CHANCE: f32 = 0.14;

/// 首領生成環：距村莊中心的最近／最遠半徑（方塊）。刻意落在暗潮之夜生成環
/// （18~36）與暗影村莊庇護圈（48）**之外**——首領生在真正的遠方，找到它本身就是一趟遠征。
pub const RING_MIN: f32 = 90.0;
pub const RING_MAX: f32 = 160.0;

/// 兩次有效挖擊之間的最短間隔（秒）：伺服器端節流，擋封包連發瞬殺（濫用防護，與暗影同款）。
pub const HIT_MIN_INTERVAL_SECS: f32 = 0.25;

/// 首領體型高度（巨大體型，遠比暗影(0.9)或居民壯碩，一眼可辨是「首領」；供觸及判定取中心
/// 用，前端渲染尺寸亦以此為準，不另立一份數值）。
pub const BOSS_HEIGHT: f32 = 3.2;

/// 挖擊觸及判定的額外餘裕（方塊）：首領體型巨大，餘裕略寬於暗影，貼近巨獸周身都打得到。
pub const REACH_BONUS: f32 = 1.6;

/// 擊倒獎勵：一次性掉落的乙太礦數量（遠高於一般暗影的 1~2 枚，值回一趟遠征的溫柔獎勵）。
pub const DEFEAT_REWARD_SHARDS: u32 = 12;

/// 首領存續上限（秒）：生成後這麼久仍未被打倒就自行撤退消散，讓出下一次生成的機會。
/// 一遊戲日 ≈ 600 秒真實時間，1800 秒 ≈ 3 個日夜——夠玩家組隊遠征，又不會在首領落在
/// 難以抵達的地點時（如深水/懸崖）永久卡死「僅在無首領在世才擲骰」的下一次生成。
pub const BOSS_LIFETIME_SECS: u64 = 1800;

// ── 面向玩家字串（集中一處，i18n 友善）─────────────────────────────────────────

pub const FEED_KIND: &str = "遠征首領";
pub const FEED_ACTOR: &str = "乙太方界";

/// 首領現身橫幅（模板，`{dir}` 換成方位詞如「西北方」）。
pub fn spawn_msg(dir: &str) -> String {
    format!("🌋 {BOSS_NAME}現身於{dir}遠處——集結夥伴、帶上武器，去會一會這頭巨獸吧！")
}

/// 首領現身動態牆句。
pub fn spawn_feed(dir: &str) -> String {
    format!("{dir}遠處傳來低沉的巨響，一頭{BOSS_NAME}現身了——這是一趟值得召集夥伴的遠征。")
}

/// 首領擊倒橫幅。
pub fn defeat_msg() -> String {
    format!("🎉 {BOSS_NAME}倒下了！大家一起完成了這趟遠征。")
}

/// 首領擊倒動態牆句。
pub fn defeat_feed() -> String {
    format!("{BOSS_NAME}在遠方倒下，化成一堆乙太礦——這趟遠征，值得。")
}

/// 首領逾期未被打倒、自行撤退時的橫幅（與擊倒 razor-sharp 區隔：沒有慶祝、沒有獎勵，
/// 只是溫柔告知「這次沒趕上」，隔天仍有機會再遇到新的一位）。
pub fn retreat_msg() -> String {
    format!("🌫️ {BOSS_NAME}悄悄退回了更遠的地方，這次沒能趕上……或許改天還會再遇見。")
}

/// 首領撤退動態牆句。
pub fn retreat_feed() -> String {
    format!("{BOSS_NAME}在遠方徘徊了許久，始終無人趕到，終究悄悄退去了。")
}

/// 首領是否已逾存續上限（純函式、可測；`now`/`spawned_at` 皆為秒數時間戳，
/// `saturating_sub` 防時鐘異常/重播亂序下溢 panic）。
pub fn is_expired(spawned_at: u64, now: u64) -> bool {
    now.saturating_sub(spawned_at) >= BOSS_LIFETIME_SECS
}

// ── 首領本體 ─────────────────────────────────────────────────────────────────

/// 遠征首領的權威狀態（伺服器算，客戶端只渲染＋畫 HP 條）。全服至多同時存在一位。
#[derive(Clone, Debug)]
pub struct WorldBoss {
    /// 腳底位置（與暗影/居民同語意：y = AABB 底）。原地不動，v1 刻意不追人。
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// 目前血量（0 表示已倒下，倒下當下即從世界移除，理論上不會被序列化出 0）。
    pub hp: u32,
    /// 生成時的秒數時間戳，用於 [`is_expired`] 判定是否該撤退——防止生成在難以抵達的
    /// 地點時永久卡死「僅在無首領在世才擲骰」的下一次生成（follow-up，PR #1260 review）。
    pub spawned_at: u64,
    /// 居民聞訊馳援 v2（ROADMAP 983）累計對這位首領造成的傷害：達 [`ASSIST_TOTAL_DAMAGE_CAP`]
    /// 後居民即使仍貼著首領打轉，傷害也不再生效——這是「陪伴不是代打」唯一被真正強制的地方
    /// （單看擊擊間隔/單次傷害不足以限制「打多久」，見模組頭註）。新生成的首領歸零。
    pub assist_damage: u32,
}

// ── 純函式（確定性、可測）────────────────────────────────────────────────────

/// 今天這次檢查是否該生成一位首領（僅在無首領在世、機率擲中時）。純函式、可測。
pub fn should_spawn(active: bool, roll: f32) -> bool {
    !active && roll < DAWN_SPAWN_CHANCE
}

/// 依 [0,1) 隨機數在生成環 [`RING_MIN`]..[`RING_MAX`] 內取一個距村莊中心的生成距離（方塊）。
/// 隨機性由呼叫端給，這裡只做確定性映射（界外輸入夾住永不越環）。純函式、可測。
pub fn spawn_dist(roll: f32) -> f32 {
    RING_MIN + roll.clamp(0.0, 1.0) * (RING_MAX - RING_MIN)
}

/// 依村莊中心＋角度＋距離算出首領生成點（幾何純函式，與暗影/暗潮同構手法）。
pub fn spawn_pos(vcx: f32, vcz: f32, angle: f32, dist: f32) -> (f32, f32) {
    (vcx + angle.cos() * dist, vcz + angle.sin() * dist)
}

/// 一次挖擊套用在首領血量上：至少扣 1 點（power=0 防卡死永不打倒），到 0 即倒下。
/// 回傳（新血量, 是否倒下）。純函式、可測。
pub fn register_hit(hp: u32, power: u8) -> (u32, bool) {
    let dmg = power.max(1) as u32;
    let nh = hp.saturating_sub(dmg);
    (nh, nh == 0)
}

/// 挖擊觸及驗證：玩家眼睛到首領中心的距離平方 ≤ (REACH+餘裕)²
/// （**後端權威**：客戶端只自報「我在打首領」，打不打得到由伺服器算）。純函式、可測。
pub fn hit_in_reach(px: f32, py: f32, pz: f32, bx: f32, by: f32, bz: f32) -> bool {
    let dx = bx - px;
    let dy = (by + BOSS_HEIGHT * 0.5) - (py + crate::voxel::EYE_HEIGHT);
    let dz = bz - pz;
    let max = crate::voxel::REACH + REACH_BONUS;
    dx * dx + dy * dy + dz * dz <= max * max
}

// ── v2：居民聞訊馳援 調性參數（自主提案切片，ROADMAP 983）─────────────────────────

/// 世界層級同時馳援首領的居民人數上限：v1 讓玩家孤軍作戰，即使巨蝕者近在咫尺、居民依然
/// 我行我素——這違背「AI 居民真的活著」的核心信念。v2 讓極少數（**至多 2 位，不分玩家**）
/// 真正閒著的居民自發啟程去現場陪你——刻意鎖死這麼低的上限，遠不足以扛住 36 血，
/// 玩家仍是撐起這場遠征的主力，居民只是溫暖的陪伴，不是代打的隊友。
pub const ASSIST_MAX_RESIDENTS: usize = 2;

/// 每次週期檢查「要不要去馳援」時擲中的機率：偏低（三成上下），讓居民出發**不是保證會來**，
/// 是「聽到消息、剛好那時候閒著，就起念去看看」的自然機率感，不是首領一現身就秒收到 NPC 支援。
pub const ASSIST_JOIN_CHANCE: f32 = 0.35;

/// 週期檢查間隔（秒）：夠久才擲一次骰，避免同一位居民短時間內被反覆判定、也避免每 tick
/// 都在算機率浪費算力；夠短則首領現身後不至於等上老半天都等不到任何居民反應。
pub const ASSIST_CHECK_INTERVAL_SECS: f32 = 25.0;

/// 居民對首領的挖擊間隔（秒）：遠比玩家的 [`HIT_MIN_INTERVAL_SECS`]（0.25 秒）慢十幾倍——
/// 這是「陪你去」而非「代打你」的核心紀律：居民絕不可能比你更快削掉首領的血。
pub const ASSIST_HIT_INTERVAL_SECS: f32 = 4.0;

/// 居民單次挖擊的傷害：象徵性的 1 點（首領 36 血，兩位居民從頭陪到尾、每 4 秒 1 點，
/// 全場撐死也只削掉個位數血量——貢獻看得見，但打倒首領的主力永遠是玩家）。
pub const ASSIST_HIT_POWER: u8 = 1;

/// 一次馳援（因首領倒下或撤退而結束）後的冷卻秒數：同一位居民不會下一刻又立刻再被選中，
/// 讓「這位也去過」有份量，也把馳援名額騰給其他居民雨露均霑。
pub const ASSIST_COOLDOWN_SECS: f32 = 300.0;

/// 馳援抵達後貼著首領打轉的閒晃半徑（比照小圈子聚會 [`crate::voxel_clique::GATHER_WANDER_RADIUS`]
/// 同量級的小範圍）：要夠小才能穩定落在 [`hit_in_reach`] 判定範圍內，不會閒晃著閒晃著又走遠。
pub const ASSIST_WANDER_RADIUS: f32 = 4.0;

/// 居民對**同一位**首領累計能造成的傷害世界層級硬上限：象徵性個位數，是「陪伴不是代打」
/// 唯一被程式碼真正強制的地方——[`ASSIST_HIT_INTERVAL_SECS`]／[`ASSIST_HIT_POWER`] 只限制
/// 「多快」，不限制「打多久」；首領存續上限 [`BOSS_LIFETIME_SECS`] 長達 1800 秒，若無這道
/// 封頂，兩位居民全程陪打理論上能在數分鐘內單獨磨死首領。達封頂後居民仍會貼著首領打轉、
/// 冒挖擊動作（陪伴感不變），只是傷害不再生效。
pub const ASSIST_TOTAL_DAMAGE_CAP: u32 = 6;

/// 本次還能讓居民對首領造成多少傷害（純函式、可測）：`requested` 超過剩餘額度時打折，
/// 額度用完後（`dealt_so_far` ≥ 上限）恆回 0，不會因 `requested` 再大而溢出上限。
pub fn assist_damage_remaining(dealt_so_far: u32, requested: u32) -> u32 {
    requested.min(ASSIST_TOTAL_DAMAGE_CAP.saturating_sub(dealt_so_far))
}

/// 記憶哨兵鍵（比照 `voxel_expedition::EXPEDITION_MEMORY_PLAYER` 同款慣例）：馳援面對的是
/// 首領這個世界事件，不是某位特定玩家，記憶的 `player` 欄位掛此標籤供日記/回想引用辨識。
pub const ASSIST_MEMORY_PLAYER: &str = "__voxel_boss_assist__";

/// 是否該啟程馳援（純函式、可測）：目前世界馳援人數未達上限，且擲中機率門檻。
pub fn should_join_assist(active_count: usize, roll: f32) -> bool {
    active_count < ASSIST_MAX_RESIDENTS && roll < ASSIST_JOIN_CHANCE
}

/// 居民決定啟程馳援時的冒泡句（i18n 集中池，帶入 [`BOSS_NAME`]）。
pub fn assist_join_bubble(pick: usize) -> String {
    const POOL: [&str; 3] = [
        "聽說{boss}就在附近……我去看看能不能幫上一點忙！",
        "反正現在也沒什麼事，我也去{boss}那邊撐個場面！",
        "與其在這裡閒著，不如去{boss}那邊陪你一起打！",
    ];
    POOL[pick % POOL.len()].replace("{boss}", BOSS_NAME)
}

/// 首領倒下時，留給有到場馳援的居民各自一筆記憶（純函式、可測）。
pub fn assist_defeat_memory() -> String {
    format!("跟大家一起去攻打了{BOSS_NAME}，牠倒下的那一刻，我也在場！")
}

/// 首領倒下時，額外點名參與馳援的居民的動態牆句（純函式、可測；`names` 需至少 1 位，
/// 呼叫端只在馳援名單非空時才呼叫此函式）。
pub fn assist_defeat_feed(names: &[String]) -> String {
    let joined = names.join("、");
    format!("{joined}也隨隊出力，見證了{BOSS_NAME}倒下的那一刻。")
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_spawn_only_when_inactive_and_under_threshold() {
        assert!(should_spawn(false, 0.0), "無首領在世＋擲到最小值 → 生成");
        assert!(should_spawn(false, DAWN_SPAWN_CHANCE - 0.001), "門檻前一點點 → 生成");
        assert!(!should_spawn(false, DAWN_SPAWN_CHANCE), "剛好等於門檻 → 不生成（嚴格小於）");
        assert!(!should_spawn(false, 0.99), "多數天都不會生成");
        assert!(!should_spawn(true, 0.0), "已有首領在世 → 無論擲骰多小都不再生成");
    }

    #[test]
    fn spawn_dist_stays_within_ring_and_clamps() {
        assert_eq!(spawn_dist(0.0), RING_MIN);
        assert_eq!(spawn_dist(1.0), RING_MAX);
        let mid = spawn_dist(0.5);
        assert!(mid > RING_MIN && mid < RING_MAX);
        // 界外輸入（理論上呼叫端不會給，但防禦性夾住）永不越環。
        assert_eq!(spawn_dist(-1.0), RING_MIN);
        assert_eq!(spawn_dist(2.0), RING_MAX);
    }

    #[test]
    fn spawn_pos_geometry_sane() {
        let (x, z) = spawn_pos(100.0, 100.0, 0.0, 50.0);
        assert!((x - 150.0).abs() < 0.01, "角度 0 → 正 x 方向");
        assert!((z - 100.0).abs() < 0.01);
        let (x2, z2) = spawn_pos(0.0, 0.0, std::f32::consts::FRAC_PI_2, 10.0);
        assert!(x2.abs() < 0.01, "角度 90 度 → x 分量趨近 0");
        assert!((z2 - 10.0).abs() < 0.01);
    }

    #[test]
    fn register_hit_reduces_and_floors_at_zero() {
        let (hp, dead) = register_hit(10, 3);
        assert_eq!(hp, 7);
        assert!(!dead);
        let (hp2, dead2) = register_hit(2, 5);
        assert_eq!(hp2, 0, "扣過頭不會下溢，鎖底在 0");
        assert!(dead2);
        let (hp3, dead3) = register_hit(5, 0);
        assert_eq!(hp3, 4, "power=0 仍至少扣 1，防卡死永不打倒");
        assert!(!dead3);
        let (hp4, dead4) = register_hit(1, 1);
        assert_eq!(hp4, 0);
        assert!(dead4, "剛好扣到 0 才算倒下");
    }

    #[test]
    fn hit_in_reach_true_within_false_beyond() {
        assert!(hit_in_reach(0.0, 0.0, 0.0, 1.0, 0.0, 1.0), "貼近首領理應打得到");
        assert!(!hit_in_reach(0.0, 0.0, 0.0, 200.0, 0.0, 200.0), "遠在天邊打不到");
    }

    #[test]
    fn player_strings_mention_direction_and_nonempty() {
        assert!(spawn_msg("西北方").contains("西北方"));
        assert!(spawn_feed("東方").contains("東方"));
        assert!(!defeat_msg().is_empty());
        assert!(defeat_feed().contains(BOSS_NAME));
        assert!(!BOSS_NAME.is_empty());
        assert!(retreat_msg().contains(BOSS_NAME));
        assert!(retreat_feed().contains(BOSS_NAME));
    }

    #[test]
    fn is_expired_only_after_lifetime_elapsed() {
        assert!(!is_expired(1000, 1000), "剛生成不算逾期");
        assert!(!is_expired(1000, 1000 + BOSS_LIFETIME_SECS - 1), "還差一秒未到上限");
        assert!(is_expired(1000, 1000 + BOSS_LIFETIME_SECS), "剛好到上限即算逾期");
        assert!(is_expired(1000, 1000 + BOSS_LIFETIME_SECS + 500), "遠遠超過上限");
        assert!(!is_expired(1000, 500), "時鐘異常倒退（now < spawned_at）不 panic、視為未逾期");
    }

    #[test]
    fn boss_is_meaningfully_tougher_than_a_single_shadow_wisp() {
        // 首領血量遠高於暗影（3 下消散），確保「值得召集夥伴」的體感，不是換皮暗影。
        assert!(BOSS_MAX_HP > 10);
        // 生成環在暗潮之夜生成環（18~36）與暗影村莊庇護圈（48）之外，是真正的遠方。
        assert!(RING_MIN > 48.0);
    }

    // ── v2：居民聞訊馳援 ────────────────────────────────────────────────────

    #[test]
    fn should_join_assist_respects_max_residents() {
        // 未達上限 + 擲中門檻 → 啟程。
        assert!(should_join_assist(0, 0.0));
        assert!(should_join_assist(1, ASSIST_JOIN_CHANCE - 0.001));
        // 剛好等於門檻 → 不啟程（嚴格小於，同 should_spawn 慣例）。
        assert!(!should_join_assist(0, ASSIST_JOIN_CHANCE));
        assert!(!should_join_assist(1, 0.99));
        // 已達上限 → 無論擲骰多小都不再啟程（世界層級硬上限，不因機率破例）。
        assert!(!should_join_assist(ASSIST_MAX_RESIDENTS, 0.0));
        assert!(!should_join_assist(ASSIST_MAX_RESIDENTS + 1, 0.0));
    }

    #[test]
    fn should_join_assist_boundary_just_under_max() {
        // 剛好差一位就滿（active_count = MAX - 1）仍可啟程，湊滿上限後才擋。
        assert!(should_join_assist(ASSIST_MAX_RESIDENTS - 1, 0.0));
    }

    #[test]
    fn assist_join_bubble_pool_nonempty_and_mentions_boss() {
        for pick in 0..6 {
            let line = assist_join_bubble(pick);
            assert!(!line.is_empty());
            assert!(line.contains(BOSS_NAME), "冒泡句應提及首領名，讓玩家看得懂在講誰");
        }
        // 至少兩種不同句子輪替（防退化成單一模板、喪失「輪替 i18n 池」的意義）。
        let a = assist_join_bubble(0);
        let b = assist_join_bubble(1);
        assert_ne!(a, b, "不同 pick 應輪替到不同句子");
    }

    #[test]
    fn assist_defeat_memory_nonempty_and_mentions_boss() {
        let mem = assist_defeat_memory();
        assert!(!mem.is_empty());
        assert!(mem.contains(BOSS_NAME));
    }

    #[test]
    fn assist_defeat_feed_lists_single_name() {
        let names = vec!["露娜".to_string()];
        let feed = assist_defeat_feed(&names);
        assert!(feed.contains("露娜"));
        assert!(feed.contains(BOSS_NAME));
    }

    #[test]
    fn assist_defeat_feed_joins_multiple_names() {
        let names = vec!["露娜".to_string(), "諾娃".to_string()];
        let feed = assist_defeat_feed(&names);
        assert!(feed.contains("露娜"));
        assert!(feed.contains("諾娃"));
        assert!(feed.contains("、"), "多人應以頓號串接，讀起來像一份點名名單");
    }

    #[test]
    fn assist_hit_interval_far_slower_than_player() {
        // 護欄：居民挖擊間隔必須遠慢於玩家（HIT_MIN_INTERVAL_SECS），確保絕不代打。
        assert!(ASSIST_HIT_INTERVAL_SECS > HIT_MIN_INTERVAL_SECS * 10.0);
    }

    #[test]
    fn assist_hit_power_is_symbolic() {
        // 護欄：單次傷害象徵性極輕，兩人全程陪打也遠遠打不完 36 血。
        assert!(ASSIST_HIT_POWER <= 2);
        assert!(BOSS_MAX_HP as u32 > ASSIST_MAX_RESIDENTS as u32 * ASSIST_HIT_POWER as u32 * 5);
    }

    #[test]
    fn assist_memory_player_sentinel_nonempty() {
        assert!(!ASSIST_MEMORY_PLAYER.is_empty());
    }

    #[test]
    fn assist_damage_remaining_caps_and_floors_at_zero() {
        // 額度充足時全額放行。
        assert_eq!(assist_damage_remaining(0, 2), 2);
        // 超過剩餘額度時打折，只給剩下的份額。
        assert_eq!(assist_damage_remaining(ASSIST_TOTAL_DAMAGE_CAP - 1, 5), 1);
        // 恰好用完後（含超過）恆回 0，不因 requested 再大而溢出上限。
        assert_eq!(assist_damage_remaining(ASSIST_TOTAL_DAMAGE_CAP, 5), 0);
        assert_eq!(assist_damage_remaining(ASSIST_TOTAL_DAMAGE_CAP + 3, 5), 0);
        // requested=0（本 tick 沒人打中）恆回 0，不會憑空生出傷害。
        assert_eq!(assist_damage_remaining(0, 0), 0);
    }

    #[test]
    fn assist_total_damage_cap_far_below_boss_hp_even_at_full_lifetime() {
        // 護欄：即使首領全程存活滿 BOSS_LIFETIME_SECS、兩位居民全程陪打不間斷，累計傷害仍
        // 遠遠打不完 36 血——這是「陪伴不是代打」唯一真正被強制的地方（見模組頭註）。
        let max_possible_hits_per_resident =
            (BOSS_LIFETIME_SECS as f32 / ASSIST_HIT_INTERVAL_SECS).ceil() as u32;
        let theoretical_max_without_cap =
            max_possible_hits_per_resident * ASSIST_MAX_RESIDENTS as u32 * ASSIST_HIT_POWER as u32;
        // 沒有封頂的話理論傷害遠遠超過首領血量（證明封頂確實有存在的必要）。
        assert!(theoretical_max_without_cap > BOSS_MAX_HP as u32 * 10);
        // 有封頂後，居民能造成的傷害遠低於首領血量。
        assert!(ASSIST_TOTAL_DAMAGE_CAP < BOSS_MAX_HP as u32 / 2);
    }
}
