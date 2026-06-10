//! 工匠工坊訂單系統（ROADMAP 52）。
//!
//! 工匠熟練度第七活動路線：在主城工坊 NPC 接取加急訂單，
//! 帶指定物品交付換取乙太報酬 + 工匠熟練度 XP。
//!
//! 規則：
//! - 工坊同時提供 5 種不同訂單，玩家自選一張接取。
//! - 接取後有 3 分鐘完成時限（超時自動取消，不受懲罰）。
//! - 完成後有 8 分鐘冷卻，才能接下一張。
//! - 只有在故鄉（home planet）才能與工坊 NPC 互動。

use crate::inventory::{Inventory, ItemKind};
use crate::npc::SHOP_REACH;

/// 完成訂單後給工匠熟練度的 XP（基礎值，依訂單難度微調）。
pub const WORKSHOP_BASE_XP: u32 = 20;

/// 接取後的完成時限（秒）。
pub const WORKSHOP_ORDER_TIMEOUT: f32 = 180.0;

/// 完成後的冷卻時間（秒），才能接下一張。
pub const WORKSHOP_COOLDOWN_SECS: f32 = 480.0;

/// 工坊 NPC 的世界座標（主城商人正上方 120px）。
pub const WORKSHOP_NPC_X: f32 = 2120.0;
pub const WORKSHOP_NPC_Y: f32 = 2080.0;

/// 判斷玩家是否在工坊 NPC 互動範圍內。
pub fn is_near_workshop(px: f32, py: f32) -> bool {
    let dx = px - WORKSHOP_NPC_X;
    let dy = py - WORKSHOP_NPC_Y;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 一張靜態工坊訂單的定義。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkshopOrder {
    pub id: u8,
    /// 中文名稱（前端顯示用）。
    pub name: &'static str,
    /// 所需物品種類。
    pub required_item: ItemKind,
    /// 所需數量。
    pub required_qty: u32,
    /// 乙太獎勵。
    pub reward: u32,
    /// 工匠熟練度 XP。
    pub xp: u32,
}

/// 工坊同時提供的 5 種訂單（由易到難）。
pub const WORKSHOP_ORDERS: &[WorkshopOrder] = &[
    WorkshopOrder { id: 1, name: "木材補給",  required_item: ItemKind::Wood,          required_qty: 4, reward:  8, xp: 20 },
    WorkshopOrder { id: 2, name: "石磚訂單",  required_item: ItemKind::Stone,         required_qty: 4, reward: 10, xp: 22 },
    WorkshopOrder { id: 3, name: "礦石急件",  required_item: ItemKind::Ether,         required_qty: 3, reward: 14, xp: 25 },
    WorkshopOrder { id: 4, name: "晶石訂單",  required_item: ItemKind::CrystalShard,  required_qty: 2, reward: 18, xp: 28 },
    WorkshopOrder { id: 5, name: "翠幽急料",  required_item: ItemKind::JadeShard,     required_qty: 1, reward: 24, xp: 35 },
];

/// 依 id 查詢靜態訂單。
pub fn find_order(id: u8) -> Option<&'static WorkshopOrder> {
    WORKSHOP_ORDERS.iter().find(|o| o.id == id)
}

/// 玩家目前接取的工坊訂單（記憶體前置，重啟清空）。
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveOrder {
    pub order_id: u8,
    /// 剩餘完成秒數（>0 = 進行中；≤0 = 超時）。
    pub remaining_secs: f32,
}

/// 嘗試接取工坊訂單。
///
/// 失敗條件：已有進行中訂單、冷卻中、找不到 order_id。
/// 成功回傳新的 `ActiveOrder`。
pub fn try_take(
    order_id: u8,
    active: &Option<ActiveOrder>,
    cooldown: f32,
) -> Option<ActiveOrder> {
    if active.is_some() || cooldown > 0.0 {
        return None;
    }
    find_order(order_id)?;
    Some(ActiveOrder { order_id, remaining_secs: WORKSHOP_ORDER_TIMEOUT })
}

/// 嘗試交付訂單。
///
/// 成功條件：有進行中訂單 + 背包有足夠物品。
/// 成功回傳 `(reward_ether, xp)`；失敗回傳 `None`。
pub fn try_fulfill(
    active: &Option<ActiveOrder>,
    inventory: &Inventory,
) -> Option<(u32, u32)> {
    let a = active.as_ref()?;
    let order = find_order(a.order_id)?;
    if inventory.count(order.required_item) >= order.required_qty {
        Some((order.reward, order.xp))
    } else {
        None
    }
}

