//! 乙太方界·居民晨間思念玩家 v1（作息 × 記憶驅動行為，ROADMAP 746）。
//!
//! **設計依據**：晨間探友 v1（745，`voxel_morning`）讓居民醒來讀昨晚昇華的「睡前反思」記憶
//! （744），若那份牽掛裡出現另一位**居民**的名字，今天第一件事就是走去找他——記憶第一次
//! 改變了居民今天的去向。但這一刀只認得「居民」：當你昨天教了露娜燒玻璃、或陪牠一起種田，
//! 牠睡前回味、昇華成記憶的那件最有感的事，主角其實是**你（玩家）**；可 745 掃反思時只比對
//! 居民名單，撞不到玩家，於是牠醒來對「昨晚一直想著你」這件事毫無反應，只念一句通用早安。
//! 世界的清晨少了對玩家最直接的一筆溫柔：**居民帶著昨晚對你的牽掛醒來、一早就來找你。**
//!
//! 本模組把這一環補上——這是路線圖「②記憶→行為」把觸角伸向玩家的一刀，也是與 745 最本質的
//! 對稱補完：**745 是記憶讓居民去找另一位居民，本模組是記憶讓居民來找「你」。** 居民醒來讀
//! 睡前反思，若那份牽掛裡出現了某位**此刻在線**玩家的名字，牠今天的第一件事就是——放下平常的
//! 閒晃／採集，朝你走過來，抵達時暖暖打一聲招呼、並把「今早特地來找你」記成一筆與你的記憶。
//! 玩家一上線就能撞見：「露娜昨晚睡前一直惦記著你教牠的燒玻璃，天一亮就往你這走來了。」——
//! 記憶第一次不只帶著居民去找另一位居民，而是**把牠的腳步帶到了你面前。**
//!
//! **與既有元素的定位區隔**：孤獨尋伴（678，`voxel_comfort`）是**心情驅動**的求陪——Lonely 時
//! 走向「最近的任一玩家」求人陪；本模組是**記憶指名**的思念——去找的正是昨晚睡前惦記的**那一位
//! 特定玩家**（縱使他不是最近的、縱使居民此刻並不孤獨），觸發來源（心情 vs 昨晚的記憶）與敘事
//! （求陪 vs 惦記著你來找你）都不同。晨間探友（745）去找的是居民、本模組去找的是玩家。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式；記憶讀取、鎖存取、
//! 朝玩家走的狀態機、記憶昇華與 Feed 廣播都在 `voxel_ws.rs`（沿用晨間探友 / 打氣走動的短鎖手法）。

/// 醒來時「因昨晚惦記著某位在線玩家而動身來找他」而非只念通用早安的機率：多數帶著牽掛的清晨
/// 都會動身（前提是昨晚的反思裡真的惦記到某位此刻在線的玩家），偶爾只是單純起床——讀起來不機械。
pub const MISS_CHANCE: f32 = 0.7;

/// 晨思泡泡台詞的字元上限（比照其他泡泡台詞）。
pub const GREET_MAX_CHARS: usize = 40;

/// 抵達玩家身邊的判定距離（世界座標，平方比較用）：落在此半徑內即視為「走到你面前了」。
/// 比照打氣走動的抵達距離量級，讓玩家清楚看見牠是專程走到跟前才打招呼。
pub const ARRIVE_DIST: f32 = 2.2;

/// 朝玩家走的逾時秒數：啟程時設此值、每 tick 遞減；走太久（地形擋路等）還沒到就放下這份牽掛，
/// 不無限追著跑（玩家也可能一直在移動）。稀少事件、寬裕即可。
pub const SEEK_TIMEOUT_SECS: f32 = 45.0;

/// 是否要在醒來時因昨晚的牽掛而動身來找玩家：昨晚的反思裡有惦記到某位在線玩家 + 過機率門檻。
/// `roll` 由呼叫端以 `rand::random::<f32>()` 取真隨機餵入（與本專案其他機率骰同慣例）。
pub fn should_miss(has_reflection_player: bool, roll: f32) -> bool {
    has_reflection_player && roll < MISS_CHANCE
}

/// 從昨晚的「睡前反思」記憶摘要裡，找出被惦記到的那位**此刻在線**玩家，回傳其在 `player_names`
/// 中的索引。
///
/// - `summary`：昨晚昇華的睡前反思記憶摘要（如「💤睡前回味：小美教我燒玻璃」）。
/// - `player_names`：此刻在線玩家的顯示名（順序需與呼叫端的座標快照對齊，讓回傳索引可直接查座標）。
///
/// 規則：掃出所有在摘要中出現的在線玩家名（排除空名），取**在摘要裡最早出現**的那位
/// （最靠前 ≈ 這句話的主角）。沒有任何在線玩家被提到 → `None`（退回通用早安、不動身）。
/// **只比對在線玩家**——這是本函式的安全閥：離線玩家不在 `player_names` 裡、絕不會誤中，
/// 也讓「去找的人此刻真的在世界裡」這件事在偵測階段就成立。純函式、確定性、無 IO。
pub fn mentioned_player(summary: &str, player_names: &[&str]) -> Option<usize> {
    // best = (在摘要中的位元組起點, player_names 索引)；取起點最小者。
    let mut best: Option<(usize, usize)> = None;
    for (i, &n) in player_names.iter().enumerate() {
        if n.is_empty() {
            continue;
        }
        if let Some(pos) = summary.find(n) {
            match best {
                Some((bp, _)) if pos >= bp => {} // 已有更早出現的名字，保留原本
                _ => best = Some((pos, i)),
            }
        }
    }
    best.map(|(_, i)| i)
}

