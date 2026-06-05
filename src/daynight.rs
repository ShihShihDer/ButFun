//! 伺服器驅動的日夜循環（Phase 0-G 療癒核心的純邏輯地基）。
//!
//! GDD 第 9 章「要做」明列「伺服器驅動的日夜循環」、0-G 驗收提「隨日夜收成」，
//! 但這塊目前完全沒有模型。這層先把「現在是一天裡的什麼時候、有多亮、是哪個階段」
//! 抽成純資料 + 純函式，無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試。之後接上：
//!   - 遊戲迴圈：每 tick 對共享的 `DayNight` 呼叫 `advance(dt)` 推進時間。
//!   - WebSocket：把 `phase()` / `light_level()` 隨快照廣播，前端依此做環境染色。
//!   - 前端：依亮度疊一層由亮到暗的色調，給「日夜流轉」的療癒體感。
//!   - 作物（選用）：白天成長略快、夜裡放慢——把 0-G 的「隨日夜成長」收尾。
//!   - 持久化（接 0-E）：把 `elapsed` 序列化存回，重啟後從同一個時刻接續。
//!
//! 刻意只做「時間 → 階段 / 亮度」的純映射，先不耦合作物成長（那是接線，留待後續），
//! 一如 `crops.rs` / `field.rs` 當初先落地純邏輯地基、之後才接遊戲迴圈的慣例。
//! 接線時移除本檔的 `allow(dead_code)`。

// 整塊地基目前尚未接上遊戲迴圈 / ws / 前端，所有公開項在非測試建置下都還沒有
// 呼叫端——比照 `crops.rs` / `field.rs` 前置階段的慣例先標起來，接線時移除。
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use std::f32::consts::TAU;

/// 一個完整日夜循環的長度（秒）。10 分鐘一輪——療癒節奏，看得到流轉又不催促。
pub const DAY_LENGTH_SECS: f32 = 600.0;

/// 夜裡的最低亮度。刻意不歸零，讓畫面在最暗時仍看得見（療癒、不是恐怖）。
pub const MIN_LIGHT: f32 = 0.2;

/// 一天裡「最亮的時刻」落在循環的哪個比例（日正當中，約在白天階段中段）。
/// 亮度曲線以此為峰、半圈之外（午夜）為谷。
const PEAK_FRACTION: f32 = 0.325;

/// 日夜的四個階段（依在循環中的比例推導）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 破曉：天色由暗轉亮。
    Dawn,
    /// 白天：最明亮、最適合農作的時段。
    Day,
    /// 黃昏：天色由亮轉暗。
    Dusk,
    /// 夜晚：最暗的時段（仍保有 `MIN_LIGHT` 的微光）。
    Night,
}

/// 依在循環中的比例 `f`（[0,1)）推導目前階段。純函式。
/// 邊界刻意對齊亮度曲線的感受：破曉短、白天長、黃昏短、夜晚長。
pub fn phase_for(f: f32) -> Phase {
    if f < 0.15 {
        Phase::Dawn
    } else if f < 0.5 {
        Phase::Day
    } else if f < 0.65 {
        Phase::Dusk
    } else {
        Phase::Night
    }
}

/// 依在循環中的比例 `f` 推導環境亮度，落在 `[MIN_LIGHT, 1.0]`。純函式。
/// 用餘弦平滑：白天中段（`PEAK_FRACTION`）最亮、半圈外（午夜）最暗，沒有突跳，
/// 給前端做柔和的明暗過場。`f` 非有限時退回最低亮度（不讓壞值算出 NaN 染色）。
pub fn light_for(f: f32) -> f32 {
    if !f.is_finite() {
        return MIN_LIGHT;
    }
    // 0..1 的鐘形：峰在 PEAK_FRACTION（cos(0)=1）、谷在其半圈之外（cos(π)=-1）。
    let raw = 0.5 + 0.5 * (TAU * (f - PEAK_FRACTION)).cos();
    MIN_LIGHT + (1.0 - MIN_LIGHT) * raw
}

/// 伺服器權威的日夜時鐘。只存「這一輪內已經過的秒數」，階段 / 亮度都由它推導，
/// 確保單一真實來源。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DayNight {
    /// 這一輪循環內已經過的秒數，恆落在 `[0, DAY_LENGTH_SECS)`。
    elapsed: f32,
}

impl DayNight {
    /// 從破曉開始一個新循環。
    pub fn new() -> Self {
        Self { elapsed: 0.0 }
    }

    /// 從一個已經過秒數重建時鐘（持久化載入入口，接 0-E）。
    /// 契約：回傳的 `elapsed` 一定有限且落在 `[0, DAY_LENGTH_SECS)`——
    /// 非有限退回 0（破曉）、其餘一律取模繞回（界外 / 負值都安全），
    /// 不讓壞檔 / 被竄改的值算出非有限的階段或亮度。延續
    /// `positions::spawn_at` / `field::from_tiles` / `crops::is_loadable`
    /// 的載入時驗證脈絡。
    pub fn at(elapsed: f32) -> Self {
        let wrapped = if elapsed.is_finite() {
            elapsed.rem_euclid(DAY_LENGTH_SECS)
        } else {
            0.0
        };
        Self { elapsed: wrapped }
    }

