//! 乙太方界·種田 v1——撒種‧等待‧收割（ROADMAP 659）。
//! 水耕農業 v1（ROADMAP 686）：農田土鄰近水源時生長加速（90s→45s）。
//! 第二種作物 v1：胡蘿蔔——種田系統第一次有兩種作物可選（小麥慢而多用途／胡蘿蔔快而輕巧），
//! 玩家依當下需求（急著要收成 vs 存糧存種子）第一次能真的「選種什麼」。
//! 第三種作物 v1：馬鈴薯——慢熟但收成量大（2 顆／次），三種作物第一次湊出快／中／慢
//! 完整節奏光譜，玩家能依「急著吃」「囤糧」兩種心態真的挑對作物。
//!
//! **純邏輯層**：`FarmStore` + 生長計時純函式，確定性、無副作用、全可測。
//! 鎖 / WS / IO 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。
//!
//! 種植流程（依 [`CropKind`] 分岔）：
//!   FarmSoil(11)  →[Plant seed]→  FarmSoilSeeded(12)  →[~90s / 水耕 ~45s]→  WheatMature(13)
//!   FarmSoil(11)  →[Plant carrot seed]→  CarrotSeeded(46)  →[~60s / 水耕 ~30s]→  CarrotMature(47)
//!   FarmSoil(11)  →[Plant potato seed]→  PotatoSeeded(50)  →[~120s / 水耕 ~60s]→  PotatoMature(51)
//!
//! 收穫：Break WheatMature → Seeds(14)×1 + Wheat(18)×1 + FarmSoil(11)（得顆粒以合麵包）。
//!       Break CarrotMature → CarrotSeeds(48)×1 + Carrot(49)×1 + FarmSoil(11)。
//!       Break PotatoMature → PotatoSeeds(52)×1 + Potato(53)×2 + FarmSoil(11)（量大是特色）。
//! 取消種植：Break FarmSoilSeeded/CarrotSeeded/PotatoSeeded → 對應種子×1 + FarmSoil(11)（退還種子）。
//! 麵包：3 Wheat(18) → Bread(19)（2×2 合成格一排）。
//!
//! FarmStore **純記憶體**（與世界 delta 行為一致：重啟後農地重置）。
//! 之後需持久化再加 jsonl 層，此版先讓玩家看到「有感的農地時間維度」。

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 種子物品 id（純 inventory 物品，無對應 Block enum；Block::from_u8(SEEDS_ID) = None）。
/// 從葉片(6)/成熟小麥(13)/幼苗(12)破壞後掉落。
pub const SEEDS_ID: u8 = 14;

/// 小麥顆粒物品 id（純 inventory 物品，從成熟小麥(13)收割時掉落 ×1）。
/// 3 顆粒在 2×2 合成格合一排 → 1 麵包（療癒農業循環終點）。
pub const WHEAT_ID: u8 = 18;

/// 麵包物品 id（純 inventory 物品，3 小麥顆粒在 2×2 格一排 → 1 麵包）。
/// 可送給居民當禮物（居民特別開心）。
pub const BREAD_ID: u8 = 19;

/// 胡蘿蔔種子物品 id（第二種作物 v1；純 inventory 物品，無對應 Block enum）。
/// 從草地(1)破壞後額外掉落（草地仍照舊掉落自身，種子是附加收穫）。
pub const CARROT_SEEDS_ID: u8 = 48;

/// 胡蘿蔔物品 id（第二種作物 v1；純 inventory 物品，從成熟胡蘿蔔(47)收割時掉落 ×1）。
/// 可送給居民當禮物，也是麵包之外第二種食物類贈禮。
pub const CARROT_ID: u8 = 49;

/// 馬鈴薯種子物品 id（第三種作物 v1；純 inventory 物品，無對應 Block enum）。
/// 從泥土(2)破壞後額外掉落（泥土仍照舊掉落自身，種子是附加收穫，與胡蘿蔔種子取自草地區隔）。
pub const POTATO_SEEDS_ID: u8 = 52;

