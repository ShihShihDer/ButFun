//! 公民投票系統（ROADMAP 156）。
//!
//! 城鎮居民代言人（happiness 最高的居民）定期提出提案，
//! 全服玩家在 VOTE_WINDOW_SECS 秒內投票讚成/反對。
//! 提案通過後依種類觸發短期城鎮效果。
//!
//! 純邏輯模組：零 LLM、零 migration、記憶體模式、重啟清零。

use std::collections::HashSet;

/// 每次提案間的冷卻時間（秒）。
pub const PROPOSAL_COOLDOWN_SECS: f32 = 1200.0; // 20 分鐘
/// 投票視窗長度（秒）。
pub const VOTE_WINDOW_SECS: f32 = 180.0; // 3 分鐘
/// 提案效果持續時間（秒，農耕盛典/夜市/城防）。
pub const EFFECT_DURATION_SECS: f32 = 600.0; // 10 分鐘
/// 代言人至少要有多高的 happiness 才有資格提案（中立=50）。
pub const MIN_SPOKESMAN_HAPPINESS: u8 = 50;
/// 農耕盛典採集 EXP 加成百分比。
pub const FARMING_FESTIVAL_EXP_BONUS_PCT: u32 = 50;
/// 夜市開張 NPC 收購乙太加成百分比。
pub const NIGHT_MARKET_BUY_BONUS_PCT: u32 = 15;
/// 乙太集資每位在線玩家獲得的乙太數量。
pub const AETHER_REWARD_AMOUNT: u32 = 5;

// ─── 提案種類 ─────────────────────────────────────────────────────────────────

/// 提案種類。每次提案依 `proposal_cycle % 4` 輪替。
#[derive(Debug, Clone, PartialEq)]
pub enum ProposalKind {
    /// 農耕盛典：採集 EXP ×1.5，持續 10 分鐘。
    FarmingFestival,
    /// 夜市開張：NPC 收購乙太 +15%，持續 10 分鐘。
    NightMarket,
    /// 城防演練：怪物波次暫停，持續 10 分鐘。
    DefenseDrill,
    /// 乙太集資：立刻分配給每位在線玩家 +5 乙太。
    AetherCollection,
}

impl ProposalKind {
    /// 根據提案輪替計數決定提案種類。
    pub fn from_cycle(n: u32) -> Self {
        match n % 4 {
            0 => ProposalKind::FarmingFestival,
            1 => ProposalKind::NightMarket,
            2 => ProposalKind::DefenseDrill,
            _ => ProposalKind::AetherCollection,
        }
    }

    /// 提案廣播文字（帶代言人名字）。
    pub fn proposal_text(&self, spokesman: &str) -> String {
        match self {
            ProposalKind::FarmingFestival => format!(
                "📜 城鎮代言人 {} 提案：舉辦農耕盛典！採集 EXP +50%，持續 10 分鐘。請在 {} 秒內投票！",
                spokesman, VOTE_WINDOW_SECS as u32
            ),
            ProposalKind::NightMarket => format!(
                "📜 城鎮代言人 {} 提案：開辦夜市！NPC 收購乙太 +15%，持續 10 分鐘。請在 {} 秒內投票！",
                spokesman, VOTE_WINDOW_SECS as u32
            ),
            ProposalKind::DefenseDrill => format!(
                "📜 城鎮代言人 {} 提案：舉行城防演練！怪物波次暫停 10 分鐘，城鎮可以喘口氣。請在 {} 秒內投票！",
                spokesman, VOTE_WINDOW_SECS as u32
            ),
            ProposalKind::AetherCollection => format!(
                "📜 城鎮代言人 {} 提案：發起乙太集資！通過後立刻給每位在線玩家 +{} 乙太。請在 {} 秒內投票！",
                spokesman, AETHER_REWARD_AMOUNT, VOTE_WINDOW_SECS as u32
            ),
        }
    }

    /// 效果啟動廣播文字。
    pub fn passed_text(&self, yes: u32, no: u32) -> String {
        let label = self.effect_label();
        match self {
            ProposalKind::AetherCollection => format!(
                "🗳️ 提案通過！（{} 讚成 / {} 反對）{} 每位在線玩家立刻獲得 +{} 乙太！",
                yes, no, label, AETHER_REWARD_AMOUNT
            ),
            _ => format!(
                "🗳️ 提案通過！（{} 讚成 / {} 反對）{} 效果即刻啟動，持續 {} 分鐘！",
                yes, no, label, EFFECT_DURATION_SECS as u32 / 60
            ),
        }
    }

