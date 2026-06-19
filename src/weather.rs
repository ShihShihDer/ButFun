//! 天氣系統（ROADMAP 93）——生態域天氣粒子特效的後端邏輯。
//!
//! 每 8 分鐘輪換一次天氣，影響指定生態域的視覺效果；
//! 切換時廣播聊天公告，並在對應生態域採集時給 +1 採集量加成。
//! 純邏輯層，不呼叫 LLM，不依賴 IO。

use serde::Serialize;

/// 天氣類型。`Clear` 表示晴天（無粒子特效）；其餘各對應一個生態域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherType {
    /// 晴天——無特效。
    Clear,
    /// 草原細雨——草原 / 森林生態域飄落藍綠色雨滴。
    GrasslandRain,
    /// 沙漠風沙——沙漠生態域捲起棕黃色沙塵。
    DesertSandstorm,
    /// 岩地晶塵——岩地生態域漂浮藍白色晶塵六角片。
    RockyCrystalDust,
    /// 水域海霧——水域生態域浮現青白色泡泡與霧氣。
    WaterSeaMist,
}

impl WeatherType {
    /// 對應的生態域名稱（前端 biome 字串）；`Clear` 無對應返回 `None`。
    pub fn biome(&self) -> Option<&'static str> {
        match self {
            WeatherType::Clear => None,
            WeatherType::GrasslandRain => Some("meadow"), // 草原/森林都算
            WeatherType::DesertSandstorm => Some("sand"),
            WeatherType::RockyCrystalDust => Some("rocky"),
            WeatherType::WaterSeaMist => Some("water"),
        }
    }

    /// 天氣切換時的聊天公告文字。
    pub fn announce_text(&self) -> &'static str {
        match self {
            WeatherType::Clear => "☀️ [世界天氣] 天氣恢復晴朗，微風宜人。",
            WeatherType::GrasslandRain => "🌧️ [世界天氣] 草原細雨降臨！草原/森林採集加成 + ☔ 露天農地自動澆灌中！（持續 8 分鐘）",
            WeatherType::DesertSandstorm => "🌪️ [世界天氣] 沙漠風沙肆虐！在沙漠探索採集有額外收穫！（持續 8 分鐘）",
            WeatherType::RockyCrystalDust => "✨ [世界天氣] 岩地晶塵飄揚！在岩地採集有額外收穫！（持續 8 分鐘）",
            WeatherType::WaterSeaMist => "🌊 [世界天氣] 水域海霧瀰漫！在水域採集有額外收穫！（持續 8 分鐘）",
        }
    }

    /// snake_case 類型字串（與前端 / WeatherView 契約一致）。
    pub fn as_str(&self) -> &'static str {
        match self {
            WeatherType::Clear => "clear",
            WeatherType::GrasslandRain => "grassland_rain",
            WeatherType::DesertSandstorm => "desert_sandstorm",
            WeatherType::RockyCrystalDust => "rocky_crystal_dust",
            WeatherType::WaterSeaMist => "water_sea_mist",
        }
    }

    /// 輪換到下一個天氣類型（Clear→Rain→Sandstorm→Crystal→SeaMist→Clear→…）。
    /// `pub` 供氣象預報台（405）確定性推演接下來的天氣序列。
    pub fn next(self) -> WeatherType {
        match self {
            WeatherType::Clear => WeatherType::GrasslandRain,
            WeatherType::GrasslandRain => WeatherType::DesertSandstorm,
            WeatherType::DesertSandstorm => WeatherType::RockyCrystalDust,
            WeatherType::RockyCrystalDust => WeatherType::WaterSeaMist,
            WeatherType::WaterSeaMist => WeatherType::Clear,
        }
    }
}

/// 每次天氣持續的秒數（8 分鐘）。
pub const WEATHER_DURATION_SECS: f32 = 480.0;

/// 氣象預報台（405）：對外預報接下來幾種天氣（含當前的下一個起算）。
pub const FORECAST_LEN: usize = 3;

/// 強度淡入/淡出的比例（前後各 15% 時間用來漸變強度）。
const FADE_FRACTION: f32 = 0.15;

/// 天氣狀態——伺服器權威的天氣計時器。
#[derive(Debug, Clone)]
pub struct WeatherState {
    pub weather_type: WeatherType,
    /// 目前天氣已持續的秒數，`[0, WEATHER_DURATION_SECS)`。
    elapsed: f32,
    /// 世界風向用的累積時間（ROADMAP 430）。與 `elapsed` 並行、但**不隨天氣切換歸零**，
    /// 故風向能在天氣交替間連續緩轉；`rem_euclid(wind::DIR_PERIOD)` 防浮點長大。
    /// 記憶體前置、零持久化、零 migration（重啟從 0 重新起風）。
    age: f32,
}

