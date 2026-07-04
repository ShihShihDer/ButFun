//! 乙太方界·親手煮的暖食，自己也能享用 v1（voxel-savor-own-meal）。
//!
//! **缺口 / 為誰做**：至今乙太方界的料理只服務**居民**——玩家把麵包／烤魚／烤地薯／野菜暖湯
//! 送出去，居民收下、道謝、細細享用（savor 765）；可玩家自己辛辛苦苦「採集→種田／垂釣→
//! 爐火烹製」出來的一道熱食，卻**從沒法自己嚐一口**。這正對著維護者 2026-06-30 的話：
//! 「記得操作／合成等系統也要，畢竟**我也想享受這世界**。」本刀補上療癒循環缺的最後一拍：
//! **玩家吃下自己親手煮的料理，得到一段純正向的「暖意」回饋**，把「採集→合成→餽贈」的循環
//! 補成「採集→合成→**自己享用**（或餽贈）」——人類第一次也能在這片方塊天地裡好好吃頓飯。
//!
//! **交織點（PLAN_ETHERVOX 玩家遊玩段·「人類的樂趣與 AI 的生活在同一片方塊天地交織」）**：
//! 若你剛好站在某位居民身邊享用，居民會**被你的滿足感染**——冒一句暖泡泡、記下「看著你享用
//! 親手煮的料理、我也跟著暖起來」這一筆社交記憶（深化交情）、動態牆留一行。你享用手藝的那份
//! 安穩，第一次也溫暖到了身邊的居民。這是實打實的行為後果（心情在乙太方界是驅動行為的真狀態、
//! 記憶累積成交情與日記），不是純美術。
//!
//! **這裡只放確定性純邏輯**（可享用料理判定、暖句挑選、附近居民分享的暖句／記憶／Feed），
//! 零 LLM、零鎖、零 IO、零 async，可單元測試。連線 / 鎖 / 心情 / Feed / 里程碑觸發全留在
//! `voxel_ws.rs`（沿用贈禮 660／享用 765 那條已驗證的短鎖循序慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：不觸發 LLM、不開對外端點、不收玩家自由文字——暖句／記憶／Feed 全為
//! 固定模板，只嵌玩家**顯示名**與料理繁中名（皆本就出現在贈禮道謝／動態牆），**永不回放記憶
//! 原文或玩家原話**（無注入 / NSFW 面）。「吃」本身要真的消耗一份熟食＝天然節流；「感染附近
//! 居民」這一拍另設每連線冷卻（[`SHARE_COOLDOWN_SECS`]）防囤糧狂吃洗版居民泡泡 / 動態牆。
//! 零 migration、零新美術；料理判定沿用既有食物 id 常數（單一真相，不重複一份清單）。

use crate::voxel_berry::JAM_ID;
use crate::voxel_craft::STEW_ID;
use crate::voxel_farm::{BAKED_POTATO_ID, BREAD_ID};
use crate::voxel_fishing::COOKED_FISH_ID;

/// 「暖意分享」觸及半徑（水平 XZ 平面，方塊距離）：居民要離你夠近，才「看得見」你在享用美食。
/// 與贈禮 `GIFT_REACH`(5.0) 同量級、稍寬半格，讓身邊自然路過的鄰居也可能被你的滿足感染。
pub const SHARE_RADIUS: f32 = 5.5;

/// 「暖意分享」的每連線冷卻（秒）：吃東西本身（消耗一份熟食＋自己的暖意回饋）**不受此限**，
/// 只有「感染附近居民」這一拍受節流——防止玩家囤一堆熟食狂吃、把居民泡泡 / 動態牆洗爆。
/// 45 秒＝比贈禮更寬鬆的社交事件、稀疏而有份量。
pub const SHARE_COOLDOWN_SECS: f32 = 45.0;

