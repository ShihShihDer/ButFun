//! 乙太方界·玩家羈絆帳本 v1（voxel-player-bond，自主提案切片，ROADMAP 985；接續 984，
//! reviewer 明令離開遠征首領／馳援維度，回主軸挑一個內聚新維度）。
//!
//! **v2 追加（自主提案切片，本輪 PR）：摯友協作加成**——v1 讓玩家↔玩家的交情第一次「被
//! 世界記住」（升級提示＋動態牆），但交情再深，日常互動的結果仍與陌生人無異，是「記憶→
//! 行為」這個北極星原則唯一還沒接上玩家↔玩家這條線的地方。本刀補上 [`confidant_yield_bonus`]：
//! 並肩協作範圍內每有一位已是摯友層級的旅伴，額外多收一份材料——讓「處出交情」這件事
//! 第一次真的改寫互動結果，而不只是一句溫情的提示。
//!
//! **真缺口**：`voxel_coop_gather.rs`（827 並肩協作）與 `voxel_dropitem.rs`（828 掉落物
//! 轉手）自己的模組頭註解都白紙黑字寫著「玩家↔玩家至今只有兩三種互動」——但這兩種互動
//! 都是**瞬間的、不留痕跡**：並肩協作只在挖礦那一擊當下算一次加成，收穫進背包後什麼都
//! 沒留下；掉落物一撿走就從世界消失，連撿的人與丟的人之間發生過這件事都無跡可尋。跟居民
//! 彼此的情誼（672 `voxel_bonds`）天差地遠——居民之間的交情會累積、會分級、會被世界看見
//! （問候語隨熟識度變化、升級時全村都知道）；玩家與玩家之間，哪怕天天一起挖礦、天天互相
//! 分材料，什麼都不會留下、什麼都不會被世界記住。這正是 reviewer 在 dropitem/coop_gather
//! 註解裡自己點名、至今沒人接手的真空。
//!
//! **做法**：鏡射 `voxel_bonds`（居民情誼）分級精神——陌生→旅伴→摯友——但主體換成**兩位
//! 真人玩家**。配對鍵用既有登入解出的 `name` 字串（與 `voxel_mastery`/`voxel_prosperity`
//! 等玩家狀態系統同一套慣例）。兩個觸發點都是全庫已存在、玩家日常會撞見的真實互動：
//! ①並肩協作命中時，協作半徑內的真旅伴各記一次；②撿到別人（非自己）丟下的東西時也記一次。
//! 每對玩家設**冷卻**（[`PLAYER_BOND_COOLDOWN_SECS`]）——單次事件只算一次 tick，避免站著
//! 連續採礦或互丟互撿洗刷交情，讓「處出交情」這件事本身也需要真實時間累積，不是一次性
//! 動作就能刷滿。升到旅伴／摯友時，雙方各自跳一句提示、動態牆記一筆——世界第一次「看得見」
//! 玩家之間萌生的交情，而非查無此事。
//!
//! **與既有系統 razor-sharp 區隔**：827/828 是「這一刻發生了什麼」（協作加成／轉手一次）；
//! 本刀是「這些事累積起來代表了什麼」（一段被世界記得的交情）——非重做，是同一份拼圖的
//! 下一層。與居民情誼（672 `voxel_bonds`）也刻意不共用同一份資料結構——主體是真人玩家、
//! 冷卻節奏與門檻皆為玩家互動量身訂做，跟居民 tick 驅動的拜訪節奏不同軸。
//!
//! **純邏輯層**：分級、冷卻判定、台詞全是確定性純函式，零 LLM、零鎖、零 async；IO（鎖／
//! 廣播／持久化 append）在 `voxel_ws.rs`。持久化採 append-only delta 記錄（一行一次真實
//! 互動，比照 `voxel_mastery`／`voxel_inventory`，重啟逐行加總還原，非快照整份帳本）。
//!
//! **成本 / 濫用防護鐵律**：零 LLM（純計數＋確定性文案）、零新美術、零協議破壞（新訊息型別
//! `player_bond_up` additive，舊前端忽略無影響）、FPS 零影響（只在既有 827/828 成功路徑
//! 觸發，非每幀/每 tick）。不收玩家自由文字輸入；不觸發 LLM；不開對外端點；互動與否、冷卻
//! 是否已過、交情帳本升不升級全由伺服器權威判定（協作半徑由伺服器讀既有 players map 算、
//! 掉落物歸屬由伺服器既有 `dropped_by` 欄位判定），玩家無法自報或催發；每對玩家帳本封頂
//! （[`PLAYER_TICK_CAP`]），持久化檔案增長天然有終點。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── 常數 ──────────────────────────────────────────────────────────────────────

