//! 懸賞告示板系統（ROADMAP 53）。
//!
//! 戰士熟練度第八活動路線：在主城懸賞告示板接取狩獵令，
//! 限時獵殺指定數量的怪物，完成後直接得到乙太報酬 + 戰士熟練度 XP。
//!
//! 規則：
//! - 告示板同時提供 5 張懸賞令（由易到難，怪種固定）。
//! - 接取後有 15 分鐘完成時限（超時自動取消，不受懲罰）。
//! - 完成後有 8 分鐘冷卻，才能接下一張。
//! - 擊殺匹配目標怪種即自動計進度，達標自動完成並發放獎勵。
//! - 只有在故鄉（home planet）才能與告示板互動。

use crate::combat::EnemyKind;
use crate::npc::SHOP_REACH;

/// 完成懸賞後給戰士熟練度的 XP（依難度微調）。
pub const BOUNTY_BASE_XP: u32 = 25;

/// 接取後的完成時限（秒），15 分鐘。
pub const BOUNTY_TIMEOUT: f32 = 900.0;

/// 完成後的冷卻時間（秒），8 分鐘。
pub const BOUNTY_COOLDOWN_SECS: f32 = 480.0;

/// 懸賞告示板 NPC 的世界座標（工坊 NPC 右方 120px）。
pub const BOUNTY_NPC_X: f32 = 2240.0;
pub const BOUNTY_NPC_Y: f32 = 2080.0;

/// 判斷玩家是否在懸賞告示板 NPC 互動範圍內。
pub fn is_near_bounty_board(px: f32, py: f32) -> bool {
    let dx = px - BOUNTY_NPC_X;
    let dy = py - BOUNTY_NPC_Y;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 一張靜態懸賞令的定義。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BountyCard {
    pub id: u8,
    /// 中文名稱（前端顯示用）。
    pub name: &'static str,
    /// 目標怪種。
    pub target_kind: EnemyKind,
    /// 所需擊殺數。
    pub required_kills: u32,
    /// 乙太獎勵。
    pub reward: u32,
    /// 戰士熟練度 XP。
    pub xp: u32,
}

/// 告示板同時提供的 5 張懸賞令（由易到難）。
pub const BOUNTY_CARDS: &[BountyCard] = &[
    BountyCard { id: 1, name: "廢鐵清掃令", target_kind: EnemyKind::ScrapDrone,       required_kills: 3, reward: 10, xp: 25 },
    BountyCard { id: 2, name: "精靈驅散令", target_kind: EnemyKind::FlutterSprite,    required_kills: 3, reward: 12, xp: 28 },
    BountyCard { id: 3, name: "蕈菇剿滅令", target_kind: EnemyKind::MushroomStalker,  required_kills: 3, reward: 16, xp: 32 },
    BountyCard { id: 4, name: "傀儡討伐令", target_kind: EnemyKind::CrystalGolem,     required_kills: 2, reward: 20, xp: 38 },
    BountyCard { id: 5, name: "守衛緝拿令", target_kind: EnemyKind::RuneGuardian,     required_kills: 2, reward: 24, xp: 45 },
];

/// 依 id 查詢靜態懸賞令。
pub fn find_card(id: u8) -> Option<&'static BountyCard> {
    BOUNTY_CARDS.iter().find(|c| c.id == id)
}

/// 玩家目前接取的懸賞任務（記憶體前置，重啟清空）。
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveBounty {
    pub card_id: u8,
    /// 已擊殺數量。
    pub kills_done: u32,
    /// 剩餘完成秒數（>0 = 進行中；≤0 = 超時）。
    pub remaining_secs: f32,
}

/// 嘗試接取懸賞令。
///
/// 失敗條件：已有進行中任務、冷卻中、找不到 card_id。
/// 成功回傳新的 `ActiveBounty`。
pub fn try_accept(
    card_id: u8,
    active: &Option<ActiveBounty>,
    cooldown: f32,
) -> Option<ActiveBounty> {
    if active.is_some() || cooldown > 0.0 {
        return None;
    }
    find_card(card_id)?;
    Some(ActiveBounty { card_id, kills_done: 0, remaining_secs: BOUNTY_TIMEOUT })
}

/// 通報一次擊殺，若目標種類匹配則推進進度。
///
/// 回傳 `Some((reward, xp))` 表示此次擊殺導致懸賞完成；否則回傳 `None`。
/// 呼叫端負責：扣除 active、設置冷卻、發獎勵。
pub fn on_kill(
    active: &mut Option<ActiveBounty>,
    kind: EnemyKind,
) -> Option<(u32, u32)> {
    let a = active.as_mut()?;
    let card = find_card(a.card_id)?;
    if card.target_kind != kind {
        return None;
    }
    a.kills_done += 1;
    if a.kills_done >= card.required_kills {
        Some((card.reward, card.xp))
    } else {
        None
    }
}

