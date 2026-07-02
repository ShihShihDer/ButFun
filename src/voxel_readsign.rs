//! 乙太方界·居民讀牌 —— 居民路過玩家立的告示牌（ROADMAP 740）時，偶爾停下腳步、
//! 念出牌上的字並回應一句（v1）；讀到的牌**還會在居民心裡留下印象**（v2）。
//!
//! **核心信念**：玩家的建造要有後果——你親手寫下的字，世界裡的 AI 居民真的會「看見」，
//! 而且**記得**。告示牌（740）本來是死的：文字浮在牌上，只有玩家彼此讀得到。
//! v1 讓居民也讀得懂——你在自家門口立一塊「露娜的家」，路過的居民會抬頭念出來、
//! 輕輕感嘆一句；你在礦坑口插一塊「往礦坑↓」，居民讀到會意識到那是路標。
//! **v2 再推一步**：居民讀過的牌**寫進牠的長期記憶**，日後翻開牠的日記（ROADMAP 5
//! 生命故事），會看到牠對「有人在這片土地上留下想說的話」的內心反思——你親手立的牌子，
//! 第一次在 AI 居民的內在生活裡留下一頁。
//!
//! **成本紀律**：零 LLM——念牌純規則式分類 + 確定性選句，記憶摘要純字串拼接；搭既有 `say`
//! 泡泡廣播與既有記憶/日記管線（前端無需改一行）。低頻 + 長冷卻，稀少才有感、不洗版。
//!
//! **純邏輯層**：零 async、零鎖、零 IO；確定性純函式，窮舉可測。
//! 鎖 / 距離掃描 / 副作用在 `voxel_ws.rs`（短鎖即釋、不巢狀、守死鎖鐵律）。

/// 走到牌子這麼近（方塊，XZ 平面）才會注意到、念出來。
pub const READ_RANGE: f32 = 3.0;
/// 讀牌冷卻（秒，純記憶體、重啟歸零）：一次讀牌後要等這麼久才可再讀，稀少才有感。
pub const READ_COOLDOWN: f32 = 240.0;
/// 每 tick 觸發機率（在 READ_RANGE 內有牌子、且沒在說話時才計算；低頻不洗版）。
pub const READ_CHANCE_PER_TICK: f32 = 0.03;
/// 泡泡裡最多展示幾個牌面字元（過長截斷加「…」，避免泡泡超框）。
pub const READ_QUOTE_CHARS: usize = 14;

/// 居民讀牌記憶的識別前綴（居民讀牌 v2）：把「讀到某塊告示牌」寫進居民記憶時，
/// 摘要一律以此開頭，讓日記（`voxel_diary`）能一眼認出這是「讀牌」而非「對話」記憶，
/// 昇華成正確的內心反思。確定性字面標記、零 LLM。
pub const SIGN_MEMORY_TAG: &str = "🪧讀到告示牌";

/// 讀牌記憶掛在哪個「玩家身份鍵」下（居民讀牌 v2）：告示牌不記作者，故用這個
/// 世界級哨兵鍵，讓讀牌記憶**不污染任何真實玩家**的好感/對話上下文，只在**日記**
/// （跨玩家彙整居民所有記憶）裡浮現。刻意用不會與真實玩家鍵相撞的字面。
pub const SIGN_MEMORY_PLAYER: &str = "__voxel_world_sign__";

/// 把「讀到一塊告示牌」昇華成一筆記憶摘要（居民讀牌 v2）。確定性、可測。
///
/// 格式＝[`SIGN_MEMORY_TAG`] + 截短後的引號牌面（複用 [`display_quote`] 的截斷規則，
/// 界定長度、避免超長牌面塞爆記憶）。日記端以 `SIGN_MEMORY_TAG` 前綴辨識。
pub fn sign_memory_summary(sign_text: &str) -> String {
    format!("{SIGN_MEMORY_TAG}{}", display_quote(sign_text))
}

