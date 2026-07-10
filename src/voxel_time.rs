//! 乙太方界世界時間模組——晝夜循環純邏輯（無 IO、無鎖、可測試）。
//! 一遊戲日 = `DAY_DURATION_SECS` 秒真實時間（預設 600 秒 = 10 分鐘）。

/// 一遊戲日的真實時間長度（秒）。
pub const DAY_DURATION_SECS: f32 = 600.0;

/// 世界時鐘，保存已流逝秒數，逢一日自動歸零循環。
#[derive(Debug, Clone)]
pub struct WorldTime {
    elapsed_secs: f32,
    /// 已流逝的完整遊戲日數（季節輪替 v1，ROADMAP 798）：每逢日界（`elapsed_secs` 繞回 0）
    /// 累加一日，供 `voxel_season::season_for_day` 推算當前季節。純記憶體、重啟歸零
    /// （比照天氣／彩虹狀態，世界重啟從初春重新流轉，可接受）。
    days_elapsed: u64,
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
        Self { elapsed_secs: DAY_DURATION_SECS * 0.42, days_elapsed: 0 }
    }

    /// 推進時鐘，`dt` 超過一日則截斷（防異常 dt 跳轉）。
    pub fn tick(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let dt = dt.min(DAY_DURATION_SECS);
        // dt 已截在一日內，最多跨一次日界；`raw >= 一日` 即代表繞過了午夜 → 累加一日
        // （季節輪替 v1，ROADMAP 798）。
        let raw = self.elapsed_secs + dt;
        if raw >= DAY_DURATION_SECS {
            self.days_elapsed = self.days_elapsed.saturating_add(1);
        }
        self.elapsed_secs = raw.rem_euclid(DAY_DURATION_SECS);
    }

    /// 已流逝的完整遊戲日數（季節輪替 v1，ROADMAP 798）：供 `voxel_season` 推算當前季節。
    pub fn days_elapsed(&self) -> u64 {
        self.days_elapsed
    }

    /// 回傳 0.0–1.0 的一日進度（0.0 = 午夜、0.5 = 正午、1.0 ≈ 午夜）。
    pub fn time_of_day(&self) -> f32 {
        self.elapsed_secs / DAY_DURATION_SECS
    }

    /// 睡覺跳過夜晚（床 v1）：直接把世界時鐘撥到隔天黎明起點（`time_of_day = 0.20`），
    /// 讓睡覺有實際效果——玩家不必苦等整段黑夜，一覺醒來就是清晨。
    /// 一覺睡到隔天 → 累加一日（季節輪替 v1，ROADMAP 798）。
    pub fn skip_to_dawn(&mut self) {
        self.elapsed_secs = DAY_DURATION_SECS * 0.20;
        self.days_elapsed = self.days_elapsed.saturating_add(1);
    }

    /// 直接把時鐘撥到指定的一日進度（0.0–1.0，超界夾住）。
    /// **QA 專用**：只被 `voxel_ws` 裡受 `BUTFUN_QA_DEBUG=1` 門禁的 `qa_set_time` 訊息呼叫，
    /// 讓隔離 QA 伺服器能把時間快轉到夜晚驗暗影生物；正式線上無入口。不動 days_elapsed。
    pub fn set_time_of_day(&mut self, t: f32) {
        self.elapsed_secs = DAY_DURATION_SECS * t.clamp(0.0, 0.9999);
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

// ── 日夜作息——純邏輯（居民行為乘數）──────────────────────────────────────────

/// 依時段回傳居民閒晃速度/半徑乘數：深夜緩行縮圈，讓世界夜間安靜下來。純函式、可測。
pub fn wander_mult(phase: TimePhase) -> f32 {
    match phase {
        TimePhase::Night | TimePhase::Evening => 0.4,
        TimePhase::Dawn | TimePhase::Dusk => 0.7,
        TimePhase::Day => 1.0,
    }
}

/// 依時段回傳居民每次換目標後「停在原地」的額外秒數：深夜居民多停久一點，更有安靜感。
/// 純函式、可測。
pub fn rest_wait_extra(phase: TimePhase) -> f32 {
    match phase {
        TimePhase::Night | TimePhase::Evening => 2.5,
        TimePhase::Dawn | TimePhase::Dusk => 0.5,
        TimePhase::Day => 0.0,
    }
}

/// 是否處於「可以睡覺跳過夜晚」的時段（床 v1）：只有深夜/入夜過渡才有意義
/// （白天/黎明/黃昏睡覺沒有效果，比照 Minecraft 只有夜晚才能睡覺跳過的設計）。純函式、可測。
pub fn is_sleepable(phase: TimePhase) -> bool {
    matches!(phase, TimePhase::Night | TimePhase::Evening)
}

/// 時段轉換時的台詞池——夜間（深夜/入夜）。
const NIGHT_PHRASES: &[&str] = &[
    "夜深了，星星好多……",
    "天黑了，小心點喔。",
    "安靜的夜晚……世界真美。",
    "夜裡的風好涼，你也感受到了嗎？",
];

/// 時段轉換時的台詞池——黎明。
const DAWN_PHRASES: &[&str] = &[
    "天亮了！新的一天開始了。",
    "清晨的空氣好清新！",
    "睡了一覺，感覺活力滿滿！",
    "好久不見，早安！",
];

/// 時段切換時挑一句過渡台詞（Night/Evening 或 Dawn 才有台詞，其餘回 None）。
/// `seed` 由呼叫端提供（不走亂數，保確定性、方便測試）。
pub fn transition_phrase(new_phase: TimePhase, seed: u32) -> Option<&'static str> {
    match new_phase {
        TimePhase::Night | TimePhase::Evening => {
            Some(NIGHT_PHRASES[seed as usize % NIGHT_PHRASES.len()])
        }
        TimePhase::Dawn => Some(DAWN_PHRASES[seed as usize % DAWN_PHRASES.len()]),
        _ => None,
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_advances_elapsed() {
        let mut t = WorldTime { elapsed_secs: 0.0, days_elapsed: 0 };
        t.tick(10.0);
        assert!((t.elapsed_secs - 10.0).abs() < 0.001);
    }

    #[test]
    fn tick_wraps_at_day_boundary() {
        let mut t = WorldTime { elapsed_secs: DAY_DURATION_SECS - 1.0, days_elapsed: 0 };
        t.tick(2.0);
        // 應從 599→601，mod 600 = 1
        assert!(t.elapsed_secs < 2.0, "應包圍到 0..2，得 {}", t.elapsed_secs);
    }

    #[test]
    fn days_elapsed_counts_day_boundaries() {
        // 季節輪替 v1（ROADMAP 798）：跨午夜才累加一日，日內推進不加。
        let mut t = WorldTime { elapsed_secs: 0.0, days_elapsed: 0 };
        t.tick(DAY_DURATION_SECS * 0.5); // 半日內，不跨界
        assert_eq!(t.days_elapsed(), 0);
        t.tick(DAY_DURATION_SECS * 0.6); // 越過午夜一次
        assert_eq!(t.days_elapsed(), 1);
        // 再滿滿推一日（dt 截在一日內，剛好再跨一次界）。
        t.tick(DAY_DURATION_SECS);
        assert_eq!(t.days_elapsed(), 2);
        // 睡覺跳黎明 = 睡到隔天，也累加一日。
        t.skip_to_dawn();
        assert_eq!(t.days_elapsed(), 3);
    }

    #[test]
    fn tick_ignores_non_positive_dt() {
        let mut t = WorldTime { elapsed_secs: 100.0, days_elapsed: 0 };
        t.tick(0.0);
        t.tick(-5.0);
        assert!((t.elapsed_secs - 100.0).abs() < 0.001);
    }

    #[test]
    fn time_of_day_range() {
        let mut t = WorldTime { elapsed_secs: 0.0, days_elapsed: 0 };
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
            let t = WorldTime { elapsed_secs: DAY_DURATION_SECS * frac, days_elapsed: 0 };
            assert_eq!(t.phase(), expected, "t_of_day={frac} 應是 {expected:?}");
        }
    }

    #[test]
    fn greeting_non_empty_all_phases() {
        let fracs = [0.10, 0.25, 0.50, 0.75, 0.90];
        for frac in fracs {
            let t = WorldTime { elapsed_secs: DAY_DURATION_SECS * frac, days_elapsed: 0 };
            assert!(!t.greeting().is_empty(), "問候語不應為空（frac={frac}）");
        }
    }

    #[test]
    fn tick_caps_huge_dt() {
        // 超過一日的 dt 不應讓 elapsed 超出 [0, DAY_DURATION)
        let mut t = WorldTime { elapsed_secs: 0.0, days_elapsed: 0 };
        t.tick(DAY_DURATION_SECS * 3.0);
        let tod = t.time_of_day();
        assert!((0.0..1.0).contains(&tod));
    }

    // ── 日夜作息純邏輯測試 ─────────────────────────────────────────────────

    #[test]
    fn wander_mult_range() {
        // 夜間最低、白天最高、黎明/黃昏居中。
        assert!((wander_mult(TimePhase::Night) - 0.4).abs() < 0.001);
        assert!((wander_mult(TimePhase::Evening) - 0.4).abs() < 0.001);
        assert!((wander_mult(TimePhase::Dawn) - 0.7).abs() < 0.001);
        assert!((wander_mult(TimePhase::Dusk) - 0.7).abs() < 0.001);
        assert!((wander_mult(TimePhase::Day) - 1.0).abs() < 0.001);
    }

    #[test]
    fn wander_mult_monotone() {
        // 夜 ≤ 黎明 ≤ 白天。
        assert!(wander_mult(TimePhase::Night) <= wander_mult(TimePhase::Dawn));
        assert!(wander_mult(TimePhase::Dawn) <= wander_mult(TimePhase::Day));
        // 所有乘數在合理範圍（0 < x ≤ 1）。
        for phase in [
            TimePhase::Night, TimePhase::Dawn, TimePhase::Day,
            TimePhase::Dusk, TimePhase::Evening,
        ] {
            let m = wander_mult(phase);
            assert!(m > 0.0 && m <= 1.0, "wander_mult 超出 (0,1]：{m}");
        }
    }

    #[test]
    fn rest_wait_extra_ordering() {
        // 夜間等待最長、白天為零、黎明/黃昏居中。
        assert!(rest_wait_extra(TimePhase::Night) > rest_wait_extra(TimePhase::Day));
        assert!((rest_wait_extra(TimePhase::Day) - 0.0).abs() < 0.001);
        assert!(rest_wait_extra(TimePhase::Dawn) > 0.0);
    }

    #[test]
    fn transition_phrase_coverage() {
        // Night/Evening/Dawn 應有台詞；Day/Dusk 應回 None。
        assert!(transition_phrase(TimePhase::Night, 0).is_some());
        assert!(transition_phrase(TimePhase::Evening, 1).is_some());
        assert!(transition_phrase(TimePhase::Dawn, 2).is_some());
        assert!(transition_phrase(TimePhase::Day, 3).is_none());
        assert!(transition_phrase(TimePhase::Dusk, 4).is_none());
    }

    #[test]
    fn transition_phrase_deterministic() {
        // 同 seed 每次得到同一句。
        assert_eq!(
            transition_phrase(TimePhase::Night, 0),
            transition_phrase(TimePhase::Night, 0)
        );
        // 台詞不為空。
        let p = transition_phrase(TimePhase::Night, 7).unwrap();
        assert!(!p.is_empty());
        let p2 = transition_phrase(TimePhase::Dawn, 7).unwrap();
        assert!(!p2.is_empty());
    }

    #[test]
    fn transition_phrase_pool_exhaustive() {
        // seed 0..N 不 panic，pool 有 4 句確保覆蓋。
        for seed in 0u32..8 {
            let _ = transition_phrase(TimePhase::Night, seed);
            let _ = transition_phrase(TimePhase::Dawn, seed);
        }
    }

    // ── 床 v1：睡覺跳過夜晚純邏輯測試 ───────────────────────────────────────────

    #[test]
    fn is_sleepable_only_at_night() {
        // 只有深夜/入夜過渡才「睡得著」；其餘時段（黎明/白晝/黃昏）睡不著。
        assert!(is_sleepable(TimePhase::Night));
        assert!(is_sleepable(TimePhase::Evening));
        assert!(!is_sleepable(TimePhase::Dawn));
        assert!(!is_sleepable(TimePhase::Day));
        assert!(!is_sleepable(TimePhase::Dusk));
    }

    #[test]
    fn skip_to_dawn_lands_in_dawn_phase() {
        // 睡覺後時鐘應落在 Dawn 時段起點，且不再是可睡覺的夜晚。
        let mut t = WorldTime { elapsed_secs: 5.0, days_elapsed: 0 }; // 深夜
        assert!(is_sleepable(t.phase()));
        t.skip_to_dawn();
        assert_eq!(t.phase(), TimePhase::Dawn);
        assert!(!is_sleepable(t.phase()));
    }

    #[test]
    fn skip_to_dawn_idempotent_from_any_starting_time() {
        // 無論睡前時鐘在哪，睡完都落在同一個黎明起點（確定性、可重現）。
        let mut t1 = WorldTime { elapsed_secs: 1.0, days_elapsed: 0 };
        let mut t2 = WorldTime { elapsed_secs: DAY_DURATION_SECS * 0.95, days_elapsed: 0 };
        t1.skip_to_dawn();
        t2.skip_to_dawn();
        assert_eq!(t1.time_of_day(), t2.time_of_day());
        assert!((t1.time_of_day() - 0.20).abs() < 0.001);
    }

    #[test]
    fn set_time_of_day_clamps_and_lands_exact() {
        // QA 撥鐘：精確落點 + 超界夾住（不會把時鐘撥出一日範圍）。
        let mut t = WorldTime::new();
        t.set_time_of_day(0.05);
        assert_eq!(t.phase(), TimePhase::Night, "0.05 應是深夜");
        t.set_time_of_day(0.5);
        assert_eq!(t.phase(), TimePhase::Day, "0.5 應是白晝");
        t.set_time_of_day(2.0);
        assert!(t.time_of_day() < 1.0, "超界應被夾住");
        t.set_time_of_day(-1.0);
        assert_eq!(t.time_of_day(), 0.0, "負值夾到 0");
        // 撥鐘不影響已流逝日數（季節不跳）。
        assert_eq!(t.days_elapsed(), 0);
    }
}
