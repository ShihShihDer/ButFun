//! 乙太方界·玩家追到邊陲，巧遇正在遠行的居民 v1（PLAN_ETHERVOX item 7「居民散佈世界各處住」
//! × item 3「你的互動有後果」的第二次交會——821 讓留守居民追去邊陲找老朋友，本刀把同一種
//! 「追到荒野盡頭找到你」的驚喜，第一次伸向**玩家**）。
//!
//! **真缺口**：散居（756~762）讓奧瑞（漂泊）與諾娃（尋地）偶爾遠行到邊陲住上一陣子；821 讓
//! 留守主城的居民（露娜/賽勒）交情夠深時會追去邊陲找她們——但玩家從沒有這條路：即使你真的
//! 一路跑到世界邊陲、恰好撞見正在遠行的居民，牠此刻的反應跟在主城裡遇到牠**一模一樣**，完全
//! 感受不到「我特地跑這麼遠找到你」的驚喜。散居把居民的足跡撒進了荒野，卻沒有人為玩家的
//! 「追過去」寫一句台詞。
//!
//! 本刀補上：當一位居民正在邊陲逗留（`expedition` 有值且已抵達、非睡眠中）時，若你恰好走到
//! 牠身邊，牠會**認出「你是特地追到這麼遠來的」**，比在主城相遇更驚喜的一句招呼，並把「你千里
//! 迢迢追到邊陲找我」記進交情、日後浮進日記；世界動態牆也會記上一筆，讓沒跟去的其他玩家讀到
//! 「有人在邊陲巧遇了正在遠行的誰」。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **821 邊陲跋涉探友**＝居民↔居民（另一位**居民**主動追過去）；本刀＝居民↔**玩家**（把同一種
//!   「追到邊陲找到你」的機制，第一次讓玩家也能觸發），角色換成人類，台詞與記憶對象皆不同。
//! - **830 玩家家訪**＝居民在**主城**認出你立的家牌、朝聖抵達；本刀＝居民在**遠離主城的邊陲**
//!   被你找到，地點與觸發方向相反（居民走向你的家 vs 你走向居民的邊陲），且不需要任何告示牌。
//! - **一般 Talk 攀談**＝玩家主動按鍵開口才觸發；本刀＝**被動環境反應**（比照顧家駐足 816／臨水
//!   垂釣 814），你只要靠近就會被認出，不必開口。
//!
//! **純函式層**：確定性、零 LLM、零鎖、零 async、零 IO、可單元測試。連線／鎖／廣播／記憶／Feed
//! 觸發全留在 `voxel_ws.rs`（沿用既有短鎖循序＋鎖外事件佇列慣例，守 prod 死鎖鐵律）。
//!
//! **成本紀律**：零 LLM（確定性選句）、零 migration（純記憶體冷卻欄位，重啟歸零，比照顧家駐足／
//! 臨水垂釣慣例）、零新協議欄位（沿用既有 say 泡泡／記憶／Feed 管線）、零新美術、零前端改動。

/// 巧遇冷卻（秒）：一次巧遇後隔這麼久才會再巧遇同一位居民，避免你賴在邊陲原地不動時
/// 每 tick 狂刷驚喜台詞。刻意短於一趟遠行的逗留時長（[`crate::voxel_expedition::EXPEDITION_STAY_SECS`]
//  = 120 秒），確保同一趟遠行實務上大約只巧遇一次。
pub const FIND_COOLDOWN_SECS: f32 = 100.0;

/// 每次符合條件（在邊陲逗留＋玩家在近旁＋冷卻到期）時的巧遇觸發機率——其餘時候居民只是
/// 照常在營地附近閒晃，不是你一靠近就立刻反應，稀疏一點更像「抬頭才發現你」的自然驚喜。
pub const FIND_CHANCE: f32 = 0.22;

/// 判定「你就在附近」的半徑（世界方塊）：與顧家駐足 [`crate::voxel_homegaze::GAZE_PLAYER_RADIUS`]／
/// 玩家在家 [`crate::voxel_player_home::PLAYER_HOME_DIST`] 同量級（=5.0），沿用「近到能說上話」的距離慣例。
pub const FIND_PLAYER_RADIUS: f32 = 5.0;

/// 動態牆事件種類標籤（邊陲巧遇）。
pub const FEED_KIND: &str = "邊陲巧遇";

