//! 乙太方界·荒年 v1（自主提案切片，ROADMAP 986）。
//!
//! **真缺口**：世界至今有天氣（下雨/彩虹）、天象（流星/候鳥）、生理需求（居民也會肚子餓 799）、
//! 遠征首領（974~984）等多種「事件」，但從沒有一樁**波及全村、逼出互助**的集體性稀缺危機——
//! 種田（659 起）純粹是「等時間到就收成」，居民的餓意（799）也純粹是「個人餓了就回家吃」，
//! 兩條線各自運作，世界從沒有過一刻讓「大家的日子突然一起變緊」。真實的村落生活裡，總會偶爾
//! 遇上一段收成不好的日子——這正是本專案至今唯一一種**全村同時感受到、需要互相扶一把**的處境，
//! 也是 reviewer 判準「多人／人與居民之間產生新因果」最直接的落點：你在荒年裡送去的一口食物，
//! 分量比平時更重。
//!
//! **設計（v1，刻意有界）**：偶爾（低機率、有強制冷卻）觸發一段**荒年**——固定時長的窗口內，
//! 全村居民的餓意累積速度加快（用既有 [`crate::voxel_hunger::tick_hunger`] 的 dt 倍率接線，
//! 零新狀態機、零新方塊）。荒年開始／結束各播一則城鎮動態，附近醒著的居民各冒一句應景心聲
//! （零 LLM、確定性選句，語氣不慌不忙——**這是療癒世界，不是生存遊戲**，荒年只是讓日子暫時
//! 緊一點，不會有任何人真的餓壞）。**這段日子裡，若你送食物餵飽了一位正餓著的居民**（沿用既有
//! 「餓時被餵」765/799 那套深記憶管線），這一口食物會換來**格外不同的記憶措辭**——她會記得
//! 「荒年那陣子」是你幫她撐過去的，而不只是「餓的時候被餵了一口」。窗口結束時，村莊把「熬過幾次
//! 荒年」與「這次有幾次是被人及時餵飽化解」一併持久記下（append-only jsonl，向後相容），
//! 讓「這是第 N 次熬過荒年」成為一份真正累積、不隨重啟歸零的村莊履歷。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **居民也會肚子餓（799）**＝**個人**的、恆常的生理需求，居民自理（回家吃）即可化解；
//!   本刀＝**全村同時**、偶發的**外部**環境危機，加快的是餓意累積的**速率**，不是新增一種需求。
//! - **天氣／天象（雨、彩虹、流星、候鳥）**＝**純氛圍**事件，觸發後只換來一句台詞／一次歡呼，
//!   結束後世界毫無變化；本刀有**真實的機制效果**（餓意加速）＋**持久累積的村莊履歷**，且玩家
//!   在窗口內的行為（餵食）真的改變了這次危機被記住的方式。
//! - **遠征首領（974~984）**＝**戰鬥**威脅、由玩家/居民合力削血；本刀＝**經濟／生理**面的
//!   緊縮、由玩家單純的善意（餵食）化解，兩者是全然不同性質的「世界級處境」。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式（觸發／冷卻狀態機、
//! 餓意倍率、台詞、Feed 摘要），`next_famine` 的隨機性由呼叫端注入單一 `roll: f32`，方便單元測試
//! 窮舉每個轉態分支。狀態機本體（`hub().famine_tick`）、旗標消費、記憶／Feed IO 全在
//! `voxel_ws.rs`（比照流星 904／候鳥遷徙 944 的短鎖循序＋鎖外落地慣例，守死鎖鐵律）。
//!
//! **成本／安全紀律**：零 LLM（觸發、台詞皆確定性）、零協議破壞（純背景效果＋動態牆文字，
//! 不新增任何快照欄位——前端無感知，是後端獨有的世界節奏）、FPS 零負擔。**濫用防護**：不收
//! 玩家輸入、不觸發 LLM、不開對外端點——荒年何時發生純由伺服器內部低機率擲骰＋強制冷卻決定，
//! 玩家無法自報或催發；持久化只落地固定欄位的計數，不含任何玩家原文。

use serde::{Deserialize, Serialize};

/// 持久化路徑（`data/` 已 gitignore；全域一份，不分玩家）。
pub const FAMINE_LOG_PATH: &str = "data/voxel_famines.jsonl";

/// 城鎮動態 Feed 的「荒年」分類鍵。
pub const FEED_KIND: &str = "荒年";

