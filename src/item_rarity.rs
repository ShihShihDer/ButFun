//! 採集稀有度——採集時偶發品質光環，為採集增加驚喜感。
//!
//! 純邏輯、確定性（種子推導，測試可重現）、零 IO、零持久化、零 migration。
//! 採集後以玩家 UID ⊕ EXP 當種子滾動品質；等級與工具微幅影響機率。
//!
//! 接線端（ws.rs）負責：
//!   1. 呼叫 `roll_rarity` 取得品質。
//!   2. 把 `qty_bonus` 加進這次採集的最終數量。
//!   3. 如果品質 ≥ `Uncommon`，廣播 `GatherQuality` 事件供前端演出飄字。
//!   4. 如果品質 ≥ `Rare`，另發世界頻道宣告。

/// 採集品質等級，由低到高。
/// `wire_str()` 是協議穩定契約——**不可重排、不可改拼字**，改了即破壞前端解析。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rarity {
    /// 普通（~75%）：無特別通知，靜默落袋。
    Common,
    /// 不凡（~20%）：個人飄字，安靜多一個。
    Uncommon,
    /// 稀有（~4%）：個人飄字＋世界頻道宣告。
    Rare,
    /// 史詩（~1%）：個人飄字＋世界大喊。
    Epic,
}

impl Rarity {
    /// 基礎採集量以外額外給的數量。
    pub fn qty_bonus(self) -> u32 {
        match self {
            Rarity::Common   => 0,
            Rarity::Uncommon => 1,
            Rarity::Rare     => 2,
            Rarity::Epic     => 4,
        }
    }

    /// 面向玩家的繁中顯示名。
    pub fn display_zh(self) -> &'static str {
        match self {
            Rarity::Common   => "普通",
            Rarity::Uncommon => "不凡",
            Rarity::Rare     => "稀有",
            Rarity::Epic     => "史詩",
        }
    }

    /// Wire 協議字串（穩定契約，別改、別重排）。
    pub fn wire_str(self) -> &'static str {
        match self {
            Rarity::Common   => "common",
            Rarity::Uncommon => "uncommon",
            Rarity::Rare     => "rare",
            Rarity::Epic     => "epic",
        }
    }

    /// 對應 emoji（Common 為空字串，其餘非空）。
    pub fn emoji(self) -> &'static str {
        match self {
            Rarity::Common   => "",
            Rarity::Uncommon => "✨",
            Rarity::Rare     => "💎",
            Rarity::Epic     => "🌟",
        }
    }

    /// 是否值得通知玩家（不凡以上才飄字）。
    pub fn is_notable(self) -> bool {
        !matches!(self, Rarity::Common)
    }

    /// 是否值得廣播全服（稀有以上才世界宣告）。
    pub fn is_world_announce(self) -> bool {
        matches!(self, Rarity::Rare | Rarity::Epic)
    }
}

