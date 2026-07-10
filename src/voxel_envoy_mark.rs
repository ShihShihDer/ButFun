//! 乙太方界·引夢使者印記深化 v1（自主提案切片）。
//!
//! **真缺口**：這片天地的居民早就會對玩家生出交情／印象（`voxel_bonds`／`affinity_count`），
//! 也有零星的「你為我做過的一件事」被記住——守夜恩人（#888，你替她驅散暗影）、收到野花／
//! 禮物時的道謝。但那些好事各自散落在不同系統，居民從沒把「這位引夢使者為我、為這村
//! 累積做過的具體好事」攢成一份自己的記憶、時不時回想感念。玩家＝這世界的「引夢使者」
//! （點火者），最該有的獨特體驗，正是「我真的在這世界留下了印記，而且它被記住了」。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - 不是 `affinity_count`（累計的 episodic 記憶筆數＝泛泛好感），本模組專記**引夢使者**
//!   （帶稱號的維護者）做過的**具體善舉種類與次數**，是「你的功績」這一層，不是「好感值」。
//! - 不是守夜恩人（#888）的當下道謝——那是事發那一刻的即時反應；本模組把那些善舉**攢起來**，
//!   讓居民在**日後平常相處**時偶爾回想、提起（「還記得那晚你替我趕走暗影…」）。
//!
//! **純邏輯層**：善舉分類（[`DeedKind`]）、累積帳本（[`EnvoyMarkStore`]）、回想台詞
//! （[`recall_line`]）、動態牆句（[`feed_line`]）、持久化 IO（[`load_marks`]／[`save_marks`]）
//! 全是確定性純函式／同步資料結構，零 LLM、零鎖、零 async。鎖／位置判定／實際廣播全在
//! `voxel_ws.rs`，沿用既有「短鎖即釋、不巢狀」慣例，於低頻（15 秒）tick 節拍檢查。
//!
//! **成本／濫用防護**：全由**伺服器權威事件**驅動（守夜驅暗影／送禮／送花皆後端判定），
//! 玩家無法自報或催發；只有帶稱號的引夢使者觸發（訪客／一般玩家零負擔）；台詞全為固定
//! 模板、永不回放玩家原話（無注入／NSFW 風險）；每位居民回想有全域冷卻（呼叫端把關）。
//! 持久化採 append-only 快照（向後相容、讀最後快照遮蓋舊行），零 migration、零協議破壞、
//! FPS 零影響（純後端、低頻 tick）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 引夢使者為居民／村子做過的善舉種類。刻意收斂在少數幾種**由伺服器權威事件驅動、
/// 玩家無法自報**的具體好事上（v1 三種）；日後要加「教技能／幫整地／圓夢見證」等，
/// 只要在此擴一個變體＋一句 clause 即可，帳本／回想管線不用動。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DeedKind {
    /// 守夜恩人（#888）：夜裡替居民驅散逼近的暗影。
    SaveShadow,
    /// 送禮：親手把一份材料／珍寶送到居民手上（野花除外，另計 Flower）。
    Gift,
    /// 送花：摘一束野花相贈（純粹示好的心意）。
    Flower,
}

impl DeedKind {
    /// 穩定的持久化 / 協議編號（**只增不改**，向後相容）。
    pub fn to_wire(self) -> u8 {
        match self {
            DeedKind::SaveShadow => 1,
            DeedKind::Gift => 2,
            DeedKind::Flower => 3,
        }
    }

    /// 由編號還原；未知編號回 None（讀到未來版本寫的新種類時安全略過，不 panic）。
    pub fn from_wire(w: u8) -> Option<DeedKind> {
        match w {
            1 => Some(DeedKind::SaveShadow),
            2 => Some(DeedKind::Gift),
            3 => Some(DeedKind::Flower),
            _ => None,
        }
    }

    /// 面板用的短標籤（繁中，i18n 集中於此）。
    pub fn label(self) -> &'static str {
        match self {
            DeedKind::SaveShadow => "驅散暗影",
            DeedKind::Gift => "餽贈禮物",
            DeedKind::Flower => "餽贈野花",
        }
    }

    /// 回想台詞裡「那件事」的子句（插進 [`recall_line`] 的 `{d}`）。
    fn recall_clause(self) -> &'static str {
        match self {
            DeedKind::SaveShadow => "那晚你替我趕走了逼近的暗影",
            DeedKind::Gift => "你特地送到我手上的那份心意",
            DeedKind::Flower => "你摘來送我的那束野花",
        }
    }
}

