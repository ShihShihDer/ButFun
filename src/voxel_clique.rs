//! 乙太方界·居民小圈子聚會 v1（ROADMAP 711）——三位以上互為老朋友的居民相約碰面。
//!
//! **設計依據**：`docs/PLAN_ETHERVOX.md` §4 居民↔居民關係——「小圈子自己湧現＝小社會」。
//! 情誼（672）至今只停在「兩兩之間」的數字，居民交情網（708）讓玩家看得見這份數字，
//! 但世界從沒真的演出過「這幾位真的是一夥的」這個畫面。本模組偵測互為老朋友
//! （`BondTier::Friend`）的小圈子，讓他們不時相約在其中一位的家域碰面小聚——
//! 玩家第一次能親眼撞見「一群人聚在一起」，而不只是查表得知交情深淺。
//!
//! **設計取捨**：只挑「彼此皆為老朋友」的最大圈子（不重複回傳其子集合），
//! 避免同一群人被判定成多個重疊的小聚會。相聚地點固定用圈子裡 id 最小的
//! 那位居民家域（`session_tag` 同時做為辨識鍵——同一時刻只會有一組聚會在跑，
//! 用不到 tag 衝突處理，但仍保留字串鍵設計以利未來擴充多組並行）。
//!
//! 純邏輯層：零 IO、零鎖、零 LLM、零 async；確定性可測。
//! 鎖 / 移動 / 觸發掃描 / 抵達判定全在 `voxel_ws.rs`（短鎖即釋、不巢狀）。

use crate::voxel_bonds::BondTier;

/// 每 agency tick，圈子成員皆閒（無其他任務）+ 冷卻到期時，觸發一次聚會的機率。
/// 與探訪（`VISIT_CHANCE`=0.008）同量級：稀少才有感，不淪為背景雜訊。
pub const GATHER_CHANCE: f32 = 0.01;
/// 聚會冷卻（秒）：一次聚會結束（或放棄）後，圈子成員要等這麼久才可能再被選中。
pub const GATHER_COOLDOWN_SECS: f32 = 300.0;
/// 抵達聚會點的判定半徑（方塊，XZ 平面），與探訪 `VISIT_ARRIVE_DIST` 同量級。
pub const GATHER_ARRIVE_DIST: f32 = 5.0;
/// 抵達後在聚會點附近閒晃的半徑（比探訪小，讓一群人看起來聚在一塊，不散開）。
pub const GATHER_WANDER_RADIUS: f32 = 4.0;
/// 單一成員從被指派聚會起最多等待秒數：超過仍等不到其他成員到齊 → 放棄、各自散去
/// （防某成員被地形卡住，導致整組永遠等不到、卡在半聚會狀態）。
pub const GATHER_MAX_WAIT_SECS: f32 = 90.0;
/// 聚會事件的 Feed 分類。
pub const FEED_KIND: &str = "小圈子聚會";

/// 把一組居民 id 轉成穩定、確定性的聚會識別鍵（由小到大排序後以 `+` 串接）。
/// 同一組成員無論傳入順序為何，都會得到相同的 tag——供呼叫端辨識「這是同一場聚會」。
pub fn session_tag(members: &[String]) -> String {
    let mut sorted = members.to_vec();
    sorted.sort();
    sorted.join("+")
}

/// 從一組居民 id 中，找出所有「彼此皆為老朋友」的**最大**小圈子（大小 ≥ 3）。
///
/// `ids`：全部居民 id（穩定順序，如 `["vox_res_0", ...]`）。
/// `tier_of`：查任兩位居民間情誼層級的函式（呼叫端從 `ResidentBonds` 快照提供）。
///
/// 只回傳極大團（不回傳已被更大圈子涵蓋的子集合），避免同一群人被算成多場聚會。
/// 居民數極少（現行 4 位）用窮舉即可，零效能疑慮。
pub fn find_friend_cliques(
    ids: &[String],
    tier_of: impl Fn(&str, &str) -> BondTier,
) -> Vec<Vec<String>> {
    let n = ids.len();
    if n < 3 {
        return Vec::new();
    }
    let mut cliques: Vec<Vec<usize>> = Vec::new();
    for mask in 1u32..(1u32 << n) {
        if (mask.count_ones() as usize) < 3 {
            continue;
        }
        let members: Vec<usize> = (0..n).filter(|i| mask & (1 << i) != 0).collect();
        let mut all_friend = true;
        'pairs: for a in 0..members.len() {
            for b in (a + 1)..members.len() {
                if tier_of(&ids[members[a]], &ids[members[b]]) != BondTier::Friend {
                    all_friend = false;
                    break 'pairs;
                }
            }
        }
        if all_friend {
            cliques.push(members);
        }
    }
    // 只留極大團：丟掉是其他已收錄圈子真子集合的那些。
    cliques.sort_by_key(|c| std::cmp::Reverse(c.len()));
    let mut kept: Vec<Vec<usize>> = Vec::new();
    for c in cliques {
        let is_subset = kept
            .iter()
            .any(|bigger| c.iter().all(|m| bigger.contains(m)));
        if !is_subset {
            kept.push(c);
        }
    }
    kept.into_iter()
        .map(|idxs| idxs.into_iter().map(|i| ids[i].clone()).collect())
        .collect()
}

