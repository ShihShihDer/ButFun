//! 白日工位對話（ROADMAP 81）。
//!
//! 白天 NPC 在各自工位偶爾互相閒聊，對話廣播到世界聊天頻道。
//! 與廣場夜談（ROADMAP 76）互補：夜晚廣場聊家常，白天工位聊公事。
//!
//! 成本紀律：
//! - 每 TALK_COOLDOWN_SECS 秒觸發一次（冷卻期間不觸發）。
//! - 只有白天（Phase::Day）才倒數；黎明/黃昏/夜晚暫停計時。
//! - 全局 Semaphore(1)，防止並發 LLM 呼叫。
//! - 降級鏈：Groq → ollama → 罐頭。
//! - 只有白天且有玩家在線時觸發。
//! - `BUTFUN_NPC_LLM=1` 未設定時直接回罐頭，不呼叫任何外部 API。

/// 白日工位對話觸發冷卻（秒）。
pub const TALK_COOLDOWN_SECS: f32 = 480.0; // 8 分鐘

/// 伺服器啟動後首次觸發的最短等待（秒）。
const FIRST_TALK_WAIT_SECS: f32 = 180.0; // 3 分鐘，避免剛啟動就廣播

/// 白日工位對話專屬 Semaphore 容量：同時最多 1 個 AI 呼叫。
pub const MAX_CONCURRENT_TALKS: usize = 1;

/// 一次白日對話的說話者／聆聽者對（皆為靜態 NPC id）。
#[derive(Debug, Clone)]
pub struct DaytimeTalkPair {
    pub speaker_id: &'static str,
    pub listener_id: &'static str,
}

/// 白日工位對話狀態（純記憶體，重啟清零）。
pub struct DaytimeTalkState {
    /// 距下次觸發倒數（秒）；只在白天遞減。
    cooldown: f32,
    /// 輪換對話組合的索引。
    pair_index: usize,
}

/// 預定義白天輪換對話搭檔（speaker, listener）。
/// 與廣場夜談（plaza_talk.rs）的搭檔完全不重複，反映白天工作日常。
static PAIRS: &[(&str, &str)] = &[
    ("merchant", "workshop_npc"),         // 商人詢問工匠最新需求
    ("workshop_npc", "bounty_npc"),       // 工匠問獵手需要什麼工具
    ("bounty_npc", "village_chief"),      // 獵手向長老報告邊境情況
    ("expedition_npc", "procurement_npc"), // 探勘員分享星際發現
    ("farm_fair_npc", "merchant"),        // 老農詢問農產品市場行情
    ("village_chief", "expedition_npc"),  // 長老詢問探勘進度
    ("procurement_npc", "merchant"),      // 採購代理人詢問貨源
    ("bounty_npc", "farm_fair_npc"),      // 獵手與老農聊城外動態
    ("workshop_npc", "expedition_npc"),   // 工匠問探勘員需要哪些工具
    ("village_chief", "procurement_npc"), // 長老關心星際貿易情況
];

impl DaytimeTalkState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_TALK_WAIT_SECS,
            pair_index: 0,
        }
    }

    /// 推進時間。只在白天（is_day=true）才倒數；有對話就回傳並重置冷卻。
    pub fn tick(&mut self, dt: f32, is_day: bool) -> Option<DaytimeTalkPair> {
        if !is_day {
            return None;
        }
        self.cooldown -= dt;
        if self.cooldown <= 0.0 {
            self.cooldown = TALK_COOLDOWN_SECS;
            let (sid, lid) = PAIRS[self.pair_index % PAIRS.len()];
            self.pair_index += 1;
            Some(DaytimeTalkPair { speaker_id: sid, listener_id: lid })
        } else {
            None
        }
    }
}

/// 取得 NPC 顯示名稱（從 npc_chat 共用資料）。
pub fn display_name(id: &str) -> &'static str {
    crate::npc_chat::find_npc(id).map(|n| n.display).unwrap_or("村民")
}

/// 建立 AI 白日對話的 system prompt。
pub fn build_system_prompt(speaker_id: &str, listener_id: &str) -> String {
    let persona = crate::npc_chat::find_npc(speaker_id)
        .map(|n| n.persona)
        .unwrap_or("");
    let l_name = display_name(listener_id);
    format!(
        "{persona}\
        現在是白天，你正在工作崗位上，剛好遇到{l_name}。\
        請用 25 字以內的繁體中文說一句白天工作相關的話語——可以聊訂單、任務、材料、\
        近況或與對方工作有關的話題。語氣自然，符合你的職業個性。\
        只輸出那一句話，不加引號、不加前綴。"
    )
}

