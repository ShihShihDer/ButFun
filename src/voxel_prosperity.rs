//! 乙太方界·領地繁榮 v1（自主提案切片，ROADMAP 979；接續 963/966/967 領地保護系列）。
//!
//! **真缺口**：領地保護（963 起）讓玩家的家第一次「別人動不得」——但保護之外，那塊地
//! 從立牌那一刻起就靜止不動：既不會長大，也不會產出任何東西，跟一塊空地唯一的差別只是
//! 「插了一支牌、多了一圈看不見的結界」。reviewer 在退回 PR #1265（同模板第三次換皮）時
//! 親口點名這是仍未回應的真缺口：「#963 領地只擋得住鎬子、不會長大或產出」，並定下下一輪
//! 三條判準——①有狀態、會累積（持久化進度，非重連歸零的 additive bool）②多人／人與居民
//! 之間產生新因果（別人的行為改變你的處境，不只是「別人看得到你」）③不是「合成物品＋熱鍵
//! toggle」。本刀直接回應這三條。
//!
//! **做法**：領地立牌後，**只要那塊「家」牌還在，繁榮度就隨真實時間持續累積**——不必
//! 玩家做任何動作、也不必在線，是這個世界第一塊「你不在的時候也在悄悄變好」的地方。
//! **信任名單裡的朋友（966 `TrustStore`）站在你的領地附近時，繁榮成長會加快**——別人的
//! 行為第一次真的改變你的處境（①②同時成立：累積的是持久經驗值，加成來自朋友的實際造訪，
//! 不是「別人看得到你」這種被動旁觀）。繁榮練到一定等級起，領地**被動產出乙太幣**（沿用
//! 既有 [`crate::voxel_craft::COIN_ID`]，零新物品）、累積進專屬庫存（設上限，不放著不管
//! 也不會無限暴漲）；領主本人回到自家附近時自動收進背包。繁榮練到更高等級起，路過的居民
//! 會被吸引駐足由衷讚賞這戶人家的用心經營，記進心裡——世界不只是「你自己看得到地變好」，
//! 居民也真的注意到了。**全程零新熱鍵、零新合成物品**（③成立）。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **領地保護 v1/v2/v4（`voxel_landclaim`，963/966/967）**＝規則層「誰能動這塊地」，
//!   全有或全無、不隨時間變化；本刀＝領地**本身**隨時間長出的一條連續進程，兩者共用同一塊
//!   「家」牌判定 [`crate::voxel_landclaim::is_home_sign`]，但一個是保護規則、一個是成長曲線。
//! - **玩家熟練度（`voxel_mastery`，接續 724）**＝**你這個人**練得多深（採集/耕種/垂釣，
//!   隨你的動作累積）；本刀＝**你的地**長得多繁榮（隨真實時間＋朋友來訪累積，與你本人做了
//!   什麼動作無關）——熟練度累積的是個人技藝，繁榮累積的是地方本身，一縱一橫互不重疊，
//!   但沿用同一套「append-only delta、重啟逐行加總還原」的持久化範式（比照 `voxel_mastery`）。
//! - **建造讚賞／種田讚賞（773/`voxel_farm_admire`）**＝居民對**你正在做的連續動作**由衷
//!   佩服（觸發於動作瞬間、事後就過去了）；本刀的居民駐足讚賞＝對**領地本身這個持久成果**
//!   的反應（不看你此刻在不在做什麼，只看這塊地繁榮到了什麼程度），觸發源、判定邏輯、
//!   台詞語氣皆不同，各自獨立冷卻互不影響。
//! - **976/977/978 系列（獨輪車／手風琴／被退回的披風）**＝「合成一件物品→按一個熱鍵
//!   翻一個布林」模板；本刀**沒有任何新熱鍵、沒有任何新合成物品**，是一條隨真實時間＋
//!   他人行為推進的背景進程，reviewer 停止線之後的第一刀正面回應。
//!
//! **成本紀律**：零 LLM（成長/產出/讚賞全確定性）、零 migration（append-only JSONL，比照
//! `voxel_mastery` 範式）、零新美術（產出沿用既有乙太幣 `COIN_ID`）、成長節拍併入既有 15
//! 秒全域 ticker、每 [`PROSPERITY_TICK_EVERY_N`] 拍才真的算一次（5 分鐘一次，非熱路徑）、
//! 領地數量與玩家數同量級，全掃成本可忽略。
//!
//! **濫用防護**：本刀**不收任何玩家自由輸入**；產出上限 [`MAX_STOCKPILE`] 封頂，離線再久
//! 回來也只有一筆小額進帳，不值得刻意掛機；信任加成只認 [`crate::voxel_landclaim::TrustStore`]
//! 裡**由領地主人親手加入**的帳號（需先在 [`crate::voxel_landclaim::TRUST_REACH`] 內邀請，
//! 見 966），小號互掛信任也只換來每 5 分鐘一份小額加速，價值遠低於自動化的成本；讚賞台詞
//! 全為固定模板、只嵌顯示名，無注入／NSFW／洗版風險；領地歸屬全由伺服器內部（`owner_key`＝
//! 帳號 email）判定，玩家無從偽造。
//!
//! **純邏輯層**：等級曲線、成長量、產出量、台詞、`ProsperityStore` 全是確定性同步資料結構
//! ／函式，零 IO（除 append/load 兩顆持久化函式）、零鎖、零 async，窮舉可測。鎖 / 距離掃描 /
//! 廣播全留在 `voxel_ws.rs`（短鎖即釋、循序不巢狀，守死鎖鐵律）。

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};

