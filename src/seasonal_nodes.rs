//! 季節性野外特產節點（ROADMAP 138）。
//!
//! 每個季節在城鎮外圍生成 3 個特有採集點，玩家走近可採集季節性素材。
//! 每個節點有 3 次採集次數（可供多人共享），季節切換時節點重置。
//!
//! 素材用途：
//!   🌸 春 → 野花（WildFlower）→ 合成春日香囊（回血 25hp + 重置回血冷卻）
//!   ☀️ 夏 → 太陽碎片（SolarShard）→ 合成夏日精粹（回血 15hp + 15 乙太）
//!   🍂 秋 → 楓葉（MapleLeaf）→ 合成秋日補藥（回血 20hp + 農夫熟練度 +20）
//!   ❄️ 冬 → 冰晶碎片（IceShard）→ 合成冬日神藥（回復至滿血）
//!
//! 成本紀律：純本機邏輯，零 LLM，零 migration，記憶體模式，重啟從當前季節重新生成。

use crate::{inventory::ItemKind, season::Season};

/// 採集有效距離（像素）。
pub const GATHER_REACH: f32 = 80.0;
/// 每個節點的採集次數上限（多人共享）。
pub const NODE_CHARGES: u8 = 3;
/// 每季節的採集節點數量。
pub const NODES_PER_SEASON: usize = 3;

// 各季節的節點位置（世界座標，分散於城鎮不同方位）。
// 刻意與流星雨節點（約 2100~2700）錯開，讓玩家需要探索。

/// 春天節點位置（城鎮西北花田附近）。
const SPRING_POSITIONS: [(f32, f32); NODES_PER_SEASON] = [
    (1900.0, 1800.0),
    (2050.0, 1700.0),
    (1800.0, 2100.0),
];

/// 夏天節點位置（城鎮東方日照強的開闊地）。
const SUMMER_POSITIONS: [(f32, f32); NODES_PER_SEASON] = [
    (2800.0, 2000.0),
    (2950.0, 2300.0),
    (2700.0, 2600.0),
];

/// 秋天節點位置（城鎮南方楓林區）。
const AUTUMN_POSITIONS: [(f32, f32); NODES_PER_SEASON] = [
    (2200.0, 2900.0),
    (2500.0, 3050.0),
    (2000.0, 3100.0),
];

/// 冬天節點位置（城鎮西北寒冷高地）。
const WINTER_POSITIONS: [(f32, f32); NODES_PER_SEASON] = [
    (1700.0, 2400.0),
    (1850.0, 2700.0),
    (1600.0, 2200.0),
];

/// 一個季節性採集節點。
#[derive(Debug, Clone)]
pub struct SeasonalNode {
    /// 節點唯一 ID（遞增計數器確保跨季節不重複）。
    pub id: u32,
    /// 世界座標。
    pub wx: f32,
    pub wy: f32,
    /// 剩餘採集次數（0 = 已耗盡）。
    pub charges: u8,
    /// 此節點屬於哪個季節（決定採集物品種類）。
    pub season: Season,
}

/// 季節性採集節點狀態（記憶體模式）。
pub struct SeasonalNodesState {
    /// 當前活躍節點列表（隨季節切換重置）。
    pub nodes: Vec<SeasonalNode>,
    /// 節點 ID 遞增計數器（跨季節唯一）。
    node_counter: u32,
    /// 目前節點對應的季節（用於偵測季節切換）。
    pub active_season: Season,
}

impl SeasonalNodesState {
    /// 建立初始狀態，從春天開始生成節點（與 SeasonState 一致）。
    pub fn new() -> Self {
        let mut state = Self {
            nodes: vec![],
            node_counter: 0,
            active_season: Season::Spring,
        };
        state.spawn_nodes(Season::Spring);
        state
    }

    /// 取得有剩餘採集次數的節點（供快照廣播給前端）。
    pub fn active_nodes(&self) -> impl Iterator<Item = &SeasonalNode> {
        self.nodes.iter().filter(|n| n.charges > 0)
    }

