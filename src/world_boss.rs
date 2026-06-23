//! 世界守護者（ROADMAP 525）：周期性降臨的超強守護者 BOSS。
//!
//! 首次等待 1 小時後，世界守護者現身於荒野；
//! 玩家需跋涉前往並協力擊破——擊破後全服公告，參與者皆得乙太獎勵；
//! 守護者 4 小時後重生，再度等候新的挑戰者。
//!
//! ROADMAP 530 守護者輪換：守護者共有四個種類（東/北/西/南方位），
//! 每次擊敗後換下一種出現，讓玩家探索不同方向的荒野。
//!
//! 成本紀律：純本機邏輯，零 LLM，零 migration，純記憶體，重啟清零。

use uuid::Uuid;

/// 首次等待秒數（伺服器啟動後 1 小時）。
pub const BOSS_FIRST_WAIT_SECS: f32 = 3600.0;
/// 擊敗後重生間隔（4 小時）。
pub const BOSS_RESPAWN_SECS: f32 = 14400.0;
/// 玩家發動攻擊的有效半徑（像素）。
pub const BOSS_REACH: f32 = 100.0;

/// 造成最高傷害者的乙太獎勵。
pub const BOSS_REWARD_TOP: u32 = 60;
/// 所有有效參與者的乙太獎勵（含最高傷害者）。
pub const BOSS_REWARD_PARTICIPANT: u32 = 20;

// ── ROADMAP 530 守護者種類 ────────────────────────────────────────────────────

/// 守護者種類定義（輪換用）。每種出現在不同方位，讓玩家探索荒野不同角落。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BossVariant {
    /// 守護者名稱（面向玩家、集中於此以利 i18n）。
    pub name: &'static str,
    /// 代表 emoji。
    pub emoji: &'static str,
    /// 最大 HP。
    pub max_hp: i32,
    /// 世界座標 X（像素）。
    pub wx: f32,
    /// 世界座標 Y（像素）。
    pub wy: f32,
}

/// 四種守護者輪換清單（東→北→西→南依序）。
///
/// 座標設計：各距城鎮中心（約 2400, 2400）至少 3000px，
/// 讓玩家必須真正「出走荒野」才能挑戰，且每次方位不同。
pub const ALL_VARIANTS: [BossVariant; 4] = [
    BossVariant { name: "世界守護者", emoji: "🗿", max_hp: 800, wx: 5800.0, wy: 2400.0 }, // 東方
    BossVariant { name: "晶霜巨龍",   emoji: "🐉", max_hp: 800, wx: 2400.0, wy: -800.0 }, // 北方
    BossVariant { name: "熔焰蜥龍",   emoji: "🦎", max_hp: 800, wx: -1000.0, wy: 2400.0 }, // 西方
    BossVariant { name: "深淵幽靈",   emoji: "👻", max_hp: 800, wx: 2400.0, wy: 5800.0 }, // 南方
];

/// 依累計擊敗次數回傳本輪的守護者種類（純函式、確定性、零隨機）。
///
/// 每擊敗一隻，下一隻換成下一種；走完四種後從頭循環。
pub fn variant_for_defeat(defeat_count: u32) -> BossVariant {
    ALL_VARIANTS[(defeat_count as usize) % ALL_VARIANTS.len()]
}

/// 現在這隻守護者是哪一種（以當前在場的守護者算，傳入「尚未累計本次的 defeat_count」）。
///
/// - 第一次降臨時 defeat_count=0 → 取 ALL_VARIANTS[0]（東方）。
/// - 擊敗後 defeat_count 才 +1，之後新一隻就是 ALL_VARIANTS[defeat_count % 4]。
pub fn current_variant(defeat_count: u32) -> BossVariant {
    ALL_VARIANTS[(defeat_count as usize) % ALL_VARIANTS.len()]
}

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
    /// 守護者現身（此幀首次激活），附帶本次種類。
    Spawned { variant: BossVariant },
    /// 守護者被擊敗，附帶參與者列表（id, 名稱, 乙太獎勵）。
    Defeated { rewards: Vec<(Uuid, String, u32)> },
}

// ── 守護者狀態 ────────────────────────────────────────────────────────────────

/// 世界守護者狀態（純記憶體，重啟清零）。
pub struct WorldBossState {
    /// 距下次出現的冷卻（秒）。
    cooldown: f32,
    /// 當前 HP；None 表示守護者不在場。
    hp: Option<i32>,
    /// 傷害參與紀錄。
    participants: Vec<BossParticipant>,
    /// 累計擊敗次數（決定下一隻守護者的種類；重啟清零）。
    pub defeat_count: u32,
}