/// 同一對玩家之間，兩次真實互動至少要間隔這麼多秒才算數——避免站著連續協作採集或
/// 互丟互撿瞬間洗刷交情，讓累積本身需要真實時間份量。
pub const PLAYER_BOND_COOLDOWN_SECS: u64 = 90;

/// 累積 n 次真實互動後升到「旅伴」。
pub const PLAYER_ACQUAINTANCE_TICKS: u32 = 3;

/// 累積 n 次真實互動後升到「摯友」。
pub const PLAYER_FRIEND_TICKS: u32 = 10;

/// 每對玩家累積次數上限（防超長壽伺服器無限累加、持久化檔案無限長大）。
pub const PLAYER_TICK_CAP: u32 = 30;

// ── 羈絆層級 ──────────────────────────────────────────────────────────────────

/// 兩位真人玩家之間的羈絆層級。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlayerBondTier {
    /// 陌生——尚未累積出足夠的真實互動。
    Stranger,
    /// 旅伴——一起協作／互相轉手過幾次，算得上熟面孔。
    Companion,
    /// 摯友——長期互動下來，交情深厚。
    Confidant,
}

impl PlayerBondTier {
    fn from_ticks(ticks: u32) -> Self {
        if ticks >= PLAYER_FRIEND_TICKS {
            PlayerBondTier::Confidant
        } else if ticks >= PLAYER_ACQUAINTANCE_TICKS {
            PlayerBondTier::Companion
        } else {
            PlayerBondTier::Stranger
        }
    }
}

/// 羈絆層級 → 穩定字串鍵（供協議/持久化用，與顯示文案分開，文案改動不牽動協議）。
pub fn tier_key(tier: PlayerBondTier) -> &'static str {
    match tier {
        PlayerBondTier::Stranger => "stranger",
        PlayerBondTier::Companion => "companion",
        PlayerBondTier::Confidant => "confidant",
    }
}

/// 對稱鍵：確保 (a,b) == (b,a)。
fn pair_key(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

// ── 持久化格式（append-only delta：一行 = 一次計入的真實互動）──────────────────

/// 一筆持久化記錄：這一對玩家之間發生了一次計入的真實互動。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerBondEntry {
    pub id_a: String,
    pub id_b: String,
}

// ── 羈絆帳本 ──────────────────────────────────────────────────────────────────

/// 玩家羈絆帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct PlayerBonds {
    /// key = (min_name, max_name) → 累積 tick 數（已封頂 [`PLAYER_TICK_CAP`]）。
    ticks: HashMap<(String, String), u32>,
    /// 每對玩家上次計入 tick 的時刻（純記憶體，重啟歸零——比照 `voxel_bonds::sulking` 慣例，
    /// 冷卻本身是短命狀態，不影響已持久化的交情本身；重啟後第一次互動不會被誤判冷卻中）。
    last_tick_secs: HashMap<(String, String), u64>,
}

impl PlayerBonds {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化 delta 記錄還原（啟動時呼叫一次）。自我配對（髒資料防呆）安全略過。
    pub fn from_entries(entries: impl IntoIterator<Item = PlayerBondEntry>) -> Self {
        let mut b = Self::new();
        for e in entries {
            if e.id_a == e.id_b {
                continue;
            }
            let key = pair_key(&e.id_a, &e.id_b);
            let entry = b.ticks.entry(key).or_insert(0);
            *entry = (*entry + 1).min(PLAYER_TICK_CAP);
        }
        b
    }

