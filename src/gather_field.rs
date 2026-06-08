//! 採集節點的世界佈置與互動（Phase 1-A 採集動作的純邏輯地基之二）。
//!
//! `gather.rs` 解了「單一節點怎麼被採、採空怎麼重生」；接線還缺另一半——
//! 「**節點擺在世界哪裡、玩家走近時採到哪一個**」。本層就是那塊純幾何 + 純互動。
//!
//! ③ 無限世界（切片 B）：改為區塊式確定性生成。不再有固定數量的 `NODE_COUNT`，
//! 而是隨玩家探索動態生成區塊內容。

use std::collections::HashMap;
use world_core::{chunk_key, CHUNK_SIZE};

use crate::gather::{NodeKind, ResourceNode};

/// 每區塊平均生成的節點數。
const NODES_PER_CHUNK: usize = 2;

/// 玩家採集的伸手範圍：站在節點這個距離內才採得到（比一格略大，走近即可）。
pub const GATHER_REACH: f32 = 56.0;

/// 世界裡一個有座標的採集節點。
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedNode {
    /// 唯一 ID，格式為 `(chunk_x, chunk_y, index_in_chunk)`。
    /// 用來在重生或狀態同步時識別同一個節點。
    pub id: (i32, i32, usize),
    /// 世界座標 X。
    pub x: f32,
    /// 世界座標 Y。
    pub y: f32,
    /// 節點本身（耐久 / 重生狀態）。
    pub node: ResourceNode,
    /// 已重生次數。
    pub respawns: u32,
}

/// 散佈在世界裡的一整組採集節點，改為區塊式無限生成。
#[derive(Debug, Clone, PartialEq)]
pub struct NodeField {
    /// 緩存已生成的區塊。
    chunks: HashMap<(i32, i32), Vec<PlacedNode>>,
}

// 整個模組是前置地基，接線輪才有呼叫端，比照 `plots.rs` / `gather.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
impl NodeField {
    /// 建立空的節點欄位。區塊隨玩家探索動態生成。
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
        }
    }

    /// 目前已載入的所有節點（供前端畫、供測試檢視）。
    pub fn nodes(&self) -> Vec<PlacedNode> {
        self.chunks.values().flatten().cloned().collect()
    }

    /// 確保玩家周圍的區塊已生成。
    pub fn ensure_chunks_around(&mut self, px: f32, py: f32, radius: f32) {
        let (cx_min, cy_min) = chunk_key(px - radius, py - radius);
        let (cx_max, cy_max) = chunk_key(px + radius, py + radius);

        for cy in cy_min..=cy_max {
            for cx in cx_min..=cx_max {
                self.chunks.entry((cx, cy)).or_insert_with(|| generate_chunk(cx, cy));
            }
        }
    }

    /// 推進 `dt` 秒：對所有已載入區塊的節點呼叫 `tick`（採空的倒數重生，其餘 no-op）。
    pub fn tick(&mut self, dt: f32) {
        for ((_cx, _cy), nodes) in self.chunks.iter_mut() {
            for placed in nodes.iter_mut() {
                let was_harvestable = placed.node.is_harvestable();
                placed.node.tick(dt);
                if !was_harvestable && placed.node.is_harvestable() {
                    // 剛從採空狀態重生：位置搬遷。
                    placed.respawns = placed.respawns.wrapping_add(1);
                    let (nx, ny) = place_for_id(placed.id, placed.node.kind(), placed.respawns);
                    placed.x = nx;
                    placed.y = ny;
                }
            }
        }
    }

    /// 玩家在 `(px, py)` 採集：在 `GATHER_REACH` 內、仍可採的節點中挑**最近**的採一下。
    pub fn gather_near(&mut self, px: f32, py: f32) -> Option<(NodeKind, u32)> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        
        // 確保目前座標所在的區塊已載入（防禦性）
        self.ensure_chunks_around(px, py, GATHER_REACH);

        let (cx, cy) = chunk_key(px, py);
        let mut best: Option<( (i32, i32, usize), f32 )> = None;
        let reach_sq = GATHER_REACH * GATHER_REACH;

        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(nodes) = self.chunks.get(&(cx + dx, cy + dy)) {
                    for placed in nodes {
                        if !placed.node.is_harvestable() {
                            continue;
                        }
                        let dist_x = placed.x - px;
                        let dist_y = placed.y - py;
                        let dist_sq = dist_x * dist_x + dist_y * dist_y;
                        if dist_sq <= reach_sq {
                            if best.as_ref().map_or(true, |(_, b)| dist_sq < *b) {
                                best = Some((placed.id, dist_sq));
                            }
                        }
                    }
                }
            }
        }

        if let Some((id, _)) = best {
            let chunk_pos = (id.0, id.1);
            let nodes = self.chunks.get_mut(&chunk_pos).unwrap();
            let placed = nodes.iter_mut().find(|n| n.id == id).unwrap();
            let kind = placed.node.kind();
            placed.node.gather().map(|amount| (kind, amount))
        } else {
            None
        }
    }

    /// 暫時保留對舊存檔的相容性。
    pub fn from_saved(saved: Vec<ResourceNode>) -> Option<Self> {
        let mut field = Self::new();
        for (i, node) in saved.into_iter().enumerate() {
            if !node.is_loadable() { continue; }
            let id = (0, 0, i);
            let (x, y) = place_for_id(id, node.kind(), 0);
            let cx_cy = chunk_key(x, y);
            field.chunks.entry(cx_cy).or_default().push(PlacedNode {
                id,
                x,
                y,
                node,
                respawns: 0,
            });
        }
        Some(field)
    }
}

