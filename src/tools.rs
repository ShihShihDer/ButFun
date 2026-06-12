//! 工具效用模型（Phase 1-D 的純邏輯地基）。
//!
//! 合成（1-C）給背包素材一個「去處」——木×3＋石×2 合出鎬子。但鎬子合出來若無用，
//! 玩法鏈就斷在這：素材 → 合成 → ？。1-D 補上那一環的第一條——**身上有鎬子採礦更快、
//! 沒有就慢**——讓「採集 → 合成 → 更快採」閉成第一個完整正回饋圈（PLAN slice 2）。
//!
//! 這層只管「玩家身上有什麼工具、拿來採集能加速多少」，是純資料 + 純函式，無 IO、
//! 不碰 WebSocket／遊戲迴圈，便於自動測試。
//!
//! 採集接線已落地：玩家採集（Phase 1-A `ResourceNode::gather`）時，`ws` 依背包
//! `gather_speed_multiplier` 算出加速倍率 `m`——一次採集動作相當於採 `m` 下，採礦更快；
//! 沒鎬子就用拳頭（`m == 1`）。見 `ws.rs` 的 `Gather` handler。
//!   - 前端：HUD 顯示手上的工具與它對採集的效用。
//!
//! 薄切片刻意**先只做鎬子×採礦一條**（對齊 PLAN slice 2「先只做鎬子×採礦一條」）：
//! 翻土加速（鋤頭，Phase 0-G `Field::till`）等 `ItemKind::Hoe` 與鋤頭配方落地後，再把
//! 這裡擴成帶「動作種類」參數的查表即可——容器與接法不變。倍率走整數 `u32`：接線時把
//! 「一次動作」放大成 `m` 下即可，不引入浮點誤差，也與 `gather` 以整數耐久計次的模型咬合。

use crate::inventory::{Inventory, ItemKind};

/// 玩家用來採集的工具。`Fist` 是沒有合適工具時的退路（只有基礎速度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// 徒手——任何人都能採，但只有基礎速度。
    Fist,
    /// 鎬子（合成產物）：採礦更快。
    Pickaxe,
    /// 強化鎬（以鎬子＋素材升級而成）：採礦比普通鎬子又更快，是工具進程的第二層。
    ReinforcedPickaxe,
}

/// 徒手的基礎採集倍率（單位速度）。沒有鎬子就是這個。
pub const FIST_MULTIPLIER: u32 = 1;

/// 鎬子採集的加速倍率。對應驗收「採礦速度提升 X 倍」的 X：一次採集動作相當於採這麼多下。
pub const PICKAXE_GATHER_MULTIPLIER: u32 = 3;

/// 強化鎬採集的加速倍率：嚴格高於普通鎬子，讓「升級」這條配方鏈真的有感、值得攢素材去合。
pub const REINFORCED_PICKAXE_GATHER_MULTIPLIER: u32 = 5;

impl ToolKind {
    /// 此工具拿來採集的速度倍率。鎬子回 `PICKAXE_GATHER_MULTIPLIER`，徒手回基礎
    /// `FIST_MULTIPLIER`。
    pub fn gather_multiplier(self) -> u32 {
        match self {
            ToolKind::ReinforcedPickaxe => REINFORCED_PICKAXE_GATHER_MULTIPLIER,
            ToolKind::Pickaxe => PICKAXE_GATHER_MULTIPLIER,
            ToolKind::Fist => FIST_MULTIPLIER,
        }
    }
}

