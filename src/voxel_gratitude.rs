//! 乙太方界·知恩圖報 v1——居民記得誰在牠餓時分過飯，換牠有餘力時優先回報那一口（自主提案切片）。
//!
//! **缺口 / 為誰做**：800「守望相助」讓餓著的居民路遇**交情已到相識以上**的閒著鄰居時，被分一口飯——
//! 但那頓飯的情，至今只沉進了記憶與情誼帳本，**沒有任何回聲**：被分過飯的居民，日後看到當初那位恩人
//! 餓著肚子，牠不會特別做什麼，那口飯的「後果」到此為止。這正對著 PLAN_ETHERVOX §3「你的互動有後果
//! （幫過牠 → 回報）」＋§4「居民↔居民關係 → 小社會湧現」——一個真的活著的小村，受過的恩會記在心上、
//! 有機會就還。本刀把「被分一口飯」這件事，接上一條**跨時間的回報鏈**。
//!
//! **記憶驅動行為（北極星）＋ 打破常規的例外**：800 的鐵律是「只有相識以上才分食，陌生人擦身而過不會」。
//! 本刀讓**記憶對這條規則產生真實例外**——如果一位居民**記得**某人曾在牠餓時分過牠飯（欠著一口飯的情），
//! 那麼日後那位恩人餓著、而牠正好閒著有餘力時，牠會**回報那一口飯，就算兩人其實還沒處出交情（陌生人）**。
//! 你會親眼看到一件 800 裡永遠不會發生的事：兩個交情還淺的居民之間，一方主動對另一方分食——只因為
//! 牠**記得**對方當年的那份好。記憶不只是背景，它讓居民做出了原本規則不允許的善舉。
//!
//! **與既有系統的分界（換維度·非同軸重複）**：
//! - 不是 800 守望相助——那是**即時**的、看**當下情誼**（相識以上）決定分不分；本刀是**跨時間**的、看
//!   **過去那一筆恩情**（誰欠誰一口飯）決定回不回報，且**專門打破** 800 的相識門檻（回報連陌生人也還）。
//! - 不是 667 居民回禮（好感達門檻 → 居民主動送玩家小禮）——那是**居民對玩家**、一生一次的定額禮；本刀是
//!   **居民對居民**、由「曾被分食」這件具體往事觸發的**對等回報**。
//! - 不是 748 居民互贈（搬既有背包材料）——本刀還的是**那一口飯**（餓意），承接 799/800 的生理需求線。
//!
//! **這裡只放確定性純邏輯**（欠飯帳本、回報門檻、台詞/記憶/Feed 文案），零 LLM、零鎖、零 IO、零 async，
//! 可單元測試。配對掃描 / 鎖 / 走動 / 廣播全留在 `voxel_ws.rs`（沿用 800 守望相助的「位置快照 → i≠j 循序
//! 掃描 → 每 tick 最多一對 → 記憶/Feed 鎖外落地」慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：不觸發 LLM、不開對外端點、不收玩家自由文字——回報/道謝台詞與記憶全為確定性模板、
//! 只嵌居民系統顯示名（本就出現在動態牆），永不回放記憶原文或玩家原話（無注入 / NSFW 面）；觸發純由伺服器
//! tick 內部狀態（曾被分食的欠飯記錄 + 餓意 + 分食冷卻 + 低機率），玩家無法自報、無法從外部催發；沿用 800
//! 每位分食者的長冷卻 + 每 tick 至多一對 = 天然節流，不洗版泡泡/動態牆。欠飯帳本純記憶體、重啟歸零（過場
//! 狀態、零資料風險、零 migration），回報落地的記憶/情誼走既有 append-only 管線，不碰玩家資料 / 帳號權限。

use std::collections::{HashMap, HashSet};

/// 條件都滿足後，這一 tick 真的回報那口飯的機率——刻意**高於** 800 的 `SHARE_CHANCE`（0.5）：
/// 受過的恩記在心上，一有機會就想還，比對陌生人隨手分食更積極、更少猶豫。
pub const REPAY_CHANCE: f32 = 0.9;

/// 回報門檻：分食冷卻已過 + 過了機率骰。純函式、確定性、可測（與 800 `should_share` 同形，
/// 但用 `REPAY_CHANCE`，語意是「還一筆記著的恩」而非「隨手分食」）。
pub fn should_repay(cooldown: f32, roll: f32, chance: f32) -> bool {
    cooldown <= 0.0 && roll < chance
}

/// 欠飯帳本（純記憶體、重啟歸零）：記錄「誰欠誰一口飯」——鍵是**曾被分食者**（欠飯者）的居民 id，
/// 值是牠欠著一口飯的一群**恩人**（分食者）的 id 集合。以穩定的居民 id 記帳（非顯示名），
/// 名字改動也不影響這筆恩情。
#[derive(Default)]
pub struct MealDebts {
    debts: HashMap<String, HashSet<String>>,
}

