//! 星際貿易路線（ROADMAP 51）。
//!
//! 商人熟練度第六活動路線：在不同星球商人之間跑商。
//! 玩家靠近星球商人可接取一個「貿易包裹」任務：把指定商品帶到目標星球商人交付，
//! 換取乙太報酬 + 商人熟練度 XP。
//!
//! 規則：
//! - 一次只能攜帶一個包裹。
//! - 每條路線（route_id）有 5 分鐘接取冷卻，防止同條反覆刷。
//! - 玩家死亡不掉包裹（療癒向設計）。
//! - 可隨時取消任務（丟棄包裹，不受懲罰）。

use std::collections::HashMap;

/// 每次成功交付給商人熟練度的 XP。
pub const TRADE_MERCHANT_XP: u32 = 20;

/// 每條貿易路線接取後的冷卻秒數（5 分鐘）。
pub const TRADE_COOLDOWN_SECS: f32 = 300.0;

/// 一條靜態貿易路線的描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeRoute {
    /// 路線編號（1~N，穩定識別用）。
    pub id: u8,
    /// 接取星球 wire key。
    pub origin: &'static str,
    /// 目標交付星球 wire key。
    pub dest: &'static str,
    /// 商品名稱（中文，前端顯示用）。
    pub cargo_name: &'static str,
    /// 成功交付後的乙太獎勵。
    pub reward: u32,
}

/// 玩家正在攜帶的貿易包裹（記憶體狀態，重啟清空）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeCargo {
    /// 對應路線 id（用於設定接取冷卻）。
    pub route_id: u8,
    /// 接取星球 wire key。
    pub origin: String,
    /// 目標交付星球 wire key。
    pub dest: String,
    /// 商品名稱（前端顯示用）。
    pub cargo_name: String,
    /// 交付後的乙太獎勵。
    pub reward: u32,
}

/// 各條路線接取冷卻的剩餘秒數。Key = route_id，Value = 剩餘秒（0 = 可接取）。
pub type TradeCooldowns = HashMap<u8, f32>;

/// 全部可用貿易路線（靜態定義）。
///
/// 六條路線涵蓋主要星球連線，越遠報酬越高；虛空星路線是最深遠的高報酬選項。
pub const TRADE_ROUTES: &[TradeRoute] = &[
    TradeRoute { id: 1, origin: "home",    dest: "verdant", cargo_name: "故鄉星光水晶",  reward: 15 },
    TradeRoute { id: 2, origin: "verdant", dest: "home",    cargo_name: "翠幽草藥精華",  reward: 12 },
    TradeRoute { id: 3, origin: "home",    dest: "crimson", cargo_name: "冷卻星能結晶",  reward: 18 },
    TradeRoute { id: 4, origin: "crimson", dest: "home",    cargo_name: "熔岩礦石精華",  reward: 15 },
    TradeRoute { id: 5, origin: "crimson", dest: "verdant", cargo_name: "赤焰焦晶",      reward: 22 },
    TradeRoute { id: 6, origin: "void",    dest: "home",    cargo_name: "虛空晶礦精華",  reward: 25 },
];

/// 依星球 wire key 取得在此可接取的所有路線（靜態引用）。
pub fn routes_for_planet(planet: &str) -> Vec<&'static TradeRoute> {
    TRADE_ROUTES.iter().filter(|r| r.origin == planet).collect()
}

/// 依路線 id 查詢路線（靜態引用）。
pub fn find_route(route_id: u8) -> Option<&'static TradeRoute> {
    TRADE_ROUTES.iter().find(|r| r.id == route_id)
}

/// 嘗試接取貿易任務。
///
/// 失敗條件：
/// - `current_cargo` 已有包裹（一次只能一個）
/// - `cooldowns` 中對應 route_id 還有剩餘冷卻
/// - 玩家不在該路線的 origin 星球
/// - 找不到 route_id
///
/// 成功回傳 `TradeCargo`，失敗回傳 `None`。
pub fn try_pickup(
    route_id: u8,
    player_planet: &str,
    current_cargo: &Option<TradeCargo>,
    cooldowns: &TradeCooldowns,
) -> Option<TradeCargo> {
    // 已有包裹
    if current_cargo.is_some() {
        return None;
    }
    // 路線冷卻中
    if cooldowns.get(&route_id).copied().unwrap_or(0.0) > 0.0 {
        return None;
    }
    let route = find_route(route_id)?;
    // 不在接取星球
    if route.origin != player_planet {
        return None;
    }
    Some(TradeCargo {
        route_id,
        origin: route.origin.to_string(),
        dest: route.dest.to_string(),
        cargo_name: route.cargo_name.to_string(),
        reward: route.reward,
    })
}

