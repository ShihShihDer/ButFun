//! NPC 午鐘廣播（ROADMAP 79）。
//!
//! 日夜循環每次從黎明轉入白天時，工匠老胡在世界聊天廣播一句午間開工令。
//! 與晨喚（凱爾長老，黎明開始）及暮告（薇拉商人，黃昏開始）共同形成三時段節律：
//!   - 黎明 → 凱爾長老晨喚（迎接一天開始）
//!   - 黎明→白天 → 工匠老胡午鐘（日照全開，工時開始）
//!   - 白天→黃昏 → 商人薇拉暮告（傍晚收攤）
//! 讓玩家感受到世界一天有三個明確的時刻，NPC 各司其職。
//!
//! 成本紀律：
//! - 每次相位轉換最多觸發一次（冷卻：NOON_BELL_COOLDOWN_SECS）。
//! - 全局 Semaphore(1)，防並發 LLM 呼叫。
//! - 降級鏈：Groq → ollama → 罐頭。
//! - `BUTFUN_NPC_LLM=1` 未設定時直接回罐頭，不呼叫任何外部 API。
//! - 無玩家在線時不觸發（不浪費 LLM token）。

use crate::daynight::Phase;

/// 每次午鐘的冷卻（秒）——略短於一個完整日夜循環（600 秒），每天約觸發一次。
pub const NOON_BELL_COOLDOWN_SECS: f32 = 480.0;

/// 伺服器啟動後首次觸發的最短等待（秒），避免剛啟動就立刻廣播。
const FIRST_CALL_WAIT_SECS: f32 = 90.0;

/// noon_bell 專屬 Semaphore 容量：同時最多 1 個 AI 呼叫。
pub const MAX_CONCURRENT_CALLS: usize = 1;

/// 工匠老胡的顯示名稱（與 npc_chat.rs 的 NPCS 保持一致）。
pub const HU_DISPLAY_NAME: &str = "工匠老胡";

/// 午鐘狀態（純記憶體，重啟清零）。
pub struct NoonBellState {
    /// 距下次可觸發的倒數（秒）；觸發後重設為 NOON_BELL_COOLDOWN_SECS。
    cooldown: f32,
    /// 上一個 tick 的日夜階段，用於偵測 Dawn → Day 轉換。
    last_phase: Phase,
}

impl NoonBellState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_CALL_WAIT_SECS,
            // 預設從黎明開始，確保第一次黎明→白天轉換能被偵測到。
            last_phase: Phase::Dawn,
        }
    }

    /// 推進時間，回傳 `true` 表示應觸發午鐘廣播。
    ///
    /// 觸發條件：
    /// 1. 上一個 tick 是 Dawn（黎明）。
    /// 2. 當前 tick 進入 Day（白天）。
    /// 3. 冷卻已歸零。
    pub fn tick(&mut self, dt: f32, current_phase: Phase) -> bool {
        self.cooldown -= dt;

        let transition_to_day =
            self.last_phase == Phase::Dawn && current_phase == Phase::Day;

        let should_trigger = transition_to_day && self.cooldown <= 0.0;

        self.last_phase = current_phase;

        if should_trigger {
            self.cooldown = NOON_BELL_COOLDOWN_SECS;
            true
        } else {
            false
        }
    }
}

/// 建立 AI 午鐘廣播的 system prompt。
pub fn build_system_prompt(online_count: usize) -> String {
    let count_hint = if online_count == 0 {
        String::new()
    } else {
        format!("目前有 {online_count} 位冒險者在線。")
    };
    format!(
        "你是蒸汽龐克太空歌劇世界主城工坊的老師傅——老胡。\
        黎明剛過，日照全開，正是一天最適合動工的時候。{count_hint}\
        你站在工坊門口，用粗嗓門宣告開工。\
        請用 30 字以內的繁體中文說一句午間開工令：簡短有力，帶點老師傅的口吻，\
        可以提到日照、材料、手藝或叫人別偷懶。\
        只輸出那一句話，不要引號、前綴或額外說明。"
    )
}

