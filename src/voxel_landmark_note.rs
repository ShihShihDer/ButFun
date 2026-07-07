//! 乙太方界·地標旅人留言 v1（自主提案切片，接續 838/839 古代遺跡／溫泉遺跡、840 探索紀事）。
//!
//! **真缺口**：840 探索紀事讓玩家自己回頭翻閱「我去過哪裡」，但那是**私人**的紀錄——
//! 你找到的遺跡/溫泉，其他玩家找到同一處時完全看不到你留下的任何痕跡，世界對第一位
//! 發現者跟第一百位發現者一視同仁、毫無累積感。跟居民早就有的鄰里認家網（749~763）、
//! 玩家自己的告示牌（740）比起來，「地標」是唯一一種**多位玩家共同造訪、卻彼此看不見
//! 對方存在**的地點——探索永遠是孤獨的一人紀事，從沒有變成旅人之間的共同記憶。
//!
//! **做法**：在 838/839 既有的「第一次發現這個地標」那一刻（`voxel_discovery::record`
//! 回 `Some`），順手看看先前造訪過的旅人留了什麼話（若有），再讓這位玩家選擇要不要也
//! 留一句給後來的人——像每處地標旁掛著一本共同的旅人留言簿，一頁頁疊上去。
//!
//! **與 840 探索紀事的定位區隔**：探索紀事是**玩家視角**（我去過哪裡，私人、按時間序）；
//! 本模組是**地標視角**（這裡有誰來過、說了什麼，公開、按地標分組）——一個記「我」，
//! 一個記「這裡」，互補而不重複。
//!
//! **去重/有界設計**：同一玩家在同一地標留言，視為「改寫」既有留言（取代舊文字），不會
//! 每次都新增一筆——避免同一人反覆留言把留言簿灌爆；每處地標另設硬上限
//! [`MAX_NOTES_PER_LANDMARK`]，滿了才擠掉最舊的一筆（多半不會觸頂，因為每人只佔一筆）。
//!
//! 純邏輯層：零 async、零鎖、零 IO 外包；鎖/IO/廣播/里程碑觸發全在 `voxel_ws.rs`。
//! **成本紀律**：零 LLM、零新協議破壞（新增 WS 訊息 additive）、零新美術。

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

use crate::voxel_discovery::LandmarkKind;

/// 持久化路徑（`data/` 已 gitignore）。
pub const NOTE_PATH: &str = "data/voxel_landmark_notes.jsonl";

/// 每處地標最多保留這麼多筆留言（滿了先丟最舊的一筆，避免無限成長）。同一玩家的留言
/// 會原地改寫而非新增，實務上只有極熱門地標吸引夠多不同旅人時才會真的觸頂。
pub const MAX_NOTES_PER_LANDMARK: usize = 20;

/// 一筆旅人留言（持久化單位）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TravelerNote {
    pub player: String,
    pub kind: LandmarkKind,
    /// 去重座標：與 [`crate::voxel_discovery::DiscoveryEntry`] 同一套去重鍵（遺跡＝乙太礦
    /// 本身座標；溫泉＝所屬格子），讓同一處地標的留言穩定聚在同一組，不受踏入點微差影響。
    pub dedup_x: i32,
    pub dedup_z: i32,
    /// 已清洗過的留言文字（呼叫端負責清洗，本模組不重複清洗）。
    pub text: String,
    /// 單調遞增序號（供排序 / replay 時延續最大值）。
    pub seq: u64,
}

/// 全局地標留言簿：(種類, 去重座標) → 依留言順序的清單（有界成長）。
#[derive(Default)]
pub struct LandmarkNoteStore {
    by_landmark: HashMap<(LandmarkKind, i32, i32), Vec<TravelerNote>>,
    next_seq: u64,
}

impl LandmarkNoteStore {
    /// 空 store（測試 / 首次啟動）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建 store（重啟後從 JSONL replay）。
    pub fn from_entries(entries: Vec<TravelerNote>) -> Self {
        let mut store = Self::new();
        for e in entries {
            store.insert_raw(e);
        }
        store
    }