    /// 依季節取得對應的採集物品。
    pub fn item_for_season(season: Season) -> ItemKind {
        match season {
            Season::Spring => ItemKind::WildFlower,
            Season::Summer => ItemKind::SolarShard,
            Season::Autumn => ItemKind::MapleLeaf,
            Season::Winter => ItemKind::IceShard,
        }
    }

    /// 當季節切換時重置節點。
    pub fn on_season_change(&mut self, new_season: Season) {
        self.active_season = new_season;
        self.spawn_nodes(new_season);
    }

    /// 嘗試採集指定節點（驗證距離 + 剩餘次數）。
    /// 回傳 `Some(ItemKind)` 表示採集成功（回傳對應物品種類），`None` 表示失敗。
    pub fn try_gather(&mut self, node_id: u32, px: f32, py: f32) -> Option<ItemKind> {
        let node = self.nodes.iter_mut().find(|n| n.id == node_id && n.charges > 0)?;
        let dx = node.wx - px;
        let dy = node.wy - py;
        if dx * dx + dy * dy > GATHER_REACH * GATHER_REACH {
            return None;
        }
        node.charges = node.charges.saturating_sub(1);
        Some(Self::item_for_season(node.season))
    }

    /// 生成指定季節的節點。
    fn spawn_nodes(&mut self, season: Season) {
        let positions: &[(f32, f32)] = match season {
            Season::Spring => &SPRING_POSITIONS,
            Season::Summer => &SUMMER_POSITIONS,
            Season::Autumn => &AUTUMN_POSITIONS,
            Season::Winter => &WINTER_POSITIONS,
        };
        self.nodes = positions.iter().enumerate().map(|(i, &(wx, wy))| {
            let id = self.node_counter + i as u32;
            SeasonalNode { id, wx, wy, charges: NODE_CHARGES, season }
        }).collect();
        self.node_counter = self.node_counter.wrapping_add(NODES_PER_SEASON as u32);
    }
}

