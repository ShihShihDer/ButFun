//! 乙太方界·為馴服的動物取名 v1（voxel-pet-name，自主提案切片，ROADMAP 895）。
//!
//! **這一刀補的缺口**：野兔（847 起）與雞（870）如今能餵食馴服（`tamed`）、會跟著你走
//! （851 跟隨、855 生寶寶、870 生蛋）——你已經和牠們之間有了羈絆，但那份羈絆一直**沒有名字**。
//! 馴服兔子跟隨你 v1（851）的說明裡自己講明「無寵物 UI」；至今一隻你親手馴服、每天跟前跟後的
//! 小夥伴，和路邊任何一隻野生動物在畫面上**看不出分別**——牠是「你的」這件事沒有被世界記住。
//! 本刀補上情感羈絆線缺的那一拍：**點一下你已馴服的小夥伴，替牠取個名字**，從此牠頭頂浮起一塊
//! 專屬名牌，跟著你走過草地時一眼就認得出「這是我的某某」。馴服→跟隨→繁殖→生蛋這條線，第一次
//! 有了「牠屬於你、你們有故事」的收尾。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **馴服（847/870）**＝一次性把「怕人」變「不怕人」，是**關係的起點**；本刀＝在已建立的
//!   羈絆之上**替牠命名**，是關係的**署名**，兩者作用點與觸發完全不同。
//! - **居民名牌（既有）**＝伺服器權威的居民身分；本刀的寵物名由**玩家自己**取，是玩家在世界裡
//!   留下的私人印記（比照 749 個人渴望署名的精神，但對象是動物、載體是名牌）。
//!
//! **設計調性仍是療癒**：命名純粹是溫柔的羈絆，不掛任何數值/戰力/加成——就只是「牠有名字了」。
//!
//! **濫用防護（鐵律）**：寵物名是**玩家自由輸入**，一律先過 [`sanitize_pet_name`]——去除控制字元
//! /換行、前後去空白、拒空、字數硬上限 [`MAX_NAME_CHARS`]——才可能被接受。名字**只**出現在
//! 該寵物的名牌廣播（回給所有在場玩家渲染），**絕不**被寫進任何居民記憶/日記/口耳相傳，
//! 杜絕越獄/注入/洗版別人畫面的面。伺服器另權威複驗：這隻動物存在、已馴服、在觸及範圍
//! （[`NAME_REACH`]）內——客戶端不自報合法性。
//!
//! **純邏輯層**：本檔只放確定性純函式（清洗、選句），零 IO、零鎖、零 LLM、零 async，
//! 可窮舉單元測試。鎖與副作用全在 `voxel_ws.rs`（沿用餵食馴服 847／守夜恩人 888 那條已驗證的
//! 短鎖循序慣例，守 prod 死鎖鐵律）。不抄外部碼；繁中註解；面向玩家字串集中於此，i18n 友善。

/// 命名的觸及範圍（方塊，XZ 平面）：你要站在小夥伴身邊才取得了名。
/// 比餵食馴服（[`crate::voxel_wildlife::TAME_REACH`]=3.0）稍寬一點點——馴服要追上受驚的牠，
/// 命名時牠已經不怕你、乖乖在腳邊，站近一點就好。
pub const NAME_REACH: f32 = 4.5;

/// 寵物名字數硬上限（以字元計，含中英數與 emoji 各算一枚）。
/// 短——名牌只有巴掌大，太長塞不下也失了暱稱的親暱感。
pub const MAX_NAME_CHARS: usize = 12;

/// 一位近旁醒著的居民「注意到你為小夥伴命名」的見證範圍（方塊，XZ 平面）。
/// 比命名範圍寬——遠遠望見這溫柔的一幕也會心生暖意（比照守夜恩人的救援半徑量級）。
pub const WITNESS_RANGE: f32 = 15.0;
/// 見證範圍平方（呼叫端用距離平方比較，免開根號）。
pub const WITNESS_RANGE_SQ: f32 = WITNESS_RANGE * WITNESS_RANGE;

/// 清洗玩家輸入的寵物名（濫用防護的第一道也是唯一一道關）：
/// 1. 濾掉所有控制字元（含 `\n`/`\r`/`\t`）——防換行破壞名牌/JSON、防隱藏字元。
/// 2. 前後去空白。
/// 3. 拒空（清洗後為空 → `None`，呼叫端回「名字不能空白」）。
/// 4. 字數硬上限 [`MAX_NAME_CHARS`]（以 `char` 計，超過即截斷）。
///
/// 回傳 `Some(乾淨名字)` 或 `None`（無效輸入）。確定性、無副作用、可窮舉測試。
pub fn sanitize_pet_name(raw: &str) -> Option<String> {
    // ① 濾控制字元（保留一般可見字元與空白，空白稍後 trim）。
    let filtered: String = raw.chars().filter(|c| !c.is_control()).collect();
    // ② 前後去空白。
    let trimmed = filtered.trim();
    // ③ 拒空。
    if trimmed.is_empty() {
        return None;
    }
    // ④ 字數硬上限（以字元計，超過截斷）。
    let clean: String = trimmed.chars().take(MAX_NAME_CHARS).collect();
    Some(clean)
}

