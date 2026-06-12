//! 生態清剿委託（ROADMAP 172）：生態壓力超標時，懸賞官蘭卡自動發布
//! 「清剿XX巢穴」全服委託；所有玩家共同把目標巢穴族群打到指定數量，
//! 達成後全服在線玩家各得乙太獎勵。
//!
//! 成本紀律：
//! - 純罐頭廣播，零 LLM，零額外費用。
//! - 20 分鐘超時、30 分鐘冷卻，防止持續刷屏。
//! - 純記憶體模式，重啟清零，零 migration，不破壞玩家資料。

/// 觸發委託所需的最低生態壓力值（0-100）。
pub const TRIGGER_PRESSURE: f32 = 60.0;

/// 委託超時（秒）：20 分鐘。
pub const MISSION_TIMEOUT_SECS: f32 = 1200.0;

/// 觸發後至下次觸發的冷卻（秒）：30 分鐘。
pub const COOLDOWN_SECS: f32 = 1800.0;

/// 委託完成時所有在線玩家各得乙太。
pub const REWARD_PER_PLAYER: u32 = 12;

/// 伺服器啟動後的首次觸發等待（秒），避免重啟立刻發出委託。
const FIRST_TRIGGER_WAIT: f32 = 600.0;

/// 需殺目標的最低值（至少要殺這麼多隻才算完成）。
pub const MIN_KILL_TARGET: u32 = 3;

// ─────────────────────────────────────────────
// 資料結構
// ─────────────────────────────────────────────

/// 一筆活躍生態清剿委託的資料。
#[derive(Debug, Clone)]
pub struct ActiveEcoBounty {
    /// 目標巢穴 ID（對應 MonsterColony::id）。
    pub colony_id: u32,
    /// 目標巢穴顯示名稱。
    pub colony_name: String,
    /// 委託發布時的初始族群數（僅記錄供廣播，不影響邏輯）。
    pub start_population: u32,
    /// 需要累積擊殺的怪物數才算完成。
    pub kill_target: u32,
    /// 已累積擊殺數。
    pub kills_so_far: u32,
    /// 委託獎勵（每位在線玩家各得）。
    pub reward_per_player: u32,
    /// 剩餘有效期（秒）。
    pub lifetime: f32,
}

impl ActiveEcoBounty {
    /// 委託是否已達成（kills_so_far >= kill_target）。
    pub fn is_complete(&self) -> bool {
        self.kills_so_far >= self.kill_target
    }

    /// 進度比例（0.0 ~ 1.0）。
    pub fn progress_pct(&self) -> f32 {
        if self.kill_target == 0 {
            return 1.0;
        }
        (self.kills_so_far as f32 / self.kill_target as f32).min(1.0)
    }
}

/// `tick()` 與 `on_colony_kill()` 可回傳的委託事件，由 game.rs 消化後廣播。
#[derive(Debug, Clone, PartialEq)]
pub enum EcoBountyEvent {
    /// 新委託開始（需要廣播公告）。
    Started {
        colony_name: String,
        kill_target: u32,
    },
    /// 委託逾時消失（可選擇廣播提醒）。
    Expired { colony_name: String },
    /// 委託達成（需廣播並發給在線玩家乙太）。
    Completed {
        colony_name: String,
        reward_per_player: u32,
    },
}

/// 生態清剿委託管理器（純記憶體，重啟清零）。
pub struct EcoBountyState {
    /// 當前活躍委託（同時最多一筆）。
    pub active: Option<ActiveEcoBounty>,
    /// 距下次允許觸發的倒數（秒）。
    cooldown: f32,
}

impl Default for EcoBountyState {
    fn default() -> Self {
        Self {
            active: None,
            cooldown: FIRST_TRIGGER_WAIT,
        }
    }
}

