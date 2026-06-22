//! 草原細雨庇護（ROADMAP 496）：天氣系統首次影響戰鬥——下草原細雨時，戶外玩家緩緩回血。
//!
//! 在此之前，草原細雨（GrasslandRain）對玩家有三件事：農作物自動澆水、採集有加成、
//! 前端畫粒子特效——卻對**戰鬥與生命值完全沒有影響**，玩家感受不到「雨天在戶外」
//! 有任何不同。本切片補上**天時第一次影響戰場**：細雨中戶外脫戰的玩家緩緩回血，
//! 每 10 秒回 1 HP——就像在雨中感受到世界的照顧。
//!
//! 設計基調：
//! - 溫和，不破壞戰鬥張力：回血速率 0.1 HP/秒（10 秒才累積 1 HP）；
//!   `regen_cooldown > 0`（剛挨打）時一律 no-op，由 `Vitals::rain_regen` 守。
//! - 戶外才享有：室內（`indoor_plot_id.is_some()`）不套用，詩意是「走在雨裡」。
//! - 與林蔭小憩（shade_regen）、合奏療癒（ensemble_regen）正交疊加：站在林蔭下又下雨，
//!   兩個加成都生效——玩家感受到世界細節的豐富。
//! - 零 migration，零 LLM，零 IO：純記憶體前置，重啟清零，不持久化。

/// 草原細雨每秒回血量（HP/秒）。
/// 累積器驅動：0.1 × 10 秒 = 1 HP；讓玩家「慢慢感受到雨的庇護」而非秒感暴力回血。
pub const RAIN_REGEN_PER_SEC: f32 = 0.1;

/// 計算這一 tick 草原細雨應給的每秒回血量。
/// 純函式、確定性；實際「累積 + 整數截斷」由 `Vitals::rain_regen` 處理（與 shade_regen 同模式）。
/// `is_raining`：伺服器當前天氣是否為草原細雨。
/// `outdoor`：玩家是否在戶外（`indoor_plot_id.is_none()`）。
pub fn rain_regen_per_sec(is_raining: bool, outdoor: bool) -> f32 {
    if is_raining && outdoor {
        RAIN_REGEN_PER_SEC
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_regen_when_not_raining() {
        // 天晴——不論戶內外，一律無雨天回血。
        assert_eq!(rain_regen_per_sec(false, true), 0.0);
        assert_eq!(rain_regen_per_sec(false, false), 0.0);
    }

    #[test]
    fn no_regen_when_indoor() {
        // 雨天在室內：沒有「走在雨裡」的詩意，不回血。
        assert_eq!(rain_regen_per_sec(true, false), 0.0);
    }

    #[test]
    fn regen_when_raining_and_outdoor() {
        // 雨天戶外：回血速率非零、大於 0。
        let r = rain_regen_per_sec(true, true);
        assert_eq!(r, RAIN_REGEN_PER_SEC);
        assert!(r > 0.0);
    }

    #[test]
    fn ten_seconds_yields_exactly_one_hp() {
        // 累積 10 秒恰好 +1 HP（不多不少）——確認常數設定正確。
        let total = rain_regen_per_sec(true, true) * 10.0;
        assert!((total - 1.0).abs() < f32::EPSILON * 10.0,
            "10 秒應累積 1 HP，實際 {total}");
    }

    #[test]
    fn regen_is_symmetric_in_all_false_combo() {
        // (false, false) 同樣不回血（防止未來重構出錯）。
        assert_eq!(rain_regen_per_sec(false, false), 0.0);
    }
}
