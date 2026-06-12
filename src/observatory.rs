//! 天文台星象預報（ROADMAP 132）。
//!
//! 蒸汽天文台竣工後，每個遊戲黎明（Night/Dusk → Dawn 轉換）廣播一次「今日星象」，
//! 並啟用一種持續 10 分鐘的全服短期加成。四種加成循環：
//!
//! - `ExpBoost`：吉星高照，採集/戰鬥 EXP +25%。
//! - `TravelDiscount`：星際順風，旅行費 -10 乙太。
//! - `GatherExtra`：豐收星象，採集每次 +1 物品。
//! - `NpcBonus`：金星入市，NPC 收購 +15%。
//!
//! 成本紀律：
//! - 僅在 `TownProjectStatus::Completed` 後啟用；未完工時靜默不觸發。
//! - 全局 Semaphore(1)，防並發 LLM 呼叫。
//! - 降級鏈：Groq → ollama → 罐頭（25 字以內）。
//! - `BUTFUN_NPC_LLM=1` 未設定時直接回罐頭，不呼叫任何外部 API。
//! - 純記憶體模式，重啟清零，零 migration。

use crate::daynight::Phase;

/// 星象預報加成類型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StarForecastBonus {
    /// ⭐ 吉星高照：採集/戰鬥 EXP +25%。
    ExpBoost,
    /// 🌬️ 星際順風：旅行費 -10 乙太。
    TravelDiscount,
    /// 🌾 豐收星象：採集每次 +1 物品。
    GatherExtra,
    /// 💰 金星入市：NPC 收購 +15%。
    NpcBonus,
}

impl StarForecastBonus {
    /// 從循環索引取得類型（輪換四種）。
    pub fn from_index(index: usize) -> Self {
        match index % 4 {
            0 => StarForecastBonus::ExpBoost,
            1 => StarForecastBonus::TravelDiscount,
            2 => StarForecastBonus::GatherExtra,
            _ => StarForecastBonus::NpcBonus,
        }
    }

    /// 加成簡短名稱（供 HUD 顯示）。
    pub fn name(&self) -> &'static str {
        match self {
            StarForecastBonus::ExpBoost => "⭐ 吉星高照 EXP+25%",
            StarForecastBonus::TravelDiscount => "🌬️ 星際順風 旅行-10✨",
            StarForecastBonus::GatherExtra => "🌾 豐收星象 採集+1",
            StarForecastBonus::NpcBonus => "💰 金星入市 收購+15%",
        }
    }

    /// 加成類型字串（供快照廣播，前端依此判斷顯示）。
    pub fn kind_str(&self) -> &'static str {
        match self {
            StarForecastBonus::ExpBoost => "exp_boost",
            StarForecastBonus::TravelDiscount => "travel_discount",
            StarForecastBonus::GatherExtra => "gather_extra",
            StarForecastBonus::NpcBonus => "npc_bonus",
        }
    }

    /// 旅行折扣乙太數量。
    pub fn travel_discount_ether() -> u32 { 10 }
    /// EXP 加成百分比（整數）。
    pub fn exp_bonus_pct() -> u32 { 25 }
    /// 採集額外物品數。
    pub fn gather_extra_qty() -> u32 { 1 }
    /// NPC 收購加成百分比（整數，疊乘基礎 100）。
    pub fn npc_bonus_pct() -> u32 { 15 }
}

/// 天文台星象預報狀態（純記憶體）。
pub struct ObservatoryState {
    /// 距下次觸發的冷卻倒數（秒）。
    pub cooldown: f32,
    /// 當前加成剩餘秒數（0 表示無活躍加成）。
    pub remaining: f32,
    /// 當前（或上次）加成類型。
    pub current_bonus: StarForecastBonus,
    /// 上一個 tick 的日夜階段，用於偵測 → Dawn 轉換。
    pub last_phase: Phase,
    /// 歷次預報計數，用於循環加成類型與罐頭語句輪換。
    pub forecast_count: usize,
}

/// 星象預報 Semaphore 容量：同時最多 1 個 AI 呼叫。
pub const MAX_CONCURRENT_CALLS: usize = 1;
/// 觸發後的冷卻時間（秒）——略長於一個完整日夜循環（~300s），確保每天僅觸發一次。
pub const FORECAST_COOLDOWN_SECS: f32 = 320.0;
/// 加成持續時間（秒）——10 分鐘。
pub const FORECAST_DURATION_SECS: f32 = 600.0;
/// 伺服器啟動後首次觸發最短等待（秒）。
const FIRST_WAIT_SECS: f32 = 120.0;

