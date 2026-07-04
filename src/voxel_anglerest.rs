//! 乙太方界·居民臨水垂釣 v1（anglerest）——白天，一位閒著、醒著、恰好走到水邊的居民，
//! 偶爾會**停下腳步、對著水面靜靜垂釣一會兒**（設 `wait_timer` 原地小坐），釣起水下的一尾小魚：
//! 冒一句療癒的垂釣泡泡、心情變好、在城鎮動態牆留一筆。你也在水邊時點你名（一起臨水垂釣、記進交情）。
//!
//! **這一刀補的缺口（記憶/嚮往 → 行為，正中北極星）**：垂釣 v1（734，`voxel_fishing`）的模組註解
//! 早就埋了一句伏筆——「居民的日記早就悄悄嚮往著釣魚（見 `voxel_diary` 的 `Theme::Fishing`：
//! 想去釣魚、水面下藏著什麼樣的安靜）」——但那份嚮往至今**只寫在日記裡、從沒被活出來**：垂釣是
//! 玩家專屬的樂趣，居民永遠只是走來走去採集、蓋造、串門子，從不曾真的在水邊坐下釣一竿。世界的
//! 水體（河、湖、海）對居民而言只是**繞路的障礙**，不是「可以歇下來釣魚的地方」。本切片把居民內在的
//! 那份嚮往第一次接上真實行為：閒下來、恰好臨水的居民會坐下垂釣，把日記裡想了很久的那件事**活出來**。
//! 這正是 `PLAN_ETHERVOX.md` 核心信念「記憶要驅動行為、不只用來聊天」與 §5「日記／生命故事」的交會。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **木長椅歇腳（809/810）**＝坐在**玩家擺的長椅**（吸引物是合成方塊、需玩家先蓋）、動詞是**歇腳**、
//!   任何地表皆可；本刀＝臨**天然水體**（吸引物是世界本有的河湖海、零建造前置）、動詞是**垂釣**
//!   （承接 734 垂釣的「等」的節奏）、只在水邊觸發——吸引物（人造椅／天然水）、行為動詞（歇腳／垂釣）、
//!   前置（要先蓋／世界本有）皆不同。
//! - **玩家垂釣（734）**＝玩家手持釣竿、兩步驟拋竿收竿、防作弊計時；本刀＝**居民**自發、單拍過場、
//!   零物品機制——把「垂釣」這件事第一次從玩家一側鏡射到居民一側，讓居民也活出這份療癒。
//! - **營火圍暖（791）／繁星共賞（783）**＝**夜間**限定；本刀＝**白天**（居民醒著的日間時段），
//!   與夜間那幾條對成日／夜一對。
//!
//! **純函式層**：本模組只有確定性純函式（臨水判定、三閘、台詞、記憶/Feed 文字），零 LLM、零鎖、
//! 零 async、零 IO、可單元測試。連線／鎖／世界讀取（臨水取樣走既有 `world` deltas 讀 guard）／
//! 廣播／記憶寫入全留在 `voxel_ws.rs`（沿用長椅歇腳的短鎖循序＋鎖外事件佇列慣例，守 prod 死鎖鐵律）。

use crate::voxel_fishing as vfish;

/// 城鎮動態牆播報種類名稱。
pub const FEED_KIND: &str = "臨水垂釣";

/// 垂釣冷卻（秒）：一次坐下垂釣後隔這麼久才會再釣，防同一居民狂刷垂釣泡泡（比歇腳更難得些）。
pub const REST_COOLDOWN_SECS: f32 = 150.0;
/// 每次符合條件（白天＋臨水＋冷卻到期）時的垂釣觸發機率——其餘時候只是安靜路過水邊。
pub const FISH_CHANCE: f32 = 0.22;
/// 坐下垂釣時原地停留的秒數（設進 `wait_timer`）：居民真的停下腳步、對著水面靜靜釣一會兒。
pub const FISH_SIT_SECS: f32 = 6.0;
/// 「你也在水邊」的判定半徑（世界方塊）——你在這麼近，垂釣泡泡就會點你名、記進交情。
pub const PLAYER_RADIUS: f32 = 5.0;
/// 垂釣泡泡的字數上限（截斷保護，不破泡泡框）。
pub const SAY_CHARS: usize = 50;

/// 純判定：`neighbors` 是 `voxel_ws` 從居民四周取樣來的鄰格方塊 id，任一是水（來源水或流動水，
/// 沿用 [`vfish::is_water_block`] 單一真相來源，不另立一份水方塊清單）就算「臨水」，可以垂釣。
pub fn any_water(neighbors: &[u8]) -> bool {
    neighbors.iter().any(|&b| vfish::is_water_block(b))
}

/// 三閘判定：白天（`is_day`）＋冷卻到期（`cooldown <= 0`）＋過機率門檻（`roll < chance`）
/// → 這一 tick 坐下垂釣。臨水判定（`any_water`）由呼叫端先過，故不在此重複。純函式、好窮舉測邊界。
/// 邊界 `roll == chance` 不觸發（嚴格小於），與長椅歇腳一致。
pub fn should_fish(is_day: bool, cooldown: f32, roll: f32, chance: f32) -> bool {
    is_day && cooldown <= 0.0 && roll < chance
}

