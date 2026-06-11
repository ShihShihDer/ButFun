//! ROADMAP 111 住家內裝——進得去的室內空間（地基切片）。
//!
//! 每個 FreeBuild 地塊的地主可進入一個 8×8 格（256×256px）的私人室內空間。
//! 室內獨立於世界地形：木板地板 + 石磚牆，前端直接按規格繪製，後端只傳位置。
//! 設計取捨：
//!   - 本切片只做「進出地基」，裝飾/多人可見室內留後續切片。
//!   - 移動用簡單線性插值（無世界碰撞），邊界夾緊在內部可移動範圍。
//!   - 不持久化（記憶體模式），重連後玩家在室外（地基切片可接受）。

use crate::land_plot::LAND_PLOTS;

/// 室內格數（長寬均等）。
pub const INTERIOR_COLS: u8 = 8;
pub const INTERIOR_ROWS: u8 = 8;
/// 室內每格像素（與世界 TILE_PX 相同）。
pub const INTERIOR_TILE_PX: f32 = 32.0;
/// 室內空間寬高（像素）。
pub const INTERIOR_WIDTH: f32 = INTERIOR_COLS as f32 * INTERIOR_TILE_PX; // 256
pub const INTERIOR_HEIGHT: f32 = INTERIOR_ROWS as f32 * INTERIOR_TILE_PX; // 256

/// 移動邊界保留（牆厚）。玩家不能走到最外一格格子（石磚牆）。
const MARGIN: f32 = INTERIOR_TILE_PX * 1.5;

/// 進入觸發距離（離地塊中心幾像素以內可進入）。
pub const ENTER_REACH: f32 = 96.0;

/// 室內移動速度（與世界玩家速度一致）。
pub const INDOOR_SPEED: f32 = world_core::PLAYER_SPEED;

/// 玩家進入室內時的初始位置（中央靠南，靠近入口）。
pub fn entry_position() -> (f32, f32) {
    (INTERIOR_WIDTH / 2.0, INTERIOR_HEIGHT - MARGIN - 16.0)
}

/// 在室內按方向鍵移動 `dt` 秒後的新位置，夾緊在可移動區域。
pub fn indoor_step(ix: f32, iy: f32, up: bool, down: bool, left: bool, right: bool, dt: f32) -> (f32, f32) {
    let mut dx = 0.0_f32;
    let mut dy = 0.0_f32;
    if up    { dy -= 1.0; }
    if down  { dy += 1.0; }
    if left  { dx -= 1.0; }
    if right { dx += 1.0; }
    if dx != 0.0 && dy != 0.0 {
        let inv = 1.0 / (2.0_f32).sqrt();
        dx *= inv;
        dy *= inv;
    }
    let nx = (ix + dx * INDOOR_SPEED * dt).clamp(MARGIN, INTERIOR_WIDTH - MARGIN);
    let ny = (iy + dy * INDOOR_SPEED * dt).clamp(MARGIN, INTERIOR_HEIGHT - MARGIN);
    (nx, ny)
}

/// 取得 FreeBuild 地塊的中心世界坐標（供進入距離判斷）。
/// 找不到（地塊不存在）回 `None`。
pub fn plot_center(plot_id: u32) -> Option<(f32, f32)> {
    LAND_PLOTS.iter().find(|p| p.plot_id == plot_id).map(|p| {
        let cx = ((p.min_gx + p.max_gx) as f32 / 2.0 + 0.5) * INTERIOR_TILE_PX;
        let cy = ((p.min_gy + p.max_gy) as f32 / 2.0 + 0.5) * INTERIOR_TILE_PX;
        (cx, cy)
    })
}

/// 玩家世界坐標 (wx, wy) 是否靠近指定 FreeBuild 地塊的中心（可進入判定）。
pub fn near_home(plot_id: u32, wx: f32, wy: f32) -> bool {
    let Some((cx, cy)) = plot_center(plot_id) else { return false; };
    let dist = ((wx - cx).powi(2) + (wy - cy).powi(2)).sqrt();
    dist <= ENTER_REACH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_position_within_bounds() {
        let (ix, iy) = entry_position();
        assert!(ix >= MARGIN && ix <= INTERIOR_WIDTH - MARGIN, "ix={ix} 超出範圍");
        assert!(iy >= MARGIN && iy <= INTERIOR_HEIGHT - MARGIN, "iy={iy} 超出範圍");
    }

    #[test]
    fn indoor_step_clamps_at_wall() {
        // 往左走很遠應夾在 MARGIN
        let (nx, _) = indoor_step(MARGIN, INTERIOR_HEIGHT / 2.0, false, false, true, false, 100.0);
        assert!((nx - MARGIN).abs() < 0.01, "應夾在左邊界 MARGIN={MARGIN}，實際={nx}");

        // 往右走很遠應夾在右邊
        let (nx, _) = indoor_step(INTERIOR_WIDTH - MARGIN, INTERIOR_HEIGHT / 2.0, false, false, false, true, 100.0);
        assert!((nx - (INTERIOR_WIDTH - MARGIN)).abs() < 0.01);

        // 往上走很遠應夾在上邊
        let (_, ny) = indoor_step(INTERIOR_WIDTH / 2.0, MARGIN, true, false, false, false, 100.0);
        assert!((ny - MARGIN).abs() < 0.01);

        // 往下走很遠應夾在下邊
        let (_, ny) = indoor_step(INTERIOR_WIDTH / 2.0, INTERIOR_HEIGHT - MARGIN, false, true, false, false, 100.0);
        assert!((ny - (INTERIOR_HEIGHT - MARGIN)).abs() < 0.01);
    }

    #[test]
    fn indoor_step_diagonal_not_faster() {
        // 用極短 dt 避免夾牆，驗對角線速度 = 單軸速度（已正規化）。
        let (x0, y0) = (INTERIOR_WIDTH / 2.0, INTERIOR_HEIGHT / 2.0);
        let dt = 0.01;
        let (x1, y1) = indoor_step(x0, y0, true, false, false, true, dt);
        let dist = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        let expected = INDOOR_SPEED * dt;
        // 對角線位移應約等於單軸位移（已對角線正規化）。
        assert!((dist - expected).abs() < 0.5, "對角線速度應 ≈ {expected}，實際={dist}");
    }

    #[test]
    fn indoor_step_no_input_stays_put() {
        let (x0, y0) = (128.0, 128.0);
        let (x1, y1) = indoor_step(x0, y0, false, false, false, false, 0.5);
        assert_eq!((x1, y1), (x0, y0));
    }

    #[test]
    fn plot_center_plot0_reasonable() {
        let (cx, cy) = plot_center(0).expect("地塊 0 應存在");
        // 地塊 0：min_gx=43, max_gx=50, min_gy=26, max_gy=33
        // cx = ((43+50)/2 + 0.5) * 32 = 47 * 32 = 1504
        // cy = ((26+33)/2 + 0.5) * 32 = 30 * 32 = 960
        assert!((cx - 1504.0).abs() < 1.0, "cx={cx}");
        assert!((cy - 960.0).abs() < 1.0, "cy={cy}");
    }

    #[test]
    fn near_home_detects_proximity() {
        let (cx, cy) = plot_center(0).unwrap();
        assert!(near_home(0, cx, cy), "中心應在範圍內");
        assert!(!near_home(0, cx + 200.0, cy), "200px 外不應觸發");
    }

    #[test]
    fn plot_center_invalid_id_returns_none() {
        assert!(plot_center(999).is_none());
    }
}
