//! 採集節點模型（Phase 1-A 採集動作的純邏輯地基）。
//!
//! 這層只管「一個採集節點怎麼被採、採空後怎麼重生」，是純資料 + 純函式，無 IO、
//! 不碰 WebSocket / 遊戲迴圈，便於自動測試。延續 `crops.rs` / `field.rs` /
//! `daynight.rs` / `plots.rs` 的前置慣例：純邏輯先落地、標 `allow(dead_code)`，
//! 接線輪（世界撒佈節點、走近按鍵採集進背包、遊戲迴圈每 tick 推進重生）才有呼叫端。
//!
//! 之後接上：
//!   - 世界：在地圖上撒佈若干 `ResourceNode`（樹 / 石 / 乙太礦）。
//!   - ws：玩家走近按鍵 → `gather()` → 把產出加進背包（Phase 1-B）。
//!   - 遊戲迴圈：每 tick 對採空的節點呼叫 `tick(dt)` 倒數重生。
//!   - 持久化（接 0-E）：把節點狀態序列化（載入時走 `is_loadable` 驗證）。
//!
//! 採集迴圈刻意做成「節點有耐久、採空要等重生」：每採一下扣一點耐久、給固定產出，
//! 耐久歸零即採空、進入重生倒數，倒數到了補滿耐久再次可採——資源不是無限白拿，
//! 採完一處得換地方或等它長回來，給世界一點節奏。

use serde::{Deserialize, Serialize};

/// 採集節點的種類。種類決定產出什麼資源、耐久多少、重生多久。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// 樹：採集得木材。
    Tree,
    /// 石：採集得礦石。
    Rock,
    /// 乙太礦：採集得乙太（療癒種田之外，採集也是乙太來源）。
    EtherOre,
}

// 整個模組是前置地基：接線輪（世界撒佈節點、ws 採集、遊戲迴圈推進重生）才有呼叫端，
// 在此之前公開項目皆無外部呼叫，比照 `plots.rs` / `plot_registry.rs` 逐項標 `allow(dead_code)`。
#[allow(dead_code)]
impl NodeKind {
    /// 此種類滿耐久時可採的次數。採空（耐久歸零）後進入重生。
    pub fn max_durability(self) -> u32 {
        match self {
            NodeKind::Tree => 5,
            NodeKind::Rock => 4,
            NodeKind::EtherOre => 3,
        }
    }

    /// 每採一下得到的資源量。
    pub fn yield_per_gather(self) -> u32 {
        match self {
            NodeKind::Tree => 1,
            NodeKind::Rock => 1,
            // 乙太礦耐久低、但每下產出多一點，貼合「稀有資源」的手感。
            NodeKind::EtherOre => 2,
        }
    }

    /// 採空後到補滿耐久所需的重生秒數。
    pub fn respawn_secs(self) -> f32 {
        match self {
            NodeKind::Tree => 30.0,
            NodeKind::Rock => 45.0,
            NodeKind::EtherOre => 60.0,
        }
    }
}

/// 世界裡一個可重複採集的資源節點。
///
/// 狀態只有「剩餘耐久」與「重生倒數」兩個欄位，階段（可採／採空）皆由耐久推導，
/// 維持單一真實來源——比照 `Crop` 以內部 `growth`/`moisture` 推導階段的做法。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceNode {
    /// 節點種類（決定產出 / 耐久 / 重生時間）。
    kind: NodeKind,
    /// 剩餘耐久（可再採的次數）。歸零＝採空。
    remaining: u32,
    /// 重生倒數（秒）。只有採空（`remaining == 0`）時才有意義；倒數到 0 補滿耐久。
    respawn_timer: f32,
}

#[allow(dead_code)] // 同上：前置地基，接線輪才有呼叫端。
impl ResourceNode {
    /// 生出一個滿耐久、可立即採的新節點。
    pub fn new(kind: NodeKind) -> Self {
        Self {
            kind,
            remaining: kind.max_durability(),
            respawn_timer: 0.0,
        }
    }

    /// 節點種類。
    pub fn kind(&self) -> NodeKind {
        self.kind
    }

    /// 剩餘耐久。
    pub fn remaining(&self) -> u32 {
        self.remaining
    }

    /// 是否已採空（需等重生）。
    pub fn is_depleted(&self) -> bool {
        self.remaining == 0
    }

    /// 是否還能採。
    pub fn is_harvestable(&self) -> bool {
        self.remaining > 0
    }

