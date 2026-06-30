//! 自主 agent 的 live 接線（P0「由 AI 棲居的世界」第一步）。
//!
//! `npc_agent.rs` 是純決策地基（`SenseInput` → `npc_think` → `AgentDecision`），碰不到世界、
//! 不持鎖、不 await 改狀態。本模組只補上接線**所需的純邏輯與一個無鎖的決策匯流排**——
//! 真正的「短鎖快照 → drop → tokio::spawn 思考 → 下一 tick 同步套用」全在 `game.rs` 主迴圈裡，
//! 比照 `ws.rs` 處理 `TalkToNpc` 的做法。
//!
//! 設計鐵律（嚴守 prod 死鎖事故的教訓）：
//! - 決策**不在 async task 裡改世界**：task 只把 `(居民 id, AgentDecision)` 投進 `AgentBus`，
//!   主迴圈下一 tick 在同步、短鎖、不 await 的情況下套用。
//! - `AgentBus` 的兩把 `Mutex` 只在「投／取／標記」的瞬間短暫上鎖，**絕不跨 await 持有**。
//! - 低頻（每 ~15 秒想一次）、極少量（只有頭 `AGENT_ENABLED_COUNT` 位居民）、可整個關閉。

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

use crate::npc_agent::AgentDecision;
use crate::resident_npc::ResidentPersona;

/// 會「自己思考」的居民數量上限：只有 id 為 `resident_0` .. `resident_{N-1}` 的居民是 agent。
/// 刻意極少（成本鐵律 + 風險最小化）；要更多再往上調。
pub const AGENT_ENABLED_COUNT: usize = 2;

/// 兩次思考的間隔（秒）。**刻意拉長到 90 秒**（原 45）：背景思考量直接砍半，
/// 大幅省下免費腦池額度（12 位居民 × 每 45 秒思考一次曾把額度燒爆）。思考已改走便宜／
/// 獨立額度（Cerebras/Gemini/ollama，不碰 Groq，見 `npc_chat::think_llm_chat`），間隔再放長
/// 讓那些獨立額度也耐用；居民「想事情」沒那麼頻繁，反而更像真人慢生活、不洗版。
pub const THINK_INTERVAL_SECS: f32 = 90.0;

/// 感知半徑（像素）：蒐集這個範圍內的玩家 / 可採節點餵給 agent 當情境。
pub const SENSE_RADIUS: f32 = 360.0;

/// 整個 agent live 接線的總開關。預設**開**（這是 live 接線）；設 `BUTFUN_NPC_AGENT=0` 可整個關掉，
/// 關掉後居民完全回到原本的模板行為、零額外動作。
///
/// 注意這與 `BUTFUN_NPC_LLM` 是**兩件不同的事**：
/// - 本開關控「要不要跑 agent 思考」。
/// - `BUTFUN_NPC_LLM` 控「思考時要不要真的呼 LLM」——關閉時 `npc_think` 內部自然走 `canned_action`
///   規則化後備，agent **仍會動**，只是不花 LLM。
pub fn agents_enabled() -> bool {
    std::env::var("BUTFUN_NPC_AGENT").map(|v| v != "0").unwrap_or(true)
}

/// 現行 agent 名額：可用環境變數 `BUTFUN_AGENT_COUNT` 覆寫（**不必重建即可調**），
/// 未設或壞值時退回常數 `AGENT_ENABLED_COUNT`（2）。設成 ≥ `MAX_POPULATION`(12) ＝ 全部居民都是 agent。
pub fn agent_count() -> usize {
    std::env::var("BUTFUN_AGENT_COUNT")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(AGENT_ENABLED_COUNT)
}

/// 判斷某居民 id（`"resident_{n}"`）是否落在 agent 名額內（`n < agent_count()`）。
pub fn is_agent_id(id: &str) -> bool {
    id.strip_prefix("resident_")
        .and_then(|n| n.parse::<usize>().ok())
        .map(|n| n < agent_count())
        .unwrap_or(false)
}

/// 決策匯流排：async 思考 task 把決策投進來，主迴圈下一 tick 取走套用。
///
/// 兩把鎖都只在「投／取／標記」的瞬間短暫上鎖，**絕不跨 `.await` 持有**（守 prod 死鎖鐵律）。
pub struct AgentBus {
    /// 待套用的決策佇列：`(居民 id, 決策)`。主迴圈每 tick 排空。
    inbox: Mutex<VecDeque<(String, AgentDecision)>>,
    /// 正在思考中的居民 id 集合：防同一居民上一次思考還沒回來、就又被發起新的一次
    /// （思考可能比間隔久，例如 ollama 逾時 20 秒 > 15 秒間隔）。
    thinking: Mutex<HashSet<String>>,
}

impl AgentBus {
    pub fn new() -> Self {
        Self {
            inbox: Mutex::new(VecDeque::new()),
            thinking: Mutex::new(HashSet::new()),
        }
    }

    /// 嘗試把某居民標記為「思考中」。回 `true` 表示先前沒有思考在飛、可以發起新的一次；
    /// 回 `false` 表示已有一次在進行，這次略過。短鎖、不跨 await。
    pub fn try_begin_thinking(&self, id: &str) -> bool {
        // HashSet::insert 回 true = 原本不存在 = 這次才是新發起的。
        self.thinking.lock().unwrap().insert(id.to_string())
    }

    /// 思考結束（不論成功失敗）解除「思考中」標記。短鎖、不跨 await。
    pub fn end_thinking(&self, id: &str) {
        self.thinking.lock().unwrap().remove(id);
    }

    /// 投入一筆決策，等主迴圈下一 tick 套用。短鎖、不跨 await。
    pub fn push_decision(&self, id: String, decision: AgentDecision) {
        self.inbox.lock().unwrap().push_back((id, decision));
    }