/// 玩家「親手煮的熟食」——原料（採集／種田／垂釣）經爐火或工作台**烹製**而成的成品，值得
/// 停下來自己細細享用。生食（生小魚 / 生馬鈴薯 / 胡蘿蔔 / 小麥顆粒）不在此列——那是原料、
/// 不是一道菜。**單一真相**：日後新增一道熟食料理時，只在這裡加一條（判定即隨之生效）。
///   - `BREAD_ID`=19（小麥→麵包）
///   - `COOKED_FISH_ID`=63（生魚→烤魚）
///   - `BAKED_POTATO_ID`=64（生馬鈴薯→烤地薯）
///   - `STEW_ID`=67（胡蘿蔔＋馬鈴薯＋小麥→野菜暖湯）
///   - `JAM_ID`=78（莓果→莓果醬，世界第一種甜點；ROADMAP 808）
pub fn is_edible_dish(item_id: u8) -> bool {
    matches!(
        item_id,
        BREAD_ID | COOKED_FISH_ID | BAKED_POTATO_ID | STEW_ID | JAM_ID
    )
}

/// 玩家「莓果醬是甜點、不是正餐」——享用它時的暖意回饋句與其他熱食**刻意不同**：
/// 不強調「熱騰騰」，而是甜味與小確幸（莓果醬 v1 ROADMAP 808，4 句輪替、≤40 字）。
/// 呼叫時機：`item_id == JAM_ID`（其餘熟食走 [`savor_self_line`]）。
pub fn savor_sweet_line(pick: usize) -> String {
    const LINES: &[&str] = &[
        "舀一口自己熬的莓果醬，酸甜在舌尖化開，嘴角忍不住揚起來。",
        "甜滋滋的莓果醬……熬了這麼久果然值得，這一刻小小的幸福好滿足。",
        "莓園的收成熬成了一罐莓果醬，慢慢品著，心也跟著甜了起來。",
        "抹一口莓果醬含在嘴裡，酸酸甜甜的，療癒得整個人都放鬆了。",
    ];
    LINES[pick % LINES.len()].to_string()
}

/// 玩家咬下一口自己煮的熱食時、畫面浮出的暖意回饋句（4 句輪替、≤40 字）。
/// `dish`＝料理繁中名（呼叫端用 `voxel_gift::item_name_zh` 取，不在此重複一份名表）。
pub fn savor_self_line(dish: &str, pick: usize) -> String {
    const LINES: &[&str] = &[
        "咬下一口熱騰騰的{dish}，暖意從指尖一路暖到心底……",
        "你親手煮的{dish}，一口下去，整個人都鬆了下來。",
        "{dish}的香氣還在鼻尖，慢慢享用著自己的手藝，好療癒。",
        "熱熱的{dish}下肚，這一刻的世界安安靜靜，真好。",
    ];
    LINES[pick % LINES.len()].replace("{dish}", dish)
}

/// 附近居民看見你享用親手煮的料理時、被你的滿足感染而冒出的暖泡泡（3 句輪替、≤40 字）。
/// 只嵌料理名（居民名由泡泡本身呈現，不重複嵌進句子）。
pub fn share_line(dish: &str, pick: usize) -> String {
    const LINES: &[&str] = &[
        "看你吃{dish}吃得那麼香，我也覺得暖暖的～",
        "你煮的{dish}聞起來好香，看你享用我也開心。",
        "自己煮的{dish}最香了對吧？看你這樣我也跟著暖起來。",
    ];
    LINES[pick % LINES.len()].replace("{dish}", dish)
}

/// 居民記進心裡的一筆社交記憶：看著你享用親手煮的料理（累積交情、進日記）。
/// 空玩家名安全退成「旅人」。
pub fn share_memory_line(player: &str, dish: &str) -> String {
    let who = if player.is_empty() { "旅人" } else { player };
    format!("看著{who}享用親手煮的{dish}，我也跟著暖起來、心情好了些")
}

