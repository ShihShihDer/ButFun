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
    /// 車目前停放的世界座標（騎乘中不更新，僅下車時設成下車者當下座標——前端對有乘客的車
    /// 直接畫在乘客腳下，故騎乘期間車座標 stale 無妨，省掉每幀同步車位置的巢狀鎖）。
    pub x: f32,
    pub y: f32,
    /// 目前乘客（None = 空車可上）。
    pub rider: Option<Uuid>,
}

/// 全世界的蒸汽腳踏車場（記憶體前置、零持久化）。
#[derive(Debug, Clone, Default)]
pub struct VehicleField {
    cycles: Vec<SteamCycle>,
}

/// 上車結果：成功時回乘上的車 id（供接線端把玩家標記為騎乘該車）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardOutcome {
    /// 成功上車（附車 id）。
    Boarded(u32),
    /// 附近沒有可上的空車。
    None,
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

    /// 玩家在 `(px,py)` 嘗試上最近的空車。成功則把該車標記為此乘客所有並回 `Boarded(id)`；
    /// 附近無空車回 `None`。會先把該乘客可能還騎著的別台車釋放（防同時佔兩台，理論上不會發生）。
    pub fn board(&mut self, rider: Uuid, px: f32, py: f32) -> BoardOutcome {
        match self.nearest_boardable(px, py) {
            Some(cid) => {
                self.release_rider(rider);
                if let Some(c) = self.cycles.iter_mut().find(|c| c.id == cid) {
                    c.rider = Some(rider);
                }
                BoardOutcome::Boarded(cid)
            }
            None => BoardOutcome::None,
        }
    }

    /// 乘客下車：把車停在下車者當下座標 `(px,py)`、清空乘客，回剛下的車 id（本來就沒在騎回 None）。
    pub fn dismount(&mut self, rider: Uuid, px: f32, py: f32) -> Option<u32> {
        let cycle = self.cycles.iter_mut().find(|c| c.rider == Some(rider))?;
        cycle.rider = None;
        if px.is_finite() && py.is_finite() {
            cycle.x = px;
            cycle.y = py;
        }
        Some(cycle.id)
    }

    /// 此乘客目前騎著哪台車（沒騎回 None）。
    pub fn cycle_of_rider(&self, rider: Uuid) -> Option<u32> {
        self.cycles
            .iter()
            .find(|c| c.rider == Some(rider))
            .map(|c| c.id)
    }

    /// 釋放某乘客騎著的車（玩家離線／換星球時呼叫，讓車回到空車可被別人騎）。
    /// 車留在原停放座標（不知道玩家最後位置時的保守作法）。
    pub fn release_rider(&mut self, rider: Uuid) {
        for c in self.cycles.iter_mut() {
            if c.rider == Some(rider) {
                c.rider = None;
            }
        }
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
                SteamCycle { id: 1, x: 100.0, y: 0.0, rider: None },
                SteamCycle { id: 2, x: 40.0, y: 0.0, rider: None },
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
        let did = f.dismount(rider, cx + 500.0, cy + 500.0);
        assert_eq!(did, Some(0));
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
                SteamCycle { id: 1, x: 0.0, y: 0.0, rider: None },
                SteamCycle { id: 2, x: 10.0, y: 0.0, rider: None },
            ],
        };
        let rider = uid(5);
        f.board(rider, 0.0, 0.0); // 上 id=1（最近）
        f.board(rider, 10.0, 0.0); // 再上 id=2，應先釋放 id=1
        let held: Vec<u32> = f.cycles().iter().filter(|c| c.rider == Some(rider)).map(|c| c.id).collect();
        assert_eq!(held, vec![2], "同一乘客同時只能佔一台車");
    }
}