use serde::{Deserialize, Serialize};

/// 持久化路徑（`data/` 已 gitignore，執行期產生）。
pub const PROSPERITY_PATH: &str = "data/voxel_prosperity.jsonl";

/// 成長節拍併入既有 15 秒全域 ticker 的第幾拍才真的算一次（20 拍 × 15 秒 = 5 分鐘一次），
/// 避免每 15 秒就寫一行 JSONL、也讓「累積」這件事有真實時間的份量、值得專程回來看看。
pub const PROSPERITY_TICK_EVERY_N: u32 = 20;

/// 每次成長節拍的基礎繁榮經驗值——領地只要還在，這份最低限度的成長永遠都有，不必任何人在場。
pub const BASE_GROWTH_XP: u32 = 1;

/// 節拍當下若有被信任的朋友（[`crate::voxel_landclaim::TrustStore`]）站在領地附近，
/// 額外加成的繁榮經驗值——朋友的造訪讓你的家長得更快，這是本刀的核心多人因果。
pub const TRUSTED_VISIT_BONUS_XP: u32 = 2;

/// 每級所需經驗（累積制、線性，比照 [`crate::voxel_mastery::LEVEL_XP_STEP`] 同款設計）。
pub const LEVEL_XP_STEP: u32 = 6;

/// 頂級封頂——經驗仍可累積但不再升級，避免數字無限跑。
pub const MAX_LEVEL: u32 = 8;

/// 練到這一級起，領地開始被動產出乙太幣。
pub const PRODUCE_UNLOCK_LEVEL: u32 = 3;

/// 每次達門檻的成長節拍，領地產出多少乙太幣。
pub const PRODUCE_PER_TICK: u32 = 1;

/// 未收取的乙太幣庫存上限——離線再久回來也只有一筆小額進帳，不必無限記帳、也天然防掛機屯幣。
pub const MAX_STOCKPILE: u32 = 15;

/// 領地繁榮練到這一級起，路過的居民會被吸引駐足讚賞。
pub const ADMIRE_UNLOCK_LEVEL: u32 = 4;

/// 居民要多靠近領地中心，才會注意到它的繁榮（世界方塊距離，水平 XZ 平面）。
/// 比 [`crate::voxel_landclaim::CLAIM_RADIUS`] 稍大——路過周邊也看得出這戶人家的用心經營，
/// 不必貼著牆才看得見。
pub const ADMIRE_RADIUS: f32 = 8.0;

