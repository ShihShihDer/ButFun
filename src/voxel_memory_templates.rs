//! 乙太方界 居民記憶「句子模板庫」——純函式、零 LLM、確定性、可測。
//!
//! ## 為什麼要有這個模組
//! 居民日常會反覆做同一類事（黃昏採集、發明失敗、好奇某目標、搬去與誰為鄰），
//! 若每次都套同一句罐頭文字，記憶／動態牆／日記讀起來就是「複製貼上」的疲乏感。
//! 本模組把「同一件事的**不同上下文**」映射到**不同措辭**：
//! - **時間尺度的第 N 次**（這一季第幾回黃昏採集）
//! - **在場人數**（獨自 / 三兩個 / 一群人）
//! - **序次**（第幾次嘗試發明、第幾輪好奇）
//! - **情感梯度**（搬去與**至交**為鄰 vs 泛泛鄰居）
//!
//! 讓同一件事在不同情境下產出**不同句子**，記憶自然去重、不再罐頭。
//!
//! ## 邊界
//! 本模組**只放與連線/鎖/LLM/落地無關的確定性純字串生成**——不 await、不碰檔案、不觸鎖。
//! 呼叫源（把這些句子接進 `voxel_custom`/`relations`/`invent` 等）是後續 PR 的事，
//! 這一版只提供「拿輸入吐句子」的模板庫本身，附窮舉測試證明「同輸入不同上下文 → 不同句」。
//!
//! 不抄外部碼、繁中註解、機敏值不涉入；純函式無副作用。

// 整個模組是前置地基，接線輪（把句子接進 voxel_custom/relations/invent）才有呼叫端，
// 比照 `gather_field.rs` 等未接線純邏輯模組標 `allow(dead_code)`。
#![allow(dead_code)]

// ── 情感分檔小工具 ────────────────────────────────────────────────────────────

/// 把「在場人數」摺成三檔語氣：0=獨自、1=三兩人、2=一群人。
#[inline]
fn headcount_tier(headcount: usize) -> usize {
    match headcount {
        0 | 1 => 0, // 只有自己（或身邊沒別人）
        2..=3 => 1, // 三兩個
        _ => 2,     // 一群人
    }
}

// ── 1. 黃昏採集 ───────────────────────────────────────────────────────────────

/// 黃昏採集記憶句。
///
/// 依「這一季第 `nth_this_season` 次黃昏採集」×「在場人數 `headcount`」×「季節中文名 `season_zh`」
/// 產出不同措辭：第一次帶新鮮感、往後帶熟稔感；獨自帶靜謐、有伴帶熱鬧。
///
/// - `nth_this_season`：這一季第幾次（1 起算；0 視同第 1 次）。
/// - `headcount`：黃昏時身邊（含自己）約略人數。
/// - `season_zh`：季節中文名（如「春天」「秋天」），直接嵌進句子。
pub fn dusk_gather_line(nth_this_season: usize, headcount: usize, season_zh: &str) -> String {
    let tier = headcount_tier(headcount);
    let nth = nth_this_season.max(1);

    // 第一次 vs 往後：第一次有「今年頭一回」的儀式感
    if nth == 1 {
        let first = [
            format!("{season_zh}的頭一回黃昏採集，天邊燒得正紅，我一個人慢慢拾著。"),
            format!("{season_zh}第一次趁黃昏出來採集，身邊有人一起，暖著呢。"),
            format!("{season_zh}的第一場黃昏採集，一群人邊採邊笑，熱鬧得很。"),
        ];
        return first[tier].clone();
    }

    // 往後：把「第幾次」也編進去，愈採愈熟門熟路；次數多（≥5）語氣更淡定
    let depth = if nth >= 5 { 1 } else { 0 };
    let later = [
        [
            format!("{season_zh}裡又獨自採了一回黃昏（這季第{nth}次了），路都摸熟了。"),
            format!("這{season_zh}第{nth}趟黃昏採集，有伴同行，比一個人快活。"),
            format!("{season_zh}第{nth}次黃昏採集，人多手雜，一下就滿載了。"),
        ],
        [
            format!("黃昏又出來拾東西，這{season_zh}第{nth}回，我一個人也自在。"),
            format!("{season_zh}第{nth}次黃昏採集，幾個人結伴，說說笑笑就採完了。"),
            format!("這{season_zh}都第{nth}次黃昏採集了，一大群人，像趕集似的。"),
        ],
    ];
    later[depth][tier].clone()
}

// ── 2. 發明失敗 ───────────────────────────────────────────────────────────────

