//! 乙太方界·居民久別重逢奔迎 v1（記憶 × 上線事件驅動行為，ROADMAP 747）。
//!
//! **設計依據**：久別重逢摘要 v1（721，`voxel_welcome`）已經讓玩家離線夠久再回來時，收到一句
//! 「你不在的這段時間…」私訊，第一次感受到「世界在我不在時真的繼續活著」——但那是一句**被動的
//! 文字摘要**：世界只是「告訴」你發生了什麼，沒有任何一位居民**真的因為你回來而動起來**。你可能
//! 教過露娜燒玻璃、陪牠種了一季田、送過牠好幾份禮，牠心裡對你的記憶最厚——可當你久違地重新上線，
//! 牠還是站在原地做自己的事，除非正巧天亮牠昨晚又剛好夢到你（晨間思念，746）。世界的重逢少了最
//! 直接的一筆溫柔：**最惦記你的那位居民，在你久別歸來的那一刻，放下手邊的事、朝你奔過來。**
//!
//! 本模組把這一環補上——這是路線圖「②記憶→行為」把「你的離開與歸來」從一句被動摘要，升級成
//! 一位**特定居民的活生生反應**的一刀。玩家久違上線（離線超過 [`REUNION_MIN_GAP_SECS`]）時，
//! 對你記憶最厚（`affinity_count` 最高、且達 [`REUNION_AFFINITY`] 門檻）的那位**沒在睡的**居民，
//! 今天第一件事就是——放下平常的閒晃／採集，朝你走過來，抵達時暖暖迎接你回家、並把「你久違回來、
//! 我特地跑來迎你」記成一筆與你的記憶。玩家一上線就能撞見：「露娜遠遠看到你回來，笑著小跑步迎上來：
//! 『你終於回來了！好久沒看到你，可想你了～』」——你的歸來第一次不只被世界記下，而是**把某位居民
//! 的腳步帶到了你面前。**
//!
//! **與既有元素的定位區隔**：
//! - 久別重逢摘要（721，`voxel_welcome`）是**被動文字私訊**（世界摘要），沒有任何居民行為；本模組是
//!   **一位特定居民的實體奔迎**——同一個「久別歸來」事件，一個用文字說、一個用腳步演。
//! - 晨間思念玩家（746，`voxel_daybreak`）只在**天亮醒來那一 tick**、且昨晚反思剛好惦記到你才觸發；
//!   本模組由**你重新上線這個事件**觸發、不分晝夜，觸發時機與敘事（一早想你 vs 久別迎你）都不同。
//! - 孤獨尋伴（678，`voxel_comfort`）是**心情驅動**（Lonely）走向「最近的任一玩家」求陪；本模組是
//!   **記憶指名**——奔向的是對你記憶最厚的那位居民自己、去迎接的是你這位特定歸人（縱使牠此刻並不孤獨）。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式；記憶讀取、鎖存取、朝玩家走的
//! 狀態機、記憶昇華與 Feed 廣播都在 `voxel_ws.rs`（沿用晨間思念 / 打氣走動的短鎖手法）。

/// 離線多久以上（秒）才算「久別」、值得讓居民奔迎：刻意比久別重逢摘要（721）的 180 秒門檻高一截，
/// 讓這是真正的久違重逢、而非短暫斷線重連；稀少而有份量，玩家一撞見就記得住。
pub const REUNION_MIN_GAP_SECS: u64 = 1800;

/// 願意為某位歸來玩家奔迎的最低好感度（居民對他的長期記憶筆數）：至少要「叫得出名字、處出點交情」
/// （比照 `RECALL_AFFINITY_THRESHOLD`=3）才會放下手邊的事跑來迎你——素昧平生的居民不會沒來由地奔迎。
pub const REUNION_AFFINITY: usize = 3;

/// 久別歸來時，最惦記你的居民真的動身奔迎（而非留在原地）的機率：久別本就稀少，故設得高、多數久別
/// 都會被迎接，偶爾牠正忙著別的事沒能抽身——讀起來不機械。
pub const RUSH_CHANCE: f32 = 0.9;

