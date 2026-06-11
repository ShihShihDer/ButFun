//! 廣場夜談（ROADMAP 76）。
//!
//! 夜晚廣場上的 NPC 偶爾互相閒聊，對話廣播到世界聊天頻道。
//!
//! 成本紀律：
//! - 每 TALK_COOLDOWN_SECS 秒觸發一次（冷卻期間不觸發）。
//! - 只有夜間才倒數、白天暫停計時。
//! - 全局 Semaphore(1)，防止並發 LLM 呼叫。
//! - 降級鏈：Groq → ollama → 罐頭。
//! - 只有夜間且有玩家在線時觸發。
//! - `BUTFUN_NPC_LLM=1` 未設定時直接回罐頭。

/// 廣場夜談觸發冷卻（秒）。
pub const TALK_COOLDOWN_SECS: f32 = 300.0; // 5 分鐘

/// 伺服器啟動後首次觸發的最短等待（秒）。
const FIRST_TALK_WAIT_SECS: f32 = 120.0;

/// 廣場夜談專屬 Semaphore 容量：同時最多 1 個 AI 呼叫。
pub const MAX_CONCURRENT_TALKS: usize = 1;

/// 一次廣場夜談的說話者／聆聽者對（皆為靜態 NPC id）。
#[derive(Debug, Clone)]
pub struct TalkPair {
    pub speaker_id: &'static str,
    pub listener_id: &'static str,
}

/// 廣場夜談狀態（純記憶體，重啟清零）。
pub struct PlazaTalkState {
    /// 距下次觸發倒數（秒）；只在夜間遞減。
    cooldown: f32,
    /// 輪換對話組合的索引。
    pair_index: usize,
}

/// 預定義輪換對話搭檔（speaker, listener）——每輪不同搭配，讓廣場聊天不重複。
static PAIRS: &[(&str, &str)] = &[
    ("merchant", "bounty_npc"),
    ("bounty_npc", "expedition_npc"),
    ("expedition_npc", "merchant"),
    ("workshop_npc", "farm_fair_npc"),
    ("farm_fair_npc", "procurement_npc"),
    ("procurement_npc", "workshop_npc"),
    ("village_chief", "merchant"),
    ("bounty_npc", "workshop_npc"),
    ("expedition_npc", "farm_fair_npc"),
    ("merchant", "village_chief"),
];

impl PlazaTalkState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_TALK_WAIT_SECS,
            pair_index: 0,
        }
    }

    /// 推進時間。只在夜間才倒數；有對話就回傳並重置冷卻。
    pub fn tick(&mut self, dt: f32, is_night: bool) -> Option<TalkPair> {
        if !is_night {
            return None;
        }
        self.cooldown -= dt;
        if self.cooldown <= 0.0 {
            self.cooldown = TALK_COOLDOWN_SECS;
            let (sid, lid) = PAIRS[self.pair_index % PAIRS.len()];
            self.pair_index += 1;
            Some(TalkPair { speaker_id: sid, listener_id: lid })
        } else {
            None
        }
    }
}

/// 取得 NPC 顯示名稱（從 npc_chat 共用資料）。
pub fn display_name(id: &str) -> &'static str {
    crate::npc_chat::find_npc(id).map(|n| n.display).unwrap_or("村民")
}

/// 建立 AI 夜談的 system prompt。
pub fn build_system_prompt(speaker_id: &str, listener_id: &str) -> String {
    let persona = crate::npc_chat::find_npc(speaker_id)
        .map(|n| n.persona)
        .unwrap_or("");
    let l_name = display_name(listener_id);
    format!(
        "{persona}\
        現在是夜晚，你和{l_name}在廣場上閒聊。\
        請用 25 字以內的繁體中文說一句輕鬆的夜晚話語——可以聊今天的見聞、村裡的近況、\
        對方的工作、星空或任何自然浮現的話題。語氣自然，符合你的個性。\
        只輸出那一句話，不加引號、不加「你說」之類的前綴。"
    )
}

/// 罐頭夜談（LLM 未啟用或呼叫失敗時的後備）。
pub fn canned_talk(speaker_id: &str, listener_id: &str) -> String {
    let l_name = display_name(listener_id);
    match speaker_id {
        "merchant" => format!("今晚生意清淡，不過星空很美。{l_name}，最近怎麼樣？"),
        "workshop_npc" => format!("夜裡安靜最適合打磨工具。{l_name}，你的裝備保養了嗎？"),
        "bounty_npc" => format!("聽說東邊的怪最近又往城牆靠了，{l_name}，你有聽到風聲嗎？"),
        "expedition_npc" => format!("每次看到這片星空就想出發。{l_name}，你有沒有想探的地方？"),
        "procurement_npc" => format!("跨星貿易今天波動有點大，{l_name}，你那邊物資夠用嗎？"),
        "farm_fair_npc" => format!("夜露重，明天作物應該長得不錯。{l_name}，你有在種東西嗎？"),
        "village_chief" => format!("夜深了，村子安靜就是福。{l_name}，今天辛苦了。"),
        _ => format!("夜色不錯，{l_name}，在這裡納涼嗎？"),
    }
}

