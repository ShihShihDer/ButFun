//! 安靜打坐（ROADMAP 391）——在黃昏或夜晚的安全地帶靜止不動 30 秒，
//! 緩緩恢復 30% HP ＋ 15 乙太；打坐者身旁漾出柔和光圈，讓附近玩家也看得見「有人在靜心」。
//!
//! 設計鐵律：
//! - **純邏輯可測**：`can_meditate`／`is_calm_phase`／`Meditation::is_interrupted`／
//!   `is_complete`／`progress`／`hp_heal` 皆純函式，無副作用。
//! - **零持久化、零 migration**：`meditation` 與 `last_meditate` 是 Player 上的記憶體前置欄位，
//!   斷線/重啟清空，無需儲存。
//! - **玩家一眼有感**：打坐進度（`meditating: bool`）放進 `PlayerView` 快照廣播，
//!   前端對周圍打坐者畫柔和金色呼吸光圈，旁觀者一眼看得見「有人在靜心」。
//! - **不碰任何遊戲邏輯**：只讀安全區與時段，不改戰鬥 / 採集 / 合成規則。

use std::time::Instant;
use crate::daynight::Phase;

/// 完成打坐所需靜止秒數。
pub const MEDITATE_DURATION_SECS: f32 = 30.0;
/// 打坐完成後的乙太獎勵。
pub const MEDITATE_ETHER: u32 = 15;
/// 打坐完成後的 HP 回復比例（百分比，30 = 回復 max HP 的 30%）。
pub const MEDITATE_HP_PCT: u32 = 30;
/// 每次打坐之間的最短冷卻時間（秒）。
pub const MEDITATE_COOLDOWN_SECS: u64 = 600;
/// 移動超過此距離（像素）視為打坐被打斷，取得比腳步漂移誤差略大的值。
pub const ABORT_MOVE_PX: f32 = 8.0;

/// 判斷是否可開始打坐（冷卻已過）。
pub fn can_meditate(last: Option<Instant>, now: Instant) -> bool {
    match last {
        None => true,
        Some(t) => now.duration_since(t).as_secs() >= MEDITATE_COOLDOWN_SECS,
    }
}

/// 此時段是否適合打坐（黃昏、夜晚、黎明——光線偏暗的「靜心時刻」）。
pub fn is_calm_phase(phase: Phase) -> bool {
    matches!(phase, Phase::Dusk | Phase::Night | Phase::Dawn)
}

/// 計算打坐完成後的 HP 回復量（至少 1）。
pub fn hp_heal(max_hp: u32, pct: u32) -> u32 {
    (max_hp * pct / 100).max(1)
}

/// 進行中的打坐狀態。
#[derive(Debug, Clone, Copy)]
pub struct Meditation {
    /// 打坐開始的時間點。
    pub start: Instant,
    /// 打坐開始時的玩家 X 座標（世界像素）。
    pub start_x: f32,
    /// 打坐開始時的玩家 Y 座標（世界像素）。
    pub start_y: f32,
}

impl Meditation {
    /// 建立一次新的打坐（以當下座標為基準點）。
    pub fn new(now: Instant, x: f32, y: f32) -> Self {
        Self { start: now, start_x: x, start_y: y }
    }

    /// 計算打坐進度 [0.0, 1.0]。
    pub fn progress(&self, now: Instant) -> f32 {
        (now.duration_since(self.start).as_secs_f32() / MEDITATE_DURATION_SECS).clamp(0.0, 1.0)
    }

    /// 玩家是否因移動而被打斷（位移 > `ABORT_MOVE_PX`）。
    pub fn is_interrupted(&self, x: f32, y: f32) -> bool {
        let dx = x - self.start_x;
        let dy = y - self.start_y;
        (dx * dx + dy * dy).sqrt() > ABORT_MOVE_PX
    }

    /// 打坐是否已完成（靜止時間達到要求且未被打斷）。
    pub fn is_complete(&self, now: Instant) -> bool {
        now.duration_since(self.start).as_secs_f32() >= MEDITATE_DURATION_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn inst(secs_ago: u64) -> Instant {
        Instant::now() - Duration::from_secs(secs_ago)
    }

    #[test]
    fn no_cooldown_can_meditate() {
        assert!(can_meditate(None, Instant::now()));
    }

    #[test]
    fn within_cooldown_cannot() {
        assert!(!can_meditate(Some(inst(60)), Instant::now()));
    }

    #[test]
    fn expired_cooldown_can() {
        assert!(can_meditate(Some(inst(MEDITATE_COOLDOWN_SECS + 1)), Instant::now()));
    }

    #[test]
    fn boundary_cooldown_exact() {
        // 恰好等於冷卻時間算通過
        assert!(can_meditate(Some(inst(MEDITATE_COOLDOWN_SECS)), Instant::now()));
    }

    #[test]
    fn calm_phases_correct() {
        assert!(is_calm_phase(Phase::Dusk));
        assert!(is_calm_phase(Phase::Night));
        assert!(is_calm_phase(Phase::Dawn));
        assert!(!is_calm_phase(Phase::Day));
    }

    #[test]
    fn no_move_not_interrupted() {
        let m = Meditation::new(Instant::now(), 100.0, 100.0);
        assert!(!m.is_interrupted(100.0, 100.0));
        assert!(!m.is_interrupted(104.0, 100.0)); // 4px < 8px
    }

    #[test]
    fn big_move_interrupted() {
        let m = Meditation::new(Instant::now(), 100.0, 100.0);
        assert!(m.is_interrupted(200.0, 100.0)); // 100px > 8px
    }

    #[test]
    fn diagonal_move_interrupted() {
        let m = Meditation::new(Instant::now(), 0.0, 0.0);
        // 對角 6px*√2 ≈ 8.49px > 8px
        assert!(m.is_interrupted(6.0, 6.0));
    }

    #[test]
    fn not_complete_fresh() {
        let m = Meditation::new(Instant::now(), 0.0, 0.0);
        assert!(!m.is_complete(Instant::now()));
    }

    #[test]
    fn complete_after_duration() {
        let m = Meditation {
            start: inst(MEDITATE_DURATION_SECS as u64 + 1),
            start_x: 0.0,
            start_y: 0.0,
        };
        assert!(m.is_complete(Instant::now()));
    }

    #[test]
    fn progress_clamps_to_one() {
        let m = Meditation {
            start: inst(MEDITATE_DURATION_SECS as u64 + 10),
            start_x: 0.0,
            start_y: 0.0,
        };
        assert_eq!(m.progress(Instant::now()), 1.0);
    }

    #[test]
    fn hp_heal_calculation() {
        assert_eq!(hp_heal(100, 30), 30);
        assert_eq!(hp_heal(1, 30), 1); // 至少 1
        assert_eq!(hp_heal(200, 30), 60);
        assert_eq!(hp_heal(0, 30), 1); // 0 max_hp 也回 1（防呆）
    }
}
