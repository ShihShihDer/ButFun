//! 歸家羅盤（ROADMAP 418）的純幾何核心。
//!
//! 世界遼闊、農地散佈各處，玩家一往外探索就容易找不到回自家田的路——尤其手機小螢幕、
//! 新手最容易迷路。這個模組提供**伺服器權威**的尋路幾何：給定「玩家當下座標」與「他的
//! 農地序號」，算出回家的**方位角、距離、八方位**，由伺服器塞進該玩家自己的快照欄位，
//! 前端據此畫一枚永遠指向家的羅盤。
//!
//! 全為**純函式、零狀態、確定性**：好測、可被伺服器熱路徑安心呼叫（零鎖、零配置）。
//! 方位慣例對齊螢幕／世界座標（y 向下為正）：**北＝上＝-y，順時針增**（東＝90、南＝180、
//! 西＝270），與前端 `rotate(deg)`（畫面上為 0、順時針）天生對齊，免任何鏡像換算。

/// 八方位。`as u8` 即 wire 索引（0=北，順時針到 7=西北），前端據此查繁中字（i18n 集中在前端）。
/// 順序刻意配合 `((bearing+22.5)/45).floor() % 8`，扇區與索引一一對應。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinal {
    North = 0,
    NorthEast = 1,
    East = 2,
    SouthEast = 3,
    South = 4,
    SouthWest = 5,
    West = 6,
    NorthWest = 7,
}

/// 一次算好的回家指引，給伺服器塞進玩家自己的快照。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HomeGuide {
    /// 回家方位角（度，0=北、順時針）。前端 `rotate(bearing deg)` 即指向家。
    pub bearing: f32,
    /// 到家直線距離（像素）。
    pub distance: f32,
    /// 八方位（給前端顯示「西北」等文字）。
    pub cardinal: Cardinal,
}

/// 農地的「家錨點」＝該序號農地的中心點（世界座標）。
/// 用初始尺寸（未計擴張）算中心：擴張只往下長，錨點落在初始田正中、足夠當回家指標。
/// 確定性——同序號永遠同錨點。
pub fn plot_home_anchor(plot_index: usize) -> (f32, f32) {
    let (ox, oy) = crate::plots::plot_origin(plot_index);
    (
        ox + crate::plots::PLOT_WIDTH * 0.5,
        oy + crate::plots::PLOT_HEIGHT * 0.5,
    )
}

/// 由位移算羅盤方位角（度，0=北/上/-y，順時針）。
/// `dx = 目標x - 我x`、`dy = 目標y - 我y`（世界座標，y 向下）。
/// 推導：北＝-dy 方向、東＝+dx 方向，故 `atan2(dx, -dy)`；負值補成 0..360。
pub fn bearing_deg(dx: f32, dy: f32) -> f32 {
    let deg = dx.atan2(-dy).to_degrees();
    if deg < 0.0 {
        deg + 360.0
    } else {
        deg
    }
}

/// 直線距離（像素）。
pub fn distance(dx: f32, dy: f32) -> f32 {
    dx.hypot(dy)
}

/// 把方位角分到八方位。扇區寬 45°、以正北為中心對稱：`[-22.5,22.5)`＝北，依此類推。
/// 入參可為任意角度（會先正規化），環繞邊界（如 359.9°）正確回到北。
pub fn cardinal8(bearing_deg: f32) -> Cardinal {
    // +22.5 讓正北落在扇區正中；floor 後 mod 8 取索引；先 rem_euclid 容忍負角／超界。
    let idx = (((bearing_deg + 22.5) / 45.0).floor() as i64).rem_euclid(8);
    match idx {
        0 => Cardinal::North,
        1 => Cardinal::NorthEast,
        2 => Cardinal::East,
        3 => Cardinal::SouthEast,
        4 => Cardinal::South,
        5 => Cardinal::SouthWest,
        6 => Cardinal::West,
        _ => Cardinal::NorthWest, // idx == 7
    }
}

