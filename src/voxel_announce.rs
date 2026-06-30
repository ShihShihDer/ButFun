//! 乙太方界·建物完工廣播——居民完成蓋造時對所有玩家宣告，讓世界的成長被看見（ROADMAP 669）。
//!
//! **核心玩法**：居民花數分鐘把花圃/小屋/水井/瞭望台一磚一瓦蓋完後，世界廣播「{居民名}完成了{建物}！」
//! 同時冒出喜悅慶賀泡泡——不管玩家有沒有在場見證，都會在聊天欄看到這個里程碑。
//!
//! **純邏輯層**：`build_complete_say`（慶賀泡泡台詞）與 `build_complete_msg`（JSON 廣播格式）皆純函式。
//! 鎖 / IO / 廣播觸發全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。

use crate::voxel_building::BuildKind;

/// 建物完工時居民冒出的喜悅泡泡（依建物種類 + 居民名字雜湊選模板，確定性）。
/// 字元數控制在 40 以內（泡泡框上限）。
pub fn build_complete_say(resident_name: &str, kind: BuildKind) -> String {
    // 依名字雜湊讓不同居民說不同風格的話。
    let idx = resident_name.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize));
    match kind {
        BuildKind::Garden => {
            let lines = [
                "花圃種好了！你來看看嗎？",
                "我的花圃完工啦，好美喔！",
                "終於種完了！花圃在那邊呢。",
                "花圃蓋好了，世界更美了！",
            ];
            lines[idx % lines.len()].to_string()
        }
        BuildKind::House => {
            let lines = [
                "我的家蓋好了！有個窩真好。",
                "小屋完工啦！終於有家了。",
                "家蓋好了，真是太高興了！",
                "小屋完成了，快來瞧瞧吧！",
            ];
            lines[idx % lines.len()].to_string()
        }
        BuildKind::Well => {
            let lines = [
                "水井完工了！以後不愁水喝。",
                "水井蓋好了，好開心！",
                "我的水井完成了，乾杯！",
                "水井好啦！大家都能用了。",
            ];
            lines[idx % lines.len()].to_string()
        }
        BuildKind::Tower => {
            let lines = [
                "瞭望台蓋好了！能看好遠呢。",
                "瞭望台完工啦，快來爬上來！",
                "終於蓋到頂了！好有成就感。",
                "瞭望台完成了，今晚來觀星！",
            ];
            lines[idx % lines.len()].to_string()
        }
    }
}

/// 建物完工廣播的 WS JSON 字串（broadcast 給所有玩家）。
/// `kind_name` 已是繁中顯示名（如「花圃」「小屋」），由呼叫端提供。
pub fn build_complete_msg(resident_name: &str, kind_name: &str) -> String {
    serde_json::json!({
        "t": "build_complete",
        "resident": resident_name,
        "kind": kind_name,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garden_say_contains_flower_hint() {
        let say = build_complete_say("露娜", BuildKind::Garden);
        assert!(say.contains("花圃"), "花圃台詞應提到花圃：{say}");
        assert!(say.chars().count() <= 40, "台詞不超過40字：{}", say.chars().count());
    }

    #[test]
    fn house_say_contains_house_hint() {
        let say = build_complete_say("諾娃", BuildKind::House);
        assert!(say.contains("屋") || say.contains("家"), "小屋台詞應提到家/屋：{say}");
        assert!(say.chars().count() <= 40);
    }

    #[test]
    fn well_say_contains_well_hint() {
        let say = build_complete_say("賽勒", BuildKind::Well);
        assert!(say.contains("水井") || say.contains("水"), "水井台詞應提到水：{say}");
        assert!(say.chars().count() <= 40);
    }

    #[test]
    fn tower_say_contains_tower_hint() {
        let say = build_complete_say("奧瑞", BuildKind::Tower);
        assert!(say.contains("瞭望台") || say.contains("塔") || say.contains("爬"), "瞭望台台詞應提到瞭望台/塔：{say}");
        assert!(say.chars().count() <= 40);
    }

    #[test]
    fn different_residents_may_have_different_lines() {
        // 不同名字雜湊可能產生不同台詞（不保證絕對不同，但至少函式可呼叫）。
        let a = build_complete_say("露娜", BuildKind::Garden);
        let b = build_complete_say("諾娃", BuildKind::Garden);
        // 兩者都是合法台詞即可（不強要求不同，hash 碰撞是正常的）。
        assert!(!a.is_empty());
        assert!(!b.is_empty());
    }

    #[test]
    fn build_complete_msg_has_resident_and_kind() {
        let msg = build_complete_msg("露娜", "花圃");
        assert!(msg.contains("build_complete"), "type 欄位應有 build_complete");
        assert!(msg.contains("露娜"), "應含居民名");
        assert!(msg.contains("花圃"), "應含建物名");
    }

    #[test]
    fn build_complete_msg_is_valid_json() {
        let msg = build_complete_msg("諾娃", "小木屋");
        let v: serde_json::Value = serde_json::from_str(&msg).expect("應為合法 JSON");
        assert_eq!(v["t"], "build_complete");
        assert_eq!(v["resident"], "諾娃");
        assert_eq!(v["kind"], "小木屋");
    }
}
