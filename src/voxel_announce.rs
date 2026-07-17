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
        BuildKind::Pavilion => {
            let lines = [
                "涼亭搭好了！來乘涼歇歇腳吧。",
                "我的涼亭完工啦，下雨也不怕了！",
                "涼亭蓋好了，大家來坐坐！",
                "亭子好啦！夜裡點著燈真溫暖。",
            ];
            lines[idx % lines.len()].to_string()
        }
        BuildKind::Workshop => {
            let lines = [
                "工坊蓋好了！以後在這裡做東西。",
                "我的工坊完工啦，工作台都擺上了！",
                "工坊好啦！來看我打鐵吧。",
                "作坊完成了，可以好好做工了！",
            ];
            lines[idx % lines.len()].to_string()
        }
        BuildKind::Millhouse => {
            let lines = [
                "磨坊蓋好了！水輪轉起來啦。",
                "我的磨坊完工啦，靠著水邊真好！",
                "磨坊好啦，聽那水車嘎吱嘎吱～",
                "水車磨坊完成了，快來瞧瞧！",
            ];
            lines[idx % lines.len()].to_string()
        }
        BuildKind::Monument => {
            let lines = [
                "紀念碑立起來了！遠遠就看得見。",
                "我的紀念碑完工啦，記住這一刻。",
                "石碑高高立好了，來看看吧！",
                "紀念碑完成了，碑頂的燈夜裡好亮。",
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

// ── 合力蓋家完工功勞 v1（ROADMAP 834）───────────────────────────────────────────
//
// 696「居民互助蓋家」讓老朋友到訪時順手幫忙推進一塊，但完工那一刻此前只有屋主一人
// 被祝賀／被廣播——本節補上「有人幫忙就一起感謝」，讓小社會的集體行動在完工瞬間可見。

/// 建物完工時的慶賀泡泡：有協力者就改口一起感謝，沒有則與 [`build_complete_say`] 完全相同
/// （零協力者時行為不變，舊呼叫端／既有測試不受影響）。
pub fn build_complete_say_with_helpers(resident_name: &str, kind: BuildKind, helpers: &[String]) -> String {
    if helpers.is_empty() {
        return build_complete_say(resident_name, kind);
    }
    let idx = resident_name.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize));
    let names = helpers.join("、");
    let pool: &[&str] = &[
        "{k}蓋好了！多虧{h}幫忙，一起完成的！",
        "跟{h}一起把{k}蓋好啦，謝謝你們！",
        "{k}完工！這是我和{h}一起蓋的。",
        "有{h}幫忙，{k}蓋得特別快，謝謝！",
    ];
    pool[idx % pool.len()]
        .replace("{h}", &names)
        .replace("{k}", kind.display_name())
        .chars()
        .take(40)
        .collect()
}

/// 建物完工廣播的 WS JSON 字串，額外帶上協力者名單（additive `helpers` 欄位，
/// 舊前端安全忽略；新前端可據此在系統訊息裡多提一句「與誰合力」）。
pub fn build_complete_msg_with_helpers(resident_name: &str, kind_name: &str, helpers: &[String]) -> String {
    serde_json::json!({
        "t": "build_complete",
        "resident": resident_name,
        "kind": kind_name,
        "helpers": helpers,
    })
    .to_string()
}

// ── 心願真的成真 v1（ROADMAP 720）──────────────────────────────────────────────
//
// 玩家的話種下居民的心願（`voxel_desires`），居民照著心願蓋（`voxel_building`），
// 但完工那一刻此前只用通用台詞/廣播，從不提「這是因為你」——本節補上指名感謝，
// 讓「你的互動真的有後果」在心願實現的當下被玩家親眼看見。

/// 心願成真時，居民對啟發者指名感謝的喜悅泡泡（依居民名字雜湊選模板，確定性，≤40 字）。
pub fn wish_come_true_say(resident_name: &str, kind: BuildKind, player_name: &str) -> String {
    let idx = resident_name.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize));
    let pool: &[&str] = &[
        "{p}，記得你說過的話嗎？我把{k}蓋好了！",
        "多虧{p}當初那句話，{k}終於蓋成真的了！",
        "{p}，你的話我一直記著——{k}蓋好啦！",
        "因為{p}，我才有動力把{k}蓋完，謝謝你！",
    ];
    pool[idx % pool.len()]
        .replace("{p}", player_name)
        .replace("{k}", kind.display_name())
        .chars()
        .take(40)
        .collect()
}

