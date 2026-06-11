//! 易腐品腐壞系統（ROADMAP 106）。
//!
//! 食物/作物在玩家背包或倉庫中存放超時會自動腐壞；
//! 礦石/木材/乙太等耐久品不腐——那些是存款，不是牛排。
//!
//! **非懲罰不上線**：計時器只在玩家連線（在 `app.players` 中）時才遞減；
//! 斷線後計時暫停，重啟歸零（記憶體模式，零 migration）。
//! 玩家永遠有機會在倒計時完成前賣出或烹飪。

use std::collections::BTreeMap;

use crate::inventory::{Inventory, ItemKind};
use crate::warehouse::Warehouse;

// ── 常數 ────────────────────────────────────────────────────────────────────

/// 剩餘秒數低於此值時向玩家發出預警（5 分鐘）。
pub const WARN_SECS: f32 = 300.0;

// ── 公開函式 ─────────────────────────────────────────────────────────────────

/// 取得物品的基礎腐壞秒數（線上時間）；耐久品回 `None`。
pub fn decay_secs(item: ItemKind) -> Option<f32> {
    match item {
        // 生鮮魚類（最易腐）：30 分鐘
        ItemKind::FishSmall | ItemKind::FishStar | ItemKind::FishDeep => Some(1800.0),
        // 雞蛋：30 分鐘
        ItemKind::Egg => Some(1800.0),
        // 生鮮作物：40 分鐘
        ItemKind::WheatGrain | ItemKind::Carrot | ItemKind::Potato => Some(2400.0),
        // 熟食料理（已加工，略耐放）：60 分鐘
        ItemKind::GrilledFish
        | ItemKind::StarSashimi
        | ItemKind::DeepBroth
        | ItemKind::FriedEgg
        | ItemKind::Bread
        | ItemKind::CarrotSoup
        | ItemKind::PotatoGratin => Some(3600.0),
        // 其他（礦石、木材、乙太、武器、藥水、碎片…）：耐久品，永不腐
        _ => None,
    }
}

/// 物品是否為易腐品。
pub fn is_perishable(item: ItemKind) -> bool {
    decay_secs(item).is_some()
}

/// 易腐品的中文顯示名（供聊天訊息用）。
pub fn item_display_zh(item: ItemKind) -> &'static str {
    match item {
        ItemKind::FishSmall   => "小魚",
        ItemKind::FishStar    => "星星魚",
        ItemKind::FishDeep    => "深海魚",
        ItemKind::Egg         => "雞蛋",
        ItemKind::WheatGrain  => "小麥穗",
        ItemKind::Carrot      => "胡蘿蔔",
        ItemKind::Potato      => "馬鈴薯",
        ItemKind::GrilledFish => "烤魚",
        ItemKind::StarSashimi => "星燦刺身",
        ItemKind::DeepBroth   => "深海濃湯",
        ItemKind::FriedEgg    => "煎蛋",
        ItemKind::Bread       => "麵包",
        ItemKind::CarrotSoup  => "蔬菜湯",
        ItemKind::PotatoGratin => "焗烤馬鈴薯",
        // 呼叫端應先 is_perishable 過濾，這裡只是防呆
        _ => "食物",
    }
}

// ── 易腐品枚舉陣列 ────────────────────────────────────────────────────────────

/// 全部 14 種易腐物品（供 tick 內部遍歷）。
const PERISHABLE_ITEMS: [ItemKind; 14] = [
    ItemKind::FishSmall,
    ItemKind::FishStar,
    ItemKind::FishDeep,
    ItemKind::Egg,
    ItemKind::WheatGrain,
    ItemKind::Carrot,
    ItemKind::Potato,
    ItemKind::GrilledFish,
    ItemKind::StarSashimi,
    ItemKind::DeepBroth,
    ItemKind::FriedEgg,
    ItemKind::Bread,
    ItemKind::CarrotSoup,
    ItemKind::PotatoGratin,
];

// ── 事件 ──────────────────────────────────────────────────────────────────────

/// 腐壞事件：由 [`PerishableDecayState::tick`] 回傳給呼叫端處理。
#[derive(Debug, Clone, PartialEq)]
pub enum DecayEvent {
    /// 物品已腐壞（呼叫端應從背包/倉庫移除）。
    Spoiled(ItemKind),
    /// 物品快腐壞了（剩餘秒數 ≤ WARN_SECS；每個物品只觸發一次）。
    Warning { item: ItemKind, remaining_secs: u32 },
}

// ── 主結構 ────────────────────────────────────────────────────────────────────

