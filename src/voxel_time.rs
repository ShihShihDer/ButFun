//! 乙太方界世界時間模組——晝夜循環純邏輯（無 IO、無鎖、可測試）。
//! 一遊戲日 = `DAY_DURATION_SECS` 秒真實時間（預設 600 秒 = 10 分鐘）。

/// 一遊戲日的真實時間長度（秒）。
pub const DAY_DURATION_SECS: f32 = 600.0;

/// 世界時鐘，保存已流逝秒數，逢一日自動歸零循環。
#[derive(Debug, Clone)]
pub struct WorldTime {
    elapsed_secs: f32,
}

/// 一日四個時段，用於居民招呼與前端氛圍。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimePhase {
    /// 0.0–0.20：深夜
    Night,
    /// 0.20–0.35：清晨/黎明
    Dawn,
    /// 0.35–0.70：白晝
    Day,
    /// 0.70–0.85：傍晚/黃昏
    Dusk,
    /// 0.85–1.00：入夜過渡（仍歸 Night 系列，但有別於深夜）
    Evening,
}

impl WorldTime {
    /// 從午夜（`time_of_day = 0.0`）開始。
    pub fn new() -> Self {
        // 預設從上午 10 點（time_of_day ≈ 0.42）開始，讓玩家一進遊戲就是白天。
        Self { elapsed_secs: DAY_DURATION_SECS * 0.42 }
    }

    /// 推進時鐘，`dt` 超過一日則截斷（防異常 dt 跳轉）。
    pub fn tick(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let dt = dt.min(DAY_DURATION_SECS);
        self.elapsed_secs = (self.elapsed_secs + dt).rem_euclid(DAY_DURATION_SECS);
    }

    /// 回傳 0.0–1.0 的一日進度（0.0 = 午夜、0.5 = 正午、1.0 ≈ 午夜）。
    pub fn time_of_day(&self) -> f32 {
        self.elapsed_secs / DAY_DURATION_SECS
    }

    /// 根據時刻判斷所在時段。
    pub fn phase(&self) -> TimePhase {
        let t = self.time_of_day();
        if t < 0.20 {
            TimePhase::Night
        } else if t < 0.35 {
            TimePhase::Dawn
        } else if t < 0.70 {
            TimePhase::Day
        } else if t < 0.85 {
            TimePhase::Dusk
        } else {
            TimePhase::Evening
        }
    }

    /// 依時段回傳居民主動招呼用的時間問候語（台灣用語）。
    pub fn greeting(&self) -> &'static str {
        match self.phase() {
            TimePhase::Night | TimePhase::Evening => "夜深了，你還沒睡？",
            TimePhase::Dawn => "早安！今天的晨光真美。",
            TimePhase::Day => "你好！今天天氣真好呢。",
            TimePhase::Dusk => "傍晚了，今天過得如何？",
        }
    }
}

impl Default for WorldTime {
    fn default() -> Self {
        Self::new()
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_advances_elapsed() {
        let mut t = WorldTime { elapsed_secs: 0.0 };
        t.tick(10.0);
        assert!((t.elapsed_secs - 10.0).abs() < 0.001);
    }

    #[test]
    fn tick_wraps_at_day_boundary() {
        let mut t = WorldTime { elapsed_secs: DAY_DURATION_SECS - 1.0 };
        t.tick(2.0);
        // 應從 599→601，mod 600 = 1
        assert!(t.elapsed_secs < 2.0, "應包圍到 0..2，得 {}", t.elapsed_secs);
    }

    #[test]
    fn tick_ignores_non_positive_dt() {
        let mut t = WorldTime { elapsed_secs: 100.0 };
        t.tick(0.0);
        t.tick(-5.0);
        assert!((t.elapsed_secs - 100.0).abs() < 0.001);
    }

    #[test]
    fn time_of_day_range() {
        let mut t = WorldTime { elapsed_secs: 0.0 };
        for _ in 0..100 {
            let tod = t.time_of_day();
            assert!((0.0..1.0).contains(&tod), "time_of_day 超界：{tod}");
            t.tick(6.0);
        }
    }

    #[test]
    fn phase_classification() {
        let cases = [
            (0.10, TimePhase::Night),
            (0.25, TimePhase::Dawn),
            (0.50, TimePhase::Day),
            (0.75, TimePhase::Dusk),
            (0.90, TimePhase::Evening),
        ];
        for (frac, expected) in cases {
            let t = WorldTime { elapsed_secs: DAY_DURATION_SECS * frac };
            assert_eq!(t.phase(), expected, "t_of_day={frac} 應是 {expected:?}");
        }
    }

    #[test]
    fn greeting_non_empty_all_phases() {
        let fracs = [0.10, 0.25, 0.50, 0.75, 0.90];
        for frac in fracs {
            let t = WorldTime { elapsed_secs: DAY_DURATION_SECS * frac };
            assert!(!t.greeting().is_empty(), "問候語不應為空（frac={frac}）");
        }
    }

    #[test]
    fn tick_caps_huge_dt() {
        // 超過一日的 dt 不應讓 elapsed 超出 [0, DAY_DURATION)
        let mut t = WorldTime { elapsed_secs: 0.0 };
        t.tick(DAY_DURATION_SECS * 3.0);
        let tod = t.time_of_day();
        assert!((0.0..1.0).contains(&tod));
    }
}