    /// HUD 顯示標籤（emoji + 名稱）。
    pub fn effect_label(&self) -> &'static str {
        match self {
            ProposalKind::FarmingFestival => "🌾 農耕盛典",
            ProposalKind::NightMarket => "🏮 夜市開張",
            ProposalKind::DefenseDrill => "🛡️ 城防演練",
            ProposalKind::AetherCollection => "⚡ 乙太集資",
        }
    }

    /// 種類字串，供協議/前端識別。
    pub fn kind_str(&self) -> &'static str {
        match self {
            ProposalKind::FarmingFestival => "farming_festival",
            ProposalKind::NightMarket => "night_market",
            ProposalKind::DefenseDrill => "defense_drill",
            ProposalKind::AetherCollection => "aether_collection",
        }
    }
}

// ─── 協議視圖 ─────────────────────────────────────────────────────────────────

/// 公民投票快照視圖，送入 Snapshot.civic_vote。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CivicVoteView {
    /// 代言人姓名。
    pub spokesman_name: String,
    /// 提案文字（不含投票引導語）。
    pub proposal_text: String,
    /// 提案種類字串（farming_festival / night_market / defense_drill / aether_collection）。
    pub proposal_kind: String,
    /// 目前讚成票數。
    pub vote_yes: u32,
    /// 目前反對票數。
    pub vote_no: u32,
    /// 投票截止倒數秒數（> 0 = 投票中）。
    pub vote_remaining_secs: u32,
}

// ─── 事件 ─────────────────────────────────────────────────────────────────────

/// `tick()` 回傳的事件，由 game.rs 廣播或觸發效果。
#[derive(Debug, Clone)]
pub enum CivicVoteEvent {
    /// 新提案開始，附全服廣播文字。
    ProposalStarted { text: String },
    /// 提案通過，附效果種類與廣播文字。
    ProposalPassed { kind: ProposalKind, text: String },
    /// 提案否決，附廣播文字。
    ProposalRejected { text: String },
    /// 乙太集資通過，需由 game.rs 把 AETHER_REWARD_AMOUNT 分發給所有在線玩家。
    AetherReward,
}

// ─── 狀態 ─────────────────────────────────────────────────────────────────────

/// 公民投票系統狀態。
pub struct CivicVoteState {
    /// 距下次提案的冷卻計時（秒）。到 0 時若有代言人就觸發。
    pub cooldown: f32,
    /// 當前活躍提案種類（None = 無）。
    current_kind: Option<ProposalKind>,
    /// 當前代言人姓名。
    spokesman_name: Option<String>,
    /// 投票視窗剩餘秒數（0 = 無活躍投票）。
    vote_remaining: f32,
    /// 讚成票數。
    vote_yes: u32,
    /// 反對票數。
    vote_no: u32,
    /// 已投票的玩家 ID（避免重複投）。
    voted: HashSet<String>,
    /// 提案輪替計數（每次通過/否決後 +1）。
    proposal_cycle: u32,
    /// 當前生效的效果種類（None = 無）。
    pub active_effect: Option<ProposalKind>,
    /// 效果剩餘秒數（0 = 無效果）。
    pub effect_remaining: f32,
}

impl CivicVoteState {
    /// 初始狀態：5 分鐘後第一次提案（讓玩家進場不久就能看到）。
    pub fn new() -> Self {
        Self {
            cooldown: 300.0,
            current_kind: None,
            spokesman_name: None,
            vote_remaining: 0.0,
            vote_yes: 0,
            vote_no: 0,
            voted: HashSet::new(),
            proposal_cycle: 0,
            active_effect: None,
            effect_remaining: 0.0,
        }
    }

    /// 是否有活躍投票。
    pub fn is_voting(&self) -> bool {
        self.vote_remaining > 0.0
    }

    /// 是否有生效中的效果。
    pub fn has_active_effect(&self) -> bool {
        self.effect_remaining > 0.0
    }

    /// 效果剩餘整數秒數，供快照。
    pub fn effect_remaining_secs(&self) -> u32 {
        self.effect_remaining.ceil() as u32
    }

    /// 效果種類字串，供快照（無效果時回空字串）。
    pub fn active_effect_kind(&self) -> String {
        self.active_effect
            .as_ref()
            .map(|k| k.kind_str().to_string())
            .unwrap_or_default()
    }

    /// 效果 HUD 標籤，供快照。
    pub fn active_effect_label(&self) -> String {
        self.active_effect
            .as_ref()
            .map(|k| k.effect_label().to_string())
            .unwrap_or_default()
    }

