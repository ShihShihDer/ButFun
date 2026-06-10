//! 城外產權地塊（ROADMAP 34）——純邏輯層。
//!
//! 主城（新手村）城牆外環帶預定義 20 塊 8×8 格地塊（每塊 256×256 像素），供登入玩家花乙太購買。
//! 購買後地塊進入「產權保護」：只有地主可在其範圍內挖/放，外人的 Dig/Place 一律拒絕。
//!
//! 設計取捨：
//!   - **幾何靜態、所有權動態**：20 塊地的幾何在編譯期決定（`LAND_PLOTS`），只有「誰擁有哪塊」
//!     存進 DB (`land_plot_store`)。這讓前端不需要從伺服器下載格局、降低首幀流量。
//!   - **每玩家限購一塊**：避免大戶壟斷地皮；切片 2 再視需求鬆綁。
//!   - **未購地塊對所有玩家開放**：可進入、可挖/放（沙盒感），但一旦被買下立刻上鎖。
//!   - 幾何純函式（`plot_id_at_tile`、`is_protected_from`）**零 IO**，便於單元測試。

use std::collections::HashMap;

use uuid::Uuid;

/// 每塊地塊的購買價格（乙太）。比農地（20）貴，因為有產權保護、用途更廣。
pub const LAND_PLOT_COST: u32 = 60;

/// 地形格邊長（像素）——與 world_core::TILE_PX 一致，不 import 以避免循環依賴。
const TILE_PX: f32 = 32.0;

/// 一塊城外地塊的靜態幾何（格座標，包含兩端）。
#[derive(Debug, Clone, Copy)]
pub struct PlotGeom {
    pub plot_id: u32,
    /// 左上角格座標（含）。
    pub min_gx: i32,
    pub min_gy: i32,
    /// 右下角格座標（含）。8×8 格，故 max = min + 7。
    pub max_gx: i32,
    pub max_gy: i32,
}

/// 全部 20 塊預定義地塊。
///
/// 主城中心：(73, 71) 格，半寬 half_tiles=34，城牆在 Chebyshev 距離 34 處。
/// 地塊與城牆保持 ≥4 格距離（|plot_edge - wall_tile| ≥ 4），地塊間留 2 格走道。
pub const LAND_PLOTS: &[PlotGeom] = &[
    // ── 北環（gy 26..33；北牆 gy=37，gap=4） ─────────────
    PlotGeom { plot_id:  0, min_gx: 43, min_gy: 26, max_gx: 50, max_gy: 33 },
    PlotGeom { plot_id:  1, min_gx: 53, min_gy: 26, max_gx: 60, max_gy: 33 },
    PlotGeom { plot_id:  2, min_gx: 63, min_gy: 26, max_gx: 70, max_gy: 33 },
    PlotGeom { plot_id:  3, min_gx: 73, min_gy: 26, max_gx: 80, max_gy: 33 },
    PlotGeom { plot_id:  4, min_gx: 83, min_gy: 26, max_gx: 90, max_gy: 33 },
    // ── 南環（gy 110..117；南牆 gy=105，gap=4） ──────────
    PlotGeom { plot_id:  5, min_gx: 43, min_gy: 110, max_gx: 50, max_gy: 117 },
    PlotGeom { plot_id:  6, min_gx: 53, min_gy: 110, max_gx: 60, max_gy: 117 },
    PlotGeom { plot_id:  7, min_gx: 63, min_gy: 110, max_gx: 70, max_gy: 117 },
    PlotGeom { plot_id:  8, min_gx: 73, min_gy: 110, max_gx: 80, max_gy: 117 },
    PlotGeom { plot_id:  9, min_gx: 83, min_gy: 110, max_gx: 90, max_gy: 117 },
    // ── 西環（gx 27..34；西牆 gx=39，gap=4） ─────────────
    PlotGeom { plot_id: 10, min_gx: 27, min_gy: 43, max_gx: 34, max_gy: 50 },
    PlotGeom { plot_id: 11, min_gx: 27, min_gy: 53, max_gx: 34, max_gy: 60 },
    PlotGeom { plot_id: 12, min_gx: 27, min_gy: 63, max_gx: 34, max_gy: 70 },
    PlotGeom { plot_id: 13, min_gx: 27, min_gy: 73, max_gx: 34, max_gy: 80 },
    PlotGeom { plot_id: 14, min_gx: 27, min_gy: 83, max_gx: 34, max_gy: 90 },
    // ── 東環（gx 112..119；東牆 gx=107，gap=4） ──────────
    PlotGeom { plot_id: 15, min_gx: 112, min_gy: 43, max_gx: 119, max_gy: 50 },
    PlotGeom { plot_id: 16, min_gx: 112, min_gy: 53, max_gx: 119, max_gy: 60 },
    PlotGeom { plot_id: 17, min_gx: 112, min_gy: 63, max_gx: 119, max_gy: 70 },
    PlotGeom { plot_id: 18, min_gx: 112, min_gy: 73, max_gx: 119, max_gy: 80 },
    PlotGeom { plot_id: 19, min_gx: 112, min_gy: 83, max_gx: 119, max_gy: 90 },
];