/// 馬鈴薯物品 id（第三種作物 v1；純 inventory 物品，從成熟馬鈴薯(51)收割時掉落 ×2）。
/// 可送給居民當禮物，也是食物類贈禮之一；慢熟換來的量大收成適合囤糧。
pub const POTATO_ID: u8 = 53;

/// 烤地薯物品 id（烤地薯 v1；純 inventory 物品，住背包不可放置）。
/// 把生馬鈴薯(53)放進熔爐烤成的熱騰騰佳餚，居民最愛的美味贈禮——把種田的收成
/// 也接上「採集→烹飪→餽贈」的療癒循環（比照烤魚 63 之於垂釣）。
/// 由熔爐配方 `smelt_potato`（生馬鈴薯→烤地薯）產出。id 64 是 63(烤魚)之後首個空號。
pub const BAKED_POTATO_ID: u8 = 64;

/// 幼苗成熟所需秒數（~90 秒 = 1.5 分鐘）。調校讓玩家在一輪遊玩中體驗完整循環。
pub const GROW_SECS: u64 = 90;

/// 水耕加速後的生長秒數（有水源鄰近時縮短為原本的一半）。
pub const IRRIGATED_GROW_SECS: u64 = 45;

/// 胡蘿蔔生長秒數（~60 秒）——比小麥快、但收成量小，快慢兩種節奏讓玩家真的有得選。
pub const CARROT_GROW_SECS: u64 = 60;

/// 胡蘿蔔水耕加速後的生長秒數（有水源鄰近時縮短為原本的一半）。
pub const CARROT_IRRIGATED_GROW_SECS: u64 = 30;

/// 馬鈴薯生長秒數（~120 秒）——三種作物中最慢，但收成量最大，補上「囤糧」節奏。
pub const POTATO_GROW_SECS: u64 = 120;

/// 馬鈴薯水耕加速後的生長秒數（有水源鄰近時縮短為原本的一半）。
pub const POTATO_IRRIGATED_GROW_SECS: u64 = 60;

/// 農田土偵測水源的最大曼哈頓距離（X/Z 各 ±4 格、Y 差 ±1 格）。
pub const FARM_WATER_RANGE: i32 = 4;

/// 作物種類（第二種作物 v1）——決定生長秒數與收成方塊/物品。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropKind {
    Wheat,
    Carrot,
    Potato,
}

/// 依作物種類 + 水耕狀態回傳有效生長秒數。
pub fn effective_grow_secs(kind: CropKind, irrigated: bool) -> u64 {
    match (kind, irrigated) {
        (CropKind::Wheat, false) => GROW_SECS,
        (CropKind::Wheat, true) => IRRIGATED_GROW_SECS,
        (CropKind::Carrot, false) => CARROT_GROW_SECS,
        (CropKind::Carrot, true) => CARROT_IRRIGATED_GROW_SECS,
        (CropKind::Potato, false) => POTATO_GROW_SECS,
        (CropKind::Potato, true) => POTATO_IRRIGATED_GROW_SECS,
    }
}

/// 一塊農地的種植記錄。
#[derive(Clone, Debug, PartialEq)]
pub struct FarmPlot {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 種下去的 Unix 秒數（用來判斷是否成熟）。
    pub planted_secs: u64,
    /// 種下的作物種類（第二種作物 v1）。
    pub kind: CropKind,
}

/// 農地 store（純記憶體，重啟後農地重置，與世界 delta 行為一致）。
#[derive(Default)]
pub struct FarmStore {
    plots: HashMap<(i32, i32, i32), FarmPlot>,
}

