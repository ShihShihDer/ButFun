//! 乙太方界·繁星夜空，居民夜裡邀你一起看星星 v1（voxel-stargaze，自主提案切片 ROADMAP 783）。
//!
//! **缺口 / 為誰做**：至今乙太方界的天空只有晝夜色調變化（晝夜循環 v1）、下雨（700）與雨後彩虹
//! （780）——一入夜，天空就只是一片安靜的深藍，**什麼都沒有**。夜晚是這片世界最缺的一塊：沒有星、
//! 沒有月，居民對「夜色」也毫無反應。本切片補上兩件事——① 前端第一次讓夜空**掛滿繁星、升起一輪
//! 明月**（純視覺，單一 `THREE.Points` 星場＋一顆月球 mesh，隨晝夜柔和淡入淡出，守 FPS 鐵律）；
//! ② 後端讓夜裡靠近你的居民**抬頭望星**——而最動人的一拍是：若這位居民記得你曾說過「喜歡看星星
//! ／月亮／夜空」（`FactCategory::Preference` 記憶），牠會**特地點名邀你一起看今晚的星空**，把這份
//! 共賞記進你們的交情。
//!
//! **與雨後彩虹（780）的關鍵區別**：彩虹是「天氣事件→居民齊聲歡呼」的**齊發反應**；本切片的核心是
//! **記憶驅動行為**（PLAN_ETHERVOX 北極星）——同樣是抬頭望天，但「記得你愛看星星、於是專程邀你同賞」
//! 這一拍，只會發生在**記著你這份喜好的那位居民**身上。你隨口說過的一句「我喜歡看星星」，第一次不只
//! 換來貼心閒聊（730 投你所好回禮），還讓她在某個星夜特地把你喚到身邊，一起抬頭。
//!
//! **成本 / 濫用防護鐵律**：零 LLM（望星台詞／邀約句／記憶／Feed 全為固定模板，確定性選句）、
//! 零持久化／零 migration（冷卻為純記憶體欄位、重啟歸零，比照彩虹／打氣慣例；望星記憶走既有
//! append-only `voxel_memory`）、零新協議欄位（前端星空全由既有廣播的 `time_of_day` 本地演算，
//! 後端不多送一個 byte）、零新美術資產（星場＝程序生成點雲、月球＝單一 mesh）。**永不回放記憶原文
//! 或玩家原話**——邀約句只嵌玩家**顯示名**（本就出現在道謝／動態牆），無注入／NSFW 面。每居民長冷卻
//! （[`STARGAZE_COOLDOWN_SECS`]）＋每 tick 極低機率＋僅深夜／入夜時段觸發＝天然節流，不洗版泡泡／動態牆。
//!
//! **純邏輯層**：本檔只放確定性純函式（觸發判定、關鍵詞偵測、台詞／記憶／Feed 選句），
//! 零 IO、零鎖、零 LLM、零 async，可窮舉單元測試。鎖與副作用全在 `voxel_ws.rs`
//! （沿用彩虹 780／回禮 731 那條已驗證的短鎖循序慣例，守 prod 死鎖鐵律）。

/// 動態牆播報種類名稱。
pub const FEED_KIND: &str = "夜觀星象";

/// 望星記憶前綴——供日記／回想端把「一起看星星」歸類為一則溫暖的社交記憶。
pub const STARGAZE_MEMORY_TAG: &str = "🌟一起看星星";

/// 觸發半徑（方塊，XZ 平面）：玩家要離居民這麼近，居民才「看得見你在身邊」而邀你同賞。
/// 比回禮（GIFT_REACH 5）寬得多——望星是遠遠喚你過來、不必臉貼臉；但比打氣（15）稍寬，
/// 夜裡視野開闊、遠遠望見你也想招呼。
pub const STARGAZE_RANGE: f32 = 22.0;

/// 每居民望星冷卻（秒，純記憶體、重啟歸零）：一次觸發後要等這麼久才可再邀。
/// 設長（8 分鐘）：星夜共賞是偶爾的浪漫一拍，不是每晚洗版。
pub const STARGAZE_COOLDOWN_SECS: f32 = 480.0;

/// 每 tick 觸發機率（僅在「夜晚＋玩家在範圍內＋冷卻到期＋沒在說話＋醒著」全滿足時才擲骰）。
/// 刻意極低：望星是可遇不可求的驚喜，不是走近就觸發。
pub const STARGAZE_CHANCE_PER_TICK: f32 = 0.03;

