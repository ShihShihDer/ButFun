//! 角色稱號系統（ROADMAP 389）：達成里程碑自動解鎖稱號，玩家可選擇展示的稱號顯示在名牌旁。
//!
//! 純邏輯層（無 IO / 無 WebSocket）：定義稱號種類、解鎖條件判斷、選擇管理。
//! 記憶體前置，重啟清空（稱號鼓勵再次達成里程碑）。
//!
//! 稱號來源：
//! - 等級 10 → 旅者
//! - 等級 20 → 冒險家
//! - 等級 30 → 傳說
//! - 首次鍛造儀式配方 → 工匠
//! - 首次解讀古代秘文 → 考古學家
//! - 史詩品質採集 → 福星

use std::collections::HashSet;

/// 所有可解鎖的稱號。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Title {
    /// 達到 Lv.10
    Level10,
    /// 達到 Lv.20
    Level20,
    /// 達到 Lv.30
    Level30,
    /// 首次鍛造儀式高階物品
    FirstCraft,
    /// 首次解讀古代秘文
    Inscription,
    /// 史詩品質採集（機率 ~1%）
    EpicGather,
}

impl Title {
    pub fn all() -> &'static [Title] {
        use Title::*;
        &[Level10, Level20, Level30, FirstCraft, Inscription, EpicGather]
    }

    /// 顯示名稱（繁中，名牌與面板用）。
    pub fn display_name(self) -> &'static str {
        use Title::*;
        match self {
            Level10    => "旅者",
            Level20    => "冒險家",
            Level30    => "傳說",
            FirstCraft => "工匠",
            Inscription => "考古學家",
            EpicGather => "福星",
        }
    }

    /// Wire key（前後端通訊用，snake_case）。
    pub fn wire_key(self) -> &'static str {
        use Title::*;
        match self {
            Level10    => "level_10",
            Level20    => "level_20",
            Level30    => "level_30",
            FirstCraft => "first_craft",
            Inscription => "inscription",
            EpicGather => "epic_gather",
        }
    }

    /// 從 wire key 反查（前端送 SetTitle 時用）。
    pub fn from_wire_key(key: &str) -> Option<Title> {
        Title::all().iter().find(|t| t.wire_key() == key).copied()
    }
}

/// 玩家的稱號集合（記憶體前置，重啟清空）。
#[derive(Debug, Clone, Default)]
pub struct TitleSet {
    unlocked: HashSet<Title>,
    /// 目前選擇展示的稱號（None = 不展示）。
    active: Option<Title>,
}

impl TitleSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// 解鎖稱號。尚未解鎖時插入並回傳 `true`（新稱號）；已解鎖回傳 `false`。
    pub fn unlock(&mut self, t: Title) -> bool {
        self.unlocked.insert(t)
    }

    pub fn has(&self, t: Title) -> bool {
        self.unlocked.contains(&t)
    }

    /// 設定要展示的稱號。玩家未持有該稱號時回傳 `false`（不更新）；`None` 可清除。
    pub fn set_active(&mut self, t: Option<Title>) -> bool {
        match t {
            None => { self.active = None; true }
            Some(title) if self.has(title) => { self.active = Some(title); true }
            _ => false, // 未持有
        }
    }

    pub fn active(&self) -> Option<Title> {
        self.active
    }

    /// 目前展示的稱號 wire key（序列化給前端用）。
    pub fn active_wire_key(&self) -> Option<&'static str> {
        self.active.map(|t| t.wire_key())
    }

    /// 目前展示的稱號顯示名稱。
    pub fn active_display_name(&self) -> Option<&'static str> {
        self.active.map(|t| t.display_name())
    }

    /// 已解鎖的 wire key 清單（供快照廣播，前端稱號面板顯示）。
    pub fn unlocked_wire_keys(&self) -> Vec<&'static str> {
        let mut keys: Vec<&'static str> = self.unlocked.iter().map(|t| t.wire_key()).collect();
        keys.sort_unstable();
        keys
    }

    pub fn unlocked_count(&self) -> usize {
        self.unlocked.len()
    }
}

