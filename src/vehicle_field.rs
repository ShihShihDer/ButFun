//! 蒸汽載具的世界佈置與上車偵測（Phase 1-E 蒸汽載具 MVP 的純邏輯地基之二）。
//!
//! `vehicle.rs` 解了「上車後，方向輸入怎麼推進一台會慣性／甩尾的載具」；接線還缺
//! 另一半——「**載具停在世界哪裡、玩家走近時上的是哪一台**」。本層就是那塊純幾何 +
//! 純偵測：一組停在世界裡的 `Vehicle`，加上
//!   - `tick(dt, ...)`：實際每 tick 推進是由接線層對「有人騎的那台」呼叫
//!     `Vehicle::step`，故本層不自帶 tick——空閒載具靜止不動，沒有要推進的狀態。
//!   - `nearest_within_reach(x, y)`：玩家在 `(x,y)` 想上車時，挑最近、在上車範圍內的
//!     那台，回它的序號（接線層據此讓玩家上車；「這台有沒有人騎」由另一張
//!     騎乘登記表管，比照 `plots.rs` 幾何與 `plot_registry.rs` 歸屬分屬兩層）。
//!
//! 延續 `gather_field.rs` / `plots.rs` / `field.rs` 的前置慣例：純函式、無 IO、不碰
//! WebSocket / 遊戲迴圈 / 廣播 shape，標 `allow(dead_code)`，接線輪（AppState 持有
//! `VehicleField`、ws 上下車、遊戲迴圈推進有人騎的載具、前端畫車與騎乘）才有呼叫端。
//!
//! 佈置刻意停在「家園附近」：世界中央是出生點與家園農莊（`plots.rs` 從中心往外排），
//! 載具停在離中心不遠的一圈，走幾步就騎得到——出門到外圈曠野採集（`gather_field`
//! 把節點散在更外圈）時可騎車代步。佈置由序號決定（角度等分、固定半徑），確定性、
//! 不靠亂數／時鐘，故重啟後空閒載具落在同一處。
//!
//! 與 `gather_field` 的關鍵差異：採集節點位置固定（可由序號重算），但**載具會被騎著
//! 到處跑**，所以位置是會變、要存的狀態——`from_saved` 不重算座標，而是信任存檔位置
//! （經 `Vehicle::is_loadable` 驗證、`Vehicle::at` 已夾過界）。

use crate::state::{Input, WORLD_HEIGHT, WORLD_WIDTH};
use crate::vehicle::Vehicle;

/// 停在世界裡的載具總數。Phase 1-E MVP 先放少少幾台「走得到、找得到」即可。
const VEHICLE_COUNT: usize = 3;

/// 載具停放的環半徑（距世界中心）。刻意不遠：出生點在中心，走幾步就騎得到。
const PARK_RADIUS: f32 = 320.0;

/// 載具距世界邊界至少留這麼多（理論上 PARK_RADIUS 已遠在界內，保險夾一下）。
const EDGE_MARGIN: f32 = 60.0;

/// 玩家上車的伸手範圍：走到載具這個距離內才上得了車（與採集 `GATHER_REACH` 同級，
/// 走近即可）。
pub const BOARD_REACH: f32 = 56.0;

/// 停在世界裡的一整組蒸汽載具。
#[derive(Debug, Clone, PartialEq)]
pub struct VehicleField {
    vehicles: Vec<Vehicle>,
}

// 整個模組是前置地基，接線輪才有呼叫端，比照 `gather_field.rs` / `vehicle.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
impl VehicleField {
    /// 用確定性佈置生出一組全新（靜止）的載具，停在家園附近的環上。
    pub fn new() -> Self {
        let vehicles = (0..VEHICLE_COUNT)
            .map(|i| {
                let (x, y) = park_position(i);
                Vehicle::at(x, y)
            })
            .collect();
        Self { vehicles }
    }

    /// 目前的載具（供前端畫、供遊戲迴圈推進、供測試檢視）。
    pub fn vehicles(&self) -> &[Vehicle] {
        &self.vehicles
    }

    /// 取第 `i` 台載具的可變參考（接線層對「有人騎的那台」呼叫 `Vehicle::step` 用）。
    /// 序號界外回 `None`（客戶端送來的騎乘序號可能無效，權威端不信任）。
    pub fn get_mut(&mut self, i: usize) -> Option<&mut Vehicle> {
        self.vehicles.get_mut(i)
    }

