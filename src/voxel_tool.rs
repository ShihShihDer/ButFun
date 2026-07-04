//! 乙太方界·工欲善其事：手持對的工具採集，會多收一份 v1（ROADMAP 790）。
//!
//! **缺口**：鎬/斧/鏟自合成台 v1（687/689/690）就能造，但它們至今只在**前端**加快
//! 採礦手感——採集的**產出**完全不看工具：赤手空拳挖一塊石頭，和拿著石鎬挖，掉的東西
//! 一模一樣。工具做出來，除了挖得快一點，沒有任何「值得帶著」的實質回報。這一刀補上
//! 經典的採集進程回饋：**手持「對的」工具採集對應的方塊時，有機率多掉一份材料**——鎬採
//! 石／礦、斧砍原木、鏟挖泥沙，各有適配；工具階級越高（木→石→鐵）多收的機率越大。
//! 採集第一次因為「你帶了對的工具」而真的更有收穫，療癒循環（採集→合成→更好的工具→
//! 採得更多）第一次閉環。
//!
//! **與既有工具加速的分界（互補、非重複）**：687/689/690 讓工具在**前端**加快挖掘手感
//! （純體感、伺服器不參與）；本刀是**伺服器權威**的**產出加成**（多掉一份材料、走既有背包
//! 持久化）。一個管「挖得多快」、一個管「挖得多值」，兩者相加才讓工具真正名副其實。
//!
//! **純邏輯層**：本模組只有確定性純函式——工具階級→加成機率（[`tool_tier`]）、方塊→適配
//! 工具類別（[`block_tool_kind`]）、綜合判定「這次採集多不多掉一份、掉什麼」
//! （[`tool_bonus_drop`]）。零 LLM、零鎖、零 async、零 IO，全可單元測試。鎖 / 背包寫入 /
//! 廣播全在 `voxel_ws.rs`，且沿用既有「葉片機率掉樹苗」（`SAPLING_DROP_CHANCE`）那條
//! 已驗證的 `rand::random::<f32>()` 機率骰慣例。
//!
//! **成本／護欄鐵律**：零 LLM、零 migration（加成材料走既有背包 append-only 持久化）、
//! 零新美術（多掉的就是該方塊自身材料）、零協議破壞（`Break` 的 `tool` 欄位 additive、
//! 舊前端不送即完全無加成、行為與今日一致）。**濫用防護**：加成只是額外一份既有材料、
//! 不觸發 LLM／不收玩家自由文字／不開對外端點／不動帳號權限；工具持有由**伺服器查背包
//! 權威判定**（前端自報手持工具 id，伺服器必須確認背包真有該工具才給加成，防偽報白嫖）；
//! 療癒沙盒、地形無限、資源本就不稀缺，加成純屬「帶對工具的小確幸」，無經濟破壞面。

use crate::voxel::Block;
use crate::voxel_craft::{
    AXE_IRON_ID, AXE_STONE_ID, AXE_WOOD_ID, PICKAXE_IRON_ID, PICKAXE_STONE_ID, PICKAXE_WOOD_ID,
    SHOVEL_IRON_ID, SHOVEL_STONE_ID, SHOVEL_WOOD_ID,
};

/// 工具三大類別——決定它適配哪些方塊。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// 鎬：採石與礦。
    Pickaxe,
    /// 斧：砍原木。
    Axe,
    /// 鏟：挖泥沙。
    Shovel,
}

/// 木階工具的加成機率（20%）。
pub const TIER_WOOD_CHANCE: f32 = 0.20;
/// 石階工具的加成機率（30%）。
pub const TIER_STONE_CHANCE: f32 = 0.30;
/// 鐵階工具的加成機率（45%）——階級越高越值得帶。
pub const TIER_IRON_CHANCE: f32 = 0.45;

