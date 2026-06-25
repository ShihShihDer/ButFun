//! 蒸汽載具 MVP（Phase 1-E·北極星·Phase 1「載具」垂直切片）。
//!
//! 故鄉草原上撒著幾台可乘騎的蒸汽腳踏車：玩家走近按「🚲 上車」即坐上、移動明顯比走路快，
//! 按「🚶 下車」回到原本走法，車就停在下車處等下一位旅人。第一次讓「駕駛」這條北極星
//! 支柱在腳邊落地——玩家一眼看得到、騎上去馬上有感。
//!
//! 設計原則（守 CLAUDE.md／worker.prompt 鐵律）：
//! - **複用 world-core 移動，絕不另寫物理**：騎乘只是把 `Player::step` 的步進 `dt` 再乘一個
//!   倍率（見 `ride_effective_dt`），對角線正規化、水域阻擋、實心格碰撞全部沿用
//!   `world_core::step_with_keys` 那唯一一份移動數學——騎車與走路共用同一套碰撞，車也過不了牆、
//!   下不了水，與 wasm 前端預測一致。
//! - 純邏輯、可測、無 IO、不碰鎖／廣播（接線交給 `ws.rs`／`game.rs`）。
//! - 記憶體前置、零持久化、零 migration——重啟回到初始車況（車回原位、無人乘騎）。

use uuid::Uuid;

/// 騎乘時移動速度倍率（相對步行）。BACKLOG Phase 1-E：「速度比走路快 3 倍」。
pub const VEHICLE_SPEED_MULT: f32 = 3.0;

/// 走近多少像素內可上車（玩家權威座標與車座標的距離門檻）。
pub const BOARD_RADIUS: f32 = 80.0;
/// 上車距離門檻平方（省一次開根號）。
pub const BOARD_RADIUS_SQ: f32 = BOARD_RADIUS * BOARD_RADIUS;

/// 把步進 `dt` 依騎乘狀態縮放——這是「車輛物理整合」的核心純函式：
/// 騎乘時回 `dt * VEHICLE_SPEED_MULT`（接進 `Player::step` 後等同移動變快、碰撞邏輯一字不改），
/// 沒騎就回原值。刻意抽成純函式以便自動測試鎖住「騎車就是快 3 倍、否則照舊」這條契約。
pub fn ride_effective_dt(base_dt: f32, riding: bool) -> f32 {
    if riding {
        base_dt * VEHICLE_SPEED_MULT
    } else {
        base_dt
    }
}

/// 一台蒸汽腳踏車的權威狀態。
#[derive(Debug, Clone)]
pub struct SteamCycle {
    /// 穩定 id（前端追蹤、上下車指定用）。
    pub id: u32,
    /// 車目前世界座標。空車＝停放座標；有駕駛時由遊戲迴圈每拍同步成駕駛當下座標
    /// （`VehicleField::sync_positions`），讓「走近共乘」的鄰近判定與快照都對得上真實位置。
    pub x: f32,
    pub y: f32,
    /// 目前駕駛（None = 空車可上）。
    pub rider: Option<Uuid>,
    /// 目前後座乘客（共乘，ROADMAP 538；None = 後座空、可被附近的人坐上一同兜風）。
    /// 只有「車有駕駛」時後座才可被坐上；乘客不操控移動，由迴圈每拍黏到駕駛座標。
    pub passenger: Option<Uuid>,
}

/// 全世界的蒸汽腳踏車場（記憶體前置、零持久化）。
#[derive(Debug, Clone, Default)]
pub struct VehicleField {
    cycles: Vec<SteamCycle>,
}

/// 上車結果：成功時回乘上的車 id（供接線端把玩家標記為騎乘該車）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardOutcome {
    /// 成為駕駛坐上空車（附車 id）。
    Boarded(u32),
    /// 坐上他人車的後座共乘（附車 id，ROADMAP 538）——接線端據此標記為「乘客」。
    BoardedAsPassenger(u32),
    /// 附近既沒空車可駕、也沒可共乘的後座。
    None,
}

/// 下車結果（ROADMAP 538）：附帶「駕駛下車時連帶被請下後座的乘客」，讓接線端一併清掉其騎乘旗標。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DismountOutcome {
    /// 剛下的車 id。
    pub cycle_id: u32,
    /// 駕駛下車時被一起請下車的後座乘客（自己是乘客下車時為 None）。
    pub ejected_passenger: Option<Uuid>,
}

