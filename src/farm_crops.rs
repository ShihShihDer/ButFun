//! 農田地塊作物種植系統（ROADMAP 49）——農田地塊上種植小麥、胡蘿蔔、馬鈴薯。
//!
//! 玩家在自己購買的農田（Farm）類型地塊上種植作物；作物自動生長（90 秒成熟），
//! 成熟後點「收割」進背包，同時給農夫熟練度 XP。
//! 作物收穫物可賣給 NPC，或在合成台做成食物（麵包/蔬菜湯/焗烤馬鈴薯）。
//!
//! 設計取捨：
//!   - **記憶體模式**：同 ranching.rs，重啟後作物清空，玩家重新種植。零 migration 風險。
//!   - **每塊農田最多 3 株作物**：對稱雞隻上限，不讓農田無限刷材料。
//!   - **90 秒成熟**：比雞蛋（60 秒）慢一點，讓種田和養雞的節奏有區別。
//!   - **三種作物不同種植成本**：小麥最便宜（10 乙太）、胡蘿蔔中等（12）、馬鈴薯最貴（15），
//!     但收益（NPC 收購價 + 食物回血量）也隨之遞增——「多條路徑」設計（ROADMAP 39 立規）。

use std::collections::{BTreeMap, HashMap};

use crate::inventory::ItemKind;

/// 作物種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropKind {
    Wheat,
    Carrot,
    Potato,
}

impl CropKind {
    /// 從 wire key 解析（ws.rs 收到的 crop_type 字串）。未知回 None。
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "wheat"  => Some(Self::Wheat),
            "carrot" => Some(Self::Carrot),
            "potato" => Some(Self::Potato),
            _        => None,
        }
    }

    /// 對應的收穫物品種類。
    pub fn harvest_item(self) -> ItemKind {
        match self {
            Self::Wheat  => ItemKind::WheatGrain,
            Self::Carrot => ItemKind::Carrot,
            Self::Potato => ItemKind::Potato,
        }
    }

    /// 種植費用（乙太）。
    pub fn plant_cost(self) -> u32 {
        match self {
            Self::Wheat  => PLANT_COST_WHEAT,
            Self::Carrot => PLANT_COST_CARROT,
            Self::Potato => PLANT_COST_POTATO,
        }
    }

    /// 前端顯示 emoji。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wheat  => "wheat",
            Self::Carrot => "carrot",
            Self::Potato => "potato",
        }
    }
}

/// 種植小麥的乙太費用。
pub const PLANT_COST_WHEAT: u32 = 10;
/// 種植胡蘿蔔的乙太費用。
pub const PLANT_COST_CARROT: u32 = 12;
/// 種植馬鈴薯的乙太費用。
pub const PLANT_COST_POTATO: u32 = 15;

/// 作物從種下到成熟所需秒數。
pub const GROW_TIME_SECS: f32 = 90.0;

/// 「正當時」黃金熟成期長度（秒，ROADMAP 560）。作物剛成熟後的這段時間風味正盛、
/// 採收最划算；過了這段才收，作物已不夠新鮮，只給基準收成（不倒扣，療癒向）。
/// 玩家若留心倒數、在這段時間回來採收，就多得一份——把「成熟即定格、何時收都一樣」
/// 變成一個值得抓準的時機（鏡像 552 打坐三重定境「撐到正點換更甜，還是隨手收」的取捨）。
pub const GOLDEN_WINDOW_SECS: f32 = 45.0;

/// 在「正當時」採收，每株作物多得的收穫數。封頂正向加成、無倒扣，呼應 soil_vitality
/// 「純正向」設計：只獎勵用心抓準時機的玩家，不懲罰隨手採收的玩家。
pub const GOLDEN_BONUS_PER_CROP: u32 = 1;

/// 成熟後計時的封頂（秒）：只需區分「還在黃金期內／已過期」，超過即夾住，避免浮點無限增長。
const RIPE_TIMER_CAP: f32 = GOLDEN_WINDOW_SECS + 1.0;

fn is_zero_eta(v: &u16) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// 每塊農田最多可種的作物株數。
pub const MAX_CROPS: usize = 3;

/// 每次收割操作給農夫熟練度 XP（有成熟作物才算一次收割）。
pub const HARVEST_FARMER_XP: u32 = 10;

