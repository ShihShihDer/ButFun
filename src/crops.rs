//! 乙太作物的成長模型（Phase 0-G 種田起源的純邏輯地基）。
//!
//! 這層只管「一株作物怎麼長大、什麼時候能收成」，是純資料 + 純函式，無 IO、
//! 不碰 WebSocket / 遊戲迴圈，便於自動測試。之後接上：
//!   - 遊戲迴圈：每 tick 對地裡的作物呼叫 `grow(dt)`。
//!   - 持久化（接 0-E）：把 `Crop` 序列化進農地狀態。
//!   - 前端：依 `stage()` 畫出對應階段的圖。
//!
//! 療癒迴圈刻意做成「澆水才會長」：播種後要澆水，濕度會隨成長慢慢被消耗，
//! 乾了就停滯、得再澆——讓「照顧」本身有意義，而不是種下去放著就好。
//!
//! 持久化（接 0-E）：`Crop` 衍生 serde，存的是內部 `growth`/`moisture`（秒），
//! 而非推導出的階段——這樣「成長到一半」的作物重啟後能原封不動接續長，而不是被
//! 四捨五入到某個階段。序列化格式刻意對齊記憶體表示，是農地持久化的格式地基。

use serde::{Deserialize, Serialize};

/// 作物的成長階段（依累積成長時間推導）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropStage {
    /// 剛播種的種子。
    Seed,
    /// 發芽、抽長中。
    Sprout,
    /// 成熟，可以收成。
    Ripe,
}

/// 累積成長到此秒數後發芽。
pub const SPROUT_AT: f32 = 30.0;
/// 累積成長到此秒數後成熟可收。
pub const RIPE_AT: f32 = 90.0;
/// 一次澆水給的濕度（秒）：可支撐這麼久的成長後才需要再澆。
pub const MOISTURE_PER_WATER: f32 = 60.0;
/// 收成一株成熟作物得到的乙太量。
pub const ETHER_PER_HARVEST: u32 = 3;

/// 依累積成長時間推導目前階段。純函式。
pub fn stage_for(growth: f32) -> CropStage {
    if growth >= RIPE_AT {
        CropStage::Ripe
    } else if growth >= SPROUT_AT {
        CropStage::Sprout
    } else {
        CropStage::Seed
    }
}

/// 一株種在某格耕地上的乙太作物。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    /// 累積有效成長時間（秒）。只有有濕度時才會增加。
    growth: f32,
    /// 剩餘濕度（秒）。澆水補滿，成長時被消耗；歸零後停止成長。
    moisture: f32,
}

impl Crop {
    /// 播下一株新種子（乾的，需要澆水才會開始長）。
    pub fn plant() -> Self {
        Self {
            growth: 0.0,
            moisture: 0.0,
        }
    }

    /// 澆水：把濕度補滿。
    pub fn water(&mut self) {
        self.moisture = MOISTURE_PER_WATER;
    }

    /// 推進 `dt` 秒。只有在有濕度時才成長，並同步消耗濕度。
    /// 成長與濕度都不會超過各自界線。抽成純函式以便測試。
    pub fn grow(&mut self, dt: f32) {
        self.grow_boosted(dt, 1.0);
    }

    /// 推進 `dt` 秒，但成長速度乘上 `growth_mult`（ROADMAP 367 連片沃土：連片照料的作物
    /// 成長加速）。**濕度仍按真實 `dt` 消耗**、不因成長加速而更快乾——加速只回饋在「長得快」，
    /// 不讓沃土更耗水，維持公平。`growth_mult ≤ 0` 或非有限時退回不加速（防呆）。
    pub fn grow_boosted(&mut self, dt: f32, growth_mult: f32) {
        if self.moisture <= 0.0 || dt <= 0.0 {
            return;
        }
        let mult = if growth_mult.is_finite() && growth_mult > 0.0 {
            growth_mult
        } else {
            1.0
        };
        // 這段時間內實際能長多久，受限於剩餘濕度（加速放大的是成長、非耗水）。
        let effective = dt.min(self.moisture);
        self.growth = (self.growth + effective * mult).min(RIPE_AT);
        self.moisture = (self.moisture - dt).max(0.0);
    }

    /// 目前成長階段。
    pub fn stage(&self) -> CropStage {
        stage_for(self.growth)
    }

    /// 是否成熟可收。
    pub fn is_ripe(&self) -> bool {
        self.stage() == CropStage::Ripe
    }

    /// 是否需要澆水（已乾）。
    pub fn needs_water(&self) -> bool {
        self.moisture <= 0.0
    }

    /// 從存檔載入的值是否「健全」：成長與濕度都是有限且非負。
    /// 這是與調校常數無關的最小不變式——正常流程（`plant` 起 0、`water` 補滿、
    /// `grow` 一律夾在 `>= 0`）絕不會產生 `NaN` / `Inf` / 負值，所以這些只會來自
    /// 壞檔或被竄改的存檔。上界（`RIPE_AT` / `MOISTURE_PER_WATER`）刻意不檢查：
    /// 它們是會調整的常數，且即使 growth 載入時偏大，`grow` 下一 tick 也會把它夾回
    /// 上限、過量濕度只是多撐一下，皆無害。持久化（接 0-E）載入時用它逐株驗證。
    /// 接 0-E 載入路徑時移除此 `allow`（沿用本檔前置地基的慣例）。
    #[allow(dead_code)]
    pub fn is_loadable(&self) -> bool {
        self.growth.is_finite()
            && self.moisture.is_finite()
            && self.growth >= 0.0
            && self.moisture >= 0.0
    }

