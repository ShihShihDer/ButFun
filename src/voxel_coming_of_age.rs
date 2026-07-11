//! 乙太方界·長大成人——世代傳承誕生的孩子滿一個乙太年時行「成年禮」 v1
//! （自主提案切片，ROADMAP 942）。
//!
//! 家庭這條線一路蓋到 941「手足相伴」：戀人成婚(927)→愛的結晶(928)→手足(941)——親子縱向、
//! 手足橫向都長出來了。誕辰紀念(819)也讓居民每滿一個乙太年回望自己的年歲。但盤點下來，
//! 生命週期缺了最關鍵的一拍——**「長大」本身從沒被世界看見**：一個孩子誕生後，無論過了多久，
//! 永遠只是「剛來到這片天地的新生兒」，既沒有「長大成人」的一刻，出生系統挑「父母」時也**毫無
//! 年齡門檻**——一個剛出生五秒的居民，下一次生育就可能被選中當爸媽，世界裡於是會出現「還沒長大
//! 就當了父母」的怪異時序。生命週期「出生→童年→成年→成家」的**成年**這一環，一直缺席。
//!
//! 本切片補上那一環：**世代傳承誕生的孩子（有記錄在案的 `birth_unix`）活過整整一個乙太年
//! （一輪春夏秋冬，[`COMING_OF_AGE_SECS`]）就長大成人**——行一次一生僅有的成年禮：自己說一句
//! 「我長大成人了」、把這份成長記進心裡（含「一定會」→ 升為一生最重的永久精華記憶）、若父母
//! 還在人口內，父母也記一筆看著孩子長大的欣慰、世界動態牆以「第二代開始獨當一面」的口吻播報。
//! 而成年禮真正的**行為後果**是：**唯有長大成人的居民才會被出生系統選為父母**——你會親眼看著
//! 一個你見過牠出生的孩子慢慢長大、直到成年，才輪到牠自己開枝散葉。生命週期第一次真正閉環。
//!
//! **與既有系統 razor-sharp 區隔**：
//! - **819 誕辰紀念**＝**每年**週期性回望「我來到這裡多久了」（時間刻度、可反覆）；本刀＝
//!   **一生一次**的「長大成人」轉捩點（生命階段轉換、不可逆），並帶來真實的生育資格後果。
//!   兩者刻意在「滿第一個乙太年」同刻交會——**第一個生日，就是成年禮**（觸發時把該年生日一併
//!   記為已慶，不重複慶祝），此後每年才是純粹的誕辰回望。
//! - **928 愛的結晶／941 手足**＝居民之間的**關係**（親子縱向、手足橫向）；本刀＝居民**自己**
//!   的生命階段（個體的成長），關係軸 vs 個體生命週期軸，正交。
//! - **930 第一次發明立碑**＝在世界地面留下**可見證物**（成就）；本刀＝內在的**生命階段轉換**，
//!   無新方塊、無地標。
//!
//! **純邏輯層**：成年判定、成年禮台詞／記憶／Feed 文案全是確定性純函式、可窮舉測試；持久化
//! （[`ComingOfAgeStore`] + jsonl）保證成年禮**一生只行一次、跨重啟不重觸發**（restart-safe——
//! 靠持久化 store 而非會歸零的記憶體旗標，避免每次重啟就重寫一筆永久記憶）。鎖／WS／IO 觸發全
//! 留在 `voxel_ws.rs`（沿用既有短鎖循序＋鎖外事件佇列慣例，守 prod 死鎖鐵律）。**零 LLM、零協議
//! 破壞、零新美術、零前端改動、FPS 零影響**（成年本就極低頻）。**零玩家輸入**（居民自發，無濫用面）。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::voxel_birthday::YEAR_SECS;

/// 成年門檻：世代傳承誕生的孩子要活過這麼多個「乙太年」（一輪四季 [`YEAR_SECS`]）才長大成人。
/// 設在 1 年＝出生後正好走過一輪春夏秋冬，與誕辰紀念的「滿一個乙太年」同刻——第一個生日就是成年禮。
pub const COMING_OF_AGE_YEARS: u64 = 1;