/// 由手持工具的物品 id 判定其類別與加成機率。
/// 回傳 `None` 代表手上的不是（會加成的）工具——赤手／持方塊採集無加成，維持今日行為。
pub fn tool_tier(tool_id: u8) -> Option<(ToolKind, f32)> {
    match tool_id {
        // 鎬
        PICKAXE_WOOD_ID => Some((ToolKind::Pickaxe, TIER_WOOD_CHANCE)),
        PICKAXE_STONE_ID => Some((ToolKind::Pickaxe, TIER_STONE_CHANCE)),
        PICKAXE_IRON_ID => Some((ToolKind::Pickaxe, TIER_IRON_CHANCE)),
        // 斧
        AXE_WOOD_ID => Some((ToolKind::Axe, TIER_WOOD_CHANCE)),
        AXE_STONE_ID => Some((ToolKind::Axe, TIER_STONE_CHANCE)),
        AXE_IRON_ID => Some((ToolKind::Axe, TIER_IRON_CHANCE)),
        // 鏟
        SHOVEL_WOOD_ID => Some((ToolKind::Shovel, TIER_WOOD_CHANCE)),
        SHOVEL_STONE_ID => Some((ToolKind::Shovel, TIER_STONE_CHANCE)),
        SHOVEL_IRON_ID => Some((ToolKind::Shovel, TIER_IRON_CHANCE)),
        _ => None,
    }
}

/// 一個方塊「適配哪種工具」——只認**天然採集得到的原料方塊**（石／礦／原木／泥沙），
/// 刻意**不含**任何玩家合成／擺放的加工方塊（石磚、平滑石、木板等），避免「擺了再挖」複製
/// 加工品；也不含農作方塊（那已有自己的收割掉落）。回傳 `None` 代表這方塊無工具加成。
pub fn block_tool_kind(block: Block) -> Option<ToolKind> {
    match block {
        // 鎬：天然石與各種礦脈。
        Block::Stone | Block::CoalOre | Block::IronOre | Block::AetherOre => Some(ToolKind::Pickaxe),
        // 斧：原木（木板是加工品、不列入）。
        Block::Wood => Some(ToolKind::Axe),
        // 鏟：泥土與沙（草地已有胡蘿蔔種子附加掉落、不重複加成）。
        Block::Dirt | Block::Sand => Some(ToolKind::Shovel),
        _ => None,
    }
}