    /// 推進 `dt` 秒並繞回。`dt` 非正或非有限時不動作（防壞 tick 把時鐘推成 NaN）。
    pub fn advance(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.elapsed = (self.elapsed + dt).rem_euclid(DAY_LENGTH_SECS);
    }

    /// 目前在循環中的比例，落在 `[0, 1)`。
    pub fn fraction(&self) -> f32 {
        self.elapsed / DAY_LENGTH_SECS
    }

    /// 目前階段。
    pub fn phase(&self) -> Phase {
        phase_for(self.fraction())
    }

    /// 目前環境亮度，`[MIN_LIGHT, 1.0]`。
    pub fn light_level(&self) -> f32 {
        light_for(self.fraction())
    }
}

impl Default for DayNight {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_at_dawn() {
        let d = DayNight::new();
        assert_eq!(d.fraction(), 0.0);
        assert_eq!(d.phase(), Phase::Dawn);
    }

    #[test]
    fn advance_accumulates_time() {
        let mut d = DayNight::new();
        d.advance(DAY_LENGTH_SECS * 0.3); // 進到白天
        assert!((d.fraction() - 0.3).abs() < 1e-6);
        assert_eq!(d.phase(), Phase::Day);
    }

    #[test]
    fn advance_wraps_around_full_cycle() {
        let mut d = DayNight::new();
        d.advance(DAY_LENGTH_SECS + DAY_LENGTH_SECS * 0.1); // 繞一整圈再多 0.1
        assert!((d.fraction() - 0.1).abs() < 1e-6);
        assert!(d.fraction() < 1.0);
    }

    #[test]
    fn advance_ignores_non_positive_and_non_finite_dt() {
        let mut d = DayNight::new();
        d.advance(0.0);
        d.advance(-5.0);
        d.advance(f32::NAN);
        d.advance(f32::INFINITY);
        assert_eq!(d.fraction(), 0.0); // 時鐘沒被壞 dt 推動
    }

    #[test]
    fn fraction_always_in_unit_range() {
        let mut d = DayNight::new();
        // 連續推進很多步，比例必須始終落在 [0,1)。
        for _ in 0..1000 {
            d.advance(7.3);
            let f = d.fraction();
            assert!((0.0..1.0).contains(&f), "fraction 越界：{f}");
        }
    }

    #[test]
    fn phase_thresholds_cover_full_cycle() {
        assert_eq!(phase_for(0.0), Phase::Dawn);
        assert_eq!(phase_for(0.149), Phase::Dawn);
        assert_eq!(phase_for(0.15), Phase::Day);
        assert_eq!(phase_for(0.49), Phase::Day);
        assert_eq!(phase_for(0.5), Phase::Dusk);
        assert_eq!(phase_for(0.64), Phase::Dusk);
        assert_eq!(phase_for(0.65), Phase::Night);
        assert_eq!(phase_for(0.999), Phase::Night);
    }

    #[test]
    fn light_peaks_at_midday_and_dips_at_midnight() {
        let midday = light_for(PEAK_FRACTION);
        let midnight = light_for((PEAK_FRACTION + 0.5).rem_euclid(1.0));
        // 日中最亮（接近 1）、午夜最暗（接近 MIN_LIGHT）。
        assert!((midday - 1.0).abs() < 1e-4, "midday={midday}");
        assert!((midnight - MIN_LIGHT).abs() < 1e-4, "midnight={midnight}");
        assert!(midday > midnight);
    }

    #[test]
    fn light_always_within_bounds() {
        // 掃過一整圈，亮度恆落在 [MIN_LIGHT, 1.0] 且有限。
        for i in 0..1000 {
            let f = i as f32 / 1000.0;
            let l = light_for(f);
            assert!(l.is_finite());
            assert!((MIN_LIGHT - 1e-4..=1.0 + 1e-4).contains(&l), "light={l} f={f}");
        }
    }

    #[test]
    fn light_falls_back_on_non_finite() {
        // 壞值不該算出 NaN 染色，退回最低亮度。
        assert_eq!(light_for(f32::NAN), MIN_LIGHT);
        assert_eq!(light_for(f32::INFINITY), MIN_LIGHT);
    }

    #[test]
    fn at_clamps_and_wraps_loaded_value() {
        // 正常範圍原樣保留。
        assert!((DayNight::at(120.0).fraction() - 120.0 / DAY_LENGTH_SECS).abs() < 1e-6);
        // 界外取模繞回。
        assert!((DayNight::at(DAY_LENGTH_SECS + 120.0).fraction() - 120.0 / DAY_LENGTH_SECS).abs() < 1e-6);
        // 負值（被竄改）安全繞回 [0,1)。
        let f = DayNight::at(-120.0).fraction();
        assert!((0.0..1.0).contains(&f));
        // 非有限退回破曉。
        assert_eq!(DayNight::at(f32::NAN).fraction(), 0.0);
        assert_eq!(DayNight::at(f32::INFINITY).fraction(), 0.0);
    }

    #[test]
    fn serialized_day_night_round_trips() {
        // 持久化格式地基：存到一半的時刻序列化再讀回要一模一樣（接 0-E 跨重啟接續）。
        let mut d = DayNight::new();
        d.advance(DAY_LENGTH_SECS * 0.42);
        let json = serde_json::to_string(&d).unwrap();
        let back: DayNight = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.phase(), d.phase());
    }
}