    /// 內部共用：把一筆（可能是 replay、也可能是新留言）記進記憶體結構——同一玩家在同一
    /// 地標的舊留言原地改寫（取代文字＋序號），不同玩家各自佔一筆；超過上限丟最舊一筆。
    /// 不做 IO——呼叫端（`leave`／`from_entries`）各自決定要不要落地。
    fn insert_raw(&mut self, entry: TravelerNote) {
        self.next_seq = self.next_seq.max(entry.seq + 1);
        let key = (entry.kind, entry.dedup_x, entry.dedup_z);
        let list = self.by_landmark.entry(key).or_default();
        if let Some(existing) = list.iter_mut().find(|n| n.player == entry.player) {
            *existing = entry;
        } else {
            list.push(entry);
            if list.len() > MAX_NOTES_PER_LANDMARK {
                list.remove(0); // 丟最舊一筆，保持有界成長。
            }
        }
    }

    /// 嘗試留一句話：`clean_text` 須是呼叫端已清洗過的非空文字（空字串一律回 `None`，
    /// 呼叫端不必落地/廣播）。同一玩家在同一地標重複呼叫會改寫既有留言。
    pub fn leave(
        &mut self,
        kind: LandmarkKind,
        dedup_key: (i32, i32),
        player: &str,
        clean_text: &str,
    ) -> Option<TravelerNote> {
        if clean_text.is_empty() {
            return None;
        }
        let entry = TravelerNote {
            player: player.to_string(),
            kind,
            dedup_x: dedup_key.0,
            dedup_z: dedup_key.1,
            text: clean_text.to_string(),
            seq: self.next_seq,
        };
        self.insert_raw(entry.clone());
        Some(entry)
    }

    /// 取這處地標目前的留言，最新的排在最前面（供「先前旅人留言」面板顯示）。
    pub fn notes_at(&self, kind: LandmarkKind, dedup_key: (i32, i32)) -> Vec<TravelerNote> {
        let key = (kind, dedup_key.0, dedup_key.1);
        let mut list = self.by_landmark.get(&key).cloned().unwrap_or_default();
        list.reverse();
        list
    }
}

/// 載回所有地標留言（伺服器啟動時呼叫一次）。檔不存在 / 壞行皆容忍（比照其餘 append-only
/// store 慣例：寧可少一筆歷史留言，也不讓啟動因為單一壞行而失敗）。
pub fn load_notes() -> Vec<TravelerNote> {
    let Ok(f) = fs::File::open(NOTE_PATH) else { return vec![] };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<TravelerNote>(&l).ok())
        .collect()
}

