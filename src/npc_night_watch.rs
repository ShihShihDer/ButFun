//! NPC 入夜守衛令（ROADMAP 80）。
//!
//! 日夜循環每次從黃昏轉入夜晚時，獵手蘭卡在世界聊天廣播一句入夜守衛令。
//! 與晨喚（凱爾長老）、午鐘（老胡）及暮告（薇拉）共同形成四時段節律：
//!   - 夜晚 → 黎明：凱爾長老晨喚（ROADMAP 77）
//!   - 黎明 → 白天：工匠老胡午鐘（ROADMAP 79）
//!   - 白天 → 黃昏：商人薇拉暮告（ROADMAP 78）
//!   - 黃昏 → 夜晚：獵手蘭卡守衛令（本模組）
//! 讓玩家感受到世界一天有四個明確的時刻，NPC 各司其職。
//!
//! 成本紀律：
//! - 每次相位轉換最多觸發一次（冷卻：NIGHT_WATCH_COOLDOWN_SECS）。
//! - 全局 Semaphore(1)，防並發 LLM 呼叫。
//! - 降級鏈：Groq → ollama → 罐頭。
//! - `BUTFUN_NPC_LLM=1` 未設定時直接回罐頭，不呼叫任何外部 API。
//! - 無玩家在線時不觸發（不浪費 LLM token）。

use crate::daynight::Phase;

/// 每次守衛令的冷卻（秒）——略短於一個完整日夜循環（600 秒），每天約觸發一次。
pub const NIGHT_WATCH_COOLDOWN_SECS: f32 = 480.0;

/// 伺服器啟動後首次觸發的最短等待（秒），避免剛啟動就立刻廣播。
const FIRST_CALL_WAIT_SECS: f32 = 90.0;

/// night_watch 專屬 Semaphore 容量：同時最多 1 個 AI 呼叫。
pub const MAX_CONCURRENT_CALLS: usize = 1;

/// 獵手蘭卡的顯示名稱（與 npc_chat.rs 的 NPCS 保持一致）。
pub const RANKA_DISPLAY_NAME: &str = "獵手蘭卡";

/// 入夜守衛令狀態（純記憶體，重啟清零）。
pub struct NightWatchState {
    /// 距下次可觸發的倒數（秒）；觸發後重設為 NIGHT_WATCH_COOLDOWN_SECS。
    cooldown: f32,
    /// 上一個 tick 的日夜階段，用於偵測 Dusk → Night 轉換。
    last_phase: Phase,
}

impl NightWatchState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_CALL_WAIT_SECS,
            // 預設從黃昏開始，確保第一次黃昏→夜晚轉換能被偵測到。
            last_phase: Phase::Dusk,
        }
    }

    /// 推進時間，回傳 `true` 表示應觸發入夜守衛令廣播。
    ///
    /// 觸發條件：
    /// 1. 上一個 tick 是 Dusk（黃昏）。
    /// 2. 當前 tick 進入 Night（夜晚）。
    /// 3. 冷卻已歸零。
    pub fn tick(&mut self, dt: f32, current_phase: Phase) -> bool {
        self.cooldown -= dt;

        let transition_to_night =
            self.last_phase == Phase::Dusk && current_phase == Phase::Night;

        let should_trigger = transition_to_night && self.cooldown <= 0.0;

        self.last_phase = current_phase;

        if should_trigger {
            self.cooldown = NIGHT_WATCH_COOLDOWN_SECS;
            true
        } else {
            false
        }
    }
}

/// 建立 AI 入夜守衛令的 system prompt。
pub fn build_system_prompt(online_count: usize) -> String {
    let count_hint = if online_count == 0 {
        String::new()
    } else {
        format!("目前有 {online_count} 位冒險者在線。")
    };
    format!(
        "你是蒸汽龐克太空歌劇世界中經驗豐富的獵手——蘭卡。\
        黃昏剛過，夜幕降臨，你正式接班守衛主城周邊。\
        你低調、警覺、話不多，但每句話都有分量。{count_hint}\
        請用 30 字以內的繁體中文說一句入夜守衛令：簡短有力，帶獵手的警覺與沉穩口吻，\
        可以提到夜晚的危險、怪物的動態或叮嚀冒險者保持戒備。\
        只輸出那一句話，不要引號、前綴或額外說明。"
    )
}

