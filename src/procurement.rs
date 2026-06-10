//! 星際採購令系統（ROADMAP 55）——商人熟練度第十活動路線。
//!
//! 主城新增「星際採購代理人 NPC」（探勘公告欄右方 120px），同時提供 5 張「採購令」。
//! 玩家接取後需前往指定星球採集指定數量的星球特產碎片，返回主城交付代理人即完成。
//!
//! 規則：
//! - 代理人同時提供 5 張採購令（由近到遠、由少到多）。
//! - 接取後有 20 分鐘完成時限（超時自動取消，不受懲罰）。
//! - 完成後有 8 分鐘冷卻，才能接下一張。
//! - 玩家背包有足夠的目標碎片且靠近代理人時，送 DeliverProcurement 即完成交付並發獎。
//! - 碎片從背包扣除（模擬「交給代理人」）。
//! - 只有在故鄉（home planet）才能與代理人互動及接取/交付任務。

use crate::inventory::ItemKind;
use crate::npc::SHOP_REACH;

/// 完成採購令後給商人熟練度的基礎 XP。
pub const PROCUREMENT_BASE_XP: u32 = 25;

/// 接取後的完成時限（秒），20 分鐘。
pub const PROCUREMENT_TIMEOUT: f32 = 1200.0;

/// 完成後的冷卻時間（秒），8 分鐘。
pub const PROCUREMENT_COOLDOWN_SECS: f32 = 480.0;

/// 星際採購代理人 NPC 的世界座標（探勘公告欄右方 120px）。
pub const PROCUREMENT_NPC_X: f32 = 2480.0;
pub const PROCUREMENT_NPC_Y: f32 = 2080.0;

/// 判斷玩家是否在採購代理人 NPC 互動範圍內。
pub fn is_near_procurement_agent(px: f32, py: f32) -> bool {
    let dx = px - PROCUREMENT_NPC_X;
    let dy = py - PROCUREMENT_NPC_Y;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 一張靜態採購令的定義。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcurementOrder {
    pub id: u8,
    /// 中文名稱（前端顯示用）。
    pub name: &'static str,
    /// 目標星球碎片種類。
    pub required_item: ItemKind,
    /// 目標碎片顯示名（前端用）。
    pub item_name: &'static str,
    /// 所需碎片數量。
    pub required_qty: u32,
    /// 乙太獎勵。
    pub reward: u32,
    /// 商人熟練度 XP。
    pub xp: u32,
}

/// 代理人同時提供的 5 張採購令（由近到遠星球、由少到多材料）。
pub const PROCUREMENT_ORDERS: &[ProcurementOrder] = &[
    ProcurementOrder {
        id: 1,
        name: "翠幽星採購令",
        required_item: ItemKind::JadeShard,
        item_name: "翠幽碎片",
        required_qty: 3,
        reward: 28,
        xp: 25,
    },
    ProcurementOrder {
        id: 2,
        name: "赤焰星採購令",
        required_item: ItemKind::LavaCrystal,
        item_name: "熔晶碎片",
        required_qty: 3,
        reward: 36,
        xp: 32,
    },
    ProcurementOrder {
        id: 3,
        name: "虛空星採購令",
        required_item: ItemKind::VoidShard,
        item_name: "虛空碎片",
        required_qty: 2,
        reward: 45,
        xp: 40,
    },
    ProcurementOrder {
        id: 4,
        name: "霧醚星採購令",
        required_item: ItemKind::AetherShard,
        item_name: "霧醚碎片",
        required_qty: 2,
        reward: 56,
        xp: 48,
    },
    ProcurementOrder {
        id: 5,
        name: "星源星採購令",
        required_item: ItemKind::OriginShard,
        item_name: "源晶碎片",
        required_qty: 1,
        reward: 70,
        xp: 58,
    },
];

/// 依 id 查詢靜態採購令。
pub fn find_order(id: u8) -> Option<&'static ProcurementOrder> {
    PROCUREMENT_ORDERS.iter().find(|o| o.id == id)
}

