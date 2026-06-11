//! 怪物王戰術指揮（ROADMAP 117）。
//!
//! 菁英精英（notorious enemy，level ≥ base_level+3）每 `DECISION_COOLDOWN_SECS` 秒
//! 決定一次戰術，並命令附近小怪執行：包圍、集火、撤退、集結。
//!
//! 戰術本身由罐頭邏輯即時決定（保證零延遲），AI（Groq→ollama→罐頭）只用於生成廣播台詞。
//!
//! 成本紀律：
//! - 每隻菁英最多每 `DECISION_COOLDOWN_SECS` 秒決策一次。
//! - 全局獨立 Semaphore（容量 1），防止多 boss 同時燒 LLM。
//! - `BUTFUN_NPC_LLM=1` 未設定時直接使用罐頭台詞，不呼叫外部 API。

use std::collections::{HashMap, HashSet};

/// 每隻菁英決策的冷卻（秒）。
pub const DECISION_COOLDOWN_SECS: f32 = 90.0;

/// 伺服器啟動後首次決策的最短等待（秒）。
const FIRST_DECISION_WAIT_SECS: f32 = 75.0;

/// boss_ai 專屬 Semaphore 容量：同時最多 1 個 AI 台詞呼叫。
pub const MAX_CONCURRENT_DECISIONS: usize = 1;

/// 命令波及半徑（像素）：boss 附近這個範圍內的小怪受到戰術影響。
pub const COMMAND_RADIUS: f32 = 500.0;

/// 戰術持續秒數：效果維持多久後自動消退。
pub const TACTIC_DURATION_SECS: f32 = 15.0;

/// 集結戰術的額外波及半徑（比基礎包圍更大）。
pub const RALLY_RADIUS: f32 = 600.0;

/// 四種戰術。
#[derive(Debug, Clone, PartialEq)]
pub enum BossTactic {
    /// 包圍：附近所有小怪從四面八方夾擊玩家，不疊在同一點。
    Surround,
    /// 集火：附近所有小怪集中攻擊同一個目標（boss 最近的那個玩家）。
    FocusFire,
    /// 撤退：boss 自身強制逃跑，讓小怪繼續拖住玩家。
    Retreat,
    /// 集結：呼喚同種小怪（600px 範圍）全部湧向玩家。
    Rally,
}

impl BossTactic {
    /// 繁體中文短名（用於廣播前綴）。
    pub fn display_name(&self) -> &'static str {
        match self {
            BossTactic::Surround  => "包圍",
            BossTactic::FocusFire => "集火",
            BossTactic::Retreat   => "撤退",
            BossTactic::Rally     => "集結",
        }
    }

    /// AI 輸出的解析關鍵字（大寫英文）。
    fn parse_keyword(&self) -> &'static str {
        match self {
            BossTactic::Surround  => "SURROUND",
            BossTactic::FocusFire => "FOCUSFIRE",
            BossTactic::Retreat   => "RETREAT",
            BossTactic::Rally     => "RALLY",
        }
    }
}

/// boss_ai 狀態（純記憶體，重啟清零）。
pub struct BossAiState {
    /// enemy_id → 距下次決策倒數（秒）。≤ 0 時觸發。
    cooldowns: HashMap<(i32, i32, usize), f32>,
}

/// `tick()` 傳入每隻菁英精英的上下文資訊。
#[derive(Debug, Clone)]
pub struct TacticInput {
    pub id: (i32, i32, usize),
    pub kind_name: &'static str,
    pub level: u32,
    pub x: f32,
    pub y: f32,
    /// 當前 HP 百分比（0.0~1.0）。
    pub hp_pct: f32,
}

/// `tick()` 回傳的待處理決策候選。
#[derive(Debug, Clone)]
pub struct TacticCandidate {
    pub id: (i32, i32, usize),
    pub kind_name: String,
    pub level: u32,
    pub x: f32,
    pub y: f32,
    pub hp_pct: f32,
    /// 由罐頭邏輯即時決定的戰術（不依賴 LLM）。
    pub tactic: BossTactic,
}

impl BossAiState {
    pub fn new() -> Self {
        Self { cooldowns: HashMap::new() }
    }

