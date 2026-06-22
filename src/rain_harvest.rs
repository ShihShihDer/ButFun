//! 雨天豐澤（ROADMAP 502）：草原細雨中收成的作物每株多給 +1 乙太。
//!
//! 設計基調：
//! - **雨水不只保護作物，也滋養豐收**：既有 `rain_regen`（ROADMAP 496）讓細雨在戶外緩緩回血、
//!   `GrasslandRain` 已自動替農地澆灌（ROADMAP 109）——本切片把「雨天照料農地」的酬賞鏈收尾：
//!   親手在雨中收成，每株多得 1 乙太（稱為「雨天豐澤」），作為雨水滋養的具體回饋。
//! - **純正向、無懲罰**：不下雨時收成與往常完全相同；下雨時只有加成、沒有任何倒扣。
//! - **語意明確、不燒 LLM、零持久化、零 migration**：bonus 由 `is_raining` 即算即用，
//!   不需新欄位、不改資料表、重啟無損。

/// 每株成熟作物在草原細雨中多得的乙太（「雨天豐澤」加成）。
pub const RAIN_HARVEST_BONUS: u32 = 1;

/// 計算雨天豐澤加成：下雨時回傳 `RAIN_HARVEST_BONUS`，否則 0。
/// 純函式、確定性；呼叫端把返回值疊加到每株作物的乙太結算裡。
pub fn rain_harvest_bonus(is_raining: bool) -> u32 {
    if is_raining { RAIN_HARVEST_BONUS } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_bonus_when_not_raining() {
        assert_eq!(rain_harvest_bonus(false), 0);
    }

    #[test]
    fn bonus_when_raining() {
        assert_eq!(rain_harvest_bonus(true), RAIN_HARVEST_BONUS);
        assert!(rain_harvest_bonus(true) > 0);
    }

    #[test]
    fn bonus_value_is_one() {
        // 確認加成值為 1（療癒向、不破壞乙太平衡）。
        assert_eq!(RAIN_HARVEST_BONUS, 1);
    }

    #[test]
    fn bonus_is_additive_with_count() {
        // 多株收成時，加成應逐株累加（模擬 ws.rs 的結算方式）。
        let count = 3u32;
        let bonus_per = rain_harvest_bonus(true);
        assert_eq!(bonus_per * count, 3);
    }

    #[test]
    fn not_raining_zero_regardless_of_context() {
        // 不下雨就是 0，不受任何其他狀態影響。
        for _ in 0..5 {
            assert_eq!(rain_harvest_bonus(false), 0);
        }
    }
}
