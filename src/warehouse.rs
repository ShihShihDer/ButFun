//! 倉庫系統（ROADMAP 105 倉儲上限 + 倉庫）。
//!
//! ## 設計
//! - **背包上限**：玩家身上最多同時持有 `MAX_INVENTORY_ITEM_KINDS` 種物品；
//!   每種可堆疊到 `inventory::MAX_STACK`，限的是「種類數」而非「總量」。
//! - **倉庫**：花乙太購買擴充（每次 `WAREHOUSE_EXPANSION_COST`，一次增加
//!   `WAREHOUSE_SLOTS_PER_EXPANSION` 種），最多 `MAX_WAREHOUSE_EXPANSIONS` 次。
//! - **溢出規則**：`add_item_overflow()` 先嘗試加入背包，背包滿時自動轉存倉庫；
//!   倉庫也滿時才丟棄（實際上 58 種物品的宇宙不太可能真的全塞滿）。

use serde::{Deserialize, Serialize};

use crate::inventory::{Inventory, ItemKind, MAX_STACK};

// ── 常數 ────────────────────────────────────────────────────────────────────

/// 背包最多持有的物品「種類數」。
pub const MAX_INVENTORY_ITEM_KINDS: usize = 20;

/// 每次購買倉庫擴充增加的物品種類槽數。
pub const WAREHOUSE_SLOTS_PER_EXPANSION: usize = 20;

/// 每次購買倉庫擴充的費用（乙太）。
pub const WAREHOUSE_EXPANSION_COST: u32 = 50;

/// 倉庫最多可購買的擴充次數（所以最多 60 種倉庫槽 = 80 種合計，超過全遊戲 58 種）。
pub const MAX_WAREHOUSE_EXPANSIONS: u8 = 3;

// ── 主結構 ──────────────────────────────────────────────────────────────────

/// 玩家的個人倉庫：已購擴充次數 + 倉庫物品。
/// 記憶體模式：重啟歸零（倉庫是「貴的擴容」，重啟=世界換季；零 migration）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Warehouse {
    /// 已購擴充次數（0 = 未購，倉庫容量 0；每次 +20 種槽）。
    pub expansions: u8,
    /// 倉庫裡的物品（與背包共用 `Inventory` 容器）。
    inventory: Inventory,
}

impl Warehouse {
    /// 倉庫目前最大容量（種類數）。
    pub fn capacity(&self) -> usize {
        self.expansions as usize * WAREHOUSE_SLOTS_PER_EXPANSION
    }

    /// 倉庫目前持有的物品種類數。
    pub fn kind_count(&self) -> usize {
        self.inventory.kind_count()
    }

    /// 倉庫是否已滿（新種類放不下；已有的種類繼續堆疊不算滿）。
    pub fn is_full_for_new_kind(&self, item: ItemKind) -> bool {
        // 已有此種類 → 還能堆疊，不算「種類槽滿」
        if self.inventory.count(item) > 0 {
            return false;
        }
        self.kind_count() >= self.capacity()
    }

    /// 向倉庫加物品（先試背包不行才到這）。
    /// 回傳實際加入量（可能因 MAX_STACK 而少於請求量）。
    /// 若倉庫容量不足以接受新種類，回傳 0。
    pub fn add(&mut self, item: ItemKind, qty: u32) -> u32 {
        if self.is_full_for_new_kind(item) && self.inventory.count(item) == 0 {
            // 容量不足且是新種類 → 拒絕
            return 0;
        }
        self.inventory.add(item, qty)
    }

    /// 從倉庫取出物品（放回背包用）。語意同 `Inventory::take`。
    pub fn take(&mut self, item: ItemKind, qty: u32) -> bool {
        self.inventory.take(item, qty)
    }

    /// 查詢倉庫某物品的數量。
    pub fn count(&self, item: ItemKind) -> u32 {
        self.inventory.count(item)
    }

