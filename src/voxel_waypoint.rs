//! 乙太方界·個人路標 v1（自主提案切片，接續 PLAN_ETHERVOX「玩家遊玩」並行軸線）。
//!
//! **真缺口**：820 雷達小地圖讓玩家一眼看出「居民散在哪一側」，705 羅盤能導向居民，但世界
//! 完全沒有一種「玩家自己標記的地點」——你深挖出的礦坑入口、看中的蓋房地基、走遠找到的
//! 一處風景，走開幾步就再也找不回去，只能憑記憶亂走或土法煉鋼放告示牌（740，但告示牌要
//! 站在現場才看得到文字，不會出現在羅盤/雷達上引路）。世界很大（散居村莊+程序生成地形），
//! 玩家自己的「這裡很重要」卻無法被記住、無法被導航——这是「操作手感」這條並行軸線裡一個
//! 純粹的玩家向缺口，不是又一種「居民對你做的反應」。
//!
//! **做法**：玩家在目前所站的位置插一支路標、取個短名字（如「礦坑」「我家」），之後在既有
//! 羅盤／雷達面板（705/820）裡，自己的路標會跟居民座標並列顯示方向與距離——世界第一次有
//! 「玩家自己選的地標」可以被導航回去。同一個名字重插＝原地改寫座標（比照 830 告示牌
//! 認養／862 地標留言同一玩家改寫既有留言的慣例）；每位玩家的路標數量有硬上限，插滿了
//! 要先刪一個才能插新的（[`MAX_WAYPOINTS_PER_PLAYER`]），避免無限洗版。
//!
//! **與既有系統的分界（換維度·非同軸重複）**：
//! - 705 羅盤／820 雷達：導向的是**居民**（AI 角色即時座標）；本模組導向的是**玩家自己
//!   選定的地點**，兩者資料來源不同、在同一面板並列顯示互補。
//! - 740 告示牌：**放在世界裡的實體方塊**，文字要走到牌子前才看得到，任何人都看得到、
//!   會被居民讀（741）；本模組是**純資料、無方塊**，只有插這支路標的玩家自己看得到，
//!   不進世界 delta、不會被居民讀取或談論——是「我自己的筆記」，不是給別人看的告示。
//! - 862 地標旅人留言：只認**已知地標種類**（遺跡／溫泉）這種固定地點；本模組是玩家**任意
//!   座標**都能標記，不限定在特定地形上。
//!
//! 純邏輯層：零 async、零鎖、零 IO 外包；鎖/IO/廣播全在 `voxel_ws.rs`。
//! **成本紀律**：零 LLM、零協議破壞（新增 WS 訊息 additive）、零新美術（前端純色圖示）。

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

/// 持久化路徑（`data/` 已 gitignore）。
pub const WAYPOINT_PATH: &str = "data/voxel_waypoints.jsonl";

/// 每位玩家最多同時保留這麼多支路標，滿了要先刪一支才能插新的（防洗版/防無限成長）。
pub const MAX_WAYPOINTS_PER_PLAYER: usize = 8;

/// 路標名稱上限（字元數，非 byte）——一個短標籤，過長截斷。
pub const LABEL_MAX_CHARS: usize = 12;

/// 清洗玩家輸入的路標名稱：去頭尾空白、控制字元（含換行/tab）換成空白、
/// 截到 [`LABEL_MAX_CHARS`] 字元、再去一次頭尾空白。確定性、無副作用、可測。
/// 回傳空字串代表輸入不合法（呼叫端應拒絕、不插旗）。
pub fn sanitize_label(raw: &str) -> String {
    let cleaned: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    cleaned.trim().chars().take(LABEL_MAX_CHARS).collect::<String>().trim().to_string()
}

/// 一筆路標寫入事件（append-only JSONL 最小單元）。`removed=true` 代表這是一次刪除
/// （tombstone，座標無意義，replay 時只用來把該玩家該名字的路標從記憶體清掉）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WaypointEntry {
    pub player: String,
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 單調遞增序號（replay 時延續最大值）。
    pub seq: u64,
    #[serde(default)]
    pub removed: bool,
}

