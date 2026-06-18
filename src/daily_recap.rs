//! 世界冒險日報（ROADMAP 385）。
//!
//! 每個遊戲黎明（日夜週期從非 Dawn 轉入 Dawn 時），收集一日精彩並廣播到世界頻道：
//! 最長連殺玩家、最稀有採集、升等人次、解鎖秘文次數。
//!
//! 純邏輯、零 IO、零持久化、零 migration、零 LLM（純模板文字）。

use crate::daynight::Phase;

/// 今日至今的精彩紀錄（純記憶體，每個遊戲黎明重置）。
pub struct DailyHighlights {
    /// 今日最高連殺紀錄：(玩家名, 連殺數)。
    top_streak: Option<(String, u8)>,
    /// 今日最稀有採集：(玩家名, 品質分數, 品質顯示名, emoji, 物品名)。
    top_gather: Option<(String, u32, &'static str, &'static str, String)>,
    /// 今日升等總次數（上限 u8::MAX，不失控）。
    level_ups: u8,
    /// 今日解鎖古代秘文總次數（首次解碼）。
    inscriptions: u8,
    /// 上一個 tick 的日夜階段，用於偵測非 Dawn → Dawn 轉換。
    last_phase: Phase,
}

impl DailyHighlights {
    pub fn new() -> Self {
        Self {
            top_streak: None,
            top_gather: None,
            level_ups: 0,
            inscriptions: 0,
            last_phase: Phase::Night,
        }
    }

    /// 記錄一次連殺里程碑；保留今日最高連殺數的玩家。
    pub fn update_streak(&mut self, player_name: &str, streak: u8) {
        let current_best = self.top_streak.as_ref().map_or(0, |(_, s)| *s);
        if streak > current_best {
            self.top_streak = Some((player_name.to_string(), streak));
        }
    }

    /// 記錄一次稀有採集事件；保留今日最稀有的那筆（以 qty_bonus 分數比較）。
    pub fn update_gather(
        &mut self,
        player_name: &str,
        rarity_score: u32,
        rarity_display: &'static str,
        rarity_emoji: &'static str,
        item_name: &str,
    ) {
        let current_best = self.top_gather.as_ref().map_or(0, |(_, s, _, _, _)| *s);
        if rarity_score > current_best {
            self.top_gather = Some((
                player_name.to_string(),
                rarity_score,
                rarity_display,
                rarity_emoji,
                item_name.to_string(),
            ));
        }
    }

    /// 記錄一次升等事件（累計，飽和於 u8::MAX）。
    pub fn on_level_up(&mut self) {
        self.level_ups = self.level_ups.saturating_add(1);
    }

    /// 記錄一次古代秘文首次解鎖（累計，飽和於 u8::MAX）。
    pub fn on_inscription(&mut self) {
        self.inscriptions = self.inscriptions.saturating_add(1);
    }

    /// 推進日夜時鐘，偵測黎明轉換。
    ///
    /// 當從非 Dawn 相位切入 Dawn 時，若今日有任何精彩紀錄，
    /// 回傳日報行（Vec<String>）並重置統計；否則回傳 None。
    /// `dt` 僅供未來擴充（目前僅做相位比較），不為 0 不為負即可。
    pub fn tick(&mut self, _dt: f32, current_phase: Phase) -> Option<Vec<String>> {
        let transition = self.last_phase != Phase::Dawn && current_phase == Phase::Dawn;
        self.last_phase = current_phase;

        if !transition {
            return None;
        }

        // 黎明到來：組日報，無論如何都重置今日統計。
        let lines = self.compose_recap();
        self.reset_daily();
        if lines.is_empty() { None } else { Some(lines) }
    }

    /// 組合日報文字行（純函式邏輯，供測試）。
    fn compose_recap(&self) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();

        if let Some((name, streak)) = &self.top_streak {
            lines.push(format!("⚔️ 今日連殺之星：{} 達成 {} 連殺，戰意沸騰！", name, streak));
        }

        if let Some((name, _, rarity_display, rarity_emoji, item)) = &self.top_gather {
            lines.push(format!(
                "{} 今日奇遇：{} 採集到了{}品質的{}！",
                rarity_emoji, name, rarity_display, item
            ));
        }

        if self.level_ups > 0 {
            lines.push(format!("🌟 今日共有 {} 位冒險者升等，世界在成長！", self.level_ups));
        }

        if self.inscriptions > 0 {
            lines.push(format!(
                "📜 今日共解讀 {} 篇古代秘文，遺跡的秘密又少了一分！",
                self.inscriptions
            ));
        }

        if lines.is_empty() {
            return lines;
        }

