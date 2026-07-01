//! 乙太方界·合成台 v1 + 工作台 3×3 v1 + 熔爐 v1 + 鎬具 v1 + 斧頭 v1 + 鏟子 v1
//! （ROADMAP 658/665/666/687/689/690）。
//!
//! **玩家有感**：挖了木頭可合成更工整的木板（2×2），合出工作台後放置到世界→
//! 右鍵互動開 3×3 格→合成大量物品或混合配方；合出熔爐→放置→右鍵開冶煉面板→
//! 把石頭冶煉成拋光石（獨特建材）——「採集→合成→冶煉→建造」循環更深。
//! 鎬具 v1（ROADMAP 687）：木鎬/石鎬（背包 2×2）＋鐵鎬（工作台 3×3）合成；
//! 持鎬採石/礦時，前端採礦手感大幅加速——療癒循環再加一圈。
//! 斧頭 v1（ROADMAP 689）：木斧/石斧（背包 2×2）＋鐵斧（工作台 3×3）合成；
//! 持斧砍木頭/葉片/木板時前端大幅加速，讓砍樹第一次有「工具感」。
//! 鏟子 v1（ROADMAP 690）：木鏟/石鏟（背包 2×2）＋鐵鏟（工作台 3×3）合成；
//! 持鏟挖草地/泥土/沙地/農田時前端大幅加速，完成採集三件套（鎬+斧+鏟）。
//!
//! **純邏輯層**：`Recipe` 表 + `find_recipe` + `find_workbench_recipe` +
//! `find_furnace_recipe` + `find_any_recipe` + `can_craft`，確定性、無副作用、全可測。
//! 鎖 / WS / IO 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。
//!
//! **成本鐵律**：零 LLM、零 migration、零新協議欄位（WS "craft" 訊息 additive）。

// ── 鎬具物品 ID（純物品，不可放置於世界，與 Block enum 分開）─────────────────────
/// 木鎬（ROADMAP 687）——物品 ID 32；不可放置，合成後住在物品欄。
pub const PICKAXE_WOOD_ID: u8 = 32;
/// 石鎬（ROADMAP 687）——物品 ID 33。
pub const PICKAXE_STONE_ID: u8 = 33;
/// 鐵鎬（ROADMAP 687）——物品 ID 34，需工作台合成。
pub const PICKAXE_IRON_ID: u8 = 34;

// ── 斧頭物品 ID（純物品，不可放置於世界，ROADMAP 689）─────────────────────────────
/// 木斧（ROADMAP 689）——物品 ID 36；砍木頭/葉片/木板加速 2.5×。
pub const AXE_WOOD_ID: u8 = 36;
/// 石斧（ROADMAP 689）——物品 ID 37；砍木材加速 4×。
pub const AXE_STONE_ID: u8 = 37;
/// 鐵斧（ROADMAP 689）——物品 ID 38；砍木材加速 6×，需工作台合成。
pub const AXE_IRON_ID: u8 = 38;

