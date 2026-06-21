//! 街頭合奏·共鳴樂團（ROADMAP 472）——把 399 的「一個人在廣場獻奏」長成「眾人合奏、療癒圍聽」。
//!
//! 當兩位以上的玩家在廣場上彼此靠得夠近（`ENSEMBLE_RADIUS_PX` 內）同時獻奏，便自然湊成一支
//! 「共鳴樂團」；人越多、和聲越厚。樂團演奏時，圍在身旁聆賞的群眾（非獻奏、脫戰、室外的玩家）
//! 隨和聲緩緩回血——街頭獻奏第一次從「賺一點打賞」變成「合奏療癒一整圈聽眾」的正和社交。
//!
//! 設計鐵律（刻意與既有系統乾淨分工、避開最近連發的骨架）：
//! - **純邏輯可測**：`cluster_size`／`is_ensemble`／`harmony_regen_per_sec`／`within_ensemble_range`
//!   皆純函式、無副作用、無 IO、零鎖；遊戲迴圈只在幀初快照上呼叫。
//! - **零持久化、零 migration**：合奏人數是 Player 上每 tick 重算的記憶體前置欄位，不入存檔。
//! - **零經濟擾動**：不改既有打賞乙太（399 的 `tip_ether` 原封不動）；本切片新增的價值全在
//!   「圍聽者的療癒光環」——是脫戰才生效的溫和回血，不發貨幣、不影響掉落／經驗／平衡。
//! - **零 LLM、零外部呼叫**：合奏判定與療癒速率純由距離與人數算術推導。
//! - **玩家一眼有感**：合奏人數放進快照廣播，前端對樂團畫漸強的和聲音符與暖光，圍聽者看見
//!   「🎵 圍聽療癒」並緩緩回血——廣場第一次因玩家自發合奏而熱鬧、互相療癒。

/// 兩位獻奏者相距在此半徑（像素）內即視為同一支共鳴樂團。略大於 399 的聆賞半徑（160），
/// 讓「站得開一點、各自吸引聽眾」的兩位街頭樂手仍能彼此呼應、合成一團。
pub const ENSEMBLE_RADIUS_PX: f32 = 220.0;

/// 兩人合奏時，圍聽者每秒額外回復的基礎 HP（與 467 林蔭小憩同等溫和、不破壞戰鬥張力）。
pub const HARMONY_BASE_PER_SEC: f32 = 1.0;
/// 每多一位合奏者，圍聽療癒速率再加這麼多（人越多和聲越療癒）。
pub const HARMONY_PER_EXTRA_PER_SEC: f32 = 0.5;
/// 圍聽療癒速率上限（防刷：再多人合奏也只回到這麼快）。
pub const HARMONY_MAX_PER_SEC: f32 = 3.0;

/// 判斷兩位獻奏者是否落在同一支樂團的半徑內（純距離比較；非有限座標保守回 false）。
pub fn within_ensemble_range(ax: f32, ay: f32, bx: f32, by: f32) -> bool {
    let dx = bx - ax;
    let dy = by - ay;
    if !dx.is_finite() || !dy.is_finite() {
        return false;
    }
    dx * dx + dy * dy <= ENSEMBLE_RADIUS_PX * ENSEMBLE_RADIUS_PX
}

/// 算出以 `me` 為中心的合奏人數：`buskers` 內（須含 `me` 自己）落在 `ENSEMBLE_RADIUS_PX`
/// 內的獻奏者數量。自己到自己距離 0、必然計入，故同星球單獨獻奏回 1、與另一人靠近回 2……
/// 採「以我為圓心數半徑內的人」這種簡單一致的判定（鏡像 399 數聆賞者的骨架），不做連通分量，
/// 保證決定性、可窮舉測試、零分配以外副作用。
pub fn cluster_size(me: (f32, f32), buskers: &[(f32, f32)]) -> usize {
    buskers
        .iter()
        .filter(|(bx, by)| within_ensemble_range(me.0, me.1, *bx, *by))
        .count()
}

/// 合奏人數是否足以湊成一支樂團（至少 2 人才有和聲）。
pub fn is_ensemble(size: usize) -> bool {
    size >= 2
}

/// 依合奏人數算圍聽者每秒額外回血速率：未成團（< 2 人）回 0；成團後基礎 + 每多一人加成、封頂。
pub fn harmony_regen_per_sec(size: usize) -> f32 {
    if !is_ensemble(size) {
        return 0.0;
    }
    let extra = (size - 2) as f32 * HARMONY_PER_EXTRA_PER_SEC;
    (HARMONY_BASE_PER_SEC + extra).min(HARMONY_MAX_PER_SEC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_includes_center_and_edges() {
        assert!(within_ensemble_range(0.0, 0.0, 0.0, 0.0));
        assert!(within_ensemble_range(0.0, 0.0, ENSEMBLE_RADIUS_PX - 1.0, 0.0));
        assert!(!within_ensemble_range(0.0, 0.0, ENSEMBLE_RADIUS_PX + 1.0, 0.0));
    }

    #[test]
    fn range_rejects_non_finite() {
        assert!(!within_ensemble_range(0.0, 0.0, f32::NAN, 0.0));
        assert!(!within_ensemble_range(0.0, 0.0, 0.0, f32::INFINITY));
    }

    #[test]
    fn solo_busker_counts_self_only() {
        // 只有自己一人在獻奏 → 合奏人數 1（含自己），不成團。
        let buskers = [(100.0, 100.0)];
        assert_eq!(cluster_size((100.0, 100.0), &buskers), 1);
        assert!(!is_ensemble(1));
    }

    #[test]
    fn two_nearby_form_ensemble() {
        let buskers = [(100.0, 100.0), (180.0, 100.0)]; // 相距 80px < 220
        assert_eq!(cluster_size((100.0, 100.0), &buskers), 2);
        assert!(is_ensemble(2));
    }

    #[test]
    fn far_busker_not_in_cluster() {
        let buskers = [(0.0, 0.0), (1000.0, 0.0)]; // 相距 1000px > 220
        assert_eq!(cluster_size((0.0, 0.0), &buskers), 1);
    }

    #[test]
    fn three_in_range_counts_all() {
        let buskers = [(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)];
        assert_eq!(cluster_size((0.0, 0.0), &buskers), 3);
    }

    #[test]
    fn harmony_zero_below_two() {
        assert_eq!(harmony_regen_per_sec(0), 0.0);
        assert_eq!(harmony_regen_per_sec(1), 0.0);
    }

    #[test]
    fn harmony_scales_with_size() {
        assert_eq!(harmony_regen_per_sec(2), HARMONY_BASE_PER_SEC);
        assert_eq!(
            harmony_regen_per_sec(3),
            HARMONY_BASE_PER_SEC + HARMONY_PER_EXTRA_PER_SEC
        );
        assert_eq!(
            harmony_regen_per_sec(4),
            HARMONY_BASE_PER_SEC + 2.0 * HARMONY_PER_EXTRA_PER_SEC
        );
    }

    #[test]
    fn harmony_caps_at_max() {
        // 大團也不超過上限。
        assert_eq!(harmony_regen_per_sec(99), HARMONY_MAX_PER_SEC);
        // 恰好觸頂的人數（1.0 + (n-2)*0.5 = 3.0 → n=6）。
        assert_eq!(harmony_regen_per_sec(6), HARMONY_MAX_PER_SEC);
        assert!(harmony_regen_per_sec(5) <= HARMONY_MAX_PER_SEC);
    }
}
