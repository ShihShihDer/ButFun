//! 廣場獻奏·街頭樂手（ROADMAP 399）——在安全的村落廣場站定、靜止獻奏約 15 秒，
//! 完成後得到「打賞乙太」（隨身旁聆賞的鄰近玩家人數遞增）並累積一次演奏資歷；
//! 獻奏期間頭頂飄出音符，讓附近玩家一眼看見「有人在街頭獻奏」，廣場因此熱鬧起來。
//!
//! 設計鐵律（鏡像 391 安靜打坐的接線骨架，但刻意換維度）：
//! - **純邏輯可測**：`can_busk`／`Busking::is_interrupted`／`is_complete`／`progress`／
//!   `tip_ether`／`note_symbol` 皆純函式，無副作用。
//! - **零持久化、零 migration**：`busking`／`last_busk`／`busk_count` 是 Player 上的記憶體前置欄位，
//!   斷線/重啟清空，無需儲存。
//! - **玩家一眼有感**：獻奏進度（`busking: bool`）放進 `PlayerView` 快照廣播，
//!   前端對周圍獻奏者頭頂畫飄動音符，旁觀者一眼看得見「有人在街頭獻奏」。
//! - **社交正向（與 391 打坐刻意分流）**：打坐是獨自靜心回血、限黃昏夜晚；獻奏是廣場社交、
//!   不限時段，打賞隨「聆賞的鄰近玩家」人數遞增——人越多、打賞越多，鼓勵聚眾同樂（正和、療癒）。
//! - **不碰任何遊戲邏輯**：只讀安全區與位置，不改戰鬥 / 採集 / 合成規則；零 LLM。

use std::time::Instant;

/// 完成一場獻奏所需靜止獻奏的秒數（比打坐 30 秒短，一首小曲）。
pub const BUSK_DURATION_SECS: f32 = 15.0;
/// 每次獻奏之間的最短冷卻時間（秒）。
pub const BUSK_COOLDOWN_SECS: u64 = 300;
/// 獻奏完成的基礎打賞乙太（即使無人聆賞，獻奏本身亦有微薄回饋）。
pub const BUSK_BASE_TIP: u32 = 8;
/// 每位聆賞的鄰近玩家額外增加的打賞乙太。
pub const BUSK_PER_LISTENER: u32 = 3;
/// 計入打賞的聆賞玩家人數上限（防刷：再多人也只算到這麼多）。
pub const BUSK_MAX_LISTENERS: u32 = 5;
/// 移動超過此距離（像素）視為獻奏被打斷，與打坐取同一容差（略大於腳步漂移誤差）。
pub const ABORT_MOVE_PX: f32 = 8.0;
/// 判定「聆賞玩家」的半徑（像素）——站在這個範圍內的其他在線玩家算作聽眾。
pub const LISTEN_RADIUS_PX: f32 = 160.0;

/// 判斷是否可開始獻奏（冷卻已過）。
pub fn can_busk(last: Option<Instant>, now: Instant) -> bool {
    match last {
        None => true,
        Some(t) => now.duration_since(t).as_secs() >= BUSK_COOLDOWN_SECS,
    }
}

/// 依聆賞的鄰近玩家人數計算打賞乙太：基礎 + 每位聽眾加成（上限封頂）。
pub fn tip_ether(listeners: u32) -> u32 {
    BUSK_BASE_TIP + BUSK_PER_LISTENER * listeners.min(BUSK_MAX_LISTENERS)
}

/// 依種子挑一個音符符號（前端飄字用，集中於此便於 i18n／一致性與測試）。
pub fn note_symbol(seed: u64) -> &'static str {
    const NOTES: [&str; 3] = ["🎵", "🎶", "🎼"];
    NOTES[(seed % NOTES.len() as u64) as usize]
}

/// 進行中的獻奏狀態。
#[derive(Debug, Clone, Copy)]
pub struct Busking {
    /// 獻奏開始的時間點。
    pub start: Instant,
    /// 獻奏開始時的玩家 X 座標（世界像素）。
    pub start_x: f32,
    /// 獻奏開始時的玩家 Y 座標（世界像素）。
    pub start_y: f32,
}

impl Busking {
    /// 建立一場新的獻奏（以當下座標為基準點）。
    pub fn new(now: Instant, x: f32, y: f32) -> Self {
        Self { start: now, start_x: x, start_y: y }
    }

    /// 計算獻奏進度 [0.0, 1.0]。
    pub fn progress(&self, now: Instant) -> f32 {
        (now.duration_since(self.start).as_secs_f32() / BUSK_DURATION_SECS).clamp(0.0, 1.0)
    }

    /// 玩家是否因移動而被打斷（位移 > `ABORT_MOVE_PX`）。
    pub fn is_interrupted(&self, x: f32, y: f32) -> bool {
        let dx = x - self.start_x;
        let dy = y - self.start_y;
        (dx * dx + dy * dy).sqrt() > ABORT_MOVE_PX
    }