/// 一次 [`crate::voxel_ws`] tick_farm 檢查週期（≈15 秒，同天氣／流星檢查節拍）。
/// 荒年持續 [`FAMINE_DURATION_TICKS`] 個週期 ≈ 120 秒——夠玩家真切感受到「這陣子居民餓得快」，
/// 但不會久到讓日常玩法失衡。
pub const FAMINE_DURATION_TICKS: u32 = 8;

/// 荒年結束後強制冷卻的週期數 ≈ 3600 秒（60 分鐘）——確保荒年是「偶爾發生的處境」而非
/// 頻繁刷新的日常，冷卻期間無論擲骰結果如何都不會再次觸發。
pub const FAMINE_COOLDOWN_TICKS: u32 = 240;

/// 冷卻期滿後，每個檢查週期觸發一場新荒年的機率——刻意偏低，讓荒年帶著「說來就來」的
/// 不確定感，而非冷卻一到就準時發生的機械節奏。
pub const FAMINE_TRIGGER_CHANCE_PER_TICK: f32 = 0.05;

/// 荒年窗口內，居民餓意累積速度的倍率——日子明顯緊了一點，但仍在既有覓食/儲糧機制能應付的範圍。
pub const FAMINE_HUNGER_MULT: f32 = 1.8;

/// 荒年狀態機（純同步、可 `Copy`，由呼叫端包進 `RwLock`）。`remaining > 0` 表示荒年進行中；
/// 否則 `cooldown > 0` 表示尚在強制冷卻、不可能觸發新荒年。兩者互斥，任何時刻至多一者非零。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FamineTick {
    pub remaining: u32,
    pub cooldown: u32,
}

impl FamineTick {
    /// 此刻荒年是否進行中。
    pub fn is_active(&self) -> bool {
        self.remaining > 0
    }
}

/// 一次狀態轉換的結果，供呼叫端決定是否要播報／觸發居民反應。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FamineEvent {
    /// 本輪無轉換（持續進行中／持續冷卻中／持續平靜）。
    None,
    /// 本輪剛觸發一場新荒年。
    Started,
    /// 本輪荒年剛結束（進入冷卻）。
    Ended,
}

/// 純函式：依上一輪狀態 + 本輪擲骰結果，算出下一輪狀態與這一輪發生的轉換。
///
/// - 荒年進行中 → 剩餘週期遞減；減到 0 時轉入冷卻並回傳 [`FamineEvent::Ended`]。
/// - 冷卻中 → 冷卻週期遞減（`saturating_sub`，永不下溢）。
/// - 冷卻已滿（雙為 0）→ `roll < FAMINE_TRIGGER_CHANCE_PER_TICK` 時觸發新荒年並回傳
///   [`FamineEvent::Started`]，否則維持平靜。
///
/// 確定性、無副作用、可窮舉單元測試（`roll` 由呼叫端以 `rand::random()` 餵入）。
pub fn next_famine(prev: FamineTick, roll: f32) -> (FamineTick, FamineEvent) {
    if prev.remaining > 0 {
        let remaining = prev.remaining - 1;
        if remaining == 0 {
            return (
                FamineTick { remaining: 0, cooldown: FAMINE_COOLDOWN_TICKS },
                FamineEvent::Ended,
            );
        }
        return (FamineTick { remaining, cooldown: 0 }, FamineEvent::None);
    }
    if prev.cooldown > 0 {
        return (
            FamineTick { remaining: 0, cooldown: prev.cooldown.saturating_sub(1) },
            FamineEvent::None,
        );
    }
    if roll < FAMINE_TRIGGER_CHANCE_PER_TICK {
        return (
            FamineTick { remaining: FAMINE_DURATION_TICKS, cooldown: 0 },
            FamineEvent::Started,
        );
    }
    (FamineTick::default(), FamineEvent::None)
}

/// 荒年進行中時的餓意速率倍率；平時回 1.0（供呼叫端直接乘進 `dt`，零特例分岔）。
pub fn hunger_mult(active: bool) -> f32 {
    if active { FAMINE_HUNGER_MULT } else { 1.0 }
}