    /// 排空所有待套用決策（主迴圈同步呼叫）。短鎖、不跨 await。
    pub fn drain(&self) -> Vec<(String, AgentDecision)> {
        self.inbox.lock().unwrap().drain(..).collect()
    }
}

impl Default for AgentBus {
    fn default() -> Self {
        Self::new()
    }
}

/// 把居民的行為類型轉成餵給 LLM 的人設字串（純函式、可測）。
/// 刻意維持「平凡居民」口吻——他們不是有完整生命週期的主要 NPC，只是讓世界有人氣的小角色。
pub fn resident_agent_persona(name: &str, persona: ResidentPersona) -> String {
    let trait_line = match persona {
        ResidentPersona::MarketBrowser => "你平時在市集攤區一帶閒逛，愛湊熱鬧、和商販與路人聊天。",
        ResidentPersona::FarmWorker => "你平時在城南的公共農田一帶勞動，務實勤懇，喜歡靠雙手過日子。",
        ResidentPersona::TownSquare => "你平時待在城鎮廣場與中心一帶，喜歡看人來人往、和鄰里寒暄。",
        ResidentPersona::Wanderer => "你喜歡在整座城裡四處走動，好奇心重，哪裡有趣就往哪裡去。",
    };
    format!(
        "你是住在新手村主城的一位平凡居民，名叫{name}。{trait_line}你不是什麼大人物，\
        只是過著自己的小日子——會在城裡走動、和遇到的人攀談、活力低時就近採點資源補貼生活。\
        請以這個身份，依當下情境決定你接下來最自然會做的一件小事。"
    )
}

/// 把採集節點種類轉成簡短可讀標籤（餵給 agent 感知用）。
pub fn node_kind_label(kind: crate::gather::NodeKind) -> &'static str {
    match kind {
        crate::gather::NodeKind::Tree => "樹",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc_agent::{AgentAction, AgentDecision};

    #[test]
    fn is_agent_id_and_count_env() {
        // 同一個 env 變數的測試全放這支裡、順序執行，避免平行測試互搶 process 全域 env。
        // 也釘住值，讓本測試不受部署環境 .env 的 BUTFUN_AGENT_COUNT 影響（否則 cargo test 會掛、部署中止）。
        // 名額讀 env：未設退回預設、壞值退回預設、不 panic。
        std::env::remove_var("BUTFUN_AGENT_COUNT");
        assert_eq!(agent_count(), AGENT_ENABLED_COUNT, "未設時退回預設常數");
        std::env::set_var("BUTFUN_AGENT_COUNT", "壞值");
        assert_eq!(agent_count(), AGENT_ENABLED_COUNT, "壞值退回預設、不 panic");
        std::env::set_var("BUTFUN_AGENT_COUNT", "12");
        assert_eq!(agent_count(), 12);
        // 名額 = 2 時只有 resident_0/1 是 agent。
        std::env::set_var("BUTFUN_AGENT_COUNT", "2");
        assert!(is_agent_id("resident_0"));
        assert!(is_agent_id("resident_1"));
        assert!(!is_agent_id("resident_2"));
        assert!(!is_agent_id("resident_9"));
        // 名額 = 12 時 resident_9 也是 agent（驗證 env 真的生效）。
        std::env::set_var("BUTFUN_AGENT_COUNT", "12");
        assert!(is_agent_id("resident_9"));
        // 格式不符一律 false（不 panic）。
        assert!(!is_agent_id("merchant"));
        assert!(!is_agent_id("resident_"));
        assert!(!is_agent_id("resident_abc"));
        assert!(!is_agent_id(""));
        std::env::remove_var("BUTFUN_AGENT_COUNT");
    }

    #[test]
    fn persona_string_mentions_name_and_is_non_empty() {
        for p in [
            ResidentPersona::MarketBrowser,
            ResidentPersona::FarmWorker,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let s = resident_agent_persona("露娜", p);
            assert!(!s.is_empty());
            assert!(s.contains("露娜"), "人設字串應含居民名字：{s}");
        }
    }

    #[test]
    fn node_label_non_empty() {
        assert!(!node_kind_label(crate::gather::NodeKind::Tree).is_empty());
    }

    #[test]
    fn bus_thinking_guard_blocks_reentry() {
        let bus = AgentBus::new();
        // 第一次標記成功（可發起）。
        assert!(bus.try_begin_thinking("resident_0"));
        // 還在飛 → 第二次被擋。
        assert!(!bus.try_begin_thinking("resident_0"));
        // 結束後可再次發起。
        bus.end_thinking("resident_0");
        assert!(bus.try_begin_thinking("resident_0"));
    }

    #[test]
    fn bus_inbox_push_then_drain() {
        let bus = AgentBus::new();
        assert!(bus.drain().is_empty());
        bus.push_decision(
            "resident_0".to_string(),
            AgentDecision::new(AgentAction::Gather, "", "採點木頭"),
        );
        bus.push_decision("resident_1".to_string(), AgentDecision::idle());
        let drained = bus.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].0, "resident_0");
        assert_eq!(drained[0].1.action, AgentAction::Gather);
        // 排空後再排空為空。
        assert!(bus.drain().is_empty());
    }

    #[test]
    fn agents_enabled_default_on_when_unset() {
        std::env::remove_var("BUTFUN_NPC_AGENT");
        assert!(agents_enabled(), "未設環境變數時應預設開啟（live 接線）");
        std::env::set_var("BUTFUN_NPC_AGENT", "0");
        assert!(!agents_enabled(), "設為 0 時應關閉");
        std::env::remove_var("BUTFUN_NPC_AGENT");
    }
}