impl MealDebts {
    /// 記下「`debtor` 欠了 `benefactor` 一口飯」。空 id 或自己欠自己一律略過（防髒資料）。
    pub fn owe(&mut self, debtor: &str, benefactor: &str) {
        if debtor.is_empty() || benefactor.is_empty() || debtor == benefactor {
            return;
        }
        self.debts
            .entry(debtor.to_string())
            .or_default()
            .insert(benefactor.to_string());
    }

    /// `debtor` 此刻是否欠著 `benefactor` 一口飯（＝該不該優先回報、且可打破相識門檻）。
    pub fn owes(&self, debtor: &str, benefactor: &str) -> bool {
        self.debts
            .get(debtor)
            .map_or(false, |set| set.contains(benefactor))
    }

    /// 結清「`debtor` → `benefactor`」這一筆欠飯（回報後呼叫）。回傳是否真的清掉一筆
    /// （沒欠則回 false）。集合清空後順手移除該鍵，帳本不留空殼。
    pub fn repay(&mut self, debtor: &str, benefactor: &str) -> bool {
        let cleared = if let Some(set) = self.debts.get_mut(debtor) {
            let c = set.remove(benefactor);
            if set.is_empty() {
                self.debts.remove(debtor);
            }
            c
        } else {
            false
        };
        cleared
    }

    /// 目前帳本上總共記著幾筆欠飯（測試/觀測用）。
    pub fn total_debts(&self) -> usize {
        self.debts.values().map(|s| s.len()).sum()
    }
}

/// 回報者主動遞回那口飯時的暖泡泡（四句輪替，比照 800 `sharer_line` 不嵌名、保持口語短句；
/// 「我記得是你」的點名交給記憶/Feed 那層）。
pub fn repay_sharer_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "上回你分我一口，這次換我",
        "還記得你那頓飯，這口還你",
        "當初你幫過我，這次我來",
        "輪到我了，這口你先吃",
    ];
    LINES[pick % LINES.len()]
}

/// 被回報的恩人延遲後冒出的道謝泡泡（嵌回報者名，四句輪替，截字前先組好）。
/// `repayer` 空 → 退成泛稱、不留空洞。
pub fn repay_thanks_line(repayer: &str, pick: usize) -> String {
    const LINES: [&str; 4] = [
        "你還記得那頓飯呀，真暖心",
        "沒想到你會記著，謝謝你",
        "這下換你照應我了，感激",
        "有來有往，這份情更深了",
    ];
    let base = LINES[pick % LINES.len()];
    if repayer.is_empty() {
        base.to_string()
    } else {
        format!("{repayer}，{base}")
    }
}

/// 回報者掛在恩人名下的暖記憶（episodic，累積情誼）。`benefactor` 空 → 泛稱。
pub fn repay_memory_for_repayer(benefactor: &str) -> String {
    if benefactor.is_empty() {
        "當年有位鄰居在我餓時分我一口飯，今天換牠餓著，我把那口飯還了回去。".to_string()
    } else {
        format!("當年{benefactor}在我餓時分我一口飯，今天換牠餓著，我把那口飯還了回去。")
    }
}

/// 恩人掛在回報者名下的記憶（episodic，累積情誼）。`repayer` 空 → 泛稱。
pub fn repay_memory_for_benefactor(repayer: &str) -> String {
    if repayer.is_empty() {
        "我曾在一位鄰居餓時分過牠一口飯，今天換我餓，牠竟記著、把那口飯還給了我。".to_string()
    } else {
        format!("我曾在{repayer}餓時分過牠一口飯，今天換我餓，牠竟記著、把那口飯還給了我。")
    }
}

/// 城鎮動態牆一行：讓不在場 / 回來的玩家也讀到「當年那口飯，今天有了回聲」。
pub fn repay_feed_line(repayer: &str, benefactor: &str) -> String {
    let r = if repayer.is_empty() { "有位鄰居" } else { repayer };
    let b = if benefactor.is_empty() {
        "曾分過牠飯的鄰居"
    } else {
        benefactor
    };
    format!("{b}餓了，{r}記著當年那口飯的情，回報了一口——知恩圖報，暖了整座村子。")
}

