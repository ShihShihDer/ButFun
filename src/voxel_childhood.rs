//! 乙太方界·孩子的模樣與玩耍時光 v1（voxel-childhood，自主提案切片，接續 942/987
//! 生命週期軸線，reviewer 明令別再連發「彙整清單→依序瀏覽面板」，回頭補一個真正的行為新維度）。
//!
//! **真缺口**：家庭這條線讓孩子誕生（928 愛的結晶）、活過一整個乙太年長大成人（942 成年禮）、
//! 甚至活到晚年（987）——生命週期的每個「轉捩點」都被世界鄭重其事地記下。但轉捩點之間那段
//! 實打實的**童年本身**，從沒被世界看見過：一個剛出生的孩子，體型、動作、日常舉止都跟成年
//! 居民一模一樣，玩家完全認不出誰是孩子、誰是大人——「童年」至今只是後端一個計時器，從沒有
//! 任何一眼可辨的樣子。
//!
//! 本切片給童年本身一個模樣：**世代傳承誕生、尚未行過成年禮的孩子，體型比成年居民小一圈**
//! （前端渲染縮放，`crate::voxel_coming_of_age::is_adult` 單一事實來源，零新持久化狀態），
//! 閒晃時偶爾會忍不住玩起純真的小把戲（追蝴蝶、繞圈圈、踢石子），與居民一貫沉穩的日常
//! （工作、閒聊、串門子）判然不同——是「記憶要驅動行為」北極星底下，第一次連**生命階段本身**
//! 都反過來改寫了行為與樣貌，而不只是轉捩點那一瞬間的儀式。
//!
//! **與既有系統 razor-sharp 區隔**：
//! - 942 成年禮／987 晚年＝生命階段**轉換那一刻**的一次性儀式（記憶/持久化/父母欣慰）；
//!   本刀＝轉換之間那段**持續期間**的日常樣貌與行為，兩者互補、無重疊（判定共用同一顆
//!   `is_adult`，但本刀不寫任何持久化、不觸發永久記憶，純粹是「當下是不是孩子」的即時渲染）。
//! - 715 拌嘴／788 哼歌等既有 flavor 系統都是「成年居民之間」的互動；本刀是童年獨有、
//!   與任何居民互動無關的**獨處**小把戲，觸發條件（`is_child`）與其餘系統完全正交。
//!
//! **成本鐵律**：純規則式（挑句確定性、零 LLM、零 IO、零鎖），可單元測試；不新增任何
//! 持久化欄位——是否為孩子由既有 `birth_unix` 現算，重啟不受影響；不新增協議破壞欄位
//! （廣播 `ResidentView` 新增的 `is_child` 為 additive bool，舊前端安全忽略）。
//! **濫用防護**：不收玩家輸入、不觸發 LLM、不開新端點——孩子的年歲與是否玩耍純由伺服器
//! 內部時鐘與出生紀錄決定，玩家無從自報或催發。

use crate::voxel_coming_of_age::is_adult;

/// 前端契約：以此前綴起頭的 `say` 廣播＝孩子正玩起小把戲（前端據此播放較活潑的跳動姿態），
/// 比照 [`crate::voxel_humming::NOTE_PREFIX`] 音符前綴同一套「零新協議欄位」手法。
pub const PLAY_PREFIX: &str = "☆";

/// 孩子玩耍冷卻（秒）：一次玩耍後設此值，歸零前不再觸發——偶爾一拍才有童趣、不洗版。
pub const PLAY_COOLDOWN_SECS: f32 = 120.0;

/// 每次「符合條件的 tick」真的玩起來的機率（低）——配合長冷卻＝天然節流。
pub const PLAY_CHANCE: f32 = 0.05;

/// 前端渲染縮放倍率：孩子的體型是成年居民的這個比例（矮小圓潤、一眼可辨）。
pub const CHILD_SCALE: f32 = 0.62;

/// 判斷這位居民此刻是不是孩子——單一事實來源沿用 [`is_adult`]（世代傳承誕生、尚未滿一個
/// 乙太年即為孩子；初始四位居民 `birth_unix == 0` 恆成年，永不判定為孩子）。
pub fn is_child(birth_unix: u64, now: u64) -> bool {
    !is_adult(birth_unix, now)
}

/// 是否此刻玩起小把戲：是孩子＋手邊沒正事（外層先確認 say 為空／醒著／非朝聖遠行）＋
/// 冷卻已過＋過機率門檻，缺一不可。純函式、可測。
pub fn should_play(child: bool, idle_ok: bool, cooldown_ok: bool, roll: f32) -> bool {
    child && idle_ok && cooldown_ok && roll < PLAY_CHANCE
}

/// 孩子獨處玩耍的台詞池——純真的小把戲，貼合療癒基調，不涉及任何目的性行為。
const PLAY_LINES: [&str; 6] = [
    "追著一隻蝴蝶跑了好幾步，咯咯笑個不停！",
    "原地轉起圈圈，轉到自己都站不穩、笑倒在地上。",
    "撿起一顆小石子，踢著踢著就繞了半個院子。",
    "追著自己的影子繞圈圈，怎麼追都追不上。",
    "蹦蹦跳跳地跳起格子，一格都不肯踩線。",
    "蹲下來對著一朵小花看了好久，又對著它笑了。",
];

/// 依 `pick` 確定性選一句玩耍台詞，以 [`PLAY_PREFIX`] 起頭。
pub fn play_line(pick: usize) -> String {
    format!("{PLAY_PREFIX}{}", PLAY_LINES[pick % PLAY_LINES.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    const YEAR: u64 = crate::voxel_coming_of_age::COMING_OF_AGE_SECS;

    #[test]
    fn founders_birth_zero_never_child() {
        assert!(!is_child(0, 0));
        assert!(!is_child(0, 999_999_999));
    }

    #[test]
    fn newborn_is_child() {
        let birth = 1_000_000;
        assert!(is_child(birth, birth));
        assert!(is_child(birth, birth + YEAR - 1));
    }

    #[test]
    fn grown_up_at_threshold_is_not_child() {
        let birth = 1_000_000;
        assert!(!is_child(birth, birth + YEAR));
        assert!(!is_child(birth, birth + YEAR * 5));
    }

    #[test]
    fn should_play_requires_all_gates() {
        assert!(should_play(true, true, true, 0.0));
        assert!(!should_play(false, true, true, 0.0), "非孩子不觸發");
        assert!(!should_play(true, false, true, 0.0), "手邊有正事不觸發");
        assert!(!should_play(true, true, false, 0.0), "冷卻中不觸發");
        assert!(!should_play(true, true, true, 0.999), "機率門檻擋下");
    }

    #[test]
    fn should_play_respects_chance_threshold() {
        assert!(should_play(true, true, true, PLAY_CHANCE - 0.001));
        assert!(!should_play(true, true, true, PLAY_CHANCE));
    }

    #[test]
    fn play_line_prefixed_and_deterministic() {
        for pick in 0..PLAY_LINES.len() * 2 {
            let line = play_line(pick);
            assert!(line.starts_with(PLAY_PREFIX));
            assert_eq!(line, play_line(pick), "同 pick 恆選同句");
        }
    }

    #[test]
    fn play_line_pool_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for line in PLAY_LINES {
            assert!(seen.insert(line), "台詞池不該有重複句");
        }
    }

    #[test]
    fn play_line_pick_cycles_through_pool() {
        let n = PLAY_LINES.len();
        assert_eq!(play_line(0), play_line(n));
        assert_ne!(play_line(0), play_line(1));
    }
}
