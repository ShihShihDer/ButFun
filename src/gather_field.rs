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

/// 騎乘巡採（ROADMAP 544）的採集臂伸手範圍（像素）：比徒步 `GATHER_REACH` 略大——
/// 蒸汽載具側掛的採集臂探得遠一點，讓「騎車經過順手撿」順暢些，但仍要貼著節點開過。
pub const VEHICLE_COLLECT_REACH: f32 = 80.0;

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

    /// 只走訪與 `(ax, ay)` 半徑 `radius` AABB 重疊的區塊，回傳半徑內的節點。
    /// 唯讀：不生成新區塊（感知不該觸發世界擴張，與 `nodes()` 同語義、只是空間裁剪）。
    /// M5：取代「`nodes()` clone 全世界再過濾」——成本從 O(已探索世界) 降為 O(半徑內節點)。
    pub fn nodes_near(&self, ax: f32, ay: f32, radius: f32) -> Vec<PlacedNode> {
        if !ax.is_finite() || !ay.is_finite() {
            return Vec::new();
        }
        let (cx_min, cy_min) = chunk_key(ax - radius, ay - radius);
        let (cx_max, cy_max) = chunk_key(ax + radius, ay + radius);
        let r_sq = radius * radius;
        let mut out = Vec::new();
        for cy in cy_min..=cy_max {
            for cx in cx_min..=cx_max {
                if let Some(nodes) = self.chunks.get(&(cx, cy)) {
                    for n in nodes {
                        let dx = n.x - ax;
                        let dy = n.y - ay;
                        if dx * dx + dy * dy <= r_sq {
                            out.push(n.clone());
                        }
                    }
                }
            }
        }
        out
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
        self.gather_within(px, py, GATHER_REACH)
    }

    /// 騎乘巡採（ROADMAP 544）：以載具採集臂範圍 `VEHICLE_COLLECT_REACH` 採最近的可採節點。
    /// 行為與 `gather_near` 完全一致（同一份節點耐久/重生邏輯），只是伸手範圍換成載具的。
    /// **回傳基礎產量**（呼叫端刻意不疊任何採集加成）——騎車是順路撿的便利、不取代手動精採。
    pub fn collect_for_vehicle(&mut self, px: f32, py: f32) -> Option<(NodeKind, u32)> {
        self.gather_within(px, py, VEHICLE_COLLECT_REACH)
    }

    /// 在 `(px, py)` 的 `reach` 範圍內挑**最近**仍可採的節點採一下，回基礎產量。
    /// `gather_near`（徒步）與 `collect_for_vehicle`（騎乘巡採）共用這份唯一實作，
    /// 只差伸手範圍——節點耐久/重生/防隔空採集判定一字不改。
    fn gather_within(&mut self, px: f32, py: f32, reach: f32) -> Option<(NodeKind, u32)> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }

        // 確保目前座標所在的區塊已載入（防禦性）
        self.ensure_chunks_around(px, py, reach);

        let (cx, cy) = chunk_key(px, py);
        // 記住節點「實際被找到的 chunk」——別從 id 推:節點重生會換位,id 內含的原 chunk 可能對不上。
        let mut best: Option<((i32, i32), (i32, i32, usize), f32)> = None; // (找到的 chunk, 節點 id, dist²)
        let reach_sq = reach * reach;

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
                            if best.as_ref().map_or(true, |(_, _, b)| dist_sq < *b) {
                                best = Some(((cx + dx, cy + dy), placed.id, dist_sq));
                            }
                        }
                    }
                }
            }
        }

        if let Some((found_chunk, id, _)) = best {
            // 用實際 chunk 重查;查不到回 None,**絕不 unwrap**(None 一 unwrap 會炸死遊戲迴圈、全服沒快照)。
            if let Some(nodes) = self.chunks.get_mut(&found_chunk) {
                if let Some(placed) = nodes.iter_mut().find(|n| n.id == id) {
                    let kind = placed.node.kind();
                    return placed.node.gather().map(|amount| (kind, amount));
                }
            }
            None
        } else {
            None
        }
    }

    /// 唯讀判定：玩家在 `(px, py)` 的 `GATHER_REACH` 內是否有可採的節點（樹）。
    /// 供伐木連揮小遊戲（ROADMAP 403）開揮前驗格用——只看、不採。
    pub fn has_harvestable_near(&mut self, px: f32, py: f32) -> bool {
        if !px.is_finite() || !py.is_finite() {
            return false;
        }
        self.ensure_chunks_around(px, py, GATHER_REACH);
        let (cx, cy) = chunk_key(px, py);
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
                        if dist_x * dist_x + dist_y * dist_y <= reach_sq {
                            return true;
                        }
                    }
                }
            }
        }
        false
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

