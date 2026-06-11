//! NPC 暮告（ROADMAP 78）。
//!
//! 日夜循環每次從白天轉入黃昏時，商人薇拉在世界聊天廣播一句傍晚感言。
//! 與晨喚（凱爾長老，ROADMAP 77）形成完整的日夜節律：
//!   - 黎明 → 凱爾長老晨喚
//!   - 黃昏 → 商人薇拉暮告
//! 讓玩家感受到世界有「每天的結束」，NPC 也有自己的作息。
//!
//! 成本紀律：
//! - 每次日夜轉換最多觸發一次（冷卻：DUSK_CALL_COOLDOWN_SECS）。
//! - 全局 Semaphore(1)，防並發 LLM 呼叫。
//! - 降級鏈：Groq → ollama → 罐頭。
//! - `BUTFUN_NPC_LLM=1` 未設定時直接回罐頭，不呼叫任何外部 API。
//! - 無玩家在線時不觸發（不浪費 LLM token）。

use crate::daynight::Phase;

/// 每次暮告的冷卻（秒）——略短於一個完整日夜循環（600 秒），每天約觸發一次。
pub const DUSK_CALL_COOLDOWN_SECS: f32 = 480.0;

/// 伺服器啟動後首次觸發的最短等待（秒），避免剛啟動就立刻廣播。
const FIRST_CALL_WAIT_SECS: f32 = 90.0;

/// dusk_call 專屬 Semaphore 容量：同時最多 1 個 AI 呼叫。
pub const MAX_CONCURRENT_CALLS: usize = 1;

/// 商人薇拉的顯示名稱（與 npc.rs 保持一致）。
pub const VELA_DISPLAY_NAME: &str = "商人薇拉";

/// 暮告狀態（純記憶體，重啟清零）。
pub struct DuskCallState {
    /// 距下次可觸發的倒數（秒）；觸發後重設為 DUSK_CALL_COOLDOWN_SECS。
    cooldown: f32,
    /// 上一個 tick 的日夜階段，用於偵測 Day → Dusk 轉換。
    last_phase: Phase,
}

impl DuskCallState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_CALL_WAIT_SECS,
            // 預設從白天開始，確保第一次黃昏轉換能被偵測到。
            last_phase: Phase::Day,
        }
    }

    /// 推進時間，回傳 `true` 表示應觸發暮告。
    ///
    /// 觸發條件：
    /// 1. 上一個 tick 是 Day（白天）。
    /// 2. 當前 tick 進入 Dusk（黃昏）。
    /// 3. 冷卻已歸零。
    pub fn tick(&mut self, dt: f32, current_phase: Phase) -> bool {
        self.cooldown -= dt;

        let transition_to_dusk =
            self.last_phase == Phase::Day && current_phase == Phase::Dusk;

        let should_trigger = transition_to_dusk && self.cooldown <= 0.0;

        self.last_phase = current_phase;

        if should_trigger {
            self.cooldown = DUSK_CALL_COOLDOWN_SECS;
            true
        } else {
            false
        }
    }
}

/// 建立 AI 暮告的 system prompt。
pub fn build_system_prompt(online_count: usize) -> String {
    let count_hint = if online_count == 0 {
        String::new()
    } else {
        format!("目前有 {online_count} 位冒險者在線。")
    };
    format!(
        "你是蒸汽龐克太空歌劇世界中熱情的商人——薇拉。\
        黃昏降臨，你正在收拾攤子準備打烊，\
        用輕鬆而溫暖的語氣向廣場上的冒險者說傍晚的感言。{count_hint}\
        請用 30 字以內的繁體中文說一句傍晚感言：可以提到今日生意、夜晚的氣氛、\
        感謝光顧或是叮嚀夜間安全。\
        只輸出那一句話，不要引號、前綴或額外說明。"
    )
}

/// 暮告罐頭後備（LLM 未啟用或呼叫失敗時回傳）。
pub fn canned_dusk_call(index: usize) -> &'static str {
    const CANNED: &[&str] = &[
        "今日生意收攤，感謝各位光顧，夜晚請注意安全。",
        "黃昏了，各位冒險者，夜裡怪物更兇，記得結伴行動。",
        "收攤囉！今天生意不錯，明天見，好好休息。",
        "傍晚的風有點涼，帶件披風再出門吧，夜安。",
        "一天又過了，感謝大家的支持，夜裡注意保暖。",
    ];
    CANNED[index % CANNED.len()]
}

