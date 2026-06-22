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

use crate::crop_variety::CropVariety;

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
/// 收成一株成熟作物得到的乙太量（平凡品質的基礎量）。
pub const ETHER_PER_HARVEST: u32 = 3;

// ── 用心栽培·作物品質（ROADMAP 406）────────────────────────────────────────────
//
// 在此之前，澆水只是「不讓作物乾死停滯」——細心一路顧著澆與隨手放著乾涸，收成
// 完全一樣（都拿 ETHER_PER_HARVEST）。本切片讓「照顧的用心程度」第一次有回報：
// 作物記住成長期間「渴著停滯（乾涸又還沒成熟）」累積的秒數，收成時據此分品質——
// 從頭到尾不讓它渴的細心照料拿「優質」收成、多得乙太；放著乾涸越久品質越低。
// 純記憶體＋持久化欄位（serde default 向後相容），零 migration、零 LLM、療癒向。

/// 渴秒數在此（含）以內 → 優質收成（⭐）。
/// 留一點寬容餘地（而非嚴格 0）：每幀推進的離散誤差、或澆水稍慢一兩秒都仍算用心。
pub const PREMIUM_MAX_PARCHED: f32 = 3.0;
/// 渴秒數在此（含）以內、但超過 `PREMIUM_MAX_PARCHED` → 用心收成（🌿）。
pub const FINE_MAX_PARCHED: f32 = 30.0;
/// 用心收成（Fine）相對基礎多得的乙太。
pub const ETHER_BONUS_FINE: u32 = 1;
/// 優質收成（Premium）相對基礎多得的乙太。
pub const ETHER_BONUS_PREMIUM: u32 = 2;

/// 一株作物收成時的品質——由成長期累積的「渴秒數」推導，越用心照顧（越少渴著）越高。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropQuality {
    /// 平凡：放任乾涸較久，只得基礎乙太。
    Plain,
    /// 用心：大致顧著澆，略有疏失。
    Fine,
    /// 優質：從不讓它渴，最細心的照料。
    Premium,
}

impl CropQuality {
    /// 線格式字串（前端據此挑飄字顏色／文案；snake_case 對齊既有事件約定）。
    pub fn as_str(self) -> &'static str {
        match self {
            CropQuality::Plain => "plain",
            CropQuality::Fine => "fine",
            CropQuality::Premium => "premium",
        }
    }

    /// 線格式碼（隨田格快照下傳：0=平凡 1=用心 2=優質），給前端在成熟作物上畫品質光點。
    pub fn code(self) -> u8 {
        match self {
            CropQuality::Plain => 0,
            CropQuality::Fine => 1,
            CropQuality::Premium => 2,
        }
    }

    /// 這個品質相對基礎收成多得的乙太（平凡 0）。純函式。
    pub fn ether_bonus(self) -> u32 {
        match self {
            CropQuality::Plain => 0,
            CropQuality::Fine => ETHER_BONUS_FINE,
            CropQuality::Premium => ETHER_BONUS_PREMIUM,
        }
    }
}

/// 依成長期累積的渴秒數推導品質。純函式——伺服器結算與測試共用同一套門檻。
pub fn quality_for(parched: f32) -> CropQuality {
    // 壞值（NaN/負）保守當作最用心（不冤枉玩家），但正常流程不會產生。
    if !(parched > PREMIUM_MAX_PARCHED) {
        CropQuality::Premium
    } else if parched <= FINE_MAX_PARCHED {
        CropQuality::Fine
    } else {
        CropQuality::Plain
    }
}

