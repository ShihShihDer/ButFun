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

use crate::crops::{Crop, CropStage};
use crate::protocol::{FieldView, TileView};

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
/// 要照顧農地，玩家至少得站在地塊裡、或緊鄰邊緣這個距離內（像素）。
/// 權威伺服器用它擋掉「人根本不在農地卻送座標隔空遙控」的客戶端。
pub const FARM_REACH: f32 = TILE_SIZE;

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

/// 玩家對一格做了「一鍵照顧」後，實際發生了什麼（給上層回饋 / 加乙太）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FarmOutcome {
    /// 把自然地翻成空土。
    Tilled,
    /// 在空土上播了種。
    Planted,
    /// 替作物澆了水。
    Watered,
    /// 收成了成熟作物，拿到這麼多乙太。
    Harvested(u32),
    /// 沒對應到任何格或無事可做。
    Nothing,
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
        // 先擋非有限座標：客戶端可能送 NaN / Inf，而 `NaN < 0.0` 為 false 不會被下面
        // 的範圍檢查擋下，且 `(NaN / TILE_SIZE) as usize` 在 Rust 飽和轉型成 0，會讓
        // 垃圾座標誤落到 (0,0) 格。權威伺服器一律視為界外。
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
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
    /// 生產端走 `view()` 取整塊地，單格查詢目前只用於測試斷言；
    /// 之後接持久化逐格存取時再放開。
    #[cfg(test)]
    pub fn tile(&self, col: usize, row: usize) -> Option<&Tile> {
        Self::index(col, row).map(|i| &self.tiles[i])
    }

    /// 某格作物目前的成長階段；該格沒種東西或超出範圍回 `None`。
    #[cfg(test)]
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

    /// 「一鍵照顧」：依某格目前狀態自動決定要做什麼，並執行：
    /// 自然地→翻土、空土→播種、未熟作物→澆水、成熟作物→收成。
    /// 越界回 `Nothing`。把「該做哪個動作」的判斷集中在這裡，前端只要送座標。
    pub fn interact(&mut self, col: usize, row: usize) -> FarmOutcome {
        let Some(i) = Self::index(col, row) else {
            return FarmOutcome::Nothing;
        };
        // 先唯讀地決定動作，放掉借用後再做可變操作（避免 borrow 衝突）。
        enum Act {
            Till,
            Plant,
            Water,
            Harvest,
        }
        let act = match &self.tiles[i] {
            Tile::Untilled => Act::Till,
            Tile::Tilled => Act::Plant,
            Tile::Planted(c) if c.is_ripe() => Act::Harvest,
            Tile::Planted(_) => Act::Water,
        };
        // 走既有的單一動作方法，集中各自的狀態前提檢查。
        match act {
            Act::Till => {
                self.till(col, row);
                FarmOutcome::Tilled
            }
            Act::Plant => {
                self.plant(col, row);
                FarmOutcome::Planted
            }
            Act::Water => {
                self.water(col, row);
                FarmOutcome::Watered
            }
            Act::Harvest => FarmOutcome::Harvested(self.harvest(col, row).unwrap_or(0)),
        }
    }

    /// 把整塊地轉成給前端的可見快照。
    pub fn view(&self) -> FieldView {
        FieldView {
            origin_x: FIELD_ORIGIN_X,
            origin_y: FIELD_ORIGIN_Y,
            tile_size: TILE_SIZE,
            cols: FIELD_COLS,
            rows: FIELD_ROWS,
            cells: self.tiles.iter().map(tile_view).collect(),
        }
    }
}

/// 玩家位於 (px,py) 時，是否近到能在農地上操作（在地塊內、或離邊緣 `FARM_REACH` 內）。
/// 量的是「點到農地矩形的最近距離」，所以站在地塊任一處都算，不必貼著某一格。純函式。
pub fn within_field_reach(px: f32, py: f32) -> bool {
    let right = FIELD_ORIGIN_X + FIELD_COLS as f32 * TILE_SIZE;
    let bottom = FIELD_ORIGIN_Y + FIELD_ROWS as f32 * TILE_SIZE;
    // 把玩家座標夾到農地矩形上，得到矩形上的最近點。
    let nx = px.clamp(FIELD_ORIGIN_X, right);
    let ny = py.clamp(FIELD_ORIGIN_Y, bottom);
    let dx = px - nx;
    let dy = py - ny;
    dx * dx + dy * dy <= FARM_REACH * FARM_REACH
}

