//! 古蹟探勘系統（ROADMAP 54）——探索者熟練度第九活動路線。
//!
//! 主城新增「探勘公告欄 NPC」（懸賞告示板右方 120px），同時提供 5 張「探勘令」。
//! 玩家接取後需前往指定生態域的深處（距主城一定距離以上）採樣，完成後直接得到乙太 +
//! 探索者熟練度 XP。
//!
//! 規則：
//! - 公告欄同時提供 5 張探勘令（草原→水域，距離由近到遠）。
//! - 接取後有 20 分鐘完成時限（超時自動取消，不受懲罰）。
//! - 完成後有 8 分鐘冷卻，才能接下一張。
//! - 玩家到達目標生態域且距主城達 min_dist，送 SurveyExpedition 即完成採樣並立即發獎。
//! - 只有在故鄉（home planet）才能與公告欄互動及接取任務。

use crate::npc::SHOP_REACH;
use world_core::Biome;

/// 完成探勘後給探索者熟練度的基礎 XP。
pub const EXPEDITION_BASE_XP: u32 = 22;

/// 接取後的完成時限（秒），20 分鐘。
pub const EXPEDITION_TIMEOUT: f32 = 1200.0;

/// 完成後的冷卻時間（秒），8 分鐘。
pub const EXPEDITION_COOLDOWN_SECS: f32 = 480.0;

/// 探勘公告欄 NPC 的世界座標（懸賞告示板右方 120px）。
pub const EXPEDITION_NPC_X: f32 = 2360.0;
pub const EXPEDITION_NPC_Y: f32 = 2080.0;

/// 主城中心（格 73,71 → tile_px=32 → 像素 2336, 2272）。
/// 用來計算玩家是否已探索到足夠遠離主城的地方。
pub const HOME_TOWN_X: f32 = 73.0 * 32.0; // 2336.0
pub const HOME_TOWN_Y: f32 = 71.0 * 32.0; // 2272.0

/// 判斷玩家是否在探勘公告欄 NPC 互動範圍內。
pub fn is_near_expedition_board(px: f32, py: f32) -> bool {
    let dx = px - EXPEDITION_NPC_X;
    let dy = py - EXPEDITION_NPC_Y;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 一張靜態探勘令的定義。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpeditionOrder {
    pub id: u8,
    /// 中文名稱（前端顯示用）。
    pub name: &'static str,
    /// 目標生態域。
    pub target_biome: Biome,
    /// 目標生態域顯示名。
    pub biome_name: &'static str,
    /// 距主城中心最低要求距離（像素）。
    pub min_dist: u32,
    /// 乙太獎勵。
    pub reward: u32,
    /// 探索者熟練度 XP。
    pub xp: u32,
}

/// 公告欄同時提供的 5 張探勘令（由近到遠）。
pub const EXPEDITION_ORDERS: &[ExpeditionOrder] = &[
    ExpeditionOrder { id: 1, name: "草原野地調查令", target_biome: Biome::Meadow, biome_name: "草原",  min_dist: 800,  reward: 14, xp: 22 },
    ExpeditionOrder { id: 2, name: "森林深處探採令", target_biome: Biome::Forest, biome_name: "森林",  min_dist: 1100, reward: 18, xp: 28 },
    ExpeditionOrder { id: 3, name: "岩地深層探勘令", target_biome: Biome::Rocky,  biome_name: "岩地",  min_dist: 1400, reward: 23, xp: 35 },
    ExpeditionOrder { id: 4, name: "沙漠荒原考察令", target_biome: Biome::Sand,   biome_name: "沙漠",  min_dist: 1700, reward: 28, xp: 42 },
    ExpeditionOrder { id: 5, name: "深水域探勘令",   target_biome: Biome::Water,  biome_name: "水域",  min_dist: 2000, reward: 35, xp: 52 },
];

/// 依 id 查詢靜態探勘令。
pub fn find_order(id: u8) -> Option<&'static ExpeditionOrder> {
    EXPEDITION_ORDERS.iter().find(|o| o.id == id)
}

