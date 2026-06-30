//! 乙太方界·合成台 v1 + 工作台 3×3 v1（ROADMAP 658 / 665）。
//!
//! **玩家有感**：挖了木頭可合成更工整的木板（2×2），合出工作台後放置到世界→
//! 右鍵互動開 3×3 格→合成大量物品或混合配方——「採集→合成→建造」循環更深。
//!
//! **純邏輯層**：`Recipe` 表 + `find_recipe` + `find_workbench_recipe` + `find_any_recipe` +
//! `can_craft`，確定性、無副作用、全可測。
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

/// 背包 2×2 合成配方（稀少、有感、易驗證）。
///
/// 材料 id 常數（對齊 `Block` enum）：
///   Wood=5, Stone=3, Sand=4 → Plank=8, StoneBrick=9, Glass=10
///   Dirt=2 → FarmSoil=11（種田 v1）
///   Plank=8 → Workbench=15（工作台 v1，ROADMAP 665）
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
    Recipe {
        id: "till",
        name_zh: "農田土",
        inputs: &[(2, 2)],   // 2 泥土 → 2 農田土（種田 v1）
        output_block: 11,
        output_count: 2,
    },
    Recipe {
        id: "workbench",
        name_zh: "工作台",
        inputs: &[(8, 4)],   // 4 木板 → 1 工作台（2×2 剛好放滿四格）
        output_block: 15,
        output_count: 1,
    },
];

/// 工作台 3×3 合成配方（需放置工作台方塊後右鍵開啟面板才能合成）。
///
/// 這些配方需要 5-6 格材料，超出 2×2 上限，或組合多種材料，
/// 象徵「精工合成」需要工作台才能完成。
pub const WORKBENCH_RECIPES: &[Recipe] = &[
    Recipe {
        id: "plank_wb",
        name_zh: "木板（大量）",
        inputs: &[(5, 6)],      // 6 木 → 16 木板（高效批量，需 3×3）
        output_block: 8,
        output_count: 16,
    },
    Recipe {
        id: "stone_brick_wb",
        name_zh: "石磚（大量）",
        inputs: &[(3, 6)],      // 6 石 → 10 石磚（大批建材）
        output_block: 9,
        output_count: 10,
    },
    Recipe {
        id: "glass_wb",
        name_zh: "玻璃（大量）",
        inputs: &[(4, 6)],      // 6 沙 → 8 玻璃（大批玻璃）
        output_block: 10,
        output_count: 8,
    },
    Recipe {
        id: "stone_wood_mix",
        name_zh: "混合石磚",
        inputs: &[(3, 3), (8, 3)],  // 3 石 + 3 木板 → 6 石磚（混合工藝，獨特配方）
        output_block: 9,
        output_count: 6,
    },
    Recipe {
        id: "farm_kit",
        name_zh: "農耕大包",
        inputs: &[(2, 4), (5, 2)],  // 4 泥土 + 2 木 → 8 農田土（農場快速開墾）
        output_block: 11,
        output_count: 8,
    },
];

/// 依 id 找背包配方（2×2，找不到回 None）。
pub fn find_recipe(id: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.id == id)
}

/// 依 id 找工作台配方（3×3，找不到回 None）。
pub fn find_workbench_recipe(id: &str) -> Option<&'static Recipe> {
    WORKBENCH_RECIPES.iter().find(|r| r.id == id)
}