/// 午鐘廣播罐頭後備（LLM 未啟用或呼叫失敗時回傳）。
pub fn canned_noon_bell(index: usize) -> &'static str {
    const CANNED: &[&str] = &[
        "日頭都上來了，動作快點！工坊今天排了不少活兒。",
        "好了好了，黎明過了，該幹活兒啦。磨刀不誤砍柴工，快來。",
        "日照正好，是開工的時辰了。有材料的快送來。",
        "別賴著了，陽光這麼亮，在工坊裡手藝做出來都更好看。",
        "工坊開門！今天的訂單不少，有要做東西的早點來。",
    ];
    CANNED[index % CANNED.len()]
}

/// 靜態計數器：每次觸發午鐘時遞增，用來輪換罐頭語句。
static CALL_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 非同步生成午鐘廣播文字（Groq → ollama → 罐頭降級鏈）。
pub async fn generate_noon_bell(online_count: usize) -> String {
    let idx = CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let system = build_system_prompt(online_count);
    let user = "請說今天的開工令。";
    match crate::npc_chat::raw_llm_call(&system, user).await {
        Some(text) => {
            let t = text.trim().to_string();
            if t.is_empty() {
                canned_noon_bell(idx).to_string()
            } else {
                t
            }
        }
        None => canned_noon_bell(idx).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daynight::Phase;

    #[test]
    fn does_not_trigger_before_first_wait() {
        let mut state = NoonBellState::new();
        // 還在初始冷卻內，即使發生 Dawn→Day 轉換也不應觸發。
        let r = state.tick(1.0, Phase::Day);
        assert!(!r, "初始冷卻期間不應觸發午鐘");
    }

    #[test]
    fn does_not_trigger_without_phase_transition() {
        let mut state = NoonBellState::new();
        // 冷卻歸零，但相位從 Day 到 Day（沒有轉換）。
        state.last_phase = Phase::Day;
        state.cooldown = -1.0;
        let r = state.tick(0.1, Phase::Day);
        assert!(!r, "相位沒有轉換時不應觸發");
    }

    #[test]
    fn triggers_on_dawn_to_day_after_wait() {
        let mut state = NoonBellState::new();
        // 模擬冷卻歸零。
        state.cooldown = -1.0;
        // 上次是黎明，這次進入白天 → 應觸發。
        state.last_phase = Phase::Dawn;
        let r = state.tick(0.1, Phase::Day);
        assert!(r, "Dawn→Day 且冷卻歸零後應觸發午鐘");
    }

    #[test]
    fn does_not_trigger_on_night_to_day() {
        let mut state = NoonBellState::new();
        state.cooldown = -1.0;
        // 只有 Dawn→Day 才觸發，Night→Day（理論上不合相位順序）不觸發。
        state.last_phase = Phase::Night;
        let r = state.tick(0.1, Phase::Day);
        assert!(!r, "Night→Day 不應觸發午鐘");
    }

    #[test]
    fn does_not_trigger_on_dusk_to_day() {
        let mut state = NoonBellState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Dusk;
        let r = state.tick(0.1, Phase::Day);
        assert!(!r, "Dusk→Day 不應觸發午鐘");
    }

    #[test]
    fn resets_cooldown_after_trigger() {
        let mut state = NoonBellState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Dawn;
        let triggered = state.tick(0.1, Phase::Day);
        assert!(triggered);
        // 觸發後冷卻應重設。
        assert!(state.cooldown > 0.0, "觸發後冷卻應重設為 NOON_BELL_COOLDOWN_SECS");
        // 立刻再 tick 一次：相位仍是 Day，不應再觸發。
        let r2 = state.tick(0.1, Phase::Day);
        assert!(!r2, "同一個白天期間不應連續觸發");
    }

    #[test]
    fn does_not_trigger_during_night() {
        let mut state = NoonBellState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Day;
        let r = state.tick(0.1, Phase::Night);
        assert!(!r, "夜晚不應觸發午鐘");
    }

    #[test]
    fn canned_noon_bell_non_empty_for_all_indices() {
        for i in 0..10 {
            let msg = canned_noon_bell(i);
            assert!(!msg.is_empty(), "罐頭午鐘 index {i} 不應為空");
        }
    }

    #[test]
    fn build_system_prompt_includes_online_count() {
        let prompt = build_system_prompt(4);
        assert!(prompt.contains("4"), "system prompt 應包含在線玩家數");
    }

    #[test]
    fn build_system_prompt_omits_count_when_zero() {
        let prompt = build_system_prompt(0);
        // online_count 為 0 時不注入人數字串。
        assert!(!prompt.contains("0 位"), "在線 0 人時不應注入計數");
    }
}
