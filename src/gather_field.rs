//! 採集節點的世界佈置與互動（Phase 1-A 採集動作的純邏輯地基之二）。
//!
//! `gather.rs` 解了「單一節點怎麼被採、採空怎麼重生」；接線還缺另一半——
//! 「**節點擺在世界哪裡、玩家走近時採到哪一個**」。本層就是那塊純幾何 + 純互動：
//! 一組散佈在世界裡的 `PlacedNode`（座標 + `ResourceNode`），加上
//!   - `tick(dt)`：一次推進全部節點的重生倒數（遊戲迴圈每 tick 呼叫）。
//!   - `gather_near(x, y)`：玩家在 `(x,y)` 採集，挑最近、仍可採、在採集範圍內的那個採一下。
//!
//! 延續 `plots.rs` / `field.rs` / `daynight.rs` 的前置慣例：純函式、無 IO、不碰
//! WebSocket / 遊戲迴圈 / 廣播 shape，標 `allow(dead_code)`，接線輪（AppState 持有
//! `NodeField`、遊戲迴圈 tick、ws 採集進背包、前端畫節點）才有呼叫端。
//!
//! 佈置刻意做成「環繞家園的曠野」：世界中央留一塊空地給家園農莊（`plots.rs` 的地塊
//! 從中心往外排），資源節點散在外圈一圈——出門到曠野採集、採完換地方或等重生，
//! 與療癒種田（待在家照顧自己的地）形成「居家 vs 外出」兩種節奏。佈置由節點序號
//! 決定（確定性雜湊，不靠亂數 / 時鐘），所以同一份存檔重啟後節點落在同一處。

use crate::gather::{NodeKind, ResourceNode};
use crate::state::{WORLD_HEIGHT, WORLD_WIDTH};

/// 散佈在世界裡的節點總數（三種輪流分配，故會是 3 的倍數較均勻）。大世界放大後一併增量,
/// 讓散開的玩家走不遠就能遇到可採的東西。
const NODE_COUNT: usize = 60;

/// 節點距世界邊界至少留這麼多，免得卡在邊上採不到。
const EDGE_MARGIN: f32 = 60.0;

/// 玩家採集的伸手範圍：站在節點這個距離內才採得到（比一格略大，走近即可）。
pub const GATHER_REACH: f32 = 56.0;

/// 世界裡一個有座標的採集節點。
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedNode {
    /// 世界座標 X。
    pub x: f32,
    /// 世界座標 Y。
    pub y: f32,
    /// 節點本身（耐久 / 重生狀態）。
    pub node: ResourceNode,
    /// 已重生次數。每次採空→重生時 +1，並用它當搬遷種子讓節點換到新位置
    /// （像麥塊的資源在別處重新長出來，而非原地補滿）。初次佈置為 0。
    /// 不入存檔（位置一律由序號重推，重啟回到初始佈置），故不參與序列化。
    pub respawns: u32,
}

/// 散佈在世界裡的一整組採集節點。
#[derive(Debug, Clone, PartialEq)]
pub struct NodeField {
    nodes: Vec<PlacedNode>,
}

// 整個模組是前置地基，接線輪才有呼叫端，比照 `plots.rs` / `gather.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
impl NodeField {
    /// 用確定性佈置生出一組全新（滿耐久）的節點。
    pub fn new() -> Self {
        let nodes = (0..NODE_COUNT)
            .map(|i| {
                let kind = kind_for(i);
                let (x, y) = place_for_kind(i, kind, 0);
                PlacedNode {
                    x,
                    y,
                    node: ResourceNode::new(kind),
                    respawns: 0,
                }
            })
            .collect();
        Self { nodes }
    }

    /// 目前的節點（供前端畫、供測試檢視）。
    pub fn nodes(&self) -> &[PlacedNode] {
        &self.nodes
    }

