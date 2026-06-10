//! 農產品展覽會系統（ROADMAP 56）——農夫熟練度第十一活動路線。
//!
//! 主城新增「農展評審 NPC」（採購代理人右方 120px），同時提供 5 張「展覽委託」。
//! 玩家接取委託後需在 15 分鐘內備齊指定農產品，返回農展評審處交付即完成。
//!
//! 規則：
//! - 評審同時提供 5 張展覽委託（由易到難）。
//! - 接取後有 15 分鐘完成時限（超時自動取消，不受懲罰）。
//! - 完成後有 8 分鐘冷卻，才能接下一張。
//! - 玩家背包有足夠的目標農產品且靠近評審時，送 SubmitFairOrder 即完成交付並發獎。
//! - 物品從背包扣除（視為「提交展品」）。
//! - 只有在故鄉（home planet）才能與農展評審互動。

use crate::inventory::ItemKind;
use crate::npc::SHOP_REACH;

/// 接取後的完成時限（秒），15 分鐘。
pub const FAIR_TIMEOUT: f32 = 900.0;

/// 完成後的冷卻時間（秒），8 分鐘。
pub const FAIR_COOLDOWN_SECS: f32 = 480.0;

/// 農展評審 NPC 的世界座標（採購代理人右方 120px）。
pub const FAIR_NPC_X: f32 = 2600.0;
pub const FAIR_NPC_Y: f32 = 2080.0;

/// 判斷玩家是否在農展評審 NPC 互動範圍內。
pub fn is_near_fair_judge(px: f32, py: f32) -> bool {
    let dx = px - FAIR_NPC_X;
    let dy = py - FAIR_NPC_Y;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 一張展覽委託中單一物品的需求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FairReq {
    pub item: ItemKind,
    /// 前端顯示名（繁中）。
    pub item_name: &'static str,
    pub qty: u32,
}

/// 一張靜態展覽委託的定義。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FairOrder {
    pub id: u8,
    /// 中文名稱（前端顯示用）。
    pub name: &'static str,
    /// 所有需求物品（1~3 種）。
    pub reqs: &'static [FairReq],
    /// 乙太獎勵。
    pub reward: u32,
    /// 農夫熟練度 XP。
    pub xp: u32,
}

/// 評審同時提供的 5 張展覽委託（由易到難）。
pub const FAIR_ORDERS: &[FairOrder] = &[
    FairOrder {
        id: 1,
        name: "農村初展",
        reqs: &[FairReq { item: ItemKind::Egg, item_name: "雞蛋", qty: 4 }],
        reward: 14,
        xp: 20,
    },
    FairOrder {
        id: 2,
        name: "蔬果展示",
        reqs: &[
            FairReq { item: ItemKind::Carrot, item_name: "胡蘿蔔", qty: 3 },
            FairReq { item: ItemKind::WheatGrain, item_name: "小麥穗", qty: 2 },
        ],
        reward: 24,
        xp: 30,
    },
    FairOrder {
        id: 3,
        name: "漁獲特展",
        reqs: &[
            FairReq { item: ItemKind::FishSmall, item_name: "小魚", qty: 4 },
            FairReq { item: ItemKind::FishStar, item_name: "星星魚", qty: 1 },
        ],
        reward: 32,
        xp: 38,
    },
    FairOrder {
        id: 4,
        name: "農漁聯展",
        reqs: &[
            FairReq { item: ItemKind::WheatGrain, item_name: "小麥穗", qty: 4 },
            FairReq { item: ItemKind::Egg, item_name: "雞蛋", qty: 3 },
        ],
        reward: 40,
        xp: 46,
    },
    FairOrder {
        id: 5,
        name: "頂級農展",
        reqs: &[
            FairReq { item: ItemKind::FishDeep, item_name: "深海魚", qty: 1 },
            FairReq { item: ItemKind::Potato, item_name: "馬鈴薯", qty: 3 },
            FairReq { item: ItemKind::FishStar, item_name: "星星魚", qty: 2 },
        ],
        reward: 52,
        xp: 58,
    },
];

