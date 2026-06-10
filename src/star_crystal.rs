//! 夜採星晶系統（ROADMAP 50）——夜間限定採集活動路線（探索者熟練度第五活動路線）。
//!
//! 夜晚降臨時，世界各處隨機出現「星晶礦脈」；玩家靠近可採集「星晶碎片」，
//! 天亮後礦脈自動消失。星晶碎片可賣給 NPC 或合成夜幻藥水（強效回血）。
//! 採集給探索者熟練度 XP，鼓勵夜間探索。
//!
//! 設計取捨：
//!   - **記憶體模式**：星晶礦脈每夜重置；重啟後無礦脈，等下一個夜晚重生。
//!   - **固定 20 個節點**：分散在出生點方圓 2000px 內。
//!   - **單採耗盡**：每個礦脈採一次即消失，鼓勵玩家競速探索。
//!   - **探索者 XP**：每採一個 +15 XP（第五條活動路線）。

use serde::Serialize;

/// 廣播給前端的星晶礦脈視圖。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StarCrystalView {
    pub x: f32,
    pub y: f32,
}

/// 玩家採集星晶礦脈的伸手範圍（像素）。
pub const GATHER_REACH: f32 = 80.0;

/// 每夜生成的星晶礦脈數量。
pub const NODE_COUNT: usize = 20;

/// 採集一個星晶礦脈給予的探索者熟練度 XP。
pub const GATHER_EXPLORER_XP: u32 = 15;

/// 夜晚礦脈散布的最大半徑（像素）。
const SCATTER_RADIUS: f32 = 2000.0;

/// 一個星晶礦脈節點。
#[derive(Debug, Clone)]
pub struct StarCrystalNode {
    pub x: f32,
    pub y: f32,
    /// 已被採集則為 true，廣播時過濾掉。
    pub gathered: bool,
}

/// 夜間星晶礦脈管理器。
#[derive(Debug, Default)]
pub struct StarCrystalField {
    nodes: Vec<StarCrystalNode>,
    /// 目前第幾夜（每次進夜間 +1），用來生成不同的礦脈位置。
    night_cycle: u64,
}

impl StarCrystalField {
    pub fn new() -> Self {
        Self::default()
    }

    /// 進入夜間時呼叫：生成本夜的 NODE_COUNT 個礦脈。
    pub fn spawn_for_night(&mut self) {
        self.night_cycle = self.night_cycle.wrapping_add(1);
        self.nodes.clear();
        for i in 0..NODE_COUNT {
            let (x, y) = scatter_position(self.night_cycle, i as u64);
            self.nodes.push(StarCrystalNode { x, y, gathered: false });
        }
    }

    /// 退出夜間時呼叫：清除所有礦脈。
    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    /// 嘗試採集距離 (px, py) 最近且未採集的礦脈。
    /// 成功採集回傳 `true`，無礦脈或太遠回傳 `false`。
    pub fn gather_near(&mut self, px: f32, py: f32) -> bool {
        let reach_sq = GATHER_REACH * GATHER_REACH;
        let best = self.nodes.iter_mut()
            .filter(|n| !n.gathered)
            .filter(|n| {
                let dx = n.x - px;
                let dy = n.y - py;
                dx * dx + dy * dy <= reach_sq
            })
            .min_by(|a, b| {
                let da = (a.x - px).powi(2) + (a.y - py).powi(2);
                let db = (b.x - px).powi(2) + (b.y - py).powi(2);
                da.partial_cmp(&db).unwrap()
            });
        if let Some(node) = best {
            node.gathered = true;
            true
        } else {
            false
        }
    }

    /// 回傳目前仍可採集的礦脈清單（供廣播用）。
    pub fn active_nodes(&self) -> Vec<(f32, f32)> {
        self.nodes.iter()
            .filter(|n| !n.gathered)
            .map(|n| (n.x, n.y))
            .collect()
    }

    /// 回傳給前端廣播的 View 清單。
    pub fn views(&self) -> Vec<StarCrystalView> {
        self.nodes.iter()
            .filter(|n| !n.gathered)
            .map(|n| StarCrystalView { x: n.x, y: n.y })
            .collect()
    }

    /// 目前有無任何活躍礦脈。
    pub fn has_active(&self) -> bool {
        self.nodes.iter().any(|n| !n.gathered)
    }
}

/// 確定性位置生成：依夜晚週期 + 節點索引散布在出生點附近。
fn scatter_position(cycle: u64, idx: u64) -> (f32, f32) {
    // 兩次 Wang hash 混合，分別作為 x/y 軸
    let mut h = cycle.wrapping_mul(0x517C_C1B7_2722_0A95);
    h = h.wrapping_add(idx.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;

    let fx = ((h & 0xFFFF) as f32 / 65535.0) * 2.0 - 1.0; // -1..1
    let fy = (((h >> 16) & 0xFFFF) as f32 / 65535.0) * 2.0 - 1.0;
    (fx * SCATTER_RADIUS, fy * SCATTER_RADIUS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_generates_node_count_nodes() {
        let mut field = StarCrystalField::new();
        field.spawn_for_night();
        assert_eq!(field.active_nodes().len(), NODE_COUNT);
    }

    #[test]
    fn clear_removes_all_nodes() {
        let mut field = StarCrystalField::new();
        field.spawn_for_night();
        field.clear();
        assert_eq!(field.active_nodes().len(), 0);
    }

    #[test]
    fn gather_near_succeeds_within_reach() {
        let mut field = StarCrystalField::new();
        field.spawn_for_night();
        // 把第一個節點的座標取出，站在它正上方
        let (nx, ny) = (field.nodes[0].x, field.nodes[0].y);
        assert!(field.gather_near(nx, ny));
        // 採完後該節點不再出現在 active_nodes
        assert_eq!(field.active_nodes().len(), NODE_COUNT - 1);
    }

    #[test]
    fn gather_near_fails_out_of_reach() {
        let mut field = StarCrystalField::new();
        field.spawn_for_night();
        let (nx, ny) = (field.nodes[0].x, field.nodes[0].y);
        // 站在礦脈 GATHER_REACH + 1 之外
        let result = field.gather_near(nx + GATHER_REACH + 1.0, ny);
        assert!(!result);
        assert_eq!(field.active_nodes().len(), NODE_COUNT);
    }

    #[test]
    fn gather_same_node_twice_fails() {
        let mut field = StarCrystalField::new();
        field.spawn_for_night();
        let (nx, ny) = (field.nodes[0].x, field.nodes[0].y);
        assert!(field.gather_near(nx, ny));
        assert!(!field.gather_near(nx, ny));
    }

    #[test]
    fn different_night_cycles_produce_different_positions() {
        let mut field = StarCrystalField::new();
        field.spawn_for_night();
        let pos1: Vec<_> = field.active_nodes();
        field.clear();
        field.spawn_for_night();
        let pos2: Vec<_> = field.active_nodes();
        // 至少有一個節點位置不同
        assert!(pos1.iter().zip(pos2.iter()).any(|(a, b)| a != b));
    }

    #[test]
    fn scatter_positions_within_radius() {
        let mut field = StarCrystalField::new();
        field.spawn_for_night();
        for n in &field.nodes {
            let dist = (n.x * n.x + n.y * n.y).sqrt();
            assert!(dist <= SCATTER_RADIUS * 1.42, // 允許對角線誤差
                "節點 ({},{}) 超出散布半徑 {}", n.x, n.y, SCATTER_RADIUS);
        }
    }
}