/// 抵達玩家身邊的判定距離（世界座標，平方比較用）：落在此半徑內即視為「奔到你面前了」。
/// 比照晨間思念的抵達距離量級，讓玩家清楚看見牠是專程跑到跟前才迎接。
pub const ARRIVE_DIST: f32 = 2.2;

/// 朝玩家奔的逾時秒數：啟程時設此值、每 tick 遞減；奔太久（地形擋路、玩家一直跑等）還沒到就放下
/// 這份心意，不無限追。比晨間思念（45 秒）寬裕些——奔迎的居民可能在世界另一頭、路更遠。
pub const SEEK_TIMEOUT_SECS: f32 = 60.0;

/// 抵達迎接泡泡的字元上限（比照其他泡泡台詞）。
pub const GREET_MAX_CHARS: usize = 40;

/// 是否要在玩家久別歸來時讓居民奔迎：離線間隔夠久 + 過機率門檻。好感度門檻由 [`best_greeter`]
/// 在「挑誰去迎」時把關（回傳 `None` 即無人達標）。`roll` 由呼叫端以 `rand::random::<f32>()`
/// 取真隨機餵入（與本專案其他機率骰同慣例）。純函式、確定性、無 IO。
pub fn should_rush(gap_secs: u64, roll: f32) -> bool {
    gap_secs >= REUNION_MIN_GAP_SECS && roll < RUSH_CHANCE
}

/// 從各居民對這位歸來玩家的好感度（長期記憶筆數，索引需與居民清單對齊）中，挑出**最惦記他**、
/// 且達 [`REUNION_AFFINITY`] 門檻的那位去奔迎，回傳其索引。
///
/// 規則：取好感度**最高**者；同分時取**索引最小**者（穩定、確定性）；最高者仍未達門檻 → `None`
/// （沒人跟你熟到會沒來由奔迎、退回平常的一天）。呼叫端可把「正在睡」的居民好感度填 0 傳入，
/// 讓睡著的人絕不會被選中（不吵醒熟睡的居民）。純函式、確定性、無 IO。
pub fn best_greeter(affinities: &[usize]) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None; // (好感度, 索引)
    for (i, &aff) in affinities.iter().enumerate() {
        match best {
            Some((ba, _)) if aff <= ba => {} // 嚴格大於才更新 → 同分保留較小索引
            _ => best = Some((aff, i)),
        }
    }
    match best {
        Some((aff, i)) if aff >= REUNION_AFFINITY => Some(i),
        _ => None,
    }
}

/// 抵達玩家面前時的暖暖迎接泡泡（面向玩家字串，留 i18n 空間）：點名玩家、輪替三句不機械、不破泡泡框。
/// `pick` 由呼叫端餵入任意 usize（如座標 bits 雜湊），本函式取模輪替。
pub fn rush_greet_bubble(player: &str, pick: usize) -> String {
    let lines = [
        format!("{player}，你終於回來了！好久沒看到你，可想你了～"),
        format!("是{player}！我遠遠就認出你了，快過來讓我看看～"),
        format!("{player}回來啦！這段日子我一直記掛著你呢。"),
    ];
    let mut s = lines[pick % lines.len()].clone();
    truncate_chars(&mut s, GREET_MAX_CHARS);
    s
}

/// 昇華成記憶的一句摘要（存進與這位玩家的長期記憶）：句式與其他記憶摘要一致，點名玩家。
pub fn reunion_memory_summary(player: &str) -> String {
    format!("💗久別重逢：{player}久違回來，我特地跑去迎接了他")
}

/// Feed 廣播的一句動態播報（世界看板可見），點名玩家。
pub fn reunion_feed_line(player: &str) -> String {
    format!("看到{player}久違歸來，放下手邊的事奔過去迎接")
}