/// 同一位居民對同一塊領地的讚賞冷卻（秒），讓讚賞稀有有份量、不洗版泡泡與動態牆。
pub const ADMIRE_COOLDOWN_SECS: u64 = 300;

/// 依總繁榮經驗算出目前等級（純函式、確定性、封頂 [`MAX_LEVEL`]）。
pub fn level_for_xp(xp: u32) -> u32 {
    (xp / LEVEL_XP_STEP).min(MAX_LEVEL)
}

/// 這次成長節拍該加多少繁榮經驗——`trusted_nearby` 由呼叫端算好帶入（節拍當下領地附近
/// 是否有被信任的朋友）。
pub fn growth_this_tick(trusted_nearby: bool) -> u32 {
    BASE_GROWTH_XP + if trusted_nearby { TRUSTED_VISIT_BONUS_XP } else { 0 }
}

/// 這個等級的領地，這次節拍該產出多少乙太幣（未達門檻回 0）。
pub fn production_yield(level: u32) -> u32 {
    if level >= PRODUCE_UNLOCK_LEVEL {
        PRODUCE_PER_TICK
    } else {
        0
    }
}

/// 封頂加法：加到上限就停，不溢出、不無限累積。
pub fn capped_add(current: u32, delta: u32, cap: u32) -> u32 {
    current.saturating_add(delta).min(cap)
}

/// 稱號（純展示，Feed/提示句用）。
pub fn title_for_level(level: u32) -> &'static str {
    match level {
        0..=1 => "剛落腳",
        2..=3 => "漸有生氣",
        4..=5 => "欣欣向榮",
        6..=7 => "門庭若市",
        _ => "傳頌一方",
    }
}

/// 升級 Feed 明細（面向玩家字串，i18n 友善）。
pub fn levelup_feed_detail(owner_name: &str, level: u32) -> String {
    let title = title_for_level(level);
    format!("{owner_name}的家繁榮度升到 Lv.{level}（{title}）——用心經營，看得見的成長。")
}

/// 領主回到領地附近、自動收進背包時的提示句（`{n}` 換成收取數量）。
pub fn collect_line(amount: u32) -> String {
    format!("🏡 你的家最近很熱鬧，攢下了 {amount} 枚乙太幣，順手收進了背包。")
}

/// 居民駐足讚賞的台詞池（確定性選句，由呼叫端傳 `pick` 索引）。
const ADMIRE_LINES: [&str; 5] = [
    "這戶人家最近越來越熱鬧了呢。",
    "每次經過這裡，都覺得又更有生氣了一點。",
    "這麼用心經營的家，難怪這麼多人喜歡來坐坐。",
    "這裡的一磚一瓦，看得出主人很上心。",
    "住在這附近，感覺日子都跟著暖了起來。",
];

/// 依 `pick` 索引挑一句居民駐足讚賞的台詞。
pub fn admire_say_line(pick: usize) -> &'static str {
    ADMIRE_LINES[pick % ADMIRE_LINES.len()]
}

/// 讚賞記進居民心裡的記憶摘要。
pub fn admire_memory_line(owner_name: &str) -> String {
    format!("由衷佩服{owner_name}把家打理得欣欣向榮，路過都忍不住多看兩眼。")
}

/// 讚賞的 Feed 明細。
pub fn admire_feed_detail(resident_name: &str, owner_name: &str) -> String {
    format!("{resident_name}路過{owner_name}的家，被那份欣欣向榮吸引，停下腳步多看了兩眼。")
}

// ── 持久化格式（append-only：一行一筆增量，重啟時逐行加總還原，比照 voxel_mastery）────────

/// 一筆繁榮事件：`xp_delta` 是這次節拍加的經驗（0＝純收取事件）；`produced_delta` 是這次
/// 節拍新產出、進了庫存的乙太幣（0＝未達產出門檻或這次沒觸發）；`collected_delta` 是這次
/// 事件被領主收走的乙太幣（0＝純成長事件）。三個欄位彼此獨立累加，重播順序無關緊要。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProsperityEntry {
    pub owner_key: String,
    #[serde(default)]
    pub xp_delta: u32,
    #[serde(default)]
    pub produced_delta: u32,
    #[serde(default)]
    pub collected_delta: u32,
}