/// 牌面內容粗分類（確定性、無副作用）：決定居民念完後回應的語氣。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignTone {
    /// 像是在標記「家 / 住處」。
    Home,
    /// 像是在指路（含「往／去／路」或箭頭）。
    Direction,
    /// 其它一般留言。
    Generic,
}

/// 依牌面文字粗分類語氣。純字面比對、確定性、可測。
pub fn classify(text: &str) -> SignTone {
    // 指路訊號：方向動詞 + 常見箭頭符號。
    const DIR_WORDS: [&str; 6] = ["往", "去", "路", "口", "→", "↓"];
    const DIR_MORE: [&str; 4] = ["←", "↑", "這邊", "那邊"];
    // 住處訊號。
    const HOME_WORDS: [&str; 5] = ["家", "屋", "窩", "居", "巢"];

    let has = |pats: &[&str]| pats.iter().any(|p| text.contains(p));
    if has(&HOME_WORDS) {
        SignTone::Home
    } else if has(&DIR_WORDS) || has(&DIR_MORE) {
        SignTone::Direction
    } else {
        SignTone::Generic
    }
}

/// 把牌面文字截到展示上限（超過加「…」），供泡泡引用。確定性、可測。
pub fn display_quote(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= READ_QUOTE_CHARS {
        format!("「{}」", text)
    } else {
        let shown: String = chars.into_iter().take(READ_QUOTE_CHARS).collect();
        format!("「{shown}…」")
    }
}

/// 居民讀牌台詞：念出牌上的字（引號）+ 依語氣回應一句。
/// `pick` 通常取居民座標／名字的雜湊，讓不同居民／不同時機說不同句（確定性）。
/// 保證回傳非空、字元數 ≤ 40（泡泡框上限）。
pub fn read_sign_line(sign_text: &str, pick: usize) -> String {
    // 引號部分本身可能因文字過長被 display_quote 截斷；語氣尾巴選短句，
    // 兩者相加控制在泡泡框內（下方再保險 take(40)）。
    const HOME_TAILS: [&str; 3] = ["——原來這是誰的家呀。", "，住這兒的人真幸福。", "，看著就覺得溫暖。"];
    const DIR_TAILS: [&str; 3] = ["，是在指路呢。", "，往那邊走嗎？", "，這是路標吧。"];
    const GEN_TAILS: [&str; 3] = ["，上面寫了字呢。", "，我念了一遍。", "，誰留的話呢？"];

    let quote = display_quote(sign_text);
    let tail = match classify(sign_text) {
        SignTone::Home => HOME_TAILS[pick % HOME_TAILS.len()],
        SignTone::Direction => DIR_TAILS[pick % DIR_TAILS.len()],
        SignTone::Generic => GEN_TAILS[pick % GEN_TAILS.len()],
    };
    let line = format!("{quote}{tail}");
    // 泡泡框保險：≤ 40 字元（正常情況引號已截短、尾巴短，不會觸發）。
    line.chars().take(40).collect()
}

// ── 居民「重返念念不忘的告示牌」v3（ROADMAP 743）─────────────────────────────────────
//
// **核心信念的關鍵一步**：v2 讓讀過的牌寫進居民記憶，但那筆記憶只躺在日記裡——居民的
// **去向**沒有因此改變。v3 讓記憶第一次**驅動移動**：居民讀到一塊讓牠印象深刻的牌子時，
// 把牌子的位置記在心裡當作「心中的地標」；日後閒暇時偶爾會**放下手邊的閒晃、特地走回**
// 那塊牌子前駐足，再念一次、留下一筆「我又回來看看」的記憶。玩家親手立的牌子，第一次真的
// **改變了 AI 居民走去哪裡**——這正是「記憶要驅動行為，不只聊天」的最小可見證明。
//
// **成本紀律**：零 LLM（純規則決策 + 確定性選句）、零持久化（心中地標純記憶體、重啟歸零）、
// 搭既有 `say` 泡泡／記憶／Feed 管線。長冷卻 + 極低機率 + 距離上限，稀少才有感、不洗版、
// 不長途尋路卡死（純函式層界定所有門檻，鎖 / 尋路 / 副作用留在 `voxel_ws.rs`）。

