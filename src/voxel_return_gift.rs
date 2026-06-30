//! 乙太方界·互動有後果——居民回禮 v1（ROADMAP 667）。
//!
//! 核心信念：「你的互動有後果」。當玩家好感度達到門檻，居民在玩家靠近時
//! 主動送回一份小禮並附上溫暖的話——讓玩家感受到「我的善意被記住了、居民珍視我」。
//!
//! 設計鐵律：
//! - 每對（居民, 玩家）一生只送一次（防重複感）。
//! - 零 LLM：台詞確定性程式生成，成本鐵律。
//! - 純邏輯層（無 hub / 鎖 / async），由 voxel_ws.rs 包進鎖後呼叫。
//! - 持久化到 data/voxel_return_gifts.jsonl（append-only，重啟後仍記得送過）。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// 觸發回禮的好感度門檻（玩家在該居民的長期記憶筆數）。
/// 對話每次 +1、贈禮每次 +2（GIFT_MEMORY_COUNT），達 5 就有緣份。
pub const RETURN_GIFT_THRESHOLD: usize = 5;

/// 回禮距離（方塊距離，XZ 平面）：比招呼距離稍短，需要真的走到面前。
pub const RETURN_GIFT_REACH: f32 = 6.0;

/// 回禮選項（block_id, 數量）：依居民 ID 確定性選一種。
/// 5=木頭（自然材料）、14=種子（農耕傳遞），皆是居民「從自己的生活給出的東西」。
const RETURN_GIFT_OPTIONS: &[(u8, u32)] = &[
    (5, 2),  // 木頭 ×2
    (14, 3), // 種子 ×3
];

/// 回禮物品名（面向玩家顯示，集中可 i18n）。
pub fn return_item_name(block_id: u8) -> &'static str {
    match block_id {
        5 => "木頭",
        14 => "種子",
        _ => "小禮",
    }
}

/// 依居民 ID 確定性選回禮（block_id, qty）。同一居民送給不同玩家都送同一種，有個性。
pub fn pick_return_gift(resident_id: &str) -> (u8, u32) {
    let idx = resident_id.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize));
    RETURN_GIFT_OPTIONS[idx % RETURN_GIFT_OPTIONS.len()]
}

/// 回禮台詞（溫暖、帶名字，字元 ≤ 40 剛好在泡泡框內）。
/// 確定性（依居民 ID 挑模板），零 LLM，零 random。
pub fn return_gift_message(resident_name: &str, player_name: &str, item_name: &str) -> String {
    let idx = resident_name.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize));
    let pool: &[&str] = &[
        "{p}，你一直那麼好，這{i}是我的心意。",
        "{p}，謝謝你陪著我，拿去{i}吧。",
        "{p}，我想把這{i}送給你，收下吧！",
        "{p}，你對我這麼好，{i}給你，高興嗎？",
    ];
    pool[idx % pool.len()]
        .replace("{p}", player_name)
        .replace("{i}", item_name)
        .chars()
        .take(40)
        .collect()
}

/// 判斷是否應觸發回禮（純函式、可測）。
/// - `affinity`：玩家對此居民的長期記憶筆數（由 voxel_memory::affinity_count 取得）。
/// - `already_given`：此對是否已送過。
pub fn should_return_gift(affinity: usize, already_given: bool) -> bool {
    affinity >= RETURN_GIFT_THRESHOLD && !already_given
}

// ── 持久化 key（內部；不對外暴露）─────────────────────────────────────────────
fn pair_key(resident_id: &str, player_name: &str) -> String {
    format!("{resident_id}\x00{player_name}")
}

// ── ReturnGiftStore ────────────────────────────────────────────────────────────

/// 「已回禮」記錄 store（純同步結構，由 voxel_ws.rs 包進 RwLock）。
#[derive(Default)]
pub struct ReturnGiftStore {
    given: HashSet<String>,
}

impl ReturnGiftStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。
    pub fn from_entries(entries: Vec<ReturnGiftEntry>) -> Self {
        let mut s = Self::default();
        for e in entries {
            s.given.insert(pair_key(&e.resident_id, &e.player_name));
        }
        s
    }

    /// 此對是否已送過。
    pub fn already_given(&self, resident_id: &str, player_name: &str) -> bool {
        self.given.contains(&pair_key(resident_id, player_name))
    }

    /// 標記已送，回傳要持久化的 entry（呼叫端負責 append）。
    pub fn mark_given(&mut self, resident_id: &str, player_name: &str) -> ReturnGiftEntry {
        self.given.insert(pair_key(resident_id, player_name));
        ReturnGiftEntry {
            resident_id: resident_id.to_string(),
            player_name: player_name.to_string(),
        }
    }
}

// ── 持久化 ─────────────────────────────────────────────────────────────────────