impl WeatherState {
    /// 從晴天開始。
    pub fn new() -> Self {
        WeatherState {
            weather_type: WeatherType::Clear,
            elapsed: 0.0,
            age: 0.0,
        }
    }

    /// 推進 `dt` 秒。若天氣切換，返回 `Some(new_type)` 讓呼叫方廣播聊天公告。
    pub fn advance(&mut self, dt: f32) -> Option<WeatherType> {
        if !dt.is_finite() || dt <= 0.0 {
            return None;
        }
        // 風向時間：持續累加並回捲（不隨天氣切換歸零，讓風向連續緩轉）。
        self.age = (self.age + dt).rem_euclid(crate::wind::DIR_PERIOD);
        self.elapsed += dt;
        if self.elapsed >= WEATHER_DURATION_SECS {
            self.elapsed = self.elapsed.rem_euclid(WEATHER_DURATION_SECS);
            self.weather_type = self.weather_type.next();
            return Some(self.weather_type);
        }
        None
    }

    /// 目前的粒子強度，[0.0, 1.0]。晴天固定 0；其餘淡入/淡出。
    pub fn intensity(&self) -> f32 {
        if self.weather_type == WeatherType::Clear {
            return 0.0;
        }
        let f = self.elapsed / WEATHER_DURATION_SECS;
        if f < FADE_FRACTION {
            // 淡入
            f / FADE_FRACTION
        } else if f > 1.0 - FADE_FRACTION {
            // 淡出
            (1.0 - f) / FADE_FRACTION
        } else {
            1.0
        }
    }

    /// 本次天氣還剩多少秒就切換（氣象預報台 405 倒數用）。
    /// 夾在 `[0, WEATHER_DURATION_SECS]`，永不為負（防壞值）。
    pub fn remaining_secs(&self) -> f32 {
        (WEATHER_DURATION_SECS - self.elapsed).clamp(0.0, WEATHER_DURATION_SECS)
    }

    /// 確定性推演接下來的 `n` 種天氣與各自「還有多久才開始」（秒）。
    /// 天氣是固定輪換（見 `WeatherType::next`）＋固定時長，故未來完全可預期。
    /// 回傳 `[(類型, 距現在開始的秒數)]`：第 1 筆＝下一個天氣（eta＝本次剩餘秒），
    /// 第 k 筆 eta＝本次剩餘秒 +（k−1）×時長。eta 單調遞增、必為正。
    pub fn forecast(&self, n: usize) -> Vec<(WeatherType, f32)> {
        let mut out = Vec::with_capacity(n);
        let mut ty = self.weather_type;
        let base = self.remaining_secs();
        for k in 0..n {
            ty = ty.next();
            let eta = base + (k as f32) * WEATHER_DURATION_SECS;
            out.push((ty, eta));
        }
        out
    }

    /// 此天氣帶來的「額外風力」（ROADMAP 430）：晴天 0（只剩保底微風），
    /// 起風天氣按烈度排序（沙暴最強、細雨次之）。純資料對應、無副作用。
    pub fn wind_base(&self) -> f32 {
        match self.weather_type {
            WeatherType::Clear => 0.0,
            WeatherType::GrasslandRain => 0.35,
            WeatherType::DesertSandstorm => 0.8,
            WeatherType::RockyCrystalDust => 0.2,
            WeatherType::WaterSeaMist => 0.3,
        }
    }

    /// 目前的世界風（ROADMAP 430）：由此天氣的額外風力、當下強度（淡入淡出）、
    /// 累積時間 `age`（決定風向）組成。全服共享、伺服器權威。
    pub fn wind(&self) -> crate::wind::Wind {
        crate::wind::wind_for(self.wind_base(), self.intensity(), self.age)
    }

    /// 目前是否正在下雨（草原細雨）——用來決定露天農地是否自動澆灌。
    pub fn is_raining(&self) -> bool {
        self.weather_type == WeatherType::GrasslandRain
    }

    /// 判斷指定的 `biome_name`（前端 biome 字串）是否在本次天氣的加成範圍內。
    /// `GrasslandRain` 額外覆蓋 `forest` 生態域。
    pub fn is_gather_bonus_biome(&self, biome_name: &str) -> bool {
        match self.weather_type.biome() {
            None => false,
            Some("meadow") => biome_name == "meadow" || biome_name == "forest",
            Some(b) => biome_name == b,
        }
    }