/// Append 一筆地標留言到 jsonl。append-only、絕不覆寫/刪除既有行；失敗只記 log 不 panic。
/// 同玩家改寫留言時會 append 新的一行——重啟 replay 時 `insert_raw` 的改寫語意會自動只留
/// 最後一筆生效，不需要在檔案層面做覆寫。
pub fn append_note(entry: &TravelerNote) {
    let Ok(line) = serde_json::to_string(entry) else { return };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(NOTE_PATH) else {
        tracing::warn!("無法寫入地標留言檔 {NOTE_PATH}");
        return;
    };
    let _ = writeln!(f, "{line}");
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leave_first_note_returns_some_and_stored() {
        let mut s = LandmarkNoteStore::new();
        let e = s.leave(LandmarkKind::Ruin, (10, 20), "阿光", "路過留個名");
        assert!(e.is_some());
        let notes = s.notes_at(LandmarkKind::Ruin, (10, 20));
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].text, "路過留個名");
    }

    #[test]
    fn leave_empty_text_returns_none() {
        let mut s = LandmarkNoteStore::new();
        assert!(s.leave(LandmarkKind::Ruin, (1, 1), "阿光", "").is_none());
        assert!(s.notes_at(LandmarkKind::Ruin, (1, 1)).is_empty());
    }

    #[test]
    fn same_player_leaving_again_replaces_not_appends() {
        let mut s = LandmarkNoteStore::new();
        s.leave(LandmarkKind::HotSpring, (2, 2), "阿光", "第一句");
        s.leave(LandmarkKind::HotSpring, (2, 2), "阿光", "改成第二句");
        let notes = s.notes_at(LandmarkKind::HotSpring, (2, 2));
        assert_eq!(notes.len(), 1, "同一玩家重複留言不該累加");
        assert_eq!(notes[0].text, "改成第二句");
    }

    #[test]
    fn different_players_each_get_own_entry_at_same_landmark() {
        let mut s = LandmarkNoteStore::new();
        s.leave(LandmarkKind::Ruin, (5, 5), "阿光", "光留言");
        s.leave(LandmarkKind::Ruin, (5, 5), "阿星", "星留言");
        assert_eq!(s.notes_at(LandmarkKind::Ruin, (5, 5)).len(), 2);
    }

    #[test]
    fn cap_drops_oldest_when_many_distinct_players_exceed_cap() {
        let mut s = LandmarkNoteStore::new();
        for i in 0..(MAX_NOTES_PER_LANDMARK + 5) {
            s.leave(LandmarkKind::Ruin, (0, 0), &format!("旅人{i}"), "哈囉");
        }
        let notes = s.notes_at(LandmarkKind::Ruin, (0, 0));
        assert_eq!(notes.len(), MAX_NOTES_PER_LANDMARK, "超過上限應丟最舊一筆維持上限");
    }

    #[test]
    fn different_kinds_at_same_coord_are_independent() {
        let mut s = LandmarkNoteStore::new();
        s.leave(LandmarkKind::Ruin, (3, 3), "阿光", "這是遺跡");
        s.leave(LandmarkKind::HotSpring, (3, 3), "阿光", "這是溫泉");
        assert_eq!(s.notes_at(LandmarkKind::Ruin, (3, 3)).len(), 1);
        assert_eq!(s.notes_at(LandmarkKind::HotSpring, (3, 3)).len(), 1);
    }

    #[test]
    fn notes_at_returns_newest_first() {
        let mut s = LandmarkNoteStore::new();
        s.leave(LandmarkKind::Ruin, (0, 0), "阿光", "第一位");
        s.leave(LandmarkKind::Ruin, (0, 0), "阿星", "第二位");
        let notes = s.notes_at(LandmarkKind::Ruin, (0, 0));
        assert_eq!(notes[0].player, "阿星", "最新留言應排最前面");
        assert_eq!(notes[1].player, "阿光");
    }

    #[test]
    fn notes_at_empty_landmark_returns_empty() {
        let s = LandmarkNoteStore::new();
        assert!(s.notes_at(LandmarkKind::HotSpring, (99, 99)).is_empty());
    }

    #[test]
    fn from_entries_replays_and_continues_seq() {
        let entries = vec![
            TravelerNote { player: "阿光".into(), kind: LandmarkKind::Ruin, dedup_x: 1, dedup_z: 1, text: "留言A".into(), seq: 0 },
            TravelerNote { player: "阿星".into(), kind: LandmarkKind::Ruin, dedup_x: 1, dedup_z: 1, text: "留言B".into(), seq: 1 },
        ];
        let mut s = LandmarkNoteStore::from_entries(entries);
        assert_eq!(s.notes_at(LandmarkKind::Ruin, (1, 1)).len(), 2);
        let fresh = s.leave(LandmarkKind::Ruin, (1, 1), "阿新", "留言C").unwrap();
        assert!(fresh.seq >= 2, "新留言序號不該撞上 replay 過的舊序號");
    }

    #[test]
    fn from_entries_preserves_replace_semantics() {
        // append-only 檔案裡同玩家改寫留言會留下兩行；replay 後只該生效最後一筆。
        let entries = vec![
            TravelerNote { player: "阿光".into(), kind: LandmarkKind::HotSpring, dedup_x: 2, dedup_z: 2, text: "舊留言".into(), seq: 0 },
            TravelerNote { player: "阿光".into(), kind: LandmarkKind::HotSpring, dedup_x: 2, dedup_z: 2, text: "新留言".into(), seq: 1 },
        ];
        let s = LandmarkNoteStore::from_entries(entries);
        let notes = s.notes_at(LandmarkKind::HotSpring, (2, 2));
        assert_eq!(notes.len(), 1, "同玩家的改寫在 replay 後不該變成兩筆");
        assert_eq!(notes[0].text, "新留言");
    }
}