/// 依 id 查詢靜態展覽委託。
pub fn find_order(id: u8) -> Option<&'static FairOrder> {
    FAIR_ORDERS.iter().find(|o| o.id == id)
}

/// 玩家目前接取的展覽委託狀態（記憶體前置，重啟清空）。
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveFairOrder {
    pub order_id: u8,
    /// 剩餘完成秒數（>0 = 進行中；≤0 = 超時）。
    pub remaining_secs: f32,
}

/// 嘗試接取展覽委託。
///
/// 失敗條件：已有進行中任務、冷卻中、找不到 order_id。
/// 成功回傳新的 `ActiveFairOrder`。
pub fn try_accept(
    order_id: u8,
    active: &Option<ActiveFairOrder>,
    cooldown: f32,
) -> Option<ActiveFairOrder> {
    if active.is_some() || cooldown > 0.0 {
        return None;
    }
    find_order(order_id)?;
    Some(ActiveFairOrder {
        order_id,
        remaining_secs: FAIR_TIMEOUT,
    })
}

/// 嘗試提交展覽委託。
///
/// `get_qty` 為查詢玩家背包物品數量的閉包。
/// 成功回傳 `Some((reward, xp, vec![(item, qty)...]))`；呼叫端負責消耗物品、清除 active、設冷卻、發獎勵。
/// 失敗（無任務或物品不足）回傳 `None`。
pub fn try_submit(
    active: &Option<ActiveFairOrder>,
    get_qty: impl Fn(ItemKind) -> u32,
) -> Option<(u32, u32, Vec<(ItemKind, u32)>)> {
    let a = active.as_ref()?;
    let order = find_order(a.order_id)?;
    // 逐一確認每種需求物品背包數量足夠
    for req in order.reqs {
        if get_qty(req.item) < req.qty {
            return None;
        }
    }
    let deductions: Vec<(ItemKind, u32)> = order.reqs.iter().map(|r| (r.item, r.qty)).collect();
    Some((order.reward, order.xp, deductions))
}

