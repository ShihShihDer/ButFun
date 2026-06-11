//! NPC 探勘加碼令（ROADMAP 86：Wave 2 第十五塊）。
//!
//! 當探勘員芙利亞的安全感高（≥ 70）時，她自主在世界頻道宣告「野外加碼令」：
//! 接下來 10 分鐘內完成任意探勘採樣，每筆額外獲得 +8 乙太獎勵，先到先得限 10 份。
//! 達到配額或逾時後，加碼令結束。
//!
//! 此為薇拉急收令（ROADMAP 85）的對稱設計：
//! - 薇拉：繁榮感「低」→ 自主發布急收令（守勢，需要補給）
//! - 芙利亞：安全感「高」→ 自主發布加碼令（進攻，鼓勵探索）
//!
//! 成本紀律：
//! - 純罐頭訊息，**不**呼叫 LLM，零額外費用。
//! - 10 分鐘逾時、30 分鐘冷卻，防止持續刷屏。
//! - 純記憶體模式，重啟清零，零 migration，不破壞玩家資料。

/// 探勘員的顯示名稱。
pub const EXPEDITION_NPC_NAME: &str = "探勘員芙利亞";

/// 安全感高於此值時，芙利亞才考慮發布加碼令。
pub const SAFETY_THRESHOLD: i32 = 70;

/// 每筆探勘採樣的額外獎勵（乙太）。
pub const BONUS_PER_SURVEY: u32 = 8;

/// 加碼配額：達到此份數後視為加碼令完成。
pub const BOOST_QUOTA: u32 = 10;

/// 加碼有效期（秒）。
pub const BOOST_DURATION_SECS: f32 = 600.0; // 10 分鐘

/// 加碼冷卻（秒）：加碼令結束後多久才能再發。
pub const ANNOUNCE_COOLDOWN_SECS: f32 = 1800.0; // 30 分鐘

/// 伺服器啟動後的首次觸發等待（秒），避免重啟立刻觸發。
const FIRST_ANNOUNCE_WAIT_SECS: f32 = 300.0; // 5 分鐘

/// 一筆活躍加碼令的資料。
#[derive(Debug, Clone)]
pub struct ActiveBoost {
    /// 每筆探勘的額外獎勵乙太。
    pub bonus_per_survey: u32,
    /// 配額：達到此份數後自動完成。
    pub quota: u32,
    /// 已累積完成的份數。
    pub filled: u32,
    /// 剩餘有效期（秒）。
    pub lifetime: f32,
}

impl ActiveBoost {
    fn new() -> Self {
        Self {
            bonus_per_survey: BONUS_PER_SURVEY,
            quota: BOOST_QUOTA,
            filled: 0,
            lifetime: BOOST_DURATION_SECS,
        }
    }
}

/// NPC 探勘加碼全域狀態（純記憶體，重啟清零）。
pub struct NpcExpeditionBoostState {
    /// 當前活躍加碼令（同時最多一筆）。
    pub active: Option<ActiveBoost>,
    /// 距下次允許發布加碼令的倒數（秒）。
    announce_cooldown: f32,
}

impl Default for NpcExpeditionBoostState {
    fn default() -> Self {
        Self {
            active: None,
            announce_cooldown: FIRST_ANNOUNCE_WAIT_SECS,
        }
    }
}

impl NpcExpeditionBoostState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 每 tick 推進計時器。
    ///
    /// - `dt`：本幀秒數
    /// - `expedition_safety`：探勘員當前安全感（0~100）
    ///
    /// 回傳：
    /// - `BoostEvent::NewBoost { bonus, quota }` — 剛觸發新加碼令（呼叫端負責廣播）
    /// - `BoostEvent::Expired` — 加碼令逾時消失（呼叫端可選擇廣播）
    /// - `None` — 無事發生
    pub fn tick(
        &mut self,
        dt: f32,
        expedition_safety: i32,
    ) -> Option<BoostEvent> {
        // 推進活躍加碼令的剩餘時間；過期則清除並回傳 Expired。
        if let Some(ref mut b) = self.active {
            b.lifetime -= dt;
            if b.lifetime <= 0.0 {
                self.active = None;
                self.announce_cooldown = ANNOUNCE_COOLDOWN_SECS;
                return Some(BoostEvent::Expired);
            }
        }

        // 推進公告冷卻。
        if self.announce_cooldown > 0.0 {
            self.announce_cooldown -= dt;
        }

        // 觸發條件：安全感高 + 無活躍加碼令 + 冷卻結束。
        if expedition_safety < SAFETY_THRESHOLD
            || self.active.is_some()
            || self.announce_cooldown > 0.0
        {
            return None;
        }

        self.active = Some(ActiveBoost::new());
        self.announce_cooldown = ANNOUNCE_COOLDOWN_SECS;

        Some(BoostEvent::NewBoost {
            bonus: BONUS_PER_SURVEY,
            quota: BOOST_QUOTA,
        })
    }

    /// 玩家完成探勘採樣時呼叫。
    ///
    /// 若有活躍加碼令，扣減配額並回傳額外獎勵乙太。
    /// 若加碼令因此達到配額，透過回傳值的 `fulfilled` 標記通知呼叫端。
    /// 呼叫端負責廣播完成公告。
    pub fn on_surveyed(&mut self) -> SurveyResult {
        let Some(ref mut b) = self.active else {
            return SurveyResult { bonus: 0, fulfilled: false };
        };

        let bonus = b.bonus_per_survey;
        b.filled = b.filled.saturating_add(1);

        let fulfilled = b.filled >= b.quota;
        if fulfilled {
            self.active = None;
            self.announce_cooldown = ANNOUNCE_COOLDOWN_SECS;
        }

        SurveyResult { bonus, fulfilled }
    }
}

