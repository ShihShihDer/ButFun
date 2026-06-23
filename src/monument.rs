//! 旅人紀念碑（ROADMAP 526）——城鎮廣場石碑銘記首批重大成就。
//!
//! 紀念碑銘記四類成就：
//! - 世界奇觀首探：每座奇觀的第一位踏入者（永久，不可覆蓋）。
//! - 守護者首殺：首位率先擊破世界守護者的旅人（永久，不可覆蓋）。
//! - 釣魚大賽冠軍：最近一次釣魚大賽的金牌得主（隨賽程更新）。
//! - 礦脈爭奪冠軍：最近一次礦脈爭奪的採礦最多者（隨事件更新）。
//!
//! 玩家在城鎮廣場走近紀念碑（MONUMENT_WX, MONUMENT_WY）時可查看列表。
//! 純記憶體、零 migration、零 LLM；重啟清零（成就是體驗，不需持久化）。

use crate::protocol::MonumentEntryView;

/// 紀念碑世界座標 X（廣場北方）。
pub const MONUMENT_WX: f32 = 0.0;
/// 紀念碑世界座標 Y（廣場北方，負值往上）。
pub const MONUMENT_WY: f32 = -600.0;
/// 靠近判定半徑（像素）。
pub const MONUMENT_RADIUS: f32 = 150.0;

// ── 插槽鍵（決定每條目的唯一性與可覆蓋性）─────────────────────────────────
const SLOT_BOSS_FIRST_KILL: &str = "boss_first_kill";
const SLOT_FISHING_CHAMPION: &str = "fishing_champion";
const SLOT_GOLD_RUSH_CHAMPION: &str = "gold_rush_champion";

/// 一條紀念碑刻文。
#[derive(Debug, Clone)]
pub struct MonumentEntry {
    /// 插槽鍵（用於去重與可覆蓋判定，不對外暴露）。
    slot: String,
    /// 是否永久（true = 只刻一次，false = 可由後來者覆蓋）。
    permanent: bool,
    /// 面向玩家的成就標題。
    pub label: String,
    /// 立功旅人名稱。
    pub player_name: String,
    /// 補充說明（如「釣出 142cm」）。
    pub detail: String,
}

/// 紀念碑整體狀態（純記憶體）。
#[derive(Debug, Default)]
pub struct Monument {
    entries: Vec<MonumentEntry>,
}

impl Monument {
    pub fn new() -> Self {
        Self::default()
    }

    // ── 純函式：靠近判定 ───────────────────────────────────────────────────────

    /// 回傳玩家是否在紀念碑靠近範圍內（NaN / Infinity 保守回 false）。
    pub fn is_near(px: f32, py: f32) -> bool {
        if !px.is_finite() || !py.is_finite() {
            return false;
        }
        let dx = px - MONUMENT_WX;
        let dy = py - MONUMENT_WY;
        dx * dx + dy * dy <= MONUMENT_RADIUS * MONUMENT_RADIUS
    }

    // ── 記錄方法 ───────────────────────────────────────────────────────────────

    /// 記錄世界奇觀首探者。若該奇觀已有首探者則靜默忽略（永久）。
    pub fn record_wonder(&mut self, wonder_key: &str, wonder_emoji: &str, wonder_name_zh: &str, player_name: &str) {
        let slot = format!("wonder_{}", wonder_key);
        if self.entries.iter().any(|e| e.slot == slot && e.permanent) {
            return; // 已有首探者，不覆蓋
        }
        self.upsert(MonumentEntry {
            slot,
            permanent: true,
            label: format!("{} {}首探", wonder_emoji, wonder_name_zh),
            player_name: player_name.to_string(),
            detail: "大膽踏入神秘秘境，留名千古".to_string(),
        });
    }

    /// 記錄守護者首殺者。只記一次，永久不可覆蓋。
    pub fn record_boss_first_kill(&mut self, player_name: &str) {
        let slot = SLOT_BOSS_FIRST_KILL.to_string();
        if self.entries.iter().any(|e| e.slot == slot && e.permanent) {
            return; // 已有首殺者
        }
        self.upsert(MonumentEntry {
            slot,
            permanent: true,
            label: "🗿 守護者首殺".to_string(),
            player_name: player_name.to_string(),
            detail: "率先擊破世界守護者，英名永載石碑".to_string(),
        });
    }

    /// 記錄最近一次釣魚大賽冠軍（可更新）。
    pub fn record_fishing_champion(&mut self, player_name: &str, total_cm: u32, catch_count: u32) {
        self.upsert(MonumentEntry {
            slot: SLOT_FISHING_CHAMPION.to_string(),
            permanent: false,
            label: "🎣 釣魚大賽冠軍".to_string(),
            player_name: player_name.to_string(),
            detail: format!("釣出 {}cm 魚獲 / 共 {} 尾", total_cm, catch_count),
        });
    }