    /// 推進 `dt` 秒：對全部節點呼叫 `tick`（採空的倒數重生，其餘 no-op）。
    ///
    /// **採空 → 重生的那一刻，節點會搬到新位置**（像麥塊的資源在別處重新長出來），而不是
    /// 原地補滿——資源因此在世界各處游移，採完一處得去別處找，世界更動態。搬遷種子隨重生
    /// 次數遞增（每次落在不同點）、仍由序號決定大致散佈。位置不入存檔（重啟回初始佈置）。
    pub fn tick(&mut self, dt: f32) {
        for (i, placed) in self.nodes.iter_mut().enumerate() {
            let was_harvestable = placed.node.is_harvestable();
            placed.node.tick(dt);
            if !was_harvestable && placed.node.is_harvestable() {
                // 剛從採空狀態重生：換一個新位置長出來。
                placed.respawns = placed.respawns.wrapping_add(1);
                let (x, y) = place_for_kind(i, placed.node.kind(), placed.respawns);
                placed.x = x;
                placed.y = y;
            }
        }
    }

    /// 玩家在 `(px, py)` 採集：在 `GATHER_REACH` 內、仍可採的節點中挑**最近**的採一下，
    /// 回傳 `(種類, 產出量)`；範圍內沒有可採節點時回 `None`（比照 `Field::interact`
    /// 由伺服器權威判定，客戶端只送意圖）。
    pub fn gather_near(&mut self, px: f32, py: f32) -> Option<(NodeKind, u32)> {
        // 非有限座標一律視為採不到（延續 `cell_at` 的載入防線脈絡）。
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        let reach_sq = GATHER_REACH * GATHER_REACH;
        let mut best: Option<(usize, f32)> = None;
        for (i, placed) in self.nodes.iter().enumerate() {
            if !placed.node.is_harvestable() {
                continue;
            }
            let dx = placed.x - px;
            let dy = placed.y - py;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= reach_sq && best.is_none_or(|(_, b)| dist_sq < b) {
                best = Some((i, dist_sq));
            }
        }
        let (idx, _) = best?;
        let kind = self.nodes[idx].node.kind();
        self.nodes[idx].node.gather().map(|amount| (kind, amount))
    }

    /// 載入入口（接 0-E 從存檔還原節點狀態用）：佈置（座標）一律由序號重新推導，
    /// 只有「耐久 / 重生倒數」這組會變的狀態取自存檔。延續 `field::from_tiles` 的
    /// 載入時驗證——存檔節點數必須與目前佈置一致、且每個都 `is_loadable`，否則整組
    /// 拒收回 `None`，呼叫端退回 `NodeField::new()`（全新一組）。
    pub fn from_saved(saved: Vec<ResourceNode>) -> Option<Self> {
        if saved.len() != NODE_COUNT {
            return None;
        }
        let mut nodes = Vec::with_capacity(NODE_COUNT);
        for (i, node) in saved.into_iter().enumerate() {
            // 種類也得對齊佈置：存檔種類若和序號推導的不符，視為壞檔。
            if node.kind() != kind_for(i) || !node.is_loadable() {
                return None;
            }
            let (x, y) = place_for_kind(i, node.kind(), 0);
            nodes.push(PlacedNode {
                x,
                y,
                node,
                respawns: 0,
            });
        }
        Some(Self { nodes })
    }
}

impl Default for NodeField {
    fn default() -> Self {
        Self::new()
    }
}

/// 第 `i` 個節點的種類：三種輪流分配，數量大致均衡。
fn kind_for(i: usize) -> NodeKind {
    match i % 3 {
        0 => NodeKind::Tree,
        1 => NodeKind::Rock,
        _ => NodeKind::EtherOre,
    }
}

