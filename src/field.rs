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

use serde::{Deserialize, Serialize};

use crate::crop_variety::CropVariety;
use crate::crops::{Crop, CropStage};
use crate::season::Season;
use crate::protocol::{FieldView, TileView};

/// 每格耕地的邊長（世界像素）。
pub const TILE_SIZE: f32 = 48.0;
/// 農地的欄數（橫向格數）。
pub const FIELD_COLS: usize = 6;
/// 農地的列數（縱向格數）。
pub const FIELD_ROWS: usize = 4;
/// 農地左上角在世界中的位置（像素）。挑在地圖中央附近，讓初來的玩家走幾步就到。
/// 大世界 6000×6000、農地 288×192,**置中於世界中心**(3000,3000)後左上角約在此。
/// 序號 0 的地塊在此(世界中心),其餘玩家的地塊由此一圈圈往外螺旋撒滿大圖(見 plots.rs)。
pub const FIELD_ORIGIN_X: f32 = 2856.0;
pub const FIELD_ORIGIN_Y: f32 = 2904.0;
/// 要照顧農地，玩家至少得站在地塊裡、或緊鄰邊緣這個距離內（像素）。
/// 權威伺服器用它擋掉「人根本不在農地卻送座標隔空遙控」的客戶端。
pub const FARM_REACH: f32 = TILE_SIZE;

/// 一格耕地的狀態。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// 收成了成熟作物，拿到這麼多乙太（已含品質加成＋沃土加成），並帶上這次收成的品質
    /// （ROADMAP 406 用心栽培，供上層演出「優質收成」飄字）與沃土加成乙太
    /// （ROADMAP 438，已含進總乙太裡，供上層演出「🌱 沃土 +N」飄字；0=這格沒養出地力）。
    Harvested(u32, crate::crops::CropQuality, u32),
    /// 沒對應到任何格或無事可做。
    Nothing,
}

/// 一鍵收成（ROADMAP 446）的彙總結果：把整塊田所有成熟作物一次收完後，回報這次共收了
/// 幾株、總共拿到多少乙太（已含品質與沃土加成）、其中沃土加成乙太合計，以及各品質的株數。
/// 供上層演出「🌾 一鍵收成：N 株 +M 乙太」摘要與飄字。純資料、確定性、好測。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HarvestAllSummary {
    /// 這次共收成幾株成熟作物。
    pub count: u32,
    /// 總乙太（已含每株品質加成＋沃土加成；不含 class 收成加成，那由 ws 層另計）。
    pub ether: u32,
    /// 其中沃土加成乙太合計（ROADMAP 438；已含進 `ether`，供演出「🌱 沃土 +N」）。
    pub soil_bonus: u32,
    /// ⭐優質株數（ROADMAP 406）。
    pub premium: u32,
    /// 🌿用心株數。
    pub fine: u32,
    /// 🌾平凡株數。
    pub plain: u32,
}

/// 一塊可擴張農地（row-major 的格子陣列），知道自己在世界裡的左上角 origin。
///
/// 衍生 serde 作為持久化格式地基（接 0-E）：每格 `Tile` 可序列化存回、重啟載入。
/// 列數由 `tiles.len() / FIELD_COLS` 動態決定（初始 `FIELD_ROWS`，`grow()` 每次加一列）；
/// **origin 不入存檔**（`#[serde(skip)]`）——它由地塊序號決定（見 `for_plot`），
/// 載入時由呼叫端依序號重建供入。載入時以 `from_tiles` 驗證格數，壞檔一律拒收。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// 長度為 `FIELD_COLS * actual_rows`，以 row-major index 對映 (col,row)。
    /// 初始 `FIELD_COLS * FIELD_ROWS`；每次 `grow()` 加 `FIELD_COLS` 個 `Untilled` 格。
    tiles: Vec<Tile>,
    /// 這塊地左上角在世界中的座標（像素）。不入存檔；由建構子供入。
    #[serde(skip)]
    origin_x: f32,
    #[serde(skip)]
    origin_y: f32,
    /// 家園擺飾索引（ROADMAP 402）：0=不擺，1..=`home_decor::DECOR_COUNT` 各對應一件療癒小物。
    /// 純裝飾、不影響耕作，由田主自選、訪客看得到。**入存檔**（整塊 `Field` 序列化進既有 `tiles`
    /// 欄，舊存檔無此欄時 `#[serde(default)]` 回 0=不擺，向後相容、免 migration）。
    #[serde(default)]
    home_decor: u8,
    /// 家園庭園的擺放位（ROADMAP 416）：長度至多 `home_decor::GARDEN_SLOTS`，每格一個擺飾索引
    /// （0=該位不擺）。把 402 的「單件」深化成「一座可佈置的庭園」。**入存檔**（整塊 `Field`
    /// 序列化，舊存檔無此欄時 `#[serde(default)]` 回空陣列、向後相容、免 migration；舊存檔只有
    /// `home_decor` 時於 `reseated` 自動升成 slot 0，既有擺飾零損失）。
    #[serde(default)]
    garden: Vec<u8>,
    /// 每格「地力」（ROADMAP 438 沃土輪休），row-major 與 `tiles` 等長、細格點存
    /// （0..=`soil_vitality::SOIL_MAX_FINE`）。翻好卻空著（休耕）的格子隨時間養出地力，
    /// 收成時換成額外乙太、收成後歸零。**入存檔**（整塊 `Field` 序列化；舊存檔無此欄時
    /// `#[serde(default)]` 回空 Vec，由 `ensure_soil` 惰性補零對齊 `tiles`，向後相容、免 migration）。
    #[serde(default)]
    soil: Vec<u16>,
}

impl Field {
    /// 建一塊全是自然地的新農地，落在現有全域農地位置（＝地塊序號 0）。
    pub fn new() -> Self {
        Self::fresh_at(FIELD_ORIGIN_X, FIELD_ORIGIN_Y)
    }

    /// 在任意世界座標蓋一塊全自然地的新農地（公共農地或特殊用途）。
    pub fn at(origin_x: f32, origin_y: f32) -> Self {
        Self::fresh_at(origin_x, origin_y)
    }

