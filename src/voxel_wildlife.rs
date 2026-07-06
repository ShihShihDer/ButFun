//! 乙太方界·野兔 v1——世界第一種環境生物（自主提案切片，ROADMAP 847）。
//!
//! **真缺口**：乙太方界至今只有 4 位具名 AI 居民會在世界裡走動，草地與森林從未有過
//! 任何一絲野生的動態——世界看得出「有人住」，卻看不出「有生機」。本模組補上世界
//! 第一種環境生物：幾隻在村莊周圍悠閒遊蕩的野兔，見到玩家靠近就受驚跳開。
//!
//! **刻意的範圍收斂**：純點綴、無 AI 大腦（零 LLM）、無戰鬥、無記憶、無持久化
//! （純記憶體、重啟於固定家域點重新生成，比照既有 `drops`/`stalls` 世界暫態慣例）。
//! 這不是居民↔居民/居民↔玩家關係軸線的第 N 刀，是全新的「世界環境」軸線第一刀。
//!
//! **純邏輯層**：家域遊蕩沿用既有 [`crate::voxel_residents`] 的
//! `wander_target`/`wander_center`/`step_toward`/`gravity_step`/`dry_ground_spawn`，
//! 本模組只補野兔專屬的「受驚偵測」與「逃跑方向」兩個確定性純函式，
//! 零 LLM、零鎖、零 IO，鎖/連線/tick 驅動全在 `voxel_ws.rs`。

/// 野兔閒晃速度（方塊/秒）——比居民散步（2.6）更悠閒，符合小動物碎步的觀感。
pub const WANDER_SPEED: f32 = 1.4;
/// 受驚逃跑速度——明顯比閒晃快，一眼看得出「嚇到了」。
pub const FLEE_SPEED: f32 = 4.2;
/// 玩家進入這個距離內，野兔就會受驚逃跑（方塊）。
pub const FLEE_RADIUS: f32 = 4.0;
/// 已受驚時，玩家要遠到超過這個距離才安心恢復閒晃（比 [`FLEE_RADIUS`] 稍大，
/// 這道遲滯（hysteresis）避免野兔在臨界距離上受驚/平靜來回抖動）。
pub const CALM_RADIUS: f32 = 6.0;
/// 逃跑目標離當下位置的距離（方塊）。
pub const FLEE_DIST: f32 = 6.0;
/// 野兔閒晃半徑下限（方塊）——比居民 `HOME_RADIUS`（20）小得多，野兔活動範圍更侷限。
pub const WANDER_MIN_R: f32 = 1.5;
/// 野兔閒晃半徑上限（方塊）。
pub const WANDER_MAX_R: f32 = 6.0;

/// 依「目前是否已受驚」+「與最近玩家的距離平方」，判斷這一 tick 該不該受驚（或維持受驚）。
///
/// 用遲滯避免抖動：平靜時要近到 [`FLEE_RADIUS`] 內才受驚；已受驚時要遠到
/// [`CALM_RADIUS`] 外才平靜下來——兩段式門檻讓「快靠近/快遠離」的邊界不反覆橫跳。
pub fn should_flee(currently_fleeing: bool, nearest_player_dist_sq: f32) -> bool {
    let threshold = if currently_fleeing { CALM_RADIUS } else { FLEE_RADIUS };
    nearest_player_dist_sq < threshold * threshold
}

/// 由兔子座標與（最近）玩家座標算出「逃離玩家」的目標點（純幾何、無隨機性、可測）。
///
/// 玩家與兔子恰好同座標（距離為 0，退化情況）時預設往 +x 方向逃，避免除以零。
pub fn flee_target(rx: f32, rz: f32, px: f32, pz: f32) -> (f32, f32) {
    let dx = rx - px;
    let dz = rz - pz;
    let dist = (dx * dx + dz * dz).sqrt();
    if dist < 1e-4 {
        (rx + FLEE_DIST, rz)
    } else {
        (rx + dx / dist * FLEE_DIST, rz + dz / dist * FLEE_DIST)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_flee_triggers_within_flee_radius_when_calm() {
        assert!(should_flee(false, 3.9 * 3.9));
        assert!(!should_flee(false, 4.1 * 4.1));
    }

    #[test]
    fn should_flee_hysteresis_keeps_fleeing_until_calm_radius() {
        // 已受驚：距離落在 flee~calm 之間仍算受驚（遲滯，不提早平靜）。
        assert!(should_flee(true, 5.0 * 5.0));
        // 遠到超過 calm 半徑才真正平靜。
        assert!(!should_flee(true, 6.1 * 6.1));
    }

    #[test]
    fn should_flee_exact_boundary_is_exclusive() {
        // 距離恰好等於門檻不算「進入」（< 而非 <=），邊界一致不誤觸。
        assert!(!should_flee(false, FLEE_RADIUS * FLEE_RADIUS));
        assert!(!should_flee(true, CALM_RADIUS * CALM_RADIUS));
    }

    #[test]
    fn flee_target_points_directly_away_from_player() {
        let (tx, tz) = flee_target(0.0, 0.0, 1.0, 0.0);
        assert!(tx < 0.0, "玩家在 +x，兔子該往 -x 逃，得到 tx={tx}");
        assert!(tz.abs() < 1e-4);
    }

    #[test]
    fn flee_target_scales_to_flee_dist() {
        let (tx, tz) = flee_target(0.0, 0.0, 0.0, 3.0);
        let dist = (tx * tx + tz * tz).sqrt();
        assert!((dist - FLEE_DIST).abs() < 1e-3);
    }

    #[test]
    fn flee_target_handles_zero_distance_without_panic() {
        let (tx, tz) = flee_target(5.0, 5.0, 5.0, 5.0);
        assert!((tx - (5.0 + FLEE_DIST)).abs() < 1e-4);
        assert!((tz - 5.0).abs() < 1e-4);
    }

    #[test]
    fn flee_target_diagonal_direction() {
        let (tx, tz) = flee_target(0.0, 0.0, 1.0, 1.0);
        assert!(tx < 0.0 && tz < 0.0, "玩家在右上方，兔子該往左下逃");
        // 對角逃跑距離仍應是 FLEE_DIST（正規化過的方向向量）。
        let dist = (tx * tx + tz * tz).sqrt();
        assert!((dist - FLEE_DIST).abs() < 1e-3);
    }
}