impl EcoBountyState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 每 tick 推進計時器。
    ///
    /// - `dt`: 本幀秒數
    /// - `eco_pressure`: 當前生態壓力（0.0-100.0）
    /// - `busiest_colony`: 族群最多的巢穴 `(id, name, population)`（由 game.rs 傳入）
    ///
    /// 回傳 `Some(event)` 代表有需要廣播的事件。
    pub fn tick(
        &mut self,
        dt: f32,
        eco_pressure: f32,
        busiest_colony: Option<(u32, String, u32)>,
    ) -> Option<EcoBountyEvent> {
        // 推進活躍委託倒數；過期則清除並回傳 Expired。
        if let Some(ref mut b) = self.active {
            b.lifetime -= dt;
            if b.lifetime <= 0.0 {
                let name = b.colony_name.clone();
                self.active = None;
                self.cooldown = COOLDOWN_SECS;
                return Some(EcoBountyEvent::Expired { colony_name: name });
            }
        }

        // 推進冷卻。
        if self.cooldown > 0.0 {
            self.cooldown -= dt;
        }

        // 嘗試觸發：壓力夠、無活躍委託、冷卻結束。
        if eco_pressure < TRIGGER_PRESSURE || self.active.is_some() || self.cooldown > 0.0 {
            return None;
        }

        let (id, name, pop) = busiest_colony?;

        // 族群太小時不觸發（怪物太少清剿沒意義）。
        if pop < MIN_KILL_TARGET {
            return None;
        }

        // 需殺目標 = max(MIN_KILL_TARGET, population / 2 無條件進位)
        let kill_target = MIN_KILL_TARGET.max((pop + 1) / 2);

        self.active = Some(ActiveEcoBounty {
            colony_id: id,
            colony_name: name.clone(),
            start_population: pop,
            kill_target,
            kills_so_far: 0,
            reward_per_player: REWARD_PER_PLAYER,
            lifetime: MISSION_TIMEOUT_SECS,
        });
        self.cooldown = COOLDOWN_SECS;

        Some(EcoBountyEvent::Started {
            colony_name: name,
            kill_target,
        })
    }

    /// 玩家在目標巢穴擊殺一隻對應怪物時呼叫。
    ///
    /// - `colony_id`: 被擊殺怪物所屬的巢穴 ID
    ///
    /// 回傳 `Some(EcoBountyEvent::Completed)` 若本次擊殺達成委託；
    /// 進度推進但未達成則回傳 `None`；非目標巢穴也回傳 `None`。
    pub fn on_colony_kill(&mut self, colony_id: u32) -> Option<EcoBountyEvent> {
        let b = self.active.as_mut()?;
        if b.colony_id != colony_id {
            return None;
        }
        b.kills_so_far += 1;
        if b.is_complete() {
            let name = b.colony_name.clone();
            let reward = b.reward_per_player;
            self.active = None;
            self.cooldown = COOLDOWN_SECS;
            return Some(EcoBountyEvent::Completed {
                colony_name: name,
                reward_per_player: reward,
            });
        }
        None
    }

    /// 回傳供快照廣播的視圖（無活躍委託時為 None）。
    pub fn view(&self) -> Option<EcoBountyView> {
        self.active.as_ref().map(EcoBountyView::from_active)
    }
}

// ─────────────────────────────────────────────
// 快照視圖（供前端）
// ─────────────────────────────────────────────

/// 供快照廣播的生態清剿委託視圖。
#[derive(Debug, Clone, serde::Serialize)]
pub struct EcoBountyView {
    pub colony_name: String,
    pub kill_target: u32,
    pub kills_so_far: u32,
    pub reward_per_player: u32,
    /// 剩餘時間（秒，取整）。
    pub time_left_secs: u32,
}

impl EcoBountyView {
    pub fn from_active(b: &ActiveEcoBounty) -> Self {
        Self {
            colony_name: b.colony_name.clone(),
            kill_target: b.kill_target,
            kills_so_far: b.kills_so_far,
            reward_per_player: b.reward_per_player,
            time_left_secs: b.lifetime.max(0.0) as u32,
        }
    }
}

