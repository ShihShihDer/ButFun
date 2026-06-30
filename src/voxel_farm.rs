//! 乙太方界·種田 v1——撒種‧等待‧收割（ROADMAP 659）。
//!
//! **純邏輯層**：`FarmStore` + 生長計時純函式，確定性、無副作用、全可測。
//! 鎖 / WS / IO 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。
//!
//! 種植流程：
//!   FarmSoil(11)  →[Plant action]→  FarmSoilSeeded(12)  →[~90s]→  WheatMature(13)
//!
//! 收穫：Break WheatMature → Seeds(14)×1 + Wheat(18)×1 + FarmSoil(11)（得顆粒以合麵包）。
//! 取消種植：Break FarmSoilSeeded → Seeds(14)×1 + FarmSoil(11)（退還種子）。
//! 麵包：3 Wheat(18) → Bread(19)（2×2 合成格一排）。
//!
//! FarmStore **純記憶體**（與世界 delta 行為一致：重啟後農地重置）。
//! 之後需持久化再加 jsonl 層，此版先讓玩家看到「有感的農地時間維度」。

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 種子物品 id（純 inventory 物品，無對應 Block enum；Block::from_u8(SEEDS_ID) = None）。
/// 從葉片(6)/成熟小麥(13)/幼苗(12)破壞後掉落。
pub const SEEDS_ID: u8 = 14;

/// 小麥顆粒物品 id（純 inventory 物品，從成熟小麥(13)收割時掉落 ×1）。
/// 3 顆粒在 2×2 合成格合一排 → 1 麵包（療癒農業循環終點）。
pub const WHEAT_ID: u8 = 18;

/// 麵包物品 id（純 inventory 物品，3 小麥顆粒在 2×2 格一排 → 1 麵包）。
/// 可送給居民當禮物（居民特別開心）。
pub const BREAD_ID: u8 = 19;

/// 幼苗成熟所需秒數（~90 秒 = 1.5 分鐘）。調校讓玩家在一輪遊玩中體驗完整循環。
pub const GROW_SECS: u64 = 90;

/// 一塊農地的種植記錄。
#[derive(Clone, Debug, PartialEq)]
pub struct FarmPlot {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 種下去的 Unix 秒數（用來判斷是否成熟）。
    pub planted_secs: u64,
}

/// 農地 store（純記憶體，重啟後農地重置，與世界 delta 行為一致）。
#[derive(Default)]
pub struct FarmStore {
    plots: HashMap<(i32, i32, i32), FarmPlot>,
}

impl FarmStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 種下種子：記錄農地 + planted_secs。重複種同格 → 覆蓋（重置計時）。
    pub fn plant(&mut self, x: i32, y: i32, z: i32, now_secs: u64) -> FarmPlot {
        let plot = FarmPlot { x, y, z, planted_secs: now_secs };
        self.plots.insert((x, y, z), plot.clone());
        plot
    }

    /// 移除農地記錄（方塊被挖掉 / 成熟後從 store 清掉）。
    pub fn remove(&mut self, x: i32, y: i32, z: i32) {
        self.plots.remove(&(x, y, z));
    }

    /// 此座標是否有農地記錄。
    pub fn has_plot(&self, x: i32, y: i32, z: i32) -> bool {
        self.plots.contains_key(&(x, y, z))
    }

    /// 回傳所有已成熟的農地座標（planted_secs + GROW_SECS ≤ now_secs）。
    pub fn mature_plots(&self, now_secs: u64) -> Vec<(i32, i32, i32)> {
        self.plots
            .iter()
            .filter(|(_, p)| now_secs >= p.planted_secs.saturating_add(GROW_SECS))
            .map(|(&coord, _)| coord)
            .collect()
    }
}

/// 取得目前 Unix 秒數（農地計時用）。
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plant_creates_plot() {
        let mut s = FarmStore::new();
        s.plant(1, 5, 2, 1000);
        assert!(s.has_plot(1, 5, 2));
        assert!(!s.has_plot(0, 0, 0));
    }

    #[test]
    fn remove_clears_plot() {
        let mut s = FarmStore::new();
        s.plant(3, 5, 7, 1000);
        s.remove(3, 5, 7);
        assert!(!s.has_plot(3, 5, 7));
    }

    #[test]
    fn immature_before_grow_secs() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 1000);
        // planted_secs=1000, now=1089（剛好差 89 秒，未達 90 秒門檻）
        assert!(s.mature_plots(1000 + GROW_SECS - 1).is_empty());
    }

    #[test]
    fn mature_at_exactly_grow_secs() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 1000);
        // planted_secs=1000, now=1090（剛好 90 秒 → 成熟）
        let m = s.mature_plots(1000 + GROW_SECS);
        assert_eq!(m.len(), 1);
        assert!(m.contains(&(0, 5, 0)));
    }

    #[test]
    fn mature_well_past_grow_secs() {
        let mut s = FarmStore::new();
        s.plant(2, 5, 3, 500);
        let m = s.mature_plots(1000); // 差 500 秒 >> 90 秒
        assert!(m.contains(&(2, 5, 3)));
    }

    #[test]
    fn only_mature_plots_returned() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 1000); // planted 1000，差10秒未熟
        s.plant(1, 5, 0, 910);  // planted 910，差100秒 > 90：成熟
        // now_secs = 1010
        let m = s.mature_plots(1010);
        assert_eq!(m.len(), 1);
        assert!(m.contains(&(1, 5, 0)));
        assert!(!m.contains(&(0, 5, 0)));
    }

    #[test]
    fn remove_after_harvest_clears_store() {
        let mut s = FarmStore::new();
        s.plant(5, 5, 5, 0);
        let mature = s.mature_plots(GROW_SECS);
        for c in &mature {
            s.remove(c.0, c.1, c.2);
        }
        assert!(!s.has_plot(5, 5, 5));
    }

    #[test]
    fn plant_overwrites_resets_timer() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 1000);
        // 重新種：計時器重置到 2000
        s.plant(0, 5, 0, 2000);
        // 在 2000+89 時應未熟（以新計時為準）
        assert!(s.mature_plots(2000 + GROW_SECS - 1).is_empty());
        // 2000+90 才熟
        assert_eq!(s.mature_plots(2000 + GROW_SECS).len(), 1);
    }

    #[test]
    fn empty_store_no_mature_plots() {
        let s = FarmStore::new();
        assert!(s.mature_plots(99999).is_empty());
    }

    #[test]
    fn multiple_plots_all_mature() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 0);
        s.plant(1, 5, 0, 0);
        s.plant(2, 5, 0, 0);
        let m = s.mature_plots(GROW_SECS);
        assert_eq!(m.len(), 3);
    }

    // ── 麵包 v1（ROADMAP 668）常數一致性測試 ──────────────────────────────────
    #[test]
    fn item_ids_unique_and_in_range() {
        // 四個物品 id 互不相同
        assert_ne!(SEEDS_ID, WHEAT_ID);
        assert_ne!(SEEDS_ID, BREAD_ID);
        assert_ne!(WHEAT_ID, BREAD_ID);
        // 皆在合法 u8 範圍；14 是 SEEDS 不被方塊 enum 佔用，18/19 同理。
        assert_eq!(SEEDS_ID, 14);
        assert_eq!(WHEAT_ID, 18);
        assert_eq!(BREAD_ID, 19);
    }
}
