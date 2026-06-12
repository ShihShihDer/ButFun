//! 城鎮記憶石（ROADMAP 157）——記錄世界大事供玩家讀取，讓城鎮有自己的歷史。
//!
//! 設計：
//! - 引擎各系統在重要事件發生時呼叫 `push_event()`。
//! - 最多保留 MAX_ENTRIES 條（先進先出環形緩衝）。
//! - 玩家靠近記憶石 → 前端送 ReadTownMemory → 後端回傳 TownMemoryList。
//! - 零 migration、零 LLM、純記憶體模式。

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

/// 最多保留幾條事件。
pub const MAX_ENTRIES: usize = 30;

/// 記憶石的世界座標（接在 village_chief 右方、NPC 排右端）。
pub const MEMORY_STONE_X: f32 = 2840.0;
pub const MEMORY_STONE_Y: f32 = 2080.0;

/// 互動範圍（像素），比 SHOP_REACH 略大，讓靠近就能讀。
pub const INTERACT_REACH: f32 = 100.0;

/// 判斷玩家是否在記憶石互動範圍內。
pub fn is_near_stone(px: f32, py: f32) -> bool {
    let dx = px - MEMORY_STONE_X;
    let dy = py - MEMORY_STONE_Y;
    dx * dx + dy * dy <= INTERACT_REACH * INTERACT_REACH
}

/// 一則世界大事記錄。
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryEntry {
    /// 事件圖示（emoji）。
    pub icon: String,
    /// 事件描述文字（繁中）。
    pub text: String,
    /// UNIX 時間戳（秒），用於前端顯示「N 分鐘前」。
    pub ts_secs: u64,
}

/// 城鎮記憶石狀態——環形緩衝，重啟清空（純記憶體）。
#[derive(Debug, Default)]
pub struct TownMemory {
    entries: VecDeque<MemoryEntry>,
}

impl TownMemory {
    pub fn new() -> Self {
        Self { entries: VecDeque::new() }
    }

    /// 記錄一則世界大事（引擎寫入，玩家文字永遠進不來）。
    /// 超出 MAX_ENTRIES 時自動丟棄最舊的一條。
    pub fn push_event(&mut self, icon: impl Into<String>, text: impl Into<String>) {
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.pop_front();
        }
        let ts_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.entries.push_back(MemoryEntry {
            icon: icon.into(),
            text: text.into(),
            ts_secs,
        });
    }

    /// 取得最近所有記錄（由舊到新）。
    pub fn recent(&self) -> &VecDeque<MemoryEntry> {
        &self.entries
    }

    /// 取得最新的 N 筆（由新到舊），供前端顯示。
    pub fn recent_desc(&self, limit: usize) -> Vec<&MemoryEntry> {
        self.entries.iter().rev().take(limit).collect()
    }
}

// ─── 測試 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_recent_basic() {
        let mut m = TownMemory::new();
        m.push_event("⚔️", "守城勝利");
        m.push_event("📜", "提案通過");
        assert_eq!(m.recent().len(), 2);
        assert_eq!(m.recent()[0].icon, "⚔️");
        assert_eq!(m.recent()[1].text, "提案通過");
    }

    #[test]
    fn max_entries_cap_keeps_newest() {
        let mut m = TownMemory::new();
        for i in 0..MAX_ENTRIES + 5 {
            m.push_event("🌸", format!("事件 {}", i));
        }
        assert_eq!(m.recent().len(), MAX_ENTRIES);
        // 最舊的已被丟棄，最新的仍在末尾
        assert_eq!(m.recent().back().unwrap().text, format!("事件 {}", MAX_ENTRIES + 4));
    }

    #[test]
    fn recent_desc_order_newest_first() {
        let mut m = TownMemory::new();
        m.push_event("1️⃣", "第一");
        m.push_event("2️⃣", "第二");
        m.push_event("3️⃣", "第三");
        let desc = m.recent_desc(3);
        assert_eq!(desc[0].icon, "3️⃣");
        assert_eq!(desc[1].icon, "2️⃣");
        assert_eq!(desc[2].icon, "1️⃣");
    }

    #[test]
    fn recent_desc_limit_respected() {
        let mut m = TownMemory::new();
        for i in 0..10 {
            m.push_event("🔮", format!("e{}", i));
        }
        assert_eq!(m.recent_desc(5).len(), 5);
    }

    #[test]
    fn is_near_stone_within_reach() {
        assert!(is_near_stone(MEMORY_STONE_X + 50.0, MEMORY_STONE_Y));
        assert!(is_near_stone(MEMORY_STONE_X, MEMORY_STONE_Y + 99.0));
    }

    #[test]
    fn is_near_stone_outside_reach() {
        assert!(!is_near_stone(MEMORY_STONE_X + 200.0, MEMORY_STONE_Y));
        assert!(!is_near_stone(0.0, 0.0));
    }

    #[test]
    fn ts_secs_populated() {
        let mut m = TownMemory::new();
        m.push_event("🎉", "測試");
        let ts = m.recent().back().unwrap().ts_secs;
        assert!(ts > 0, "時間戳應大於 0");
    }

    #[test]
    fn empty_memory_has_no_entries() {
        let m = TownMemory::new();
        assert!(m.recent().is_empty());
        assert!(m.recent_desc(10).is_empty());
    }

    #[test]
    fn push_event_icon_and_text_preserved() {
        let mut m = TownMemory::new();
        m.push_event("🏗️", "天文台大工程完工！");
        let e = m.recent().front().unwrap();
        assert_eq!(e.icon, "🏗️");
        assert_eq!(e.text, "天文台大工程完工！");
    }

    #[test]
    fn single_entry_recent_desc() {
        let mut m = TownMemory::new();
        m.push_event("🌠", "流星雨降臨");
        let desc = m.recent_desc(5);
        assert_eq!(desc.len(), 1);
        assert_eq!(desc[0].icon, "🌠");
    }
}
