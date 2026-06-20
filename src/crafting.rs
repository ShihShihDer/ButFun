//! 合成系統與配方目錄（Phase 1-C 合成系統的純邏輯地基）。
//!
//! 這層負責「投入素材、產出物品」的邏輯與配方表。是純資料 + 純函式，無 IO、
//! 不碰 WebSocket / 遊戲迴圈，便於自動測試。
//!
//! 互動模型：
//!   - 玩家點擊合成台 UI → 送出合成意圖（產物 id）。
//!   - 伺服器校驗素材是否足夠、背包是否放得下產物。
//!   - 扣除素材、增加產物。
//!
//! 目前配方：
//!   - 鎬子 (pickaxe)：木×3 + 石×2 -> 鎬子×1
//!   - 強化鎬 (reinforced_pickaxe)：鎬子×1 + 木×2 + 石×4 -> 強化鎬×1
//!   - 武器 (weapon)：石×4 + 乙太×2 -> 武器×1 (Phase 1 武器 MVP)
//!   - 活力藥水 (healing_potion)：野花種子×3 -> 活力藥水×1 (ROADMAP 14 生態資源合成)
//!   - 晶石強化液 (crystal_potion)：晶石碎片×5 -> 晶石強化液×1 (ROADMAP 15 更多生態玩法)
//!   - 蕈菇活化液 (mushroom_elixir)：蕈菇孢子×4 -> 蕈菇活化液×1 (ROADMAP 15)
//!   - 古代乙太丸 (ether_pill)：古代碎片×3 -> 古代乙太丸×1 (ROADMAP 15)
//!   - 珍珠復原藥 (pearl_potion)：深海珍珠×1 -> 珍珠復原藥×1 (ROADMAP 15)

use crate::inventory::{Inventory, ItemKind, MAX_STACK};

/// 一條合成配方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// 唯一的配方 ID（通常與產物 ItemKind 的 snake_case 名一致）。
    pub id: &'static str,
    /// 所需素材：(種類, 數量)。
    pub inputs: &'static [(ItemKind, u32)],
    /// 產出的物品種類。
    pub output: ItemKind,
    /// 產出的數量（通常為 1）。
    pub output_qty: u32,
}

impl Recipe {
    /// 檢查 `inv` 是否湊得齊素材、且產物放得下（不超過堆疊上限）。
    /// 不改變背包。
    pub fn can_craft(&self, inv: &Inventory) -> bool {
        // 1. 素材是否足夠。
        for &(item, qty) in self.inputs {
            if !inv.has(item, qty) {
                return false;
            }
        }
        // 2. 產物是否放得下。
        // 為簡化邏輯，暫不考慮「扣掉素材後騰出的空間」，直接以當前量判斷。
        // 現有配方產物（工具）與素材（資源）不相交，此捷徑成立。
        inv.count(self.output).saturating_add(self.output_qty) <= MAX_STACK
    }

    /// 執行合成：扣除素材、加入產物。
    /// 成功回 `true`；若 `can_craft` 失敗則回 `false` 且完全不改變背包（原子性）。
    pub fn craft(&self, inv: &mut Inventory) -> bool {
        self.craft_with_discount(inv, 0)
    }

    /// 工匠折扣合成：每項素材減 `input_reduction`（最少取 1）。
    /// 成功回 `true`；材料不足或產物放不下回 `false`（背包不變）。
    pub fn craft_with_discount(&self, inv: &mut Inventory, input_reduction: u32) -> bool {
        // 1. 素材是否足夠（套用折扣後的需求量）。
        for &(item, qty) in self.inputs {
            let needed = qty.saturating_sub(input_reduction).max(1);
            if !inv.has(item, needed) {
                return false;
            }
        }
        // 2. 產物是否放得下。
        if inv.count(self.output).saturating_add(self.output_qty) > MAX_STACK {
            return false;
        }
        // 扣料。
        for &(item, qty) in self.inputs {
            let needed = qty.saturating_sub(input_reduction).max(1);
            let ok = inv.take(item, needed);
            debug_assert!(ok, "craft_with_discount 通過但 take 失敗：{:?}", item);
        }
        // 給產物。
        inv.add(self.output, self.output_qty);
        true
    }
}

/// 鎬子配方：木×3 + 石×2 -> 鎬子×1。
fn pickaxe() -> Recipe {
    Recipe {
        id: "pickaxe",
        inputs: &[(ItemKind::Wood, 3), (ItemKind::Stone, 2)],
        output: ItemKind::Pickaxe,
        output_qty: 1,
    }
}

/// 強化鎬配方：鎬子×1 + 木×2 + 石×4 -> 強化鎬×1。
fn reinforced_pickaxe() -> Recipe {
    Recipe {
        id: "reinforced_pickaxe",
        inputs: &[
            (ItemKind::Pickaxe, 1),
            (ItemKind::Wood, 2),
            (ItemKind::Stone, 4),
        ],
        output: ItemKind::ReinforcedPickaxe,
        output_qty: 1,
    }
}

/// 武器配方 (Phase 1 戰鬥 MVP)：石×4 + 乙太×2 -> 武器×1。
fn weapon() -> Recipe {
    Recipe {
        id: "weapon",
        inputs: &[(ItemKind::Stone, 4), (ItemKind::Ether, 2)],
        output: ItemKind::Weapon,
        output_qty: 1,
    }
}

/// 斧頭配方 (ROADMAP 433 林間揮斧工具進程)：木×4 + 石×3 -> 斧頭×1。
/// 比鎬子（木×3＋石×2）略貴，是另一條「採集→合成→更快採」的工具，專加速伐木。
fn axe() -> Recipe {
    Recipe {
        id: "axe",
        inputs: &[(ItemKind::Wood, 4), (ItemKind::Stone, 3)],
        output: ItemKind::Axe,
        output_qty: 1,
    }
}

