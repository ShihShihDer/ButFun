//! 工具效用模型（Phase 1-D 的純邏輯地基）。
//!
//! 合成（1-C）給背包素材一個「去處」——木×3＋石×2 合出鎬子。但鎬子合出來若無用，
//! 玩法鏈就斷在這：素材 → 合成 → ？。1-D 補上那一環的第一條——**身上有鎬子採礦更快、
//! 沒有就慢**——讓「採集 → 合成 → 更快採」閉成第一個完整正回饋圈（PLAN slice 2）。
//!
//! 這層只管「玩家身上有什麼工具、拿來採集能加速多少」，是純資料 + 純函式，無 IO、
//! 不碰 WebSocket／遊戲迴圈，便於自動測試。延續 `crafting.rs` / `inventory.rs` /
//! `gather.rs` 的前置慣例：純邏輯先落地、標 `allow(dead_code)`，接線輪才有呼叫端。
//!
//! 之後接上（接線輪，動 live 廣播屬架構級、由維護者一次一條 land）：
//!   - ws 採集：玩家採集（Phase 1-A `ResourceNode::gather`）時，依背包 `gather_speed_multiplier`
//!     算出加速倍率 `m`——一次採集動作相當於採 `m` 下，採礦更快；沒鎬子就用拳頭（`m == 1`）。
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
}

/// 徒手的基礎採集倍率（單位速度）。沒有鎬子就是這個。
pub const FIST_MULTIPLIER: u32 = 1;

/// 鎬子採集的加速倍率。對應驗收「採礦速度提升 X 倍」的 X：一次採集動作相當於採這麼多下。
pub const PICKAXE_GATHER_MULTIPLIER: u32 = 3;

// 整個模組是前置地基：接線輪（採集依工具加速）才有呼叫端，在此之前公開項目皆無外部
// 呼叫，比照 `crafting.rs` / `inventory.rs` / `gather.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
impl ToolKind {
    /// 此工具拿來採集的速度倍率。鎬子回 `PICKAXE_GATHER_MULTIPLIER`，徒手回基礎
    /// `FIST_MULTIPLIER`。
    pub fn gather_multiplier(self) -> u32 {
        match self {
            ToolKind::Pickaxe => PICKAXE_GATHER_MULTIPLIER,
            ToolKind::Fist => FIST_MULTIPLIER,
        }
    }
}

/// 某個背包物品若是採集工具，回對應的 `ToolKind`；不是工具（資源原料）回 `None`。
/// 刻意用窮舉 `match`（不寫 `_` 萬用分支）：日後在 `ItemKind` 加新工具變體（如鋤頭）時，
/// 編譯器會強制回來補上它對應的工具，避免漏接。
#[allow(dead_code)]
pub fn tool_from_item(item: ItemKind) -> Option<ToolKind> {
    match item {
        ItemKind::Pickaxe => Some(ToolKind::Pickaxe),
        ItemKind::Wood | ItemKind::Stone | ItemKind::Ether => None,
    }
}

/// 玩家背包裡採集最有效的工具：挑出持有工具中採集倍率最高者；都沒有就回 `Fist`。
/// 供採集接線時決定加速倍率。
#[allow(dead_code)]
pub fn best_gather_tool(inv: &Inventory) -> ToolKind {
    inv.entries()
        .filter_map(|(item, _)| tool_from_item(item))
        .max_by_key(|tool| tool.gather_multiplier())
        .unwrap_or(ToolKind::Fist)
}

/// 玩家採集的速度倍率（自動取背包裡最好的工具）。`1`＝徒手基礎速度。
/// 接線時：一次採集動作相當於連採 `gather_speed_multiplier` 下（有鎬子更快）。
#[allow(dead_code)]
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
        inv.add(NodeKind::Rock.into(), 1);
        inv.add(NodeKind::Rock.into(), 1);

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
}