/// 插旗失敗原因（純函式回傳，呼叫端決定怎麼回覆玩家）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetErr {
    /// 名稱清洗後是空字串。
    EmptyLabel,
    /// 這個名字是新的（非改寫既有），但這位玩家已插滿上限。
    TooMany,
}

/// 全局個人路標 store：player → 這位玩家目前所有路標（有界成長）。
#[derive(Default)]
pub struct WaypointStore {
    by_player: HashMap<String, Vec<WaypointEntry>>,
    next_seq: u64,
}

impl WaypointStore {
    /// 空 store（測試 / 首次啟動）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建 store（重啟後從 JSONL replay）。忠實重放歷史事件本身，
    /// 不在 replay 時套用 [`MAX_WAYPOINTS_PER_PLAYER`] 上限檢查——上限只在活著呼叫
    /// [`Self::set`] 當下把關，寫進檔案的歷史一律信任。
    pub fn from_entries(entries: Vec<WaypointEntry>) -> Self {
        let mut store = Self::new();
        for e in entries {
            store.next_seq = store.next_seq.max(e.seq + 1);
            let list = store.by_player.entry(e.player.clone()).or_default();
            if e.removed {
                list.retain(|w| w.label != e.label);
            } else if let Some(existing) = list.iter_mut().find(|w| w.label == e.label) {
                *existing = e;
            } else {
                list.push(e);
            }
        }
        store
    }

    /// 插旗／改寫既有同名路標。座標由呼叫端提供（`voxel_ws.rs` 一律傳伺服器算出的玩家
    /// 目前座標，不信任客戶端自報位置）。同名重插＝原地改寫，不佔用新的一格上限。
    pub fn set(&mut self, player: &str, label: &str, x: i32, y: i32, z: i32) -> Result<WaypointEntry, SetErr> {
        if label.is_empty() {
            return Err(SetErr::EmptyLabel);
        }
        let list = self.by_player.entry(player.to_string()).or_default();
        let is_new = !list.iter().any(|w| w.label == label);
        if is_new && list.len() >= MAX_WAYPOINTS_PER_PLAYER {
            return Err(SetErr::TooMany);
        }
        let entry = WaypointEntry {
            player: player.to_string(),
            label: label.to_string(),
            x,
            y,
            z,
            seq: self.next_seq,
            removed: false,
        };
        self.next_seq += 1;
        if let Some(existing) = list.iter_mut().find(|w| w.label == label) {
            *existing = entry.clone();
        } else {
            list.push(entry.clone());
        }
        Ok(entry)
    }

    /// 刪除這位玩家指定名字的路標。找不到回 `None`（呼叫端不落地/不回覆成功）；
    /// 找到則從記憶體移除並回傳一份 tombstone（`removed=true`）供呼叫端 append 落地。
    pub fn remove(&mut self, player: &str, label: &str) -> Option<WaypointEntry> {
        let list = self.by_player.get_mut(player)?;
        let idx = list.iter().position(|w| w.label == label)?;
        let removed = list.remove(idx);
        let tombstone = WaypointEntry {
            seq: self.next_seq,
            removed: true,
            ..removed
        };
        self.next_seq += 1;
        Some(tombstone)
    }

    /// 這位玩家目前所有路標，依插旗先後排序（穩定順序，供面板顯示）。
    pub fn list(&self, player: &str) -> Vec<WaypointEntry> {
        let mut items = self.by_player.get(player).cloned().unwrap_or_default();
        items.sort_by_key(|w| w.seq);
        items
    }
}