/// 綜合判定：手持 `tool_id` 採集 `block`、擲出 `roll`（∈ [0,1)，由呼叫端 `rand::random::<f32>()`
/// 提供）時，是否多掉一份、掉什麼。
///
/// 規則：手上的工具類別必須與方塊適配的類別一致，且 `roll` 落在該工具階級的機率內，才多掉
/// **一份該方塊自身的材料**（`Some((方塊 id, 1))`）。任何不匹配 / 沒中機率 / 手上非工具 →
/// `None`（無加成、基礎掉落照舊）。
///
/// 純函式：同樣的輸入永遠得同樣的輸出，零副作用、可窮舉測試。
pub fn tool_bonus_drop(tool_id: u8, block: Block, roll: f32) -> Option<(u8, u32)> {
    let (kind, chance) = tool_tier(tool_id)?;
    let need = block_tool_kind(block)?;
    if kind != need {
        return None; // 帶錯工具（鎬挖泥沙、鏟採礦…）無加成。
    }
    if roll < chance {
        Some((block as u8, 1))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_map_to_correct_kind_and_chance() {
        assert_eq!(tool_tier(PICKAXE_WOOD_ID), Some((ToolKind::Pickaxe, TIER_WOOD_CHANCE)));
        assert_eq!(tool_tier(PICKAXE_STONE_ID), Some((ToolKind::Pickaxe, TIER_STONE_CHANCE)));
        assert_eq!(tool_tier(PICKAXE_IRON_ID), Some((ToolKind::Pickaxe, TIER_IRON_CHANCE)));
        assert_eq!(tool_tier(AXE_STONE_ID), Some((ToolKind::Axe, TIER_STONE_CHANCE)));
        assert_eq!(tool_tier(SHOVEL_IRON_ID), Some((ToolKind::Shovel, TIER_IRON_CHANCE)));
        // 非工具（如原木方塊 id、種子）→ 無階級。
        assert_eq!(tool_tier(Block::Wood as u8), None);
        assert_eq!(tool_tier(0), None);
    }

    #[test]
    fn tier_chance_strictly_increases_with_material() {
        assert!(TIER_WOOD_CHANCE < TIER_STONE_CHANCE);
        assert!(TIER_STONE_CHANCE < TIER_IRON_CHANCE);
        // 機率是合法比例。
        for c in [TIER_WOOD_CHANCE, TIER_STONE_CHANCE, TIER_IRON_CHANCE] {
            assert!(c > 0.0 && c < 1.0);
        }
    }

    #[test]
    fn only_natural_raw_blocks_have_a_tool_kind() {
        // 天然原料。
        assert_eq!(block_tool_kind(Block::Stone), Some(ToolKind::Pickaxe));
        assert_eq!(block_tool_kind(Block::CoalOre), Some(ToolKind::Pickaxe));
        assert_eq!(block_tool_kind(Block::IronOre), Some(ToolKind::Pickaxe));
        assert_eq!(block_tool_kind(Block::AetherOre), Some(ToolKind::Pickaxe));
        assert_eq!(block_tool_kind(Block::Wood), Some(ToolKind::Axe));
        assert_eq!(block_tool_kind(Block::Dirt), Some(ToolKind::Shovel));
        assert_eq!(block_tool_kind(Block::Sand), Some(ToolKind::Shovel));
        // 加工品 / 農作 / 雜項不加成（避免擺了再挖複製、或與收割掉落重複）。
        assert_eq!(block_tool_kind(Block::StoneBrick), None);
        assert_eq!(block_tool_kind(Block::SmoothStone), None);
        assert_eq!(block_tool_kind(Block::Plank), None);
        assert_eq!(block_tool_kind(Block::Grass), None); // 已有胡蘿蔔種子附加掉落
        assert_eq!(block_tool_kind(Block::WheatMature), None);
        assert_eq!(block_tool_kind(Block::Glass), None);
    }

    #[test]
    fn matching_tool_and_low_roll_yields_one_bonus_of_the_block() {
        // 石鎬採石、roll 0.0 必中 → 多一顆石頭。
        assert_eq!(
            tool_bonus_drop(PICKAXE_STONE_ID, Block::Stone, 0.0),
            Some((Block::Stone as u8, 1))
        );
        // 石斧砍原木必中 → 多一根原木。
        assert_eq!(
            tool_bonus_drop(AXE_STONE_ID, Block::Wood, 0.0),
            Some((Block::Wood as u8, 1))
        );
        // 鐵鎬採乙太礦必中 → 多一份乙太礦。
        assert_eq!(
            tool_bonus_drop(PICKAXE_IRON_ID, Block::AetherOre, 0.0),
            Some((Block::AetherOre as u8, 1))
        );
    }

    #[test]
    fn roll_at_or_above_chance_gives_no_bonus() {
        // 恰好等於機率門檻（roll == chance）不觸發（嚴格 <）。
        assert_eq!(tool_bonus_drop(PICKAXE_WOOD_ID, Block::Stone, TIER_WOOD_CHANCE), None);
        // 遠高於門檻更不觸發。
        assert_eq!(tool_bonus_drop(PICKAXE_IRON_ID, Block::Stone, 0.99), None);
    }

    #[test]
    fn wrong_tool_for_block_never_bonuses_even_on_zero_roll() {
        // 鎬挖泥土（該用鏟）→ 無加成。
        assert_eq!(tool_bonus_drop(PICKAXE_STONE_ID, Block::Dirt, 0.0), None);
        // 鏟採石（該用鎬）→ 無加成。
        assert_eq!(tool_bonus_drop(SHOVEL_STONE_ID, Block::Stone, 0.0), None);
        // 斧挖沙 → 無加成。
        assert_eq!(tool_bonus_drop(AXE_WOOD_ID, Block::Sand, 0.0), None);
    }

    #[test]
    fn no_tool_in_hand_never_bonuses() {
        // 手持原木方塊（非工具）採石 → 無加成，維持今日赤手行為。
        assert_eq!(tool_bonus_drop(Block::Wood as u8, Block::Stone, 0.0), None);
        // 手持空氣 → 無加成。
        assert_eq!(tool_bonus_drop(Block::Air as u8, Block::Stone, 0.0), None);
    }
}
