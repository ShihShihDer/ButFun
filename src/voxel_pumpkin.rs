//! 乙太方界·季限作物·秋南瓜 v1（ROADMAP 933）——農業第一次有「只在對的季節種得起來」的作物。
//!
//! 時令作物（811）／時令豐收（812）已讓既有三作物「種在時令長得快、收在時令收得多」——但
//! 不論哪一季，小麥／胡蘿蔔／馬鈴薯**四季都能種**，季節只調快慢與收量。本刀補上農業與季節連動的
//! 另一維度：一種**季限**作物——🎃 **秋南瓜只在秋天種得起來**。你看過整片天地換上秋色，如今那份
//! 秋意成了一扇會開會關的窗：秋天一到，野花裡才藏得住南瓜種子、田裡才種得活南瓜；錯過這一季，
//! 只能等下一個秋天。應季的期待與「趁著這個秋天多種幾畦」的驚喜，第一次真的落進玩法裡。
//!
//! **療癒優先、只擋不罰**（守資料安全鐵律）：非秋天種南瓜只是**溫柔地種不起來**（回一句
//! 「南瓜只在秋天種得活」的提示、退還種子、不扣任何資料），不是懲罰、不枯萎、不損玩家存檔。
//! 一旦在秋天種下，它就是一株普通的作物——即便種下後季節流轉到冬天，它照長照收（不會中途枯死），
//! 只是身為秋作物，它的**時令**本就是秋天，自然也吃得到 811 的種植 head-start 與 812 的收成豐收。
//!
//! **與既有時令機制 razor-sharp 區隔**：811/812 管的是「**四季都能種**的作物在時令時**更划算**」；
//! 本刀管的是「一種作物**只有一季種得起來**」——前者調數值、後者開關「能不能種」。兩者正交、
//! 各自獨立，一株秋天種下的南瓜同時吃得到本刀的季限與 811/812 的時令加成。
//!
//! **純邏輯層**：確定性、零 LLM、零持久化、零 migration、可窮舉測試；季節取得、種植季限攔截、
//! 回饋廣播、方塊/物品接線全在 `voxel_ws.rs`（比照胡蘿蔔／馬鈴薯的種植 handler 慣例）。

use crate::voxel_season::Season;

/// 南瓜只在哪一季種得起來（季限）。取秋天——呼應「秋收的沉甸甸果實」意象，也與 811 裡
/// 馬鈴薯同屬秋令的設定不衝突（811 是「四季都能種、秋天更划算」，本刀是「只有秋天種得起來」）。
pub const PLANTABLE_SEASON: Season = Season::Autumn;

/// 此刻能不能種南瓜：唯有當前季節正是 [`PLANTABLE_SEASON`]（秋天）才可種。
///
/// 確定性純函式、可窮舉四季測試。非秋天回 `false`（呼叫端據此溫柔攔截、退還種子、不損資料）。
pub fn can_plant(season: Season) -> bool {
    season == PLANTABLE_SEASON
}

/// 非當季嘗試種南瓜時，回給玩家的溫柔提示句（只擋不罰、附上「該在哪一季種」的指引）。
///
/// 僅在 [`can_plant`] 為 `false` 時呼叫。確定性、面向玩家、i18n 友善（字串集中此處）。
pub fn out_of_season_hint(current: Season) -> String {
    format!(
        "🎃 南瓜只在{}種得活呀，現在是{}，等秋天再來種吧。",
        PLANTABLE_SEASON.display_name(),
        current.display_name(),
    )
}

/// 當季（秋天）成功種下南瓜時，回給玩家的暖回饋句（確定性、面向玩家、i18n 友善）。
///
/// 僅在 [`can_plant`] 為 `true`、且真的種下南瓜時呼叫。
pub fn planted_line() -> String {
    format!(
        "🎃 趁著{}種下了南瓜——慢慢長，等它熟了會是好大一顆。",
        PLANTABLE_SEASON.display_name(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只有秋天種得起來；其餘三季一律種不起來。
    #[test]
    fn only_autumn_is_plantable() {
        assert!(can_plant(Season::Autumn), "秋天應可種南瓜");
        for s in [Season::Spring, Season::Summer, Season::Winter] {
            assert!(!can_plant(s), "{s:?} 不該種得起南瓜");
        }
    }

    /// 四季窮舉：恰好只有一季（秋天）可種。
    #[test]
    fn exactly_one_plantable_season() {
        let all = [Season::Spring, Season::Summer, Season::Autumn, Season::Winter];
        let plantable: Vec<Season> = all.into_iter().filter(|&s| can_plant(s)).collect();
        assert_eq!(plantable.len(), 1, "應恰好只有一季可種南瓜");
        assert_eq!(plantable[0], PLANTABLE_SEASON);
        assert_eq!(PLANTABLE_SEASON, Season::Autumn);
    }

    /// 非當季提示：非空、嵌得到「秋天」與當前季節名。
    #[test]
    fn out_of_season_hint_content() {
        let hint = out_of_season_hint(Season::Spring);
        assert!(!hint.is_empty());
        assert!(hint.contains("秋天"), "提示應點出該在秋天種");
        assert!(hint.contains("春天"), "提示應點出目前是春天");
        // 其餘非當季也各自嵌對當前季節名。
        assert!(out_of_season_hint(Season::Summer).contains("夏天"));
        assert!(out_of_season_hint(Season::Winter).contains("冬天"));
    }

    /// 成功種下的暖回饋：非空、點出秋天。
    #[test]
    fn planted_line_content() {
        let line = planted_line();
        assert!(!line.is_empty());
        assert!(line.contains("秋天"));
        assert!(line.contains("南瓜"));
    }
}
