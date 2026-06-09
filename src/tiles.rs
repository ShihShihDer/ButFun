//! 地形格世界（delta-save 架構）：記憶體前置、確定性生成 + 玩家修改差異。
//!
//! 設計：世界地形以 `tile_kind_at`（world-core）確定性生成，**不存整張世界**；
//! 玩家挖 / 建後偏離預設的格子才進 `deltas`（稀疏 Map），持久化到 `tile_deltas` 表。
//!
//! C-1（本切片）：只有讀取路徑（生成 + delta 覆蓋）；`apply_delta` 在 C-2 挖掘才用到。
//! C-2 挖掘：`dig_reach` 距離判定 + `drop_for_tile` 掉落材料 + `apply_delta` 設 Empty。
//! C-3 碰撞：`is_solid` 判定實心格擋移動。
//! C-4 建造：`apply_delta` 從 Empty → 實心格。

use std::collections::HashMap;

use world_core::{tile_kind_at, TileKind, CHUNK_SIZE, TILE_PX, TILES_PER_CHUNK};

use crate::inventory::ItemKind;

/// 玩家能挖掘地形格的最大距離（像素，格中心距玩家位置）。
/// 約 2.5 格寬，比採集（56px）略寬，讓挖掘手感不要太精準難操作。
pub const DIG_REACH: f32 = 80.0;

/// 挖掘一格後掉落的物品種類與數量。
/// `Empty` 不可挖（回 None）；其餘對應各自的建造材料。
///
/// 純函式，便於測試；接線在 ws.rs 的 `Dig` handler。
pub fn drop_for_tile(kind: TileKind) -> Option<(ItemKind, u32)> {
    match kind {
        TileKind::Empty => None,
        TileKind::Dirt  => Some((ItemKind::Dirt, 1)),
        TileKind::Stone => Some((ItemKind::Stone, 1)),
        TileKind::Ore   => Some((ItemKind::Ether, 1)),
    }
}

/// 記憶體裡的地形格世界（記憶體前置、寫後非同步落地到 `TileStore`）。
pub struct TileWorld {
    /// 玩家修改的差異：鍵 = (chunk_cx, chunk_cy, cell_x, cell_y)，值 = 覆蓋材質。
    /// 初始世界裡任何座標都由 `tile_kind_at` 決定；被挖／建過才進 deltas。
    deltas: HashMap<(i32, i32, u8, u8), TileKind>,
}

impl TileWorld {
    pub fn new() -> Self {
        Self { deltas: HashMap::new() }
    }

    /// 把啟動時從 DB 讀出的 delta 全部載入。
    pub fn with_deltas(deltas: HashMap<(i32, i32, u8, u8), TileKind>) -> Self {
        Self { deltas }
    }

    /// 查某個 cell 的當前種類：先查 delta 覆蓋、再回落確定性生成。
    pub fn tile_kind(&self, cx: i32, cy: i32, tx: u8, ty: u8) -> TileKind {
        if let Some(&k) = self.deltas.get(&(cx, cy, tx, ty)) {
            return k;
        }
        // 格中心的世界像素座標
        let wx = cx as f64 * CHUNK_SIZE as f64 + (tx as f64 + 0.5) * TILE_PX as f64;
        let wy = cy as f64 * CHUNK_SIZE as f64 + (ty as f64 + 0.5) * TILE_PX as f64;
        tile_kind_at(wx, wy)
    }

    /// 這一格是否為實心（擋移動）。C-3 遊戲迴圈用快照接線；此方法供 C-4 Place 等單點查詢用。
    #[allow(dead_code)]
    pub fn is_solid(&self, cx: i32, cy: i32, tx: u8, ty: u8) -> bool {
        self.tile_kind(cx, cy, tx, ty) != TileKind::Empty
    }

    /// C-2 起使用：把一格改成指定材質並記入 deltas（C-2 挖掘 / C-4 建造）。
    pub fn apply_delta(&mut self, cx: i32, cy: i32, tx: u8, ty: u8, kind: TileKind) {
        self.deltas.insert((cx, cy, tx, ty), kind);
    }

    /// 取當前 deltas 的 immutable 參考（供 TileStore flush 用）。
    pub fn deltas(&self) -> &HashMap<(i32, i32, u8, u8), TileKind> {
        &self.deltas
    }
}

