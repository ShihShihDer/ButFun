//! 乙太方界·工具耐久＋鐵匠代客保養 v1（自主提案切片；接續 790「工欲善其事」）。
//!
//! **真缺口**：鎬／斧／鏟自合成台 v1（687/689/690）就能造，工欲善其事（790）更讓「對的工具」
//! 有機率多收一份材料——但工具本身，做出來的那一刻起就**永遠堪用**，全專案 39 刀自主提案
//! 切片裡唯一完全空白的資源經濟軸線：赤手空拳與挖了一千次石頭的鐵鎬，對世界而言毫無分別。
//!
//! **做法**：真實使用（用對的工具挖對應的天然方塊）會讓工具**慢慢磨損**；磨損到門檻後，
//! 790 的「多收一份」加成暫時失靈——工具還能用，只是不再值得帶著的那份小確幸沒了。玩家帶著
//! 磨鈍的工具走近任一位居民，花點材料／乙太幣就能請她保養——**常去找同一位居民修，她會漸漸
//! 把你當老主顧**，保養費隨拜訪次數累積打折（封頂）。挖礦／砍樹／鋤地是這個世界幾乎每次上線
//! 都會做的核心迴圈，磨損天生是全遊戲觸發頻率最高的一條累積線，不是稀有偶發事件。
//!
//! **四項判準逐條回應（reviewer 對 976~980 定下的方向）**：①**有累積狀態**——耐久隨每次真實
//! 使用增長、老主顧關係隨每次保養增長，皆持久化、非重連歸零的一次性事件。②**多人/玩家↔居民
//! 新因果**——保養是玩家主動發起、需要居民服務的**重複往來**，你越常找同一人修就越划算，這是
//! 「持續關係」而非一次性互動。③**非「合成物品＋熱鍵」**——零新物品、零新熱鍵，玩法是既有
//! 挖／砍／鋤地動作多了一個真實副作用，維修沿用既有「走近居民互動」介面。④**玩家日常常見**——
//! 觸發頻率等同挖礦本身，不會有「980 觸發條件太窄」的問題。
//!
//! **與既有系統 razor-sharp 區隔**：
//! - **工欲善其事（790，`voxel_tool`）**＝手持對的工具，有機率多收一份材料（產出加成）；
//!   本刀＝那把工具本身會不會隨使用磨損（工具的狀態），兩者互補——磨損到門檻後 790 的加成
//!   暫時失靈，修好即恢復，是同一份「工具值不值得帶」拼圖的另一半。
//! - **玩家熟練度（`voxel_mastery`，724）**＝**你這個人**練得多深，正向成長曲線；本刀＝
//!   **你的工具**越用越損，負向消耗曲線，服務對象都不同。
//! - **領地繁榮（`voxel_prosperity`，979）**＝**你的地**隨真實時間＋朋友造訪自然變好；本刀＝
//!   **你的工具**隨你自己的真實動作耗損，累積方向相反（一長一消），觸發源也不同（時間 vs 動作）。
//! - **居民贈禮（`voxel_gift`，660）**＝單向送禮換好感，不求對價；**居民交易（`voxel_trade`，
//!   670/874/958）**＝以物易物／付幣換居民的特產；本刀＝**雙向服務交易**（付材料/幣、換回
//!   修好的工具），且核心計量是「常去找同一人修的次數」而非好感度或物價。
//! - **849（居民教你獨門配方）**＝好感度門檻觸發的一次性驚喜；本刀＝**沒有好感度門檻**、
//!   每次維修都收費、是重複的服務關係而非教學，任一位居民都能提供（不必特定交情）。
//!
//! **成本紀律**：零 LLM（磨損/折扣/文案全確定性純函式）、零新美術（沿用既有工具/乙太幣圖示）、
//! 零協議破壞（新訊息型別，additive）、FPS 零影響（磨損只在 `Break` 成功時計算一次，維修只在
//! 玩家主動發起時計算一次，非每幀/每 tick 成本）。**零破壞性後果**：耐久見底只是「效率加成
//! 暫時失靈」，工具**絕不會消失或損毀**——修理是「讓它變好用」，不是「防止它壞掉」。
//!
//! **濫用防護**：不收任何玩家自由文字輸入；不觸發 LLM／不開對外端點；工具持有／磨損狀態／
//! 是否真的「該修了」皆由伺服器權威判定（`voxel_ws.rs` 查真實背包+真實耐久帳本，玩家無法
//! 偽報「我的工具壞了」來騙修理特效）；老主顧折扣封頂（[`MAX_REPAIR_DISCOUNT_PCT`]），
//! 修理費用有下限（絕不到 0），無法無限刷折扣套利。
//!
//! **持久化守衛（append-only 不無限膨脹的鐵律）**：踩過「拆除==放置即恆真無限 churn」（示範
//! 地花圃事故）與「領地繁榮滿級仍寫入」（#1267）兩次教訓——寫入的守衛必須是「這一拍是否真的
//! 改變了可觀察狀態」，不能是「該不該跑這一拍」。本模組耐久／老主顧次數皆已封頂
//! （[`MAX_DURABILITY`]／[`VISITS_FOR_MAX_DISCOUNT`]），一旦封頂，`add_wear`／`record_visit`
//! 回傳 `None`（真實 delta 為 0），呼叫端據此略過 append，帳本增長天然有終點。
//!
//! **純邏輯層**：耐久／折扣曲線、`WearStore`／`RepairLedger`、面向玩家文案全是確定性函式與
//! 同步資料結構，零 IO（除 append/load 四顆持久化函式）、零鎖、零 async，窮舉可測。鎖／背包
//! 讀寫／廣播全留在 `voxel_ws.rs`（短鎖即釋、循序不巢狀，守死鎖鐵律）。

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};