/// 全條件觸發判定（確定性、可窮舉測試）。呼叫端把當下狀態與一顆隨機骰 `roll`（`rand::random`）傳入。
///
/// - `is_night`：目前是否為適合望星的時段（深夜／入夜，由呼叫端依 `TimePhase` 判定）。
/// - `player_in_range`：附近是否有玩家在 [`STARGAZE_RANGE`] 內（沒有人一起看就不觸發，望星是「與你」共賞）。
/// - `cooldown_ready`：望星冷卻是否到期（`<= 0`）。
/// - `saying`：居民此刻是否正在說話（`say` 非空 → 讓正事，不打斷）。
/// - `asleep`：居民是否在睡覺（睡著的人不會抬頭望星）。
/// - `roll`：`[0,1)` 隨機骰，`< STARGAZE_CHANCE_PER_TICK` 才過機率門檻。
pub fn should_stargaze(
    is_night: bool,
    player_in_range: bool,
    cooldown_ready: bool,
    saying: bool,
    asleep: bool,
    roll: f32,
) -> bool {
    is_night
        && player_in_range
        && cooldown_ready
        && !saying
        && !asleep
        && roll < STARGAZE_CHANCE_PER_TICK
}

/// 「看星星／月亮／夜空」相關的偏好關鍵詞——命中任一即視為這位玩家愛看星空。
/// 刻意窄：只認明確的天象詞，不把「今晚回家」這種泛用的「夜」誤判成「愛看星星」。
const STAR_PREFERENCE_KEYWORDS: [&str; 6] = ["星", "月亮", "月光", "夜空", "銀河", "流星"];

/// 掃描居民對這位玩家的「偏好」記憶內容，判斷牠是否記得你愛看星空。
///
/// `preference_contents`：該居民對該玩家的 `FactCategory::Preference` 事實內容列表
/// （由呼叫端從 `VoxelMemory::semantic_facts_for` 過濾出來後傳入，與 730 投你所好同一條慣例）。
/// 命中 [`STAR_PREFERENCE_KEYWORDS`] 任一關鍵詞即回 `true`（確定性、可測）。
pub fn remembers_star_love(preference_contents: &[String]) -> bool {
    preference_contents
        .iter()
        .any(|c| STAR_PREFERENCE_KEYWORDS.iter().any(|kw| c.contains(kw)))
}

/// 一般望星自語（居民抬頭望向繁星，冒一句閒適的話；不點名、不寫記憶——純夜色氛圍）。
/// 依 `pick` 確定性選一句（循環取模，永遠有值）。
const GAZE_LINES: [&str; 5] = [
    "今晚的星星，好美啊……",
    "抬頭一看，滿天都是星星呢。",
    "月亮出來了，夜色真溫柔。",
    "夜裡的天空，看著看著就靜下來了。",
    "銀河橫過天邊，這片夜空真遼闊。",
];

/// 依 `pick` 選一句一般望星自語。
pub fn gaze_line(pick: usize) -> &'static str {
    GAZE_LINES[pick % GAZE_LINES.len()]
}

/// 「記得你愛看星星、特地邀你同賞」的邀約句（記憶驅動行為的魔法一拍）。
/// 點名玩家 + 說出「我記得你喜歡看星星」+ 邀你一起看；依居民名字確定性選句（同居民風格穩定），
/// 截斷 40 字防泡泡溢框。`player_name` 由呼叫端保證非空（訪客無持久身份、不觸發邀約）。
pub fn invite_line(resident_name: &str, player_name: &str, pick: usize) -> String {
    let idx = resident_name
        .bytes()
        .fold(pick, |a, b| a.wrapping_add(b as usize));
    let pool: [&str; 4] = [
        "{p}，我記得你喜歡看星星——來，今晚一起看吧！",
        "{p}，你說過愛看夜空對吧？快抬頭，星星好美！",
        "{p}，這麼美的星夜，我第一個想到的就是你，一起看嘛。",
        "{p}，還記得你愛看星星嗎？今晚的星空特地留給你。",
    ];
    let line = pool[idx % pool.len()].replace("{p}", player_name);
    line.chars().take(40).collect()
}