/// 命名成功回給玩家的確認句（單播給命名者本人）。`name` 已過清洗。
pub fn name_ack_line(name: &str) -> String {
    format!("🐾 從今天起，牠就叫「{name}」了。")
}

/// 一位近旁居民見證你為小夥伴命名時，浮出的一句暖泡泡（固定模板、確定性選句）。
/// **刻意不嵌任何玩家輸入**（不含寵物名、不含玩家名）——零注入面、可被口耳相傳也絕對安全。
pub fn witness_say(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "看你替牠取了名字，真好。",
        "牠有名字了呀，真替你們高興。",
        "有了名字，就是一家人了呢。",
        "這小傢伙遇到你，真是福氣。",
    ];
    LINES[pick % LINES.len()]
}

/// 居民把「見證你為小夥伴命名」記進心裡的一句（第一人稱、固定模板、確定性）。
/// **刻意不嵌任何玩家輸入**——只以「有位旅人」「小動物」泛稱，杜絕注入、也符合日記不洩漏私訊的隱私鐵律。
pub fn witness_memory_line() -> String {
    "有位旅人替一隻跟著他的小動物取了名字，看他們在一起的模樣，我心裡也暖暖的。".to_string()
}

/// 見證這一幕寫進動態牆的一句（世界之聲；`resident` 是見證的居民名）。
pub fn witness_feed_line(resident: &str) -> String {
    format!("{resident} 看著一位旅人替跟著他的小動物取了名字，露出了笑容。")
}

/// 動態牆事件種類名稱。
pub const FEED_KIND: &str = "命名小夥伴";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_empty_and_whitespace() {
        assert_eq!(sanitize_pet_name(""), None);
        assert_eq!(sanitize_pet_name("   "), None);
        assert_eq!(sanitize_pet_name("\t\n  \r"), None);
    }

    #[test]
    fn sanitize_trims_and_keeps_normal_name() {
        assert_eq!(sanitize_pet_name("  小白  ").as_deref(), Some("小白"));
        assert_eq!(sanitize_pet_name("Bun").as_deref(), Some("Bun"));
        assert_eq!(sanitize_pet_name("胡蘿蔔君").as_deref(), Some("胡蘿蔔君"));
    }

    #[test]
    fn sanitize_strips_control_chars() {
        // 換行/回車/Tab/其他控制字元一律濾掉，剩餘拼回。
        assert_eq!(sanitize_pet_name("小\n白").as_deref(), Some("小白"));
        assert_eq!(sanitize_pet_name("a\u{0007}b\tc").as_deref(), Some("abc"));
    }

    #[test]
    fn sanitize_caps_length_by_chars() {
        // 13 個字 → 截成 12 個（以字元計，非位元組）。
        let long = "一二三四五六七八九十百千萬"; // 13 個中文字
        let out = sanitize_pet_name(long).unwrap();
        assert_eq!(out.chars().count(), MAX_NAME_CHARS);
        assert_eq!(out, "一二三四五六七八九十百千");
    }

    #[test]
    fn sanitize_length_boundary_exact() {
        // 剛好 12 字不截。
        let exact = "一二三四五六七八九十百千"; // 12 字
        assert_eq!(sanitize_pet_name(exact).unwrap().chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn name_reach_wider_than_tame_reach() {
        // 命名時牠已乖乖在腳邊，範圍比追著餵食時稍寬。
        assert!(NAME_REACH > crate::voxel_wildlife::TAME_REACH);
    }

    #[test]
    fn witness_lines_nonempty_and_cycle() {
        for i in 0..8 {
            assert!(!witness_say(i).is_empty());
        }
        // 索引越界安全（取模）。
        assert_eq!(witness_say(0), witness_say(4));
    }

    #[test]
    fn ack_and_memory_lines_safe() {
        // 確認句含名字；記憶/動態牆句刻意不含任何玩家輸入。
        assert!(name_ack_line("小白").contains("小白"));
        assert!(!witness_memory_line().is_empty());
        assert!(witness_feed_line("露娜").contains("露娜"));
    }
}