// ── 觸發判斷（純函式，供 ws.rs 呼叫）────────────────────────────────────────

/// 升到 `new_level` 時應解鎖哪個稱號（等級稱號依最高里程碑）。
pub fn title_for_level(new_level: u32) -> Option<Title> {
    if new_level >= 30 { Some(Title::Level30) }
    else if new_level >= 20 { Some(Title::Level20) }
    else if new_level >= 10 { Some(Title::Level10) }
    else { None }
}

/// 全服首次稱號廣播文字。
pub fn world_first_text(player_name: &str, title: Title) -> String {
    format!(
        "🏅 全服首位！【{}】在 ButFun 獲得了稱號「{}」！",
        player_name,
        title.display_name()
    )
}

/// 玩家解鎖稱號的個人通知文字。
pub fn unlock_text(title: Title) -> String {
    format!(
        "✨ 你解鎖了稱號「{}」！在角色面板選擇展示。",
        title.display_name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlock_returns_true_only_first_time() {
        let mut set = TitleSet::new();
        assert!(set.unlock(Title::Level10));
        assert!(!set.unlock(Title::Level10), "重複解鎖應回 false");
        assert_eq!(set.unlocked_count(), 1);
    }

    #[test]
    fn has_after_unlock() {
        let mut set = TitleSet::new();
        assert!(!set.has(Title::Level20));
        set.unlock(Title::Level20);
        assert!(set.has(Title::Level20));
    }

    #[test]
    fn set_active_requires_ownership() {
        let mut set = TitleSet::new();
        // 未持有時 set_active 回 false、不更新
        assert!(!set.set_active(Some(Title::Level10)));
        assert!(set.active().is_none(), "未持有不可設為 active");
        // 解鎖後可設
        set.unlock(Title::Level10);
        assert!(set.set_active(Some(Title::Level10)));
        assert_eq!(set.active(), Some(Title::Level10));
    }

    #[test]
    fn set_active_none_clears() {
        let mut set = TitleSet::new();
        set.unlock(Title::Level20);
        set.set_active(Some(Title::Level20));
        assert!(set.set_active(None));
        assert!(set.active().is_none());
    }

    #[test]
    fn active_wire_key_reflects_active() {
        let mut set = TitleSet::new();
        set.unlock(Title::EpicGather);
        set.set_active(Some(Title::EpicGather));
        assert_eq!(set.active_wire_key(), Some("epic_gather"));
    }

    #[test]
    fn title_for_level_boundaries() {
        assert!(title_for_level(9).is_none());
        assert_eq!(title_for_level(10), Some(Title::Level10));
        assert_eq!(title_for_level(19), Some(Title::Level10));
        assert_eq!(title_for_level(20), Some(Title::Level20));
        assert_eq!(title_for_level(29), Some(Title::Level20));
        assert_eq!(title_for_level(30), Some(Title::Level30));
        assert_eq!(title_for_level(99), Some(Title::Level30));
    }

    #[test]
    fn all_titles_have_unique_wire_keys() {
        let keys: Vec<_> = Title::all().iter().map(|t| t.wire_key()).collect();
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(keys.len(), unique.len(), "wire key 應唯一");
    }

    #[test]
    fn from_wire_key_round_trip() {
        for t in Title::all() {
            assert_eq!(Title::from_wire_key(t.wire_key()), Some(*t));
        }
        assert!(Title::from_wire_key("nonexistent").is_none());
    }

    #[test]
    fn unlocked_wire_keys_sorted() {
        let mut set = TitleSet::new();
        set.unlock(Title::Level30);
        set.unlock(Title::Level10);
        let keys = set.unlocked_wire_keys();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "wire keys 應依字母排序");
    }
}