/// 玩家目前接取的採購任務（記憶體前置，重啟清空）。
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveProcurement {
    pub order_id: u8,
    /// 剩餘完成秒數（>0 = 進行中；≤0 = 超時）。
    pub remaining_secs: f32,
}

/// 嘗試接取採購令。
///
/// 失敗條件：已有進行中任務、冷卻中、找不到 order_id。
/// 成功回傳新的 `ActiveProcurement`。
pub fn try_accept(
    order_id: u8,
    active: &Option<ActiveProcurement>,
    cooldown: f32,
) -> Option<ActiveProcurement> {
    if active.is_some() || cooldown > 0.0 {
        return None;
    }
    find_order(order_id)?;
    Some(ActiveProcurement {
        order_id,
        remaining_secs: PROCUREMENT_TIMEOUT,
    })
}

/// 嘗試交付採購令（驗背包數量）。
///
/// - `inventory_qty` 為玩家背包中目標碎片的數量。
/// - 成功條件：有進行中任務、背包碎片 >= required_qty。
/// - 成功回傳 `Some((reward, xp, item, qty_to_consume))`；呼叫端負責消耗物品、清除 active、設冷卻、發獎勵。
pub fn try_deliver(
    active: &Option<ActiveProcurement>,
    inventory_qty: u32,
) -> Option<(u32, u32, ItemKind, u32)> {
    let a = active.as_ref()?;
    let order = find_order(a.order_id)?;
    if inventory_qty < order.required_qty {
        return None;
    }
    Some((order.reward, order.xp, order.required_item, order.required_qty))
}