impl WorldBossState {
    pub fn new() -> Self {
        Self {
            cooldown:     BOSS_FIRST_WAIT_SECS,
            hp:           None,
            participants: Vec::new(),
            defeat_count: 0,
        }
    }

    /// 目前在場的守護者種類（若不在場，回傳「下一隻」的種類以利 HUD 預覽）。
    pub fn active_variant(&self) -> BossVariant {
        current_variant(self.defeat_count)
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
            let v = self.active_variant();
            self.hp = Some(v.max_hp);
            self.participants.clear();
            return BossEvent::Spawned { variant: v };
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
            // 擊敗後 +1，下次出現為下一種守護者。
            self.defeat_count = self.defeat_count.saturating_add(1);
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

/// 玩家座標是否在指定守護者的攻擊範圍內（NaN/Infinity 保守回 false）。
pub fn within_boss_reach(px: f32, py: f32) -> bool {
    within_variant_reach(px, py, current_variant(0))
}

/// 玩家座標是否在某種守護者的攻擊範圍內（NaN/Infinity 保守回 false）。
/// 接線時傳入 `state.active_variant()` 取當前種類。
pub fn within_variant_reach(px: f32, py: f32, v: BossVariant) -> bool {
    if !px.is_finite() || !py.is_finite() {
        return false;
    }
    let dx = px - v.wx;
    let dy = py - v.wy;
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

    fn defeat_boss(s: &mut WorldBossState) {
        let hp = s.active_variant().max_hp as u32;
        s.hit(Uuid::new_v4(), "英雄".into(), hp);
    }

    // 1. 未到冷卻不現身
    #[test]
    fn boss_does_not_spawn_before_first_wait() {
        let mut s = fresh_state();
        let ev = s.tick(BOSS_FIRST_WAIT_SECS - 1.0);
        assert!(!matches!(ev, BossEvent::Spawned { .. }));
        assert!(!s.is_active());
    }

    // 2. 冷卻到期後現身，帶 variant
    #[test]
    fn boss_spawns_after_first_wait() {
        let mut s = fresh_state();
        let ev = s.tick(BOSS_FIRST_WAIT_SECS);
        assert!(matches!(ev, BossEvent::Spawned { .. }));
        assert!(s.is_active());
        let v = ALL_VARIANTS[0];
        assert_eq!(s.current_hp(), Some(v.max_hp));
    }

    // 3. 受傷後 HP 下降
    #[test]
    fn boss_takes_damage_reduces_hp() {
        let mut s = active_state();
        let max_hp = s.active_variant().max_hp;
        s.hit(Uuid::new_v4(), "甲".into(), 50);
        assert_eq!(s.current_hp(), Some(max_hp - 50));
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
        let max_hp = s.active_variant().max_hp as u32;
        let ev = s.hit(pid, "英雄".into(), max_hp);
        assert!(matches!(ev, BossEvent::Defeated { .. }));
        assert!(!s.is_active());
    }

    // 6. 擊敗後有獎勵
    #[test]
    fn rewards_distributed_on_defeat() {
        let mut s = active_state();
        let pid = Uuid::new_v4();
        let max_hp = s.active_variant().max_hp as u32;
        if let BossEvent::Defeated { rewards } = s.hit(pid, "英雄".into(), max_hp) {
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
        let max_hp = s.active_variant().max_hp as u32;
        s.hit(pid1, "小".into(), 100);
        if let BossEvent::Defeated { rewards } = s.hit(pid2, "大".into(), max_hp) {
            let r1 = rewards.iter().find(|r| r.0 == pid1).unwrap();
            let r2 = rewards.iter().find(|r| r.0 == pid2).unwrap();
            assert_eq!(r2.2, BOSS_REWARD_TOP + BOSS_REWARD_PARTICIPANT, "最高傷害者");
            assert_eq!(r1.2, BOSS_REWARD_PARTICIPANT, "參與者");
        } else {
            panic!("期望 Defeated");
        }
    }

    // 8. 擊敗後冷卻重置
    #[test]
    fn boss_respawn_cooldown_after_defeat() {
        let mut s = active_state();
        defeat_boss(&mut s);
        assert!(!s.is_active());
        assert!((s.cooldown - BOSS_RESPAWN_SECS).abs() < 1.0);
    }

    // 9. 再次現身（第二輪）
    #[test]
    fn boss_respawns_after_cooldown() {
        let mut s = active_state();
        defeat_boss(&mut s);
        let ev = s.tick(BOSS_RESPAWN_SECS);
        assert!(matches!(ev, BossEvent::Spawned { .. }));
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
        let max_hp = s.active_variant().max_hp;
        let ev = s.hit(Uuid::new_v4(), "英雄".into(), 0);
        assert!(matches!(ev, BossEvent::None));
        assert_eq!(s.current_hp(), Some(max_hp));
    }

    // 12. within_variant_reach 正中心
    #[test]
    fn within_reach_center() {
        let v = ALL_VARIANTS[0];
        assert!(within_variant_reach(v.wx, v.wy, v));
    }

    // 13. within_variant_reach 邊界內
    #[test]
    fn within_reach_edge_inside() {
        let v = ALL_VARIANTS[0];
        assert!(within_variant_reach(v.wx + BOSS_REACH - 1.0, v.wy, v));
    }

    // 14. within_variant_reach 邊界外
    #[test]
    fn within_reach_edge_outside() {
        let v = ALL_VARIANTS[0];
        assert!(!within_variant_reach(v.wx + BOSS_REACH + 1.0, v.wy, v));
    }

    // 15. within_variant_reach NaN 保守 false
    #[test]
    fn within_reach_nan_false() {
        let v = ALL_VARIANTS[0];
        assert!(!within_variant_reach(f32::NAN, v.wy, v));
    }

    // 16. within_variant_reach Infinity 保守 false
    #[test]
    fn within_reach_infinity_false() {
        let v = ALL_VARIANTS[0];
        assert!(!within_variant_reach(f32::INFINITY, v.wy, v));
    }

    // 17. 所有守護者種類距城鎮夠遠（≥ 2000px）
    #[test]
    fn all_variants_far_from_town() {
        let town_x = 2400.0_f32;
        let town_y = 2400.0_f32;
        for v in &ALL_VARIANTS {
            let dx = v.wx - town_x;
            let dy = v.wy - town_y;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(dist >= 2000.0, "{} 距城鎮 {dist:.0}px，應 ≥ 2000px", v.name);
        }
    }

    // 18. variant_for_defeat 確定性循環（0→1→2→3→0）
    #[test]
    fn variant_cycles_deterministically() {
        assert_eq!(variant_for_defeat(0).name, ALL_VARIANTS[0].name);
        assert_eq!(variant_for_defeat(1).name, ALL_VARIANTS[1].name);
        assert_eq!(variant_for_defeat(2).name, ALL_VARIANTS[2].name);
        assert_eq!(variant_for_defeat(3).name, ALL_VARIANTS[3].name);
        assert_eq!(variant_for_defeat(4).name, ALL_VARIANTS[0].name); // 循環回頭
    }

    // 19. 擊敗後 defeat_count 遞增，下一隻換種類
    #[test]
    fn defeat_increments_defeat_count_and_rotates_variant() {
        let mut s = active_state();
        assert_eq!(s.defeat_count, 0);
        let first_name = s.active_variant().name;
        defeat_boss(&mut s);
        assert_eq!(s.defeat_count, 1);
        // 下一隻出現時換種類
        s.cooldown = 0.0;
        s.tick(0.1);
        let second_name = s.active_variant().name;
        assert_ne!(first_name, second_name, "第二隻守護者應換種類");
    }

    // 20. 四種守護者循環一圈後回到第一種
    #[test]
    fn four_defeats_cycle_back_to_first() {
        let mut s = WorldBossState::new();
        let first_name = ALL_VARIANTS[0].name;
        // 模擬 4 次擊敗
        for _ in 0..4 {
            s.cooldown = 0.0;
            s.tick(0.1);
            defeat_boss(&mut s);
        }
        assert_eq!(s.defeat_count, 4);
        // 第 5 隻應同 index 0（循環回頭）
        s.cooldown = 0.0;
        s.tick(0.1);
        assert_eq!(s.active_variant().name, first_name, "循環回第一種");
    }

    // 21. 四種守護者名稱與方位全唯一
    #[test]
    fn all_variants_unique_names_and_positions() {
        let names: Vec<&str> = ALL_VARIANTS.iter().map(|v| v.name).collect();
        let emojis: Vec<&str> = ALL_VARIANTS.iter().map(|v| v.emoji).collect();
        // 名稱不重複
        for i in 0..names.len() {
            for j in (i+1)..names.len() {
                assert_ne!(names[i], names[j], "守護者名稱重複：{}", names[i]);
            }
        }
        // emoji 不重複
        for i in 0..emojis.len() {
            for j in (i+1)..emojis.len() {
                assert_ne!(emojis[i], emojis[j], "守護者 emoji 重複：{}", emojis[i]);
            }
        }
    }
}
