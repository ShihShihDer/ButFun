// ROADMAP 107: 供應鏈 + 純賣商進貨成本——乙太閉迴圈最後一塊
//
// 「純賣商」是只賣不買的 NPC，他的貨來自上游供應商；
// 每次補貨需付乙太進貨成本（乙太排水孔）。
// 讓「純賣商怎麼活」有真實解：販售收入 > 進貨成本 → 有利潤餘裕；
// 進貨成本從商人金庫扣，與商隊收入旋鈕共同構成閉迴圈。
//
// 素材商品（木材、石頭、泥土）是商人收購玩家的原料再轉售，
// 進貨成本為 0（原料路線，由收購市場自然調節）。
// 工具（鎬子、武器）是外部製造的成品，需付進貨成本。

use crate::inventory::ItemKind;

/// 補充一個單位的進貨成本（乙太）。
/// 工具類（精密製造、稀缺）成本占售價約 40%；素材類成本為 0（自行收購轉手）。
pub fn supply_cost_per_unit(item: ItemKind) -> u32 {
    match item {
        ItemKind::Pickaxe => 6,   // 販售 15，進貨 6（40%）
        ItemKind::Weapon  => 10,  // 販售 25，進貨 10（40%）
        _                 => 0,
    }
}

/// 計算一批補貨事件的總進貨成本。
/// `restocked` 是 (ItemKind, 補了幾個) 的列表，由 NpcStockState::tick_restock_with_delta 回傳。
pub fn total_supply_cost(restocked: &[(ItemKind, u32)]) -> u32 {
    restocked.iter()
        .map(|(item, qty)| supply_cost_per_unit(*item).saturating_mul(*qty))
        .fold(0u32, u32::saturating_add)
}

// ─── 純邏輯單元測試 ────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickaxe_has_supply_cost() {
        assert_eq!(supply_cost_per_unit(ItemKind::Pickaxe), 6);
    }

    #[test]
    fn weapon_has_supply_cost() {
        assert_eq!(supply_cost_per_unit(ItemKind::Weapon), 10);
    }

    #[test]
    fn materials_have_zero_cost() {
        // 素材類由商人自行收購轉手，進貨成本為 0。
        assert_eq!(supply_cost_per_unit(ItemKind::Wood),  0);
        assert_eq!(supply_cost_per_unit(ItemKind::Stone), 0);
        assert_eq!(supply_cost_per_unit(ItemKind::Dirt),  0);
        assert_eq!(supply_cost_per_unit(ItemKind::Ether), 0);
    }

    #[test]
    fn total_cost_empty_is_zero() {
        assert_eq!(total_supply_cost(&[]), 0);
    }

    #[test]
    fn total_cost_sums_correctly() {
        // 補 2 把鎬子（×6）+ 1 把武器（×10）= 12 + 10 = 22
        let restocked = [
            (ItemKind::Pickaxe, 2u32),
            (ItemKind::Weapon,  1u32),
        ];
        assert_eq!(total_supply_cost(&restocked), 22);
    }

    #[test]
    fn total_cost_skips_zero_cost_items() {
        let restocked = [
            (ItemKind::Wood,  50u32),
            (ItemKind::Stone, 50u32),
        ];
        assert_eq!(total_supply_cost(&restocked), 0);
    }

    #[test]
    fn total_cost_no_overflow_on_large_qty() {
        // supply_cost_per_unit 最高 10，qty 最大 u32::MAX → saturating_mul 防溢。
        let restocked = [(ItemKind::Weapon, u32::MAX)];
        let cost = total_supply_cost(&restocked);
        assert_eq!(cost, u32::MAX, "saturating_mul 溢位時應飽和為 u32::MAX");
    }

    #[test]
    fn tool_sell_price_exceeds_supply_cost() {
        // 驗收「有利可圖」不變式：工具售價必須高於進貨成本，否則純賣商長期虧損。
        let pickaxe_sell = 15u32;
        let weapon_sell = 25u32;
        assert!(
            pickaxe_sell > supply_cost_per_unit(ItemKind::Pickaxe),
            "鎬子售價應高於進貨成本"
        );
        assert!(
            weapon_sell > supply_cost_per_unit(ItemKind::Weapon),
            "武器售價應高於進貨成本"
        );
    }
}
