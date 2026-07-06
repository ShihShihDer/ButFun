//! 乙太方界·戀人牽掛 v1（ROADMAP 852）——居民戀愛心動（846）讓兩位老朋友締結成戀人，但成了
//! 戀人之後，這份羈絆從沒有改變過任何行為：平常閒晃、作息，跟締結前一模一樣，戀人身份只是
//! 交情網面板上一枚安靜的 ❤️ 標籤。本模組讓「戀人」第一次真的影響行為：分開得夠遠、又冷卻到期、
//! 戀人醒著、過機率門檻 → 一位戀人會放下手邊的事，起身走去找對方，重逢那一刻雙方各自留下一筆
//! 暖記憶。**記憶驅動行為**（北極星）第一次接上浪漫軸線——戀人不只是稱謂，是會讓人牽掛、
//! 讓腳步轉向的關係。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **久別重逢奔迎（`voxel_reunion`，747）／晨間思念（`voxel_daybreak`，746）**＝**居民↔玩家**、
//!   由「玩家登入／破曉」事件觸發的一次性奔赴；本模組＝**居民↔居民**、由「戀人分開太久」持續
//!   周期性判定觸發，觸發者與對象皆不同，機制沿用同一套「逐 tick 重查目標座標步步逼近」手法但
//!   完全獨立的狀態欄位與觸發條件，不共用、不重疊。
//! - **戀愛心動（`voxel_romance`，846）**＝締結戀人的那一刻（一次性質變）；本模組＝締結**之後**，
//!   戀人這個身份在日常裡持續發揮的效果（重複性行為），兩者前後接續、非同軸重複。
//!
//! 純邏輯層（無 IO、無鎖、無 async、無 LLM），IO／狀態接線在 `voxel_ws.rs`。純記憶體、
//! 重啟歸零（比照 `voxel_reunion`/`voxel_daybreak` 的 `reunion_seek`/`daybreak_seek` 慣例，
//! 這份牽掛本就是「當下」的心情，不必persist）。

/// 兩位戀人之間的 XZ 距離平方，超過這個門檻才算「分開得夠遠」，值得放下手邊的事去找對方
/// （避免明明近在咫尺也頻繁觸發，顯得太黏）。
pub const MIN_APART_DIST: f32 = 10.0;

/// 一次牽掛落幕（抵達或放棄）之後的靜置冷卻秒數：天然節流，避免同一對戀人反覆觸發洗版。
pub const SEEK_COOLDOWN_SECS: f32 = 240.0;

/// 起身去找戀人之後，最長願意奔走多久（逾時代表對方走得比追得上的還快 / 途中出了什麼變化，
/// 放下這份牽掛、回到平常的一天，而不是無止盡追下去）。
pub const SEEK_TIMEOUT_SECS: f32 = 60.0;

/// 每次符合「分開夠遠 + 冷卻到期 + 戀人醒著」時，真的起念去找對方的機率。刻意不高：
/// 多數時候戀人只是各自忙自己的事，偶爾才會忽然很想見對方一面。
pub const SEEK_CHANCE: f32 = 0.12;

/// 走到這個距離內就算「找到了」，停下相見。
pub const ARRIVE_DIST: f32 = 2.5;

/// 是否該放下手邊的事、起身去找戀人（純函式、`roll` 由呼叫端 `rand::random::<f32>()` 提供，
/// 確定可測）：戀人必須醒著（不吵醒對方）、分開得夠遠（不是黏在旁邊也觸發）、冷卻已到期、
/// 且過機率門檻。
pub fn should_seek(dist_sq: f32, cooldown_secs: f32, partner_asleep: bool, roll: f32) -> bool {
    !partner_asleep
        && cooldown_secs <= 0.0
        && dist_sq > MIN_APART_DIST * MIN_APART_DIST
        && roll < SEEK_CHANCE
}

/// 起身出發時冒的心聲（不點名戀人——這句只是「我要去找他」的自白，抵達才是重逢那一刻的驚喜）。
const SEEK_LINES: [&str; 6] = [
    "忽然好想見他一面，我去找他。",
    "手邊的事先放一放，我想去看看他在做什麼。",
    "有點想他了，去找他說說話吧。",
    "不知道怎麼回事，就是很想去見他。",
    "想他了，這就過去找他。",
    "先去找他一下，很快回來。",
];

