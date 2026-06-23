//! 世界守護者（ROADMAP 525）：周期性降臨的超強守護者 BOSS。
//!
//! 首次等待 1 小時後，世界守護者現身於荒野遠東（距城鎮約 3.4km）；
//! 玩家需跋涉前往並協力擊破——擊破後全服公告，參與者皆得乙太獎勵；
//! 守護者 4 小時後重生，再度等候新的挑戰者。
//!
//! 成本紀律：純本機邏輯，零 LLM，零 migration，純記憶體，重啟清零。

use uuid::Uuid;

/// 首次等待秒數（伺服器啟動後 1 小時）。
pub const BOSS_FIRST_WAIT_SECS: f32 = 3600.0;
/// 擊敗後重生間隔（4 小時）。
pub const BOSS_RESPAWN_SECS: f32 = 14400.0;
/// 守護者最大 HP。
pub const BOSS_MAX_HP: i32 = 800;
/// 玩家發動攻擊的有效半徑（像素）。
pub const BOSS_REACH: f32 = 100.0;
/// 守護者世界座標 X（荒野遠東，距城鎮 ~3400px）。
pub const BOSS_WX: f32 = 5800.0;
/// 守護者世界座標 Y。
pub const BOSS_WY: f32 = 2400.0;

/// 造成最高傷害者的乙太獎勵。
pub const BOSS_REWARD_TOP: u32 = 60;
/// 所有有效參與者的乙太獎勵（含最高傷害者）。
pub const BOSS_REWARD_PARTICIPANT: u32 = 20;

// ── 參與者紀錄 ─────────────────────────────────────────────────────────────

/// 對守護者造成過傷害的玩家紀錄（純記憶體）。
#[derive(Debug, Clone)]
pub struct BossParticipant {
    pub id:     Uuid,
    pub name:   String,
    pub damage: u32,
}

// ── 事件回傳 ─────────────────────────────────────────────────────────────────

/// `tick` 與 `hit` 回傳的事件。
pub enum BossEvent {
    None,
    /// 守護者現身（此幀首次激活）。
    Spawned,
    /// 守護者被擊敗，附帶參與者列表（id, 名稱, 乙太獎勵）。
    Defeated { rewards: Vec<(Uuid, String, u32)> },
}

// ── 守護者狀態 ────────────────────────────────────────────────────────────────

/// 世界守護者狀態（純記憶體，重啟清零）。
pub struct WorldBossState {
    /// 距下次出現的冷卻（秒）。
    cooldown: f32,
    /// 當前 HP；None 表示守護者不在場。
    hp:           Option<i32>,
    /// 傷害參與紀錄。
    participants: Vec<BossParticipant>,
}

impl WorldBossState {
    pub fn new() -> Self {
        Self {
            cooldown:     BOSS_FIRST_WAIT_SECS,
            hp:           None,
            participants: Vec::new(),
        }
    }

    /// 守護者目前是否在場。
    pub fn is_active(&self) -> bool {
        self.hp.is_some()
    }

    /// 當前 HP（None = 不在場）。
    pub fn current_hp(&self) -> Option<i32> {
        self.hp
    }

    /// 參與人數。
    pub fn participant_count(&self) -> u32 {
        self.participants.len() as u32
    }

    /// 推進時間 dt 秒；回傳是否有重大事件。
    pub fn tick(&mut self, dt: f32) -> BossEvent {
        if self.is_active() {
            return BossEvent::None;
        }
        self.cooldown -= dt;
        if self.cooldown <= 0.0 {
            self.hp = Some(BOSS_MAX_HP);
            self.participants.clear();
            return BossEvent::Spawned;
        }
        BossEvent::None
    }

    /// 玩家打了守護者 `damage` 點傷害。
    /// 回傳 `BossEvent::Defeated { rewards }` 若守護者此擊歸零，否則 `None`。
    pub fn hit(&mut self, player_id: Uuid, name: String, damage: u32) -> BossEvent {
        let hp = match &mut self.hp {
            Some(h) => h,
            None    => return BossEvent::None, // 守護者不在場
        };
        if damage == 0 {
            return BossEvent::None;
        }

        // 累積傷害（同一玩家多次打，疊加）。
        if let Some(p) = self.participants.iter_mut().find(|p| p.id == player_id) {
            p.damage = p.damage.saturating_add(damage);
        } else {
            self.participants.push(BossParticipant { id: player_id, name: name.clone(), damage });
        }

        *hp = hp.saturating_sub(damage as i32);
        if *hp <= 0 {
            self.hp = None;
            self.cooldown = BOSS_RESPAWN_SECS;
            let rewards = self.build_rewards(name);
            self.participants.clear();
            return BossEvent::Defeated { rewards };
        }
        BossEvent::None
    }

