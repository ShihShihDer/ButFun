//! 乙太方界·植樹造林 v1（ROADMAP 738）——樹苗‧成長‧成林。
//!
//! 天然樹是地形的一部分（`voxel::tree_block_at`，挖了不會長回來），木材因此是**有限資源**。
//! 本模組讓玩家第一次能**親手種樹**：砍天然樹葉有機率掉「樹苗」→ 種在土地上 → 靜候成長 →
//! 一株麥塊風的小樹（樹幹＋樹冠）自己長出來，可再砍伐取木。空地／沙漠因此能被玩家綠化成
//! 自己種的森林——把「採集→種植→成長→收成」的療癒循環第一次接到**可再生的木材**上。
//!
//! **純邏輯層**：`GroveStore`（記錄種下的樹苗＋時間）＋成長計時＋樹形狀純函式，
//! 確定性、無副作用、全可測。鎖 / WS / IO / delta 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。
//!
//! 種植流程：
//!   砍天然樹葉(Leaves=6) →[機率 SAPLING_DROP_CHANCE]→ 樹苗(SAPLING_ID=65)
//!   樹苗種在土地上（草/土/沙/雪/農田土之上）→ Sapling 方塊(65) →[SAPLING_GROW_SECS 後]→
//!   `grown_tree_blocks` 展開的樹幹(Wood=5)＋樹冠(Leaves=6)（比照天然樹形狀）。
//!
//! GroveStore **純記憶體**（與世界 delta / farm store 行為一致：重啟後種下的樹苗重置，
//! 但已長成的樹是 delta 方塊、會持久）。

use std::collections::HashMap;

/// 樹苗物品 / 方塊 id（`Block::Sapling`）。既是背包可持有的物品，也是種下後在世界裡的方塊，
/// 沿用「可放置方塊 item_id == block_id」慣例（如木頭5、玻璃10）。id 65 是烤地薯(64)之後首個空號。
pub const SAPLING_ID: u8 = 65;

/// 樹苗長成大樹所需秒數（~150 秒 = 2.5 分鐘）——比作物慢（馬鈴薯最慢 120s），
/// 呼應「種一棵樹是更長線的投資」的節奏，也讓玩家有時間看著它從一格小苗抽成一株樹。
pub const SAPLING_GROW_SECS: u64 = 150;

/// 砍一格天然樹葉掉出樹苗的機率（~1/3）。樹上葉片多，不必每片都掉、又保證砍幾片就能取得，
/// 讓「發現植樹玩法」的門檻低卻不氾濫。比照垂釣稀有度慣例：機率骰在 `voxel_ws` 呼叫端取真隨機，
/// 這裡只定常數、純函式照樣以固定值驗證。
pub const SAPLING_DROP_CHANCE: f32 = 0.34;

/// 樹苗可種在哪些「土地」方塊之上（草/土/沙/雪/農田土）。純函式，供 `voxel_ws` 種植前驗證。
/// 傳入的是「樹苗腳下那格」的方塊 id。
pub fn is_plantable_ground(block_id: u8) -> bool {
    matches!(
        block_id,
        1  // Grass 草地
        | 2  // Dirt 泥土
        | 4  // Sand 沙地（能在沙漠綠化）
        | 11 // FarmSoil 農田土
        | 55 // Snow 雪原地表
    )
}

/// 一棵長成的樹要放下的方塊種類。讓本模組不必依賴 `voxel::Block`——
/// 由 `voxel_ws` 把 `Trunk`→`Block::Wood`、`Leaf`→`Block::Leaves`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroveBlock {
    /// 樹幹（Wood）。
    Trunk,
    /// 樹冠葉片（Leaves）。
    Leaf,
}

