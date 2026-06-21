//! 田鴉啄食與稻草人守望（ROADMAP 476）的純邏輯。
//!
//! 農作迴圈至今教的是「用心照顧 → 好品質」（406 渴秒數）、「換種旺長」（454）、「漚肥滋養」（473）——
//! 全是**長得多好**的正向疊加，卻沒有一個「**收成時機**」的決策：作物熟了之後，放著不收沒有任何代價，
//! 玩家攢一批種子全長熟也不急著回來收。本切片補上農作迴圈第一個「該回來收了」的張力：
//! 作物**熟成後久置無人看守**會招來田鴉啄食，收成品質掉一階；田裡立一座**稻草人**即可在其
//! 守護半徑內把田鴉嚇退。於是玩家要嘛及時回來收、要嘛立稻草人守住要緊的那一片——
//! 這是農作第一個關於「**空間 × 時機**」的取捨（稻草人守不了整片大田，得選守哪一塊）。
//!
//! 刻意與既有農作切片分工乾淨：406/454/473 是「種得多用心 → 長得多好」的成長期正向加成；
//! 本切片是「**成熟後的時機與防護**」——生命週期階段（成熟後）、骨架（會掉一階的代價＋可佈防的對策）
//! 都不同，且是農作第一個**負向風險**（仍極溫和：只掉一階、永不毀作物、立個稻草人就全免）。
//!
//! 全純函式、確定性（純看「熟了多久 × 有沒有被守護」、零亂數）、零 LLM、零 IO——
//! 伺服器結算（`crops`/`field`）與測試共用同一套門檻。

use crate::crops::CropQuality;

/// 作物成熟後、無稻草人看守時，放置多久（成長秒數，與 `crops::RIPE_AT` 同單位）就會招來田鴉啄食。
/// 取一整個成熟期的量級（≈`RIPE_AT`）——要真的久置不收才會被啄，及時回來收的玩家完全不受影響，
/// 維持「溫和提醒、不懲罰」的療癒基調。
pub const RAID_EXPOSURE_SECS: f32 = 90.0;

/// 稻草人守護半徑（格，切比雪夫距離）：以稻草人所在格為中心、此距離內的格子（含對角）免遭田鴉啄食。
/// 取 1（3×3 共 9 格）——一座稻草人守不住整片田（初始 6×4=24 格，擴張後更大），
/// 玩家得選「守哪一片」或「離得遠的及時收」，造就空間取捨。
pub const SCARECROW_GUARD_RADIUS: usize = 1;

/// 某格 `cell`（col,row）是否落在稻草人 `scarecrow`（col,row）的守護範圍內
///（切比雪夫距離 ≤ `radius`）。`None`＝田裡沒立稻草人＝一律不守護。純函式、確定性。
pub fn guarded(scarecrow: Option<(usize, usize)>, cell: (usize, usize), radius: usize) -> bool {
    match scarecrow {
        None => false,
        Some((sc, sr)) => sc.abs_diff(cell.0) <= radius && sr.abs_diff(cell.1) <= radius,
    }
}

/// 一株成熟作物此刻是否該被田鴉啄食：熟成後曝露夠久（`ripe_secs ≥ RAID_EXPOSURE_SECS`）、
/// 未受稻草人守護、且尚未被啄過（啄食一次定案、不重複扣）。壞值（NaN/負）保守當作不啄，不冤枉玩家。
/// 純函式、確定性。
pub fn should_peck(ripe_secs: f32, guarded: bool, already_pecked: bool) -> bool {
    ripe_secs.is_finite() && ripe_secs >= RAID_EXPOSURE_SECS && !guarded && !already_pecked
}

/// 被田鴉啄食後的收成品質：降一階（優質→用心→平凡；平凡見底、不會更糟）。純函式。
/// 故意只掉一階＝溫和代價：被啄過的優質作物仍是「用心」級，不是直接歸零。
pub fn pecked_quality(q: CropQuality) -> CropQuality {
    match q {
        CropQuality::Premium => CropQuality::Fine,
        CropQuality::Fine => CropQuality::Plain,
        CropQuality::Plain => CropQuality::Plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_scarecrow_never_guards() {
        // 田裡沒立稻草人：任何格都不受守護。
        assert!(!guarded(None, (0, 0), SCARECROW_GUARD_RADIUS));
        assert!(!guarded(None, (3, 2), 5));
    }

    #[test]
    fn guard_covers_chebyshev_radius_including_diagonals() {
        let sc = Some((3, 3));
        // 中心格與四方/對角鄰格（切比雪夫距離 1）皆受守護。
        assert!(guarded(sc, (3, 3), 1));
        assert!(guarded(sc, (2, 3), 1));
        assert!(guarded(sc, (4, 4), 1)); // 對角也算（切比雪夫）
        assert!(guarded(sc, (2, 2), 1));
        // 距離 2（出半徑）不守護。
        assert!(!guarded(sc, (1, 3), 1));
        assert!(!guarded(sc, (5, 5), 1));
    }

    #[test]
    fn should_peck_only_when_exposed_unguarded_and_fresh() {
        // 曝露夠久 + 沒守護 + 還沒被啄 → 啄。
        assert!(should_peck(RAID_EXPOSURE_SECS, false, false));
        assert!(should_peck(RAID_EXPOSURE_SECS + 50.0, false, false));
        // 還沒曝露夠久 → 不啄（剛熟、給足收成窗口）。
        assert!(!should_peck(RAID_EXPOSURE_SECS - 0.1, false, false));
        assert!(!should_peck(0.0, false, false));
        // 被稻草人守護 → 不啄（即使久置）。
        assert!(!should_peck(RAID_EXPOSURE_SECS * 5.0, true, false));
        // 已經被啄過 → 不再重複扣（啄食一次定案、冪等）。
        assert!(!should_peck(RAID_EXPOSURE_SECS * 5.0, false, true));
    }

    #[test]
    fn should_peck_ignores_bad_ripe_secs() {
        // 壞值保守當作不啄（不冤枉玩家）。
        assert!(!should_peck(f32::NAN, false, false));
        assert!(!should_peck(f32::INFINITY, false, false));
        assert!(!should_peck(-5.0, false, false));
    }

    #[test]
    fn pecked_quality_drops_one_tier_with_floor() {
        assert_eq!(pecked_quality(CropQuality::Premium), CropQuality::Fine);
        assert_eq!(pecked_quality(CropQuality::Fine), CropQuality::Plain);
        // 平凡見底——被啄也不會更糟（沒有比平凡更低的品質）。
        assert_eq!(pecked_quality(CropQuality::Plain), CropQuality::Plain);
    }
}
