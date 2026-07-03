//! 乙太方界·居民會注意到你親手蓋的東西 v1（voxel-admire）。
//!
//! **北極星**：至今「蓋東西」都是單向的——**居民**蓋家、玩家在旁看（652/1023），
//! 玩家自己一磚一瓦堆起來的作品，卻從沒有任何居民「看見」過。這一刀把方向反過來：
//! 玩家在某位居民身邊接連堆起一片方塊（一面牆、一座小屋、一道橋）時，**居民會注意到、
//! 停下來讚賞一句、並把「我看著這位旅人親手蓋起了東西」記進心裡**——你的創造第一次
//! 被這個世界的居民看見、記住，交情也因此更深一層。正中 PLAN_ETHERVOX 核心信念
//! 「記憶要驅動行為、你的互動真的有後果」，也是人類建造樂趣與 AI 生活第一次交織：
//! 你蓋、她看，兩條主軸在同一片方塊天地裡對望。
//!
//! **純邏輯層**：連續放置的「創作連段」判定（[`advance_streak`]）、是否觸發讚賞
//! （[`admire_triggers`]）、讚賞台詞（[`admire_say_line`]）、記憶摘要（[`admire_memory_line`]）
//! 全是確定性純函式，零 LLM、零鎖、零 IO。鎖 / 廣播 / 記憶寫入全在 `voxel_ws.rs`，
//! 且沿用既有「玩家幫忙蓋家」（699/769）那條已驗證的鎖序與冷卻節流慣例。
//!
//! **成本 / 濫用防護**：台詞全為固定模板、**永不回放玩家原話**（無注入 / NSFW 風險）；
//! 每位居民對每位玩家設 [`ADMIRE_COOLDOWN_SECS`] 冷卻，配合「一段連續建造才算一次創作」
//! 的連段門檻（[`ADMIRE_STREAK_MIN`]），單塊放置不觸發、狂放方塊也只換來偶爾一句讚賞，
//! 天然防洗版、也防好感（＝記憶筆數）被刷爆。零 migration、零新協議欄位、零前端改動、
//! 零新美術、FPS 零影響（純後端）。

/// 讚賞觸及半徑（方塊距離，水平 XZ 平面）：居民要離你剛放的那塊夠近才「看得見」。
/// 與贈禮 GIFT_REACH(5.0) 同量級、稍寬一格，讓走過路過的鄰居也可能瞥見你的手藝。
pub const ADMIRE_RADIUS: f32 = 6.0;

/// 觸發讚賞所需的最短「創作連段」長度：連續放到第幾塊才算「真的在蓋一件東西」。
/// 設 4——單塊、雙塊只是隨手擺，一段四塊以上才像牆／屋／橋的雛形，值得被讚賞。
pub const ADMIRE_STREAK_MIN: u32 = 4;

/// 同一位居民對同一位玩家的讚賞冷卻（秒）。比照幫忙蓋家記憶冷卻同量級（150s），
/// 讓讚賞稀有有份量、也把「靠放方塊刷好感」的速率天然夾死。
pub const ADMIRE_COOLDOWN_SECS: u64 = 150;

/// 連段「還算同一件創作」的距離門檻（方塊）：這次放的位置離上一塊多遠內才續接連段。
/// 設 8——沿著牆／往上疊都在範圍內續接；換去老遠另起爐灶就重新起算連段。
pub const STREAK_NEAR_DIST: f32 = 8.0;

/// 連段「還算連續」的時間窗（秒）：離上一塊太久沒放，視為兩次獨立建造、連段歸零。
pub const STREAK_WINDOW_SECS: u64 = 40;

/// 一段連續建造的進度：`(連段塊數, 上一塊 x, 上一塊 z, 上一塊放置時刻秒)`。
pub type BuildStreak = (u32, f32, f32, u64);

/// 依「這次放的位置與時刻」推進玩家的建造連段（純函式、確定性）。
///
/// - 若上一塊還在（`prev` 有值）、與這次夠近（≤[`STREAK_NEAR_DIST`]）且夠新
///   （≤[`STREAK_WINDOW_SECS`]）→ 續接連段，塊數 +1。
/// - 否則（第一次、或跳去老遠、或隔太久）→ 連段重新從 1 起算。
///
/// 回傳新的連段狀態（塊數＋這次的位置＋時刻），由呼叫端存回連線區域變數。
pub fn advance_streak(prev: Option<BuildStreak>, x: f32, z: f32, now_secs: u64) -> BuildStreak {
    if let Some((count, lx, lz, lsecs)) = prev {
        let dx = x - lx;
        let dz = z - lz;
        let near = dx * dx + dz * dz <= STREAK_NEAR_DIST * STREAK_NEAR_DIST;
        let fresh = now_secs.saturating_sub(lsecs) <= STREAK_WINDOW_SECS;
        if near && fresh {
            return (count + 1, x, z, now_secs);
        }
    }
    (1, x, z, now_secs)
}

/// 是否該觸發居民讚賞（純函式）：連段夠長 ＋ 居民離作品夠近 ＋ 冷卻已過。
/// 「居民此刻是否有空／有沒有正在冒別的泡泡」由呼叫端另外把關（見 `voxel_ws.rs`）。
pub fn admire_triggers(streak: u32, resident_dist_sq: f32, cooldown_ok: bool) -> bool {
    streak >= ADMIRE_STREAK_MIN
        && resident_dist_sq <= ADMIRE_RADIUS * ADMIRE_RADIUS
        && cooldown_ok
}

