//! 乙太方界·居民戀愛心動 v1（ROADMAP 846）——居民↔居民關係全庫至今只有一條軸線：
//! 情誼（`voxel_bonds`，陌生→相識→老朋友，拜訪次數驅動、任何互動都可能升溫）。本模組補上
//! 一條全新、獨立的浪漫軸線：**老朋友**並肩坐在長椅上閒聊（`voxel_bench_chat`）時，偶爾激起
//! 一次心動火花，締結成一對「戀人」——小社會裡至今唯一空白的人性化羈絆。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **情誼（`voxel_bonds`）**＝「熟不熟」的量尺，拜訪/互動次數線性累積、任何一對都會慢慢升溫；
//!   本模組＝「老朋友」門檻之上的**機率質變**，多數老朋友終究只是朋友，只有少數擦出火花。
//! - **居民夜觀星象（`voxel_stargaze`）**＝**玩家↔居民**的浪漫一拍（記得你愛看星星、邀你同賞）；
//!   本模組＝**居民↔居民**之間的戀愛關係，觸發者與對象皆不同，不重疊。
//! - **一生只有一位戀人**（現實張力）：已締結的一方不會再與第三人擦出新的火花，避免「腳踏兩條
//!   船」的荒謬感；多數老朋友終究不會擦出火花（機率門檻低），戀人因此稀有、有份量。
//!
//! 純邏輯層（無 IO、無鎖、無 async、無 LLM），IO 在 `voxel_ws.rs`。
//! 持久化格式：`data/voxel_romance.jsonl`（每行一對 `RomanceEntry`，append-only 快照，比照
//! `voxel_bonds` 手法——戀人對數極少，每次締結時整份快照重寫一行也不會無限長大）。

use std::collections::HashSet;
use serde::{Deserialize, Serialize};

/// 每次符合條件（並坐閒聊 + 交情已到老朋友 + 雙方皆未有戀人）時，真的擦出心動火花的機率。
/// 刻意偏低：戀愛是可遇不可求的稀有質變，不是每對老朋友遲早都會擦出火花。
pub const SPARK_CHANCE: f32 = 0.12;

/// 一筆持久化記錄：一對戀人（`id_a`/`id_b` 為居民**顯示名**，比照 `voxel_bonds::BondEntry`
/// 以顯示名記帳，避免系統 id 與顯示名兩套鍵值不一致的既有教訓）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RomanceEntry {
    pub id_a: String,
    pub id_b: String,
}

/// 正規化一對名字的鍵順序，讓 (a,b) 與 (b,a) 落在同一個鍵。
fn norm(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// 居民戀愛帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct ResidentRomance {
    pairs: HashSet<(String, String)>,
}

impl ResidentRomance {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。
    pub fn from_entries(entries: impl IntoIterator<Item = RomanceEntry>) -> Self {
        let mut r = Self::new();
        for e in entries {
            r.pairs.insert(norm(&e.id_a, &e.id_b));
        }
        r
    }

    /// 這兩位是否已是戀人。
    pub fn is_sweetheart(&self, a: &str, b: &str) -> bool {
        self.pairs.contains(&norm(a, b))
    }

    /// 這位居民是否已有戀人（一生只能有一位，見模組說明）。
    pub fn has_partner(&self, name: &str) -> bool {
        self.pairs.iter().any(|(x, y)| x == name || y == name)
    }

    /// 記一次心動火花（冪等）：真正新締結才回傳 `true`——呼叫端只在回傳 `true` 時才落地
    /// 持久化 / 廣播 / 寫記憶，避免重複觸發洗版。
    pub fn record_spark(&mut self, a: &str, b: &str) -> bool {
        let key = norm(a, b);
        if self.pairs.contains(&key) {
            return false;
        }
        self.pairs.insert(key);
        true
    }

    /// 快照成持久化記錄清單（供 `save_romance` 整份 append 一行）。
    pub fn to_entries(&self) -> Vec<RomanceEntry> {
        self.pairs
            .iter()
            .map(|(a, b)| RomanceEntry { id_a: a.clone(), id_b: b.clone() })
            .collect()
    }
}

/// 是否擲中心動火花（純函式、roll 由呼叫端 `rand::random::<f32>()` 提供，確定可測）。
pub fn spark_roll(roll: f32) -> bool {
    roll < SPARK_CHANCE
}

/// 締結戀人當下，雙方各自寫進記憶的一句（掛在對方名下）。
pub fn sweetheart_memory_line(other_name: &str) -> String {
    format!("那天並肩坐著閒聊，我忽然發現自己對{other_name}心動了——我們，成了戀人。")
}

