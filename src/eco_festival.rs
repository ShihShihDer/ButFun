//! 生態豐收節（ROADMAP 178）：生態壓力曾衝上危機（≥70）後被玩家壓回安寧（<25）時，
//! 全城自動舉辦豐收節——全服廣播、在線玩家各得乙太、生態面板亮起金色慶典橫幅。
//!
//! 設計動機：先前的生態功能（Alpha 覺醒、霸主、跨族結盟）幾乎都讓壓力「往上」，
//! 唯一降壓路徑（清剿委託）完成後沒有「世界回歸和平」的正向回饋。本模組補上閉環：
//! 玩家齊力把危機壓回安寧 → 親眼看到城鎮歡慶、拿到獎勵，讓「降壓」終於有感、有甜頭。
//!
//! 成本紀律：
//! - 純罐頭廣播，零 LLM，零額外費用。
//! - 「先武裝（曾達危機）、後觸發（跌回安寧）」+ 30 分鐘冷卻，雙重防刷屏。
//! - 純記憶體模式，重啟清零，零 migration，不破壞玩家資料。

/// 「武裝」所需的生態壓力高水位：壓力曾達此值，代表世界真的歷經過危機，
/// 之後跌回安寧才值得慶祝（避免一直低壓時亂發節慶）。
pub const ARM_PRESSURE: f32 = 70.0;

/// 觸發豐收節的安寧門檻：已武裝狀態下壓力跌破此值即開節。
pub const PEACE_PRESSURE: f32 = 25.0;

/// 豐收節持續時間（秒）：3 分鐘。
pub const FESTIVAL_DURATION_SECS: f32 = 180.0;

/// 觸發後至下次允許觸發的冷卻（秒）：30 分鐘。
pub const COOLDOWN_SECS: f32 = 1800.0;

/// 開節時所有在線玩家各得乙太。
pub const REWARD_PER_PLAYER: u32 = 15;

// ─────────────────────────────────────────────
// 資料結構
// ─────────────────────────────────────────────

/// 一場進行中的豐收節資料。
#[derive(Debug, Clone)]
pub struct ActiveFestival {
    /// 剩餘慶典時間（秒）。
    pub lifetime: f32,
    /// 本場慶典每位在線玩家獲得的乙太（記錄供前端顯示）。
    pub reward_per_player: u32,
}

/// `tick()` 可回傳的豐收節事件，由 game.rs 消化後廣播 / 發獎。
#[derive(Debug, Clone, PartialEq)]
pub enum EcoFestivalEvent {
    /// 豐收節開始（需廣播並發給在線玩家乙太）。
    Started { reward_per_player: u32 },
    /// 豐收節落幕（可選擇廣播）。
    Ended,
}

/// 生態豐收節管理器（純記憶體，重啟清零）。
pub struct EcoFestivalState {
    /// 當前進行中的慶典（無慶典時為 None）。
    pub active: Option<ActiveFestival>,
    /// 是否已「武裝」：壓力曾達 ARM_PRESSURE，安寧後即可開節。
    armed: bool,
    /// 距下次允許觸發的倒數（秒）。
    cooldown: f32,
}

impl Default for EcoFestivalState {
    fn default() -> Self {
        Self {
            active: None,
            // 啟動時未武裝：必須先親歷一次壓力衝高，才會在事後安寧時慶祝。
            armed: false,
            cooldown: 0.0,
        }
    }
}

