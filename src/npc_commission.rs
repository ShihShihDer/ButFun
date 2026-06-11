//! NPC 主動資材委託（ROADMAP 85：Wave 2 第十四塊）。
//!
//! 當商人薇拉的繁榮感跌破閾值時，她自主在世界頻道發出「急收令」：
//! 指定一種物品並承諾加價收購，玩家在委託有效期內賣給她即可得到每份額外獎勵。
//! 達到配額或逾時後，委託結束。
//!
//! 成本紀律：
//! - 純罐頭訊息，**不**呼叫 LLM，零額外費用。
//! - 15 分鐘逾時、30 分鐘冷卻，防止持續刷屏。
//! - 純記憶體模式，重啟清零，零 migration，不破壞玩家資料。

use crate::inventory::ItemKind;

/// 商人的顯示名稱。
pub const MERCHANT_DISPLAY_NAME: &str = "商人薇拉";

/// 繁榮感低於此值時，薇拉才考慮發布急收令。
pub const PROSPERITY_THRESHOLD: i32 = 20;

/// 每份物品的加價獎勵（乙太）。
pub const BONUS_PER_UNIT: u32 = 5;

/// 委託配額：達到此數量後視為委託完成。
pub const COMMISSION_QUOTA: u32 = 20;

/// 委託有效期（秒）。
pub const COMMISSION_DURATION_SECS: f32 = 900.0; // 15 分鐘

/// 委託冷卻（秒）：委託結束後多久才能再發。
pub const ANNOUNCE_COOLDOWN_SECS: f32 = 1800.0; // 30 分鐘

/// 伺服器啟動後的首次觸發等待（秒），避免重啟立刻觸發。
const FIRST_ANNOUNCE_WAIT_SECS: f32 = 300.0; // 5 分鐘

/// 委託指定的物品種類（按輪數循環）。
const COMMISSION_ITEMS: &[ItemKind] = &[
    ItemKind::Wood,
    ItemKind::Stone,
    ItemKind::Ether,
];

/// 各物品的繁中顯示名稱（對應 `COMMISSION_ITEMS` 順序）。
const ITEM_ZH_NAMES: &[&str] = &["木材", "石頭", "乙太礦"];

/// 一筆活躍委託的資料。
#[derive(Debug, Clone)]
pub struct ActiveCommission {
    /// 急收物品種類。
    pub item: ItemKind,
    /// 物品繁中名稱（用於廣播文字）。
    pub item_name: &'static str,
    /// 每份加價乙太。
    pub bonus_per_unit: u32,
    /// 配額：達到此數量後自動完成。
    pub quota: u32,
    /// 已累積收到的數量。
    pub filled: u32,
    /// 剩餘有效期（秒）。
    pub lifetime: f32,
}

impl ActiveCommission {
    fn new(item: ItemKind, item_name: &'static str) -> Self {
        Self {
            item,
            item_name,
            bonus_per_unit: BONUS_PER_UNIT,
            quota: COMMISSION_QUOTA,
            filled: 0,
            lifetime: COMMISSION_DURATION_SECS,
        }
    }
}

/// NPC 資材委託全域狀態（純記憶體，重啟清零）。
pub struct NpcCommissionState {
    /// 當前活躍委託（同時最多一筆）。
    pub active: Option<ActiveCommission>,
    /// 距下次允許發布委託的倒數（秒）。
    announce_cooldown: f32,
    /// 累計觸發次數（用於循環決定下一個物品）。
    trigger_count: usize,
}

impl Default for NpcCommissionState {
    fn default() -> Self {
        Self {
            active: None,
            announce_cooldown: FIRST_ANNOUNCE_WAIT_SECS,
            trigger_count: 0,
        }
    }
}

