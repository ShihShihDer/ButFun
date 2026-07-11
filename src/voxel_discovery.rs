//! 乙太方界·探索紀事 v1（自主提案切片，接續 838 古代遺跡／839 溫泉遺跡）。
//!
//! **真缺口**：838/839 讓世界第一次有了值得走遠尋訪的地標——遺跡的乙太礦、溫泉的暖意，
//! 但找到之後除了那一拍的驚喜／回饋，**什麼都沒留下**：玩家找過幾處遺跡、發現過幾泓
//! 溫泉、它們在哪裡，全都只能憑自己的記憶或事先抄座標，找不到任何管道回頭翻閱。跟居民
//! 早就有的技能簿（719）／交情網（708）、玩家自己的里程碑牆（724）比起來，「地標探索」
//! 是這條療癒循環裡唯一沒有留下紀錄的一段。
//!
//! **與既有元素的定位區隔**：
//! - 里程碑（724）記的是「有沒有做過某件事」（一次性、無座標）；本模組記的是「在哪裡做到
//!   的」（可能多筆、每筆帶座標）——兩者互補，本切片同時也替遺跡／溫泉補上對應的里程碑
//!   （之前這兩個系統上線時漏補，見下方 `voxel_ws.rs` 接線）。
//! - 居民日記（`voxel_diary`）是居民內心視角，記居民自己的所見所感；本模組是**玩家視角**
//!   的探索足跡，記玩家親自找到的地標座標——一個寫「我想了什麼」，一個寫「我去過哪裡」。
//!
//! **去重設計**：遺跡的乙太礦每處恰有一塊、位置固定、挖掉即成空氣不會再生——破壞它本身
//! 就是天然不會重複的「發現一處新遺跡」信號。溫泉則不同：同一泓溫泉可以被反覆踏入踏出，
//! 若不去重，探索紀事會被同一處灌爆——因此以溫泉所屬的格子座標
//! （[`crate::voxel::hot_spring_cell_of`]）當穩定去重鍵，同一玩家對同一泓溫泉只記第一次。
//!
//! 純邏輯層：零 async、零鎖、零 IO 外包；鎖/IO/廣播/里程碑觸發全在 `voxel_ws.rs`。
//! **成本紀律**：零 LLM、零新協議破壞（新增 WS 訊息 additive）、零新美術。

use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

/// 持久化路徑（`data/` 已 gitignore）。
pub const DISCOVERY_PATH: &str = "data/voxel_discoveries.jsonl";

/// 每位玩家最多保留這麼多筆探索紀事（滿了先丟最舊的一筆，避免無限成長；長期探索者
/// 仍能穩定回顧近期路線，比照 `voxel_memory::EPISODIC_CAP` 有界成長慣例）。
pub const MAX_PER_PLAYER: usize = 200;

/// 地標種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LandmarkKind {
    /// 古代遺跡（838）——一次性挖礦驚喜。
    Ruin,
    /// 溫泉遺跡（839）——可重複造訪的功能性地標。
    HotSpring,
    /// 邊陲營地（881）——居民親手搭起、有床有立牌的荒野據點。與前兩者不同：座標非世界生成
    /// 種子決定，而是由該居民的家座標純函式算出（見 `voxel_ws.rs::player_near_built_outpost`）。
    Outpost,
    /// 野外殖民地（分村殖民 v1）——居民奠基的有名字聚落，遠行撞見的人為地標。
    Colony,
    /// 世界奇觀·乙太世界樹（ROADMAP 940）——全世界唯一一座天然大奇觀，座標由世界種子確定性
    /// 決定、獨一無二、遠離主村（`voxel::worldtree_base`）。與遺跡/溫泉的「格狀重複、四處散落」
    /// 不同，這是走到世界邊陲才撞見的單一壯麗天然地標；玩家走近樹腳即記一筆探索紀事。
    Wonder,
}

