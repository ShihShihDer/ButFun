//! 家園擺飾（ROADMAP 402）的純邏輯地基——「一塊自家田能擺哪一件療癒小物」的權威定義。
//!
//! 這層只管一件事：**哪些擺飾索引是合法的**。索引 0 代表「不擺」（素地），1..=`DECOR_COUNT`
//! 各對應一件擺飾（花圃／石燈籠／風鈴／鳥居／小松／蘑菇……）。擺飾的**外觀與名稱**（emoji、
//! 繁中標籤）一律住在前端（單一呈現來源、好在地化），後端只當「契約守門人」：驗證玩家送來的
//! 索引在範圍內，越界一律當「不擺」處理（防偽造／髒值塞進田裡）。
//!
//! 純資料 + 純函式，無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試。接線見：
//!   - `field.rs`：`Field` 帶一個 `home_decor: u8`（legacy 單件）＋ `garden: Vec<u8>`（ROADMAP
//!     416 多格庭園），`set_home_decor` / `set_garden_slot` 走這裡 `sanitize` / `sanitize_garden`。
//!   - `ws.rs`：`ClientMsg::SetHomeDecor` / `SetGardenSlot` 把玩家選的索引交給自己田。
//!   - 前端：依索引在田上畫對應擺飾、提供擺飾選擇器（索引↔emoji/標籤在前端對映）。

/// 合法擺飾的數量（不含「不擺」的 0）。前端的擺飾目錄長度須為 `DECOR_COUNT + 1`（含 0=不擺），
/// 兩端以此數字為契約對齊；要加新擺飾時，這裡 +1、前端目錄同步補一格即可。
/// ROADMAP 416：庭園化把目錄從 6 擴到 12（向日葵／蓮花／玫瑰／盆栽／石像／噴泉）。
pub const DECOR_COUNT: u8 = 12;

/// 自家庭園的擺放位數量（ROADMAP 416）：田面有這麼多個固定位置，各自能擺一件小物（或不擺）。
pub const GARDEN_SLOTS: usize = 6;

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

/// 把整座庭園的擺放位陣列夾成合法狀態（ROADMAP 416）：超過 `GARDEN_SLOTS` 的多餘格一律截掉
/// （防偽造塞超量），每一格走 `sanitize`（越界／髒值回 0=不擺）。原地修改。
pub fn sanitize_garden(slots: &mut Vec<u8>) {
    slots.truncate(GARDEN_SLOTS);
    for s in slots.iter_mut() {
        *s = sanitize(*s);
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

    // ── ROADMAP 416 庭園 ────────────────────────────────────────────────────
    #[test]
    fn garden_slot_count_is_stable_contract() {
        // 擺放位數量是前後端契約：前端 GARDEN_SPOTS 長度須與此一致。
        assert_eq!(GARDEN_SLOTS, 6);
    }

    #[test]
    fn sanitize_garden_truncates_overlong() {
        // 偽造塞超量：超過 GARDEN_SLOTS 的多餘格一律截掉。
        let mut slots = vec![1u8; GARDEN_SLOTS + 4];
        sanitize_garden(&mut slots);
        assert_eq!(slots.len(), GARDEN_SLOTS, "超量被截到擺放位數");
        assert!(slots.iter().all(|&s| s == 1), "保留的格原值不變");
    }

    #[test]
    fn sanitize_garden_clamps_each_dirty_slot() {
        // 每格越界／髒值各自回 0=不擺，合法格原樣保留。
        let mut slots = vec![1, DECOR_COUNT, DECOR_COUNT + 1, u8::MAX, 0, 7];
        sanitize_garden(&mut slots);
        assert_eq!(slots, vec![1, DECOR_COUNT, 0, 0, 0, 7]);
    }

    #[test]
    fn sanitize_garden_leaves_clean_short_array_intact() {
        // 合法且不超量：原樣保留（不補零、不截）。
        let mut slots = vec![3, 0, 11];
        sanitize_garden(&mut slots);
        assert_eq!(slots, vec![3, 0, 11]);
    }
}