impl Default for SeasonalNodesState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── 純邏輯單元測試 ────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> SeasonalNodesState { SeasonalNodesState::new() }

    #[test]
    fn starts_with_spring_nodes() {
        let s = make();
        assert_eq!(s.nodes.len(), NODES_PER_SEASON, "春天應有 3 個節點");
        assert!(s.nodes.iter().all(|n| n.season == Season::Spring), "初始節點應為春天");
    }

    #[test]
    fn spring_nodes_give_wild_flower() {
        assert_eq!(SeasonalNodesState::item_for_season(Season::Spring), ItemKind::WildFlower);
    }

    #[test]
    fn summer_nodes_give_solar_shard() {
        assert_eq!(SeasonalNodesState::item_for_season(Season::Summer), ItemKind::SolarShard);
    }

    #[test]
    fn autumn_nodes_give_maple_leaf() {
        assert_eq!(SeasonalNodesState::item_for_season(Season::Autumn), ItemKind::MapleLeaf);
    }

    #[test]
    fn winter_nodes_give_ice_shard() {
        assert_eq!(SeasonalNodesState::item_for_season(Season::Winter), ItemKind::IceShard);
    }

    #[test]
    fn all_nodes_start_with_full_charges() {
        let s = make();
        assert!(s.nodes.iter().all(|n| n.charges == NODE_CHARGES), "所有節點應有滿格次數");
    }

    #[test]
    fn try_gather_in_range_succeeds() {
        let mut s = make();
        let node = &s.nodes[0];
        let (wx, wy, id) = (node.wx, node.wy, node.id);
        let result = s.try_gather(id, wx + 10.0, wy + 10.0);
        assert_eq!(result, Some(ItemKind::WildFlower), "在範圍內應成功採集");
    }

    #[test]
    fn try_gather_reduces_charges() {
        let mut s = make();
        let id = s.nodes[0].id;
        let (wx, wy) = (s.nodes[0].wx, s.nodes[0].wy);
        s.try_gather(id, wx, wy);
        assert_eq!(s.nodes[0].charges, NODE_CHARGES - 1, "採集後次數應減 1");
    }

    #[test]
    fn try_gather_out_of_range_fails() {
        let mut s = make();
        let node = &s.nodes[0];
        let (wx, wy, id) = (node.wx, node.wy, node.id);
        let result = s.try_gather(id, wx + 200.0, wy + 200.0);
        assert!(result.is_none(), "超出範圍不應成功");
        assert_eq!(s.nodes[0].charges, NODE_CHARGES, "失敗時次數不應改變");
    }

    #[test]
    fn node_exhausted_after_all_charges_used() {
        let mut s = make();
        let id = s.nodes[0].id;
        let (wx, wy) = (s.nodes[0].wx, s.nodes[0].wy);
        for _ in 0..NODE_CHARGES {
            s.try_gather(id, wx, wy);
        }
        assert_eq!(s.nodes[0].charges, 0, "耗盡後 charges 應為 0");
        let result = s.try_gather(id, wx, wy);
        assert!(result.is_none(), "耗盡後再採集應失敗");
    }

    #[test]
    fn active_nodes_excludes_exhausted() {
        let mut s = make();
        let id = s.nodes[0].id;
        let (wx, wy) = (s.nodes[0].wx, s.nodes[0].wy);
        for _ in 0..NODE_CHARGES {
            s.try_gather(id, wx, wy);
        }
        let active_count = s.active_nodes().count();
        assert_eq!(active_count, NODES_PER_SEASON - 1, "耗盡節點不應出現在 active_nodes");
    }

    #[test]
    fn season_change_resets_nodes() {
        let mut s = make();
        // 耗盡一個節點
        let id = s.nodes[0].id;
        let (wx, wy) = (s.nodes[0].wx, s.nodes[0].wy);
        for _ in 0..NODE_CHARGES { s.try_gather(id, wx, wy); }
        // 切換到夏天
        s.on_season_change(Season::Summer);
        assert_eq!(s.active_season, Season::Summer);
        assert_eq!(s.nodes.len(), NODES_PER_SEASON, "切換後應有新節點");
        assert!(s.nodes.iter().all(|n| n.season == Season::Summer), "切換後應全為夏天節點");
        assert!(s.nodes.iter().all(|n| n.charges == NODE_CHARGES), "新節點應有滿格次數");
    }

    #[test]
    fn node_ids_unique_across_seasons() {
        let mut s = make();
        let spring_ids: Vec<u32> = s.nodes.iter().map(|n| n.id).collect();
        s.on_season_change(Season::Summer);
        let summer_ids: Vec<u32> = s.nodes.iter().map(|n| n.id).collect();
        for id in &summer_ids {
            assert!(!spring_ids.contains(id), "不同季節節點 ID 不應重複");
        }
    }

    #[test]
    fn four_seasons_all_have_distinct_positions() {
        let mut s = make();
        let spring_pos: Vec<_> = s.nodes.iter().map(|n| (n.wx as i32, n.wy as i32)).collect();
        s.on_season_change(Season::Summer);
        let summer_pos: Vec<_> = s.nodes.iter().map(|n| (n.wx as i32, n.wy as i32)).collect();
        s.on_season_change(Season::Autumn);
        let autumn_pos: Vec<_> = s.nodes.iter().map(|n| (n.wx as i32, n.wy as i32)).collect();
        s.on_season_change(Season::Winter);
        let winter_pos: Vec<_> = s.nodes.iter().map(|n| (n.wx as i32, n.wy as i32)).collect();
        // 各季節的位置集合彼此不重疊
        for pos in &summer_pos { assert!(!spring_pos.contains(pos)); }
        for pos in &autumn_pos { assert!(!spring_pos.contains(pos)); assert!(!summer_pos.contains(pos)); }
        for pos in &winter_pos { assert!(!spring_pos.contains(pos)); assert!(!autumn_pos.contains(pos)); }
    }
}
