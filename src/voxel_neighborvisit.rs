//! 乙太方界·居民登門找鄰居串門子 v1（ROADMAP 751）
//!
//! 750（居民認得鄰居的家）讓居民讀到**另一位居民**親手立的家牌（749）時認出「這是誰家」、
//! 把「某某住在這一帶」記成掛在那位鄰居名下的記憶，同時把那塊牌記成心中的地標（743 朝聖）。
//! 但至此為止，居民日後「重返」那塊牌時（743）仍只是**獨自**對牌懷念一句——牠明明認得
//! 這是鄰居的家，卻沒把這趟走過去當成一次真正的**登門拜訪**。
//!
//! 本切片把兩條接成閉環：當居民朝聖的心中地標其實是**某位鄰居的家牌**，抵達那一刻不再是
//! 孤獨的懷舊，而是一次**登門串門子**——牠會暖暖點名招呼、把「特地繞到某某家門口探望」記成
//! 掛在那位鄰居名下的記憶，而且這趟登門會讓**彼此的情誼加溫**（沿用 672 的情誼帳本
//! `record_visit`，可能因此升級成老朋友）。記憶第一次把居民的腳步導向「某位鄰居的家門口」
//! 找本人，個體↔個體因「你記得我把家安在哪、還特地繞來」而真的更親。
//!
//! **純邏輯層**：招呼台詞／記憶／Feed 皆確定性純函式、可單元測試；朝聖狀態機、情誼帳本寫入、
//! Feed／記憶 IO 全留在 `voxel_ws.rs`（沿用既有短鎖不巢狀的鎖序）。零新協議、零 migration、零 LLM。

/// 登門串門子記憶前綴：日記／回想端可據此把這筆歸為「鄰里往來」主題。
pub const NEIGHBOR_VISIT_TAG: &str = "🏡登門串門子";

/// 居民走到鄰居家門口（朝聖地標其實是鄰居家牌時）冒出的暖招呼泡泡。
/// 比 743 的獨自懷舊語（`revisit_sign_line`）更親暱、點名那位鄰居——這是一次「登門」而非「懷舊」。
/// `pick` 取居民座標／時機雜湊，讓不同居民／不同時機說不同句（確定性、零 LLM）。
pub fn visit_line(neighbor: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "繞到{n}家門口了，難得順路來看看～",
        "記得{n}就住這兒，特地過來串個門子。",
        "{n}在家嗎？路過就想來坐坐呢。",
    ];
    let s = TEMPLATES[pick % TEMPLATES.len()].replace("{n}", neighbor);
    // 泡泡框保險：控制在 40 字元內（與 741/750 一致）。
    s.chars().take(40).collect()
}

/// 掛在「那位鄰居」名下的記憶：特地繞到某某家門口探望（日後回想／日記可引用）。
pub fn visit_memory(neighbor: &str) -> String {
    format!("{NEIGHBOR_VISIT_TAG}：特地繞到{neighbor}家門口探望，跟這位鄰居更熟了。")
}

/// 城鎮動態 Feed 一行：某居民特地登門找某鄰居串門子。
pub fn visit_feed(reader: &str, neighbor: &str) -> String {
    format!("{reader}特地繞到{neighbor}家門口串門子。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_names_neighbor_and_fits_bubble() {
        for pick in 0..6usize {
            let s = visit_line("諾娃", pick);
            assert!(s.contains("諾娃"), "泡泡應點名鄰居: {s}");
            assert!(s.chars().count() <= 40, "泡泡應 ≤40 字元: {s}");
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn line_varies_with_pick() {
        // 至少兩個不同 pick 給出不同台詞（避免退化成單句）。
        let a = visit_line("露娜", 0);
        let b = visit_line("露娜", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn memory_carries_tag_and_neighbor() {
        let m = visit_memory("賽勒");
        assert!(m.starts_with(NEIGHBOR_VISIT_TAG), "應以登門串門子前綴開頭: {m}");
        assert!(m.contains("賽勒"), "記憶應點名鄰居: {m}");
    }

    #[test]
    fn feed_names_both() {
        let f = visit_feed("露娜", "奧瑞");
        assert!(f.contains("露娜") && f.contains("奧瑞"));
    }
}