/// 抵達戀人面前那一刻的暖招呼。
const ARRIVE_LINES: [&str; 5] = [
    "終於找到你了！",
    "來找你啦，剛剛好想你。",
    "看到你，就安心了。",
    "跑來找你，沒有打擾到你吧？",
    "想你了，過來看看你。",
];

/// 出發時的心聲（依 `pick` 輪替，確定性可測）。
pub fn seek_bubble_line(pick: usize) -> &'static str {
    SEEK_LINES[pick % SEEK_LINES.len()]
}

/// 抵達時的暖招呼（依 `pick` 輪替，確定性可測）。
pub fn arrive_greet_line(pick: usize) -> &'static str {
    ARRIVE_LINES[pick % ARRIVE_LINES.len()]
}

/// 重逢那一刻，雙方各自寫進記憶的一句（掛在對方名下；空名退泛稱，不留「，，」）。
pub fn arrive_memory_line(other_name: &str) -> String {
    let who = if other_name.is_empty() { "他" } else { other_name };
    format!("忽然好想{who}，放下手邊的事跑去找{who}，見到面那一刻，心就定了。")
}

/// 重逢那一刻的城鎮動態牆播報（空名退泛稱）。
pub fn arrive_feed_line(seeker: &str, partner: &str) -> String {
    let s = if seeker.is_empty() { "一位居民" } else { seeker };
    let p = if partner.is_empty() { "戀人" } else { partner };
    format!("{s}忽然放下手邊的事，跑去找戀人{p}，兩人相視一笑，安心了。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_seek_requires_partner_awake() {
        let far = MIN_APART_DIST * MIN_APART_DIST + 1.0;
        assert!(!should_seek(far, 0.0, true, 0.0));
        assert!(should_seek(far, 0.0, false, 0.0));
    }

    #[test]
    fn should_seek_requires_apart_far_enough() {
        let just_under = MIN_APART_DIST * MIN_APART_DIST - 1.0;
        let just_over = MIN_APART_DIST * MIN_APART_DIST + 1.0;
        assert!(!should_seek(just_under, 0.0, false, 0.0));
        assert!(should_seek(just_over, 0.0, false, 0.0));
    }

    #[test]
    fn should_seek_requires_cooldown_ready() {
        let far = MIN_APART_DIST * MIN_APART_DIST + 1.0;
        assert!(!should_seek(far, 1.0, false, 0.0));
        assert!(should_seek(far, 0.0, false, 0.0));
        assert!(should_seek(far, -1.0, false, 0.0));
    }

    #[test]
    fn should_seek_chance_boundary() {
        let far = MIN_APART_DIST * MIN_APART_DIST + 1.0;
        assert!(should_seek(far, 0.0, false, SEEK_CHANCE - 0.001));
        assert!(!should_seek(far, 0.0, false, SEEK_CHANCE));
        assert!(!should_seek(far, 0.0, false, 0.999));
    }

    #[test]
    fn seek_and_arrive_lines_are_nonempty_and_vary() {
        let seek_a = seek_bubble_line(0);
        let seek_b = seek_bubble_line(1);
        assert!(!seek_a.is_empty());
        assert_ne!(seek_a, seek_b);
        assert!(seek_bubble_line(SEEK_LINES.len() + 2).len() > 0); // pick 溢出取模不 panic

        let arrive_a = arrive_greet_line(0);
        let arrive_b = arrive_greet_line(1);
        assert!(!arrive_a.is_empty());
        assert_ne!(arrive_a, arrive_b);
        assert!(arrive_greet_line(ARRIVE_LINES.len() + 5).len() > 0);
    }

    #[test]
    fn arrive_memory_line_embeds_name_or_falls_back() {
        let named = arrive_memory_line("奧瑞");
        assert!(named.contains("奧瑞"));
        let empty = arrive_memory_line("");
        assert!(empty.contains("他"));
        assert!(!empty.is_empty());
    }

    #[test]
    fn arrive_feed_line_embeds_both_names_or_falls_back() {
        let both = arrive_feed_line("露娜", "奧瑞");
        assert!(both.contains("露娜"));
        assert!(both.contains("奧瑞"));
        let empty = arrive_feed_line("", "");
        assert!(empty.contains("一位居民"));
        assert!(empty.contains("戀人"));
    }

    #[test]
    fn long_names_do_not_panic() {
        let long = "很".repeat(200);
        let _ = arrive_memory_line(&long);
        let _ = arrive_feed_line(&long, &long);
    }
}