/// 全服領地繁榮帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct ProsperityStore {
    xp: HashMap<String, u32>,
    produced: HashMap<String, u32>,
    collected: HashMap<String, u32>,
}

impl ProsperityStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 由持久化紀錄重建（純加總，任意順序重播結果一致）。
    pub fn from_entries(entries: impl IntoIterator<Item = ProsperityEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            *s.xp.entry(e.owner_key.clone()).or_insert(0) += e.xp_delta;
            *s.produced.entry(e.owner_key.clone()).or_insert(0) += e.produced_delta;
            *s.collected.entry(e.owner_key).or_insert(0) += e.collected_delta;
        }
        s
    }

    /// 這塊領地目前累積的繁榮經驗（未紀錄過回 0）。
    pub fn xp_for(&self, owner_key: &str) -> u32 {
        *self.xp.get(owner_key).unwrap_or(&0)
    }

    /// 這塊領地目前的繁榮等級。
    pub fn level_for(&self, owner_key: &str) -> u32 {
        level_for_xp(self.xp_for(owner_key))
    }

    /// 這塊領地目前未收取的乙太幣庫存（已產出減已收取）。
    pub fn stockpile_for(&self, owner_key: &str) -> u32 {
        let produced = *self.produced.get(owner_key).unwrap_or(&0);
        let collected = *self.collected.get(owner_key).unwrap_or(&0);
        produced.saturating_sub(collected)
    }

    /// 一次成長節拍：加經驗，若達產出門檻且庫存未滿則順帶產出（封頂 [`MAX_STOCKPILE`]）。
    /// 回傳 `(供 append 的事件, 這次是否剛好升級, 新等級)`。
    pub fn grow(&mut self, owner_key: &str, xp_delta: u32) -> (ProsperityEntry, bool, u32) {
        let before_level = self.level_for(owner_key);
        let after_xp = self.xp_for(owner_key).saturating_add(xp_delta);
        self.xp.insert(owner_key.to_string(), after_xp);
        let after_level = level_for_xp(after_xp);

        let current_stock = self.stockpile_for(owner_key);
        let desired = production_yield(after_level);
        let produced_delta = desired.min(MAX_STOCKPILE.saturating_sub(current_stock));
        if produced_delta > 0 {
            *self.produced.entry(owner_key.to_string()).or_insert(0) += produced_delta;
        }

        let entry = ProsperityEntry {
            owner_key: owner_key.to_string(),
            xp_delta,
            produced_delta,
            collected_delta: 0,
        };
        (entry, after_level > before_level, after_level)
    }

    /// 領主回到領地附近，把庫存全數收走。庫存為 0 時回 `None`（呼叫端不必 append 空事件）。
    pub fn collect(&mut self, owner_key: &str) -> Option<(u32, ProsperityEntry)> {
        let amount = self.stockpile_for(owner_key);
        if amount == 0 {
            return None;
        }
        *self.collected.entry(owner_key.to_string()).or_insert(0) += amount;
        let entry = ProsperityEntry {
            owner_key: owner_key.to_string(),
            xp_delta: 0,
            produced_delta: 0,
            collected_delta: amount,
        };
        Some((amount, entry))
    }
}

fn write_line(path: &str, line: &str) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = fs::create_dir_all(parent);
    }
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("無法寫入領地繁榮記錄 {path}: {e}"),
    }
}

/// append 一筆繁榮事件（IO，呼叫端須在鎖外呼叫）。
pub fn append_prosperity(entry: &ProsperityEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(PROSPERITY_PATH, &line);
    }
}