impl ObservatoryState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_WAIT_SECS,
            remaining: 0.0,
            current_bonus: StarForecastBonus::ExpBoost,
            last_phase: Phase::Night,
            forecast_count: 0,
        }
    }

    /// 目前是否有活躍的星象加成。
    pub fn is_active(&self) -> bool {
        self.remaining > 0.0
    }

    /// 活躍加成剩餘整數秒（供快照廣播）。
    pub fn remaining_secs(&self) -> u32 {
        self.remaining.ceil() as u32
    }

    /// 當前加成類型字串（供快照廣播）；無加成時回空字串。
    pub fn bonus_kind_str(&self) -> &'static str {
        if self.is_active() {
            self.current_bonus.kind_str()
        } else {
            ""
        }
    }

    /// 推進時間，回傳 `Some(bonus)` 表示本 tick 應觸發新預報。
    ///
    /// 觸發條件：
    /// 1. 天文台已竣工（`project_completed == true`）。
    /// 2. 上一個 tick 不是 Dawn，本 tick 進入 Dawn。
    /// 3. 冷卻已歸零。
    pub fn tick(
        &mut self,
        dt: f32,
        current_phase: Phase,
        project_completed: bool,
    ) -> Option<StarForecastBonus> {
        // 活躍加成倒計時。
        if self.remaining > 0.0 {
            self.remaining -= dt;
            if self.remaining < 0.0 {
                self.remaining = 0.0;
            }
        }

        // 未完工時不觸發。
        if !project_completed {
            self.last_phase = current_phase;
            return None;
        }

        self.cooldown -= dt;

        let transition_to_dawn =
            self.last_phase != Phase::Dawn && current_phase == Phase::Dawn;

        self.last_phase = current_phase;

        if transition_to_dawn && self.cooldown <= 0.0 {
            // 決定本次加成類型（循環輪換）。
            let bonus = StarForecastBonus::from_index(self.forecast_count);
            self.current_bonus = bonus;
            self.remaining = FORECAST_DURATION_SECS;
            self.cooldown = FORECAST_COOLDOWN_SECS;
            self.forecast_count = self.forecast_count.wrapping_add(1);
            Some(bonus)
        } else {
            None
        }
    }
}

impl Default for ObservatoryState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── AI 生成 ────────────────────────────────────────────────────────────────

/// 建立星象預報的 AI system prompt。
pub fn build_system_prompt(bonus: StarForecastBonus) -> String {
    let bonus_hint = match bonus {
        StarForecastBonus::ExpBoost =>
            "今日星象預示著學習與成長的大好時機，EXP 獲得提升。",
        StarForecastBonus::TravelDiscount =>
            "今日星際氣流順暢，旅行費用減少。",
        StarForecastBonus::GatherExtra =>
            "今日大地慷慨，採集有額外收穫。",
        StarForecastBonus::NpcBonus =>
            "今日商市活絡，NPC 收購意願旺盛。",
    };
    format!(
        "你是蒸汽龐克太空歌劇世界中剛落成的蒸汽天文台的星象師。\
        黎明時分，你透過銅製望遠鏡觀測星盤，向全體拓荒者廣播今日星象。\
        {bonus_hint}\
        請用 25 字以內的繁體中文說一句詩意又帶有蒸汽龐克氣息的星象預言。\
        只輸出那一句話，不要引號、前綴或額外說明。"
    )
}

/// 星象預報罐頭後備（LLM 未啟用或呼叫失敗時回傳）。
pub fn canned_forecast(bonus: StarForecastBonus, index: usize) -> &'static str {
    match bonus {
        StarForecastBonus::ExpBoost => {
            const C: &[&str] = &[
                "星盤大吉，諸天星辰匯聚，今日所學將刻入靈魂深處。",
                "七星連珠，乙太迴旋，拓荒者的智慧今日倍增。",
                "北斗高懸，知識之光普照，奮力前行吧，勇者。",
            ];
            C[index % C.len()]
        }
        StarForecastBonus::TravelDiscount => {
            const C: &[&str] = &[
                "星際乙太潮湧動，今日星艦動力充裕，旅途更為輕省。",
                "銀河氣流向外伸展，航道清晰，此刻出發事半功倍。",
                "蒸汽之風穿越虛空，今日星際旅行耗能最低。",
            ];
            C[index % C.len()]
        }
        StarForecastBonus::GatherExtra => {
            const C: &[&str] = &[
                "大地乙太豐沛，礦脈與田野皆大方吐露，今日收穫加倍。",
                "星土感應共鳴，拓荒者的雙手今日特別靈巧。",
                "豐收之星升東天，一切採集皆有額外饋贈。",
            ];
            C[index % C.len()]
        }
        StarForecastBonus::NpcBonus => {
            const C: &[&str] = &[
                "商星閃耀，市場之神青睞今日，帶去貨物必得善價。",
                "金星入市，收購之門大開，今日賣出收益豐厚。",
                "乙太匯流入市井，商人荷包寬裕，今日出售必有好價。",
            ];
            C[index % C.len()]
        }
    }
}