/// 推進展覽計時（每個 game tick 呼叫）。
///
/// - 任務逾時：自動取消（設 None），**不**啟動冷卻（超時放棄不懲罰）。
/// - 冷卻倒數至 0 為止。
pub fn tick(active: &mut Option<ActiveFairOrder>, cooldown: &mut f32, dt: f32) {
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

    /// 評審共有 5 張展覽委託。
    #[test]
    fn fair_has_five_orders() {
        assert_eq!(FAIR_ORDERS.len(), 5);
    }

    /// find_order 能查到正確令資料。
    #[test]
    fn find_order_returns_correct_data() {
        let o = find_order(3).unwrap();
        assert_eq!(o.name, "漁獲特展");
        assert_eq!(o.reward, 32);
        assert_eq!(o.xp, 38);
    }

    /// 條件全滿足時 try_accept 成功，且 remaining_secs 等於 FAIR_TIMEOUT。
    #[test]
    fn try_accept_succeeds_when_idle() {
        let result = try_accept(1, &None, 0.0);
        assert!(result.is_some());
        let a = result.unwrap();
        assert_eq!(a.order_id, 1);
        assert_eq!(a.remaining_secs, FAIR_TIMEOUT);
    }

    /// 已有進行中任務時 try_accept 失敗。
    #[test]
    fn try_accept_blocked_when_active() {
        let existing = Some(ActiveFairOrder { order_id: 1, remaining_secs: 60.0 });
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

    /// 無進行中任務時 try_submit 回傳 None。
    #[test]
    fn try_submit_fails_no_active() {
        let result = try_submit(&None, |_| 99);
        assert!(result.is_none());
    }

    /// 某物品不足時 try_submit 失敗（農村初展需要雞蛋×4，但只有 3 個）。
    #[test]
    fn try_submit_fails_insufficient_items() {
        let active = Some(ActiveFairOrder { order_id: 1, remaining_secs: 600.0 });
        let result = try_submit(&active, |_| 3); // 只有 3 個雞蛋
        assert!(result.is_none());
    }

    /// 物品恰好足夠時 try_submit 成功，並回傳正確扣除列表。
    #[test]
    fn try_submit_succeeds_exact_qty() {
        let active = Some(ActiveFairOrder { order_id: 1, remaining_secs: 600.0 });
        let result = try_submit(&active, |_| 4); // 恰好 4 個雞蛋
        assert!(result.is_some());
        let (reward, xp, deductions) = result.unwrap();
        assert_eq!(reward, 14);
        assert_eq!(xp, 20);
        assert_eq!(deductions.len(), 1);
        assert_eq!(deductions[0], (ItemKind::Egg, 4));
    }

    /// 多物品委託（蔬果展示）需要全部物品足夠才能提交。
    #[test]
    fn try_submit_multi_req_needs_all_items() {
        let active = Some(ActiveFairOrder { order_id: 2, remaining_secs: 600.0 });
        // 胡蘿蔔足夠，但小麥穗不足
        let result = try_submit(&active, |item| match item {
            ItemKind::Carrot => 5,
            ItemKind::WheatGrain => 1, // 需要 2，只有 1
            _ => 0,
        });
        assert!(result.is_none());
    }

    /// 多物品委託所有物品足夠時 try_submit 成功，回傳兩筆扣除記錄。
    #[test]
    fn try_submit_multi_req_succeeds() {
        let active = Some(ActiveFairOrder { order_id: 2, remaining_secs: 600.0 });
        let result = try_submit(&active, |item| match item {
            ItemKind::Carrot => 3,
            ItemKind::WheatGrain => 2,
            _ => 0,
        });
        assert!(result.is_some());
        let (reward, xp, deductions) = result.unwrap();
        assert_eq!(reward, 24);
        assert_eq!(xp, 30);
        assert_eq!(deductions.len(), 2);
    }

    /// tick 正確遞減剩餘秒數。
    #[test]
    fn tick_decrements_remaining() {
        let mut active = Some(ActiveFairOrder { order_id: 1, remaining_secs: 60.0 });
        let mut cooldown = 0.0;
        tick(&mut active, &mut cooldown, 10.0);
        assert!((active.as_ref().unwrap().remaining_secs - 50.0).abs() < 0.001);
    }

    /// tick 超時後自動取消任務，且不啟動冷卻。
    #[test]
    fn tick_cancels_on_timeout_without_cooldown() {
        let mut active = Some(ActiveFairOrder { order_id: 1, remaining_secs: 1.0 });
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

    /// 農展評審 NPC 精確位置在互動範圍內。
    #[test]
    fn is_near_fair_judge_at_npc_pos() {
        assert!(is_near_fair_judge(FAIR_NPC_X, FAIR_NPC_Y));
    }

    /// 遠離 NPC 應在範圍外。
    #[test]
    fn is_near_fair_judge_far_away_returns_false() {
        assert!(!is_near_fair_judge(0.0, 0.0));
    }

    /// 五張展覽委託獎勵與 XP 皆由低到高遞增。
    #[test]
    fn orders_reward_and_xp_increase_with_difficulty() {
        for pair in FAIR_ORDERS.windows(2) {
            assert!(pair[1].reward > pair[0].reward, "reward 應遞增");
            assert!(pair[1].xp > pair[0].xp, "xp 應遞增");
        }
    }

    /// 頂級農展含三種不同物品需求。
    #[test]
    fn top_order_has_three_requirements() {
        let o = find_order(5).unwrap();
        assert_eq!(o.reqs.len(), 3);
    }
}