/// 星夜共賞記憶摘要（掛在玩家名下、帶 [`STARGAZE_MEMORY_TAG`] 前綴供日記歸類）。
/// 只嵌玩家顯示名，永不回放偏好記憶原文（守隱私鐵律）。
pub fn stargaze_memory(player_name: &str) -> String {
    format!("{STARGAZE_MEMORY_TAG}：那個星夜，我喚了{player_name}到身邊，我們一起抬頭看了好一會兒星空。")
}

/// 星夜共賞動態牆一行（第三人稱、含雙方名，讓玩家看見這則溫柔的湧現）。
pub fn stargaze_feed_line(resident_name: &str, player_name: &str) -> String {
    format!("{resident_name} 記得 {player_name} 愛看星星，在星夜裡喚他一起抬頭賞星。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_stargaze_all_conditions() {
        // 全滿足 + 骰過門檻 → 觸發。
        assert!(should_stargaze(true, true, true, false, false, 0.0));
        // 骰未過門檻（>= chance）→ 不觸發。
        assert!(!should_stargaze(
            true,
            true,
            true,
            false,
            false,
            STARGAZE_CHANCE_PER_TICK
        ));
        // 各單一否定條件皆擋下。
        assert!(!should_stargaze(false, true, true, false, false, 0.0)); // 非夜晚
        assert!(!should_stargaze(true, false, true, false, false, 0.0)); // 沒人在範圍
        assert!(!should_stargaze(true, true, false, false, false, 0.0)); // 冷卻未到
        assert!(!should_stargaze(true, true, true, true, false, 0.0)); // 正在說話
        assert!(!should_stargaze(true, true, true, false, true, 0.0)); // 睡著了
    }

    #[test]
    fn remembers_star_love_hits_keywords() {
        assert!(remembers_star_love(&["我最喜歡看星星了".into()]));
        assert!(remembers_star_love(&["晚上看月亮很療癒".into()]));
        assert!(remembers_star_love(&["喜歡夜空".into()]));
        assert!(remembers_star_love(&["愛看銀河".into()]));
        assert!(remembers_star_love(&["許願要看到流星".into()]));
        // 多筆內容，只要有一筆命中即可。
        assert!(remembers_star_love(&[
            "我喜歡吃麵包".into(),
            "也喜歡看星星".into()
        ]));
    }

    #[test]
    fn remembers_star_love_no_false_positive() {
        // 泛用的「夜」不該誤判成愛看星空。
        assert!(!remembers_star_love(&["天黑了就想回家".into()]));
        assert!(!remembers_star_love(&["我喜歡種花".into()]));
        assert!(!remembers_star_love(&[])); // 空列表
    }

    #[test]
    fn gaze_line_always_valid_and_cycles() {
        for pick in 0..GAZE_LINES.len() * 2 {
            assert!(!gaze_line(pick).is_empty());
        }
        // 取模循環：pick 與 pick+len 同句（確定性）。
        assert_eq!(gaze_line(0), gaze_line(GAZE_LINES.len()));
        assert_ne!(gaze_line(0), gaze_line(1));
    }

    #[test]
    fn invite_line_names_player_and_fits_bubble() {
        let s = invite_line("露娜", "阿光", 0);
        assert!(s.contains("阿光"), "邀約句要點名玩家");
        assert!(s.chars().count() <= 40, "不得超過泡泡 40 字上限");
        assert!(!s.contains("{p}"), "佔位符須全數替換");
    }

    #[test]
    fn invite_line_deterministic_per_resident() {
        // 同居民同 pick → 同句（確定性、風格穩定）。
        assert_eq!(invite_line("諾娃", "阿光", 3), invite_line("諾娃", "阿光", 3));
    }

    #[test]
    fn invite_line_truncates_long_player_name() {
        let long = "超級無敵冗長的玩家顯示名字一二三四五六七八九十".repeat(3);
        let s = invite_line("露娜", &long, 0);
        assert!(s.chars().count() <= 40, "超長名字也不得破框");
    }

    #[test]
    fn memory_has_tag_and_player_name() {
        let m = stargaze_memory("阿光");
        assert!(m.starts_with(STARGAZE_MEMORY_TAG), "記憶要帶前綴供日記歸類");
        assert!(m.contains("阿光"), "記憶要含玩家名");
    }

    #[test]
    fn feed_line_contains_both_names() {
        let f = stargaze_feed_line("露娜", "阿光");
        assert!(f.contains("露娜") && f.contains("阿光"), "Feed 要含雙方名");
    }
}