    /// 記錄最近一次礦脈爭奪冠軍（可更新）。
    pub fn record_gold_rush_champion(&mut self, player_name: &str, ore_count: u32) {
        self.upsert(MonumentEntry {
            slot: SLOT_GOLD_RUSH_CHAMPION.to_string(),
            permanent: false,
            label: "⛏️ 礦脈爭奪冠軍".to_string(),
            player_name: player_name.to_string(),
            detail: format!("本場採礦 {} 顆，全服第一", ore_count),
        });
    }

    // ── 讀取 ───────────────────────────────────────────────────────────────────

    /// 輸出所有刻文（供 Snapshot 廣播）。
    pub fn view(&self) -> Vec<MonumentEntryView> {
        self.entries.iter().map(|e| MonumentEntryView {
            label:       e.label.clone(),
            player_name: e.player_name.clone(),
            detail:      e.detail.clone(),
        }).collect()
    }

    // ── 私有工具 ───────────────────────────────────────────────────────────────

    /// 依插槽鍵更新或插入刻文。
    fn upsert(&mut self, entry: MonumentEntry) {
        if let Some(pos) = self.entries.iter().position(|e| e.slot == entry.slot) {
            self.entries[pos] = entry;
        } else {
            self.entries.push(entry);
        }
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_monument_is_empty() {
        let m = Monument::new();
        assert!(m.view().is_empty());
    }

    #[test]
    fn is_near_center_true() {
        assert!(Monument::is_near(0.0, -600.0));
    }

    #[test]
    fn is_near_edge_true() {
        // 邊界：距離恰好等於 MONUMENT_RADIUS
        assert!(Monument::is_near(MONUMENT_WX + MONUMENT_RADIUS, MONUMENT_WY));
    }

    #[test]
    fn is_near_outside_false() {
        assert!(!Monument::is_near(MONUMENT_WX + MONUMENT_RADIUS + 1.0, MONUMENT_WY));
    }

    #[test]
    fn is_near_nan_false() {
        assert!(!Monument::is_near(f32::NAN, 0.0));
        assert!(!Monument::is_near(0.0, f32::NAN));
    }

    #[test]
    fn is_near_infinity_false() {
        assert!(!Monument::is_near(f32::INFINITY, 0.0));
        assert!(!Monument::is_near(0.0, f32::NEG_INFINITY));
    }

    #[test]
    fn record_wonder_appears_in_view() {
        let mut m = Monument::new();
        m.record_wonder("star_core", "💎", "星核晶宮", "旅人甲");
        let v = m.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].player_name, "旅人甲");
        assert!(v[0].label.contains("星核晶宮"));
    }

    #[test]
    fn same_wonder_second_record_ignored() {
        let mut m = Monument::new();
        m.record_wonder("star_core", "💎", "星核晶宮", "旅人甲");
        m.record_wonder("star_core", "💎", "星核晶宮", "旅人乙"); // 應被忽略
        let v = m.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].player_name, "旅人甲");
    }

    #[test]
    fn different_wonders_all_recorded() {
        let mut m = Monument::new();
        m.record_wonder("star_core", "💎", "星核晶宮", "甲");
        m.record_wonder("jade_tree", "🌳", "翡翠古樹", "乙");
        m.record_wonder("moon_temple", "🏛️", "黃沙月神殿", "丙");
        assert_eq!(m.view().len(), 3);
    }

    #[test]
    fn record_boss_first_kill_appears() {
        let mut m = Monument::new();
        m.record_boss_first_kill("英雄甲");
        let v = m.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].player_name, "英雄甲");
        assert!(v[0].label.contains("守護者首殺"));
    }

    #[test]
    fn record_boss_first_kill_second_time_ignored() {
        let mut m = Monument::new();
        m.record_boss_first_kill("英雄甲");
        m.record_boss_first_kill("英雄乙"); // 不可覆蓋
        let v = m.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].player_name, "英雄甲");
    }

    #[test]
    fn record_fishing_champion_updates() {
        let mut m = Monument::new();
        m.record_fishing_champion("釣魚高手", 142, 5);
        {
            let v = m.view();
            assert_eq!(v[0].player_name, "釣魚高手");
        }
        m.record_fishing_champion("超級釣手", 200, 8); // 可更新
        let v = m.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].player_name, "超級釣手");
        assert!(v[0].detail.contains("200cm"));
    }

    #[test]
    fn record_gold_rush_champion_updates() {
        let mut m = Monument::new();
        m.record_gold_rush_champion("礦工甲", 30);
        m.record_gold_rush_champion("礦工乙", 50); // 可更新
        let v = m.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].player_name, "礦工乙");
        assert!(v[0].detail.contains("50"));
    }

    #[test]
    fn mix_of_all_types_correct_count() {
        let mut m = Monument::new();
        m.record_wonder("star_core", "💎", "星核晶宮", "甲");
        m.record_boss_first_kill("乙");
        m.record_fishing_champion("丙", 100, 3);
        m.record_gold_rush_champion("丁", 20);
        assert_eq!(m.view().len(), 4);
    }
}