use serde::{Deserialize, Serialize};

/// 走近居民才能請她保養（世界方塊距離，水平 XZ 平面）。比照 [`crate::voxel_gift::GIFT_REACH`]
/// 同款距離，需要走近才能遞出壞掉的工具。
pub const REPAIR_REACH: f32 = 5.0;

/// 工具的滿耐久值（純計量單位，非顯示用百分比）。
pub const MAX_DURABILITY: u32 = 100;

/// 每次「手持對的工具、採集對應的天然方塊」磨損多少（見 [`crate::voxel_tool::tool_tier`] /
/// [`crate::voxel_tool::block_tool_kind`] 判定是否算「真的用對地方」）。100 / 2 = 50 次見底，
/// 貼近一趟正常挖礦/砍樹的節奏——磨損天生比任何其他自主提案切片都常見。
pub const WEAR_PER_USE: u32 = 2;

/// 剩餘耐久 ≤ 這個百分比，就算「該修了」（790 的多收一份加成暫時失靈，需保養才恢復）。
pub const WORN_THRESHOLD_PCT: u32 = 25;

/// 基礎保養費（乙太幣，沿用既有 [`crate::voxel_craft::COIN_ID`]，零新物品）。
pub const BASE_REPAIR_COST: u32 = 5;

/// 每次找同一位居民保養，「老主顧」折扣累加多少個百分點。
pub const DISCOUNT_PER_VISIT_PCT: u32 = 5;

/// 折扣封頂（絕不到全免——修理費用永遠 ≥ 1 枚，無法無限刷折扣套利）。
pub const MAX_REPAIR_DISCOUNT_PCT: u32 = 60;

/// 拜訪次數達到這個數字後，折扣不再增加（帳本增長天然有終點）。
pub const VISITS_FOR_MAX_DISCOUNT: u32 = MAX_REPAIR_DISCOUNT_PCT / DISCOUNT_PER_VISIT_PCT;

/// 依已耗損量算目前剩餘耐久百分比（0~100，`worn` 超過 `max` 一律視為 0%）。
/// 純函式：`worn=0` → 100%（全新）；`worn>=max` → 0%（磨到見底）。
pub fn durability_pct(worn: u32, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    let worn = worn.min(max);
    100 - (worn * 100 / max)
}

/// 剩餘耐久是否已低到「該修了」（790 加成暫時失靈的門檻）。
pub fn is_worn_out(worn: u32) -> bool {
    durability_pct(worn, MAX_DURABILITY) <= WORN_THRESHOLD_PCT
}

