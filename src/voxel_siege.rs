//! 乙太方界·暗潮之夜（Shadow Siege）v1（自主提案切片）——把散落的暗影第一次串成一場
//! **有起承轉合的集體事件**：某些夜晚（低機率）暗潮湧向村莊中心，全村居民一起警醒奔回家
//! 躲避，你持劍在燈火之間守護大家，天亮暗潮退去，全村一起鬆一口氣、道一聲「我們守住了」。
//!
//! **這一刀補的缺口**：暗影（`voxel_shadow`）至今只是「遠離村莊、全圖最多 6 隻、朝最近的人
//! 緩緩漂」的夜色點綴——夜復一夜都一樣，村莊本身**從不曾真正被威脅**。驅影之劍（887）給了
//! 戰鬥、守夜恩人（888）給了戰鬥的社交後果，卻獨缺一場**值得守護的戰役**。本刀把既有零件
//! （劍／光＝庇護／居民害怕逃家／動態牆）串成那一夜：暗影不再散落村外，而是集中湧向村莊、
//! 突破平時的村莊庇護圈，逼你把村子點亮、守到天明。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **平時暗影（`voxel_shadow`）**＝零散、遠村生成（[`voxel_shadow::VILLAGE_SAFE_RADIUS`] 外
//!   絕不生成）、朝最近的人漂、無事件感；本刀＝**集中、破村庇護圈、湧向村莊中心**的一夜戰役。
//! - **守夜恩人（888）**＝你為單一居民驅散單一暗影換得她的道謝；本刀＝**全村級**的集體警醒與
//!   天明後全村的集體鬆一口氣，尺度與觸發全然不同。
//! - **紀念柱／里程碑（856/885）**＝居民「蓋東西」的成就；本刀是一場**防禦事件**，不蓋任何方塊。
//!
//! **設計調性仍是療癒**（守 `voxel_shadow` 底線）：居民只會怕、奔回家躲避，**絕不掉血**；
//! 天亮暗潮必退、村莊必然守住——張力來自「一起撐過這一夜」的暖，不是輸贏。
//!
//! **純函式層**：確定性、零 LLM、零鎖、零 async、零 IO、可單元測試。狀態機（今晚是否暗潮／
//! 生成／廣播／集體反應）全留在 `voxel_ws.rs`，沿用暗影 tick 既有的短鎖循序＋鎖外處理慣例，
//! 嚴守 prod 死鎖鐵律。

// ── 調性參數（集中一處，日後平衡好調）───────────────────────────────────────────

/// 一個夜晚成為「暗潮之夜」的機率：約每 4~5 夜一次，稀有到值得記得，不會夜夜狼來了。
/// 只在入夜當下擲一次骰（見 `voxel_ws` 的每夜一次判定），決定今晚是不是暗潮之夜。
pub const SIEGE_NIGHT_CHANCE: f32 = 0.22;

/// 暗潮之夜全圖暗影上限：平時是 [`voxel_shadow::MAX_WISPS`]=6，戰役夜翻倍到 12——
/// 明顯更多以成「潮」，但仍遠低於需要 InstancedMesh 的量級（FPS 鐵律，見 #614/#820），
/// 前端沿用既有逐隻渲染即可。
pub const SIEGE_MAX_WISPS: usize = 12;

/// 暗潮之夜每次生成檢查的生成機率：平時 0.5、戀役夜拉高到 0.85——暗影冒得又快又密，
/// 幾十秒內就從村外圍逼到滿潮，營造「一波波湧來」的壓力。
pub const SIEGE_SPAWN_CHANCE: f32 = 0.85;

/// 暗潮生成環：距村莊中心的最近／最遠半徑（方塊）。刻意落在
/// [`voxel_shadow::VILLAGE_SAFE_RADIUS`]=48 **之內**——暗潮之夜「突破」了平時的村莊庇護圈，
/// 在村子周身一圈的暗處現身、朝中心逼近，這正是它與平時「遠村點綴」最根本的分野。
pub const SIEGE_RING_MIN: f32 = 18.0;
pub const SIEGE_RING_MAX: f32 = 36.0;

// ── 面向玩家字串（集中一處，i18n 友善）─────────────────────────────────────────

/// 動態牆事件種類標籤。
pub const FEED_KIND: &str = "暗潮之夜";
/// 動態牆事件主體（世界之聲）。
pub const FEED_ACTOR: &str = "乙太方界";

/// 暗潮降臨橫幅（onset，一夜一次）：告訴玩家發生什麼、該怎麼守。
pub const ONSET_MSG: &str = "🌑 暗潮之夜——暗影正湧向村莊！點亮燈火、持劍，一起守護大家。";
/// 暗潮降臨動態牆句。
pub const ONSET_FEED: &str = "暗潮湧向村莊，居民都躲回了家——快點亮燈火、持劍守護村子！";

/// 暗潮退去橫幅（victory，天亮一次）：一起撐過了這一夜。
pub const VICTORY_MSG: &str = "🌅 天亮了，暗潮退去——大家一起守住了村莊。";
/// 暗潮退去動態牆句。
pub const VICTORY_FEED: &str = "天亮了，暗潮退去。村莊在燈火與守護下平安撐過了這一夜。";

/// 暗潮降臨時，全村居民集體警醒的台詞池（比平時零散害怕更急、更有「大家一起」的味道）。
pub const ALARM_LINES: &[&str] = &[
    "暗潮來了…大家快回家！",
    "今晚不一樣，影子好多…回家躲著！",
    "快、快回屋裡，把燈都點上！",
    "別怕，躲好，撐到天亮就沒事了！",
];

