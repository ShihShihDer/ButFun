//! 街頭演奏者的個人曲目精進（ROADMAP 535）——把 399 廣場獻奏的「資歷數字」
//! 長成一條看得見的個人藝途：你獻奏的場次越多，頭頂飄出的音符就越華麗，
//! 並一階階晉升曲目身段（街頭新手 → 廣場樂手 → 城鎮名伶 → 傳奇吟遊）。
//!
//! 設計鐵律（刻意與既有 busking 系統乾淨分工、避開最近連發的骨架）：
//! - **與 399 獻奏（即時打賞）、472 合奏（群眾療癒）刻意分維度**：
//!   399 是「單場完成得打賞」、472 是「多人靠近湊成樂團療癒圍聽者」，
//!   本切片是「**單一演奏者跨多場累積的個人表現層精進**」——純粹自我表達的成長，
//!   不發貨幣、不改打賞、不碰療癒速率、不 gate 任何戰力（刻意有別於熟練度系吃加成的骨架）。
//! - **純邏輯可測**：`tier_for_count`／`is_tier_up`／`note_symbol` 皆純函式、無副作用、無 IO、零鎖。
//! - **零持久化、零 migration**：階段由 Player 既有的記憶體前置欄位 `busk_count` 當下推導，不入存檔。
//! - **零 LLM、零外部呼叫、零經濟擾動**：階段只決定「飄哪些音符」與「身段名」，與平衡無關。
//! - **玩家一眼有感**：階段（`busk_tier`）放進快照廣播，旁觀者看見高階演奏者飄出更豐富的音符；
//!   晉升的當下，本人收到一則「晉升為 ◯◯」的暖訊。面向玩家的身段名留在前端集中可在地化。

/// 街頭演奏者的曲目身段。值即 wire 值（穩定契約，前端據此選音符調色盤與身段名，別重排）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepertoireTier {
    /// 街頭新手——剛開始獻奏（0~4 場）。
    Novice = 0,
    /// 廣場樂手——小有資歷（5~14 場）。
    Player = 1,
    /// 城鎮名伶——廣場常客（15~39 場）。
    Diva = 2,
    /// 傳奇吟遊——獻奏成痴（40 場以上）。
    Bard = 3,
}

/// 晉升到下一身段所需的累計獻奏場次門檻（由低到高）。
/// 索引對齊 `RepertoireTier` 的 wire 值：到達 `TIER_THRESHOLDS[t]` 場即進入身段 `t`。
/// Novice 門檻為 0（人人起步即是）；其後 5 / 15 / 40。
const TIER_THRESHOLDS: [u32; 4] = [0, 5, 15, 40];

impl RepertoireTier {
    /// wire 值（0~3）。前端據此挑音符調色盤與身段名。
    #[inline]
    pub fn wire(self) -> u8 {
        self as u8
    }

    /// 此身段頭頂可飄的音符調色盤（越高階越華麗）。集中於此便於一致性與測試；
    /// 前端另有對應調色盤畫圖，wire 值是兩端共用的穩定契約。
    pub fn palette(self) -> &'static [&'static str] {
        match self {
            RepertoireTier::Novice => &["🎵"],
            RepertoireTier::Player => &["🎵", "🎶"],
            RepertoireTier::Diva => &["🎵", "🎶", "🎼"],
            RepertoireTier::Bard => &["🎵", "🎶", "🎼", "🎷"],
        }
    }
}

/// 依累計獻奏場次推得當下身段（純函式、單調不減）。
pub fn tier_for_count(busk_count: u32) -> RepertoireTier {
    if busk_count >= TIER_THRESHOLDS[3] {
        RepertoireTier::Bard
    } else if busk_count >= TIER_THRESHOLDS[2] {
        RepertoireTier::Diva
    } else if busk_count >= TIER_THRESHOLDS[1] {
        RepertoireTier::Player
    } else {
        RepertoireTier::Novice
    }
}

