//! 乙太方界·雨天葉傘避雨 v1（voxel_raincover）——下雨時，閒著、醒著、恰好在雨裡的居民，
//! 偶爾**摘一片闊葉舉在頭頂當傘、停下腳步躲一會兒雨**：說句避雨的話、心情因這點小小的遮蔽
//! 而安穩一格；你也在近旁時，牠會招呼你「一起躲我這片葉傘下」、把「和你一起避雨」記進交情。
//!
//! **這一刀補的缺口**：下雨天氣（700）與雨天反應（701）至今，雨對居民**只改變了牠們「說什麼」**
//! （雨剛下時冒一句應景台詞），卻**從沒改變牠們「做什麼」**——一場雨從頭下到尾，居民照樣一刻不停
//! 地在雨裡走來走去採集、蓋造、串門子，彷彿天沒在下。這是世界第一次讓**雨真的改變居民的行為**：
//! 淋著雨的居民會停下腳步、摘片葉子擋一擋、躲一會兒——環境不再只是背景色調與頭頂的雨滴，而是
//! 真真切切地牽動了村民此刻在做的事（PLAN_ETHERVOX：環境 × 居民即時反應、狀態/環境驅動**行為**）。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **雨天反應（701）**＝雨**剛下那一刻**冒一句台詞、隨即照常活動；本刀＝雨**下著的整段期間**，
//!   居民**真的停步躲雨**（設 `wait_timer`）——從「說一句」升級成「做一件事」，動詞全新。
//! - **雨後彩虹（780）**＝雨**停**後抬頭望天歡呼；本刀＝雨**正下**時低頭找遮蔽，時機相反、動作相反。
//! - **長椅歇腳（810）／臨水垂釣（814）**＝**晴天**白日、被**椅／水**吸引、悠閒駐足；本刀＝**雨天**、
//!   被**雨**逼著、找遮蔽的躲避——觸發物（好天氣的閒情／壞天氣的驅避）恰成一對相反。
//!
//! **純函式層**：確定性、零 LLM、零鎖、零 async、零 IO、可單元測試。連線／鎖／廣播／記憶／Feed
//! 觸發全留在 `voxel_ws.rs`（沿用既有短鎖循序＋鎖外事件佇列慣例，守 prod 死鎖鐵律）。

/// 居民躲雨冷卻（秒）：一次躲雨後隔這麼久才會再躲，防同一居民在一場雨裡狂刷避雨泡泡。
/// 比營火取暖（150）／垂釣同量級——避雨是一場雨裡偶爾一拍，不是每 tick 都躲。
pub const SHELTER_COOLDOWN_SECS: f32 = 150.0;

/// 每次符合條件（下雨＋冷卻到期）時的躲雨觸發機率——其餘時候只是在雨裡照常走著。
/// 略低於長椅歇腳（0.28）：不是每個淋雨的人都會停下躲，讓「停步避雨」稀疏而自然。
pub const SHELTER_CHANCE: f32 = 0.22;

/// 躲雨時原地停留的秒數（設進 `wait_timer`）：居民真的停下腳步、在葉傘底下躲一會兒雨。
pub const SHELTER_HUDDLE_SECS: f32 = 5.0;

/// 「你也在近旁」的判定半徑（世界方塊）——你在這麼近，居民的避雨話就會點你名、招呼你共避、記進交情。
pub const SHELTER_PLAYER_RADIUS: f32 = 5.0;

/// 避雨泡泡台詞最多顯示字數（截斷防超長玩家名撐破泡泡框）。
pub const SAY_CHARS: usize = 50;

/// 動態牆事件種類標籤（雨中避雨）。
pub const FEED_KIND: &str = "雨中避雨";

/// 各居民首次躲雨冷卻的錯開偏移（秒）：避免一下雨同一 tick 一群人齊聲說避雨話。
/// 依居民序 `i` 遞增，比照 `vangler::fish_cd_offset` 慣例。
pub fn shelter_cd_offset(i: usize) -> f32 {
    70.0 + i as f32 * 25.0
}

/// 三閘判定：正下雨（`raining`）＋冷卻到期（`cooldown <= 0`）＋過機率門檻（`roll < chance`）
/// → 這一 tick 停步躲雨。純函式，好窮舉測邊界。
pub fn should_shelter(raining: bool, cooldown: f32, roll: f32, chance: f32) -> bool {
    raining && cooldown <= 0.0 && roll < chance
}