/// 醒來動身去找昨晚惦記的玩家時，冒的一句晨思泡泡（面向玩家，集中可 i18n）。
pub fn wake_bubble(player: &str, pick: usize) -> String {
    let line = match pick % 3 {
        0 => format!("昨晚睡前一直惦記著{player}，一醒來就想去找他～"),
        1 => format!("早安！睜開眼還想著{player}，這就去看看他。"),
        _ => format!("一夜好眠，忽然好想見見{player}——這就去。"),
    };
    line.chars().take(GREET_MAX_CHARS).collect()
}

/// 走到玩家面前時，暖暖打的一聲招呼（面向玩家，集中可 i18n）。點名對方、點出「一早特地來找你」。
pub fn arrive_greet_bubble(player: &str, pick: usize) -> String {
    let line = match pick % 3 {
        0 => format!("{player}！我昨晚睡前一直想著你，一早就來找你啦～"),
        1 => format!("找到你了，{player}！醒來滿腦子都是你，特地過來的。"),
        _ => format!("{player}，早安呀！一睜眼就惦記著你，這不就來了。"),
    };
    line.chars().take(GREET_MAX_CHARS).collect()
}

/// 「今早特地來找你」昇華成的一筆記憶摘要（掛在該玩家名下，算進與你的情誼）。
pub fn miss_memory_summary(player: &str) -> String {
    format!("今天一早醒來還惦記著{player}，特地走過去找了他")
}

/// 晨間思念玩家寫進動態 Feed 的一句（讓非同步回訪的玩家也讀得到「牠一早惦記著誰、去找了誰」）。
pub fn miss_feed_line(player: &str) -> String {
    format!("醒來惦記著昨晚的事，一早就往{player}走去了")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYERS: [&str; 3] = ["小美", "阿宏", "旅人"];

    #[test]
    fn should_miss_needs_player_and_passes_chance() {
        // 有惦記到的在線玩家 + roll 低於門檻 → 動身。
        assert!(should_miss(true, MISS_CHANCE - 0.01));
        // roll 達門檻（含）→ 這天只是單純起床。
        assert!(!should_miss(true, MISS_CHANCE));
        assert!(!should_miss(true, 0.99));
        // 昨晚沒惦記到任何在線玩家 → 無論 roll 多低都不動身。
        assert!(!should_miss(false, 0.0));
    }

    #[test]
    fn mentioned_player_finds_the_player() {
        // 反思裡提到小美 → 回小美的索引。
        let s = "💤睡前回味：小美教我燒玻璃";
        assert_eq!(mentioned_player(s, &PLAYERS), Some(0));
    }

    #[test]
    fn mentioned_player_picks_earliest_in_summary() {
        // 同時提到兩位：取在摘要裡最早出現的那位（阿宏在前 → 索引 1，非旅人的 2）。
        let s = "💤睡前回味：阿宏和旅人都來看過我";
        assert_eq!(mentioned_player(s, &PLAYERS), Some(1));
    }

    #[test]
    fn mentioned_player_none_when_no_online_player() {
        // 反思只跟事情／居民有關、沒提到任何在線玩家 → None（離線玩家不在名單、絕不誤中）。
        let s = "💤睡前回味：和奧瑞一起整地";
        assert_eq!(mentioned_player(s, &PLAYERS), None);
        // 空摘要 → None、不 panic。
        assert_eq!(mentioned_player("", &PLAYERS), None);
    }

    #[test]
    fn mentioned_player_skips_empty_names() {
        // 名單含空字串（保守退化，如未命名訪客）：不誤中、不 panic。
        let names = ["", "阿宏", ""];
        assert_eq!(mentioned_player("💤睡前回味：找阿宏聊天", &names), Some(1));
        // 全空名單 → None。
        assert_eq!(mentioned_player("💤睡前回味：小美教我燒玻璃", &["", ""]), None);
    }

    #[test]
    fn wake_and_arrive_bubbles_vary_and_are_bounded() {
        for pick in 0..3usize {
            let w = wake_bubble("小美", pick);
            let a = arrive_greet_bubble("小美", pick);
            for line in [&w, &a] {
                assert!(!line.is_empty());
                assert!(line.chars().count() <= GREET_MAX_CHARS);
                assert!(line.contains("小美"));
            }
        }
        // 三種 pick 的醒來泡泡彼此不同（不機械）。
        assert_ne!(wake_bubble("小美", 0), wake_bubble("小美", 1));
        assert_ne!(wake_bubble("小美", 1), wake_bubble("小美", 2));
        // pick 取模不越界。
        let _ = wake_bubble("小美", usize::MAX);
        let _ = arrive_greet_bubble("小美", usize::MAX);
    }

    #[test]
    fn memory_and_feed_embed_player_and_are_nonempty() {
        let mem = miss_memory_summary("旅人");
        assert!(mem.contains("旅人"));
        assert!(!mem.is_empty());
        let feed = miss_feed_line("旅人");
        assert!(feed.contains("旅人"));
        assert!(!feed.is_empty());
    }

    #[test]
    fn arrive_greet_bubbles_vary_with_pick() {
        assert_ne!(arrive_greet_bubble("阿宏", 0), arrive_greet_bubble("阿宏", 1));
        assert_ne!(arrive_greet_bubble("阿宏", 1), arrive_greet_bubble("阿宏", 2));
    }
}
