//! 蒸汽載具物理模型（Phase 1-E 蒸汽載具 MVP 的純邏輯地基）。
//!
//! 這層只管「上車後，方向輸入怎麼推進一台會慣性／甩尾的載具」，是純資料 + 純函式，
//! 無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試。延續 `tools.rs` / `gather.rs` /
//! `crafting.rs` / `crops.rs` 的前置慣例：純邏輯先落地、標 `allow(dead_code)`，
//! 接線輪才有呼叫端。
//!
//! 之後接上：
//!   - 世界：地圖上撒佈若干蒸汽腳踏車實體（座標由序號推導，比照 `gather_field`）。
//!   - ws：玩家走近按鍵「上下車」；上車後把該玩家的方向輸入導向 `Vehicle::step`，
//!     下車回到原本走路整合（`Player::step`）。
//!   - 遊戲迴圈：每 tick 對有人騎的載具呼叫 `step(dt)`，位置隨快照廣播。
//!   - 持久化（接 0-E）：載具靜止狀態序列化（載入時走 `is_loadable` / `at` 驗證）。
//!
//! 操控刻意做成「慣性 + 輕微甩尾」（GDD Phase 1-E）：油門對著輸入方向加速、放開後靠
//! 阻力滑行（慣性），轉向時舊速度不會瞬間歸零、會帶著往前甩一下再順過來（甩尾）。
//! 最高速約走路的 3 倍——騎車明顯比走路快。整合與夾制沿用 `Player::step` 的世界邊界語意。

use serde::{Deserialize, Serialize};

use crate::state::{Input, PLAYER_SPEED, WORLD_HEIGHT, WORLD_WIDTH};

/// 載具最高速度（像素 / 秒）。驗收「移動明顯比走快」：定為走路速度的 3 倍。
pub const VEHICLE_MAX_SPEED: f32 = PLAYER_SPEED * 3.0;

/// 阻力係數（每秒）。放開油門後速度依此衰減 → 滑行的慣性感；數值越大越快停。
pub const VEHICLE_DRAG: f32 = 2.5;

/// 油門加速度（像素 / 秒²）。刻意取 `MAX_SPEED * DRAG`，使滿油門對齊方向時的
/// 平衡速度（加速與阻力打平）正好收斂到 `VEHICLE_MAX_SPEED`，硬上限只當安全夾制。
pub const VEHICLE_ACCEL: f32 = VEHICLE_MAX_SPEED * VEHICLE_DRAG;

/// 一台蒸汽載具的權威物理狀態：位置 + 速度向量。速度讓它有慣性與甩尾，
/// 不像走路那樣放開鍵就立刻停。
// 前置地基：接線輪（撒佈載具、ws 上下車、遊戲迴圈推進）才有呼叫端，
// 在此之前公開項目皆無外部呼叫，比照 `gather.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vehicle {
    pub x: f32,
    pub y: f32,
    /// 速度向量（像素 / 秒）。慣性／甩尾都靠它在 tick 之間延續。
    pub vx: f32,
    pub vy: f32,
}

#[allow(dead_code)]
impl Vehicle {
    /// 在 `(x, y)` 造一台靜止的載具。座標一律經 `at` 守門（非有限退回地圖中央、
    /// 界外夾回邊界），與 `positions::spawn_at` 同一道載入防線語意。
    pub fn at(x: f32, y: f32) -> Self {
        let (x, y) = sanitize_pos(x, y);
        Vehicle {
            x,
            y,
            vx: 0.0,
            vy: 0.0,
        }
    }

    /// 目前速率（速度向量長度，像素 / 秒）。供接線時判斷「在動沒」、前端顯示時速。
    pub fn speed(&self) -> f32 {
        self.vx.hypot(self.vy)
    }

    /// 依方向輸入把載具往前推進 `dt` 秒（權威整合，含慣性、甩尾與邊界夾制）。
    /// `input` 沿用走路那套方向意圖（上車後同一組鍵改控車）。非正／非有限 dt 視為沒動。
    ///
    /// 模型：油門沿輸入方向加速 → 阻力讓速度衰減（放開油門就滑行的慣性）→ 速率封頂
    /// → 位置整合並夾回世界。轉向時舊速度衰減而非瞬間歸零，於是會「帶著往前甩一下」
    /// 再順向新方向——即輕微甩尾。
    pub fn step(&mut self, input: Input, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }

        // 1) 阻力：先讓既有速度向零衰減，給放開油門後滑行的慣性。夾在 [0,1] 防大 dt
        //    把速度反向。先衰減再加油門，使滿油門對齊方向的離散平衡速率正好收斂到
        //    `VEHICLE_ACCEL / VEHICLE_DRAG == VEHICLE_MAX_SPEED`（且與 dt 無關）。
        let retain = (1.0 - VEHICLE_DRAG * dt).clamp(0.0, 1.0);
        self.vx *= retain;
        self.vy *= retain;

