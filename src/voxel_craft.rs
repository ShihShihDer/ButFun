//! 乙太方界·合成台 v1——採集的材料→合成新型方塊（ROADMAP 658）。
//!
//! **玩家有感**：挖了木頭可合成更工整的木板、石頭→石磚、沙→玻璃——
//! 「採集→合成→建造」循環第一次有了「精煉」這一環，世界蓋出來更好看。
//!
//! **純邏輯層**：`Recipe` 表 + `find_recipe` + `can_craft`，確定性、無副作用、全可測。
//! 鎖 / WS / IO 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。
//!
//! **成本鐵律**：零 LLM、零 migration、零新協議欄位（WS "craft" 訊息 additive）。

use crate::voxel_inventory::InvStore;

/// 一條合成配方（靜態常數，不含任何鎖或 IO 呼叫）。
pub struct Recipe {
    /// 配方識別碼（英文、穩定，WS 訊息用）。
    pub id: &'static str,
    /// 面向玩家的繁中名稱（i18n 集中管理用）。
    pub name_zh: &'static str,
    /// 配料列表：`(block_id, count)`。block_id 對齊後端 `Block as u8`。
    pub inputs: &'static [(u8, u32)],
    /// 產出方塊 id（合成台 v1 新方塊：Plank=8 / StoneBrick=9 / Glass=10）。
    pub output_block: u8,
    /// 一次合成的產出數量。
    pub output_count: u32,
}

/// 所有合成配方（稀少、有感、易驗證）。
///
/// 材料 id 常數（對齊 `Block` enum）：
///   Wood=5, Stone=3, Sand=4 → Plank=8, StoneBrick=9, Glass=10
pub const RECIPES: &[Recipe] = &[
    Recipe {
        id: "plank",
        name_zh: "木板",
        inputs: &[(5, 2)],   // 2 木 → 4 木板
        output_block: 8,
        output_count: 4,
    },
    Recipe {
        id: "stone_brick",
        name_zh: "石磚",
        inputs: &[(3, 2)],   // 2 石 → 2 石磚
        output_block: 9,
        output_count: 2,
    },
    Recipe {
        id: "glass",
        name_zh: "玻璃",
        inputs: &[(4, 2)],   // 2 沙 → 1 玻璃
        output_block: 10,
        output_count: 1,
    },
];

/// 依 id 找配方（找不到回 None）。
pub fn find_recipe(id: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.id == id)
}

/// 玩家是否有足夠材料合成指定配方（純讀、不改狀態、可在鎖外呼叫）。
pub fn can_craft(recipe: &Recipe, store: &InvStore, player: &str) -> bool {
    recipe.inputs.iter().all(|&(block_id, count)| store.count(player, block_id) >= count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_recipe_returns_correct_recipes() {
        let r = find_recipe("plank").unwrap();
        assert_eq!(r.id, "plank");
        assert_eq!(r.output_block, 8, "木板 id 應為 8（Plank）");
        assert_eq!(r.output_count, 4);
        assert!(find_recipe("stone_brick").is_some());
        assert!(find_recipe("glass").is_some());
    }

    #[test]
    fn find_recipe_returns_none_for_unknown() {
        assert!(find_recipe("unknown_xyz").is_none());
        assert!(find_recipe("").is_none());
        // 大小寫不符也回 None（id 是嚴格精確配對）。
        assert!(find_recipe("PLANK").is_none());
    }

    #[test]
    fn all_recipes_have_nonempty_inputs() {
        for r in RECIPES {
            assert!(!r.inputs.is_empty(), "配方「{}」應有配料", r.id);
            // 配料數量應 > 0
            for &(_, cnt) in r.inputs {
                assert!(cnt > 0, "配方「{}」配料數量應 > 0", r.id);
            }
        }
    }

    #[test]
    fn all_recipes_output_crafted_block_ids() {
        // 合成品 id 應落在合成台 v1 範圍（Plank=8 / StoneBrick=9 / Glass=10）。
        for r in RECIPES {
            assert!(
                r.output_block >= 8 && r.output_block <= 10,
                "配方「{}」產出 id={} 應在 8~10 之間",
                r.id, r.output_block
            );
            assert!(r.output_count > 0, "配方「{}」產出數量應 > 0", r.id);
        }
    }

    #[test]
    fn can_craft_with_sufficient_materials() {
        let mut store = InvStore::default();
        store.give("旅人", 5, 4); // 4 木，需 2
        let r = find_recipe("plank").unwrap();
        assert!(can_craft(r, &store, "旅人"), "4 木應能合成木板");
    }

    #[test]
    fn can_craft_with_exact_materials() {
        let mut store = InvStore::default();
        store.give("旅人", 5, 2); // 剛好 2 木
        let r = find_recipe("plank").unwrap();
        assert!(can_craft(r, &store, "旅人"), "剛好 2 木應能合成");
    }

    #[test]
    fn cannot_craft_with_insufficient_materials() {
        let mut store = InvStore::default();
        store.give("旅人", 5, 1); // 只有 1 木，需 2
        let r = find_recipe("plank").unwrap();
        assert!(!can_craft(r, &store, "旅人"), "只有 1 木不夠合成木板");
    }

    #[test]
    fn cannot_craft_with_zero_materials() {
        let store = InvStore::default();
        for r in RECIPES {
            assert!(!can_craft(r, &store, "旅人"), "零材料不能合成「{}」", r.id);
        }
    }

    #[test]
    fn can_craft_each_recipe_with_enough_materials() {
        let mut store = InvStore::default();
        store.give("旅人", 5, 10); // Wood (plank 用)
        store.give("旅人", 3, 10); // Stone (stone_brick 用)
        store.give("旅人", 4, 10); // Sand (glass 用)
        for r in RECIPES {
            assert!(can_craft(r, &store, "旅人"), "配方「{}」材料足夠應可合成", r.id);
        }
    }

    #[test]
    fn can_craft_different_players_independent() {
        let mut store = InvStore::default();
        store.give("玩家A", 5, 2); // A 有木
        let r = find_recipe("plank").unwrap();
        // A 可以合成，B 不行（零材料）。
        assert!(can_craft(r, &store, "玩家A"));
        assert!(!can_craft(r, &store, "玩家B"));
    }
}