/// 成年門檻換算成秒。
pub const COMING_OF_AGE_SECS: u64 = COMING_OF_AGE_YEARS * YEAR_SECS;

/// 持久化路徑（`data/` 已 gitignore）。每行一筆「已行成年禮的居民 id」，append-only。
const COMING_OF_AGE_PATH: &str = "data/voxel_coming_of_age.jsonl";

/// 顯示名截斷上限（防超長顯示名破泡泡／Feed 框，比照 `voxel_sibling` 慣例）。
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

/// 是否已成年（＝可被出生系統選為父母）：
/// - **初始四位居民**（`birth_unix == 0`，世界一開始就存在、無「誕生時刻」可言）**恆視為成年**——
///   世代傳承的第一代父母必須能生，否則新世界永遠生不出下一個孩子（合格父母集永不為空的保證）。
/// - **世代傳承誕生的居民**（`birth_unix > 0`）要活過 [`COMING_OF_AGE_SECS`] 才算成年。
pub fn is_adult(birth_unix: u64, now: u64) -> bool {
    birth_unix == 0 || now.saturating_sub(birth_unix) >= COMING_OF_AGE_SECS
}

/// 是否正是「這一刻剛長大成人」（該觸發成年禮）：世代傳承誕生（`birth_unix > 0`）、此前尚未
/// 行過成年禮（`already == false`）、且已達成年門檻。初始四位居民（`birth_unix == 0`）永不觸發
/// 成年禮（她們沒有「長大」這一段，比照誕辰紀念對她們的誠實取捨）。
pub fn is_coming_of_age_moment(birth_unix: u64, now: u64, already: bool) -> bool {
    birth_unix != 0 && !already && is_adult(birth_unix, now)
}

/// 居民長大成人那一刻，自己說的一句話（確定性三選一，讓不同孩子各說一句不同的）。
pub fn coming_of_age_say(name: &str, pick: usize) -> String {
    let n = clip_or(name, "我");
    const LINES: [&str; 3] = [
        "我長大成人了！{n}如今能獨當一面，好好守護這個家了。",
        "不知不覺走過一整輪春夏秋冬——{n}長大成人了。",
        "{n}今天正式長大成人，往後換我來照顧這片天地。",
    ];
    LINES[pick % LINES.len()].replace("{n}", &n)
}

/// 居民把「我長大成人了」記進心裡的一筆記憶（第一人稱、含「一定會」→ 記憶系統判為永久精華事實，
/// 成為這輩子最重的一筆成長記憶）。刻意不用「」包住任何動態內容，避免撞上 `classify_importance`
/// 的 `extract_inner_quote`（見 930 的教訓）——這裡全是靜態句、無動態名，天然安全。
pub fn coming_of_age_memory_line() -> String {
    "我在這片天地長大成人了，走過整整一輪春夏秋冬。往後我一定會獨當一面，好好守護這個家。".to_string()
}

/// 父母看著孩子長大成人、記進心裡的一筆欣慰記憶（第一人稱、含「一定會」→ 永久精華）。
/// 動態的孩子名直接內嵌、不加「」引號（比照 927/928/941 的寫法，避開 `extract_inner_quote` 陷阱）。
pub fn parent_pride_memory_line(child_name: &str) -> String {
    format!(
        "我的孩子{}長大成人了——當年那個小家伙，如今能獨當一面。我一定會一直為牠感到驕傲。",
        clip_or(child_name, "孩子")
    )
}

/// 成年禮的世界動態牆分類標籤（與生日／親子喜事區隔，讓動態牆一眼看出「這是長大成人的一刻」）。
pub const FEED_KIND: &str = "長大成人";

