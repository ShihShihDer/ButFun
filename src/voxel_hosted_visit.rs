//! 乙太方界·登門遇主人在家便當面迎客 v1（ROADMAP 752）
//!
//! 751（居民登門找鄰居串門子）讓居民朝聖抵達的其實是某位鄰居親手立的家牌時，把這趟走過去
//! 當成一次真正的「登門拜訪」——暖暖點名招呼、記憶掛在那位鄰居名下、情誼因這趟登門而加溫。
//! 但至此為止，這趟登門**永遠只是對著門口**：那位鄰居此刻在不在家、有沒有真的見到本人，
//! 完全沒差——訪客照樣念一句「路過就想來坐坐呢」，主人就算正好站在自家門前也毫無反應。
//! 讀起來像是每次都撲了個空、跟真實的串門子差得遠。
//!
//! 本切片讓「主人在不在家」第一次有差：當訪客登門抵達的那一刻，那位鄰居**正好也在家**
//!（站在自家牌子附近）時，這趟登門就不再是撲空——訪客會雀躍地說「真的見到本人了」，而**主人**
//! 也第一次有了迎客的能動性：牠會回一句點名訪客的暖招呼，並把「某鄰居特地登門、我正好在家、
//! 親自迎了迎」記成一筆掛在訪客名下的記憶。小社會的鄰里往來，第一次從「單向走過去」長成了
//! 「當面碰上、你來我往」——這是路線圖④「居民↔居民關係→小社會湧現」把 751 的登門補上另一半的一刀。
//!
//! **情誼不重複記帳**：兩位居民的情誼帳本（672）對稱（`bond_key(a,b)` 正規化），751 抵達時
//! 已 `record_visit(訪客, 主人)` 記了這對的一次往來——本切片**不再加記一次**，只補上「見到本人」
//! 的當面互動與主人側的記憶／迎客泡泡，避免情誼被灌爆（守成本紀律）。
//!
//! **純邏輯層**：主人在家判定／台詞／記憶／Feed 皆確定性純函式、可單元測試；居民狀態機、
//! 位置快照、記憶／Feed IO 全留在 `voxel_ws.rs`（沿用既有短鎖不巢狀的鎖序）。零新協議、零 migration、零 LLM。

/// 主人算「在家」的半徑（世界座標，方塊）：站在自家牌子這半徑內才算正好在家迎客。
/// 比朝聖抵達半徑（`PILGRIMAGE_ARRIVE_DIST` = 2.5）稍大——主人「在家附近」就算在家，
/// 不必剛好站在牌子腳下；訪客抵達（2.5 內）時主人若也在這 5 格圈內，兩人就碰上了。
pub const HOST_HOME_DIST: f32 = 5.0;

/// 主人側記憶前綴：日記／回想端可據此把這筆歸為「在家迎客」主題。
pub const HOST_WELCOME_TAG: &str = "🏡在家迎客";

/// 那位鄰居此刻是否正好在家（站在自家牌子附近）。
///
/// 以自家牌子座標 `(plate_x, plate_z)` 為家的中心，主人當前座標在 `HOST_HOME_DIST` 內即算在家。
/// 任一座標非有限值（壞資料）時保守回 `false`——寧可當成撲空走既有路徑，也不誤判在家。
pub fn host_is_home(host_x: f32, host_z: f32, plate_x: f32, plate_z: f32) -> bool {
    if !(host_x.is_finite() && host_z.is_finite() && plate_x.is_finite() && plate_z.is_finite()) {
        return false;
    }
    let dx = host_x - plate_x;
    let dz = host_z - plate_z;
    dx * dx + dz * dz <= HOST_HOME_DIST * HOST_HOME_DIST
}