/// 發明失敗記憶句。
///
/// 依「想做的東西 `goal`」×「第 `attempt_nth` 次嘗試」產出不同措辭：
/// 前幾次帶懊惱、屢敗屢戰後帶不服輸／自嘲的情感梯度。
///
/// - `goal`：想發明／做出來的東西（如「風車」），嵌進句子。
/// - `attempt_nth`：第幾次嘗試（1 起算；0 視同第 1 次）。
pub fn invent_fail_line(goal: &str, attempt_nth: usize) -> String {
    let n = attempt_nth.max(1);
    // 情感梯度：1 首挫、2~3 懊惱、4~6 不服輸、≥7 自嘲豁達
    let tier = match n {
        1 => 0,
        2..=3 => 1,
        4..=6 => 2,
        _ => 3,
    };
    let lines = [
        format!("第一次試著做「{goal}」，沒成。差在哪還沒摸清，再想想。"),
        format!("「{goal}」又失敗了，這是第{n}次。哪個環節不對呢……"),
        format!("第{n}次做「{goal}」還是不成，可我偏不信邪，明天接著來。"),
        format!("「{goal}」試到第{n}次了，我都快跟它耗上了，笑一笑，繼續。"),
    ];
    lines[tier].clone()
}

// ── 3. 好奇某目標 ─────────────────────────────────────────────────────────────

/// 對某目標的好奇記憶句。
///
/// 依「好奇的對象 `goal`」×「第 `cycle` 輪好奇」產出不同措辭：
/// 剛萌生好奇時帶探頭探腦的新鮮，反覆惦記後帶執念般的深入。
///
/// - `goal`：好奇／惦記的東西（如「那口古井」）。
/// - `cycle`：第幾輪好奇（1 起算；0 視同第 1 次）。
pub fn curiosity_line(goal: &str, cycle: usize) -> String {
    let c = cycle.max(1);
    // 梯度：1 萌芽、2~3 上心、≥4 執念
    let tier = match c {
        1 => 0,
        2..=3 => 1,
        _ => 2,
    };
    let lines = [
        format!("我開始對「{goal}」上心了，總忍不住多看幾眼。"),
        format!("又惦記起「{goal}」，這已經是第{c}回了，愈想愈有意思。"),
        format!("「{goal}」在我心裡盤了第{c}輪，不弄明白，我大概睡不安穩。"),
    ];
    lines[tier].clone()
}

// ── 4. 搬遷（與誰為鄰）─────────────────────────────────────────────────────────