/// 締結戀人當下，城鎮動態牆的一句播報。
pub fn sweetheart_feed_line(a: &str, b: &str) -> String {
    format!("{a}和{b}並肩坐著聊著聊著，忽然相視一笑——他們，成了戀人。")
}

// ── 持久化 IO（只有函式，鎖在 voxel_ws.rs）──────────────────────────────────

const ROMANCE_FILE: &str = "data/voxel_romance.jsonl";

/// 從 `data/voxel_romance.jsonl` 讀取所有記錄（檔案不存在回空 Vec）。
pub fn load_romance() -> Vec<RomanceEntry> {
    let content = match std::fs::read_to_string(ROMANCE_FILE) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// 把整份戀愛帳本快照 append 一行到 `data/voxel_romance.jsonl`（比照 `voxel_bonds::save_bonds`：
/// 戀人對數極少，每次締結時整份快照重寫一行也不會無限長大）。
pub fn save_romance(romance: &ResidentRomance) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ROMANCE_FILE)
    {
        for entry in romance.to_entries() {
            if let Ok(line) = serde_json::to_string(&entry) {
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
    fn is_sweetheart_symmetric() {
        let mut r = ResidentRomance::new();
        assert!(r.record_spark("露娜", "奧瑞"));
        assert!(r.is_sweetheart("露娜", "奧瑞"));
        assert!(r.is_sweetheart("奧瑞", "露娜")); // 對稱：順序不影響查詢
    }

    #[test]
    fn record_spark_idempotent() {
        let mut r = ResidentRomance::new();
        assert!(r.record_spark("露娜", "奧瑞")); // 第一次：真正新締結
        assert!(!r.record_spark("露娜", "奧瑞")); // 第二次：已是戀人，不重複觸發
        assert!(!r.record_spark("奧瑞", "露娜")); // 順序調換仍視為同一對
    }

    #[test]
    fn has_partner_blocks_third_party() {
        let mut r = ResidentRomance::new();
        r.record_spark("露娜", "奧瑞");
        assert!(r.has_partner("露娜"));
        assert!(r.has_partner("奧瑞"));
        assert!(!r.has_partner("諾娃")); // 沒締結過的居民不受影響
    }

    #[test]
    fn from_entries_reloads_state() {
        let entries = vec![RomanceEntry { id_a: "奧瑞".into(), id_b: "露娜".into() }];
        let r = ResidentRomance::from_entries(entries);
        assert!(r.is_sweetheart("露娜", "奧瑞")); // 重啟後仍記得（順序與寫入時相反也一致）
        assert!(r.has_partner("露娜"));
    }

    #[test]
    fn from_entries_empty_is_empty() {
        let r = ResidentRomance::from_entries(vec![]);
        assert!(!r.is_sweetheart("露娜", "奧瑞"));
        assert!(!r.has_partner("露娜"));
    }

    #[test]
    fn spark_roll_boundary() {
        assert!(spark_roll(0.0));
        assert!(spark_roll(SPARK_CHANCE - 0.001));
        assert!(!spark_roll(SPARK_CHANCE));
        assert!(!spark_roll(0.999));
    }

    #[test]
    fn to_entries_roundtrip_via_from_entries() {
        let mut r = ResidentRomance::new();
        r.record_spark("露娜", "奧瑞");
        let reloaded = ResidentRomance::from_entries(r.to_entries());
        assert!(reloaded.is_sweetheart("露娜", "奧瑞"));
    }

    #[test]
    fn memory_and_feed_lines_embed_names_and_nonempty() {
        let mem = sweetheart_memory_line("奧瑞");
        assert!(mem.contains("奧瑞"));
        assert!(!mem.is_empty());
        let feed = sweetheart_feed_line("露娜", "奧瑞");
        assert!(feed.contains("露娜"));
        assert!(feed.contains("奧瑞"));
    }

    #[test]
    fn multiple_pairs_independent() {
        let mut r = ResidentRomance::new();
        assert!(r.record_spark("露娜", "奧瑞"));
        assert!(r.has_partner("諾娃") == false);
        // 諾娃/賽勒是另一對，與露娜/奧瑞互不影響。
        assert!(r.record_spark("諾娃", "賽勒"));
        assert!(r.is_sweetheart("諾娃", "賽勒"));
        assert!(r.is_sweetheart("露娜", "奧瑞"));
        assert!(!r.is_sweetheart("露娜", "諾娃"));
    }
}