/// 一格 → 前端可見狀態。純函式。
/// state：0=自然地 1=空土 2=種子 3=發芽 4=成熟；dry 只在「未成熟且已乾」時為真。
fn tile_view(tile: &Tile) -> TileView {
    match tile {
        Tile::Untilled => TileView {
            state: 0,
            dry: false,
        },
        Tile::Tilled => TileView {
            state: 1,
            dry: false,
        },
        Tile::Planted(c) => {
            let state = match c.stage() {
                CropStage::Seed => 2,
                CropStage::Sprout => 3,
                CropStage::Ripe => 4,
            };
            TileView {
                state,
                dry: !c.is_ripe() && c.needs_water(),
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
    fn cell_at_rejects_non_finite_coords() {
        // 客戶端送 NaN / Inf 不該被當成 (0,0)；權威伺服器視為界外。
        assert_eq!(Field::cell_at(f32::NAN, FIELD_ORIGIN_Y), None);
        assert_eq!(Field::cell_at(FIELD_ORIGIN_X, f32::NAN), None);
        assert_eq!(Field::cell_at(f32::INFINITY, f32::INFINITY), None);
        assert_eq!(Field::cell_at(f32::NEG_INFINITY, FIELD_ORIGIN_Y), None);
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

    #[test]
    fn interact_walks_the_care_cycle() {
        let mut f = Field::new();
        // 自然地 → 翻土
        assert_eq!(f.interact(0, 0), FarmOutcome::Tilled);
        assert_eq!(f.tile(0, 0), Some(&Tile::Tilled));
        // 空土 → 播種
        assert_eq!(f.interact(0, 0), FarmOutcome::Planted);
        assert_eq!(f.crop_stage(0, 0), Some(CropStage::Seed));
        // 未熟作物 → 澆水
        assert_eq!(f.interact(0, 0), FarmOutcome::Watered);
        // 長到成熟
        f.tick(MOISTURE_PER_WATER);
        f.interact(0, 0); // 再澆一次
        f.tick(RIPE_AT - MOISTURE_PER_WATER);
        assert_eq!(f.crop_stage(0, 0), Some(CropStage::Ripe));
        // 成熟作物 → 收成拿乙太，回到空土
        assert_eq!(
            f.interact(0, 0),
            FarmOutcome::Harvested(crate::crops::ETHER_PER_HARVEST)
        );
        assert_eq!(f.tile(0, 0), Some(&Tile::Tilled));
    }

    #[test]
    fn interact_out_of_bounds_is_nothing() {
        let mut f = Field::new();
        assert_eq!(f.interact(FIELD_COLS, 0), FarmOutcome::Nothing);
        assert_eq!(f.interact(0, FIELD_ROWS), FarmOutcome::Nothing);
    }

    #[test]
    fn within_reach_inside_field_is_true() {
        // 農地正中央。
        let cx = FIELD_ORIGIN_X + FIELD_COLS as f32 * TILE_SIZE / 2.0;
        let cy = FIELD_ORIGIN_Y + FIELD_ROWS as f32 * TILE_SIZE / 2.0;
        assert!(within_field_reach(cx, cy));
        // 左上角格的中心也算。
        assert!(within_field_reach(FIELD_ORIGIN_X, FIELD_ORIGIN_Y));
    }

    #[test]
    fn within_reach_just_outside_edge_is_true() {
        // 緊貼左緣外 FARM_REACH 內。
        assert!(within_field_reach(
            FIELD_ORIGIN_X - FARM_REACH * 0.5,
            FIELD_ORIGIN_Y + TILE_SIZE
        ));
    }

    #[test]
    fn within_reach_far_away_is_false() {
        // 站在世界另一頭，不能隔空照顧。
        assert!(!within_field_reach(0.0, 0.0));
        let right = FIELD_ORIGIN_X + FIELD_COLS as f32 * TILE_SIZE;
        // 離右緣超過 FARM_REACH。
        assert!(!within_field_reach(right + FARM_REACH * 2.0, FIELD_ORIGIN_Y));
    }

    #[test]
    fn view_reports_origin_size_and_cell_count() {
        let v = Field::new().view();
        assert_eq!(v.origin_x, FIELD_ORIGIN_X);
        assert_eq!(v.origin_y, FIELD_ORIGIN_Y);
        assert_eq!(v.tile_size, TILE_SIZE);
        assert_eq!(v.cols, FIELD_COLS);
        assert_eq!(v.rows, FIELD_ROWS);
        assert_eq!(v.cells.len(), FIELD_COLS * FIELD_ROWS);
        // 全新地每格都是自然地、不需澆水。
        assert!(v.cells.iter().all(|c| c.state == 0 && !c.dry));
    }

    #[test]
    fn view_marks_planted_seed_dry_then_wet() {
        let mut f = Field::new();
        f.till(0, 0);
        f.plant(0, 0);
        // 剛種下、還沒澆水：種子且乾。
        let v = f.view();
        assert_eq!(v.cells[0], TileView { state: 2, dry: true });
        // 澆水後不再標乾。
        f.water(0, 0);
        assert_eq!(f.view().cells[0], TileView { state: 2, dry: false });
    }

    #[test]
    fn view_ripe_crop_is_not_marked_dry() {
        let mut f = Field::new();
        f.till(0, 0);
        f.plant(0, 0);
        f.water(0, 0);
        f.tick(MOISTURE_PER_WATER);
        f.water(0, 0);
        f.tick(RIPE_AT - MOISTURE_PER_WATER);
        // 成熟即使濕度耗盡也不該再叫玩家澆水。
        assert_eq!(f.view().cells[0], TileView { state: 4, dry: false });
    }
}