/// 靜態計數器：每次觸發暮告時遞增，用來輪換罐頭語句。
static CALL_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 非同步生成暮告文字（Groq → ollama → 罐頭降級鏈）。
pub async fn generate_dusk_call(online_count: usize) -> String {
    let idx = CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let system = build_system_prompt(online_count);
    let user = "請說今天的傍晚感言。";
    match crate::npc_chat::raw_llm_call(&system, user).await {
        Some(text) => {
            let t = text.trim().to_string();
            if t.is_empty() {
                canned_dusk_call(idx).to_string()
            } else {
                t
            }
        }
        None => canned_dusk_call(idx).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daynight::Phase;

    #[test]
    fn does_not_trigger_before_first_wait() {
        let mut state = DuskCallState::new();
        // 還在初始冷卻內，即使發生 Day→Dusk 轉換也不應觸發。
        let r = state.tick(1.0, Phase::Dusk);
        assert!(!r, "初始冷卻期間不應觸發暮告");
    }

    #[test]
    fn does_not_trigger_without_phase_transition() {
        let mut state = DuskCallState::new();
        // 冷卻歸零，但相位從 Dusk 到 Dusk（沒有轉換）。
        state.last_phase = Phase::Dusk;
        state.cooldown = -1.0;
        let r = state.tick(0.1, Phase::Dusk);
        assert!(!r, "相位沒有轉換時不應觸發");
    }

    #[test]
    fn triggers_on_day_to_dusk_after_wait() {
        let mut state = DuskCallState::new();
        // 先模擬冷卻歸零。
        state.cooldown = -1.0;
        // 上次是白天，這次進入黃昏 → 應觸發。
        state.last_phase = Phase::Day;
        let r = state.tick(0.1, Phase::Dusk);
        assert!(r, "Day→Dusk 且冷卻歸零後應觸發暮告");
    }

    #[test]
    fn does_not_trigger_on_night_to_dusk() {
        let mut state = DuskCallState::new();
        state.cooldown = -1.0;
        // 只有 Day→Dusk 才觸發，Night→Dusk 不算（相位倒退不合理，不觸發）。
        state.last_phase = Phase::Night;
        let r = state.tick(0.1, Phase::Dusk);
        assert!(!r, "Night→Dusk 不應觸發暮告（只有 Day→Dusk 才合法）");
    }

    #[test]
    fn resets_cooldown_after_trigger() {
        let mut state = DuskCallState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Day;
        let triggered = state.tick(0.1, Phase::Dusk);
        assert!(triggered);
        // 觸發後冷卻應重設。
        assert!(state.cooldown > 0.0, "觸發後冷卻應重設為 DUSK_CALL_COOLDOWN_SECS");
        // 立刻再 tick 一次：相位仍是 Dusk（非 Day→Dusk 轉換），不應再觸發。
        let r2 = state.tick(0.1, Phase::Dusk);
        assert!(!r2, "同一個黃昏期間不應連續觸發");
    }

    #[test]
    fn does_not_trigger_during_night() {
        let mut state = DuskCallState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Dusk;
        let r = state.tick(0.1, Phase::Night);
        assert!(!r, "夜晚不應觸發暮告");
    }

    #[test]
    fn does_not_trigger_during_day() {
        let mut state = DuskCallState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Dawn;
        let r = state.tick(0.1, Phase::Day);
        assert!(!r, "白天不應觸發暮告");
    }

    #[test]
    fn canned_dusk_call_non_empty_for_all_indices() {
        for i in 0..10 {
            let msg = canned_dusk_call(i);
            assert!(!msg.is_empty(), "罐頭暮告 index {i} 不應為空");
        }
    }

    #[test]
    fn build_system_prompt_includes_online_count() {
        let prompt = build_system_prompt(3);
        assert!(prompt.contains("3"), "system prompt 應包含在線玩家數");
    }

    #[test]
    fn build_system_prompt_omits_count_when_zero() {
        let prompt = build_system_prompt(0);
        // online_count 為 0 時不注入人數字串。
        assert!(!prompt.contains("0 位"), "在線 0 人時不應注入計數");
    }
}
