//! 乙太方界·月有圓缺 v1（voxel-moonphase，自主提案切片 ROADMAP 946）。
//!
//! **缺口 / 為誰做**：繁星夜空（783）第一次讓 voxel 夜空升起一輪明月——但那輪月**永遠是滿的**，
//! 從不盈虧。世界的天象一路長出流星（904）、彩虹（780）、極光（930）、候鳥（944），唯獨那輪月
//! 始終圓得一成不變，與「時間的流逝」毫無關聯。舊 2D 世界早有月相（`src/moon.rs`，依真實朔望月盈虧），
//! voxel 卻一直缺這條最古老的時間軸。本刀讓月亮**隨乙太曆盈虧**——朔望週期只有 [`SYNODIC_DAYS`] 個
//! 遊戲日，於是短短幾個夜就看得出「昨晚還是彎彎的細月、今晚已鼓成半輪」的圓缺變化（不像真實 29.5 天
//! 一整個月才走一輪、玩家一輩子看不到變化）；並在**滿月夜**讓夜空真的圓、醒著靠近你的居民偶爾抬頭
//! 對著那輪圓月出神、輕聲許一句願、心情跟著微亮一格，城鎮動態牆也留下一筆「今夜月圓」讓離線回訪的
//! 玩家讀得到。
//!
//! **與繁星夜空（783）razor-sharp 區隔**：783＝**夜夜都在**的星＋**恆滿**的月＋「記得你愛看星星就
//! 邀你同賞」（**偏好驅動**、對象是**你**、任何夜晚都可能發生）；本刀＝月亮**會變形**（相位驅動的
//! 純視覺，前端依權威受光比例把圓月削成弦月／細月）＋只在**滿月那數夜**才發生的**對月出神**（**月相
//! 驅動**、對象是**那輪月**、每個朔望月一次），觸發條件、情緒與台詞全然不同軸。望星是「與你共賞繁星」、
//! 本刀是「獨自對著圓月出神許願」，一社交一內省，刻意不撞。
//!
//! **成本 / 濫用防護鐵律**：零 LLM（受光比例／出神台詞／動態牆全為確定性公式與固定模板）、
//! 零持久化／零 migration（月相由既有世界時鐘 `days_elapsed`＋`time_of_day` 現算，滿月夜播報去重旗標
//! 與出神冷卻皆純記憶體、重啟歸零，比照流星／彩虹慣例）、零新美術（前端沿用既有月球 mesh，只依受光
//! 比例削出相位）、零新協議破壞（快照只**新增**一個唯讀 float `moon_illum`，舊前端安全忽略）。
//! 本切片**不收任何玩家輸入、不觸發 LLM、不開對外端點、不動帳號權限**——月相全由伺服器端世界時鐘與
//! 確定性純函式驅動，台詞不嵌任何玩家輸入，玩家無從主動觸發或洗版。
//!
//! **純邏輯層**：本檔只放確定性純函式（受光比例、滿月判定、盈虧判定、出神觸發、台詞／動態牆選句），
//! 零 IO、零鎖、零 LLM、零 async，可窮舉單元測試。鎖與副作用全在 `voxel_ws.rs`
//! （沿用繁星 783／流星 904 那條已驗證的短鎖循序慣例，守 prod 死鎖鐵律）。

use std::f64::consts::PI;

/// 一個乙太朔望月＝幾個遊戲日（新月→滿月→新月走一輪）。刻意短：一遊戲日＝
/// `voxel_time::DAY_DURATION_SECS`（600 秒），4 日一輪 ⇒ 每個夜晚相位進約 1/4，
/// 幾個夜就看得出圓缺（真實 29.5 天太慢、玩家一輩子看不到變化）。
pub const SYNODIC_DAYS: f64 = 4.0;

/// 「視為滿月」的受光比例門檻——`illum ∈ [0,1]`（新月 0、滿月 1）。0.90 對應相位約 0.40~0.60
/// 的窗口（朔望週期的兩成、約 0.8 個遊戲日），讓「滿月夜」是一段自然的數夜、涵蓋得住當夜的夜晚時段，
/// 而非只圓得最足的那一瞬。前端畫相位、後端判滿月夜都用這同一門檻，符號與行為一致。
pub const FULL_MOON_ILLUM: f64 = 0.90;

/// 玩家要在這半徑（格）內、居民抬頭對月出神才觸發——比照望星（[`crate::voxel_stargaze::STARGAZE_RANGE`]），
/// 讓你看得見那顆對月出神的泡泡。
pub const MOONGAZE_RANGE: f32 = 22.0;

/// 對月出神冷卻（秒）：一次出神後歸零前不再觸發——月圓出神是滿月夜偶爾的靜謐一拍、不洗版。
pub const MOONGAZE_COOLDOWN_SECS: f32 = 600.0;

