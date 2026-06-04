//! 農地格資料結構（Phase 0-G 種田起源的純邏輯地基，第二塊）。
//!
//! `crops.rs` 管「一株作物怎麼長」；這層管「一塊地有哪些格、每格現在是什麼狀態、
//! 玩家在某個世界座標互動時對應到哪一格」。同樣是純資料 + 純函式，無 IO、不碰
//! WebSocket / 遊戲迴圈，便於自動測試。之後接上：
//!   - 遊戲迴圈：每 tick 對整塊地呼叫 `tick(dt)` 讓所有作物成長。
//!   - WebSocket：玩家點某格 →（世界座標）`cell_at` →`till` / `plant` / `water` /
//!     `harvest`，把收成的乙太加進背包。
//!   - 持久化（接 0-E）：把 `Field`（每格的 `Tile`）序列化存回。
//!   - 前端：依每格 `Tile` / 作物 `stage()` 畫出翻土 / 各成長階段。
//!
//! 療癒迴圈：自然地 → 翻土 → 播種 → 澆水 → 成長 → 收成 → 回到翻好的空地可再種。
//! 每一步都要玩家主動做，「照顧」本身就是玩法。
//!
//! 註：尚未接上遊戲迴圈 / 前端 / 持久化，先放行未使用警告，接線後即可移除。
#![allow(dead_code)]

use crate::crops::{Crop, CropStage};

/// 每格耕地的邊長（世界像素）。
pub const TILE_SIZE: f32 = 48.0;
/// 農地的欄數（橫向格數）。
pub const FIELD_COLS: usize = 6;
/// 農地的列數（縱向格數）。
pub const FIELD_ROWS: usize = 4;
/// 農地左上角在世界中的位置（像素）。挑在地圖中央附近，讓初來的玩家走幾步就到。
/// 世界 2000×2000、農地 288×192，置中後左上角約在此。
pub const FIELD_ORIGIN_X: f32 = 856.0;
pub const FIELD_ORIGIN_Y: f32 = 904.0;

/// 一格耕地的狀態。
#[derive(Debug, Clone, PartialEq)]
pub enum Tile {
    /// 還沒翻過的自然地，要先翻土才能種。
    Untilled,
    /// 翻好的空土，可以播種。
    Tilled,
    /// 種了一株作物（成長狀態在 `Crop` 裡）。
    Planted(Crop),
}

/// 一塊固定位置、固定大小的農地（row-major 的格子陣列）。
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// 長度固定為 `FIELD_COLS * FIELD_ROWS`，以 `index` 對映 (col,row)。
    tiles: Vec<Tile>,
}

impl Field {
    /// 建一塊全是自然地的新農地。
    pub fn new() -> Self {
        Self {
            tiles: vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS],
        }
    }

    /// (col,row) → tiles 陣列索引；超出範圍回 `None`。純函式。
    fn index(col: usize, row: usize) -> Option<usize> {
        if col < FIELD_COLS && row < FIELD_ROWS {
            Some(row * FIELD_COLS + col)
        } else {
            None
        }
    }

    /// 世界座標 (x,y) → 落在哪一格 (col,row)；不在農地範圍內回 `None`。純函式。
    pub fn cell_at(x: f32, y: f32) -> Option<(usize, usize)> {
        let local_x = x - FIELD_ORIGIN_X;
        let local_y = y - FIELD_ORIGIN_Y;
        if local_x < 0.0 || local_y < 0.0 {
            return None;
        }
        let col = (local_x / TILE_SIZE) as usize;
        let row = (local_y / TILE_SIZE) as usize;
        if col < FIELD_COLS && row < FIELD_ROWS {
            Some((col, row))
        } else {
            None
        }
    }

    /// 讀某格狀態（唯讀）；超出範圍回 `None`。
    pub fn tile(&self, col: usize, row: usize) -> Option<&Tile> {
        Self::index(col, row).map(|i| &self.tiles[i])
    }

    /// 某格作物目前的成長階段；該格沒種東西或超出範圍回 `None`。
    pub fn crop_stage(&self, col: usize, row: usize) -> Option<CropStage> {
        match self.tile(col, row) {
            Some(Tile::Planted(c)) => Some(c.stage()),
            _ => None,
        }
    }

    /// 翻土：只有自然地能翻成空土。成功回 `true`，否則（已翻 / 已種 / 越界）回 `false`。
    pub fn till(&mut self, col: usize, row: usize) -> bool {
        match Self::index(col, row) {
            Some(i) if self.tiles[i] == Tile::Untilled => {
                self.tiles[i] = Tile::Tilled;
                true
            }
            _ => false,
        }
    }

    /// 播種：只有翻好的空土能播。成功回 `true`，否則回 `false`。
    pub fn plant(&mut self, col: usize, row: usize) -> bool {
        match Self::index(col, row) {
            Some(i) if self.tiles[i] == Tile::Tilled => {
                self.tiles[i] = Tile::Planted(Crop::plant());
                true
            }
            _ => false,
        }
    }

    /// 澆水：只有種了作物的格能澆。成功回 `true`，否則回 `false`。
    pub fn water(&mut self, col: usize, row: usize) -> bool {
        match Self::index(col, row) {
            Some(i) => {
                if let Tile::Planted(c) = &mut self.tiles[i] {
                    c.water();
                    return true;
                }
                false
            }
            None => false,
        }
    }

    /// 收成：成熟才給乙太，並把該格回復成翻好的空土（可直接再播種）。
    /// 未成熟 / 沒種 / 越界回 `None`、不改變狀態。
    pub fn harvest(&mut self, col: usize, row: usize) -> Option<u32> {
        let i = Self::index(col, row)?;
        if let Tile::Planted(c) = &mut self.tiles[i] {
            // 先借出可變參考收成；成熟才會回 Some 並消費這格。
            if let Some(ether) = c.harvest() {
                // 收成後不留新種子，回到空土讓玩家自行決定要不要再種。
                self.tiles[i] = Tile::Tilled;
                return Some(ether);
            }
        }
        None
    }

    /// 推進 `dt` 秒：讓地裡所有作物成長（無濕度的不會長，見 `Crop::grow`）。
    pub fn tick(&mut self, dt: f32) {
        for t in &mut self.tiles {
            if let Tile::Planted(c) = t {
                c.grow(dt);
            }
        }
    }
}