    /// 列出倉庫所有物品（供協議序列化用）。
    pub fn entries(&self) -> impl Iterator<Item = (ItemKind, u32)> + '_ {
        self.inventory.entries()
    }

    /// 是否還可以購買更多擴充（守上限）。
    pub fn can_buy_expansion(&self) -> bool {
        self.expansions < MAX_WAREHOUSE_EXPANSIONS
    }

    /// 購買一次擴充（不含乙太扣除，由呼叫端先扣）。
    /// 回傳 true = 成功；false = 已達上限。
    pub fn buy_expansion(&mut self) -> bool {
        if !self.can_buy_expansion() {
            return false;
        }
        self.expansions += 1;
        true
    }
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_warehouse_has_zero_capacity() {
        let w = Warehouse::default();
        assert_eq!(w.capacity(), 0);
        assert!(!w.can_buy_expansion() || w.expansions == 0);
    }

    #[test]
    fn buy_expansion_increases_capacity() {
        let mut w = Warehouse::default();
        assert!(w.buy_expansion());
        assert_eq!(w.capacity(), WAREHOUSE_SLOTS_PER_EXPANSION);
        assert!(w.buy_expansion());
        assert_eq!(w.capacity(), WAREHOUSE_SLOTS_PER_EXPANSION * 2);
    }

    #[test]
    fn buy_expansion_caps_at_max() {
        let mut w = Warehouse::default();
        for _ in 0..MAX_WAREHOUSE_EXPANSIONS {
            assert!(w.buy_expansion());
        }
        assert!(!w.buy_expansion());
        assert_eq!(w.capacity(), MAX_WAREHOUSE_EXPANSIONS as usize * WAREHOUSE_SLOTS_PER_EXPANSION);
    }

    #[test]
    fn add_to_zero_capacity_returns_zero() {
        let mut w = Warehouse::default();
        // 倉庫容量 0，任何新種類都應拒絕
        assert_eq!(w.add(ItemKind::Wood, 5), 0);
    }

    #[test]
    fn add_after_expansion_works() {
        let mut w = Warehouse::default();
        w.buy_expansion();
        assert_eq!(w.add(ItemKind::Wood, 5), 5);
        assert_eq!(w.count(ItemKind::Wood), 5);
    }

    #[test]
    fn add_existing_kind_succeeds_even_when_kind_slots_full() {
        let mut w = Warehouse::default();
        w.buy_expansion(); // 20 slots
        // 填滿 20 種
        let all_items = [
            ItemKind::Wood, ItemKind::Stone, ItemKind::Ether, ItemKind::Dirt,
            ItemKind::Pickaxe, ItemKind::ReinforcedPickaxe, ItemKind::Weapon,
            ItemKind::CrystalShard, ItemKind::MushroomSpore, ItemKind::AncientFragment,
            ItemKind::DeepSeaPearl, ItemKind::WildflowerSeed, ItemKind::HealingPotion,
            ItemKind::CrystalPotion, ItemKind::MushroomElixir, ItemKind::EtherPill,
            ItemKind::PearlPotion, ItemKind::CrystalBlade, ItemKind::CoralLance,
            ItemKind::MeadowAmulet,
        ];
        for &item in &all_items {
            w.add(item, 1);
        }
        assert_eq!(w.kind_count(), 20);
        // 已有的種類繼續堆疊 → 成功
        assert_eq!(w.add(ItemKind::Wood, 3), 3);
        // 新種類 → 拒絕（槽滿）
        assert_eq!(w.add(ItemKind::StarChart, 1), 0);
    }

    #[test]
    fn take_from_warehouse() {
        let mut w = Warehouse::default();
        w.buy_expansion();
        w.add(ItemKind::Stone, 10);
        assert!(w.take(ItemKind::Stone, 4));
        assert_eq!(w.count(ItemKind::Stone), 6);
        assert!(!w.take(ItemKind::Stone, 100));
    }

    #[test]
    fn warehouse_constants_are_reasonable() {
        // 最多擴充後合計容量 > 全遊戲物品種數（58 種），確保不會永久卡住
        let max_total = MAX_INVENTORY_ITEM_KINDS
            + MAX_WAREHOUSE_EXPANSIONS as usize * WAREHOUSE_SLOTS_PER_EXPANSION;
        assert!(max_total >= 58, "最大倉儲容量應覆蓋全部 58 種物品");
        assert!(WAREHOUSE_EXPANSION_COST > 0);
        assert!(MAX_WAREHOUSE_EXPANSIONS >= 1);
    }

    #[test]
    fn can_buy_expansion_reflects_limit() {
        let mut w = Warehouse::default();
        for _ in 0..MAX_WAREHOUSE_EXPANSIONS {
            assert!(w.can_buy_expansion());
            w.buy_expansion();
        }
        assert!(!w.can_buy_expansion());
    }
}