/// 每 tick 觸發機率（僅在「滿月夜＋玩家在範圍內＋冷卻到期＋沒在說話＋醒著」全滿足才擲骰）。
/// 刻意極低：對月出神是可遇不可求的靜謐一刻，不是走近就觸發。
pub const MOONGAZE_CHANCE_PER_TICK: f32 = 0.03;

/// 動態牆播報種類名稱。
pub const FEED_KIND: &str = "月圓之夜";

/// 朔望相位 ∈ [0,1)：0＝新月、0.5＝滿月、趨近 1 再回新月。由世界累計日數＋當日時刻現算
/// （`days_elapsed + time_of_day` 是連續的「天」數，對 [`SYNODIC_DAYS`] 取模歸一）。
pub fn cycle_phase(days_elapsed: u64, time_of_day: f32) -> f64 {
    let t = days_elapsed as f64 + time_of_day.clamp(0.0, 1.0) as f64;
    (t / SYNODIC_DAYS).rem_euclid(1.0)
}

/// 把相位映成受光（明亮）面比例 `illum ∈ [0,1]`：新月 0、上/下弦 0.5、滿月 1。
/// 採天文標準 `(1-cos(2π·相位))/2`，與 `src/moon.rs`／`web/game.js` 舊 2D 月相同公式，
/// 讓前端得以照著同一條曲線把圓月削成弦月。
pub fn illumination(days_elapsed: u64, time_of_day: f32) -> f64 {
    let cyc = cycle_phase(days_elapsed, time_of_day);
    let illum = (1.0 - (2.0 * PI * cyc).cos()) / 2.0;
    illum.clamp(0.0, 1.0)
}

/// 此刻受光比例是否已達滿月門檻。
pub fn is_full_moon(illum: f64) -> bool {
    illum >= FULL_MOON_ILLUM
}

/// 是否正在「盈」（相位 < 0.5，新月往滿月走）——供前端決定弦月缺口朝左或朝右。
pub fn is_waxing(days_elapsed: u64, time_of_day: f32) -> bool {
    cycle_phase(days_elapsed, time_of_day) < 0.5
}

/// 全條件對月出神觸發判定（確定性、可窮舉測試）。呼叫端把當下狀態與一顆隨機骰 `roll`
/// （`rand::random`）傳入。
///
/// - `is_full_moon_night`：目前是否為滿月＋夜晚時段（由呼叫端合成）。
/// - `player_in_range`：附近是否有玩家在 [`MOONGAZE_RANGE`] 內（讓你看得見那顆出神泡泡）。
/// - `cooldown_ready`：出神冷卻是否到期（`<= 0`）。
/// - `saying`：居民此刻是否正在說話（`say` 非空 → 讓正事／不與望星同 tick 搶話）。
/// - `asleep`：居民是否在睡覺（睡著的人不會抬頭望月）。
/// - `roll`：`[0,1)` 隨機骰，`< MOONGAZE_CHANCE_PER_TICK` 才過機率門檻。
pub fn should_moongaze(
    is_full_moon_night: bool,
    player_in_range: bool,
    cooldown_ready: bool,
    saying: bool,
    asleep: bool,
    roll: f32,
) -> bool {
    is_full_moon_night
        && player_in_range
        && cooldown_ready
        && !saying
        && !asleep
        && roll < MOONGAZE_CHANCE_PER_TICK
}

/// 滿月夜對月出神自語（居民抬頭望向那輪圓月，冒一句靜謐的話；不點名、不寫記憶——純夜色氛圍）。
/// 每句都明確扣住「圓月／滿月」，與望星台詞（[`crate::voxel_stargaze`]，講星／泛用夜空）刻意不撞。
/// 依 `pick` 確定性選一句（循環取模，永遠有值）。
const MOONGAZE_LINES: [&str; 5] = [
    "今晚的月亮好圓啊……看著看著，心也跟著圓滿了。",
    "一輪滿月高高掛著，把整片方界都照亮了呢。",
    "月圓之夜，總讓人想靜靜地許個願。",
    "你看那月亮，圓得像一面銀鏡似的。",
    "滿月的光灑下來，連夜色都變得溫柔了。",
];

/// 依 `pick` 選一句對月出神自語。
pub fn moongaze_line(pick: usize) -> &'static str {
    MOONGAZE_LINES[pick % MOONGAZE_LINES.len()]
}

