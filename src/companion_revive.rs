//! 同伴扶起·把倒下的旅人扶起來（ROADMAP 464「療癒多人世界」社交切片）。
//!
//! 在此之前，玩家被打趴（`Vitals::is_downed`，血歸零）只能**獨自**趴在原地等 `RECOVERY_SECS`
//! 秒的休息倒數跑完，再被傳回新手村（見 `game.rs` 的「從倒地復原的那一 tick」）——同伴就算
//! 站在身旁也幫不上忙，只能眼睜睜看他被拉走。本模組讓倒地第一次有了**同伴救援**：附近站著的
//! 旅人走過去、伸手把他扶起來——倒地者就地半血起身（免去折返新手村的失落），救人者與被救者
//! 之間升起一道暖光。這正是一個療癒多人世界該有的拉力：受傷時有人會來扶你，而不是各自孤獨。
//!
//! ## 設計鐵律
//! - **純邏輯可測**：扶起的「能不能搆到」半徑判定是純函式，與 IO／鎖／WebSocket 無關，可獨立
//!   窮舉測試。實際「扶起誰」的跨玩家寫入在 `ws.rs`（先放掉救人者的借用、再單獨一輪寫鎖，
//!   不巢狀上鎖，守 prod 死鎖鐵律）；半血起身的狀態變更收斂在 `Vitals::revive`（亦純函式可測）。
//! - **療癒向、零平衡風險**：扶起只是把倒地者就地半血喚醒，不送物品／乙太／戰力／經驗，
//!   不縮短任何冷卻；對經濟與戰鬥平衡零擾動。救人者本身毫髮無損地付出一份善意。
//! - **零持久化、零 migration、零 LLM**：扶起是一瞬的社交事件，純記憶體前置（重連／重啟清空）。

/// 扶起搆得到的半徑（像素）：救人者要靠到倒地者這個範圍內才扶得起來。
/// 取「蹲下伸手」的親近距離——必須真的湊到身邊，鼓勵玩家跑過去救人、而非隔空。
/// 刻意比圍爐分食（`meal_share::SHARE_RADIUS_PX` = 150）更近：分食是聞香外溢，扶起是貼身相助。
pub const REVIVE_RADIUS_PX: f32 = 72.0;

/// 救人者 `(rx, ry)` 與倒地者 `(dx, dy)` 是否近到搆得著扶起（含半徑端點）。
/// 任一座標非有限一律回 `false`（壞資料保守不扶起，不致誤判或隔空救人）。純函式。
pub fn within_revive_range(rx: f32, ry: f32, dx: f32, dy: f32) -> bool {
    let (ox, oy) = (rx - dx, ry - dy);
    ox.is_finite() && oy.is_finite() && (ox * ox + oy * oy) <= REVIVE_RADIUS_PX * REVIVE_RADIUS_PX
}

/// 在一群倒地候選者中，挑出離救人者「最近、且搆得著」的那一位的索引。
/// `candidates` 為各倒地者的座標；回傳最近者在 `candidates` 中的索引，沒人搆得著回 `None`。
/// 純函式：不碰玩家結構、只吃座標，方便窮舉測試「多人倒地時扶最近的那個」。
pub fn nearest_revivable(rx: f32, ry: f32, candidates: &[(f32, f32)]) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, &(cx, cy)) in candidates.iter().enumerate() {
        if !within_revive_range(rx, ry, cx, cy) {
            continue;
        }
        let (ox, oy) = (rx - cx, ry - cy);
        let d2 = ox * ox + oy * oy;
        match best {
            Some((_, bd)) if bd <= d2 => {}
            _ => best = Some((i, d2)),
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_range_includes_endpoint_and_excludes_beyond() {
        // 原點上正好半徑端點：搆得著（含端點）。
        assert!(within_revive_range(0.0, 0.0, REVIVE_RADIUS_PX, 0.0));
        // 超過一點點：搆不著。
        assert!(!within_revive_range(0.0, 0.0, REVIVE_RADIUS_PX + 0.1, 0.0));
        // 貼身：搆得著。
        assert!(within_revive_range(10.0, 10.0, 12.0, 9.0));
    }

    #[test]
    fn non_finite_coords_never_in_range() {
        assert!(!within_revive_range(f32::NAN, 0.0, 0.0, 0.0));
        assert!(!within_revive_range(0.0, 0.0, f32::INFINITY, 0.0));
    }

    #[test]
    fn nearest_picks_closest_within_range() {
        // 三位倒地者：第 1 位最近且在範圍內 → 應選 index 1。
        let cands = [(60.0, 0.0), (20.0, 0.0), (50.0, 0.0)];
        assert_eq!(nearest_revivable(0.0, 0.0, &cands), Some(1));
    }

    #[test]
    fn nearest_ignores_those_out_of_range() {
        // 唯一在範圍內的是 index 2（其餘都太遠）。
        let cands = [(500.0, 0.0), (0.0, 400.0), (40.0, 20.0)];
        assert_eq!(nearest_revivable(0.0, 0.0, &cands), Some(2));
    }

    #[test]
    fn nearest_returns_none_when_all_out_of_range() {
        let cands = [(500.0, 0.0), (0.0, 600.0)];
        assert_eq!(nearest_revivable(0.0, 0.0, &cands), None);
    }

    #[test]
    fn nearest_returns_none_on_empty() {
        assert_eq!(nearest_revivable(0.0, 0.0, &[]), None);
    }
}
