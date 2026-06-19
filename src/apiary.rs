//! 養蜂釀蜜（ROADMAP 412）：玩家在自家農地安置蜂箱，蜜蜂隨真實時間釀蜜。
//!
//! 核心湧現：產蜜速率隨「自家田裡正在生長的作物數（蜜源）」放大——蜜源越豐，
//! 蜂蜜釀得越快越多。把農耕與養蜂接成一條雙向羈絆：你越用心種田、田裡越多作物
//! 抽穗開花，蜂群回報你的蜂蜜就越多。沒在種田時蜂群仍從野地採得零星花蜜（基礎速率），
//! 只是慢得多。蜂巢滿了蜂群便歇息，輕輕提醒玩家來採收、把循環接下去（療癒向、無懲罰）。
//!
//! 純記憶體（重啟歸零、零持久化、零 migration），純邏輯抽成可測函式。
//! 蜂蜜本身是 `ItemKind::Honey`（甜食，可食用回一點血＋暖食飽足，亦可賣 NPC）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── 平衡常數 ────────────────────────────────────────────────────────────────
/// 安置蜂箱的乙太成本（略高於購雞 15，因蜂蜜是被動產出）。
pub const PLACE_HIVE_COST: u32 = 20;
/// 蜂巢蜂蜜容量上限（滿了蜂群歇息、不再累積，提醒採收）。
pub const MAX_HONEY: u32 = 12;
/// 每罐蜂蜜採收時給的農夫熟練度 XP。
pub const HARVEST_XP_PER_HONEY: u32 = 3;

/// 無作物時的基礎產蜜速率（每秒花蜜進度；滿 1.0 釀成 1 罐）。
/// 1/120 → 空田時約 120 秒釀一罐（野地零星花蜜，慢）。
pub const BASE_NECTAR_PER_SEC: f32 = 1.0 / 120.0;
/// 每株「生長中作物」額外貢獻的每秒花蜜進度。
pub const NECTAR_PER_BLOOM: f32 = 1.0 / 900.0;
/// 蜜源加成計入的作物數上限（防爆；田再滿也不無限加速）。
pub const BLOOM_RATE_CAP: u32 = 12;

// ── 蜂巢成熟階段（純看蜂蜜占比，給前端/面板顯示用）───────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiveStage {
    /// 空巢（honey == 0）
    Empty,
    /// 漸滿（0 < honey < 半）
    Filling,
    /// 滿溢前（半 <= honey < 滿）
    Brimming,
    /// 滿巢（honey >= MAX_HONEY，蜂群歇息）
    Full,
}

impl HiveStage {
    /// 由蜂蜜罐數判定階段。半 = `MAX_HONEY / 2`（以乘法避免整除截斷的邊界歧義）。
    pub fn from_honey(honey: u32) -> Self {
        if honey == 0 {
            HiveStage::Empty
        } else if honey >= MAX_HONEY {
            HiveStage::Full
        } else if honey * 2 >= MAX_HONEY {
            HiveStage::Brimming
        } else {
            HiveStage::Filling
        }
    }

    /// 對前端的穩定字串契約（前端據此換蜂巢外觀/色調）。
    pub fn wire(self) -> &'static str {
        match self {
            HiveStage::Empty => "empty",
            HiveStage::Filling => "filling",
            HiveStage::Brimming => "brimming",
            HiveStage::Full => "full",
        }
    }
}

/// 純函式：依蜜源（生長中作物數）算每秒花蜜累積速率。
/// 封頂防爆；blooms 直接是 u32，天然非負，無 NaN 之虞。
pub fn nectar_rate(blooms: u32) -> f32 {
    let capped = blooms.min(BLOOM_RATE_CAP);
    BASE_NECTAR_PER_SEC + NECTAR_PER_BLOOM * capped as f32
}

/// 採蜜結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HoneyHarvest {
    /// 收到的蜂蜜罐數。
    pub honey: u32,
    /// 農夫熟練度 XP。
    pub xp: u32,
}

/// 單一玩家的蜂巢狀態（記憶體模式）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hive {
    /// 已釀好的蜂蜜（0..=MAX_HONEY）。
    pub honey: u32,
    /// 花蜜醞釀進度（[0,1)），累滿 1.0 釀成一罐蜂蜜。
    pub nectar: f32,
    /// 上次推進時記下的蜜源（自家田裡生長中作物數，已封頂）。
    /// 供快照／面板顯示「目前蜜源」，免得快照時還要回查農地。
    pub last_blooms: u32,
}

impl Default for Hive {
    fn default() -> Self {
        Hive { honey: 0, nectar: 0.0, last_blooms: 0 }
    }
}

impl Hive {
    pub fn new() -> Self {
        Hive::default()
    }