        // 2) 油門：沿正規化後的輸入方向加速（沒按方向就不加速，只靠慣性滑行）。
        let (dx, dy) = direction(input);
        self.vx += dx * VEHICLE_ACCEL * dt;
        self.vy += dy * VEHICLE_ACCEL * dt;

        // 3) 速率封頂：等比例縮回 `VEHICLE_MAX_SPEED`，保留行進方向（甩尾不被破壞）。
        let speed = self.speed();
        if speed > VEHICLE_MAX_SPEED {
            let scale = VEHICLE_MAX_SPEED / speed;
            self.vx *= scale;
            self.vy *= scale;
        }

        // 4) 整合位置並夾回世界；撞到邊界就把該軸速度歸零，不沿牆持續累積（沿用
        //    `Player::step` 的世界邊界語意）。
        let nx = self.x + self.vx * dt;
        let ny = self.y + self.vy * dt;
        let cx = nx.clamp(0.0, WORLD_WIDTH);
        let cy = ny.clamp(0.0, WORLD_HEIGHT);
        if cx != nx {
            self.vx = 0.0;
        }
        if cy != ny {
            self.vy = 0.0;
        }
        self.x = cx;
        self.y = cy;
    }

    /// 載入時的最小不變式（接 0-E 從持久化讀回時驗證，比照 `crops::is_loadable`）：
    /// 位置與速度皆有限。位置界外不在此擋（交給 `at` 夾回）；速率上界不檢查（`step`
    /// 下一 tick 自會封頂）。壞值（NaN／Inf，接 Postgres float 後可能存進）一律拒收。
    pub fn is_loadable(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.vx.is_finite() && self.vy.is_finite()
    }
}

/// 把方向輸入轉成正規化方向向量（對角線正規化，避免斜走變快）。沒按方向回 `(0, 0)`。
/// 與 `Player::step` 的方向處理同語意，抽出共用於載具油門方向。
fn direction(input: Input) -> (f32, f32) {
    let mut dx = 0.0;
    let mut dy = 0.0;
    if input.up {
        dy -= 1.0;
    }
    if input.down {
        dy += 1.0;
    }
    if input.left {
        dx -= 1.0;
    }
    if input.right {
        dx += 1.0;
    }
    if dx != 0.0 && dy != 0.0 {
        let inv = 1.0 / (2.0_f32).sqrt();
        dx *= inv;
        dy *= inv;
    }
    (dx, dy)
}

