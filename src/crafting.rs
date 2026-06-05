//! 合成配方模型（Phase 1-C 合成台的純邏輯地基）。
//!
//! 這層只管「一份配方要哪些材料、產出什麼、背包夠不夠合、合了怎麼增減」，是純資料 +
//! 純函式，無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試。延續 `inventory.rs` /
//! `gather.rs` / `crops.rs` / `field.rs` 的前置慣例：純邏輯先落地、標 `allow(dead_code)`，
//! 接線輪（在地盤蓋合成台、互動開菜單、選配方扣料給產物、前端顯示配方清單）才有呼叫端。
//!
//! 之後接上：
//!   - 世界 / ws：玩家在地盤蓋一個「合成台」實體，走近互動開菜單。
//!   - ws：選一份配方 → `craft(&mut player.inventory)` 扣材料、加產物，回饋成功 / 缺料。
//!   - 前端：列出 `RECIPES`、依 `can_craft` 反灰不可合的項、顯示缺哪些材料。
//!
//! 配方刻意做成編譯期 `&'static` 常數表（`RECIPES`）：第一份配方（鎬子）寫死在這，
//! 日後加配方只在表裡多一筆即可，合成邏輯本身不用動。所有材料 / 產物都走 `ItemKind`
//! 型別（而非字串 id），拼錯的物品在編譯期就被擋掉。

use crate::inventory::{Inventory, ItemKind, MAX_STACK};

/// 一份配方的單項材料需求。
pub struct Ingredient {
    /// 需要的物品種類。
    pub item: ItemKind,
    /// 需要的數量。
    pub qty: u32,
}

/// 一份合成配方：消耗 `inputs` 列出的材料，產出 `output_qty` 個 `output`。
///
/// 約定：`inputs` 裡每種物品**只列一筆**（不重複同一 `ItemKind`）——`craft` 的全有全無
/// 檢查以每筆獨立判斷，重複列同物會讓檢查失準。目前唯一的配方（鎬子）符合此約定。
pub struct Recipe {
    /// 產物種類。
    pub output: ItemKind,
    /// 一次合成產出的數量。
    pub output_qty: u32,
    /// 所需材料清單。
    pub inputs: &'static [Ingredient],
}

/// 第一份配方：木×3 + 石×2 = 鎬子（Phase 1-C 驗收用）。
// 前置地基：接線輪才有呼叫端，比照本模組其他項標 `allow(dead_code)`。
#[allow(dead_code)]
pub const PICKAXE: Recipe = Recipe {
    output: ItemKind::Pickaxe,
    output_qty: 1,
    inputs: &[
        Ingredient {
            item: ItemKind::Wood,
            qty: 3,
        },
        Ingredient {
            item: ItemKind::Stone,
            qty: 2,
        },
    ],
};

/// 全部可用配方表。接線時前端列這張表、ws 依索引 / 名稱挑一份來合。
#[allow(dead_code)]
pub const RECIPES: &[Recipe] = &[PICKAXE];

// 整個模組是前置地基：接線輪（合成台實體、ws 互動、前端菜單）才有呼叫端，
// 在此之前公開項目皆無外部呼叫，比照 `inventory.rs` / `gather.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
impl Recipe {
    /// 背包是否備齊**所有**材料（不改變背包，供 UI 反灰 / 合成前預檢）。
    /// 注意：只看材料夠不夠，不看產物放不放得下——後者由 `has_output_room` 另判。
    pub fn has_inputs(&self, inv: &Inventory) -> bool {
        self.inputs.iter().all(|ing| inv.has(ing.item, ing.qty))
    }

    /// 背包放不放得下這次的產物（合成後 `output` 不會超過堆疊上限）。
    /// 用來確保「材料不會被吃掉、產物卻因爆倉憑空消失」——維持全有全無。
    pub fn has_output_room(&self, inv: &Inventory) -> bool {
        // 約定 output 不同時是某個 input（工具配方本就如此），故扣料不影響此判斷。
        MAX_STACK - inv.count(self.output) >= self.output_qty
    }

    /// 是否可合：材料齊備**且**背包放得下產物。
    pub fn can_craft(&self, inv: &Inventory) -> bool {
        self.has_inputs(inv) && self.has_output_room(inv)
    }