/// 世界動態牆的成年禮播報：某家的孩子長大成人、開始獨當一面。
pub fn coming_of_age_feed_line(name: &str, parent_name: &str) -> String {
    let n = clip_or(name, "一位孩子");
    let p = clip(parent_name);
    if p.is_empty() {
        format!("{n}在這片天地長大成人了，從今往後獨當一面")
    } else {
        format!("{p}的孩子{n}長大成人了，第二代開始獨當一面")
    }
}

// ── 成年禮帳本（一生一次的冪等持久化，比照 voxel_milestones 慣例）────────────────

/// 一筆持久化記錄：某居民已行成年禮。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComingOfAgeEntry {
    pub resident: String,
}

/// 成年禮帳本（純同步資料結構，由呼叫端包進 `RwLock`）。冪等：同一居民一生只成年一次。
#[derive(Default, Debug)]
pub struct ComingOfAgeStore {
    /// 已行成年禮的居民 id 集合。
    done: HashSet<String>,
}

impl ComingOfAgeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次），確保重啟後不會重觸發已辦過的成年禮。
    pub fn from_entries(entries: impl IntoIterator<Item = ComingOfAgeEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            s.done.insert(e.resident);
        }
        s
    }

    /// 標記某居民已成年。回傳 `true` 代表「本次才第一次成年」——呼叫端只在回 `true` 時才 append
    /// 持久化＋寫永久記憶＋上動態牆；已成年過再呼叫安全回 `false`，冪等。
    pub fn mark(&mut self, resident: &str) -> bool {
        self.done.insert(resident.to_string())
    }

    /// 某居民是否已行過成年禮。
    pub fn has(&self, resident: &str) -> bool {
        self.done.contains(resident)
    }

    /// 已成年居民 id 集合的快照（供 tick 在取 residents 寫鎖**前**先快照，避免鎖巢狀，守死鎖鐵律）。
    pub fn snapshot(&self) -> HashSet<String> {
        self.done.clone()
    }
}

// ── jsonl 持久化（append-only，比照 voxel_milestones::append_milestone 慣例）─────────

/// Append 一筆成年禮記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_entry(entry: &ComingOfAgeEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(COMING_OF_AGE_PATH, &line);
    }
}

