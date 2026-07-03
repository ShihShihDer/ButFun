//! 乙太方界·居民認得鄰居的家 v1（ROADMAP 750）
//!
//! 749（居民立牌命名 v1）讓居民蓋完家後親手在門前立一塊「露娜的家」「諾娃的水井」
//! 的告示牌；741（居民讀牌）讓居民路過**玩家**立的牌時停下念一句、寫進世界級記憶。
//! 本切片把這兩條合流：當居民讀到的牌不是玩家寫的，而是**另一位居民**立的自建物銘牌，
//! 牠會**認出這是哪位鄰居的家**——念一句更親暱的招呼、並把「記住了某某住在這一帶」
//! 寫成一筆**掛在那位鄰居名下**的記憶（不再落到世界級哨兵鍵），讓讀牌記憶第一次
//! 連到「某個具體的鄰居」。小社會的鄰里認知第一次從方塊上長出來。
//!
//! **純邏輯層**：偵測/台詞/記憶皆純函式、確定性、可單元測試；接線（讀牌 tick 分支、
//! 記憶寫入、Feed、泡泡）留在 `voxel_ws.rs`。零新協議、零 migration、零 LLM。

/// 鄰里認家記憶前綴：日記／回想端可據此把這筆歸為「鄰里」主題。
pub const NEIGHBOR_SIGN_TAG: &str = "🏡認得鄰居的家";

/// 從牌面文字認出「這是哪位鄰居立的牌」。
///
/// 749 的銘牌文字恆為「{居民名}的{建物}」（如「露娜的家」「諾娃的水井」）。
/// 若牌面（去頭尾空白後）以名冊裡某位居民的名字**緊接著「的」**開頭，且那位不是
/// 讀牌者本人，就回傳該鄰居名字；否則回 `None`（交回既有的世界級讀牌路徑，行為不變）。
///
/// 「的」的緊接判定是刻意的：避免把玩家隨手寫的「露娜你好嗎」誤認成露娜的自建銘牌，
/// 也讓名字剛好是別人前綴的情況（本作四名無此情形，但仍穩健）被後面的「的」消歧。
pub fn identify_nameplate<'a>(text: &str, reader: &str, roster: &[&'a str]) -> Option<&'a str> {
    let t = text.trim();
    for &name in roster {
        if name == reader {
            continue;
        }
        if let Some(rest) = t.strip_prefix(name) {
            if rest.starts_with('的') {
                return Some(name);
            }
        }
    }
    None
}

/// 居民認出鄰居家牌時的招呼泡泡（比 741 的通用讀牌語更親暱、點名那位鄰居）。
/// `quote` 傳已截短的引文（沿用 [`crate::voxel_readsign::display_quote`]），確保泡泡不超框。
/// `pick` 取居民座標／名字雜湊，讓不同居民／不同時機說不同句（確定性）。
pub fn neighbor_sign_line(neighbor: &str, quote: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "哦——{q}，原來{n}住在這一帶呀。",
        "{q}……記起來了，這是{n}的地方。",
        "路過{n}立的牌子{q}，親手署的名呢。",
    ];
    let s = TEMPLATES[pick % TEMPLATES.len()]
        .replace("{q}", quote)
        .replace("{n}", neighbor);
    // 泡泡框保險：控制在 40 字元內（與 741 一致）。
    s.chars().take(40).collect()
}

/// 掛在「那位鄰居」名下的記憶：記住某某住在這一帶（日後回想／日記可引用）。
/// `quote` 同樣傳截短引文，避免超長牌面塞爆 episodic。
pub fn neighbor_sign_memory(neighbor: &str, quote: &str) -> String {
    format!("{NEIGHBOR_SIGN_TAG}：路過{quote}，記住了{neighbor}就住在這一帶。")
}

/// 城鎮動態 Feed 一行：某居民認出了某鄰居的住處。
pub fn neighbor_sign_feed(reader: &str, neighbor: &str) -> String {
    format!("{reader}路過{neighbor}立的牌子，認出了鄰居的住處。")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROSTER: [&str; 4] = ["露娜", "諾娃", "賽勒", "奧瑞"];

    #[test]
    fn identifies_neighbor_nameplate() {
        // 749 銘牌格式「{名}的{建物}」都認得，且掛到正確鄰居。
        assert_eq!(
            identify_nameplate("諾娃的家", "露娜", &ROSTER),
            Some("諾娃")
        );
        assert_eq!(
            identify_nameplate("賽勒的瞭望台", "露娜", &ROSTER),
            Some("賽勒")
        );
        assert_eq!(
            identify_nameplate("奧瑞的花圃", "諾娃", &ROSTER),
            Some("奧瑞")
        );
    }

    #[test]
    fn does_not_identify_own_nameplate() {
        // 讀到自己立的牌不算「認得鄰居」——交回既有世界級路徑。
        assert_eq!(identify_nameplate("露娜的家", "露娜", &ROSTER), None);
    }

    #[test]
    fn requires_de_right_after_name() {
        // 玩家隨手寫的、只是提到某居民名字的牌，不誤認成該居民的自建銘牌。
        assert_eq!(identify_nameplate("露娜你好嗎", "諾娃", &ROSTER), None);
        assert_eq!(identify_nameplate("往露娜家↓", "諾娃", &ROSTER), None);
        // 名字未出現在開頭也不算。
        assert_eq!(identify_nameplate("歡迎光臨諾娃的家", "露娜", &ROSTER), None);
    }

    #[test]
    fn ignores_non_nameplate_and_blank() {
        assert_eq!(identify_nameplate("往礦坑↓", "露娜", &ROSTER), None);
        assert_eq!(identify_nameplate("", "露娜", &ROSTER), None);
        assert_eq!(identify_nameplate("   ", "露娜", &ROSTER), None);
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            identify_nameplate("  諾娃的水井  ", "露娜", &ROSTER),
            Some("諾娃")
        );
    }

    #[test]
    fn line_names_neighbor_and_fits_bubble() {
        for pick in 0..6usize {
            let s = neighbor_sign_line("諾娃", "「諾娃的家」", pick);
            assert!(s.contains("諾娃"), "泡泡應點名鄰居: {s}");
            assert!(s.chars().count() <= 40, "泡泡應 ≤40 字元: {s}");
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn memory_carries_tag_and_neighbor() {
        let m = neighbor_sign_memory("賽勒", "「賽勒的瞭望台」");
        assert!(m.starts_with(NEIGHBOR_SIGN_TAG), "應以鄰里前綴開頭: {m}");
        assert!(m.contains("賽勒"), "記憶應點名鄰居: {m}");
    }

    #[test]
    fn feed_names_both() {
        let f = neighbor_sign_feed("露娜", "諾娃");
        assert!(f.contains("露娜") && f.contains("諾娃"));
    }
}
