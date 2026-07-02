//! 乙太方界·告示牌系統 v1（ROADMAP 740）。
//!
//! 玩家合成告示牌方塊後放置於世界，右鍵互動寫上一行短字（如「露娜的家」「往礦坑↓」），
//! 文字浮在牌子上、所有人都看得到——讓「採集→合成→建造」的基地第一次能被玩家親手
//! 命名、標記、導覽。人類建造／導覽維度（`docs/PLAN_ETHERVOX.md`「蓋造：更多方塊型別」）。
//!
//! **設計**：告示牌文字以世界座標 `(wx, wy, wz)` 為鍵，值為一行短字（`SIGN_MAX_CHARS` 上限）。
//! 比照箱子（ROADMAP 692）的「每座標側資料 + append-only JSONL」範式：多位玩家共用同一
//! 世界，任何人都能改寫既有牌子（先寫先廣播，序列化由 WS handler 的 RwLock 解決）。
//! 告示牌被破壞時文字一併清除（不留孤兒文字）。
//!
//! **persist**：append-only JSONL（`data/voxel_signs.jsonl`），每次寫入記一行；
//! 重啟後 replay 取每座標「最新一筆」重建現況（空字串＝清除，與破壞語意一致）。
//!
//! 純邏輯層：零 async、零鎖、零 IO 外包；鎖/IO/廣播全在 `voxel_ws.rs`。

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

/// 持久化路徑。
pub const SIGN_PATH: &str = "data/voxel_signs.jsonl";

/// 告示牌文字上限（字元數，非 byte）——一行短標記，過長截斷。
pub const SIGN_MAX_CHARS: usize = 30;

/// 世界座標鍵（字串格式 "wx,wy,wz"，JSONL 序列化用；與箱子同格式）。
pub fn pos_key(wx: i32, wy: i32, wz: i32) -> String {
    format!("{wx},{wy},{wz}")
}

/// 清洗玩家輸入的告示牌文字：去頭尾空白、控制字元（含換行/tab）換成空白、
/// 截到 `SIGN_MAX_CHARS` 字元、再去一次頭尾空白。確定性、無副作用、可測。
/// 回傳空字串代表「清除這面牌子」。
pub fn sanitize_text(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cleaned.trim().chars().take(SIGN_MAX_CHARS).collect::<String>().trim().to_string()
}

/// 一筆告示牌寫入事件（append-only JSONL 最小單元）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignEntry {
    /// 告示牌世界座標鍵。
    pub pos: String,
    /// 已清洗的文字（空字串＝清除該座標的牌子）。
    pub text: String,
    /// 單調遞增序號（replay 時取每座標最大 seq 者為現況）。
    pub seq: u64,
}

/// 全局告示牌 store：pos_key → 文字（只存非空）。
#[derive(Default)]
pub struct SignStore {
    signs: HashMap<String, String>,
    next_seq: u64,
}

impl SignStore {
    /// 空 store（測試 / 首次啟動）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建 store（重啟後從 JSONL replay，每座標取最新 seq）。
    pub fn from_entries(entries: Vec<SignEntry>) -> Self {
        // 先找每座標最新（seq 最大）那筆，避免事件亂序時舊蓋新。
        let mut latest: HashMap<String, &SignEntry> = HashMap::new();
        let mut max_seq = 0u64;
        for e in &entries {
            max_seq = max_seq.max(e.seq);
            match latest.get(&e.pos) {
                Some(prev) if prev.seq >= e.seq => {}
                _ => { latest.insert(e.pos.clone(), e); }
            }
        }
        let mut signs = HashMap::new();
        for (pos, e) in latest {
            if !e.text.is_empty() {
                signs.insert(pos, e.text.clone());
            }
        }
        Self { signs, next_seq: max_seq.saturating_add(1) }
    }

    /// 查詢某座標的告示牌文字（無牌子回 None）。
    pub fn get(&self, pos: &str) -> Option<&str> {
        self.signs.get(pos).map(|s| s.as_str())
    }

