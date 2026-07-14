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
    }

    #[test]
    fn boss_is_meaningfully_tougher_than_a_single_shadow_wisp() {
        // 首領血量遠高於暗影（3 下消散），確保「值得召集夥伴」的體感，不是換皮暗影。
        assert!(BOSS_MAX_HP > 10);
        // 生成環在暗潮之夜生成環（18~36）與暗影村莊庇護圈（48）之外，是真正的遠方。
        assert!(RING_MIN > 48.0);
    }
}