    /// 建立獎勵清單（最高傷害者多拿，其餘玩家各得基礎獎勵）。
    fn build_rewards(&self, final_blow_name: String) -> Vec<(Uuid, String, u32)> {
        if self.participants.is_empty() {
            return Vec::new();
        }
        // 找最高傷害者（id 最小者為 tie-break，確保確定性）。
        let top_idx = self.participants.iter().enumerate()
            .max_by(|(_, a), (_, b)| {
                a.damage.cmp(&b.damage).then_with(|| b.id.cmp(&a.id))
            })
            .map(|(i, _)| i)
            .unwrap_or(0);

        self.participants.iter().enumerate().map(|(i, p)| {
            let ether = if i == top_idx {
                BOSS_REWARD_TOP + BOSS_REWARD_PARTICIPANT
            } else {
                BOSS_REWARD_PARTICIPANT
            };
            let _ = &final_blow_name; // final blow 已在廣播層使用，這裡只管分獎
            (p.id, p.name.clone(), ether)
        }).collect()
    }
}

// ── 純函式（可測、可匯出至前端）─────────────────────────────────────────────

/// 玩家座標是否在守護者攻擊範圍內（NaN/Infinity 保守回 false）。
pub fn within_boss_reach(px: f32, py: f32) -> bool {
    if !px.is_finite() || !py.is_finite() {
        return false;
    }
    let dx = px - BOSS_WX;
    let dy = py - BOSS_WY;
    dx * dx + dy * dy <= BOSS_REACH * BOSS_REACH
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_state() -> WorldBossState {
        WorldBossState::new()
    }

    fn active_state() -> WorldBossState {
        let mut s = WorldBossState::new();
        s.cooldown = 0.0;
        s.tick(0.1); // 觸發出現
        s
    }

    // 1. 未到冷卻不現身
    #[test]
    fn boss_does_not_spawn_before_first_wait() {
        let mut s = fresh_state();
        let ev = s.tick(BOSS_FIRST_WAIT_SECS - 1.0);
        assert!(!matches!(ev, BossEvent::Spawned));
        assert!(!s.is_active());
    }

    // 2. 冷卻到期後現身
    #[test]
    fn boss_spawns_after_first_wait() {
        let mut s = fresh_state();
        let ev = s.tick(BOSS_FIRST_WAIT_SECS);
        assert!(matches!(ev, BossEvent::Spawned));
        assert!(s.is_active());
        assert_eq!(s.current_hp(), Some(BOSS_MAX_HP));
    }

    // 3. 受傷後 HP 下降
    #[test]
    fn boss_takes_damage_reduces_hp() {
        let mut s = active_state();
        s.hit(Uuid::new_v4(), "甲".into(), 50);
        assert_eq!(s.current_hp(), Some(BOSS_MAX_HP - 50));
    }

    // 4. 傷害累積至玩家紀錄
    #[test]
    fn participant_damage_accumulates() {
        let mut s = active_state();
        let pid = Uuid::new_v4();
        s.hit(pid, "乙".into(), 30);
        s.hit(pid, "乙".into(), 40);
        let p = s.participants.iter().find(|p| p.id == pid).unwrap();
        assert_eq!(p.damage, 70);
    }

    // 5. HP 歸零後擊敗守護者
    #[test]
    fn boss_defeat_when_hp_zero() {
        let mut s = active_state();
        let pid = Uuid::new_v4();
        let ev = s.hit(pid, "英雄".into(), BOSS_MAX_HP as u32);
        assert!(matches!(ev, BossEvent::Defeated { .. }));
        assert!(!s.is_active());
    }

    // 6. 擊敗後有獎勵
    #[test]
    fn rewards_distributed_on_defeat() {
        let mut s = active_state();
        let pid = Uuid::new_v4();
        if let BossEvent::Defeated { rewards } = s.hit(pid, "英雄".into(), BOSS_MAX_HP as u32) {
            assert_eq!(rewards.len(), 1);
            assert_eq!(rewards[0].0, pid);
            assert_eq!(rewards[0].2, BOSS_REWARD_TOP + BOSS_REWARD_PARTICIPANT);
        } else {
            panic!("期望 Defeated");
        }
    }

    // 7. 最高傷害者獎勵更多
    #[test]
    fn top_damage_player_gets_extra_reward() {
        let mut s = active_state();
        let pid1 = Uuid::new_v4();
        let pid2 = Uuid::new_v4();
        s.hit(pid1, "小".into(), 100);
        if let BossEvent::Defeated { rewards } = s.hit(pid2, "大".into(), BOSS_MAX_HP as u32) {
            let r1 = rewards.iter().find(|r| r.0 == pid1).unwrap();
            let r2 = rewards.iter().find(|r| r.0 == pid2).unwrap();
            assert_eq!(r2.2, BOSS_REWARD_TOP + BOSS_REWARD_PARTICIPANT, "最高傷害者");
            assert_eq!(r1.2, BOSS_REWARD_PARTICIPANT, "參與者");
        } else {
            panic!("期望 Defeated");
        }
    }

    // 8. 擊敗後冷卻重置為 BOSS_RESPAWN_SECS
    #[test]
    fn boss_respawn_cooldown_after_defeat() {
        let mut s = active_state();
        s.hit(Uuid::new_v4(), "英雄".into(), BOSS_MAX_HP as u32);
        assert!(!s.is_active());
        assert!((s.cooldown - BOSS_RESPAWN_SECS).abs() < 1.0);
    }

    // 9. 再次現身（第二輪）
    #[test]
    fn boss_respawns_after_cooldown() {
        let mut s = active_state();
        s.hit(Uuid::new_v4(), "英雄".into(), BOSS_MAX_HP as u32);
        let ev = s.tick(BOSS_RESPAWN_SECS);
        assert!(matches!(ev, BossEvent::Spawned));
        assert!(s.is_active());
    }

    // 10. 不在場時打 hit 靜默無效
    #[test]
    fn hit_when_inactive_returns_none() {
        let mut s = fresh_state();
        let ev = s.hit(Uuid::new_v4(), "英雄".into(), 100);
        assert!(matches!(ev, BossEvent::None));
    }

    // 11. 零傷害靜默忽略
    #[test]
    fn zero_damage_ignored() {
        let mut s = active_state();
        let ev = s.hit(Uuid::new_v4(), "英雄".into(), 0);
        assert!(matches!(ev, BossEvent::None));
        assert_eq!(s.current_hp(), Some(BOSS_MAX_HP));
    }

    // 12. within_boss_reach 正中心
    #[test]
    fn within_reach_center() {
        assert!(within_boss_reach(BOSS_WX, BOSS_WY));
    }

    // 13. within_boss_reach 邊界內
    #[test]
    fn within_reach_edge_inside() {
        assert!(within_boss_reach(BOSS_WX + BOSS_REACH - 1.0, BOSS_WY));
    }

    // 14. within_boss_reach 邊界外
    #[test]
    fn within_reach_edge_outside() {
        assert!(!within_boss_reach(BOSS_WX + BOSS_REACH + 1.0, BOSS_WY));
    }

    // 15. within_boss_reach NaN 保守 false
    #[test]
    fn within_reach_nan_false() {
        assert!(!within_boss_reach(f32::NAN, BOSS_WY));
    }

    // 16. within_boss_reach Infinity 保守 false
    #[test]
    fn within_reach_infinity_false() {
        assert!(!within_boss_reach(f32::INFINITY, BOSS_WY));
    }

    // 17. BOSS_WX/WY 距城鎮夠遠
    #[test]
    fn boss_position_is_far_from_town() {
        // 城鎮大致座標（安全產生點附近）
        let town_x = 2400.0_f32;
        let town_y = 2400.0_f32;
        let dx = BOSS_WX - town_x;
        let dy = BOSS_WY - town_y;
        let dist = (dx * dx + dy * dy).sqrt();
        assert!(dist >= 2000.0, "守護者應距城鎮 ≥ 2000px，實際 {dist:.0}");
    }
}