/// 某個背包物品若是採集工具，回對應的 `ToolKind`；不是工具（資源原料）回 `None`。
/// 刻意用窮舉 `match`（不寫 `_` 萬用分支）：日後在 `ItemKind` 加新工具變體（如鋤頭）時，
/// 編譯器會強制回來補上它對應的工具，避免漏接。
pub fn tool_from_item(item: ItemKind) -> Option<ToolKind> {
    match item {
        ItemKind::Pickaxe => Some(ToolKind::Pickaxe),
        ItemKind::ReinforcedPickaxe => Some(ToolKind::ReinforcedPickaxe),
        // 資源材料、武器、消耗道具不是採集工具（戰鬥那側由 `combat::weapon_power` 查表）。
        ItemKind::Wood | ItemKind::Dirt | ItemKind::Stone | ItemKind::Ether
        | ItemKind::Weapon | ItemKind::CrystalShard | ItemKind::MushroomSpore
        | ItemKind::AncientFragment | ItemKind::DeepSeaPearl | ItemKind::WildflowerSeed
        | ItemKind::HealingPotion | ItemKind::CrystalPotion | ItemKind::MushroomElixir
        | ItemKind::EtherPill | ItemKind::PearlPotion
        | ItemKind::CrystalBlade | ItemKind::CoralLance
        | ItemKind::MeadowAmulet | ItemKind::CrystalShield | ItemKind::StarChart
        | ItemKind::MushroomStaff | ItemKind::RuneBlade
        | ItemKind::JadeShard | ItemKind::JadeElixir | ItemKind::JadeBlade
        | ItemKind::LavaCrystal | ItemKind::SteamElixir | ItemKind::CrimsonBlade
        | ItemKind::VoidShard | ItemKind::VoidElixir | ItemKind::VoidBlade
        | ItemKind::AetherShard | ItemKind::AetherEssence | ItemKind::AetherBlade
        | ItemKind::OriginShard | ItemKind::OriginEssence | ItemKind::OriginBlade
        | ItemKind::RiftShard | ItemKind::CosmicShield
        | ItemKind::FishSmall | ItemKind::FishStar | ItemKind::FishDeep
        | ItemKind::GrilledFish | ItemKind::StarSashimi | ItemKind::DeepBroth
        | ItemKind::Egg | ItemKind::FriedEgg
        | ItemKind::WheatGrain | ItemKind::Carrot | ItemKind::Potato
        | ItemKind::Bread | ItemKind::CarrotSoup | ItemKind::PotatoGratin
        | ItemKind::StarCrystalShard | ItemKind::NightPotion | ItemKind::Sprinkler
        | ItemKind::TownBrew | ItemKind::VibrantElixir
        | ItemKind::StarDust | ItemKind::StarAmulet
        | ItemKind::RainbowStarDust | ItemKind::StarGuardianAmulet
        | ItemKind::HardenedBlade | ItemKind::StarCrystalBlade | ItemKind::RiftBlade
        | ItemKind::CoralArmor | ItemKind::RuneArmor | ItemKind::StarCrystalArmor
        | ItemKind::EtherBow | ItemKind::CrystalBallista | ItemKind::VoidCannon
        | ItemKind::WildFlower | ItemKind::SolarShard | ItemKind::MapleLeaf | ItemKind::IceShard
        | ItemKind::SpringSachet | ItemKind::SummerElixir | ItemKind::AutumnTonic | ItemKind::WinterMedicine
        | ItemKind::SteamBed | ItemKind::AetherChest | ItemKind::EtherPlant
        | ItemKind::StarLantern | ItemKind::AncientDeco
        | ItemKind::EtherOverlordCore | ItemKind::EtherOverlordBlade => None,
    }
}

/// 玩家背包裡採集最有效的工具：挑出持有工具中採集倍率最高者；都沒有就回 `Fist`。
/// 採集接線據此決定加速倍率。
pub fn best_gather_tool(inv: &Inventory) -> ToolKind {
    inv.entries()
        .filter_map(|(item, _)| tool_from_item(item))
        .max_by_key(|tool| tool.gather_multiplier())
        .unwrap_or(ToolKind::Fist)
}