/// 城鎮動態牆的一行旁白（第三人稱）。空玩家名安全退成「旅人」。
pub fn share_feed_line(resident: &str, player: &str, dish: &str) -> String {
    let who = if player.is_empty() { "旅人" } else { player };
    format!("{resident}看著{who}享用親手煮的{dish}，也跟著暖了起來")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cooked_dishes_are_edible() {
        // 四道熟食皆可享用。
        assert!(is_edible_dish(BREAD_ID), "麵包可享用");
        assert!(is_edible_dish(COOKED_FISH_ID), "烤魚可享用");
        assert!(is_edible_dish(BAKED_POTATO_ID), "烤地薯可享用");
        assert!(is_edible_dish(STEW_ID), "野菜暖湯可享用");
        assert!(is_edible_dish(JAM_ID), "莓果醬（甜點）可享用");
        // 生食 / 原料 / 非食物一律不可「享用手藝」。
        assert!(!is_edible_dish(77), "莓果（生）是原料、不是一道菜");
        assert!(!is_edible_dish(49), "胡蘿蔔（生）不是一道煮好的菜");
        assert!(!is_edible_dish(53), "生馬鈴薯不是一道煮好的菜");
        assert!(!is_edible_dish(61), "生小魚不是一道煮好的菜");
        assert!(!is_edible_dish(18), "小麥顆粒是原料、不是菜");
        assert!(!is_edible_dish(5), "木頭不是食物");
        assert!(!is_edible_dish(0), "空氣不是食物");
    }

    #[test]
    fn savor_sweet_line_non_empty_bounded_and_rotates() {
        // 甜點享用句：非空、≤40 字、提到「莓果醬」的甜味療癒感，且會輪替。
        for pick in 0..8 {
            let s = savor_sweet_line(pick);
            assert!(!s.is_empty(), "pick={pick} 甜點暖句不得為空");
            assert!(s.contains("莓果醬"), "pick={pick} 甜點暖句該提到莓果醬：{s}");
            assert!(s.chars().count() <= 40, "pick={pick} 甜點暖句 ≤40 字不破框：{s}");
        }
        assert_ne!(savor_sweet_line(0), savor_sweet_line(1), "相鄰 pick 應輪到不同句");
    }

    #[test]
    fn savor_self_line_non_empty_no_placeholder_and_rotates() {
        // 每個 pick 值：非空、不含未替換的 {dish}、含料理名、≤40 字。
        for pick in 0..8 {
            let s = savor_self_line("野菜暖湯", pick);
            assert!(!s.is_empty(), "pick={pick} 暖句不得為空");
            assert!(!s.contains("{dish}"), "pick={pick} 佔位符要被替換：{s}");
            assert!(s.contains("野菜暖湯"), "pick={pick} 暖句該提到料理名：{s}");
            assert!(s.chars().count() <= 40, "pick={pick} 暖句 ≤40 字不破框：{s}");
        }
        // 輪替：相鄰 pick 至少有一對不同句。
        let a = savor_self_line("麵包", 0);
        let b = savor_self_line("麵包", 1);
        assert_ne!(a, b, "相鄰 pick 應輪到不同句");
    }

    #[test]
    fn share_line_non_empty_no_placeholder_and_bounded() {
        for pick in 0..6 {
            let s = share_line("烤魚", pick);
            assert!(!s.is_empty(), "pick={pick} 分享句不得為空");
            assert!(!s.contains("{dish}"), "pick={pick} 佔位符要被替換：{s}");
            assert!(s.contains("烤魚"), "pick={pick} 分享句該提到料理名：{s}");
            assert!(s.chars().count() <= 40, "pick={pick} 分享句 ≤40 字不破框：{s}");
        }
    }

    #[test]
    fn share_memory_and_feed_contain_names_and_dish() {
        let mem = share_memory_line("小星", "野菜暖湯");
        assert!(mem.contains("小星") && mem.contains("野菜暖湯"), "記憶含玩家名＋料理名：{mem}");
        let feed = share_feed_line("露娜", "小星", "烤地薯");
        assert!(
            feed.contains("露娜") && feed.contains("小星") && feed.contains("烤地薯"),
            "Feed 含居民名＋玩家名＋料理名：{feed}"
        );
    }

    #[test]
    fn empty_player_name_falls_back_safely() {
        // 空玩家名不露破碎字串、不 panic，退成「旅人」。
        let mem = share_memory_line("", "麵包");
        assert!(mem.contains("旅人") && !mem.starts_with("看著，"), "空名退旅人：{mem}");
        let feed = share_feed_line("露娜", "", "麵包");
        assert!(feed.contains("旅人"), "Feed 空名退旅人：{feed}");
    }

    #[test]
    fn constants_are_sane() {
        // 觸及半徑與贈禮同量級（正、有限、不過大）；分享冷卻明顯長於一次互動、稀疏有份量。
        assert!(SHARE_RADIUS > 0.0 && SHARE_RADIUS <= 8.0, "分享半徑落在合理範圍");
        assert!(SHARE_COOLDOWN_SECS >= 30.0, "分享冷卻夠長、防洗版");
    }
}