impl Default for Field {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crops::{MOISTURE_PER_WATER, RIPE_AT, SPROUT_AT};

    #[test]
    fn new_field_is_all_untilled() {
        let f = Field::new();
        for row in 0..FIELD_ROWS {
            for col in 0..FIELD_COLS {
                assert_eq!(f.tile(col, row), Some(&Tile::Untilled));
            }
        }
    }

    #[test]
    fn cell_at_maps_origin_to_first_cell() {
        assert_eq!(Field::cell_at(FIELD_ORIGIN_X, FIELD_ORIGIN_Y), Some((0, 0)));
    }

    #[test]
    fn cell_at_maps_within_tile_to_same_cell() {
        // 第 (1,2) 格的中心點應落回該格。
        let x = FIELD_ORIGIN_X + 1.0 * TILE_SIZE + TILE_SIZE / 2.0;
        let y = FIELD_ORIGIN_Y + 2.0 * TILE_SIZE + TILE_SIZE / 2.0;
        assert_eq!(Field::cell_at(x, y), Some((1, 2)));
    }

    #[test]
    fn cell_at_is_none_outside_field() {
        // 左上之外
        assert_eq!(Field::cell_at(FIELD_ORIGIN_X - 1.0, FIELD_ORIGIN_Y), None);
        assert_eq!(Field::cell_at(FIELD_ORIGIN_X, FIELD_ORIGIN_Y - 1.0), None);
        // 右下之外
        let far_x = FIELD_ORIGIN_X + FIELD_COLS as f32 * TILE_SIZE;
        let far_y = FIELD_ORIGIN_Y + FIELD_ROWS as f32 * TILE_SIZE;
        assert_eq!(Field::cell_at(far_x, far_y), None);
    }

    #[test]
    fn till_only_works_on_untilled() {
        let mut f = Field::new();
        assert!(f.till(0, 0));
        assert_eq!(f.tile(0, 0), Some(&Tile::Tilled));
        // 已翻過再翻不動作。
        assert!(!f.till(0, 0));
    }

    #[test]
    fn till_out_of_bounds_is_noop() {
        let mut f = Field::new();
        assert!(!f.till(FIELD_COLS, 0));
        assert!(!f.till(0, FIELD_ROWS));
    }

    #[test]
    fn cannot_plant_on_untilled() {
        let mut f = Field::new();
        assert!(!f.plant(0, 0));
        assert_eq!(f.tile(0, 0), Some(&Tile::Untilled));
    }

    #[test]
    fn plant_after_till_creates_seed() {
        let mut f = Field::new();
        f.till(2, 1);
        assert!(f.plant(2, 1));
        assert_eq!(f.crop_stage(2, 1), Some(CropStage::Seed));
    }

    #[test]
    fn cannot_water_empty_cell() {
        let mut f = Field::new();
        assert!(!f.water(0, 0));
        f.till(0, 0);
        assert!(!f.water(0, 0));
    }

    #[test]
    fn full_cycle_till_plant_water_grow_harvest() {
        let mut f = Field::new();
        f.till(0, 0);
        f.plant(0, 0);
        // 單次澆水只夠長 MOISTURE_PER_WATER 秒，需再澆一次才到成熟。
        assert!(f.water(0, 0));
        f.tick(MOISTURE_PER_WATER);
        assert!(f.water(0, 0));
        f.tick(RIPE_AT - MOISTURE_PER_WATER);
        assert_eq!(f.crop_stage(0, 0), Some(CropStage::Ripe));
        // 收成拿到乙太，該格回到翻好的空土。
        assert!(f.harvest(0, 0).is_some());
        assert_eq!(f.tile(0, 0), Some(&Tile::Tilled));
    }

    #[test]
    fn harvest_unripe_returns_none_and_keeps_crop() {
        let mut f = Field::new();
        f.till(0, 0);
        f.plant(0, 0);
        f.water(0, 0);
        assert_eq!(f.harvest(0, 0), None);
        assert_eq!(f.crop_stage(0, 0), Some(CropStage::Seed));
    }

    #[test]
    fn tick_only_grows_watered_crops() {
        let mut f = Field::new();
        f.till(0, 0);
        f.plant(0, 0); // 沒澆水
        f.till(1, 0);
        f.plant(1, 0);
        f.water(1, 0); // 澆了水
        f.tick(SPROUT_AT);
        assert_eq!(f.crop_stage(0, 0), Some(CropStage::Seed)); // 乾的沒長
        assert_eq!(f.crop_stage(1, 0), Some(CropStage::Sprout)); // 濕的長了
    }

    #[test]
    fn ops_on_one_cell_do_not_affect_others() {
        let mut f = Field::new();
        f.till(0, 0);
        assert_eq!(f.tile(1, 0), Some(&Tile::Untilled));
        assert_eq!(f.tile(0, 1), Some(&Tile::Untilled));
    }
}
