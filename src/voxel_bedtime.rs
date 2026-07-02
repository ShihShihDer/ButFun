//! 乙太方界·居民就寢反思 v1（作息，ROADMAP 744）。
//!
//! **設計依據**：睡覺 v1（739，`voxel_sleep`）讓居民深夜回到自家躺下、頭頂冒 💤、天亮才醒——
//! 世界第一次有了「入夜就寢」的作息輪廓。但那個入睡的瞬間，居民念的只是三選一的**通用**
//! 睡前語（「睏了……回家睡了。」），跟牠今天實際經歷了什麼毫無關係——不管今天是被玩家教了
//! 一手燒玻璃、還是和奧瑞一起整平了一片地，躺下時說的都一樣。世界的夜晚少了最有生活感的
//! 一筆：**睡前，回味今天最有感的那件事。**
//!
//! 本模組把這個節拍補上：居民入睡時，會從「今天的近況記憶」裡挑一筆最值得回味的（優先高價值
//! 的目標／偏好／承諾／人際事件，同級取最近），據此冒一句**個人化的反思泡泡**，並把這份反思
//! **昇華成一筆「睡前反思」記憶**存進記憶庫、同時記進動態 Feed——沒在線上的玩家隔天回來，也
//! 讀得到「露娜昨晚睡前回味著：今天和奧瑞一起整地」。記憶第一次驅動了「就寢」這個作息節拍，
//! 居民不只是躺下睡著，而是**帶著今天的心境入睡**。
//!
//! **與既有元素的定位區隔**：睡覺 v1（739）管「到家 → 躺下 → 天亮醒」的狀態機；本模組只在
//! 「躺下」那一刻多做一件事——回味今天。記憶回想泡泡（`recall_bubble`）是**友人靠近時**主動
//! 說出「我記得你說過…」（社交觸發），本模組是**獨自入睡時**的內省回味（作息觸發），一個對外
//! 一個對內、觸發點與語氣都不同。反思昇華成的記憶掛在專屬偽玩家標籤下（[`REFLECT_MEMORY_PLAYER`]），
//! 不會被誤當成某位玩家的互動、也不會回頭回味自己。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式；記憶讀寫、鎖存取、
//! Feed 廣播都在 `voxel_ws.rs`（沿用睡覺 v1 / 讀牌 v3 的短鎖循序手法）。

/// 「睡前反思」記憶掛的偽玩家標籤（比照讀牌 v3 的 `SIGN_MEMORY_PLAYER`）：讓反思記憶可被辨識、
/// 不會被誤算進某位玩家的好感度、挑「今天回味哪件事」時也會跳過自己昨晚的反思（不遞迴回味）。
pub const REFLECT_MEMORY_PLAYER: &str = "__voxel_bedtime__";

/// 反思冒泡與昇華記憶摘要的字元上限（比照其他泡泡台詞）。
pub const REFLECT_MAX_CHARS: usize = 40;

/// 把記憶摘要縮成「反思核心」的字元上限：太長會撐爆泡泡、也讀不完，
/// 比照讀牌引文（`READ_QUOTE_CHARS = 14`）取一個略寬、讀得順的量級。
pub const REFLECT_CORE_CHARS: usize = 16;

/// 入睡時「回味今天」而非只念通用睡前語的機率：多數夜晚都會回味（前提是有可回味的記憶），
/// 偶爾只是單純睡下——讀起來不機械、不會每晚同一套。
pub const REFLECT_CHANCE: f32 = 0.75;

/// 挑「今天最值得回味的一筆」時，最多往回看幾筆近況記憶（別翻太舊、當成今天的事回味）。
pub const RECENT_WINDOW: usize = 8;

/// 是否要在入睡時回味今天：有可回味的記憶 + 過機率門檻。
/// `roll` 由呼叫端以 `rand::random::<f32>()` 取真隨機餵入（與本專案其他機率骰同慣例）。
pub fn should_reflect(has_memory: bool, roll: f32) -> bool {
    has_memory && roll < REFLECT_CHANCE
}

/// 從近況記憶清單挑「最值得回味」的一筆，回傳其索引（空清單回 `None`）。
///
/// 傳入 `(是否高價值, seq)` 清單（呼叫端已從記憶庫取好、每筆是否 `Importance::Persistent`
/// 用 `classify_importance` 判定）；規則：**先比是否高價值**（目標／偏好／承諾等 persistent 事實
/// 優先於寒暄瑣事），**同級再取最近**（seq 越大越新）——挑出的就是「今天最有感的那件事」。
pub fn most_memorable(items: &[(bool, u64)]) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.0.cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|(i, _)| i)
}