    /// 內部共用建構入口：在指定 origin 蓋一塊全自然地的新農地。
    fn fresh_at(origin_x: f32, origin_y: f32) -> Self {
        Self {
            tiles: vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS],
            origin_x,
            origin_y,
            home_decor: 0,
            garden: Vec::new(),
            // 地力惰性配置：留空，首次 tick/harvest 由 `ensure_soil` 補零對齊（與 serde 載入同路徑）。
            soil: Vec::new(),
        }
    }

    /// 建第 `index` 塊地（per-player 用）：origin 由 `plots::plot_origin` 依序號決定，
    /// 一圈一圈往外排、互不重疊；序號 0 正好對齊現有全域農地（與 `new()` 同位置）。
    pub fn for_plot(index: usize) -> Self {
        let (origin_x, origin_y) = crate::plots::plot_origin(index);
        Self::fresh_at(origin_x, origin_y)
    }

    /// 建第 `index` 塊已擴張的地（帶 `expansions` 格擴張，每格 = 多一列 FIELD_COLS 自然地）。
    pub fn for_plot_expanded(index: usize, expansions: u32) -> Self {
        let mut f = Self::for_plot(index);
        for _ in 0..expansions {
            f.grow();
        }
        f
    }

    /// 擴張一列（FIELD_COLS 格自然地）；即 `buy_expansion` 成功後呼叫。
    pub fn grow(&mut self) {
        self.tiles.extend(vec![Tile::Untilled; FIELD_COLS]);
        // 地力陣列若已對齊，同步補上新一列的零地力（新翻自然地從貧土起算）。
        if !self.soil.is_empty() {
            self.soil.resize(self.tiles.len(), 0);
        }
    }

    /// 把地力陣列惰性補齊到與 `tiles` 等長（ROADMAP 438）。舊存檔 / 新建田的 `soil` 為空，
    /// 首次需要讀寫地力前呼叫一次即可——既往格子全當「貧土」（0）起算，向後相容、零 migration。
    fn ensure_soil(&mut self) {
        if self.soil.len() != self.tiles.len() {
            self.soil.resize(self.tiles.len(), 0);
        }
    }

    /// 目前列數（初始 FIELD_ROWS；每次 `grow()` +1）。
    pub fn rows(&self) -> usize {
        self.tiles.len() / FIELD_COLS
    }

    /// 設定家園擺飾（ROADMAP 402）：把玩家選的索引夾成合法值後存下（越界→0=不擺）。
    /// 改的是哪塊地由呼叫端決定（`ws.rs` 只取得玩家自己的田），這裡不做所有權判斷。
    /// ROADMAP 416：legacy 單件入口統一委派到庭園 slot 0——舊前端（只送 `SetHomeDecor`）也會
    /// 寫進新的庭園資料、`home_decor` 永遠等於 `garden[0]`，新舊兩端共用同一份權威狀態。
    pub fn set_home_decor(&mut self, index: u8) {
        self.set_garden_slot(0, index);
    }

    /// 目前的家園擺飾索引（0=不擺）。
    #[cfg(test)]
    pub fn home_decor(&self) -> u8 {
        self.home_decor
    }

    /// 設定家園庭園某個擺放位的小物（ROADMAP 416）：把第 `slot` 格設成 `index`（夾成合法值）。
    /// `slot` 超出 `GARDEN_SLOTS` 一律忽略（防偽造）。內部把 garden 補齊到 `GARDEN_SLOTS` 長以
    /// 便定位寫入，並把 legacy `home_decor` 同步成 slot 0，讓**舊前端**仍看得到第一格那件擺飾。
    /// 改的是哪塊地由呼叫端決定（`ws.rs` 只取得玩家自己的田），這裡不做所有權判斷。
    pub fn set_garden_slot(&mut self, slot: u8, index: u8) {
        let slot = slot as usize;
        if slot >= crate::home_decor::GARDEN_SLOTS {
            return;
        }
        if self.garden.len() < crate::home_decor::GARDEN_SLOTS {
            self.garden.resize(crate::home_decor::GARDEN_SLOTS, 0);
        }
        self.garden[slot] = crate::home_decor::sanitize(index);
        // legacy 同步：舊前端只讀 home_decor，讓它對齊 slot 0（向後相容、不丟第一格那件）。
        self.home_decor = self.garden.first().copied().unwrap_or(0);
    }

    /// 目前的家園庭園擺放位（ROADMAP 416）。全空時回空切片（前端／快照據此略過繪製）。
    pub fn garden(&self) -> &[u8] {
        &self.garden
    }

    /// 這塊地左上角在世界中的座標（像素）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn origin(&self) -> (f32, f32) {
        (self.origin_x, self.origin_y)
    }

    /// 從存檔的格子陣列重建農地（持久化載入入口）。兩道防線都過才收：
    /// 其一，格數必須是 FIELD_COLS 的倍數且 ≥ 初始格數（支援已擴張的農地）；
    /// 其二，每株作物成長值都得健全，擋壞檔 / 被竄改的存檔。
    /// 任一不符回 `None`，呼叫端可退回全新地。
    pub fn from_tiles(index: usize, tiles: Vec<Tile>) -> Option<Self> {
        let min = FIELD_COLS * FIELD_ROWS;
        let max = FIELD_COLS * (FIELD_ROWS + crate::economy::MAX_EXPANSIONS as usize);
        if tiles.len() < min || tiles.len() > max || tiles.len() % FIELD_COLS != 0 {
            return None;
        }
        if tiles
            .iter()
            .any(|t| matches!(t, Tile::Planted(c) if !c.is_loadable()))
        {
            return None;
        }
        let (origin_x, origin_y) = crate::plots::plot_origin(index);
        Some(Self {
            tiles,
            origin_x,
            origin_y,
            home_decor: 0,
            garden: Vec::new(),
            soil: Vec::new(),
        })
    }

    /// 把（serde 還原後 origin 退回 (0,0) 的）農地安置回第 `index` 塊地——持久化載入入口。
    /// 驗證不過（壞檔）回 `None`，呼叫端可退回全新地。
    /// `from_tiles` 只用 `tiles` 重建（origin 由序號決定），故這裡要把擺飾索引接回去——
    /// 否則重啟載入會把玩家擺好的家園擺飾默默清掉。順手 `sanitize` 防壞檔塞髒索引。
    pub fn reseated(self, index: usize) -> Option<Self> {
        let decor = crate::home_decor::sanitize(self.home_decor);
        // garden 也要一起接回（from_tiles 只搬 tiles），載入時順手夾合法。
        let mut garden = self.garden;
        crate::home_decor::sanitize_garden(&mut garden);
        // ROADMAP 438：地力陣列也要接回（from_tiles 只搬 tiles），否則重啟會把養好的地默默清掉。
        let soil = self.soil;
        Self::from_tiles(index, self.tiles).map(|mut f| {
            f.home_decor = decor;
            f.garden = garden;
            // 接回地力：長度若與 tiles 對不上（舊存檔空 Vec／壞檔）一律當貧土重來（ensure_soil 補零）。
            if soil.len() == f.tiles.len() {
                f.soil = soil;
            }
            // 舊存檔遷移（ROADMAP 416）：只有 legacy home_decor、還沒有庭園資料時，把那一件
            // 升成 slot 0，玩家原本擺好的擺飾零損失。set_garden_slot 會同步把 home_decor 設回。
            if decor > 0 && f.garden.iter().all(|&x| x == 0) {
                f.set_garden_slot(0, decor);
            }
            f
        })
    }

    /// (col,row) → tiles 陣列索引；超出範圍（含超過目前列數）回 `None`。
    fn index_at(&self, col: usize, row: usize) -> Option<usize> {
        if col < FIELD_COLS && row < self.rows() {
            Some(row * FIELD_COLS + col)
        } else {
            None
        }
    }

    /// 世界座標 (x,y) → 落在這塊地的哪一格 (col,row)；不在範圍內回 `None`。
    /// 以這塊地自己的 origin 為原點（per-player：各塊地在世界不同位置）。
    pub fn cell_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        // 先擋非有限座標：客戶端可能送 NaN / Inf，而 `NaN < 0.0` 為 false 不會被下面
        // 的範圍檢查擋下，且 `(NaN / TILE_SIZE) as usize` 在 Rust 飽和轉型成 0，會讓
        // 垃圾座標誤落到 (0,0) 格。權威伺服器一律視為界外。
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let local_x = x - self.origin_x;
        let local_y = y - self.origin_y;
        if local_x < 0.0 || local_y < 0.0 {
            return None;
        }
        let col = (local_x / TILE_SIZE) as usize;
        let row = (local_y / TILE_SIZE) as usize;
        if col < FIELD_COLS && row < self.rows() {
            Some((col, row))
        } else {
            None
        }
    }

    /// 讀某格狀態（唯讀）；超出範圍回 `None`。
    /// 生產端走 `view()` 取整塊地，單格查詢目前只用於測試斷言。
    #[cfg(test)]
    pub fn tile(&self, col: usize, row: usize) -> Option<&Tile> {
        self.index_at(col, row).map(|i| &self.tiles[i])
    }

    /// 某格作物目前的成長階段；該格沒種東西或超出範圍回 `None`。
    #[cfg(test)]
    pub fn crop_stage(&self, col: usize, row: usize) -> Option<CropStage> {
        match self.tile(col, row) {
            Some(Tile::Planted(c)) => Some(c.stage()),
            _ => None,
        }
    }

    /// 蜜源：田裡正在生長中（已種、未收成）的作物格數。
    /// 養蜂釀蜜（ROADMAP 412）：蜂巢產蜜速率隨自家田裡的蜜源放大——種得越多、蜜釀越快。
    pub fn blooming_count(&self) -> u32 {
        self.tiles
            .iter()
            .filter(|t| matches!(t, Tile::Planted(_)))
            .count() as u32
    }

    /// 翻土：只有自然地能翻成空土。成功回 `true`，否則（已翻 / 已種 / 越界）回 `false`。
    pub fn till(&mut self, col: usize, row: usize) -> bool {
        match self.index_at(col, row) {
            Some(i) if self.tiles[i] == Tile::Untilled => {
                self.tiles[i] = Tile::Tilled;
                true
            }
            _ => false,
        }
    }

    /// 播種（預設品種主食穀）：只有翻好的空土能播。成功回 `true`，否則回 `false`。
    /// 保留既有無品種入口供內部／測試沿用；要指定品種走 `plant_kind`。
    pub fn plant(&mut self, col: usize, row: usize) -> bool {
        self.plant_kind(col, row, CropVariety::default())
    }

    /// 播種指定品種（ROADMAP 452）：只有翻好的空土能播。除品種外與 `plant` 完全相同。
    pub fn plant_kind(&mut self, col: usize, row: usize, kind: CropVariety) -> bool {
        match self.index_at(col, row) {
            Some(i) if self.tiles[i] == Tile::Tilled => {
                self.tiles[i] = Tile::Planted(Crop::plant_kind(kind));
                true
            }
            _ => false,
        }
    }

    /// 澆水：只有種了作物的格能澆。成功回 `true`，否則回 `false`。
    pub fn water(&mut self, col: usize, row: usize) -> bool {
        match self.index_at(col, row) {
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
    /// ROADMAP 406：回傳「乙太（已含品質加成）＋品質」——品質由成長期是否用心照顧決定。
    /// ROADMAP 438：回傳值多帶「沃土加成乙太」——這格休耕養出的地力換成的額外乙太
    /// （已含進總乙太），供上層演出「🌱 沃土 +N」飄字；收成後該格地力歸零（被作物吸收）。
    pub fn harvest(&mut self, col: usize, row: usize) -> Option<(u32, crate::crops::CropQuality, u32)> {
        let i = self.index_at(col, row)?;
        self.ensure_soil();
        if let Tile::Planted(c) = &mut self.tiles[i] {
            // 收成前先讀品質（harvest 會把這株重置，事後就讀不到了）。
            let quality = c.quality();
            // 先借出可變參考收成；成熟才會回 Some 並消費這格。
            if let Some(base) = c.harvest() {
                // ROADMAP 438：把這格累積的地力換成額外乙太（純正向、永不倒扣），收成後歸零。
                let soil_bonus = crate::soil_vitality::harvest_bonus(self.soil[i]);
                self.soil[i] = 0;
                // 收成後不留新種子，回到空土讓玩家自行決定要不要再種（也從零重新養地）。
                self.tiles[i] = Tile::Tilled;
                return Some((base + quality.ether_bonus() + soil_bonus, quality, soil_bonus));
            }
        }
        None
    }

    /// 把每格「是否種了作物」攤成 row-major 的佔用遮罩（連片沃土判定的輸入）。純函式。
    fn planted_mask(&self) -> Vec<bool> {
        self.tiles
            .iter()
            .map(|t| matches!(t, Tile::Planted(_)))
            .collect()
    }

    /// 推進 `dt` 秒：讓地裡所有作物成長（無濕度的不會長，見 `Crop::grow`）。
    /// ROADMAP 367 連片沃土：先算「哪些格屬於連片田畝」，連片格的作物以
    /// `THRIVE_GROWTH_MULT` 加速成長（濕度仍按真實 `dt` 消耗，見 `Crop::grow_boosted`）。
    /// ROADMAP 453 作物品種季節偏好：每株再依「品種 × 當季」疊一層偏好倍率
    /// （`CropVariety::season_affinity`，主食穀恆 1.0＝向後相容；速生菜耐寒、乙太瓜戀夏畏寒）。
    /// `season` 是世界當前季節（呼叫端 game.rs 從 `app.season` 取得）。
    pub fn tick(&mut self, dt: f32, season: Season) {
        let thriving = crate::field_thrive::thriving_mask(
            &self.planted_mask(),
            FIELD_COLS,
            crate::field_thrive::THRIVE_MIN_PATCH,
        );
        self.ensure_soil();
        for (i, t) in self.tiles.iter_mut().enumerate() {
            match t {
                Tile::Planted(c) => {
                    let patch_mult = if thriving[i] {
                        crate::field_thrive::THRIVE_GROWTH_MULT
                    } else {
                        1.0
                    };
                    // 品種 × 當季偏好：與連片沃土倍率正交、各自獨立疊乘。
                    let mult = patch_mult * c.kind().season_affinity(season);
                    c.grow_boosted(dt, mult);
                    // 種了作物：地力「鎖住」不再累積（待收成兌現），也不流失。
                }
                // ROADMAP 438：空翻好土歇著就慢慢養出地力（休耕養地）。
                // 自然地（Untilled）尚未開墾、不養地（維持 0）。
                Tile::Tilled => {
                    self.soil[i] = crate::soil_vitality::accrue(self.soil[i], dt);
                }
                Tile::Untilled => {}
            }
        }
    }

    /// 降雨自動澆灌／一鍵澆水：對所有缺水的作物格補滿濕度，回傳「實際澆到」的格數。
    /// （ROADMAP 109 降雨呼叫此函式只圖補水、不看回傳；ROADMAP 422 一鍵澆水用回傳值
    /// 告訴玩家澆了幾株。）呼叫前不需判斷生態域——由呼叫端負責決定要不要澆。
    pub fn water_all_planted(&mut self) -> u32 {
        let mut watered = 0;
        for t in &mut self.tiles {
            if let Tile::Planted(c) = t {
                if c.needs_water() {
                    c.water();
                    watered += 1;
                }
            }
        }
        watered
    }

    /// 一鍵收成（ROADMAP 446）：把整塊田所有「已成熟」的作物一次收完，回傳彙總
    /// （株數／總乙太／沃土加成／各品質株數）。逐格沿用與 `harvest` 完全相同的結算
    /// （品質加成＋沃土加成＋收成後歸零地力＋格子回到空土），故與「逐格手收 N 次」在
    /// 經濟上完全等價——只是省去逐格點擊（對稱於 422 一鍵澆水）。未成熟的格略過、不受影響。
    /// 純邏輯、無 IO；class 收成加成由 ws 層按 `count` 另計（與單格收成一致）。
    pub fn harvest_all_ripe(&mut self) -> HarvestAllSummary {
        self.ensure_soil();
        let mut s = HarvestAllSummary::default();
        for i in 0..self.tiles.len() {
            if let Tile::Planted(c) = &mut self.tiles[i] {
                // 收成前先讀品質（harvest 會把這株重置，事後就讀不到了）。
                let quality = c.quality();
                // 只有成熟才回 Some 並消費這格；未熟回 None、原樣留著。
                if let Some(base) = c.harvest() {
                    let soil_bonus = crate::soil_vitality::harvest_bonus(self.soil[i]);
                    self.soil[i] = 0;
                    self.tiles[i] = Tile::Tilled;
                    let gained = base
                        .saturating_add(quality.ether_bonus())
                        .saturating_add(soil_bonus);
                    s.count = s.count.saturating_add(1);
                    s.ether = s.ether.saturating_add(gained);
                    s.soil_bonus = s.soil_bonus.saturating_add(soil_bonus);
                    match quality {
                        crate::crops::CropQuality::Premium => s.premium = s.premium.saturating_add(1),
                        crate::crops::CropQuality::Fine => s.fine = s.fine.saturating_add(1),
                        crate::crops::CropQuality::Plain => s.plain = s.plain.saturating_add(1),
                    }
                }
            }
        }
        s
    }

    /// 「一鍵照顧」（預設品種主食穀）：保留既有無品種入口供內部／測試沿用。
    /// 要在播種時指定品種（ROADMAP 452）走 `interact_kind`。
    pub fn interact(&mut self, col: usize, row: usize) -> FarmOutcome {
        self.interact_kind(col, row, CropVariety::default())
    }

    /// 「一鍵照顧」：依某格目前狀態自動決定要做什麼，並執行：
    /// 自然地→翻土、空土→播種（種下 `kind` 品種，ROADMAP 452）、未熟作物→澆水、成熟作物→收成。
    /// `kind` 只在「空土→播種」這步用到，其餘動作忽略它。
    /// 越界回 `Nothing`。把「該做哪個動作」的判斷集中在這裡，前端只要送座標＋想種的品種。
    pub fn interact_kind(&mut self, col: usize, row: usize, kind: CropVariety) -> FarmOutcome {
        let Some(i) = self.index_at(col, row) else {
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
                self.plant_kind(col, row, kind);
                FarmOutcome::Planted
            }
            Act::Water => {
                self.water(col, row);
                FarmOutcome::Watered
            }
            Act::Harvest => match self.harvest(col, row) {
                Some((ether, quality, soil_bonus)) => {
                    FarmOutcome::Harvested(ether, quality, soil_bonus)
                }
                None => FarmOutcome::Nothing,
            },
        }
    }

    /// 把整塊地轉成給前端的可見快照（origin 用這塊地自己的，前端據此畫在世界對的位置）。
    /// `owner` 先填 `nil`——`Field` 本身不知道自己屬於誰；由廣播層在送出快照前戳上。
    pub fn view(&self) -> FieldView {
        FieldView {
            owner: uuid::Uuid::nil(),
            origin_x: self.origin_x,
            origin_y: self.origin_y,
            tile_size: TILE_SIZE,
            cols: FIELD_COLS,
            rows: self.rows(),
            reach: FARM_REACH,
            // ROADMAP 367：每格帶上「是否屬於連片沃土」，前端把連片田畝畫得更蒼翠。
            cells: {
                let thriving = crate::field_thrive::thriving_mask(
                    &self.planted_mask(),
                    FIELD_COLS,
                    crate::field_thrive::THRIVE_MIN_PATCH,
                );
                self.tiles
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        // ROADMAP 438：地力顯示等級（0~3）。soil 可能尚未惰性配置（空）→當貧土 0。
                        let soil_level = crate::soil_vitality::display_level(
                            self.soil.get(i).copied().unwrap_or(0),
                        );
                        tile_view(t, thriving[i], soil_level)
                    })
                    .collect()
            },
            // ROADMAP 402：帶上家園擺飾索引，前端依此在田上畫對應小物（0=不擺則不畫）。
            home_decor: self.home_decor,
            // ROADMAP 416：帶上整座庭園的擺放位；全空則略去省流量（skip_serializing_if）。
            garden: if self.garden.iter().any(|&x| x != 0) {
                self.garden.clone()
            } else {
                Vec::new()
            },
        }
    }
}

