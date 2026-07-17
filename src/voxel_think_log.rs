//! 乙太方界·居民思考可觀測基礎設施 A4（思考日誌 + 脈絡優先級排序，純邏輯／輕量 IO）。
//!
//! **真缺口**：居民每一拍都在「感知附近 → 決策 action → 給自己一個 reason」（見
//! [`crate::npc_agent::AgentDecision`]），這個內心運算過程目前只在當下被消費掉、從不落地——
//! 想回頭問「這位居民為什麼那樣做」「玩家在場時牠的選擇有沒有變」時，完全沒有可觀測的
//! 軌跡可翻。跟居民自己讀得到的日記（`voxel_diary`，第一人稱敘事）不同，本模組是給
//! **開發者/維護者**看的機器可讀思考軌跡（誰、第幾拍、附近有沒有玩家、做了什麼、為什麼、
//! 有沒有因此冒出新心願），供 M7 的接線把每次 `spawn_resident_think` 的決策順手記一筆。
//!
//! 本檔另含純函式 [`order_context`]：把要餵給思考 LLM 的多層脈絡照優先級排序——玩家在場時
//! 把玩家相關層（如 recall_note 對這位玩家的回憶）置頂，脈絡總長超標時優先砍掉可有可無的
//! 寒暄／社交層，好把有限的 token 預算留給「當下最該在意的事」。供 M7 組脈絡時取用。
//!
//! 純邏輯層：零 async、零鎖、零廣播。IO 只有一支比照 [`crate::voxel_memory::append_memory`]
//! 的輕量同步小檔 append（絕不覆寫/刪除既有行 → 向後相容、不破壞既有日誌）。
//! **成本紀律**：零 LLM、零新協議破壞、零新美術。**本模組不碰 voxel_ws.rs 接線**（那是 M7 的事）。

use serde::{Deserialize, Serialize};

/// 思考日誌落地檔（執行期產生、隨 `data/` gitignore）。
pub const THINK_LOG_PATH: &str = "data/voxel_think_log.jsonl";

/// 脈絡總長超過這個字元數就算「超長」，觸發砍寒暄社交層（見 [`order_context`]）。
/// 取值寬鬆：一般思考脈絡遠低於此，只有堆疊了大量社交回憶時才會觸頂。
pub const CONTEXT_LEN_BUDGET: usize = 1200;

/// 一筆居民思考軌跡（給開發者看的機器可讀紀錄，非居民自己讀的日記）。
///
/// 所有欄位皆 `#[serde(default)]` 友善：日後新增欄位時，舊行仍能被讀回而不報錯（向後相容）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThinkLogEntry {
    /// 這位居民的識別字串（名字）。
    pub resident: String,
    /// 這位居民思考序號（單調遞增，方便排序/去重；比照 memory 的 seq 慣例）。
    #[serde(default)]
    pub seq: u64,
    /// 這一拍附近有沒有玩家在場（影響脈絡優先級，見 [`order_context`]）。
    #[serde(default)]
    pub had_nearby_player: bool,
    /// 決策選了什麼行動（`AgentAction` 的可讀標籤，例：「Gather」「Talk:露娜」「Idle」）。
    #[serde(default)]
    pub action: String,
    /// 決策給自己的理由（`AgentDecision::reason`，可空）。
    #[serde(default)]
    pub reason: String,
    /// 這次思考有沒有順手冒出一個新心願（禱告/desire），有才記（多半為 None）。
    #[serde(default)]
    pub sparked_desire: Option<String>,
}

impl ThinkLogEntry {
    /// 方便建構：帶上必填的居民、序號、玩家在場旗標，其餘留預設。
    pub fn new(resident: impl Into<String>, seq: u64, had_nearby_player: bool) -> Self {
        Self {
            resident: resident.into(),
            seq,
            had_nearby_player,
            action: String::new(),
            reason: String::new(),
            sparked_desire: None,
        }
    }
}