/// 三閘判定：居民正在邊陲逗留（`settled`）＋你在近旁（`player_near`）＋冷卻到期＋過機率門檻
/// → 這一 tick 觸發巧遇。純函式，好窮舉測邊界。
pub fn should_find(settled: bool, player_near: bool, cooldown: f32, roll: f32, chance: f32) -> bool {
    settled && player_near && cooldown <= 0.0 && roll < chance
}

/// 巧遇時居民對你說的驚喜台詞（點名你、帶上遠行方位，比在主城相遇更雀躍）——四句輪替，
/// 玩家名截斷不破泡泡框。`pick` 由呼叫端用座標 bits 合成，讓每次挑到的句子自然分散。
pub fn found_bubble(player: &str, bearing: &str, pick: usize) -> String {
    let name = clip_name(player);
    const TEMPLATES: [&str; 4] = [
        "咦？！{name}你怎麼找到{bearing}這麼遠的地方來了！",
        "哇，{name}真的追過來了？這裡可是{bearing}的邊陲呢！",
        "沒想到{name}會一路跑到{bearing}找我，太驚喜了！",
        "{name}，你也太厲害了，連我躲到{bearing}都被你找到。",
    ];
    TEMPLATES[pick % TEMPLATES.len()]
        .replace("{name}", &name)
        .replace("{bearing}", bearing)
}

/// 昇華成一筆「你千里迢迢追到邊陲找到我」的記憶（點名玩家、不含換行，走既有 append-only 記憶管線）。
pub fn found_memory_line(player: &str, bearing: &str) -> String {
    format!(
        "{}一路追到{bearing}的邊陲找到正在遠行的我，那份特地跑一趟的心意，我記下了。",
        clip_name(player)
    )
    .replace('\n', " ")
}

/// 動態牆播報（非同步層，其他玩家能讀到「誰在邊陲巧遇了誰」）。
pub fn found_feed_line(resident: &str, player: &str, bearing: &str) -> String {
    format!("{player}在{bearing}的邊陲，巧遇了正在遠行的{resident}——牠很驚喜你追了這麼遠來找牠。")
}

/// 玩家名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_find_needs_all_four_gates() {
        assert!(should_find(true, true, 0.0, 0.1, FIND_CHANCE));
        // 沒在邊陲逗留（還在路上或已回家）→ 否。
        assert!(!should_find(false, true, 0.0, 0.1, FIND_CHANCE));
        // 玩家不在近旁 → 否。
        assert!(!should_find(true, false, 0.0, 0.1, FIND_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_find(true, true, 1.0, 0.1, FIND_CHANCE));
        // 沒過機率門檻 → 否。
        assert!(!should_find(true, true, 0.0, 0.9, FIND_CHANCE));
    }

    #[test]
    fn found_bubble_names_player_and_bearing_within_limit() {
        for pick in 0..8 {
            let s = found_bubble("阿宅", "東方", pick);
            assert!(s.contains("阿宅"), "應點名玩家: {s}");
            assert!(s.contains("東方"), "應帶上遠行方位: {s}");
            assert!(s.chars().count() <= 40, "泡泡框限制 40 字: {s}");
        }
    }

    #[test]
    fn found_bubble_templates_are_distinct() {
        let lines: std::collections::HashSet<String> =
            (0..4).map(|p| found_bubble("旅人", "西方", p)).collect();
        assert_eq!(lines.len(), 4, "四句應各不相同");
    }

    #[test]
    fn found_bubble_wraps_pick_index() {
        // pick 超出模板數也不 panic，且與取餘後同索引結果一致。
        let a = found_bubble("旅人", "南方", 4);
        let b = found_bubble("旅人", "南方", 0);
        assert_eq!(a, b);
    }

    #[test]
    fn found_bubble_clips_long_player_name() {
        let long_name = "超級無敵霹靂長的玩家顯示名字測試用";
        let s = found_bubble(long_name, "北方", 0);
        assert!(s.chars().count() <= 40, "超長玩家名截斷後仍不破泡泡框: {s}");
    }

    #[test]
    fn found_memory_line_names_player_and_bearing_no_newline() {
        let line = found_memory_line("阿宅", "東方");
        assert!(line.contains("阿宅"));
        assert!(line.contains("東方"));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn found_feed_line_names_both_and_bearing() {
        let line = found_feed_line("露娜", "阿宅", "東方");
        assert!(line.contains("露娜"));
        assert!(line.contains("阿宅"));
        assert!(line.contains("東方"));
    }

    #[test]
    fn clip_name_bounds_length() {
        let long_name = "一二三四五六七八九十十一十二";
        assert!(clip_name(long_name).chars().count() <= 8);
        assert_eq!(clip_name("短名"), "短名");
    }
}
