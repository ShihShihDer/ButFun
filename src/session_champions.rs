//! 廣場英雄碑——本次對話最佳旅人（ROADMAP 503）。
//!
//! 記憶體前置、重啟清零。每位玩家身上的 session_gather_count / session_harvest_count /
//! kill_count 在行動中累加；快照廣播時掃一遍所有在線玩家找各維度最高的那位。
//!
//! 純函式層：只吃 (&name, gather, harvest, kill) 元組，零 IO、零副作用，便於單元測試。
//! 與 `world_tally`（ROADMAP 495，全伺服器累計數字）定位互補：
//!   - world_tally ＝「這個世界今天做了多少」——全域無名累計；
//!   - session_champions ＝「是誰做了最多」——有名有姓、當下排名。

use serde::{Deserialize, Serialize};

/// 某維度本次對話的最佳旅人名字與計數。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SessionChampion {
    /// 旅人名字（依 PlayerView 的 name 欄位）。
    pub name: String,
    /// 計數（採集次數 / 收穫次數 / 擊殺次數）。
    pub count: u32,
}

/// 廣場英雄碑快照（ROADMAP 503）：本次對話採集 / 收穫 / 擊殺三維度各最多的旅人。
/// 若某維度所有在線玩家計數均為 0，對應欄位為 None（石碑上那欄留空）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionChampionsView {
    /// 本次對話採集最多的旅人。
    #[serde(default)]
    pub top_gather: Option<SessionChampion>,
    /// 本次對話收穫最多的旅人。
    #[serde(default)]
    pub top_harvest: Option<SessionChampion>,
    /// 本次對話擊殺最多的旅人。
    #[serde(default)]
    pub top_kill: Option<SessionChampion>,
}

/// 從玩家列表計算英雄碑快照。
///
/// 每個元素為 `(name, gather_count, harvest_count, kill_count)`。
/// 計數為 0 的玩家不上榜（避免全零列刷滿石碑）。同分時先出現的勝出（穩定性）。
/// 純函式、確定性，零 IO 副作用。
pub fn compute_champions<'a>(
    players: impl IntoIterator<Item = (&'a str, u32, u32, u32)>,
) -> SessionChampionsView {
    let mut best_gather: Option<(&'a str, u32)> = None;
    let mut best_harvest: Option<(&'a str, u32)> = None;
    let mut best_kill: Option<(&'a str, u32)> = None;

    for (name, gather, harvest, kill) in players {
        if gather > 0 && best_gather.map_or(true, |(_, c)| gather > c) {
            best_gather = Some((name, gather));
        }
        if harvest > 0 && best_harvest.map_or(true, |(_, c)| harvest > c) {
            best_harvest = Some((name, harvest));
        }
        if kill > 0 && best_kill.map_or(true, |(_, c)| kill > c) {
            best_kill = Some((name, kill));
        }
    }

    SessionChampionsView {
        top_gather: best_gather.map(|(n, c)| SessionChampion { name: n.to_string(), count: c }),
        top_harvest: best_harvest.map(|(n, c)| SessionChampion { name: n.to_string(), count: c }),
        top_kill: best_kill.map(|(n, c)| SessionChampion { name: n.to_string(), count: c }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list_returns_all_none() {
        let v = compute_champions(std::iter::empty());
        assert!(v.top_gather.is_none());
        assert!(v.top_harvest.is_none());
        assert!(v.top_kill.is_none());
    }

    #[test]
    fn all_zero_counts_returns_none() {
        let v = compute_champions([("旅人甲", 0u32, 0u32, 0u32)]);
        assert!(v.top_gather.is_none());
        assert!(v.top_harvest.is_none());
        assert!(v.top_kill.is_none());
    }

    #[test]
    fn single_player_nonzero_appears_on_board() {
        let v = compute_champions([("旅人甲", 5, 3, 2)]);
        assert_eq!(v.top_gather.as_ref().map(|c| c.name.as_str()), Some("旅人甲"));
        assert_eq!(v.top_gather.as_ref().map(|c| c.count), Some(5));
        assert_eq!(v.top_harvest.as_ref().map(|c| c.count), Some(3));
        assert_eq!(v.top_kill.as_ref().map(|c| c.count), Some(2));
    }

    #[test]
    fn highest_count_wins_each_category() {
        let players = [
            ("旅人甲", 10, 2, 5),
            ("旅人乙", 3, 8, 1),
            ("旅人丙", 7, 5, 9),
        ];
        let v = compute_champions(players);
        assert_eq!(v.top_gather.as_ref().map(|c| c.name.as_str()), Some("旅人甲")); // 10 最多
        assert_eq!(v.top_harvest.as_ref().map(|c| c.name.as_str()), Some("旅人乙")); // 8 最多
        assert_eq!(v.top_kill.as_ref().map(|c| c.name.as_str()), Some("旅人丙")); // 9 最多
    }

    #[test]
    fn zero_count_player_excluded_from_category() {
        // 旅人甲採集 0 不上采集榜，旅人乙採集 3 上榜
        let players = [("旅人甲", 0, 5, 1), ("旅人乙", 3, 2, 0)];
        let v = compute_champions(players);
        assert_eq!(v.top_gather.as_ref().map(|c| c.name.as_str()), Some("旅人乙"));
        assert_eq!(v.top_harvest.as_ref().map(|c| c.name.as_str()), Some("旅人甲")); // 5 > 2
        // 擊殺：甲 1 > 乙 0
        assert_eq!(v.top_kill.as_ref().map(|c| c.count), Some(1));
        assert_eq!(v.top_kill.as_ref().map(|c| c.name.as_str()), Some("旅人甲"));
    }

    #[test]
    fn first_player_wins_on_tie() {
        // 同分時先出現者勝（穩定性）
        let players = [("旅人甲", 5, 5, 5), ("旅人乙", 5, 5, 5)];
        let v = compute_champions(players);
        assert_eq!(v.top_gather.as_ref().map(|c| c.name.as_str()), Some("旅人甲"));
    }
}