/// 心願成真廣播的 WS JSON 字串（broadcast 給所有在線玩家；additive 新欄位，舊前端安全忽略）。
pub fn wish_come_true_msg(resident_name: &str, kind_name: &str, player_name: &str) -> String {
    serde_json::json!({
        "t": "wish_come_true",
        "resident": resident_name,
        "kind": kind_name,
        "player": player_name,
    })
    .to_string()
}

/// 記進啟發者玩家記憶庫的摘要句（居民自己記得「我幫你完成了這件事」，供之後對話回想引用）。
pub fn wish_come_true_memory(kind_name: &str) -> String {
    format!("我因為你的一句話，把{kind_name}蓋好了，謝謝你。")
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

    #[test]
    fn wish_come_true_say_contains_player_and_kind_within_limit() {
        for kind in [BuildKind::Garden, BuildKind::House, BuildKind::Well, BuildKind::Tower] {
            let say = wish_come_true_say("露娜", kind, "旅人");
            assert!(say.contains("旅人"), "應含玩家名：{say}");
            assert!(say.contains(kind.display_name()), "應含建物名：{say}");
            assert!(say.chars().count() <= 40, "不超過泡泡上限 40 字：{}", say.chars().count());
        }
    }

    #[test]
    fn wish_come_true_say_all_residents_nonempty() {
        for name in ["露娜", "諾娃", "賽勒", "奧瑞"] {
            let say = wish_come_true_say(name, BuildKind::House, "小明");
            assert!(!say.is_empty());
            assert!(say.chars().count() <= 40);
        }
    }

    #[test]
    fn wish_come_true_msg_is_valid_json_with_player() {
        let msg = wish_come_true_msg("露娜", "小木屋", "旅人");
        let v: serde_json::Value = serde_json::from_str(&msg).expect("應為合法 JSON");
        assert_eq!(v["t"], "wish_come_true");
        assert_eq!(v["resident"], "露娜");
        assert_eq!(v["kind"], "小木屋");
        assert_eq!(v["player"], "旅人");
    }

    #[test]
    fn wish_come_true_memory_contains_kind_and_bounded() {
        let msg = wish_come_true_memory("瞭望台");
        assert!(msg.contains("瞭望台"));
        assert!(msg.chars().count() <= 80, "在 SUMMARY_MAX_CHARS 之內：{}", msg.chars().count());
    }

    // ── 合力蓋家完工功勞（ROADMAP 834）────────────────────────────────────────────

    #[test]
    fn say_with_no_helpers_matches_original() {
        let a = build_complete_say_with_helpers("露娜", BuildKind::House, &[]);
        let b = build_complete_say("露娜", BuildKind::House);
        assert_eq!(a, b, "零協力者時應與原函式完全相同，舊行為不變");
    }

    #[test]
    fn say_with_helpers_mentions_helper_and_kind_within_limit() {
        let helpers = vec!["賽勒".to_string()];
        let say = build_complete_say_with_helpers("露娜", BuildKind::Well, &helpers);
        assert!(say.contains("賽勒"), "應提到協力者：{say}");
        assert!(say.contains("水井"), "應提到建物種類：{say}");
        assert!(say.chars().count() <= 40, "不超過泡泡上限 40 字：{}", say.chars().count());
    }

    #[test]
    fn say_with_multiple_helpers_joined_and_bounded() {
        let helpers = vec!["賽勒".to_string(), "諾娃".to_string()];
        for kind in [BuildKind::Garden, BuildKind::House, BuildKind::Well, BuildKind::Tower, BuildKind::Pavilion] {
            let say = build_complete_say_with_helpers("奧瑞", kind, &helpers);
            assert!(say.chars().count() <= 40, "不超過泡泡上限 40 字：{}", say.chars().count());
        }
    }

    #[test]
    fn msg_with_helpers_is_valid_json_and_lists_helpers() {
        let helpers = vec!["賽勒".to_string(), "諾娃".to_string()];
        let msg = build_complete_msg_with_helpers("露娜", "小木屋", &helpers);
        let v: serde_json::Value = serde_json::from_str(&msg).expect("應為合法 JSON");
        assert_eq!(v["t"], "build_complete");
        assert_eq!(v["resident"], "露娜");
        assert_eq!(v["kind"], "小木屋");
        assert_eq!(v["helpers"], serde_json::json!(["賽勒", "諾娃"]));
    }

    #[test]
    fn msg_with_no_helpers_has_empty_array() {
        let msg = build_complete_msg_with_helpers("諾娃", "水井", &[]);
        let v: serde_json::Value = serde_json::from_str(&msg).expect("應為合法 JSON");
        assert_eq!(v["helpers"], serde_json::json!([]));
    }
}