    /// 記錄一次真實互動（並肩協作命中／收下對方轉手的掉落物）。
    ///
    /// 回傳 `None` 代表這次不計入（冷卻中／已達上限／自己與自己／空字串防呆）——
    /// 呼叫端據此**跳過**持久化 append，帳本增長天然有終點，不會無限寫檔。
    /// 回傳 `Some((新層級, 是否在此次升級))` 代表真的計入了一次 tick。
    pub fn record_interaction(&mut self, a: &str, b: &str, now_secs: u64) -> Option<(PlayerBondTier, bool)> {
        if a == b || a.trim().is_empty() || b.trim().is_empty() {
            return None;
        }
        let key = pair_key(a, b);
        let old_ticks = self.ticks.get(&key).copied().unwrap_or(0);
        if old_ticks >= PLAYER_TICK_CAP {
            return None;
        }
        if let Some(&last) = self.last_tick_secs.get(&key) {
            if now_secs.saturating_sub(last) < PLAYER_BOND_COOLDOWN_SECS {
                return None;
            }
        }
        let old_tier = PlayerBondTier::from_ticks(old_ticks);
        let new_ticks = old_ticks + 1;
        self.ticks.insert(key.clone(), new_ticks);
        self.last_tick_secs.insert(key, now_secs);
        let new_tier = PlayerBondTier::from_ticks(new_ticks);
        Some((new_tier, new_tier > old_tier))
    }

    /// 查詢羈絆層級（不改狀態）。
    pub fn tier_of(&self, a: &str, b: &str) -> PlayerBondTier {
        PlayerBondTier::from_ticks(self.tick_count(a, b))
    }

    /// 查詢累積 tick 數（不改狀態）。
    pub fn tick_count(&self, a: &str, b: &str) -> u32 {
        self.ticks.get(&pair_key(a, b)).copied().unwrap_or(0)
    }
}

// ── 摯友協作加成（自主提案切片，接續本模組 985；換維度回應「羈絆的深度該不該有實際
// 後果」——居民彼此的交情早已改寫互動本身（`voxel_bonds` 熟識度調整問候語），玩家↔玩家
// 的交情至今卻只是一句升級提示與動態牆記錄，交情再深，日常互動的結果仍與陌生人無異）───

/// 每有一位摯友層級的協作旅伴，並肩採集額外多收的材料量。
pub const CONFIDANT_BONUS_QTY: u32 = 1;

/// 封頂人數：協作範圍內摯友再多，加成也不無限疊加（防洗刷）。
pub const MAX_CONFIDANT_BONUS_PARTNERS: usize = 2;

/// 純函式：協作範圍內有幾位「摯友」層級的旅伴 → 額外掉落量（封頂）。
/// 與 827 並肩協作的「人數」加成各自獨立疊加——那份加成獎勵「人多」，
/// 本刀獎勵「交情深」，兩者判斷順序互不影響。
pub fn confidant_yield_bonus(confidant_count: usize) -> u32 {
    confidant_count.min(MAX_CONFIDANT_BONUS_PARTNERS) as u32 * CONFIDANT_BONUS_QTY
}

/// 摯友加成觸發時的回饋句（確定性、非 LLM）。
pub fn confidant_bonus_toast_line(other: &str) -> String {
    format!("🤝 你和摯友 {other} 默契十足，多收了一份！")
}

// ── 台詞（確定性、零 LLM）─────────────────────────────────────────────────────

/// 升級當下跳給雙方看的提示句（呼叫端只在 `tier != Stranger` 時使用）。
pub fn tier_up_toast_line(tier: PlayerBondTier, other: &str) -> String {
    match tier {
        PlayerBondTier::Stranger => String::new(),
        PlayerBondTier::Companion => format!("🚶 你和 {other} 好像處出交情了——成了旅伴！"),
        PlayerBondTier::Confidant => format!("🤝 你和 {other} 成了摯友！這份交情，世界都記住了。"),
    }
}

/// 升級當下寫進動態牆的一行（呼叫端只在 `tier != Stranger` 時使用）。
pub fn tier_up_feed_line(tier: PlayerBondTier, a: &str, b: &str) -> String {
    match tier {
        PlayerBondTier::Stranger => format!("{a} 和 {b} 第一次相遇"),
        PlayerBondTier::Companion => format!("{a} 和 {b} 處出了交情，成了旅伴"),
        PlayerBondTier::Confidant => format!("🤝 {a} 和 {b} 成了摯友！"),
    }
}

// ── 持久化 IO（只有函式，鎖在 voxel_ws.rs）──────────────────────────────────

const PLAYER_BOND_FILE: &str = "data/voxel_player_bonds.jsonl";