/// 嘗試交付包裹。
///
/// 成功條件：玩家在目標星球、持有包裹。
/// 成功回傳乙太獎勵；失敗回傳 0（前端按鈕已依此禁用，0 為靜默忽略）。
pub fn try_deliver(player_planet: &str, current_cargo: &Option<TradeCargo>) -> u32 {
    if let Some(cargo) = current_cargo {
        if cargo.dest == player_planet {
            return cargo.reward;
        }
    }
    0
}

/// 推進所有路線冷卻計時（每個 game tick 呼叫，dt = 自上次 tick 的秒數）。
pub fn tick_cooldowns(cooldowns: &mut TradeCooldowns, dt: f32) {
    for v in cooldowns.values_mut() {
        *v = (*v - dt).max(0.0);
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 故鄉有 2 條可接取路線（id=1 往翠幽、id=3 往赤焰）。
    #[test]
    fn routes_for_home_has_two_routes() {
        let routes = routes_for_planet("home");
        assert_eq!(routes.len(), 2);
    }

    /// find_route 能查到正確路線資料。
    #[test]
    fn find_route_returns_correct_data() {
        let r = find_route(1).unwrap();
        assert_eq!(r.origin, "home");
        assert_eq!(r.dest, "verdant");
        assert_eq!(r.reward, 15);
    }

    /// 條件全滿足時 try_pickup 成功。
    #[test]
    fn try_pickup_succeeds_on_valid_conditions() {
        let cargo = try_pickup(1, "home", &None, &TradeCooldowns::new());
        assert!(cargo.is_some());
        let c = cargo.unwrap();
        assert_eq!(c.route_id, 1);
        assert_eq!(c.dest, "verdant");
        assert_eq!(c.reward, 15);
    }

    /// 已有包裹時 try_pickup 失敗。
    #[test]
    fn try_pickup_blocked_when_already_carrying() {
        let existing = Some(TradeCargo {
            route_id: 2, origin: "verdant".into(), dest: "home".into(),
            cargo_name: "test".into(), reward: 12,
        });
        let result = try_pickup(1, "home", &existing, &TradeCooldowns::new());
        assert!(result.is_none());
    }

    /// 路線冷卻中時 try_pickup 失敗。
    #[test]
    fn try_pickup_blocked_on_cooldown() {
        let mut cds = TradeCooldowns::new();
        cds.insert(1, 100.0);
        let result = try_pickup(1, "home", &None, &cds);
        assert!(result.is_none());
    }

    /// 不在接取星球時 try_pickup 失敗。
    #[test]
    fn try_pickup_blocked_wrong_planet() {
        let result = try_pickup(1, "crimson", &None, &TradeCooldowns::new());
        assert!(result.is_none());
    }

    /// 在目標星球且持有包裹時 try_deliver 回傳正確報酬。
    #[test]
    fn try_deliver_returns_reward_on_success() {
        let cargo = Some(TradeCargo {
            route_id: 1, origin: "home".into(), dest: "verdant".into(),
            cargo_name: "test".into(), reward: 15,
        });
        assert_eq!(try_deliver("verdant", &cargo), 15);
    }

    /// 在錯誤星球交付回傳 0。
    #[test]
    fn try_deliver_zero_on_wrong_planet() {
        let cargo = Some(TradeCargo {
            route_id: 1, origin: "home".into(), dest: "verdant".into(),
            cargo_name: "test".into(), reward: 15,
        });
        assert_eq!(try_deliver("crimson", &cargo), 0);
    }

    /// tick_cooldowns 正確遞減。
    #[test]
    fn tick_cooldowns_decrements_values() {
        let mut cds = TradeCooldowns::new();
        cds.insert(3, 10.0);
        tick_cooldowns(&mut cds, 3.5);
        assert!((cds[&3] - 6.5).abs() < 0.001);
    }

    /// tick_cooldowns 不低於 0。
    #[test]
    fn tick_cooldowns_clamps_to_zero() {
        let mut cds = TradeCooldowns::new();
        cds.insert(5, 1.0);
        tick_cooldowns(&mut cds, 100.0);
        assert_eq!(cds[&5], 0.0);
    }

    /// TRADE_COOLDOWN_SECS 在合理範圍（60~600 秒）。
    #[test]
    fn trade_cooldown_is_reasonable() {
        assert!(TRADE_COOLDOWN_SECS >= 60.0 && TRADE_COOLDOWN_SECS <= 600.0);
    }

    /// TRADE_MERCHANT_XP 為正。
    #[test]
    fn trade_merchant_xp_is_positive() {
        assert!(TRADE_MERCHANT_XP > 0);
    }
}