    /// 玩家在 `(px, py)` 想上車：在 `BOARD_REACH` 內挑**最近**的那台，回它的序號；
    /// 範圍內沒有載具時回 `None`（比照 `gather_field::gather_near` 由伺服器權威判定，
    /// 客戶端只送「我要上車」的意圖）。非有限座標一律視為上不了車（延續載入防線脈絡）。
    ///
    /// 本層不管「這台有沒有人騎」——那由接線層的騎乘登記表判斷（同一台不會被兩人騎），
    /// 比照 `plots.rs`（幾何）與 `plot_registry.rs`（歸屬）分屬兩層。
    pub fn nearest_within_reach(&self, px: f32, py: f32) -> Option<usize> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        let reach_sq = BOARD_REACH * BOARD_REACH;
        let mut best: Option<(usize, f32)> = None;
        for (i, v) in self.vehicles.iter().enumerate() {
            let dx = v.x - px;
            let dy = v.y - py;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= reach_sq && best.is_none_or(|(_, b)| dist_sq < b) {
                best = Some((i, dist_sq));
            }
        }
        best.map(|(i, _)| i)
    }

    /// 推進「有人騎的那台」載具 `dt` 秒：把該玩家的方向輸入導向 `Vehicle::step`。
    /// 沒人騎的載具靜止不動，故只動指定序號這一台。序號界外則 no-op。
    pub fn step_ridden(&mut self, index: usize, input: Input, dt: f32) {
        if let Some(v) = self.vehicles.get_mut(index) {
            v.step(input, dt);
        }
    }

    /// 載入入口（接 0-E 從存檔還原載具狀態用）。與 `gather_field::from_saved` 不同：
    /// 載具**會被騎著移動**，位置是會變、要存的狀態，故不重算座標——只驗「數量一致、
    /// 每台 `is_loadable`（位置／速度皆有限）」，否則整組拒收回 `None`，呼叫端退回
    /// `VehicleField::new()`（全新一組停回原位）。延續 `field::from_tiles` 的載入時驗證。
    pub fn from_saved(saved: Vec<Vehicle>) -> Option<Self> {
        if saved.len() != VEHICLE_COUNT {
            return None;
        }
        if saved.iter().all(|v| v.is_loadable()) {
            Some(Self { vehicles: saved })
        } else {
            None
        }
    }
}

impl Default for VehicleField {
    fn default() -> Self {
        Self::new()
    }
}

