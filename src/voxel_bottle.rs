//! 乙太方界·漂流瓶 v1（自主提案切片）。
//!
//! **真缺口（換維度）**：至今所有社交互動都是「玩家↔居民」（贈禮/交易/心願/道別留言）或
//! 「居民↔居民」（情誼網/以物易物/串門子/長椅閒聊）——這是一個多人共居的世界，**玩家↔玩家**
//! 這條線卻完全空白：兩位真人玩家即使在同一個世界裡，彼此之間沒有任何一絲留給對方的痕跡。
//! 本刀補上這條缺口：玩家合成一只空玻璃瓶、寫上一句話丟進水裡，另一位路過水邊的玩家會
//! 撿起它、讀到一位陌生旅人的留言——世界第一次有了「玩家留給玩家」的溫柔巧遇。
//!
//! **設計**：比照告示牌（`voxel_sign.rs`）的 pos_key + append-only JSONL 範式，但語意是
//! 「一次性拾起」而非「常駐可讀」——瓶子被撿走後就從世界移除（一封信只給一位有緣人），
//! 比照箱子/告示牌「用最新一筆 replay」的持久化寫法，claim（清除）事件同樣以空字串表示。
//! **匿名**：瓶中信不記、不亮出投瓶人是誰（reader 讀不到作者），保留「陌生旅人」的巧遇感、
//! 也降低被針對騷擾的風險——內容仍在寫入前經過內容審查（`voxel_moderation::screen`）。
//! **有界成長**：全局同時存在的未拾起瓶子數有上限（`MAX_ACTIVE_BOTTLES`），避免世界被無限
//! 堆積的瓶子塞滿（狂合成瓶子亂丟）；超過上限時新的瓶子暫時丟不出去。
//!
//! 純邏輯層：零 async、零鎖、零 IO 外包；鎖/IO/廣播/內容審查/登入判定全在 `voxel_ws.rs`。

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

/// 持久化路徑。
pub const BOTTLE_PATH: &str = "data/voxel_bottles.jsonl";

/// 空玻璃瓶——純物品 id（不可放置於世界，合成後住在物品欄；首個空號，接續蛋 82 之後）。
pub const BOTTLE_ID: u8 = 83;

/// 瓶中信文字上限（字元數，非 byte）——一句短留言，過長截斷。
pub const BOTTLE_MAX_CHARS: usize = 60;

/// 全局同時存在（尚未被撿走）的瓶子數上限，避免無限堆積。
pub const MAX_ACTIVE_BOTTLES: usize = 150;

/// 世界座標鍵（字串格式 "wx,wy,wz"；與告示牌/箱子同格式）。
pub fn pos_key(wx: i32, wy: i32, wz: i32) -> String {
    format!("{wx},{wy},{wz}")
}

/// 清洗玩家輸入的瓶中信文字：去頭尾空白、控制字元（含換行/tab）換成空白、
/// 截到 `BOTTLE_MAX_CHARS` 字元、再去一次頭尾空白。確定性、無副作用、可測。
/// 回傳空字串代表「這句話清洗後等於沒寫」，呼叫端應拒絕丟出（拒空）。
pub fn sanitize_text(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cleaned.trim().chars().take(BOTTLE_MAX_CHARS).collect::<String>().trim().to_string()
}

/// 一筆瓶中信寫入事件（append-only JSONL 最小單元）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BottleEntry {
    /// 瓶子世界座標鍵。
    pub pos: String,
    /// 已清洗的文字（空字串＝這只瓶子被撿走/清除）。
    pub text: String,
    /// 單調遞增序號（replay 時取每座標最大 seq 者為現況）。
    pub seq: u64,
}

/// 全局漂流瓶 store：pos_key → 文字（只存尚未被撿走的）。
#[derive(Default)]
pub struct BottleStore {
    bottles: HashMap<String, String>,
    next_seq: u64,
}

impl BottleStore {
    /// 空 store（測試 / 首次啟動）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建 store（重啟後從 JSONL replay，每座標取最新 seq）。
    pub fn from_entries(entries: Vec<BottleEntry>) -> Self {
        let mut latest: HashMap<String, &BottleEntry> = HashMap::new();
        let mut max_seq = 0u64;
        for e in &entries {
            max_seq = max_seq.max(e.seq);
            match latest.get(&e.pos) {
                Some(prev) if prev.seq >= e.seq => {}
                _ => { latest.insert(e.pos.clone(), e); }
            }
        }
        let mut bottles = HashMap::new();
        for (pos, e) in latest {
            if !e.text.is_empty() {
                bottles.insert(pos, e.text.clone());
            }
        }
        Self { bottles, next_seq: max_seq.saturating_add(1) }
    }

    /// 目前世界上尚未被撿走的瓶子數量（供丟瓶前檢查是否已達上限）。
    pub fn len(&self) -> usize {
        self.bottles.len()
    }

    /// 是否已無瓶子（clippy 慣例：len 配對 is_empty）。
    pub fn is_empty(&self) -> bool {
        self.bottles.is_empty()
    }

    /// 某座標是否已有一只瓶子（丟瓶前避免同座標疊放）。
    pub fn has(&self, pos: &str) -> bool {
        self.bottles.contains_key(pos)
    }