impl Default for TileWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// 世界像素座標 → (chunk_cx, chunk_cy, cell_x, cell_y)。
/// 供 Dig / Place handler 換算輸入座標。
pub fn world_to_cell(wx: f32, wy: f32) -> (i32, i32, u8, u8) {
    let gx = (wx / TILE_PX).floor() as i32;
    let gy = (wy / TILE_PX).floor() as i32;
    let n = TILES_PER_CHUNK as i32;
    let cx = gx.div_euclid(n);
    let cy = gy.div_euclid(n);
    let tx = gx.rem_euclid(n) as u8;
    let ty = gy.rem_euclid(n) as u8;
    (cx, cy, tx, ty)
}

/// cell 座標 → 格中心的世界像素座標（給可及性範圍檢查）。
pub fn cell_center(cx: i32, cy: i32, tx: u8, ty: u8) -> (f32, f32) {
    let wx = cx as f32 * CHUNK_SIZE + (tx as f32 + 0.5) * TILE_PX;
    let wy = cy as f32 * CHUNK_SIZE + (ty as f32 + 0.5) * TILE_PX;
    (wx, wy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_for_tile_empty_returns_none() {
        assert_eq!(drop_for_tile(TileKind::Empty), None);
    }

    #[test]
    fn drop_for_tile_solid_tiles_return_materials() {
        assert_eq!(drop_for_tile(TileKind::Dirt),  Some((ItemKind::Dirt,  1)));
        assert_eq!(drop_for_tile(TileKind::Stone), Some((ItemKind::Stone, 1)));
        assert_eq!(drop_for_tile(TileKind::Ore),   Some((ItemKind::Ether, 1)));
    }

    #[test]
    fn world_to_cell_origin() {
        // 世界原點 → chunk (0,0) 的 cell (0,0)。
        assert_eq!(world_to_cell(0.0, 0.0), (0, 0, 0, 0));
    }

    #[test]
    fn world_to_cell_tile_boundary() {
        // 一格寬度剛好落在 (1,0)。
        assert_eq!(world_to_cell(TILE_PX, 0.0), (0, 0, 1, 0));
    }

    #[test]
    fn world_to_cell_chunk_boundary() {
        // 一個 chunk 寬度落在下一個 chunk。
        assert_eq!(world_to_cell(CHUNK_SIZE, 0.0), (1, 0, 0, 0));
    }

    #[test]
    fn world_to_cell_negative_coords() {
        // 負座標：無限世界往負方向延伸，cell 仍要正確。
        let (cx, cy, tx, ty) = world_to_cell(-TILE_PX, 0.0);
        assert_eq!((cx, cy, tx, ty), (-1, 0, 15, 0));
    }

    #[test]
    fn cell_center_round_trips() {
        // 把 world_to_cell 算出的格找回世界座標，應等於格中心（輸入點在格中央）。
        // 輸入點改用格中心以避免 tile 邊界的 ±half 邊界情形（floor 取左邊格）。
        let wx = 1616.5f32; // 格中心 + 0.5，清楚落在格內
        let wy = 816.5f32;
        let (cx, cy, tx, ty) = world_to_cell(wx, wy);
        let (cx2, cy2) = cell_center(cx, cy, tx, ty);
        // 格中心應與輸入點距離 ≤ tile 半格
        assert!((cx2 - wx).abs() <= TILE_PX / 2.0, "X 偏差={}", (cx2 - wx).abs());
        assert!((cy2 - wy).abs() <= TILE_PX / 2.0, "Y 偏差={}", (cy2 - wy).abs());
    }

    #[test]
    fn delta_overrides_generated_tile() {
        let mut world = TileWorld::new();
        // 把 (0,0,0,0) 強制改成 Empty（不管生成值是什麼）
        world.apply_delta(0, 0, 0, 0, TileKind::Empty);
        assert_eq!(world.tile_kind(0, 0, 0, 0), TileKind::Empty);
        // 再改成 Stone
        world.apply_delta(0, 0, 0, 0, TileKind::Stone);
        assert_eq!(world.tile_kind(0, 0, 0, 0), TileKind::Stone);
    }

    #[test]
    fn non_delta_cell_uses_generation() {
        let world = TileWorld::new();
        // 不同座標生成值應具決定性（不 panic、且連呼叫兩次結果相同）
        let k1 = world.tile_kind(3, 2, 7, 11);
        let k2 = world.tile_kind(3, 2, 7, 11);
        assert_eq!(k1, k2);
    }
}
