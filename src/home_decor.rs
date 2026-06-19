//! 家園擺飾（ROADMAP 402）的純邏輯地基——「一塊自家田能擺哪一件療癒小物」的權威定義。
//!
//! 這層只管一件事：**哪些擺飾索引是合法的**。索引 0 代表「不擺」（素地），1..=`DECOR_COUNT`
//! 各對應一件擺飾（花圃／石燈籠／風鈴／鳥居／小松／蘑菇……）。擺飾的**外觀與名稱**（emoji、
//! 繁中標籤）一律住在前端（單一呈現來源、好在地化），後端只當「契約守門人」：驗證玩家送來的
//! 索引在範圍內，越界一律當「不擺」處理（防偽造／髒值塞進田裡）。
//!
//! 純資料 + 純函式，無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試。接線見：
//!   - `field.rs`：`Field` 帶一個 `home_decor: u8`，`set_home_decor` 走這裡 `sanitize`。
//!   - `ws.rs`：`ClientMsg::SetHomeDecor` 把玩家選的索引交給自己田的 `set_home_decor`。
//!   - 前端：依索引在田上畫對應擺飾、提供擺飾選擇器（索引↔emoji/標籤在前端對映）。

/// 合法擺飾的數量（不含「不擺」的 0）。前端的擺飾目錄長度須為 `DECOR_COUNT + 1`（含 0=不擺），
/// 兩端以此數字為契約對齊；要加新擺飾時，這裡 +1、前端目錄同步補一格即可。
pub const DECOR_COUNT: u8 = 6;

/// 索引是否為合法擺飾值（0=不擺 也算合法）。
pub fn is_valid(index: u8) -> bool {
    index <= DECOR_COUNT
}

/// 把外來索引夾成合法值：合法則原樣保留，越界（髒值／偽造）一律回 0（不擺）。
/// 刻意「越界→不擺」而非「夾到最大」——免得垃圾索引被默默解讀成某件真擺飾。
pub fn sanitize(index: u8) -> u8 {
    if is_valid(index) {
        index
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_means_none_and_is_valid() {
        assert!(is_valid(0));
        assert_eq!(sanitize(0), 0);
    }

    #[test]
    fn all_catalogued_indices_are_valid() {
        for i in 0..=DECOR_COUNT {
            assert!(is_valid(i), "索引 {i} 應在目錄內");
            assert_eq!(sanitize(i), i, "合法索引應原樣保留");
        }
    }

    #[test]
    fn out_of_range_sanitizes_to_none() {
        assert!(!is_valid(DECOR_COUNT + 1));
        assert_eq!(sanitize(DECOR_COUNT + 1), 0, "越界索引應回 0=不擺");
        assert_eq!(sanitize(u8::MAX), 0, "極端髒值也回 0");
    }
}