impl VehicleField {
    /// 撒下故鄉草原開局的幾台車——座標選在新手村東側開闊草地（玩家出生點 (1360,200) 一帶外圍），
    /// 都是可行走的 Empty 地形、避開水域與牆，玩家四處走動很快就會遇到一台。
    pub fn with_default() -> Self {
        let spots = [
            (1700.0, 480.0),
            (2100.0, 760.0),
            (1520.0, 1040.0),
            (2480.0, 520.0),
        ];
        let cycles = spots
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| SteamCycle {
                id: i as u32,
                x,
                y,
                rider: None,
                passenger: None,
            })
            .collect();
        Self { cycles }
    }

    /// 找出玩家權威座標 `BOARD_RADIUS` 內、最近的一台**空車** id（沒有就回 None）。
    /// 純查詢、不改狀態——接線端先用它判斷該不該顯示「上車」鈕、或實際上車時挑車。
    pub fn nearest_boardable(&self, px: f32, py: f32) -> Option<u32> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        self.cycles
            .iter()
            .filter(|c| c.rider.is_none())
            .filter_map(|c| {
                let d2 = (c.x - px).powi(2) + (c.y - py).powi(2);
                if d2 <= BOARD_RADIUS_SQ {
                    Some((c.id, d2))
                } else {
                    None
                }
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(id, _)| id)
    }

    /// 找出玩家權威座標 `BOARD_RADIUS` 內、最近一台「有駕駛、後座空、且駕駛不是自己」的車 id
    /// （共乘候選，ROADMAP 538）。純查詢、不改狀態；車座標由迴圈每拍同步成駕駛當下座標，故鄰近
    /// 判定對得上駕駛真實位置。`rider` 是想共乘者自己的 id（不可坐上自己駕駛的車的後座）。
    pub fn nearest_co_ridable(&self, rider: Uuid, px: f32, py: f32) -> Option<u32> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        self.cycles
            .iter()
            .filter(|c| c.rider.is_some() && c.rider != Some(rider) && c.passenger.is_none())
            .filter_map(|c| {
                let d2 = (c.x - px).powi(2) + (c.y - py).powi(2);
                if d2 <= BOARD_RADIUS_SQ {
                    Some((c.id, d2))
                } else {
                    None
                }
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(id, _)| id)
    }

    /// 玩家在 `(px,py)` 嘗試上最近的空車。成功則把該車標記為此乘客所有並回 `Boarded(id)`；
    /// 附近無空車回 `None`。會先把該乘客可能還騎著的別台車釋放（防同時佔兩台，理論上不會發生）。
    pub fn board(&mut self, rider: Uuid, px: f32, py: f32) -> BoardOutcome {
        match self.nearest_boardable(px, py) {
            Some(cid) => {
                self.detach(rider);
                if let Some(c) = self.cycles.iter_mut().find(|c| c.id == cid) {
                    c.rider = Some(rider);
                }
                BoardOutcome::Boarded(cid)
            }
            None => BoardOutcome::None,
        }
    }

    /// 玩家在 `(px,py)` 嘗試坐上最近一台有駕駛的車的後座共乘（ROADMAP 538）。成功回
    /// `BoardedAsPassenger(id)`；附近無可共乘的車回 `None`。先解除此人可能還佔著的別座位（防一人兩座）。
    pub fn board_passenger(&mut self, rider: Uuid, px: f32, py: f32) -> BoardOutcome {
        match self.nearest_co_ridable(rider, px, py) {
            Some(cid) => {
                self.detach(rider);
                if let Some(c) = self.cycles.iter_mut().find(|c| c.id == cid) {
                    c.passenger = Some(rider);
                }
                BoardOutcome::BoardedAsPassenger(cid)
            }
            None => BoardOutcome::None,
        }
    }

    /// 把每台「在騎」車輛的座標同步成其駕駛當下座標（ROADMAP 538，遊戲迴圈每拍呼叫）。
    /// `driver_pos`：車 id → 駕駛 (x,y)。空車不在表內、座標保持停放處不動。
    pub fn sync_positions(&mut self, driver_pos: &std::collections::HashMap<u32, (f32, f32)>) {
        for c in self.cycles.iter_mut() {
            if let Some(&(x, y)) = driver_pos.get(&c.id) {
                c.x = x;
                c.y = y;
            }
        }
    }

    /// 下車（ROADMAP 538，兼容駕駛與乘客兩種身分）：
    /// - 此人是**駕駛** → 把車停在其當下座標 `(px,py)`、清空駕駛，並連帶把後座乘客一起請下車
    ///   （回傳該乘客 id 供接線端清旗標）；
    /// - 此人是**後座乘客** → 只清自己這個後座，車續歸駕駛、座標不動；
    /// - 本來就沒在車上 → 回 `None`。
    pub fn dismount(&mut self, rider: Uuid, px: f32, py: f32) -> Option<DismountOutcome> {
        // 先找此人當駕駛的車。
        if let Some(c) = self.cycles.iter_mut().find(|c| c.rider == Some(rider)) {
            c.rider = None;
            let ejected = c.passenger.take();
            if px.is_finite() && py.is_finite() {
                c.x = px;
                c.y = py;
            }
            return Some(DismountOutcome {
                cycle_id: c.id,
                ejected_passenger: ejected,
            });
        }
        // 否則找此人當後座乘客的車：只下後座，車不動。
        if let Some(c) = self.cycles.iter_mut().find(|c| c.passenger == Some(rider)) {
            c.passenger = None;
            return Some(DismountOutcome {
                cycle_id: c.id,
                ejected_passenger: None,
            });
        }
        None
    }

    /// 此人目前以駕駛身分騎著哪台車（沒當駕駛回 None；不含後座乘客）。
    pub fn cycle_of_rider(&self, rider: Uuid) -> Option<u32> {
        self.cycles
            .iter()
            .find(|c| c.rider == Some(rider))
            .map(|c| c.id)
    }

    /// 此人目前以後座乘客身分坐在哪台車（沒共乘回 None，ROADMAP 538）。
    pub fn cycle_of_passenger(&self, rider: Uuid) -> Option<u32> {
        self.cycles
            .iter()
            .find(|c| c.passenger == Some(rider))
            .map(|c| c.id)
    }

    /// 內部：把某人從所有座位（駕駛或後座）拔下來，不停車、不回傳——上車前防呆用。
    fn detach(&mut self, rider: Uuid) {
        for c in self.cycles.iter_mut() {
            if c.rider == Some(rider) {
                c.rider = None;
            }
            if c.passenger == Some(rider) {
                c.passenger = None;
            }
        }
    }

    /// 釋放某人佔著的座位（玩家離線／換星球時呼叫）。回傳「因其為駕駛而被連帶請下車的後座乘客」
    /// id（ROADMAP 538，供接線端清該乘客旗標）；其本身若是後座乘客則只清後座、回 None。
    /// 車留在原座標（不知道玩家最後位置時的保守作法）。
    pub fn release_rider(&mut self, rider: Uuid) -> Option<Uuid> {
        let mut ejected = None;
        for c in self.cycles.iter_mut() {
            if c.rider == Some(rider) {
                c.rider = None;
                ejected = ejected.or_else(|| c.passenger.take());
            }
            if c.passenger == Some(rider) {
                c.passenger = None;
            }
        }
        ejected
    }

    /// 唯讀檢視所有車（給快照廣播）。
    pub fn cycles(&self) -> &[SteamCycle] {
        &self.cycles
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn ride_effective_dt_triples_when_riding() {
        let dt = 0.1_f32;
        assert!((ride_effective_dt(dt, true) - dt * 3.0).abs() < 1e-6);
        assert!((ride_effective_dt(dt, false) - dt).abs() < 1e-6);
    }

    #[test]
    fn default_field_has_several_empty_cycles() {
        let f = VehicleField::with_default();
        assert!(f.cycles().len() >= 2, "故鄉應撒下數台車");
        assert!(f.cycles().iter().all(|c| c.rider.is_none()), "開局皆空車");
        // id 不重號
        let mut ids: Vec<u32> = f.cycles().iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), f.cycles().len(), "車 id 不得撞號");
    }

    #[test]
    fn nearest_boardable_respects_radius() {
        let f = VehicleField::with_default();
        let (cx, cy) = (f.cycles()[0].x, f.cycles()[0].y);
        // 站在車上：搆得著
        assert_eq!(f.nearest_boardable(cx, cy), Some(f.cycles()[0].id));
        // 站在半徑外一點點：搆不著
        assert_eq!(f.nearest_boardable(cx + BOARD_RADIUS + 1.0, cy), None);
        // 非有限座標：保守回 None
        assert_eq!(f.nearest_boardable(f32::NAN, cy), None);
    }

    #[test]
    fn nearest_boardable_picks_closest() {
        let mut f = VehicleField {
            cycles: vec![
                SteamCycle { id: 1, x: 100.0, y: 0.0, rider: None, passenger: None },
                SteamCycle { id: 2, x: 40.0, y: 0.0, rider: None, passenger: None },
            ],
        };
        assert_eq!(f.nearest_boardable(0.0, 0.0), Some(2));
        // 把近的那台佔走後，改挑次近且仍在半徑內的（這裡 100px 在半徑外 → None）
        f.board(uid(9), 40.0, 0.0);
        assert_eq!(f.nearest_boardable(0.0, 0.0), None);
    }

    #[test]
    fn board_then_dismount_roundtrip() {
        let mut f = VehicleField::with_default();
        let (cx, cy) = (f.cycles()[0].x, f.cycles()[0].y);
        let rider = uid(1);
        // 上車
        assert_eq!(f.board(rider, cx, cy), BoardOutcome::Boarded(0));
        assert_eq!(f.cycle_of_rider(rider), Some(0));
        // 該車已被佔，別人搆到同一台時不該再回它
        assert!(f.nearest_boardable(cx, cy) != Some(0));
        // 騎到別處後下車：車停在下車座標
        let did = f.dismount(rider, cx + 500.0, cy + 500.0).expect("應有下車結果");
        assert_eq!(did.cycle_id, 0);
        assert_eq!(did.ejected_passenger, None);
        assert_eq!(f.cycle_of_rider(rider), None);
        let c0 = &f.cycles()[0];
        assert!((c0.x - (cx + 500.0)).abs() < 1e-3 && (c0.y - (cy + 500.0)).abs() < 1e-3);
    }

    #[test]
    fn board_when_none_in_range() {
        let mut f = VehicleField::with_default();
        // 遠離所有車
        assert_eq!(f.board(uid(1), 99_999.0, 99_999.0), BoardOutcome::None);
        assert_eq!(f.cycle_of_rider(uid(1)), None);
    }

    #[test]
    fn dismount_when_not_riding_is_noop() {
        let mut f = VehicleField::with_default();
        assert_eq!(f.dismount(uid(7), 0.0, 0.0), None);
    }

    // ── 雙人共乘（ROADMAP 538）────────────────────────────────────────────────

    #[test]
    fn co_ride_only_offered_when_cycle_has_a_driver() {
        let mut f = VehicleField::with_default();
        let (cx, cy) = (f.cycles()[0].x, f.cycles()[0].y);
        // 空車不可共乘（後座共乘只在已有駕駛時開放）。
        assert_eq!(f.nearest_co_ridable(uid(2), cx, cy), None);
        // 有人駕駛後，附近他人可共乘該車。
        f.board(uid(1), cx, cy);
        assert_eq!(f.nearest_co_ridable(uid(2), cx, cy), Some(0));
        // 駕駛本人不能坐自己車的後座。
        assert_eq!(f.nearest_co_ridable(uid(1), cx, cy), None);
    }

    #[test]
    fn board_passenger_fills_back_seat_then_seat_full() {
        let mut f = VehicleField::with_default();
        let (cx, cy) = (f.cycles()[0].x, f.cycles()[0].y);
        f.board(uid(1), cx, cy); // 駕駛
        assert_eq!(f.board_passenger(uid(2), cx, cy), BoardOutcome::BoardedAsPassenger(0));
        assert_eq!(f.cycle_of_passenger(uid(2)), Some(0));
        // 後座已滿：第三人搆到同車也共乘不上。
        assert_eq!(f.nearest_co_ridable(uid(3), cx, cy), None);
        assert_eq!(f.board_passenger(uid(3), cx, cy), BoardOutcome::None);
    }

    #[test]
    fn driver_dismount_ejects_passenger() {
        let mut f = VehicleField::with_default();
        let (cx, cy) = (f.cycles()[0].x, f.cycles()[0].y);
        f.board(uid(1), cx, cy);
        f.board_passenger(uid(2), cx, cy);
        // 駕駛下車：車停在下車座標，後座乘客被一起請下車。
        let out = f.dismount(uid(1), cx + 300.0, cy).expect("駕駛應有下車結果");
        assert_eq!(out.cycle_id, 0);
        assert_eq!(out.ejected_passenger, Some(uid(2)));
        assert_eq!(f.cycle_of_rider(uid(1)), None);
        assert_eq!(f.cycle_of_passenger(uid(2)), None);
    }

    #[test]
    fn passenger_dismount_leaves_driver_riding() {
        let mut f = VehicleField::with_default();
        let (cx, cy) = (f.cycles()[0].x, f.cycles()[0].y);
        f.board(uid(1), cx, cy);
        f.board_passenger(uid(2), cx, cy);
        // 後座乘客自行下車：只清自己後座，駕駛續騎、不被波及。
        let out = f.dismount(uid(2), cx, cy).expect("乘客應有下車結果");
        assert_eq!(out.cycle_id, 0);
        assert_eq!(out.ejected_passenger, None);
        assert_eq!(f.cycle_of_rider(uid(1)), Some(0));
        assert_eq!(f.cycle_of_passenger(uid(2)), None);
    }

    #[test]
    fn release_driver_ejects_passenger_release_passenger_does_not() {
        let mut f = VehicleField::with_default();
        let (cx, cy) = (f.cycles()[0].x, f.cycles()[0].y);
        f.board(uid(1), cx, cy);
        f.board_passenger(uid(2), cx, cy);
        // 乘客離線：只清後座、不波及駕駛、無連帶請下車。
        assert_eq!(f.release_rider(uid(2)), None);
        assert_eq!(f.cycle_of_rider(uid(1)), Some(0));
        assert_eq!(f.cycle_of_passenger(uid(2)), None);
        // 再讓乘客回來，這次換駕駛離線：連帶請後座乘客下車。
        f.board_passenger(uid(2), cx, cy);
        assert_eq!(f.release_rider(uid(1)), Some(uid(2)));
        assert_eq!(f.cycle_of_rider(uid(1)), None);
        assert_eq!(f.cycle_of_passenger(uid(2)), None);
    }

    #[test]
    fn sync_positions_moves_ridden_cycles_to_driver() {
        let mut f = VehicleField::with_default();
        let (cx, cy) = (f.cycles()[0].x, f.cycles()[0].y);
        f.board(uid(1), cx, cy);
        let mut pos = std::collections::HashMap::new();
        pos.insert(0u32, (cx + 1234.0, cy - 567.0));
        f.sync_positions(&pos);
        let c0 = f.cycles().iter().find(|c| c.id == 0).unwrap();
        assert!((c0.x - (cx + 1234.0)).abs() < 1e-3 && (c0.y - (cy - 567.0)).abs() < 1e-3);
        // 不在表內的空車座標不動。
        let c1_before = f.cycles().iter().find(|c| c.id == 1).map(|c| (c.x, c.y)).unwrap();
        f.sync_positions(&std::collections::HashMap::new());
        let c1_after = f.cycles().iter().find(|c| c.id == 1).map(|c| (c.x, c.y)).unwrap();
        assert_eq!(c1_before, c1_after);
    }

    #[test]
    fn boarding_as_passenger_detaches_prior_seat() {
        // 一人不可同時佔兩個座位：先當某車駕駛，再去坐別車後座，原駕駛座應釋放。
        let mut f = VehicleField {
            cycles: vec![
                SteamCycle { id: 1, x: 0.0, y: 0.0, rider: Some(uid(9)), passenger: None },
                SteamCycle { id: 2, x: 10.0, y: 0.0, rider: None, passenger: None },
            ],
        };
        // uid(5) 先當 id=2 駕駛
        f.board(uid(5), 10.0, 0.0);
        assert_eq!(f.cycle_of_rider(uid(5)), Some(2));
        // 再去坐 id=1（uid(9) 駕駛中）的後座 → 應先釋放 id=2 駕駛座
        assert_eq!(f.board_passenger(uid(5), 0.0, 0.0), BoardOutcome::BoardedAsPassenger(1));
        assert_eq!(f.cycle_of_rider(uid(5)), None);
        assert_eq!(f.cycle_of_passenger(uid(5)), Some(1));
    }

    #[test]
    fn release_rider_frees_the_cycle() {
        let mut f = VehicleField::with_default();
        let (cx, cy) = (f.cycles()[0].x, f.cycles()[0].y);
        let rider = uid(3);
        f.board(rider, cx, cy);
        assert_eq!(f.cycle_of_rider(rider), Some(0));
        f.release_rider(rider);
        assert_eq!(f.cycle_of_rider(rider), None);
        // 釋放後又成空車、可再被搆到
        assert_eq!(f.nearest_boardable(cx, cy), Some(0));
    }

    #[test]
    fn rider_cannot_hold_two_cycles() {
        let mut f = VehicleField {
            cycles: vec![
                SteamCycle { id: 1, x: 0.0, y: 0.0, rider: None, passenger: None },
                SteamCycle { id: 2, x: 10.0, y: 0.0, rider: None, passenger: None },
            ],
        };
        let rider = uid(5);
        f.board(rider, 0.0, 0.0); // 上 id=1（最近）
        f.board(rider, 10.0, 0.0); // 再上 id=2，應先釋放 id=1
        let held: Vec<u32> = f.cycles().iter().filter(|c| c.rider == Some(rider)).map(|c| c.id).collect();
        assert_eq!(held, vec![2], "同一乘客同時只能佔一台車");
    }
}