/// 一株作物的狀態。
#[derive(Debug, Clone)]
pub struct CropSlot {
    pub kind: CropKind,
    /// 已成長秒數（達到 GROW_TIME_SECS 即成熟）。
    pub grow_timer: f32,
    /// 成熟後經過的秒數（ROADMAP 560）：尚未成熟時為 0，成熟後每 tick 累積、封頂於 `RIPE_TIMER_CAP`。
    /// 用來判定是否仍在「正當時」黃金熟成期內。
    pub ripe_timer: f32,
}

impl CropSlot {
    pub fn new(kind: CropKind) -> Self {
        Self { kind, grow_timer: 0.0, ripe_timer: 0.0 }
    }

    pub fn is_ripe(&self) -> bool {
        self.grow_timer >= GROW_TIME_SECS
    }

    /// 是否正處於「正當時」黃金熟成期（ROADMAP 560）：已成熟、且成熟後尚未超過 `GOLDEN_WINDOW_SECS`。
    /// 在這段時間採收每株多得 `GOLDEN_BONUS_PER_CROP`。純函式、只看自身狀態、好測。
    pub fn is_golden(&self) -> bool {
        self.is_ripe() && self.ripe_timer <= GOLDEN_WINDOW_SECS
    }

    /// 熟成進度 [0,1]（`grow_timer / GROW_TIME_SECS`，夾住上下界；壞值回 0）。
    /// 與城鎮公田 `crops.rs` 的進度同口徑，供前端在個人地塊作物 sprite 下畫熟成進度條
    /// （ROADMAP 457，對齊公田 421）。純函式、只看自身狀態，好測。
    pub fn progress(&self) -> f32 {
        let p = self.grow_timer / GROW_TIME_SECS;
        if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// 單一農田地塊的作物狀態（記憶體模式）。
#[derive(Debug, Default)]
pub struct FarmCropState {
    /// 正在生長中或已成熟的作物槽（上限 MAX_CROPS）。
    pub crops: Vec<CropSlot>,
}

impl FarmCropState {
    fn new() -> Self {
        Self::default()
    }
}

/// 全伺服器所有農田地塊的作物狀態（記憶體模式）。
#[derive(Default)]
pub struct FarmCropRegistry {
    plots: HashMap<u32, FarmCropState>,
}

impl FarmCropRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 嘗試在 `plot_id` 農田地塊種植 `kind` 作物。
    /// - 失敗條件：已達 MAX_CROPS。
    /// - 成功：加入新作物槽，回 `true`。
    pub fn plant(&mut self, plot_id: u32, kind: CropKind) -> bool {
        let state = self.plots.entry(plot_id).or_insert_with(FarmCropState::new);
        if state.crops.len() >= MAX_CROPS {
            return false;
        }
        state.crops.push(CropSlot::new(kind));
        true
    }

    /// 收割 `plot_id` 農田地塊所有成熟作物。
    /// 回傳 `HarvestYield`；無成熟作物回空收成（items 為空、xp 與 golden 皆 0）。
    /// 收穫物清單含各種作物的 (ItemKind, 數量)——同種合併。
    /// ROADMAP 560：在「正當時」黃金熟成期內採收的作物，每株多得 `GOLDEN_BONUS_PER_CROP`。
    pub fn harvest(&mut self, plot_id: u32) -> HarvestYield {
        let state = match self.plots.get_mut(&plot_id) {
            Some(s) => s,
            None    => return HarvestYield::empty(),
        };
        let ripe_count = state.crops.iter().filter(|c| c.is_ripe()).count();
        if ripe_count == 0 {
            return HarvestYield::empty();
        }
        // 分批統計各種作物收穫數，每株成熟作物給 2 個（確定性，簡單直接）；
        // 正當時採收者每株再多 GOLDEN_BONUS_PER_CROP。
        let mut totals: BTreeMap<ItemKind, u32> = BTreeMap::new();
        let mut golden_crops = 0u32;
        let mut bonus_items = 0u32;
        state.crops.retain(|c| {
            if c.is_ripe() {
                let bonus = if c.is_golden() { GOLDEN_BONUS_PER_CROP } else { 0 };
                *totals.entry(c.kind.harvest_item()).or_default() += 2 + bonus;
                if bonus > 0 {
                    golden_crops += 1;
                    bonus_items += bonus;
                }
                false // 移除這株（收割後消失）
            } else {
                true  // 未成熟的留著繼續長
            }
        });
        HarvestYield {
            items: totals.into_iter().collect(),
            farmer_xp: HARVEST_FARMER_XP,
            golden_crops,
            bonus_items,
        }
    }

    /// 取得某地塊的作物快照（供廣播用）。地塊不存在回空 vec。
    pub fn state_of(&self, plot_id: u32) -> Vec<CropSlotView> {
        self.plots.get(&plot_id)
            .map(|s| s.crops.iter().map(|c| {
                let ripe = c.is_ripe();
                CropSlotView {
                    kind: c.kind.as_str().to_string(),
                    ripe,
                    golden: c.is_golden(),
                    grow: (c.progress() * 100.0).round() as u8,
                    eta_secs: if ripe { 0 } else {
                        (GROW_TIME_SECS - c.grow_timer).max(0.0).ceil().min(u16::MAX as f32) as u16
                    },
                }
            }).collect())
            .unwrap_or_default()
    }

    /// 每遊戲 tick 推進所有地塊的作物生長計時器。
    /// `rain_bonus`：下雨時為 true，作物成長速度提升 50%（ROADMAP 109）。
    pub fn tick(&mut self, dt: f32, rain_bonus: bool) {
        // 雨水滋潤：成長速度提升 1.5 倍。
        let effective = if rain_bonus { dt * 1.5 } else { dt };
        for state in self.plots.values_mut() {
            for crop in state.crops.iter_mut() {
                if !crop.is_ripe() {
                    crop.grow_timer = (crop.grow_timer + effective).min(GROW_TIME_SECS);
                } else {
                    // 已成熟：累積「成熟後經過時間」以判定正當時黃金期。
                    // 用原始 dt（雨水加速生長，不影響熟成後的時間流逝）；封頂避免浮點無限增長。
                    crop.ripe_timer = (crop.ripe_timer + dt).min(RIPE_TIMER_CAP);
                }
            }
        }
    }

    /// 匯出有作物的地塊快照（供 Snapshot 廣播）。
    pub fn all_active_views(&self) -> Vec<FarmCropPlotView> {
        self.plots.iter()
            .filter(|(_, s)| !s.crops.is_empty())
            .map(|(&plot_id, s)| FarmCropPlotView {
                plot_id,
                crops: s.crops.iter().map(|c| {
                    let ripe = c.is_ripe();
                    CropSlotView {
                        kind: c.kind.as_str().to_string(),
                        ripe,
                        golden: c.is_golden(),
                        grow: (c.progress() * 100.0).round() as u8,
                        eta_secs: if ripe { 0 } else {
                            (GROW_TIME_SECS - c.grow_timer).max(0.0).ceil().min(u16::MAX as f32) as u16
                        },
                    }
                }).collect(),
            })
            .collect()
    }
}

/// 快照裡一株作物的可見狀態。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CropSlotView {
    pub kind: String,
    pub ripe: bool,
    /// ROADMAP 560「正當時」黃金熟成期旗標：剛成熟、仍在黃金期內＝true，前端據此在作物上
    /// 畫 ✨ 提示「現在收最划算」。由 `ripe_timer` 即時推導、零持久化新欄；
    /// `serde(default, skip)` 在非黃金期省略以節流，舊前端忽略即可（向後相容）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub golden: bool,
    /// 熟成進度百分比（0~100，成熟＝100；ROADMAP 457）。前端在 sprite 下畫熟成進度條。
    /// 由 `grow_timer` 即時推導、不入存檔（零持久化新欄）；Serialize-only，舊前端忽略即可。
    pub grow: u8,
    /// ROADMAP 501 作物熟成倒數：距成熟的估計剩餘秒數（以無雨條件計；下雨時實際更快）。
    /// 已成熟一律 0；Serialize-only（`serde(default)` 省略 0 值減少流量）。舊前端忽略即可。
    #[serde(default, skip_serializing_if = "is_zero_eta")]
    pub eta_secs: u16,
}