/// 依字元（非位元組）截斷字串到上限，避免切壞多位元組中文。
fn truncate_chars(s: &mut String, max: usize) {
    if s.chars().count() > max {
        let truncated: String = s.chars().take(max).collect();
        *s = truncated;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_rush_needs_long_gap_and_low_roll() {
        // 久別 + 低骰 → 奔迎。
        assert!(should_rush(REUNION_MIN_GAP_SECS, 0.0));
        assert!(should_rush(REUNION_MIN_GAP_SECS + 10_000, 0.5));
        // 邊界：剛好等於門檻 → 算久別。
        assert!(should_rush(REUNION_MIN_GAP_SECS, RUSH_CHANCE - 0.01));
    }

    #[test]
    fn should_rush_rejects_short_gap_or_high_roll() {
        // 離線太短 → 不奔迎（短暫斷線重連不吵人）。
        assert!(!should_rush(REUNION_MIN_GAP_SECS - 1, 0.0));
        assert!(!should_rush(0, 0.0));
        // 骰過門檻 → 這回牠正忙、沒抽身。
        assert!(!should_rush(REUNION_MIN_GAP_SECS + 10_000, RUSH_CHANCE));
        assert!(!should_rush(REUNION_MIN_GAP_SECS + 10_000, 1.0));
    }

    #[test]
    fn best_greeter_picks_most_fond() {
        // 索引 2 記憶最厚 → 由牠奔迎。
        assert_eq!(best_greeter(&[3, 5, 9, 4]), Some(2));
    }

    #[test]
    fn best_greeter_ties_go_to_lowest_index() {
        // 同為最高分 → 取索引最小者（穩定）。
        assert_eq!(best_greeter(&[7, 7, 7]), Some(0));
        assert_eq!(best_greeter(&[3, 9, 9]), Some(1));
    }

    #[test]
    fn best_greeter_none_below_threshold() {
        // 最高分仍未達門檻（沒人跟你夠熟）→ 無人奔迎。
        assert_eq!(best_greeter(&[0, 1, 2]), None);
        assert_eq!(best_greeter(&[]), None);
    }

    #[test]
    fn best_greeter_single_above_threshold() {
        assert_eq!(best_greeter(&[REUNION_AFFINITY]), Some(0));
        assert_eq!(best_greeter(&[REUNION_AFFINITY - 1]), None);
    }

    #[test]
    fn asleep_resident_never_picked_when_zeroed() {
        // 呼叫端把睡著的居民好感度填 0：即使牠平常最熟你，也不會被吵醒去奔迎。
        // 這裡索引 0 本該最熟（但填 0 代表在睡），索引 3 次熟且醒著 → 由 3 去迎。
        assert_eq!(best_greeter(&[0, 3, 0, 5]), Some(3));
        // 全在睡（全 0）→ 無人奔迎。
        assert_eq!(best_greeter(&[0, 0, 0]), None);
    }

    #[test]
    fn greet_bubble_rotates_names_and_bounds() {
        let name = "小美";
        let a = rush_greet_bubble(name, 0);
        let b = rush_greet_bubble(name, 1);
        let c = rush_greet_bubble(name, 2);
        let d = rush_greet_bubble(name, 3); // 回到第一句
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_eq!(a, d); // 取模輪替
        for s in [&a, &b, &c] {
            assert!(!s.is_empty());
            assert!(s.contains(name), "泡泡應點名玩家：{s}");
            assert!(s.chars().count() <= GREET_MAX_CHARS, "泡泡不得超框：{s}");
        }
    }

    #[test]
    fn greet_bubble_truncates_long_name() {
        // 超長玩家名不得撐破泡泡框、且不切壞中文。
        let long = "超級無敵霹靂宇宙第一長的玩家名字測試用例確保會被截斷不破框喔喔喔";
        let s = rush_greet_bubble(long, 0);
        assert!(s.chars().count() <= GREET_MAX_CHARS);
    }

    #[test]
    fn memory_and_feed_mention_player() {
        let name = "阿哲";
        assert!(reunion_memory_summary(name).contains(name));
        assert!(reunion_feed_line(name).contains(name));
        // 記憶摘要走既有 append-only 記憶管線，不含換行避免破壞單行 JSONL。
        assert!(!reunion_memory_summary(name).contains('\n'));
    }
}