/// 一筆持久化的回禮紀錄（每行一筆 JSON）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReturnGiftEntry {
    pub resident_id: String,
    pub player_name: String,
}

const RETURN_GIFT_PATH: &str = "data/voxel_return_gifts.jsonl";

/// Append 一筆回禮紀錄（不持任何鎖時呼叫；失敗只 log 不 panic）。
pub fn append_return_gift(entry: &ReturnGiftEntry) {
    let safe = ReturnGiftEntry {
        resident_id: sanitize(&entry.resident_id),
        player_name: sanitize(&entry.player_name),
    };
    if safe.resident_id.is_empty() || safe.player_name.is_empty() {
        return;
    }
    if let Ok(line) = serde_json::to_string(&safe) {
        write_line(RETURN_GIFT_PATH, &line);
    }
}

/// 載回所有已回禮紀錄（伺服器啟動時呼叫一次）。
pub fn load_return_gifts() -> Vec<ReturnGiftEntry> {
    read_lines(RETURN_GIFT_PATH)
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

fn write_line(path: &str, line: &str) {
    use std::io::Write;
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(f, "{line}");
}

fn read_lines(path: &str) -> Vec<ReturnGiftEntry> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

// ── 測試 ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_return_gift_threshold_boundary() {
        assert!(!should_return_gift(4, false), "4 筆尚未達門檻");
        assert!(should_return_gift(5, false), "5 筆剛好達門檻");
        assert!(should_return_gift(10, false), "10 筆超過門檻");
    }

    #[test]
    fn should_return_gift_already_given_blocks() {
        assert!(!should_return_gift(5, true), "已送過就不再送");
        assert!(!should_return_gift(100, true), "就算好感很高也不重送");
    }

    #[test]
    fn pick_return_gift_valid_option() {
        let (bid, qty) = pick_return_gift("vox_res_0");
        assert!(
            RETURN_GIFT_OPTIONS.iter().any(|&(b, q)| b == bid && q == qty),
            "必須是合法選項之一"
        );
        assert!(qty > 0, "數量必須 > 0");
    }

    #[test]
    fn pick_return_gift_deterministic() {
        assert_eq!(pick_return_gift("vox_res_0"), pick_return_gift("vox_res_0"));
        assert_eq!(pick_return_gift("vox_res_1"), pick_return_gift("vox_res_1"));
    }

    #[test]
    fn return_gift_message_contains_player_and_item() {
        let msg = return_gift_message("露娜", "小明", "木頭");
        assert!(msg.contains("小明"), "應含玩家名");
        assert!(msg.contains("木頭"), "應含物品名");
        assert!(msg.chars().count() <= 40, "不超過泡泡上限 40 字");
    }

    #[test]
    fn return_gift_message_all_residents() {
        for i in 0..4 {
            let rid = format!("vox_res_{i}");
            let msg = return_gift_message("諾娃", "旅人", "種子");
            assert!(!msg.is_empty(), "{rid} 台詞不應空白");
            assert!(msg.chars().count() <= 40, "{rid} 台詞超過 40 字");
        }
    }

    #[test]
    fn return_item_name_known_ids() {
        assert_eq!(return_item_name(5), "木頭");
        assert_eq!(return_item_name(14), "種子");
        assert_eq!(return_item_name(99), "小禮", "未知 id 退回「小禮」保守降級");
    }

    #[test]
    fn store_starts_empty() {
        let store = ReturnGiftStore::new();
        assert!(!store.already_given("vox_res_0", "小明"));
    }

    #[test]
    fn store_mark_and_check() {
        let mut store = ReturnGiftStore::new();
        store.mark_given("vox_res_0", "小明");
        assert!(store.already_given("vox_res_0", "小明"), "標記後應回 true");
        assert!(!store.already_given("vox_res_0", "小美"), "其他玩家不受影響");
        assert!(!store.already_given("vox_res_1", "小明"), "其他居民不受影響");
    }

    #[test]
    fn store_from_entries_restores_state() {
        let entries = vec![ReturnGiftEntry {
            resident_id: "vox_res_0".into(),
            player_name: "小明".into(),
        }];
        let store = ReturnGiftStore::from_entries(entries);
        assert!(store.already_given("vox_res_0", "小明"), "應還原已送狀態");
        assert!(!store.already_given("vox_res_1", "小明"), "其他對不受影響");
    }

    #[test]
    fn store_from_entries_dedup() {
        let entries = vec![
            ReturnGiftEntry { resident_id: "vox_res_0".into(), player_name: "小明".into() },
            ReturnGiftEntry { resident_id: "vox_res_0".into(), player_name: "小明".into() },
        ];
        let store = ReturnGiftStore::from_entries(entries);
        // 重複不應 panic，且仍正確
        assert!(store.already_given("vox_res_0", "小明"));
    }
}
