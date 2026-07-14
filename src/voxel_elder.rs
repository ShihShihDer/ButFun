//! 乙太方界·居民晚年 v1（voxel_elder，自主提案切片，ROADMAP 987）。
//!
//! **真缺口**：生命週期軸線已蓋到「出生（819 誕辰）→長大成人（942）→成家（927/928）→
//! 手足（941）」，但走到成年禮（942）之後就戛然而止——世代傳承誕生的居民一旦長大成人，
//! 終其一生（無論再活過幾個乙太年）永遠只是「成年人」，生命週期只有起點與中點，沒有終點
//! 之前的那一段。真實的人生會走進晚年，乙太方界至今完全沒有這一段。
//!
//! **本刀補上晚年這一環**：世代傳承誕生（`birth_unix > 0`）的居民活過整整
//! [`ELDER_YEARS`] 個乙太年，第二次、也是這輩子最後一次生命階段轉換——步入晚年。
//! 一生僅有一次，行一場安靜的感言：自己說一句歷經歲月的話、記進心裡一筆含「一定會」的
//! 永久精華記憶、動態牆播報「村中多了一位長者」。
//!
//! **真實行為後果（不是純氛圍）**：晚年不只是換個稱謂——它**真的改變既有互動的走向**。
//! 老朋友到訪本可能演一齣小拌嘴（715 `voxel_quarrel`），但若主人已是長者，歷經歲月的
//! 從容會**化解**這場本該發生的小摩擦，改演一齣「長者一笑置之」的溫暖橋段，而非真的
//! 吵起來。生命階段第一次反過來影響既有系統的走向，而非只是新增一條平行支線。
//!
//! **與既有系統 razor-sharp 區隔**：
//! - **942 長大成人**＝生命階段轉換**第一拍**（新生兒→成年人），真實後果是「可被選為
//!   父母」；本刀＝生命階段轉換**第二拍、也是最後一拍**（成年人→長者），真實後果是
//!   「化解拌嘴」。兩者都是一生一次的門檻事件，發生在完全不同的年歲、換來完全不同的
//!   行為後果，非同一維度的重複。
//! - **715 拌嘴**＝老朋友到訪的既有隨機小插曲；本刀**不新增**拌嘴的觸發條件，只是在
//!   拌嘴本該觸發的那一刻，依主人的生命階段接手改寫它的走向——兩個模組協作而非重疊。
//! - **819 誕辰紀念**＝**每年**都會發生的週期性回望；本刀（比照 942）＝**一生一次**的
//!   不可逆轉捩點。
//!
//! **純邏輯層**：轉態判定、感言／記憶／Feed 文案、拌嘴化解文案全是確定性純函式，可窮舉
//! 單元測試。持久化（[`ElderStore`] + jsonl）保證晚年感言一生只行一次、跨重啟不重觸發
//! （restart-safe，比照 942 用持久化 store 而非會歸零的記憶體旗標）。鎖／WS／IO 觸發全
//! 留在 `voxel_ws.rs`（短鎖循序、鎖外事件佇列，守 prod 死鎖鐵律）。**零 LLM、零協議
//! 破壞、零新美術、零前端改動、FPS 零影響**（晚年轉換與拌嘴本就極低頻）。**零玩家輸入**
//! （居民自發，無濫用面）。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::voxel_birthday::YEAR_SECS;

/// 晚年門檻：世代傳承誕生的居民要活過這麼多個「乙太年」才步入晚年。
/// 設在成年門檻（[`crate::voxel_coming_of_age::COMING_OF_AGE_YEARS`]＝1 年）之後的第 3 年，
/// 讓「成年」與「晚年」在時間軸上明顯拉開一段，不會同一輪春夏秋冬內接連轉兩次階段。
pub const ELDER_YEARS: u64 = 3;

/// 晚年門檻換算成秒。
pub const ELDER_SECS: u64 = ELDER_YEARS * YEAR_SECS;

/// 持久化路徑（`data/` 已 gitignore）。每行一筆「已步入晚年的居民 id」，append-only。
const ELDER_PATH: &str = "data/voxel_elder.jsonl";

/// 顯示名截斷上限（防超長顯示名破泡泡／Feed 框，比照 `voxel_coming_of_age` 慣例）。
const NAME_MAX: usize = 12;

/// 截斷顯示名到上限（以字元計，中文安全）。
fn clip(name: &str) -> String {
    name.chars().take(NAME_MAX).collect()
}

