//! 合成配方（Phase 1-C 純邏輯地基）。
//!
//! 玩法鏈缺的那一環：採集／打怪／農地三個來源都在灌背包，木／石／廢料只進不出。
//! 合成給這些素材一個「去處」——查配方、扣素材、產出工具，是 GDD 紀律「乙太有產出
//! 也要有去處」往素材延伸的第一步。
//!
//! 這層純資料 + 純函式，無 IO、不碰 WebSocket／遊戲迴圈，便於自動測試：
//!   - `RECIPES`：靜態配方表（輸入素材 → 產出物品），單一真實來源。
//!   - `Recipe::can_craft(&inv)`：背包夠不夠料、產物放不放得下（UI 反灰用）。
//!   - `Recipe::craft(&mut inv)`：**全有全無**——夠才一次扣全部素材、加產物；任一條件
//!     不滿足回 `false` 且完全不動背包（不會扣到一半卻拿不到產物）。
//!   - `recipe_by_id(id)`：接線時 client 送 `Craft{ recipe: "pickaxe" }`，伺服器查表。
//!
//! additive、不動廣播 shape：背包已隨快照廣播（見 `protocol::InventoryView`），合成只是
//! 多扣／多加背包內容，前端只需多一個合成面板，零契約變更。延續 `inventory.rs` /
//! `gather.rs` / `field.rs` 的前置慣例：純邏輯先落地、標 `allow(dead_code)`，接線輪
//! （ws 收 `Craft` → `recipe_by_id` → `craft` → 背包走既有快照）才有呼叫端。

use crate::inventory::{Inventory, ItemKind, MAX_STACK};

/// 一條合成配方：吃 `inputs` 列出的素材，產出 `output_qty` 個 `output`。
///
/// `id` 是給前端／網路用的穩定字串（snake_case，對齊 `ItemKind` 的序列化命名）：
/// client 送 `Craft{ recipe: id }`，伺服器以 `recipe_by_id` 查回配方，避免讓客戶端
/// 直接送一整份配方內容（素材／產量一律由伺服器這份表說了算，client 只送意圖）。
pub struct Recipe {
    /// 穩定字串 id（網路／前端用），全表唯一。
    pub id: &'static str,
    /// 合成出的物品。
    pub output: ItemKind,
    /// 一次合成產出的數量。
    pub output_qty: u32,
    /// 需要消耗的素材 `(物品, 數量)`。同一物品在一條配方裡只出現一次
    /// （見 `recipe_table_is_well_formed` 測試把關），故 `can_craft` 的逐項檢查無須疊加。
    pub inputs: &'static [(ItemKind, u32)],
}

/// 全部配方表（單一真實來源）。薄切片先只有鎬子一條：木×3 + 石×2 → 鎬子×1，
/// 把採集／打怪堆起來的木石導向第一件工具。日後加配方只要往這個陣列加一筆。
pub const RECIPES: &[Recipe] = &[Recipe {
    id: "pickaxe",
    output: ItemKind::Pickaxe,
    output_qty: 1,
    inputs: &[(ItemKind::Wood, 3), (ItemKind::Stone, 2)],
}];

// 整個模組是前置地基：接線輪（ws 收 `Craft` → 查表 → `craft` → 背包隨快照廣播）才有
// 呼叫端，在此之前公開項目皆無外部呼叫，比照 `inventory.rs` / `gather.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
impl Recipe {
    /// 此刻能否合成：每種素材都夠，**且**產物加得進背包（不會撞 `MAX_STACK`）。
    /// 把「產物放得下」一併納入，是為了讓 `craft` 的全有全無語意成立——否則素材被扣、
    /// 產物卻被堆疊上限夾掉，玩家平白損失素材。
    pub fn can_craft(&self, inv: &Inventory) -> bool {
        let inputs_ok = self.inputs.iter().all(|&(item, qty)| inv.has(item, qty));
        // 產物若正好是某個素材（理論上的自反配方），這裡用「扣掉素材後的餘量」會更精確；
        // 但現有配方產物（工具）與素材（資源）不相交，故用當前數量檢查即可、且更保守。
        let output_fits = inv.count(self.output).saturating_add(self.output_qty) <= MAX_STACK;
        inputs_ok && output_fits
    }

    /// 嘗試合成：**全有全無**。`can_craft` 通過才動手——逐項扣素材（已驗夠、必成功）、
    /// 加產物，回 `true`；否則完全不動背包、回 `false`。
    pub fn craft(&self, inv: &mut Inventory) -> bool {
        if !self.can_craft(inv) {
            return false;
        }
        for &(item, qty) in self.inputs {
            // `can_craft` 已確保每項都夠且素材互不重複，這裡的 `take` 必定成功；
            // debug 下加斷言，防日後改動讓不變式悄悄破裂。
            let took = inv.take(item, qty);
            debug_assert!(took, "can_craft 通過後 take 不該失敗");
        }
        inv.add(self.output, self.output_qty);
        true
    }
}