/// 濾掉控制字元並 trim（比照 [`crate::voxel_memory`] 的 sanitize：日誌欄位是單行）。
fn sanitize_field(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

/// 把一筆思考軌跡 append 到 [`THINK_LOG_PATH`]。
///
/// append-only、絕不覆寫/刪除既有行 → 向後相容、不破壞既有日誌。
/// 鐵律：只在**不持任何鎖**的情境呼叫（防 prod 死鎖）。寫失敗只記 log、不 panic。
/// `resident` 空字串直接略過（沒有主體的日誌無意義）。
pub fn append_think_log(entry: &ThinkLogEntry) {
    let safe = ThinkLogEntry {
        resident: sanitize_field(&entry.resident),
        seq: entry.seq,
        had_nearby_player: entry.had_nearby_player,
        action: sanitize_field(&entry.action),
        reason: sanitize_field(&entry.reason),
        sparked_desire: entry
            .sparked_desire
            .as_deref()
            .map(sanitize_field)
            .filter(|s| !s.is_empty()),
    };
    if safe.resident.is_empty() {
        return;
    }
    write_think_line(THINK_LOG_PATH, &safe);
}

/// 從 [`THINK_LOG_PATH`] 載回所有思考軌跡（壞行略過、檔缺回空）。
pub fn load_think_logs() -> Vec<ThinkLogEntry> {
    read_think_lines(THINK_LOG_PATH)
}

/// 實際把一行 JSON append 進檔。寫失敗只記 log、不 panic（比照 `voxel_memory::write_memory_line`）。
fn write_think_line(path: &str, entry: &ThinkLogEntry) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut file) => {
            if let Ok(line) = serde_json::to_string(entry) {
                let _ = writeln!(file, "{line}");
            }
        }
        Err(e) => tracing::warn!("無法寫入居民思考日誌檔 {path}: {e}"),
    }
}

/// 讀回所有日誌行。壞行略過、檔缺回空（比照 `voxel_memory::read_memory_lines`）。
fn read_think_lines(path: &str) -> Vec<ThinkLogEntry> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                None
            } else {
                serde_json::from_str::<ThinkLogEntry>(line).ok()
            }
        })
        .collect()
}

// ── 脈絡優先級排序（純函式，供 M7 組思考脈絡時取用）───────────────────────────

/// 一層脈絡的標籤前綴：帶這些前綴的層視為「玩家相關」，玩家在場時置頂。
/// 用 `starts_with` 比對，故 `"recall_note"` 能一併涵蓋 `"recall_note:露娜"` 這類帶後綴的標籤。
const PLAYER_LAYER_PREFIXES: &[&str] = &["recall_note", "player", "玩家"];

/// 一層脈絡的標籤前綴：帶這些前綴的層視為「寒暄／社交」，脈絡超長時**優先砍掉**。
const SOCIAL_LAYER_PREFIXES: &[&str] = &["greeting", "smalltalk", "social", "寒暄", "社交"];

/// 判斷某層標籤是否命中某組前綴（英文標籤大小寫不敏感、中文標籤原樣比對）。
fn label_matches(label: &str, prefixes: &[&str]) -> bool {
    let lower = label.to_ascii_lowercase();
    prefixes
        .iter()
        .any(|p| lower.starts_with(&p.to_ascii_lowercase()) || label.starts_with(*p))
}