    /// 推進 `dt` 秒：依附近蜜源（生長中作物數）累積花蜜、滿 1.0 釀成一罐蜂蜜。
    /// 蜂巢已滿則蜂群歇息（不累積、花蜜進度清零，等採收騰出空間）。
    /// 對非正/非有限 `dt` 保守 no-op（守住啟動瞬間的怪異 dt）。
    pub fn accumulate(&mut self, dt: f32, blooms: u32) {
        // 記下目前蜜源（封頂），供快照/面板顯示——即便 dt 不正常也要反映現況。
        self.last_blooms = blooms.min(BLOOM_RATE_CAP);
        if !(dt > 0.0) || !dt.is_finite() {
            return;
        }
        if self.honey >= MAX_HONEY {
            self.nectar = 0.0; // 滿巢歇息，不浪費也不溢出
            return;
        }
        self.nectar += nectar_rate(blooms) * dt;
        while self.nectar >= 1.0 && self.honey < MAX_HONEY {
            self.honey += 1;
            self.nectar -= 1.0;
        }
        if self.honey >= MAX_HONEY {
            self.nectar = 0.0; // 釀到滿即歇息
        }
    }

    pub fn stage(&self) -> HiveStage {
        HiveStage::from_honey(self.honey)
    }

    /// 採收：取走所有蜂蜜、巢清空，回報蜂蜜罐數與農夫熟練度。
    /// 花蜜醞釀進度（`nectar`）刻意不清零——採收不浪費正醞釀中的那部分（療癒向、不懲罰）。
    pub fn harvest(&mut self) -> HoneyHarvest {
        let honey = self.honey;
        self.honey = 0;
        HoneyHarvest {
            honey,
            xp: honey.saturating_mul(HARVEST_XP_PER_HONEY),
        }
    }
}

/// 快照裡的可見蜂巢狀態（按 owner 鍵，前端用 owner 對到該玩家農地座標渲染蜂箱）。
/// 新欄位皆 `#[serde(default)]` 向後相容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiveView {
    /// 蜂巢主人。前端比對自己的 id：相同才在面板顯示採收鈕；所有人都看得到世界裡的蜂箱。
    pub owner: Uuid,
    /// 已釀好的蜂蜜罐數。
    pub honey: u32,
    /// 成熟階段 wire key（empty/filling/brimming/full）。
    #[serde(default)]
    pub stage: String,
    /// 目前蜜源（自家田裡生長中作物數，已封頂），供面板顯示「蜜源越豐、產蜜越快」。
    #[serde(default)]
    pub blooms: u32,
}

/// 全服蜂巢匯集（記憶體模式，按 owner Uuid 鍵；一人一巢）。
#[derive(Debug, Default)]
pub struct ApiaryRegistry {
    hives: HashMap<Uuid, Hive>,
}