/// 玩家目前接取的探勘任務（記憶體前置，重啟清空）。
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveExpedition {
    pub order_id: u8,
    /// 剩餘完成秒數（>0 = 進行中；≤0 = 超時）。
    pub remaining_secs: f32,
}

/// 嘗試接取探勘令。
///
/// 失敗條件：已有進行中任務、冷卻中、找不到 order_id。
/// 成功回傳新的 `ActiveExpedition`。
pub fn try_accept(
    order_id: u8,
    active: &Option<ActiveExpedition>,
    cooldown: f32,
) -> Option<ActiveExpedition> {
    if active.is_some() || cooldown > 0.0 {
        return None;
    }
    find_order(order_id)?;
    Some(ActiveExpedition { order_id, remaining_secs: EXPEDITION_TIMEOUT })
}

/// 嘗試在當前位置採樣（驗生態域 + 距主城距離）。
///
/// 對外 API：讀取 `world_core::biome_at` 決定生態域，內部委派給 `try_survey_with_biome`。
pub fn try_survey(
    active: &Option<ActiveExpedition>,
    px: f32,
    py: f32,
) -> Option<(u32, u32)> {
    let biome = world_core::biome_at(px as f64, py as f64);
    try_survey_with_biome(active, px, py, biome)
}

/// 採樣核心邏輯（接受明確的 biome 參數，方便單元測試）。
///
/// 成功條件：有進行中任務、生態域符合、距主城 >= min_dist。
/// 成功回傳 `Some((reward, xp))`；呼叫端負責清除 active、設冷卻、發獎勵。
pub fn try_survey_with_biome(
    active: &Option<ActiveExpedition>,
    px: f32,
    py: f32,
    biome: Biome,
) -> Option<(u32, u32)> {
    let a = active.as_ref()?;
    let order = find_order(a.order_id)?;
    if biome != order.target_biome {
        return None;
    }
    let dx = px - HOME_TOWN_X;
    let dy = py - HOME_TOWN_Y;
    let dist_sq = dx * dx + dy * dy;
    let min_dist = order.min_dist as f32;
    if dist_sq < min_dist * min_dist {
        return None;
    }
    Some((order.reward, order.xp))
}