    /// 推進時間，回傳第一個冷卻歸零的菁英（最多一個，避免同幀觸發大量 LLM）。
    pub fn tick(
        &mut self,
        dt: f32,
        notorious: &[TacticInput],
        player_count: usize,
    ) -> Option<TacticCandidate> {
        // 清除已消失的菁英（防 HashMap 洩漏）。
        let active_ids: HashSet<_> = notorious.iter().map(|e| e.id).collect();
        self.cooldowns.retain(|id, _| active_ids.contains(id));

        let mut candidate: Option<TacticCandidate> = None;
        for e in notorious {
            let timer = self.cooldowns.entry(e.id).or_insert(FIRST_DECISION_WAIT_SECS);
            *timer -= dt;
            if *timer <= 0.0 && candidate.is_none() {
                *timer = DECISION_COOLDOWN_SECS;
                let tactic = canned_tactic(e.hp_pct, player_count);
                candidate = Some(TacticCandidate {
                    id: e.id,
                    kind_name: e.kind_name.to_string(),
                    level: e.level,
                    x: e.x,
                    y: e.y,
                    hp_pct: e.hp_pct,
                    tactic,
                });
            }
        }
        candidate
    }
}

/// 根據血量與玩家數即時決定最合理的戰術（零 LLM、零延遲）。
pub fn canned_tactic(hp_pct: f32, player_count: usize) -> BossTactic {
    if hp_pct < 0.3 {
        BossTactic::Retreat
    } else if player_count == 0 {
        BossTactic::Rally
    } else if player_count >= 3 {
        BossTactic::Surround
    } else {
        BossTactic::FocusFire
    }
}

/// 建立 AI 台詞的 system prompt。
pub fn build_message_prompt(kind_name: &str, level: u32, tactic: &BossTactic) -> String {
    let tactic_name = tactic.display_name();
    format!(
        "你是蒸汽龐克太空歌劇世界的怪物王「{kind_name}」，等級 {level}。\
        你剛下達「{tactic_name}」戰術指令。\
        請以怪物王的口吻，用 15 字以內的繁體中文說出下令台詞，充滿氣勢與壓迫感。\
        只輸出台詞本身，不要引號、不要前綴。"
    )
}

/// 依戰術回傳罐頭台詞（LLM 未啟用或呼叫失敗時的後備）。
pub fn canned_message(tactic: &BossTactic) -> &'static str {
    match tactic {
        BossTactic::Surround  => "從四面八方圍住他們，一個也別逃！",
        BossTactic::FocusFire => "全員集中——先把最弱的那個殺了！",
        BossTactic::Retreat   => "先退！讓手下去纏住他們！",
        BossTactic::Rally     => "兄弟們，全員集結——我需要你們！",
    }
}

/// 解析 AI 輸出，嘗試從開頭關鍵字（SURROUND/FOCUSFIRE/RETREAT/RALLY）識別戰術。
/// 找不到時回 None，呼叫端可退回罐頭台詞。
pub fn parse_tactic_from_text(text: &str) -> Option<BossTactic> {
    let upper = text.to_uppercase();
    if upper.starts_with("SURROUND") {
        Some(BossTactic::Surround)
    } else if upper.starts_with("FOCUSFIRE") || upper.starts_with("FOCUS_FIRE") {
        Some(BossTactic::FocusFire)
    } else if upper.starts_with("RETREAT") {
        Some(BossTactic::Retreat)
    } else if upper.starts_with("RALLY") {
        Some(BossTactic::Rally)
    } else {
        None
    }
}