// ── 鏟子物品 ID（純物品，不可放置於世界，ROADMAP 690）─────────────────────────────
/// 木鏟（ROADMAP 690）——物品 ID 39；挖草地/泥土/沙地/農田加速 2.5×。
pub const SHOVEL_WOOD_ID: u8 = 39;
/// 石鏟（ROADMAP 690）——物品 ID 40；挖軟方塊加速 4×。
pub const SHOVEL_STONE_ID: u8 = 40;
/// 鐵鏟（ROADMAP 690）——物品 ID 41；挖軟方塊加速 6×，需工作台合成。
pub const SHOVEL_IRON_ID: u8 = 41;

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
///   Wheat=18 → Bread=19（麵包 v1，ROADMAP 668；18/19 為純物品 id，非方塊 enum）
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
    Recipe {
        id: "bread",
        name_zh: "麵包",
        inputs: &[(18, 3)],  // 3 小麥顆粒 → 1 麵包（療癒農業循環終點；ROADMAP 668）
        output_block: 19,
        output_count: 1,
    },
    Recipe {
        id: "torch",
        name_zh: "火把",
        inputs: &[(5, 1), (20, 1)],  // 1 木頭 + 1 煤礦 → 4 火把（ROADMAP 685）
        output_block: 31,
        output_count: 4,
    },
    // ── 梯子 v1（ROADMAP 688）：3 木板 → 3 梯子（垂直攀爬，深礦上下自如）──────────
    Recipe {
        id: "ladder",
        name_zh: "梯子",
        inputs: &[(8, 3)],              // 3 木板 → 3 梯子（剛好用掉三格 2×2）
        output_block: 35,               // Block::Ladder = 35
        output_count: 3,
    },
    // ── 鎬具 v1（ROADMAP 687）：採石/採礦更快、療癒循環加深 ─────────────────────
    Recipe {
        id: "wood_pickaxe",
        name_zh: "木鎬",
        inputs: &[(5, 3), (8, 1)],   // 3 木頭 + 1 木板 → 1 木鎬（剛好放滿 2×2 四格）
        output_block: PICKAXE_WOOD_ID,
        output_count: 1,
    },
    Recipe {
        id: "stone_pickaxe",
        name_zh: "石鎬",
        inputs: &[(3, 3), (8, 1)],   // 3 石頭 + 1 木板 → 1 石鎬（比木鎬耐用、速度更快）
        output_block: PICKAXE_STONE_ID,
        output_count: 1,
    },
    // ── 斧頭 v1（ROADMAP 689）：砍木加速，和鎬具互補的工具線 ─────────────────────
    Recipe {
        id: "wood_axe",
        name_zh: "木斧",
        inputs: &[(5, 3), (8, 1)],   // 3 木頭 + 1 木板 → 1 木斧（剛好填滿 2×2 四格）
        output_block: AXE_WOOD_ID,
        output_count: 1,
    },
    Recipe {
        id: "stone_axe",
        name_zh: "石斧",
        inputs: &[(3, 3), (8, 1)],   // 3 石頭 + 1 木板 → 1 石斧（4× 砍木速度）
        output_block: AXE_STONE_ID,
        output_count: 1,
    },
    // ── 鏟子 v1（ROADMAP 690）：挖草/土/沙/農田加速，完成採集三件套 ─────────────
    Recipe {
        id: "wood_shovel",
        name_zh: "木鏟",
        inputs: &[(5, 1), (8, 1)],   // 1 木頭 + 1 木板 → 1 木鏟（輕巧配方，2 格填 2×2）
        output_block: SHOVEL_WOOD_ID,
        output_count: 1,
    },
    Recipe {
        id: "stone_shovel",
        name_zh: "石鏟",
        inputs: &[(3, 1), (8, 1)],   // 1 石頭 + 1 木板 → 1 石鏟（4× 挖軟方塊速度）
        output_block: SHOVEL_STONE_ID,
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
    Recipe {
        id: "furnace_wb",
        name_zh: "熔爐",
        inputs: &[(3, 8)],          // 8 石頭 → 1 熔爐（需工作台大格）
        output_block: 16,
        output_count: 1,
    },
    Recipe {
        id: "iron_block",
        name_zh: "鐵磚",
        inputs: &[(22, 6)],         // 6 鐵錠 → 2 鐵磚（ROADMAP 684）壓縮精煉金屬建材
        output_block: 23,
        output_count: 2,
    },
    Recipe {
        id: "iron_pickaxe",
        name_zh: "鐵鎬",
        inputs: &[(22, 3), (8, 2)], // 3 鐵錠 + 2 木板 → 1 鐵鎬（ROADMAP 687；5 格需工作台）
        output_block: PICKAXE_IRON_ID,
        output_count: 1,
    },
    // ── 鐵斧（ROADMAP 689）：5 格需工作台 3×3 ──────────────────────────────────────
    Recipe {
        id: "iron_axe",
        name_zh: "鐵斧",
        inputs: &[(22, 3), (8, 2)], // 3 鐵錠 + 2 木板 → 1 鐵斧（6× 砍木速度）
        output_block: AXE_IRON_ID,
        output_count: 1,
    },
    // ── 鐵鏟（ROADMAP 690）：2 鐵錠 + 3 木板（5 格，需工作台）──────────────────────
    Recipe {
        id: "iron_shovel",
        name_zh: "鐵鏟",
        inputs: &[(22, 2), (8, 3)], // 2 鐵錠 + 3 木板 → 1 鐵鏟（6× 挖軟方塊速度）
        output_block: SHOVEL_IRON_ID,
        output_count: 1,
    },
    // ── 箱子 v1（ROADMAP 692）：8 木板 → 1 箱子（工作台合成，需 8 格）────────────────
    Recipe {
        id: "chest",
        name_zh: "箱子",
        inputs: &[(8, 8)],   // 8 木板 → 1 箱子（工作台合成，放置後可儲存物品）
        output_block: 42,    // Block::Chest = 42
        output_count: 1,
    },
];