/// 依拜訪次數算老主顧折扣（百分比，封頂 [`MAX_REPAIR_DISCOUNT_PCT`]）。
pub fn repair_discount_pct(visits: u32) -> u32 {
    (visits.min(VISITS_FOR_MAX_DISCOUNT) * DISCOUNT_PER_VISIT_PCT).min(MAX_REPAIR_DISCOUNT_PCT)
}

/// 依拜訪次數算這次實際要付的乙太幣（折扣後，永遠 ≥ 1，絕不到全免）。
pub fn repair_cost(visits: u32) -> u32 {
    let discount = repair_discount_pct(visits);
    (BASE_REPAIR_COST * (100 - discount) / 100).max(1)
}

// ── 面向玩家文案（集中於此，i18n 友善；繁中）───────────────────────────────────

/// 工具剛磨到「該修了」那一刻，單播提醒（比照 [`crate::voxel_tool`] 的 `tool_bonus` 提示慣例，
/// 只在跨過門檻的那一刻提醒一次，不逐次採集洗版）。
pub fn worn_out_line(tool_name: &str) -> String {
    format!("你的{tool_name}用得有點鈍了，多收一份材料的手感也跟著弱了——去找村裡的居民保養一下吧。")
}

/// 第一次找這位居民保養這把工具（`cost` 為實付乙太幣）。
pub fn first_repair_say_line(cost: u32) -> String {
    format!("讓我看看……交給我吧，{cost}枚乙太幣就好。")
}

/// 已是老主顧（`visits` 為保養前的既有拜訪次數，`cost` 為這次實付、已含折扣）。
pub fn regular_repair_say_line(visits: u32, cost: u32) -> String {
    format!("又是你啊，老主顧了——這次只收你{cost}枚乙太幣（第{visits}次照顧生意的折扣）。")
}

/// 保養完成，回應玩家的提示句。
pub fn repair_done_line(tool_name: &str, resident_name: &str) -> String {
    format!("{resident_name}把你的{tool_name}擦拭、敲打了一番，還你一把煥然一新的{tool_name}。")
}

/// 記進居民心裡的記憶（她記得又幫你修過一次）。
pub fn repair_memory_line(player_name: &str, tool_name: &str) -> String {
    format!("幫{player_name}保養了她那把用舊的{tool_name}，希望她用得順手。")
}

/// 城鎮動態牆明細。
pub fn repair_feed_line(resident_name: &str, player_name: &str, tool_name: &str) -> String {
    format!("{resident_name}替{player_name}保養了一把磨鈍的{tool_name}，讓它重新趁手。")
}

// ── 持久化格式（append-only：一行一筆增量，重啟逐行加總還原，比照 `voxel_prosperity`）──────

/// 工具耐久持久化路徑（`data/` 已 gitignore，執行期產生）。
pub const WEAR_PATH: &str = "data/voxel_tool_wear.jsonl";

/// 老主顧拜訪次數持久化路徑。
pub const REPAIR_VISIT_PATH: &str = "data/voxel_tool_repair.jsonl";

/// 一筆磨損事件：`wear_delta` 是這次真的新增了多少磨損量（`repair_delta` 為修好時的歸零量，
/// 兩者互斥，皆非負，重播順序無關緊要）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WearEntry {
    pub player_key: String,
    pub tool_id: u8,
    #[serde(default)]
    pub wear_delta: u32,
    #[serde(default)]
    pub repair_delta: u32,
}

/// 一筆老主顧拜訪事件：`visit_delta` 是這次新增的拜訪次數（恆為 0 或 1，已封頂時為 0）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepairVisitEntry {
    pub player_key: String,
    pub resident_key: String,
    pub visit_delta: u32,
}

/// 全服工具耐久帳本（`(玩家, 工具id)` → 目前已磨損量，封頂 [`MAX_DURABILITY`]）。
#[derive(Default, Debug)]
pub struct WearStore {
    worn: HashMap<(String, u8), u32>,
}