/// 一位居民名下、對某一種引夢使者善舉的累積記憶片段（持久化記錄）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeedRecord {
    /// 居民 id（如 `vox_res_0`）。
    pub resident_id: String,
    /// 居民顯示名（快照，供面板／動態牆免再查）。
    pub resident_name: String,
    /// 善舉種類（[`DeedKind::to_wire`]）。
    pub kind: u8,
    /// 這位居民收到這種善舉的累積次數。
    pub count: u32,
    /// 最後一次發生的序號（單調遞增；用來挑「最近一件」回想）。
    pub last_seq: u64,
}

/// 引夢使者印記帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
/// key＝`(resident_id, kind_wire)`，每格攢一種善舉的次數與最後發生序號。
/// 一律以 [`EnvoyMarkStore::from_entries`] 建構（空 Vec＝全新帳本，`next_seq` 從 1 起）。
pub struct EnvoyMarkStore {
    marks: HashMap<(String, u8), DeedRecord>,
    next_seq: u64,
}

impl EnvoyMarkStore {
    /// 從持久化記錄重建（重啟後印記還在）。`next_seq` 取所有記錄最大序號 +1，
    /// 保證新記的善舉序號嚴格遞增（「最近一件」判定不會被舊記錄壓過）。
    pub fn from_entries(entries: Vec<DeedRecord>) -> Self {
        let mut marks = HashMap::new();
        let mut max_seq = 0u64;
        for e in entries {
            max_seq = max_seq.max(e.last_seq);
            // append-only 快照：同 key 後寫者遮蓋先寫者（讀到最後一行為準）。
            marks.insert((e.resident_id.clone(), e.kind), e);
        }
        Self { marks, next_seq: max_seq.saturating_add(1) }
    }

    /// 記下一件引夢使者對某居民的善舉：該格次數 +1、刷新最後序號，回傳快照供落地。
    pub fn record(&mut self, resident_id: &str, resident_name: &str, kind: DeedKind) -> DeedRecord {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        let key = (resident_id.to_string(), kind.to_wire());
        let rec = self.marks.entry(key).or_insert(DeedRecord {
            resident_id: resident_id.to_string(),
            resident_name: resident_name.to_string(),
            kind: kind.to_wire(),
            count: 0,
            last_seq: seq,
        });
        rec.count = rec.count.saturating_add(1);
        rec.resident_name = resident_name.to_string(); // 名字可能變（改名 v1）→ 跟著更新
        rec.last_seq = seq;
        rec.clone()
    }

    /// 這位居民名下所有善舉記錄（未排序）。
    pub fn deeds_for(&self, resident_id: &str) -> Vec<&DeedRecord> {
        self.marks.values().filter(|r| r.resident_id == resident_id).collect()
    }

    /// 這位居民「最近一件」善舉的種類（挑 `last_seq` 最大者），供回想選材。
    /// 沒有任何印記回 None。
    pub fn latest_kind_for(&self, resident_id: &str) -> Option<DeedKind> {
        self.marks
            .values()
            .filter(|r| r.resident_id == resident_id)
            .max_by_key(|r| r.last_seq)
            .and_then(|r| DeedKind::from_wire(r.kind))
    }