/// 單一玩家的易腐品倒數計時狀態。
///
/// 存在 `Player` 結構體上（記憶體模式）；玩家連線時跑、離線時暫停。
#[derive(Debug, Clone, Default)]
pub struct PerishableDecayState {
    /// item → 剩餘秒數（> 0.0）。
    timers: BTreeMap<ItemKind, f32>,
    /// item → 是否已發過預警（避免重複提示）。
    warned: BTreeMap<ItemKind, bool>,
}

impl PerishableDecayState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 推進倒數（`dt` 為此 tick 的秒數），回傳本 tick 產生的事件清單。
    ///
    /// 此函式自動：
    /// 1. 為有物品但無計時器的易腐品啟動新計時器。
    /// 2. 移除已清空物品的計時器（物品賣掉/用掉後不再追蹤）。
    /// 3. 遞減剩餘秒數，返回 `Spoiled`（到期）或 `Warning`（5 分鐘預警）事件。
    ///
    /// **不**直接修改 `inventory` / `warehouse`；由呼叫端處理 `Spoiled` 事件後移除物品。
    pub fn tick(
        &mut self,
        dt: f32,
        inventory: &Inventory,
        warehouse: &Warehouse,
    ) -> Vec<DecayEvent> {
        // 1. 為新進貨的易腐品啟動計時器
        for &item in &PERISHABLE_ITEMS {
            let count = inventory.count(item) + warehouse.count(item);
            if count > 0 && !self.timers.contains_key(&item) {
                if let Some(secs) = decay_secs(item) {
                    self.timers.insert(item, secs);
                    self.warned.insert(item, false);
                }
            }
        }

        // 2. 移除已清空物品的計時器（物品全部被用掉/賣掉）
        let timers = &mut self.timers;
        let warned = &mut self.warned;
        timers.retain(|&item, _| inventory.count(item) + warehouse.count(item) > 0);
        warned.retain(|item, _| timers.contains_key(item));

        // 3. 推進計時器，收集事件
        let mut events = Vec::new();
        let mut expired = Vec::new();

        for (&item, remaining) in self.timers.iter_mut() {
            let was_above_warn = *remaining > WARN_SECS;
            *remaining -= dt;

            if *remaining <= 0.0 {
                expired.push(item);
                events.push(DecayEvent::Spoiled(item));
            } else if was_above_warn && *remaining <= WARN_SECS {
                let w = self.warned.entry(item).or_insert(false);
                if !*w {
                    *w = true;
                    events.push(DecayEvent::Warning {
                        item,
                        remaining_secs: *remaining as u32,
                    });
                }
            }
        }

        for item in expired {
            self.timers.remove(&item);
            self.warned.remove(&item);
        }

        events
    }

    /// 迭代所有正在計時的易腐品與其剩餘秒數（供 `PlayerView` 廣播給前端）。
    pub fn all_timers(&self) -> impl Iterator<Item = (ItemKind, u32)> + '_ {
        self.timers.iter().map(|(&k, &v)| (k, v as u32))
    }

    /// 取得指定物品的剩餘秒數（0 = 已腐壞/不追蹤）。
    pub fn remaining_secs(&self, item: ItemKind) -> u32 {
        self.timers.get(&item).copied().unwrap_or(0.0) as u32
    }
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warehouse::Warehouse;

    fn inv_with(item: ItemKind, qty: u32) -> Inventory {
        let mut inv = Inventory::new();
        inv.add(item, qty);
        inv
    }

    #[test]
    fn decay_secs_fish_are_perishable() {
        assert!(decay_secs(ItemKind::FishSmall).is_some());
        assert!(decay_secs(ItemKind::FishStar).is_some());
        assert!(decay_secs(ItemKind::FishDeep).is_some());
        assert!(is_perishable(ItemKind::FishSmall));
    }

    #[test]
    fn decay_secs_ores_are_not_perishable() {
        assert!(decay_secs(ItemKind::Wood).is_none());
        assert!(decay_secs(ItemKind::Stone).is_none());
        assert!(decay_secs(ItemKind::Ether).is_none());
        assert!(decay_secs(ItemKind::CrystalShard).is_none());
        assert!(!is_perishable(ItemKind::Wood));
    }

    #[test]
    fn decay_secs_potions_not_perishable() {
        // 藥水/精粹是魔法濃縮品，不腐
        assert!(!is_perishable(ItemKind::HealingPotion));
        assert!(!is_perishable(ItemKind::JadeElixir));
        assert!(!is_perishable(ItemKind::NightPotion));
    }

    #[test]
    fn perishable_items_count_is_fourteen() {
        assert_eq!(PERISHABLE_ITEMS.len(), 14);
        // 每種都是真的易腐品
        for &item in &PERISHABLE_ITEMS {
            assert!(is_perishable(item), "{item:?} 應該是易腐品");
        }
    }

    #[test]
    fn timer_starts_when_item_acquired() {
        let mut state = PerishableDecayState::new();
        let inv = inv_with(ItemKind::FishSmall, 2);
        let wh = Warehouse::default();
        let events = state.tick(0.1, &inv, &wh);
        assert!(events.is_empty(), "剛取得時不應有事件");
        // 計時器應該已啟動
        assert!(state.remaining_secs(ItemKind::FishSmall) > 0);
    }

    #[test]
    fn timer_clears_when_item_used() {
        let mut state = PerishableDecayState::new();
        let mut inv = inv_with(ItemKind::FishSmall, 1);
        let wh = Warehouse::default();
        state.tick(0.1, &inv, &wh); // 啟動計時器
        // 使用/賣出後物品清空
        inv.take(ItemKind::FishSmall, 1);
        state.tick(0.1, &inv, &wh); // 應清除計時器
        assert_eq!(state.remaining_secs(ItemKind::FishSmall), 0);
    }

    #[test]
    fn item_spoils_when_timer_expires() {
        let mut state = PerishableDecayState::new();
        let inv = inv_with(ItemKind::FishSmall, 3);
        let wh = Warehouse::default();
        // 先啟動計時器
        state.tick(0.1, &inv, &wh);
        // 強制倒數到 0（使用大 dt 跳過）
        let events = state.tick(99999.0, &inv, &wh);
        assert!(
            events.iter().any(|e| matches!(e, DecayEvent::Spoiled(ItemKind::FishSmall))),
            "應產生 Spoiled 事件"
        );
        // 腐壞後計時器清除
        assert_eq!(state.remaining_secs(ItemKind::FishSmall), 0);
    }

    #[test]
    fn warning_fires_once_near_expiry() {
        let mut state = PerishableDecayState::new();
        let inv = inv_with(ItemKind::Egg, 1);
        let wh = Warehouse::default();
        state.tick(0.1, &inv, &wh); // 啟動
        // 快轉到快腐壞（剩 WARN_SECS - 1）
        let to_warn = decay_secs(ItemKind::Egg).unwrap() - WARN_SECS + 1.0;
        let events = state.tick(to_warn, &inv, &wh);
        assert!(
            events.iter().any(|e| matches!(e, DecayEvent::Warning { item: ItemKind::Egg, .. })),
            "應產生 Warning 事件"
        );
        // 再 tick 一次，不應再發警告
        let events2 = state.tick(1.0, &inv, &wh);
        assert!(
            !events2.iter().any(|e| matches!(e, DecayEvent::Warning { item: ItemKind::Egg, .. })),
            "警告只應發一次"
        );
    }

    #[test]
    fn no_events_for_non_perishable_items() {
        let mut state = PerishableDecayState::new();
        let inv = inv_with(ItemKind::Wood, 10);
        let wh = Warehouse::default();
        let events = state.tick(99999.0, &inv, &wh);
        assert!(events.is_empty(), "耐久品不應產生任何腐壞事件");
        assert_eq!(state.remaining_secs(ItemKind::Wood), 0);
    }

    #[test]
    fn fish_spoil_before_cooked_food() {
        // 生魚腐壞時間短於熟食
        let fish = decay_secs(ItemKind::FishSmall).unwrap();
        let cooked = decay_secs(ItemKind::GrilledFish).unwrap();
        assert!(fish < cooked, "生魚應比熟食更快腐壞");
    }

    #[test]
    fn all_timers_returns_tracked_items() {
        let mut state = PerishableDecayState::new();
        let mut inv = Inventory::new();
        inv.add(ItemKind::FishSmall, 1);
        inv.add(ItemKind::Egg, 2);
        let wh = Warehouse::default();
        state.tick(0.1, &inv, &wh);
        let tracked: Vec<_> = state.all_timers().collect();
        assert_eq!(tracked.len(), 2, "應追蹤 2 種易腐品");
    }

    #[test]
    fn item_in_warehouse_also_tracked() {
        let mut state = PerishableDecayState::new();
        let inv = Inventory::new();
        let mut wh = Warehouse::default();
        wh.buy_expansion();
        wh.add(ItemKind::Carrot, 5);
        let events = state.tick(99999.0, &inv, &wh);
        assert!(
            events.iter().any(|e| matches!(e, DecayEvent::Spoiled(ItemKind::Carrot))),
            "倉庫裡的易腐品也應腐壞"
        );
    }

    #[test]
    fn cooked_food_lasts_longer_than_raw_crops() {
        let raw = decay_secs(ItemKind::Potato).unwrap();
        let cooked = decay_secs(ItemKind::PotatoGratin).unwrap();
        assert!(cooked > raw, "焗烤馬鈴薯應比生馬鈴薯耐放");
    }
}