    /// 測試用：直接組出指定 `growth` / `moisture`（含壞值）的作物，驗證載入防線。
    #[cfg(test)]
    pub fn from_raw(growth: f32, moisture: f32) -> Self {
        Self { growth, moisture }
    }

    /// 收成：成熟才給乙太，並把這格重置成可再種的新種子。
    /// 未成熟時回 `None`、不改變狀態。
    pub fn harvest(&mut self) -> Option<u32> {
        if !self.is_ripe() {
            return None;
        }
        *self = Crop::plant();
        Some(ETHER_PER_HARVEST)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_thresholds() {
        assert_eq!(stage_for(0.0), CropStage::Seed);
        assert_eq!(stage_for(SPROUT_AT - 0.1), CropStage::Seed);
        assert_eq!(stage_for(SPROUT_AT), CropStage::Sprout);
        assert_eq!(stage_for(RIPE_AT - 0.1), CropStage::Sprout);
        assert_eq!(stage_for(RIPE_AT), CropStage::Ripe);
    }

    #[test]
    fn freshly_planted_is_dry_seed() {
        let c = Crop::plant();
        assert_eq!(c.stage(), CropStage::Seed);
        assert!(c.needs_water());
        assert!(!c.is_ripe());
    }

    #[test]
    fn does_not_grow_without_water() {
        let mut c = Crop::plant();
        c.grow(100.0);
        assert_eq!(c.stage(), CropStage::Seed);
        assert_eq!(c, Crop::plant());
    }

    #[test]
    fn watering_lets_it_grow() {
        let mut c = Crop::plant();
        c.water();
        c.grow(SPROUT_AT);
        assert_eq!(c.stage(), CropStage::Sprout);
    }

    #[test]
    fn growth_is_capped_by_remaining_moisture() {
        let mut c = Crop::plant();
        c.water(); // 濕度 = 60
        c.grow(100.0); // 一次給很久，但只能長 60 秒份
        // 60 秒成長 < RIPE_AT(90)，所以還不到成熟。
        assert_eq!(c.stage(), CropStage::Sprout);
        assert!(c.needs_water());
    }

    #[test]
    fn rewatering_continues_growth_to_ripe() {
        let mut c = Crop::plant();
        c.water();
        c.grow(60.0); // 長 60，濕度歸零
        assert!(c.needs_water());
        c.water();
        c.grow(60.0); // 再長到 90 上限
        assert!(c.is_ripe());
    }

    #[test]
    fn harvest_only_when_ripe() {
        let mut c = Crop::plant();
        c.water();
        c.grow(SPROUT_AT);
        assert_eq!(c.harvest(), None); // 還沒熟
        assert_eq!(c.stage(), CropStage::Sprout); // 狀態不變
    }

    #[test]
    fn harvest_yields_ether_and_resets() {
        let mut c = Crop::plant();
        // 單次澆水僅夠長 MOISTURE_PER_WATER 秒，需再澆一次才到 RIPE_AT。
        c.water();
        c.grow(MOISTURE_PER_WATER);
        c.water();
        c.grow(RIPE_AT - MOISTURE_PER_WATER);
        assert!(c.is_ripe());
        assert_eq!(c.harvest(), Some(ETHER_PER_HARVEST));
        // 收成後回到可再種的乾種子。
        assert_eq!(c, Crop::plant());
    }

    #[test]
    fn growth_never_exceeds_ripe_cap() {
        let mut c = Crop::plant();
        for _ in 0..10 {
            c.water();
            c.grow(MOISTURE_PER_WATER);
        }
        // 多次澆水後成長被夾在 RIPE_AT；再收成只拿固定量。
        assert!(c.is_ripe());
        assert_eq!(c.harvest(), Some(ETHER_PER_HARVEST));
    }

    #[test]
    fn is_loadable_accepts_normal_and_rejects_corrupt() {
        // 正常流程產出的值都該可載入。
        assert!(Crop::plant().is_loadable());
        let mut c = Crop::plant();
        c.water();
        c.grow(SPROUT_AT);
        assert!(c.is_loadable());
        // 壞值：NaN / Inf / 負成長 / 負濕度，皆非正常流程能產生，視為壞檔。
        assert!(!Crop::from_raw(f32::NAN, 0.0).is_loadable());
        assert!(!Crop::from_raw(0.0, f32::INFINITY).is_loadable());
        assert!(!Crop::from_raw(-1.0, 0.0).is_loadable());
        assert!(!Crop::from_raw(0.0, -1.0).is_loadable());
    }

    #[test]
    fn zero_dt_is_a_noop() {
        let mut c = Crop::plant();
        c.water();
        let before = c.clone();
        c.grow(0.0);
        assert_eq!(c, before);
    }
}