/// 截斷；空名退回泛稱（記憶／Feed 永不出現空洞的名字）。
fn clip_or(name: &str, fallback: &str) -> String {
    let s = clip(name);
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

/// 是否正是「這一刻剛步入晚年」（該觸發晚年感言）：世代傳承誕生（`birth_unix > 0`）、
/// 此前尚未步入晚年（`already == false`）、且已活過 [`ELDER_SECS`]。初始四位居民
/// （`birth_unix == 0`）永不觸發——她們沒有「誕生時刻」，比照 942 對她們的誠實取捨。
pub fn is_elder_moment(birth_unix: u64, now: u64, already: bool) -> bool {
    birth_unix != 0 && !already && now.saturating_sub(birth_unix) >= ELDER_SECS
}

/// 居民步入晚年那一刻，自己說的一句話（確定性三選一）。
pub fn elder_say_line(name: &str, pick: usize) -> String {
    let n = clip_or(name, "我");
    const LINES: [&str; 3] = [
        "不知不覺，{n}也走到了這把年紀啊——往後就換年輕人多擔待點了。",
        "{n}這些年看著這片天地一點一滴長大，如今也算是村裡的長者了。",
        "歲月是真的走得快，{n}如今回頭看，滿滿都是這片天地的日子。",
    ];
    LINES[pick % LINES.len()].replace("{n}", &n)
}

/// 居民把「我步入晚年了」記進心裡的一筆記憶（第一人稱、含「一定會」→ 記憶系統判為永久
/// 精華事實）。刻意不用「」包住任何動態內容、也不內嵌動態名，全是靜態句、天然安全
/// （比照 942 對 `extract_inner_quote` 陷阱的取捨）。
pub fn elder_memory_line() -> String {
    "我在這片天地度過了大半輩子，如今也步入晚年了。往後我一定會把這些年攢下的從容，
留給還年輕的大家。"
        .replace('\n', "")
}

/// 晚年感言的世界動態牆分類標籤。
pub const FEED_KIND: &str = "步入晚年";

/// 世界動態牆的晚年播報：村裡多了一位歷經歲月的長者。
pub fn elder_feed_line(name: &str) -> String {
    let n = clip_or(name, "一位居民");
    format!("{n}不知不覺也走到了晚年，成了村裡受人敬重的長者")
}

/// 拌嘴化解的世界動態牆分類標籤（與 715 拌嘴刻意區隔，讓動態牆一眼看出「這次沒吵起來」）。
pub const FEED_KIND_DEFUSE: &str = "長者化解";

/// 拌嘴化解主題池，與 `voxel_quarrel` 的拌嘴主題一一對應（同一件雞毛蒜皮小事，只是這次
/// 被長者的從容接住了）。
const DEFUSE_TOPICS: [&str; 6] = [
    "該誰澆花",
    "木頭該怎麼分才公平",
    "誰又忘記關門",
    "工具該放在哪裡",
    "誰把院子踩出一條新路",
    "誰吃掉了最後一顆胡蘿蔔",
];

/// 依 `pick` 確定性選一個化解主題（循環取模，與 `voxel_quarrel::pick_topic` 同一套模數）。
fn pick_defuse_topic(pick: usize) -> &'static str {
    DEFUSE_TOPICS[pick % DEFUSE_TOPICS.len()]
}

/// 長者化解拌嘴的一句話（冒在長者頭頂，取代原本會發生的拌嘴）。
pub fn elder_defuse_say_line(host: &str, pick: usize) -> String {
    let topic = pick_defuse_topic(pick);
    format!("「{topic}呀……」{host}笑了笑擺擺手，這點小事，值得吵嗎？")
}

/// Feed 播報文字（第三人稱，讓沒在場的玩家也知道世界有這麼一齣）。
pub fn elder_defuse_feed_line(visitor: &str, host: &str, pick: usize) -> String {
    let topic = pick_defuse_topic(pick);
    format!("{visitor}本想跟{host}為了{topic}拌幾句嘴，卻被長者的從容一笑帶過了")
}

// ── 晚年帳本（一生一次的冪等持久化，比照 voxel_coming_of_age 慣例）───────────────

/// 一筆持久化記錄：某居民已步入晚年。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ElderEntry {
    pub resident: String,
}

/// 晚年帳本（純同步資料結構，由呼叫端包進 `RwLock`）。冪等：同一居民一生只步入晚年一次。
#[derive(Default, Debug)]
pub struct ElderStore {
    /// 已步入晚年的居民 id 集合。
    done: HashSet<String>,
}

impl ElderStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次），確保重啟後不會重觸發已辦過的晚年感言。
    pub fn from_entries(entries: impl IntoIterator<Item = ElderEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            s.done.insert(e.resident);
        }
        s
    }

    /// 標記某居民已步入晚年。回傳 `true` 代表「本次才第一次步入晚年」——呼叫端只在回
    /// `true` 時才 append 持久化＋寫永久記憶＋上動態牆；已步入過再呼叫安全回 `false`，冪等。
    pub fn mark(&mut self, resident: &str) -> bool {
        self.done.insert(resident.to_string())
    }

    /// 某居民是否已步入晚年（供拌嘴化解判定使用）。
    pub fn has(&self, resident: &str) -> bool {
        self.done.contains(resident)
    }

    /// 已步入晚年居民 id 集合的快照（供 tick 在取 residents 寫鎖**前**先快照，避免鎖巢狀，
    /// 守死鎖鐵律）。
    pub fn snapshot(&self) -> HashSet<String> {
        self.done.clone()
    }
}

// ── jsonl 持久化（append-only，比照 voxel_coming_of_age::append_entry 慣例）──────

/// Append 一筆晚年記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_entry(entry: &ElderEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(ELDER_PATH, &line);
    }
}