static FORECAST_COUNT_CANNED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// 非同步生成星象預報文字（Groq → ollama → 罐頭降級鏈）。
pub async fn generate_forecast(bonus: StarForecastBonus) -> String {
    let idx = FORECAST_COUNT_CANNED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let system = build_system_prompt(bonus);
    let user = "請說今日的星象預言。";
    match crate::npc_chat::raw_llm_call(&system, user).await {
        Some(text) => {
            let t = text.trim().to_string();
            if t.is_empty() {
                canned_forecast(bonus, idx).to_string()
            } else {
                t
            }
        }
        None => canned_forecast(bonus, idx).to_string(),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daynight::Phase;

    fn make_state(cooldown: f32, remaining: f32) -> ObservatoryState {
        ObservatoryState {
            cooldown,
            remaining,
            current_bonus: StarForecastBonus::ExpBoost,
            last_phase: Phase::Night,
            forecast_count: 0,
        }
    }

    #[test]
    fn new_state_not_active() {
        let s = ObservatoryState::new();
        assert!(!s.is_active());
    }

    #[test]
    fn does_not_trigger_if_not_completed() {
        let mut s = make_state(-1.0, 0.0);
        let result = s.tick(1.0, Phase::Dawn, false);
        assert!(result.is_none(), "未完工時不應觸發預報");
    }

    #[test]
    fn does_not_trigger_outside_dawn_transition() {
        // 上次也是 Dawn，沒有轉換。
        let mut s = make_state(-1.0, 0.0);
        s.last_phase = Phase::Dawn;
        let result = s.tick(1.0, Phase::Dawn, true);
        assert!(result.is_none(), "沒有 Dawn 轉換時不觸發");
    }

    #[test]
    fn does_not_trigger_before_cooldown_expires() {
        let mut s = make_state(100.0, 0.0);
        s.last_phase = Phase::Night;
        let result = s.tick(1.0, Phase::Dawn, true);
        assert!(result.is_none(), "冷卻未結束時不觸發");
    }

    #[test]
    fn triggers_on_night_to_dawn_after_cooldown() {
        let mut s = make_state(-1.0, 0.0);
        s.last_phase = Phase::Night;
        let result = s.tick(0.1, Phase::Dawn, true);
        assert!(result.is_some(), "Night→Dawn 且冷卻結束後應觸發");
        assert!(s.is_active(), "觸發後應有活躍加成");
    }

    #[test]
    fn bonus_cycles_through_four_types() {
        let mut s = make_state(-1.0, 0.0);
        let types = [
            StarForecastBonus::ExpBoost,
            StarForecastBonus::TravelDiscount,
            StarForecastBonus::GatherExtra,
            StarForecastBonus::NpcBonus,
        ];
        for expected in &types {
            s.last_phase = Phase::Night;
            s.cooldown = -1.0;
            let bonus = s.tick(0.1, Phase::Dawn, true).unwrap();
            assert_eq!(bonus, *expected);
            s.remaining = 0.0; // 手動清空，不等倒數
        }
    }

    #[test]
    fn remaining_secs_rounds_up() {
        let s = make_state(0.0, 9.3);
        assert_eq!(s.remaining_secs(), 10);
    }

    #[test]
    fn active_bonus_counts_down() {
        let mut s = make_state(0.0, 5.0);
        assert!(s.is_active());
        s.tick(10.0, Phase::Day, true);
        assert!(!s.is_active(), "加成應在倒數後結束");
        assert_eq!(s.remaining_secs(), 0);
    }

    #[test]
    fn resets_cooldown_after_trigger() {
        let mut s = make_state(-1.0, 0.0);
        s.last_phase = Phase::Night;
        s.tick(0.1, Phase::Dawn, true);
        assert!(s.cooldown > 0.0, "觸發後冷卻應重設");
    }

    #[test]
    fn bonus_kind_str_empty_when_inactive() {
        let s = ObservatoryState::new();
        assert_eq!(s.bonus_kind_str(), "");
    }

    #[test]
    fn canned_forecast_non_empty_for_all_types_and_indices() {
        let types = [
            StarForecastBonus::ExpBoost,
            StarForecastBonus::TravelDiscount,
            StarForecastBonus::GatherExtra,
            StarForecastBonus::NpcBonus,
        ];
        for bonus in &types {
            for i in 0..6 {
                let msg = canned_forecast(*bonus, i);
                assert!(!msg.is_empty(), "罐頭預報不應為空：{bonus:?} index {i}");
            }
        }
    }

    #[test]
    fn build_system_prompt_contains_bonus_hint() {
        let prompt = build_system_prompt(StarForecastBonus::TravelDiscount);
        assert!(prompt.contains("旅行費用"), "系統提示應包含旅行折扣相關文字");
    }
}