/// 從玩家座標到指定農地的完整回家指引（方位＋距離＋八方位）。
pub fn guide_home(player_x: f32, player_y: f32, plot_index: usize) -> HomeGuide {
    let (hx, hy) = plot_home_anchor(plot_index);
    let (dx, dy) = (hx - player_x, hy - player_y);
    HomeGuide {
        bearing: bearing_deg(dx, dy),
        distance: distance(dx, dy),
        cardinal: cardinal8(bearing_deg(dx, dy)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 方位的浮點比較容差。
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.01, "期望 {b}，得到 {a}");
    }

    #[test]
    fn bearing_cardinal_axes() {
        // 北＝正上方（-y）
        close(bearing_deg(0.0, -10.0), 0.0);
        // 東＝正右方（+x）
        close(bearing_deg(10.0, 0.0), 90.0);
        // 南＝正下方（+y）
        close(bearing_deg(0.0, 10.0), 180.0);
        // 西＝正左方（-x）
        close(bearing_deg(-10.0, 0.0), 270.0);
    }

    #[test]
    fn bearing_diagonals() {
        close(bearing_deg(10.0, -10.0), 45.0); // 東北
        close(bearing_deg(10.0, 10.0), 135.0); // 東南
        close(bearing_deg(-10.0, 10.0), 225.0); // 西南
        close(bearing_deg(-10.0, -10.0), 315.0); // 西北
    }

    #[test]
    fn bearing_always_in_range() {
        for &(dx, dy) in &[(1.0, 1.0), (-3.0, 2.0), (0.5, -7.0), (-9.0, -9.0)] {
            let b = bearing_deg(dx, dy);
            assert!((0.0..360.0).contains(&b), "方位 {b} 超界");
        }
    }

    #[test]
    fn distance_is_hypot() {
        close(distance(3.0, 4.0), 5.0);
        close(distance(0.0, 0.0), 0.0);
    }

    #[test]
    fn cardinal8_main_directions() {
        assert_eq!(cardinal8(0.0), Cardinal::North);
        assert_eq!(cardinal8(45.0), Cardinal::NorthEast);
        assert_eq!(cardinal8(90.0), Cardinal::East);
        assert_eq!(cardinal8(135.0), Cardinal::SouthEast);
        assert_eq!(cardinal8(180.0), Cardinal::South);
        assert_eq!(cardinal8(225.0), Cardinal::SouthWest);
        assert_eq!(cardinal8(270.0), Cardinal::West);
        assert_eq!(cardinal8(315.0), Cardinal::NorthWest);
    }

    #[test]
    fn cardinal8_sector_edges() {
        // 北扇區 [-22.5, 22.5)
        assert_eq!(cardinal8(22.4), Cardinal::North);
        assert_eq!(cardinal8(22.5), Cardinal::NorthEast); // 邊界歸下一扇
        // 環繞：359.9° 仍是北
        assert_eq!(cardinal8(359.9), Cardinal::North);
        assert_eq!(cardinal8(337.5), Cardinal::North); // 337.5 是 NW↔N 邊界，歸下一扇（北）
        assert_eq!(cardinal8(337.4), Cardinal::NorthWest); // 邊界內側仍是西北
    }

    #[test]
    fn cardinal8_handles_out_of_range_angles() {
        // 負角與超界角先正規化，不 panic、結果穩定。
        assert_eq!(cardinal8(-45.0), Cardinal::NorthWest); // -45° ≡ 315° ＝ 西北
        assert_eq!(cardinal8(720.0), Cardinal::North);
        assert_eq!(cardinal8(405.0), Cardinal::NorthEast); // 405 ≡ 45
    }

    #[test]
    fn cardinal_wire_index_stable() {
        // wire 契約：as u8 即 0..7，前端查表靠這個，順序不可亂動。
        assert_eq!(Cardinal::North as u8, 0);
        assert_eq!(Cardinal::East as u8, 2);
        assert_eq!(Cardinal::South as u8, 4);
        assert_eq!(Cardinal::West as u8, 6);
        assert_eq!(Cardinal::NorthWest as u8, 7);
    }

    #[test]
    fn home_anchor_is_plot_center_and_deterministic() {
        for idx in 0..6 {
            let (ox, oy) = crate::plots::plot_origin(idx);
            let (ax, ay) = plot_home_anchor(idx);
            close(ax, ox + crate::plots::PLOT_WIDTH * 0.5);
            close(ay, oy + crate::plots::PLOT_HEIGHT * 0.5);
            // 確定性
            assert_eq!(plot_home_anchor(idx), (ax, ay));
        }
    }

    #[test]
    fn guide_home_points_back_at_anchor() {
        let idx = 3;
        let (hx, hy) = plot_home_anchor(idx);
        // 站在家錨點正北方一段距離 → 回家方位該朝南（180），距離正確。
        let g = guide_home(hx, hy - 500.0, idx);
        close(g.distance, 500.0);
        close(g.bearing, 180.0);
        assert_eq!(g.cardinal, Cardinal::South);
    }

    #[test]
    fn guide_home_zero_distance_when_on_anchor() {
        let (hx, hy) = plot_home_anchor(0);
        let g = guide_home(hx, hy, 0);
        close(g.distance, 0.0);
    }
}