impl NpcCommissionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 每 tick 推進計時器。
    ///
    /// - `dt`：本幀秒數
    /// - `merchant_prosperity`：商人當前繁榮感（0~100）
    /// - `players_online`：在線玩家數
    ///
    /// 回傳：
    /// - `CommissionEvent::NewCommission { item_name }` — 剛觸發新委託（呼叫端負責廣播）
    /// - `CommissionEvent::Expired` — 委託逾時消失（呼叫端可選擇廣播）
    /// - `None` — 無事發生
    pub fn tick(
        &mut self,
        dt: f32,
        merchant_prosperity: i32,
        players_online: usize,
    ) -> Option<CommissionEvent> {
        // 推進活躍委託的剩餘時間；過期則清除並回傳 Expired。
        if let Some(ref mut c) = self.active {
            c.lifetime -= dt;
            if c.lifetime <= 0.0 {
                self.active = None;
                self.announce_cooldown = ANNOUNCE_COOLDOWN_SECS;
                return Some(CommissionEvent::Expired);
            }
        }

        // 推進公告冷卻。
        if self.announce_cooldown > 0.0 {
            self.announce_cooldown -= dt;
        }

        // 觸發條件：繁榮感低 + 無活躍委託 + 冷卻結束 + 有玩家在線。
        if players_online == 0
            || merchant_prosperity >= PROSPERITY_THRESHOLD
            || self.active.is_some()
            || self.announce_cooldown > 0.0
        {
            return None;
        }

        // 循環選下一個委託物品。
        let idx = self.trigger_count % COMMISSION_ITEMS.len();
        let item = COMMISSION_ITEMS[idx];
        let item_name = ITEM_ZH_NAMES[idx];
        self.trigger_count += 1;

        self.active = Some(ActiveCommission::new(item, item_name));
        self.announce_cooldown = ANNOUNCE_COOLDOWN_SECS;

        Some(CommissionEvent::NewCommission {
            item_name,
            bonus: BONUS_PER_UNIT,
            quota: COMMISSION_QUOTA,
        })
    }

    /// 玩家向商人賣出物品時呼叫。
    ///
    /// 若有活躍委託且物品符合，扣減配額並回傳加價乙太數（`qty * bonus_per_unit`）。
    /// 若委託因此達到配額，同時回傳 `Some(CommissionEvent::Fulfilled)`（透過 `fulfilled` flag）。
    /// 呼叫端負責廣播完成公告。
    pub fn on_sold(
        &mut self,
        item: ItemKind,
        qty: u32,
    ) -> SellResult {
        let Some(ref mut c) = self.active else {
            return SellResult { bonus: 0, fulfilled: false };
        };
        if c.item != item {
            return SellResult { bonus: 0, fulfilled: false };
        }

        let bonus = c.bonus_per_unit.saturating_mul(qty);
        c.filled = c.filled.saturating_add(qty);

        let fulfilled = c.filled >= c.quota;
        if fulfilled {
            self.active = None;
            self.announce_cooldown = ANNOUNCE_COOLDOWN_SECS;
        }

        SellResult { bonus, fulfilled }
    }
}

/// `tick()` 的回傳事件。
#[derive(Debug, Clone, PartialEq)]
pub enum CommissionEvent {
    /// 剛觸發新委託。
    NewCommission {
        item_name: &'static str,
        bonus: u32,
        quota: u32,
    },
    /// 委託逾時消失（未被填滿）。
    Expired,
}

/// `on_sold()` 的回傳結果。
#[derive(Debug, Clone)]
pub struct SellResult {
    /// 額外獎勵乙太（0 = 無委託加成）。
    pub bonus: u32,
    /// 本次賣出後委託是否達成配額。
    pub fulfilled: bool,
}

/// 委託發布公告文字（罐頭，供廣播）。
pub fn announce_text(item_name: &str, bonus: u32, quota: u32) -> String {
    format!(
        "急收 {item_name}！今後 15 分鐘內賣給我，每份多給 {bonus} 乙太！\
        （共需 {quota} 份，先到先得）"
    )
}