/// 把載入／生成的座標夾回合法世界範圍：非有限退回地圖中央，界外夾回邊界。
/// 與 `positions::spawn_at` 同一道載入防線語意。
fn sanitize_pos(x: f32, y: f32) -> (f32, f32) {
    let cx = if x.is_finite() {
        x.clamp(0.0, WORLD_WIDTH)
    } else {
        WORLD_WIDTH / 2.0
    };
    let cy = if y.is_finite() {
        y.clamp(0.0, WORLD_HEIGHT)
    } else {
        WORLD_HEIGHT / 2.0
    };
    (cx, cy)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 方便造方向輸入。
    fn input(up: bool, down: bool, left: bool, right: bool) -> Input {
        Input {
            up,
            down,
            left,
            right,
        }
    }

    #[test]
    fn starts_at_rest() {
        let v = Vehicle::at(100.0, 200.0);
        assert_eq!((v.x, v.y), (100.0, 200.0));
        assert_eq!(v.speed(), 0.0);
    }

    #[test]
    fn at_sanitizes_bad_coords() {
        // 非有限退回地圖中央。
        let v = Vehicle::at(f32::NAN, f32::INFINITY);
        assert_eq!((v.x, v.y), (WORLD_WIDTH / 2.0, WORLD_HEIGHT / 2.0));
        // 界外夾回邊界。
        let v = Vehicle::at(-50.0, WORLD_HEIGHT + 999.0);
        assert_eq!((v.x, v.y), (0.0, WORLD_HEIGHT));
    }

    #[test]
    fn throttle_builds_speed_and_moves() {
        let mut v = Vehicle::at(100.0, 100.0);
        let start = v.x;
        v.step(input(false, false, false, true), 0.1); // 向右
        assert!(v.vx > 0.0, "油門應沿輸入方向加速 vx={}", v.vx);
        assert!(v.x > start, "應往右移動 x={}", v.x);
        assert_eq!(v.vy, 0.0);
    }

    #[test]
    fn glides_after_releasing_throttle() {
        // 先加速，再放開油門：靠慣性滑行——速度衰減但不會瞬間歸零。
        let mut v = Vehicle::at(100.0, 100.0);
        for _ in 0..10 {
            v.step(input(false, false, false, true), 1.0 / 15.0);
        }
        let cruising = v.vx;
        assert!(cruising > 0.0);
        let before = v.x;
        v.step(input(false, false, false, false), 1.0 / 15.0); // 放開
        assert!(v.vx > 0.0, "放開油門仍應有殘速（慣性）vx={}", v.vx);
        assert!(v.vx < cruising, "速度應衰減 {} -> {}", cruising, v.vx);
        assert!(v.x > before, "滑行仍前進");
    }

    #[test]
    fn carries_momentum_when_turning_drift() {
        // 先往右開到有速度，再改按下：舊的 +x 速度不會瞬間消失，會帶著甩一下。
        let mut v = Vehicle::at(500.0, 500.0);
        for _ in 0..10 {
            v.step(input(false, false, false, true), 1.0 / 15.0);
        }
        assert!(v.vx > 0.0);
        v.step(input(false, true, false, false), 1.0 / 15.0); // 改按下
        assert!(v.vx > 0.0, "轉向後仍帶原方向殘速（甩尾）vx={}", v.vx);
        assert!(v.vy > 0.0, "同時開始往新方向加速 vy={}", v.vy);
    }

    #[test]
    fn speed_caps_near_max_and_beats_walking() {
        // 滿油門開很久：速率收斂到上限且不超過，並明顯快於走路。每 tick 把位置重置回
        // 地圖中央，只觀察速度收斂、不讓它一路撞到邊界把速度歸零（測的是速率上限本身）。
        let mut v = Vehicle::at(1000.0, 1000.0);
        for _ in 0..200 {
            v.step(input(false, false, false, true), 1.0 / 15.0);
            v.x = 1000.0;
            v.y = 1000.0;
        }
        assert!(
            v.speed() <= VEHICLE_MAX_SPEED + 0.01,
            "不得超過上限 speed={}",
            v.speed()
        );
        assert!(
            v.speed() > VEHICLE_MAX_SPEED - 1.0,
            "滿油門應收斂到接近上限 speed={}",
            v.speed()
        );
        // 約走路的 3 倍。
        assert!(v.speed() > PLAYER_SPEED * 2.9);
    }

    #[test]
    fn diagonal_not_faster_than_straight() {
        // 對角線正規化：斜開的平衡速率不應超過直開（仍受同一上限）。同樣每 tick 重置
        // 位置，避免撞到角落把速度歸零而讓斷言失去意義。
        let mut v = Vehicle::at(1000.0, 1000.0);
        for _ in 0..200 {
            v.step(input(false, true, false, true), 1.0 / 15.0); // 右下
            v.x = 1000.0;
            v.y = 1000.0;
        }
        assert!(v.speed() <= VEHICLE_MAX_SPEED + 0.01, "speed={}", v.speed());
        // 斜開也應收斂到接近上限（正規化後與直開同速）。
        assert!(v.speed() > VEHICLE_MAX_SPEED - 1.0, "speed={}", v.speed());
    }

    #[test]
    fn clamped_to_world_bounds() {
        // 貼著右邊界往右猛開：位置夾在世界內，撞牆該軸速度歸零。
        let mut v = Vehicle::at(WORLD_WIDTH, 1000.0);
        for _ in 0..30 {
            v.step(input(false, false, false, true), 1.0 / 15.0);
        }
        assert!(v.x <= WORLD_WIDTH && v.x >= 0.0, "x={}", v.x);
        assert!(v.y >= 0.0 && v.y <= WORLD_HEIGHT);
        assert_eq!(v.vx, 0.0, "撞右牆後該軸速度應歸零");
    }

    #[test]
    fn non_positive_or_nonfinite_dt_is_noop() {
        let mut v = Vehicle::at(300.0, 300.0);
        v.vx = 50.0;
        let before = v; // Vehicle 是 Copy
        v.step(input(false, false, false, true), 0.0);
        v.step(input(false, false, false, true), -0.5);
        v.step(input(false, false, false, true), f32::NAN);
        assert_eq!(v, before, "非正／非有限 dt 不應改變狀態");
    }

    #[test]
    fn is_loadable_rejects_non_finite() {
        assert!(Vehicle::at(10.0, 10.0).is_loadable());
        let bad = Vehicle {
            x: 10.0,
            y: 10.0,
            vx: f32::NAN,
            vy: 0.0,
        };
        assert!(!bad.is_loadable());
    }

    #[test]
    fn serde_round_trips() {
        let mut v = Vehicle::at(123.0, 456.0);
        v.step(input(false, false, false, true), 0.2);
        let json = serde_json::to_string(&v).unwrap();
        let back: Vehicle = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}
