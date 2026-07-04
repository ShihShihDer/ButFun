//! 乙太方界·居民會做夢 v1（作息 × 記憶驅動行為，ROADMAP 805）。
//!
//! **設計依據**：睡覺 v1（739，`voxel_sleep`）讓居民深夜回到自家躺下、頭頂冒 💤、天亮才醒；
//! 就寢反思 v1（744，`voxel_bedtime`）又讓居民在**躺下那一刻**有意識地回味今天最有感的一件事。
//! 但那之後、整個漫漫長夜，居民就只是安靜地躺著、什麼也沒有——世界的睡眠少了最後一筆最有
//! 「內心活著」的節拍：**睡著之後，會做夢。** 一段一直放在心底的珍貴往事，會在夜裡不由自主地
//! 浮成一個夢。
//!
//! 本模組把這個節拍補上：居民熟睡中，偶爾會從**整座記憶庫裡挑一段珍貴（persistent）的往事**
//! 浮現成夢，冒一個「💤 夢見…」的泡泡、記進城鎮動態 Feed——夜裡路過的玩家會瞥見熟睡居民
//! 頭頂飄著一個夢，離線回訪的玩家隔天也讀得到「露娜昨晚夢見了那天……」。這是北極星
//! 「**記憶是讓居民真的活著的土壤**」最純粹的一種呈現：記憶不只用來聊天、驅動白天的行為，
//! 連睡夢裡都在悄悄浮現——居民的內心，連睡著時都活著。
//!
//! **與 744 就寢反思的定位區隔（razor-sharp，不同軸）**：
//! - **觸發點**：744 在「躺下那一刻」觸發一次；本刀在「已經睡著之後」的深夜偶爾觸發（有冷卻）。
//! - **記憶取樣**：744 只看「今天最近的 8 筆」、取最有感的**一筆最近**；本刀從**整座記憶庫**
//!   （更廣更舊的窗）裡挑**珍貴的往事**，可觸及很久以前一直放在心上的事，且每晚輪替、夢的不
//!   總是同一件（`pick` 隨做夢次數變化）。
//! - **語氣**：744 是有意識的「今天啊……帶著這份心情睡了」回味；本刀是不由自主的「💤 夢見…」
//!   潛意識畫面。
//! 一個對「今天」、一個對「深藏的往事」；一個清醒回味、一個睡夢浮現——觸發點、取樣、語氣皆不同。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式；記憶讀取、鎖存取、
//! Feed 廣播都在 `voxel_ws.rs`（沿用睡覺 v1／就寢反思 v1 的短鎖循序手法，守死鎖鐵律）。
//! 隱私沿用 744 的作法：只取居民**自己已抽象過的記憶摘要**、截成一小段核心（[`DREAM_CORE_CHARS`]），
//! 絕不整包倒出對話謄本。

/// 夢泡與 Feed 文字的字元上限（比照其他泡泡台詞 / `voxel_bedtime::REFLECT_MAX_CHARS`）。
pub const DREAM_MAX_CHARS: usize = 40;

/// 把記憶摘要縮成「夢的核心」的字元上限（比照 `voxel_bedtime::REFLECT_CORE_CHARS`）。
pub const DREAM_CORE_CHARS: usize = 16;

/// 熟睡中、冷卻到期後每個 tick 浮出一個夢的機率。tick 為 10Hz（`RESIDENT_DT=0.1`），
/// 這個機率讓冷卻一過大約幾秒內就會做上一個夢，配合 [`DREAM_COOLDOWN_SECS`] 讓一夜大約
/// 做上幾個夢、不至於每個 tick 都在夢、也不至於整夜無夢。
pub const DREAM_CHANCE: f32 = 0.02;

/// 兩個夢之間的最短間隔（秒）：做完一個夢後要等這麼久才可能再做下一個，避免夢泡洗版。
/// 約 110 秒 ＝ 一夜熟睡大約做上幾個夢的節奏。
pub const DREAM_COOLDOWN_SECS: f32 = 110.0;

/// 做夢時往回翻多少筆記憶當「可夢的往事」候選——取整座 episodic 記憶庫的量級
/// （`voxel_memory::EPISODIC_CAP` 為 24），讓夢可以觸及很舊、一直放在心上的珍藏，
/// 這正是它與 744「只看今天最近 8 筆」的關鍵區隔。
pub const DREAM_WINDOW: usize = 24;

/// 是否在這個 tick 做夢：只看機率骰（是否**有可夢的珍貴往事**由呼叫端讀記憶後另判）。
/// `roll` 由呼叫端以 `rand::random::<f32>()` 取真隨機餵入（與本專案其他機率骰同慣例）。
pub fn should_dream(roll: f32) -> bool {
    roll < DREAM_CHANCE
}

