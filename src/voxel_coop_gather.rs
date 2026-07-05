//! 乙太方界·並肩協作 v1——和其他真人玩家一起採集天然方塊，默契讓收穫更豐（自主提案切片）。
//!
//! **真缺口**：世界至今所有生產活動都是各採各的——兩位真人玩家就算站在同一棵樹旁一起砍，
//! 收穫也跟獨自一人毫無差別。玩家↔玩家這條線至今唯一的互動是漂流瓶（825）：**非同步、
//! 匿名、一次性**（丟瓶時對方不必在場，撿讀也不知道是誰丟的）。本刀補上第一個**即時、
//! 同步**的玩家協作：你挖一塊天然方塊時，若旁邊有其他真人玩家一起忙活，默契讓這塊
//! **多掉一點**——不是新指令、不是特效煙火，是把「並肩」這件事第一次接進核心採集回報。
//!
//! **换維度（非同軸重複）**：與 825 漂流瓶刻意區隔——本刀是**同步**的（需要真的站在附近、
//! 每次挖都即時算），漂流瓶是**非同步**的（丟瓶時對方可能根本不在線）。
//!
//! **舊 2D 精神、voxel 全新實作**：舊 2D 曾有「並肩協作·結伴勞動的默契加成」
//! （`coop_labour.rs`，ROADMAP 414，2D 已封存）——同樣的「湊在一起做同件事、彼此都多得
//! 一點」精神，本刀為 voxel 3D 座標與方塊採集模型重新寫一份全新純函式，非抄舊碼。
//!
//! **只認天然採集方塊、不重複既有加成**：沿用 790 工具加成 [`crate::voxel_tool::block_tool_kind`]
//! 同一張「天然採集得到的原料方塊」適配表（石／礦／原木／泥沙）——單一真相來源、不重立
//! 一份表。石磚／木板／玻璃等加工品不算（避免「擺了再挖」複製加工品）；農作物已有
//! 811~813 時令加成，刻意不疊加，避免同軸重複。
//!
//! **純邏輯層**：`count_partners`（算附近真人玩家數，封頂防刷）＋`coop_yield_bonus`
//! （依同伴數算額外掉落量）＋`coop_eligible_block`（複用工具適配表）＋`coop_toast_line`
//! （回饋句）。全確定性純函式，零 LLM、零鎖、零 async、零 IO，可單元測試。鎖／背包寫入／
//! 廣播在 `voxel_ws.rs`。
//!
//! **成本／護欄鐵律**：零 LLM（純距離與計數）、零 migration（加成材料走既有背包
//! append-only 持久化）、零協議破壞（新增 `coop_bonus` 訊息 additive，舊前端忽略即無影響）、
//! 零新美術（多掉的就是該方塊自身材料）。**濫用防護**：加成只是額外一份既有材料、不觸發
//! LLM／不收玩家自由文字／不開對外端點／不動帳號權限；同伴數由**伺服器讀既有 players
//! map 權威判定**（非客戶端自報）；療癒沙盒、地形無限、資源本就不稀缺，多人站在一起挖礦
//! 是「陪伴的小確幸」，無經濟破壞面（封頂 `MAX_PARTNERS`、加成極小）。

use crate::voxel::Block;
use crate::voxel_tool::block_tool_kind;

/// 並肩協作半徑（世界座標／方塊單位）：與其他真人玩家相距在此之內，才算「一起忙活」。
/// 取 8——比擊掌／問候這類貼身互動寬（勞動各忙各的、不必擠在一起），
/// 但仍需「看得見彼此在同一片地上採集」的近。
pub const COOP_RADIUS: f32 = 8.0;

/// 計入默契的最大同伴數（封頂防刷）：再多人圍著也只算到這麼多。
pub const MAX_PARTNERS: usize = 3;

/// 每位並肩同伴帶來的額外掉落量。
pub const BONUS_QTY_PER_PARTNER: u32 = 1;

/// 這個方塊是否適用並肩協作加成——複用 790 工具加成同一張「天然採集方塊」適配表
/// （石／礦／原木／泥沙），單一真相來源；加工品／農作物一律不適用。
pub fn coop_eligible_block(block: Block) -> bool {
    block_tool_kind(block).is_some()
}