/// 罐頭白日對話（LLM 未啟用或呼叫失敗時的後備）。
pub fn canned_talk(speaker_id: &str, listener_id: &str) -> String {
    let l_name = display_name(listener_id);
    match speaker_id {
        "merchant" => format!("{l_name}，最近有沒有什麼貨想出手？我這邊需求旺盛。"),
        "workshop_npc" => format!("{l_name}，工具要定期保養，有需要的話隨時找我。"),
        "bounty_npc" => format!("{l_name}，東邊幾隻怪最近特別囂張，你有空嗎？"),
        "expedition_npc" => format!("{l_name}，上次探勘發現了有趣的礦脈，改天說給你聽。"),
        "farm_fair_npc" => format!("{l_name}，今季收成不錯，有沒有需要補貨的品項？"),
        "village_chief" => format!("{l_name}，村子的事都靠你們了，有任何事跟我說。"),
        "procurement_npc" => format!("{l_name}，最近跨星物資有批好貨，你有興趣嗎？"),
        _ => format!("{l_name}，白天辛苦了，有什麼需要幫忙的嗎？"),
    }
}

/// 呼叫 LLM 生成白日對話（Groq → ollama → 罐頭降級）。
/// 為純 async 函式，由 game.rs `tokio::spawn` 呼叫，不阻塞遊戲迴圈。
pub async fn generate_talk(speaker_id: &str, listener_id: &str) -> String {
    let system = build_system_prompt(speaker_id, listener_id);
    let l_name = display_name(listener_id);
    let user = format!("你看向{l_name}，說一句白天工作的話。");
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
    fn does_not_trigger_during_night() {
        let mut state = DaytimeTalkState::new();
        let r = state.tick(FIRST_TALK_WAIT_SECS + 1.0, false);
        assert!(r.is_none(), "夜晚不應觸發白日工位對話");
    }

    #[test]
    fn does_not_trigger_before_first_wait_during_day() {
        let mut state = DaytimeTalkState::new();
        let r = state.tick(1.0, true);
        assert!(r.is_none(), "白天剛開始 1 秒不應立刻觸發");
    }

    #[test]
    fn triggers_after_first_wait_during_day() {
        let mut state = DaytimeTalkState::new();
        let r = state.tick(FIRST_TALK_WAIT_SECS + 1.0, true);
        assert!(r.is_some(), "等待時間過後白天應觸發工位對話");
        let pair = r.unwrap();
        assert!(!pair.speaker_id.is_empty());
        assert!(!pair.listener_id.is_empty());
        assert_ne!(pair.speaker_id, pair.listener_id, "說話者不能是聆聽者");
    }

    #[test]
    fn resets_cooldown_after_trigger() {
        let mut state = DaytimeTalkState::new();
        state.tick(FIRST_TALK_WAIT_SECS + 1.0, true);
        let r = state.tick(1.0, true);
        assert!(r.is_none(), "觸發後應重置冷卻，不立刻再觸發");
    }

    #[test]
    fn triggers_again_after_full_cooldown() {
        let mut state = DaytimeTalkState::new();
        state.tick(FIRST_TALK_WAIT_SECS + 1.0, true);
        let r = state.tick(TALK_COOLDOWN_SECS + 1.0, true);
        assert!(r.is_some(), "冷卻結束後白天應再次觸發");
    }

    #[test]
    fn rotates_pairs() {
        let mut state = DaytimeTalkState::new();
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
            "merchant",
            "workshop_npc",
            "bounty_npc",
            "expedition_npc",
            "procurement_npc",
            "farm_fair_npc",
            "village_chief",
            "unknown_npc",
        ];
        for s in &npcs {
            let t = canned_talk(s, "merchant");
            assert!(!t.is_empty(), "{s} 的罐頭對話不應為空");
        }
    }

    #[test]
    fn build_system_prompt_contains_listener_display_name() {
        let prompt = build_system_prompt("merchant", "workshop_npc");
        assert!(prompt.contains("工匠老胡"), "prompt 應包含聆聽者顯示名稱");
    }

    #[test]
    fn night_does_not_count_down_cooldown() {
        let mut state = DaytimeTalkState::new();
        // 只在夜晚 tick 超過 FIRST_TALK_WAIT_SECS，白天首次才應觸發
        state.tick(FIRST_TALK_WAIT_SECS * 2.0, false);
        // 夜晚計時不減，白天第一次 tick 仍要等
        let r = state.tick(1.0, true);
        assert!(r.is_none(), "夜晚計時不應影響白日對話冷卻");
    }

    #[test]
    fn all_pairs_have_different_speaker_and_listener() {
        for (s, l) in PAIRS {
            assert_ne!(s, l, "說話者與聆聽者不應相同: {s}");
        }
    }
}
