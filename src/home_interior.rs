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

/// 室內可擺放家具的地板格範圍（內部 6×6，去掉外圍一格石磚牆）。
/// ROADMAP 323 住家家具擺位：家具只能放在地板格上，不能擺進牆裡。
pub const FLOOR_MIN_CELL: u8 = 1;
pub const FLOOR_MAX_CELL: u8 = 6;

/// 室內格 (col, row) 是否為可擺放家具的地板格（落在內部 6×6、非外圍石磚牆）。
pub fn is_floor_cell(col: u8, row: u8) -> bool {
    (FLOOR_MIN_CELL..=FLOOR_MAX_CELL).contains(&col)
        && (FLOOR_MIN_CELL..=FLOOR_MAX_CELL).contains(&row)
}

/// 把室內座標 (ix, iy) 換算成所在的地板格 (col, row)，夾緊在可擺放範圍內。
/// 玩家可移動範圍（`indoor_step` 夾在 [MARGIN, INTERIOR-MARGIN]）天然落在地板格內，
/// 故夾緊只是防呆；ROADMAP 323 用它把「玩家當前所站的格」當成家具落點。
pub fn cell_of(ix: f32, iy: f32) -> (u8, u8) {
    let clamp_cell = |v: f32| -> u8 {
        let c = (v / INTERIOR_TILE_PX).floor() as i32;
        c.clamp(FLOOR_MIN_CELL as i32, FLOOR_MAX_CELL as i32) as u8
    };
    (clamp_cell(ix), clamp_cell(iy))
}

/// 居家風格主題（ROADMAP 325）。
///
/// 玩家可在自己室內循環切換的裝潢風格，決定地板與牆面的視覺色調。
/// 後端只持有語意代碼（snake_case 契約），實際色票與中文名稱一律由前端決定，
/// 保留 i18n 空間、也讓美術調色集中在繪製層。純記憶體模式（與家具同步，重啟歸零）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum HomeStyle {
    /// 木屋（預設）：暖棕木板 + 石磚牆，沿用 111 原始風格。
    #[default]
    WoodCabin,
    /// 石砌廳堂：冷灰石板地 + 深岩牆，沉穩厚重。
    StoneHall,
    /// 乙太水晶：泛藍晶亮地坪 + 幽光晶牆，蒸汽龐克科幻感。
    AetherCrystal,
    /// 溫馨田園：奶黃軟木地 + 暖陶牆，柔和居家。
    CozyPastoral,
    /// 星空雅居：靛紫夜色地 + 星塵牆，靜謐療癒。
    Starlit,
}

/// 所有風格的固定順序（循環切換用，也是前端對應色票的權威次序）。
pub const HOME_STYLES: &[HomeStyle] = &[
    HomeStyle::WoodCabin,
    HomeStyle::StoneHall,
    HomeStyle::AetherCrystal,
    HomeStyle::CozyPastoral,
    HomeStyle::Starlit,
];

impl HomeStyle {
    /// 穩定的字串代碼（snake_case），作為前後端契約。
    pub fn code(self) -> &'static str {
        match self {
            HomeStyle::WoodCabin => "wood_cabin",
            HomeStyle::StoneHall => "stone_hall",
            HomeStyle::AetherCrystal => "aether_crystal",
            HomeStyle::CozyPastoral => "cozy_pastoral",
            HomeStyle::Starlit => "starlit",
        }
    }

    /// 從 snake_case 字串解析；未知字串回 `None`。
    pub fn from_str(s: &str) -> Option<Self> {
        HOME_STYLES.iter().copied().find(|st| st.code() == s)
    }

    /// 循環切換到下一個風格（最後一個繞回第一個）。玩家按「換風格」即推進。
    pub fn next(self) -> Self {
        let idx = HOME_STYLES.iter().position(|&st| st == self).unwrap_or(0);
        HOME_STYLES[(idx + 1) % HOME_STYLES.len()]
    }
}

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
    fn floor_cell_excludes_walls() {
        // 內部 6×6（1..=6）為地板格；外圍 0 與 7 為石磚牆。
        assert!(is_floor_cell(1, 1));
        assert!(is_floor_cell(6, 6));
        assert!(is_floor_cell(3, 4));
        assert!(!is_floor_cell(0, 3), "col=0 是牆");
        assert!(!is_floor_cell(3, 0), "row=0 是牆");
        assert!(!is_floor_cell(7, 3), "col=7 是牆");
        assert!(!is_floor_cell(3, 7), "row=7 是牆");
    }

    #[test]
    fn cell_of_maps_walkable_range_to_floor_cells() {
        // 玩家可移動範圍內任一點換算出的格都應落在地板格 1..=6。
        for &(ix, iy) in &[
            (MARGIN, MARGIN),
            (INTERIOR_WIDTH - MARGIN, INTERIOR_HEIGHT - MARGIN),
            (INTERIOR_WIDTH / 2.0, INTERIOR_HEIGHT / 2.0),
        ] {
            let (col, row) = cell_of(ix, iy);
            assert!(is_floor_cell(col, row), "({ix},{iy}) → ({col},{row}) 應為地板格");
        }
        // 進入點（玩家初始所站處）也應在地板格。
        let (ix, iy) = entry_position();
        let (col, row) = cell_of(ix, iy);
        assert!(is_floor_cell(col, row));
        // 中央 (128,128) 落在第 4 格（128/32 = 4）。
        assert_eq!(cell_of(128.0, 128.0), (4, 4));
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

    // —— ROADMAP 325：居家風格主題 ——

    #[test]
    fn home_style_default_is_wood_cabin() {
        // 預設風格＝木屋，沿用 111 原始外觀；serde default 也據此向後相容舊快照。
        assert_eq!(HomeStyle::default(), HomeStyle::WoodCabin);
    }

    #[test]
    fn home_style_codes_are_unique_and_roundtrip() {
        // 前後端契約：每個風格代碼唯一，且 from_str(code()) 能還原。
        use std::collections::HashSet;
        let codes: HashSet<&str> = HOME_STYLES.iter().map(|s| s.code()).collect();
        assert_eq!(codes.len(), HOME_STYLES.len(), "風格代碼必須唯一");
        for &st in HOME_STYLES {
            assert_eq!(HomeStyle::from_str(st.code()), Some(st), "{} 應可還原", st.code());
        }
    }

    #[test]
    fn home_style_from_str_unknown_is_none() {
        assert_eq!(HomeStyle::from_str("nonexistent_style"), None);
    }

    #[test]
    fn home_style_next_cycles_through_all_and_wraps() {
        // 循環切換：依 HOME_STYLES 次序前進，最後一個繞回第一個。
        let mut st = HomeStyle::WoodCabin;
        let mut seen = vec![st];
        for _ in 1..HOME_STYLES.len() {
            st = st.next();
            seen.push(st);
        }
        // 走完一輪應恰好覆蓋全部風格（不重不漏）。
        for &s in HOME_STYLES {
            assert!(seen.contains(&s), "{} 應在循環中出現", s.code());
        }
        // 再 next 一次繞回起點。
        assert_eq!(st.next(), HomeStyle::WoodCabin, "最後一個應繞回第一個");
    }
}