/// 依累積成長時間推導「離成熟還有多少」的進度（0.0=剛播下、1.0=成熟可收）。純函式。
/// ROADMAP 421 作物熟成進度：前端據此在成長中的作物格畫一條熟成進度條，讓玩家在離散的
/// 種子／發芽／成熟三階段之間，也一眼看得出「還差多久收成」——回應建議箱多次反映的
/// 「想看到作物週期進度、感受接近目標的動能、少一點空轉感」。壞值（NaN/負）夾回 0、
/// 超過上限夾回 1，永遠回傳 [0,1]。
pub fn progress_for(growth: f32) -> f32 {
    if !(growth > 0.0) {
        // NaN 或 ≤0 一律當作剛起步（0 進度）；不讓壞值汙染前端進度條。
        return 0.0;
    }
    (growth / RIPE_AT).min(1.0)
}

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
    /// ROADMAP 406 用心栽培：成長期間「渴著停滯」（已開始成長、尚未成熟、且乾涸）
    /// 累積的秒數，越大表示越疏於照顧、收成品質越低。`#[serde(default)]` 讓舊存檔
    /// 安全讀回 0（在途作物一次性視為最用心，向後相容、不破壞玩家資料）。
    #[serde(default)]
    parched: f32,
    /// ROADMAP 452 作物品種：這株是哪個品種，決定成長速度（`grow_boosted` 取其 `grow_rate`）
    /// 與收成乙太（`harvest` 取其 `harvest_ether`）。`#[serde(default)]` 讓舊存檔／無品種欄的
    /// 在途作物安全讀回預設 `Staple`（＝改動前的單一作物數值），向後相容、零 migration、不破壞玩家資料。
    #[serde(default)]
    kind: CropVariety,
    /// ROADMAP 473 堆肥循環：這株播種時是否漚進了一份堆肥。滋養的作物成長更快（`field::tick`
    /// 疊上 `compost::NOURISH_GROWTH_MULT`）、收成多得乙太（`field::harvest` 加 `NOURISH_ETHER_BONUS`）。
    /// `#[serde(default)]` 讓舊存檔／無此欄的在途作物安全讀回 `false`＝未滋養（＝改動前數值），
    /// 向後相容、零 migration、不破壞玩家資料。
    #[serde(default)]
    nourished: bool,
    /// ROADMAP 476 稻草人守望：這株「成熟後」累積的曝露秒數（與 `growth` 同單位，由 `grow_boosted`
    /// 在成熟後逐 tick 累加）。久置無人看守會招田鴉啄食（見 `field::tick` × `crop_raid`）。
    /// `#[serde(default)]` 讓舊存檔／無此欄的在途作物安全讀回 0＝剛熟（向後相容、零 migration）。
    #[serde(default)]
    ripe_secs: f32,
    /// ROADMAP 476：這株成熟後是否曾被田鴉啄食。為真＝收成品質降一階（折進 `quality()`）。
    /// 啄食一次定案、不重複扣。`#[serde(default)]` 讓舊存檔安全讀回 `false`＝沒被啄（向後相容、零 migration）。
    #[serde(default)]
    pecked: bool,
}

impl Crop {
    /// 播下一株新種子（乾的，需要澆水才會開始長）——預設品種主食穀、未滋養。
    /// 既有呼叫端與測試沿用此入口；要指定品種走 `plant_kind`、要漚堆肥走 `plant_kind_nourished`。
    pub fn plant() -> Self {
        Self::plant_kind(CropVariety::default())
    }

    /// 播下指定品種的一株新種子（ROADMAP 452）。除品種外與 `plant` 完全相同（未滋養）。
    pub fn plant_kind(kind: CropVariety) -> Self {
        Self::plant_kind_nourished(kind, false)
    }

    /// 播下指定品種、且可指定是否漚過堆肥的一株新種子（ROADMAP 473）。
    /// `nourished=true` 時這株帶滋養加成（成長加速＋收成多得乙太）。
    pub fn plant_kind_nourished(kind: CropVariety, nourished: bool) -> Self {
        Self {
            growth: 0.0,
            moisture: 0.0,
            parched: 0.0,
            kind,
            nourished,
            // ROADMAP 476：新種下還沒熟，曝露計時與被啄旗標皆從零起。
            ripe_secs: 0.0,
            pecked: false,
        }
    }

