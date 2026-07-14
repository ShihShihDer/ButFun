//! 乙太方界·家居擺設 v1（自主提案切片，接續 931「玩家裝飾傢俱 v1」）——玩家送的家具，
//! 第一次真的擺進她家、鄰居看了會心生嚮往。
//!
//! **補的缺口**：931 讓玩家能合成小地毯／花盆／小圓桌／掛旗四樣純裝飾傢俱，自己蓋房子時
//! 隨手佈置。但「送給居民」這條路從沒真的接上——`voxel_gift::item_name_zh` 甚至沒登記這
//! 四樣的名字（送出去只顯示 fallback「物品」），道謝後材料憑空消失，跟蓋家（652/990）當年
//! 「材料憑空生方塊」是同一種缺口。本刀讓「你送的家具」第一次真的變成她家的一部分：擺進
//! 世界、持久顯示、寫進她的記憶。
//!
//! **居民↔居民的新因果**：鄰居走近另一位居民的家，看到一件自己家還沒有的擺設，會心生
//! 嚮往、記下想要哪一款——這份嚮往只等你送對東西就能實現（不會自己生出來），讓「一戶人家
//! 的品味」有機會漸漸在鄰里之間擴散，而不是四棟房子永遠互不相干。
//!
//! **與既有系統 razor-sharp 區隔**：931 是「傢俱存在、可放置」；本刀是「傢俱被居民珍視、
//! 擺進她的家、鄰居會羨慕」——傢俱本身的方塊定義不變，本刀只接「送給誰、擺哪、誰會想要」
//! 這條全新因果線。與 858 見賢思齊（羨慕地標→想蓋同類建物）刻意區隔：見賢思齊羨慕的是
//! 「一整座建物種類」，本刀羨慕的是「鄰居家裡的一件擺設」，一大一小、互不重疊。
//!
//! 純邏輯層（無 IO、無鎖、無 async），IO／鎖在 `voxel_ws.rs`。
//! 持久化格式：`data/voxel_home_decor.jsonl`（append-only，每行一筆 `DecorEntry`）。

use crate::voxel_furniture as vfurn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── 常數 ──────────────────────────────────────────────────────────────────────

/// 一戶人家最多同時展示幾件裝飾傢俱——刻意小（療癒向的「幾件珍藏」，非無限堆積雜物間）。
pub const DECOR_SLOT_MAX: usize = 3;

/// 展示格相對居民家錨點的水平位移（依展示序位取模，越後面的離家門越遠一點）；
/// y 由呼叫端各自算 `surface_y` 貼地，不寫死高度。
const SLOT_OFFSETS: [(i32, i32); DECOR_SLOT_MAX] = [(2, 1), (-2, 1), (0, -2)];

/// 鄰居嚮往（見賢思齊同款節奏）：同一位居民兩次心生嚮往之間的冷卻秒數。設得長，
/// 讓「注意到鄰居家的擺設」稀有有份量，不會每次路過都心猿意馬。
pub const ENVY_COOLDOWN_SECS: f32 = 600.0;

/// 走到鄰居家多近才算「看得到擺設」（世界方塊，水平 XZ 平面）——比照 858 見賢思齊同量級。
pub const ENVY_RADIUS: f32 = 14.0;

/// 每次符合條件（在鄰居家附近＋冷卻到期）時真的心生嚮往的機率。刻意不設 1.0——
/// 不是每次路過都會觸發，偶爾才羨慕，像真的生活裡不經意的一瞥。
pub const ENVY_CHANCE: f32 = 0.35;

/// 泡泡／Feed 字元上限（與其他社交泡泡同框，超長截斷不破框）。
const SAY_MAX_CHARS: usize = 50;

// ── 純函式 ────────────────────────────────────────────────────────────────────

/// 依展示序位（0-based，`% SLOT_OFFSETS.len()` 越界安全）算出相對家錨點的水平位移。
/// 純函式、確定性、可測。
pub fn slot_offset(index: usize) -> (i32, i32) {
    SLOT_OFFSETS[index % SLOT_OFFSETS.len()]
}