/// 把多層脈絡照優先級排成一段可餵給思考 LLM 的文字。
///
/// - `layers`：一組 `(標籤, 內容)`，依呼叫端原始順序給進來。
/// - `has_player`：這一拍附近有沒有玩家在場。
///
/// 規則：
/// 1. **玩家在場**時，把玩家相關層（[`PLAYER_LAYER_PREFIXES`]，如 `recall_note`）整批**置頂**，
///    其餘層維持原相對順序接在後面——當下最該在意的是眼前這位玩家。
/// 2. 內容為空（trim 後）的層直接略過。
/// 3. 串接後若總長超過 [`CONTEXT_LEN_BUDGET`]，**優先砍掉寒暄／社交層**
///    （[`SOCIAL_LAYER_PREFIXES`]）再重組；砍完仍超長也就這樣（不硬截斷單層，交由呼叫端/LLM 端處理）。
///
/// 輸出：每層一段 `"標籤: 內容"`、以換行分隔的純文字。無有效層時回空字串。
pub fn order_context(layers: &[(&str, &str)], has_player: bool) -> String {
    // 先濾掉空內容層，保留 (標籤, 內容) 借用。
    let live: Vec<(&str, &str)> = layers
        .iter()
        .filter(|(_, body)| !body.trim().is_empty())
        .copied()
        .collect();

    // 依「玩家在場則玩家層置頂」重排（維持各群組內原相對順序 → stable partition 語意）。
    let ordered: Vec<(&str, &str)> = if has_player {
        let mut player_layers: Vec<(&str, &str)> = Vec::new();
        let mut rest: Vec<(&str, &str)> = Vec::new();
        for &(label, body) in &live {
            if label_matches(label, PLAYER_LAYER_PREFIXES) {
                player_layers.push((label, body));
            } else {
                rest.push((label, body));
            }
        }
        player_layers.extend(rest);
        player_layers
    } else {
        live
    };

    let render = |ls: &[(&str, &str)]| -> String {
        ls.iter()
            .map(|(label, body)| format!("{}: {}", label, body.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let full = render(&ordered);
    if full.chars().count() <= CONTEXT_LEN_BUDGET {
        return full;
    }

    // 超長：砍掉寒暄／社交層再重組。
    let trimmed: Vec<(&str, &str)> = ordered
        .into_iter()
        .filter(|(label, _)| !label_matches(label, SOCIAL_LAYER_PREFIXES))
        .collect();
    render(&trimmed)
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn think_log_round_trips_through_json() {
        let entry = ThinkLogEntry {
            resident: "露娜".to_string(),
            seq: 42,
            had_nearby_player: true,
            action: "Talk:旅人".to_string(),
            reason: "眼前的旅人剛跟我打招呼，我想回應".to_string(),
            sparked_desire: Some("想替旅人蓋一座橋".to_string()),
        };
        let line = serde_json::to_string(&entry).expect("序列化");
        let back: ThinkLogEntry = serde_json::from_str(&line).expect("反序列化");
        assert_eq!(entry, back);
    }

    #[test]
    fn append_then_load_round_trips_via_file() {
        // 用暫存目錄避免污染真實 data/。
        let dir = std::env::temp_dir().join(format!("butfun_thinklog_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("think.jsonl");
        let path_str = path.to_str().unwrap();

        let e1 = ThinkLogEntry::new("諾娃", 1, false);
        let mut e2 = ThinkLogEntry::new("露娜", 2, true);
        e2.action = "Gather".to_string();
        e2.reason = "附近有一棵樹".to_string();

        write_think_line(path_str, &e1);
        write_think_line(path_str, &e2);

        let loaded = read_think_lines(path_str);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0], e1);
        assert_eq!(loaded[1], e2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_skips_empty_resident() {
        // resident 空字串（純空白/控制字元）sanitize 後應為空 → append_think_log 會略過。
        let empty = ThinkLogEntry::new("   ", 1, false);
        let safe_resident = sanitize_field(&empty.resident);
        assert!(safe_resident.is_empty());
    }

    #[test]
    fn sanitize_strips_control_chars_and_trims() {
        assert_eq!(sanitize_field("  你好\n世界\t "), "你好世界");
    }

    #[test]
    fn order_context_puts_player_layers_on_top_when_player_present() {
        let layers = [
            ("greeting", "路過打個招呼"),
            ("recall_note", "上次這位旅人送了我胡蘿蔔"),
            ("surroundings", "附近有一棵樹"),
        ];
        let out = order_context(&layers, true);
        let lines: Vec<&str> = out.lines().collect();
        // 玩家相關層（recall_note）必須排在最前面。
        assert!(lines[0].starts_with("recall_note:"), "recall_note 應置頂，實得：{out}");
        // 其餘層維持原相對順序（greeting 在 surroundings 之前）。
        let greet_idx = lines.iter().position(|l| l.starts_with("greeting:")).unwrap();
        let surr_idx = lines.iter().position(|l| l.starts_with("surroundings:")).unwrap();
        assert!(greet_idx < surr_idx);
    }

    #[test]
    fn order_context_keeps_order_when_no_player() {
        let layers = [
            ("recall_note", "某段回憶"),
            ("surroundings", "附近有礦脈"),
        ];
        let out = order_context(&layers, false);
        let lines: Vec<&str> = out.lines().collect();
        // 沒玩家時不重排，維持原順序。
        assert!(lines[0].starts_with("recall_note:"));
        assert!(lines[1].starts_with("surroundings:"));
    }

    #[test]
    fn order_context_skips_empty_layers() {
        let layers = [("recall_note", "   "), ("surroundings", "有東西")];
        let out = order_context(&layers, true);
        assert!(!out.contains("recall_note"));
        assert!(out.contains("surroundings"));
    }

    #[test]
    fn order_context_drops_social_layers_when_too_long() {
        // 堆一段超長社交層 + 一段關鍵層，超過預算時社交層應被砍。
        let long_social = "寒暄".repeat(CONTEXT_LEN_BUDGET); // 遠超預算
        let layers = [
            ("greeting", long_social.as_str()),
            ("recall_note", "關鍵：這位旅人答應幫我"),
        ];
        let out = order_context(&layers, true);
        assert!(!out.contains("greeting:"), "超長時寒暄層應被砍，實得：{out}");
        assert!(out.contains("recall_note:"), "關鍵層應保留");
    }

    #[test]
    fn order_context_keeps_social_layer_when_within_budget() {
        // 短脈絡時寒暄層不該被砍。
        let layers = [
            ("greeting", "嗨"),
            ("recall_note", "上次聊得開心"),
        ];
        let out = order_context(&layers, true);
        assert!(out.contains("greeting:"));
        assert!(out.contains("recall_note:"));
    }
}