/// 居民讚賞玩家手藝的台詞（繁中、面向玩家、i18n 集中於此；確定性依 `pick` 選句）。
/// 刻意**不含玩家原話**——只誇「你蓋的東西」，無注入 / NSFW 風險。
pub fn admire_say_line(player_name: &str, pick: usize) -> String {
    const LINES: &[&str] = &[
        "{p}，這是你親手蓋的嗎？真氣派！",
        "哇，{p} 堆的方塊愈來愈有樣子了呢。",
        "我一直看著你蓋，{p}，這手藝真用心。",
        "{p}，你把這一帶點綴得真好看。",
        "瞧瞧 {p} 蓋的——這世界因你多了一角風景。",
    ];
    LINES[pick % LINES.len()].replace("{p}", player_name)
}

/// 把「我看著這位旅人親手蓋起了東西」寫成一段居民的記憶摘要（第一人稱、episodic）。
///
/// 刻意避開 [`crate::voxel_memory::classify_importance`] 的目標／偏好／承諾／名字關鍵詞
/// （不含「要蓋 / 想要 / 喜歡 / 記住 / 我是」等），讓它停在情節記憶層、只累積好感，
/// 不誤升級成語意精華——與 769「玩家幫忙蓋家」記憶同款設計。
pub fn admire_memory_line(player_name: &str) -> String {
    format!("我看著{player_name}在附近親手蓋起了一片方塊，真了不起。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streak_starts_at_one() {
        // 沒有前一塊 → 連段從 1 起算。
        let (c, x, z, t) = advance_streak(None, 10.0, 20.0, 100);
        assert_eq!(c, 1);
        assert_eq!((x, z, t), (10.0, 20.0, 100));
    }

    #[test]
    fn streak_chains_when_near_and_fresh() {
        // 續接：離上一塊 1 格、隔 5 秒 → 塊數 +1。
        let prev = Some((3, 10.0, 20.0, 100));
        let (c, x, z, t) = advance_streak(prev, 11.0, 20.0, 105);
        assert_eq!(c, 4);
        assert_eq!((x, z, t), (11.0, 20.0, 105));
    }

    #[test]
    fn streak_resets_when_far() {
        // 跳去老遠（距離 > STREAK_NEAR_DIST）→ 連段歸 1。
        let prev = Some((5, 0.0, 0.0, 100));
        let (c, ..) = advance_streak(prev, 100.0, 100.0, 102);
        assert_eq!(c, 1);
    }

    #[test]
    fn streak_resets_when_stale() {
        // 離上一塊很近、但隔太久（> STREAK_WINDOW_SECS）→ 連段歸 1。
        let prev = Some((5, 0.0, 0.0, 100));
        let (c, ..) = advance_streak(prev, 1.0, 0.0, 100 + STREAK_WINDOW_SECS + 1);
        assert_eq!(c, 1);
    }

    #[test]
    fn streak_boundary_exactly_at_window_still_chains() {
        // 剛好落在時間窗邊界（==STREAK_WINDOW_SECS）→ 仍算連續。
        let prev = Some((2, 0.0, 0.0, 100));
        let (c, ..) = advance_streak(prev, 0.0, 0.0, 100 + STREAK_WINDOW_SECS);
        assert_eq!(c, 3);
    }

    #[test]
    fn triggers_needs_all_three() {
        // 三條件（連段足、夠近、冷卻過）全滿足才觸發。
        assert!(admire_triggers(ADMIRE_STREAK_MIN, 0.0, true));
        // 連段不足 → 不觸發。
        assert!(!admire_triggers(ADMIRE_STREAK_MIN - 1, 0.0, true));
        // 冷卻未過 → 不觸發。
        assert!(!admire_triggers(ADMIRE_STREAK_MIN, 0.0, false));
        // 太遠（剛好超出半徑）→ 不觸發。
        let just_out = ADMIRE_RADIUS * ADMIRE_RADIUS + 0.01;
        assert!(!admire_triggers(ADMIRE_STREAK_MIN, just_out, true));
    }

    #[test]
    fn triggers_at_exact_radius() {
        // 剛好在半徑邊界上 → 算看得見（≤）。
        let on_edge = ADMIRE_RADIUS * ADMIRE_RADIUS;
        assert!(admire_triggers(ADMIRE_STREAK_MIN, on_edge, true));
    }

    #[test]
    fn say_line_is_deterministic_and_carries_name() {
        // 同 pick → 同句；含玩家名；不同 pick 取到不同模板（覆蓋率）。
        let a = admire_say_line("露娜客", 0);
        let b = admire_say_line("露娜客", 0);
        assert_eq!(a, b);
        assert!(a.contains("露娜客"));
        assert!(!admire_say_line("阿旅", 0).contains("{p}"));
    }

    #[test]
    fn memory_line_stays_episodic() {
        // 記憶摘要不得誤觸重要性關鍵詞（否則會被升級成語意精華、亂佔上限）。
        let line = admire_memory_line("建築師");
        assert!(matches!(
            crate::voxel_memory::classify_importance(&line),
            crate::voxel_memory::Importance::Ephemeral
        ));
        assert!(line.contains("建築師"));
    }
}