impl LandmarkKind {
    /// 面向玩家的繁中名稱（i18n 集中管理用）。
    pub fn label(self) -> &'static str {
        match self {
            LandmarkKind::Ruin => "古代遺跡",
            LandmarkKind::HotSpring => "溫泉",
            LandmarkKind::Outpost => "邊陲營地",
            LandmarkKind::Colony => "野外村落",
            LandmarkKind::Wonder => "乙太世界樹",
        }
    }

    /// wire 契約用的穩定字串鍵（前端/持久化皆用此鍵，不用 enum 的 derive 序列化名稱，
    /// 避免內部改名牽動外部契約）。
    pub fn wire_id(self) -> &'static str {
        match self {
            LandmarkKind::Ruin => "ruin",
            LandmarkKind::HotSpring => "hot_spring",
            LandmarkKind::Outpost => "outpost",
            LandmarkKind::Colony => "colony",
            LandmarkKind::Wonder => "wonder",
        }
    }

    /// emoji 圖示。
    pub fn icon(self) -> &'static str {
        match self {
            LandmarkKind::Ruin => "🏛️",
            LandmarkKind::HotSpring => "♨️",
            LandmarkKind::Outpost => "⛺",
            LandmarkKind::Colony => "🏘️",
            LandmarkKind::Wonder => "🌳",
        }
    }
}

/// 一筆探索紀事（持久化單位）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryEntry {
    pub player: String,
    pub kind: LandmarkKind,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 去重座標（遺跡＝乙太礦本身座標，等於 `(x, z)`；溫泉＝所屬格子，可能不同於
    /// `(x, z)`——同一泓溫泉不論從哪裡踏進去都換算出同一格）。與 `(x, y, z)` 分開持久化，
    /// 讓 replay（`insert_raw`）重建的去重鍵永遠與 `record()` 當下算出的一致，不必在
    /// replay 時重新猜測「這個座標屬於哪一格」。
    pub dedup_x: i32,
    pub dedup_z: i32,
    /// 單調遞增序號（供排序；replay 時延續最大值，不必依賴檔案行序）。
    pub seq: u64,
}

/// 全局探索紀事 store：玩家 → 依時間序的紀事清單（有界成長）。
#[derive(Default)]
pub struct DiscoveryStore {
    by_player: HashMap<String, Vec<DiscoveryEntry>>,
    /// 去重鍵集合：(玩家, 地標種類, 去重座標)。去重座標對遺跡是乙太礦本身座標
    /// （天然不重複，仍一併登記以統一去重路徑）；對溫泉是 `hot_spring_cell_of` 換算出的格子。
    seen: HashSet<(String, LandmarkKind, (i32, i32))>,
    next_seq: u64,
}

impl DiscoveryStore {
    /// 空 store（測試 / 首次啟動）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建 store（重啟後從 JSONL replay）。
    pub fn from_entries(entries: Vec<DiscoveryEntry>) -> Self {
        let mut store = Self::new();
        for e in entries {
            store.insert_raw(e);
        }
        store
    }

    /// 內部共用：把一筆（可能是 replay、也可能是新事件）記進記憶體結構，維持去重鍵與
    /// 每位玩家上限。不做 IO——呼叫端（`record`／`from_entries`）各自決定要不要落地。
    fn insert_raw(&mut self, entry: DiscoveryEntry) {
        self.next_seq = self.next_seq.max(entry.seq + 1);
        let key = (entry.player.clone(), entry.kind, (entry.dedup_x, entry.dedup_z));
        self.seen.insert(key);
        let list = self.by_player.entry(entry.player.clone()).or_default();
        list.push(entry);
        if list.len() > MAX_PER_PLAYER {
            list.remove(0); // 丟最舊一筆，保持有界成長。
        }
    }