/// 呼叫 LLM 生成夜談（Groq → ollama → 罐頭降級）。
/// 為純 async 函式，由 game.rs `tokio::spawn` 呼叫，不阻塞遊戲迴圈。
pub async fn generate_talk(speaker_id: &str, listener_id: &str) -> String {
    let system = build_system_prompt(speaker_id, listener_id);
    let l_name = display_name(listener_id);
    let user = format!("你看向{l_name}，說一句夜晚輕鬆閒聊的話。");
    match crate::npc_chat::raw_llm_call(&system, &user).await {
        Some(text) => {
            let t = text.trim().to_string();
            if t.is_empty() {
                canned_talk(speaker_id, listener_id)
            } else {
                t
            }
        }
        None => canned_talk(speaker_id, listener_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_trigger_during_day() {
        let mut state = PlazaTalkState::new();
        let r = state.tick(FIRST_TALK_WAIT_SECS + 1.0, false);
        assert!(r.is_none(), "白天不應觸發廣場夜談");
    }

    #[test]
    fn does_not_trigger_before_first_wait_at_night() {
        let mut state = PlazaTalkState::new();
        let r = state.tick(1.0, true);
        assert!(r.is_none(), "夜晚剛開始 1 秒不應立刻觸發");
    }

    #[test]
    fn triggers_after_first_wait_at_night() {
        let mut state = PlazaTalkState::new();
        let r = state.tick(FIRST_TALK_WAIT_SECS + 1.0, true);
        assert!(r.is_some(), "等待時間過後夜間應觸發夜談");
        let pair = r.unwrap();
        assert!(!pair.speaker_id.is_empty());
        assert!(!pair.listener_id.is_empty());
        assert_ne!(pair.speaker_id, pair.listener_id, "說話者不能是自己");
    }

    #[test]
    fn resets_cooldown_after_trigger() {
        let mut state = PlazaTalkState::new();
        state.tick(FIRST_TALK_WAIT_SECS + 1.0, true);
        let r = state.tick(1.0, true);
        assert!(r.is_none(), "觸發後應重置冷卻，不立刻再觸發");
    }

    #[test]
    fn triggers_again_after_full_cooldown() {
        let mut state = PlazaTalkState::new();
        state.tick(FIRST_TALK_WAIT_SECS + 1.0, true);
        let r = state.tick(TALK_COOLDOWN_SECS + 1.0, true);
        assert!(r.is_some(), "冷卻結束後夜間應再次觸發");
    }

    #[test]
    fn rotates_pairs() {
        let mut state = PlazaTalkState::new();
        let first = state.tick(FIRST_TALK_WAIT_SECS + 1.0, true).unwrap();
        let second = state.tick(TALK_COOLDOWN_SECS + 1.0, true).unwrap();
        assert_ne!(
            (first.speaker_id, first.listener_id),
            (second.speaker_id, second.listener_id),
            "連續兩次應是不同對話組合"
        );
    }

    #[test]
    fn canned_talk_non_empty_for_all_known_npcs() {
        let npcs = [
            "merchant", "workshop_npc", "bounty_npc", "expedition_npc",
            "procurement_npc", "farm_fair_npc", "village_chief", "unknown_npc",
        ];
        for s in &npcs {
            let t = canned_talk(s, "merchant");
            assert!(!t.is_empty(), "{s} 的罐頭夜談不應為空");
        }
    }

    #[test]
    fn build_system_prompt_contains_listener_display_name() {
        let prompt = build_system_prompt("merchant", "bounty_npc");
        assert!(prompt.contains("獵手蘭卡"), "prompt 應包含聆聽者顯示名稱");
    }

    #[test]
    fn day_does_not_count_down_cooldown() {
        let mut state = PlazaTalkState::new();
        // 只在白天 tick 超過 FIRST_TALK_WAIT_SECS，夜間首次才應觸發
        state.tick(FIRST_TALK_WAIT_SECS * 2.0, false);
        // 還沒進夜晚，cooldown 不減，所以夜間第一次 tick 還是要等
        let r = state.tick(1.0, true);
        assert!(r.is_none(), "白天計時不應影響夜談冷卻");
    }
}
