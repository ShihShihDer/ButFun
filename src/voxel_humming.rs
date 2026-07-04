//! 乙太方界·居民哼起歌來——心情好時飄出音符，因你而開心時哼給你聽 v1（voxel-humming，自主提案切片 ROADMAP 788）。
//!
//! **缺口 / 為誰做（換維度·非同軸重複）**：乙太方界至今累了滿滿一牆的「居民↔居民到訪劇本」
//! （問候／八卦／互助／拌嘴／傳授／易物／小圈子），這條軸線已被 reviewer 多輪示警飽和；近幾刀又
//! 連走了天象（雨／彩虹／繁星）與慶祝（煙火）。唯獨一個維度這片世界**從來沒有過**——**音樂**。
//! 居民會走、會說話、會有心情（`voxel_mood`），卻從不曾哼過一句歌；世界有人聲、有天象、有煙火，
//! 就是沒有一段旋律。本切片補上這片天地的第一段歌聲：**一位剛因為某件開心事而心情正好的居民，
//! 偶爾會忍不住輕輕哼起歌來，頭頂飄出幾個跳動的音符**。
//!
//! **記憶驅動行為的一拍（PLAN_ETHERVOX 核心信念）**：哼歌不是無來由的裝飾——它的觸發條件正是
//! 「這位居民此刻心情正好」（`mood_boost_secs > 0`，也就是**剛剛才因為一次互動而開心**，可能是你
//! 陪牠聊了天、送了牠禮，也可能是牠剛和老朋友處得愉快）。而最動人的一拍是：**若你此刻正好就在牠
//! 身邊**，牠哼的那句會**輕輕點到你**（「有你在，連哼歌都特別起勁～」），並把「和你在一起、忍不住
//! 哼起歌來」這份好心情記進你們的交情、上動態牆。你陪牠的一段時光，第一次不只換來一句道謝或一份
//! 回禮，還化成了牠嘴邊哼出的一段旋律——你的互動真的有後果，而這回後果是一首歌。
//!
//! **與既有系統的分界**：
//! - 不是心情自語（677，`voxel_mood::spontaneous_line`）——那是依**長期基準心情**（bonds＋記憶多寡）
//!   偶爾自語一句反映近況的話；本切片的觸發是**剛被互動點亮的當下那份新鮮喜悅**（transient
//!   `mood_boost`），是「開心到哼出聲」的那一刻，而且它的招牌是**世界裡第一次飄出的音符**（前端視覺），
//!   自語沒有。兩者觸發源（基準 vs 當下）、表現（純文字 vs 音符＋歌詞）都不同。
//! - 不是望星邀約（783）／睹物思人（784）——那兩者各由天象＋偏好、或物件觸發；本切片由「剛被點亮的
//!   好心情」觸發，是居民內在狀態滿溢成歌，不依賴任何外物。
//!
//! **前端契約（零新協議欄位）**：所有哼歌台詞一律以音符符號 [`NOTE_PREFIX`]（♪）起頭。前端在既有
//! 居民 `say` 廣播上偵測「以 ♪ 起頭」即知這是一段歌聲，於該居民頭頂生成飄浮音符點雲——後端不必為此
//! 多送任何一個欄位，舊前端也只會把它當一句普通泡泡安全落回。
//!
//! **成本 / 濫用防護鐵律**：
//! - **純邏輯層**：觸發判定與哼歌台詞／記憶／Feed 全為確定性純函式，零 LLM、零鎖、零 IO、可窮舉
//!   單元測試。鎖與副作用全在 `voxel_ws.rs`（短鎖即釋、不巢狀、記憶／Feed 走鎖外事件佇列，守 prod
//!   死鎖鐵律）。
//! - 台詞永不回放玩家原話——只嵌玩家**顯示名**（既有安全字串），無注入 / NSFW / 洗版風險。
//!   訪客（名字空白）只哼無詞的調子、不記交情、不上動態牆。
//! - 觸發須「心情正好」＋每居民長冷卻（[`HUM_COOLDOWN_SECS`]）＋極低機率（[`HUM_CHANCE`]）＝
//!   哼歌是偶爾滿溢的一拍、不洗版；各居民初始冷卻錯開。
//! - 面向玩家字串集中此處（i18n 友善）；繁中註解；不碰玩家資料表；零持久化、零 migration
//!   （哼歌冷卻純記憶體、重啟歸零，記憶走既有 append-only 管線）。