/// 是否該觸發鄰居嚮往：冷卻已到期 + 過機率門檻。純函式、確定性（機率骰由呼叫端傳入）。
pub fn envy_triggers(cooldown_ready: bool, roll: f32) -> bool {
    cooldown_ready && roll < ENVY_CHANCE
}

// ── 持久化格式 ────────────────────────────────────────────────────────────────

/// 一筆持久化記錄：某位居民家裡展示了一件裝飾傢俱。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecorEntry {
    pub resident: String,
    pub item_id: u8,
    pub seq: u64,
}

// ── 家居擺設帳本 ──────────────────────────────────────────────────────────────

/// 家居擺設帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct DecorStore {
    /// resident id → 已展示的傢俱 id（依展示順序，去重、封頂 `DECOR_SLOT_MAX`）。
    displayed: HashMap<String, Vec<u8>>,
    /// resident id → 目前嚮往的一款傢俱 id（見賢思齊同款「同時只掛一個」，純記憶體、
    /// 重啟歸零——這是尚未實現的念頭，不是既成事實，跟隨 `open_request` 同款慣例不入持久化）。
    wanted: HashMap<String, u8>,
    next_seq: u64,
}

impl DecorStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。依序重播，天然去重＋封頂（與執行期 `add_piece`
    /// 同一套規則，故重播結果與執行期累積永遠一致）。
    pub fn from_entries(entries: impl IntoIterator<Item = DecorEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            let v = s.displayed.entry(e.resident).or_default();
            if !v.contains(&e.item_id) && v.len() < DECOR_SLOT_MAX {
                v.push(e.item_id);
            }
        }
        s
    }

    /// 這位居民家裡已展示的傢俱 id 列表（依展示順序）。
    pub fn displayed_for(&self, resident: &str) -> &[u8] {
        self.displayed.get(resident).map_or(&[], |v| v.as_slice())
    }

    /// 這位居民家裡是否已展示過這款傢俱。
    pub fn has_piece(&self, resident: &str, item_id: u8) -> bool {
        self.displayed
            .get(resident)
            .is_some_and(|v| v.contains(&item_id))
    }

    /// 展示格是否已滿（達 `DECOR_SLOT_MAX`）。
    pub fn is_full(&self, resident: &str) -> bool {
        self.displayed.get(resident).is_some_and(|v| v.len() >= DECOR_SLOT_MAX)
    }

    /// 新增一件展示傢俱。已有這款、或展示格已滿 → 回 `None`（無新事實，呼叫端不落地，
    /// 天然防止「送同一款很多次」或「超過展示上限」無界累積——同款守衛見 981 工具耐久）。
    /// 成功則回傳供呼叫端 append 落地的記錄。
    pub fn add_piece(&mut self, resident: &str, item_id: u8) -> Option<DecorEntry> {
        if self.has_piece(resident, item_id) || self.is_full(resident) {
            return None;
        }
        self.displayed
            .entry(resident.to_string())
            .or_default()
            .push(item_id);
        let rec = DecorEntry {
            resident: resident.to_string(),
            item_id,
            seq: self.next_seq,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        Some(rec)
    }

    /// 這位居民目前嚮往哪一款（`None` = 沒有）。
    pub fn wanted_for(&self, resident: &str) -> Option<u8> {
        self.wanted.get(resident).copied()
    }

    /// 立下一個新的嚮往。已擁有這款、或已經嚮往同一款 → 回 `false`（無變化，呼叫端據此
    /// 略過冒泡/記憶/動態牆，避免每次掃描都重覆觸發同一件事）。同時只掛一個嚮往
    /// （見賢思齊同款慣例），新嚮往會覆蓋舊的（見異思遷也是人之常情）。
    pub fn set_wanted(&mut self, resident: &str, item_id: u8) -> bool {
        if self.has_piece(resident, item_id) {
            return false;
        }
        if self.wanted.get(resident) == Some(&item_id) {
            return false;
        }
        self.wanted.insert(resident.to_string(), item_id);
        true
    }

    /// 清除嚮往（心願已被滿足，或呼叫端另有理由要重置）。
    pub fn clear_wanted(&mut self, resident: &str) {
        self.wanted.remove(resident);
    }
}

