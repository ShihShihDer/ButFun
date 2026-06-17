//! 雨後彩虹（ROADMAP 361）——天氣這條維度的第一個「天象 → 全服共享療癒光環」玩法。
//!
//! 草原細雨停下、天氣切換的那一瞬間（且天還亮著、有日光），天空架起一道彩虹，
//! 持續 `RAINBOW_DURATION_SECS`。彩虹高掛期間，全服**存活的玩家**每隔 `HEAL_PULSE_SECS`
//! 同享一次溫和的「🌈 彩虹祝福」緩回血——這是一道全服共享、伺服器權威的療癒天象。
//!
//! 設計取捨（刻意換骨架，不複製既有套路）：
//! - 流星雨（133）／夜間乙太泉（162）／季節節點（154）都是「走近採集節點」骨架；
//!   彩虹**不是節點**，而是「天象在頭頂、祝福灑全場」的被動共享療癒光環，療癒向、零採集動作。
//! - 前端早有一道**純裝飾、各客戶端各自偵測「雨→停」**的彩虹（ROADMAP 191）；本切片把它
//!   第一次接成**伺服器權威的全服同步天象**：所有人同時看見同一道彩虹，且彩虹在＝祝福在。
//!
//! 成本／安全紀律：
//! - 純記憶體模式，重啟清零。零 migration、零 LLM、零持久化、不碰玩家存檔。
//! - 純邏輯可獨立測試（出現時機、倒數、回血脈衝節律、隱去事件），不依賴 WebSocket／遊戲迴圈。
//! - 療癒分寸：每次脈衝只回 `HEAL_AMOUNT` 點且封頂於玩家上限，不影響任何戰鬥／經濟平衡。

/// 彩虹高掛總時長（秒）——架起後持續 60 秒，期間祝福生效。
pub const RAINBOW_DURATION_SECS: f32 = 60.0;
/// 回血脈衝間隔（秒）——每隔 6 秒對全服存活玩家發一次溫和回血。
pub const HEAL_PULSE_SECS: f32 = 6.0;
/// 單次脈衝回復的生命點數（療癒分寸，小量、封頂於上限）。
pub const HEAL_AMOUNT: u32 = 3;

/// 彩虹出現時的世界聊天公告（面向玩家字串集中於此，便於 i18n 替換）。
pub const APPEAR_TEXT: &str =
    "🌈 雨過天青——天邊架起一道彩虹！戶外的旅人都沐浴在彩虹的祝福裡，緩緩回復生機。";
/// 彩虹隱去時的世界聊天公告。
pub const VANISH_TEXT: &str = "🌈 彩虹漸漸隱入天色，祝福也隨之消散。";

/// 一幀推進後回報的兩件事：本幀是否該發一次全服回血脈衝、彩虹是否剛隱去（供廣播）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RainbowTick {
    /// 本幀是否觸發一次全服回血脈衝。
    pub heal_pulse: bool,
    /// 本幀彩虹是否剛好走到盡頭、隱去（供廣播一則告別公告）。
    pub vanished: bool,
}

/// 伺服器權威的彩虹狀態——一道天象的倒數與回血脈衝節律。
#[derive(Debug, Clone)]
pub struct RainbowState {
    /// 彩虹剩餘秒數；`0` 表示目前沒有彩虹。
    remaining: f32,
    /// 回血脈衝的累積秒數；滿 `HEAL_PULSE_SECS` 就發一次脈衝並扣回。
    heal_accum: f32,
}

impl RainbowState {
    /// 初始無彩虹。
    pub fn new() -> Self {
        RainbowState {
            remaining: 0.0,
            heal_accum: 0.0,
        }
    }

    /// 彩虹出現（雨過天青那一刻呼叫）。重置倒數與脈衝累積；若彩虹正巧仍在，等同延長刷新。
    pub fn appear(&mut self) {
        self.remaining = RAINBOW_DURATION_SECS;
        self.heal_accum = 0.0;
    }

    /// 此刻是否有彩虹高掛。
    pub fn is_active(&self) -> bool {
        self.remaining > 0.0
    }

    /// 剩餘秒數（無條件進位；前端 HUD pill 倒數與淡出判斷用）。
    pub fn remaining_secs(&self) -> u32 {
        self.remaining.max(0.0).ceil() as u32
    }

