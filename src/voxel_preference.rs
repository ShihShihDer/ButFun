//! 乙太方界·居民投你所好 v1（ROADMAP 730）——記憶第一次「讀你的言外之意」驅動回禮。
//!
//! 核心信念（PLAN_ETHERVOX 北極星）：**記憶要驅動行為，不只聊天。**
//! 居民的長期精華記憶早就存下你隨口說過的偏好（`FactCategory::Preference`，如
//! 「我最喜歡看星星」「我愛花」），但這份「她記得你喜歡什麼」至今只被餵進對話 prompt——
//! 換來幾句更貼心的閒聊，從沒真的改變過她**做的事**。
//!
//! 本模組讓偏好第一次有後果：居民回禮（667/728）時，若記得你曾說過的喜好，
//! 就**挑一份呼應那份喜好的禮物**送你，並說出「我記得你喜歡…」——
//! 這比「她親手採到的東西」（728）更進一步：不是給她手邊有的，而是**特地投你所好**。
//! 這是「你隨口一句、她真的放在心上」的魔法，也是心願系統（720/722 回應明講的「我想要X」）
//! 之外的另一條路：偏好是**言外之意**（「我喜歡花」→她推斷你會想要種子），她讀懂了。
//!
//! 設計鐵律：
//! - **純邏輯層**：零 IO、零鎖、零 LLM、零 async；確定性純函式、可窮舉測試。
//! - 鎖與副作用全在 `voxel_ws.rs`（短鎖即釋、不巢狀、守 prod 死鎖鐵律）。
//! - 面向玩家字串集中此處（i18n 友善）；繁中註解；不碰玩家資料表。

/// 一條「偏好 → 禮物」對照規則：關鍵詞命中任一即採用。
/// 欄位：(關鍵詞集合, 禮物 block_id, 數量, 面向玩家的主題名「你喜歡的東西」)。
/// **優先序＝陣列順序**：由上往下第一個命中的規則勝出（確定性、可測）。
type PrefRule = (&'static [&'static str], u8, u32, &'static str);

/// 偏好 → 禮物對照表。block_id 皆為玩家背包可持有、且 `voxel_gift::item_name_zh`
/// 有對應名稱的方塊：14=種子、31=火把、19=麵包、5=木頭、3=石頭。
/// 送的都是「她生活裡拿得出、又呼應你喜好」的小東西——不送物品／乙太／戰力，守成本紀律。
const PREFERENCE_RULES: &[PrefRule] = &[
    // 花草／庭園／自然美 → 種子（讓你自己種出喜歡的花草）。
    (&["花", "園", "植", "綠", "草"], 14, 3, "花草"),
    // 夜／星／月／燈光 → 火把（點亮你愛的夜色）。
    (&["星", "夜", "月", "燈", "光", "亮"], 31, 2, "夜裡的光"),
    // 吃／食／溫飽 → 麵包。
    (&["麵包", "吃", "食", "餓", "糧"], 19, 2, "溫飽"),
    // 家／溫暖／木造 → 木頭（蓋一個溫暖的窩）。
    (&["家", "暖", "住", "窩", "木"], 5, 3, "溫暖的家"),
    // 堅固／石造／城牆 → 石頭。
    (&["石", "堅", "牆", "固", "城", "堡"], 3, 3, "堅固的家"),
    // 田／農／耕作 → 種子（回到田園）。
    (&["田", "農", "種", "麥"], 14, 3, "田園生活"),
];

/// 掃描居民對這位玩家的所有「偏好」記憶內容，挑出第一份能呼應的禮物。
///
/// - `preference_contents`：該居民對該玩家的 `FactCategory::Preference` 事實內容列表
///   （由呼叫端從 `VoxelMemory::semantic_facts_for` 過濾出來後傳入）。
/// - 回傳 `Some((block_id, qty, 主題名))`：找到可投其所好的禮物；`None`：沒有可對應的偏好，
///   呼叫端回退到 728（她親手採到的）→ 667（憑空的木頭/種子）。
///
/// 掃描規則（確定性、可測）：對每條偏好內容，依 [`PREFERENCE_RULES`] 由上往下試，
/// 命中關鍵詞即採用該規則；先掃完一條內容再換下一條，先命中者勝（穩定、可預期）。
pub fn gift_for_preference(preference_contents: &[String]) -> Option<(u8, u32, &'static str)> {
    for content in preference_contents {
        for (keywords, block_id, qty, theme) in PREFERENCE_RULES {
            if keywords.iter().any(|kw| content.contains(kw)) {
                return Some((*block_id, *qty, *theme));
            }
        }
    }
    None
}