/// `tick()` 的回傳事件。
#[derive(Debug, Clone, PartialEq)]
pub enum BoostEvent {
    /// 剛觸發新加碼令。
    NewBoost { bonus: u32, quota: u32 },
    /// 加碼令逾時消失（未被填滿）。
    Expired,
}

/// `on_surveyed()` 的回傳結果。
#[derive(Debug, Clone)]
pub struct SurveyResult {
    /// 額外獎勵乙太（0 = 無加碼）。
    pub bonus: u32,
    /// 本次採樣後加碼令是否達到配額。
    pub fulfilled: bool,
}

/// 加碼令發布公告文字（罐頭，供廣播）。
pub fn announce_text(bonus: u32, quota: u32) -> String {
    format!(
        "今日野外風和日麗，適合探勘！接下來 10 分鐘，所有探勘採樣額外獲得 {bonus} 乙太！\
        （限 {quota} 份，先到先得）"
    )
}

/// 加碼令完成公告文字（罐頭，供廣播）。
pub fn fulfilled_text() -> &'static str {
    "加碼已達上限！感謝各位探勘員出動，今天的野外調查收穫豐碩！"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_state() -> NpcExpeditionBoostState {
        let mut s = NpcExpeditionBoostState::new();
        s.announce_cooldown = 0.0;
        s
    }


    #[test]
    fn no_trigger_when_safety_low() {
        let mut s = ready_state();
        let result = s.tick(1.0, SAFETY_THRESHOLD - 1);
        assert!(result.is_none(), "安全感未達閾值時不應觸發加碼令");
    }

    #[test]
    fn no_trigger_at_exact_threshold_minus_one() {
        let mut s = ready_state();
        assert!(s.tick(1.0, 69).is_none(), "安全感 69 不應觸發");
    }

    #[test]
    fn no_trigger_during_cooldown() {
        let mut s = NpcExpeditionBoostState::new();
        let result = s.tick(1.0, 80);
        assert!(result.is_none(), "初始等待冷卻期間不應觸發加碼令");
    }

    #[test]
    fn triggers_when_all_conditions_met() {
        let mut s = ready_state();
        let result = s.tick(1.0, SAFETY_THRESHOLD);
        assert!(result.is_some(), "條件全部成立時應觸發加碼令");
        match result.unwrap() {
            BoostEvent::NewBoost { bonus, quota } => {
                assert_eq!(bonus, BONUS_PER_SURVEY);
                assert_eq!(quota, BOOST_QUOTA);
            }
            _ => panic!("應為 NewBoost"),
        }
    }

    #[test]
    fn active_boost_blocks_new_trigger() {
        let mut s = ready_state();
        let r1 = s.tick(1.0, 80);
        assert!(r1.is_some(), "第一次應觸發");
        s.announce_cooldown = 0.0;
        let r2 = s.tick(1.0, 80);
        assert!(r2.is_none(), "已有活躍加碼令時不應再觸發");
    }

    #[test]
    fn boost_expires_after_lifetime() {
        let mut s = ready_state();
        let _ = s.tick(1.0, 80);
        assert!(s.active.is_some());
        let r = s.tick(BOOST_DURATION_SECS + 1.0, 80);
        assert_eq!(r, Some(BoostEvent::Expired));
        assert!(s.active.is_none());
    }

    #[test]
    fn on_surveyed_gives_bonus_when_active() {
        let mut s = ready_state();
        s.tick(1.0, 80); // 觸發加碼令
        let result = s.on_surveyed();
        assert_eq!(result.bonus, BONUS_PER_SURVEY, "應得到加成");
        assert!(!result.fulfilled, "一份未達配額");
    }

    #[test]
    fn on_surveyed_no_bonus_without_active_boost() {
        let mut s = NpcExpeditionBoostState::new();
        let result = s.on_surveyed();
        assert_eq!(result.bonus, 0, "無加碼令時不應有獎勵");
        assert!(!result.fulfilled);
    }

    #[test]
    fn on_surveyed_fulfills_when_quota_reached() {
        let mut s = ready_state();
        s.tick(1.0, 80); // 觸發加碼令
        // 連續完成到配額
        for _ in 0..BOOST_QUOTA - 1 {
            let r = s.on_surveyed();
            assert!(!r.fulfilled, "未到配額時不應完成");
        }
        let final_result = s.on_surveyed();
        assert_eq!(final_result.bonus, BONUS_PER_SURVEY);
        assert!(final_result.fulfilled, "達配額時應回傳 fulfilled");
        assert!(s.active.is_none(), "加碼令完成後應清除");
    }

    #[test]
    fn announce_text_contains_key_info() {
        let txt = announce_text(8, 10);
        assert!(txt.contains('8'), "公告文字應含獎勵數");
        assert!(txt.contains("10"), "公告文字應含配額數");
    }

    #[test]
    fn fulfilled_text_is_nonempty() {
        assert!(!fulfilled_text().is_empty(), "完成公告文字不應為空");
    }
}