/// 委託完成公告文字（罐頭，供廣播）。
pub fn fulfilled_text(item_name: &str) -> String {
    format!("急收令完成！感謝各位帶來 {item_name}，今日生意好多了！")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_state() -> NpcCommissionState {
        let mut s = NpcCommissionState::new();
        s.announce_cooldown = 0.0;
        s
    }

    #[test]
    fn no_trigger_without_players() {
        let mut s = ready_state();
        let result = s.tick(1.0, 10, 0);
        assert!(result.is_none(), "無玩家在線時不應觸發委託");
    }

    #[test]
    fn no_trigger_when_prosperity_high() {
        let mut s = ready_state();
        let result = s.tick(1.0, PROSPERITY_THRESHOLD, 2);
        assert!(result.is_none(), "繁榮感達閾值時不應觸發委託");
    }

    #[test]
    fn no_trigger_during_cooldown() {
        let mut s = NpcCommissionState::new();
        // 保持初始等待冷卻
        let result = s.tick(1.0, 10, 2);
        assert!(result.is_none(), "冷卻期間不應觸發委託");
    }

    #[test]
    fn triggers_when_all_conditions_met() {
        let mut s = ready_state();
        let result = s.tick(1.0, 10, 2);
        assert!(result.is_some(), "條件全部成立時應觸發委託");
        match result.unwrap() {
            CommissionEvent::NewCommission { bonus, quota, .. } => {
                assert_eq!(bonus, BONUS_PER_UNIT);
                assert_eq!(quota, COMMISSION_QUOTA);
            }
            _ => panic!("應為 NewCommission"),
        }
    }

    #[test]
    fn active_commission_blocks_new_trigger() {
        let mut s = ready_state();
        let r1 = s.tick(1.0, 10, 2);
        assert!(r1.is_some(), "第一次應觸發");
        s.announce_cooldown = 0.0;
        let r2 = s.tick(1.0, 10, 2);
        assert!(r2.is_none(), "已有活躍委託時不應再觸發");
    }

    #[test]
    fn commission_cycles_through_items() {
        let mut s = ready_state();
        // 第 1 次 → Wood
        let r1 = s.tick(1.0, 10, 1).unwrap();
        let name1 = match r1 { CommissionEvent::NewCommission { item_name, .. } => item_name, _ => panic!() };
        assert_eq!(name1, "木材");

        // 手動完成委託並清除冷卻，觸發第 2 次
        s.active = None;
        s.announce_cooldown = 0.0;
        let r2 = s.tick(1.0, 10, 1).unwrap();
        let name2 = match r2 { CommissionEvent::NewCommission { item_name, .. } => item_name, _ => panic!() };
        assert_eq!(name2, "石頭");

        // 第 3 次 → Ether
        s.active = None;
        s.announce_cooldown = 0.0;
        let r3 = s.tick(1.0, 10, 1).unwrap();
        let name3 = match r3 { CommissionEvent::NewCommission { item_name, .. } => item_name, _ => panic!() };
        assert_eq!(name3, "乙太礦");

        // 第 4 次 → 回到 Wood
        s.active = None;
        s.announce_cooldown = 0.0;
        let r4 = s.tick(1.0, 10, 1).unwrap();
        let name4 = match r4 { CommissionEvent::NewCommission { item_name, .. } => item_name, _ => panic!() };
        assert_eq!(name4, "木材");
    }

    #[test]
    fn commission_expires_after_lifetime() {
        let mut s = ready_state();
        let _ = s.tick(1.0, 10, 1);
        assert!(s.active.is_some());
        // 推進超過有效期
        let r = s.tick(COMMISSION_DURATION_SECS + 1.0, 10, 1);
        assert_eq!(r, Some(CommissionEvent::Expired));
        assert!(s.active.is_none());
    }

    #[test]
    fn on_sold_gives_bonus_for_matching_item() {
        let mut s = ready_state();
        s.tick(1.0, 10, 1); // 觸發 Wood 委託
        let result = s.on_sold(ItemKind::Wood, 3);
        assert_eq!(result.bonus, BONUS_PER_UNIT * 3, "應得到 3 份加成");
        assert!(!result.fulfilled, "3 份未達配額，委託應仍存在");
    }

    #[test]
    fn on_sold_no_bonus_for_wrong_item() {
        let mut s = ready_state();
        s.tick(1.0, 10, 1); // 觸發 Wood 委託
        let result = s.on_sold(ItemKind::Stone, 5);
        assert_eq!(result.bonus, 0, "賣錯物品不應有加成");
        assert!(!result.fulfilled);
    }

    #[test]
    fn on_sold_fulfills_when_quota_reached() {
        let mut s = ready_state();
        s.tick(1.0, 10, 1); // 觸發 Wood 委託
        let result = s.on_sold(ItemKind::Wood, COMMISSION_QUOTA);
        assert_eq!(result.bonus, BONUS_PER_UNIT * COMMISSION_QUOTA);
        assert!(result.fulfilled, "達配額時應回傳 fulfilled");
        assert!(s.active.is_none(), "委託完成後應清除");
    }

    #[test]
    fn on_sold_no_active_commission_returns_zero() {
        let mut s = NpcCommissionState::new();
        let result = s.on_sold(ItemKind::Wood, 5);
        assert_eq!(result.bonus, 0);
        assert!(!result.fulfilled);
    }

    #[test]
    fn announce_text_contains_key_info() {
        let txt = announce_text("木材", 5, 20);
        assert!(txt.contains("木材"));
        assert!(txt.contains('5'));
        assert!(txt.contains("20"));
    }

    #[test]
    fn fulfilled_text_contains_item_name() {
        let txt = fulfilled_text("石頭");
        assert!(txt.contains("石頭"));
    }
}