// ─────────────────────────────────────────────
// 廣播文字（罐頭）
// ─────────────────────────────────────────────

/// 委託開始廣播。
pub fn started_text(colony_name: &str, kill_target: u32, reward: u32) -> String {
    format!(
        "📋 【生態清剿委託・蘭卡】{colony_name}威脅升高！\
全服共同清剿 {kill_target} 隻，完成後每位在線玩家得 {reward} 乙太！"
    )
}

/// 委託完成廣播。
pub fn completed_text(colony_name: &str, reward: u32) -> String {
    format!(
        "✅ 【清剿委託完成！】{colony_name}威脅平息，在線玩家各得 {reward} 乙太——感謝各位勇者！"
    )
}

/// 委託逾時廣播。
pub fn expired_text(colony_name: &str) -> String {
    format!(
        "⏰ 【清剿委託逾時】{colony_name}清剿未能在時限內完成，威脅仍存——繼續努力！"
    )
}

// ─────────────────────────────────────────────
// 單元測試
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_state() -> EcoBountyState {
        let mut s = EcoBountyState::new();
        s.cooldown = 0.0;
        s
    }

    fn colony(id: u32, pop: u32) -> Option<(u32, String, u32)> {
        Some((id, format!("巢穴{id}"), pop))
    }

    #[test]
    fn no_trigger_when_pressure_low() {
        let mut s = ready_state();
        let r = s.tick(1.0, 50.0, colony(1, 6));
        assert!(r.is_none(), "壓力不足不應觸發");
    }

    #[test]
    fn no_trigger_during_cooldown() {
        let mut s = EcoBountyState::new();
        let r = s.tick(1.0, 80.0, colony(1, 6));
        assert!(r.is_none(), "冷卻中不應觸發");
    }

    #[test]
    fn triggers_when_conditions_met() {
        let mut s = ready_state();
        let r = s.tick(1.0, 65.0, colony(1, 6)).unwrap();
        match r {
            EcoBountyEvent::Started { colony_name, kill_target } => {
                assert_eq!(colony_name, "巢穴1");
                assert_eq!(kill_target, 3); // max(3, (6+1)/2=3) = 3
            }
            _ => panic!("應為 Started"),
        }
        assert!(s.active.is_some());
    }

    #[test]
    fn kill_target_half_of_population() {
        let mut s = ready_state();
        let r = s.tick(1.0, 70.0, colony(2, 8)).unwrap();
        match r {
            EcoBountyEvent::Started { kill_target, .. } => {
                assert_eq!(kill_target, 4); // max(3, (8+1)/2=4) = 4
            }
            _ => panic!("應為 Started"),
        }
    }

    #[test]
    fn kill_target_minimum_enforced_for_small_population() {
        let mut s = ready_state();
        let r = s.tick(1.0, 70.0, colony(3, 4)).unwrap();
        match r {
            EcoBountyEvent::Started { kill_target, .. } => {
                assert_eq!(kill_target, 3); // max(3, (4+1)/2=2) = 3
            }
            _ => panic!("應為 Started"),
        }
    }

    #[test]
    fn no_trigger_when_colony_too_small() {
        let mut s = ready_state();
        let r = s.tick(1.0, 70.0, colony(1, 2)); // pop < MIN_KILL_TARGET
        assert!(r.is_none(), "族群太小不應觸發");
    }

    #[test]
    fn no_trigger_when_no_colony() {
        let mut s = ready_state();
        let r = s.tick(1.0, 70.0, None);
        assert!(r.is_none(), "無巢穴不應觸發");
    }

    #[test]
    fn active_bounty_blocks_new_trigger() {
        let mut s = ready_state();
        let _ = s.tick(1.0, 70.0, colony(1, 6));
        s.cooldown = 0.0;
        let r = s.tick(1.0, 80.0, colony(2, 8));
        assert!(r.is_none(), "有活躍委託不應再觸發");
    }

    #[test]
    fn on_kill_wrong_colony_ignored() {
        let mut s = ready_state();
        let _ = s.tick(1.0, 70.0, colony(1, 6));
        let r = s.on_colony_kill(99);
        assert!(r.is_none(), "非目標巢穴應無效");
        assert!(s.active.is_some());
    }

    #[test]
    fn on_kill_increments_progress() {
        let mut s = ready_state();
        let _ = s.tick(1.0, 70.0, colony(1, 6)); // kill_target = 3
        let r = s.on_colony_kill(1);
        assert!(r.is_none(), "未達目標不完成");
        assert_eq!(s.active.as_ref().unwrap().kills_so_far, 1);
    }

    #[test]
    fn on_kill_completes_at_target() {
        let mut s = ready_state();
        let _ = s.tick(1.0, 70.0, colony(1, 6)); // kill_target = 3
        s.on_colony_kill(1);
        s.on_colony_kill(1);
        let r = s.on_colony_kill(1).unwrap(); // 第 3 擊
        match r {
            EcoBountyEvent::Completed { colony_name, reward_per_player } => {
                assert_eq!(colony_name, "巢穴1");
                assert_eq!(reward_per_player, REWARD_PER_PLAYER);
            }
            _ => panic!("應為 Completed"),
        }
        assert!(s.active.is_none(), "完成後應清除活躍委託");
    }

    #[test]
    fn expires_after_timeout() {
        let mut s = ready_state();
        let _ = s.tick(1.0, 70.0, colony(1, 6));
        let r = s.tick(MISSION_TIMEOUT_SECS + 1.0, 70.0, None).unwrap();
        match r {
            EcoBountyEvent::Expired { colony_name } => {
                assert_eq!(colony_name, "巢穴1");
            }
            _ => panic!("應為 Expired"),
        }
        assert!(s.active.is_none());
    }

    #[test]
    fn cooldown_prevents_immediate_retrigger_after_complete() {
        let mut s = ready_state();
        let _ = s.tick(1.0, 70.0, colony(1, 6)); // kill_target = 3
        s.on_colony_kill(1);
        s.on_colony_kill(1);
        s.on_colony_kill(1); // 完成
        // 完成後冷卻應重置
        let r = s.tick(1.0, 80.0, colony(2, 8));
        assert!(r.is_none(), "完成後冷卻期間不應立刻再觸發");
    }

    #[test]
    fn progress_pct_correct() {
        let b = ActiveEcoBounty {
            colony_id: 1,
            colony_name: "test".to_string(),
            start_population: 6,
            kill_target: 4,
            kills_so_far: 2,
            reward_per_player: 12,
            lifetime: 1000.0,
        };
        assert!((b.progress_pct() - 0.5).abs() < 0.01, "進度比例應為 0.5");
    }

    #[test]
    fn view_returns_none_when_no_active() {
        let s = EcoBountyState::new();
        assert!(s.view().is_none());
    }

    #[test]
    fn view_returns_some_when_active() {
        let mut s = ready_state();
        let _ = s.tick(1.0, 70.0, colony(1, 6));
        let v = s.view().unwrap();
        assert_eq!(v.kill_target, 3);
        assert_eq!(v.kills_so_far, 0);
        assert_eq!(v.reward_per_player, REWARD_PER_PLAYER);
        assert!(v.time_left_secs > 0);
    }

    #[test]
    fn started_text_has_key_info() {
        let t = started_text("靈蛾巢", 3, 12);
        assert!(t.contains("靈蛾巢") && t.contains('3') && t.contains("12"));
    }

    #[test]
    fn completed_text_has_key_info() {
        let t = completed_text("靈蛾巢", 12);
        assert!(t.contains("靈蛾巢") && t.contains("12"));
    }

    #[test]
    fn expired_text_has_colony_name() {
        let t = expired_text("靈蛾巢");
        assert!(t.contains("靈蛾巢"));
    }
}