/// 滿月夜城鎮動態牆一行（世界級事件、非某位居民，讓離線回訪的玩家讀得到「昨夜月圓」）。
/// 純固定模板、無換行、不嵌任何動態內容（杜絕注入／洗版）。
pub fn full_moon_feed_line() -> &'static str {
    "一輪滿月高掛乙太方界的夜空——今夜是月圓之夜。"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illumination_new_and_full_moon_extremes() {
        // 相位 0（新月）受光趨近 0；相位 0.5（滿月）受光趨近 1。
        // 令 days_elapsed 對 SYNODIC_DAYS 整除 ⇒ 相位 0；再加半週期 ⇒ 相位 0.5。
        let new_moon = illumination(0, 0.0);
        assert!(new_moon < 0.001, "新月受光應趨近 0，實得 {new_moon}");
        let half = (SYNODIC_DAYS / 2.0) as u64; // 2 日 = 半個朔望月
        let full_moon = illumination(half, 0.0);
        assert!(full_moon > 0.999, "滿月受光應趨近 1，實得 {full_moon}");
    }

    #[test]
    fn illumination_always_bounded() {
        // 掃一整輪 + 跨數輪，受光比例恆在 [0,1]、絕不越界。
        for d in 0..(SYNODIC_DAYS as u64 * 4 + 3) {
            for step in 0..10 {
                let tod = step as f32 / 10.0;
                let il = illumination(d, tod);
                assert!((0.0..=1.0).contains(&il), "day {d} tod {tod} 受光越界：{il}");
            }
        }
    }

    #[test]
    fn cycle_phase_wraps_over_synodic_month() {
        // 相位恆在 [0,1)；每過 SYNODIC_DAYS 個遊戲日回到同一相位（週期性）。
        let p0 = cycle_phase(0, 0.3);
        let p_wrap = cycle_phase(SYNODIC_DAYS as u64, 0.3);
        assert!((p0 - p_wrap).abs() < 1e-9, "整週期後相位應回到原點");
        for d in 0..20 {
            for step in 0..8 {
                let p = cycle_phase(d, step as f32 / 8.0);
                assert!((0.0..1.0).contains(&p), "相位越界 day {d}: {p}");
            }
        }
    }

    #[test]
    fn full_moon_detection_matches_threshold() {
        assert!(is_full_moon(FULL_MOON_ILLUM));
        assert!(is_full_moon(1.0));
        assert!(!is_full_moon(FULL_MOON_ILLUM - 0.001));
        assert!(!is_full_moon(0.0));
        // 半個朔望月處（滿月）確實被判為滿月夜。
        let half = (SYNODIC_DAYS / 2.0) as u64;
        assert!(is_full_moon(illumination(half, 0.0)));
        // 新月處絕非滿月。
        assert!(!is_full_moon(illumination(0, 0.0)));
    }

    #[test]
    fn waxing_flips_across_full_moon() {
        // 相位 < 0.5＝盈（往滿月）、>= 0.5＝虧。新月處在盈、剛過滿月處在虧。
        assert!(is_waxing(0, 0.1), "新月剛過應在盈");
        let just_past_full = SYNODIC_DAYS as u64 / 2; // 相位 0.5
        assert!(!is_waxing(just_past_full, 0.05), "剛到／過滿月應在虧");
    }

    #[test]
    fn should_moongaze_all_conditions() {
        // 全滿足 + 骰過門檻 → 觸發。
        assert!(should_moongaze(true, true, true, false, false, 0.0));
        // 任一條件不滿足 → 不觸發。
        assert!(!should_moongaze(false, true, true, false, false, 0.0), "非滿月夜不觸發");
        assert!(!should_moongaze(true, false, true, false, false, 0.0), "玩家不在範圍不觸發");
        assert!(!should_moongaze(true, true, false, false, false, 0.0), "冷卻未到不觸發");
        assert!(!should_moongaze(true, true, true, true, false, 0.0), "正在說話不觸發");
        assert!(!should_moongaze(true, true, true, false, true, 0.0), "睡著不觸發");
        // 骰沒過門檻 → 不觸發。
        assert!(!should_moongaze(true, true, true, false, false, 1.0), "骰值超過機率門檻不觸發");
        assert!(!should_moongaze(true, true, true, false, false, MOONGAZE_CHANCE_PER_TICK));
    }

    #[test]
    fn moongaze_line_cycles_and_mentions_round_moon() {
        // 任意 pick（含極大值）取模有界、永遠有值；每句都扣住「圓／滿月」的意象，與望星不撞。
        for pick in [0usize, 1, 4, 5, 999, usize::MAX] {
            let line = moongaze_line(pick);
            assert!(!line.is_empty());
            assert!(!line.contains('\n'), "台詞不得含換行（防注入／破框）");
            assert!(
                line.contains('月') || line.contains("銀鏡"),
                "對月出神台詞須扣住月的意象：{line}"
            );
        }
        // 五句互異。
        let all: Vec<&str> = (0..MOONGAZE_LINES.len()).map(moongaze_line).collect();
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "第 {i}/{j} 句重複");
            }
        }
    }

    #[test]
    fn full_moon_feed_line_is_safe_single_line() {
        let f = full_moon_feed_line();
        assert!(!f.is_empty());
        assert!(!f.contains('\n'), "動態牆一行不得含換行（防注入）");
        assert!(f.contains("月圓") || f.contains("滿月"), "動態牆須點明月圓之夜");
    }
}
