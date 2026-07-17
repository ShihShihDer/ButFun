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

/// 「附近」距離平方——兩位老朋友此刻水平距離平方在此門檻內才算「湊在一塊」，供
/// M1「第二火花入口」判定（除了同坐長椅，白天正巧走到彼此身旁也能擦出火花）。
/// 取 6 格（6*6=36），與 `voxel_wedding::PAIR_NEAR_SQ` 同一量級、可調——調大讓火花更易發生、
/// 調小則要更貼近才算數。純常數，實際距離取樣與擲骰在呼叫端。
pub const PAIR_NEAR_SQ: f32 = 36.0;

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

    /// 這位居民的戀人是誰（沒有戀人回 `None`）。供「戀人牽掛」（ROADMAP 852）判斷分開的
    /// 是不是自己的戀人、該去找誰。
    pub fn partner_of(&self, name: &str) -> Option<String> {
        self.pairs.iter().find_map(|(x, y)| {
            if x == name {
                Some(y.clone())
            } else if y == name {
                Some(x.clone())
            } else {
                None
            }
        })
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

    /// 全部戀人對的快照（顯示名對，正規化順序）。供「戀人愛巢」（築巢資格掃描）等需要遍歷所有
    /// 戀人對的呼叫端使用；回傳擁有所有權的 `Vec`，讓呼叫端能短取讀鎖後即釋、鎖外慢慢處理。
    pub fn all_pairs(&self) -> Vec<(String, String)> {
        self.pairs.iter().cloned().collect()
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

/// **第二條火花入口**（M1 成家漏斗拓寬）的獨立擲骰機率——刻意**不複用**上面那條長椅入口的
/// `SPARK_CHANCE`（0.12），兩條入口機率各自可調、互不影響。
///
/// **為何要第二條入口**：既有唯一火花入口（長椅並坐閒聊，`voxel_ws.rs::bench_chat`）要求
/// 「兩居民並肩坐**同一張長椅** + 都已是老朋友 + 過長椅閒聊機率 + 過 `SPARK_CHANCE`」的複合稀有
/// 條件；實測 prod 至今火花數＝0（入口流量趨零），整條成家鏈因此恆早退——無戀人→婚禮掃描恆早退
/// →後代全走單親 fallback。本入口放寬成「兩位老朋友此刻走得夠近（≤ [`PAIR_NEAR_SQ`]）、都醒著、
/// 都閒著」就有機會，不再要求同坐長椅；因觸發面更廣，機率取得**更低**（0.03，遠低於長椅的 0.12）
/// 以免洗版，並疊全村冷卻雙重節流。
pub const NEARBY_SPARK_CHANCE: f32 = 0.03;

/// 第二條火花入口是否擲中心動火花（純函式、roll 由呼叫端提供，確定可測）。
pub fn nearby_spark_roll(roll: f32) -> bool {
    roll < NEARBY_SPARK_CHANCE
}

/// 第二條火花入口締結戀人當下，雙方各自寫進記憶的一句（掛在對方名下）——與長椅版
/// [`sweetheart_memory_line`] 語意區隔：這是「日常走著走著」而非「並肩坐著閒聊」擦出的火花。
pub fn nearby_sweetheart_memory_line(other_name: &str) -> String {
    format!("那天我們正巧走到彼此身旁，四目相接，我心裡忽然為{other_name}漾起漣漪——我們，成了戀人。")
}

/// 第二條火花入口締結戀人當下，城鎮動態牆的一句播報。
pub fn nearby_sweetheart_feed_line(a: &str, b: &str) -> String {
    format!("{a}和{b}在村裡不期而遇，相視間忽然心頭一動——他們，互生情愫，成了戀人。")
}

// ── 戀愛漏斗快照（純函式，供 M1 第二火花入口/觀測用）───────────────────────────

/// 戀愛弧漏斗的一張數字快照：從「夠熟的老朋友對」→「已擦出火花的戀人」→「已成婚」層層收窄，
/// 讓呼叫端（M1 第二火花入口的觸發掃描、除錯輸出、未來 HUD/Feed）一眼看清目前小社會的
/// 感情推進到哪一層、還有多少對老朋友尚未擦出火花可供撮合。純數字、無所有權對名單。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FunnelStats {
    /// 已達「老朋友」門檻的居民對數（漏斗最上層：有機會擦出火花的候選母體）。
    pub eligible_friend_pairs: usize,
    /// 已締結成戀人的對數（`sweethearts`）。
    pub sweethearts: usize,
    /// 已是老朋友、但還沒成為戀人的對數（＝可供第二火花入口撮合的剩餘候選）。
    pub unwed_deep_pairs: usize,
    /// 已成婚的對數（漏斗最底層）。
    pub weddings: usize,
}

/// 掃出戀愛弧的漏斗快照（純函式、收唯讀引用、不取鎖）。
///
/// - `eligible_friend_pairs`：由 `bonds` 的快照裡數出已達 [`crate::voxel_bonds::BondTier::Friend`]
///   的對數（漏斗母體）。
/// - `sweethearts`：直接數 `romance` 的戀人對數。
/// - `unwed_deep_pairs`：老朋友對之中「還不是戀人」的數量——即第二火花入口還能撮合的剩餘候選；
///   已成戀人的老朋友對會被排除（`has_partner`/`is_sweetheart` 語意，不重複撮合）。
/// - `weddings`：直接數 `weddings` 的成婚對數。
///
/// 註：`unwed_deep_pairs` 只算「兩人都尚無戀人」的老朋友對——只要其中一方已另有戀人（一生一位，
/// 見模組說明），這對就不可能再擦出火花，故不列為候選。
pub fn funnel_snapshot(
    bonds: &crate::voxel_bonds::ResidentBonds,
    romance: &ResidentRomance,
    weddings: &crate::voxel_wedding::ResidentWeddings,
) -> FunnelStats {
    use crate::voxel_bonds::FRIEND_VISITS;

    // 從情誼帳本快照數出所有「老朋友」對（visits 已達 Friend 門檻）。
    let friend_pairs: Vec<(String, String)> = bonds
        .to_entries()
        .into_iter()
        .filter(|e| e.visits >= FRIEND_VISITS)
        .map(|e| (e.id_a, e.id_b))
        .collect();

    let eligible_friend_pairs = friend_pairs.len();

    // 老朋友對之中，還能撮合的剩餘候選：兩人都尚無戀人（也就必然還不是彼此的戀人）。
    let unwed_deep_pairs = friend_pairs
        .iter()
        .filter(|(a, b)| !romance.has_partner(a) && !romance.has_partner(b))
        .count();

    FunnelStats {
        eligible_friend_pairs,
        sweethearts: romance.all_pairs().len(),
        unwed_deep_pairs,
        weddings: weddings.all_pairs().len(),
    }
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
    fn partner_of_returns_other_side_or_none() {
        let mut r = ResidentRomance::new();
        r.record_spark("露娜", "奧瑞");
        assert_eq!(r.partner_of("露娜"), Some("奧瑞".to_string()));
        assert_eq!(r.partner_of("奧瑞"), Some("露娜".to_string()));
        assert_eq!(r.partner_of("諾娃"), None);
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

    // ── PAIR_NEAR_SQ / funnel_snapshot ─────────────────────────────────────────

    #[test]
    fn pair_near_sq_is_positive() {
        assert!(PAIR_NEAR_SQ > 0.0, "附近門檻要是正數才有意義");
    }

    /// 用真的 `ResidentBonds` 湊出幾對不同交情層級，方便測 funnel 的母體計數。
    fn bonds_with(pairs: &[(&str, &str, u32)]) -> crate::voxel_bonds::ResidentBonds {
        let entries = pairs.iter().map(|(a, b, v)| crate::voxel_bonds::BondEntry {
            id_a: (*a).to_string(),
            id_b: (*b).to_string(),
            visits: *v,
        });
        crate::voxel_bonds::ResidentBonds::from_entries(entries)
    }

    #[test]
    fn funnel_empty_world_is_all_zero() {
        let bonds = crate::voxel_bonds::ResidentBonds::new();
        let romance = ResidentRomance::new();
        let weddings = crate::voxel_wedding::ResidentWeddings::new();
        let s = funnel_snapshot(&bonds, &romance, &weddings);
        assert_eq!(s, FunnelStats::default());
    }

    #[test]
    fn funnel_counts_only_friend_tier_pairs_as_eligible() {
        use crate::voxel_bonds::FRIEND_VISITS;
        // 露娜/奧瑞達老朋友；諾娃/賽勒只到相識門檻下（不算母體）。
        let bonds = bonds_with(&[("露娜", "奧瑞", FRIEND_VISITS), ("諾娃", "賽勒", 1)]);
        let romance = ResidentRomance::new();
        let weddings = crate::voxel_wedding::ResidentWeddings::new();
        let s = funnel_snapshot(&bonds, &romance, &weddings);
        assert_eq!(s.eligible_friend_pairs, 1, "只有達 Friend 門檻的對算母體");
        assert_eq!(s.sweethearts, 0);
        assert_eq!(s.unwed_deep_pairs, 1, "唯一的老朋友對還沒成戀人");
        assert_eq!(s.weddings, 0);
    }

    #[test]
    fn funnel_sweethearts_excluded_from_unwed_deep() {
        use crate::voxel_bonds::FRIEND_VISITS;
        // 兩對都是老朋友；其中露娜/奧瑞已成戀人。
        let bonds = bonds_with(&[
            ("露娜", "奧瑞", FRIEND_VISITS),
            ("諾娃", "賽勒", FRIEND_VISITS + 5),
        ]);
        let mut romance = ResidentRomance::new();
        romance.record_spark("露娜", "奧瑞");
        let weddings = crate::voxel_wedding::ResidentWeddings::new();
        let s = funnel_snapshot(&bonds, &romance, &weddings);
        assert_eq!(s.eligible_friend_pairs, 2, "兩對都是老朋友，母體 2");
        assert_eq!(s.sweethearts, 1);
        assert_eq!(s.unwed_deep_pairs, 1, "已成戀人的露娜/奧瑞不再列為撮合候選");
        assert_eq!(s.weddings, 0);
    }

    #[test]
    fn funnel_partner_elsewhere_removes_pair_from_unwed_deep() {
        use crate::voxel_bonds::FRIEND_VISITS;
        // 露娜/奧瑞是老朋友，但露娜的戀人其實是別人（凱依）→ 這對不可能再擦火花。
        let bonds = bonds_with(&[("露娜", "奧瑞", FRIEND_VISITS)]);
        let mut romance = ResidentRomance::new();
        romance.record_spark("露娜", "凱依");
        let weddings = crate::voxel_wedding::ResidentWeddings::new();
        let s = funnel_snapshot(&bonds, &romance, &weddings);
        assert_eq!(s.eligible_friend_pairs, 1);
        assert_eq!(s.sweethearts, 1, "露娜/凱依這對戀人被數到");
        assert_eq!(s.unwed_deep_pairs, 0, "露娜已另有戀人，露娜/奧瑞不再是候選");
    }

    #[test]
    fn funnel_counts_weddings() {
        use crate::voxel_bonds::FRIEND_VISITS;
        let bonds = bonds_with(&[("露娜", "奧瑞", FRIEND_VISITS)]);
        let mut romance = ResidentRomance::new();
        romance.record_spark("露娜", "奧瑞");
        let mut weddings = crate::voxel_wedding::ResidentWeddings::new();
        weddings.record_wedding("露娜", "奧瑞");
        let s = funnel_snapshot(&bonds, &romance, &weddings);
        assert_eq!(s.weddings, 1);
        assert_eq!(s.sweethearts, 1);
    }

    // ── 第二火花入口（M1 成家漏斗拓寬）─────────────────────────────────────────

    #[test]
    fn nearby_spark_chance_is_lower_than_bench() {
        // 第二入口觸發面更廣，機率刻意比長椅入口更低以免洗版。
        assert!(NEARBY_SPARK_CHANCE < SPARK_CHANCE, "附近入口機率要比長椅入口更低");
        assert!(NEARBY_SPARK_CHANCE > 0.0, "但仍要是正機率，才可能締結戀人");
    }

    #[test]
    fn nearby_spark_roll_boundary() {
        assert!(nearby_spark_roll(0.0));
        assert!(nearby_spark_roll(NEARBY_SPARK_CHANCE - 0.001));
        assert!(!nearby_spark_roll(NEARBY_SPARK_CHANCE));
        assert!(!nearby_spark_roll(0.999));
    }

    #[test]
    fn nearby_memory_and_feed_lines_embed_names_and_differ_from_bench() {
        let mem = nearby_sweetheart_memory_line("奧瑞");
        assert!(mem.contains("奧瑞") && !mem.is_empty());
        // 與長椅版語意區隔（不同措辭），才不會兩條入口的記憶讀起來一模一樣。
        assert_ne!(mem, sweetheart_memory_line("奧瑞"));
        let feed = nearby_sweetheart_feed_line("露娜", "奧瑞");
        assert!(feed.contains("露娜") && feed.contains("奧瑞"));
        assert_ne!(feed, sweetheart_feed_line("露娜", "奧瑞"));
    }
}