/// 玩家採集的速度倍率（自動取背包裡最好的工具）。`1`＝徒手基礎速度。
/// 採集接線：一次採集動作相當於連採 `gather_speed_multiplier` 下（有鎬子更快）。
pub fn gather_speed_multiplier(inv: &Inventory) -> u32 {
    best_gather_tool(inv).gather_multiplier()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 編譯期不變式：鎬子一定比徒手快，否則 1-D「採礦更快」這條閉環不成立。
    const _: () = assert!(PICKAXE_GATHER_MULTIPLIER > FIST_MULTIPLIER);

    #[test]
    fn pickaxe_speeds_gathering() {
        assert_eq!(ToolKind::Pickaxe.gather_multiplier(), PICKAXE_GATHER_MULTIPLIER);
    }

    #[test]
    fn fist_is_base_speed() {
        assert_eq!(ToolKind::Fist.gather_multiplier(), FIST_MULTIPLIER);
    }

    #[test]
    fn only_tools_map_from_items() {
        assert_eq!(tool_from_item(ItemKind::Pickaxe), Some(ToolKind::Pickaxe));
        assert_eq!(
            tool_from_item(ItemKind::ReinforcedPickaxe),
            Some(ToolKind::ReinforcedPickaxe)
        );
        // 資源原料不是工具。
        assert_eq!(tool_from_item(ItemKind::Wood), None);
        assert_eq!(tool_from_item(ItemKind::Stone), None);
        assert_eq!(tool_from_item(ItemKind::Ether), None);
    }

    #[test]
    fn empty_inventory_falls_back_to_fist() {
        let inv = Inventory::new();
        assert_eq!(best_gather_tool(&inv), ToolKind::Fist);
        assert_eq!(gather_speed_multiplier(&inv), FIST_MULTIPLIER);
    }

    #[test]
    fn pickaxe_in_inventory_speeds_gathering() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Pickaxe, 1);
        assert_eq!(best_gather_tool(&inv), ToolKind::Pickaxe);
        assert_eq!(gather_speed_multiplier(&inv), PICKAXE_GATHER_MULTIPLIER);
    }

    // 編譯期不變式：強化鎬一定嚴格快過普通鎬子，否則升級配方鏈白費素材。
    const _: () = assert!(REINFORCED_PICKAXE_GATHER_MULTIPLIER > PICKAXE_GATHER_MULTIPLIER);

    #[test]
    fn reinforced_pickaxe_is_faster_than_pickaxe() {
        assert_eq!(
            ToolKind::ReinforcedPickaxe.gather_multiplier(),
            REINFORCED_PICKAXE_GATHER_MULTIPLIER
        );
        assert!(
            ToolKind::ReinforcedPickaxe.gather_multiplier()
                > ToolKind::Pickaxe.gather_multiplier()
        );
    }

    #[test]
    fn best_tool_prefers_reinforced_over_plain_pickaxe() {
        // 背包同時有鎬子與強化鎬：採集自動取最快的強化鎬。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Pickaxe, 1);
        inv.add(ItemKind::ReinforcedPickaxe, 1);
        assert_eq!(best_gather_tool(&inv), ToolKind::ReinforcedPickaxe);
        assert_eq!(
            gather_speed_multiplier(&inv),
            REINFORCED_PICKAXE_GATHER_MULTIPLIER
        );
    }

    #[test]
    fn resources_alone_do_not_speed_gathering() {
        // 背包只有資源、沒工具：採集仍是徒手基礎速度。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 50);
        inv.add(ItemKind::Stone, 50);
        assert_eq!(best_gather_tool(&inv), ToolKind::Fist);
        assert_eq!(gather_speed_multiplier(&inv), FIST_MULTIPLIER);
    }

    #[test]
    fn crafted_pickaxe_then_gathers_faster_end_to_end() {
        // 端到端閉環模擬（PLAN slice 2 的正回饋圈）：採集素材 → 合成鎬子 → 採集變快。
        use crate::crafting::recipe_by_id;
        use crate::gather::NodeKind;

        let mut inv = Inventory::new();
        // 採集前：徒手基礎速度。
        assert_eq!(gather_speed_multiplier(&inv), FIST_MULTIPLIER);

        // 採到木×3、石×2（採集產出灌進背包）。
        for _ in 0..3 {
            inv.add(NodeKind::Tree.into(), 1);
        }
        inv.add(ItemKind::Stone, 1);
        inv.add(ItemKind::Stone, 1);

        // 合出鎬子。
        assert!(recipe_by_id("pickaxe").unwrap().craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);

        // 合成後：採集變快——閉合「素材→合成→更快採」第一個正回饋圈。
        assert_eq!(gather_speed_multiplier(&inv), PICKAXE_GATHER_MULTIPLIER);
    }

    #[test]
    fn every_craftable_tool_is_worth_crafting() {
        // 跨模組不變式（1-C 合成 × 1-D 工具效用的核心承諾）：配方表裡**每一條**會產出
        // 工具的配方，其產物拿來採集都必須**嚴格快過徒手**。這正是 PLAN 給合成定的存在
        // 理由——給素材一個「去處」、閉合「素材→合成→更快採」正回饋圈；一條合出來卻不比
        // 拳頭快的「工具」配方等於讓玩家白花木石，是條死合成。
        //
        // 此前唯一守這條的 `crafted_pickaxe_then_gathers_faster_end_to_end` **寫死鎬子**，
        // 察覺不到第二把工具的回歸。PLAN 自己就指向再加工具（斧／鋤），屆時若忘了給新工具
        // 設加速倍率、或漏在 `tool_from_item` 補上對應映射（產物對不到 `ToolKind`、被當資源、
        // 採集不加速），會變成「合得出來卻沒用」的線上靜默 bug。趁配方表還小，把「合成出的
        // 工具一定值得合」鎖成遍歷整張表的組合測試：日後加工具配方時打錯倍率／漏接映射當場
        // 紅燈，而非接線後玩家才發現新工具沒用。
        use crate::crafting::RECIPES;

        let mut saw_tool_recipe = false;
        for r in RECIPES {
            // 只看「產物是工具」的配方；產出資源／材料的配方（若日後有）不在此不變式內。
            if let Some(tool) = tool_from_item(r.output) {
                saw_tool_recipe = true;
                assert!(
                    tool.gather_multiplier() > FIST_MULTIPLIER,
                    "配方 `{}` 的產物 {:?} 是工具，但採集倍率 {} 沒比徒手 {} 快——合出來卻沒用，\
                     是條死合成；請給它設 > {} 的倍率，或確認 `tool_from_item` 映射正確",
                    r.id,
                    r.output,
                    tool.gather_multiplier(),
                    FIST_MULTIPLIER,
                    FIST_MULTIPLIER
                );
            }
        }
        // 至少要有一條工具配方守住這條鏈——否則整個不變式空轉、悄悄失去意義
        // （例如有人把唯一的鎬子配方刪了還讓本測試綠燈）。
        assert!(
            saw_tool_recipe,
            "配方表裡找不到任何會產出工具的配方——1-C→1-D 的『合成出工具』玩法鏈斷了"
        );
    }

    #[test]
    fn every_tool_item_is_obtainable() {
        // 跨模組不變式（1-B 物品 × 1-C 合成 × 1-D 工具效用），與
        // `every_craftable_tool_is_worth_crafting` **互補的另一個方向**：那條守「配方→產物
        // 採集更快」（每條工具配方都值得合）；這條守「工具→拿得到」（每個工具都有來源）。
        //
        // 兩者是不同的失敗模式：加了新工具配方卻忘了在 `tool_from_item` 補映射 / 設錯倍率，
        // 前者紅燈；但若加了工具 `ItemKind` 變體＋`tool_from_item` 映射、**卻忘了給它配方**，
        // 前者察覺不到——而工具不可採集（`From<NodeKind>` 只把採集節點映成 Wood/Stone/Ether
        // 三種資源，見 `inventory.rs`），合成是唯一取得途徑，少了配方該工具就成為玩家**永遠
        // 拿不到的死物品**。PLAN 自己就指向再加工具（斧／鋤），屆時這正是會踩的坑。趁物品宇宙
        // 還小，把「凡工具必有來源」鎖成遍歷 `ItemKind::ALL` 的組合測試。
        use crate::crafting::RECIPES;
        use crate::inventory::ItemKind;

        for &item in ItemKind::ALL {
            // 只看「是工具」的物品；資源原料（木／石／乙太）靠採集取得，不在此不變式內。
            if tool_from_item(item).is_some() {
                let craftable = RECIPES.iter().any(|r| r.output == item);
                assert!(
                    craftable,
                    "工具物品 {item:?} 沒有任何配方產出它——工具不可採集（gather 只產資源），\
                     合成是唯一來源，少了配方它就是玩家永遠拿不到的死物品；請給它加一條合成\
                     配方，或若改設計成採集／起始道具取得，再更新本不變式",
                );
            }
        }
    }
}