/// 載回所有晚年記錄（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_entries() -> Vec<ElderEntry> {
    let content = match std::fs::read_to_string(ELDER_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                None
            } else {
                serde_json::from_str::<ElderEntry>(l).ok()
            }
        })
        .collect()
}

fn write_line(path: &str, line: &str) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("無法寫入晚年記錄 {path}: {e}"),
    }
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn founders_never_become_elder() {
        // 初始四位居民 birth_unix == 0：無「誕生時刻」，永不觸發晚年（比照 942 誠實取捨）。
        assert!(!is_elder_moment(0, 0, false));
        assert!(!is_elder_moment(0, 999_999_999, false));
    }

    #[test]
    fn newborn_not_elder_until_threshold() {
        let birth = 1_000_000;
        assert!(!is_elder_moment(birth, birth, false));
        assert!(!is_elder_moment(birth, birth + ELDER_SECS - 1, false));
        assert!(is_elder_moment(birth, birth + ELDER_SECS, false));
        assert!(is_elder_moment(birth, birth + ELDER_SECS * 5, false));
    }

    #[test]
    fn elder_moment_idempotent_when_already_marked() {
        let birth = 1_000_000;
        let old = birth + ELDER_SECS;
        assert!(is_elder_moment(birth, old, false));
        assert!(!is_elder_moment(birth, old, true)); // 已步入過 → 不重複觸發
    }

    #[test]
    fn is_elder_moment_handles_clock_going_backwards() {
        // now < birth_unix（時鐘回退等異常）：saturating_sub → 0，非晚年，不 panic。
        assert!(!is_elder_moment(1_000_000, 500_000, false));
    }

    #[test]
    fn elder_secs_is_three_ether_years() {
        assert_eq!(ELDER_SECS, YEAR_SECS * 3);
    }

    #[test]
    fn elder_years_after_coming_of_age_years() {
        // 晚年門檻須晚於成年門檻，兩次生命階段轉換不會擠在同一輪年歲。
        assert!(ELDER_YEARS > crate::voxel_coming_of_age::COMING_OF_AGE_YEARS);
    }

    #[test]
    fn elder_say_line_mentions_name_and_varies_with_pick() {
        let a = elder_say_line("露娜", 0);
        let b = elder_say_line("露娜", 1);
        assert!(a.contains('露'));
        assert_ne!(a, b);
    }

    #[test]
    fn elder_say_line_empty_name_falls_back() {
        let line = elder_say_line("", 0);
        assert!(line.contains('我'));
        assert!(!line.is_empty());
    }

    #[test]
    fn elder_memory_line_is_permanent_essence_and_single_line() {
        let line = elder_memory_line();
        assert!(line.contains("一定會"));
        assert!(!line.contains('\n'));
        assert!(!line.is_empty());
    }

    #[test]
    fn elder_feed_line_mentions_name() {
        let line = elder_feed_line("諾娃");
        assert!(line.contains("諾娃"));
        assert!(line.contains("長者"));
    }

    #[test]
    fn elder_feed_line_empty_name_falls_back() {
        let line = elder_feed_line("");
        assert!(!line.is_empty());
    }

    #[test]
    fn elder_defuse_say_line_mentions_host_and_varies_with_pick() {
        let a = elder_defuse_say_line("賽勒", 0);
        let b = elder_defuse_say_line("賽勒", 1);
        assert!(a.contains("賽勒"));
        assert_ne!(a, b);
    }

    #[test]
    fn elder_defuse_feed_line_mentions_both_names() {
        let line = elder_defuse_feed_line("露娜", "諾娃", 2);
        assert!(line.contains("露娜"));
        assert!(line.contains("諾娃"));
    }

    #[test]
    fn pick_defuse_topic_wraps_around_deterministically() {
        let n = DEFUSE_TOPICS.len();
        assert_eq!(pick_defuse_topic(0), pick_defuse_topic(n));
        assert_eq!(pick_defuse_topic(1), pick_defuse_topic(n + 1));
    }

    #[test]
    fn pick_defuse_topic_covers_all_entries_across_first_cycle() {
        let n = DEFUSE_TOPICS.len();
        let seen: std::collections::HashSet<&str> = (0..n).map(pick_defuse_topic).collect();
        assert_eq!(seen.len(), n);
    }

    #[test]
    fn elder_store_round_trip_and_idempotent_mark() {
        let mut store = ElderStore::new();
        assert!(!store.has("vox_res_0"));
        assert!(store.mark("vox_res_0")); // 第一次 → true
        assert!(!store.mark("vox_res_0")); // 已標記過 → false，冪等
        assert!(store.has("vox_res_0"));

        let restored = ElderStore::from_entries(vec![ElderEntry { resident: "vox_res_1".into() }]);
        assert!(restored.has("vox_res_1"));
        assert!(!restored.has("vox_res_0"));
    }

    #[test]
    fn elder_store_snapshot_is_independent_copy() {
        let mut store = ElderStore::new();
        store.mark("vox_res_2");
        let snap = store.snapshot();
        store.mark("vox_res_3");
        assert!(snap.contains("vox_res_2"));
        assert!(!snap.contains("vox_res_3")); // 快照不受之後的變更影響
    }
}