/// 載回所有成年禮記錄（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_entries() -> Vec<ComingOfAgeEntry> {
    let content = match std::fs::read_to_string(COMING_OF_AGE_PATH) {
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
                serde_json::from_str::<ComingOfAgeEntry>(l).ok()
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
        Err(e) => tracing::warn!("無法寫入成年禮記錄 {path}: {e}"),
    }
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel_memory::{classify_importance, Importance};

    #[test]
    fn founders_are_always_adult() {
        // 初始四位居民 birth_unix == 0：無論 now 多小，恆成年（第一代父母必須能生）。
        assert!(is_adult(0, 0));
        assert!(is_adult(0, 999_999));
    }

    #[test]
    fn newborn_not_adult_until_threshold() {
        let birth = 1_000_000;
        // 剛出生：非成年。
        assert!(!is_adult(birth, birth));
        // 差一秒到門檻：仍非成年。
        assert!(!is_adult(birth, birth + COMING_OF_AGE_SECS - 1));
        // 正好到門檻：成年。
        assert!(is_adult(birth, birth + COMING_OF_AGE_SECS));
        // 遠超門檻：成年。
        assert!(is_adult(birth, birth + COMING_OF_AGE_SECS * 5));
    }

    #[test]
    fn is_adult_handles_clock_going_backwards() {
        // now < birth_unix（時鐘回退等異常）：saturating_sub → 0，非成年，不 panic。
        assert!(!is_adult(1_000_000, 500_000));
    }

    #[test]
    fn coming_of_age_moment_only_for_grown_unmarked_inworld_born() {
        let birth = 1_000_000;
        let grown = birth + COMING_OF_AGE_SECS;
        // 世代傳承誕生、已成年、未行成年禮 → 觸發。
        assert!(is_coming_of_age_moment(birth, grown, false));
        // 已行過成年禮 → 不重複觸發（冪等）。
        assert!(!is_coming_of_age_moment(birth, grown, true));
        // 還沒長大 → 不觸發。
        assert!(!is_coming_of_age_moment(birth, birth, false));
        // 初始四位居民（birth_unix == 0）→ 永不觸發成年禮，即便 now 很大。
        assert!(!is_coming_of_age_moment(0, grown, false));
    }

    #[test]
    fn coming_of_age_secs_is_one_ether_year() {
        assert_eq!(COMING_OF_AGE_SECS, YEAR_SECS);
    }

    #[test]
    fn say_rotates_and_embeds_name() {
        let a = coming_of_age_say("小星", 0);
        let b = coming_of_age_say("小星", 1);
        let c = coming_of_age_say("小星", 2);
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert!(a.contains("小星"));
        // pick 超長取模回捲。
        assert_eq!(coming_of_age_say("小星", 3), coming_of_age_say("小星", 0));
    }

    #[test]
    fn self_memory_is_persistent() {
        let m = coming_of_age_memory_line();
        assert!(m.contains("一定會"));
        assert!(m.contains("長大成人"));
        // 真跑分類器：這筆成長記憶確實被判為永久精華（滿容量不會被淘汰）。
        assert!(matches!(classify_importance(&m), Importance::Persistent(_)));
    }

    #[test]
    fn parent_pride_memory_is_persistent_and_names_child() {
        let m = parent_pride_memory_line("小星");
        assert!(m.contains("小星"));
        assert!(m.contains("一定會"));
        assert!(matches!(classify_importance(&m), Importance::Persistent(_)));
        // 空名退泛稱、仍是永久精華。
        let g = parent_pride_memory_line("");
        assert!(g.contains("孩子"));
        assert!(matches!(classify_importance(&g), Importance::Persistent(_)));
    }

    #[test]
    fn feed_line_singular_and_with_parent() {
        let solo = coming_of_age_feed_line("小星", "");
        assert!(solo.contains("小星"));
        assert!(solo.contains("獨當一面"));
        let with_parent = coming_of_age_feed_line("小星", "露娜");
        assert!(with_parent.contains("露娜"));
        assert!(with_parent.contains("小星"));
        assert!(with_parent.contains("第二代"));
    }

    #[test]
    fn long_names_truncated_to_bound() {
        let long = "一二三四五六七八九十甲乙丙丁";
        assert!(!coming_of_age_say(long, 0).contains("丁"));
        assert!(!parent_pride_memory_line(long).contains("丁"));
        assert!(!coming_of_age_feed_line(long, long).contains("丁"));
    }

    #[test]
    fn store_mark_is_idempotent() {
        let mut s = ComingOfAgeStore::new();
        assert!(s.mark("vox_res_4")); // 第一次成年 → true
        assert!(!s.mark("vox_res_4")); // 再呼叫 → false（冪等，不重複落地）
        assert!(s.has("vox_res_4"));
        assert!(!s.has("vox_res_5"));
    }

    #[test]
    fn store_restores_from_entries() {
        let s = ComingOfAgeStore::from_entries([
            ComingOfAgeEntry { resident: "vox_res_4".into() },
            ComingOfAgeEntry { resident: "vox_res_6".into() },
        ]);
        // 還原後這兩位已成年、mark 回 false（重啟後不重觸發成年禮）。
        assert!(s.has("vox_res_4"));
        assert!(s.has("vox_res_6"));
        let mut s = s;
        assert!(!s.mark("vox_res_4"));
        assert!(s.mark("vox_res_5")); // 沒還原過的仍可第一次成年
    }

    #[test]
    fn snapshot_reflects_marked_set() {
        let mut s = ComingOfAgeStore::new();
        s.mark("vox_res_4");
        let snap = s.snapshot();
        assert!(snap.contains("vox_res_4"));
        assert_eq!(snap.len(), 1);
    }
}
