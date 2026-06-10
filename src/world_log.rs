//! 引擎世界事件公共記憶（ROADMAP 65：公共記憶 = 引擎世界事件）。
//!
//! 設計鐵律：
//! - **只有引擎能寫**：玩家的文字永遠進不來（不是 TalkToNpc 的輸入）。
//! - **NPC 讀取後可自然提及**：讓 NPC 對話更像真實地居民，知道村子裡發生了什麼。
//! - **記憶體模式，重啟清空**：世界大事是近況快訊，不需持久化；每次重啟世界重新開始。
//! - **容量有限（最近 10 條）**：防止 prompt 膨脹，超出自動丟棄最舊的。

use std::collections::VecDeque;

/// 公共記憶最多保留這麼多條事件（最新的）。
pub const MAX_LOG_EVENTS: usize = 10;

/// 引擎世界事件公共記憶。只有引擎能 push，NPC 讀取後自然提及。
#[derive(Debug, Default)]
pub struct WorldLog {
    events: VecDeque<String>,
}

impl WorldLog {
    pub fn new() -> Self {
        Self { events: VecDeque::new() }
    }

    /// 記錄一條世界大事（引擎寫入，玩家文字絕不進來）。
    /// 超出 MAX_LOG_EVENTS 時自動丟棄最舊的。
    pub fn push(&mut self, event: impl Into<String>) {
        if self.events.len() >= MAX_LOG_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event.into());
    }

    /// 取得最近 N 條事件（由舊到新）。
    pub fn recent(&self) -> &VecDeque<String> {
        &self.events
    }

    /// 組成可插入 system prompt 的段落。若無事件回空字串（不汙染 prompt）。
    /// 讓 NPC 知道世界近況，可自然提及——但 LLM 仍只生成文字、不影響遊戲狀態。
    pub fn to_prompt_section(&self) -> String {
        if self.events.is_empty() {
            return String::new();
        }
        let mut s = "\n\n【近期世界大事（引擎紀錄・純事實，你可自然提及）】\n".to_string();
        for ev in &self.events {
            s.push_str(&format!("・{}\n", ev));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_recent() {
        let mut log = WorldLog::new();
        log.push("裂縫在東北方開啟");
        log.push("兇名精英被討伐");
        assert_eq!(log.recent().len(), 2);
        assert_eq!(log.recent()[0], "裂縫在東北方開啟");
        assert_eq!(log.recent()[1], "兇名精英被討伐");
    }

    #[test]
    fn max_events_cap() {
        let mut log = WorldLog::new();
        for i in 0..15 {
            log.push(format!("事件 {}", i));
        }
        assert_eq!(log.recent().len(), MAX_LOG_EVENTS);
        // 最舊 5 條被丟棄，留下事件 5..14
        assert_eq!(log.recent()[0], "事件 5");
        assert_eq!(log.recent()[MAX_LOG_EVENTS - 1], "事件 14");
    }

    #[test]
    fn to_prompt_section_empty() {
        let log = WorldLog::new();
        assert!(log.to_prompt_section().is_empty());
    }

    #[test]
    fn to_prompt_section_contains_events() {
        let mut log = WorldLog::new();
        log.push("村落節慶開始");
        let s = log.to_prompt_section();
        assert!(s.contains("近期世界大事"));
        assert!(s.contains("村落節慶開始"));
    }

    #[test]
    fn max_log_events_constant_sane() {
        assert!(MAX_LOG_EVENTS >= 5, "至少保留 5 條，NPC 才有足夠上下文");
        assert!(MAX_LOG_EVENTS <= 20, "最多 20 條，防止 prompt 過長");
    }
}
