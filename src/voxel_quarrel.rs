//! 乙太方界·居民偶爾小小拌嘴又和好 v1（ROADMAP 715）。
//!
//! **設計依據**：`docs/PLAN_ETHERVOX.md` §4 居民↔居民關係——
//! 「居民記得彼此、有關係（**熟識/幫過/吵過**）→ 友誼、合作、小圈子自己湧現＝小社會」。
//! 「熟識」（672 情誼帳本）、「幫過」（696 互助蓋家）都已上線，唯獨「吵過」這一格
//! 從未真的實作：情誼帳本一路只漲不跌、老朋友到訪不是溫馨問候（672）就是互助蓋家（696）
//! 或口耳相傳（694），關係永遠一片和樂——讀起來扁平，不像真實友誼會有的小摩擦。
//!
//! 本模組讓老朋友到訪時，偶爾為了雞毛蒜皮小事拌幾句嘴、隨即又和好如初——**不傷情誼**
//! （不扣拜訪次數、不降層級、不影響後續任何機率），只是讓關係第一次有了「有笑有鬧」的
//! 真實感，也讓日記（702/703）讀起來不只是單調的好感度數字上升。
//!
//! 純邏輯層：零 IO、零鎖、零 LLM、零 async；確定性純函式。
//! 鎖與副作用在 `voxel_ws.rs`（`bond_arrive_events` 迴圈內，短鎖即釋、不巢狀）。

use crate::voxel_bonds::BondTier;

/// Feed 播報種類名稱。
pub const FEED_KIND: &str = "居民拌嘴";

/// 每次「老朋友」到訪、且這次到訪沒有觸發互助蓋家（696）時，觸發小拌嘴的機率。
/// 刻意設低：拌嘴是偶爾出現的生活情趣，不是常態，多數到訪仍是溫馨問候。
pub const QUARREL_CHANCE: f32 = 0.12;

/// 拌嘴主題池——皆為雞毛蒜皮小事，貼合「療癒世界」基調，不涉及真正衝突。
const QUARREL_TOPICS: [&str; 6] = [
    "該誰澆花",
    "木頭該怎麼分才公平",
    "誰又忘記關門",
    "工具該放在哪裡",
    "誰把院子踩出一條新路",
    "誰吃掉了最後一顆胡蘿蔔",
];

/// 依 `pick` 確定性選一個拌嘴主題（循環取模）。
fn pick_topic(pick: usize) -> &'static str {
    QUARREL_TOPICS[pick % QUARREL_TOPICS.len()]
}

/// 判斷這次到訪是否要小拌嘴。
///
/// 只有「老朋友」夠熟才會拌嘴（陌生/相識還在客套，沒到能鬥嘴的交情）；
/// 且要求這次到訪沒有觸發互助蓋家（696）——同一次到訪只演一齣戲，避免
/// 「剛拌完嘴又立刻幫忙推進」讀起來精神分裂。
pub fn should_quarrel(tier: BondTier, help_happened: bool, roll: f32) -> bool {
    tier == BondTier::Friend && !help_happened && roll < QUARREL_CHANCE
}

/// 拌嘴＋和好的完整台詞（單句演完一齣小插曲，冒在訪客頭頂）。
pub fn quarrel_say_line(host: &str, pick: usize) -> String {
    let topic = pick_topic(pick);
    format!("「{topic}啦！」跟{host}為了這事拌了幾句嘴，不過轉眼又笑了出來，還是老朋友。")
}

/// Feed 播報文字（第三人稱，讓沒在場的玩家也知道世界有這麼一齣）。
pub fn quarrel_feed_line(visitor: &str, host: &str, pick: usize) -> String {
    let topic = pick_topic(pick);
    format!("{visitor}跟{host}為了{topic}小小拌了嘴，不過很快就和好如初！")
}

/// 拌嘴進訪客記憶的摘要文字（訪客視角）。
pub fn quarrel_memory_line_visitor(host: &str, pick: usize) -> String {
    let topic = pick_topic(pick);
    format!("跟{host}為了{topic}拌了幾句嘴，後來還是和好了")
}

/// 拌嘴進主人記憶的摘要文字（主人視角）。
pub fn quarrel_memory_line_host(visitor: &str, pick: usize) -> String {
    let topic = pick_topic(pick);
    format!("跟{visitor}為了{topic}拌了幾句嘴，後來還是和好了")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_quarrel_only_for_friend_tier() {
        assert!(!should_quarrel(BondTier::Stranger, false, 0.0));
        assert!(!should_quarrel(BondTier::Acquaintance, false, 0.0));
        assert!(should_quarrel(BondTier::Friend, false, 0.0));
    }

    #[test]
    fn should_quarrel_never_when_help_already_happened() {
        // 互助蓋家（696）已觸發時，本次到訪不再演拌嘴，避免同一次到訪演兩齣戲。
        assert!(!should_quarrel(BondTier::Friend, true, 0.0));
    }

    #[test]
    fn should_quarrel_respects_chance_threshold() {
        assert!(should_quarrel(BondTier::Friend, false, QUARREL_CHANCE - 0.001));
        assert!(!should_quarrel(BondTier::Friend, false, QUARREL_CHANCE));
        assert!(!should_quarrel(BondTier::Friend, false, 0.99));
    }

    #[test]
    fn pick_topic_wraps_around_deterministically() {
        let n = QUARREL_TOPICS.len();
        assert_eq!(pick_topic(0), pick_topic(n));
        assert_eq!(pick_topic(1), pick_topic(n + 1));
    }

    #[test]
    fn pick_topic_covers_all_entries_across_first_cycle() {
        let n = QUARREL_TOPICS.len();
        let seen: std::collections::HashSet<&str> = (0..n).map(pick_topic).collect();
        assert_eq!(seen.len(), n);
    }

    #[test]
    fn quarrel_say_line_mentions_host_and_reconciliation() {
        let line = quarrel_say_line("諾娃", 0);
        assert!(line.contains('諾'));
        assert!(line.contains("和好") || line.contains("老朋友"));
    }

    #[test]
    fn quarrel_say_line_varies_with_pick() {
        let a = quarrel_say_line("諾娃", 0);
        let b = quarrel_say_line("諾娃", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn quarrel_feed_line_mentions_both_names() {
        let line = quarrel_feed_line("露娜", "諾娃", 2);
        assert!(line.contains("露娜"));
        assert!(line.contains("諾娃"));
    }

    #[test]
    fn quarrel_memory_line_visitor_mentions_host() {
        let line = quarrel_memory_line_visitor("諾娃", 3);
        assert!(line.contains("諾娃"));
        assert!(line.contains("和好"));
    }

    #[test]
    fn quarrel_memory_line_host_mentions_visitor() {
        let line = quarrel_memory_line_host("露娜", 3);
        assert!(line.contains("露娜"));
        assert!(line.contains("和好"));
    }

    #[test]
    fn quarrel_memory_lines_are_not_empty_for_all_topics() {
        for pick in 0..QUARREL_TOPICS.len() {
            assert!(!quarrel_memory_line_visitor("露娜", pick).is_empty());
            assert!(!quarrel_memory_line_host("露娜", pick).is_empty());
        }
    }
}