/// 躲雨泡泡台詞（通用、不點名）——五句輪替，字數短不破泡泡框。`pick` 由呼叫端用座標 bits
/// 合成，讓每次挑到的句子自然分散。
pub fn shelter_bubble(pick: usize) -> &'static str {
    const LINES: [&str; 5] = [
        "雨下起來了，摘片葉子擋一擋。",
        "呀，這場雨，先躲一會兒吧。",
        "雨真急，在葉子底下躲躲。",
        "淋著雨可不行，避一避。",
        "拿片闊葉當傘，雨中也自在。",
    ];
    LINES[pick % LINES.len()]
}

/// 你也在近旁時的避雨泡泡（點名玩家、招呼共避一葉傘，更親近）——四句輪替，玩家名截斷不破泡泡框。
pub fn shelter_bubble_with_player(player: &str, pick: usize) -> String {
    let name = clip_name(player);
    const TEMPLATES: [&str; 4] = [
        "{name}，這雨大，來我葉傘底下躲躲。",
        "{name}，一起擠這片葉子下避避雨吧。",
        "別淋著了{name}，來這兒躲雨。",
        "有{name}一起躲雨，這場雨也不惱人了。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 昇華成一筆「和你一起擠在葉傘下避雨」的記憶（點名玩家、不含換行，走既有 append-only 記憶管線）。
pub fn shelter_memory_line(player: &str) -> String {
    format!("下雨天，和{}一起擠在一片葉傘底下躲了會兒雨。", clip_name(player)).replace('\n', " ")
}

/// 動態牆播報（非同步層，訪客回來能讀到誰在雨裡躲過雨）。
pub fn shelter_feed_line(rname: &str) -> String {
    format!("{rname}摘了片闊葉當傘，在雨裡躲了會兒。")
}

/// 玩家名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_shelter_needs_all_three_gates() {
        // 三閘齊備才觸發。
        assert!(should_shelter(true, 0.0, 0.1, SHELTER_CHANCE));
        // 沒下雨 → 否（大晴天不會躲雨）。
        assert!(!should_shelter(false, 0.0, 0.1, SHELTER_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_shelter(true, 5.0, 0.1, SHELTER_CHANCE));
        // 骰過門檻 → 否；邊界 roll == chance 不觸發（嚴格小於）。
        assert!(!should_shelter(true, 0.0, SHELTER_CHANCE, SHELTER_CHANCE));
        assert!(!should_shelter(true, 0.0, 0.99, SHELTER_CHANCE));
    }

    #[test]
    fn cd_offset_staggers_by_index_and_is_positive() {
        // 各居民錯開、遞增、皆為正（避免一下雨同一 tick 齊躲）。
        assert!(shelter_cd_offset(0) > 0.0);
        assert!(shelter_cd_offset(1) > shelter_cd_offset(0));
        assert!(shelter_cd_offset(3) > shelter_cd_offset(2));
    }

    #[test]
    fn bubbles_rotate_and_stay_in_frame() {
        // 通用避雨語輪替、非空。
        for p in 0..10 {
            assert!(!shelter_bubble(p).is_empty());
        }
        assert_ne!(shelter_bubble(0), shelter_bubble(1));
        // pick 溢出取模不 panic、仍回合法句。
        assert!(!shelter_bubble(usize::MAX).is_empty());
    }

    #[test]
    fn player_bubble_embeds_name_rotates_and_clips() {
        // 點名版含玩家名、輪替、超長名截斷不破框。
        let s = shelter_bubble_with_player("旅人", 0);
        assert!(s.contains("旅人"));
        assert_ne!(
            shelter_bubble_with_player("旅人", 0),
            shelter_bubble_with_player("旅人", 1)
        );
        let long = shelter_bubble_with_player("超級無敵長長長長長長長名字", 2);
        assert!(long.chars().count() < 40, "超長名應被截斷不破泡泡框：{long}");
    }

    #[test]
    fn memory_and_feed_embed_names_no_newline() {
        // 記憶點名、去換行（防注入撐破 jsonl 行）。
        let m = shelter_memory_line("諾娃\n注入");
        assert!(m.contains("諾娃"));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
        // 空名安全不 panic、仍成句。
        assert!(!shelter_memory_line("").is_empty());
        let f = shelter_feed_line("露娜");
        assert!(f.contains("露娜"));
    }

    #[test]
    fn constants_are_sane() {
        // 機率在 (0,1) 開區間、冷卻與停留為正、玩家半徑為正。
        assert!(SHELTER_CHANCE > 0.0 && SHELTER_CHANCE < 1.0);
        assert!(SHELTER_COOLDOWN_SECS > 0.0);
        assert!(SHELTER_HUDDLE_SECS > 0.0);
        assert!(SHELTER_PLAYER_RADIUS > 0.0);
        assert!(!FEED_KIND.is_empty());
    }
}