/// 重返冷卻（秒，純記憶體）：一次朝聖後至少隔這麼久才可能再啟程——稀少才有感。
pub const PILGRIMAGE_COOLDOWN: f32 = 900.0;
/// 閒置時每個合格 tick 啟程朝聖的機率（10Hz 下 0.003 ≈ 平均數十秒才偶發，且要過冷卻）。
pub const PILGRIMAGE_CHANCE_PER_TICK: f32 = 0.003;
/// 走到牌子這麼近（方塊，XZ）即算「抵達」，停下駐足、念一句。
pub const PILGRIMAGE_ARRIVE_DIST: f32 = 2.5;
/// 朝聖逾時（秒）：啟程後走這麼久還沒抵達（被地形擋住等）就放棄，設冷卻，不無限走。
pub const PILGRIMAGE_TIMEOUT: f32 = 60.0;
/// 朝聖距離上限（方塊）：只重返這個半徑內的牌子——太遠的不去，避免長途尋路卡死。
pub const PILGRIMAGE_MAX_RANGE: f32 = 48.0;

/// 是否此刻該啟程重返心中的牌子（純決策、確定性、可測）。
/// - `has_cherished`：心裡是否記著一塊牌子。
/// - `idle_free`：目前是否閒置自由（沒在採集/跑腿/探訪/打氣/聚會/跟隨/發明/睡覺）。
/// - `cooldown`：重返冷卻剩餘秒（> 0 不啟程）。
/// - `say_empty`：目前沒在說話（別打斷正在冒的泡泡）。
/// - `roll`：本 tick 擲出的 [0,1) 亂數。
pub fn should_pilgrimage(
    has_cherished: bool,
    idle_free: bool,
    cooldown: f32,
    say_empty: bool,
    roll: f32,
) -> bool {
    has_cherished && idle_free && cooldown <= 0.0 && say_empty && roll < PILGRIMAGE_CHANCE_PER_TICK
}

/// 心中地標的距離是否適合朝聖：在上限內、又不是已經站在牌子腳下（純函式、可測）。
/// `d2` 是居民到牌子的平方水平距離。太近（已在牌前）不必特地走；太遠不去（防卡死）。
pub fn pilgrimage_worth_going(d2: f32) -> bool {
    d2 > PILGRIMAGE_ARRIVE_DIST * PILGRIMAGE_ARRIVE_DIST
        && d2 <= PILGRIMAGE_MAX_RANGE * PILGRIMAGE_MAX_RANGE
}

/// 抵達心中的牌子時冒的泡泡台詞。`quote` 是 [`display_quote`] 產出的（已含引號、已截短）
/// 牌面引文。保證非空、≤ 40 字元（泡泡框上限）。
pub fn revisit_sign_line(quote: &str, pick: usize) -> String {
    const TAILS: [&str; 3] = [
        "……還是忍不住又走回來看看。",
        "，我又回來讀了一遍。",
        "，總會想起這裡寫著的字。",
    ];
    let tail = TAILS[pick % TAILS.len()];
    let line = format!("{quote}{tail}");
    line.chars().take(40).collect()
}

/// 把「重返一塊念念不忘的牌子」昇華成一筆記憶摘要（確定性、可測）。
/// 沿用 [`SIGN_MEMORY_TAG`] 前綴——日記端據此仍歸為「讀牌」主題，讓這份「又回來看看」的
/// 心情併入居民對「有人在此留字」的內心反思，而非污染玩家對話記憶。
pub fn revisit_memory_summary(quote: &str) -> String {
    format!("{SIGN_MEMORY_TAG}（又特地走回來看）{quote}")
}