/// 搬遷記憶句。
///
/// 依「搬遷主體 `subject`」×「去處 `dest`」×「是否搬去與**至交**為鄰 `is_close_friend`」
/// 產出不同措辭：與至交為鄰帶雀躍親暱，與泛泛鄰居為鄰帶平實。
///
/// - `subject`：搬遷的人（如「露娜」）。
/// - `dest`：搬去的地方／某人身旁（如「諾娃家旁」）。
/// - `is_close_friend`：去處是否為至交身旁——決定情感梯度。
pub fn relocation_line(subject: &str, dest: &str, is_close_friend: bool) -> String {
    if is_close_friend {
        format!("{subject}搬到了{dest}，能天天和知心的人做鄰居，笑得合不攏嘴。")
    } else {
        format!("{subject}搬到了{dest}，換個地方落腳，慢慢和新鄰居熟起來。")
    }
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // ── headcount_tier 基礎行為 ───────────────────────────────────────────────

    #[test]
    fn headcount_tier_buckets() {
        assert_eq!(headcount_tier(0), 0);
        assert_eq!(headcount_tier(1), 0);
        assert_eq!(headcount_tier(2), 1);
        assert_eq!(headcount_tier(3), 1);
        assert_eq!(headcount_tier(4), 2);
        assert_eq!(headcount_tier(100), 2);
    }

    // ── 1. dusk_gather_line ───────────────────────────────────────────────────

    #[test]
    fn dusk_gather_nonempty_and_contains_season() {
        let s = dusk_gather_line(1, 1, "春天");
        assert!(!s.is_empty());
        assert!(s.contains("春天"), "應嵌入季節名：{s}");
    }

    #[test]
    fn dusk_gather_headcount_changes_wording() {
        // 同一次、同一季，只有人數不同 → 句子必須不同
        let alone = dusk_gather_line(1, 1, "秋天");
        let small = dusk_gather_line(1, 3, "秋天");
        let crowd = dusk_gather_line(1, 8, "秋天");
        let set: HashSet<_> = [alone, small, crowd].into_iter().collect();
        assert_eq!(set.len(), 3, "獨自/小群/一群人應各不同句");
    }

    #[test]
    fn dusk_gather_first_differs_from_later() {
        // 同人數、同季，第 1 次 vs 第 3 次 → 不同句
        let first = dusk_gather_line(1, 1, "夏天");
        let later = dusk_gather_line(3, 1, "夏天");
        assert_ne!(first, later, "頭一回與往後應措辭不同");
    }

    #[test]
    fn dusk_gather_familiarity_depth_gradient() {
        // 次數少 vs 次數多（≥5）→ 熟稔梯度不同句
        let few = dusk_gather_line(2, 1, "冬天");
        let many = dusk_gather_line(9, 1, "冬天");
        assert_ne!(few, many, "第 2 次與第 9 次應有熟稔梯度差異");
    }

    #[test]
    fn dusk_gather_zero_nth_treated_as_first() {
        assert_eq!(dusk_gather_line(0, 1, "春天"), dusk_gather_line(1, 1, "春天"));
    }

    // ── 2. invent_fail_line ───────────────────────────────────────────────────

    #[test]
    fn invent_fail_contains_goal_and_nonempty() {
        let s = invent_fail_line("風車", 1);
        assert!(s.contains("風車"), "應含目標名：{s}");
        assert!(!s.is_empty());
    }

    #[test]
    fn invent_fail_emotional_gradient_all_distinct() {
        // 四檔情感梯度（1 / 2-3 / 4-6 / ≥7）應各不同句
        let g = "水車";
        let l1 = invent_fail_line(g, 1);
        let l2 = invent_fail_line(g, 2);
        let l3 = invent_fail_line(g, 5);
        let l4 = invent_fail_line(g, 9);
        let set: HashSet<_> = [l1, l2, l3, l4].into_iter().collect();
        assert_eq!(set.len(), 4, "四個情感檔位應各異");
    }

    #[test]
    fn invent_fail_same_tier_embeds_attempt_number() {
        // 同一檔位（2~3）→ 同模板但嵌不同次數
        let a = invent_fail_line("鐘", 2);
        let b = invent_fail_line("鐘", 3);
        assert!(a.contains("第2次") && b.contains("第3次"));
    }

    #[test]
    fn invent_fail_zero_attempt_treated_as_first() {
        assert_eq!(invent_fail_line("犁", 0), invent_fail_line("犁", 1));
    }

    // ── 3. curiosity_line ─────────────────────────────────────────────────────

    #[test]
    fn curiosity_contains_goal_and_nonempty() {
        let s = curiosity_line("那口古井", 1);
        assert!(s.contains("那口古井"));
        assert!(!s.is_empty());
    }

    #[test]
    fn curiosity_gradient_distinct() {
        let g = "會發光的礦石";
        let c1 = curiosity_line(g, 1);
        let c2 = curiosity_line(g, 2);
        let c3 = curiosity_line(g, 5);
        let set: HashSet<_> = [c1, c2, c3].into_iter().collect();
        assert_eq!(set.len(), 3, "萌芽/上心/執念三檔應各異");
    }

    #[test]
    fn curiosity_zero_cycle_treated_as_first() {
        assert_eq!(curiosity_line("星圖", 0), curiosity_line("星圖", 1));
    }

    // ── 4. relocation_line ────────────────────────────────────────────────────

    #[test]
    fn relocation_contains_subject_and_dest() {
        let s = relocation_line("露娜", "諾娃家旁", true);
        assert!(s.contains("露娜"));
        assert!(s.contains("諾娃家旁"));
    }

    #[test]
    fn relocation_close_friend_changes_tone() {
        let close = relocation_line("露娜", "諾娃家旁", true);
        let plain = relocation_line("露娜", "諾娃家旁", false);
        assert_ne!(close, plain, "至交為鄰 vs 泛泛鄰居應措辭不同");
    }

    // ── 綜合：同一件事不同上下文 → 不同句（模組核心保證）──────────────────────

    #[test]
    fn same_event_different_context_yields_different_lines() {
        // 黃昏採集：把「同一件事（採集）」放進多種上下文，確認散得開
        let mut seen: HashSet<String> = HashSet::new();
        for nth in [1usize, 2, 5] {
            for hc in [1usize, 3, 8] {
                for season in ["春天", "秋天"] {
                    seen.insert(dusk_gather_line(nth, hc, season));
                }
            }
        }
        // 3 nth × 3 headcount × 2 season = 18 組上下文，去重後應遠多於 1（不罐頭）
        assert!(seen.len() >= 10, "同一件事在不同上下文應產出多樣句子，實得 {}", seen.len());
    }
}
