//! 農地大豐收廣播（ROADMAP 532）。
//!
//! 玩家一鍵收成（HarvestAll）時，若某品種單次收穫株數 ≥ `BOUNTIFUL_THRESHOLD`，
//! 世界頻道廣播一行慶祝文字，讓「辛苦耕耘的大豐收」第一次在全服可見。
//!
//! 設計準則：
//! - 門檻 10 株：田地最大 30 格，門檻是標準田一半，不輕易觸發也不太罕見。
//! - 只在 HarvestAll（玩家主動意圖）觸發，非逐格收割，避免頻繁廣播。
//! - 零 LLM、零持久化、零 migration。

use crate::crop_variety::CropVariety;

/// 觸發大豐收廣播的最低單次收穫株數（同品種）。
pub const BOUNTIFUL_THRESHOLD: u32 = 10;

/// 給定各品種收成株數（索引 = `CropVariety::code()`），找出「最多且達門檻」的品種與數量。
/// 多品種同時達門檻取數量最多者；平手取 `code` 最小（確定性）。
/// 全都未達門檻回 `None`。
pub fn bountiful_variety(kind_counts: &[u32]) -> Option<(CropVariety, u32)> {
    let mut best: Option<(CropVariety, u32)> = None;
    for code in 0..=2u8 {
        let count = kind_counts.get(code as usize).copied().unwrap_or(0);
        if count < BOUNTIFUL_THRESHOLD {
            continue;
        }
        let variety = match CropVariety::from_code(code) {
            Some(v) => v,
            None => continue,
        };
        match best {
            None => best = Some((variety, count)),
            Some((_, prev_count)) if count > prev_count => best = Some((variety, count)),
            // 平手：code 較小的先出現（循環從 0 開始），保持不換，確定性。
            _ => {}
        }
    }
    best
}

/// 大豐收廣播文字（依 count 輪替 3 種語氣）。
pub fn bountiful_msg(player_name: &str, variety: CropVariety, count: u32) -> String {
    let idx = (count as usize) % 3;
    let zh = variety.zh_name();
    [
        format!("🌾 {player_name} 大豐收！一次收了 {count} 株{zh}——田間真豐盛！"),
        format!("🌾 {player_name} 一鍵收了 {count} 株{zh}，豐收豐收！願四季常豐！"),
        format!("🌾 哇！{player_name} 收了 {count} 株{zh}，這才叫大豐收！"),
    ][idx]
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_harvest_no_bounty() {
        // 9 株，低於門檻，不觸發
        let counts = [9u32, 0, 0];
        assert_eq!(bountiful_variety(&counts), None);
    }

    #[test]
    fn exact_threshold_triggers() {
        // 恰好 10 株即觸發
        let counts = [10u32, 0, 0];
        let result = bountiful_variety(&counts);
        assert!(result.is_some(), "10 株應觸發大豐收");
        let (v, c) = result.unwrap();
        assert_eq!(v, CropVariety::Staple);
        assert_eq!(c, 10);
    }

    #[test]
    fn picks_highest_variety() {
        // 速生菜 12 vs 主食穀 9：回速生菜
        let counts = [9u32, 12, 0];
        let result = bountiful_variety(&counts);
        assert!(result.is_some());
        let (v, c) = result.unwrap();
        assert_eq!(v, CropVariety::Sprout);
        assert_eq!(c, 12);
    }

    #[test]
    fn tie_picks_lower_code() {
        // 主食穀(code=0) 10 株 vs 速生菜(code=1) 10 株：取 code 0（主食穀）
        let counts = [10u32, 10, 0];
        let result = bountiful_variety(&counts);
        assert!(result.is_some());
        let (v, _) = result.unwrap();
        assert_eq!(v, CropVariety::Staple, "平手取 code 最小（主食穀）");
    }

    #[test]
    fn all_below_threshold_none() {
        let counts = [5u32, 3, 9];
        assert_eq!(bountiful_variety(&counts), None);
    }

    #[test]
    fn etherbloom_alone_triggers() {
        let counts = [0u32, 0, 15];
        let result = bountiful_variety(&counts);
        assert!(result.is_some());
        let (v, c) = result.unwrap();
        assert_eq!(v, CropVariety::Etherbloom);
        assert_eq!(c, 15);
    }

    #[test]
    fn message_contains_player_name() {
        let msg = bountiful_msg("薇拉", CropVariety::Etherbloom, 12);
        assert!(msg.contains("薇拉"), "廣播應含玩家名稱");
    }

    #[test]
    fn message_contains_count() {
        let msg = bountiful_msg("農夫甲", CropVariety::Staple, 14);
        assert!(msg.contains("14"), "廣播應含收成數量");
    }

    #[test]
    fn message_contains_variety_name() {
        let msg = bountiful_msg("農夫乙", CropVariety::Sprout, 11);
        assert!(msg.contains("速生菜"), "廣播應含品種名稱");
    }

    #[test]
    fn message_varies_by_count() {
        // 確認輪替有效——不同 count 模 3 產生不同文字開頭
        let m0 = bountiful_msg("X", CropVariety::Staple, 12); // 12%3=0
        let m1 = bountiful_msg("X", CropVariety::Staple, 13); // 13%3=1
        let m2 = bountiful_msg("X", CropVariety::Staple, 14); // 14%3=2
        assert_ne!(m0, m1);
        assert_ne!(m1, m2);
        assert_ne!(m0, m2);
    }
}