/// 樹苗成熟時，以樹苗所在格 `(x, y, z)` 為底座，展開成一株樹的所有方塊（世界絕對座標）。
/// 形狀刻意鏡像天然樹（`voxel::tree_block_hit`）：樹幹柱 3 格 + top-1/top 兩層 3×3 樹冠環
/// + top+1 十字頂蓋，讓玩家種的樹跟世界原生的樹長得一樣、視覺一致。純函式、確定性、可測。
///
/// 底座 `(x, y, z)`（原本是樹苗方塊）會被樹幹底段覆蓋成 `Trunk`。
pub fn grown_tree_blocks(x: i32, y: i32, z: i32) -> Vec<(i32, i32, i32, GroveBlock)> {
    // 樹幹高度（格）。天然樹 4~6，本 v1 固定 3 格，小而工整、不遮天。
    const TRUNK: i32 = 3;
    let top = y + TRUNK - 1; // 樹幹最高一格的 y（底座 y 起算，含底座共 TRUNK 格）
    let mut out = Vec::new();

    // 樹幹柱：底座 y 起連續 TRUNK 格（含底座，覆蓋樹苗）。
    for ty in y..=top {
        out.push((x, ty, z, GroveBlock::Trunk));
    }
    // 樹冠：top-1、top 兩層 3×3 環（不含樹幹柱本身）。
    for cy in [top - 1, top] {
        for dx in -1..=1 {
            for dz in -1..=1 {
                if dx == 0 && dz == 0 {
                    continue; // 樹幹柱本身
                }
                out.push((x + dx, cy, z + dz, GroveBlock::Leaf));
            }
        }
    }
    // 樹冠頂蓋：top+1 的十字（正上方 + 四鄰）。
    out.push((x, top + 1, z, GroveBlock::Leaf));
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        out.push((x + dx, top + 1, z + dz, GroveBlock::Leaf));
    }
    out
}

/// 一株種下的樹苗記錄。
#[derive(Clone, Debug, PartialEq)]
pub struct Sapling {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 種下的 Unix 秒數（用來判斷是否長成）。
    pub planted_secs: u64,
}

/// 樹苗 store（純記憶體，重啟後重置；已長成的樹是 delta 方塊會持久）。
#[derive(Default)]
pub struct GroveStore {
    saplings: HashMap<(i32, i32, i32), Sapling>,
}