    /// 這株作物的品種（ROADMAP 452）。供田格快照下傳給前端畫品種微異。
    pub fn kind(&self) -> CropVariety {
        self.kind
    }

    /// 這株是否漚過堆肥、帶滋養加成（ROADMAP 473）。供 `field` 算成長倍率／收成乙太，
    /// 並隨田格快照下傳給前端畫「滋養」記號。
    pub fn nourished(&self) -> bool {
        self.nourished
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
        // ROADMAP 476：已成熟的作物開始累積「曝露秒數」（不論有沒有水——熟了就等著被收或被啄）。
        // 放在最前面、與成長／濕度邏輯正交：久置無人看守的成熟作物終會招田鴉（啄食判定在 `field::tick`）。
        // dt 須為有限正值才計（防 NaN 汙染）；未熟（growth < RIPE_AT）不累積。
        if dt.is_finite() && dt > 0.0 && self.growth >= RIPE_AT {
            self.ripe_secs += dt;
        }
        if self.moisture <= 0.0 || dt <= 0.0 {
            // 乾涸（或無效 dt）不成長。ROADMAP 406：若作物「已開始成長、尚未成熟」卻渴著停滯，
            // 累積渴秒數（收成品質的扣分依據）。剛播下還沒澆過的種子（growth==0）不計，避免懲罰起步；
            // 已成熟的（growth>=RIPE_AT）品質已定，不再累積。dt 須為有限正值才計（防 NaN 汙染）。
            if dt > 0.0 && dt.is_finite() && self.moisture <= 0.0 && self.growth > 0.0 && self.growth < RIPE_AT {
                self.parched += dt;
            }
            return;
        }
        let mult = if growth_mult.is_finite() && growth_mult > 0.0 {
            growth_mult
        } else {
            1.0
        };
        // ROADMAP 452：品種的成長速度倍率（速生菜快、乙太瓜慢；主食穀＝1.0 與改動前一致）。
        // 與沃土加速（`mult`）相乘——兩者都只放大「長得多快」、不多耗水，維持公平。
        let kind_rate = self.kind.grow_rate();
        // 這段時間內實際能長多久，受限於剩餘濕度（加速放大的是成長、非耗水）。
        let effective = dt.min(self.moisture);
        self.growth = (self.growth + effective * mult * kind_rate).min(RIPE_AT);
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

    /// ROADMAP 421：離成熟還有多少的進度（0.0~1.0）。前端據此畫熟成進度條。
    pub fn progress(&self) -> f32 {
        progress_for(self.growth)
    }

    /// 是否需要澆水（已乾）。
    pub fn needs_water(&self) -> bool {
        self.moisture <= 0.0
    }

    /// ROADMAP 501 作物熟成倒數：假設持續充足澆水情境下，距成熟的估計剩餘秒數。
    /// 已成熟回 0；未熟依品種成長速率反推（速生菜快、乙太瓜慢）。
    /// 用於田格快照 `TileView.eta_secs`，讓玩家知道「約幾分鐘後回來收最划算」。
    pub fn eta_secs(&self) -> u16 {
        if self.is_ripe() {
            return 0;
        }
        let remaining = (RIPE_AT - self.growth).max(0.0);
        let rate = self.kind.grow_rate();
        // rate 絕對正有限（crop_variety 單元測試把關），但防呆仍夾界。
        if !rate.is_finite() || rate <= 0.0 {
            return u16::MAX;
        }
        let secs = remaining / rate;
        secs.ceil().min(u16::MAX as f32) as u16
    }

    /// ROADMAP 406：依成長期累積的渴秒數推導的收成品質（越用心照顧越高）。
    /// 對任何階段都可問，但只有成熟（`is_ripe`）作物收成時才真正套用。
    /// ROADMAP 476：若這株成熟後曾被田鴉啄食（`pecked`），品質再降一階——把「久置不收」的
    /// 代價直接折進品質，故所有讀 `quality()` 的地方（收成乙太、一鍵收成、田格快照顯示）一致生效。
    pub fn quality(&self) -> CropQuality {
        let q = quality_for(self.parched);
        if self.pecked {
            crate::crop_raid::pecked_quality(q)
        } else {
            q
        }
    }

    /// ROADMAP 476：這株成熟後累積的曝露秒數（供 `field::tick` 判斷是否該招田鴉啄食）。
    pub fn ripe_secs(&self) -> f32 {
        self.ripe_secs
    }

    /// ROADMAP 476：這株成熟後是否曾被田鴉啄食（供田格快照標記啄痕、收成品質折一階）。
    pub fn is_pecked(&self) -> bool {
        self.pecked
    }

    /// ROADMAP 476：把這株標記為「已被田鴉啄食」。冪等——已標記再標記無變化。
    /// 守護／曝露判定由 `field::tick` 以 `crop_raid::should_peck` 把關，本方法只負責落旗標。
    pub fn mark_pecked(&mut self) {
        self.pecked = true;
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
            && self.parched.is_finite()
            && self.ripe_secs.is_finite()
            && self.growth >= 0.0
            && self.moisture >= 0.0
            && self.parched >= 0.0
            && self.ripe_secs >= 0.0
    }

    /// 測試用：直接組出指定 `growth` / `moisture`（含壞值）的作物，驗證載入防線。
    #[cfg(test)]
    pub fn from_raw(growth: f32, moisture: f32) -> Self {
        Self { growth, moisture, parched: 0.0, kind: CropVariety::default(), nourished: false, ripe_secs: 0.0, pecked: false }
    }

    /// 收成：成熟才給乙太，並把這格重置成可再種的新種子。
    /// 未成熟時回 `None`、不改變狀態。
    pub fn harvest(&mut self) -> Option<u32> {
        if !self.is_ripe() {
            return None;
        }
        // ROADMAP 452：收成乙太依品種（主食穀＝既有 ETHER_PER_HARVEST）。
        let ether = self.kind.harvest_ether();
        // 收成後回到「同品種」的新乾種子：這格本來種什麼、重置後仍是什麼（field 層收成後實際會把
        // 格子改回空土另行重種，這裡保留品種只為 crops 層語意自洽與測試可預期）。
        let kind = self.kind;
        *self = Crop::plant_kind(kind);
        Some(ether)
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

    // ── 用心栽培·作物品質（ROADMAP 406）────────────────────────────────────────

    #[test]
    fn attentive_care_yields_premium() {
        // 全程不讓它渴：澆水→長到濕度將盡前再澆，渴秒數累積為 0 → 優質。
        let mut c = Crop::plant();
        c.water();
        c.grow(MOISTURE_PER_WATER); // 長 60，濕度剛好歸零，但這一刻還沒「停滯」累渴
        c.water(); // 馬上補水
        c.grow(RIPE_AT - MOISTURE_PER_WATER); // 長到成熟
        assert!(c.is_ripe());
        assert_eq!(c.quality(), CropQuality::Premium, "從不讓它渴＝優質");
    }

    #[test]
    fn long_neglect_yields_plain() {
        // 長時間渴著停滯（遠超 FINE_MAX_PARCHED）→ 平凡。
        let mut c = Crop::plant();
        c.water();
        c.grow(SPROUT_AT); // 開始成長、濕度仍有餘
        c.grow(MOISTURE_PER_WATER); // 把濕度用乾（已開始成長、未成熟）
        c.grow(FINE_MAX_PARCHED + 20.0); // 乾涸停滯一大段，累積渴秒數
        c.water();
        c.grow(RIPE_AT); // 補水長到成熟
        assert!(c.is_ripe());
        assert_eq!(c.quality(), CropQuality::Plain, "放任乾涸越久越平凡");
    }

    #[test]
    fn mild_neglect_yields_fine() {
        // 渴著一小段（介於 PREMIUM 與 FINE 門檻之間）→ 用心（非滿分也非墊底）。
        let mut c = Crop::plant();
        c.water();
        c.grow(SPROUT_AT); // 開始成長
        c.grow(MOISTURE_PER_WATER); // 用乾濕度
        c.grow(15.0); // 渴 15 秒（>3、<=30）
        c.water();
        c.grow(RIPE_AT);
        assert!(c.is_ripe());
        assert_eq!(c.quality(), CropQuality::Fine);
    }

    #[test]
    fn unwatered_seed_does_not_accrue_neglect() {
        // 剛播下、還沒澆過的種子放著乾，不該累積渴秒數（不懲罰起步）。
        let mut c = Crop::plant();
        c.grow(100.0); // 乾種子放很久（growth 仍為 0）
        assert_eq!(c, Crop::plant(), "未澆過的種子放著＝完全沒推進、零渴");
        assert_eq!(c.quality(), CropQuality::Premium);
    }

    #[test]
    fn ripe_crop_stops_accruing_neglect() {
        // 成熟後放著沒收（且乾），品質已定、不再下降。
        let mut c = Crop::plant();
        c.water();
        c.grow(MOISTURE_PER_WATER);
        c.water();
        c.grow(RIPE_AT - MOISTURE_PER_WATER); // 用心長到成熟＝優質
        assert!(c.is_ripe());
        assert_eq!(c.quality(), CropQuality::Premium);
        c.grow(500.0); // 成熟後乾放很久
        assert_eq!(c.quality(), CropQuality::Premium, "成熟後品質鎖定不再下降");
    }

    #[test]
    fn quality_for_thresholds_and_bonus() {
        assert_eq!(quality_for(0.0), CropQuality::Premium);
        assert_eq!(quality_for(PREMIUM_MAX_PARCHED), CropQuality::Premium);
        assert_eq!(quality_for(PREMIUM_MAX_PARCHED + 0.1), CropQuality::Fine);
        assert_eq!(quality_for(FINE_MAX_PARCHED), CropQuality::Fine);
        assert_eq!(quality_for(FINE_MAX_PARCHED + 0.1), CropQuality::Plain);
        // 壞值保守不冤枉玩家。
        assert_eq!(quality_for(f32::NAN), CropQuality::Premium);
        // 品質越高加成越多、平凡為 0。
        assert_eq!(CropQuality::Plain.ether_bonus(), 0);
        assert!(CropQuality::Fine.ether_bonus() > CropQuality::Plain.ether_bonus());
        assert!(CropQuality::Premium.ether_bonus() > CropQuality::Fine.ether_bonus());
        // 線格式碼遞增、字串穩定。
        assert_eq!(CropQuality::Plain.code(), 0);
        assert_eq!(CropQuality::Premium.code(), 2);
        assert_eq!(CropQuality::Premium.as_str(), "premium");
    }

    #[test]
    fn harvest_resets_neglect_for_next_crop() {
        // 收成後回到全新種子，渴秒數歸零，下一輪重新計品質。
        let mut c = Crop::plant();
        c.water();
        c.grow(SPROUT_AT);
        c.grow(MOISTURE_PER_WATER);
        c.grow(FINE_MAX_PARCHED + 20.0); // 累渴
        c.water();
        c.grow(RIPE_AT);
        assert_eq!(c.quality(), CropQuality::Plain);
        assert_eq!(c.harvest(), Some(ETHER_PER_HARVEST));
        assert_eq!(c, Crop::plant(), "收成後渴秒數隨整株重置歸零");
    }

    // ── 作物熟成進度（ROADMAP 421）──────────────────────────────────────────────

    #[test]
    fn progress_spans_zero_to_one_over_growth() {
        // 剛播下＝0、長到一半＝0.5、到成熟＝1（並夾住超出上限）。
        assert_eq!(progress_for(0.0), 0.0);
        assert!((progress_for(RIPE_AT / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(progress_for(RIPE_AT), 1.0);
        assert_eq!(progress_for(RIPE_AT * 2.0), 1.0, "超過上限夾回 1");
    }

    #[test]
    fn progress_clamps_bad_values() {
        // 壞值（NaN / 負）一律回 0，不讓進度條被汙染。
        assert_eq!(progress_for(f32::NAN), 0.0);
        assert_eq!(progress_for(-5.0), 0.0);
    }

    #[test]
    fn crop_progress_tracks_growth() {
        let mut c = Crop::plant();
        assert_eq!(c.progress(), 0.0, "剛播下進度 0");
        c.water();
        c.grow(SPROUT_AT); // 長到發芽（30/90）
        assert!((c.progress() - SPROUT_AT / RIPE_AT).abs() < 1e-6);
        c.water();
        c.grow(RIPE_AT); // 補水長到成熟
        assert_eq!(c.progress(), 1.0, "成熟＝滿進度");
    }

    #[test]
    fn zero_dt_is_a_noop() {
        let mut c = Crop::plant();
        c.water();
        let before = c.clone();
        c.grow(0.0);
        assert_eq!(c, before);
    }

    // ── 作物品種（ROADMAP 452）─────────────────────────────────────────────────

    #[test]
    fn default_plant_is_staple_matching_legacy_growth() {
        // 無品種入口＝主食穀，成長速度＝1.0：與改動前完全一致（向後相容地基）。
        let mut c = Crop::plant();
        assert_eq!(c.kind(), CropVariety::Staple);
        c.water();
        c.grow(SPROUT_AT); // 主食穀長 30 秒 → 剛好發芽（與改動前同）
        assert_eq!(c.stage(), CropStage::Sprout);
    }

    #[test]
    fn sprout_grows_faster_than_staple() {
        // 速生菜倍率 >1：同樣澆一次水、長同樣的真實秒數，速生菜累積更多有效成長。
        let mut sprout = Crop::plant_kind(CropVariety::Sprout);
        let mut staple = Crop::plant_kind(CropVariety::Staple);
        sprout.water();
        staple.water();
        sprout.grow(40.0);
        staple.grow(40.0);
        // 速生菜倍率 1.6：40×1.6=64 → 已成熟（>=90? 不，64<90）但比主食穀（40）更接近。
        assert!(sprout.progress() > staple.progress(), "速生菜長得比主食穀快");
    }

    #[test]
    fn sprout_ripens_within_one_water_but_staple_needs_more() {
        // 速生菜一次澆水（60 秒濕度）就能成熟：60×1.6=96 >= RIPE_AT(90)。
        let mut sprout = Crop::plant_kind(CropVariety::Sprout);
        sprout.water();
        sprout.grow(MOISTURE_PER_WATER);
        assert!(sprout.is_ripe(), "速生菜一次澆水即可成熟");
        // 主食穀一次澆水只長 60 < 90，還沒熟（與改動前一致）。
        let mut staple = Crop::plant_kind(CropVariety::Staple);
        staple.water();
        staple.grow(MOISTURE_PER_WATER);
        assert!(!staple.is_ripe(), "主食穀一次澆水還不夠成熟");
    }

    #[test]
    fn harvest_ether_differs_by_variety() {
        // 各品種收成基礎乙太不同：速生菜少、主食穀＝既有、乙太瓜多。
        fn ripen_and_harvest(kind: CropVariety) -> u32 {
            let mut c = Crop::plant_kind(kind);
            // 多澆幾次水確保任何品種都長到成熟（乙太瓜倍率低需更久）。
            for _ in 0..6 {
                c.water();
                c.grow(MOISTURE_PER_WATER);
            }
            assert!(c.is_ripe(), "{:?} 應已成熟", kind);
            c.harvest().expect("成熟可收")
        }
        assert_eq!(ripen_and_harvest(CropVariety::Staple), ETHER_PER_HARVEST);
        assert!(ripen_and_harvest(CropVariety::Sprout) < ETHER_PER_HARVEST, "速生菜收得少");
        assert!(ripen_and_harvest(CropVariety::Etherbloom) > ETHER_PER_HARVEST, "乙太瓜收得多");
    }

    #[test]
    fn harvest_preserves_variety_for_next_crop() {
        // 收成後回到「同品種」的新乾種子（field 層另會改回空土，但 crops 層語意自洽）。
        let mut c = Crop::plant_kind(CropVariety::Etherbloom);
        for _ in 0..6 {
            c.water();
            c.grow(MOISTURE_PER_WATER);
        }
        assert!(c.harvest().is_some());
        assert_eq!(c.kind(), CropVariety::Etherbloom, "收成後品種沿用");
        assert_eq!(c, Crop::plant_kind(CropVariety::Etherbloom));
    }

    #[test]
    fn quality_is_orthogonal_to_variety() {
        // 品質仍由「是否用心照顧」決定，與品種無關：速生菜全程不渴也拿優質。
        let mut c = Crop::plant_kind(CropVariety::Sprout);
        c.water();
        c.grow(MOISTURE_PER_WATER); // 一次水就熟（速生菜），全程沒渴
        assert!(c.is_ripe());
        assert_eq!(c.quality(), CropQuality::Premium, "用心照顧＝優質，與品種正交");
    }

    #[test]
    fn variety_survives_serde_round_trip_and_legacy_defaults_to_staple() {
        // 帶品種的作物序列化往返後品種不變（持久化地基）。
        let c = Crop::plant_kind(CropVariety::Etherbloom);
        let json = serde_json::to_string(&c).unwrap();
        let back: Crop = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind(), CropVariety::Etherbloom);
        // 舊存檔（無 kind 欄）反序列化退回預設主食穀——向後相容、不破壞玩家資料。
        let legacy: Crop =
            serde_json::from_str(r#"{"growth":12.0,"moisture":30.0,"parched":0.0}"#).unwrap();
        assert_eq!(legacy.kind(), CropVariety::Staple);
        // ROADMAP 476：舊存檔無 ripe_secs／pecked 欄 → 安全退回 0／false（剛熟、沒被啄）。
        assert_eq!(legacy.ripe_secs(), 0.0);
        assert!(!legacy.is_pecked());
    }

    #[test]
    fn ripe_secs_accrues_only_after_ripe() {
        // ROADMAP 476：成熟前不累積曝露秒數（成長中還在長，不是「等著被收」）。
        let mut c = Crop::plant();
        c.water();
        c.grow(60.0); // 長到 60 < RIPE_AT(90)，未熟
        assert!(!c.is_ripe());
        assert_eq!(c.ripe_secs(), 0.0, "未熟不累積曝露");
        // 熟了之後即使沒水也持續累積曝露秒數（等著被收或被啄）。
        c.water();
        c.grow(30.0); // 補到 90＝成熟
        assert!(c.is_ripe());
        let before = c.ripe_secs();
        c.grow(20.0); // 成熟後再推 20 秒（這時可能已沒水）
        assert!(c.ripe_secs() >= before + 20.0 - 1e-3, "成熟後逐 tick 累積曝露秒數");
    }

    #[test]
    fn pecked_drops_quality_one_tier() {
        // ROADMAP 476：用心照顧到優質，但被田鴉啄食 → 收成品質折一階成「用心」。
        // 一次澆水只給 MOISTURE_PER_WATER(60) 濕度，撐不到 RIPE_AT(90)；及時補第二次水
        // 讓全程不渴＝優質（不補水會在乾旱中累積渴秒數而掉階，那是 406 的事、非本測本意）。
        let mut c = Crop::plant();
        c.water();
        c.grow(MOISTURE_PER_WATER); // 第一段：長到 60（濕度剛好用完，未渴）
        c.water();
        c.grow(RIPE_AT - MOISTURE_PER_WATER); // 第二段：補水後長到 90＝成熟，全程不渴
        assert!(c.is_ripe());
        assert_eq!(c.quality(), CropQuality::Premium);
        c.mark_pecked();
        assert!(c.is_pecked());
        assert_eq!(c.quality(), CropQuality::Fine, "被啄食 → 品質降一階");
    }

    #[test]
    fn harvest_resets_pecked_and_ripe_secs() {
        // ROADMAP 476：收成後這格回到乾淨的新種子——曝露計時與被啄旗標都歸零（不殘留）。
        let mut c = Crop::plant_kind(CropVariety::Sprout);
        c.water();
        c.grow(MOISTURE_PER_WATER); // 速生菜一次水就熟
        c.grow(crate::crop_raid::RAID_EXPOSURE_SECS); // 久置一段
        c.mark_pecked();
        assert!(c.ripe_secs() > 0.0 && c.is_pecked());
        assert!(c.harvest().is_some());
        assert_eq!(c.ripe_secs(), 0.0, "收成後曝露計時歸零");
        assert!(!c.is_pecked(), "收成後被啄旗標清除");
    }

    // ROADMAP 501 作物熟成倒數單元測試 ────────────────────────────────

    #[test]
    fn eta_secs_ripe_crop_is_zero() {
        // 成熟作物 eta_secs 必為 0。
        let mut c = Crop::plant();
        c.water();
        c.grow(MOISTURE_PER_WATER);
        c.water();
        c.grow(RIPE_AT - MOISTURE_PER_WATER);
        assert!(c.is_ripe());
        assert_eq!(c.eta_secs(), 0, "成熟作物應回 0");
    }

    #[test]
    fn eta_secs_fresh_seed_matches_full_grow_time() {
        // 剛播下的主食穀（grow=0, rate=1.0）：eta ≈ RIPE_AT 秒。
        let c = Crop::plant(); // 主食穀
        let eta = c.eta_secs() as f32;
        assert!(
            (eta - RIPE_AT).abs() < 1.0,
            "剛播主食穀 eta ≈ {RIPE_AT}，實際 {eta}"
        );
    }

    #[test]
    fn eta_secs_sprout_faster_than_staple() {
        // 速生菜（grow_rate > 1.0）應比主食穀長得快，eta 應更短。
        let staple = Crop::plant(); // Staple
        let sprout = Crop::plant_kind(crate::crop_variety::CropVariety::Sprout);
        assert!(
            sprout.eta_secs() < staple.eta_secs(),
            "速生菜 eta 應 < 主食穀 eta"
        );
    }

    #[test]
    fn eta_secs_etherbloom_slower_than_staple() {
        // 乙太瓜（grow_rate < 1.0）應比主食穀長得慢，eta 應更長。
        let staple = Crop::plant();
        let etherbloom = Crop::plant_kind(crate::crop_variety::CropVariety::Etherbloom);
        assert!(
            etherbloom.eta_secs() > staple.eta_secs(),
            "乙太瓜 eta 應 > 主食穀 eta"
        );
    }

    #[test]
    fn eta_secs_decreases_as_crop_grows() {
        // 作物越成熟，eta_secs 越小。
        let mut c = Crop::plant();
        c.water();
        let eta_start = c.eta_secs();
        c.grow(30.0); // 長一段
        let eta_mid = c.eta_secs();
        assert!(
            eta_mid < eta_start,
            "成長後 eta 應遞減：{eta_mid} < {eta_start}"
        );
    }

    #[test]
    fn eta_secs_never_exceeds_u16_max() {
        // 任何合法品種 eta_secs 都在 u16 範圍內，不 overflow。
        for v in &crate::crop_variety::CropVariety::ALL {
            let c = Crop::plant_kind(*v);
            let _ = c.eta_secs(); // 不 panic、不 overflow
        }
    }
}