    /// 有印記的居民 id 清單（去重＋排序，供 tick 掃「誰記得引夢使者的好」）。
    pub fn residents_with_marks(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.marks.values().map(|r| r.resident_id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// 全部善舉的總次數（面板頂部「你在風禾方界留下的印記」總計）。
    pub fn total_count(&self) -> u64 {
        self.marks.values().map(|r| r.count as u64).sum()
    }

    /// 快照全部記錄（寫檔用）。
    pub fn to_entries(&self) -> Vec<DeedRecord> {
        self.marks.values().cloned().collect()
    }
}

/// 居民日後回想引夢使者某件善舉的台詞（繁中、面向玩家、i18n 集中於此；確定性依 `pick` 選句）。
/// 刻意**不含玩家原話**、只帶玩家（引夢使者）名與固定 clause——無注入／NSFW 風險。
pub fn recall_line(player_name: &str, kind: DeedKind, pick: usize) -> String {
    const FRAMES: &[&str] = &[
        "{p}，還記得嗎——{d}，我到現在都記著。",
        "我常想起{d}，{p}，謝謝你。",
        "有件事我一直沒忘：{d}。",
        "{d}……那份情，我始終放在心上。",
    ];
    FRAMES[pick % FRAMES.len()]
        .replace("{p}", player_name)
        .replace("{d}", kind.recall_clause())
}

/// 把「某居民回想起引夢使者的一件善舉」寫成一句動態牆（繁中；不含玩家原話）。
pub fn feed_line(resident_name: &str, player_name: &str, kind: DeedKind) -> String {
    format!("{resident_name}想起了{player_name}為她{label}的那份情。", label = kind.label())
}

/// 動態牆種類標籤（`voxel_feed`）。
pub const FEED_KIND: &str = "envoy_mark";

/// 居民回想印記的觸及半徑（方塊距離，水平 XZ，居民↔引夢使者）。與 773/774/875 讚賞
/// `ADMIRE_RADIUS` 同量級：得站得夠近、像是面對面說起這件事，而非隔著半個村喊話。
pub const RECALL_RADIUS: f32 = 6.0;

/// 同一位居民回想的冷卻（秒，全域）：回想過一次後這麼久內不再觸發，讓回想稀有有份量、
/// 也把「站在居民旁邊刷回想」的速率天然夾死。比讚賞冷卻更長——回想是更慎重的一刻。
pub const RECALL_COOLDOWN_SECS: u64 = 300;

/// 每輪掃描命中後仍要擲過的機率閘：保持回想稀疏，不是一走近就必定觸發。
pub const RECALL_CHANCE: f32 = 0.35;

/// 是否該觸發回想（純函式）：站得夠近 ＋ 冷卻已過。「這位居民此刻有沒有空、有沒有印記可回想」
/// 由呼叫端另外把關（見 `voxel_ws.rs`）。機率閘也留給呼叫端（IO/RNG 不進純邏輯層）。
pub fn recall_triggers(dist_sq: f32, cooldown_ok: bool) -> bool {
    dist_sq <= RECALL_RADIUS * RECALL_RADIUS && cooldown_ok
}

// ── 持久化 IO（只有函式，鎖在 voxel_ws.rs）──────────────────────────────────

const ENVOY_MARK_FILE: &str = "data/voxel_envoy_marks.jsonl";

/// 從 `data/voxel_envoy_marks.jsonl` 讀取所有記錄（檔案不存在回空 Vec）。
/// 壞行／未知欄位個別略過，不因一行壞掉丟掉整份（向後相容）。
pub fn load_marks() -> Vec<DeedRecord> {
    let content = match std::fs::read_to_string(ENVOY_MARK_FILE) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// 把整份帳本快照 append 一行行到 `data/voxel_envoy_marks.jsonl`。
/// append-only 策略：重啟時 `from_entries` 讀最後快照、老記錄自然被同 key 後寫者遮蓋。
/// 印記格數 = 居民數 × 善舉種類數，極小（數十格頂天），每次寫整份也無壓力。
/// 傳入快照（呼叫端在鎖內取 `to_entries()`、鎖外呼叫本函式），避免持鎖 IO。
pub fn save_marks(entries: &[DeedRecord]) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(ENVOY_MARK_FILE) {
        for entry in entries {
            if let Ok(line) = serde_json::to_string(entry) {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_roundtrips_for_all_kinds() {
        for k in [DeedKind::SaveShadow, DeedKind::Gift, DeedKind::Flower] {
            assert_eq!(DeedKind::from_wire(k.to_wire()), Some(k));
        }
        assert_eq!(DeedKind::from_wire(0), None);
        assert_eq!(DeedKind::from_wire(99), None, "未知編號安全回 None、不 panic");
    }

    #[test]
    fn record_increments_count_and_seq() {
        let mut s = EnvoyMarkStore::from_entries(Vec::new());
        let r1 = s.record("vox_res_0", "諾娃", DeedKind::SaveShadow);
        assert_eq!(r1.count, 1);
        let r2 = s.record("vox_res_0", "諾娃", DeedKind::SaveShadow);
        assert_eq!(r2.count, 2, "同居民同善舉再記一次應累積");
        assert!(r2.last_seq > r1.last_seq, "序號應嚴格遞增");
    }

    #[test]
    fn different_kinds_are_separate_buckets() {
        let mut s = EnvoyMarkStore::from_entries(Vec::new());
        s.record("vox_res_0", "諾娃", DeedKind::Gift);
        s.record("vox_res_0", "諾娃", DeedKind::Flower);
        let deeds = s.deeds_for("vox_res_0");
        assert_eq!(deeds.len(), 2, "送禮與送花應分開兩格");
        assert_eq!(s.total_count(), 2);
    }

    #[test]
    fn latest_kind_tracks_most_recent_deed() {
        let mut s = EnvoyMarkStore::from_entries(Vec::new());
        s.record("vox_res_1", "露娜", DeedKind::SaveShadow);
        s.record("vox_res_1", "露娜", DeedKind::Flower);
        assert_eq!(s.latest_kind_for("vox_res_1"), Some(DeedKind::Flower), "最近一件應為野花");
        // 再送一次禮 → 最近一件變禮物。
        s.record("vox_res_1", "露娜", DeedKind::Gift);
        assert_eq!(s.latest_kind_for("vox_res_1"), Some(DeedKind::Gift));
    }

    #[test]
    fn latest_kind_none_when_no_marks() {
        let s = EnvoyMarkStore::from_entries(Vec::new());
        assert_eq!(s.latest_kind_for("vox_res_9"), None);
    }

    #[test]
    fn residents_with_marks_is_deduped_and_sorted() {
        let mut s = EnvoyMarkStore::from_entries(Vec::new());
        s.record("vox_res_2", "米拉", DeedKind::Gift);
        s.record("vox_res_2", "米拉", DeedKind::Flower);
        s.record("vox_res_0", "諾娃", DeedKind::SaveShadow);
        assert_eq!(s.residents_with_marks(), vec!["vox_res_0".to_string(), "vox_res_2".to_string()]);
    }

    #[test]
    fn from_entries_reconstructs_and_continues_seq() {
        let mut s = EnvoyMarkStore::from_entries(Vec::new());
        s.record("vox_res_0", "諾娃", DeedKind::SaveShadow);
        s.record("vox_res_0", "諾娃", DeedKind::SaveShadow);
        let entries = s.to_entries();
        let mut s2 = EnvoyMarkStore::from_entries(entries);
        assert_eq!(s2.deeds_for("vox_res_0")[0].count, 2, "重建後次數保留");
        // 重建後再記一件，序號必須大於重建前的最大序號。
        let r = s2.record("vox_res_0", "諾娃", DeedKind::Gift);
        assert!(r.last_seq >= 3, "重建後序號應接續遞增");
    }

    #[test]
    fn record_updates_resident_name_on_rename() {
        let mut s = EnvoyMarkStore::from_entries(Vec::new());
        s.record("vox_res_0", "舊名", DeedKind::Gift);
        let r = s.record("vox_res_0", "新名", DeedKind::Gift);
        assert_eq!(r.resident_name, "新名", "改名後快照名字應更新");
    }

    #[test]
    fn recall_line_is_deterministic_and_carries_name() {
        let a = recall_line("引夢使者", DeedKind::SaveShadow, 0);
        let b = recall_line("引夢使者", DeedKind::SaveShadow, 0);
        assert_eq!(a, b);
        assert!(a.contains("引夢使者"));
        assert!(a.contains("暗影"), "驅散暗影的回想應提到暗影");
        assert!(!a.contains("{p}") && !a.contains("{d}"), "佔位符應全被替換");
    }

    #[test]
    fn recall_line_clauses_differ_by_kind() {
        let shadow = recall_line("旅人", DeedKind::SaveShadow, 2);
        let flower = recall_line("旅人", DeedKind::Flower, 2);
        assert!(shadow.contains("暗影"));
        assert!(flower.contains("野花"));
        assert_ne!(shadow, flower);
    }

    #[test]
    fn recall_line_pick_wraps_without_panic() {
        let line = recall_line("旅人", DeedKind::Gift, usize::MAX);
        assert!(!line.is_empty());
    }

    #[test]
    fn recall_frames_are_distinct() {
        let lines: std::collections::HashSet<String> =
            (0..4).map(|p| recall_line("旅人", DeedKind::Gift, p)).collect();
        assert_eq!(lines.len(), 4, "四句框架應各不相同");
    }

    #[test]
    fn feed_line_mentions_resident_and_player_and_no_newline() {
        let line = feed_line("諾娃", "引夢使者", DeedKind::Flower);
        assert!(line.contains("諾娃"));
        assert!(line.contains("引夢使者"));
        assert!(line.contains("野花"));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn recall_triggers_needs_near_and_cooldown() {
        let r = RECALL_RADIUS;
        assert!(recall_triggers(r * r - 0.01, true));
        assert!(recall_triggers(r * r, true), "恰好在半徑上應算近旁（<=）");
        assert!(!recall_triggers(r * r + 0.01, true), "太遠不該觸發");
        assert!(!recall_triggers(0.0, false), "冷卻未到不該觸發");
    }

    #[test]
    fn recall_chance_is_a_valid_probability() {
        assert!((0.0..=1.0).contains(&RECALL_CHANCE));
    }

    #[test]
    fn labels_are_stable_nonempty() {
        for k in [DeedKind::SaveShadow, DeedKind::Gift, DeedKind::Flower] {
            assert!(!k.label().is_empty());
        }
    }
}