/// 訪客當面見到本人時冒的暖句（比 751 對空門口的 `visit_line` 更雀躍——這次真的見到人了）。
/// `pick` 取居民座標／時機雜湊，讓不同居民／不同時機說不同句（確定性、零 LLM）。
pub fn met_line(host: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "{n}真的在家！太好了，特地繞來看看你～",
        "哎呀{n}你在呀！那我就不客氣進來坐坐囉。",
        "可算碰上{n}本人了，來串個門子、聊聊近況！",
    ];
    let s = TEMPLATES[pick % TEMPLATES.len()].replace("{n}", host);
    // 泡泡框保險：控制在 40 字元內（與 741/750/751 一致）。
    s.chars().take(40).collect()
}

/// 主人在家、迎接登門訪客時回的暖招呼泡泡（點名那位訪客）。
/// `pick` 由呼叫端傳入（沿用訪客抵達時算好的雜湊），確定性、零 LLM。
pub fn host_welcome_line(guest: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "是{n}呀！快進來快進來，難得你特地繞來～",
        "{n}來啦！我正好在家，來得剛好，坐坐吧。",
        "哎呀是{n}登門！稀客稀客，我這就沏茶去。",
    ];
    let s = TEMPLATES[pick % TEMPLATES.len()].replace("{n}", guest);
    s.chars().take(40).collect()
}

/// 掛在「那位訪客」名下、主人側的記憶：某鄰居特地登門、我正好在家、親自迎了迎。
/// 主人的記憶第一次記下「有人來串門子、我在家接待了」——鄰里往來從主人這一側也留下了痕跡。
pub fn host_welcome_memory(guest: &str) -> String {
    format!("{HOST_WELCOME_TAG}：{guest}特地登門串門子，我正好在家，親自迎了迎，聊得暖暖的。")
}

/// 城鎮動態 Feed 一行：主人正好在家、親自迎接了登門的訪客。
pub fn hosted_feed(guest: &str, host: &str) -> String {
    format!("{host}正好在家，親自迎接了登門串門子的{guest}！")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_home_within_radius() {
        // 剛好站在牌子腳下：在家。
        assert!(host_is_home(10.0, 10.0, 10.0, 10.0));
        // 半徑內（3,4 → 距離 5 = 邊界）：含邊界算在家。
        assert!(host_is_home(13.0, 14.0, 10.0, 10.0));
        // 半徑外：撲空。
        assert!(!host_is_home(20.0, 20.0, 10.0, 10.0));
    }

    #[test]
    fn host_home_rejects_bad_coords() {
        assert!(!host_is_home(f32::NAN, 0.0, 0.0, 0.0));
        assert!(!host_is_home(0.0, 0.0, f32::INFINITY, 0.0));
    }

    #[test]
    fn met_line_names_host_and_fits_bubble() {
        for pick in 0..6usize {
            let s = met_line("諾娃", pick);
            assert!(s.contains("諾娃"), "泡泡應點名主人: {s}");
            assert!(s.chars().count() <= 40, "泡泡應 ≤40 字元: {s}");
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn met_line_varies_with_pick() {
        assert_ne!(met_line("露娜", 0), met_line("露娜", 1));
    }

    #[test]
    fn host_welcome_line_names_guest_and_fits_bubble() {
        for pick in 0..6usize {
            let s = host_welcome_line("賽勒", pick);
            assert!(s.contains("賽勒"), "泡泡應點名訪客: {s}");
            assert!(s.chars().count() <= 40, "泡泡應 ≤40 字元: {s}");
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn host_welcome_line_varies_with_pick() {
        assert_ne!(host_welcome_line("奧瑞", 0), host_welcome_line("奧瑞", 1));
    }

    #[test]
    fn host_memory_carries_tag_and_guest() {
        let m = host_welcome_memory("諾娃");
        assert!(m.starts_with(HOST_WELCOME_TAG), "應以在家迎客前綴開頭: {m}");
        assert!(m.contains("諾娃"), "記憶應點名訪客: {m}");
    }

    #[test]
    fn hosted_feed_names_both() {
        let f = hosted_feed("露娜", "奧瑞");
        assert!(f.contains("露娜") && f.contains("奧瑞"));
    }
}