/// 載回所有繁榮事件（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_prosperity() -> Vec<ProsperityEntry> {
    let Ok(f) = fs::File::open(PROSPERITY_PATH) else {
        return Vec::new();
    };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                None
            } else {
                serde_json::from_str::<ProsperityEntry>(l).ok()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 純函式 ──────────────────────────────────────────────────────────────────

    #[test]
    fn level_for_xp_steps_and_caps() {
        assert_eq!(level_for_xp(0), 0);
        assert_eq!(level_for_xp(LEVEL_XP_STEP - 1), 0);
        assert_eq!(level_for_xp(LEVEL_XP_STEP), 1);
        assert_eq!(level_for_xp(LEVEL_XP_STEP * MAX_LEVEL), MAX_LEVEL);
        assert_eq!(level_for_xp(999_999), MAX_LEVEL, "封頂，不無限跑");
    }

    #[test]
    fn growth_this_tick_base_and_bonus() {
        assert_eq!(growth_this_tick(false), BASE_GROWTH_XP);
        assert_eq!(growth_this_tick(true), BASE_GROWTH_XP + TRUSTED_VISIT_BONUS_XP);
    }

    #[test]
    fn production_yield_gated_by_unlock_level() {
        assert_eq!(production_yield(0), 0);
        assert_eq!(production_yield(PRODUCE_UNLOCK_LEVEL - 1), 0);
        assert_eq!(production_yield(PRODUCE_UNLOCK_LEVEL), PRODUCE_PER_TICK);
        assert_eq!(production_yield(MAX_LEVEL), PRODUCE_PER_TICK);
    }

    #[test]
    fn capped_add_normal_and_capped() {
        assert_eq!(capped_add(0, 5, 15), 5);
        assert_eq!(capped_add(10, 10, 15), 15, "加到上限就停");
        assert_eq!(capped_add(15, 5, 15), 15, "已在上限不再加");
    }

    #[test]
    fn title_for_level_progression() {
        assert_eq!(title_for_level(0), "剛落腳");
        assert_eq!(title_for_level(1), "剛落腳");
        assert_eq!(title_for_level(2), "漸有生氣");
        assert_eq!(title_for_level(4), "欣欣向榮");
        assert_eq!(title_for_level(6), "門庭若市");
        assert_eq!(title_for_level(MAX_LEVEL), "傳頌一方");
    }

    #[test]
    fn levelup_feed_detail_contains_owner_and_level() {
        let line = levelup_feed_detail("阿星", 4);
        assert!(line.contains("阿星"));
        assert!(line.contains("Lv.4"));
    }

    #[test]
    fn collect_line_contains_amount() {
        assert!(collect_line(7).contains('7'));
    }

    #[test]
    fn admire_say_line_wraps_and_non_empty() {
        for pick in 0..10 {
            let line = admire_say_line(pick);
            assert!(!line.is_empty());
        }
        assert_eq!(admire_say_line(0), admire_say_line(ADMIRE_LINES.len()));
    }

    #[test]
    fn admire_memory_and_feed_contain_names() {
        assert!(admire_memory_line("露娜").contains("露娜"));
        let feed = admire_feed_detail("露娜", "阿星");
        assert!(feed.contains("露娜") && feed.contains("阿星"));
    }

    // ── ProsperityStore ────────────────────────────────────────────────────────

    #[test]
    fn store_new_is_empty() {
        let store = ProsperityStore::new();
        assert_eq!(store.xp_for("a@example.com"), 0);
        assert_eq!(store.level_for("a@example.com"), 0);
        assert_eq!(store.stockpile_for("a@example.com"), 0);
    }

    #[test]
    fn grow_accumulates_xp_and_reports_levelup() {
        let mut store = ProsperityStore::new();
        for i in 0..LEVEL_XP_STEP - 1 {
            let (_, leveled, level) = store.grow("a@example.com", 1);
            assert!(!leveled, "第 {i} 次不該升級");
            assert_eq!(level, 0);
        }
        let (_, leveled, level) = store.grow("a@example.com", 1);
        assert!(leveled, "剛好跨過門檻該升級");
        assert_eq!(level, 1);
        assert_eq!(store.xp_for("a@example.com"), LEVEL_XP_STEP);
    }

    #[test]
    fn grow_produces_only_after_unlock_level() {
        let mut store = ProsperityStore::new();
        // 練到剛好 PRODUCE_UNLOCK_LEVEL 之前都不產出（最後一拍之前）。
        let ticks_to_unlock = LEVEL_XP_STEP * PRODUCE_UNLOCK_LEVEL;
        for _ in 0..ticks_to_unlock - 1 {
            store.grow("a@example.com", 1);
        }
        assert_eq!(store.level_for("a@example.com"), PRODUCE_UNLOCK_LEVEL - 1);
        assert_eq!(store.stockpile_for("a@example.com"), 0, "還沒到門檻，不產出");
        // 剛好跨過門檻的這一拍，等級與產出同時生效。
        let (entry, _, _) = store.grow("a@example.com", 1);
        assert_eq!(store.level_for("a@example.com"), PRODUCE_UNLOCK_LEVEL);
        assert_eq!(entry.produced_delta, PRODUCE_PER_TICK);
        assert_eq!(store.stockpile_for("a@example.com"), PRODUCE_PER_TICK);
    }

    #[test]
    fn grow_caps_stockpile_at_max() {
        let mut store = ProsperityStore::new();
        // 先練到解鎖產出的等級。
        for _ in 0..(LEVEL_XP_STEP * PRODUCE_UNLOCK_LEVEL) {
            store.grow("a@example.com", 1);
        }
        // 之後每拍都在解鎖等級以上、狂刷產出，庫存不該超過上限。
        for _ in 0..(MAX_STOCKPILE + 10) {
            store.grow("a@example.com", 0);
        }
        assert_eq!(store.stockpile_for("a@example.com"), MAX_STOCKPILE);
    }

    #[test]
    fn collect_drains_stockpile_and_second_collect_is_none() {
        let mut store = ProsperityStore::new();
        for _ in 0..(LEVEL_XP_STEP * PRODUCE_UNLOCK_LEVEL) {
            store.grow("a@example.com", 1);
        }
        store.grow("a@example.com", 0); // 觸發一次產出
        let stock = store.stockpile_for("a@example.com");
        assert!(stock > 0);
        let (amount, entry) = store.collect("a@example.com").expect("庫存非空該收到");
        assert_eq!(amount, stock);
        assert_eq!(entry.collected_delta, stock);
        assert_eq!(store.stockpile_for("a@example.com"), 0);
        assert!(store.collect("a@example.com").is_none(), "收空之後再收該回 None");
    }

    #[test]
    fn grow_is_scoped_per_owner() {
        let mut store = ProsperityStore::new();
        store.grow("a@example.com", 5);
        assert_eq!(store.xp_for("a@example.com"), 5);
        assert_eq!(store.xp_for("b@example.com"), 0, "不同領地互不影響");
    }

    #[test]
    fn from_entries_replay_matches_direct_grow_and_collect() {
        let mut store = ProsperityStore::new();
        let mut entries = Vec::new();
        for _ in 0..(LEVEL_XP_STEP * PRODUCE_UNLOCK_LEVEL) {
            let (e, _, _) = store.grow("a@example.com", 1);
            entries.push(e);
        }
        let (e, _, _) = store.grow("a@example.com", 0);
        entries.push(e);
        let (_, e2) = store.collect("a@example.com").unwrap();
        entries.push(e2);

        // 重播順序打亂也該得到一致結果（純加總、與順序無關）。
        let mut shuffled = entries.clone();
        shuffled.reverse();
        let replayed = ProsperityStore::from_entries(shuffled);
        assert_eq!(replayed.xp_for("a@example.com"), store.xp_for("a@example.com"));
        assert_eq!(replayed.stockpile_for("a@example.com"), store.stockpile_for("a@example.com"));
        assert_eq!(replayed.level_for("a@example.com"), store.level_for("a@example.com"));
    }
}