/// 快照裡一塊農田地塊的作物可見狀態（送給前端）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FarmCropPlotView {
    pub plot_id: u32,
    pub crops: Vec<CropSlotView>,
}

/// 一次收割的成果（ROADMAP 560）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HarvestYield {
    /// 收穫物清單（各種作物的 ItemKind 與數量，同種已合併）。
    pub items: Vec<(ItemKind, u32)>,
    /// 本次收割給的農夫熟練度 XP。
    pub farmer_xp: u32,
    /// 在「正當時」黃金期內採收的作物株數。
    pub golden_crops: u32,
    /// 因「正當時」多得的收穫總數。
    pub bonus_items: u32,
}

impl HarvestYield {
    /// 空收成（無成熟作物／地塊不存在）。
    fn empty() -> Self {
        Self::default()
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 種植基本流程：首次種植成功，作物出現在地塊。
    #[test]
    fn plant_success() {
        let mut reg = FarmCropRegistry::new();
        assert!(reg.plant(0, CropKind::Wheat));
        assert_eq!(reg.state_of(0).len(), 1);
    }

    /// 同一地塊可種至 MAX_CROPS，超出則拒絕。
    #[test]
    fn plant_respects_max() {
        let mut reg = FarmCropRegistry::new();
        for _ in 0..MAX_CROPS {
            assert!(reg.plant(1, CropKind::Carrot));
        }
        assert!(!reg.plant(1, CropKind::Potato), "超過上限應被拒");
        assert_eq!(reg.state_of(1).len(), MAX_CROPS);
    }

    /// 新種的作物尚未成熟，收割回空。
    #[test]
    fn harvest_immature_gives_nothing() {
        let mut reg = FarmCropRegistry::new();
        assert!(reg.plant(2, CropKind::Wheat));
        let y = reg.harvest(2);
        assert!(y.items.is_empty());
        assert_eq!(y.farmer_xp, 0);
    }

    /// tick 讓作物成長，到期後成熟。
    #[test]
    fn tick_matures_crop() {
        let mut reg = FarmCropRegistry::new();
        assert!(reg.plant(3, CropKind::Potato));
        reg.tick(GROW_TIME_SECS + 0.1, false);
        let crops = reg.state_of(3);
        assert_eq!(crops.len(), 1);
        assert!(crops[0].ripe, "tick 到期後應成熟");
    }

    /// 收割成熟作物：得到物品、農夫 XP，槽位移除。
    #[test]
    fn harvest_ripe_gives_items_and_xp() {
        let mut reg = FarmCropRegistry::new();
        assert!(reg.plant(4, CropKind::Carrot));
        reg.tick(GROW_TIME_SECS, false);
        let y = reg.harvest(4);
        assert!(!y.items.is_empty(), "應有胡蘿蔔掉落");
        assert_eq!(y.farmer_xp, HARVEST_FARMER_XP);
        assert!(reg.state_of(4).is_empty(), "收割後槽位應清空");
    }

    /// 地塊不存在時收割回空。
    #[test]
    fn harvest_nonexistent_plot() {
        let mut reg = FarmCropRegistry::new();
        let y = reg.harvest(999);
        assert!(y.items.is_empty());
        assert_eq!(y.farmer_xp, 0);
    }

    /// 未成熟作物在收割後仍留在地塊。
    #[test]
    fn harvest_only_removes_ripe_crops() {
        let mut reg = FarmCropRegistry::new();
        // 種 2 株，讓 1 株成熟、1 株未成熟。
        assert!(reg.plant(5, CropKind::Wheat));
        assert!(reg.plant(5, CropKind::Carrot));
        // 讓第一株成熟。
        {
            let slot = reg.plots.get_mut(&5).unwrap().crops.get_mut(0).unwrap();
            slot.grow_timer = GROW_TIME_SECS;
        }
        let y = reg.harvest(5);
        assert!(!y.items.is_empty(), "應收到成熟的小麥");
        assert_eq!(y.farmer_xp, HARVEST_FARMER_XP);
        // 未成熟的胡蘿蔔應還在。
        assert_eq!(reg.state_of(5).len(), 1, "未成熟胡蘿蔔應留下");
        assert!(!reg.state_of(5)[0].ripe, "留下的應未成熟");
    }

    /// grow_timer 不超過 GROW_TIME_SECS。
    #[test]
    fn grow_timer_capped_at_max() {
        let mut reg = FarmCropRegistry::new();
        assert!(reg.plant(6, CropKind::Potato));
        for _ in 0..20 {
            reg.tick(GROW_TIME_SECS, false);
        }
        let slot = &reg.plots.get(&6).unwrap().crops[0];
        assert!(
            slot.grow_timer <= GROW_TIME_SECS,
            "grow_timer 不應超過 GROW_TIME_SECS"
        );
    }

    /// all_active_views 只回傳有作物的地塊。
    #[test]
    fn all_active_views_only_active_plots() {
        let mut reg = FarmCropRegistry::new();
        reg.plant(7, CropKind::Wheat);
        let views = reg.all_active_views();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].plot_id, 7);
    }

    /// 各作物種植費用差異。
    #[test]
    fn plant_costs_are_ordered() {
        assert!(
            PLANT_COST_WHEAT < PLANT_COST_CARROT && PLANT_COST_CARROT < PLANT_COST_POTATO,
            "種植費用應小麥 < 胡蘿蔔 < 馬鈴薯"
        );
    }

    /// 雨水加成：相同時間下雨天作物比晴天長得更快。
    /// 用 2/3 GROW_TIME_SECS 的 dt：有 1.5x 加成時剛好成熟，無加成時尚未成熟。
    #[test]
    fn rain_bonus_speeds_up_growth() {
        let dt = GROW_TIME_SECS * 2.0 / 3.0; // = 60 秒
        let mut reg_rain = FarmCropRegistry::new();
        reg_rain.plant(10, CropKind::Wheat);
        reg_rain.tick(dt, true); // 60 * 1.5 = 90 → 成熟
        assert!(reg_rain.state_of(10)[0].ripe, "雨天加速後應已成熟");

        let mut reg_dry = FarmCropRegistry::new();
        reg_dry.plant(10, CropKind::Wheat);
        reg_dry.tick(dt, false); // 60 < 90 → 未成熟
        assert!(!reg_dry.state_of(10)[0].ripe, "晴天相同時間應尚未成熟");
    }

    // ─── 熟成進度（ROADMAP 457，對齊公田 421）─────────────────────────────

    /// 進度從 0（剛種）跨到中段再到成熟＝1，且全程夾在 [0,1]。
    #[test]
    fn progress_spans_zero_to_one() {
        let mut c = CropSlot::new(CropKind::Wheat);
        assert_eq!(c.progress(), 0.0, "剛種＝0");
        c.grow_timer = GROW_TIME_SECS / 2.0;
        assert!((c.progress() - 0.5).abs() < 1e-6, "半程＝0.5");
        c.grow_timer = GROW_TIME_SECS; // 成熟
        assert_eq!(c.progress(), 1.0, "成熟＝1");
        c.grow_timer = GROW_TIME_SECS * 2.0; // 過熟仍夾 1
        assert_eq!(c.progress(), 1.0, "過熟仍夾在 1");
    }

    /// 壞值（NaN／負）一律回 0，不汙染前端進度條。
    #[test]
    fn progress_clamps_bad_values() {
        let mut c = CropSlot::new(CropKind::Carrot);
        c.grow_timer = f32::NAN;
        assert_eq!(c.progress(), 0.0, "NaN 退 0");
        c.grow_timer = -10.0;
        assert_eq!(c.progress(), 0.0, "負值夾 0");
    }

    /// 快照 view 的 `grow` 隨 `grow_timer` 正確推進（state_of 與 all_active_views 一致）。
    #[test]
    fn view_grow_tracks_timer() {
        let mut reg = FarmCropRegistry::new();
        reg.plant(20, CropKind::Wheat);
        // 剛種：grow＝0。
        assert_eq!(reg.state_of(20)[0].grow, 0, "剛種 grow＝0");
        // 推進半程：grow≈50。
        reg.tick(GROW_TIME_SECS / 2.0, false);
        let g = reg.state_of(20)[0].grow;
        assert!((49..=51).contains(&g), "半程 grow 應約 50，實得 {g}");
        // 兩處快照建構口徑一致。
        let active = reg.all_active_views();
        let view_g = active.iter().find(|p| p.plot_id == 20).unwrap().crops[0].grow;
        assert_eq!(view_g, g, "state_of 與 all_active_views 的 grow 應一致");
        // 推進到成熟：grow＝100。
        reg.tick(GROW_TIME_SECS, false);
        assert_eq!(reg.state_of(20)[0].grow, 100, "成熟 grow＝100");
    }

    // ─── 「正當時」黃金熟成期（ROADMAP 560）─────────────────────────────────

    /// 剛成熟的作物處於黃金期，採收每株多得 GOLDEN_BONUS_PER_CROP。
    #[test]
    fn fresh_ripe_is_golden_and_pays_bonus() {
        let mut reg = FarmCropRegistry::new();
        reg.plant(30, CropKind::Wheat);
        reg.tick(GROW_TIME_SECS, false); // 剛好成熟，ripe_timer 仍 0 → 黃金期
        assert!(reg.state_of(30)[0].golden, "剛成熟應在正當時");
        let y = reg.harvest(30);
        assert_eq!(y.golden_crops, 1, "一株正當時收成");
        assert_eq!(y.bonus_items, GOLDEN_BONUS_PER_CROP);
        // 小麥每株 2 + 正當時 1 = 3。
        let total: u32 = y.items.iter().map(|(_, n)| *n).sum();
        assert_eq!(total, 2 + GOLDEN_BONUS_PER_CROP, "正當時收成＝基準＋加成");
    }

    /// 拖過黃金期才採收：不再有加成，但仍給基準收成（不倒扣，療癒向）。
    #[test]
    fn overripe_loses_bonus_but_keeps_base() {
        let mut reg = FarmCropRegistry::new();
        reg.plant(31, CropKind::Carrot);
        reg.tick(GROW_TIME_SECS, false);           // 成熟
        reg.tick(GOLDEN_WINDOW_SECS + 1.0, false); // 拖過黃金期
        assert!(!reg.state_of(31)[0].golden, "過了黃金期不該再亮正當時");
        let y = reg.harvest(31);
        assert_eq!(y.golden_crops, 0, "已非正當時，無黃金株");
        assert_eq!(y.bonus_items, 0);
        let total: u32 = y.items.iter().map(|(_, n)| *n).sum();
        assert_eq!(total, 2, "過熟仍給基準收成、不倒扣");
    }

    /// 黃金期邊界：恰在 GOLDEN_WINDOW_SECS 仍算正當時，超過一點即過期。
    #[test]
    fn golden_window_boundary() {
        let mut c = CropSlot::new(CropKind::Potato);
        c.grow_timer = GROW_TIME_SECS;
        c.ripe_timer = GOLDEN_WINDOW_SECS; // 邊界內
        assert!(c.is_golden(), "恰在窗邊界內仍算正當時");
        c.ripe_timer = GOLDEN_WINDOW_SECS + 0.01;
        assert!(!c.is_golden(), "超過窗邊界即過期");
    }

    /// 未成熟的作物永遠不是黃金期，且 ripe_timer 不累積。
    #[test]
    fn immature_never_golden() {
        let mut reg = FarmCropRegistry::new();
        reg.plant(32, CropKind::Wheat);
        reg.tick(GROW_TIME_SECS / 2.0, false);
        assert!(!reg.state_of(32)[0].golden, "未成熟不可能正當時");
        let slot = &reg.plots.get(&32).unwrap().crops[0];
        assert_eq!(slot.ripe_timer, 0.0, "未成熟 ripe_timer 不累積");
    }

    /// ripe_timer 封頂於 RIPE_TIMER_CAP，長期不超載（浮點不無限增長）。
    #[test]
    fn ripe_timer_capped() {
        let mut reg = FarmCropRegistry::new();
        reg.plant(33, CropKind::Wheat);
        for _ in 0..50 {
            reg.tick(GROW_TIME_SECS, false); // 成熟後持續 tick
        }
        let slot = &reg.plots.get(&33).unwrap().crops[0];
        assert!(slot.ripe_timer <= RIPE_TIMER_CAP, "ripe_timer 應被封頂");
    }

    /// 同地塊多株：只有仍在黃金期的那株拿加成，過期的拿基準。
    #[test]
    fn mixed_golden_and_overripe_in_one_plot() {
        let mut reg = FarmCropRegistry::new();
        reg.plant(34, CropKind::Wheat);  // 第 0 株：將拖過黃金期
        reg.tick(GROW_TIME_SECS, false); // 兩株都尚未種第二株——先讓第 0 株成熟
        reg.tick(GOLDEN_WINDOW_SECS + 2.0, false); // 第 0 株過期
        reg.plant(34, CropKind::Wheat);  // 第 1 株：剛種
        reg.tick(GROW_TIME_SECS, false); // 第 1 株成熟（剛好正當時），第 0 株 ripe_timer 已封頂仍過期
        let y = reg.harvest(34);
        assert_eq!(y.golden_crops, 1, "只有新鮮那株正當時");
        assert_eq!(y.bonus_items, GOLDEN_BONUS_PER_CROP);
    }
}