    /// 給快照廣播用的可見狀態（返回 `protocol::WeatherView`）。
    pub fn view(&self) -> crate::protocol::WeatherView {
        // 氣象預報台（405）：附帶本次剩餘秒與接下來 3 種天氣的確定性預報。
        let forecast = self
            .forecast(FORECAST_LEN)
            .into_iter()
            .map(|(ty, eta)| crate::protocol::WeatherForecastView {
                weather_type: ty.as_str().to_string(),
                eta_secs: eta,
            })
            .collect();
        crate::protocol::WeatherView {
            weather_type: self.weather_type.as_str().to_string(),
            intensity: self.intensity(),
            remaining_secs: self.remaining_secs(),
            forecast,
            wind: self.wind(), // ROADMAP 430 世界風：隨天氣快照每幀廣播
        }
    }
}

impl Default for WeatherState {
    fn default() -> Self {
        Self::new()
    }
}

// ── 單元測試 ─────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_clear() {
        let w = WeatherState::new();
        assert_eq!(w.weather_type, WeatherType::Clear);
        assert_eq!(w.intensity(), 0.0);
    }

    #[test]
    fn advance_switches_after_duration() {
        let mut w = WeatherState::new();
        // 推到快到期但還沒切
        w.advance(WEATHER_DURATION_SECS - 1.0);
        assert!(w.advance(0.5).is_none()); // 還在 Clear 內
        // 再推過期限
        let next = w.advance(1.0);
        assert_eq!(next, Some(WeatherType::GrasslandRain));
        assert_eq!(w.weather_type, WeatherType::GrasslandRain);
    }

    #[test]
    fn wraps_through_all_types() {
        let mut w = WeatherState::new();
        let expected = [
            WeatherType::GrasslandRain,
            WeatherType::DesertSandstorm,
            WeatherType::RockyCrystalDust,
            WeatherType::WaterSeaMist,
            WeatherType::Clear,
            WeatherType::GrasslandRain, // 再次循環
        ];
        for exp in expected {
            // 推到下一次切換
            let switched = w.advance(WEATHER_DURATION_SECS);
            assert_eq!(switched, Some(exp), "expected {:?}", exp);
        }
    }

    #[test]
    fn gather_bonus_matches_correct_biome() {
        let mut w = WeatherState::new();
        w.advance(WEATHER_DURATION_SECS); // → GrasslandRain
        assert!(w.is_gather_bonus_biome("meadow"));
        assert!(w.is_gather_bonus_biome("forest")); // 森林也受雨水加成
        assert!(!w.is_gather_bonus_biome("sand"));

        w.advance(WEATHER_DURATION_SECS); // → DesertSandstorm
        assert!(w.is_gather_bonus_biome("sand"));
        assert!(!w.is_gather_bonus_biome("meadow"));

        w.advance(WEATHER_DURATION_SECS); // → RockyCrystalDust
        assert!(w.is_gather_bonus_biome("rocky"));
        assert!(!w.is_gather_bonus_biome("sand"));

        w.advance(WEATHER_DURATION_SECS); // → WaterSeaMist
        assert!(w.is_gather_bonus_biome("water"));
        assert!(!w.is_gather_bonus_biome("rocky"));
    }

    #[test]
    fn clear_weather_no_gather_bonus() {
        let w = WeatherState::new();
        assert!(!w.is_gather_bonus_biome("meadow"));
        assert!(!w.is_gather_bonus_biome("sand"));
        assert!(!w.is_gather_bonus_biome("water"));
    }

    #[test]
    fn intensity_zero_during_clear() {
        let mut w = WeatherState::new();
        w.advance(WEATHER_DURATION_SECS * 0.5);
        assert_eq!(w.intensity(), 0.0);
    }

    #[test]
    fn intensity_fades_in_and_out() {
        let mut w = WeatherState::new();
        w.advance(WEATHER_DURATION_SECS); // → GrasslandRain, elapsed reset to ~0
        // 剛切換後，強度應在淡入階段（接近 0）
        let early = w.intensity();
        assert!(early < 0.5, "剛切換強度 {early} 應在淡入中");

        // 推到中段，強度應接近 1.0
        w.advance(WEATHER_DURATION_SECS * 0.5);
        let mid = w.intensity();
        assert!(mid > 0.9, "中段強度 {mid} 應接近 1.0");

        // 推到末段，強度應在淡出
        w.advance(WEATHER_DURATION_SECS * (1.0 - FADE_FRACTION * 0.5) - WEATHER_DURATION_SECS * 0.5);
        let late = w.intensity();
        assert!(late < 1.0, "末段強度 {late} 應在淡出中");
    }

    #[test]
    fn view_reflects_state() {
        let mut w = WeatherState::new();
        let v = w.view();
        assert_eq!(v.weather_type, "clear");
        assert_eq!(v.intensity, 0.0);

        w.advance(WEATHER_DURATION_SECS); // → GrasslandRain
        let v2 = w.view();
        assert_eq!(v2.weather_type, "grassland_rain");
    }

    // ── 氣象預報台（405）─────────────────────────────────────────────────

    #[test]
    fn as_str_round_trips_all_types() {
        // as_str 與 view 的字串契約一致，五型全覆蓋。
        let mut w = WeatherState::new();
        assert_eq!(w.weather_type.as_str(), "clear");
        let expected = [
            "grassland_rain",
            "desert_sandstorm",
            "rocky_crystal_dust",
            "water_sea_mist",
            "clear",
        ];
        for exp in expected {
            w.advance(WEATHER_DURATION_SECS);
            assert_eq!(w.weather_type.as_str(), exp);
        }
    }

    #[test]
    fn remaining_secs_counts_down_within_clamp() {
        let mut w = WeatherState::new();
        // 剛起步：整段時長都還剩。
        assert!((w.remaining_secs() - WEATHER_DURATION_SECS).abs() < 1e-3);
        w.advance(WEATHER_DURATION_SECS * 0.25);
        // 過了 1/4，剩約 3/4。
        assert!((w.remaining_secs() - WEATHER_DURATION_SECS * 0.75).abs() < 1e-2);
        // 永遠夾在 [0, DURATION]，不為負。
        assert!(w.remaining_secs() >= 0.0);
        assert!(w.remaining_secs() <= WEATHER_DURATION_SECS);
    }

    #[test]
    fn forecast_predicts_deterministic_rotation() {
        // 從晴天起算，接下來依序＝雨→沙暴→晶塵。
        let w = WeatherState::new();
        let f = w.forecast(3);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].0, WeatherType::GrasslandRain);
        assert_eq!(f[1].0, WeatherType::DesertSandstorm);
        assert_eq!(f[2].0, WeatherType::RockyCrystalDust);
    }

    #[test]
    fn forecast_eta_is_monotonic_and_positive() {
        let mut w = WeatherState::new();
        w.advance(WEATHER_DURATION_SECS * 0.5); // 本次過半
        let f = w.forecast(3);
        // 第 1 筆 eta＝本次剩餘秒（約半段）。
        assert!((f[0].1 - WEATHER_DURATION_SECS * 0.5).abs() < 1e-2);
        // eta 逐筆遞增、每筆相差一個完整時長、皆為正。
        assert!(f[0].1 > 0.0);
        assert!(f[1].1 > f[0].1);
        assert!(f[2].1 > f[1].1);
        assert!((f[1].1 - f[0].1 - WEATHER_DURATION_SECS).abs() < 1e-2);
        assert!((f[2].1 - f[1].1 - WEATHER_DURATION_SECS).abs() < 1e-2);
    }

    #[test]
    fn forecast_wraps_around_cycle() {
        // 從海霧起算，下一個應繞回晴天。
        let mut w = WeatherState::new();
        for _ in 0..4 {
            w.advance(WEATHER_DURATION_SECS); // Clear→Rain→Sand→Crystal→SeaMist
        }
        assert_eq!(w.weather_type, WeatherType::WaterSeaMist);
        let f = w.forecast(2);
        assert_eq!(f[0].0, WeatherType::Clear);
        assert_eq!(f[1].0, WeatherType::GrasslandRain);
    }

    #[test]
    fn forecast_zero_len_is_empty() {
        let w = WeatherState::new();
        assert!(w.forecast(0).is_empty());
    }

    #[test]
    fn view_carries_forecast_and_remaining() {
        let w = WeatherState::new();
        let v = w.view();
        assert_eq!(v.forecast.len(), FORECAST_LEN);
        assert_eq!(v.forecast[0].weather_type, "grassland_rain");
        assert!(v.remaining_secs > 0.0);
        // 預報 eta 與類型字串契約一致。
        assert!(v.forecast[0].eta_secs > 0.0);
    }

    #[test]
    fn advance_ignores_non_positive_dt() {
        let mut w = WeatherState::new();
        assert!(w.advance(0.0).is_none());
        assert!(w.advance(-5.0).is_none());
        assert!(w.advance(f32::NAN).is_none());
        assert_eq!(w.weather_type, WeatherType::Clear);
    }

    #[test]
    fn is_raining_only_during_grassland_rain() {
        let mut w = WeatherState::new();
        assert!(!w.is_raining(), "晴天不算下雨");
        w.advance(WEATHER_DURATION_SECS); // → GrasslandRain
        assert!(w.is_raining(), "草原細雨時應返回 true");
        w.advance(WEATHER_DURATION_SECS); // → DesertSandstorm
        assert!(!w.is_raining(), "沙漠風沙不算下雨");
    }
}
