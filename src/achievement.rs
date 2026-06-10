//! 成就系統（ROADMAP 30）：12 個成就涵蓋星際探索、戰鬥、社交、升級四類。
//!
//! 純邏輯層（無 IO / 無 WebSocket）：定義成就種類、解鎖條件判斷、集合管理。
//! 重啟後成就清空（v1 記憶體前置）。

use std::collections::HashSet;

/// 所有可解鎖的成就。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Achievement {
    // ── 星際探索 ──────────────────────────────────────────────
    /// 首次前往翠幽星
    TravelVerdant,
    /// 首次前往赤焰星
    TravelCrimson,
    /// 首次前往虛空星
    TravelVoid,
    /// 首次前往霧醚星
    TravelAether,
    /// 首次前往星源星
    TravelOrigin,
    // ── 戰鬥 ─────────────────────────────────────────────────
    /// 首次擊殺敵人
    FirstKill,
    /// 擊殺 50 個敵人
    Hunter,
    // ── 升級 ─────────────────────────────────────────────────
    /// 達到 5 級
    LevelFive,
    /// 達到 10 級
    LevelTen,
    /// 達到 20 級
    LevelTwenty,
    // ── 社交 ─────────────────────────────────────────────────
    /// 加入或建立一個公會
    GuildMember,
    /// 參與完成一次全服任務
    QuestHero,
}

impl Achievement {
    pub fn all() -> &'static [Achievement] {
        use Achievement::*;
        &[
            TravelVerdant, TravelCrimson, TravelVoid, TravelAether, TravelOrigin,
            FirstKill, Hunter,
            LevelFive, LevelTen, LevelTwenty,
            GuildMember, QuestHero,
        ]
    }

    /// 成就名稱（繁中，顯示用）。
    pub fn display_name(self) -> &'static str {
        use Achievement::*;
        match self {
            TravelVerdant => "翠幽先驅者",
            TravelCrimson => "赤焰先驅者",
            TravelVoid    => "虛空先驅者",
            TravelAether  => "霧醚先驅者",
            TravelOrigin  => "星源先驅者",
            FirstKill     => "初次獵殺",
            Hunter        => "老練獵人",
            LevelFive     => "Lv.5 冒險者",
            LevelTen      => "Lv.10 精英",
            LevelTwenty   => "Lv.20 傳說",
            GuildMember   => "公會成員",
            QuestHero     => "任務英雄",
        }
    }

    /// 成就說明（繁中，面板顯示用）。
    pub fn description(self) -> &'static str {
        use Achievement::*;
        match self {
            TravelVerdant => "首次前往翠幽星",
            TravelCrimson => "首次前往赤焰星",
            TravelVoid    => "首次前往虛空星",
            TravelAether  => "首次前往霧醚星",
            TravelOrigin  => "首次前往星源星",
            FirstKill     => "擊殺第一個敵人",
            Hunter        => "累積擊殺 50 個敵人",
            LevelFive     => "達到 5 級",
            LevelTen      => "達到 10 級",
            LevelTwenty   => "達到 20 級",
            GuildMember   => "加入或建立一個公會",
            QuestHero     => "參與完成一次全服任務",
        }
    }

    /// Wire key（前端用，snake_case）。
    pub fn wire_key(self) -> &'static str {
        use Achievement::*;
        match self {
            TravelVerdant => "travel_verdant",
            TravelCrimson => "travel_crimson",
            TravelVoid    => "travel_void",
            TravelAether  => "travel_aether",
            TravelOrigin  => "travel_origin",
            FirstKill     => "first_kill",
            Hunter        => "hunter",
            LevelFive     => "level_five",
            LevelTen      => "level_ten",
            LevelTwenty   => "level_twenty",
            GuildMember   => "guild_member",
            QuestHero     => "quest_hero",
        }
    }

    /// 從 wire key 反查（前端傳回時用，目前僅供測試）。
    pub fn from_wire_key(key: &str) -> Option<Achievement> {
        Achievement::all().iter().find(|a| a.wire_key() == key).copied()
    }
}

/// 玩家的成就集合（記憶體前置，重啟清空）。
#[derive(Debug, Clone, Default)]
pub struct AchievementSet {
    inner: HashSet<Achievement>,
}

impl AchievementSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// 解鎖成就。尚未解鎖時插入並回傳 `true`（新成就）；已解鎖回傳 `false`。
    pub fn unlock(&mut self, a: Achievement) -> bool {
        self.inner.insert(a)
    }

    pub fn has(&self, a: Achievement) -> bool {
        self.inner.contains(&a)
    }

    pub fn count(&self) -> usize {
        self.inner.len()
    }

    /// 依 wire key 字母序輸出已解鎖的清單（快照廣播用）。
    pub fn as_wire_keys(&self) -> Vec<&'static str> {
        let mut keys: Vec<&'static str> = self.inner.iter().map(|a| a.wire_key()).collect();
        keys.sort_unstable();
        keys
    }
}