/// 依 id 搜尋兩套配方表，背包優先（找不到回 None）。
/// WS Craft handler 用——前端送 recipe_id，後端統一查這裡。
pub fn find_any_recipe(id: &str) -> Option<&'static Recipe> {
    find_recipe(id).or_else(|| find_workbench_recipe(id))
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
    fn find_recipe_workbench_in_2x2_list() {
        // 工作台配方在 2×2 表——4 木板合成
        let r = find_recipe("workbench").unwrap();
        assert_eq!(r.output_block, 15, "工作台 id 應為 15");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(8, 4)], "工作台需要 4 木板");
    }

    #[test]
    fn find_recipe_returns_none_for_unknown() {
        assert!(find_recipe("unknown_xyz").is_none());
        assert!(find_recipe("").is_none());
        // 大小寫不符也回 None（id 是嚴格精確配對）。
        assert!(find_recipe("PLANK").is_none());
    }

    #[test]
    fn find_workbench_recipe_returns_correct_recipes() {
        let r = find_workbench_recipe("plank_wb").unwrap();
        assert_eq!(r.id, "plank_wb");
        assert_eq!(r.output_block, 8, "plank_wb 應產出木板(8)");
        assert_eq!(r.output_count, 16);
        assert!(find_workbench_recipe("stone_brick_wb").is_some());
        assert!(find_workbench_recipe("glass_wb").is_some());
        assert!(find_workbench_recipe("stone_wood_mix").is_some());
        assert!(find_workbench_recipe("farm_kit").is_some());
    }

    #[test]
    fn find_workbench_recipe_returns_none_for_bag_recipes() {
        // 工作台配方表不包含 2×2 配方
        assert!(find_workbench_recipe("plank").is_none());
        assert!(find_workbench_recipe("glass").is_none());
    }

    #[test]
    fn find_any_recipe_finds_both_lists() {
        // 背包配方
        assert!(find_any_recipe("plank").is_some());
        assert!(find_any_recipe("workbench").is_some());
        // 工作台配方
        assert!(find_any_recipe("plank_wb").is_some());
        assert!(find_any_recipe("stone_wood_mix").is_some());
        // 不存在的
        assert!(find_any_recipe("does_not_exist").is_none());
    }

    #[test]
    fn all_recipes_have_nonempty_inputs() {
        for r in RECIPES.iter().chain(WORKBENCH_RECIPES.iter()) {
            assert!(!r.inputs.is_empty(), "配方「{}」應有配料", r.id);
            for &(_, cnt) in r.inputs {
                assert!(cnt > 0, "配方「{}」配料數量應 > 0", r.id);
            }
        }
    }

    #[test]
    fn all_recipes_output_crafted_block_ids() {
        // 2×2 配方產出 id 應在 8–11 或 15（工作台）
        for r in RECIPES {
            let ok = (r.output_block >= 8 && r.output_block <= 11) || r.output_block == 15;
            assert!(ok, "配方「{}」產出 id={} 應在 8~11 或 15", r.id, r.output_block);
            assert!(r.output_count > 0, "配方「{}」產出數量應 > 0", r.id);
        }
        // 3×3 配方產出 id 應在合成台範圍
        for r in WORKBENCH_RECIPES {
            assert!(
                r.output_block >= 8 && r.output_block <= 15,
                "工作台配方「{}」產出 id={} 超出範圍",
                r.id, r.output_block
            );
            assert!(r.output_count > 0, "工作台配方「{}」產出數量應 > 0", r.id);
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
        for r in RECIPES.iter().chain(WORKBENCH_RECIPES.iter()) {
            assert!(!can_craft(r, &store, "旅人"), "零材料不能合成「{}」", r.id);
        }
    }

    #[test]
    fn can_craft_each_recipe_with_enough_materials() {
        let mut store = InvStore::default();
        store.give("旅人", 5, 10);  // Wood
        store.give("旅人", 3, 10);  // Stone
        store.give("旅人", 4, 10);  // Sand
        store.give("旅人", 2, 10);  // Dirt
        store.give("旅人", 8, 10);  // Plank（工作台 + stone_wood_mix 用）
        for r in RECIPES.iter().chain(WORKBENCH_RECIPES.iter()) {
            assert!(can_craft(r, &store, "旅人"), "配方「{}」材料足夠應可合成", r.id);
        }
    }

    #[test]
    fn can_craft_workbench_recipe_multi_material() {
        let mut store = InvStore::default();
        store.give("旅人", 3, 3); // 3 石
        store.give("旅人", 8, 3); // 3 木板
        let r = find_workbench_recipe("stone_wood_mix").unwrap();
        assert!(can_craft(r, &store, "旅人"), "3 石 + 3 木板應能合成混合石磚");
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

    #[test]
    fn workbench_recipe_requires_more_materials_than_2x2() {
        // 所有工作台配方的總材料格數應 > 4（無法在 2×2 中完成）
        for r in WORKBENCH_RECIPES {
            let total: u32 = r.inputs.iter().map(|&(_, c)| c).sum();
            assert!(total > 4, "工作台配方「{}」總材料 {} 不夠大（應 > 4 格）", r.id, total);
        }
    }
}