/// 推進採購計時（每個 game tick 呼叫）。
///
/// - 任務逾時：自動取消（設 None），**不**啟動冷卻（超時放棄不懲罰）。
/// - 冷卻倒數至 0 為止。
pub fn tick(active: &mut Option<ActiveProcurement>, cooldown: &mut f32, dt: f32) {
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

    /// 代理人共有 5 張採購令。
    #[test]
    fn procurement_agent_has_five_orders() {
        assert_eq!(PROCUREMENT_ORDERS.len(), 5);
    }

    /// find_order 能查到正確令資料。
    #[test]
    fn find_order_returns_correct_data() {
        let o = find_order(3).unwrap();
        assert_eq!(o.required_item, ItemKind::VoidShard);
        assert_eq!(o.required_qty, 2);
        assert_eq!(o.reward, 45);
        assert_eq!(o.xp, 40);
    }

    /// 條件全滿足時 try_accept 成功。
    #[test]
    fn try_accept_succeeds_when_idle() {
        let result = try_accept(1, &None, 0.0);
        assert!(result.is_some());
        let a = result.unwrap();
        assert_eq!(a.order_id, 1);
        assert_eq!(a.remaining_secs, PROCUREMENT_TIMEOUT);
    }

    /// 已有進行中任務時 try_accept 失敗。
    #[test]
    fn try_accept_blocked_when_active() {
        let existing = Some(ActiveProcurement { order_id: 1, remaining_secs: 60.0 });
        assert!(try_accept(2, &existing, 0.0).is_none());
    }

    /// 冷卻中時 try_accept 失敗。
    #[test]
    fn try_accept_blocked_on_cooldown() {
        assert!(try_accept(1, &None, 100.0).is_none());
    }

    /// 無效 order_id 時 try_accept 失敗。
    #[test]
    fn try_accept_blocked_on_invalid_id() {
        assert!(try_accept(99, &None, 0.0).is_none());
    }

    /// 無進行中任務時 try_deliver 回傳 None。
    #[test]
    fn try_deliver_fails_no_active() {
        assert!(try_deliver(&None, 5).is_none());
    }

    /// 背包碎片不足時 try_deliver 失敗。
    #[test]
    fn try_deliver_fails_insufficient_items() {
        let active = Some(ActiveProcurement { order_id: 1, remaining_secs: 600.0 }); // 翠幽×3
        assert!(try_deliver(&active, 2).is_none()); // 只有 2 個
    }

    /// 背包碎片恰好足夠時 try_deliver 成功。
    #[test]
    fn try_deliver_succeeds_exact_qty() {
        let active = Some(ActiveProcurement { order_id: 1, remaining_secs: 600.0 }); // 翠幽×3
        let result = try_deliver(&active, 3);
        assert!(result.is_some());
        let (reward, xp, item, qty) = result.unwrap();
        assert_eq!(reward, 28);
        assert_eq!(xp, 25);
        assert_eq!(item, ItemKind::JadeShard);
        assert_eq!(qty, 3);
    }

    /// 背包碎片超過所需時 try_deliver 也成功。
    #[test]
    fn try_deliver_succeeds_with_surplus() {
        let active = Some(ActiveProcurement { order_id: 5, remaining_secs: 600.0 }); // 源晶×1
        let result = try_deliver(&active, 5); // 有 5 個，超過 1
        assert!(result.is_some());
        let (reward, xp, item, qty) = result.unwrap();
        assert_eq!(reward, 70);
        assert_eq!(xp, 58);
        assert_eq!(item, ItemKind::OriginShard);
        assert_eq!(qty, 1); // 只消耗 1 個
    }

    /// tick 正確遞減剩餘秒數。
    #[test]
    fn tick_decrements_remaining() {
        let mut active = Some(ActiveProcurement { order_id: 1, remaining_secs: 60.0 });
        let mut cooldown = 0.0;
        tick(&mut active, &mut cooldown, 10.0);
        assert!((active.as_ref().unwrap().remaining_secs - 50.0).abs() < 0.001);
    }

    /// tick 超時後自動取消任務，且不啟動冷卻。
    #[test]
    fn tick_cancels_on_timeout_without_cooldown() {
        let mut active = Some(ActiveProcurement { order_id: 1, remaining_secs: 1.0 });
        let mut cooldown = 0.0;
        tick(&mut active, &mut cooldown, 5.0);
        assert!(active.is_none());
        assert_eq!(cooldown, 0.0, "超時取消不應啟動冷卻");
    }

    /// tick 冷卻正確遞減且不低於 0。
    #[test]
    fn tick_decrements_cooldown_clamped() {
        let mut active = None;
        let mut cooldown = 3.0;
        tick(&mut active, &mut cooldown, 10.0);
        assert_eq!(cooldown, 0.0);
    }

    /// 採購代理人 NPC 精確位置在互動範圍內。
    #[test]
    fn is_near_procurement_agent_at_npc_pos() {
        assert!(is_near_procurement_agent(PROCUREMENT_NPC_X, PROCUREMENT_NPC_Y));
    }

    /// 遠離 NPC 應在範圍外。
    #[test]
    fn is_near_procurement_agent_far_away_returns_false() {
        assert!(!is_near_procurement_agent(0.0, 0.0));
    }

    /// 五張採購令獎勵與 XP 皆由低到高遞增。
    #[test]
    fn orders_reward_and_xp_increase_with_difficulty() {
        for pair in PROCUREMENT_ORDERS.windows(2) {
            assert!(pair[1].reward > pair[0].reward, "reward 應遞增");
            assert!(pair[1].xp > pair[0].xp, "xp 應遞增");
        }
    }

    /// 所有採購令需要的星球碎片各不相同。
    #[test]
    fn orders_require_distinct_items() {
        let items: Vec<ItemKind> = PROCUREMENT_ORDERS.iter().map(|o| o.required_item).collect();
        for i in 0..items.len() {
            for j in (i + 1)..items.len() {
                assert_ne!(items[i], items[j], "採購令 {} 和 {} 用同一種碎片", i + 1, j + 1);
            }
        }
    }
}
