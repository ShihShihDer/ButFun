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
//! - 解鎖任一成就（ROADMAP 439）→ 同名「成就稱號」（`Title::Achieved`，把一直只是
//!   通知一閃而過、毫無去處的成就，接上既有名牌展示管線，成為可永久配戴炫耀的收藏）

use std::collections::HashSet;

use crate::achievement::Achievement;

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
    /// 成就稱號（ROADMAP 439）：解鎖某個成就同時解鎖一枚同名可展示稱號。
    /// 名稱／wire key 都委派給被包住的成就，零重複定義。
    Achieved(Achievement),
}

impl Title {
    /// 六個里程碑基礎稱號（不含成就稱號；成就稱號數量隨成就清單浮動，另由
    /// [`Achievement::all`] 列舉）。`from_wire_key` 會兩邊都查。
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
            Achieved(a) => a.display_name(),
        }
    }

    /// Wire key（前後端通訊用，snake_case）。成就稱號沿用成就自己的 wire key
    /// （與基礎稱號的 key 互不重疊，前端 TITLE_NAMES 兩套都有對照）。
    pub fn wire_key(self) -> &'static str {
        use Title::*;
        match self {
            Level10    => "level_10",
            Level20    => "level_20",
            Level30    => "level_30",
            FirstCraft => "first_craft",
            Inscription => "inscription",
            EpicGather => "epic_gather",
            Achieved(a) => a.wire_key(),
        }
    }

    /// 從 wire key 反查（前端送 SetTitle 時用）。先查基礎稱號，再退而查成就稱號。
    pub fn from_wire_key(key: &str) -> Option<Title> {
        if let Some(t) = Title::all().iter().find(|t| t.wire_key() == key).copied() {
            return Some(t);
        }
        Achievement::all()
            .iter()
            .find(|a| a.wire_key() == key)
            .map(|a| Title::Achieved(*a))
    }
}

/// 成就 → 對應的成就稱號（純函式，供 ws.rs 解鎖成就時順手解鎖同名稱號）。
pub fn title_for_achievement(a: Achievement) -> Title {
    Title::Achieved(a)
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

    // ── ROADMAP 439：成就稱號 ──────────────────────────────────────────────

    #[test]
    fn achievement_title_delegates_name_and_key() {
        let a = Achievement::Hunter;
        let t = title_for_achievement(a);
        assert_eq!(t.display_name(), a.display_name(), "成就稱號顯示名委派給成就");
        assert_eq!(t.wire_key(), a.wire_key(), "成就稱號 wire key 委派給成就");
    }

    #[test]
    fn achievement_title_round_trips_via_wire_key() {
        // 每個成就都能由其 wire key 反查回對應的成就稱號。
        for a in Achievement::all() {
            let key = a.wire_key();
            assert_eq!(
                Title::from_wire_key(key),
                Some(Title::Achieved(*a)),
                "成就稱號應能由 wire key 反查"
            );
        }
    }

    #[test]
    fn achievement_title_keys_disjoint_from_base() {
        // 成就稱號的 wire key 不可與六個基礎稱號撞號，否則 from_wire_key 會反查錯。
        let base: std::collections::HashSet<&str> =
            Title::all().iter().map(|t| t.wire_key()).collect();
        for a in Achievement::all() {
            assert!(
                !base.contains(a.wire_key()),
                "成就稱號 {} 不可與基礎稱號撞號",
                a.wire_key()
            );
        }
    }

    #[test]
    fn achievement_titles_are_collectible_and_displayable() {
        // 解鎖某成就稱號後可持有、可設為展示。
        let mut set = TitleSet::new();
        let t = title_for_achievement(Achievement::QuestHero);
        assert!(set.unlock(t), "首次解鎖回 true");
        assert!(!set.unlock(t), "重複解鎖回 false");
        assert!(set.has(t));
        assert!(set.set_active(Some(t)), "持有後可展示");
        assert_eq!(set.active_display_name(), Some(Achievement::QuestHero.display_name()));
    }
}