/// 從 `data/voxel_player_bonds.jsonl` 讀取所有 delta 記錄（檔案不存在回空 Vec）。
pub fn load_player_bonds() -> Vec<PlayerBondEntry> {
    let content = match std::fs::read_to_string(PLAYER_BOND_FILE) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// append 一筆真實互動（一行 = 一次計入的 tick）。呼叫端只在 `record_interaction`
/// 回傳 `Some` 時才呼叫，帳本增長天然有終點。
pub fn append_player_bond(a: &str, b: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(PLAYER_BOND_FILE)
    {
        let entry = PlayerBondEntry { id_a: a.to_string(), id_b: b.to_string() };
        if let Ok(line) = serde_json::to_string(&entry) {
            let _ = writeln!(f, "{line}");
        }
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> PlayerBonds {
        PlayerBonds::new()
    }

    #[test]
    fn new_pair_is_stranger() {
        let b = make();
        assert_eq!(b.tier_of("小明", "小美"), PlayerBondTier::Stranger);
        assert_eq!(b.tick_count("小明", "小美"), 0);
    }

    #[test]
    fn tier_key_stable_strings() {
        assert_eq!(tier_key(PlayerBondTier::Stranger), "stranger");
        assert_eq!(tier_key(PlayerBondTier::Companion), "companion");
        assert_eq!(tier_key(PlayerBondTier::Confidant), "confidant");
    }

    #[test]
    fn symmetry_a_b_equals_b_a() {
        let mut b = make();
        b.record_interaction("小明", "小美", 0);
        assert_eq!(b.tick_count("小明", "小美"), b.tick_count("小美", "小明"));
        assert_eq!(b.tier_of("小明", "小美"), b.tier_of("小美", "小明"));
    }

    #[test]
    fn self_pair_no_effect() {
        let mut b = make();
        assert!(b.record_interaction("小明", "小明", 0).is_none());
        assert_eq!(b.tick_count("小明", "小明"), 0);
    }

    #[test]
    fn empty_name_no_effect() {
        let mut b = make();
        assert!(b.record_interaction("", "小美", 0).is_none());
        assert!(b.record_interaction("小明", "  ", 0).is_none());
    }

    #[test]
    fn cooldown_blocks_rapid_repeats() {
        let mut b = make();
        assert!(b.record_interaction("小明", "小美", 100).is_some());
        // 冷卻內第二次不計入。
        assert!(b.record_interaction("小明", "小美", 100 + PLAYER_BOND_COOLDOWN_SECS - 1).is_none());
        assert_eq!(b.tick_count("小明", "小美"), 1);
    }

    #[test]
    fn cooldown_boundary_exactly_elapsed_counts() {
        let mut b = make();
        b.record_interaction("小明", "小美", 0);
        // 恰好過了冷卻秒數，含界計入。
        assert!(b.record_interaction("小明", "小美", PLAYER_BOND_COOLDOWN_SECS).is_some());
        assert_eq!(b.tick_count("小明", "小美"), 2);
    }

    #[test]
    fn accumulates_to_companion() {
        let mut b = make();
        for i in 0..PLAYER_ACQUAINTANCE_TICKS {
            b.record_interaction("小明", "小美", (i as u64) * PLAYER_BOND_COOLDOWN_SECS);
        }
        assert_eq!(b.tier_of("小明", "小美"), PlayerBondTier::Companion);
    }

    #[test]
    fn accumulates_to_confidant() {
        let mut b = make();
        for i in 0..PLAYER_FRIEND_TICKS {
            b.record_interaction("小明", "小美", (i as u64) * PLAYER_BOND_COOLDOWN_SECS);
        }
        assert_eq!(b.tier_of("小明", "小美"), PlayerBondTier::Confidant);
    }

    #[test]
    fn record_interaction_reports_upgrade_flag() {
        let mut b = make();
        for i in 0..(PLAYER_ACQUAINTANCE_TICKS - 1) {
            let (_, upgraded) = b.record_interaction("小明", "小美", (i as u64) * PLAYER_BOND_COOLDOWN_SECS).unwrap();
            assert!(!upgraded, "未到門檻不應升級");
        }
        let (tier, upgraded) = b
            .record_interaction("小明", "小美", (PLAYER_ACQUAINTANCE_TICKS as u64 - 1) * PLAYER_BOND_COOLDOWN_SECS)
            .unwrap();
        assert!(upgraded);
        assert_eq!(tier, PlayerBondTier::Companion);
    }

    #[test]
    fn tick_cap_enforced_and_stops_recording() {
        let mut b = make();
        for i in 0..(PLAYER_TICK_CAP + 5) {
            b.record_interaction("小明", "小美", (i as u64) * PLAYER_BOND_COOLDOWN_SECS);
        }
        assert_eq!(b.tick_count("小明", "小美"), PLAYER_TICK_CAP);
        // 已達上限後呼叫回 None（呼叫端據此跳過 append，檔案增長有終點）。
        assert!(b
            .record_interaction("小明", "小美", (PLAYER_TICK_CAP as u64 + 999) * PLAYER_BOND_COOLDOWN_SECS)
            .is_none());
    }

    #[test]
    fn from_entries_restores_state() {
        let entries = vec![
            PlayerBondEntry { id_a: "小明".into(), id_b: "小美".into() },
            PlayerBondEntry { id_a: "小美".into(), id_b: "小明".into() },
            PlayerBondEntry { id_a: "阿一".into(), id_b: "阿一".into() }, // 髒資料防呆
        ];
        let b = PlayerBonds::from_entries(entries);
        assert_eq!(b.tick_count("小明", "小美"), 2);
        assert_eq!(b.tick_count("阿一", "阿一"), 0);
    }

    #[test]
    fn from_entries_caps_excessive_ticks() {
        let entries: Vec<PlayerBondEntry> = (0..(PLAYER_TICK_CAP + 10))
            .map(|_| PlayerBondEntry { id_a: "小明".into(), id_b: "小美".into() })
            .collect();
        let b = PlayerBonds::from_entries(entries);
        assert_eq!(b.tick_count("小明", "小美"), PLAYER_TICK_CAP);
    }

    #[test]
    fn independent_pairs_do_not_interfere() {
        let mut b = make();
        b.record_interaction("小明", "小美", 0);
        assert_eq!(b.tick_count("小明", "阿一"), 0);
        assert_eq!(b.tick_count("小美", "阿一"), 0);
    }

    #[test]
    fn tier_ordering_stranger_lt_companion_lt_confidant() {
        assert!(PlayerBondTier::Stranger < PlayerBondTier::Companion);
        assert!(PlayerBondTier::Companion < PlayerBondTier::Confidant);
    }

    #[test]
    fn constants_are_monotone() {
        assert!(PLAYER_ACQUAINTANCE_TICKS < PLAYER_FRIEND_TICKS);
        assert!(PLAYER_FRIEND_TICKS < PLAYER_TICK_CAP);
        assert!(PLAYER_BOND_COOLDOWN_SECS > 0);
    }

    #[test]
    fn toast_line_non_empty_for_real_tiers() {
        assert!(!tier_up_toast_line(PlayerBondTier::Companion, "小美").is_empty());
        assert!(!tier_up_toast_line(PlayerBondTier::Confidant, "小美").is_empty());
        assert!(tier_up_toast_line(PlayerBondTier::Stranger, "小美").is_empty());
    }

    #[test]
    fn toast_line_mentions_other_name() {
        assert!(tier_up_toast_line(PlayerBondTier::Companion, "小美").contains("小美"));
        assert!(tier_up_toast_line(PlayerBondTier::Confidant, "小美").contains("小美"));
    }

    #[test]
    fn feed_line_non_empty_all_tiers_and_mentions_both_names() {
        for tier in [PlayerBondTier::Stranger, PlayerBondTier::Companion, PlayerBondTier::Confidant] {
            let line = tier_up_feed_line(tier, "小明", "小美");
            assert!(!line.is_empty());
            assert!(line.contains("小明"));
            assert!(line.contains("小美"));
        }
    }

    #[test]
    fn confidant_yield_bonus_zero_when_no_confidant() {
        assert_eq!(confidant_yield_bonus(0), 0);
    }

    #[test]
    fn confidant_yield_bonus_scales_with_count() {
        assert_eq!(confidant_yield_bonus(1), CONFIDANT_BONUS_QTY);
        assert_eq!(confidant_yield_bonus(2), CONFIDANT_BONUS_QTY * 2);
    }

    #[test]
    fn confidant_yield_bonus_caps_at_max_partners() {
        let capped = confidant_yield_bonus(MAX_CONFIDANT_BONUS_PARTNERS);
        assert_eq!(confidant_yield_bonus(MAX_CONFIDANT_BONUS_PARTNERS + 10), capped);
    }

    #[test]
    fn confidant_bonus_toast_line_mentions_name_and_nonempty() {
        let line = confidant_bonus_toast_line("小美");
        assert!(!line.is_empty());
        assert!(line.contains("小美"));
    }
}