/// 根據種子滾出採集品質。
///
/// - `seed`：以玩家 UID 低 64 位元 ⊕ EXP 組成，保持確定性且每次採集都不同。
/// - `level`：玩家等級（高等級微幅提升稀有機率，每 10 級 +0.1%，上限 +0.5%）。
/// - `has_enhanced_tool`：持有強化鎬時稀有機率額外 +0.1%。
///
/// 機率區間（以 10_000 為底）：
/// - 史詩：基礎 100（1%）+ 等級/工具加成。
/// - 稀有：基礎 400（4%）+ 等級/工具加成。
/// - 不凡：基礎 2000（20%）。
/// - 普通：其餘（~75%）。
pub fn roll_rarity(seed: u64, level: u32, has_enhanced_tool: bool) -> Rarity {
    // 線性同餘混合，確定性、無外部 crate。
    let hash = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let roll = (hash >> 33) % 10_000; // 0~9999

    // 每 10 等 +1‰ 稀有（上限 +5‰），每 20 等 +0.5‰ 史詩（上限 +2.5‰）。
    let level_rare_bonus  = ((level / 10) as u64).min(5);
    let level_epic_bonus  = ((level / 20) as u64).min(2);
    let tool_rare_bonus: u64 = if has_enhanced_tool { 1 } else { 0 };
    let tool_epic_bonus: u64 = if has_enhanced_tool { 1 } else { 0 };

    let epic_threshold     = 100 + level_epic_bonus + tool_epic_bonus;
    let rare_threshold     = epic_threshold + 400 + level_rare_bonus + tool_rare_bonus;
    let uncommon_threshold = rare_threshold + 2_000;

    if roll < epic_threshold {
        Rarity::Epic
    } else if roll < rare_threshold {
        Rarity::Rare
    } else if roll < uncommon_threshold {
        Rarity::Uncommon
    } else {
        Rarity::Common
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_rarity_is_deterministic() {
        assert_eq!(roll_rarity(42, 1, false), roll_rarity(42, 1, false));
        assert_eq!(roll_rarity(0, 0, true), roll_rarity(0, 0, true));
        assert_eq!(roll_rarity(u64::MAX, 99, false), roll_rarity(u64::MAX, 99, false));
    }

    #[test]
    fn roll_rarity_never_panics_on_any_seed() {
        for seed in [0u64, 1, 12345, u64::MAX / 2, u64::MAX] {
            for level in [0u32, 1, 10, 50, 100] {
                let r = roll_rarity(seed, level, false);
                let _ = r.display_zh();
                let _ = r.wire_str();
            }
        }
    }

    #[test]
    fn qty_bonus_strictly_increases_with_rarity() {
        assert!(Rarity::Uncommon.qty_bonus() > Rarity::Common.qty_bonus());
        assert!(Rarity::Rare.qty_bonus()     > Rarity::Uncommon.qty_bonus());
        assert!(Rarity::Epic.qty_bonus()     > Rarity::Rare.qty_bonus());
    }

    #[test]
    fn common_has_no_bonus_and_no_notification() {
        assert_eq!(Rarity::Common.qty_bonus(), 0);
        assert!(!Rarity::Common.is_notable());
        assert!(!Rarity::Common.is_world_announce());
        assert!(Rarity::Common.emoji().is_empty());
    }

    #[test]
    fn uncommon_is_notable_but_not_world_announce() {
        assert!(Rarity::Uncommon.is_notable());
        assert!(!Rarity::Uncommon.is_world_announce());
        assert!(!Rarity::Uncommon.emoji().is_empty());
    }

    #[test]
    fn rare_and_epic_are_world_announce() {
        assert!(Rarity::Rare.is_world_announce());
        assert!(Rarity::Epic.is_world_announce());
        assert!(!Rarity::Rare.emoji().is_empty());
        assert!(!Rarity::Epic.emoji().is_empty());
    }

    #[test]
    fn wire_str_is_stable() {
        assert_eq!(Rarity::Common.wire_str(),   "common");
        assert_eq!(Rarity::Uncommon.wire_str(), "uncommon");
        assert_eq!(Rarity::Rare.wire_str(),     "rare");
        assert_eq!(Rarity::Epic.wire_str(),     "epic");
    }

    #[test]
    fn display_zh_covers_all_variants_and_is_nonempty() {
        for r in [Rarity::Common, Rarity::Uncommon, Rarity::Rare, Rarity::Epic] {
            assert!(!r.display_zh().is_empty(), "{r:?} display_zh 不應為空");
        }
    }

    #[test]
    fn epic_threshold_is_rare_enough() {
        // 萬次滾輪中 Epic 出現應 < 3%（保持驚喜感，不能氾濫）。
        let epic_count = (0u64..10_000)
            .filter(|&s| roll_rarity(s, 1, false) == Rarity::Epic)
            .count();
        assert!(epic_count < 300, "史詩出現率過高：{epic_count}/10000");
    }

    #[test]
    fn enhanced_tool_nudges_probability_up() {
        // 強化鎬時稀有+史詩出現率應 ≥ 無工具時。
        let count_without: usize = (0u64..10_000)
            .filter(|&s| matches!(roll_rarity(s, 10, false), Rarity::Rare | Rarity::Epic))
            .count();
        let count_with: usize = (0u64..10_000)
            .filter(|&s| matches!(roll_rarity(s, 10, true), Rarity::Rare | Rarity::Epic))
            .count();
        assert!(count_with >= count_without, "強化工具應提升稀有機率");
    }

    #[test]
    fn high_level_nudges_probability_up() {
        let count_low: usize = (0u64..10_000)
            .filter(|&s| matches!(roll_rarity(s, 1, false), Rarity::Rare | Rarity::Epic))
            .count();
        let count_high: usize = (0u64..10_000)
            .filter(|&s| matches!(roll_rarity(s, 50, false), Rarity::Rare | Rarity::Epic))
            .count();
        assert!(count_high > count_low, "高等級應提升稀有機率");
    }

    #[test]
    fn rarity_distribution_sanity() {
        // 一萬次採集中普通應佔多數（> 50%），確保不凡/稀有/史詩仍屬少數。
        let common_count = (0u64..10_000)
            .filter(|&s| roll_rarity(s, 1, false) == Rarity::Common)
            .count();
        assert!(
            common_count > 5_000,
            "普通品質應佔多數，目前 {common_count}/10000"
        );
    }
}