// ── 文案（繁中、面向玩家，集中可 i18n）────────────────────────────────────────

/// 是否為本刀可展示的裝飾傢俱 id（沿用 931 定義，避免重複判定邏輯）。
pub fn is_decor_item(item_id: u8) -> bool {
    vfurn::is_furniture_id(item_id)
}

fn truncate(s: String) -> String {
    s.chars().take(SAY_MAX_CHARS).collect()
}

/// 傢俱真的擺進她家時的道謝泡泡（比一般贈禮更雀躍——這是「你送的家具」第一次真的
/// 變成她家的一部分）。`pick` 由呼叫端輪替，確定性。
pub fn display_thanks_line(item_name: &str, pick: usize) -> String {
    let pool = [
        format!("這下我家終於有{item_name}了，謝謝你！我要擺在最顯眼的地方。"),
        format!("哇，{item_name}！我這就把它擺進家裡，天天都能看到。"),
        format!("你送的{item_name}，我馬上找了個好位置擺上——家裡感覺不一樣了！"),
    ];
    truncate(pool[pick % pool.len()].clone())
}

/// 這款正是她先前嚮往的擺設，願望被實現時的專屬感謝（比一般展示更觸動）。
pub fn wish_fulfilled_thanks_line(item_name: &str, pick: usize) -> String {
    let pool = [
        format!("你、你怎麼知道我一直想要{item_name}！謝謝你，這下家裡終於也有一件了。"),
        format!("這正是我心心念念的{item_name}！你聽見了，真的謝謝你。"),
    ];
    truncate(pool[pick % pool.len()].clone())
}

/// 動態牆：某居民家裡多了一件玩家送的擺設。
pub fn display_feed_line(resident_name: &str, item_name: &str) -> String {
    format!("{resident_name}把你送的{item_name}擺進了家裡，天天都能看見。")
}

/// 動態牆：某居民多年的嚮往被實現了。
pub fn wish_fulfilled_feed_line(resident_name: &str, item_name: &str) -> String {
    format!("{resident_name}一直嚮往的{item_name}，終於擺進了家裡。")
}

/// 記進居民自己記憶的一筆：在家裡擺上了你送的這件擺設（第一人稱、episodic）。
pub fn display_memory_line(item_name: &str) -> String {
    format!("在家裡擺上了你送的{item_name}，每次看到都覺得家更有生活感了")
}

/// 鄰居嚮往時的泡泡：羨慕地看著鄰居家的某款擺設，也想擁有一件。
pub fn wanted_bubble(neighbor_name: &str, item_name: &str, pick: usize) -> String {
    let pool = [
        format!("{neighbor_name}家的{item_name}真好看……我家也好想要一件。"),
        format!("每次經過{neighbor_name}家都會多看兩眼那件{item_name}，好想擁有一件。"),
        format!("要是我家也能擺上一件像{neighbor_name}家那樣的{item_name}就好了。"),
    ];
    truncate(pool[pick % pool.len()].clone())
}

/// 動態牆：某居民羨慕起鄰居家的某款擺設。
pub fn wanted_feed_line(resident_name: &str, neighbor_name: &str, item_name: &str) -> String {
    format!("{resident_name}很喜歡{neighbor_name}家擺的{item_name}，也悄悄想擁有一件。")
}

/// 記進居民自己記憶的一筆：羨慕起鄰居家的擺設（第一人稱、episodic）。
pub fn wanted_memory_line(neighbor_name: &str, item_name: &str) -> String {
    format!("看見{neighbor_name}家擺著{item_name}，忍不住也想要一件")
}

// ── 持久化 IO（只有函式，鎖在 voxel_ws.rs）──────────────────────────────────