impl WearStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 由持久化紀錄重建（純加總後夾在 `[0, MAX_DURABILITY]`，任意順序重播結果一致）。
    pub fn from_entries(entries: impl IntoIterator<Item = WearEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            let key = (e.player_key, e.tool_id);
            let cur = *s.worn.get(&key).unwrap_or(&0);
            let next = cur
                .saturating_add(e.wear_delta)
                .saturating_sub(e.repair_delta)
                .min(MAX_DURABILITY);
            s.worn.insert(key, next);
        }
        s
    }

    /// 這把工具目前已磨損多少（未紀錄過回 0＝全新）。
    pub fn worn_of(&self, player_key: &str, tool_id: u8) -> u32 {
        *self.worn.get(&(player_key.to_string(), tool_id)).unwrap_or(&0)
    }

    /// 累加一次使用的磨損。已封頂（見底）時回 `None`（真實 delta 為 0，呼叫端據此略過
    /// append，避免帳本無限膨脹——比照 `voxel_prosperity::grow` 滿級守衛同款手法）。
    pub fn add_wear(&mut self, player_key: &str, tool_id: u8) -> Option<WearEntry> {
        let key = (player_key.to_string(), tool_id);
        let before = *self.worn.get(&key).unwrap_or(&0);
        if before >= MAX_DURABILITY {
            return None;
        }
        let after = before.saturating_add(WEAR_PER_USE).min(MAX_DURABILITY);
        let delta = after - before;
        self.worn.insert(key, after);
        Some(WearEntry {
            player_key: player_key.to_string(),
            tool_id,
            wear_delta: delta,
            repair_delta: 0,
        })
    }

    /// 保養完成：耐久歸零（全新狀態）。已經是全新（`worn==0`）時回 `None`（沒有真的改變
    /// 任何可觀察狀態，呼叫端據此略過 append）。
    pub fn repair(&mut self, player_key: &str, tool_id: u8) -> Option<WearEntry> {
        let key = (player_key.to_string(), tool_id);
        let before = *self.worn.get(&key).unwrap_or(&0);
        if before == 0 {
            return None;
        }
        self.worn.insert(key, 0);
        Some(WearEntry {
            player_key: player_key.to_string(),
            tool_id,
            wear_delta: 0,
            repair_delta: before,
        })
    }
}

/// 全服「老主顧」拜訪帳本（`(玩家, 居民id)` → 累積拜訪次數，封頂 [`VISITS_FOR_MAX_DISCOUNT`]）。
#[derive(Default, Debug)]
pub struct RepairLedger {
    visits: HashMap<(String, String), u32>,
}

impl RepairLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// 由持久化紀錄重建（純加總，任意順序重播結果一致）。
    pub fn from_entries(entries: impl IntoIterator<Item = RepairVisitEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            *s.visits.entry((e.player_key, e.resident_key)).or_insert(0) += e.visit_delta;
        }
        s
    }

    /// 這位玩家找這位居民修過幾次（未紀錄過回 0）。
    pub fn visits_of(&self, player_key: &str, resident_key: &str) -> u32 {
        *self
            .visits
            .get(&(player_key.to_string(), resident_key.to_string()))
            .unwrap_or(&0)
    }

    /// 記一次拜訪，回傳保養前的既有次數（用來算這次的折扣/文案）。已達封頂時 `delta` 為 0，
    /// 呼叫端可據此判斷是否仍需 append（見 [`RepairVisitEntry`]）。
    pub fn record_visit(&mut self, player_key: &str, resident_key: &str) -> (u32, RepairVisitEntry) {
        let key = (player_key.to_string(), resident_key.to_string());
        let before = *self.visits.get(&key).unwrap_or(&0);
        let delta = if before >= VISITS_FOR_MAX_DISCOUNT { 0 } else { 1 };
        if delta > 0 {
            self.visits.insert(key, before + delta);
        }
        (
            before,
            RepairVisitEntry {
                player_key: player_key.to_string(),
                resident_key: resident_key.to_string(),
                visit_delta: delta,
            },
        )
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
        Err(e) => tracing::warn!("無法寫入工具耐久記錄 {path}: {e}"),
    }
}

/// append 一筆磨損/保養事件（IO，呼叫端須在鎖外呼叫）。
pub fn append_wear(entry: &WearEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(WEAR_PATH, &line);
    }
}