/// 依生態域決定資源種類：目前僅保留「樹」作為採集節點，石/礦已統一至地形挖掘。
fn kind_for_biome(biome: world_core::Biome) -> Option<NodeKind> {
    use world_core::Biome;
    match biome {
        Biome::Forest | Biome::Meadow => Some(NodeKind::Tree),
        _ => None,
    }
}

/// 區塊內節點生成：先找非水域且非實心落點，再依生態域決定種類。
fn generate_chunk(cx: i32, cy: i32) -> Vec<PlacedNode> {
    let mut nodes = Vec::new();
    for i in 0..NODES_PER_CHUNK {
        let id = (cx, cy, i);
        // 位置先行：找非水域且非實心（Empty）落點（最多 41 次），再看那裡的生態域決定種類
        let mut pos = None;
        for salt in 0u32..=40 {
            let (x, y) = scatter_position(id, 0, salt);
            let wx = x as f64;
            let wy = y as f64;
            if world_core::biome_at(wx, wy) != world_core::Biome::Water
               && world_core::tile_kind_at(wx, wy) == world_core::TileKind::Empty {
                pos = Some((x, y));
                break;
            }
        }
        let (x, y) = match pos {
            Some(p) => p,
            None => continue, // 找不著合適落點則本槽位不生成（D-1：不准生在實心格）
        };
        let biome = world_core::biome_at(x as f64, y as f64);
        if let Some(kind) = kind_for_biome(biome) {
            nodes.push(PlacedNode {
                id,
                x,
                y,
                node: ResourceNode::new(kind),
                respawns: 0,
            });
        }
    }
    nodes
}