impl ApiaryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 安置蜂箱：已有蜂箱則回 `false`（一人一巢，不重複扣費）。
    pub fn place_hive(&mut self, owner: Uuid) -> bool {
        if self.hives.contains_key(&owner) {
            return false;
        }
        self.hives.insert(owner, Hive::new());
        true
    }

    pub fn has_hive(&self, owner: Uuid) -> bool {
        self.hives.contains_key(&owner)
    }

    /// 採蜜：無巢或空巢回 `None`（空巢不白費一次操作）。
    pub fn harvest(&mut self, owner: Uuid) -> Option<HoneyHarvest> {
        let hive = self.hives.get_mut(&owner)?;
        if hive.honey == 0 {
            return None;
        }
        Some(hive.harvest())
    }

    /// 每 tick 推進所有蜂巢。`blooms_by_owner` 給每位巢主自家田裡的蜜源作物數（缺則視為 0）。
    pub fn tick(&mut self, dt: f32, blooms_by_owner: &HashMap<Uuid, u32>) {
        for (owner, hive) in self.hives.iter_mut() {
            let blooms = blooms_by_owner.get(owner).copied().unwrap_or(0);
            hive.accumulate(dt, blooms);
        }
    }

    /// 快照：每個巢主一筆。蜜源取自蜂巢上次推進時記下的 `last_blooms`（與 tick 同源）。
    pub fn all_views(&self) -> Vec<HiveView> {
        self.hives
            .iter()
            .map(|(owner, hive)| HiveView {
                owner: *owner,
                honey: hive.honey,
                stage: hive.stage().wire().to_string(),
                blooms: hive.last_blooms,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn nectar_rate_scales_with_blooms_and_caps() {
        let empty = nectar_rate(0);
        let some = nectar_rate(5);
        let full = nectar_rate(BLOOM_RATE_CAP);
        let over = nectar_rate(BLOOM_RATE_CAP + 100);
        assert_eq!(empty, BASE_NECTAR_PER_SEC);
        assert!(some > empty, "蜜源越多速率越快");
        assert!(full > some);
        assert_eq!(full, over, "超過上限不再加速（防爆）");
    }

    #[test]
    fn hive_stage_thresholds() {
        assert_eq!(HiveStage::from_honey(0), HiveStage::Empty);
        assert_eq!(HiveStage::from_honey(1), HiveStage::Filling);
        // MAX_HONEY = 12，半 = 6
        assert_eq!(HiveStage::from_honey(5), HiveStage::Filling);
        assert_eq!(HiveStage::from_honey(6), HiveStage::Brimming);
        assert_eq!(HiveStage::from_honey(MAX_HONEY - 1), HiveStage::Brimming);
        assert_eq!(HiveStage::from_honey(MAX_HONEY), HiveStage::Full);
        assert_eq!(HiveStage::from_honey(MAX_HONEY + 5), HiveStage::Full);
    }

    #[test]
    fn accumulate_brews_honey_over_time() {
        let mut h = Hive::new();
        // 空田基礎速率 1/120：累積 130 秒應釀成至少 1 罐。
        h.accumulate(130.0, 0);
        assert!(h.honey >= 1, "空田久了也該釀出蜂蜜：{}", h.honey);
    }

    #[test]
    fn blooms_speed_up_brewing() {
        let mut barren = Hive::new();
        let mut lush = Hive::new();
        barren.accumulate(60.0, 0);
        lush.accumulate(60.0, BLOOM_RATE_CAP);
        assert!(
            lush.honey >= barren.honey,
            "蜜源豐的田同時間釀更多蜜：lush={} barren={}",
            lush.honey,
            barren.honey
        );
        // 滿蜜源 60 秒（rate≈0.0217）約釀 1 罐，空田 60 秒（rate≈0.0083）通常 0 罐。
        assert!(lush.honey >= 1);
    }

    #[test]
    fn accumulate_caps_at_max_and_rests() {
        let mut h = Hive::new();
        // 餵超長時間，應停在上限、花蜜進度清零（蜂群歇息）。
        h.accumulate(100_000.0, BLOOM_RATE_CAP);
        assert_eq!(h.honey, MAX_HONEY);
        assert_eq!(h.nectar, 0.0);
        // 滿巢再推進不再變動。
        let before = h;
        h.accumulate(500.0, BLOOM_RATE_CAP);
        assert_eq!(h, before, "滿巢歇息：不再累積");
    }

    #[test]
    fn accumulate_ignores_bad_dt() {
        let mut h = Hive::new();
        h.accumulate(0.0, BLOOM_RATE_CAP);
        h.accumulate(-5.0, BLOOM_RATE_CAP);
        h.accumulate(f32::NAN, BLOOM_RATE_CAP);
        h.accumulate(f32::INFINITY, BLOOM_RATE_CAP);
        assert_eq!(h.honey, 0);
        assert_eq!(h.nectar, 0.0);
    }

    #[test]
    fn harvest_takes_all_and_grants_xp() {
        let mut h = Hive::new();
        h.honey = 4;
        h.nectar = 0.5;
        let out = h.harvest();
        assert_eq!(out.honey, 4);
        assert_eq!(out.xp, 4 * HARVEST_XP_PER_HONEY);
        assert_eq!(h.honey, 0);
        assert_eq!(h.nectar, 0.5, "採收不浪費正在醞釀的花蜜");
    }

    #[test]
    fn place_hive_is_idempotent_per_owner() {
        let mut reg = ApiaryRegistry::new();
        let a = uid(1);
        assert!(reg.place_hive(a), "首次安置成功");
        assert!(!reg.place_hive(a), "已有蜂箱不重複安置");
        assert!(reg.has_hive(a));
        assert!(!reg.has_hive(uid(2)));
    }

    #[test]
    fn registry_harvest_guards_empty_and_missing() {
        let mut reg = ApiaryRegistry::new();
        let a = uid(1);
        assert_eq!(reg.harvest(a), None, "無巢回 None");
        reg.place_hive(a);
        assert_eq!(reg.harvest(a), None, "空巢回 None、不白費操作");
        // 灌入蜜源後釀蜜再採收。
        let mut blooms = HashMap::new();
        blooms.insert(a, BLOOM_RATE_CAP);
        reg.tick(200.0, &blooms);
        let out = reg.harvest(a).expect("應已釀出蜂蜜");
        assert!(out.honey >= 1);
    }

    #[test]
    fn tick_uses_per_owner_blooms() {
        let mut reg = ApiaryRegistry::new();
        let rich = uid(1);
        let poor = uid(2);
        reg.place_hive(rich);
        reg.place_hive(poor);
        let mut blooms = HashMap::new();
        blooms.insert(rich, BLOOM_RATE_CAP);
        // poor 沒列入 → 視為 0 蜜源。
        reg.tick(90.0, &blooms);
        let views = reg.all_views();
        let rv = views.iter().find(|v| v.owner == rich).unwrap();
        let pv = views.iter().find(|v| v.owner == poor).unwrap();
        assert!(rv.honey >= pv.honey, "蜜源豐者釀得不少於蜜源貧者");
        assert_eq!(rv.blooms, BLOOM_RATE_CAP);
        assert_eq!(pv.blooms, 0);
    }

    #[test]
    fn views_carry_stage_wire() {
        let mut reg = ApiaryRegistry::new();
        let a = uid(1);
        reg.place_hive(a);
        let v = &reg.all_views()[0];
        assert_eq!(v.stage, "empty");
        assert_eq!(v.honey, 0);
    }
}