/// Feed 分類標籤。
pub const FEED_KIND: &str = "知恩圖報";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_repay_needs_cooldown_and_roll() {
        // 冷卻未到：不回報（就算骰贏）。
        assert!(!should_repay(1.0, 0.0, REPAY_CHANCE));
        // 冷卻到、骰贏：回報。
        assert!(should_repay(0.0, 0.1, REPAY_CHANCE));
        // 冷卻到、骰輸（roll == chance 不觸發，嚴格小於）：不回報。
        assert!(!should_repay(0.0, REPAY_CHANCE, REPAY_CHANCE));
        assert!(!should_repay(0.0, 0.95, REPAY_CHANCE));
        // 冷卻恰為 0 也算到期。
        assert!(should_repay(0.0, 0.0, 1.0));
    }

    #[test]
    fn repay_chance_is_more_eager_than_share() {
        // 記著的恩要比隨手分食更積極——回報機率應高於 800 的分食機率（0.5）。
        assert!(REPAY_CHANCE > 0.5);
        assert!(REPAY_CHANCE <= 1.0);
    }

    #[test]
    fn debts_owe_and_repay_roundtrip() {
        let mut d = MealDebts::default();
        assert!(!d.owes("nova", "luna"));
        d.owe("nova", "luna");
        assert!(d.owes("nova", "luna"));
        assert_eq!(d.total_debts(), 1);
        // 方向性：nova 欠 luna 不代表 luna 欠 nova。
        assert!(!d.owes("luna", "nova"));
        // 結清後不再欠、帳本不留空殼。
        assert!(d.repay("nova", "luna"));
        assert!(!d.owes("nova", "luna"));
        assert_eq!(d.total_debts(), 0);
        // 沒欠再結清回 false。
        assert!(!d.repay("nova", "luna"));
    }

    #[test]
    fn debts_multiple_benefactors_and_bidirectional() {
        let mut d = MealDebts::default();
        d.owe("nova", "luna");
        d.owe("nova", "auri"); // nova 同時欠兩位
        d.owe("luna", "nova"); // 雙向：luna 也欠 nova（不同往事）
        assert_eq!(d.total_debts(), 3);
        // 結清一筆只清那一筆，其餘保留。
        assert!(d.repay("nova", "luna"));
        assert!(!d.owes("nova", "luna"));
        assert!(d.owes("nova", "auri"));
        assert!(d.owes("luna", "nova"));
        assert_eq!(d.total_debts(), 2);
    }

    #[test]
    fn debts_ignore_empty_and_self() {
        let mut d = MealDebts::default();
        d.owe("", "luna");
        d.owe("nova", "");
        d.owe("nova", "nova"); // 自己不欠自己
        assert_eq!(d.total_debts(), 0);
    }

    #[test]
    fn lines_rotate_and_bounded() {
        // 回報/道謝台詞輪替、非空、長度在泡泡上限（≤40 字，含嵌名後）內。
        for pick in 0..8 {
            let sl = repay_sharer_line(pick);
            assert!(!sl.is_empty() && sl.chars().count() <= 40);
            let tl = repay_thanks_line("諾娃", pick);
            assert!(!tl.is_empty() && tl.chars().count() <= 40);
        }
        // pick 溢出取模包回、不 panic。
        assert_eq!(repay_sharer_line(0), repay_sharer_line(4));
        assert_eq!(repay_thanks_line("露娜", 1), repay_thanks_line("露娜", 5));
    }

    #[test]
    fn thanks_embeds_or_falls_back() {
        let t = repay_thanks_line("賽勒", 0);
        assert!(t.contains("賽勒"));
        // 空名退泛稱、不留空洞。
        let g = repay_thanks_line("", 0);
        assert!(!g.is_empty() && !g.contains("，，"));
    }

    #[test]
    fn memories_embed_names_or_fall_back() {
        let mr = repay_memory_for_repayer("露娜");
        assert!(mr.contains("露娜") && mr.contains("飯"));
        let mb = repay_memory_for_benefactor("奧瑞");
        assert!(mb.contains("奧瑞") && mb.contains("飯"));
        // 空名皆退泛稱、仍提及回報情境、不含原話。
        assert!(repay_memory_for_repayer("").contains("飯"));
        assert!(!repay_memory_for_benefactor("").is_empty());
    }

    #[test]
    fn feed_embeds_both_or_falls_back() {
        let f = repay_feed_line("露娜", "諾娃");
        assert!(f.contains("露娜") && f.contains("諾娃"));
        // 任一空名皆退泛稱、不破句。
        let g = repay_feed_line("", "");
        assert!(g.contains("鄰居") && !g.is_empty());
    }

    #[test]
    fn long_names_do_not_break_or_panic() {
        let long = "超".repeat(200);
        // 嵌超長名不 panic；泡泡層由呼叫端截字，這裡只保證不炸。
        let _ = repay_sharer_line(0);
        let _ = repay_thanks_line(&long, 0);
        let _ = repay_memory_for_repayer(&long);
        let _ = repay_memory_for_benefactor(&long);
        let _ = repay_feed_line(&long, &long);
    }
}