/// 確定性位置生成。
fn place_for_id(id: (i32, i32, usize), kind: NodeKind, respawns: u32) -> (f32, f32) {
    let mut salt = 0;
    loop {
        let (x, y) = scatter_position(id, respawns, salt);
        let wx = x as f64;
        let wy = y as f64;
        let biome = world_core::biome_at(wx, wy);
        if biome_suits_kind(biome, kind) && world_core::tile_kind_at(wx, wy) == world_core::TileKind::Empty {
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
        (NodeKind::Tree, Biome::Forest | Biome::Meadow) => true,
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
        // 確保範圍大一點，增加遇到適合生態域（Meadow/Forest）且非實心的機率。
        f.ensure_chunks_around(0.0, 0.0, 2000.0);
        assert!(f.nodes().len() > 0);
    }

    #[test]
    fn nodes_near_returns_only_in_radius_subset_of_nodes() {
        // M5：nodes_near 只回半徑內節點、且必為 nodes() 的子集（空間裁剪不漏不多）。
        let mut f = NodeField::new();
        f.ensure_chunks_around(0.0, 0.0, 3000.0);
        let all = f.nodes();
        assert!(!all.is_empty());
        let (ax, ay, r) = (all[0].x, all[0].y, 360.0_f32);
        let near = f.nodes_near(ax, ay, r);
        // 至少含中心那個節點本身。
        assert!(near.iter().any(|n| n.id == all[0].id), "應含中心節點");
        // 全部都在半徑內。
        for n in &near {
            let d2 = (n.x - ax).powi(2) + (n.y - ay).powi(2);
            assert!(d2 <= r * r, "nodes_near 回了半徑外的節點");
        }
        // 與「全量過濾」結果一致（同一份幾何裁剪）。
        let brute: std::collections::HashSet<_> = all.iter()
            .filter(|n| (n.x - ax).powi(2) + (n.y - ay).powi(2) <= r * r)
            .map(|n| n.id)
            .collect();
        let got: std::collections::HashSet<_> = near.iter().map(|n| n.id).collect();
        assert_eq!(got, brute, "nodes_near 應等於對全量做同樣半徑過濾");
    }

    #[test]
    fn nodes_near_rejects_nan() {
        let mut f = NodeField::new();
        f.ensure_chunks_around(0.0, 0.0, 1000.0);
        assert!(f.nodes_near(f32::NAN, 0.0, 360.0).is_empty());
    }

    #[test]
    fn placement_is_deterministic() {
        let mut a = NodeField::new();
        a.ensure_chunks_around(1000.0, 1000.0, 1000.0);
        let mut b = NodeField::new();
        b.ensure_chunks_around(1000.0, 1000.0, 1000.0);
        assert_eq!(a, b);
    }

    #[test]
    fn gather_near_picks_correct_node() {
        let mut f = NodeField::new();
        f.ensure_chunks_around(0.0, 0.0, 2000.0);
        let nodes = f.nodes();
        assert!(nodes.len() > 0, "應至少生成一個節點");
        let target = nodes[0].clone();
        let got = f.gather_near(target.x, target.y);
        assert!(got.is_some());
        assert_eq!(got.unwrap().0, target.node.kind());
    }

    #[test]
    fn collect_for_vehicle_picks_node_within_arm_reach() {
        // 騎乘巡採（ROADMAP 544）：採集臂範圍比徒步寬，貼著節點開過就採得到、回基礎產量。
        let mut f = NodeField::new();
        // 多載一圈安全邊界（+CHUNK_SIZE）：`f.gather_near`/`f.collect_for_vehicle` 內部
        // 會再呼叫一次 `ensure_chunks_around(px, py, reach)`——若挑到的 target 貼著原本
        // 2000 半徑的載入邊界，偏移後的採點可能踩出邊界外，觸發**這裡的 `nodes` 快照
        // 從未見過**的新區塊（確定性生成，非隨機，但快照時還沒載入）。那個新區塊若剛好
        // 生了節點，「此距離徒步搆不到」的前提斷言就會隨機失敗——這才是全套件下
        // 約 1/3~1/8 機率炸、單獨跑必過的真正 flaky 成因（先前只解了 HashMap 迭代序，
        // 沒解到「快照邊界外還會再長新區塊」這一半）。多載這圈邊界，確保任何
        // target/px/py 用得到的區塊在快照當下都已載入、快照涵蓋全部。
        f.ensure_chunks_around(0.0, 0.0, 2000.0 + CHUNK_SIZE);
        let d = (GATHER_REACH + VEHICLE_COLLECT_REACH) / 2.0;
        let nodes = f.nodes();
        // `nodes()` 走訪 HashMap、順序不保證（跨執行緒/跨次執行可能不同）——
        // 先前直接拿 `nodes()[0]` 當 target 是 flaky bug：在節點密集處，
        // 隨機選到的 target 偏移後的採點 (target.x+d, target.y) 有機率恰好落進
        // 「另一個」節點的徒步範圍內，導致「此距離徒步搆不到」的前提斷言隨機失敗。
        // 修法：顯式挑一個「孤立」節點——偏移後的採點在 GATHER_REACH 內找不到任何節點，
        // 不依賴 HashMap 迭代順序，讓斷言在任何雜湊種子下都成立；且只從**遠低於**
        // 上方多載邊界的內圈（≤2000）挑，確保 target/px/py 需要的所有區塊都在
        // 快照時已經在 `nodes` 裡，不會漏算快照之後才生成的新區塊。
        let target = nodes
            .iter()
            .filter(|cand| cand.x.abs() <= 2000.0 && cand.y.abs() <= 2000.0)
            .find(|cand| {
                let (px, py) = (cand.x + d, cand.y);
                !nodes.iter().any(|n| {
                    let dx = n.x - px;
                    let dy = n.y - py;
                    dx * dx + dy * dy <= GATHER_REACH * GATHER_REACH
                })
            })
            .cloned()
            .expect("2000 半徑內應能找到至少一個孤立節點");
        // 站在「徒步搆不到、但採集臂搆得到」的距離外側採（GATHER_REACH < d <= VEHICLE_COLLECT_REACH）。
        let (px, py) = (target.x + d, target.y);
        assert!(
            f.gather_near(px, py).is_none(),
            "此距離徒步搆不到"
        );
        let (kind, amount) = f.collect_for_vehicle(px, py).expect("採集臂搆得到");
        // 採到的恰是該種節點的基礎產量（呼叫端刻意不疊任何加成）。
        assert_eq!(amount, kind.yield_per_gather());
    }

    #[test]
    fn node_respawns_and_moves() {
        let mut f = NodeField::new();
        f.ensure_chunks_around(0.0, 0.0, 2000.0);
        let nodes = f.nodes();
        assert!(nodes.len() > 0, "應至少生成一個節點");
        let target = nodes[0].clone();
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