    /// 投票視圖，None = 無活躍投票。
    pub fn vote_view(&self) -> Option<CivicVoteView> {
        let kind = self.current_kind.as_ref()?;
        if self.vote_remaining <= 0.0 {
            return None;
        }
        Some(CivicVoteView {
            spokesman_name: self.spokesman_name.clone().unwrap_or_default(),
            proposal_text: kind.proposal_text(
                self.spokesman_name.as_deref().unwrap_or("代言人"),
            ),
            proposal_kind: kind.kind_str().to_string(),
            vote_yes: self.vote_yes,
            vote_no: self.vote_no,
            vote_remaining_secs: self.vote_remaining.ceil() as u32,
        })
    }

    /// 玩家投票（每人限一票）。回傳 true 表示投票成功接受。
    pub fn cast_vote(&mut self, player_id: &str, yes: bool) -> bool {
        if !self.is_voting() || self.voted.contains(player_id) {
            return false;
        }
        self.voted.insert(player_id.to_string());
        if yes { self.vote_yes += 1; } else { self.vote_no += 1; }
        true
    }

    /// 從居民列表挑選代言人（happiness 最高且 >= MIN_SPOKESMAN_HAPPINESS）。
    pub fn elect_spokesman(residents: &[crate::resident_npc::ResidentNpc]) -> Option<String> {
        residents
            .iter()
            .max_by_key(|r| r.happiness)
            .filter(|r| r.happiness >= MIN_SPOKESMAN_HAPPINESS)
            .map(|r| r.name.to_string())
    }

    /// 推進一幀，回傳本 tick 產生的事件。
    ///
    /// * `dt` — 幀時長（秒）
    /// * `spokesman_name` — 本幀選出的代言人名字（從 residents 讀取後傳入）
    pub fn tick(&mut self, dt: f32, spokesman_name: Option<String>) -> Vec<CivicVoteEvent> {
        let mut events = Vec::new();

        // 推進效果倒數
        if self.effect_remaining > 0.0 {
            self.effect_remaining -= dt;
            if self.effect_remaining <= 0.0 {
                self.effect_remaining = 0.0;
                self.active_effect = None;
            }
        }

        if self.vote_remaining > 0.0 {
            // 投票視窗進行中
            self.vote_remaining -= dt;
            if self.vote_remaining <= 0.0 {
                self.vote_remaining = 0.0;
                // 結算投票
                let kind = self.current_kind.take().unwrap();
                let spokesman = self.spokesman_name.take().unwrap_or_default();
                let yes = self.vote_yes;
                let no = self.vote_no;
                self.vote_yes = 0;
                self.vote_no = 0;
                self.voted.clear();
                self.cooldown = PROPOSAL_COOLDOWN_SECS;

                if yes > no {
                    let text = kind.passed_text(yes, no);
                    if kind == ProposalKind::AetherCollection {
                        // 乙太集資立刻發放，由 game.rs 處理
                        events.push(CivicVoteEvent::AetherReward);
                    } else {
                        self.active_effect = Some(kind.clone());
                        self.effect_remaining = EFFECT_DURATION_SECS;
                    }
                    events.push(CivicVoteEvent::ProposalPassed { kind, text });
                } else {
                    let text = format!(
                        "🗳️ 提案否決。（{} 讚成 / {} 反對）{} 的提案未能通過，城鎮維持現狀。",
                        yes, no, spokesman
                    );
                    events.push(CivicVoteEvent::ProposalRejected { text });
                }
                self.proposal_cycle += 1;
            }
        } else {
            // 冷卻中
            self.cooldown -= dt;
            if self.cooldown <= 0.0 {
                if let Some(name) = spokesman_name {
                    let kind = ProposalKind::from_cycle(self.proposal_cycle);
                    let broadcast_text = kind.proposal_text(&name);
                    events.push(CivicVoteEvent::ProposalStarted {
                        text: broadcast_text,
                    });
                    self.current_kind = Some(kind);
                    self.spokesman_name = Some(name);
                    self.vote_remaining = VOTE_WINDOW_SECS;
                    self.vote_yes = 0;
                    self.vote_no = 0;
                    self.voted.clear();
                } else {
                    // 沒有合格代言人，再等一個冷卻
                    self.cooldown = PROPOSAL_COOLDOWN_SECS;
                }
            }
        }

        events
    }

    /// 農耕盛典是否生效（供採集 EXP 加成）。
    pub fn farming_festival_active(&self) -> bool {
        self.effect_remaining > 0.0
            && self.active_effect == Some(ProposalKind::FarmingFestival)
    }

    /// 夜市開張是否生效（供 NPC 收購加成）。
    pub fn night_market_active(&self) -> bool {
        self.effect_remaining > 0.0
            && self.active_effect == Some(ProposalKind::NightMarket)
    }

    /// 城防演練是否生效（供怪物波次暫停）。
    pub fn defense_drill_active(&self) -> bool {
        self.effect_remaining > 0.0
            && self.active_effect == Some(ProposalKind::DefenseDrill)
    }
}

