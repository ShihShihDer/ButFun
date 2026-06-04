//! 遊戲內建議箱 —— 玩家回饋迴圈的伺服器端。
//!
//! 這直接服務「玩家在遊戲裡送建議 → 我收到 → 改版 → 發佈」的營運迴圈。
//! 建議同時存在記憶體（即時列出）與附加到 `data/suggestions.jsonl`（重啟後仍在、
//! 方便直接讀）。之後可無痛換成 Postgres 資料表。

use std::io::Write;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

const LOG_PATH: &str = "data/suggestions.jsonl";

/// 一則玩家建議。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub from: String,
    pub text: String,
    /// Unix 毫秒時間戳。
    pub at: u64,
}

/// 進來的建議（HTTP 請求 body）。
#[derive(Debug, Deserialize)]
pub struct NewSuggestion {
    #[serde(default = "anonymous")]
    pub from: String,
    pub text: String,
}

fn anonymous() -> String {
    "匿名拓荒者".to_string()
}

/// 建議的存放處。可被複製（內部共享）。
#[derive(Clone)]
pub struct SuggestionStore {
    items: Arc<Mutex<Vec<Suggestion>>>,
}

impl SuggestionStore {
    pub fn new() -> Self {
        let items = load_from_disk();
        Self {
            items: Arc::new(Mutex::new(items)),
        }
    }

    /// 新增一則建議，回傳存好的紀錄。
    pub fn add(&self, new: NewSuggestion) -> Suggestion {
        let text = new.text.trim().to_string();
        let from = new.from.trim().to_string();
        let suggestion = Suggestion {
            from: if from.is_empty() { anonymous() } else { from },
            text,
            at: now_millis(),
        };
        append_to_disk(&suggestion);
        let mut items = self.items.lock().unwrap();
        items.push(suggestion.clone());
        tracing::info!(from = %suggestion.from, "收到玩家建議：{}", suggestion.text);
        suggestion
    }

    /// 列出所有建議（最新的在前）。
    pub fn list(&self) -> Vec<Suggestion> {
        let items = self.items.lock().unwrap();
        items.iter().rev().cloned().collect()
    }
}

impl Default for SuggestionStore {
    fn default() -> Self {
        Self::new()
    }
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn load_from_disk() -> Vec<Suggestion> {
    match std::fs::read_to_string(LOG_PATH) {
        Ok(contents) => contents
            .lines()
            .filter_map(|line| serde_json::from_str::<Suggestion>(line).ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn append_to_disk(s: &Suggestion) {
    if let Some(parent) = std::path::Path::new(LOG_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_PATH)
    {
        Ok(mut file) => {
            if let Ok(line) = serde_json::to_string(s) {
                let _ = writeln!(file, "{line}");
            }
        }
        Err(e) => tracing::warn!("無法寫入建議檔 {LOG_PATH}: {e}"),
    }
}