/// 一場獻奏完成、場次由 `prev_count` 增為 `new_count` 時，是否恰好跨入新身段。
/// 回傳 `Some(新身段)` 表示這一場讓玩家晉升（呼叫端據此送一則暖訊）；否則 `None`。
/// 防呆：`new_count <= prev_count`（理應不發生）一律回 `None`，不誤報晉升。
pub fn is_tier_up(prev_count: u32, new_count: u32) -> Option<RepertoireTier> {
    if new_count <= prev_count {
        return None;
    }
    let before = tier_for_count(prev_count);
    let after = tier_for_count(new_count);
    if after > before {
        Some(after)
    } else {
        None
    }
}

/// 依身段與種子挑一個音符符號（前端飄字用，集中於此便於 i18n／一致性與測試）。
/// 高階身段的調色盤更豐富，同一 seed 在不同身段可能飄出不同音符。
pub fn note_symbol(tier: RepertoireTier, seed: u64) -> &'static str {
    let palette = tier.palette();
    palette[(seed % palette.len() as u64) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_thresholds_map_correctly() {
        assert_eq!(tier_for_count(0), RepertoireTier::Novice);
        assert_eq!(tier_for_count(4), RepertoireTier::Novice);
        assert_eq!(tier_for_count(5), RepertoireTier::Player);
        assert_eq!(tier_for_count(14), RepertoireTier::Player);
        assert_eq!(tier_for_count(15), RepertoireTier::Diva);
        assert_eq!(tier_for_count(39), RepertoireTier::Diva);
        assert_eq!(tier_for_count(40), RepertoireTier::Bard);
        assert_eq!(tier_for_count(9999), RepertoireTier::Bard);
    }

    #[test]
    fn wire_values_are_stable() {
        assert_eq!(RepertoireTier::Novice.wire(), 0);
        assert_eq!(RepertoireTier::Player.wire(), 1);
        assert_eq!(RepertoireTier::Diva.wire(), 2);
        assert_eq!(RepertoireTier::Bard.wire(), 3);
    }

    #[test]
    fn tier_is_monotonic() {
        // 場次只增不減 → 身段只升不降。
        let mut last = RepertoireTier::Novice;
        for n in 0..60u32 {
            let t = tier_for_count(n);
            assert!(t >= last, "身段在第 {n} 場時倒退了");
            last = t;
        }
    }

    #[test]
    fn tier_up_fires_exactly_on_crossing() {
        // 第 5 場（由 4→5）恰好晉升廣場樂手。
        assert_eq!(is_tier_up(4, 5), Some(RepertoireTier::Player));
        // 第 15 場晉升城鎮名伶。
        assert_eq!(is_tier_up(14, 15), Some(RepertoireTier::Diva));
        // 第 40 場晉升傳奇吟遊。
        assert_eq!(is_tier_up(39, 40), Some(RepertoireTier::Bard));
    }

    #[test]
    fn tier_up_silent_within_same_tier() {
        assert_eq!(is_tier_up(0, 1), None);
        assert_eq!(is_tier_up(5, 6), None);
        assert_eq!(is_tier_up(40, 41), None); // 已是頂階，不再晉升
    }

    #[test]
    fn tier_up_guards_against_non_increasing() {
        assert_eq!(is_tier_up(5, 5), None);
        assert_eq!(is_tier_up(10, 3), None);
    }

    #[test]
    fn note_palette_grows_with_tier() {
        assert_eq!(RepertoireTier::Novice.palette().len(), 1);
        assert_eq!(RepertoireTier::Player.palette().len(), 2);
        assert_eq!(RepertoireTier::Diva.palette().len(), 3);
        assert_eq!(RepertoireTier::Bard.palette().len(), 4);
    }

    #[test]
    fn note_symbol_cycles_within_tier_palette() {
        // 新手只飄單一音符。
        assert_eq!(note_symbol(RepertoireTier::Novice, 0), "🎵");
        assert_eq!(note_symbol(RepertoireTier::Novice, 7), "🎵");
        // 傳奇吟遊在四種音符間循環。
        assert_eq!(note_symbol(RepertoireTier::Bard, 0), "🎵");
        assert_eq!(note_symbol(RepertoireTier::Bard, 1), "🎶");
        assert_eq!(note_symbol(RepertoireTier::Bard, 2), "🎼");
        assert_eq!(note_symbol(RepertoireTier::Bard, 3), "🎷");
        assert_eq!(note_symbol(RepertoireTier::Bard, 4), "🎵"); // 循環
    }
}