        // 插入標題行（放最前面）。
        lines.insert(0, "📰 【今日冒險回顧】黎明到來，昨日精彩留印記——".to_string());
        lines
    }

    /// 重置今日統計（黎明後呼叫）。
    fn reset_daily(&mut self) {
        self.top_streak = None;
        self.top_gather = None;
        self.level_ups = 0;
        self.inscriptions = 0;
    }
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dawn() -> Phase { Phase::Dawn }
    fn day() -> Phase { Phase::Day }
    fn night() -> Phase { Phase::Night }

    #[test]
    fn new_state_has_no_highlights() {
        let h = DailyHighlights::new();
        assert!(h.top_streak.is_none());
        assert!(h.top_gather.is_none());
        assert_eq!(h.level_ups, 0);
        assert_eq!(h.inscriptions, 0);
    }

    #[test]
    fn update_streak_keeps_highest() {
        let mut h = DailyHighlights::new();
        h.update_streak("Alice", 4);
        h.update_streak("Bob", 8);
        h.update_streak("Charlie", 2);
        let (name, streak) = h.top_streak.unwrap();
        assert_eq!(name, "Bob");
        assert_eq!(streak, 8);
    }

    #[test]
    fn update_streak_does_not_downgrade() {
        let mut h = DailyHighlights::new();
        h.update_streak("Alice", 8);
        h.update_streak("Bob", 4);
        let (name, _) = h.top_streak.unwrap();
        assert_eq!(name, "Alice");
    }

    #[test]
    fn update_gather_keeps_highest_rarity() {
        let mut h = DailyHighlights::new();
        h.update_gather("Alice", 1, "不凡", "✨", "木材");
        h.update_gather("Bob", 4, "史詩", "🌟", "乙太礦");
        h.update_gather("Charlie", 2, "稀有", "💎", "晶石");
        let (name, score, _, _, _) = h.top_gather.unwrap();
        assert_eq!(name, "Bob");
        assert_eq!(score, 4);
    }

    #[test]
    fn update_gather_same_score_keeps_first() {
        let mut h = DailyHighlights::new();
        h.update_gather("Alice", 2, "稀有", "💎", "礦石");
        h.update_gather("Bob", 2, "稀有", "💎", "木材");
        let (name, _, _, _, _) = h.top_gather.unwrap();
        assert_eq!(name, "Alice");
    }

    #[test]
    fn on_level_up_increments() {
        let mut h = DailyHighlights::new();
        h.on_level_up();
        h.on_level_up();
        assert_eq!(h.level_ups, 2);
    }

    #[test]
    fn on_level_up_saturates_at_u8_max() {
        let mut h = DailyHighlights::new();
        for _ in 0..300 {
            h.on_level_up();
        }
        assert_eq!(h.level_ups, u8::MAX);
    }

    #[test]
    fn on_inscription_increments() {
        let mut h = DailyHighlights::new();
        h.on_inscription();
        h.on_inscription();
        assert_eq!(h.inscriptions, 2);
    }

    #[test]
    fn tick_no_transition_returns_none() {
        let mut h = DailyHighlights::new();
        h.update_streak("Alice", 4);
        // 初始 last_phase = Night，現在仍是 Night → 非轉換
        assert!(h.tick(0.1, night()).is_none());
        // Day → Day 也非轉換
        h.last_phase = day();
        assert!(h.tick(0.1, day()).is_none());
    }

    #[test]
    fn tick_dawn_transition_returns_recap_and_resets() {
        let mut h = DailyHighlights::new();
        h.last_phase = night();
        h.update_streak("Alice", 6);
        h.on_level_up();
        let lines = h.tick(0.1, dawn());
        assert!(lines.is_some());
        let lines = lines.unwrap();
        assert!(!lines.is_empty());
        // 重置後再觸發應為 None（沒新事件）
        h.last_phase = night();
        assert!(h.tick(0.1, dawn()).is_none());
    }

    #[test]
    fn tick_dawn_with_no_events_returns_none() {
        let mut h = DailyHighlights::new();
        h.last_phase = night();
        // 完全沒有任何事件
        assert!(h.tick(0.1, dawn()).is_none());
    }

    #[test]
    fn recap_includes_header_when_events_exist() {
        let mut h = DailyHighlights::new();
        h.update_streak("Alice", 4);
        let lines = h.compose_recap();
        assert!(lines[0].contains("今日冒險回顧"));
    }

    #[test]
    fn recap_covers_all_four_categories() {
        let mut h = DailyHighlights::new();
        h.update_streak("Alice", 4);
        h.update_gather("Bob", 4, "史詩", "🌟", "乙太礦");
        h.on_level_up();
        h.on_inscription();
        let lines = h.compose_recap();
        // 1 標題 + 4 統計 = 5 行
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn recap_is_empty_when_no_events() {
        let h = DailyHighlights::new();
        assert!(h.compose_recap().is_empty());
    }
}