/// 入夜守衛令罐頭後備（LLM 未啟用或呼叫失敗時回傳）。
pub fn canned_night_watch(index: usize) -> &'static str {
    const CANNED: &[&str] = &[
        "夜幕降臨，保持警惕，今晚的怪物不好惹。",
        "入夜了，遠離暗處，我會在外圍巡邏。",
        "夜晚開始，城外的怪物更活躍，小心行事。",
        "黑夜屬於獵人，也屬於獵物，別讓自己成為後者。",
        "守衛令下達，夜間外出請結伴，獨行風險自負。",
    ];
    CANNED[index % CANNED.len()]
}

/// 靜態計數器：每次觸發守衛令時遞增，用來輪換罐頭語句。
static CALL_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 非同步生成入夜守衛令文字（Groq → ollama → 罐頭降級鏈）。
pub async fn generate_night_watch(online_count: usize) -> String {
    let idx = CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let system = build_system_prompt(online_count);
    let user = "請說今晚的守衛令。";
    match crate::npc_chat::raw_llm_call(&system, user).await {
        Some(text) => {
            let t = text.trim().to_string();
            if t.is_empty() {
                canned_night_watch(idx).to_string()
            } else {
                t
            }
        }
        None => canned_night_watch(idx).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daynight::Phase;

    #[test]
    fn does_not_trigger_before_first_wait() {
        let mut state = NightWatchState::new();
        // 還在初始冷卻內，即使發生 Dusk→Night 轉換也不應觸發。
        let r = state.tick(1.0, Phase::Night);
        assert!(!r, "初始冷卻期間不應觸發守衛令");
    }

    #[test]
    fn does_not_trigger_without_phase_transition() {
        let mut state = NightWatchState::new();
        // 冷卻歸零，但相位從 Night 到 Night（沒有轉換）。
        state.last_phase = Phase::Night;
        state.cooldown = -1.0;
        let r = state.tick(0.1, Phase::Night);
        assert!(!r, "相位沒有轉換時不應觸發");
    }

    #[test]
    fn triggers_on_dusk_to_night_after_wait() {
        let mut state = NightWatchState::new();
        // 模擬冷卻歸零。
        state.cooldown = -1.0;
        // 上次是黃昏，這次進入夜晚 → 應觸發。
        state.last_phase = Phase::Dusk;
        let r = state.tick(0.1, Phase::Night);
        assert!(r, "Dusk→Night 且冷卻歸零後應觸發守衛令");
    }

    #[test]
    fn does_not_trigger_on_day_to_night() {
        let mut state = NightWatchState::new();
        state.cooldown = -1.0;
        // 只有 Dusk→Night 才觸發，Day→Night（理論上不合相位順序）不觸發。
        state.last_phase = Phase::Day;
        let r = state.tick(0.1, Phase::Night);
        assert!(!r, "Day→Night 不應觸發守衛令");
    }

    #[test]
    fn does_not_trigger_on_dawn_to_night() {
        let mut state = NightWatchState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Dawn;
        let r = state.tick(0.1, Phase::Night);
        assert!(!r, "Dawn→Night 不應觸發守衛令");
    }

    #[test]
    fn resets_cooldown_after_trigger() {
        let mut state = NightWatchState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Dusk;
        let triggered = state.tick(0.1, Phase::Night);
        assert!(triggered);
        // 觸發後冷卻應重設。
        assert!(state.cooldown > 0.0, "觸發後冷卻應重設為 NIGHT_WATCH_COOLDOWN_SECS");
        // 立刻再 tick 一次：相位仍是 Night，不應再觸發。
        let r2 = state.tick(0.1, Phase::Night);
        assert!(!r2, "同一個夜晚期間不應連續觸發");
    }

    #[test]
    fn does_not_trigger_during_day() {
        let mut state = NightWatchState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Night;
        let r = state.tick(0.1, Phase::Day);
        assert!(!r, "白天不應觸發守衛令");
    }

    #[test]
    fn canned_night_watch_non_empty_for_all_indices() {
        for i in 0..10 {
            let msg = canned_night_watch(i);
            assert!(!msg.is_empty(), "罐頭守衛令 index {i} 不應為空");
        }
    }

    #[test]
    fn build_system_prompt_includes_online_count() {
        let prompt = build_system_prompt(5);
        assert!(prompt.contains("5"), "system prompt 應包含在線玩家數");
    }

    #[test]
    fn build_system_prompt_omits_count_when_zero() {
        let prompt = build_system_prompt(0);
        // online_count 為 0 時不注入人數字串。
        assert!(!prompt.contains("0 位"), "在線 0 人時不應注入計數");
    }
}