/// 地塊產權登記（記憶體權威；持久化由 `land_plot_store` 負責）。
#[derive(Default)]
pub struct LandPlotRegistry {
    /// plot_id → 地主 user_id。
    ownership: HashMap<u32, Uuid>,
}

impl LandPlotRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化資料重建（啟動載入路徑）。
    pub fn from_saved(saved: impl IntoIterator<Item = (u32, Uuid)>) -> Self {
        Self { ownership: saved.into_iter().collect() }
    }

    /// 查詢地塊地主。若地塊不存在或未購買回 `None`。
    pub fn owner_of(&self, plot_id: u32) -> Option<Uuid> {
        self.ownership.get(&plot_id).copied()
    }

    /// 查詢玩家擁有的地塊 id。每玩家限一塊；未購回 `None`。
    pub fn plot_of(&self, user_id: Uuid) -> Option<u32> {
        self.ownership.iter().find(|(_, &uid)| uid == user_id).map(|(&pid, _)| pid)
    }

    /// 嘗試購買地塊。
    /// 失敗條件（靜默回 `false`）：地塊不存在 / 已被他人買走 / 玩家已有地塊。
    pub fn buy(&mut self, plot_id: u32, user_id: Uuid) -> bool {
        if !LAND_PLOTS.iter().any(|p| p.plot_id == plot_id) { return false; }
        if self.ownership.contains_key(&plot_id) { return false; }
        if self.plot_of(user_id).is_some() { return false; }
        self.ownership.insert(plot_id, user_id);
        true
    }

    /// 世界像素座標是否在「他人已購」的地塊內（Dig/Place 保護檢查）。
    /// 返回 `true` 表示禁止操作（不是地主的人被攔住）。
    pub fn is_protected_from(&self, wx: f32, wy: f32, user_id: Uuid) -> bool {
        let gx = (wx / TILE_PX).floor() as i32;
        let gy = (wy / TILE_PX).floor() as i32;
        for p in LAND_PLOTS {
            if gx >= p.min_gx && gx <= p.max_gx && gy >= p.min_gy && gy <= p.max_gy {
                return self.ownership.get(&p.plot_id).copied()
                    .map(|owner| owner != user_id)
                    .unwrap_or(false);
            }
        }
        false
    }

    /// 依格座標找所在地塊 id（前端精準點選用；找不到回 `None`）。
    pub fn plot_id_at_tile(&self, gx: i32, gy: i32) -> Option<u32> {
        LAND_PLOTS.iter().find(|p| {
            gx >= p.min_gx && gx <= p.max_gx && gy >= p.min_gy && gy <= p.max_gy
        }).map(|p| p.plot_id)
    }

    /// 匯出全部地塊資訊（含地主名稱，供快照廣播）。`get_name` 由呼叫端提供（查玩家 store）。
    pub fn all_plots_view<F>(&self, get_name: F) -> Vec<LandPlotSnapshot>
    where
        F: Fn(Uuid) -> Option<String>,
    {
        LAND_PLOTS.iter().map(|p| {
            let owner_id = self.ownership.get(&p.plot_id).copied();
            let owner_name = owner_id.and_then(|uid| get_name(uid));
            LandPlotSnapshot {
                plot_id: p.plot_id,
                min_gx: p.min_gx,
                min_gy: p.min_gy,
                max_gx: p.max_gx,
                max_gy: p.max_gy,
                owner_id,
                owner_name,
            }
        }).collect()
    }

    /// 匯出持久化所需的全部地塊歸屬紀錄。
    pub fn all_ownerships(&self) -> Vec<(u32, Uuid)> {
        self.ownership.iter().map(|(&pid, &uid)| (pid, uid)).collect()
    }
}