impl FarmStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 種下種子：記錄農地 + planted_secs + 作物種類。重複種同格 → 覆蓋（重置計時、可換種）。
    pub fn plant(&mut self, x: i32, y: i32, z: i32, now_secs: u64, kind: CropKind) -> FarmPlot {
        let plot = FarmPlot { x, y, z, planted_secs: now_secs, kind };
        self.plots.insert((x, y, z), plot.clone());
        plot
    }

    /// 移除農地記錄（方塊被挖掉 / 成熟後從 store 清掉）。
    pub fn remove(&mut self, x: i32, y: i32, z: i32) {
        self.plots.remove(&(x, y, z));
    }

    /// 此座標是否有農地記錄。
    pub fn has_plot(&self, x: i32, y: i32, z: i32) -> bool {
        self.plots.contains_key(&(x, y, z))
    }

    /// 回傳所有已成熟的農地座標（小麥固定 GROW_SECS，不分作物——舊版相容用途，
    /// 新程式碼請改用 [`FarmStore::mature_plots_irrigated`]，它會依作物種類算對應生長秒數）。
    pub fn mature_plots(&self, now_secs: u64) -> Vec<(i32, i32, i32)> {
        self.plots
            .iter()
            .filter(|(_, p)| now_secs >= p.planted_secs.saturating_add(GROW_SECS))
            .map(|(&coord, _)| coord)
            .collect()
    }

    /// 回傳所有已成熟的農地座標＋其作物種類，考慮水耕加速。
    /// `is_irrigated`：呼叫端提供的閉包，判定某 (x, y, z) 是否鄰近水源。
    /// 依 [`FarmPlot::kind`] + 是否水耕，用 [`effective_grow_secs`] 算對應生長秒數。
    pub fn mature_plots_irrigated<F>(
        &self,
        now_secs: u64,
        is_irrigated: F,
    ) -> Vec<((i32, i32, i32), CropKind)>
    where
        F: Fn(i32, i32, i32) -> bool,
    {
        self.plots
            .iter()
            .filter(|(&(px, py, pz), p)| {
                let grow = effective_grow_secs(p.kind, is_irrigated(px, py, pz));
                now_secs >= p.planted_secs.saturating_add(grow)
            })
            .map(|(&coord, p)| (coord, p.kind))
            .collect()
    }
}