    /// 嘗試記一筆新發現：`dedup_key` 是去重座標（遺跡＝乙太礦本身座標；溫泉＝所屬格子）。
    /// 這位玩家對同一 `(kind, dedup_key)` 若已記過，回 `None`（呼叫端不必落地/廣播，
    /// 避免溫泉被反覆進出灌爆紀事）；首次發現才回 `Some(新事件)`，呼叫端接著落地持久化。
    pub fn record(
        &mut self,
        player: &str,
        kind: LandmarkKind,
        dedup_key: (i32, i32),
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<DiscoveryEntry> {
        let key = (player.to_string(), kind, dedup_key);
        if self.seen.contains(&key) {
            return None;
        }
        let entry = DiscoveryEntry {
            player: player.to_string(),
            kind,
            x,
            y,
            z,
            dedup_x: dedup_key.0,
            dedup_z: dedup_key.1,
            seq: self.next_seq,
        };
        self.insert_raw(entry.clone());
        Some(entry)
    }

    /// 取這位玩家的探索紀事（依發現順序，最舊在前）。
    pub fn list_for(&self, player: &str) -> Vec<DiscoveryEntry> {
        self.by_player.get(player).cloned().unwrap_or_default()
    }

    /// 這位玩家分別發現過幾處遺跡／溫泉／邊陲營地（供面板頂部小計）。
    pub fn counts_for(&self, player: &str) -> (usize, usize, usize) {
        let list = self.by_player.get(player);
        let Some(list) = list else { return (0, 0, 0); };
        let ruins = list.iter().filter(|e| e.kind == LandmarkKind::Ruin).count();
        let springs = list.iter().filter(|e| e.kind == LandmarkKind::HotSpring).count();
        let outposts = list.iter().filter(|e| e.kind == LandmarkKind::Outpost).count();
        (ruins, springs, outposts)
    }

    /// 這位玩家發現過幾座野外殖民地（供面板頂部小計，與 [`counts_for`](Self::counts_for)
    /// 並列；殖民地是分村殖民 v1 新增的地標種類，另開一支計數以維持 `counts_for` 舊契約）。
    pub fn colony_count_for(&self, player: &str) -> usize {
        self.by_player
            .get(player)
            .map(|l| l.iter().filter(|e| e.kind == LandmarkKind::Colony).count())
            .unwrap_or(0)
    }
}

/// 載回所有探索紀事（伺服器啟動時呼叫一次）。檔不存在 / 壞行皆容忍（比照其餘 append-only
/// store 慣例：寧可少一筆歷史紀事，也不讓啟動因為單一壞行而失敗）。
pub fn load_discoveries() -> Vec<DiscoveryEntry> {
    let Ok(f) = fs::File::open(DISCOVERY_PATH) else { return vec![] };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<DiscoveryEntry>(&l).ok())
        .collect()
}