    /// 獻奏是否已完成（靜止獻奏時間達到要求且未被打斷）。
    pub fn is_complete(&self, now: Instant) -> bool {
        now.duration_since(self.start).as_secs_f32() >= BUSK_DURATION_SECS
    }
}

/// 判斷某位其他玩家是否在獻奏者的聆賞半徑內（純距離比較；非有限座標保守回 false）。
pub fn within_listen_range(busker_x: f32, busker_y: f32, other_x: f32, other_y: f32) -> bool {
    let dx = other_x - busker_x;
    let dy = other_y - busker_y;
    if !dx.is_finite() || !dy.is_finite() {
        return false;
    }
    dx * dx + dy * dy <= LISTEN_RADIUS_PX * LISTEN_RADIUS_PX
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn inst(secs_ago: u64) -> Instant {
        Instant::now() - Duration::from_secs(secs_ago)
    }

    #[test]
    fn no_cooldown_can_busk() {
        assert!(can_busk(None, Instant::now()));
    }

    #[test]
    fn within_cooldown_cannot() {
        assert!(!can_busk(Some(inst(60)), Instant::now()));
    }

    #[test]
    fn expired_cooldown_can() {
        assert!(can_busk(Some(inst(BUSK_COOLDOWN_SECS + 1)), Instant::now()));
    }

    #[test]
    fn boundary_cooldown_exact() {
        // 恰好等於冷卻時間算通過
        assert!(can_busk(Some(inst(BUSK_COOLDOWN_SECS)), Instant::now()));
    }

    #[test]
    fn tip_no_listeners_is_base() {
        assert_eq!(tip_ether(0), BUSK_BASE_TIP);
    }

    #[test]
    fn tip_scales_with_listeners() {
        assert_eq!(tip_ether(1), BUSK_BASE_TIP + BUSK_PER_LISTENER);
        assert_eq!(tip_ether(3), BUSK_BASE_TIP + BUSK_PER_LISTENER * 3);
    }

    #[test]
    fn tip_caps_at_max_listeners() {
        let capped = BUSK_BASE_TIP + BUSK_PER_LISTENER * BUSK_MAX_LISTENERS;
        assert_eq!(tip_ether(BUSK_MAX_LISTENERS), capped);
        assert_eq!(tip_ether(BUSK_MAX_LISTENERS + 1), capped);
        assert_eq!(tip_ether(99), capped);
    }

    #[test]
    fn note_symbol_cycles_and_in_range() {
        assert_eq!(note_symbol(0), "🎵");
        assert_eq!(note_symbol(1), "🎶");
        assert_eq!(note_symbol(2), "🎼");
        assert_eq!(note_symbol(3), "🎵"); // 循環
    }

    #[test]
    fn no_move_not_interrupted() {
        let b = Busking::new(Instant::now(), 100.0, 100.0);
        assert!(!b.is_interrupted(100.0, 100.0));
        assert!(!b.is_interrupted(104.0, 100.0)); // 4px < 8px
    }

    #[test]
    fn big_move_interrupted() {
        let b = Busking::new(Instant::now(), 100.0, 100.0);
        assert!(b.is_interrupted(200.0, 100.0)); // 100px > 8px
    }

    #[test]
    fn diagonal_move_interrupted() {
        let b = Busking::new(Instant::now(), 0.0, 0.0);
        // 對角 6px*√2 ≈ 8.49px > 8px
        assert!(b.is_interrupted(6.0, 6.0));
    }

    #[test]
    fn not_complete_fresh() {
        let b = Busking::new(Instant::now(), 0.0, 0.0);
        assert!(!b.is_complete(Instant::now()));
    }

    #[test]
    fn complete_after_duration() {
        let b = Busking {
            start: inst(BUSK_DURATION_SECS as u64 + 1),
            start_x: 0.0,
            start_y: 0.0,
        };
        assert!(b.is_complete(Instant::now()));
    }

    #[test]
    fn progress_clamps_and_advances() {
        let fresh = Busking::new(Instant::now(), 0.0, 0.0);
        assert!(fresh.progress(Instant::now()) < 0.2);
        let old = Busking { start: inst(BUSK_DURATION_SECS as u64 + 5), start_x: 0.0, start_y: 0.0 };
        assert_eq!(old.progress(Instant::now()), 1.0); // 封頂
    }

    #[test]
    fn listen_range_radius() {
        // 正中央算聽眾
        assert!(within_listen_range(0.0, 0.0, 0.0, 0.0));
        // 剛好在半徑內
        assert!(within_listen_range(0.0, 0.0, LISTEN_RADIUS_PX - 1.0, 0.0));
        // 半徑外不算
        assert!(!within_listen_range(0.0, 0.0, LISTEN_RADIUS_PX + 1.0, 0.0));
    }

    #[test]
    fn listen_range_rejects_non_finite() {
        assert!(!within_listen_range(0.0, 0.0, f32::NAN, 0.0));
        assert!(!within_listen_range(0.0, 0.0, f32::INFINITY, 0.0));
    }
}