/// 取得目前 Unix 秒數（農地計時用）。
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plant_creates_plot() {
        let mut s = FarmStore::new();
        s.plant(1, 5, 2, 1000, CropKind::Wheat);
        assert!(s.has_plot(1, 5, 2));
        assert!(!s.has_plot(0, 0, 0));
    }

    #[test]
    fn remove_clears_plot() {
        let mut s = FarmStore::new();
        s.plant(3, 5, 7, 1000, CropKind::Wheat);
        s.remove(3, 5, 7);
        assert!(!s.has_plot(3, 5, 7));
    }

    #[test]
    fn immature_before_grow_secs() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 1000, CropKind::Wheat);
        // planted_secs=1000, now=1089（剛好差 89 秒，未達 90 秒門檻）
        assert!(s.mature_plots(1000 + GROW_SECS - 1).is_empty());
    }

    #[test]
    fn mature_at_exactly_grow_secs() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 1000, CropKind::Wheat);
        // planted_secs=1000, now=1090（剛好 90 秒 → 成熟）
        let m = s.mature_plots(1000 + GROW_SECS);
        assert_eq!(m.len(), 1);
        assert!(m.contains(&(0, 5, 0)));
    }

    #[test]
    fn mature_well_past_grow_secs() {
        let mut s = FarmStore::new();
        s.plant(2, 5, 3, 500, CropKind::Wheat);
        let m = s.mature_plots(1000); // 差 500 秒 >> 90 秒
        assert!(m.contains(&(2, 5, 3)));
    }

    #[test]
    fn only_mature_plots_returned() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 1000, CropKind::Wheat); // planted 1000，差10秒未熟
        s.plant(1, 5, 0, 910, CropKind::Wheat);  // planted 910，差100秒 > 90：成熟
        // now_secs = 1010
        let m = s.mature_plots(1010);
        assert_eq!(m.len(), 1);
        assert!(m.contains(&(1, 5, 0)));
        assert!(!m.contains(&(0, 5, 0)));
    }

    #[test]
    fn remove_after_harvest_clears_store() {
        let mut s = FarmStore::new();
        s.plant(5, 5, 5, 0, CropKind::Wheat);
        let mature = s.mature_plots(GROW_SECS);
        for c in &mature {
            s.remove(c.0, c.1, c.2);
        }
        assert!(!s.has_plot(5, 5, 5));
    }

    #[test]
    fn plant_overwrites_resets_timer() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 1000, CropKind::Wheat);
        // 重新種：計時器重置到 2000
        s.plant(0, 5, 0, 2000, CropKind::Wheat);
        // 在 2000+89 時應未熟（以新計時為準）
        assert!(s.mature_plots(2000 + GROW_SECS - 1).is_empty());
        // 2000+90 才熟
        assert_eq!(s.mature_plots(2000 + GROW_SECS).len(), 1);
    }

    #[test]
    fn empty_store_no_mature_plots() {
        let s = FarmStore::new();
        assert!(s.mature_plots(99999).is_empty());
    }

    #[test]
    fn multiple_plots_all_mature() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 0, CropKind::Wheat);
        s.plant(1, 5, 0, 0, CropKind::Wheat);
        s.plant(2, 5, 0, 0, CropKind::Wheat);
        let m = s.mature_plots(GROW_SECS);
        assert_eq!(m.len(), 3);
    }

    // ── 麵包 v1（ROADMAP 668）常數一致性測試 ──────────────────────────────────
    #[test]
    fn item_ids_unique_and_in_range() {
        // 四個物品 id 互不相同
        assert_ne!(SEEDS_ID, WHEAT_ID);
        assert_ne!(SEEDS_ID, BREAD_ID);
        assert_ne!(WHEAT_ID, BREAD_ID);
        // 皆在合法 u8 範圍；14 是 SEEDS 不被方塊 enum 佔用，18/19 同理。
        assert_eq!(SEEDS_ID, 14);
        assert_eq!(WHEAT_ID, 18);
        assert_eq!(BREAD_ID, 19);
    }

    // ── 水耕農業 v1（ROADMAP 686）──────────────────────────────────────────────

    #[test]
    fn effective_grow_secs_values() {
        assert_eq!(effective_grow_secs(CropKind::Wheat, true),  IRRIGATED_GROW_SECS);
        assert_eq!(effective_grow_secs(CropKind::Wheat, false), GROW_SECS);
        // 水耕應比普通快（若 IRRIGATED_GROW_SECS 被誤改就能抓到）。
        assert!(IRRIGATED_GROW_SECS < GROW_SECS, "水耕應比普通生長更快");
    }

    // ── 第二種作物 v1：胡蘿蔔 ────────────────────────────────────────────────────

    #[test]
    fn carrot_item_ids_unique_and_in_range() {
        assert_ne!(CARROT_SEEDS_ID, CARROT_ID);
        assert_ne!(CARROT_SEEDS_ID, SEEDS_ID);
        assert_ne!(CARROT_ID, WHEAT_ID);
        assert_ne!(CARROT_ID, BREAD_ID);
        assert_eq!(CARROT_SEEDS_ID, 48);
        assert_eq!(CARROT_ID, 49);
    }

    #[test]
    fn carrot_grows_faster_than_wheat_both_dry_and_irrigated() {
        assert!(CARROT_GROW_SECS < GROW_SECS, "胡蘿蔔應比小麥快熟");
        assert!(
            CARROT_IRRIGATED_GROW_SECS < IRRIGATED_GROW_SECS,
            "胡蘿蔔水耕也應比小麥水耕快熟"
        );
        assert!(
            CARROT_IRRIGATED_GROW_SECS < CARROT_GROW_SECS,
            "胡蘿蔔水耕應比胡蘿蔔乾燥快熟"
        );
    }

    #[test]
    fn effective_grow_secs_carrot_values() {
        assert_eq!(effective_grow_secs(CropKind::Carrot, false), CARROT_GROW_SECS);
        assert_eq!(effective_grow_secs(CropKind::Carrot, true), CARROT_IRRIGATED_GROW_SECS);
    }

    #[test]
    fn carrot_plot_matures_at_carrot_pace_not_wheat_pace() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 0, CropKind::Carrot);
        // 60 秒時（胡蘿蔔門檻）應成熟。
        let mature = s.mature_plots_irrigated(CARROT_GROW_SECS, |_, _, _| false);
        assert_eq!(mature.len(), 1);
        assert_eq!(mature[0], ((0, 5, 0), CropKind::Carrot));
        // 89 秒（小麥門檻前一秒）不該被誤判為還沒到——胡蘿蔔早在 60 秒就該熟過了，
        // 這裡改用一顆全新的胡蘿蔔驗證「59 秒未熟」。
        let mut s2 = FarmStore::new();
        s2.plant(1, 5, 0, 0, CropKind::Carrot);
        assert!(s2.mature_plots_irrigated(CARROT_GROW_SECS - 1, |_, _, _| false).is_empty());
    }

    #[test]
    fn mixed_wheat_and_carrot_plots_each_use_own_pace() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 0, CropKind::Wheat);  // 90 秒才熟
        s.plant(1, 5, 0, 0, CropKind::Carrot); // 60 秒就熟
        // 60 秒時：只有胡蘿蔔熟。
        let at_60 = s.mature_plots_irrigated(CARROT_GROW_SECS, |_, _, _| false);
        assert_eq!(at_60, vec![((1, 5, 0), CropKind::Carrot)]);
        // 90 秒時：兩者都熟。
        let at_90 = s.mature_plots_irrigated(GROW_SECS, |_, _, _| false);
        assert_eq!(at_90.len(), 2);
    }

    #[test]
    fn replant_can_change_crop_kind() {
        // 同一格重種可換作物種類（覆蓋語意包含換種）。
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 0, CropKind::Wheat);
        s.plant(0, 5, 0, 100, CropKind::Carrot);
        let mature = s.mature_plots_irrigated(100 + CARROT_GROW_SECS, |_, _, _| false);
        assert_eq!(mature, vec![((0, 5, 0), CropKind::Carrot)]);
    }

    // ── 第三種作物 v1：馬鈴薯 ────────────────────────────────────────────────────

    #[test]
    fn potato_item_ids_unique_and_in_range() {
        assert_ne!(POTATO_SEEDS_ID, POTATO_ID);
        assert_ne!(POTATO_SEEDS_ID, SEEDS_ID);
        assert_ne!(POTATO_SEEDS_ID, CARROT_SEEDS_ID);
        assert_ne!(POTATO_ID, WHEAT_ID);
        assert_ne!(POTATO_ID, CARROT_ID);
        assert_ne!(POTATO_ID, BREAD_ID);
        assert_eq!(POTATO_SEEDS_ID, 52);
        assert_eq!(POTATO_ID, 53);
    }

    #[test]
    fn baked_potato_id_unique_and_after_cooked_fish() {
        // 烤地薯 v1：id 64，接在烤魚(63)之後首個空號；不撞任何農作/種子 id。
        assert_eq!(BAKED_POTATO_ID, 64);
        assert_ne!(BAKED_POTATO_ID, POTATO_ID); // 生馬鈴薯 vs 烤地薯是兩件不同物品
        assert_ne!(BAKED_POTATO_ID, POTATO_SEEDS_ID);
        assert_ne!(BAKED_POTATO_ID, BREAD_ID);
        assert_ne!(BAKED_POTATO_ID, CARROT_ID);
        assert!(BAKED_POTATO_ID > 63, "應排在烤魚(63)之後");
    }

    #[test]
    fn potato_grows_slower_than_wheat_and_carrot_both_dry_and_irrigated() {
        assert!(POTATO_GROW_SECS > GROW_SECS, "馬鈴薯應比小麥慢熟");
        assert!(POTATO_GROW_SECS > CARROT_GROW_SECS, "馬鈴薯應比胡蘿蔔慢熟");
        assert!(
            POTATO_IRRIGATED_GROW_SECS < POTATO_GROW_SECS,
            "馬鈴薯水耕應比馬鈴薯乾燥快熟"
        );
    }

    #[test]
    fn effective_grow_secs_potato_values() {
        assert_eq!(effective_grow_secs(CropKind::Potato, false), POTATO_GROW_SECS);
        assert_eq!(effective_grow_secs(CropKind::Potato, true), POTATO_IRRIGATED_GROW_SECS);
    }

    #[test]
    fn potato_plot_matures_at_potato_pace_not_wheat_or_carrot_pace() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 0, CropKind::Potato);
        let mature = s.mature_plots_irrigated(POTATO_GROW_SECS, |_, _, _| false);
        assert_eq!(mature.len(), 1);
        assert_eq!(mature[0], ((0, 5, 0), CropKind::Potato));
        let mut s2 = FarmStore::new();
        s2.plant(1, 5, 0, 0, CropKind::Potato);
        assert!(s2.mature_plots_irrigated(POTATO_GROW_SECS - 1, |_, _, _| false).is_empty());
    }

    #[test]
    fn mixed_wheat_carrot_potato_plots_each_use_own_pace() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 0, CropKind::Wheat);   // 90 秒才熟
        s.plant(1, 5, 0, 0, CropKind::Carrot);  // 60 秒就熟
        s.plant(2, 5, 0, 0, CropKind::Potato);  // 120 秒才熟
        let at_60 = s.mature_plots_irrigated(CARROT_GROW_SECS, |_, _, _| false);
        assert_eq!(at_60, vec![((1, 5, 0), CropKind::Carrot)]);
        let at_90 = s.mature_plots_irrigated(GROW_SECS, |_, _, _| false);
        assert_eq!(at_90.len(), 2); // 小麥 + 胡蘿蔔，馬鈴薯仍未熟
        let at_120 = s.mature_plots_irrigated(POTATO_GROW_SECS, |_, _, _| false);
        assert_eq!(at_120.len(), 3); // 三者皆熟
    }

    #[test]
    fn irrigated_plot_matures_faster() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 0, CropKind::Wheat);
        // 45 秒時：水耕成熟、普通未熟。
        assert_eq!(s.mature_plots_irrigated(IRRIGATED_GROW_SECS, |_, _, _| true).len(), 1);
        assert!(s.mature_plots_irrigated(IRRIGATED_GROW_SECS, |_, _, _| false).is_empty());
    }

    #[test]
    fn non_irrigated_plot_matures_at_normal_time() {
        let mut s = FarmStore::new();
        s.plant(3, 5, 3, 0, CropKind::Wheat);
        // 90 秒時：有水/無水都成熟。
        assert_eq!(s.mature_plots_irrigated(GROW_SECS, |_, _, _| false).len(), 1);
        assert_eq!(s.mature_plots_irrigated(GROW_SECS, |_, _, _| true).len(), 1);
    }

    #[test]
    fn irrigated_not_mature_before_45s() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 0, CropKind::Wheat);
        // 44 秒時即便有水也未熟。
        assert!(s.mature_plots_irrigated(IRRIGATED_GROW_SECS - 1, |_, _, _| true).is_empty());
    }

    #[test]
    fn mixed_irrigated_and_dry_plots() {
        let mut s = FarmStore::new();
        s.plant(0, 5, 0, 0, CropKind::Wheat); // 乾燥農地
        s.plant(10, 5, 0, 0, CropKind::Wheat); // 水耕農地
        // 45 秒時只有水耕的成熟。
        let mature = s.mature_plots_irrigated(IRRIGATED_GROW_SECS, |x, _, _| x == 10);
        assert_eq!(mature.len(), 1);
        assert!(mature.contains(&((10, 5, 0), CropKind::Wheat)));
    }

    #[test]
    fn farm_water_range_is_positive() {
        assert!(FARM_WATER_RANGE > 0, "水耕偵測範圍應大於 0");
    }
}