    /// 推進倒數一幀，回報本幀是否該發回血脈衝、彩虹是否剛隱去。
    /// 無彩虹或 `dt` 非正時直接早退（不發脈衝、不報隱去），多數幀零成本。
    pub fn tick(&mut self, dt: f32) -> RainbowTick {
        if !dt.is_finite() || dt <= 0.0 || self.remaining <= 0.0 {
            return RainbowTick {
                heal_pulse: false,
                vanished: false,
            };
        }
        self.remaining -= dt;
        if self.remaining <= 0.0 {
            // 走到盡頭：歸零、清脈衝累積、回報「剛隱去」（這一幀不再發脈衝）。
            self.remaining = 0.0;
            self.heal_accum = 0.0;
            return RainbowTick {
                heal_pulse: false,
                vanished: true,
            };
        }
        // 仍高掛：累積脈衝時間，滿一個間隔就發一次（扣回間隔、不吃掉超出的零頭）。
        self.heal_accum += dt;
        let heal_pulse = self.heal_accum >= HEAL_PULSE_SECS;
        if heal_pulse {
            self.heal_accum -= HEAL_PULSE_SECS;
        }
        RainbowTick {
            heal_pulse,
            vanished: false,
        }
    }

    /// 給快照廣播用的可見狀態（返回 `protocol::RainbowView`）。
    pub fn view(&self) -> crate::protocol::RainbowView {
        crate::protocol::RainbowView {
            active: self.is_active(),
            remaining_secs: self.remaining_secs(),
        }
    }
}

impl Default for RainbowState {
    fn default() -> Self {
        Self::new()
    }
}

// ── 單元測試 ─────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_without_rainbow() {
        let r = RainbowState::new();
        assert!(!r.is_active());
        assert_eq!(r.remaining_secs(), 0);
    }

    #[test]
    fn appear_lights_rainbow_for_full_duration() {
        let mut r = RainbowState::new();
        r.appear();
        assert!(r.is_active());
        assert_eq!(r.remaining_secs(), RAINBOW_DURATION_SECS.ceil() as u32);
    }

    #[test]
    fn tick_without_rainbow_is_noop() {
        let mut r = RainbowState::new();
        let t = r.tick(1.0);
        assert!(!t.heal_pulse);
        assert!(!t.vanished);
    }

    #[test]
    fn tick_ignores_non_positive_or_nonfinite_dt() {
        let mut r = RainbowState::new();
        r.appear();
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let t = r.tick(dt);
            assert!(!t.heal_pulse);
            assert!(!t.vanished);
        }
        // 非法 dt 不該推進倒數。
        assert_eq!(r.remaining_secs(), RAINBOW_DURATION_SECS.ceil() as u32);
    }

    #[test]
    fn heal_pulse_fires_on_interval_boundary() {
        let mut r = RainbowState::new();
        r.appear();
        // 累積到剛好一個間隔前不發、跨過才發。
        let almost = HEAL_PULSE_SECS - 0.5;
        let t1 = r.tick(almost);
        assert!(!t1.heal_pulse, "未滿一個間隔不該發脈衝");
        let t2 = r.tick(1.0);
        assert!(t2.heal_pulse, "跨過間隔該發一次脈衝");
    }

    #[test]
    fn heal_pulse_fires_repeatedly_across_lifetime() {
        let mut r = RainbowState::new();
        r.appear();
        // 整段壽命內以小步推進，應發出約 DURATION/PULSE 次脈衝。
        let mut pulses = 0;
        let mut vanished_count = 0;
        let step = 0.5;
        let steps = (RAINBOW_DURATION_SECS / step) as u32 + 4;
        for _ in 0..steps {
            let t = r.tick(step);
            if t.heal_pulse {
                pulses += 1;
            }
            if t.vanished {
                vanished_count += 1;
            }
        }
        let expected = (RAINBOW_DURATION_SECS / HEAL_PULSE_SECS).floor() as i32;
        assert!(
            (pulses - expected).abs() <= 1,
            "脈衝數應約為 {expected}，實得 {pulses}"
        );
        assert_eq!(vanished_count, 1, "整段壽命只該報一次隱去");
        assert!(!r.is_active(), "壽命耗盡後不再活躍");
    }

    #[test]
    fn vanish_reported_exactly_once_then_quiet() {
        let mut r = RainbowState::new();
        r.appear();
        // 一口氣推進超過總時長 → 該幀回報隱去。
        let t = r.tick(RAINBOW_DURATION_SECS + 1.0);
        assert!(t.vanished);
        assert!(!t.heal_pulse);
        assert!(!r.is_active());
        // 之後再推進不再回報隱去、不再發脈衝。
        let t2 = r.tick(1.0);
        assert!(!t2.vanished);
        assert!(!t2.heal_pulse);
    }

    #[test]
    fn appear_refreshes_active_rainbow() {
        let mut r = RainbowState::new();
        r.appear();
        r.tick(RAINBOW_DURATION_SECS - 5.0);
        assert!(r.is_active());
        // 雨又停一次 → 重新刷滿倒數。
        r.appear();
        assert_eq!(r.remaining_secs(), RAINBOW_DURATION_SECS.ceil() as u32);
    }
}
