//! 乙太方界·居民夜晚睡覺 v1（作息，ROADMAP 739）。
//!
//! **設計依據**：晝夜循環（`voxel_time`）與歸巢遮蔽（`should_shelter`，ROADMAP 701）
//! 早就存在——夜裡居民會回到自己蓋的小屋附近、把閒晃半徑縮到緊靠自家。但那之後
//! 她們仍只是「在原地小步磨蹭」，世界的夜晚缺一個最基本、最有生活感的節拍：**真的睡著**。
//! 玩家半夜走過村子，看到的是居民照樣晃來晃去，而不是各自回家、安靜睡下、頭頂冒著 💤。
//!
//! 本模組把這個節拍補上：深夜時，已回到自家附近的居民會**躺下入睡**——停下一切
//! 閒晃／社交／採集／建造，只安靜地待在家、名牌旁顯示 💤；直到黎明才醒來，醒來時
//! 神清氣爽（心情提升一格）、冒一句愉快的早安。世界第一次有了「入夜就寢、天亮起床」
//! 的作息輪廓，居民也因此更像真的「住」在這裡、過著有日夜節奏的生活。
//!
//! **與既有元素的定位區隔**：`should_shelter`（701）決定居民夜裡「往哪閒晃」（縮到自家旁），
//! 本模組決定居民到家後「乾脆睡了」（完全靜止＋可見睡眠狀態）——一個管移動中心，一個管
//! 「停下來睡覺」這個新狀態，層次不同、互補不重疊。心情提升沿用既有 `mood_boost_secs`
//! 機制（681），醒來的好心情用玩家早已熟悉的 emoji 呈現。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式；睡眠狀態欄位、
//! 鎖存取、廣播都在 `voxel_ws.rs`（沿用既有短鎖循序手法）。

/// 睡眠時名牌旁顯示的 emoji（覆蓋原本的心情 emoji，一眼看出「這位在睡」）。
pub const SLEEP_MOOD_EMOJI: &str = "💤";

/// 判定「已靠近自家中心」的水平半徑（格）：只有真的回到家門口才躺下睡，
/// 而不是在半路上就地睡著。與遮蔽半徑（`SHELTER_WANDER_RADIUS = 2.5`）同量級、
/// 略寬一點容納站位誤差。
pub const SLEEP_NEAR_RADIUS: f32 = 4.0;

/// 符合入睡條件後、每次「抵達閒晃點」真的睡著的機率——不是一到家就秒睡，
/// 偶爾還會磨蹭一兩下再躺下，讀起來自然。夜深時居民抵達點很頻繁（小半徑來回），
/// 因此即使單次機率不高，通常幾秒內就會睡著。
pub const FALL_ASLEEP_CHANCE: f32 = 0.4;

/// 醒來後心情提升的持續秒數（沿用 `mood_boost_secs` 機制，681）：睡飽起床的那段
/// 早晨時光心情提升一格，讓玩家一早就看到居民帶著好心情的 emoji。
pub const WAKE_MOOD_BOOST_SECS: f32 = 30.0;

/// 是否符合「可以躺下睡」的基本條件：深夜 + 正在自家遮蔽 + 已靠近自家中心。
/// 三者皆備才算——白天不睡、沒家可回不睡（在外遊蕩不會就地睡死）、還沒到家不睡。
pub fn eligible_to_sleep(is_deep_night: bool, sheltering: bool, near_home: bool) -> bool {
    is_deep_night && sheltering && near_home
}

/// 是否在這次抵達點真的睡著：符合條件 + 目前醒著 + 過機率門檻。
/// `roll` 由呼叫端以 `rand::random::<f32>()` 取真隨機餵入（與本專案其他機率骰同慣例）。
pub fn should_fall_asleep(
    is_deep_night: bool,
    sheltering: bool,
    near_home: bool,
    already_asleep: bool,
    roll: f32,
) -> bool {
    !already_asleep
        && eligible_to_sleep(is_deep_night, sheltering, near_home)
        && roll < FALL_ASLEEP_CHANCE
}

/// 是否該醒來：已睡著、且已經不在「可睡時段」（黎明／白晝／黃昏一到就起床）。
/// `can_sleep_phase` = 現在是否仍屬夜間系列（深夜或入夜過渡），由呼叫端以
/// `vt::is_sleepable(phase)` 傳入——一旦天色轉亮就醒。
pub fn should_wake(can_sleep_phase: bool, already_asleep: bool) -> bool {
    already_asleep && !can_sleep_phase
}

