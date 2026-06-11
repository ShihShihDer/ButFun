//! 工匠老胡急修加成令（ROADMAP 87：Wave 2 第十六塊）。
//!
//! 當工匠老胡的歸屬感高（≥ 65）時，他自主在世界頻道宣告「急修加成令」：
//! 接下來 10 分鐘內完成任意工坊訂單，每筆額外獲得 +15 乙太獎勵，先到先得限 5 份。
//! 達到配額或逾時後，加成令結束。
//!
//! 對稱設計（Wave 2 NPC 自主行動矩陣）：
//! - 薇拉：繁榮感「低」→ 急收令（守勢，需要補給）
//! - 芙利亞：安全感「高」→ 探勘加碼令（進攻，鼓勵探索）
//! - 蘭卡：安全感「低」→ 懸賞令（危機，討伐威脅）
//! - 老胡：歸屬感「高」→ 急修加成令（慶典，感謝工匠夥伴）
//!
//! 成本紀律：
//! - 純罐頭訊息，**不**呼叫 LLM，零額外費用。
//! - 10 分鐘逾時、30 分鐘冷卻，防止持續刷屏。
//! - 純記憶體模式，重啟清零，零 migration，不破壞玩家資料。

/// 工匠的顯示名稱。
pub const WORKSHOP_NPC_NAME: &str = "工匠老胡";

/// 歸屬感高於此值時，老胡才考慮發布急修加成令。
pub const BELONGING_THRESHOLD: i32 = 65;

/// 每筆工坊訂單的額外獎勵（乙太）。
pub const BONUS_PER_ORDER: u32 = 15;

/// 加成配額：達到此份數後視為加成令完成。
pub const BOOST_QUOTA: u32 = 5;

/// 加成有效期（秒）。
pub const BOOST_DURATION_SECS: f32 = 600.0; // 10 分鐘

/// 加成冷卻（秒）：加成令結束後多久才能再發。
pub const ANNOUNCE_COOLDOWN_SECS: f32 = 1800.0; // 30 分鐘

/// 伺服器啟動後的首次觸發等待（秒），避免重啟立刻觸發。
const FIRST_ANNOUNCE_WAIT_SECS: f32 = 300.0; // 5 分鐘

/// 一筆活躍加成令的資料。
#[derive(Debug, Clone)]
pub struct ActiveBoost {
    /// 每筆工坊訂單的額外獎勵乙太。
    pub bonus_per_order: u32,
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
            bonus_per_order: BONUS_PER_ORDER,
            quota: BOOST_QUOTA,
            filled: 0,
            lifetime: BOOST_DURATION_SECS,
        }
    }
}

/// NPC 工坊加成全域狀態（純記憶體，重啟清零）。
pub struct NpcWorkshopBoostState {
    /// 當前活躍加成令（同時最多一筆）。
    pub active: Option<ActiveBoost>,
    /// 距下次允許發布加成令的倒數（秒）。
    announce_cooldown: f32,
}

impl Default for NpcWorkshopBoostState {
    fn default() -> Self {
        Self {
            active: None,
            announce_cooldown: FIRST_ANNOUNCE_WAIT_SECS,
        }
    }
}

impl NpcWorkshopBoostState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 每 tick 推進計時器。
    ///
    /// - `dt`：本幀秒數
    /// - `workshop_belonging`：工匠老胡當前歸屬感（0~100）
    ///
    /// 回傳：
    /// - `BoostEvent::NewBoost { bonus, quota }` — 剛觸發新加成令（呼叫端負責廣播）
    /// - `BoostEvent::Expired` — 加成令逾時消失（呼叫端可選擇廣播）
    /// - `None` — 無事發生
    pub fn tick(
        &mut self,
        dt: f32,
        workshop_belonging: i32,
    ) -> Option<BoostEvent> {
        // 推進活躍加成令的剩餘時間；過期則清除並回傳 Expired。
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

        // 觸發條件：歸屬感高 + 無活躍加成令 + 冷卻結束。
        if workshop_belonging < BELONGING_THRESHOLD
            || self.active.is_some()
            || self.announce_cooldown > 0.0
        {
            return None;
        }

        self.active = Some(ActiveBoost::new());
        self.announce_cooldown = ANNOUNCE_COOLDOWN_SECS;

        Some(BoostEvent::NewBoost {
            bonus: BONUS_PER_ORDER,
            quota: BOOST_QUOTA,
        })
    }

    /// 玩家完成工坊訂單時呼叫。
    ///
    /// 若有活躍加成令，扣減配額並回傳額外獎勵乙太。
    /// 若加成令因此達到配額，透過回傳值的 `fulfilled` 標記通知呼叫端。
    /// 呼叫端負責廣播完成公告。
    pub fn on_order_fulfilled(&mut self) -> OrderResult {
        let Some(ref mut b) = self.active else {
            return OrderResult { bonus: 0, fulfilled: false };
        };

        let bonus = b.bonus_per_order;
        b.filled = b.filled.saturating_add(1);

        let fulfilled = b.filled >= b.quota;
        if fulfilled {
            self.active = None;
            self.announce_cooldown = ANNOUNCE_COOLDOWN_SECS;
        }

        OrderResult { bonus, fulfilled }
    }
}