/// 第 `i` 台載具的停放座標：以序號在固定半徑的環上等分角度，落在家園附近，再夾進
/// 世界邊界內。確定性（同序號永遠同位置）、不靠亂數／時鐘，故重啟後（無存檔時）
/// 空閒載具落在同一處。等分角度 ⇒ 任兩台至少差一個夾角 ⇒ 不重疊。
fn park_position(i: usize) -> (f32, f32) {
    let cx = WORLD_WIDTH / 2.0;
    let cy = WORLD_HEIGHT / 2.0;
    let theta = (i as f32) * std::f32::consts::TAU / (VEHICLE_COUNT as f32);
    let x = (cx + PARK_RADIUS * theta.cos()).clamp(EDGE_MARGIN, WORLD_WIDTH - EDGE_MARGIN);
    let y = (cy + PARK_RADIUS * theta.sin()).clamp(EDGE_MARGIN, WORLD_HEIGHT - EDGE_MARGIN);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(up: bool, down: bool, left: bool, right: bool) -> Input {
        Input {
            up,
            down,
            left,
            right,
        }
    }

    #[test]
    fn new_field_has_full_count_all_at_rest() {
        let f = VehicleField::new();
        assert_eq!(f.vehicles().len(), VEHICLE_COUNT);
        // 全新一組皆靜止（速率 0）。
        assert!(f.vehicles().iter().all(|v| v.speed() == 0.0));
    }

    #[test]
    fn placement_is_deterministic() {
        // 兩次建構座標完全一致（確定性，不靠亂數／時鐘）。
        assert_eq!(VehicleField::new(), VehicleField::new());
    }

    #[test]
    fn vehicles_stay_in_world_and_near_home() {
        let cx = WORLD_WIDTH / 2.0;
        let cy = WORLD_HEIGHT / 2.0;
        for v in VehicleField::new().vehicles() {
            assert!((EDGE_MARGIN..=WORLD_WIDTH - EDGE_MARGIN).contains(&v.x));
            assert!((EDGE_MARGIN..=WORLD_HEIGHT - EDGE_MARGIN).contains(&v.y));
            // 停在家園附近的環上（離中心約 PARK_RADIUS，不會散到外圈曠野）。
            let dist = (v.x - cx).hypot(v.y - cy);
            assert!(dist <= PARK_RADIUS + 1.0, "離家太遠: {dist}");
        }
    }

    #[test]
    fn vehicles_do_not_overlap() {
        let vs = VehicleField::new();
        let list = vs.vehicles();
        for i in 0..list.len() {
            for j in (i + 1)..list.len() {
                let d = (list[i].x - list[j].x).hypot(list[i].y - list[j].y);
                // 等分角度的固定環，任兩台至少差一個上車範圍以上，不會疊在一起。
                assert!(d > BOARD_REACH, "第 {i} 與第 {j} 台太近: {d}");
            }
        }
    }

    #[test]
    fn nearest_within_reach_picks_closest_vehicle() {
        let f = VehicleField::new();
        let target = f.vehicles()[1];
        // 站在第 1 台正上方：應挑到第 1 台。
        assert_eq!(f.nearest_within_reach(target.x, target.y), Some(1));
    }

    #[test]
    fn nearest_within_reach_none_when_far() {
        let f = VehicleField::new();
        // 世界外的座標，必定上不了任何車。
        assert!(f
            .nearest_within_reach(WORLD_WIDTH + 5000.0, WORLD_HEIGHT + 5000.0)
            .is_none());
    }

    #[test]
    fn nearest_within_reach_rejects_non_finite() {
        let f = VehicleField::new();
        assert!(f.nearest_within_reach(f32::NAN, 100.0).is_none());
        assert!(f.nearest_within_reach(100.0, f32::INFINITY).is_none());
    }

    #[test]
    fn step_ridden_moves_only_that_vehicle() {
        let mut f = VehicleField::new();
        let others_before: Vec<Vehicle> = f
            .vehicles()
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 0)
            .map(|(_, v)| *v)
            .collect();
        let start_x = f.vehicles()[0].x;
        // 推進第 0 台往右開幾下。
        for _ in 0..5 {
            f.step_ridden(0, input(false, false, false, true), 1.0 / 15.0);
        }
        assert!(f.vehicles()[0].x > start_x, "被騎的那台應移動");
        // 其他台原地不動。
        let others_after: Vec<Vehicle> = f
            .vehicles()
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 0)
            .map(|(_, v)| *v)
            .collect();
        assert_eq!(others_before, others_after, "沒人騎的載具不該動");
    }

    #[test]
    fn step_ridden_out_of_range_index_is_noop() {
        let mut f = VehicleField::new();
        let before = f.clone();
        f.step_ridden(999, input(false, false, false, true), 0.1);
        assert_eq!(f, before, "界外序號不應改變任何載具");
    }

    #[test]
    fn get_mut_out_of_range_is_none() {
        let mut f = VehicleField::new();
        assert!(f.get_mut(VEHICLE_COUNT).is_none());
        assert!(f.get_mut(0).is_some());
    }

    #[test]
    fn from_saved_round_trips_moved_vehicles() {
        // 載具被騎著移動過，位置改變——存檔該原樣（含位置／速度）還原。
        let mut f = VehicleField::new();
        for _ in 0..8 {
            f.step_ridden(0, input(false, false, false, true), 1.0 / 15.0);
        }
        let saved: Vec<Vehicle> = f.vehicles().to_vec();
        let restored = VehicleField::from_saved(saved).expect("正常存檔該還原");
        assert_eq!(restored, f);
    }

    #[test]
    fn from_saved_rejects_wrong_count() {
        assert!(VehicleField::from_saved(vec![]).is_none());
        let too_few: Vec<Vehicle> = (0..VEHICLE_COUNT - 1).map(|_| Vehicle::at(100.0, 100.0)).collect();
        assert!(VehicleField::from_saved(too_few).is_none());
    }

    #[test]
    fn from_saved_rejects_non_finite_vehicle() {
        let mut saved: Vec<Vehicle> = (0..VEHICLE_COUNT).map(|_| Vehicle::at(100.0, 100.0)).collect();
        // 注入非有限速度（接 Postgres float 後可能存進）→ 整組拒收。
        saved[0].vx = f32::NAN;
        assert!(VehicleField::from_saved(saved).is_none());
    }
}