/// 第 `i` 個節點、第 `n` 次重生後的世界座標：撒滿整張圖，夾進世界邊界內。
/// `n == 0` 刻意維持**原本的序號佈置**（確定性、重啟一致、既有散開性質不變）；
/// `n > 0` 把重生次數攪進種子，每次重生落在不同新點——資源因此在世界各處游移生長。
fn scatter_position_n(i: usize, n: u32) -> (f32, f32) {
    // n=0 用原種子（i 與 2i+1）保證初始佈置與重生搬遷前完全一致；n>0 才混入重生次數。
    let (sx, sy) = if n == 0 {
        (i as u64, (i as u64).wrapping_mul(2).wrapping_add(1))
    } else {
        let base = (i as u64).wrapping_add((n as u64).wrapping_mul(0x1F12_3BB5_9E37_79B1));
        (base, base.wrapping_mul(2).wrapping_add(1))
    };
    let x = EDGE_MARGIN + hash01(sx) * (WORLD_WIDTH - 2.0 * EDGE_MARGIN);
    let y = EDGE_MARGIN + hash01(sy) * (WORLD_HEIGHT - 2.0 * EDGE_MARGIN);
    (
        x.clamp(EDGE_MARGIN, WORLD_WIDTH - EDGE_MARGIN),
        y.clamp(EDGE_MARGIN, WORLD_HEIGHT - EDGE_MARGIN),
    )
}

/// 某生態域適不適合長這種資源(生態域決定內容):木在森林/草原、石在岩地/沙地、乙太礦在岩地深處;
/// **水域不長任何資源**。生態域由 `world_core::biome_at` 判定——與前端畫的地貌同一份噪聲、逐位元一致,
/// 所以「樹長在前端畫成森林的地」這件事前後端必然對齊。
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

/// 第 `i` 個節點(種類 `kind`、第 `salt` 次擺放)該落在哪:在一串候選座標裡挑「生態域吻合且非水」
/// 的第一個 → 樹進森林、礦進岩地、水域空。找不到吻合就退而求其次(只求非水),再不行用原始座標。
/// 確定性(同 i/kind/salt 永遠同結果):`salt` 給不同擺放批次(初始=0、每次重生遞增)不同候選序列。
fn place_for_kind(i: usize, kind: NodeKind, salt: u32) -> (f32, f32) {
    let candidate = |k: u32| scatter_position_n(i, salt.wrapping_mul(64).wrapping_add(k));
    // 1) 生態域吻合 + 非水
    for k in 0..24 {
        let (x, y) = candidate(k);
        if biome_suits_kind(world_core::biome_at(x as f64, y as f64), kind) {
            return (x, y);
        }
    }
    // 2) 退一步:只要非水
    for k in 24..48 {
        let (x, y) = candidate(k);
        if world_core::biome_at(x as f64, y as f64) != world_core::Biome::Water {
            return (x, y);
        }
    }
    // 3) 最後手段:原始座標(幾乎不會走到)
    scatter_position_n(i, salt)
}