impl Default for NodeField {
    fn default() -> Self {
        Self::new()
    }
}

/// 依生態域決定資源種類：森林/草地多木材，岩地富礦石，沙漠散落石塊。
/// `slot` 用於同一生態域內的變化（如岩地交替礦石與普通石塊）。
fn kind_for_biome(biome: world_core::Biome, slot: u64) -> NodeKind {
    use world_core::Biome;
    match biome {
        Biome::Forest | Biome::Meadow => NodeKind::Tree,
        Biome::Rocky => {
            // 岩地交替：一半乙太礦、一半普通石塊，探索時才有驚喜感
            if slot % 2 == 0 { NodeKind::EtherOre } else { NodeKind::Rock }
        }
        Biome::Sand | Biome::Water => NodeKind::Rock,
    }
}

/// 區塊內節點生成：先找非水域落點，再依生態域決定種類。
fn generate_chunk(cx: i32, cy: i32) -> Vec<PlacedNode> {
    let mut nodes = Vec::new();
    for i in 0..NODES_PER_CHUNK {
        let id = (cx, cy, i);
        // 位置先行：找非水域落點（最多 41 次），再看那裡的生態域決定種類
        let mut pos = None;
        for salt in 0u32..=40 {
            let (x, y) = scatter_position(id, 0, salt);
            if world_core::biome_at(x as f64, y as f64) != world_core::Biome::Water {
                pos = Some((x, y));
                break;
            }
        }
        let (x, y) = pos.unwrap_or_else(|| scatter_position(id, 0, 0)); // 防呆
        let biome = world_core::biome_at(x as f64, y as f64);
        let kind = kind_for_biome(biome, i as u64);
        nodes.push(PlacedNode {
            id,
            x,
            y,
            node: ResourceNode::new(kind),
            respawns: 0,
        });
    }
    nodes
}

/// 確定性位置生成。
fn place_for_id(id: (i32, i32, usize), kind: NodeKind, respawns: u32) -> (f32, f32) {
    let mut salt = 0;
    loop {
        let (x, y) = scatter_position(id, respawns, salt);
        let biome = world_core::biome_at(x as f64, y as f64);
        if biome_suits_kind(biome, kind) {
            return (x, y);
        }
        salt += 1;
        if salt > 20 { return (x, y); } // 防呆
    }
}

fn scatter_position(id: (i32, i32, usize), respawns: u32, salt: u32) -> (f32, f32) {
    let mut s = (id.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    s = s.wrapping_add((id.1 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    s = s.wrapping_add(id.2 as u64);
    s = s.wrapping_add(respawns as u64);
    s = s.wrapping_add((salt as u64).wrapping_mul(0x94D0_49BB_1331_11EB));
    
    let x = (id.0 as f32) * CHUNK_SIZE + hash01(s) * CHUNK_SIZE;
    let y = (id.1 as f32) * CHUNK_SIZE + hash01(s.wrapping_add(1)) * CHUNK_SIZE;
    (x, y)
}

fn biome_suits_kind(biome: world_core::Biome, kind: NodeKind) -> bool {
    use world_core::Biome;
    match (kind, biome) {
        (_, Biome::Water) => false,
        (NodeKind::Tree, Biome::Forest | Biome::Meadow) => true,
        (NodeKind::Rock, Biome::Rocky | Biome::Sand) => true,
        (NodeKind::EtherOre, Biome::Rocky) => true,
        _ => false,
    }
}

fn hash01(n: u64) -> f32 {
    let mut z = n.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f32 / (1u64 << 53) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_field_is_empty() {
        let f = NodeField::new();
        assert_eq!(f.nodes().len(), 0);
    }

    #[test]
    fn ensure_chunks_generates_nodes() {
        let mut f = NodeField::new();
        f.ensure_chunks_around(0.0, 0.0, 100.0);
        assert!(f.nodes().len() >= NODES_PER_CHUNK);
    }

    #[test]
    fn placement_is_deterministic() {
        let mut a = NodeField::new();
        a.ensure_chunks_around(1000.0, 1000.0, 100.0);
        let mut b = NodeField::new();
        b.ensure_chunks_around(1000.0, 1000.0, 100.0);
        assert_eq!(a, b);
    }

    #[test]
    fn gather_near_picks_correct_node() {
        let mut f = NodeField::new();
        f.ensure_chunks_around(0.0, 0.0, 100.0);
        let target = f.nodes()[0].clone();
        let got = f.gather_near(target.x, target.y);
        assert!(got.is_some());
        assert_eq!(got.unwrap().0, target.node.kind());
    }

    #[test]
    fn node_respawns_and_moves() {
        let mut f = NodeField::new();
        f.ensure_chunks_around(0.0, 0.0, 100.0);
        let target = f.nodes()[0].clone();
        let kind = target.node.kind();
        
        // 採空
        let px = target.x;
        let py = target.y;
        for _ in 0..kind.max_durability() {
            f.gather_near(px, py).unwrap();
        }
        
        // 驗證已採空
        assert!(!f.nodes().iter().find(|n| n.id == target.id).unwrap().node.is_harvestable());
        
        // 推進時間直到重生
        f.tick(kind.respawn_secs() + 1.0);
        
        // 驗證已重生且位置改變 (或至少 respawns 增加)
        let updated = f.nodes().into_iter().find(|n| n.id == target.id).unwrap();
        assert!(updated.node.is_harvestable());
        assert_eq!(updated.respawns, 1);
    }
}