/// 釣竿配方 (ROADMAP 434 工欲善其釣的工具進程)：木×3 + 乙太×2 -> 釣竿×1。
/// 木桿配上一縷乙太織成的釣線——身上有它，收竿釣到的魚品質提升一階（好魚機率明顯變高）。
/// 給釣魚這條一直徒手的活動第一個工具，把「採集→合成→活動更好」正回饋圈擴到水邊。
fn fishing_rod() -> Recipe {
    Recipe {
        id: "fishing_rod",
        inputs: &[(ItemKind::Wood, 3), (ItemKind::Ether, 2)],
        output: ItemKind::FishingRod,
        output_qty: 1,
    }
}

/// 全域配方目錄。
pub const RECIPES: &[Recipe] = &[
    Recipe {
        id: "pickaxe",
        inputs: &[(ItemKind::Wood, 3), (ItemKind::Stone, 2)],
        output: ItemKind::Pickaxe,
        output_qty: 1,
    },
    Recipe {
        id: "reinforced_pickaxe",
        inputs: &[
            (ItemKind::Pickaxe, 1),
            (ItemKind::Wood, 2),
            (ItemKind::Stone, 4),
        ],
        output: ItemKind::ReinforcedPickaxe,
        output_qty: 1,
    },
    Recipe {
        id: "weapon",
        inputs: &[(ItemKind::Stone, 4), (ItemKind::Ether, 2)],
        output: ItemKind::Weapon,
        output_qty: 1,
    },
    /// 斧頭 (ROADMAP 433)：木×4 + 石×3 → 斧頭×1。伐木工具，身上有它放倒一棵樹更快、木材更多。
    Recipe {
        id: "axe",
        inputs: &[(ItemKind::Wood, 4), (ItemKind::Stone, 3)],
        output: ItemKind::Axe,
        output_qty: 1,
    },
    /// 釣竿 (ROADMAP 434)：木×3 + 乙太×2 → 釣竿×1。釣魚工具，身上有它收竿釣到的魚品質提升一階。
    Recipe {
        id: "fishing_rod",
        inputs: &[(ItemKind::Wood, 3), (ItemKind::Ether, 2)],
        output: ItemKind::FishingRod,
        output_qty: 1,
    },
    /// 活力藥水：野花種子×3 → 活力藥水×1。讓生態資源有「賣給 NPC」之外的「自用保命」出路。
    Recipe {
        id: "healing_potion",
        inputs: &[(ItemKind::WildflowerSeed, 3)],
        output: ItemKind::HealingPotion,
        output_qty: 1,
    },
    /// 晶石強化液：晶石碎片×5 → 晶石強化液×1。回復 12 HP，深層晶洞探索的 Premium 回報。
    Recipe {
        id: "crystal_potion",
        inputs: &[(ItemKind::CrystalShard, 5)],
        output: ItemKind::CrystalPotion,
        output_qty: 1,
    },
    /// 蕈菇活化液：蕈菇孢子×4 → 蕈菇活化液×1。回復 8 HP 並重置回血冷卻，讓回血立刻開始。
    Recipe {
        id: "mushroom_elixir",
        inputs: &[(ItemKind::MushroomSpore, 4)],
        output: ItemKind::MushroomElixir,
        output_qty: 1,
    },
    /// 古代乙太丸：古代碎片×3 → 古代乙太丸×1。使用即得 10 乙太，在野外兌換遺跡能量。
    Recipe {
        id: "ether_pill",
        inputs: &[(ItemKind::AncientFragment, 3)],
        output: ItemKind::EtherPill,
        output_qty: 1,
    },
    /// 珍珠復原藥：深海珍珠×1 → 珍珠復原藥×1。使用後回復至滿血，最稀有材料換來最強效果。
    Recipe {
        id: "pearl_potion",
        inputs: &[(ItemKind::DeepSeaPearl, 1)],
        output: ItemKind::PearlPotion,
        output_qty: 1,
    },
    /// 晶石之刃：晶石碎片×6 → 晶石之刃×1。持有後攻擊力 +8，Rocky 探索的進階武器。
    Recipe {
        id: "crystal_blade",
        inputs: &[(ItemKind::CrystalShard, 6)],
        output: ItemKind::CrystalBlade,
        output_qty: 1,
    },
    /// 珊瑚矛：深海珍珠×3 → 珊瑚矛×1。持有後攻擊力 +12，全遊戲最強武器。
    Recipe {
        id: "coral_lance",
        inputs: &[(ItemKind::DeepSeaPearl, 3)],
        output: ItemKind::CoralLance,
        output_qty: 1,
    },
    /// 草原護符：野花種子×8 → 草原護符×1。持有後每次受傷減 1 點傷害。
    Recipe {
        id: "meadow_amulet",
        inputs: &[(ItemKind::WildflowerSeed, 8)],
        output: ItemKind::MeadowAmulet,
        output_qty: 1,
    },
    /// 晶石護盾：晶石碎片×8 + 石頭×4 → 晶石護盾×1。持有後每次受傷減 2 點傷害。
    Recipe {
        id: "crystal_shield",
        inputs: &[(ItemKind::CrystalShard, 8), (ItemKind::Stone, 4)],
        output: ItemKind::CrystalShield,
        output_qty: 1,
    },
    /// 星圖：古代碎片×5 → 星圖×1。使用後展開遠方星球的星圖快照，多星球旅程的序章。
    Recipe {
        id: "star_chart",
        inputs: &[(ItemKind::AncientFragment, 5)],
        output: ItemKind::StarChart,
        output_qty: 1,
    },
    /// 蕈菇杖：蕈菇孢子×6 → 蕈菇杖×1。持有後攻擊力 +7，補足森林生態的武器空缺。
    /// 比基礎武器（+5）強、比晶石之刃（+8）稍弱——蕈菇孢子比晶石碎片更容易取得。
    Recipe {
        id: "mushroom_staff",
        inputs: &[(ItemKind::MushroomSpore, 6)],
        output: ItemKind::MushroomStaff,
        output_qty: 1,
    },
    /// 符文刃：古代碎片×4 → 符文刃×1。持有後攻擊力 +10，沙漠文明鍛造的精英刃。
    /// 比晶石之刃（+8）更強、比珊瑚矛（+12）略弱——沙漠探索的中段武器升段。
    Recipe {
        id: "rune_blade",
        inputs: &[(ItemKind::AncientFragment, 4)],
        output: ItemKind::RuneBlade,
        output_qty: 1,
    },
    /// 翠幽精露：翠幽碎片×2 → 翠幽精露×1。使用後回復至等級滿血並重置回血冷卻。
    /// 翠幽星異星植物萃取，結合珍珠復原藥（滿血）與蕈菇活化液（重置回血）的雙重效果。
    Recipe {
        id: "jade_elixir",
        inputs: &[(ItemKind::JadeShard, 2)],
        output: ItemKind::JadeElixir,
        output_qty: 1,
    },
    /// 翠幽刃：翠幽碎片×5 → 翠幽刃×1。持有後攻擊力 +15，翠幽星域強力武器。
    /// 翠幽星獨有，超越珊瑚矛（+12），鼓勵玩家深入翠幽星探索。
    Recipe {
        id: "jade_blade",
        inputs: &[(ItemKind::JadeShard, 5)],
        output: ItemKind::JadeBlade,
        output_qty: 1,
    },
    /// 蒸汽精粹：熔晶碎片×2 → 蒸汽精粹×1。使用後回復至等級滿血，同時獲得 8 乙太。
    /// 赤焰星蒸汽燃料轉換器——異星能量凝聚而成，結合滿血與乙太雙重獎勵。
    Recipe {
        id: "steam_elixir",
        inputs: &[(ItemKind::LavaCrystal, 2)],
        output: ItemKind::SteamElixir,
        output_qty: 1,
    },
    /// 赤焰刃：熔晶碎片×6 → 赤焰刃×1。持有後攻擊力 +20，全遊戲最強武器。
    /// 赤焰星獨有，超越翠幽刃（+15），蒸汽龐克文明的最高武裝結晶。
    Recipe {
        id: "crimson_blade",
        inputs: &[(ItemKind::LavaCrystal, 6)],
        output: ItemKind::CrimsonBlade,
        output_qty: 1,
    },
    /// 虛空精粹：虛空碎片×2 → 虛空精粹×1。使用後回復至等級滿血，同時獲得 10 乙太。
    /// 虛空星宇宙深淵能量轉換器——比蒸汽精粹（+8 乙太）更強，終極異星精華。
    Recipe {
        id: "void_elixir",
        inputs: &[(ItemKind::VoidShard, 2)],
        output: ItemKind::VoidElixir,
        output_qty: 1,
    },
    /// 虛空刃：虛空碎片×6 → 虛空刃×1。持有後攻擊力 +25，超越赤焰刃（+20），宇宙深淵高階武裝。
    Recipe {
        id: "void_blade",
        inputs: &[(ItemKind::VoidShard, 6)],
        output: ItemKind::VoidBlade,
        output_qty: 1,
    },
    /// 霧醚精粹：霧醚碎片×2 → 霧醚精粹×1。使用後回復至等級滿血，同時獲得 15 乙太。
    /// 霧醚星乙太迷霧高密度能量轉換——四大星球最強補給，乙太迷霧比宇宙深淵更富饒。
    Recipe {
        id: "aether_essence",
        inputs: &[(ItemKind::AetherShard, 2)],
        output: ItemKind::AetherEssence,
        output_qty: 1,
    },
    /// 霧醚之刃：霧醚碎片×8 → 霧醚之刃×1。持有後攻擊力 +30，全遊戲最強武器。
    /// 霧醚星獨有，超越虛空刃（+25），乙太迷霧凝結的終極宇宙武裝。
    Recipe {
        id: "aether_blade",
        inputs: &[(ItemKind::AetherShard, 8)],
        output: ItemKind::AetherBlade,
        output_qty: 1,
    },
    /// 源晶精粹：源晶碎片×2 → 源晶精粹×1。使用後回復至等級滿血，同時獲得 20 乙太。
    /// 星源星宇宙源頭能量轉換——五大星球最強補給，起源之力比乙太迷霧更為深邃。
    Recipe {
        id: "origin_essence",
        inputs: &[(ItemKind::OriginShard, 2)],
        output: ItemKind::OriginEssence,
        output_qty: 1,
    },
    /// 源晶之刃：源晶碎片×10 → 源晶之刃×1。持有後攻擊力 +40，全遊戲最強武器。
    /// 星源星獨有，超越霧醚之刃（+30），宇宙起源之力凝聚的終極武裝，只有踏上星源星才能鑄造。
    Recipe {
        id: "origin_blade",
        inputs: &[(ItemKind::OriginShard, 10)],
        output: ItemKind::OriginBlade,
        output_qty: 1,
    },
    /// 宇宙護盾：裂縫碎片×3 → 宇宙護盾×1。持有後每次受傷減 6 點傷害——全遊戲最強防禦裝備。
    /// 宇宙裂縫事件限定材料，收集裂縫碎片並鍛造，遊歷宇宙的終極護盾。
    Recipe {
        id: "cosmic_shield",
        inputs: &[(ItemKind::RiftShard, 3)],
        output: ItemKind::CosmicShield,
        output_qty: 1,
    },

    // ── 烹飪配方（ROADMAP 47 釣魚與烹飪）────────────────────────────────────
    /// 烤魚：小魚×2 → 烤魚×1。使用後回復 8 HP。把最普通的漁獲轉成療癒食物。
    Recipe {
        id: "grilled_fish",
        inputs: &[(ItemKind::FishSmall, 2)],
        output: ItemKind::GrilledFish,
        output_qty: 1,
    },
    /// 星燦刺身：星星魚×1 → 星燦刺身×1。使用後回復 15 HP。稀有魚的精緻料理。
    Recipe {
        id: "star_sashimi",
        inputs: &[(ItemKind::FishStar, 1)],
        output: ItemKind::StarSashimi,
        output_qty: 1,
    },
    /// 深海濃湯：深海魚×1 → 深海濃湯×1。使用後回復至等級滿血——最稀有漁獲換最強效果。
    Recipe {
        id: "deep_broth",
        inputs: &[(ItemKind::FishDeep, 1)],
        output: ItemKind::DeepBroth,
        output_qty: 1,
    },

    // ── 牧場料理（ROADMAP 48 牧場系統）──────────────────────────────────────
    /// 煎蛋：雞蛋×2 → 煎蛋×1。使用後回復 10 HP。農田地塊養雞的第一道料理。
    Recipe {
        id: "fried_egg",
        inputs: &[(ItemKind::Egg, 2)],
        output: ItemKind::FriedEgg,
        output_qty: 1,
    },

    // ── 農地料理（ROADMAP 49 農田地塊種作物）────────────────────────────────
    /// 麵包：小麥×3 → 麵包×1。使用後回復 12 HP。農田種植最基礎的糧食，比煎蛋多兩滴血。
    Recipe {
        id: "bread",
        inputs: &[(ItemKind::WheatGrain, 3)],
        output: ItemKind::Bread,
        output_qty: 1,
    },
    /// 蔬菜湯：胡蘿蔔×2 → 蔬菜湯×1。使用後回復 10 HP 並立即啟動自然回血。
    Recipe {
        id: "carrot_soup",
        inputs: &[(ItemKind::Carrot, 2)],
        output: ItemKind::CarrotSoup,
        output_qty: 1,
    },
    /// 焗烤馬鈴薯：馬鈴薯×2 → 焗烤馬鈴薯×1。使用後回復 15 HP——農地料理最豐盛的一道。
    Recipe {
        id: "potato_gratin",
        inputs: &[(ItemKind::Potato, 2)],
        output: ItemKind::PotatoGratin,
        output_qty: 1,
    },

    // ── 夜採星晶料理（ROADMAP 50 夜間限定採集）──────────────────────────────
    /// 夜幻藥水：星晶碎片×3 → 夜幻藥水×1。使用後回復 20 HP——夜間探索最強效的補給。
    /// 材料來自夜間採集的星晶礦脈，是探索者路線（第五活動路線）的專屬合成品。
    Recipe {
        id: "night_potion",
        inputs: &[(ItemKind::StarCrystalShard, 3)],
        output: ItemKind::NightPotion,
        output_qty: 1,
    },

    // ── 農耕自動化（ROADMAP 112 灑水器）──────────────────────────────────────
    /// 灑水器：木×3 + 石×3 → 灑水器×1。
    /// 放在農地旁，每 30 秒自動澆灌周圍 2 格的作物，與「下雨澆田」共用同一套澆水邏輯。
    Recipe {
        id: "sprinkler",
        inputs: &[(ItemKind::Wood, 3), (ItemKind::Stone, 3)],
        output: ItemKind::Sprinkler,
        output_qty: 1,
    },
    // ── 城鎮慶典配方（ROADMAP 130）────────────────────────────────────────────
    /// 城鎮特釀：野花種子×4 + 小麥×3 → 城鎮特釀×1。需城鎮達到【生機】才可合成。
    Recipe {
        id: "town_brew",
        inputs: &[(ItemKind::WildflowerSeed, 4), (ItemKind::WheatGrain, 3)],
        output: ItemKind::TownBrew,
        output_qty: 1,
    },
    /// 繁盛精露：蕈菇孢子×3 + 深海珍珠×1 + 古代碎片×2 → 繁盛精露×1。需城鎮達到【繁盛】才可合成。
    Recipe {
        id: "vibrant_elixir",
        inputs: &[
            (ItemKind::MushroomSpore, 3),
            (ItemKind::DeepSeaPearl, 1),
            (ItemKind::AncientFragment, 2),
        ],
        output: ItemKind::VibrantElixir,
        output_qty: 1,
    },
    /// 星光護符：星塵×3 → 星光護符×1（ROADMAP 133）。持有時採集/戰鬥 EXP +10%。
    Recipe {
        id: "star_amulet",
        inputs: &[(ItemKind::StarDust, 3)],
        output: ItemKind::StarAmulet,
        output_qty: 1,
    },
    /// 星際守護符：彩虹星塵×1 + 星塵×4 + 星晶碎片×2 → 星際守護符×1（ROADMAP 134）。
    /// 持有時採集/戰鬥 EXP +15%；流星雨期間每採集節點額外 +1 星塵。
    Recipe {
        id: "star_guardian_amulet",
        inputs: &[(ItemKind::RainbowStarDust, 1), (ItemKind::StarDust, 4), (ItemKind::StarCrystalShard, 2)],
        output: ItemKind::StarGuardianAmulet,
        output_qty: 1,
    },

    // ── 進階武器（ROADMAP 145 武器/裝備進階線）────────────────────────────────
    /// 硬化刃：石頭×8 + 乙太×4 → 硬化刃×1。攻擊力 +7。
    /// 不需探索特殊地形的「勤勞路」——與蕈菇杖（孢子路）同等級，給只在城附近採礦的玩家選擇。
    Recipe {
        id: "hardened_blade",
        inputs: &[(ItemKind::Stone, 8), (ItemKind::Ether, 4)],
        output: ItemKind::HardenedBlade,
        output_qty: 1,
    },
    /// 星晶之刃：星晶碎片×7 → 星晶之刃×1。攻擊力 +14（需 Lv.10）。
    /// 夜行者的橋接武器，填補珊瑚矛(+12)→翠幽刃(+15)的缺口。
    Recipe {
        id: "star_crystal_blade",
        inputs: &[(ItemKind::StarCrystalShard, 7)],
        output: ItemKind::StarCrystalBlade,
        output_qty: 1,
    },
    /// 裂縫刃：裂縫碎片×4 → 裂縫刃×1。攻擊力 +35（需 Lv.15）。
    /// 宇宙裂縫事件限定，高風險換來超越霧醚之刃(+30)的高回報。
    Recipe {
        id: "rift_blade",
        inputs: &[(ItemKind::RiftShard, 4)],
        output: ItemKind::RiftBlade,
        output_qty: 1,
    },

    // ── 進階護甲（ROADMAP 145 武器/裝備進階線）────────────────────────────────
    /// 珊瑚鎧：深海珍珠×2 + 晶石碎片×6 → 珊瑚鎧×1。每次受傷減 3 點傷害。
    /// 珊瑚礁探索者的獎賞，填補晶石護盾(def 2)→宇宙護盾(def 6)的空缺第一梯。
    Recipe {
        id: "coral_armor",
        inputs: &[(ItemKind::DeepSeaPearl, 2), (ItemKind::CrystalShard, 6)],
        output: ItemKind::CoralArmor,
        output_qty: 1,
    },
    /// 符文鎧：古代碎片×5 + 石頭×6 → 符文鎧×1。每次受傷減 4 點傷害。
    /// 沙漠遺跡文明的防禦鎧甲，進階護甲第二梯，沙漠探索者的高級裝備。
    Recipe {
        id: "rune_armor",
        inputs: &[(ItemKind::AncientFragment, 5), (ItemKind::Stone, 6)],
        output: ItemKind::RuneArmor,
        output_qty: 1,
    },
    /// 星晶鎧：星晶碎片×5 + 石頭×4 → 星晶鎧×1。每次受傷減 5 點傷害（需 Lv.10）。
    /// 夜採星晶打造的全身護甲，防禦值僅次宇宙護盾(def 6)——夜行探索者的盔甲巔峰。
    Recipe {
        id: "star_crystal_armor",
        inputs: &[(ItemKind::StarCrystalShard, 5), (ItemKind::Stone, 4)],
        output: ItemKind::StarCrystalArmor,
        output_qty: 1,
    },

    // ── 遠程武器（ROADMAP 146 遠程武器）────────────────────────────────────────
    /// 乙太弓：乙太×5 + 木材×4 → 乙太弓×1。遠程攻擊力 +9。
    /// 入門遠程武器，基礎材料就能打造——讓任何玩家都能嘗試「站遠打」的戰鬥風格。
    Recipe {
        id: "ether_bow",
        inputs: &[(ItemKind::Ether, 5), (ItemKind::Wood, 4)],
        output: ItemKind::EtherBow,
        output_qty: 1,
    },
    /// 晶石弩：晶石碎片×5 + 石頭×4 → 晶石弩×1。遠程攻擊力 +14。
    /// 深層晶洞探索者的進階遠程武器，需要冒險採集晶石才能鑄造。
    Recipe {
        id: "crystal_ballista",
        inputs: &[(ItemKind::CrystalShard, 5), (ItemKind::Stone, 4)],
        output: ItemKind::CrystalBallista,
        output_qty: 1,
    },
    /// 虛空炮：虛空碎片×5 + 石頭×3 → 虛空炮×1（需 Lv.18）。遠程攻擊力 +27。
    /// 虛空星限定頂級遠程武器，需要踏上虛空星採集材料，配合等級門檻確保高端玩家才能使用。
    Recipe {
        id: "void_cannon",
        inputs: &[(ItemKind::VoidShard, 5), (ItemKind::Stone, 3)],
        output: ItemKind::VoidCannon,
        output_qty: 1,
    },

    // ── 季節性限定合成（ROADMAP 154 季節性野外採集）────────────────────────────
    /// 春日香囊：野花×2 → 春日香囊×1。使用後回血 25hp + 重置回血冷卻。
    /// 春天城外野花田採集原料，春日獨有的療癒香囊。
    Recipe {
        id: "spring_sachet",
        inputs: &[(ItemKind::WildFlower, 2)],
        output: ItemKind::SpringSachet,
        output_qty: 1,
    },
    /// 夏日精粹：太陽碎片×2 → 夏日精粹×1。使用後回血 15hp + 獲得 15 乙太。
    /// 夏天城東日照強地帶採集，把太陽能量轉化為乙太。
    Recipe {
        id: "summer_elixir",
        inputs: &[(ItemKind::SolarShard, 2)],
        output: ItemKind::SummerElixir,
        output_qty: 1,
    },
    /// 秋日補藥：楓葉×2 → 秋日補藥×1。使用後回血 20hp + 農夫熟練度 +20 XP。
    /// 秋天城南楓林採集，滋養農夫的秋季補藥。
    Recipe {
        id: "autumn_tonic",
        inputs: &[(ItemKind::MapleLeaf, 2)],
        output: ItemKind::AutumnTonic,
        output_qty: 1,
    },
    /// 冬日神藥：冰晶碎片×2 → 冬日神藥×1。使用後回復至等級滿血。
    /// 冬天城西北寒地採集最難，換得最強效果——凜冬的治癒。
    Recipe {
        id: "winter_medicine",
        inputs: &[(ItemKind::IceShard, 2)],
        output: ItemKind::WinterMedicine,
        output_qty: 1,
    },

    // ── 住家家具（ROADMAP 155）─────────────────────────────────────────────────
    /// 蒸汽床鋪：木材×4 + 石頭×2 → 蒸汽床鋪×1。放置住家後每 30 秒回血 2。
    Recipe {
        id: "steam_bed",
        inputs: &[(ItemKind::Wood, 4), (ItemKind::Stone, 2)],
        output: ItemKind::SteamBed,
        output_qty: 1,
    },
    /// 乙太寶箱：木材×3 + 石頭×4 → 乙太寶箱×1。放置後背包種類上限 +3。
    Recipe {
        id: "aether_chest",
        inputs: &[(ItemKind::Wood, 3), (ItemKind::Stone, 4)],
        output: ItemKind::AetherChest,
        output_qty: 1,
    },
    /// 乙太花盆：野花×2 + 木材×2 → 乙太花盆×1。放置後採集 EXP +8%。
    Recipe {
        id: "ether_plant",
        inputs: &[(ItemKind::WildFlower, 2), (ItemKind::Wood, 2)],
        output: ItemKind::EtherPlant,
        output_qty: 1,
    },
    /// 星魂燈：星晶碎片×2 + 石頭×2 → 星魂燈×1。放置後夜間攻擊力 +2。
    Recipe {
        id: "star_lantern",
        inputs: &[(ItemKind::StarCrystalShard, 2), (ItemKind::Stone, 2)],
        output: ItemKind::StarLantern,
        output_qty: 1,
    },
    /// 古代擺件：古代碎片×2 + 石頭×1 → 古代擺件×1。放置後 NPC 收購 +10%。
    Recipe {
        id: "ancient_deco",
        inputs: &[(ItemKind::AncientFragment, 2), (ItemKind::Stone, 1)],
        output: ItemKind::AncientDeco,
        output_qty: 1,
    },
    /// 水族缸：木材×3 + 石頭×3 → 水族缸×1。放置後依背包養著的魚種數每 25 秒回血（ROADMAP 437）。
    Recipe {
        id: "aquarium",
        inputs: &[(ItemKind::Wood, 3), (ItemKind::Stone, 3)],
        output: ItemKind::Aquarium,
        output_qty: 1,
    },

    // ── 入侵首領限定合成（ROADMAP 160）────────────────────────────────────────
    /// 守城戰刃：霸主晶核×2 + 乙太×20 → 守城戰刃×1。攻擊力 +28。
    /// 只有擊殺入侵首領乙太霸主才能取得霸主晶核，象徵守城英雄的最高榮耀。
    Recipe {
        id: "ether_overlord_blade",
        inputs: &[(ItemKind::EtherOverlordCore, 2), (ItemKind::Ether, 20)],
        output: ItemKind::EtherOverlordBlade,
        output_qty: 1,
    },
    /// Alpha 之力：Alpha 晶核×2 + 乙太礦石×5 → Alpha 之力×1。
    /// 使用後回滿血 + 獲得 +25 乙太——Alpha 的原始生命力傾注你的身體。
    Recipe {
        id: "alpha_force",
        inputs: &[(ItemKind::AlphaCrystal, 2), (ItemKind::Ether, 5)],
        output: ItemKind::AlphaForce,
        output_qty: 1,
    },

    // ── 傳說古 Alpha 限定合成（ROADMAP 173）──────────────────────────────────
    /// 傳說戰刃：傳說晶核×1 + Alpha 晶核×3 + 乙太礦石×30 → 傳說戰刃×1。攻擊力 +55。
    /// 需擊倒傳說古 Alpha 取得傳說晶核（全遊戲最稀有），象徵「生態系征服者」的終極武器。
    Recipe {
        id: "legendary_blade",
        inputs: &[(ItemKind::LegendaryCore, 1), (ItemKind::AlphaCrystal, 3), (ItemKind::Ether, 30)],
        output: ItemKind::LegendaryBlade,
        output_qty: 1,
    },
];