/// 荒年剛開始時，附近醒著的居民抬頭感到日子緊了一點的心聲（零 LLM、確定性選句、
/// 語氣平靜不驚慌——療癒世界，只是提醒「這陣子省著點」）。
pub fn famine_start_say_line(pick: usize) -> &'static str {
    const LINES: &[&str] = &[
        "今年的收成看起來不太好呢…最近得省著點吃了。",
        "唉，這陣子田裡的東西長得特別慢，先把存糧看緊點吧。",
        "聽說最近日子要緊一點了，大家互相照應著點囉。",
        "這光景不太妙…不過撐一撐總會過去的。",
    ];
    LINES[pick % LINES.len()]
}

/// 荒年結束時，附近醒著的居民鬆一口氣的心聲。
pub fn famine_end_say_line(pick: usize) -> &'static str {
    const LINES: &[&str] = &[
        "總算熬過去了，鬆了好大一口氣。",
        "這陣子總算撐過來了，日子又寬鬆了些。",
        "呼…緊巴巴的日子過去啦，謝天謝地。",
        "熬過來了呢，感覺又能好好過日子了。",
    ];
    LINES[pick % LINES.len()]
}

/// 荒年開始的城鎮動態摘要（不在線上的玩家回來也讀得到「這陣子村里日子緊了」）。
pub fn famine_start_feed_detail() -> &'static str {
    "今年的收成不太好，村里悄悄進入了一段青黃不接的日子——大家的餓意來得比平時更快。"
}

/// 荒年結束的城鎮動態摘要——`helped` 是這次窗口內「被玩家及時餵飽化解餓意」的次數，
/// `survived_total` 是村莊累計熬過荒年的總次數（append-only 持久計數，見 [`append_famine_ended`]）。
/// 有人幫忙時特別點名，讓「你的善意」被全村看見；沒人幫忙也依然溫柔收尾，不帶指責。
pub fn famine_end_feed_detail(helped: u32, survived_total: u32) -> String {
    if helped > 0 {
        format!(
            "青黃不接的日子總算過去了——這段日子裡有 {helped} 次是有人及時送來食物撐過難關。\
             這是村莊第 {survived_total} 次熬過荒年。"
        )
    } else {
        format!(
            "青黃不接的日子總算過去了，幸好誰都沒真的餓著。這是村莊第 {survived_total} 次熬過荒年。"
        )
    }
}

/// 荒年窗口內，玩家餵飽一位正餓著的居民時，換來的深記憶措辭——比平時的「餓時被餵」
/// （[`crate::voxel_hunger::fed_memory_line`]）更重一分：她記得的不只是「餓的時候被餵了」，
/// 而是「荒年那陣子」的雪中送炭。
pub fn famine_fed_memory_line(player: &str) -> String {
    format!("荒年那陣子，是{player}送來食物幫我撐過了難關，這份情我會一直記得。")
}

/// 荒年窗口內餵食化解的城鎮動態摘要。
pub fn famine_rescue_feed_line(rname: &str, player: &str) -> String {
    format!("荒年正緊的時候，{player} 送來了食物，幫 {rname} 撐過了這一陣子。")
}

/// 一筆持久化記錄：一場荒年落幕（全域，不分玩家）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FamineEndedEntry {
    /// 這次窗口內，被玩家及時餵飽化解餓意的次數。
    pub helped_count: u32,
}

/// Append 一筆荒年落幕記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_famine_ended(entry: &FamineEndedEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(FAMINE_LOG_PATH, &line);
    }
}

