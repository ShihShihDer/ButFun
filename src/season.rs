//! 季節循環（ROADMAP 137）。
//!
//! 世界有春夏秋冬四個季節，各持續 SEASON_DURATION_SECS 秒（預設 20 分鐘）。
//! 季節影響作物成長速度（春快冬慢），讓時間在遊戲裡有了質感與意義。
//!
//! 設計原則：
//! - 純邏輯、無 IO，易於單元測試。
//! - 記憶體模式，重啟從春天開始（世界換季，行為合理）。
//! - 與現有 `daynight.growth_rate()` 疊乘——日夜週期控制晝夜節奏，
//!   季節控制長期趨勢，兩者獨立正交、互不侵犯。
//! - 零 migration、不動玩家資料、純前端 HUD pill 顯示。

/// 每個季節持續時間（秒）。四季 = 80 分鐘一個完整循環。
pub const SEASON_DURATION_SECS: f32 = 1200.0;

/// 四個季節。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    /// 前端 / 廣播用的繁中名稱（含 emoji）。
    pub fn display_name(self) -> &'static str {
        match self {
            Season::Spring => "🌸 春",
            Season::Summer => "☀️ 夏",
            Season::Autumn => "🍂 秋",
            Season::Winter => "❄️ 冬",
        }
    }

    /// 季節切換時的世界廣播公告文字。
    pub fn announce_text(self) -> &'static str {
        match self {
            Season::Spring => "🌸 【季節】春天降臨！百花盛開，作物生長加速（×1.25），大地復甦！",
            Season::Summer => "☀️ 【季節】夏日炎炎！萬物繁盛，作物正常生長，是採集的好時節！",
            Season::Autumn => "🍂 【季節】秋風蕭索！葉落知秋，作物生長放緩（×0.8），準備豐收儲糧！",
            Season::Winter => "❄️ 【季節】寒冬降臨！大地沉寂，作物幾乎停滯（×0.5），注意保暖！",
        }
    }

    /// 作物成長速度倍率（疊乘在 `daynight.growth_rate()` 之上）。
    pub fn growth_rate_modifier(self) -> f32 {
        match self {
            Season::Spring => 1.25,
            Season::Summer => 1.0,
            Season::Autumn => 0.8,
            Season::Winter => 0.5,
        }
    }

    /// 季節按春→夏→秋→冬→春循環。
    fn next(self) -> Season {
        match self {
            Season::Spring => Season::Summer,
            Season::Summer => Season::Autumn,
            Season::Autumn => Season::Winter,
            Season::Winter => Season::Spring,
        }
    }

    /// 序列化給前端的 snake_case 字串。
    pub fn as_str(self) -> &'static str {
        match self {
            Season::Spring => "spring",
            Season::Summer => "summer",
            Season::Autumn => "autumn",
            Season::Winter => "winter",
        }
    }
}

/// 季節狀態（伺服器權威）。
#[derive(Debug, Clone)]
pub struct SeasonState {
    pub current: Season,
    /// 目前季節已持續的秒數，[0, SEASON_DURATION_SECS)。
    elapsed_secs: f32,
}

impl SeasonState {
    /// 從春天開始（重啟等同世界換季）。
    pub fn new() -> Self {
        SeasonState {
            current: Season::Spring,
            elapsed_secs: 0.0,
        }
    }

    /// 作物成長速度倍率（目前季節）。
    pub fn growth_rate_modifier(&self) -> f32 {
        self.current.growth_rate_modifier()
    }

    /// 每幀推進 `dt` 秒；若季節切換，回傳 `Some(new_season)`，否則 `None`。
    /// caller 收到 `Some` 時廣播切換公告。
    pub fn tick(&mut self, dt: f32) -> Option<Season> {
        self.elapsed_secs += dt;
        if self.elapsed_secs >= SEASON_DURATION_SECS {
            self.elapsed_secs -= SEASON_DURATION_SECS;
            self.current = self.current.next();
            Some(self.current)
        } else {
            None
        }
    }

    /// 目前季節剩餘秒數（供前端顯示倒計時）。
    pub fn remaining_secs(&self) -> u32 {
        (SEASON_DURATION_SECS - self.elapsed_secs).ceil() as u32
    }
}

impl Default for SeasonState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── 純邏輯單元測試 ────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_spring() {
        let s = SeasonState::new();
        assert_eq!(s.current, Season::Spring);
    }

    #[test]
    fn growth_modifier_spring_highest() {
        let s = SeasonState::new();
        let spring_mod = s.growth_rate_modifier();
        let summer_mod = Season::Summer.growth_rate_modifier();
        let autumn_mod = Season::Autumn.growth_rate_modifier();
        let winter_mod = Season::Winter.growth_rate_modifier();
        assert!(spring_mod > summer_mod, "春天應比夏天成長快");
        assert!(summer_mod > autumn_mod, "夏天應比秋天成長快");
        assert!(autumn_mod > winter_mod, "秋天應比冬天成長快");
    }

    #[test]
    fn tick_no_change_before_duration() {
        let mut s = SeasonState::new();
        let result = s.tick(SEASON_DURATION_SECS - 1.0);
        assert!(result.is_none(), "未到期不應切換季節");
        assert_eq!(s.current, Season::Spring);
    }

    #[test]
    fn tick_changes_season_at_boundary() {
        let mut s = SeasonState::new();
        let result = s.tick(SEASON_DURATION_SECS + 0.1);
        assert!(result.is_some(), "超過 SEASON_DURATION_SECS 應切換");
        assert_eq!(result.unwrap(), Season::Summer);
        assert_eq!(s.current, Season::Summer);
    }

    #[test]
    fn full_cycle_returns_to_spring() {
        let mut s = SeasonState::new();
        s.tick(SEASON_DURATION_SECS + 0.1); // → Summer
        s.tick(SEASON_DURATION_SECS + 0.1); // → Autumn
        s.tick(SEASON_DURATION_SECS + 0.1); // → Winter
        let back = s.tick(SEASON_DURATION_SECS + 0.1); // → Spring
        assert_eq!(back.unwrap(), Season::Spring, "四個季節後應回到春天");
    }

    #[test]
    fn remaining_secs_decreases() {
        let mut s = SeasonState::new();
        let before = s.remaining_secs();
        s.tick(10.0);
        let after = s.remaining_secs();
        assert!(after < before, "推進後剩餘時間應縮短");
    }

    #[test]
    fn remaining_secs_resets_after_season_change() {
        let mut s = SeasonState::new();
        s.tick(SEASON_DURATION_SECS + 0.1);
        let rem = s.remaining_secs();
        assert!(rem > 0 && rem <= SEASON_DURATION_SECS as u32, "切換後剩餘時間應重設");
    }

    #[test]
    fn display_name_nonempty() {
        for season in [Season::Spring, Season::Summer, Season::Autumn, Season::Winter] {
            assert!(!season.display_name().is_empty());
        }
    }

    #[test]
    fn as_str_unique() {
        let strs = [Season::Spring.as_str(), Season::Summer.as_str(),
                    Season::Autumn.as_str(), Season::Winter.as_str()];
        for i in 0..strs.len() {
            for j in (i+1)..strs.len() {
                assert_ne!(strs[i], strs[j], "各季節的 as_str 應唯一");
            }
        }
    }
}