/// 呼叫 LLM 生成戰術廣播台詞（Groq → ollama → 罐頭降級）。
/// 只生成台詞，戰術本身已由 `canned_tactic` 決定。
/// 為純 async 函式，由 game.rs tokio::spawn 呼叫，不阻塞遊戲迴圈。
pub async fn generate_tactic_message(
    kind_name: &str,
    level: u32,
    tactic: &BossTactic,
) -> String {
    let system = build_message_prompt(kind_name, level, tactic);
    let user = format!("現在下令：{}！", tactic.display_name());
    match crate::npc_chat::raw_llm_call(&system, &user).await {
        Some(text) => {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                canned_message(tactic).to_string()
            } else {
                trimmed
            }
        }
        None => canned_message(tactic).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_trigger_before_first_wait() {
        let mut state = BossAiState::new();
        let input = vec![TacticInput { id: (0,0,0), kind_name: "晶石傀儡", level: 5, x: 0.0, y: 0.0, hp_pct: 1.0 }];
        let r = state.tick(1.0, &input, 2);
        assert!(r.is_none(), "首次出現 1 秒後不應觸發決策");
    }

    #[test]
    fn triggers_after_first_wait() {
        let mut state = BossAiState::new();
        let input = vec![TacticInput { id: (1,2,0), kind_name: "符文守衛", level: 7, x: 100.0, y: 200.0, hp_pct: 0.8 }];
        let r = state.tick(FIRST_DECISION_WAIT_SECS + 1.0, &input, 2);
        assert!(r.is_some(), "等待時間過後應觸發決策");
        let c = r.unwrap();
        assert_eq!(c.id, (1,2,0));
        assert_eq!(c.level, 7);
        assert_eq!(c.tactic, BossTactic::FocusFire); // 2 玩家、HP 正常 → 集火
    }

    #[test]
    fn resets_cooldown_after_trigger() {
        let mut state = BossAiState::new();
        let input = vec![TacticInput { id: (0,0,1), kind_name: "虛空幽靈", level: 10, x: 0.0, y: 0.0, hp_pct: 0.9 }];
        state.tick(FIRST_DECISION_WAIT_SECS + 1.0, &input, 1);
        // 觸發後 1 秒內不應再觸發
        let r = state.tick(1.0, &input, 1);
        assert!(r.is_none(), "觸發後應重置冷卻，不立刻再觸發");
    }

    #[test]
    fn cleans_up_gone_enemies() {
        let mut state = BossAiState::new();
        let id = (5i32, 5i32, 0usize);
        let input = vec![TacticInput { id, kind_name: "蒸汽構裝", level: 12, x: 0.0, y: 0.0, hp_pct: 1.0 }];
        state.tick(1.0, &input, 0);
        assert_eq!(state.cooldowns.len(), 1);
        state.tick(1.0, &[], 0);
        assert_eq!(state.cooldowns.len(), 0, "怪消失後應清除 cooldown");
    }

    #[test]
    fn canned_tactic_low_hp_gives_retreat() {
        assert_eq!(canned_tactic(0.2, 2), BossTactic::Retreat);
        assert_eq!(canned_tactic(0.29, 3), BossTactic::Retreat);
    }

    #[test]
    fn canned_tactic_no_players_gives_rally() {
        assert_eq!(canned_tactic(0.8, 0), BossTactic::Rally);
    }

    #[test]
    fn canned_tactic_many_players_gives_surround() {
        assert_eq!(canned_tactic(0.8, 3), BossTactic::Surround);
        assert_eq!(canned_tactic(0.5, 5), BossTactic::Surround);
    }

    #[test]
    fn canned_tactic_few_players_gives_focusfire() {
        assert_eq!(canned_tactic(0.8, 1), BossTactic::FocusFire);
        assert_eq!(canned_tactic(0.6, 2), BossTactic::FocusFire);
    }

    #[test]
    fn parse_tactic_all_keywords() {
        assert_eq!(parse_tactic_from_text("SURROUND: 包圍！"), Some(BossTactic::Surround));
        assert_eq!(parse_tactic_from_text("FOCUSFIRE: 集火！"), Some(BossTactic::FocusFire));
        assert_eq!(parse_tactic_from_text("RETREAT: 撤退！"), Some(BossTactic::Retreat));
        assert_eq!(parse_tactic_from_text("RALLY: 集結！"), Some(BossTactic::Rally));
        assert_eq!(parse_tactic_from_text("surround 來了"), Some(BossTactic::Surround));
        assert!(parse_tactic_from_text("完全不認識的東西").is_none());
    }

    #[test]
    fn canned_message_all_tactics_non_empty() {
        for t in [BossTactic::Surround, BossTactic::FocusFire, BossTactic::Retreat, BossTactic::Rally] {
            assert!(!canned_message(&t).is_empty(), "{} 的罐頭台詞不應為空", t.display_name());
        }
    }
}
