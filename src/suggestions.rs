//! 遊戲內建議箱 —— 玩家回饋迴圈的伺服器端。
//!
//! 這直接服務「玩家在遊戲裡送建議 → 我收到 → 改版 → 發佈」的營運迴圈。
//! 建議同時存在記憶體（即時列出）與附加到 `data/suggestions.jsonl`（重啟後仍在、
//! 方便直接讀）。之後可無痛換成 Postgres 資料表。

use std::io::Write;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

const LOG_PATH: &str = "data/suggestions.jsonl";

/// 建議署名最長字元數（與玩家名 `sanitize_name` 的上限一致）。
pub const MAX_FROM_CHARS: usize = 24;
/// 建議內容最長字元數。夠寫一整段心得，又擋掉「灌爆建議檔」的濫用 / 壞客戶端。
pub const MAX_TEXT_CHARS: usize = 1000;

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

/// 把進來的署名 / 內容整理成要存下的 `Suggestion`：先濾控制字元、去頭尾空白、依「字元」
/// (非位元組,中文才不會被切壞)截到上限、空署名退回匿名。抽成純函式以便測試,把這條公開
/// endpoint 的輸入加固集中在一處(對齊聊天截 200 字、`sanitize_name` 截 24 字)。
///
/// 濾控制字元是必要的,且補齊先前只做 `trim`+`take` 的缺口:建議經公開未驗身的
/// `POST /api/suggestions` 進來、又會由公開的 `GET /api/suggestions` 回出,而我(維護者)
/// 多半直接在終端機讀 `data/suggestions.jsonl` 三角化——`ESC`(0x1B)等控制字元可被用來注入
/// ANSI 轉義、偽造或破壞顯示,`NUL` / `\r` 同理。比照 `sanitize_name` / `sanitize_chat`,
/// 控制字元先濾掉(不佔截斷額度)。兩欄差別:署名是單行身分欄位(對齊 `sanitize_name`),濾掉
/// 全部控制字元;內容是多行回饋(前端 `<textarea>`),保留換行 `\n` 讓玩家能分段,只濾掉換行
/// 以外的控制字元——換行存進 JSONL 會被 serde 轉義成 `\n`、不會把一筆紀錄拆成多行。
///
/// 清乾淨後內容變空(全空白 / 全控制字元)回 `None`,呼叫端據此不存——比照 `sanitize_chat`
/// 回 `Option` 的模式。這把「擋空建議」的判斷對齊到「實際會被存下的內容」這個單一真實
/// 來源:endpoint 先前只對 raw `text.trim()` 判空,而 `trim` 不濾控制字元,一則「全控制
/// 字元」的內容(如 `\0`/`ESC`,皆非空白)會通過 raw 檢查、卻在這裡被濾成空字串,仍寫進
/// JSONL 留下空建議垃圾紀錄。
fn sanitize(from: &str, text: &str, at: u64) -> Option<Suggestion> {
    let from: String = from
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_FROM_CHARS)
        .collect();
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_TEXT_CHARS)
        .collect();
    if text.is_empty() {
        return None;
    }
    Some(Suggestion {
        from: if from.is_empty() { anonymous() } else { from },
        text,
        at,
    })
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

    /// 新增一則建議；清乾淨後內容變空(全空白 / 全控制字元)回 `None`、不存任何東西，
    /// 否則回存好的紀錄。把「擋空」收斂到實際會被存下的內容上，避免空建議垃圾進檔。
    pub fn add(&self, new: NewSuggestion) -> Option<Suggestion> {
        let suggestion = sanitize(&new.from, &new.text, now_millis())?;
        append_to_disk(&suggestion);
        let mut items = self.items.lock().unwrap();
        items.push(suggestion.clone());
        tracing::info!(from = %suggestion.from, "收到玩家建議：{}", suggestion.text);
        Some(suggestion)
    }

    /// 列出所有建議（最新的在前）。
    ///
    /// 目前沒有對外路由消費它：先前的公開 `GET /api/suggestions` 已移除（未驗身就把
    /// 全部玩家回饋整包吐出的資料曝露點，且前端從不使用）。保留此方法給日後「驗身後的
    /// 後台檢視」用，屆時把 `allow(dead_code)` 拿掉即可。
    #[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_keeps_normal_input() {
        let s = sanitize("  小明  ", "  希望有貓咪  ", 42).unwrap();
        assert_eq!(s.from, "小明");
        assert_eq!(s.text, "希望有貓咪");
        assert_eq!(s.at, 42);
    }

    #[test]
    fn empty_from_falls_back_to_anonymous() {
        assert_eq!(sanitize("", "有內容", 0).unwrap().from, anonymous());
        assert_eq!(sanitize("   ", "有內容", 0).unwrap().from, anonymous());
    }

    #[test]
    fn caps_from_by_chars() {
        let long = "字".repeat(MAX_FROM_CHARS + 10);
        let s = sanitize(&long, "x", 0).unwrap();
        assert_eq!(s.from.chars().count(), MAX_FROM_CHARS);
    }

    #[test]
    fn caps_text_by_chars_not_bytes() {
        // 全中文(每字多位元組):應以字元數截斷,不是位元組數。
        let long = "乙".repeat(MAX_TEXT_CHARS + 50);
        let s = sanitize("我", &long, 0).unwrap();
        assert_eq!(s.text.chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn keeps_text_at_exactly_the_cap() {
        let exact = "a".repeat(MAX_TEXT_CHARS);
        let s = sanitize("我", &exact, 0).unwrap();
        assert_eq!(s.text.chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn strips_control_chars_from_from() {
        // 署名是單行身分欄位：換行 / 歸位 / NUL / ESC / tab 全濾掉，
        // 不讓壞客戶端把多行或終端機轉義塞進回給公開 GET 的署名。
        let s = sanitize("小\n明\r\0\u{1b}\t", "有內容", 0).unwrap();
        assert_eq!(s.from, "小明");
    }

    #[test]
    fn keeps_newlines_in_text_but_strips_other_controls() {
        // 內容是多行回饋（textarea）：保留換行讓玩家分段，但濾掉換行以外的控制字元
        // （NUL / 歸位 / ESC——維護者多在終端機讀檔，ESC 可注入 ANSI 轉義偽造顯示）。
        let s = sanitize("我", "第一段\0\r\n第二段", 0).unwrap();
        assert_eq!(s.text, "第一段\n第二段");
    }

    #[test]
    fn stripping_esc_neutralizes_ansi_injection() {
        // 注入 ANSI 轉義靠的是 ESC（0x1B）控制位元組；濾掉它，殘留的 `[31m` 只是
        // 無害的字面文字，不再能在維護者的終端機著色 / 偽造顯示。
        let s = sanitize("我", "正常\u{1b}[31m紅字", 0).unwrap();
        assert!(!s.text.contains('\u{1b}'));
        assert_eq!(s.text, "正常[31m紅字");
    }

    #[test]
    fn control_or_whitespace_only_text_is_rejected() {
        // 清乾淨後變空的內容不該被存：全控制字元（`\0`/`ESC` 等非空白，會通過
        // endpoint 對 raw `text.trim()` 的判空）以及全空白，sanitize 一律回 None，
        // 呼叫端據此回 400、不寫進 JSONL 留下空建議垃圾紀錄。
        assert!(sanitize("我", "\0\r\u{1b}\t", 0).is_none());
        assert!(sanitize("我", "\0\u{1b}", 0).is_none());
        assert!(sanitize("我", "   ", 0).is_none());
        assert!(sanitize("我", "", 0).is_none());
        // 還有可見字元的內容仍照常存下。
        assert!(sanitize("我", "\0真有建議\u{1b}", 0).is_some());
    }

    #[test]
    fn control_chars_do_not_count_toward_cap() {
        // 控制字元先濾掉、不佔截斷額度：夾在合法字元間的控制字元被移除後，
        // 仍保留滿額的可見字元。
        let mut raw = String::new();
        for _ in 0..MAX_FROM_CHARS {
            raw.push('字');
            raw.push('\0');
        }
        let s = sanitize(&raw, "x", 0).unwrap();
        assert_eq!(s.from.chars().count(), MAX_FROM_CHARS);
        assert!(!s.from.contains('\0'));
    }
}