    /// 合成一次：**材料齊備且放得下產物才合**，扣掉所有材料、加上產物並回 `true`；
    /// 否則完全不動背包回 `false`（合成「材料不足不給合」要的全有全無語意）。
    ///
    /// 先 `can_craft` 一次性確認所有前提，確認後逐項 `take` 必定成功（已預檢 `has`），
    /// 故不會發生「扣了一半材料才發現不夠」的半成品狀態。
    pub fn craft(&self, inv: &mut Inventory) -> bool {
        if !self.can_craft(inv) {
            return false;
        }
        for ing in self.inputs {
            // 已經 has_inputs 預檢過，這裡每筆都扣得動。
            debug_assert!(inv.take(ing.item, ing.qty));
        }
        // has_output_room 預檢過，這裡不會被夾掉而丟失。
        inv.add(self.output, self.output_qty);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 備好剛好能合一支鎬子的材料。
    fn inv_with_pickaxe_materials() -> Inventory {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        inv.add(ItemKind::Stone, 2);
        inv
    }

    #[test]
    fn pickaxe_recipe_matches_backlog_spec() {
        // 驗收：木×3 + 石×2 = 鎬子×1。
        assert_eq!(PICKAXE.output, ItemKind::Pickaxe);
        assert_eq!(PICKAXE.output_qty, 1);
        let wood = PICKAXE.inputs.iter().find(|i| i.item == ItemKind::Wood);
        let stone = PICKAXE.inputs.iter().find(|i| i.item == ItemKind::Stone);
        assert_eq!(wood.map(|i| i.qty), Some(3));
        assert_eq!(stone.map(|i| i.qty), Some(2));
    }

    #[test]
    fn craft_consumes_inputs_and_yields_output() {
        let mut inv = inv_with_pickaxe_materials();
        assert!(PICKAXE.can_craft(&inv));
        assert!(PICKAXE.craft(&mut inv));
        // 材料被吃光、得到一支鎬子。
        assert_eq!(inv.count(ItemKind::Wood), 0);
        assert_eq!(inv.count(ItemKind::Stone), 0);
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn craft_keeps_surplus_materials() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 5);
        inv.add(ItemKind::Stone, 4);
        assert!(PICKAXE.craft(&mut inv));
        // 只扣掉配方所需，多的材料留著。
        assert_eq!(inv.count(ItemKind::Wood), 2);
        assert_eq!(inv.count(ItemKind::Stone), 2);
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn craft_fails_and_is_unchanged_when_short_one_material() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        inv.add(ItemKind::Stone, 1); // 少一個石頭
        assert!(!PICKAXE.has_inputs(&inv));
        assert!(!PICKAXE.can_craft(&inv));
        assert!(!PICKAXE.craft(&mut inv));
        // 失敗不動背包：木頭沒被吃掉、沒生出鎬子。
        assert_eq!(inv.count(ItemKind::Wood), 3);
        assert_eq!(inv.count(ItemKind::Stone), 1);
        assert_eq!(inv.count(ItemKind::Pickaxe), 0);
    }

    #[test]
    fn craft_fails_on_empty_inventory() {
        let mut inv = Inventory::new();
        assert!(!PICKAXE.craft(&mut inv));
        assert!(inv.is_empty());
    }

    #[test]
    fn craft_can_repeat_while_materials_last() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 6);
        inv.add(ItemKind::Stone, 4);
        // 夠合兩支。
        assert!(PICKAXE.craft(&mut inv));
        assert!(PICKAXE.craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Pickaxe), 2);
        // 材料用罄，第三次失敗。
        assert!(!PICKAXE.craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Pickaxe), 2);
    }

    #[test]
    fn craft_blocked_when_output_would_overflow() {
        let mut inv = inv_with_pickaxe_materials();
        // 鎬子已滿到上限，再合一支會爆倉。
        inv.add(ItemKind::Pickaxe, MAX_STACK);
        assert!(PICKAXE.has_inputs(&inv));
        assert!(!PICKAXE.has_output_room(&inv));
        assert!(!PICKAXE.can_craft(&inv));
        assert!(!PICKAXE.craft(&mut inv));
        // 全有全無：材料沒被吃掉。
        assert_eq!(inv.count(ItemKind::Wood), 3);
        assert_eq!(inv.count(ItemKind::Stone), 2);
        assert_eq!(inv.count(ItemKind::Pickaxe), MAX_STACK);
    }

    #[test]
    fn recipes_table_lists_pickaxe() {
        assert!(RECIPES.iter().any(|r| r.output == ItemKind::Pickaxe));
    }
}