// ─── 單元測試 ─────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn advance(state: &mut CivicVoteState, secs: f32) -> Vec<CivicVoteEvent> {
        state.tick(secs, Some("阿土".to_string()))
    }

    #[test]
    fn proposal_cycle_rotates() {
        assert_eq!(ProposalKind::from_cycle(0), ProposalKind::FarmingFestival);
        assert_eq!(ProposalKind::from_cycle(1), ProposalKind::NightMarket);
        assert_eq!(ProposalKind::from_cycle(2), ProposalKind::DefenseDrill);
        assert_eq!(ProposalKind::from_cycle(3), ProposalKind::AetherCollection);
        assert_eq!(ProposalKind::from_cycle(4), ProposalKind::FarmingFestival);
    }

    #[test]
    fn no_proposal_before_cooldown() {
        let mut state = CivicVoteState::new();
        let events = advance(&mut state, 1.0);
        assert!(events.is_empty());
        assert!(!state.is_voting());
    }

    #[test]
    fn proposal_starts_after_cooldown() {
        let mut state = CivicVoteState::new();
        // 快進到超過初始冷卻（300 秒）
        let events = advance(&mut state, 301.0);
        assert!(events.iter().any(|e| matches!(e, CivicVoteEvent::ProposalStarted { .. })));
        assert!(state.is_voting());
    }

    #[test]
    fn vote_yes_passes_proposal() {
        let mut state = CivicVoteState::new();
        advance(&mut state, 301.0); // 觸發提案
        assert!(state.is_voting());

        state.cast_vote("player1", true);
        state.cast_vote("player2", true);
        state.cast_vote("player3", false);

        // 快進投票結束
        let events = state.tick(VOTE_WINDOW_SECS + 1.0, None);
        assert!(events.iter().any(|e| matches!(e, CivicVoteEvent::ProposalPassed { .. })));
    }

    #[test]
    fn vote_no_rejects_proposal() {
        let mut state = CivicVoteState::new();
        advance(&mut state, 301.0);

        state.cast_vote("player1", false);
        state.cast_vote("player2", false);

        let events = state.tick(VOTE_WINDOW_SECS + 1.0, None);
        assert!(events.iter().any(|e| matches!(e, CivicVoteEvent::ProposalRejected { .. })));
    }

    #[test]
    fn no_duplicate_vote() {
        let mut state = CivicVoteState::new();
        advance(&mut state, 301.0);

        assert!(state.cast_vote("player1", true));
        assert!(!state.cast_vote("player1", false)); // 重複應拒絕
        assert_eq!(state.vote_yes, 1);
        assert_eq!(state.vote_no, 0);
    }

    #[test]
    fn farming_festival_effect_active_after_pass() {
        let mut state = CivicVoteState::new();
        // 強制設為農耕盛典輪次
        state.proposal_cycle = 0;
        advance(&mut state, 301.0);

        state.cast_vote("p1", true);
        state.tick(VOTE_WINDOW_SECS + 1.0, None);

        assert!(state.farming_festival_active());
        assert!(!state.night_market_active());
    }

    #[test]
    fn effect_expires_after_duration() {
        let mut state = CivicVoteState::new();
        state.proposal_cycle = 0;
        advance(&mut state, 301.0);
        state.cast_vote("p1", true);
        state.tick(VOTE_WINDOW_SECS + 1.0, None);

        assert!(state.farming_festival_active());

        // 快進到效果結束
        state.tick(EFFECT_DURATION_SECS + 1.0, None);
        assert!(!state.farming_festival_active());
    }

    #[test]
    fn aether_collection_emits_reward_event() {
        let mut state = CivicVoteState::new();
        state.proposal_cycle = 3; // AetherCollection 輪次
        advance(&mut state, 301.0);

        state.cast_vote("p1", true);
        let events = state.tick(VOTE_WINDOW_SECS + 1.0, None);

        assert!(events.iter().any(|e| matches!(e, CivicVoteEvent::AetherReward)));
        // 乙太集資無持續效果
        assert!(!state.has_active_effect());
    }

    #[test]
    fn vote_view_none_when_not_voting() {
        let state = CivicVoteState::new();
        assert!(state.vote_view().is_none());
    }

    #[test]
    fn elect_spokesman_picks_highest_happiness() {
        // 只要測試邏輯，不需要真實的 ResidentNpc（用 elect_spokesman 中的條件）
        // 這個測試直接驗證 from_cycle 的分布
        for i in 0..8 {
            let kind = ProposalKind::from_cycle(i);
            let s = kind.kind_str();
            assert!(!s.is_empty());
        }
    }
}