/// 確定性雜湊：把序號攪成 `[0, 1)` 的浮點（splitmix64 風格），佈置用。
/// 不引入亂數相依、不碰時鐘，純函式，可重現。
fn hash01(n: u64) -> f32 {
    let mut z = n.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // 取高 53 位映到 [0,1)，與 f64 尾數對齊避免取捨偏差。
    (z >> 11) as f32 / (1u64 << 53) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_field_has_full_count_all_harvestable() {
        let f = NodeField::new();
        assert_eq!(f.nodes().len(), NODE_COUNT);
        assert!(f.nodes().iter().all(|p| p.node.is_harvestable()));
    }

    #[test]
    fn placement_is_deterministic() {
        // 兩次建構座標完全一致（確定性，不靠亂數 / 時鐘）。
        let a = NodeField::new();
        let b = NodeField::new();
        assert_eq!(a, b);
    }

    #[test]
    fn nodes_stay_in_world_and_spread_wide() {
        let nodes = NodeField::new();
        let ns = nodes.nodes();
        let (mut minx, mut maxx, mut miny, mut maxy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in ns {
            // 在世界內、留邊距。
            assert!((EDGE_MARGIN..=WORLD_WIDTH - EDGE_MARGIN).contains(&p.x));
            assert!((EDGE_MARGIN..=WORLD_HEIGHT - EDGE_MARGIN).contains(&p.y));
            minx = minx.min(p.x);
            maxx = maxx.max(p.x);
            miny = miny.min(p.y);
            maxy = maxy.max(p.y);
        }
        // 撒滿大圖:節點的橫/縱跨幅應佔世界一大半以上(不再擠在中央一圈)。
        assert!(maxx - minx > WORLD_WIDTH * 0.5, "節點橫向沒散開: {}", maxx - minx);
        assert!(maxy - miny > WORLD_HEIGHT * 0.5, "節點縱向沒散開: {}", maxy - miny);
    }

    #[test]
    fn all_three_kinds_present() {
        let f = NodeField::new();
        let kinds: Vec<NodeKind> = f.nodes().iter().map(|p| p.node.kind()).collect();
        assert!(kinds.contains(&NodeKind::Tree));
        assert!(kinds.contains(&NodeKind::Rock));
        assert!(kinds.contains(&NodeKind::EtherOre));
    }

    #[test]
    fn gather_near_picks_a_node_in_reach_and_yields() {
        let mut f = NodeField::new();
        let target = f.nodes()[0].clone();
        let before = target.node.remaining();
        // 站在第一個節點正上方採集。
        let got = f.gather_near(target.x, target.y);
        assert!(got.is_some());
        let (kind, amount) = got.unwrap();
        assert_eq!(kind, target.node.kind());
        assert_eq!(amount, kind.yield_per_gather());
        assert_eq!(f.nodes()[0].node.remaining(), before - 1);
    }

    #[test]
    fn gather_near_returns_none_when_out_of_reach() {
        let mut f = NodeField::new();
        let p = f.nodes()[0].clone();
        // 離最近節點遠遠的位置（往反方向偏 reach 的好幾倍）。
        let far_x = p.x + GATHER_REACH * 10.0;
        let got = f.gather_near(far_x, p.y);
        // 偏遠到不該採到第一個；但理論上可能剛好靠近別的節點，
        // 故只斷言「不在 reach 內的座標不會採到那個節點」這個更穩的性質：
        // 用一個一定在世界外的座標確保 None。
        assert!(f.gather_near(WORLD_WIDTH + 5000.0, WORLD_HEIGHT + 5000.0).is_none());
        let _ = got; // 上面 far_x 視佈置而定，不硬斷言
    }

    #[test]
    fn gather_near_rejects_non_finite_coords() {
        let mut f = NodeField::new();
        assert!(f.gather_near(f32::NAN, 100.0).is_none());
        assert!(f.gather_near(100.0, f32::INFINITY).is_none());
    }

    #[test]
    fn depleted_node_is_skipped_then_respawns_via_tick() {
        let mut f = NodeField::new();
        let p = f.nodes()[0].clone();
        let kind = p.node.kind();
        // 把第一個節點採空。
        for _ in 0..kind.max_durability() {
            assert!(f.gather_near(p.x, p.y).is_some());
        }
        // 採空後，站原地再採——若附近沒別的可採節點則回 None；
        // 至少第一個節點本身已採空、remaining 為 0。
        assert!(f.nodes()[0].node.is_depleted());
        // 推進到重生時間，補滿可採。
        f.tick(kind.respawn_secs());
        assert!(f.nodes()[0].node.is_harvestable());
    }

    #[test]
    fn from_saved_round_trips_and_validates() {
        let f = NodeField::new();
        // 拆出節點狀態（模擬存檔），原樣還原應一致。
        let saved: Vec<ResourceNode> = f.nodes().iter().map(|p| p.node.clone()).collect();
        let restored = NodeField::from_saved(saved).expect("正常存檔該還原");
        assert_eq!(restored, f);
    }

    #[test]
    fn from_saved_rejects_wrong_count() {
        assert!(NodeField::from_saved(vec![]).is_none());
        let too_few: Vec<ResourceNode> =
            (0..NODE_COUNT - 1).map(|i| ResourceNode::new(kind_for(i))).collect();
        assert!(NodeField::from_saved(too_few).is_none());
    }

    #[test]
    fn from_saved_rejects_corrupt_or_mismatched_kind() {
        // 種類和序號推導不符 → 壞檔。
        let mut wrong_kind: Vec<ResourceNode> =
            (0..NODE_COUNT).map(|i| ResourceNode::new(kind_for(i))).collect();
        // 把序號 0（應為 Tree）換成別種。
        wrong_kind[0] = ResourceNode::new(NodeKind::Rock);
        assert!(NodeField::from_saved(wrong_kind).is_none());

        // 含壞值（耐久超上限）的存檔 → 拒收。
        let mut corrupt: Vec<ResourceNode> =
            (0..NODE_COUNT).map(|i| ResourceNode::new(kind_for(i))).collect();
        corrupt[1] = ResourceNode::from_raw(kind_for(1), 999, 0.0);
        assert!(NodeField::from_saved(corrupt).is_none());
    }

    // ── Phase 1 採集「重生換位」（麥塊式資源游移）的測試 ──────────────────────
    // 採空的節點重生時不再原地補滿，而是搬到新點長出來；下面鎖住「真的換了位置」與
    // 「不管搬到哪都還在世界內」這兩條契約。

    #[test]
    fn node_relocates_to_a_new_spot_after_respawn() {
        let mut f = NodeField::new();
        let p = f.nodes()[0].clone();
        let kind = p.node.kind();
        let (old_x, old_y) = (p.x, p.y);
        // 在節點原位把它採空。
        for _ in 0..kind.max_durability() {
            assert!(f.gather_near(old_x, old_y).is_some());
        }
        assert!(f.nodes()[0].node.is_depleted());
        // 推進到重生：節點重生並**搬到新位置**（不在原地補滿）。
        f.tick(kind.respawn_secs());
        assert!(f.nodes()[0].node.is_harvestable());
        let moved = f.nodes()[0].x != old_x || f.nodes()[0].y != old_y;
        assert!(moved, "重生後節點應換到新位置，({old_x},{old_y}) 不該原地不動");
        assert_eq!(f.nodes()[0].respawns, 1, "重生一次，respawns 應為 1");
    }

    #[test]
    fn relocated_nodes_stay_in_world() {
        let mut f = NodeField::new();
        // 反覆「對每個節點原位猛採到空 + 大步推進重生」數輪，逼出多次重生搬遷。
        for _ in 0..5 {
            let positions: Vec<(f32, f32)> = f.nodes().iter().map(|p| (p.x, p.y)).collect();
            for (x, y) in positions {
                for _ in 0..9 {
                    let _ = f.gather_near(x, y); // 採到空（9 > 任一種 max_durability）
                }
            }
            f.tick(1.0e6); // 大步推進，所有採空的都重生（換位）
        }
        // 不管搬到哪，所有節點都該還在世界邊界內。
        for p in f.nodes() {
            assert!(
                (EDGE_MARGIN..=WORLD_WIDTH - EDGE_MARGIN).contains(&p.x),
                "節點 x 出界: {}",
                p.x
            );
            assert!(
                (EDGE_MARGIN..=WORLD_HEIGHT - EDGE_MARGIN).contains(&p.y),
                "節點 y 出界: {}",
                p.y
            );
        }
        // 至少發生過搬遷（有節點 respawns > 0）。
        assert!(
            f.nodes().iter().any(|p| p.respawns > 0),
            "應至少發生一次重生搬遷"
        );
    }

    #[test]
    fn nodes_avoid_water_and_mostly_match_biome() {
        // 生態域決定內容:節點不該長在水上,且多數落在吻合的生態域(樹在森林/草原、礦在岩地)。
        // 少數因吻合生態域附近難找而退求非水,允許;但水上=0、吻合應過半。
        let f = NodeField::new();
        let mut matched = 0usize;
        for p in f.nodes() {
            let b = world_core::biome_at(p.x as f64, p.y as f64);
            assert_ne!(
                b,
                world_core::Biome::Water,
                "節點長在水上了:({},{})",
                p.x,
                p.y
            );
            if biome_suits_kind(b, p.node.kind()) {
                matched += 1;
            }
        }
        let total = f.nodes().len();
        assert!(
            matched * 2 >= total,
            "吻合生態域的節點太少:{matched}/{total}"
        );
    }
}