/// 依字串 id 查配方（接線時伺服器收到 client 的 `Craft` 意圖後用）。未知 id 回 `None`。
#[allow(dead_code)]
pub fn recipe_by_id(id: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::NodeKind;

    /// 把背包灌到剛好夠合成鎬子的素材（木 3 石 2），供多個測試共用。
    fn stocked() -> Inventory {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        inv.add(ItemKind::Stone, 2);
        inv
    }

    fn pickaxe() -> &'static Recipe {
        recipe_by_id("pickaxe").expect("鎬子配方應存在")
    }

    #[test]
    fn recipe_by_id_finds_known_and_rejects_unknown() {
        assert!(recipe_by_id("pickaxe").is_some());
        assert!(recipe_by_id("nonexistent").is_none());
        assert!(recipe_by_id("").is_none());
    }

    #[test]
    fn craft_consumes_inputs_and_yields_output() {
        let mut inv = stocked();
        let r = pickaxe();
        assert!(r.can_craft(&inv));
        assert!(r.craft(&mut inv));
        // 素材扣光、得一把鎬子。
        assert_eq!(inv.count(ItemKind::Wood), 0);
        assert_eq!(inv.count(ItemKind::Stone), 0);
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn craft_keeps_surplus_materials() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 5);
        inv.add(ItemKind::Stone, 3);
        assert!(pickaxe().craft(&mut inv));
        // 只扣掉配方所需，多的留著。
        assert_eq!(inv.count(ItemKind::Wood), 2);
        assert_eq!(inv.count(ItemKind::Stone), 1);
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn craft_fails_and_is_unchanged_when_missing_a_material() {
        // 有木沒石：can_craft 為否、craft 不動背包（驗原子性——木不該被扣掉）。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        let r = pickaxe();
        assert!(!r.can_craft(&inv));
        assert!(!r.craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Wood), 3);
        assert_eq!(inv.count(ItemKind::Stone), 0);
        assert_eq!(inv.count(ItemKind::Pickaxe), 0);
    }

    #[test]
    fn craft_fails_when_partially_short() {
        // 木夠石差一個：仍是全有全無，整筆失敗、木原封不動。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        inv.add(ItemKind::Stone, 1);
        assert!(!pickaxe().craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Wood), 3);
        assert_eq!(inv.count(ItemKind::Stone), 1);
    }

    #[test]
    fn craft_fails_and_keeps_materials_when_output_would_overflow() {
        // 產物已堆到上限：素材雖夠也不該合（否則扣了料、產物被 MAX_STACK 夾掉而平白損失）。
        let mut full = std::collections::BTreeMap::new();
        full.insert(ItemKind::Wood, 3);
        full.insert(ItemKind::Stone, 2);
        full.insert(ItemKind::Pickaxe, MAX_STACK);
        let mut inv = Inventory::from_raw(full);
        let r = pickaxe();
        assert!(!r.can_craft(&inv));
        assert!(!r.craft(&mut inv));
        // 全有全無：素材一個沒少。
        assert_eq!(inv.count(ItemKind::Wood), 3);
        assert_eq!(inv.count(ItemKind::Stone), 2);
        assert_eq!(inv.count(ItemKind::Pickaxe), MAX_STACK);
    }

    #[test]
    fn gathered_materials_flow_into_crafting() {
        // 端到端模擬玩法鏈：採集產出灌進背包 → 合成。鎖住「採集→背包→合成」同一套物品槽。
        let mut inv = Inventory::new();
        for _ in 0..3 {
            inv.add(NodeKind::Tree.into(), 1); // 採樹得木
        }
        inv.add(NodeKind::Rock.into(), 1); // 採石得石
        inv.add(NodeKind::Rock.into(), 1);
        assert!(pickaxe().craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn recipe_table_is_well_formed() {
        // 配方表健全性（與調校數值無關的不變式，防日後加配方時打錯）：
        let mut seen_ids = std::collections::BTreeSet::new();
        for r in RECIPES {
            // id 唯一。
            assert!(seen_ids.insert(r.id), "配方 id 重複：{}", r.id);
            // 產量為正。
            assert!(r.output_qty > 0, "{} 產量應 > 0", r.id);
            // 至少要有一項素材、每項數量為正、同一物品不重複。
            assert!(!r.inputs.is_empty(), "{} 應至少需要一項素材", r.id);
            let mut seen_items = std::collections::BTreeSet::new();
            for &(item, qty) in r.inputs {
                assert!(qty > 0, "{} 的素材數量應 > 0", r.id);
                assert!(seen_items.insert(item), "{} 的素材 {:?} 重複", r.id, item);
            }
        }
    }
}
