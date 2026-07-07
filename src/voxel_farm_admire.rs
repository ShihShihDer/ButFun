//! 乙太方界·居民注意到你悉心照料的農地 v1（自主提案切片，接續 773 建造讚賞）。
//!
//! **真缺口**：773 讓居民注意到玩家「連續放置方塊」蓋東西並由衷讚賞、記進心裡；但種田
//! （659 起一路疊到 811 時令作物）從頭到尾只有 753「居民順手替你照料菜園」——那是居民
//! **主動幫你做事**（推進作物生長），從沒有居民只是單純**看見你在種田、由衷讚賞這件事
//! 本身**。你翻土播種一整畦地，居民路過的反應跟你什麼都沒做時一模一樣——種田是全庫
//! 目前唯一還沒被「記憶要驅動行為」北極星碰過的玩家樂趣支線。
//!
//! **做法**：比照 773 的「連續動作」偵測手法，換成「連續種田動作」（翻土 HoeTill／播種
//! Plant 皆算，兩者交替也算同一段）——一段連續種田夠長，身邊有空的居民會停下來由衷
//! 讚賞你的用心，把「看著這位旅人在田裡忙進忙出」記進心裡（累積好感）。刻意用**farm
//! 專屬台詞**（泥土香氣／期待收成），與 773 建造讚賞的語氣區隔，不是同一句話套殼。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **753 照料菜園**＝居民主動幫忙推進生長（你的作物變快熟）；本刀＝居民只是欣賞你
//!   翻土播種的過程，**不碰你的作物一根手指**，兩者可同時發生、互不干擾（各自獨立
//!   冷卻／連段，也不共用彼此的判定）。
//! - **773 建造讚賞**＝觸發於 `Place` 放置方塊；本刀＝觸發於 `HoeTill`／`Plant`，完全
//!   不同的動作管線與冷卻鍵（同一位居民可以同時欠你一次建造讚賞、又欠你一次種田讚賞）。
//!
//! **純邏輯層**：連續種田的「農忙連段」判定（[`advance_streak`]）、是否觸發讚賞
//! （[`admire_triggers`]）、讚賞台詞（[`admire_say_line`]）、記憶摘要（[`admire_memory_line`]）
//! 全是確定性純函式，零 LLM、零鎖、零 IO。鎖 / 廣播 / 記憶寫入全在 `voxel_ws.rs`，
//! 且沿用既有 773 建造讚賞那條已驗證的鎖序與冷卻節流慣例。
//!
//! **成本 / 濫用防護**：台詞全為固定模板、永不回放玩家原話（無注入 / NSFW 風險）；每位
//! 居民對每位玩家設 [`FARM_ADMIRE_COOLDOWN_SECS`] 冷卻，配合「一段連續農忙才算一次」的
//! 連段門檻（[`FARM_ADMIRE_STREAK_MIN`]），單次翻土/播種不觸發，天然防洗版、也防好感
//! （＝記憶筆數）被刷爆。零 migration、零新協議欄位、零前端改動、零新美術、FPS 零影響
//! （純後端）。

/// 讚賞觸及半徑（方塊距離，水平 XZ 平面）：居民要離你正在忙的那塊地夠近才「看得見」。
/// 與 773 建造讚賞 `ADMIRE_RADIUS` 同量級。
pub const FARM_ADMIRE_RADIUS: f32 = 6.0;

/// 觸發讚賞所需的最短「農忙連段」長度：翻土/播種累計到第幾次才算「真的在種田」。
/// 設 3——單次翻土或播種只是隨手一下，連續三次以上才像在整理一畦地，值得被讚賞。
pub const FARM_ADMIRE_STREAK_MIN: u32 = 3;

/// 同一位居民對同一位玩家的讚賞冷卻（秒）。與 773 建造讚賞 `ADMIRE_COOLDOWN_SECS` 同量級，
/// 讓讚賞稀有有份量，也把「靠翻土播種刷好感」的速率天然夾死。
pub const FARM_ADMIRE_COOLDOWN_SECS: u64 = 150;

/// 連段「還算同一次農忙」的距離門檻（方塊）：這次動作的位置離上一次多遠內才續接連段。
/// 農地通常一畦一畦緊鄰著開墾/播種，設 6 已足夠涵蓋一整排。
pub const STREAK_NEAR_DIST: f32 = 6.0;

/// 連段「還算連續」的時間窗（秒）：離上一次動作太久，視為兩次獨立農忙、連段歸零。
pub const STREAK_WINDOW_SECS: u64 = 40;

/// 一段連續農忙的進度：`(連段次數, 上一次動作 x, 上一次動作 z, 上一次動作時刻秒)`。
pub type FarmStreak = (u32, f32, f32, u64);

