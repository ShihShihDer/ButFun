//! 乙太方界·居民為自己蓋的建物立牌命名 v1（ROADMAP 749）。
//!
//! 741「居民讀牌」讓居民**讀**玩家寫的牌；本模組是它的鏡像——居民蓋完一座建物後，
//! **親手在門前立一塊告示牌**，寫上「露娜的家」「諾娃的水井」。居民第一次拿起人類的
//! 導覽工具（告示牌 740），把自己一磚一瓦蓋出來的家**署上名**——世界從此看得出
//! 哪座是誰的家，AI 居民的建造第一次留下可讀的銘牌。
//!
//! **設計**：牌面文字由「居民名 + 建物種類」確定性生成（零 LLM）；立牌位置由
//! `voxel_ws.rs` 依建物錨點在門前／四邊找一格空地（腳下固體、頭上空氣）放置。
//! 牌子走既有告示牌管線（Sign 方塊 + SignStore + 廣播 + JSONL 持久化），零新協議。
//!
//! **純邏輯層**：`nameplate_text`／`nameplate_say` 皆純函式、確定性、可單元測試；
//! 鎖／世界寫入／廣播／持久化全在 `voxel_ws.rs`。

use crate::voxel_building::BuildKind;
use crate::voxel_sign::sanitize_text;

/// 依建物種類，取牌面用的「所屬物」後綴（繁中，玩家可讀）。
/// 刻意用比 `display_name`（小木屋／花圃）更口語親暱的家園化措辭，讓牌子像「家的名牌」。
fn kind_suffix(kind: BuildKind) -> &'static str {
    match kind {
        BuildKind::House => "的家",
        BuildKind::Well => "的水井",
        BuildKind::Tower => "的瞭望台",
        BuildKind::Garden => "的花圃",
        BuildKind::Pavilion => "的涼亭",
    }
}

/// 產生居民為自己建物立的牌面文字（如「露娜的家」）。
/// 走與玩家立牌同一套 `sanitize_text` 清洗（去控制字元、截 `SIGN_MAX_CHARS`），
/// 確定性、無副作用、可測。名字異常（空白）時回空字串，呼叫方據此略過立牌。
pub fn nameplate_text(resident: &str, kind: BuildKind) -> String {
    let name = resident.trim();
    if name.is_empty() {
        return String::new();
    }
    sanitize_text(&format!("{name}{}", kind_suffix(kind)))
}

/// 立牌當下居民冒的泡泡台詞（≤40 字不破泡泡框），確定性零 LLM。
pub fn nameplate_say(text: &str) -> String {
    let line = format!("蓋好啦，立塊牌子——「{text}」");
    line.chars().take(40).collect()
}

/// 立牌時記進動態牆（Feed）的一行描述，確定性零 LLM。
pub fn nameplate_feed(resident: &str, text: &str) -> String {
    format!("{resident} 在自己蓋好的建物前立了塊牌子「{text}」")
}

/// 居民立牌位置的候選偏移（相對建物中心 (cx,cz) 的水平偏移，依序嘗試）。
/// 建物是 3×3 footprint（dx,dz∈[-1,1]），偏移取 ±2 落在 footprint 外、不壓到牆；
/// 首選 (0,+2)＝小木屋門（門朝 +z）正前方，其餘三邊為備選。
pub const NAMEPLATE_OFFSETS: [(i32, i32); 4] = [(0, 2), (2, 0), (0, -2), (-2, 0)];

/// 立牌時在建物中心上下嘗試的 y 偏移（相對錨點 cy）：先與牆基同層，再上一層（緩坡）、
/// 再下一層（凹地），涵蓋正常地形起伏。找到「空氣＋腳下固體」即用。
pub const NAMEPLATE_Y_TRIES: [i32; 3] = [0, 1, -1];

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_per_kind() {
        assert_eq!(nameplate_text("露娜", BuildKind::House), "露娜的家");
        assert_eq!(nameplate_text("諾娃", BuildKind::Well), "諾娃的水井");
        assert_eq!(nameplate_text("賽勒", BuildKind::Tower), "賽勒的瞭望台");
        assert_eq!(nameplate_text("奧瑞", BuildKind::Garden), "奧瑞的花圃");
    }

    #[test]
    fn text_trims_name_and_strips_control() {
        // 名字含前後空白 → 清洗掉；控制字元換空白後 trim。
        assert_eq!(nameplate_text("  露娜  ", BuildKind::House), "露娜的家");
        assert_eq!(nameplate_text("露\n娜", BuildKind::House), "露 娜的家");
    }

    #[test]
    fn text_empty_name_yields_empty() {
        assert_eq!(nameplate_text("", BuildKind::House), "");
        assert_eq!(nameplate_text("   ", BuildKind::Well), "");
    }

    #[test]
    fn text_never_exceeds_sign_cap() {
        // 超長名字仍被清洗截到 SIGN_MAX_CHARS 以內，牌面不爆框。
        let long = "字".repeat(50);
        let out = nameplate_text(&long, BuildKind::House);
        assert!(out.chars().count() <= crate::voxel_sign::SIGN_MAX_CHARS);
        assert!(!out.is_empty());
    }

    #[test]
    fn say_contains_text_and_fits_bubble() {
        let s = nameplate_say("露娜的家");
        assert!(s.contains("露娜的家"), "泡泡應含牌面文字");
        assert!(s.chars().count() <= 40, "泡泡不破框");
    }

    #[test]
    fn say_caps_at_bubble_width() {
        // 牌面已達上限時泡泡仍不超過 40 字。
        let text = "字".repeat(30);
        let s = nameplate_say(&text);
        assert!(s.chars().count() <= 40);
    }

    #[test]
    fn feed_contains_name_and_text() {
        let f = nameplate_feed("諾娃", "諾娃的水井");
        assert!(f.contains("諾娃"));
        assert!(f.contains("諾娃的水井"));
    }

    #[test]
    fn offsets_are_outside_footprint() {
        // 每個候選偏移都落在 3×3 footprint（|dx|,|dz|≤1）之外，不壓到建物本體。
        for (ox, oz) in NAMEPLATE_OFFSETS {
            assert!(ox.abs() >= 2 || oz.abs() >= 2, "偏移 ({ox},{oz}) 應在 footprint 外");
        }
        // 四個偏移互不重複。
        let mut seen = std::collections::HashSet::new();
        for o in NAMEPLATE_OFFSETS {
            assert!(seen.insert(o), "偏移不應重複: {o:?}");
        }
    }
}