    /// 採一下。還有耐久就扣 1 並回傳產出量；採空時回 `None`、不改變狀態。
    /// 扣到 0 的那一下會啟動重生倒數。抽成回 `Option` 比照 `Crop::harvest`。
    pub fn gather(&mut self) -> Option<u32> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        if self.remaining == 0 {
            // 採空，開始倒數重生。
            self.respawn_timer = self.kind.respawn_secs();
        }
        Some(self.kind.yield_per_gather())
    }

    /// 推進 `dt` 秒。只有採空的節點在倒數；倒數到 0 補滿耐久、再次可採。
    /// 未採空或非正 `dt` 皆為 no-op（比照 `Crop::grow` 擋非正 dt）。
    pub fn tick(&mut self, dt: f32) {
        if self.remaining > 0 || dt <= 0.0 {
            return;
        }
        self.respawn_timer -= dt;
        if self.respawn_timer <= 0.0 {
            self.remaining = self.kind.max_durability();
            self.respawn_timer = 0.0;
        }
    }

    /// 從存檔載入的值是否「健全」：耐久不超過該種類上限、重生倒數有限且非負。
    /// 這是與調校常數無關的最小不變式——正常流程（`new` 滿耐久、`gather` 只遞減、
    /// `tick` 倒數一律夾在 `>= 0`）絕不會產生界外耐久或 `NaN`/`Inf`/負倒數，所以這些
    /// 只會來自壞檔或被竄改的存檔。`remaining` 是 `u32`、型別本身就擋掉 `NaN`/負值，
    /// 故只需驗上界。延續 `crops::is_loadable` / `field::from_tiles` 的載入時驗證脈絡；
    /// 接 0-E 載入路徑時，連同本 impl 區塊的 `allow(dead_code)` 一併移除。
    pub fn is_loadable(&self) -> bool {
        self.respawn_timer.is_finite()
            && self.respawn_timer >= 0.0
            && self.remaining <= self.kind.max_durability()
    }

    /// 測試用：直接組出指定狀態（含壞值）的節點，驗證載入防線。
    #[cfg(test)]
    pub fn from_raw(kind: NodeKind, remaining: u32, respawn_timer: f32) -> Self {
        Self {
            kind,
            remaining,
            respawn_timer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [NodeKind; 3] = [NodeKind::Tree, NodeKind::Rock, NodeKind::EtherOre];

    #[test]
    fn new_node_is_full_and_harvestable() {
        for kind in KINDS {
            let n = ResourceNode::new(kind);
            assert_eq!(n.remaining(), kind.max_durability());
            assert!(n.is_harvestable());
            assert!(!n.is_depleted());
        }
    }

    #[test]
    fn gather_yields_resource_and_decrements_durability() {
        let mut n = ResourceNode::new(NodeKind::Tree);
        assert_eq!(n.gather(), Some(NodeKind::Tree.yield_per_gather()));
        assert_eq!(n.remaining(), NodeKind::Tree.max_durability() - 1);
    }

    #[test]
    fn gathering_to_zero_depletes_and_starts_respawn() {
        let mut n = ResourceNode::new(NodeKind::Rock);
        let max = NodeKind::Rock.max_durability();
        for _ in 0..max {
            assert!(n.gather().is_some());
        }
        assert!(n.is_depleted());
        assert!(!n.is_harvestable());
    }

    #[test]
    fn gathering_a_depleted_node_yields_nothing() {
        let mut n = ResourceNode::new(NodeKind::EtherOre);
        for _ in 0..NodeKind::EtherOre.max_durability() {
            n.gather();
        }
        let depleted = n.clone();
        assert_eq!(n.gather(), None);
        // 採空後再採不改變狀態。
        assert_eq!(n, depleted);
    }

    #[test]
    fn depleted_node_respawns_after_timer() {
        let mut n = ResourceNode::new(NodeKind::Tree);
        for _ in 0..NodeKind::Tree.max_durability() {
            n.gather();
        }
        assert!(n.is_depleted());
        // 還沒到重生時間，仍採空。
        n.tick(NodeKind::Tree.respawn_secs() - 1.0);
        assert!(n.is_depleted());
        // 補足剩餘時間，補滿耐久、再次可採。
        n.tick(1.0);
        assert!(n.is_harvestable());
        assert_eq!(n.remaining(), NodeKind::Tree.max_durability());
    }

    #[test]
    fn tick_on_harvestable_node_is_noop() {
        let mut n = ResourceNode::new(NodeKind::Rock);
        let before = n.clone();
        n.tick(100.0);
        assert_eq!(n, before);
    }

    #[test]
    fn zero_or_negative_dt_is_noop() {
        let mut n = ResourceNode::new(NodeKind::EtherOre);
        for _ in 0..NodeKind::EtherOre.max_durability() {
            n.gather();
        }
        let depleted = n.clone();
        n.tick(0.0);
        assert_eq!(n, depleted);
        n.tick(-5.0);
        assert_eq!(n, depleted);
    }

    #[test]
    fn full_cycle_gather_deplete_respawn_gather_again() {
        let mut n = ResourceNode::new(NodeKind::Tree);
        // 採到空。
        for _ in 0..NodeKind::Tree.max_durability() {
            assert!(n.gather().is_some());
        }
        assert!(n.is_depleted());
        // 一次大步推過重生時間。
        n.tick(NodeKind::Tree.respawn_secs());
        assert!(n.is_harvestable());
        // 重生後又能再採一輪。
        assert_eq!(n.gather(), Some(NodeKind::Tree.yield_per_gather()));
    }

    #[test]
    fn is_loadable_accepts_normal_and_rejects_corrupt() {
        // 正常流程產出的狀態都該可載入。
        assert!(ResourceNode::new(NodeKind::Tree).is_loadable());
        let mut n = ResourceNode::new(NodeKind::Rock);
        for _ in 0..NodeKind::Rock.max_durability() {
            n.gather();
        }
        assert!(n.is_loadable()); // 採空且帶重生倒數，仍健全
        // 壞值：耐久超過上限、NaN / Inf / 負重生倒數。
        assert!(!ResourceNode::from_raw(NodeKind::EtherOre, 99, 0.0).is_loadable());
        assert!(!ResourceNode::from_raw(NodeKind::Tree, 0, f32::NAN).is_loadable());
        assert!(!ResourceNode::from_raw(NodeKind::Tree, 0, f32::INFINITY).is_loadable());
        assert!(!ResourceNode::from_raw(NodeKind::Tree, 0, -1.0).is_loadable());
    }

    #[test]
    fn serde_round_trip_preserves_state() {
        let mut n = ResourceNode::new(NodeKind::EtherOre);
        n.gather(); // 採一下，留個半採狀態
        let json = serde_json::to_string(&n).unwrap();
        let back: ResourceNode = serde_json::from_str(&json).unwrap();
        assert_eq!(n, back);
    }
}