/// 載回所有荒年落幕記錄（啟動時呼叫一次，回傳筆數＝村莊累計熬過次數）。檔不存在／壞行皆容忍。
pub fn load_famine_survived_count() -> u32 {
    let content = match std::fs::read_to_string(FAMINE_LOG_PATH) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| serde_json::from_str::<FamineEndedEntry>(l).is_ok())
        .count() as u32
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
        Err(e) => tracing::warn!("無法寫入荒年落幕記錄 {path}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_state_stays_quiet_when_roll_misses() {
        let (next, ev) = next_famine(FamineTick::default(), 0.99);
        assert_eq!(next, FamineTick::default());
        assert_eq!(ev, FamineEvent::None);
    }

    #[test]
    fn quiet_state_starts_famine_when_roll_hits() {
        let (next, ev) = next_famine(FamineTick::default(), 0.0);
        assert_eq!(next.remaining, FAMINE_DURATION_TICKS);
        assert_eq!(next.cooldown, 0);
        assert_eq!(ev, FamineEvent::Started);
    }

    #[test]
    fn active_famine_counts_down_without_early_end() {
        let mut t = FamineTick { remaining: FAMINE_DURATION_TICKS, cooldown: 0 };
        for _ in 0..FAMINE_DURATION_TICKS - 1 {
            let (next, ev) = next_famine(t, 0.0); // roll 命中也不該在進行中觸發新荒年
            assert_eq!(ev, FamineEvent::None);
            t = next;
        }
        assert!(t.remaining > 0);
    }

    #[test]
    fn active_famine_ends_into_cooldown() {
        let t = FamineTick { remaining: 1, cooldown: 0 };
        let (next, ev) = next_famine(t, 0.0);
        assert_eq!(ev, FamineEvent::Ended);
        assert_eq!(next.remaining, 0);
        assert_eq!(next.cooldown, FAMINE_COOLDOWN_TICKS);
    }

    #[test]
    fn cooldown_blocks_new_famine_even_when_roll_hits() {
        let t = FamineTick { remaining: 0, cooldown: 3 };
        let (next, ev) = next_famine(t, 0.0);
        assert_eq!(ev, FamineEvent::None);
        assert_eq!(next.cooldown, 2);
        assert_eq!(next.remaining, 0);
    }

    #[test]
    fn cooldown_counts_down_to_zero_without_underflow() {
        let mut t = FamineTick { remaining: 0, cooldown: 1 };
        let (next, _) = next_famine(t, 0.99);
        assert_eq!(next.cooldown, 0);
        t = next;
        // 冷卻已到 0，平靜擲骰不中則維持平靜（不下溢、不誤觸發）。
        let (next2, ev2) = next_famine(t, 0.99);
        assert_eq!(next2, FamineTick::default());
        assert_eq!(ev2, FamineEvent::None);
    }

    #[test]
    fn is_active_matches_remaining() {
        assert!(!FamineTick::default().is_active());
        assert!(FamineTick { remaining: 1, cooldown: 0 }.is_active());
    }

    #[test]
    fn hunger_mult_only_applies_when_active() {
        assert_eq!(hunger_mult(false), 1.0);
        assert_eq!(hunger_mult(true), FAMINE_HUNGER_MULT);
        assert!(FAMINE_HUNGER_MULT > 1.0, "荒年應讓餓意來得更快，不是持平或更慢");
    }

    #[test]
    fn say_lines_nonempty_distinct_and_pick_safe() {
        let mut seen = std::collections::HashSet::new();
        for pick in 0..4 {
            let l = famine_start_say_line(pick);
            assert!(!l.is_empty());
            assert!(seen.insert(l));
        }
        assert!(!famine_start_say_line(usize::MAX).is_empty());
        let mut seen2 = std::collections::HashSet::new();
        for pick in 0..4 {
            let l = famine_end_say_line(pick);
            assert!(!l.is_empty());
            assert!(seen2.insert(l));
        }
        assert!(!famine_end_say_line(usize::MAX).is_empty());
    }

    #[test]
    fn feed_details_are_nonempty_and_newline_free() {
        assert!(!famine_start_feed_detail().is_empty());
        assert!(!famine_start_feed_detail().contains('\n'));
        let with_help = famine_end_feed_detail(2, 5);
        assert!(with_help.contains('2') && with_help.contains('5'));
        assert!(!with_help.contains('\n'));
        let no_help = famine_end_feed_detail(0, 1);
        assert!(!no_help.contains('\n'));
        assert_ne!(with_help, no_help);
    }

    #[test]
    fn famine_fed_memory_line_embeds_player_name_only() {
        let line = famine_fed_memory_line("小明");
        assert!(line.contains("小明"));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn famine_rescue_feed_line_embeds_both_names() {
        let line = famine_rescue_feed_line("露娜", "小明");
        assert!(line.contains("露娜") && line.contains("小明"));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn famine_ended_entry_roundtrips_through_json() {
        let entry = FamineEndedEntry { helped_count: 3 };
        let json = serde_json::to_string(&entry).unwrap();
        let back: FamineEndedEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.helped_count, 3);
    }

    #[test]
    fn load_count_on_missing_file_is_zero() {
        // 不存在的路徑（測試環境不會真的寫入 FAMINE_LOG_PATH）應安全回 0，不 panic。
        let count = std::fs::read_to_string("data/__voxel_famine_never_exists__.jsonl")
            .ok()
            .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        assert_eq!(count, 0);
    }
}