/// 啟動時載回歷史路標事件（含刪除 tombstone），供 [`WaypointStore::from_entries`] 重建現況。
pub fn load_entries() -> Vec<WaypointEntry> {
    let Ok(f) = fs::File::open(WAYPOINT_PATH) else { return Vec::new(); };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

/// append 一筆事件（成功插旗或刪除都要落地，重啟後才記得）。
pub fn append_entry(entry: &WaypointEntry) {
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(WAYPOINT_PATH) else { return; };
    if let Ok(line) = serde_json::to_string(entry) {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_trims_and_strips_control() {
        assert_eq!(sanitize_label("  礦坑\n入口  "), "礦坑 入口");
    }

    #[test]
    fn sanitize_caps_length() {
        let raw = "一".repeat(30);
        assert_eq!(sanitize_label(&raw).chars().count(), LABEL_MAX_CHARS);
    }

    #[test]
    fn sanitize_empty_stays_empty() {
        assert_eq!(sanitize_label("   \n\t "), "");
    }

    #[test]
    fn set_rejects_empty_label() {
        let mut store = WaypointStore::new();
        assert_eq!(store.set("露娜", "", 1, 2, 3), Err(SetErr::EmptyLabel));
    }

    #[test]
    fn set_new_waypoint_succeeds_and_lists() {
        let mut store = WaypointStore::new();
        let entry = store.set("玩家A", "礦坑", 10, 20, 30).unwrap();
        assert_eq!(entry.x, 10);
        assert_eq!(store.list("玩家A"), vec![entry]);
    }

    #[test]
    fn set_same_label_overwrites_without_counting_twice() {
        let mut store = WaypointStore::new();
        store.set("玩家A", "礦坑", 1, 1, 1).unwrap();
        let updated = store.set("玩家A", "礦坑", 9, 9, 9).unwrap();
        let list = store.list("玩家A");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].x, updated.x);
        assert_eq!(updated.x, 9);
    }

    #[test]
    fn set_rejects_new_distinct_label_over_cap() {
        let mut store = WaypointStore::new();
        for i in 0..MAX_WAYPOINTS_PER_PLAYER {
            store.set("玩家A", &format!("點{i}"), 0, 0, 0).unwrap();
        }
        assert_eq!(store.set("玩家A", "多一個", 0, 0, 0), Err(SetErr::TooMany));
        // 但改寫既有名字（非新增）仍允許，即使已在上限。
        assert!(store.set("玩家A", "點0", 5, 5, 5).is_ok());
    }

    #[test]
    fn remove_existing_returns_tombstone_and_clears_it() {
        let mut store = WaypointStore::new();
        store.set("玩家A", "礦坑", 1, 2, 3).unwrap();
        let tomb = store.remove("玩家A", "礦坑").unwrap();
        assert!(tomb.removed);
        assert!(store.list("玩家A").is_empty());
    }

    #[test]
    fn remove_twice_second_time_returns_none() {
        let mut store = WaypointStore::new();
        store.set("玩家A", "礦坑", 1, 2, 3).unwrap();
        assert!(store.remove("玩家A", "礦坑").is_some());
        assert!(store.remove("玩家A", "礦坑").is_none());
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mut store = WaypointStore::new();
        assert!(store.remove("玩家A", "沒這個").is_none());
    }

    #[test]
    fn list_scoped_per_player() {
        let mut store = WaypointStore::new();
        store.set("玩家A", "家", 1, 1, 1).unwrap();
        store.set("玩家B", "家", 2, 2, 2).unwrap();
        assert_eq!(store.list("玩家A").len(), 1);
        assert_eq!(store.list("玩家B").len(), 1);
        assert_eq!(store.list("玩家A")[0].x, 1);
    }

    #[test]
    fn from_entries_replays_set_then_remove_ends_empty() {
        let entries = vec![
            WaypointEntry { player: "玩家A".into(), label: "礦坑".into(), x: 1, y: 2, z: 3, seq: 0, removed: false },
            WaypointEntry { player: "玩家A".into(), label: "礦坑".into(), x: 0, y: 0, z: 0, seq: 1, removed: true },
        ];
        let store = WaypointStore::from_entries(entries);
        assert!(store.list("玩家A").is_empty());
    }

    #[test]
    fn from_entries_preserves_latest_overwrite_and_continues_seq() {
        let entries = vec![
            WaypointEntry { player: "玩家A".into(), label: "家".into(), x: 1, y: 1, z: 1, seq: 0, removed: false },
            WaypointEntry { player: "玩家A".into(), label: "家".into(), x: 9, y: 9, z: 9, seq: 1, removed: false },
        ];
        let mut store = WaypointStore::from_entries(entries);
        let list = store.list("玩家A");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].x, 9);
        // seq 接續：下一次 set 應該用 seq=2，不與歷史撞號。
        let next = store.set("玩家A", "新的", 0, 0, 0).unwrap();
        assert_eq!(next.seq, 2);
    }
}