/// 抵達聚會點時冒出的台詞（確定性選句，供聚在一起的居民各自冒泡）。
pub fn gather_line(pick: usize) -> &'static str {
    const LINES: &[&str] = &[
        "難得大家都在，坐下來說說話吧～",
        "好久沒這樣聚在一起了！",
        "這樣的時光，跟你們在一起真好。",
        "來，靠近一點，我有件事想跟大家說！",
    ];
    LINES[pick % LINES.len()]
}

/// 聚會後寫進參與者記憶的摘要（提及其他所有成員的名字）。
pub fn gather_memory_line(other_names: &[&str]) -> String {
    format!("和{}聚在一起，聊了好一會兒，心裡暖暖的", other_names.join("、"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids4() -> Vec<String> {
        vec![
            "vox_res_0".to_string(),
            "vox_res_1".to_string(),
            "vox_res_2".to_string(),
            "vox_res_3".to_string(),
        ]
    }

    #[test]
    fn session_tag_stable_regardless_of_order() {
        let a = vec!["vox_res_2".to_string(), "vox_res_0".to_string(), "vox_res_1".to_string()];
        let b = vec!["vox_res_1".to_string(), "vox_res_2".to_string(), "vox_res_0".to_string()];
        assert_eq!(session_tag(&a), session_tag(&b));
        assert_eq!(session_tag(&a), "vox_res_0+vox_res_1+vox_res_2");
    }

    #[test]
    fn no_clique_when_all_strangers() {
        let ids = ids4();
        let cliques = find_friend_cliques(&ids, |_, _| BondTier::Stranger);
        assert!(cliques.is_empty());
    }

    #[test]
    fn no_clique_when_fewer_than_three_residents() {
        let ids = vec!["vox_res_0".to_string(), "vox_res_1".to_string()];
        let cliques = find_friend_cliques(&ids, |_, _| BondTier::Friend);
        assert!(cliques.is_empty());
    }

    #[test]
    fn finds_trio_when_three_of_four_are_mutual_friends() {
        let ids = ids4();
        // 0,1,2 兩兩皆老朋友；3 跟任何人都只是相識。
        let cliques = find_friend_cliques(&ids, |a, b| {
            let trio = ["vox_res_0", "vox_res_1", "vox_res_2"];
            if trio.contains(&a) && trio.contains(&b) {
                BondTier::Friend
            } else {
                BondTier::Acquaintance
            }
        });
        assert_eq!(cliques.len(), 1);
        let mut c = cliques[0].clone();
        c.sort();
        assert_eq!(c, vec!["vox_res_0", "vox_res_1", "vox_res_2"]);
    }

    #[test]
    fn all_four_mutual_friends_returns_single_group_not_subsets() {
        let ids = ids4();
        let cliques = find_friend_cliques(&ids, |_, _| BondTier::Friend);
        assert_eq!(cliques.len(), 1);
        assert_eq!(cliques[0].len(), 4);
    }

    #[test]
    fn partial_friendship_yields_no_clique() {
        let ids = ids4();
        // 只有 0-1 是老朋友，其餘配對都不到——湊不出大小 >= 3 的圈子。
        let cliques = find_friend_cliques(&ids, |a, b| {
            if (a == "vox_res_0" && b == "vox_res_1") || (a == "vox_res_1" && b == "vox_res_0") {
                BondTier::Friend
            } else {
                BondTier::Stranger
            }
        });
        assert!(cliques.is_empty());
    }

    #[test]
    fn gather_line_nonempty_and_varies() {
        let l0 = gather_line(0);
        let l1 = gather_line(1);
        assert!(!l0.is_empty());
        assert!(!l1.is_empty());
        assert_ne!(l0, l1);
    }

    #[test]
    fn gather_line_cycles_deterministically() {
        assert_eq!(gather_line(0), gather_line(4));
    }

    #[test]
    fn gather_memory_line_mentions_all_others() {
        let line = gather_memory_line(&["露娜", "諾娃"]);
        assert!(line.contains("露娜"));
        assert!(line.contains("諾娃"));
    }
}