/// 距離平方判定「已靠近自家中心」（避免開根號）。
pub fn near_home_center(bx: f32, bz: f32, cx: f32, cz: f32) -> bool {
    let dx = bx - cx;
    let dz = bz - cz;
    dx * dx + dz * dz <= SLEEP_NEAR_RADIUS * SLEEP_NEAR_RADIUS
}

/// 入睡時頭頂冒的一句話（面向玩家，集中可 i18n）。
pub fn fall_asleep_line(pick: usize) -> &'static str {
    const T: [&str; 3] = [
        "睏了……回家睡了。",
        "夜深了，該休息了。",
        "回到家，安心睡個覺～",
    ];
    T[pick % T.len()]
}

/// 醒來時頭頂冒的一句話（面向玩家，集中可 i18n）。
pub fn wake_line(pick: usize) -> &'static str {
    const T: [&str; 3] = [
        "睡飽了，神清氣爽！",
        "早安！睡了一覺真舒服～",
        "天亮了，元氣滿滿！",
    ];
    T[pick % T.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_requires_all_three() {
        assert!(eligible_to_sleep(true, true, true));
        assert!(!eligible_to_sleep(false, true, true), "白天不睡");
        assert!(!eligible_to_sleep(true, false, true), "沒家遮蔽不睡");
        assert!(!eligible_to_sleep(true, true, false), "還沒到家不睡");
    }

    #[test]
    fn should_fall_asleep_respects_conditions_and_chance() {
        // 全條件備齊 + roll 低於門檻 → 睡著。
        assert!(should_fall_asleep(true, true, true, false, FALL_ASLEEP_CHANCE - 0.01));
        // roll 達門檻（含）→ 不睡（這次磨蹭一下）。
        assert!(!should_fall_asleep(true, true, true, false, FALL_ASLEEP_CHANCE));
        assert!(!should_fall_asleep(true, true, true, false, 0.99));
        // 已經睡著 → 不再重複觸發入睡。
        assert!(!should_fall_asleep(true, true, true, true, 0.0));
        // 不符基本條件 → 無論 roll 多低都不睡。
        assert!(!should_fall_asleep(false, true, true, false, 0.0));
        assert!(!should_fall_asleep(true, false, true, false, 0.0));
        assert!(!should_fall_asleep(true, true, false, false, 0.0));
    }

    #[test]
    fn should_wake_only_when_asleep_and_daylight() {
        // 睡著 + 天亮（不在可睡時段）→ 醒。
        assert!(should_wake(false, true));
        // 睡著但仍是夜間 → 繼續睡。
        assert!(!should_wake(true, true));
        // 沒睡著 → 不觸發醒來邏輯。
        assert!(!should_wake(false, false));
        assert!(!should_wake(true, false));
    }

    #[test]
    fn near_home_center_uses_radius() {
        // 正中心 → 近。
        assert!(near_home_center(10.0, 10.0, 10.0, 10.0));
        // 剛好在半徑上 → 近（含邊界）。
        assert!(near_home_center(10.0 + SLEEP_NEAR_RADIUS, 10.0, 10.0, 10.0));
        // 超出半徑 → 不近。
        assert!(!near_home_center(10.0 + SLEEP_NEAR_RADIUS + 0.1, 10.0, 10.0, 10.0));
    }

    #[test]
    fn lines_vary_with_pick_and_are_nonempty() {
        assert_ne!(fall_asleep_line(0), fall_asleep_line(1));
        assert_ne!(wake_line(0), wake_line(1));
        assert!(!fall_asleep_line(0).is_empty());
        assert!(!wake_line(0).is_empty());
        // pick 取模不越界。
        let _ = fall_asleep_line(usize::MAX);
        let _ = wake_line(usize::MAX);
    }

    #[test]
    fn wake_boost_and_emoji_are_sane() {
        assert!(WAKE_MOOD_BOOST_SECS > 0.0);
        assert_eq!(SLEEP_MOOD_EMOJI, "💤");
        assert!(FALL_ASLEEP_CHANCE > 0.0 && FALL_ASLEEP_CHANCE < 1.0);
        assert!(SLEEP_NEAR_RADIUS > 0.0);
    }
}