/// `tick()` 的回傳事件。
#[derive(Debug, Clone, PartialEq)]
pub enum BoostEvent {
    /// 剛觸發新加成令。
    NewBoost { bonus: u32, quota: u32 },
    /// 加成令逾時消失（未被填滿）。
    Expired,
}

/// `on_order_fulfilled()` 的回傳結果。
#[derive(Debug, Clone)]
pub struct OrderResult {
    /// 額外獎勵乙太（0 = 無加成）。
    pub bonus: u32,
    /// 本次交付後加成令是否達到配額。
    pub fulfilled: bool,
}

/// 加成令發布公告文字（罐頭，供廣播）。
pub fn announce_text(bonus: u32, quota: u32) -> String {
    format!(
        "今天幹活兒特別帶勁！接下來 10 分鐘，工坊每份訂單多給 {bonus} 乙太！\
        （限 {quota} 份，先到先得）"
    )
}

/// 加成令完成公告文字（罐頭，供廣播）。
pub fn fulfilled_text() -> &'static str {
    "今日加成名額全數用完！感謝各位好手藝，工坊今天業績不錯！"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_state() -> NpcWorkshopBoostState {
        let mut s = NpcWorkshopBoostState::new();
        s.announce_cooldown = 0.0;
        s
    }


    #[test]
    fn no_trigger_when_belonging_low() {
        let mut s = ready_state();
        let result = s.tick(1.0, BELONGING_THRESHOLD - 1);
        assert!(result.is_none(), "歸屬感未達閾值時不應觸發加成令");
    }

    #[test]
    fn no_trigger_at_exact_threshold_minus_one() {
        let mut s = ready_state();
        assert!(s.tick(1.0, 64).is_none(), "歸屬感 64 不應觸發");
    }

    #[test]
    fn no_trigger_during_cooldown() {
        let mut s = NpcWorkshopBoostState::new();
        let result = s.tick(1.0, 80);
        assert!(result.is_none(), "初始等待冷卻期間不應觸發加成令");
    }

    #[test]
    fn triggers_when_all_conditions_met() {
        let mut s = ready_state();
        let result = s.tick(1.0, BELONGING_THRESHOLD);
        assert!(result.is_some(), "條件全部成立時應觸發加成令");
        match result.unwrap() {
            BoostEvent::NewBoost { bonus, quota } => {
                assert_eq!(bonus, BONUS_PER_ORDER);
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
        assert!(r2.is_none(), "已有活躍加成令時不應再觸發");
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
    fn on_order_fulfilled_gives_bonus_when_active() {
        let mut s = ready_state();
        s.tick(1.0, 80); // 觸發加成令
        let result = s.on_order_fulfilled();
        assert_eq!(result.bonus, BONUS_PER_ORDER, "應得到加成");
        assert!(!result.fulfilled, "一份未達配額");
    }

    #[test]
    fn on_order_fulfilled_no_bonus_without_active_boost() {
        let mut s = NpcWorkshopBoostState::new();
        let result = s.on_order_fulfilled();
        assert_eq!(result.bonus, 0, "無加成令時不應有獎勵");
        assert!(!result.fulfilled);
    }

    #[test]
    fn on_order_fulfilled_completes_when_quota_reached() {
        let mut s = ready_state();
        s.tick(1.0, 80); // 觸發加成令
        // 連續完成到配額
        for _ in 0..BOOST_QUOTA - 1 {
            let r = s.on_order_fulfilled();
            assert!(!r.fulfilled, "未到配額時不應完成");
        }
        let final_result = s.on_order_fulfilled();
        assert_eq!(final_result.bonus, BONUS_PER_ORDER);
        assert!(final_result.fulfilled, "達配額時應回傳 fulfilled");
        assert!(s.active.is_none(), "加成令完成後應清除");
    }

    #[test]
    fn announce_text_contains_key_info() {
        let txt = announce_text(15, 5);
        assert!(txt.contains("15"), "公告文字應含獎勵數");
        assert!(txt.contains('5'), "公告文字應含配額數");
    }

    #[test]
    fn fulfilled_text_is_nonempty() {
        assert!(!fulfilled_text().is_empty(), "完成公告文字不應為空");
    }

    #[test]
    fn constants_are_reasonable() {
        assert!(BELONGING_THRESHOLD > 50, "歸屬感閾值應高於中線");
        assert!(BOOST_DURATION_SECS > 0.0, "有效期應為正數");
        assert!(ANNOUNCE_COOLDOWN_SECS > BOOST_DURATION_SECS, "冷卻應長於有效期");
        assert!(BONUS_PER_ORDER > 0, "加成應為正數");
    }
}