/// 熔爐冶煉配方（需放置熔爐方塊後右鍵開啟冶煉面板才能使用）。
///
/// 冶煉概念：把原始方塊「精煉」成獨特材料，或比工作台更高效地產出建材。
/// Block id：Stone=3, Sand=4, Glass=10, SmoothStone=17, CoalOre=20, IronOre=21, IronIngot=22。
pub const FURNACE_RECIPES: &[Recipe] = &[
    Recipe {
        id: "smelt_stone",
        name_zh: "拋光石",
        inputs: &[(3, 3)],          // 3 石頭 → 3 拋光石（唯一能得到拋光石的途徑）
        output_block: 17,
        output_count: 3,
    },
    Recipe {
        id: "smelt_glass",
        name_zh: "玻璃（冶煉）",
        inputs: &[(4, 2)],          // 2 沙 → 3 玻璃（比背包配方 2沙→1玻璃 更高效）
        output_block: 10,
        output_count: 3,
    },
    Recipe {
        id: "smelt_brick",
        name_zh: "石磚（冶煉）",
        inputs: &[(3, 2)],          // 2 石頭 → 4 石磚（比背包配方 2石→2磚 雙倍產量）
        output_block: 9,
        output_count: 4,
    },
    Recipe {
        id: "smelt_iron",
        name_zh: "鐵錠",
        inputs: &[(21, 1), (20, 1)], // 1 鐵礦 + 1 煤礦（煤礦當燃料）→ 2 鐵錠
        output_block: 22,
        output_count: 2,
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

/// 依 id 找熔爐冶煉配方（找不到回 None）。
pub fn find_furnace_recipe(id: &str) -> Option<&'static Recipe> {
    FURNACE_RECIPES.iter().find(|r| r.id == id)
}

/// 依 id 搜尋三套配方表，背包 → 工作台 → 熔爐（找不到回 None）。
/// WS Craft handler 用——前端送 recipe_id，後端統一查這裡。
pub fn find_any_recipe(id: &str) -> Option<&'static Recipe> {
    find_recipe(id)
        .or_else(|| find_workbench_recipe(id))
        .or_else(|| find_furnace_recipe(id))
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
        for r in RECIPES.iter().chain(WORKBENCH_RECIPES.iter()).chain(FURNACE_RECIPES.iter()) {
            assert!(!r.inputs.is_empty(), "配方「{}」應有配料", r.id);
            for &(_, cnt) in r.inputs {
                assert!(cnt > 0, "配方「{}」配料數量應 > 0", r.id);
            }
        }
    }

    #[test]
    fn bread_recipe_exists_and_correct() {
        // 麵包配方：3 小麥(18) → 1 麵包(19)（ROADMAP 668）
        let r = find_recipe("bread").unwrap();
        assert_eq!(r.output_block, 19, "麵包 item id 應為 19");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(18, 3)], "麵包需要 3 小麥顆粒(18)");
    }

    #[test]
    fn all_recipes_output_crafted_block_ids() {
        // 2×2 配方產出 id：8–11、15（工作台）、19（麵包）、31（火把）、
        //   32/33（鎬具）、35（梯子）、36/37（斧頭）、39/40（鏟子 ROADMAP 690）
        for r in RECIPES {
            let ok = (r.output_block >= 8 && r.output_block <= 11)
                || r.output_block == 15
                || r.output_block == 19 // 麵包（純物品 id）
                || r.output_block == 31 // 火把（Torch，ROADMAP 685）
                || r.output_block == PICKAXE_WOOD_ID   // 木鎬（ROADMAP 687）
                || r.output_block == PICKAXE_STONE_ID  // 石鎬（ROADMAP 687）
                || r.output_block == 35 // 梯子（ROADMAP 688）
                || r.output_block == AXE_WOOD_ID   // 木斧（ROADMAP 689）
                || r.output_block == AXE_STONE_ID  // 石斧（ROADMAP 689）
                || r.output_block == SHOVEL_WOOD_ID  // 木鏟（ROADMAP 690）
                || r.output_block == SHOVEL_STONE_ID; // 石鏟（ROADMAP 690）
            assert!(ok, "配方「{}」產出 id={} 超出允許範圍", r.id, r.output_block);
            assert!(r.output_count > 0, "配方「{}」產出數量應 > 0", r.id);
        }
        // 3×3 工作台配方產出 id
        //   （8~17、23 = IronBlock，34 = 鐵鎬、38 = 鐵斧、41 = 鐵鏟 ROADMAP 690、42 = 箱子 ROADMAP 692）
        for r in WORKBENCH_RECIPES {
            let ok = (r.output_block >= 8 && r.output_block <= 17)
                || r.output_block == 23
                || r.output_block == PICKAXE_IRON_ID  // 鐵鎬（ROADMAP 687）
                || r.output_block == AXE_IRON_ID      // 鐵斧（ROADMAP 689）
                || r.output_block == SHOVEL_IRON_ID   // 鐵鏟（ROADMAP 690）
                || r.output_block == 42;               // 箱子（ROADMAP 692）
            assert!(
                ok,
                "工作台配方「{}」產出 id={} 超出範圍",
                r.id, r.output_block
            );
            assert!(r.output_count > 0, "工作台配方「{}」產出數量應 > 0", r.id);
        }
        // 熔爐冶煉配方產出 id（8~17 或 22 = IronIngot）
        for r in FURNACE_RECIPES {
            let ok = (r.output_block >= 8 && r.output_block <= 17) || r.output_block == 22;
            assert!(
                ok,
                "熔爐配方「{}」產出 id={} 超出範圍",
                r.id, r.output_block
            );
            assert!(r.output_count > 0, "熔爐配方「{}」產出數量應 > 0", r.id);
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
        for r in RECIPES.iter().chain(WORKBENCH_RECIPES.iter()).chain(FURNACE_RECIPES.iter()) {
            assert!(!can_craft(r, &store, "旅人"), "零材料不能合成「{}」", r.id);
        }
    }

    #[test]
    fn find_furnace_recipe_finds_all_smelt_recipes() {
        assert!(find_furnace_recipe("smelt_stone").is_some());
        assert!(find_furnace_recipe("smelt_glass").is_some());
        assert!(find_furnace_recipe("smelt_brick").is_some());
        assert!(find_furnace_recipe("unknown").is_none());
        // 熔爐配方不在背包 / 工作台表
        assert!(find_recipe("smelt_stone").is_none());
        assert!(find_workbench_recipe("smelt_stone").is_none());
    }

    #[test]
    fn find_any_recipe_finds_furnace_recipes() {
        assert!(find_any_recipe("smelt_stone").is_some());
        assert!(find_any_recipe("smelt_glass").is_some());
        assert!(find_any_recipe("smelt_brick").is_some());
        // 熔爐工作台配方也在 find_any_recipe 範圍
        assert!(find_any_recipe("furnace_wb").is_some());
    }

    #[test]
    fn smelt_stone_outputs_smooth_stone() {
        let r = find_furnace_recipe("smelt_stone").unwrap();
        assert_eq!(r.output_block, 17, "拋光石 id 應為 17（SmoothStone）");
        assert_eq!(r.output_count, 3);
        assert_eq!(r.inputs, &[(3, 3)], "需 3 石頭");
    }

    #[test]
    fn smelt_glass_better_yield_than_bag() {
        // 熔爐 2沙→3玻璃（1.5:1）> 背包 2沙→1玻璃（0.5:1）
        let furnace = find_furnace_recipe("smelt_glass").unwrap();
        let bag = find_recipe("glass").unwrap();
        assert_eq!(furnace.inputs, &[(4, 2)]);
        assert_eq!(bag.inputs, &[(4, 2)]);
        assert!(furnace.output_count > bag.output_count,
            "熔爐玻璃產量 {} 應 > 背包 {}", furnace.output_count, bag.output_count);
    }

    #[test]
    fn smelt_brick_double_yield_vs_bag() {
        // 熔爐 2石→4磚 > 背包 2石→2磚
        let furnace = find_furnace_recipe("smelt_brick").unwrap();
        let bag = find_recipe("stone_brick").unwrap();
        assert_eq!(furnace.inputs[0].1, bag.inputs[0].1, "消耗相同石頭數");
        assert!(furnace.output_count > bag.output_count,
            "熔爐磚產量 {} 應 > 背包 {}", furnace.output_count, bag.output_count);
    }

    #[test]
    fn furnace_wb_recipe_in_workbench_table() {
        let r = find_workbench_recipe("furnace_wb").unwrap();
        assert_eq!(r.output_block, 16, "熔爐 id 應為 16（Furnace）");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(3, 8)], "熔爐需 8 石頭");
    }

    #[test]
    fn can_craft_each_recipe_with_enough_materials() {
        let mut store = InvStore::default();
        store.give("旅人", 5, 10);  // Wood
        store.give("旅人", 3, 10);  // Stone
        store.give("旅人", 4, 10);  // Sand
        store.give("旅人", 2, 10);  // Dirt
        store.give("旅人", 8, 10);  // Plank（工作台 + stone_wood_mix + 箱子 8木板 用）
        store.give("旅人", 18, 10); // Wheat（麵包配方用，WHEAT_ID）
        store.give("旅人", 20, 10); // CoalOre（smelt_iron 燃料用）
        store.give("旅人", 21, 10); // IronOre（smelt_iron 原料用）
        store.give("旅人", 22, 10); // IronIngot（iron_block 配方用，ROADMAP 684）
        // 火把配方：1 木頭(5) + 1 煤礦(20) → 4 火把（Wood/CoalOre 已加，數量足夠）
        for r in RECIPES.iter().chain(WORKBENCH_RECIPES.iter()).chain(FURNACE_RECIPES.iter()) {
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

    #[test]
    fn smelt_iron_outputs_iron_ingot() {
        // smelt_iron：1 鐵礦(21) + 1 煤礦(20) → 2 鐵錠(22)
        let r = find_furnace_recipe("smelt_iron").unwrap();
        assert_eq!(r.output_block, 22, "鐵錠 id 應為 22（IronIngot）");
        assert_eq!(r.output_count, 2, "應產出 2 個鐵錠");
        assert!(r.inputs.contains(&(21, 1)), "smelt_iron 需要 1 鐵礦(21)");
        assert!(r.inputs.contains(&(20, 1)), "smelt_iron 需要 1 煤礦(20)當燃料");
    }

    #[test]
    fn smelt_iron_requires_both_ores() {
        // 只有鐵礦或只有煤礦都不夠
        let r = find_furnace_recipe("smelt_iron").unwrap();
        let mut store_iron_only = InvStore::default();
        store_iron_only.give("旅人", 21, 5); // 有鐵礦但沒煤礦
        assert!(!can_craft(r, &store_iron_only, "旅人"), "只有鐵礦不夠 smelt_iron");

        let mut store_coal_only = InvStore::default();
        store_coal_only.give("旅人", 20, 5); // 有煤礦但沒鐵礦
        assert!(!can_craft(r, &store_coal_only, "旅人"), "只有煤礦不夠 smelt_iron");

        let mut store_both = InvStore::default();
        store_both.give("旅人", 21, 1);
        store_both.give("旅人", 20, 1);
        assert!(can_craft(r, &store_both, "旅人"), "各 1 鐵礦+煤礦可冶煉鐵錠");
    }

    #[test]
    fn smelt_iron_in_find_any_recipe() {
        // smelt_iron 應可透過統一查詢找到
        assert!(find_any_recipe("smelt_iron").is_some());
        // 但不在 2×2 或工作台表
        assert!(find_recipe("smelt_iron").is_none());
        assert!(find_workbench_recipe("smelt_iron").is_none());
    }

    #[test]
    fn iron_block_recipe_outputs_correct_block() {
        // iron_block：6 鐵錠(22) → 2 鐵磚(23)（ROADMAP 684）
        let r = find_workbench_recipe("iron_block").unwrap();
        assert_eq!(r.output_block, 23, "鐵磚 id 應為 23（IronBlock）");
        assert_eq!(r.output_count, 2, "6 鐵錠產出 2 鐵磚");
        assert!(r.inputs.contains(&(22, 6)), "iron_block 需要 6 鐵錠(22)");
    }

    #[test]
    fn iron_block_requires_six_ingots() {
        let r = find_workbench_recipe("iron_block").unwrap();

        let mut store_5 = InvStore::default();
        store_5.give("旅人", 22, 5);
        assert!(!can_craft(r, &store_5, "旅人"), "5 鐵錠不夠合鐵磚");

        let mut store_6 = InvStore::default();
        store_6.give("旅人", 22, 6);
        assert!(can_craft(r, &store_6, "旅人"), "6 鐵錠可合 2 鐵磚");
    }

    #[test]
    fn iron_block_in_find_any_recipe() {
        // iron_block 可透過統一查詢找到（工作台配方）
        assert!(find_any_recipe("iron_block").is_some());
        assert!(find_workbench_recipe("iron_block").is_some());
        // 不在 2×2 背包或熔爐表
        assert!(find_recipe("iron_block").is_none());
        assert!(find_furnace_recipe("iron_block").is_none());
    }

    #[test]
    fn torch_recipe_exists_and_correct() {
        // 火把配方：1 木頭(5) + 1 煤礦(20) → 4 火把(31)（ROADMAP 685）
        let r = find_recipe("torch").unwrap();
        assert_eq!(r.output_block, 31, "火把 id 應為 31（Torch）");
        assert_eq!(r.output_count, 4, "1 木+1 煤礦產出 4 火把");
        assert!(r.inputs.contains(&(5, 1)), "torch 需要 1 木頭(5)");
        assert!(r.inputs.contains(&(20, 1)), "torch 需要 1 煤礦(20)");
    }

    #[test]
    fn torch_requires_both_wood_and_coal() {
        let r = find_recipe("torch").unwrap();

        let mut store_wood_only = InvStore::default();
        store_wood_only.give("旅人", 5, 5); // 只有木頭沒煤礦
        assert!(!can_craft(r, &store_wood_only, "旅人"), "只有木頭不夠合火把");

        let mut store_coal_only = InvStore::default();
        store_coal_only.give("旅人", 20, 5); // 只有煤礦沒木頭
        assert!(!can_craft(r, &store_coal_only, "旅人"), "只有煤礦不夠合火把");

        let mut store_both = InvStore::default();
        store_both.give("旅人", 5, 1);
        store_both.give("旅人", 20, 1);
        assert!(can_craft(r, &store_both, "旅人"), "各 1 木頭+煤礦可合 4 火把");
    }

    #[test]
    fn torch_in_find_any_recipe() {
        // 火把是 2×2 背包配方，應可透過統一查詢和背包表找到
        assert!(find_any_recipe("torch").is_some());
        assert!(find_recipe("torch").is_some());
        // 不在工作台或熔爐表
        assert!(find_workbench_recipe("torch").is_none());
        assert!(find_furnace_recipe("torch").is_none());
    }

    // ── 鎬具配方測試（ROADMAP 687）────────────────────────────────────────────

    #[test]
    fn wood_pickaxe_recipe_correct() {
        let r = find_recipe("wood_pickaxe").unwrap();
        assert_eq!(r.output_block, PICKAXE_WOOD_ID, "木鎬 id 應為 {}", PICKAXE_WOOD_ID);
        assert_eq!(r.output_count, 1);
        // 需要 3 木頭(5) + 1 木板(8)，恰好填滿 2×2 四格
        assert!(r.inputs.iter().any(|&(b, c)| b == 5 && c == 3), "需要 3 木頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn stone_pickaxe_recipe_correct() {
        let r = find_recipe("stone_pickaxe").unwrap();
        assert_eq!(r.output_block, PICKAXE_STONE_ID, "石鎬 id 應為 {}", PICKAXE_STONE_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 3 && c == 3), "需要 3 石頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn iron_pickaxe_in_workbench_only() {
        // 鐵鎬在工作台表（5 格材料，背包 2×2 放不下）
        let r = find_workbench_recipe("iron_pickaxe").unwrap();
        assert_eq!(r.output_block, PICKAXE_IRON_ID, "鐵鎬 id 應為 {}", PICKAXE_IRON_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 22 && c == 3), "需要 3 鐵錠");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 2), "需要 2 木板");
        // 不在 2×2 背包表
        assert!(find_recipe("iron_pickaxe").is_none(), "鐵鎬不應在 2×2 背包表");
    }

    #[test]
    fn pickaxe_ids_unique_and_sequential() {
        // 三種鎬的 id 不重疊、依序增加
        assert!(PICKAXE_WOOD_ID < PICKAXE_STONE_ID);
        assert!(PICKAXE_STONE_ID < PICKAXE_IRON_ID);
        // 不與任何現有方塊衝突（現有已知方塊上限 = Torch=31）
        assert!(PICKAXE_WOOD_ID > 31, "鎬具 id 應高於現有方塊上限(31)");
    }

    #[test]
    fn ladder_recipe_in_2x2_bag() {
        // 梯子 v1（ROADMAP 688）：3 木板 → 3 梯子，在 2×2 背包合成
        let r = find_recipe("ladder").unwrap();
        assert_eq!(r.output_block, 35, "梯子 Block id 應為 35");
        assert_eq!(r.output_count, 3, "3 木板應得 3 梯子");
        assert_eq!(r.inputs, &[(8, 3)], "梯子配料應為 3 木板(id=8)");
        // 不在工作台或熔爐表（2×2 足夠）
        assert!(find_workbench_recipe("ladder").is_none(), "梯子不需工作台");
        assert!(find_furnace_recipe("ladder").is_none(),   "梯子不需熔爐");
    }

    #[test]
    fn ladder_block_is_not_solid() {
        // 梯子非實心——玩家可穿入並攀爬（ROADMAP 688）
        use crate::voxel::Block;
        assert!(!Block::Ladder.is_solid(), "梯子不應為實心（攀爬語意）");
        assert!(Block::Ladder.is_climbable(), "梯子應為可攀爬");
        assert!(Block::Ladder.is_placeable(), "梯子應可放置");
        // 非水
        assert!(!Block::Ladder.is_any_water());
        assert!(!Block::Ladder.is_flowing_water());
    }

    // ── 斧頭配方測試（ROADMAP 689）────────────────────────────────────────────

    #[test]
    fn wood_axe_recipe_correct() {
        let r = find_recipe("wood_axe").unwrap();
        assert_eq!(r.output_block, AXE_WOOD_ID, "木斧 id 應為 {}", AXE_WOOD_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 5 && c == 3), "需要 3 木頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn stone_axe_recipe_correct() {
        let r = find_recipe("stone_axe").unwrap();
        assert_eq!(r.output_block, AXE_STONE_ID, "石斧 id 應為 {}", AXE_STONE_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 3 && c == 3), "需要 3 石頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn iron_axe_in_workbench_only() {
        // 鐵斧在工作台表（5 格材料，背包 2×2 放不下）
        let r = find_workbench_recipe("iron_axe").unwrap();
        assert_eq!(r.output_block, AXE_IRON_ID, "鐵斧 id 應為 {}", AXE_IRON_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 22 && c == 3), "需要 3 鐵錠");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 2), "需要 2 木板");
        assert!(find_recipe("iron_axe").is_none(), "鐵斧不應在 2×2 背包表");
    }

    #[test]
    fn axe_ids_unique_and_not_conflicting() {
        // 三種斧頭 id 不重疊
        assert!(AXE_WOOD_ID < AXE_STONE_ID);
        assert!(AXE_STONE_ID < AXE_IRON_ID);
        // 不與鎬具 id 衝突（鎬具 32/33/34、梯子 block=35）
        assert!(AXE_WOOD_ID > PICKAXE_IRON_ID, "斧頭 id 應高於鎬具最大 id(34)");
        // 不與梯子 block id=35 衝突（36/37/38 均 > 35）
        assert!(AXE_WOOD_ID > 35, "斧頭 id 應高於梯子 block id(35)");
    }

    // ── 鏟子配方測試（ROADMAP 690）────────────────────────────────────────────

    #[test]
    fn wood_shovel_recipe_correct() {
        let r = find_recipe("wood_shovel").unwrap();
        assert_eq!(r.output_block, SHOVEL_WOOD_ID, "木鏟 id 應為 {}", SHOVEL_WOOD_ID);
        assert_eq!(r.output_count, 1);
        // 1 木頭(5) + 1 木板(8)
        assert!(r.inputs.iter().any(|&(b, c)| b == 5 && c == 1), "需要 1 木頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn stone_shovel_recipe_correct() {
        let r = find_recipe("stone_shovel").unwrap();
        assert_eq!(r.output_block, SHOVEL_STONE_ID, "石鏟 id 應為 {}", SHOVEL_STONE_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 3 && c == 1), "需要 1 石頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn iron_shovel_in_workbench_only() {
        // 鐵鏟在工作台表（5 格材料，工藝精工需工作台）
        let r = find_workbench_recipe("iron_shovel").unwrap();
        assert_eq!(r.output_block, SHOVEL_IRON_ID, "鐵鏟 id 應為 {}", SHOVEL_IRON_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 22 && c == 2), "需要 2 鐵錠");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 3), "需要 3 木板");
        // 不在 2×2 背包表
        assert!(find_recipe("iron_shovel").is_none(), "鐵鏟不應在 2×2 背包表");
    }

    #[test]
    fn shovel_ids_unique_and_not_conflicting() {
        // 三種鏟子 id 不重疊、依序增加
        assert!(SHOVEL_WOOD_ID < SHOVEL_STONE_ID);
        assert!(SHOVEL_STONE_ID < SHOVEL_IRON_ID);
        // 不與斧頭衝突（斧頭 36/37/38）
        assert!(SHOVEL_WOOD_ID > AXE_IRON_ID, "鏟子 id 應高於斧頭最大 id(38)");
    }

    // ── 箱子配方測試（ROADMAP 692）────────────────────────────────────────────

    #[test]
    fn chest_recipe_in_workbench_only() {
        // 箱子：8 木板(8) → 1 箱子(42)，需工作台（8 格超出 2×2 上限）
        let r = find_workbench_recipe("chest").unwrap();
        assert_eq!(r.output_block, 42, "箱子 block id 應為 42");
        assert_eq!(r.output_count, 1, "8 木板得 1 箱子");
        assert_eq!(r.inputs, &[(8, 8)], "箱子需要 8 木板(id=8)");
        // 不在 2×2 背包表
        assert!(find_recipe("chest").is_none(), "箱子不在 2×2 背包表");
        assert!(find_furnace_recipe("chest").is_none(), "箱子不在熔爐表");
    }

    #[test]
    fn chest_requires_eight_planks() {
        let r = find_workbench_recipe("chest").unwrap();

        let mut store_7 = InvStore::default();
        store_7.give("旅人", 8, 7); // 只有 7 木板
        assert!(!can_craft(r, &store_7, "旅人"), "7 木板不夠合箱子（需 8）");

        let mut store_8 = InvStore::default();
        store_8.give("旅人", 8, 8); // 剛好 8 木板
        assert!(can_craft(r, &store_8, "旅人"), "8 木板可合 1 箱子");
    }

    #[test]
    fn chest_in_find_any_recipe() {
        // 箱子可透過統一查詢找到（工作台配方）
        assert!(find_any_recipe("chest").is_some());
        assert!(find_workbench_recipe("chest").is_some());
        // 不在 2×2 背包或熔爐表
        assert!(find_recipe("chest").is_none());
        assert!(find_furnace_recipe("chest").is_none());
    }
}