/// 依 ID 查配方。
pub fn recipe_by_id(id: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.id == id)
}

/// 配方的城鎮繁榮等級門檻（u8，對應 ProsperityLevel::as_u8）。
/// 0 = 無門檻；2 = 需達生機（Thriving）；3 = 需達繁盛（Vibrant）。
pub fn recipe_min_prosperity(recipe_id: &str) -> u8 {
    match recipe_id {
        "town_brew"      => 2,
        "vibrant_elixir" => 3,
        _                => 0,
    }
}

/// 配方的玩家等級門檻（ROADMAP 145 等級路徑）。
/// 0 = 無門檻；否則玩家 level 必須 >= 此值才可合成。
/// 兩條路徑鐵律：有等級門檻的配方，玩家可先攢素材（不需等到達等級），
/// 也可先升等（讓升等本身就有「解鎖新配方」的感覺）——二者任一成立即可合成。
/// 目前實作：level < 門檻時直接拒絕（素材雖齊也不合），符合「成長門禁」的設計。
/// 未來可根據玩家回饋調整為「低等級需多倍素材」的真正雙路路線。
pub fn recipe_min_level(recipe_id: &str) -> u32 {
    match recipe_id {
        "star_crystal_blade" | "star_crystal_armor" => 10,
        "rift_blade"                                => 15,
        "void_cannon"                               => 18,
        _                                           => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::NodeKind;

    #[test]
    fn pickaxe_recipe_requires_wood_and_stone() {
        let mut inv = Inventory::new();
        let p = pickaxe();
        // 料不夠。
        assert!(!p.can_craft(&inv));
        inv.add(ItemKind::Wood, 3);
        assert!(!p.can_craft(&inv));
        // 齊了。
        inv.add(ItemKind::Stone, 2);
        assert!(p.can_craft(&inv));
    }

    #[test]
    fn axe_recipe_requires_wood_and_stone() {
        // ROADMAP 433：斧頭＝木×4＋石×3。料不齊不能合，齊了才行。
        let mut inv = Inventory::new();
        let a = axe();
        assert!(!a.can_craft(&inv));
        inv.add(ItemKind::Wood, 4);
        assert!(!a.can_craft(&inv));
        inv.add(ItemKind::Stone, 3);
        assert!(a.can_craft(&inv));
        // 全域目錄查得到、id 對齊前端。
        assert!(recipe_by_id("axe").is_some(), "斧頭配方應在全域目錄");
    }

    #[test]
    fn axe_craft_consumes_and_yields() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 4);
        inv.add(ItemKind::Stone, 3);
        assert!(axe().craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Axe), 1);
        assert_eq!(inv.count(ItemKind::Wood), 0);
        assert_eq!(inv.count(ItemKind::Stone), 0);
    }

    #[test]
    fn fishing_rod_recipe_requires_wood_and_ether() {
        // ROADMAP 434：釣竿＝木×3＋乙太×2。料不齊不能合，齊了才行。
        let mut inv = Inventory::new();
        let r = fishing_rod();
        assert!(!r.can_craft(&inv));
        inv.add(ItemKind::Wood, 3);
        assert!(!r.can_craft(&inv));
        inv.add(ItemKind::Ether, 2);
        assert!(r.can_craft(&inv));
        // 全域目錄查得到、id 對齊前端。
        assert!(recipe_by_id("fishing_rod").is_some(), "釣竿配方應在全域目錄");
    }

    #[test]
    fn fishing_rod_craft_consumes_and_yields() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        inv.add(ItemKind::Ether, 2);
        assert!(fishing_rod().craft(&mut inv));
        assert_eq!(inv.count(ItemKind::FishingRod), 1);
        assert_eq!(inv.count(ItemKind::Wood), 0);
        assert_eq!(inv.count(ItemKind::Ether), 0);
    }

    #[test]
    fn crafting_consumes_materials_and_yields_output() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 10);
        inv.add(ItemKind::Stone, 10);
        assert!(pickaxe().craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
        assert_eq!(inv.count(ItemKind::Wood), 7);
        assert_eq!(inv.count(ItemKind::Stone), 8);
    }

    #[test]
    fn gathered_materials_flow_into_crafting() {
        // 端到端模擬玩法鏈：採集產出灌進背包 → 合成。鎖住「採集→背包→合成」同一套物品槽。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        inv.add(ItemKind::Stone, 2);
        assert!(pickaxe().craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn reinforced_pickaxe_upgrades_a_crafted_pickaxe() {
        // 端到端配方鏈：先採素材合出鎬子，再投入鎬子＋更多素材升級成強化鎬。
        // 鎖住「工具＋素材→升級工具」這條鏈：升級配方把當素材的鎬子吃掉、產出強化鎬。
        let mut inv = Inventory::new();
        // 先湊第一條鎬子配方的料並合出鎬子。
        inv.add(ItemKind::Wood, 3);
        inv.add(ItemKind::Stone, 2);
        assert!(recipe_by_id("pickaxe").unwrap().craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);

        // 再湊升級配方額外要的木×2、石×4（鎬子已在背包）。
        inv.add(ItemKind::Wood, 2);
        inv.add(ItemKind::Stone, 4);
        let upgrade = recipe_by_id("reinforced_pickaxe").expect("強化鎬配方應存在");
        assert!(upgrade.can_craft(&inv));
        assert!(upgrade.craft(&mut inv));

        // 鎬子被當素材消耗掉、換得一把強化鎬，素材也扣光。
        assert_eq!(inv.count(ItemKind::Pickaxe), 0);
        assert_eq!(inv.count(ItemKind::ReinforcedPickaxe), 1);
        assert_eq!(inv.count(ItemKind::Wood), 0);
        assert_eq!(inv.count(ItemKind::Stone), 0);
    }

    #[test]
    fn reinforced_pickaxe_needs_a_pickaxe_first() {
        // 沒有鎬子（只有散裝素材）湊不齊升級料：全有全無、整筆失敗、不動背包。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 2);
        inv.add(ItemKind::Stone, 4);
        let upgrade = recipe_by_id("reinforced_pickaxe").unwrap();
        assert!(!upgrade.can_craft(&inv));
        assert!(!upgrade.craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Wood), 2);
        assert_eq!(inv.count(ItemKind::Stone), 4);
        assert_eq!(inv.count(ItemKind::ReinforcedPickaxe), 0);
    }

    #[test]
    fn weapon_recipe_crafts_from_gathered_materials() {
        // 端到端武器鏈：礦石與乙太湊齊後合成出武器。鎖住「背包→合成武器」
        // 且武器配方所需素材都拿得到。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Stone, 4);
        inv.add(ItemKind::Ether, 2);
        let weapon = recipe_by_id("weapon").expect("武器配方應存在");
        assert!(weapon.can_craft(&inv));
        assert!(weapon.craft(&mut inv));
        // 素材扣光、得一把武器。
        assert_eq!(inv.count(ItemKind::Stone), 0);
        assert_eq!(inv.count(ItemKind::Ether), 0);
        assert_eq!(inv.count(ItemKind::Weapon), 1);
    }

    #[test]
    fn weapon_recipe_is_all_or_nothing_when_short() {
        // 石夠乙太差一個：全有全無，整筆失敗、礦石原封不動。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Stone, 4);
        inv.add(ItemKind::Ether, 1);
        let weapon = recipe_by_id("weapon").unwrap();
        assert!(!weapon.can_craft(&inv));
        assert!(!weapon.craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Stone), 4);
        assert_eq!(inv.count(ItemKind::Ether), 1);
        assert_eq!(inv.count(ItemKind::Weapon), 0);
    }

    #[test]
    fn healing_potion_requires_wildflower_seeds_and_produces_potion() {
        let mut inv = Inventory::new();
        let recipe = recipe_by_id("healing_potion").expect("活力藥水配方應存在");
        // 種子不夠——湊不了料。
        inv.add(ItemKind::WildflowerSeed, 2);
        assert!(!recipe.can_craft(&inv));
        // 補齊第三個種子——剛好湊齊。
        inv.add(ItemKind::WildflowerSeed, 1);
        assert!(recipe.can_craft(&inv));
        assert!(recipe.craft(&mut inv));
        // 種子全消耗、得一瓶活力藥水。
        assert_eq!(inv.count(ItemKind::WildflowerSeed), 0);
        assert_eq!(inv.count(ItemKind::HealingPotion), 1);
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

    #[test]
    fn recipe_output_is_disjoint_from_its_own_inputs() {
        for r in RECIPES {
            assert!(
                !r.inputs.iter().any(|&(item, _)| item == r.output),
                "配方 `{}` 的產物 {:?} 出現在自己的素材裡——`can_craft` 的產物容量捷徑會失準，\
                 需改用扣素材後的餘量檢查",
                r.id,
                r.output
            );
        }
    }

    /// 防漂移：窮舉所有採集節點種類 → 對應產出的物品，加上其他獲取途徑（挖掘、掉落），當作「可取得物品」的單一真實來源。
    fn obtainable_items() -> std::collections::BTreeSet<ItemKind> {
        use crate::combat::EnemyKind;

        const NODE_KINDS: &[NodeKind] = &[NodeKind::Tree];
        // 窮舉守衛：新增 NodeKind 變體時逼人回來更新。
        for &n in NODE_KINDS {
            match n {
                NodeKind::Tree => {}
            }
        }
        let mut items: std::collections::BTreeSet<ItemKind> =
            NODE_KINDS.iter().map(|&n| ItemKind::from(n)).collect();

        // 加入挖掘可得（C-2/Unified Mining）：Dirt, Stone, Ether
        items.insert(ItemKind::Dirt);
        items.insert(ItemKind::Stone);
        items.insert(ItemKind::Ether);

        // 加入生態域特殊地形挖掘可得（ROADMAP 10-14）：挖對應格掉落。
        items.insert(ItemKind::CrystalShard);
        items.insert(ItemKind::MushroomSpore);
        items.insert(ItemKind::AncientFragment);
        items.insert(ItemKind::DeepSeaPearl);
        items.insert(ItemKind::WildflowerSeed);
        // 加入跨星球特產地形挖掘（ROADMAP 21-25）：挖對應星球格掉落。
        items.insert(ItemKind::JadeShard);
        items.insert(ItemKind::LavaCrystal);
        items.insert(ItemKind::VoidShard);
        items.insert(ItemKind::AetherShard);
        items.insert(ItemKind::OriginShard);

        // 加入釣魚可得（ROADMAP 47）：站水邊垂釣，三種魚依機率上鉤。
        items.insert(ItemKind::FishSmall);
        items.insert(ItemKind::FishStar);
        items.insert(ItemKind::FishDeep);

        // 加入牧場可得（ROADMAP 48）：農田地塊養雞自動產蛋。
        items.insert(ItemKind::Egg);

        // 加入敵人掉落（窮舉守衛：新增 EnemyKind 未納入即編譯失敗）。
        const ENEMY_KINDS: &[EnemyKind] = &[
            EnemyKind::ScrapDrone,
            EnemyKind::EtherWisp,
            EnemyKind::FlutterSprite,
            EnemyKind::MushroomStalker,
            EnemyKind::CrystalGolem,
            EnemyKind::RuneGuardian,
            EnemyKind::CoralCrab,
            EnemyKind::JadeWraith,
            EnemyKind::SteamConstruct,
            EnemyKind::VoidPhantom,
            EnemyKind::AetherSpecter,
            EnemyKind::OriginGuardian,
            EnemyKind::RiftGuardian,
            EnemyKind::EtherOverlord,
        ];
        for &e in ENEMY_KINDS {
            match e {
                EnemyKind::ScrapDrone
                | EnemyKind::EtherWisp
                | EnemyKind::FlutterSprite
                | EnemyKind::MushroomStalker
                | EnemyKind::CrystalGolem
                | EnemyKind::RuneGuardian
                | EnemyKind::CoralCrab
                | EnemyKind::JadeWraith
                | EnemyKind::SteamConstruct
                | EnemyKind::VoidPhantom
                | EnemyKind::AetherSpecter
                | EnemyKind::OriginGuardian
                | EnemyKind::RiftGuardian
                | EnemyKind::EtherOverlord => {}
            }
            items.insert(e.drop_loot().0);
        }

        items
    }

    #[test]
    fn every_recipe_input_is_obtainable() {
        let obtainable = obtainable_items();
        for r in RECIPES {
            for &(item, _) in r.inputs {
                let craftable = RECIPES.iter().any(|other| other.output == item);
                // 農地種植可得（ROADMAP 49）：農田地塊種作物，成熟後收割。
                let farm_croppable = matches!(
                    item,
                    ItemKind::WheatGrain | ItemKind::Carrot | ItemKind::Potato
                );
                // 夜採可得（ROADMAP 50）：夜間採集星晶礦脈。
                let star_crystal_gatherable = item == ItemKind::StarCrystalShard;
                // 流星雨採集可得（ROADMAP 133/134）：天文台完工後流星雨期間採集地面星塵節點。
                let meteor_dust_collectible = item == ItemKind::StarDust || item == ItemKind::RainbowStarDust;
                // 季節性野外採集節點可得（ROADMAP 154）：每季城外 3 節點，各有 3 次採集次數。
                let seasonal_node_collectible = matches!(
                    item,
                    ItemKind::WildFlower | ItemKind::SolarShard | ItemKind::MapleLeaf | ItemKind::IceShard
                );
                // 巢穴 Alpha 擊殺可得（ROADMAP 168）：AttackAlpha 擊殺後殺手獲得 AlphaCrystal。
                let alpha_kill_drop = item == ItemKind::AlphaCrystal;
                // 傳說古 Alpha 擊倒可得（ROADMAP 173）：擊倒世界頭目後殺手得 LegendaryCore。
                let ancient_alpha_kill_drop = item == ItemKind::LegendaryCore;
                assert!(
                    obtainable.contains(&item) || craftable || farm_croppable || star_crystal_gatherable || meteor_dust_collectible || seasonal_node_collectible || alpha_kill_drop || ancient_alpha_kill_drop,
                    "配方 `{}` 需要素材 {:?}，但它既不可採集/挖掘/掉落，也沒有任何\
                     配方產出它——玩家永遠湊不齊料，這是條看得到卻永遠合不出的死配方；請確認該素材\
                     能由世界獲取／合成取得，或為它補上來源",
                    r.id, item
                );
            }
        }
    }

    #[test]
    fn recipe_ids_are_wire_safe_snake_case() {
        for r in RECIPES {
            assert!(!r.id.is_empty(), "配方 id 不可為空");
            assert!(
                r.id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "配方 id `{}` 含非 snake_case 字元（只允許 a-z 0-9 _）",
                r.id
            );
            assert!(
                !r.id.starts_with('_') && !r.id.ends_with('_'),
                "配方 id `{}` 不該以底線開頭或結尾",
                r.id
            );
            let looked_up = recipe_by_id(r.id).expect("自己的 id 應查得到");
            assert_eq!(looked_up.id, r.id, "配方 `{}` 用自身 id 查回的不是自己", r.id);
            assert_eq!(
                looked_up.output, r.output,
                "配方 `{}` 反查到的產物不一致",
                r.id
            );
        }
    }
}