/// 哼歌台詞的音符前綴——**前端契約**：以此起頭的 `say` 廣播會被前端認出、生成飄浮音符。
pub const NOTE_PREFIX: &str = "♪";

/// Feed 播報種類名稱（動態牆分類）。
pub const FEED_KIND: &str = "哼歌";

/// 哼歌冷卻（秒）：一次哼歌後設此值，歸零前不再觸發——偶爾滿溢一拍才有感、不洗版。
pub const HUM_COOLDOWN_SECS: f32 = 160.0;

/// 每次「符合條件的 tick」真的哼出聲的機率（極低）——配合長冷卻＝天然節流。
pub const HUM_CHANCE: f32 = 0.035;

/// 「哼給你聽」的觸發半徑（格，水平 xz）：玩家離哼歌居民多近才算「哼給你聽」、點到你名。
/// 超出此半徑則只哼無詞的調子（記憶／Feed 不寫）。用距離平方比較，避免開根號。
pub const HUM_NEAR_RADIUS: f32 = 16.0;

/// 是否此刻哼起歌來：心情正好（剛被互動點亮）＋冷卻已過＋過機率門檻，三者皆備才觸發。
/// say 是否為空、是否醒著、手邊是否有正事由呼叫端在外層先確認（沿用 stargaze／keepsake 慣例）。
/// 純函式、可測。
pub fn should_hum(happy: bool, cooldown_ok: bool, roll: f32, threshold: f32) -> bool {
    happy && cooldown_ok && roll < threshold
}

/// 玩家就在身邊時、哼給你聽的一句（點到你名，暖而不膩）。永不回放原話、只嵌玩家顯示名；
/// 依 `pick` 確定性選句、以 [`NOTE_PREFIX`] 起頭、截 40 字防泡泡溢框。
pub fn hum_to_player_line(player: &str, pick: usize) -> String {
    let pool: [&str; 4] = [
        "啦啦～有{p}在，連哼歌都特別起勁～",
        "今天心情真好，哼首歌給{p}聽～",
        "嗯～和{p}在一起的日子，忍不住哼起歌來。",
        "啦～這麼開心，{p}你也一起哼嘛～",
    ];
    with_note(clip(pool[pick % pool.len()].replace("{p}", player)))
}

/// 沒有玩家在身邊（或訪客）時、獨自哼的無詞調子——純氛圍，不寫記憶、不上 Feed。
/// 以 [`NOTE_PREFIX`] 起頭、確定性選句、截 40 字。
pub fn hum_solo_line(pick: usize) -> String {
    let pool: [&str; 4] = [
        "啦啦啦～嗯哼～今天真是好日子～",
        "嗯～嗯～哼著哼著，腳步都輕快了起來～",
        "啦～啦～心裡暖暖的，就想哼首歌～",
        "哼～哼～這樣的日子，值得哼上一段～",
    ];
    with_note(clip(pool[pick % pool.len()].to_string()))
}

/// 哼歌記憶摘要（掛在玩家名下，供日記昇華成生命故事）。只嵌玩家顯示名。
pub fn hum_memory_line(player: &str, pick: usize) -> String {
    let pool: [&str; 3] = [
        "和{p}在一起，心情好得忍不住哼起歌來。",
        "{p}在身邊的時候，我輕輕哼了一段歌。",
        "那天和{p}在一塊，開心得唱了起來。",
    ];
    clip(pool[pick % pool.len()].replace("{p}", player))
}

