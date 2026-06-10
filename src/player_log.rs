//! 玩家個人事跡日誌（ROADMAP 67：NPC 跨角色情報網）。
//!
//! 設計鐵律：
//! - **只有引擎能寫**：記錄的是玩家實際完成的任務事實（完成訂單/懸賞/探勘…），
//!   玩家的聊天文字永遠進不來。
//! - **NPC 讀取後可自然提及**：讓 NPC 對話有「整個村子都認識你」的感覺。
//! - **記憶體模式，重啟清空**：近況快訊，不需持久化；世界重啟等同記錄歸零。
//! - **容量有限（最近 8 條）**：防止 prompt 膨脹，超出自動丟棄最舊的。
//! - **per-player 隔離**：A 的事跡不會出現在 B 的 NPC 對話中。

use std::collections::VecDeque;

/// 每位玩家最多保留幾條近期事跡。
pub const MAX_PLAYER_LOG_EVENTS: usize = 8;

/// 玩家個人事跡日誌（一個玩家一份，存在 AppState 的 HashMap 裡）。
#[derive(Debug, Default)]
pub struct PlayerLog {
    events: VecDeque<String>,
}

impl PlayerLog {
    pub fn new() -> Self {
        Self { events: VecDeque::new() }
    }

    /// 記錄一條事跡（引擎寫入，玩家文字絕不進來）。
    pub fn push(&mut self, event: impl Into<String>) {
        if self.events.len() >= MAX_PLAYER_LOG_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event.into());
    }

    /// 取得最近所有事跡（由舊到新）。
    pub fn recent(&self) -> &VecDeque<String> {
        &self.events
    }

    /// 組成可插入 system prompt 的段落。
    /// 若無事跡回空字串（不汙染 prompt）。
    pub fn to_prompt_section(&self) -> String {
        if self.events.is_empty() {
            return String::new();
        }
        let mut s = "\n\n【這位拓荒者的近期事跡（引擎紀錄・純事實，你可自然提及）】\n".to_string();
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
        let mut log = PlayerLog::new();
        log.push("在工坊完成了加急訂單");
        log.push("完成了懸賞討伐任務");
        assert_eq!(log.recent().len(), 2);
        assert_eq!(log.recent()[0], "在工坊完成了加急訂單");
    }

    #[test]
    fn max_events_cap() {
        let mut log = PlayerLog::new();
        for i in 0..12 {
            log.push(format!("事跡 {}", i));
        }
        assert_eq!(log.recent().len(), MAX_PLAYER_LOG_EVENTS);
        assert_eq!(log.recent()[0], "事跡 4");
        assert_eq!(log.recent()[MAX_PLAYER_LOG_EVENTS - 1], "事跡 11");
    }

    #[test]
    fn to_prompt_section_empty() {
        let log = PlayerLog::new();
        assert!(log.to_prompt_section().is_empty());
    }

    #[test]
    fn to_prompt_section_contains_events() {
        let mut log = PlayerLog::new();
        log.push("完成了星際採購令");
        let s = log.to_prompt_section();
        assert!(s.contains("近期事跡"));
        assert!(s.contains("星際採購令"));
    }

    #[test]
    fn max_log_events_constant_sane() {
        assert!(MAX_PLAYER_LOG_EVENTS >= 5, "至少保留 5 條，NPC 才有足夠上下文");
        assert!(MAX_PLAYER_LOG_EVENTS <= 15, "最多 15 條，防 prompt 過長");
    }

    #[test]
    fn old_events_dropped_when_full() {
        let mut log = PlayerLog::new();
        log.push("舊事跡");
        for _ in 0..MAX_PLAYER_LOG_EVENTS {
            log.push("填充");
        }
        // 最舊的「舊事跡」應被丟棄
        assert!(!log.recent().iter().any(|e| e == "舊事跡"));
    }
}