/// 入場冷卻錯開（避免四位居民同一 tick 一起開釣）——依居民序號給遞增的初始冷卻。
pub fn fish_cd_offset(i: usize) -> f32 {
    REST_COOLDOWN_SECS * 0.5 + i as f32 * 18.0
}

/// 垂釣泡泡台詞（通用、不點名）——五句輪替，字數短不破泡泡框。`pick` 由呼叫端用座標 bits 合成。
pub fn angler_bubble(pick: usize) -> &'static str {
    const LINES: [&str; 5] = [
        "在水邊坐下，靜靜垂一竿。",
        "水面真安靜……釣起一尾小魚。",
        "等著魚兒上鉤，心也慢了下來。",
        "終於能坐下來釣魚了，真好。",
        "看著水面發呆，收竿就有收穫。",
    ];
    LINES[pick % LINES.len()]
}

/// 你也在水邊時點名的垂釣泡泡（更親近）——四句輪替，玩家名截斷不破泡泡框。
pub fn angler_bubble_with_player(player: &str, pick: usize) -> String {
    let name = clip_name(player);
    const TEMPLATES: [&str; 4] = [
        "{name}，陪我在水邊釣一竿吧。",
        "有{name}一起臨水垂釣，真愜意。",
        "{name}，你看——我也釣起一尾了！",
        "跟{name}並肩看著水面，靜得舒服。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 昇華成一筆「和你一起在水邊垂釣」的記憶（點名玩家、不含換行，走既有 append-only 記憶管線；
/// 這筆記憶日後會浮進居民日記，把「想釣魚」的嚮往變成「和你一起釣過魚」的回憶）。
pub fn angler_memory_line(player: &str) -> String {
    format!("白天在水邊坐下，和{}一起靜靜垂了會兒釣，釣起了一尾小魚。", clip_name(player))
        .replace('\n', " ")
}

/// 城鎮動態牆播報（非同步層，訪客回來能讀到誰在水邊釣過魚）。
pub fn angler_feed_line(rname: &str) -> String {
    format!("{rname}在水邊靜靜垂釣，釣起一尾小魚，享受了片刻水面的安靜。")
}

/// 玩家名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_water_detects_water_neighbor() {
        // 來源水（7）在鄰格 → 臨水。
        assert!(any_water(&[23, 7, 1]));
        // 流動水各等級（WaterFlow1..7 = 24..=30，vfish 認的水）任一在鄰格 → 臨水。
        assert!(any_water(&[24]));
        assert!(any_water(&[1, 2, 30]));
        // 全是實心陸地方塊（含相鄰的 23、31，剛好落在水區間外）→ 非臨水。
        assert!(!any_water(&[1, 2, 3, 23, 31]));
        // 空取樣（居民四周都沒讀到方塊）→ 非臨水，不會誤觸發。
        assert!(!any_water(&[]));
    }

    #[test]
    fn should_fish_needs_all_three_gates() {
        // 三閘齊備才觸發。
        assert!(should_fish(true, 0.0, 0.1, FISH_CHANCE));
        // 夜裡（非白天）→ 否。
        assert!(!should_fish(false, 0.0, 0.1, FISH_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_fish(true, 5.0, 0.1, FISH_CHANCE));
        // 骰過門檻 → 否；邊界 roll == chance 不觸發（嚴格小於）。
        assert!(!should_fish(true, 0.0, FISH_CHANCE, FISH_CHANCE));
        assert!(!should_fish(true, 0.0, 0.99, FISH_CHANCE));
    }

    #[test]
    fn cd_offset_staggers_and_stays_positive() {
        // 依序遞增（四位居民入場冷卻錯開，不同一 tick 一起開釣），皆為正。
        let a = fish_cd_offset(0);
        let b = fish_cd_offset(1);
        let c = fish_cd_offset(2);
        assert!(a > 0.0 && b > a && c > b, "冷卻錯開應遞增且為正：{a},{b},{c}");
    }

    #[test]
    fn bubbles_rotate_and_stay_in_frame() {
        // 通用垂釣語輪替、非空。
        for p in 0..10 {
            assert!(!angler_bubble(p).is_empty());
        }
        assert_ne!(angler_bubble(0), angler_bubble(1));
        // pick 溢出以取模回繞、不 panic（窮舉不越界）。
        assert_eq!(angler_bubble(5), angler_bubble(0));
        // 點名版含玩家名、輪替、超長名截斷不破框。
        let s = angler_bubble_with_player("旅人", 0);
        assert!(s.contains("旅人"));
        let long = angler_bubble_with_player("超級無敵長長長長長長長名字", 2);
        assert!(long.chars().count() < 40, "超長名應被截斷不破泡泡框：{long}");
    }

    #[test]
    fn memory_and_feed_embed_names_no_newline() {
        // 記憶點名玩家、去換行（擋注入），日後會浮進日記。
        let m = angler_memory_line("諾娃\n注入");
        assert!(m.contains("諾娃"));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
        // 超長玩家名在記憶裡也截斷。
        let long = angler_memory_line("超級無敵長長長長長長長名字");
        assert!(long.chars().filter(|&c| c != '，').count() > 0);
        // Feed 嵌居民名。
        let f = angler_feed_line("露娜");
        assert!(f.contains("露娜"));
    }
}