    /// 丟一只瓶子（傳入已清洗、非空文字）。回傳持久化事件供呼叫方 append。
    pub fn set(&mut self, pos: &str, text: String) -> BottleEntry {
        self.bottles.insert(pos.to_string(), text.clone());
        let seq = self.next_seq;
        self.next_seq += 1;
        BottleEntry { pos: pos.to_string(), text, seq }
    }

    /// 撿起指定座標的瓶子——回傳 `(信件原文, 供 append 的清除事件)`；沒瓶子回 None。
    /// 一次性：撿走後該座標即從世界移除，其他玩家再也讀不到同一封信。
    pub fn claim(&mut self, pos: &str) -> Option<(String, BottleEntry)> {
        let text = self.bottles.remove(pos)?;
        let seq = self.next_seq;
        self.next_seq += 1;
        Some((text, BottleEntry { pos: pos.to_string(), text: String::new(), seq }))
    }

    /// 目前所有瓶子的座標（不含內文，供新玩家連線時同步世界上已有哪些瓶子供尋寶；
    /// 已按座標鍵排序求穩定）。內文絕不在此外流——只有真的撿到才讀得到。
    pub fn all_positions(&self) -> Vec<String> {
        let mut v: Vec<String> = self.bottles.keys().cloned().collect();
        v.sort();
        v
    }
}

// ── 持久化 IO（在 voxel_ws.rs 的鎖外呼叫）────────────────────────────────────────────

/// 從磁碟載入所有瓶中信事件（啟動時呼叫一次）。
pub fn load_bottles() -> Vec<BottleEntry> {
    let Ok(f) = fs::File::open(BOTTLE_PATH) else { return vec![]; };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<BottleEntry>(&l).ok())
        .collect()
}

/// Append 單筆事件。
pub fn append_bottle(entry: &BottleEntry) {
    let Ok(line) = serde_json::to_string(entry) else { return; };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(BOTTLE_PATH) else { return; };
    let _ = writeln!(f, "{line}");
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_trims_and_strips_control() {
        assert_eq!(sanitize_text("  願你一切安好  "), "願你一切安好");
        assert_eq!(sanitize_text("旅途愉快\n保重"), "旅途愉快 保重");
        assert_eq!(sanitize_text("a\tb"), "a b");
    }

    #[test]
    fn sanitize_caps_length() {
        let long = "字".repeat(100);
        let out = sanitize_text(&long);
        assert_eq!(out.chars().count(), BOTTLE_MAX_CHARS);
    }

    #[test]
    fn sanitize_empty_stays_empty() {
        assert_eq!(sanitize_text("   "), "");
        assert_eq!(sanitize_text("\n\t "), "");
    }

    #[test]
    fn set_and_get() {
        let mut store = BottleStore::new();
        let ev = store.set("1,2,3", "願你一切安好".to_string());
        assert_eq!(ev.pos, "1,2,3");
        assert!(store.has("1,2,3"));
        assert!(!store.has("9,9,9"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn claim_removes_and_returns_text() {
        let mut store = BottleStore::new();
        store.set("5,5,5", "有緣再會".to_string());
        let (text, ev) = store.claim("5,5,5").expect("有瓶子應回內容");
        assert_eq!(text, "有緣再會");
        assert_eq!(ev.text, "", "清除事件文字應為空");
        assert!(!store.has("5,5,5"), "撿走後不該再存在");
        assert_eq!(store.len(), 0);
        // 同座標再撿一次：沒有瓶子回 None，不產生多餘事件。
        assert!(store.claim("5,5,5").is_none());
    }

    #[test]
    fn claim_missing_returns_none() {
        let mut store = BottleStore::new();
        assert!(store.claim("0,0,0").is_none());
    }

    #[test]
    fn from_entries_takes_latest_seq() {
        let entries = vec![
            BottleEntry { pos: "0,0,0".into(), text: "舊".into(), seq: 0 },
            BottleEntry { pos: "0,0,0".into(), text: "新".into(), seq: 2 },
            BottleEntry { pos: "0,0,0".into(), text: "中".into(), seq: 1 },
        ];
        let store = BottleStore::from_entries(entries);
        assert!(store.has("0,0,0"));
        assert_eq!(store.next_seq, 3);
    }

    #[test]
    fn from_entries_empty_text_removes() {
        let entries = vec![
            BottleEntry { pos: "0,0,0".into(), text: "丟瓶".into(), seq: 0 },
            BottleEntry { pos: "0,0,0".into(), text: "".into(), seq: 1 }, // 被撿走
        ];
        let store = BottleStore::from_entries(entries);
        assert!(!store.has("0,0,0"), "最新是空＝已被撿走");
    }

    #[test]
    fn all_positions_sorted_and_excludes_claimed() {
        let mut store = BottleStore::new();
        store.set("2,0,0", "乙".to_string());
        store.set("1,0,0", "甲".to_string());
        store.set("3,0,0", "丙".to_string());
        store.claim("3,0,0");
        assert_eq!(store.all_positions(), vec!["1,0,0".to_string(), "2,0,0".to_string()]);
    }

    #[test]
    fn pos_key_format() {
        assert_eq!(pos_key(1, -2, 300), "1,-2,300");
    }

    #[test]
    fn len_tracks_active_count_for_cap_check() {
        let mut store = BottleStore::new();
        assert!(store.is_empty());
        for i in 0..5 {
            store.set(&pos_key(i, 0, 0), "留言".to_string());
        }
        assert_eq!(store.len(), 5);
        store.claim(&pos_key(0, 0, 0));
        assert_eq!(store.len(), 4);
    }
}
