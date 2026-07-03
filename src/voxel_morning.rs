//! 乙太方界·居民晨間探友 v1（作息 × 記憶驅動行為，ROADMAP 745）。
//!
//! **設計依據**：就寢反思 v1（744，`voxel_bedtime`）讓居民入睡時回味今天最有感的一件事、
//! 把那份心境昇華成一筆「睡前反思」記憶存進記憶庫——但那份牽掛到早上就船過水無痕，居民醒來
//! 念的還是三選一的通用早安（睡覺 v1，`voxel_sleep::wake_line`），昨晚想著誰、惦記著什麼，
//! 對「今天要做什麼」毫無影響。世界的清晨少了最關鍵的一環：**記憶真的改變了居民今天的去向。**
//!
//! 本模組把這一環補上——這是路線圖「②記憶→行為」的一刀，也是與過往「居民念一句話」切片
//! 最本質的區隔：**它不只是再冒一個泡泡，而是讓昨晚的一筆記憶，真的決定了居民今天先往哪裡走。**
//! 居民醒來時，讀昨晚昇華的「睡前反思」記憶，若那份牽掛裡出現了另一位居民的名字，牠今天的
//! 第一件事就是——放下平常的閒晃／採集，走去對方家域找他敘敘（沿用探訪 v1 的抵達／問候／
//! 情誼／Feed 狀態機，零協議改動）。玩家一早上線就能撞見：「露娜昨晚睡前一直想著奧瑞，
//! 天一亮就往奧瑞家走去了」——記憶第一次不只被說出來，而是**把居民的腳步帶去了某個地方。**
//!
//! **與既有元素的定位區隔**：探訪 v1（671）是**情誼加權的隨機**造訪（老朋友更常被抽中，但誰去
//! 找誰是擲骰決定的）；本模組是**記憶指名的**探訪（去找的正是昨晚睡前惦記的那個人，非隨機）——
//! 一個是「關係傾向」、一個是「昨晚的心事」，觸發來源與敘事都不同。小圈子聚會（711）是三人以上
//! 同時湊到一塊；本模組是一對一、由單筆記憶驅動的清晨探訪。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式；記憶讀取、鎖存取、
//! 探訪狀態機接管、Feed 廣播都在 `voxel_ws.rs`（沿用睡覺 v1 / 就寢反思 v1 的短鎖循序手法）。

/// 醒來時「因昨晚的牽掛而動身探友」而非只念通用早安的機率：多數帶著牽掛的清晨都會動身
/// （前提是昨晚的反思裡真的有惦記到某位居民），偶爾只是單純起床——讀起來不機械、不會每天同一套。
pub const SEEK_CHANCE: f32 = 0.7;

/// 晨想探友泡泡台詞的字元上限（比照其他泡泡台詞）。
pub const SEEK_MAX_CHARS: usize = 40;

/// 是否要在醒來時因昨晚的牽掛而動身探友：昨晚的反思裡有惦記到某位居民 + 過機率門檻。
/// `roll` 由呼叫端以 `rand::random::<f32>()` 取真隨機餵入（與本專案其他機率骰同慣例）。
pub fn should_seek(has_reflection_friend: bool, roll: f32) -> bool {
    has_reflection_friend && roll < SEEK_CHANCE
}

