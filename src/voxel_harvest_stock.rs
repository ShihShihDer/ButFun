//! 乙太方界·秋收囤糧過冬 v1（voxel-harvest-stock，自主提案切片）。
//!
//! **真缺口**：季節輪替（798）自誕生以來，秋天在整個世界裡只有兩種存在感——換季那一刻
//! 居民抬頭念一句「秋天到了…」（一次性），以及前端把天地染上一層暖色調。除此之外，
//! 「秋天」對居民的**行為**至今毫無影響；冬天已經有了飄雪（900）、圍爐取暖（901）兩套
//! 因季節而改變的行為，秋天卻始終是四季裡唯一「只換了顏色、居民照樣過」的空白季節。
//! 另一頭，居民回饋糧倉（`voxel_chest_contribute`，居民往你用過的箱子存餘料）一年四季存
//! 的節奏一模一樣——箱子系統從不知道「現在是不是收成的季節」。這兩套既有系統至今從沒有
//! 一行程式碼讓對方知道對方存在。
//!
//! **本刀補上**：把「秋天＝收成的好時節」接上「居民回饋糧倉」——**入秋後，居民為過冬
//! 囤糧的意識醒來，往箱子存料更勤、台詞也從泛泛的『順手存一份』變成惦記著寒冬的『多囤點
//! 過冬』**；而**每個秋天第一次真的有人囤糧的那一刻**，城鎮動態牆留下一則「秋收到了，
//! 居民開始為過冬囤糧」的季節時刻。秋天第一次不只是換了顏色，而是**看得見居民因為
//! 『要過冬了』而改變了行為**，把秋收（798）與冬寒（901）在同一條敘事上前後扣起來。
//!
//! **與既有系統的分界（換維度·非同軸重複）**：
//! - 不是 900 飄雪 / 901 圍爐（冬天的視覺與取暖行為）——本刀是**秋天**的**囤糧**行為，
//!   季節與行為都不同，兩者在敘事上前後相承（秋天囤、冬天取暖）而非重複。
//! - 不是換季反應（798，換季那一刻念一句、之後整季再無下文）——本刀是**整個秋天**持續
//!   影響居民往箱子存料的節奏與台詞，是換季那一句話之後、居民「真的動起來」的後續。
//! - 複用居民回饋糧倉同一套「挑餘料→找箱子→存進去」的管線與鎖序，只在**機率**與
//!   **台詞／動態牆文案**上按季節分岔，不另開一條存料路徑。
//!
//! **純邏輯層**：本模組只有確定性純函式（秋季存料機率、囤糧台詞／動態牆、初囤季節時刻
//! 追蹤器），零 LLM、零鎖、零 IO、零 async。實際扣背包／存箱／持久化／廣播全留在
//! `voxel_ws.rs`（沿用回饋糧倉既有短鎖循序管線，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：台詞全為固定模板、只嵌居民系統顯示名與物品名（本就出現在動態牆），
//! 不夾帶玩家輸入、不回放記憶原文（無注入／NSFW 面）；存不存純由伺服器內部（採集背包餘量
//! ＋箱子位置＋季節＋機率＋既有長冷卻）決定，玩家無法自報或催發；秋季只是把既有機率**調高
//! 一檔**、乘完仍走回饋糧倉既有的每人長冷卻，天然防洗版；零 migration（囤糧追蹤器純記憶體、
//! 重啟歸零，比照飄雪／季節等世界暫態）、零協議破壞（不動任何 WS/HTTP 欄位）、零新美術、
//! 零前端改動。

use crate::voxel_season::Season;

/// 秋季囤糧的機率加成倍率：秋天＝收成季，居民為過冬存料更積極，把每 tick 觸發機率乘上
/// 這個倍率。刻意不設太高——存料仍走回饋糧倉既有的每人長冷卻，加成只讓「入秋後存得更勤」
/// 有感，而非把箱子塞爆洗版。非秋季一律回原機率、行為與過去完全一致。
pub const AUTUMN_CHANCE_MULTIPLIER: f32 = 2.5;

/// 依季節回傳這一 tick 的存料機率：秋天乘上 [`AUTUMN_CHANCE_MULTIPLIER`]（夾在 `[0,1]`），
/// 其餘季節原封回傳 `base`（不改變任何非秋季行為）。純函式、可測。
pub fn contribute_chance(season: Season, base: f32) -> f32 {
    if season == Season::Autumn {
        (base * AUTUMN_CHANCE_MULTIPLIER).clamp(0.0, 1.0)
    } else {
        base
    }
}

/// 動態牆分類鍵（面向玩家、集中此處統一替換，i18n 友善）。
pub const FEED_KIND: &str = "秋收囤糧";

/// 每個秋天第一次真的有人囤糧那一刻的城鎮動態牆時刻（一個秋天只播一次）。
pub const FIRST_STOCKPILE_FEED: &str = "秋收到了，居民開始為過冬囤糧，村裡的箱子一天天豐盛起來。";

/// 泡泡字元上限（與回饋糧倉泡泡框上限一致，超出截斷不破框）。
pub const STOCKPILE_SAY_MAX_CHARS: usize = 40;