impl Field {
    /// 玩家位於 (px,py) 時，是否近到能在**這塊地**上操作（在地塊內、或離邊緣
    /// `FARM_REACH` 內）。量的是「點到這塊地矩形的最近距離」，所以站在地塊任一處
    /// 都算，不必貼著某一格。以這塊地自己的 origin 為基準（per-player：各塊地各算各的）。
    pub fn within_reach(&self, px: f32, py: f32) -> bool {
        let right = self.origin_x + FIELD_COLS as f32 * TILE_SIZE;
        let bottom = self.origin_y + self.rows() as f32 * TILE_SIZE;
        // 把玩家座標夾到農地矩形上，得到矩形上的最近點。
        let nx = px.clamp(self.origin_x, right);
        let ny = py.clamp(self.origin_y, bottom);
        let dx = px - nx;
        let dy = py - ny;
        dx * dx + dy * dy <= FARM_REACH * FARM_REACH
    }
}

/// 一格 → 前端可見狀態。純函式。
/// state：0=自然地 1=空土 2=種子 3=發芽 4=成熟；dry 只在「未成熟且已乾」時為真；
/// thriving 在「這格屬於連片沃土（ROADMAP 367）」時為真（由 `view()` 算好整片再傳入）；
/// quality（ROADMAP 406）只在「成熟」時有意義：0=平凡 1=用心 2=優質，前端據此在成熟作物上
/// 畫品質光點，讓「用心照顧」在收成前就一眼看得見。
/// soil（ROADMAP 438）：這格的地力顯示等級（0~3），只在「空翻好土（state 1）」時有意義
/// ——讓玩家一眼看出哪幾格歇夠了、種下去更甜。自然地與種了作物的格一律 0（不顯地力）。
fn tile_view(tile: &Tile, thriving: bool, soil: u8) -> TileView {
    match tile {
        Tile::Untilled => TileView {
            state: 0,
            dry: false,
            thriving: false,
            quality: 0,
            grow: 0,
            soil: 0,
            kind: 0,
        },
        Tile::Tilled => TileView {
            state: 1,
            dry: false,
            thriving: false,
            quality: 0,
            grow: 0,
            soil,
            kind: 0,
        },
        Tile::Planted(c) => {
            let ripe = c.is_ripe();
            let state = match c.stage() {
                CropStage::Seed => 2,
                CropStage::Sprout => 3,
                CropStage::Ripe => 4,
            };
            TileView {
                state,
                dry: !ripe && c.needs_water(),
                thriving,
                // 只有成熟作物才把品質顯給前端；未熟時品質尚未定（渴秒數還在累積）。
                quality: if ripe { c.quality().code() } else { 0 },
                // ROADMAP 421：成長中作物的熟成進度百分比（0~100）；成熟一律 100，給前端畫進度條。
                grow: (c.progress() * 100.0).round() as u8,
                // 種了作物的格不顯地力（地力已鎖住、待收成兌現；視覺焦點留給作物本身）。
                soil: 0,
                // ROADMAP 452：作物品種碼，供前端畫品種微異／面板顯示。
                kind: c.kind().code(),
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
        let f = Field::new();
        assert_eq!(f.cell_at(FIELD_ORIGIN_X, FIELD_ORIGIN_Y), Some((0, 0)));
    }

    #[test]
    fn cell_at_maps_within_tile_to_same_cell() {
        // 第 (1,2) 格的中心點應落回該格。
        let f = Field::new();
        let x = FIELD_ORIGIN_X + 1.0 * TILE_SIZE + TILE_SIZE / 2.0;
        let y = FIELD_ORIGIN_Y + 2.0 * TILE_SIZE + TILE_SIZE / 2.0;
        assert_eq!(f.cell_at(x, y), Some((1, 2)));
    }

    #[test]
    fn cell_at_is_none_outside_field() {
        let f = Field::new();
        // 左上之外
        assert_eq!(f.cell_at(FIELD_ORIGIN_X - 1.0, FIELD_ORIGIN_Y), None);
        assert_eq!(f.cell_at(FIELD_ORIGIN_X, FIELD_ORIGIN_Y - 1.0), None);
        // 右下之外
        let far_x = FIELD_ORIGIN_X + FIELD_COLS as f32 * TILE_SIZE;
        let far_y = FIELD_ORIGIN_Y + FIELD_ROWS as f32 * TILE_SIZE;
        assert_eq!(f.cell_at(far_x, far_y), None);
    }

    #[test]
    fn cell_at_rejects_non_finite_coords() {
        // 客戶端送 NaN / Inf 不該被當成 (0,0)；權威伺服器視為界外。
        let f = Field::new();
        assert_eq!(f.cell_at(f32::NAN, FIELD_ORIGIN_Y), None);
        assert_eq!(f.cell_at(FIELD_ORIGIN_X, f32::NAN), None);
        assert_eq!(f.cell_at(f32::INFINITY, f32::INFINITY), None);
        assert_eq!(f.cell_at(f32::NEG_INFINITY, FIELD_ORIGIN_Y), None);
    }

    /// 序號 0 的地塊（`for_plot(0)`）與全域 `new()` 同位置——接線時第一個玩家無縫接續。
    #[test]
    fn for_plot_zero_matches_new_origin() {
        assert_eq!(Field::for_plot(0).origin(), Field::new().origin());
        assert_eq!(Field::new().origin(), (FIELD_ORIGIN_X, FIELD_ORIGIN_Y));
    }

    /// `for_plot(index)` 的 origin 必須等於 `plots::plot_origin(index)`（單一真實來源）。
    #[test]
    fn for_plot_origin_follows_plots_geometry() {
        for index in 0..8 {
            assert_eq!(
                Field::for_plot(index).origin(),
                crate::plots::plot_origin(index),
                "序號 {index} 的 Field origin 與 plots 幾何不一致"
            );
        }
    }

    /// 另一塊地（序號 1）的 `cell_at` / `within_reach` 都以**它自己**的 origin 為基準：
    /// 全域農地座標在它眼裡是界外，它自己的 origin 才落 (0,0)、站它上面才搆得到。
    #[test]
    fn cell_at_and_reach_are_relative_to_plot_origin() {
        let f = Field::for_plot(1);
        let (ox, oy) = f.origin();
        // 自己的 origin 落第一格、站上面搆得到。
        assert_eq!(f.cell_at(ox, oy), Some((0, 0)));
        assert!(f.within_reach(ox, oy));
        // 全域農地（序號 0）的位置在序號 1 這塊地眼裡是界外、也搆不到。
        assert_eq!(f.cell_at(FIELD_ORIGIN_X, FIELD_ORIGIN_Y), None);
        assert!(!f.within_reach(FIELD_ORIGIN_X, FIELD_ORIGIN_Y));
    }

    /// per-player 歸屬的招牌保證（鏡像 ws `Farm` 接線的核心）：玩家只對「自己這塊地」
    /// 算格，送來落在**別塊地**的世界座標一律 `cell_at → None`，於是 ws 端走不到
    /// `interact`——動不到別人的地，不必額外存一張「座標→地主」表，由幾何建構性保證。
    #[test]
    fn coords_in_another_plot_map_to_no_cell_on_my_field() {
        let mine = Field::for_plot(0);
        let other = Field::for_plot(1);
        let (ox, oy) = other.origin();
        // 別人那塊地的左上角、與其中央，在「我這塊」眼裡都是界外。
        assert_eq!(mine.cell_at(ox, oy), None);
        let cx = ox + FIELD_COLS as f32 * TILE_SIZE / 2.0;
        let cy = oy + FIELD_ROWS as f32 * TILE_SIZE / 2.0;
        assert_eq!(mine.cell_at(cx, cy), None);
        // 反向：我自己這塊的座標當然算得到格（確認上面的 None 不是因為整塊都壞）。
        let (mx, my) = mine.origin();
        assert_eq!(mine.cell_at(mx, my), Some((0, 0)));
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
        f.tick(MOISTURE_PER_WATER, Season::Summer);
        assert!(f.water(0, 0));
        f.tick(RIPE_AT - MOISTURE_PER_WATER, Season::Summer);
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

    // ── ROADMAP 438 沃土輪休 ──────────────────────────────────────────────

    /// 收成一格成熟作物的便捷流程（翻土→播種→澆兩次水→長熟→收成），回傳收成結果。
    /// `rest_before_plant` ＝播種前讓這格空翻好土休耕多少秒（養地）。
    fn grow_and_harvest(f: &mut Field, col: usize, row: usize, rest_before_plant: f32) -> FarmOutcome {
        f.till(col, row);
        if rest_before_plant > 0.0 {
            f.tick(rest_before_plant, Season::Summer); // 空翻好土休耕養地
        }
        f.plant(col, row);
        f.water(col, row);
        f.tick(MOISTURE_PER_WATER, Season::Summer);
        f.water(col, row);
        f.tick(RIPE_AT - MOISTURE_PER_WATER, Season::Summer);
        f.interact(col, row) // 成熟 → 收成
    }

    #[test]
    fn fallow_rest_pays_off_at_harvest() {
        // 休耕養滿地力（≥150 秒）的格子，收成會多拿沃土加成乙太；剛翻就種的不會。
        let mut rested = Field::new();
        let out_rested = grow_and_harvest(&mut rested, 0, 0, 160.0);
        let mut hasty = Field::new();
        let out_hasty = grow_and_harvest(&mut hasty, 0, 0, 0.0);
        match (out_rested, out_hasty) {
            (
                FarmOutcome::Harvested(ether_r, _, bonus_r),
                FarmOutcome::Harvested(ether_h, _, bonus_h),
            ) => {
                assert_eq!(bonus_h, 0, "剛翻就種、沒休耕 → 無沃土加成");
                assert_eq!(bonus_r, crate::soil_vitality::MAX_BONUS, "養滿地力 → 拿滿沃土加成");
                assert!(ether_r > ether_h, "休耕養地的收成應比急著種的多（純正向）");
                assert_eq!(ether_r - ether_h, bonus_r, "兩者差額恰為沃土加成（其餘條件相同）");
            }
            other => panic!("應為兩筆收成，得到 {other:?}"),
        }
    }

    #[test]
    fn harvest_resets_soil_so_it_must_rest_again() {
        // 收成後地力歸零：同格立刻再種一輪（中間不休耕）就拿不到沃土加成，得重新養。
        let mut f = Field::new();
        let first = grow_and_harvest(&mut f, 0, 0, 160.0);
        assert!(
            matches!(first, FarmOutcome::Harvested(_, _, b) if b == crate::soil_vitality::MAX_BONUS),
            "第一輪養滿地力應拿滿加成"
        );
        // 收成後該格回到空土、地力歸零；不再休耕、立刻播種再收。
        f.plant(0, 0);
        f.water(0, 0);
        f.tick(MOISTURE_PER_WATER, Season::Summer);
        f.water(0, 0);
        f.tick(RIPE_AT - MOISTURE_PER_WATER, Season::Summer);
        let second = f.interact(0, 0);
        assert!(
            matches!(second, FarmOutcome::Harvested(_, _, 0)),
            "收成後地力歸零、立刻再種 → 無沃土加成（得重新休耕養地），得到 {second:?}"
        );
    }

    #[test]
    fn planted_cell_does_not_accrue_soil() {
        // 種了作物的格在成長期不養地：種下後 tick 一大段時間，收成仍無沃土加成。
        let mut f = Field::new();
        f.till(0, 0);
        f.plant(0, 0); // 立刻種、之後整段時間都「種著」
        f.water(0, 0);
        f.tick(MOISTURE_PER_WATER, Season::Summer);
        f.water(0, 0);
        f.tick(RIPE_AT - MOISTURE_PER_WATER + 300.0, Season::Summer); // 多跑 300 秒（作物期不累積地力）
        let out = f.interact(0, 0);
        assert!(
            matches!(out, FarmOutcome::Harvested(_, _, 0)),
            "作物成長期不養地 → 無沃土加成，得到 {out:?}"
        );
    }

    #[test]
    fn soil_survives_serde_round_trip() {
        // 養出地力的田序列化再還原，地力原封不動（持久化格式涵蓋 soil 欄）。
        let mut f = Field::new();
        f.till(0, 0);
        f.tick(80.0, Season::Summer); // 養一點地力
        let json = serde_json::to_string(&f).unwrap();
        let back: Field = serde_json::from_str(&json).unwrap();
        assert_eq!(back.soil, f.soil, "地力欄應隨整塊地序列化往返保留");
        assert!(back.soil[0] > 0, "休耕養出的地力應被保存");
    }

    #[test]
    fn old_save_without_soil_field_loads_as_barren() {
        // 舊存檔沒有 soil 欄：serde default 回空 Vec，惰性補零當貧土，向後相容、不 panic。
        let mut f = Field::new();
        f.till(0, 0);
        // 模擬舊存檔：手動移除 soil 欄。
        let mut v: serde_json::Value = serde_json::to_value(&f).unwrap();
        v.as_object_mut().unwrap().remove("soil");
        let back: Field = serde_json::from_value(v).unwrap();
        assert!(back.soil.is_empty(), "舊存檔無 soil 欄 → default 空 Vec");
        // view() 對空 soil 安全（當貧土 0），不 panic。
        let view = back.view();
        assert_eq!(view.cells[0].soil, 0);
    }

    #[test]
    fn tick_only_grows_watered_crops() {
        let mut f = Field::new();
        f.till(0, 0);
        f.plant(0, 0); // 沒澆水
        f.till(1, 0);
        f.plant(1, 0);
        f.water(1, 0); // 澆了水
        // 預設品種＝主食穀，季節偏好恆 1.0，故任一季節 tick 行為與改動前一致。
        f.tick(SPROUT_AT, Season::Summer);
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
        f.tick(MOISTURE_PER_WATER, Season::Summer);
        f.interact(0, 0); // 再澆一次
        f.tick(RIPE_AT - MOISTURE_PER_WATER, Season::Summer);
        assert_eq!(f.crop_stage(0, 0), Some(CropStage::Ripe));
        // 成熟作物 → 收成拿乙太，回到空土。全程不讓它渴＝優質收成（ROADMAP 406），
        // 乙太＝基礎＋優質加成。
        assert_eq!(
            f.interact(0, 0),
            // 翻土後立刻播種、整個成長期作物都「種著」，這格從沒休耕過 → 地力 0、沃土加成 0。
            FarmOutcome::Harvested(
                crate::crops::ETHER_PER_HARVEST + crate::crops::ETHER_BONUS_PREMIUM,
                crate::crops::CropQuality::Premium,
                0,
            )
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
        let f = Field::new();
        // 農地正中央。
        let cx = FIELD_ORIGIN_X + FIELD_COLS as f32 * TILE_SIZE / 2.0;
        let cy = FIELD_ORIGIN_Y + FIELD_ROWS as f32 * TILE_SIZE / 2.0;
        assert!(f.within_reach(cx, cy));
        // 左上角格的中心也算。
        assert!(f.within_reach(FIELD_ORIGIN_X, FIELD_ORIGIN_Y));
    }

    #[test]
    fn within_reach_just_outside_edge_is_true() {
        let f = Field::new();
        // 緊貼左緣外 FARM_REACH 內。
        assert!(f.within_reach(
            FIELD_ORIGIN_X - FARM_REACH * 0.5,
            FIELD_ORIGIN_Y + TILE_SIZE
        ));
    }

    #[test]
    fn within_reach_far_away_is_false() {
        let f = Field::new();
        // 站在世界另一頭，不能隔空照顧。
        assert!(!f.within_reach(0.0, 0.0));
        let right = FIELD_ORIGIN_X + FIELD_COLS as f32 * TILE_SIZE;
        // 離右緣超過 FARM_REACH。
        assert!(!f.within_reach(right + FARM_REACH * 2.0, FIELD_ORIGIN_Y));
    }

    #[test]
    fn view_reports_origin_size_and_cell_count() {
        let v = Field::new().view();
        assert_eq!(v.origin_x, FIELD_ORIGIN_X);
        assert_eq!(v.origin_y, FIELD_ORIGIN_Y);
        assert_eq!(v.tile_size, TILE_SIZE);
        assert_eq!(v.cols, FIELD_COLS);
        assert_eq!(v.rows, FIELD_ROWS);
        // 照顧距離跟著快照帶給前端，與伺服器權威常數一致（避免前後端各定一套）。
        assert_eq!(v.reach, FARM_REACH);
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
        assert_eq!(v.cells[0], TileView { state: 2, dry: true, thriving: false, quality: 0, grow: 0, soil: 0, kind: 0 });
        // 澆水後不再標乾。
        f.water(0, 0);
        assert_eq!(f.view().cells[0], TileView { state: 2, dry: false, thriving: false, quality: 0, grow: 0, soil: 0, kind: 0 });
    }

    #[test]
    fn serialized_field_round_trips_mid_growth() {
        // 持久化格式地基：一塊「正種到一半」的農地序列化再讀回，要一模一樣——
        // 尤其是還沒到下一階段、成長/濕度都在中段的作物（重啟後要能原地接續長，
        // 而不是被四捨五入到某個階段）。
        let mut f = Field::new();
        f.till(0, 0); // 空土
        f.till(1, 0);
        f.plant(1, 0); // 剛種、還沒澆水的乾種子
        f.till(2, 0);
        f.plant(2, 0);
        f.water(2, 0);
        // 發芽、濕度也消耗到一半，停在階段中段；留 (3,0) 為自然地當對照。
        f.tick(SPROUT_AT + 5.0, Season::Summer);

        let json = serde_json::to_string(&f).unwrap();
        // origin 刻意不入存檔（`#[serde(skip)]`）：載入時由該玩家的序號重建供入，
        // 不靠磁碟值。所以還原走 `from_tiles(index, tiles)`，origin 來自序號。
        // 這裡先確認單純 serde 還原把 tiles 原封不動帶回（origin 退回預設 0,0）。
        let raw: Field = serde_json::from_str(&json).unwrap();
        assert_eq!(raw.origin(), (0.0, 0.0));
        // from_tiles 只搬 tiles（origin 由序號重建；home_decor/garden/soil 等附屬欄由 reseated 接回）。
        let bare = Field::from_tiles(0, raw.tiles.clone()).unwrap();
        assert_eq!(bare.tiles, f.tiles, "tiles（含中段 growth/moisture）原封不動");
        // 真正的載入入口 reseated：整塊地（含 ROADMAP 438 地力）配上序號 0 重建，一模一樣。
        let back = raw.reseated(0).unwrap();
        assert_eq!(back, f); // 整塊地（含中段的 growth/moisture、地力）＋ origin 原封不動
                             // 階段也跟著保留。
        assert_eq!(back.tile(0, 0), Some(&Tile::Tilled));
        assert_eq!(back.crop_stage(1, 0), Some(CropStage::Seed));
        assert_eq!(back.crop_stage(2, 0), Some(CropStage::Sprout));
        assert_eq!(back.tile(3, 0), Some(&Tile::Untilled));
    }

    #[test]
    fn reseated_round_trips_through_serde_with_origin_from_index() {
        // 持久化載入入口:一塊種到一半的地序列化 →(origin 退回 0,0)→ reseated(index) 安置回
        // 該序號的 origin,整塊地(含中段 growth/moisture)原封不動。與 from_tiles 同驗證,
        // 但走「整個 Field」進出,鏡像 field_store 的實際載入路徑。
        let mut f = Field::for_plot(2);
        f.till(0, 0);
        f.plant(0, 0);
        f.water(0, 0);
        f.tick(SPROUT_AT + 3.0, Season::Summer);

        let json = serde_json::to_string(&f).unwrap();
        let raw: Field = serde_json::from_str(&json).unwrap();
        assert_eq!(raw.origin(), (0.0, 0.0)); // origin 不入存檔
        let back = raw.reseated(2).unwrap();
        assert_eq!(back, f); // 序號 2 的 origin 重建後整塊一致
        assert_eq!(back.origin(), crate::plots::plot_origin(2));
    }

    #[test]
    fn home_decor_persists_through_serde_and_reseat() {
        // ROADMAP 402：擺飾索引入存檔（整塊 Field 序列化），且 reseated 會把它接回——
        // 否則重啟載入會把玩家擺好的擺飾清掉。預設 0、設了之後序列化進出仍在。
        let mut f = Field::for_plot(2);
        assert_eq!(f.home_decor(), 0, "新地預設不擺");
        f.set_home_decor(3);
        let json = serde_json::to_string(&f).unwrap();
        let back = serde_json::from_str::<Field>(&json).unwrap().reseated(2).unwrap();
        assert_eq!(back.home_decor(), 3, "擺飾索引須撐過序列化＋reseat");
        assert_eq!(back, f);
    }

    #[test]
    fn set_home_decor_sanitizes_out_of_range() {
        // 越界索引（偽造／髒值）一律當「不擺」處理，不被默默解讀成某件真擺飾。
        let mut f = Field::new();
        f.set_home_decor(crate::home_decor::DECOR_COUNT); // 邊界合法值
        assert_eq!(f.home_decor(), crate::home_decor::DECOR_COUNT);
        f.set_home_decor(250); // 越界
        assert_eq!(f.home_decor(), 0);
    }

    #[test]
    fn old_save_without_decor_field_defaults_to_none() {
        // 向後相容：舊存檔（tiles 欄 JSON 無 home_decor）載回時 serde(default) 回 0=不擺。
        let json = format!(
            "{{\"tiles\":{}}}",
            serde_json::to_string(&vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS]).unwrap()
        );
        let back = serde_json::from_str::<Field>(&json).unwrap().reseated(0).unwrap();
        assert_eq!(back.home_decor(), 0);
        assert!(back.garden().iter().all(|&x| x == 0), "舊存檔載回庭園全空");
    }

    // ── ROADMAP 416 庭園 ────────────────────────────────────────────────────
    #[test]
    fn garden_slots_persist_through_serde_and_reseat() {
        // 多格庭園入存檔且 reseated 接回——否則重啟會把玩家佈置好的庭園清掉。
        let mut f = Field::for_plot(3);
        f.set_garden_slot(0, 2);
        f.set_garden_slot(2, 9);
        f.set_garden_slot(5, 11);
        let json = serde_json::to_string(&f).unwrap();
        let back = serde_json::from_str::<Field>(&json).unwrap().reseated(3).unwrap();
        assert_eq!(back.garden(), &[2, 0, 9, 0, 0, 11], "整座庭園撐過序列化＋reseat");
        assert_eq!(back.home_decor(), 2, "legacy home_decor 同步成 slot 0");
        assert_eq!(back, f);
    }

    #[test]
    fn old_single_decor_save_migrates_into_garden_slot_zero() {
        // 向後相容遷移：舊存檔只有 home_decor（無 garden 欄）時，reseated 把那件升成庭園 slot 0，
        // 玩家原本擺好的擺飾零損失。
        let json = format!(
            "{{\"tiles\":{},\"home_decor\":4}}",
            serde_json::to_string(&vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS]).unwrap()
        );
        let back = serde_json::from_str::<Field>(&json).unwrap().reseated(0).unwrap();
        assert_eq!(back.home_decor(), 4, "legacy 那件仍在");
        assert_eq!(back.garden().first().copied(), Some(4), "自動升成庭園 slot 0");
    }

    #[test]
    fn set_garden_slot_ignores_out_of_range_slot_and_sanitizes_index() {
        // 超界 slot（防偽造）一律忽略；越界 index 夾成 0=不擺。
        let mut f = Field::for_plot(1);
        f.set_garden_slot(crate::home_decor::GARDEN_SLOTS as u8, 5); // 超界 slot
        assert!(f.garden().iter().all(|&x| x == 0), "超界 slot 不寫入任何格");
        f.set_garden_slot(1, 250); // 越界 index
        assert_eq!(f.garden().get(1).copied(), Some(0), "越界 index 回 0=不擺");
    }

    #[test]
    fn reseated_rejects_corrupt_field() {
        // reseated 套用 from_tiles 的雙重驗證:壞檔(作物 NaN)整塊拒收,呼叫端可丟棄該列。
        // 直接組壞 tiles（NaN 經 JSON 會被序列化成 null、進不了 f32,故不走 serde 路徑）。
        let mut bad = Field::new();
        bad.tiles[0] = Tile::Planted(crate::crops::Crop::from_raw(f32::NAN, 0.0));
        assert!(bad.reseated(0).is_none());
    }

    #[test]
    fn from_tiles_rejects_wrong_cell_count() {
        // 初始格數（FIELD_COLS * FIELD_ROWS）可接受。
        assert!(Field::from_tiles(0, vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS]).is_some());
        // 擴張後的格數（+1~+MAX_EXPANSIONS 列）也可接受。
        assert!(Field::from_tiles(0, vec![Tile::Untilled; FIELD_COLS * (FIELD_ROWS + 1)]).is_some());
        let max = FIELD_COLS * (FIELD_ROWS + crate::economy::MAX_EXPANSIONS as usize);
        assert!(Field::from_tiles(0, vec![Tile::Untilled; max]).is_some());
        // 空、比初始少 1、格數非 FIELD_COLS 的倍數、超過上限，一律拒絕。
        assert!(Field::from_tiles(0, vec![]).is_none());
        assert!(Field::from_tiles(0, vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS - 1]).is_none());
        assert!(Field::from_tiles(0, vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS + 1]).is_none());
        assert!(Field::from_tiles(0, vec![Tile::Untilled; max + FIELD_COLS]).is_none());
    }

    #[test]
    fn grow_adds_one_row_of_untilled_tiles() {
        let mut f = Field::for_plot(0);
        assert_eq!(f.rows(), FIELD_ROWS);
        f.grow();
        assert_eq!(f.rows(), FIELD_ROWS + 1);
        // 新格全是自然地，且可正常翻土。
        assert_eq!(f.tile(0, FIELD_ROWS), Some(&Tile::Untilled));
        assert!(f.till(0, FIELD_ROWS));
    }

    #[test]
    fn for_plot_expanded_starts_with_right_size() {
        let f = Field::for_plot_expanded(0, 3);
        assert_eq!(f.rows(), FIELD_ROWS + 3);
        // 全部格都是自然地，都搆得到（within_reach 覆蓋到擴張格）。
        let (ox, oy) = f.origin();
        let bottom_center_y = oy + (f.rows() as f32 - 0.5) * TILE_SIZE;
        assert!(f.within_reach(ox + TILE_SIZE, bottom_center_y));
    }

    /// 載入時 origin 由序號決定（不靠磁碟值）：同一份 tiles、不同序號 → 不同 origin，
    /// 且各自對齊 `plot_origin(index)`。
    #[test]
    fn from_tiles_origin_comes_from_index() {
        let tiles = vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS];
        let f0 = Field::from_tiles(0, tiles.clone()).unwrap();
        let f1 = Field::from_tiles(1, tiles).unwrap();
        assert_eq!(f0.origin(), crate::plots::plot_origin(0));
        assert_eq!(f1.origin(), crate::plots::plot_origin(1));
        assert_ne!(f0.origin(), f1.origin());
    }

    #[test]
    fn from_tiles_rejects_corrupt_crop_values() {
        use crate::crops::Crop;
        // 格數正確、但某格作物成長是 NaN（壞檔 / 被竄改）→ 整塊拒收。
        let mut tiles = vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS];
        tiles[0] = Tile::Planted(Crop::from_raw(f32::NAN, 0.0));
        assert!(Field::from_tiles(0, tiles).is_none());
        // 負濕度同樣不健全。
        let mut tiles = vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS];
        tiles[5] = Tile::Planted(Crop::from_raw(10.0, -1.0));
        assert!(Field::from_tiles(0, tiles).is_none());
        // 正常範圍內的作物可順利載入。
        let mut tiles = vec![Tile::Untilled; FIELD_COLS * FIELD_ROWS];
        tiles[3] = Tile::Planted(Crop::from_raw(SPROUT_AT, 20.0));
        assert!(Field::from_tiles(0, tiles).is_some());
    }

    #[test]
    fn view_ripe_crop_is_not_marked_dry() {
        let mut f = Field::new();
        f.till(0, 0);
        f.plant(0, 0);
        f.water(0, 0);
        f.tick(MOISTURE_PER_WATER, Season::Summer);
        f.water(0, 0);
        f.tick(RIPE_AT - MOISTURE_PER_WATER, Season::Summer);
        // 成熟即使濕度耗盡也不該再叫玩家澆水；全程用心照顧＝優質（quality 2）。
        assert_eq!(f.view().cells[0], TileView { state: 4, dry: false, thriving: false, quality: 2, grow: 100, soil: 0, kind: 0 });
    }

    #[test]
    fn water_all_planted_waters_dry_crops(){
        let mut f = Field::new();
        // 種兩株，一株已澆水、一株乾。
        f.till(0, 0); f.plant(0, 0); // 乾
        f.till(1, 0); f.plant(1, 0); f.water(1, 0); // 已澆水
        f.water_all_planted();
        // 兩株都應能繼續成長（有濕度）；tick(SPROUT_AT) 後應達到發芽 state=3。
        f.tick(SPROUT_AT, Season::Summer);
        let cells = f.view().cells;
        assert_eq!(cells[0].state, 3, "乾種子被雨水澆後 tick 到 SPROUT_AT 應成為發芽");
        assert_eq!(cells[1].state, 3, "已澆水的 tick 到 SPROUT_AT 也應成為發芽");
    }

    #[test]
    fn water_all_planted_returns_count_of_dry_crops() {
        let mut f = Field::new();
        // 兩株乾、一株已澆水 → 只應澆到 2 株。
        f.till(0, 0); f.plant(0, 0); // 乾
        f.till(1, 0); f.plant(1, 0); // 乾
        f.till(2, 0); f.plant(2, 0); f.water(2, 0); // 已澆水
        assert_eq!(f.water_all_planted(), 2, "應只澆到 2 株缺水作物");
        // 再澆一次：都已濕，沒有缺水的可澆 → 回 0（一鍵澆水按了沒作物可澆時不誤報）。
        assert_eq!(f.water_all_planted(), 0, "全濕後再澆應回 0");
    }

    #[test]
    fn water_all_planted_empty_field_returns_zero() {
        let mut f = Field::new();
        // 沒種任何作物 → 澆到 0 株、不 panic。
        assert_eq!(f.water_all_planted(), 0);
    }

    #[test]
    fn water_all_planted_skips_non_crop_tiles() {
        let mut f = Field::new();
        // 只有翻土格和未翻土格，不應 panic。
        f.till(0, 0);
        f.water_all_planted(); // 不應 panic
    }

    // ─── ROADMAP 446：一鍵收成 ───────────────────────────────────────────────

    /// 測試小工具：把某格全程用心照顧到成熟（＝優質）。
    fn ripen(f: &mut Field, col: usize, row: usize) {
        f.till(col, row);
        f.plant(col, row);
        f.water(col, row);
        f.tick(MOISTURE_PER_WATER, Season::Summer); // 補一次水撐過第二段成長
        f.water(col, row);
        f.tick(RIPE_AT - MOISTURE_PER_WATER, Season::Summer); // 補滿成長到成熟，全程不渴＝優質
    }

    #[test]
    fn harvest_all_ripe_empty_field_returns_default() {
        let mut f = Field::new();
        // 沒種任何作物 → 收到 0 株、不 panic、彙總全為 0。
        let s = f.harvest_all_ripe();
        assert_eq!(s, HarvestAllSummary::default());
        assert_eq!(s.count, 0);
        assert_eq!(s.ether, 0);
    }

    #[test]
    fn harvest_all_ripe_collects_only_ripe_and_resets_tiles() {
        let mut f = Field::new();
        // 兩株熟透、一株剛播下（未熟）。
        ripen(&mut f, 0, 0);
        ripen(&mut f, 1, 0);
        f.till(2, 0);
        f.plant(2, 0); // 種子，未熟
        let s = f.harvest_all_ripe();
        assert_eq!(s.count, 2, "只應收成 2 株成熟作物");
        assert!(s.ether > 0, "收成成熟作物應拿到乙太");
        // 收成的兩格回到空土（state=1），未熟那格仍是作物（state>=2）。
        let cells = f.view().cells;
        assert_eq!(cells[0].state, 1, "收成後該格回到空土");
        assert_eq!(cells[1].state, 1, "收成後該格回到空土");
        assert!(cells[2].state >= 2, "未熟的種子不該被收成");
        // 再按一次：沒有成熟的可收 → 回 0（按了沒東西可收時不誤報）。
        assert_eq!(f.harvest_all_ripe().count, 0, "全收完後再按應回 0 株");
    }

    #[test]
    fn harvest_all_ripe_equivalent_to_single_harvests() {
        // 一鍵收成的總乙太必須與「逐格手收 N 次」完全等價（純省點擊、不改經濟）。
        let mut batch = Field::new();
        let mut single = Field::new();
        for col in 0..3 {
            ripen(&mut batch, col, 0);
            ripen(&mut single, col, 0);
        }
        let s = batch.harvest_all_ripe();
        let mut single_total = 0u32;
        for col in 0..3 {
            if let Some((ether, _q, _soil)) = single.harvest(col, 0) {
                single_total += ether;
            }
        }
        assert_eq!(s.count, 3);
        assert_eq!(s.ether, single_total, "一鍵收成的總乙太應等於逐格手收的總和");
    }

    #[test]
    fn harvest_all_ripe_counts_quality_breakdown() {
        let mut f = Field::new();
        // 全程用心照顧到成熟＝優質；本測種兩株都用心 → 應計 2 株優質、無平凡/用心。
        ripen(&mut f, 0, 0);
        ripen(&mut f, 1, 0);
        let s = f.harvest_all_ripe();
        assert_eq!(s.count, 2);
        assert_eq!(s.premium, 2, "全程不渴的成熟作物應記為優質");
        assert_eq!(s.fine, 0);
        assert_eq!(s.plain, 0);
        // 各品質株數加總應等於 count（不漏算、不重算）。
        assert_eq!(s.premium + s.fine + s.plain, s.count);
    }

    // ─── ROADMAP 367：連片沃土 ───────────────────────────────────────────────

    /// 種好澆好某格的小工具。
    fn sow(f: &mut Field, col: usize, row: usize) {
        f.till(col, row);
        f.plant(col, row);
        f.water(col, row);
    }

    #[test]
    fn thriving_patch_grows_faster_than_isolated_crop() {
        let mut f = Field::new();
        // 三格相鄰連成一片（沃土）+ 一格孤立對照。
        sow(&mut f, 0, 0);
        sow(&mut f, 1, 0);
        sow(&mut f, 2, 0);
        sow(&mut f, 5, 3); // 遠角孤格，不成片
        // tick 25 秒（< SPROUT_AT=30）：孤格成長 25 仍是種子；連片格成長 25×1.5=37.5 已發芽。
        f.tick(25.0, Season::Summer);
        assert_eq!(
            f.crop_stage(0, 0),
            Some(CropStage::Sprout),
            "連片沃土的作物加速成長，25 秒應已發芽"
        );
        assert_eq!(
            f.crop_stage(5, 3),
            Some(CropStage::Seed),
            "孤立作物無加速，25 秒仍是種子"
        );
    }

    #[test]
    fn view_marks_thriving_only_for_connected_patch() {
        let mut f = Field::new();
        // 三格橫向連片 → 皆 thriving；遠處單格 → 非 thriving。
        sow(&mut f, 0, 0);
        sow(&mut f, 1, 0);
        sow(&mut f, 2, 0);
        sow(&mut f, 5, 3);
        let cells = f.view().cells;
        assert!(cells[0].thriving && cells[1].thriving && cells[2].thriving);
        assert!(!cells[5 + 3 * FIELD_COLS].thriving, "孤格不成片");
    }

    #[test]
    fn two_adjacent_crops_not_yet_thriving() {
        let mut f = Field::new();
        // 兩格相鄰仍不足三格門檻：不算沃土、不加速。
        sow(&mut f, 0, 0);
        sow(&mut f, 1, 0);
        let cells = f.view().cells;
        assert!(!cells[0].thriving && !cells[1].thriving);
    }
}