/// 從昨晚的「睡前反思」記憶摘要裡，找出被惦記到的那位居民，回傳其在 `names` 中的索引。
///
/// - `summary`：昨晚昇華的睡前反思記憶摘要（如「💤睡前回味：和奧瑞一起整地」）。
/// - `names`：所有居民的顯示名（順序需與呼叫端的家域快照對齊，讓回傳索引可直接查家域）。
/// - `self_name`：自己的名字——**永遠跳過自己**（不會「醒來想去找自己」）。
///
/// 規則：掃出所有在摘要中出現的居民名（排除自己與空名），取**在摘要裡最早出現**的那位
/// （最靠前 ≈ 這句話的主角）。沒有任何居民被提到 → `None`（退回通用早安、不動身）。
/// 純函式、確定性（同輸入恆同輸出），無 IO。
pub fn mentioned_resident(summary: &str, names: &[&str], self_name: &str) -> Option<usize> {
    // best = (在摘要中的位元組起點, names 索引)；取起點最小者。
    let mut best: Option<(usize, usize)> = None;
    for (i, &n) in names.iter().enumerate() {
        if n.is_empty() || n == self_name {
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

/// 醒來動身去找昨晚惦記的人時，冒的一句晨想泡泡（面向玩家，集中可 i18n）。
pub fn seek_bubble(friend: &str, pick: usize) -> String {
    let line = match pick % 3 {
        0 => format!("昨晚睡前一直惦記著{friend}，今天先去看看他吧～"),
        1 => format!("醒來還想著{friend}，早上第一件事就是去找他。"),
        _ => format!("一夜好眠，忽然好想見見{friend}——這就去。"),
    };
    line.chars().take(SEEK_MAX_CHARS).collect()
}

/// 晨間探友寫進動態 Feed 的一句（讓非同步回訪的玩家讀得到「牠一早惦記著誰、去找了誰」）。
pub fn seek_feed_line(friend: &str) -> String {
    format!("醒來惦記著昨晚的事，一早就往{friend}家走去了")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 4] = ["露娜", "諾娃", "賽勒", "奧瑞"];

    #[test]
    fn should_seek_needs_friend_and_passes_chance() {
        // 有惦記到的居民 + roll 低於門檻 → 動身。
        assert!(should_seek(true, SEEK_CHANCE - 0.01));
        // roll 達門檻（含）→ 這天只是單純起床。
        assert!(!should_seek(true, SEEK_CHANCE));
        assert!(!should_seek(true, 0.99));
        // 昨晚沒惦記到任何居民 → 無論 roll 多低都不動身。
        assert!(!should_seek(false, 0.0));
    }

    #[test]
    fn mentioned_resident_finds_the_friend() {
        // 反思裡提到奧瑞 → 回奧瑞的索引。
        let s = "💤睡前回味：和奧瑞一起整地";
        assert_eq!(mentioned_resident(s, &NAMES, "露娜"), Some(3));
    }

    #[test]
    fn mentioned_resident_skips_self() {
        // 只提到自己 → None（不會醒來想去找自己）。
        let s = "💤睡前回味：露娜今天學會了燒玻璃";
        assert_eq!(mentioned_resident(s, &NAMES, "露娜"), None);
    }

    #[test]
    fn mentioned_resident_picks_earliest_in_summary() {
        // 同時提到兩位：取在摘要裡最早出現的那位（諾娃在前 → 索引 1，非奧瑞的 3）。
        let s = "💤睡前回味：諾娃和奧瑞都來幫忙了";
        assert_eq!(mentioned_resident(s, &NAMES, "露娜"), Some(1));
    }

    #[test]
    fn mentioned_resident_none_when_no_name() {
        // 反思只跟事情有關、沒提到任何居民 → None。
        let s = "💤睡前回味：把那片地整平了";
        assert_eq!(mentioned_resident(s, &NAMES, "露娜"), None);
        // 空摘要 → None、不 panic。
        assert_eq!(mentioned_resident("", &NAMES, "露娜"), None);
    }

    #[test]
    fn mentioned_resident_skips_empty_names() {
        // 名字清單含空字串（保守退化）：不誤中、不 panic。
        let names = ["", "奧瑞", ""];
        assert_eq!(mentioned_resident("💤睡前回味：找奧瑞聊天", &names, "露娜"), Some(1));
    }

    #[test]
    fn seek_bubbles_vary_with_pick_and_are_bounded() {
        let a = seek_bubble("奧瑞", 0);
        let b = seek_bubble("奧瑞", 1);
        let c = seek_bubble("奧瑞", 2);
        assert_ne!(a, b);
        assert_ne!(b, c);
        for line in [&a, &b, &c] {
            assert!(!line.is_empty());
            assert!(line.chars().count() <= SEEK_MAX_CHARS);
            assert!(line.contains("奧瑞"));
        }
        // pick 取模不越界。
        let _ = seek_bubble("奧瑞", usize::MAX);
    }

    #[test]
    fn seek_feed_line_embeds_friend_and_is_nonempty() {
        let feed = seek_feed_line("諾娃");
        assert!(feed.contains("諾娃"));
        assert!(!feed.is_empty());
    }
}