/// 截斷輔助：保留至多 [`STOCKPILE_SAY_MAX_CHARS`] 個字元（依字元非位元組，繁中安全）。
fn truncate_chars(s: &str) -> String {
    s.chars().take(STOCKPILE_SAY_MAX_CHARS).collect()
}

/// 秋季囤糧的泡泡台詞——與泛用版（「順手存一份」）刻意不重疊、句句點出「過冬」的心思，
/// 依 `pick` 在幾組模板間確定性輪替。整句以字元截到上限內，永不破框。
pub fn stockpile_bubble(item_name: &str, qty: u32, pick: usize) -> String {
    const T: [&str; 3] = [
        "秋收了，趁現在多囤{q}份{i}進箱子，好過冬。",
        "天涼了，存{q}份{i}備著——冬天可就靠這些了。",
        "趁收成的時節，把{q}份{i}存進箱子囤著過冬。",
    ];
    let line = T[pick % T.len()]
        .replace("{i}", item_name)
        .replace("{q}", &qty.to_string());
    truncate_chars(&line)
}

/// 秋季囤糧的動態牆播報（面向玩家、不含記憶原文，純模板拼裝）。
pub fn stockpile_feed_line(resident_name: &str, item_name: &str, qty: u32) -> String {
    format!("{resident_name}為過冬把{qty}份{item_name}囤進了村裡的箱子")
}

/// 「本秋是否已播過初囤時刻」的追蹤器（純記憶體、確定性、可測）。
/// 離開秋天即重置，讓下一個秋天能再播一次；比照飄雪初雪追蹤器的季內一次性慣例。
#[derive(Default)]
pub struct AutumnStockTracker {
    announced: bool,
}

impl AutumnStockTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 每 tick 餵入「當前是否秋天」：一離開秋天就把旗標歸零，下一個秋天可重新播初囤時刻。
    pub fn sync_season(&mut self, is_autumn: bool) {
        if !is_autumn {
            self.announced = false;
        }
    }

    /// 秋天內第一次真的有人囤糧時回 `true`（同一個秋天只會回一次 `true`）；
    /// 呼叫端只在「秋天 ∧ 真的成功囤了一次」時呼叫。
    pub fn claim_first(&mut self) -> bool {
        if self.announced {
            false
        } else {
            self.announced = true;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: f32 = 0.02;

    #[test]
    fn autumn_boosts_chance_others_unchanged() {
        // 秋天：乘上倍率。
        assert!((contribute_chance(Season::Autumn, BASE) - BASE * AUTUMN_CHANCE_MULTIPLIER).abs() < 1e-6);
        // 其餘三季：原封不動，行為與過去完全一致。
        for s in [Season::Spring, Season::Summer, Season::Winter] {
            assert!((contribute_chance(s, BASE) - BASE).abs() < 1e-6, "{s:?} 應回原機率");
        }
    }

    #[test]
    fn chance_clamped_to_unit_interval() {
        // 高基準乘完超過 1 → 夾回 1，永不溢出成非法機率。
        assert_eq!(contribute_chance(Season::Autumn, 0.9), 1.0);
        // 基準 0 → 秋天也還是 0（沒有餘料時不會憑空多存）。
        assert_eq!(contribute_chance(Season::Autumn, 0.0), 0.0);
    }

    #[test]
    fn stockpile_bubble_rotates_and_fills() {
        let a = stockpile_bubble("胡蘿蔔", 3, 0);
        let b = stockpile_bubble("胡蘿蔔", 3, 1);
        let c = stockpile_bubble("胡蘿蔔", 3, 2);
        // 三句互異、確定性輪替。
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
        // pick 取模有界：第 3 句與第 0 句相同。
        assert_eq!(stockpile_bubble("胡蘿蔔", 3, 3), a);
        for s in [&a, &b, &c] {
            assert!(!s.is_empty());
            // 佔位符全部替換乾淨。
            assert!(!s.contains("{i}") && !s.contains("{q}"));
            assert!(s.contains("胡蘿蔔") && s.contains('3'));
            // 秋收版必談過冬，與泛用版明確區隔。
            assert!(s.contains("冬"));
        }
    }

    #[test]
    fn stockpile_bubble_truncates_long_item_without_panic() {
        // 超長物品名不 panic、不破框（依字元截、多位元組安全）。
        let long = "超級無敵霹靂宇宙究極豐收大蘿蔔王".repeat(5);
        let out = stockpile_bubble(&long, 99, 7);
        assert!(out.chars().count() <= STOCKPILE_SAY_MAX_CHARS);
    }

    #[test]
    fn feed_line_has_name_qty_item() {
        let line = stockpile_feed_line("露娜", "小麥", 5);
        assert!(line.contains("露娜") && line.contains("小麥") && line.contains('5'));
        assert!(line.contains("過冬"));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn tracker_announces_once_per_autumn() {
        let mut t = AutumnStockTracker::new();
        // 秋天內第一次囤糧 → true，之後同一個秋天再囤 → false。
        assert!(t.claim_first());
        assert!(!t.claim_first());
        // 還在秋天（sync 餵 true）不重置。
        t.sync_season(true);
        assert!(!t.claim_first());
        // 離開秋天（sync 餵 false）重置 → 下一個秋天可再播一次。
        t.sync_season(false);
        assert!(t.claim_first());
        assert!(!t.claim_first());
    }
}
