// ─── ROADMAP 302：後端月相（moon phase, 給「滿月嗥月」用的權威天象）──────────────
// 239 月有陰晴圓缺／240 月明星稀讓那輪月真的會依「真實世界朔望月」盈虧——但那兩筆至今
// 全活在前端（web/game.js 的 moonPhase(Date.now())）：純視覺，後端從不知道今晚月相。
// 302 滿月嗥月要讓「掠食者在滿月夜對月特別愛嚎」，是會影響生態系行為的決策，需要一份
// 後端也算得出、且與前端對得上的權威月相。本模組把前端那條月相公式原樣搬到 Rust：
// 同一組常數（朔望月長度、參考新月時刻）、同一道天文受光比例公式 (1-cos(2π·相位))/2，
// 讓後端 SystemTime::now() 與前端 Date.now() 在同一時刻算出一致的月相。
// 純函式、無 I/O、無全域狀態、可測；game.rs 每幀於 wildlife tick 前讀系統時間餵 set_moon_full。

/// 朔望月平均長度（天）——與 web/game.js 的 SYNODIC_MONTH_DAYS 同值。
const SYNODIC_MONTH_DAYS: f64 = 29.530588853;
/// 一天的毫秒數。
const DAY_MS: f64 = 86_400_000.0;
/// 已知的一個參考新月時刻（Unix epoch 毫秒）——與 web/game.js 的 KNOWN_NEW_MOON_MS 同值，
/// 確保前後端算出的月齡同步。
const KNOWN_NEW_MOON_MS: f64 = 947_182_440_000.0;

/// 「視為滿月」的受光比例門檻——illum∈[0,1]（新月 0、滿月 1）。0.96 對應滿月前後約 ±1.9 天
/// 的窗口（朔望月 29.53 天裡約 3.8 天），讓「滿月夜」是一段自然的數夜、而非僅僅圓得最足的那一刻。
/// 前端畫「對月嚎」的 🌕 也用同一門檻（web/game.js MOON_FULL_ILLUM），符號與行為一致。
pub const MOON_FULL_ILLUM: f64 = 0.96;

/// 朔望月齡（自參考新月起算、對朔望月取模的天數）∈ [0, SYNODIC_MONTH_DAYS)。
/// 與 web/game.js 的 lunarAgeDays 同算法（含參考時刻之前的負模歸正）。
fn lunar_age_days(now_ms: f64) -> f64 {
    let days = (now_ms - KNOWN_NEW_MOON_MS) / DAY_MS;
    let mut age = days % SYNODIC_MONTH_DAYS;
    if age < 0.0 {
        age += SYNODIC_MONTH_DAYS; // 參考時刻之前的負模也歸正
    }
    age
}

/// 把月齡映成受光（明亮）面比例 illum∈[0,1]：新月 0、上/下弦 0.5、滿月 1。
/// 採天文標準 (1-cos(2π·相位))/2，與 web/game.js 的 moonPhase().illum 同公式。
pub fn illumination(now_ms: f64) -> f64 {
    let cyc = lunar_age_days(now_ms) / SYNODIC_MONTH_DAYS; // 周期相位 [0,1)
    let illum = (1.0 - (2.0 * std::f64::consts::PI * cyc).cos()) / 2.0;
    illum.clamp(0.0, 1.0)
}

/// 此刻是否為「滿月」（受光比例 ≥ MOON_FULL_ILLUM）。供 302 滿月嗥月判定。
pub fn is_full_moon(now_ms: f64) -> bool {
    illumination(now_ms) >= MOON_FULL_ILLUM
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 參考新月時刻本身 illum 應≈0（新月最暗），且不算滿月。
    #[test]
    fn new_moon_epoch_is_dark() {
        let illum = illumination(KNOWN_NEW_MOON_MS);
        assert!(illum < 0.01, "新月時刻 illum 應≈0，實際 {illum}");
        assert!(!is_full_moon(KNOWN_NEW_MOON_MS), "新月不該被判為滿月");
    }

    /// 參考新月後半個朔望月＝滿月：illum 應≈1，且判為滿月。
    #[test]
    fn half_cycle_after_new_moon_is_full() {
        let full_ms = KNOWN_NEW_MOON_MS + SYNODIC_MONTH_DAYS / 2.0 * DAY_MS;
        let illum = illumination(full_ms);
        assert!(illum > 0.99, "滿月時刻 illum 應≈1，實際 {illum}");
        assert!(is_full_moon(full_ms), "半個朔望月後應判為滿月");
    }

    /// 上弦（四分之一周期）illum 應≈0.5，且不算滿月。
    #[test]
    fn quarter_cycle_is_half_lit() {
        let quarter_ms = KNOWN_NEW_MOON_MS + SYNODIC_MONTH_DAYS / 4.0 * DAY_MS;
        let illum = illumination(quarter_ms);
        assert!((illum - 0.5).abs() < 0.01, "上弦 illum 應≈0.5，實際 {illum}");
        assert!(!is_full_moon(quarter_ms), "上弦不該被判為滿月");
    }

    /// illum 永遠落在 [0,1]，且滿月窗口前後一天仍屬滿月、再遠一點就不是（窗口邊界合理）。
    #[test]
    fn full_window_spans_a_few_nights() {
        let full_ms = KNOWN_NEW_MOON_MS + SYNODIC_MONTH_DAYS / 2.0 * DAY_MS;
        // 滿月前後 1 天仍在窗口內
        assert!(is_full_moon(full_ms + 1.0 * DAY_MS), "滿月後 1 天應仍是滿月");
        assert!(is_full_moon(full_ms - 1.0 * DAY_MS), "滿月前 1 天應仍是滿月");
        // 滿月前後 4 天已出窗口
        assert!(!is_full_moon(full_ms + 4.0 * DAY_MS), "滿月後 4 天不該是滿月");
        assert!(!is_full_moon(full_ms - 4.0 * DAY_MS), "滿月前 4 天不該是滿月");
        // illum 始終在合法區間
        for k in 0..60 {
            let ms = KNOWN_NEW_MOON_MS + k as f64 * DAY_MS;
            let i = illumination(ms);
            assert!((0.0..=1.0).contains(&i), "illum 越界：{i}");
        }
    }

    /// 月相有周期性：相隔整數個朔望月的同一相位 illum 應幾乎相等。
    #[test]
    fn phase_is_periodic() {
        let base = KNOWN_NEW_MOON_MS + 3.0 * DAY_MS;
        let one_month_later = base + SYNODIC_MONTH_DAYS * DAY_MS;
        let a = illumination(base);
        let b = illumination(one_month_later);
        assert!((a - b).abs() < 1e-6, "相隔一個朔望月 illum 應相同：{a} vs {b}");
    }
}