/// 把記憶摘要縮成一段可嵌進泡泡／Feed 的「反思核心」（去頭尾空白 + 截斷）。
fn trim_core(memory_summary: &str) -> String {
    memory_summary
        .trim()
        .chars()
        .take(REFLECT_CORE_CHARS)
        .collect()
}

/// 入睡時回味今天的一句反思泡泡（面向玩家，集中可 i18n）。以今天最有感的記憶摘要為核心。
pub fn reflect_bubble(memory_summary: &str, pick: usize) -> String {
    let core = trim_core(memory_summary);
    let line = match pick % 3 {
        0 => format!("今天啊……{core}，帶著這份心情睡了。"),
        1 => format!("躺下前想起今天：{core}。晚安。"),
        _ => format!("今天最記得的是——{core}。好好睡～"),
    };
    line.chars().take(REFLECT_MAX_CHARS).collect()
}

/// 昇華成「睡前反思」記憶的摘要文字（存進記憶庫、日後翻居民日記也看得到這一頁）。
pub fn reflect_memory_summary(memory_summary: &str) -> String {
    let core = trim_core(memory_summary);
    format!("💤睡前回味：{core}")
}

/// 睡前反思寫進動態 Feed 的一句（讓非同步回訪的玩家讀得到居民昨晚睡前的心境）。
pub fn reflect_feed_line(memory_summary: &str) -> String {
    let core = trim_core(memory_summary);
    format!("睡前回味著今天：{core}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_reflect_needs_memory_and_passes_chance() {
        // 有記憶 + roll 低於門檻 → 回味。
        assert!(should_reflect(true, REFLECT_CHANCE - 0.01));
        // roll 達門檻（含）→ 這晚只是單純睡下。
        assert!(!should_reflect(true, REFLECT_CHANCE));
        assert!(!should_reflect(true, 0.99));
        // 沒有可回味的記憶 → 無論 roll 多低都不回味。
        assert!(!should_reflect(false, 0.0));
    }

    #[test]
    fn most_memorable_prefers_persistent_then_recent() {
        // 空清單 → None。
        assert_eq!(most_memorable(&[]), None);
        // 全寒暄：取最近（seq 最大）那筆。
        assert_eq!(most_memorable(&[(false, 3), (false, 9), (false, 5)]), Some(1));
        // 有高價值：即使不是最新，也優先高價值那筆。
        assert_eq!(most_memorable(&[(false, 100), (true, 2), (false, 50)]), Some(1));
        // 多筆高價值：取其中最近（seq 大）那筆。
        assert_eq!(most_memorable(&[(true, 4), (false, 99), (true, 8)]), Some(2));
        // 單筆 → 就是牠。
        assert_eq!(most_memorable(&[(false, 7)]), Some(0));
    }

    #[test]
    fn bubbles_vary_with_pick_and_are_bounded() {
        let s = "和奧瑞一起把那片地整平了";
        let a = reflect_bubble(s, 0);
        let b = reflect_bubble(s, 1);
        let c = reflect_bubble(s, 2);
        assert_ne!(a, b);
        assert_ne!(b, c);
        // 三句都非空、都夾在字元上限內、都嵌進了記憶核心的開頭。
        for line in [&a, &b, &c] {
            assert!(!line.is_empty());
            assert!(line.chars().count() <= REFLECT_MAX_CHARS);
        }
        // pick 取模不越界。
        let _ = reflect_bubble(s, usize::MAX);
    }

    #[test]
    fn long_memory_is_trimmed_into_core() {
        // 遠超上限的長記憶：核心被截到 REFLECT_CORE_CHARS，泡泡整體仍在上限內。
        let long: String = "很".repeat(200);
        let bubble = reflect_bubble(&long, 0);
        assert!(bubble.chars().count() <= REFLECT_MAX_CHARS);
        let mem = reflect_memory_summary(&long);
        // 昇華記憶帶固定前綴 + 至多 REFLECT_CORE_CHARS 個核心字。
        assert!(mem.starts_with("💤睡前回味："));
        assert!(mem.chars().count() <= "💤睡前回味：".chars().count() + REFLECT_CORE_CHARS);
    }

    #[test]
    fn memory_and_feed_lines_embed_core_and_are_nonempty() {
        let s = "玩家教我燒玻璃";
        let mem = reflect_memory_summary(s);
        let feed = reflect_feed_line(s);
        assert!(mem.contains("燒玻璃"));
        assert!(feed.contains("燒玻璃"));
        assert!(!mem.is_empty() && !feed.is_empty());
    }

    #[test]
    fn empty_or_whitespace_memory_does_not_panic() {
        // 邊界：空字串／全空白不 panic，泡泡仍是合法字串。
        let _ = reflect_bubble("", 0);
        let _ = reflect_bubble("   ", 1);
        let _ = reflect_memory_summary("");
        let _ = reflect_feed_line("   ");
    }
}