impl EcoFestivalState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 慶典是否正在進行中。
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// 是否已武裝（測試 / 內省用）。
    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// 每 tick 推進計時器與狀態機。
    ///
    /// - `dt`: 本幀秒數
    /// - `eco_pressure`: 當前生態壓力（0.0-100.0）
    /// - `invasion_active`: 是否有活躍入侵（入侵中不開節，避免違和）
    ///
    /// 回傳 `Some(event)` 代表有需要廣播 / 發獎的事件。
    pub fn tick(
        &mut self,
        dt: f32,
        eco_pressure: f32,
        invasion_active: bool,
    ) -> Option<EcoFestivalEvent> {
        // 1. 推進進行中的慶典；時間到則落幕。
        if let Some(ref mut f) = self.active {
            f.lifetime -= dt;
            if f.lifetime <= 0.0 {
                self.active = None;
                self.cooldown = COOLDOWN_SECS;
                return Some(EcoFestivalEvent::Ended);
            }
            // 慶典進行中不重複觸發。
            return None;
        }

        // 2. 推進冷卻。
        if self.cooldown > 0.0 {
            self.cooldown -= dt;
        }

        // 3. 武裝：壓力衝上危機水位 → 記下「世界歷經過危機」。
        if eco_pressure >= ARM_PRESSURE {
            self.armed = true;
        }

        // 4. 觸發：已武裝、壓力跌回安寧、無入侵、冷卻結束 → 開節。
        if self.armed
            && eco_pressure < PEACE_PRESSURE
            && !invasion_active
            && self.cooldown <= 0.0
        {
            self.armed = false; // 重置武裝，須再次衝高才能再開節
            self.cooldown = COOLDOWN_SECS;
            self.active = Some(ActiveFestival {
                lifetime: FESTIVAL_DURATION_SECS,
                reward_per_player: REWARD_PER_PLAYER,
            });
            return Some(EcoFestivalEvent::Started {
                reward_per_player: REWARD_PER_PLAYER,
            });
        }

        None
    }

    /// 回傳供快照廣播的視圖（無進行中慶典時為 None）。
    pub fn view(&self) -> Option<EcoFestivalView> {
        self.active.as_ref().map(|f| EcoFestivalView {
            time_left_secs: f.lifetime.max(0.0) as u32,
            reward_per_player: f.reward_per_player,
        })
    }
}

// ─────────────────────────────────────────────
// 快照視圖（供前端）
// ─────────────────────────────────────────────

/// 供快照廣播的豐收節視圖。
#[derive(Debug, Clone, serde::Serialize)]
pub struct EcoFestivalView {
    /// 剩餘時間（秒，取整）。
    pub time_left_secs: u32,
    /// 每位在線玩家獲得的乙太。
    pub reward_per_player: u32,
}

// ─────────────────────────────────────────────
// 廣播文案（面向玩家字串集中於此，方便日後 i18n）
// ─────────────────────────────────────────────

/// 開節廣播文案。
pub fn started_text(reward_per_player: u32) -> String {
    format!(
        "🌾【生態豐收節】野外生態回歸安寧！全城歡慶，在線玩家各得 {} 乙太！",
        reward_per_player
    )
}

/// 落幕廣播文案。
pub fn ended_text() -> String {
    "🌾 豐收節落幕——願這份安寧長存。".to_string()
}