/// 推進工坊計時（每個 game tick 呼叫）。
///
/// - 訂單逾時：自動取消（設 None），**不**啟動冷卻（超時放棄不懲罰）。
/// - 冷卻倒數至 0 為止。
pub fn tick(active: &mut Option<ActiveOrder>, cooldown: &mut f32, dt: f32) {
    if let Some(a) = active.as_mut() {
        a.remaining_secs -= dt;
        if a.remaining_secs <= 0.0 {
            *active = None;
        }
    }
    if *cooldown > 0.0 {
        *cooldown = (*cooldown - dt).max(0.0);
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Inventory;

    /// 工坊共有 5 張訂單。
    #[test]
    fn workshop_has_five_orders() {
        assert_eq!(WORKSHOP_ORDERS.len(), 5);
    }

    /// find_order 能查到正確訂單資料。
    #[test]
    fn find_order_returns_correct_data() {
        let o = find_order(1).unwrap();
        assert_eq!(o.required_item, ItemKind::Wood);
        assert_eq!(o.required_qty, 4);
        assert_eq!(o.reward, 8);
    }

    /// 條件全滿足時 try_take 成功。
    #[test]
    fn try_take_succeeds_when_idle() {
        let result = try_take(2, &None, 0.0);
        assert!(result.is_some());
        let a = result.unwrap();
        assert_eq!(a.order_id, 2);
        assert_eq!(a.remaining_secs, WORKSHOP_ORDER_TIMEOUT);
    }

    /// 已有進行中訂單時 try_take 失敗。
    #[test]
    fn try_take_blocked_when_active() {
        let existing = Some(ActiveOrder { order_id: 1, remaining_secs: 60.0 });
        assert!(try_take(2, &existing, 0.0).is_none());
    }

    /// 冷卻中時 try_take 失敗。
    #[test]
    fn try_take_blocked_on_cooldown() {
        assert!(try_take(1, &None, 100.0).is_none());
    }

    /// 找不到 order_id 時 try_take 失敗。
    #[test]
    fn try_take_blocked_on_invalid_id() {
        assert!(try_take(99, &None, 0.0).is_none());
    }

    /// 有訂單且背包夠時 try_fulfill 成功。
    #[test]
    fn try_fulfill_succeeds_with_enough_items() {
        let active = Some(ActiveOrder { order_id: 1, remaining_secs: 60.0 });
        let mut inv = Inventory::default();
        for _ in 0..4 {
            inv.add(ItemKind::Wood, 1);
        }
        let result = try_fulfill(&active, &inv);
        assert!(result.is_some());
        let (reward, xp) = result.unwrap();
        assert_eq!(reward, 8);
        assert_eq!(xp, 20);
    }

    /// 背包不足時 try_fulfill 失敗。
    #[test]
    fn try_fulfill_fails_if_not_enough_items() {
        let active = Some(ActiveOrder { order_id: 1, remaining_secs: 60.0 });
        let mut inv = Inventory::default();
        inv.add(ItemKind::Wood, 2); // 少於 4
        assert!(try_fulfill(&active, &inv).is_none());
    }

    /// 無進行中訂單時 try_fulfill 回傳 None。
    #[test]
    fn try_fulfill_fails_when_no_active() {
        let inv = Inventory::default();
        assert!(try_fulfill(&None, &inv).is_none());
    }

    /// tick 正確遞減剩餘秒數。
    #[test]
    fn tick_decrements_remaining() {
        let mut active = Some(ActiveOrder { order_id: 1, remaining_secs: 60.0 });
        let mut cooldown = 0.0;
        tick(&mut active, &mut cooldown, 10.0);
        assert!((active.as_ref().unwrap().remaining_secs - 50.0).abs() < 0.001);
    }

    /// tick 超時後自動取消訂單，且不啟動冷卻。
    #[test]
    fn tick_cancels_order_on_timeout() {
        let mut active = Some(ActiveOrder { order_id: 1, remaining_secs: 1.0 });
        let mut cooldown = 0.0;
        tick(&mut active, &mut cooldown, 5.0);
        assert!(active.is_none());
        assert_eq!(cooldown, 0.0, "超時取消不應啟動冷卻");
    }

    /// tick 冷卻正確遞減且不低於 0。
    #[test]
    fn tick_decrements_cooldown_to_zero() {
        let mut active = None;
        let mut cooldown = 3.0;
        tick(&mut active, &mut cooldown, 10.0);
        assert_eq!(cooldown, 0.0);
    }

    /// 工坊 NPC 互動範圍：精確在邊界應在範圍內。
    #[test]
    fn is_near_workshop_at_npc_pos() {
        assert!(is_near_workshop(WORKSHOP_NPC_X, WORKSHOP_NPC_Y));
    }

    /// 遠離工坊 NPC 應在範圍外。
    #[test]
    fn is_near_workshop_far_away_returns_false() {
        assert!(!is_near_workshop(0.0, 0.0));
    }
}