/// 推進探勘計時（每個 game tick 呼叫）。
///
/// - 任務逾時：自動取消（設 None），**不**啟動冷卻（超時放棄不懲罰）。
/// - 冷卻倒數至 0 為止。
pub fn tick(active: &mut Option<ActiveExpedition>, cooldown: &mut f32, dt: f32) {
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

    /// 公告欄共有 5 張探勘令。
    #[test]
    fn expedition_board_has_five_orders() {
        assert_eq!(EXPEDITION_ORDERS.len(), 5);
    }

    /// find_order 能查到正確令資料。
    #[test]
    fn find_order_returns_correct_data() {
        let o = find_order(3).unwrap();
        assert_eq!(o.target_biome, Biome::Rocky);
        assert_eq!(o.min_dist, 1400);
        assert_eq!(o.reward, 23);
        assert_eq!(o.xp, 35);
    }

    /// 條件全滿足時 try_accept 成功。
    #[test]
    fn try_accept_succeeds_when_idle() {
        let result = try_accept(1, &None, 0.0);
        assert!(result.is_some());
        let a = result.unwrap();
        assert_eq!(a.order_id, 1);
        assert_eq!(a.remaining_secs, EXPEDITION_TIMEOUT);
    }

    /// 已有進行中任務時 try_accept 失敗。
    #[test]
    fn try_accept_blocked_when_active() {
        let existing = Some(ActiveExpedition { order_id: 1, remaining_secs: 60.0 });
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

    /// 無進行中任務時 try_survey_with_biome 回傳 None。
    #[test]
    fn try_survey_fails_no_active() {
        let result = try_survey_with_biome(&None, HOME_TOWN_X + 2000.0, HOME_TOWN_Y, Biome::Meadow);
        assert!(result.is_none());
    }

    /// 生態域正確、距離足夠時採樣成功。
    #[test]
    fn try_survey_succeeds_correct_biome_far_enough() {
        let active = Some(ActiveExpedition { order_id: 1, remaining_secs: 600.0 }); // 草原令，min_dist=800
        let px = HOME_TOWN_X + 900.0; // 距主城 900px > 800，符合
        let py = HOME_TOWN_Y;
        let result = try_survey_with_biome(&active, px, py, Biome::Meadow);
        assert!(result.is_some());
        let (reward, xp) = result.unwrap();
        assert_eq!(reward, 14);
        assert_eq!(xp, 22);
    }

    /// 生態域不符時採樣失敗。
    #[test]
    fn try_survey_fails_wrong_biome() {
        let active = Some(ActiveExpedition { order_id: 1, remaining_secs: 600.0 }); // 草原令
        let px = HOME_TOWN_X + 900.0;
        let py = HOME_TOWN_Y;
        // 帶入 Forest 而非 Meadow
        let result = try_survey_with_biome(&active, px, py, Biome::Forest);
        assert!(result.is_none());
    }

    /// 距主城不足時採樣失敗。
    #[test]
    fn try_survey_fails_too_close_to_town() {
        let active = Some(ActiveExpedition { order_id: 1, remaining_secs: 600.0 }); // 草原令，min_dist=800
        let px = HOME_TOWN_X + 500.0; // 500px < 800，不符
        let py = HOME_TOWN_Y;
        let result = try_survey_with_biome(&active, px, py, Biome::Meadow);
        assert!(result.is_none());
    }

    /// 不同探勘令各自有正確獎勵。
    #[test]
    fn try_survey_deeper_orders_give_more_reward() {
        for order in EXPEDITION_ORDERS {
            let active = Some(ActiveExpedition { order_id: order.id, remaining_secs: 600.0 });
            let dist = order.min_dist as f32 + 200.0;
            let result = try_survey_with_biome(&active, HOME_TOWN_X + dist, HOME_TOWN_Y, order.target_biome);
            let (reward, xp) = result.expect("應採樣成功");
            assert_eq!(reward, order.reward);
            assert_eq!(xp, order.xp);
        }
    }

    /// 恰在 min_dist 邊界內（差 1px）採樣失敗。
    #[test]
    fn try_survey_fails_at_boundary_inside() {
        let active = Some(ActiveExpedition { order_id: 2, remaining_secs: 600.0 }); // 森林令，min_dist=1100
        // 恰好比 min_dist 少一點點（水平移動）
        let px = HOME_TOWN_X + 1099.0;
        let py = HOME_TOWN_Y;
        let result = try_survey_with_biome(&active, px, py, Biome::Forest);
        assert!(result.is_none());
    }

    /// tick 正確遞減剩餘秒數。
    #[test]
    fn tick_decrements_remaining() {
        let mut active = Some(ActiveExpedition { order_id: 1, remaining_secs: 60.0 });
        let mut cooldown = 0.0;
        tick(&mut active, &mut cooldown, 10.0);
        assert!((active.as_ref().unwrap().remaining_secs - 50.0).abs() < 0.001);
    }

    /// tick 超時後自動取消任務，且不啟動冷卻。
    #[test]
    fn tick_cancels_on_timeout_without_cooldown() {
        let mut active = Some(ActiveExpedition { order_id: 1, remaining_secs: 1.0 });
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

    /// 探勘公告欄 NPC 精確位置在互動範圍內。
    #[test]
    fn is_near_expedition_board_at_npc_pos() {
        assert!(is_near_expedition_board(EXPEDITION_NPC_X, EXPEDITION_NPC_Y));
    }

    /// 遠離 NPC 應在範圍外。
    #[test]
    fn is_near_expedition_board_far_away_returns_false() {
        assert!(!is_near_expedition_board(0.0, 0.0));
    }

    /// 五張探勘令獎勵與 XP 皆由易到難遞增。
    #[test]
    fn orders_reward_and_xp_increase_with_difficulty() {
        for pair in EXPEDITION_ORDERS.windows(2) {
            assert!(pair[1].reward > pair[0].reward, "reward 應遞增");
            assert!(pair[1].xp > pair[0].xp, "xp 應遞增");
            assert!(pair[1].min_dist > pair[0].min_dist, "min_dist 應遞增");
        }
    }
}