/// Append 一筆探索紀事到 jsonl。append-only、絕不覆寫/刪除既有行；失敗只記 log 不 panic。
pub fn append_discovery(entry: &DiscoveryEntry) {
    let Ok(line) = serde_json::to_string(entry) else { return };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(DISCOVERY_PATH) else {
        tracing::warn!("無法寫入探索紀事檔 {DISCOVERY_PATH}");
        return;
    };
    let _ = writeln!(f, "{line}");
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_first_discovery_returns_some_and_registers() {
        let mut s = DiscoveryStore::new();
        let e = s.record("阿光", LandmarkKind::Ruin, (10, 20), 10, 65, 20);
        assert!(e.is_some());
        assert_eq!(s.list_for("阿光").len(), 1);
        assert_eq!(s.counts_for("阿光"), (1, 0, 0));
    }

    #[test]
    fn record_same_dedup_key_twice_only_counts_once() {
        // 同一玩家同一泓溫泉（同一格子鍵）反覆踏入，第二次起應回 None、不再累加。
        let mut s = DiscoveryStore::new();
        assert!(s.record("阿光", LandmarkKind::HotSpring, (1, 1), 56, 65, 56).is_some());
        assert!(s.record("阿光", LandmarkKind::HotSpring, (1, 1), 58, 65, 58).is_none());
        assert_eq!(s.list_for("阿光").len(), 1, "同一格子鍵重複踏入不該累加紀事");
    }

    #[test]
    fn different_players_are_independent() {
        let mut s = DiscoveryStore::new();
        s.record("阿光", LandmarkKind::Ruin, (10, 20), 10, 65, 20);
        s.record("阿星", LandmarkKind::Ruin, (10, 20), 10, 65, 20);
        assert_eq!(s.list_for("阿光").len(), 1);
        assert_eq!(s.list_for("阿星").len(), 1, "不同玩家的去重鍵應互不影響");
    }

    #[test]
    fn different_kinds_at_same_coord_both_count() {
        // 同座標但種類不同（理論上不會真的重疊，但去重鍵含 kind，邏輯上該各自獨立）。
        let mut s = DiscoveryStore::new();
        s.record("阿光", LandmarkKind::Ruin, (5, 5), 5, 65, 5);
        s.record("阿光", LandmarkKind::HotSpring, (5, 5), 5, 65, 5);
        s.record("阿光", LandmarkKind::Outpost, (5, 5), 5, 65, 5);
        assert_eq!(s.list_for("阿光").len(), 3);
        assert_eq!(s.counts_for("阿光"), (1, 1, 1));
    }

    #[test]
    fn cap_drops_oldest_when_exceeded() {
        let mut s = DiscoveryStore::new();
        for i in 0..(MAX_PER_PLAYER as i32 + 5) {
            s.record("阿光", LandmarkKind::Ruin, (i, i), i, 65, i);
        }
        let list = s.list_for("阿光");
        assert_eq!(list.len(), MAX_PER_PLAYER, "超過上限應丟最舊一筆維持上限");
        assert_eq!(list[0].x, 5, "最舊的 5 筆應已被丟棄，留下的第一筆該是 x=5");
    }

    #[test]
    fn from_entries_rebuilds_dedup_and_seq() {
        let entries = vec![
            DiscoveryEntry { player: "阿光".into(), kind: LandmarkKind::Ruin, x: 1, y: 65, z: 1, dedup_x: 1, dedup_z: 1, seq: 0 },
            DiscoveryEntry { player: "阿光".into(), kind: LandmarkKind::HotSpring, x: 2, y: 65, z: 2, dedup_x: 2, dedup_z: 2, seq: 1 },
        ];
        let mut s = DiscoveryStore::from_entries(entries);
        assert_eq!(s.list_for("阿光").len(), 2);
        // replay 後同去重鍵仍該被擋下（重啟不會讓已發現的地標重新累加）。
        assert!(s.record("阿光", LandmarkKind::Ruin, (1, 1), 1, 65, 1).is_none());
        // 新的 seq 應接續在 replay 過的最大值之後（新事件 seq 不撞號）。
        let fresh = s.record("阿光", LandmarkKind::Ruin, (99, 99), 99, 65, 99).unwrap();
        assert!(fresh.seq >= 2);
    }

    #[test]
    fn from_entries_dedup_uses_persisted_dedup_key_not_raw_coord() {
        // 回歸測試：溫泉的去重鍵（格子座標）與顯示座標（實際踏入點）可能不同——
        // replay 後若誤用 (x,z) 當去重鍵重建，會讓同一泓溫泉在重啟後被當成新地標。
        // 這裡持久化的 dedup_x/dedup_z 是格子 (1,1)，但顯示座標 x/z 是格子內某個實際點。
        let entries = vec![DiscoveryEntry {
            player: "阿光".into(),
            kind: LandmarkKind::HotSpring,
            x: 56,
            y: 65,
            z: 56,
            dedup_x: 1,
            dedup_z: 1,
            seq: 0,
        }];
        let mut s = DiscoveryStore::from_entries(entries);
        // 同一格子、不同的實際踏入座標（58,58）——去重鍵一樣是 (1,1)，該被擋下。
        assert!(
            s.record("阿光", LandmarkKind::HotSpring, (1, 1), 58, 65, 58).is_none(),
            "replay 後同一泓溫泉（同去重格）不該被重新計入"
        );
        assert_eq!(s.list_for("阿光").len(), 1);
    }

    #[test]
    fn empty_player_has_no_entries() {
        let s = DiscoveryStore::new();
        assert_eq!(s.list_for("沒人"), vec![]);
        assert_eq!(s.counts_for("沒人"), (0, 0, 0));
    }

    #[test]
    fn label_icon_wire_id_are_distinct_per_kind() {
        let kinds = [
            LandmarkKind::Ruin,
            LandmarkKind::HotSpring,
            LandmarkKind::Outpost,
            LandmarkKind::Colony,
            LandmarkKind::Wonder,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for b in &kinds[i + 1..] {
                assert_ne!(a.label(), b.label());
                assert_ne!(a.wire_id(), b.wire_id());
                // icon 不強制全異（Wonder🌳 與 Ruin🏛️ 等各異；此處只驗 label/wire 唯一即足夠契約需求）。
            }
        }
    }

    #[test]
    fn wonder_landmark_records_and_dedups() {
        // 世界奇觀走與其他地標同一套 record 去重路徑：唯一那株用固定去重鍵，同一玩家只記第一次。
        let mut s = DiscoveryStore::new();
        assert!(s.record("阿光", LandmarkKind::Wonder, (7, 7), 7, 40, 7).is_some());
        assert!(
            s.record("阿光", LandmarkKind::Wonder, (7, 7), 7, 40, 7).is_none(),
            "同一座奇觀重複抵達不該累加"
        );
        assert_eq!(s.list_for("阿光").len(), 1);
    }
}