/// 從「可夢的珍貴往事」候選裡挑一段，回傳其索引（空候選回 `None`）。
///
/// 與 744 `most_memorable`（取唯一最有感的一筆）刻意不同：本函式**在所有珍貴往事間輪替**
/// （`pick % count`），讓不同夜晚、同一夜的不同次做夢，夢見的不總是同一件事——夢是流動的。
/// `pick` 由呼叫端摻入「已做過幾個夢」使其逐夢變化（睡著時身體靜止，若只用座標會整夜同一夢）。
pub fn pick_dream(count: usize, pick: usize) -> Option<usize> {
    if count == 0 {
        None
    } else {
        Some(pick % count)
    }
}

/// 把記憶摘要縮成一段可嵌進泡泡／Feed 的「夢的核心」（去頭尾空白 + 截斷）。
fn trim_core(memory_summary: &str) -> String {
    memory_summary
        .trim()
        .chars()
        .take(DREAM_CORE_CHARS)
        .collect()
}

/// 睡夢中浮現的一句夢泡（面向玩家，集中可 i18n）。以一段珍貴往事的記憶摘要為核心。
pub fn dream_bubble(memory_summary: &str, pick: usize) -> String {
    let core = trim_core(memory_summary);
    let line = match pick % 3 {
        0 => format!("💤 夢裡又回到那時候……{core}"),
        1 => format!("睡夢中，{core}……的畫面又浮了上來 💤"),
        _ => format!("💤 夢見了……{core}"),
    };
    line.chars().take(DREAM_MAX_CHARS).collect()
}

/// 做夢寫進動態 Feed 的一句（讓非同步回訪的玩家讀得到「居民昨晚夢見了什麼」）。
pub fn dream_feed_line(memory_summary: &str) -> String {
    let core = trim_core(memory_summary);
    format!("在睡夢裡又回到了那天——{core}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_dream_only_passes_under_chance() {
        // roll 低於門檻 → 做夢。
        assert!(should_dream(DREAM_CHANCE - 0.001));
        assert!(should_dream(0.0));
        // roll 達門檻（含）→ 這個 tick 不做夢。
        assert!(!should_dream(DREAM_CHANCE));
        assert!(!should_dream(0.9));
        assert!(!should_dream(1.0));
    }

    #[test]
    fn pick_dream_rotates_and_handles_empty() {
        // 空候選 → None（無可夢的珍貴往事 ＝ 無夢好眠）。
        assert_eq!(pick_dream(0, 0), None);
        assert_eq!(pick_dream(0, 999), None);
        // 單筆 → 就是牠。
        assert_eq!(pick_dream(1, 0), Some(0));
        assert_eq!(pick_dream(1, 7), Some(0));
        // 多筆 → 隨 pick 輪替（逐夢變化，不整夜同一夢）。
        assert_eq!(pick_dream(3, 0), Some(0));
        assert_eq!(pick_dream(3, 1), Some(1));
        assert_eq!(pick_dream(3, 2), Some(2));
        assert_eq!(pick_dream(3, 3), Some(0));
        // pick 取模不越界（極大值）。
        assert!(pick_dream(3, usize::MAX).is_some());
    }

    #[test]
    fn dream_bubbles_vary_with_pick_and_are_bounded() {
        let s = "和奧瑞一起把那片地整平了";
        let a = dream_bubble(s, 0);
        let b = dream_bubble(s, 1);
        let c = dream_bubble(s, 2);
        assert_ne!(a, b);
        assert_ne!(b, c);
        for line in [&a, &b, &c] {
            assert!(!line.is_empty());
            assert!(line.chars().count() <= DREAM_MAX_CHARS);
            // 每句都嵌進了記憶核心的開頭（夢見的確實是那段往事）。
            assert!(line.contains("整"));
        }
        // pick 取模不越界。
        let _ = dream_bubble(s, usize::MAX);
    }

    #[test]
    fn long_memory_is_trimmed_into_core() {
        // 遠超上限的長記憶：核心被截到 DREAM_CORE_CHARS，泡泡整體仍在上限內。
        let long: String = "很".repeat(200);
        let bubble = dream_bubble(&long, 0);
        assert!(bubble.chars().count() <= DREAM_MAX_CHARS);
        let feed = dream_feed_line(&long);
        // Feed 帶固定語幹 + 至多 DREAM_CORE_CHARS 個核心字。
        let stem = "在睡夢裡又回到了那天——";
        assert!(feed.starts_with(stem));
        assert!(feed.chars().count() <= stem.chars().count() + DREAM_CORE_CHARS);
    }

    #[test]
    fn feed_line_embeds_core_and_is_nonempty() {
        let s = "玩家教我燒玻璃";
        let feed = dream_feed_line(s);
        assert!(feed.contains("燒玻璃"));
        assert!(!feed.is_empty());
    }

    #[test]
    fn empty_or_whitespace_memory_does_not_panic() {
        // 邊界：空字串／全空白不 panic，泡泡與 Feed 仍是合法字串。
        let _ = dream_bubble("", 0);
        let _ = dream_bubble("   ", 1);
        let _ = dream_bubble("　", 2);
        let _ = dream_feed_line("");
        let _ = dream_feed_line("   ");
    }
}