/// 載回所有磨損/保養事件（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_wear() -> Vec<WearEntry> {
    let Ok(f) = fs::File::open(WEAR_PATH) else {
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
                serde_json::from_str::<WearEntry>(l).ok()
            }
        })
        .collect()
}

/// append 一筆老主顧拜訪事件。
pub fn append_repair_visit(entry: &RepairVisitEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(REPAIR_VISIT_PATH, &line);
    }
}

/// 載回所有老主顧拜訪事件（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_repair_visits() -> Vec<RepairVisitEntry> {
    let Ok(f) = fs::File::open(REPAIR_VISIT_PATH) else {
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
                serde_json::from_str::<RepairVisitEntry>(l).ok()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 純函式 ──────────────────────────────────────────────────────────────

    #[test]
    fn durability_pct_full_range() {
        assert_eq!(durability_pct(0, MAX_DURABILITY), 100);
        assert_eq!(durability_pct(MAX_DURABILITY, MAX_DURABILITY), 0);
        assert_eq!(durability_pct(50, MAX_DURABILITY), 50);
        // 超過 max 一律夾在 0%，不會變成「負磨損」的怪值。
        assert_eq!(durability_pct(999, MAX_DURABILITY), 0);
        // max=0 防呆。
        assert_eq!(durability_pct(0, 0), 0);
    }

    #[test]
    fn worn_out_threshold_boundary() {
        // 剩餘恰好等於門檻 → 該修了（<=）。
        let worn_at_threshold = MAX_DURABILITY - WORN_THRESHOLD_PCT;
        assert!(is_worn_out(worn_at_threshold));
        // 略高於門檻（剩餘多 1%）→ 還不用修。
        assert!(!is_worn_out(worn_at_threshold - 1));
        // 全新絕不算該修了。
        assert!(!is_worn_out(0));
    }

    #[test]
    fn repair_discount_scales_with_visits_and_caps() {
        assert_eq!(repair_discount_pct(0), 0);
        assert_eq!(repair_discount_pct(1), DISCOUNT_PER_VISIT_PCT);
        assert_eq!(repair_discount_pct(VISITS_FOR_MAX_DISCOUNT), MAX_REPAIR_DISCOUNT_PCT);
        // 遠超封頂次數，折扣不再增加。
        assert_eq!(repair_discount_pct(999), MAX_REPAIR_DISCOUNT_PCT);
    }

    #[test]
    fn repair_cost_never_reaches_zero() {
        assert_eq!(repair_cost(0), BASE_REPAIR_COST);
        // 封頂折扣下仍要付至少 1 枚。
        let capped = repair_cost(VISITS_FOR_MAX_DISCOUNT);
        assert!(capped >= 1);
        assert!(capped < BASE_REPAIR_COST);
        assert!(repair_cost(999_999) >= 1);
    }

    #[test]
    fn flavor_lines_contain_the_names() {
        assert!(worn_out_line("鐵鎬").contains("鐵鎬"));
        assert!(first_repair_say_line(5).contains('5'));
        assert!(regular_repair_say_line(3, 4).contains('3') && regular_repair_say_line(3, 4).contains('4'));
        assert!(repair_done_line("鐵鎬", "露娜").contains("鐵鎬") && repair_done_line("鐵鎬", "露娜").contains("露娜"));
        assert!(repair_memory_line("旅人", "石斧").contains("旅人") && repair_memory_line("旅人", "石斧").contains("石斧"));
        assert!(repair_feed_line("露娜", "旅人", "木鏟").contains("露娜"));
    }

    // ── WearStore ───────────────────────────────────────────────────────────

    #[test]
    fn add_wear_accumulates_and_caps_at_max() {
        let mut s = WearStore::new();
        assert_eq!(s.worn_of("p1", 34), 0);
        let e1 = s.add_wear("p1", 34).expect("首次使用一定有變化");
        assert_eq!(e1.wear_delta, WEAR_PER_USE);
        assert_eq!(s.worn_of("p1", 34), WEAR_PER_USE);
        // 用到滿之後，多用不再累積、也回 None（守衛：已改變不了可觀察狀態）。
        for _ in 0..1000 {
            s.add_wear("p1", 34);
        }
        assert_eq!(s.worn_of("p1", 34), MAX_DURABILITY);
        assert!(s.add_wear("p1", 34).is_none(), "已見底，append 應被略過");
    }

    #[test]
    fn different_tools_and_players_track_independently() {
        let mut s = WearStore::new();
        s.add_wear("p1", 34); // 玩家1的鐵鎬
        s.add_wear("p1", 38); // 玩家1的鐵斧（不同工具，各自累積）
        s.add_wear("p2", 34); // 玩家2的鐵鎬（不同玩家，互不影響）
        assert_eq!(s.worn_of("p1", 34), WEAR_PER_USE);
        assert_eq!(s.worn_of("p1", 38), WEAR_PER_USE);
        assert_eq!(s.worn_of("p2", 34), WEAR_PER_USE);
        assert_eq!(s.worn_of("p2", 38), 0);
    }

    #[test]
    fn repair_resets_to_zero_and_noop_when_already_new() {
        let mut s = WearStore::new();
        s.add_wear("p1", 34);
        s.add_wear("p1", 34);
        assert!(s.worn_of("p1", 34) > 0);
        let e = s.repair("p1", 34).expect("有磨損時保養必有變化");
        assert_eq!(e.repair_delta, 2 * WEAR_PER_USE);
        assert_eq!(s.worn_of("p1", 34), 0);
        // 已經全新時再保養一次 → no-op，回 None（避免空事件灌爆帳本）。
        assert!(s.repair("p1", 34).is_none());
    }

    #[test]
    fn from_entries_replays_wear_and_repair_regardless_of_order() {
        let entries = vec![
            WearEntry { player_key: "p1".into(), tool_id: 34, wear_delta: 40, repair_delta: 0 },
            WearEntry { player_key: "p1".into(), tool_id: 34, wear_delta: 40, repair_delta: 0 },
            WearEntry { player_key: "p1".into(), tool_id: 34, wear_delta: 0, repair_delta: 30 },
        ];
        let s = WearStore::from_entries(entries);
        assert_eq!(s.worn_of("p1", 34), 50);
    }

    // ── RepairLedger ────────────────────────────────────────────────────────

    #[test]
    fn record_visit_increments_and_caps() {
        let mut l = RepairLedger::new();
        let (before1, e1) = l.record_visit("p1", "vox_res_0");
        assert_eq!(before1, 0);
        assert_eq!(e1.visit_delta, 1);
        assert_eq!(l.visits_of("p1", "vox_res_0"), 1);
        for _ in 0..1000 {
            l.record_visit("p1", "vox_res_0");
        }
        assert_eq!(l.visits_of("p1", "vox_res_0"), VISITS_FOR_MAX_DISCOUNT);
        // 已封頂後 delta 應為 0（守衛：append 應被略過）。
        let (_, e_capped) = l.record_visit("p1", "vox_res_0");
        assert_eq!(e_capped.visit_delta, 0);
    }

    #[test]
    fn visits_are_per_player_per_resident() {
        let mut l = RepairLedger::new();
        l.record_visit("p1", "vox_res_0");
        l.record_visit("p1", "vox_res_0");
        l.record_visit("p1", "vox_res_1");
        assert_eq!(l.visits_of("p1", "vox_res_0"), 2);
        assert_eq!(l.visits_of("p1", "vox_res_1"), 1);
        assert_eq!(l.visits_of("p2", "vox_res_0"), 0, "不同玩家找同一位居民修，互不影響彼此折扣");
    }

    #[test]
    fn from_entries_sums_visits_regardless_of_order() {
        let entries = vec![
            RepairVisitEntry { player_key: "p1".into(), resident_key: "r0".into(), visit_delta: 1 },
            RepairVisitEntry { player_key: "p1".into(), resident_key: "r0".into(), visit_delta: 1 },
            RepairVisitEntry { player_key: "p1".into(), resident_key: "r1".into(), visit_delta: 1 },
        ];
        let l = RepairLedger::from_entries(entries);
        assert_eq!(l.visits_of("p1", "r0"), 2);
        assert_eq!(l.visits_of("p1", "r1"), 1);
    }
}