/// 依「這次翻土/播種的位置與時刻」推進玩家的農忙連段（純函式、確定性）。
///
/// - 若上一次還在（`prev` 有值）、與這次夠近（≤[`STREAK_NEAR_DIST`]）且夠新
///   （≤[`STREAK_WINDOW_SECS`]）→ 續接連段，次數 +1。
/// - 否則（第一次、或跳去老遠、或隔太久）→ 連段重新從 1 起算。
///
/// 回傳新的連段狀態（次數＋這次的位置＋時刻），由呼叫端存回連線區域變數。
pub fn advance_streak(prev: Option<FarmStreak>, x: f32, z: f32, now_secs: u64) -> FarmStreak {
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

/// 是否該觸發居民讚賞（純函式）：連段夠長 ＋ 居民離你夠近 ＋ 冷卻已過。
/// 「居民此刻是否有空／有沒有正在冒別的泡泡」由呼叫端另外把關（見 `voxel_ws.rs`）。
pub fn admire_triggers(streak: u32, resident_dist_sq: f32, cooldown_ok: bool) -> bool {
    streak >= FARM_ADMIRE_STREAK_MIN
        && resident_dist_sq <= FARM_ADMIRE_RADIUS * FARM_ADMIRE_RADIUS
        && cooldown_ok
}

/// 居民讚賞玩家種田用心的台詞（繁中、面向玩家、i18n 集中於此；確定性依 `pick` 選句）。
/// 刻意**不含玩家原話**——只誇「你在田裡的用心」，無注入 / NSFW 風險；語氣與 773 建造
/// 讚賞刻意區隔（泥土香氣／收成期待，不提「蓋」「堆方塊」）。
pub fn admire_say_line(player_name: &str, pick: usize) -> String {
    const LINES: &[&str] = &[
        "{p}，你這畦地翻得真仔細，我都聞到泥土的香氣了。",
        "看{p}這樣一畦一畦仔細照顧，這片田一定會長得很好。",
        "難怪{p}種的田總是長得漂亮，原來這麼下功夫。",
        "光是看{p}種田，我心情都跟著好起來了。",
        "{p}，你對這片田的用心，我都看在眼裡。",
    ];
    LINES[pick % LINES.len()].replace("{p}", player_name)
}

/// 把「看著這位旅人在田裡忙進忙出」寫成一段居民的記憶摘要（第一人稱、episodic）。
///
/// 刻意避開 [`crate::voxel_memory::classify_importance`] 的目標／偏好／承諾／身份關鍵詞
/// （不含「要蓋 / 想要 / 喜歡 / 記住 / 我是」等），讓它停在情節記憶層、只累積好感，
/// 不誤升級成語意精華——與 773 建造讚賞記憶同款設計。
pub fn admire_memory_line(player_name: &str) -> String {
    format!("看著{player_name}在田裡忙進忙出，一畦一畦仔細照顧，那份用心我都看在眼裡。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streak_starts_at_one() {
        let (c, x, z, t) = advance_streak(None, 10.0, 20.0, 100);
        assert_eq!(c, 1);
        assert_eq!((x, z, t), (10.0, 20.0, 100));
    }

    #[test]
    fn streak_chains_when_near_and_fresh() {
        let prev = Some((2, 10.0, 20.0, 100));
        let (c, x, z, t) = advance_streak(prev, 11.0, 20.0, 105);
        assert_eq!(c, 3);
        assert_eq!((x, z, t), (11.0, 20.0, 105));
    }

    #[test]
    fn streak_resets_when_far() {
        let prev = Some((5, 0.0, 0.0, 100));
        let (c, ..) = advance_streak(prev, 100.0, 100.0, 102);
        assert_eq!(c, 1);
    }

    #[test]
    fn streak_resets_when_stale() {
        let prev = Some((5, 0.0, 0.0, 100));
        let (c, ..) = advance_streak(prev, 1.0, 0.0, 100 + STREAK_WINDOW_SECS + 1);
        assert_eq!(c, 1);
    }

    #[test]
    fn streak_boundary_exactly_at_window_still_chains() {
        let prev = Some((2, 0.0, 0.0, 100));
        let (c, ..) = advance_streak(prev, 0.0, 0.0, 100 + STREAK_WINDOW_SECS);
        assert_eq!(c, 3);
    }

    #[test]
    fn triggers_needs_all_three() {
        assert!(admire_triggers(FARM_ADMIRE_STREAK_MIN, 0.0, true));
        assert!(!admire_triggers(FARM_ADMIRE_STREAK_MIN - 1, 0.0, true));
        assert!(!admire_triggers(FARM_ADMIRE_STREAK_MIN, 0.0, false));
        let just_out = FARM_ADMIRE_RADIUS * FARM_ADMIRE_RADIUS + 0.01;
        assert!(!admire_triggers(FARM_ADMIRE_STREAK_MIN, just_out, true));
    }

    #[test]
    fn triggers_at_exact_radius() {
        let on_edge = FARM_ADMIRE_RADIUS * FARM_ADMIRE_RADIUS;
        assert!(admire_triggers(FARM_ADMIRE_STREAK_MIN, on_edge, true));
    }

    #[test]
    fn say_line_is_deterministic_and_carries_name() {
        let a = admire_say_line("露娜客", 0);
        let b = admire_say_line("露娜客", 0);
        assert_eq!(a, b);
        assert!(a.contains("露娜客"));
        assert!(!admire_say_line("阿旅", 0).contains("{p}"));
    }

    #[test]
    fn say_lines_are_distinct() {
        let lines: std::collections::HashSet<String> =
            (0..5).map(|p| admire_say_line("旅人", p)).collect();
        assert_eq!(lines.len(), 5, "五句應各不相同");
    }

    #[test]
    fn memory_line_stays_episodic() {
        let line = admire_memory_line("農夫");
        assert!(matches!(
            crate::voxel_memory::classify_importance(&line),
            crate::voxel_memory::Importance::Ephemeral
        ));
        assert!(line.contains("農夫"));
    }

    #[test]
    fn memory_line_no_newline() {
        assert!(!admire_memory_line("旅人").contains('\n'));
    }
}