    /// 寫入／改寫告示牌文字（傳入已清洗文字）。空字串＝清除。
    /// 回傳持久化事件供呼叫方 append。
    pub fn set(&mut self, pos: &str, text: String) -> SignEntry {
        if text.is_empty() {
            self.signs.remove(pos);
        } else {
            self.signs.insert(pos.to_string(), text.clone());
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        SignEntry { pos: pos.to_string(), text, seq }
    }

    /// 清除指定座標的牌子（破壞方塊時呼叫）。有牌子才回傳清除事件（供 append）。
    pub fn clear(&mut self, pos: &str) -> Option<SignEntry> {
        if self.signs.remove(pos).is_none() {
            return None;
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        Some(SignEntry { pos: pos.to_string(), text: String::new(), seq })
    }

    /// 目前所有告示牌（供新玩家連線時一次送出），已按座標鍵排序求穩定。
    pub fn all(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> =
            self.signs.iter().map(|(k, t)| (k.clone(), t.clone())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }
}

// ── 持久化 IO（在 voxel_ws.rs 的鎖外呼叫）────────────────────────────────────────────

/// 從磁碟載入所有告示牌事件（啟動時呼叫一次）。
pub fn load_signs() -> Vec<SignEntry> {
    let Ok(f) = fs::File::open(SIGN_PATH) else { return vec![]; };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<SignEntry>(&l).ok())
        .collect()
}

/// Append 單筆事件。
pub fn append_sign(entry: &SignEntry) {
    let Ok(line) = serde_json::to_string(entry) else { return; };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(SIGN_PATH) else { return; };
    let _ = writeln!(f, "{line}");
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_trims_and_strips_control() {
        assert_eq!(sanitize_text("  露娜的家  "), "露娜的家");
        assert_eq!(sanitize_text("往礦坑\n往下"), "往礦坑 往下");
        assert_eq!(sanitize_text("a\tb"), "a b");
    }

    #[test]
    fn sanitize_caps_length() {
        let long = "字".repeat(50);
        let out = sanitize_text(&long);
        assert_eq!(out.chars().count(), SIGN_MAX_CHARS);
    }

    #[test]
    fn sanitize_empty_stays_empty() {
        assert_eq!(sanitize_text("   "), "");
        assert_eq!(sanitize_text("\n\t "), "");
    }

    #[test]
    fn set_and_get() {
        let mut store = SignStore::new();
        store.set("1,2,3", "家".to_string());
        assert_eq!(store.get("1,2,3"), Some("家"));
        assert_eq!(store.get("9,9,9"), None);
    }

    #[test]
    fn set_empty_clears() {
        let mut store = SignStore::new();
        store.set("0,0,0", "臨時".to_string());
        store.set("0,0,0", String::new());
        assert_eq!(store.get("0,0,0"), None);
    }

    #[test]
    fn clear_removes_and_returns_event() {
        let mut store = SignStore::new();
        store.set("5,5,5", "礦坑".to_string());
        let ev = store.clear("5,5,5").expect("有牌子應回清除事件");
        assert_eq!(ev.text, "");
        assert_eq!(store.get("5,5,5"), None);
        // 沒牌子的座標清除回 None（不產生多餘事件）。
        assert!(store.clear("5,5,5").is_none());
    }

    #[test]
    fn from_entries_takes_latest_seq() {
        let entries = vec![
            SignEntry { pos: "0,0,0".into(), text: "舊".into(), seq: 0 },
            SignEntry { pos: "0,0,0".into(), text: "新".into(), seq: 2 },
            SignEntry { pos: "0,0,0".into(), text: "中".into(), seq: 1 },
        ];
        let store = SignStore::from_entries(entries);
        assert_eq!(store.get("0,0,0"), Some("新"), "應取 seq 最大者");
        assert_eq!(store.next_seq, 3); // max_seq + 1
    }

    #[test]
    fn from_entries_empty_text_removes() {
        let entries = vec![
            SignEntry { pos: "0,0,0".into(), text: "立牌".into(), seq: 0 },
            SignEntry { pos: "0,0,0".into(), text: "".into(), seq: 1 }, // 破壞
        ];
        let store = SignStore::from_entries(entries);
        assert_eq!(store.get("0,0,0"), None, "最新是空＝已清除");
    }

    #[test]
    fn all_sorted_and_excludes_empty() {
        let mut store = SignStore::new();
        store.set("2,0,0", "乙".to_string());
        store.set("1,0,0", "甲".to_string());
        store.set("3,0,0", "".to_string()); // 空的不列
        let all = store.all();
        assert_eq!(all, vec![("1,0,0".into(), "甲".into()), ("2,0,0".into(), "乙".into())]);
    }

    #[test]
    fn pos_key_format() {
        assert_eq!(pos_key(1, -2, 300), "1,-2,300");
    }
}