// ─────────────────────────────────────────────
// 單元測試
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 從未經歷危機（未武裝）時，即使壓力很低也不開節。
    #[test]
    fn low_pressure_without_arming_does_not_trigger() {
        let mut s = EcoFestivalState::new();
        // 一路低壓，從未達 ARM_PRESSURE。
        for _ in 0..100 {
            assert_eq!(s.tick(1.0, 10.0, false), None);
        }
        assert!(!s.is_armed());
        assert!(!s.is_active());
    }

    /// 壓力衝高會武裝，但只要還高就不開節。
    #[test]
    fn high_pressure_arms_but_does_not_trigger_while_high() {
        let mut s = EcoFestivalState::new();
        assert_eq!(s.tick(1.0, 85.0, false), None);
        assert!(s.is_armed());
        // 仍高壓，不開節。
        assert_eq!(s.tick(1.0, 80.0, false), None);
        assert!(!s.is_active());
    }

    /// 武裝後壓力跌回安寧即開節，並回傳 Started 與正確獎勵。
    #[test]
    fn arm_then_peace_triggers_festival() {
        let mut s = EcoFestivalState::new();
        s.tick(1.0, 90.0, false); // 武裝
        let ev = s.tick(1.0, 20.0, false); // 安寧 → 開節
        assert_eq!(
            ev,
            Some(EcoFestivalEvent::Started {
                reward_per_player: REWARD_PER_PLAYER
            })
        );
        assert!(s.is_active());
        assert!(!s.is_armed()); // 開節後武裝重置
    }

    /// 剛好等於 ARM_PRESSURE 也算武裝（>= 邊界）。
    #[test]
    fn exact_arm_threshold_arms() {
        let mut s = EcoFestivalState::new();
        s.tick(1.0, ARM_PRESSURE, false);
        assert!(s.is_armed());
    }

    /// 剛好等於 PEACE_PRESSURE 不算安寧（< 邊界），不開節。
    #[test]
    fn exact_peace_threshold_does_not_trigger() {
        let mut s = EcoFestivalState::new();
        s.tick(1.0, 90.0, false); // 武裝
        assert_eq!(s.tick(1.0, PEACE_PRESSURE, false), None);
        assert!(!s.is_active());
        assert!(s.is_armed()); // 仍武裝，等真正跌破
    }

    /// 入侵進行中不開節（即使已武裝且安寧）。
    #[test]
    fn invasion_blocks_trigger() {
        let mut s = EcoFestivalState::new();
        s.tick(1.0, 90.0, false); // 武裝
        assert_eq!(s.tick(1.0, 10.0, true), None); // 入侵中
        assert!(!s.is_active());
        assert!(s.is_armed()); // 武裝保留，入侵結束後仍可開節
    }

    /// 慶典在持續時間後落幕，回傳 Ended。
    #[test]
    fn festival_ends_after_duration() {
        let mut s = EcoFestivalState::new();
        s.tick(1.0, 90.0, false);
        s.tick(1.0, 10.0, false); // 開節
        assert!(s.is_active());
        // 推進到剛好超過持續時間。
        let mut ended = false;
        for _ in 0..(FESTIVAL_DURATION_SECS as u32 + 2) {
            if s.tick(1.0, 10.0, false) == Some(EcoFestivalEvent::Ended) {
                ended = true;
                break;
            }
        }
        assert!(ended);
        assert!(!s.is_active());
    }

    /// 落幕後冷卻期間即使再次武裝+安寧也不立刻再開節。
    #[test]
    fn cooldown_prevents_immediate_refire() {
        let mut s = EcoFestivalState::new();
        s.tick(1.0, 90.0, false);
        s.tick(1.0, 10.0, false); // 第一次開節
                                  // 跑完整場慶典直到落幕。
        loop {
            if s.tick(1.0, 10.0, false) == Some(EcoFestivalEvent::Ended) {
                break;
            }
        }
        // 立刻再武裝 + 安寧，但仍在冷卻內 → 不開節。
        s.tick(1.0, 90.0, false);
        assert_eq!(s.tick(1.0, 10.0, false), None);
        assert!(!s.is_active());
        assert!(s.is_armed()); // 武裝就緒，等冷卻過。
    }

    /// 冷卻結束後（再次經歷危機→安寧）可再次開節。
    #[test]
    fn refires_after_cooldown() {
        let mut s = EcoFestivalState::new();
        s.tick(1.0, 90.0, false);
        s.tick(1.0, 10.0, false); // 第一次開節
        loop {
            if s.tick(1.0, 10.0, false) == Some(EcoFestivalEvent::Ended) {
                break;
            }
        }
        // 燒掉冷卻（保持低壓，但需重新武裝；此處模擬「之後又一波危機」）。
        s.tick(COOLDOWN_SECS, 90.0, false); // 一次大步進燒掉冷卻並重新武裝
        let ev = s.tick(1.0, 10.0, false);
        assert_eq!(
            ev,
            Some(EcoFestivalEvent::Started {
                reward_per_player: REWARD_PER_PLAYER
            })
        );
    }

    /// 慶典進行中 view() 回傳剩餘時間與獎勵；無慶典時為 None。
    #[test]
    fn view_reflects_active_state() {
        let mut s = EcoFestivalState::new();
        assert!(s.view().is_none());
        s.tick(1.0, 90.0, false);
        s.tick(1.0, 10.0, false); // 開節
        let v = s.view().expect("慶典中應有視圖");
        assert_eq!(v.reward_per_player, REWARD_PER_PLAYER);
        assert!(v.time_left_secs <= FESTIVAL_DURATION_SECS as u32);
        assert!(v.time_left_secs > 0);
    }

    /// 武裝需要「曾達危機」這件事不會因短暫低壓而清除（armed 一旦點亮就保留到開節）。
    #[test]
    fn arming_persists_through_intermediate_pressure() {
        let mut s = EcoFestivalState::new();
        s.tick(1.0, 90.0, false); // 武裝
                                  // 中等壓力來回，不開節也不清除武裝。
        s.tick(1.0, 50.0, false);
        s.tick(1.0, 40.0, false);
        assert!(s.is_armed());
        assert!(!s.is_active());
        // 終於跌破安寧 → 開節。
        assert!(matches!(
            s.tick(1.0, 10.0, false),
            Some(EcoFestivalEvent::Started { .. })
        ));
    }

    /// 廣播文案包含獎勵數字與關鍵字。
    #[test]
    fn texts_contain_keywords() {
        let t = started_text(15);
        assert!(t.contains("豐收節"));
        assert!(t.contains("15"));
        assert!(ended_text().contains("落幕"));
    }
}
