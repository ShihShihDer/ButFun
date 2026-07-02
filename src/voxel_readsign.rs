//! 乙太方界·居民讀牌 v1 —— 居民路過玩家立的告示牌（ROADMAP 740）時，偶爾停下腳步、
//! 念出牌上的字並回應一句。
//!
//! **核心信念**：玩家的建造要有後果——你親手寫下的字，世界裡的 AI 居民真的會「看見」。
//! 告示牌（740）本來是死的：文字浮在牌上，只有玩家彼此讀得到。這一刀讓居民也讀得懂——
//! 你在自家門口立一塊「露娜的家」，路過的居民會抬頭念出來、輕輕感嘆一句；你在礦坑口插
//! 一塊「往礦坑↓」，居民讀到會意識到那是路標。人類寫的字，第一次被 AI 居民「注意到」。
//!
//! **成本紀律**：零 LLM、零持久化、零新協議欄位——純規則式分類 + 確定性選句，搭既有 `say`
//! 泡泡廣播（前端無需改一行）。低頻 + 長冷卻，稀少才有感、不洗版。
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
}