/// 重返的 Feed 動態文案（第三人稱，供動態列顯示）。`quote` 為 `display_quote` 引文。
pub fn revisit_feed_line(quote: &str) -> String {
    format!("特地走回去，又看了一遍那塊牌子{quote}")
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_home_direction_generic() {
        assert_eq!(classify("露娜的家"), SignTone::Home);
        assert_eq!(classify("諾娃的小屋"), SignTone::Home);
        assert_eq!(classify("往礦坑↓"), SignTone::Direction);
        assert_eq!(classify("去南邊的路"), SignTone::Direction);
        assert_eq!(classify("→這邊"), SignTone::Direction);
        assert_eq!(classify("歡迎光臨"), SignTone::Generic);
    }

    #[test]
    fn home_beats_direction_when_both_present() {
        // 同時含「家」與「往」時，住處語氣優先（先判 Home）。
        assert_eq!(classify("往露娜的家"), SignTone::Home);
    }

    #[test]
    fn display_quote_wraps_and_truncates() {
        assert_eq!(display_quote("露娜的家"), "「露娜的家」");
        // 剛好上限：不截斷。
        let exact: String = "字".repeat(READ_QUOTE_CHARS);
        assert_eq!(display_quote(&exact), format!("「{exact}」"));
        // 超過上限：截到上限 + 省略號。
        let long: String = "字".repeat(READ_QUOTE_CHARS + 5);
        let q = display_quote(&long);
        assert!(q.ends_with("…」"), "超長牌面應以省略號結尾: {q}");
        // 展示字元數 = 上限（不含引號與省略號）。
        assert_eq!(q.chars().filter(|&c| c == '字').count(), READ_QUOTE_CHARS);
    }

    #[test]
    fn read_line_nonempty_and_within_bubble_cap() {
        // 各語氣、各 pick 都非空且 ≤ 40 字元。
        for text in ["露娜的家", "往礦坑↓", "歡迎光臨", &"很長的一段留言".repeat(6)] {
            for pick in 0..9 {
                let line = read_sign_line(text, pick);
                assert!(!line.is_empty(), "台詞不得為空");
                assert!(line.chars().count() <= 40, "泡泡框上限 40 字元: {line}");
            }
        }
    }

    #[test]
    fn read_line_quotes_the_sign_text() {
        // 短牌面應原樣被念出（含引號）。
        let line = read_sign_line("露娜的家", 0);
        assert!(line.contains("「露娜的家」"), "應念出牌面原文: {line}");
    }

    #[test]
    fn read_line_deterministic() {
        // 同輸入同輸出（確定性、可測）。
        assert_eq!(read_sign_line("往礦坑↓", 5), read_sign_line("往礦坑↓", 5));
        // 不同 pick 可選到不同尾巴（覆蓋三句輪替）。
        let a = read_sign_line("歡迎光臨", 0);
        let b = read_sign_line("歡迎光臨", 1);
        assert_ne!(a, b, "不同 pick 應輪替到不同台詞");
    }

    #[test]
    fn sign_memory_summary_tagged_and_quotes_text() {
        // 記憶摘要應帶識別前綴 + 引號牌面（供日記辨識為「讀牌」記憶）。
        let s = sign_memory_summary("露娜的家");
        assert!(s.starts_with(SIGN_MEMORY_TAG), "應以識別前綴開頭: {s}");
        assert!(s.contains("「露娜的家」"), "應含引號牌面原文: {s}");
    }

    #[test]
    fn sign_memory_summary_truncates_long_text() {
        // 超長牌面在記憶摘要裡也被截短（複用 display_quote 規則），不塞爆記憶。
        let long: String = "字".repeat(READ_QUOTE_CHARS + 10);
        let s = sign_memory_summary(&long);
        assert!(s.starts_with(SIGN_MEMORY_TAG));
        assert!(s.ends_with("…」"), "超長牌面記憶摘要應以省略號結尾: {s}");
        assert_eq!(
            s.chars().filter(|&c| c == '字').count(),
            READ_QUOTE_CHARS,
            "展示字元數應等於上限"
        );
    }

    #[test]
    fn sign_memory_summary_deterministic() {
        assert_eq!(sign_memory_summary("往礦坑↓"), sign_memory_summary("往礦坑↓"));
    }

    // ── 重返念念不忘的牌子 v3 ────────────────────────────────────────────────────────

    #[test]
    fn should_pilgrimage_needs_all_conditions() {
        // 全條件齊 + roll 過門檻 → 啟程。
        assert!(should_pilgrimage(true, true, 0.0, true, 0.0));
        // 心裡沒牌 → 不啟程。
        assert!(!should_pilgrimage(false, true, 0.0, true, 0.0));
        // 忙著別的事（非閒置自由）→ 不啟程。
        assert!(!should_pilgrimage(true, false, 0.0, true, 0.0));
        // 冷卻中 → 不啟程。
        assert!(!should_pilgrimage(true, true, 10.0, true, 0.0));
        // 正在說話 → 不打斷。
        assert!(!should_pilgrimage(true, true, 0.0, false, 0.0));
        // roll 高於門檻 → 這 tick 不啟程。
        assert!(!should_pilgrimage(true, true, 0.0, true, PILGRIMAGE_CHANCE_PER_TICK + 0.01));
    }

    #[test]
    fn pilgrimage_worth_going_bounds() {
        // 已站在牌前（很近）→ 不必特地走。
        assert!(!pilgrimage_worth_going(0.0));
        assert!(!pilgrimage_worth_going(PILGRIMAGE_ARRIVE_DIST * PILGRIMAGE_ARRIVE_DIST));
        // 合理距離 → 值得走。
        assert!(pilgrimage_worth_going(100.0));
        // 恰好在上限內 → 值得走；超過上限 → 太遠不去（防卡死）。
        assert!(pilgrimage_worth_going(PILGRIMAGE_MAX_RANGE * PILGRIMAGE_MAX_RANGE - 1.0));
        assert!(!pilgrimage_worth_going(PILGRIMAGE_MAX_RANGE * PILGRIMAGE_MAX_RANGE + 1.0));
    }

    #[test]
    fn revisit_sign_line_nonempty_within_cap_and_quotes() {
        let quote = display_quote("露娜的家");
        for pick in 0..9 {
            let line = revisit_sign_line(&quote, pick);
            assert!(!line.is_empty(), "台詞不得為空");
            assert!(line.chars().count() <= 40, "泡泡框上限 40 字元: {line}");
            assert!(line.contains("「露娜的家」"), "應念出牌面引文: {line}");
        }
        // 超長引文（已被 display_quote 截短）+ 尾巴仍不破框。
        let long = display_quote(&"字".repeat(READ_QUOTE_CHARS + 20));
        assert!(revisit_sign_line(&long, 0).chars().count() <= 40);
    }

    #[test]
    fn revisit_sign_line_deterministic_and_varies() {
        let q = display_quote("往礦坑↓");
        assert_eq!(revisit_sign_line(&q, 3), revisit_sign_line(&q, 3));
        assert_ne!(revisit_sign_line(&q, 0), revisit_sign_line(&q, 1));
    }

    #[test]
    fn revisit_memory_summary_tagged_for_diary() {
        // 沿用讀牌前綴 → 日記端仍歸為「讀牌」主題（Theme::Sign）。
        let s = revisit_memory_summary(&display_quote("露娜的家"));
        assert!(s.starts_with(SIGN_MEMORY_TAG), "應以讀牌前綴開頭供日記辨識: {s}");
        assert!(s.contains("「露娜的家」"), "應含引號牌面: {s}");
        // 確定性。
        assert_eq!(
            revisit_memory_summary(&display_quote("露娜的家")),
            revisit_memory_summary(&display_quote("露娜的家"))
        );
    }

    #[test]
    fn revisit_feed_line_contains_quote() {
        let line = revisit_feed_line(&display_quote("往礦坑↓"));
        assert!(line.contains("「往礦坑↓」"), "Feed 文案應含牌面引文: {line}");
    }
}