/// 「投你所好」回禮台詞：點名玩家 + 說出「我記得你喜歡…」+ 送的東西。
/// 依居民名字確定性選一句（同一居民風格穩定，有個性），截斷 40 字防溢位。
pub fn preference_gift_message(
    resident_name: &str,
    player_name: &str,
    theme: &str,
    item_name: &str,
) -> String {
    let idx = resident_name.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize));
    let pool: &[&str] = &[
        "{p}，我記得你喜歡{t}，這{i}送你。",
        "{p}，你說過喜歡{t}對吧？這{i}給你。",
        "{p}，我一直記著你喜歡{t}，收下這{i}吧！",
        "{p}，想著你喜歡{t}，特地留了這{i}給你。",
    ];
    pool[idx % pool.len()]
        .replace("{p}", player_name)
        .replace("{t}", theme)
        .replace("{i}", item_name)
        .chars()
        .take(40)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_preferences_yield_none() {
        assert_eq!(gift_for_preference(&[]), None);
    }

    #[test]
    fn unrelated_preference_yields_none() {
        // 沒有任何規則關鍵詞 → None，回退到既有回禮。
        let prefs = vec!["喜歡安靜地散步".to_string()];
        assert_eq!(gift_for_preference(&prefs), None);
    }

    #[test]
    fn flower_preference_gives_seeds() {
        let prefs = vec!["最喜歡花了".to_string()];
        let (bid, qty, theme) = gift_for_preference(&prefs).expect("花 → 種子");
        assert_eq!(bid, 14);
        assert!(qty >= 1);
        assert_eq!(theme, "花草");
    }

    #[test]
    fn star_preference_gives_torch() {
        let prefs = vec!["我最喜歡在夜裡看星星".to_string()];
        let (bid, _qty, theme) = gift_for_preference(&prefs).expect("星夜 → 火把");
        assert_eq!(bid, 31);
        assert_eq!(theme, "夜裡的光");
    }

    #[test]
    fn food_preference_gives_bread() {
        let prefs = vec!["就愛吃的東西".to_string()];
        let (bid, _qty, _theme) = gift_for_preference(&prefs).expect("吃 → 麵包");
        assert_eq!(bid, 19);
    }

    #[test]
    fn home_preference_gives_wood() {
        let prefs = vec!["喜歡溫暖的家".to_string()];
        let (bid, _qty, theme) = gift_for_preference(&prefs).expect("家 → 木頭");
        assert_eq!(bid, 5);
        assert_eq!(theme, "溫暖的家");
    }

    #[test]
    fn earlier_rule_wins_within_one_content() {
        // 同一句同時含「花」(規則1) 與「石」(規則5) → 由上往下先命中花草。
        let prefs = vec!["喜歡在石桌上擺花".to_string()];
        let (bid, _qty, theme) = gift_for_preference(&prefs).unwrap();
        assert_eq!(bid, 14);
        assert_eq!(theme, "花草");
    }

    #[test]
    fn earlier_content_wins_across_multiple() {
        // 兩條偏好，先掃第一條：第一條命中即回，不看第二條。
        let prefs = vec!["愛吃麵包".to_string(), "也喜歡花".to_string()];
        let (bid, _qty, _theme) = gift_for_preference(&prefs).unwrap();
        assert_eq!(bid, 19); // 麵包（第一條）勝
    }

    #[test]
    fn message_mentions_player_theme_and_item() {
        let msg = preference_gift_message("露娜", "旅人", "花草", "種子");
        assert!(msg.contains("旅人"));
        assert!(msg.contains("花草"));
        assert!(msg.contains("種子"));
    }

    #[test]
    fn message_is_bounded_to_40_chars() {
        let msg = preference_gift_message(
            "很長很長的居民名字",
            "很長很長很長的玩家名字",
            "非常非常非常長的主題名稱",
            "非常非常非常長的物品名稱",
        );
        assert!(msg.chars().count() <= 40);
    }

    #[test]
    fn message_is_deterministic_by_resident() {
        let a = preference_gift_message("露娜", "旅人", "花草", "種子");
        let b = preference_gift_message("露娜", "旅人", "花草", "種子");
        assert_eq!(a, b);
    }
}