/// 動態牆播報句。
pub fn hum_feed_line(resident: &str, player: &str) -> String {
    clip(format!("{resident}心情正好，哼著歌陪在{player}身邊"))
}

/// 補上音符前綴（前端契約）。若已以前綴起頭則不重複。
fn with_note(line: String) -> String {
    if line.starts_with(NOTE_PREFIX) {
        line
    } else {
        format!("{NOTE_PREFIX} {line}")
    }
}

/// 泡泡／記憶／Feed 統一截字（≤40 字，防溢框）。
fn clip(line: String) -> String {
    line.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_hum_needs_all_conditions() {
        assert!(should_hum(true, true, 0.01, HUM_CHANCE));
        // 心情沒被點亮 → 否。
        assert!(!should_hum(false, true, 0.01, HUM_CHANCE));
        // 冷卻未過 → 否。
        assert!(!should_hum(true, false, 0.01, HUM_CHANCE));
        // 骰子過門檻 → 否。
        assert!(!should_hum(true, true, 0.9, HUM_CHANCE));
    }

    #[test]
    fn should_hum_chance_boundary_strictly_less() {
        // roll 正好等於 threshold 不觸發（嚴格小於）。
        assert!(!should_hum(true, true, HUM_CHANCE, HUM_CHANCE));
        assert!(should_hum(true, true, HUM_CHANCE - 0.001, HUM_CHANCE));
    }

    #[test]
    fn all_hum_lines_start_with_note_prefix() {
        // 前端契約：哼給你聽與獨自哼的台詞都必須以音符起頭，前端才認得出來生成音符。
        for pick in 0..8 {
            assert!(hum_to_player_line("阿光", pick).starts_with(NOTE_PREFIX));
            assert!(hum_solo_line(pick).starts_with(NOTE_PREFIX));
        }
    }

    #[test]
    fn to_player_line_embeds_name_clips_and_rotates() {
        for pick in 0..8 {
            let line = hum_to_player_line("阿光", pick);
            assert!(line.contains("阿光"));
            assert!(line.chars().count() <= 42 && !line.is_empty()); // 40 內文 + "♪ " 前綴
        }
        // pick 輪替換句。
        assert_ne!(hum_to_player_line("阿光", 0), hum_to_player_line("阿光", 1));
    }

    #[test]
    fn solo_line_has_no_player_name_and_rotates() {
        // 獨自哼的調子不該嵌任何玩家名（無詞、純氛圍）。
        for pick in 0..8 {
            let line = hum_solo_line(pick);
            assert!(!line.is_empty() && line.chars().count() <= 42);
        }
        assert_ne!(hum_solo_line(0), hum_solo_line(1));
    }

    #[test]
    fn memory_line_embeds_name_clips_and_rotates() {
        for pick in 0..6 {
            let mem = hum_memory_line("諾娃", pick);
            assert!(mem.contains("諾娃"));
            assert!(mem.chars().count() <= 40 && !mem.is_empty());
        }
        assert_ne!(hum_memory_line("諾娃", 0), hum_memory_line("諾娃", 1));
    }

    #[test]
    fn feed_line_embeds_both_names_and_clips() {
        let feed = hum_feed_line("露娜", "阿光");
        assert!(feed.contains("露娜") && feed.contains("阿光"));
        assert!(feed.chars().count() <= 40 && !feed.is_empty());
    }

    #[test]
    fn lines_never_empty_or_broken_for_long_names() {
        // 超長玩家名不會截到破壞辨識或空字串、也不會吃掉音符前綴。
        let long = "超級無敵長長長長長長長長長長長長長長長長的旅人名字";
        let bubble = hum_to_player_line(long, 0);
        assert!(bubble.starts_with(NOTE_PREFIX) && !bubble.is_empty());
        let mem = hum_memory_line(long, 0);
        assert!(!mem.is_empty() && mem.chars().count() <= 40);
    }
}