/// 快照裡一塊地塊的可見狀態（送給前端）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LandPlotSnapshot {
    pub plot_id: u32,
    pub min_gx: i32,
    pub min_gy: i32,
    pub max_gx: i32,
    pub max_gy: i32,
    pub owner_id: Option<Uuid>,
    pub owner_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid() -> Uuid { Uuid::new_v4() }

    #[test]
    fn buy_success_and_protection() {
        let mut reg = LandPlotRegistry::new();
        let buyer = uid();
        assert!(reg.buy(0, buyer), "首次購買應成功");
        assert_eq!(reg.owner_of(0), Some(buyer));
        assert_eq!(reg.plot_of(buyer), Some(0));

        // 地塊中心像素：min_gx=43, min_gy=26 → 中心 = (43+4)*32 = 1504, (26+4)*32 = 960
        let (cx, cy) = (47.0 * 32.0, 30.0 * 32.0);
        let stranger = uid();
        assert!(reg.is_protected_from(cx, cy, stranger), "非地主不得操作");
        assert!(!reg.is_protected_from(cx, cy, buyer), "地主本人可以操作");
    }

    #[test]
    fn cannot_buy_already_owned_plot() {
        let mut reg = LandPlotRegistry::new();
        let a = uid();
        let b = uid();
        assert!(reg.buy(1, a));
        assert!(!reg.buy(1, b), "已有地主的地塊不能再賣");
    }

    #[test]
    fn one_plot_per_player() {
        let mut reg = LandPlotRegistry::new();
        let player = uid();
        assert!(reg.buy(2, player));
        assert!(!reg.buy(3, player), "同一玩家限購一塊");
    }

    #[test]
    fn invalid_plot_id_is_rejected() {
        let mut reg = LandPlotRegistry::new();
        assert!(!reg.buy(999, uid()), "不存在的 id 應被拒");
    }

    #[test]
    fn unowned_plot_is_not_protected() {
        let reg = LandPlotRegistry::new();
        // 地塊 0 北環中心
        let (cx, cy) = (47.0 * 32.0, 30.0 * 32.0);
        assert!(!reg.is_protected_from(cx, cy, uid()), "未購地塊應無保護");
    }

    #[test]
    fn world_outside_plots_is_not_protected() {
        let mut reg = LandPlotRegistry::new();
        let owner = uid();
        assert!(reg.buy(0, owner));
        // 世界完全不在任何地塊的位置（wx=0, wy=0）
        assert!(!reg.is_protected_from(0.0, 0.0, uid()));
    }

    #[test]
    fn from_saved_restores_ownership() {
        let player = uid();
        let reg = LandPlotRegistry::from_saved([(5, player)]);
        assert_eq!(reg.owner_of(5), Some(player));
        assert_eq!(reg.plot_of(player), Some(5));
        // 重建後不能再買
        let mut reg = reg;
        assert!(!reg.buy(5, uid()));
    }

    #[test]
    fn plot_id_at_tile_finds_correct_plot() {
        let reg = LandPlotRegistry::new();
        // 北環第 0 塊：gx=[43,50], gy=[26,33]
        assert_eq!(reg.plot_id_at_tile(45, 28), Some(0));
        assert_eq!(reg.plot_id_at_tile(43, 26), Some(0)); // 左上角
        assert_eq!(reg.plot_id_at_tile(50, 33), Some(0)); // 右下角
        // 走道（gx=51,52）是 None；plot 1 從 gx=53 開始
        assert_eq!(reg.plot_id_at_tile(51, 28), None);    // 2格走道
        assert_eq!(reg.plot_id_at_tile(53, 28), Some(1)); // plot 1 左邊界
        assert_eq!(reg.plot_id_at_tile(0, 0), None);      // 非地塊區
    }

    #[test]
    fn no_plots_overlap() {
        for (i, pi) in LAND_PLOTS.iter().enumerate() {
            for (j, pj) in LAND_PLOTS.iter().enumerate() {
                if i >= j { continue; }
                let overlap_x = pi.min_gx <= pj.max_gx && pj.min_gx <= pi.max_gx;
                let overlap_y = pi.min_gy <= pj.max_gy && pj.min_gy <= pi.max_gy;
                assert!(!(overlap_x && overlap_y),
                    "地塊 {} 與 {} 在格座標上重疊", pi.plot_id, pj.plot_id);
            }
        }
    }

    #[test]
    fn all_plots_within_4_tile_buffer_from_town_wall() {
        // 主城：center (73,71), half=34；牆在 Chebyshev 距離 34 處
        // 北牆 gy=37，南牆 gy=105，西牆 gx=39，東牆 gx=107
        for p in LAND_PLOTS {
            // 北環：最靠牆的邊是 max_gy，與 gy=37 的距離
            if p.max_gy <= 36 {
                let gap = 37 - p.max_gy;
                assert!(gap >= 4, "北環地塊 {} max_gy={} 與北牆距離僅 {}", p.plot_id, p.max_gy, gap);
            }
            // 南環：最靠牆的邊是 min_gy
            if p.min_gy >= 106 {
                let gap = p.min_gy - 105;
                assert!(gap >= 4, "南環地塊 {} min_gy={} 與南牆距離僅 {}", p.plot_id, p.min_gy, gap);
            }
            // 西環：最靠牆的邊是 max_gx
            if p.max_gx <= 38 {
                let gap = 39 - p.max_gx;
                assert!(gap >= 4, "西環地塊 {} max_gx={} 與西牆距離僅 {}", p.plot_id, p.max_gx, gap);
            }
            // 東環：最靠牆的邊是 min_gx
            if p.min_gx >= 108 {
                let gap = p.min_gx - 107;
                assert!(gap >= 4, "東環地塊 {} min_gx={} 與東牆距離僅 {}", p.plot_id, p.min_gx, gap);
            }
        }
    }
}