// ── 觸發判斷（純函式，供 ws.rs 呼叫）────────────────────────────────────────

/// 旅行到指定星球是否觸發成就。
pub fn achievement_for_planet(planet: &str) -> Option<Achievement> {
    match planet {
        "verdant" => Some(Achievement::TravelVerdant),
        "crimson" => Some(Achievement::TravelCrimson),
        "void"    => Some(Achievement::TravelVoid),
        "aether"  => Some(Achievement::TravelAether),
        "origin"  => Some(Achievement::TravelOrigin),
        _         => None,
    }
}

/// 等級升到 `new_level` 時應解鎖哪些成就（可能有多個）。
pub fn achievements_for_level(new_level: u32) -> Vec<Achievement> {
    let mut out = Vec::new();
    if new_level >= 5  { out.push(Achievement::LevelFive); }
    if new_level >= 10 { out.push(Achievement::LevelTen); }
    if new_level >= 20 { out.push(Achievement::LevelTwenty); }
    out
}

/// 累計擊殺數達到某個里程碑時觸發的成就。
pub fn achievement_for_kill_count(count: u32) -> Option<Achievement> {
    match count {
        1  => Some(Achievement::FirstKill),
        50 => Some(Achievement::Hunter),
        _  => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlock_returns_true_only_first_time() {
        let mut set = AchievementSet::new();
        assert!(set.unlock(Achievement::FirstKill));
        assert!(!set.unlock(Achievement::FirstKill), "重複解鎖應回 false");
        assert_eq!(set.count(), 1);
    }

    #[test]
    fn has_after_unlock() {
        let mut set = AchievementSet::new();
        assert!(!set.has(Achievement::GuildMember));
        set.unlock(Achievement::GuildMember);
        assert!(set.has(Achievement::GuildMember));
    }

    #[test]
    fn achievement_for_planet_maps_correctly() {
        assert_eq!(achievement_for_planet("verdant"), Some(Achievement::TravelVerdant));
        assert_eq!(achievement_for_planet("crimson"), Some(Achievement::TravelCrimson));
        assert_eq!(achievement_for_planet("void"),    Some(Achievement::TravelVoid));
        assert_eq!(achievement_for_planet("aether"),  Some(Achievement::TravelAether));
        assert_eq!(achievement_for_planet("origin"),  Some(Achievement::TravelOrigin));
        assert_eq!(achievement_for_planet("home"),    None);
        assert_eq!(achievement_for_planet("unknown"), None);
    }

    #[test]
    fn achievements_for_level_correct() {
        assert!(achievements_for_level(4).is_empty(), "Lv.4 無成就");
        let at5 = achievements_for_level(5);
        assert_eq!(at5, vec![Achievement::LevelFive]);
        let at10 = achievements_for_level(10);
        assert!(at10.contains(&Achievement::LevelFive));
        assert!(at10.contains(&Achievement::LevelTen));
        assert!(!at10.contains(&Achievement::LevelTwenty));
        let at20 = achievements_for_level(20);
        assert!(at20.contains(&Achievement::LevelTwenty));
    }

    #[test]
    fn kill_count_triggers() {
        assert_eq!(achievement_for_kill_count(1),  Some(Achievement::FirstKill));
        assert_eq!(achievement_for_kill_count(50), Some(Achievement::Hunter));
        assert_eq!(achievement_for_kill_count(2),  None);
        assert_eq!(achievement_for_kill_count(49), None);
        assert_eq!(achievement_for_kill_count(51), None);
    }

    #[test]
    fn all_achievements_have_metadata() {
        for a in Achievement::all() {
            assert!(!a.display_name().is_empty(), "display_name 不可空");
            assert!(!a.description().is_empty(), "description 不可空");
            assert!(!a.wire_key().is_empty(), "wire_key 不可空");
        }
    }

    #[test]
    fn wire_keys_are_unique() {
        let keys: Vec<_> = Achievement::all().iter().map(|a| a.wire_key()).collect();
        let unique: HashSet<_> = keys.iter().collect();
        assert_eq!(keys.len(), unique.len(), "wire key 應唯一");
    }

    #[test]
    fn as_wire_keys_sorted_alphabetically() {
        let mut set = AchievementSet::new();
        set.unlock(Achievement::Hunter);
        set.unlock(Achievement::FirstKill);
        let keys = set.as_wire_keys();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "wire keys 應依字母排序");
    }

    #[test]
    fn from_wire_key_round_trip() {
        for a in Achievement::all() {
            assert_eq!(Achievement::from_wire_key(a.wire_key()), Some(*a));
        }
    }

    #[test]
    fn total_count_is_twelve() {
        assert_eq!(Achievement::all().len(), 12, "總成就數應為 12");
    }
}
