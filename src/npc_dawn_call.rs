//! NPC 晨喚（ROADMAP 77）。
//!
//! 日夜循環每次從夜晚轉入黎明時，凱爾長老在世界聊天廣播一句晨間致辭。
//! 讓玩家感受到世界有「每天的開始」，NPC 知道時間在流逝。
//!
//! 成本紀律：
//! - 每次日夜轉換最多觸發一次（冷卻：DAWN_CALL_COOLDOWN_SECS）。
//! - 全局 Semaphore(1)，防並發 LLM 呼叫。
//! - 降級鏈：Groq → ollama → 罐頭。
//! - `BUTFUN_NPC_LLM=1` 未設定時直接回罐頭，不呼叫任何外部 API。
//! - 無玩家在線時不觸發（不浪費 LLM token）。

use crate::daynight::Phase;

/// 每次晨喚的冷卻（秒）——略短於一個完整日夜循環（600 秒），每天約觸發一次。
pub const DAWN_CALL_COOLDOWN_SECS: f32 = 480.0;

/// 伺服器啟動後首次觸發的最短等待（秒），避免剛啟動就立刻廣播。
const FIRST_CALL_WAIT_SECS: f32 = 90.0;

/// dawn_call 專屬 Semaphore 容量：同時最多 1 個 AI 呼叫。
pub const MAX_CONCURRENT_CALLS: usize = 1;

/// 凱爾長老的 NPC id（與 village_chief.rs 保持一致）。
pub const CHIEF_DISPLAY_NAME: &str = "凱爾長老";

/// 晨喚狀態（純記憶體，重啟清零）。
pub struct DawnCallState {
    /// 距下次可觸發的倒數（秒）；觸發後重設為 DAWN_CALL_COOLDOWN_SECS。
    cooldown: f32,
    /// 上一個 tick 的日夜階段，用於偵測 Night/Dusk → Dawn 轉換。
    last_phase: Phase,
}

impl DawnCallState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_CALL_WAIT_SECS,
            // 預設從夜晚開始，確保第一次黎明轉換能被偵測到。
            last_phase: Phase::Night,
        }
    }

    /// 推進時間，回傳 `true` 表示應觸發晨喚。
    ///
    /// 觸發條件：
    /// 1. 上一個 tick 不是 Dawn（Night / Dusk / Day 皆算非 Dawn）。
    /// 2. 當前 tick 進入 Dawn。
    /// 3. 冷卻已歸零。
    pub fn tick(&mut self, dt: f32, current_phase: Phase) -> bool {
        self.cooldown -= dt;

        let transition_to_dawn =
            self.last_phase != Phase::Dawn && current_phase == Phase::Dawn;

        let should_trigger = transition_to_dawn && self.cooldown <= 0.0;

        self.last_phase = current_phase;

        if should_trigger {
            self.cooldown = DAWN_CALL_COOLDOWN_SECS;
            true
        } else {
            false
        }
    }
}

/// 建立 AI 晨喚的 system prompt。
pub fn build_system_prompt(online_count: usize) -> String {
    let count_hint = if online_count == 0 {
        String::new()
    } else {
        format!("目前有 {online_count} 位冒險者在線。")
    };
    format!(
        "你是蒸汽龐克太空歌劇世界中受人敬重的村長——凱爾長老。\
        黎明剛剛降臨，你站在廣場中央，\
        用溫厚而有力的聲音宣告新的一天開始。{count_hint}\
        請用 30 字以內的繁體中文說一句晨間致辭：溫暖、帶有些許智慧，\
        可以提到冒險、合作、今日的氣候或期許。\
        只輸出那一句話，不要引號、前綴或額外說明。"
    )
}

/// 晨喚罐頭後備（LLM 未啟用或呼叫失敗時回傳）。
pub fn canned_dawn_call(index: usize) -> &'static str {
    const CANNED: &[&str] = &[
        "晨光降臨，願諸位今日順心如意。",
        "黎明了，鄉親們。好好珍惜今天的陽光。",
        "新的一天開始了。願冒險者們平安歸來。",
        "破曉，世界又迎來了清晨。互助共進，方能走得更遠。",
        "太陽升起，一切皆有可能。願今日充實而豐盛。",
    ];
    CANNED[index % CANNED.len()]
}

/// 靜態計數器：每次觸發晨喚時遞增，用來輪換罐頭語句。
static CALL_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 非同步生成晨喚文字（Groq → ollama → 罐頭降級鏈）。
pub async fn generate_dawn_call(online_count: usize) -> String {
    let idx = CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let system = build_system_prompt(online_count);
    let user = "請說今天的晨間致辭。";
    match crate::npc_chat::raw_llm_call(&system, user).await {
        Some(text) => {
            let t = text.trim().to_string();
            if t.is_empty() {
                canned_dawn_call(idx).to_string()
            } else {
                t
            }
        }
        None => canned_dawn_call(idx).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daynight::Phase;

    #[test]
    fn does_not_trigger_before_first_wait() {
        let mut state = DawnCallState::new();
        // 還在初始冷卻內，即使發生 Night→Dawn 轉換也不應觸發。
        let r = state.tick(1.0, Phase::Dawn);
        assert!(!r, "初始冷卻期間不應觸發晨喚");
    }

    #[test]
    fn does_not_trigger_without_phase_transition() {
        let mut state = DawnCallState::new();
        // 冷卻歸零，但相位從 Dawn 到 Dawn（沒有轉換）。
        state.last_phase = Phase::Dawn;
        state.cooldown = -1.0;
        let r = state.tick(0.1, Phase::Dawn);
        assert!(!r, "相位沒有轉換時不應觸發");
    }

    #[test]
    fn triggers_on_night_to_dawn_after_wait() {
        let mut state = DawnCallState::new();
        // 先推進超過 FIRST_CALL_WAIT_SECS（模擬冷卻歸零）。
        state.cooldown = -1.0;
        // 上次是夜晚，這次進入黎明 → 應觸發。
        state.last_phase = Phase::Night;
        let r = state.tick(0.1, Phase::Dawn);
        assert!(r, "Night→Dawn 且冷卻歸零後應觸發晨喚");
    }

    #[test]
    fn triggers_on_dusk_to_dawn_after_wait() {
        let mut state = DawnCallState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Dusk;
        let r = state.tick(0.1, Phase::Dawn);
        assert!(r, "Dusk→Dawn 且冷卻歸零後也應觸發晨喚");
    }

    #[test]
    fn resets_cooldown_after_trigger() {
        let mut state = DawnCallState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Night;
        let triggered = state.tick(0.1, Phase::Dawn);
        assert!(triggered);
        // 觸發後冷卻應重設。
        assert!(state.cooldown > 0.0, "觸發後冷卻應重設為 DAWN_CALL_COOLDOWN_SECS");
        // 立刻再 tick 一次：相位仍是 Dawn（非 Night→Dawn 轉換），不應再觸發。
        let r2 = state.tick(0.1, Phase::Dawn);
        assert!(!r2, "同一個黎明期間不應連續觸發");
    }

    #[test]
    fn does_not_trigger_during_day() {
        let mut state = DawnCallState::new();
        state.cooldown = -1.0;
        state.last_phase = Phase::Dawn;
        let r = state.tick(0.1, Phase::Day);
        assert!(!r, "白天不應觸發晨喚");
    }

    #[test]
    fn canned_dawn_call_non_empty_for_all_indices() {
        for i in 0..10 {
            let msg = canned_dawn_call(i);
            assert!(!msg.is_empty(), "罐頭晨喚 index {i} 不應為空");
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
