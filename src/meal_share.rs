//! 圍爐分食·暖食的香氣分給身旁旅人（ROADMAP 462「療癒多人世界」社交切片）。
//!
//! 在此之前，料理的「暖食飽足」buff（`meal_buff`，ROADMAP 395）只暖到吃下它的那一個人——
//! 一個人煮、一個人吃、一個人被療癒，跟單機沒兩樣。本模組讓進食第一次有了**社交外溢**：
//! 在身旁有旅人的地方吃下一道暖呼呼的料理，香氣會分一份「半份暖意」給附近的人，
//! 大家頭頂一起亮起暖食光暈——像圍著一鍋熱湯取暖。讓「在哪吃、跟誰一起吃」第一次有意義，
//! 也輕輕把玩家拉到彼此身邊（這正是一個療癒多人世界該有的拉力）。
//!
//! ## 設計鐵律
//! - **純邏輯可測**：半徑判定、分享份額皆為純函式，與 IO／鎖／WebSocket 無關，可獨立窮舉測試。
//!   實際「分給誰」的跨玩家寫入在 `ws.rs`（先放掉吃飯者的借用、再單獨一輪寫鎖，不巢狀上鎖）。
//! - **療癒向、零平衡風險**：分到的只是限時 HP 緩慢回復（半份），不送物品／乙太／戰力，
//!   不複製料理（吃飯者照樣只吃掉自己那一份）；對經濟與戰鬥平衡零擾動。
//! - **零持久化、零 migration、零 LLM**：分食是一瞬的社交事件，純記憶體前置（重連／重啟清空）。

use crate::meal_buff::MealBuff;

/// 分食波及半徑（像素）：吃飯者身旁這個範圍內的旅人聞香受惠。
/// 取「圍爐取暖」的親近距離——要靠得夠近才分得到，鼓勵玩家真的湊到一塊。
pub const SHARE_RADIUS_PX: f32 = 150.0;

/// 受惠旅人拿到的暖意比例（時長）：半份。分享是「順手暖一下」，不是把整鍋端給別人——
/// 自己吃下的那份永遠最足，分出去的是減半的餘暖。
pub const SHARE_FRACTION: f32 = 0.5;

/// 單次分食最多惠及的旅人數：防爆量廣播、也避免大群擠團時一口氣刷亮整片光暈。
pub const MAX_RECIPIENTS: usize = 8;

/// 吃飯者 `(ex, ey)` 與旅人 `(px, py)` 是否近到能分到香氣（含半徑端點）。
/// 任一座標非有限一律回 `false`（壞資料保守不分食，不致誤判或爆量）。純函式。
pub fn within_share_range(ex: f32, ey: f32, px: f32, py: f32) -> bool {
    let (dx, dy) = (ex - px, ey - py);
    dx.is_finite()
        && dy.is_finite()
        && (dx * dx + dy * dy) <= SHARE_RADIUS_PX * SHARE_RADIUS_PX
}

/// 由吃飯者剛吃下的這份暖食，算出要分給每位旅人的「半份暖意」。純函式（轉呼 `MealBuff::shared`）。
pub fn portion(source: &MealBuff) -> MealBuff {
    source.shared(SHARE_FRACTION)
}

/// 這份分到的暖意是否「值得刷新」旅人現有的飽足狀態：
/// 對方沒在飽足、或這份分享比對方剩下的還久時才覆蓋——絕不把人家更長的暖食蓋成更短的，
/// 分享只會讓人更暖、不會幫倒忙。純函式。
pub fn should_refresh(existing: Option<&MealBuff>, incoming: &MealBuff) -> bool {
    match existing {
        None => true,
        Some(cur) => !cur.is_active() || incoming.remaining_secs > cur.remaining_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::ItemKind;

    fn buff(total: f32) -> MealBuff {
        // 借 DeepBroth 當樣本料理；數值由 total 決定。
        MealBuff::new_for_test(ItemKind::DeepBroth, total, 0.9)
    }

    #[test]
    fn within_range_respects_radius_and_endpoint() {
        // 同點＝0 距離、必在範圍內。
        assert!(within_share_range(100.0, 100.0, 100.0, 100.0));
        // 恰在半徑上（端點）算在內。
        assert!(within_share_range(0.0, 0.0, SHARE_RADIUS_PX, 0.0));
        // 略超半徑＝聞不到。
        assert!(!within_share_range(0.0, 0.0, SHARE_RADIUS_PX + 0.5, 0.0));
        // 對角線：剛好在圓內 vs 圓外。
        let edge = SHARE_RADIUS_PX / (2.0_f32).sqrt(); // (edge, edge) 距原點＝半徑
        assert!(within_share_range(0.0, 0.0, edge - 1.0, edge - 1.0));
        assert!(!within_share_range(0.0, 0.0, edge + 2.0, edge + 2.0));
    }

    #[test]
    fn within_range_rejects_non_finite() {
        // 壞座標保守不分食（不爆、不誤判成超近）。
        assert!(!within_share_range(f32::NAN, 0.0, 0.0, 0.0));
        assert!(!within_share_range(0.0, 0.0, f32::INFINITY, 0.0));
    }

    #[test]
    fn portion_is_half_of_source() {
        let src = buff(50.0);
        let p = portion(&src);
        assert!((p.total_secs - 25.0).abs() < 1e-6, "分給旅人的是半份時長");
        assert!(p.total_secs < src.total_secs, "分出去永遠少於自己吃的那份");
        assert!(p.is_active());
    }

    #[test]
    fn should_refresh_only_when_it_helps() {
        let incoming = buff(20.0); // 剩 20s 的一份分享
        // 對方沒飽足＝該刷。
        assert!(should_refresh(None, &incoming));
        // 對方飽足更短（剩 5s）＝該刷（讓人更暖）。
        let shorter = buff(5.0);
        assert!(should_refresh(Some(&shorter), &incoming));
        // 對方飽足更長（剩 40s）＝不該刷（絕不蓋成更短，不幫倒忙）。
        let longer = buff(40.0);
        assert!(!should_refresh(Some(&longer), &incoming));
        // 對方有 buff 但已散盡（剩 0）＝該刷。
        let mut expired = buff(10.0);
        expired.tick(10.0);
        assert!(!expired.is_active());
        assert!(should_refresh(Some(&expired), &incoming));
    }
}