impl GroveStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 種下樹苗：記錄座標 + planted_secs。重複種同格 → 覆蓋（重置計時）。
    pub fn plant(&mut self, x: i32, y: i32, z: i32, now_secs: u64) -> Sapling {
        let s = Sapling { x, y, z, planted_secs: now_secs };
        self.saplings.insert((x, y, z), s.clone());
        s
    }

    /// 移除樹苗記錄（被挖掉 / 長成後清掉）。
    pub fn remove(&mut self, x: i32, y: i32, z: i32) {
        self.saplings.remove(&(x, y, z));
    }

    /// 此座標是否有樹苗記錄。
    pub fn has(&self, x: i32, y: i32, z: i32) -> bool {
        self.saplings.contains_key(&(x, y, z))
    }

    /// 回傳所有已長成（種下滿 SAPLING_GROW_SECS）的樹苗座標。
    pub fn mature_saplings(&self, now_secs: u64) -> Vec<(i32, i32, i32)> {
        self.saplings
            .iter()
            .filter(|(_, s)| now_secs >= s.planted_secs.saturating_add(SAPLING_GROW_SECS))
            .map(|(&coord, _)| coord)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sapling_id_is_after_baked_potato() {
        // 樹苗 id 65 必須落在既有 item id（烤地薯 64）之後、且不撞任何既有方塊 enum（≤59）。
        assert_eq!(SAPLING_ID, 65);
        assert!(SAPLING_ID > 64, "樹苗應排在烤地薯(64)之後");
    }

    #[test]
    fn plant_creates_sapling() {
        let mut g = GroveStore::new();
        g.plant(1, 5, 2, 1000);
        assert!(g.has(1, 5, 2));
        assert!(!g.has(0, 0, 0));
    }

    #[test]
    fn remove_clears_sapling() {
        let mut g = GroveStore::new();
        g.plant(3, 5, 7, 1000);
        g.remove(3, 5, 7);
        assert!(!g.has(3, 5, 7));
    }

    #[test]
    fn immature_before_grow_secs() {
        let mut g = GroveStore::new();
        g.plant(0, 5, 0, 1000);
        // 差 1 秒未達門檻。
        assert!(g.mature_saplings(1000 + SAPLING_GROW_SECS - 1).is_empty());
    }

    #[test]
    fn mature_at_exactly_grow_secs() {
        let mut g = GroveStore::new();
        g.plant(0, 5, 0, 1000);
        let m = g.mature_saplings(1000 + SAPLING_GROW_SECS);
        assert_eq!(m.len(), 1);
        assert!(m.contains(&(0, 5, 0)));
    }

    #[test]
    fn only_mature_saplings_returned() {
        let mut g = GroveStore::new();
        g.plant(0, 5, 0, 1000); // 差不足
        g.plant(1, 5, 0, 1000 - SAPLING_GROW_SECS - 10); // 早就長成
        let m = g.mature_saplings(1000);
        assert_eq!(m.len(), 1);
        assert!(m.contains(&(1, 5, 0)));
        assert!(!m.contains(&(0, 5, 0)));
    }

    #[test]
    fn tree_slower_than_slowest_crop() {
        // 種樹是更長線的投資：應比最慢的作物（馬鈴薯 120s）還慢。
        assert!(SAPLING_GROW_SECS > 120, "樹苗應比最慢作物更慢長成");
    }

    #[test]
    fn plantable_ground_accepts_natural_soil_rejects_others() {
        // 土地類方塊可種。
        for g in [1u8, 2, 4, 11, 55] {
            assert!(is_plantable_ground(g), "{g} 應可種樹苗");
        }
        // 非土地（空氣0/石3/水7/木5/葉6/玻璃10）不可種。
        for b in [0u8, 3, 7, 5, 6, 10] {
            assert!(!is_plantable_ground(b), "{b} 不應可種樹苗");
        }
    }

    #[test]
    fn drop_chance_is_reasonable() {
        // 機率在 (0,1) 開區間，且不氾濫（<0.5）也不苛刻（>0.1）。
        assert!(SAPLING_DROP_CHANCE > 0.1 && SAPLING_DROP_CHANCE < 0.5);
    }

    #[test]
    fn grown_tree_has_trunk_and_canopy() {
        let blocks = grown_tree_blocks(10, 5, 20);
        let trunk = blocks.iter().filter(|(_, _, _, b)| *b == GroveBlock::Trunk).count();
        let leaves = blocks.iter().filter(|(_, _, _, b)| *b == GroveBlock::Leaf).count();
        assert_eq!(trunk, 3, "樹幹應為 3 格");
        // 兩層 3×3 環（各 8）＋頂蓋十字（5）＝ 21 葉。
        assert_eq!(leaves, 21, "樹冠葉片數應為 21");
    }

    #[test]
    fn grown_tree_base_is_trunk() {
        // 底座（樹苗原位）必須被樹幹覆蓋（換掉樹苗方塊）。
        let (bx, by, bz) = (10, 5, 20);
        let blocks = grown_tree_blocks(bx, by, bz);
        assert!(
            blocks.contains(&(bx, by, bz, GroveBlock::Trunk)),
            "底座應是樹幹"
        );
    }

    #[test]
    fn grown_tree_trunk_is_a_vertical_column() {
        // 樹幹三格應在同一 (x,z) 柱、連續三個 y。
        let (bx, by, bz) = (0, 0, 0);
        let blocks = grown_tree_blocks(bx, by, bz);
        let mut trunk_ys: Vec<i32> = blocks
            .iter()
            .filter(|(x, _, z, b)| *b == GroveBlock::Trunk && *x == bx && *z == bz)
            .map(|(_, y, _, _)| *y)
            .collect();
        trunk_ys.sort_unstable();
        assert_eq!(trunk_ys, vec![by, by + 1, by + 2]);
    }

    #[test]
    fn grown_tree_canopy_sits_above_ground() {
        // 所有葉片都在底座之上（y > by），不會長到地面或地下。
        let (bx, by, bz) = (3, 7, 9);
        let blocks = grown_tree_blocks(bx, by, bz);
        for (_, y, _, b) in &blocks {
            if *b == GroveBlock::Leaf {
                assert!(*y > by, "葉片應在底座之上：y={y} by={by}");
            }
        }
    }

    #[test]
    fn grown_tree_positions_are_unique() {
        // 同一格不該被指派兩種方塊（樹幹頂與樹冠環不重疊）。
        let blocks = grown_tree_blocks(0, 0, 0);
        let mut coords: Vec<(i32, i32, i32)> =
            blocks.iter().map(|(x, y, z, _)| (*x, *y, *z)).collect();
        let n = coords.len();
        coords.sort_unstable();
        coords.dedup();
        assert_eq!(coords.len(), n, "樹的方塊座標應彼此不重複");
    }
}