/// 推進懸賞計時（每個 game tick 呼叫）。
///
/// - 任務逾時：自動取消（設 None），**不**啟動冷卻（超時放棄不懲罰）。
/// - 冷卻倒數至 0 為止。
pub fn tick(active: &mut Option<ActiveBounty>, cooldown: &mut f32, dt: f32) {
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

    /// 告示板共有 5 張懸賞令。
    #[test]
    fn bounty_board_has_five_cards() {
        assert_eq!(BOUNTY_CARDS.len(), 5);
    }

    /// find_card 能查到正確令資料。
    #[test]
    fn find_card_returns_correct_data() {
        let c = find_card(3).unwrap();
        assert_eq!(c.target_kind, EnemyKind::MushroomStalker);
        assert_eq!(c.required_kills, 3);
        assert_eq!(c.reward, 16);
    }

    /// 條件全滿足時 try_accept 成功。
    #[test]
    fn try_accept_succeeds_when_idle() {
        let result = try_accept(1, &None, 0.0);
        assert!(result.is_some());
        let a = result.unwrap();
        assert_eq!(a.card_id, 1);
        assert_eq!(a.kills_done, 0);
        assert_eq!(a.remaining_secs, BOUNTY_TIMEOUT);
    }

    /// 已有進行中任務時 try_accept 失敗。
    #[test]
    fn try_accept_blocked_when_active() {
        let existing = Some(ActiveBounty { card_id: 1, kills_done: 0, remaining_secs: 60.0 });
        assert!(try_accept(2, &existing, 0.0).is_none());
    }

    /// 冷卻中時 try_accept 失敗。
    #[test]
    fn try_accept_blocked_on_cooldown() {
        assert!(try_accept(1, &None, 100.0).is_none());
    }

    /// 無效 card_id 時 try_accept 失敗。
    #[test]
    fn try_accept_blocked_on_invalid_id() {
        assert!(try_accept(99, &None, 0.0).is_none());
    }

    /// 錯誤怪種擊殺不計進度。
    #[test]
    fn on_kill_wrong_kind_no_progress() {
        let mut active = Some(ActiveBounty { card_id: 2, kills_done: 0, remaining_secs: 60.0 });
        // card_id=2 目標是 FlutterSprite，但打了 MushroomStalker
        let result = on_kill(&mut active, EnemyKind::MushroomStalker);
        assert!(result.is_none());
        assert_eq!(active.as_ref().unwrap().kills_done, 0);
    }

    /// 正確怪種擊殺推進進度但未完成。
    #[test]
    fn on_kill_correct_kind_increments_progress() {
        let mut active = Some(ActiveBounty { card_id: 2, kills_done: 0, remaining_secs: 60.0 }); // 需要 3 隻
        let result = on_kill(&mut active, EnemyKind::FlutterSprite);
        assert!(result.is_none(), "還差 2 隻，不應完成");
        assert_eq!(active.as_ref().unwrap().kills_done, 1);
    }

    /// 達到所需擊殺數時 on_kill 回傳獎勵。
    #[test]
    fn on_kill_completes_on_last_kill() {
        let mut active = Some(ActiveBounty { card_id: 2, kills_done: 2, remaining_secs: 60.0 }); // 還差 1 隻
        let result = on_kill(&mut active, EnemyKind::FlutterSprite);
        assert!(result.is_some());
        let (reward, xp) = result.unwrap();
        assert_eq!(reward, 12);
        assert_eq!(xp, 28);
    }

    /// 無進行中任務時 on_kill 回傳 None。
    #[test]
    fn on_kill_none_when_no_active() {
        let mut active = None;
        assert!(on_kill(&mut active, EnemyKind::FlutterSprite).is_none());
    }

    /// tick 正確遞減剩餘秒數。
    #[test]
    fn tick_decrements_remaining() {
        let mut active = Some(ActiveBounty { card_id: 1, kills_done: 0, remaining_secs: 60.0 });
        let mut cooldown = 0.0;
        tick(&mut active, &mut cooldown, 10.0);
        assert!((active.as_ref().unwrap().remaining_secs - 50.0).abs() < 0.001);
    }

    /// tick 超時後自動取消任務，且不啟動冷卻。
    #[test]
    fn tick_cancels_on_timeout_without_cooldown() {
        let mut active = Some(ActiveBounty { card_id: 1, kills_done: 0, remaining_secs: 1.0 });
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

    /// 告示板 NPC 精確位置在互動範圍內。
    #[test]
    fn is_near_bounty_board_at_npc_pos() {
        assert!(is_near_bounty_board(BOUNTY_NPC_X, BOUNTY_NPC_Y));
    }

    /// 遠離告示板 NPC 應在範圍外。
    #[test]
    fn is_near_bounty_board_far_away_returns_false() {
        assert!(!is_near_bounty_board(0.0, 0.0));
    }
}