/// 天亮暗潮退去時，全村居民集體鬆一口氣的台詞池。
pub const CHEER_LINES: &[&str] = &[
    "天亮了…我們撐過來了！",
    "呼…暗潮退了，還好有你在。",
    "一起守住了村子，真好。",
    "又是平安的一天，謝謝大家。",
];

// ── 純函式（確定性、可測）────────────────────────────────────────────────────

/// 今晚是否為暗潮之夜（入夜當下擲一次骰）。純函式、可測。
pub fn is_siege_night(roll: f32) -> bool {
    roll < SIEGE_NIGHT_CHANCE
}

/// 暗潮之夜這次生成檢查是否允許再生一隻（夜間 + 未達戰役上限 + 機率擲中）。
/// 與 [`voxel_shadow::can_spawn`] 同構，但吃更高的上限與更高的生成機率。純函式、可測。
pub fn siege_can_spawn(count: usize, shadow_time: bool, roll: f32) -> bool {
    shadow_time && count < SIEGE_MAX_WISPS && roll < SIEGE_SPAWN_CHANCE
}

/// 依 [0,1) 隨機數在暗潮生成環 [`SIEGE_RING_MIN`]..[`SIEGE_RING_MAX`] 內取一個距村莊中心的
/// 生成距離（方塊）。隨機性由呼叫端給，這裡只做確定性映射。純函式、可測。
pub fn siege_spawn_dist(roll: f32) -> f32 {
    SIEGE_RING_MIN + roll.clamp(0.0, 1.0) * (SIEGE_RING_MAX - SIEGE_RING_MIN)
}

/// 依 seed 從集體警醒台詞池挑一句（確定性、可測）。
pub fn alarm_line(seed: usize) -> &'static str {
    ALARM_LINES[seed % ALARM_LINES.len()]
}

/// 依 seed 從集體鬆一口氣台詞池挑一句（確定性、可測）。
pub fn cheer_line(seed: usize) -> &'static str {
    CHEER_LINES[seed % CHEER_LINES.len()]
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel_shadow::{MAX_WISPS, VILLAGE_SAFE_RADIUS};

    #[test]
    fn siege_night_is_a_rare_gate() {
        assert!(is_siege_night(0.0), "擲到最小值 → 是暗潮之夜");
        assert!(is_siege_night(SIEGE_NIGHT_CHANCE - 0.001), "門檻前一點點 → 是暗潮之夜");
        assert!(!is_siege_night(SIEGE_NIGHT_CHANCE), "剛好等於門檻 → 不是（嚴格小於）");
        assert!(!is_siege_night(0.99), "多數夜晚都是平安夜");
        assert!(SIEGE_NIGHT_CHANCE < 0.5, "暗潮之夜必須稀有，不能過半");
    }

    #[test]
    fn siege_cap_exceeds_normal_and_respects_gate() {
        assert!(SIEGE_MAX_WISPS > MAX_WISPS, "戰役夜的暗影上限必須高過平時");
        assert!(siege_can_spawn(0, true, 0.0), "夜間、未達上限、擲中 → 可生");
        assert!(!siege_can_spawn(SIEGE_MAX_WISPS, true, 0.0), "到戰役上限絕不再生");
        assert!(siege_can_spawn(MAX_WISPS, true, 0.0), "平時的上限之外、戰役夜仍能再生（潮更滿）");
        assert!(!siege_can_spawn(0, false, 0.0), "白天絕不生成");
        assert!(!siege_can_spawn(0, true, 1.0), "沒擲中不生");
    }

    #[test]
    fn siege_ring_breaches_the_village_safe_radius() {
        // 暗潮之夜的整個生成環都落在平時的村莊庇護圈之內——這就是「破圈逼近」的定義。
        assert!(SIEGE_RING_MIN < SIEGE_RING_MAX, "環有厚度");
        assert!(SIEGE_RING_MAX < VILLAGE_SAFE_RADIUS, "生成環最外圈仍在平時庇護半徑之內＝突破了庇護圈");
    }

    #[test]
    fn siege_spawn_dist_stays_in_ring() {
        assert!((siege_spawn_dist(0.0) - SIEGE_RING_MIN).abs() < 1e-3, "roll=0 → 環最內圈");
        assert!((siege_spawn_dist(1.0) - SIEGE_RING_MAX).abs() < 1e-3, "roll=1 → 環最外圈");
        let mid = siege_spawn_dist(0.5);
        assert!(mid > SIEGE_RING_MIN && mid < SIEGE_RING_MAX, "中間值落在環內");
        // 界外輸入被夾住，永遠不越環。
        assert!((siege_spawn_dist(-1.0) - SIEGE_RING_MIN).abs() < 1e-3, "負 roll 夾到內圈");
        assert!((siege_spawn_dist(9.0) - SIEGE_RING_MAX).abs() < 1e-3, "過大 roll 夾到外圈");
    }

    #[test]
    fn lines_wrap_and_are_nonempty() {
        for i in 0..ALARM_LINES.len() * 2 {
            assert!(!alarm_line(i).is_empty(), "警醒台詞非空且循環");
        }
        for i in 0..CHEER_LINES.len() * 2 {
            assert!(!cheer_line(i).is_empty(), "鬆一口氣台詞非空且循環");
        }
        // 循環：跨過池長回到第一句。
        assert_eq!(alarm_line(0), alarm_line(ALARM_LINES.len()), "警醒台詞循環對齊");
        assert_eq!(cheer_line(0), cheer_line(CHEER_LINES.len()), "鬆一口氣台詞循環對齊");
    }
}