const DECOR_FILE: &str = "data/voxel_home_decor.jsonl";

/// 從 `data/voxel_home_decor.jsonl` 讀取所有記錄（檔案不存在或壞行皆容忍，回空/略過）。
pub fn load_entries() -> Vec<DecorEntry> {
    let content = match std::fs::read_to_string(DECOR_FILE) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Append 一筆展示記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_entry(entry: &DecorEntry) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(DECOR_FILE)
    {
        if let Ok(line) = serde_json::to_string(entry) {
            let _ = writeln!(f, "{line}");
        }
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_offset_deterministic_and_bounded() {
        for i in 0..10 {
            let (dx, dz) = slot_offset(i);
            assert_eq!((dx, dz), slot_offset(i), "純函式應確定性");
            assert!(dx.abs() <= 4 && dz.abs() <= 4, "位移應在合理小範圍內");
        }
    }

    #[test]
    fn slot_offset_wraps_safely_past_cap() {
        // 越界索引（超過 DECOR_SLOT_MAX）不 panic，安全取模。
        let _ = slot_offset(DECOR_SLOT_MAX);
        let _ = slot_offset(DECOR_SLOT_MAX * 100 + 7);
    }

    #[test]
    fn envy_triggers_requires_cooldown_and_chance() {
        assert!(!envy_triggers(false, 0.0), "冷卻未到不應觸發，就算機率骰中");
        assert!(envy_triggers(true, 0.0), "冷卻已到＋機率骰中應觸發");
        assert!(!envy_triggers(true, 0.99), "冷卻已到但機率未中不應觸發");
    }

    #[test]
    fn is_decor_item_matches_furniture() {
        for id in [vfurn::CARPET_ID, vfurn::FLOWERPOT_ID, vfurn::TABLE_ID, vfurn::BANNER_ID] {
            assert!(is_decor_item(id));
        }
        for id in [0u8, 5, 19, 45, 101] {
            assert!(!is_decor_item(id), "id={id} 不該被認成裝飾傢俱");
        }
    }

    #[test]
    fn add_piece_dedups_same_item() {
        let mut s = DecorStore::new();
        assert!(s.add_piece("露娜", 102).is_some());
        assert!(s.add_piece("露娜", 102).is_none(), "已展示過同款不應再累積");
        assert_eq!(s.displayed_for("露娜"), &[102]);
    }

    #[test]
    fn add_piece_caps_at_max_slots() {
        let mut s = DecorStore::new();
        assert!(s.add_piece("露娜", 102).is_some());
        assert!(s.add_piece("露娜", 103).is_some());
        assert!(s.add_piece("露娜", 104).is_some());
        assert_eq!(s.displayed_for("露娜").len(), DECOR_SLOT_MAX);
        // 第 4 款（不同 id、格位已滿）應被拒絕，不超過上限。
        assert!(s.add_piece("露娜", 105).is_none(), "展示格已滿應拒絕新增");
        assert_eq!(s.displayed_for("露娜").len(), DECOR_SLOT_MAX);
    }

    #[test]
    fn add_piece_independent_per_resident() {
        let mut s = DecorStore::new();
        s.add_piece("露娜", 102);
        assert!(!s.has_piece("諾娃", 102), "不同居民各自獨立累積");
        assert_eq!(s.displayed_for("諾娃"), &[] as &[u8]);
    }

    #[test]
    fn from_entries_replays_with_same_dedup_and_cap_rules() {
        let entries = vec![
            DecorEntry { resident: "露娜".into(), item_id: 102, seq: 0 },
            DecorEntry { resident: "露娜".into(), item_id: 102, seq: 1 }, // 重複，應被去重
            DecorEntry { resident: "露娜".into(), item_id: 103, seq: 2 },
            DecorEntry { resident: "露娜".into(), item_id: 104, seq: 3 },
            DecorEntry { resident: "露娜".into(), item_id: 105, seq: 4 }, // 超過上限，應被忽略
        ];
        let s = DecorStore::from_entries(entries);
        assert_eq!(s.displayed_for("露娜"), &[102, 103, 104]);
    }

    #[test]
    fn from_entries_next_seq_continues_after_max() {
        let entries = vec![
            DecorEntry { resident: "露娜".into(), item_id: 102, seq: 5 },
            DecorEntry { resident: "諾娃".into(), item_id: 103, seq: 2 },
        ];
        let mut s = DecorStore::from_entries(entries);
        let rec = s.add_piece("賽勒", 104).unwrap();
        assert_eq!(rec.seq, 6, "新記錄序號應接續在已存在的最大序號之後");
    }

    #[test]
    fn set_wanted_rejects_already_owned_piece() {
        let mut s = DecorStore::new();
        s.add_piece("露娜", 102);
        assert!(!s.set_wanted("露娜", 102), "已擁有的款式不該再嚮往");
        assert_eq!(s.wanted_for("露娜"), None);
    }

    #[test]
    fn set_wanted_no_change_if_same_as_current() {
        let mut s = DecorStore::new();
        assert!(s.set_wanted("露娜", 103));
        assert!(!s.set_wanted("露娜", 103), "重複設同一款不算新變化");
        assert_eq!(s.wanted_for("露娜"), Some(103));
    }

    #[test]
    fn set_wanted_can_switch_target() {
        let mut s = DecorStore::new();
        s.set_wanted("露娜", 103);
        assert!(s.set_wanted("露娜", 104), "換目標算新變化");
        assert_eq!(s.wanted_for("露娜"), Some(104));
    }

    #[test]
    fn clear_wanted_resets_to_none() {
        let mut s = DecorStore::new();
        s.set_wanted("露娜", 103);
        s.clear_wanted("露娜");
        assert_eq!(s.wanted_for("露娜"), None);
    }

    #[test]
    fn wanted_independent_per_resident() {
        let mut s = DecorStore::new();
        s.set_wanted("露娜", 103);
        assert_eq!(s.wanted_for("諾娃"), None);
    }

    #[test]
    fn display_thanks_line_contains_item_name_and_bounded() {
        for pick in 0..5 {
            let line = display_thanks_line("花盆", pick);
            assert!(line.contains("花盆"));
            assert!(line.chars().count() <= SAY_MAX_CHARS);
        }
    }

    #[test]
    fn wish_fulfilled_thanks_line_contains_item_name_and_bounded() {
        for pick in 0..5 {
            let line = wish_fulfilled_thanks_line("掛旗", pick);
            assert!(line.contains("掛旗"));
            assert!(line.chars().count() <= SAY_MAX_CHARS);
        }
    }

    #[test]
    fn wanted_bubble_contains_names_and_bounded() {
        for pick in 0..5 {
            let line = wanted_bubble("露娜", "小地毯", pick);
            assert!(line.contains("露娜"));
            assert!(line.contains("小地毯"));
            assert!(line.chars().count() <= SAY_MAX_CHARS);
        }
    }

    #[test]
    fn feed_lines_contain_names_and_item() {
        assert!(display_feed_line("露娜", "小圓桌").contains("露娜"));
        assert!(display_feed_line("露娜", "小圓桌").contains("小圓桌"));
        assert!(wish_fulfilled_feed_line("諾娃", "花盆").contains("諾娃"));
        assert!(wanted_feed_line("賽勒", "奧瑞", "掛旗").contains("賽勒"));
        assert!(wanted_feed_line("賽勒", "奧瑞", "掛旗").contains("奧瑞"));
        assert!(wanted_feed_line("賽勒", "奧瑞", "掛旗").contains("掛旗"));
    }

    #[test]
    fn memory_lines_contain_item_or_neighbor() {
        assert!(display_memory_line("小地毯").contains("小地毯"));
        assert!(wanted_memory_line("露娜", "花盆").contains("露娜"));
        assert!(wanted_memory_line("露娜", "花盆").contains("花盆"));
    }
}