/// 數出「我」身旁在協作半徑內的其他真人玩家數，封頂於 `MAX_PARTNERS`。
///
/// `others` 應是**其他**在線玩家的座標（呼叫端需先排除自己）。以距離平方比較、
/// 免開根號；3D 全向距離（挖礦坑道時上下鄰近也算）。半徑採**含界**（恰好在半徑上也算）。
pub fn count_partners(mine: (f32, f32, f32), others: &[(f32, f32, f32)]) -> usize {
    let r2 = COOP_RADIUS * COOP_RADIUS;
    others
        .iter()
        .filter(|&&(ox, oy, oz)| {
            let dx = ox - mine.0;
            let dy = oy - mine.1;
            let dz = oz - mine.2;
            dx * dx + dy * dy + dz * dz <= r2
        })
        .count()
        .min(MAX_PARTNERS)
}

/// 依並肩同伴數算額外掉落量（已對 `MAX_PARTNERS` 防禦性封頂）。
/// 0 名同伴 → 0（獨自遊玩照舊、零加成、零懲罰）。
pub fn coop_yield_bonus(partners: usize) -> u32 {
    partners.min(MAX_PARTNERS) as u32 * BONUS_QTY_PER_PARTNER
}

/// 並肩協作觸發時的回饋句（呼叫端只在 `bonus > 0` 時使用）。
pub fn coop_toast_line(partners: usize, bonus: u32) -> String {
    format!(
        "🤝 和附近 {} 位旅人並肩採集，默契多收了 {} 份！",
        partners, bonus
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_blocks_match_tool_bonus_table() {
        assert!(coop_eligible_block(Block::Stone));
        assert!(coop_eligible_block(Block::CoalOre));
        assert!(coop_eligible_block(Block::IronOre));
        assert!(coop_eligible_block(Block::AetherOre));
        assert!(coop_eligible_block(Block::Wood));
        assert!(coop_eligible_block(Block::Dirt));
        assert!(coop_eligible_block(Block::Sand));
    }

    #[test]
    fn ineligible_blocks_stay_none() {
        // 加工品：不算天然採集方塊。
        assert!(!coop_eligible_block(Block::StoneBrick));
        assert!(!coop_eligible_block(Block::Plank));
        assert!(!coop_eligible_block(Block::Glass));
        // 農作物：已有時令加成，不重複。
        assert!(!coop_eligible_block(Block::WheatMature));
        assert!(!coop_eligible_block(Block::Grass));
        assert!(!coop_eligible_block(Block::Air));
    }

    #[test]
    fn no_partners_when_alone() {
        assert_eq!(count_partners((0.0, 0.0, 0.0), &[]), 0);
    }

    #[test]
    fn counts_partner_within_radius() {
        let others = [(5.0, 0.0, 0.0)];
        assert_eq!(count_partners((0.0, 0.0, 0.0), &others), 1);
    }

    #[test]
    fn boundary_at_exact_radius_counts() {
        let others = [(COOP_RADIUS, 0.0, 0.0)];
        assert_eq!(count_partners((0.0, 0.0, 0.0), &others), 1);
    }

    #[test]
    fn just_outside_radius_not_counted() {
        let others = [(COOP_RADIUS + 0.01, 0.0, 0.0)];
        assert_eq!(count_partners((0.0, 0.0, 0.0), &others), 0);
    }

    #[test]
    fn caps_at_max_partners() {
        let others = [
            (1.0, 0.0, 0.0),
            (2.0, 0.0, 0.0),
            (3.0, 0.0, 0.0),
            (4.0, 0.0, 0.0),
            (5.0, 0.0, 0.0),
        ];
        assert_eq!(count_partners((0.0, 0.0, 0.0), &others), MAX_PARTNERS);
    }

    #[test]
    fn yield_bonus_scales_with_partners_and_caps() {
        assert_eq!(coop_yield_bonus(0), 0);
        assert_eq!(coop_yield_bonus(1), 1);
        assert_eq!(coop_yield_bonus(2), 2);
        assert_eq!(coop_yield_bonus(3), 3);
        // 防禦性封頂：即使呼叫端傳入超過 MAX_PARTNERS 的數字也不失控。
        assert_eq!(coop_yield_bonus(99), MAX_PARTNERS as u32);
    }

    #[test]
    fn toast_line_mentions_partner_and_bonus_count() {
        let line = coop_toast_line(2, 2);
        assert!(line.contains('2'));
        assert!(line.contains("並肩"));
    }

    #[test]
    fn vertical_distance_counts_too_for_mining_shafts() {
        // 挖礦坑道時上下鄰近也算「並肩」——3D 全向距離，非僅水平。
        let others = [(0.0, 5.0, 0.0)];
        assert_eq!(count_partners((0.0, 0.0, 0.0), &others), 1);
    }
}
