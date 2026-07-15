//! 乙太方界·孩童玩伴 v1（voxel-playmate，自主提案切片，接續 1016 孩子的模樣與玩耍時光；
//! reviewer 明令別再連發彙整清單→browse panel，本刀往「居民↔居民行為的新湧現」推進）。
//!
//! **真缺口**：1016 讓每個孩子會忍不住獨自玩起小把戲（追蝴蝶／繞圈圈／踢石子），但兩個孩子
//! 擦身而過時彼此毫無反應，像兩個各自跑圖的 NPC——村裡從沒有任何一刻讓玩家看得出「這兩個孩子
//! 是玩伴」。本刀給孩童玩耍加上第一個**對象**：**兩個此刻都在閒晃、都是孩子的居民靠得夠近時，
//! 會一起玩同一場遊戲、各自的頭頂泡泡點名對方**——這是童年行為第一次從「獨處」延伸成
//! 「兩人共享」。
//!
//! **與既有系統 razor-sharp 區隔**：
//! - [`crate::voxel_childhood`] 獨處小把戲＝任一孩子單獨觸發，不涉及任何其他居民；本刀＝
//!   兩個孩子**同時**符合條件、彼此在範圍內才觸發，觸發後兩人各自的 `say` 都點名對方——
//!   playmate 關係純粹因「同時在場＋都是孩子」自然結成，不靠血緣（`sibling.rs`）也不靠
//!   先攢交情（`clique.rs` 的老朋友聚會），是孩子獨有、正交於既有 bonds 系統的新關係起點。
//! - 沿用 [`vchild::PLAY_PREFIX`] 前綴契約（頭頂泡泡以 ☆ 起頭 → 前端播放活潑跳動姿態），
//!   零新協議欄位、零前端改動。
//!
//! **成本鐵律**：純規則式配對（貪婪、依 id 排序求確定可重現，鏡像 `pet_play::detect`／
//! `high_five::match_pairs` 手法）＋純規則式挑句，零 LLM、零 IO、零鎖。**不新增任何持久化**——
//! 是否玩在一起、跟誰玩，純由伺服器內部座標與年齡當下決定；不寫記憶、不上動態牆（比照
//! 1016 v1 純氛圍範式，記憶/牆的深化留給未來需要時再上）。
//! **濫用防護**：不收玩家輸入、不觸發 LLM、不開新端點——配對與觸發全由伺服器內部座標/計時
//! 決定性算出，玩家無從自報或催發。

use crate::voxel_childhood::PLAY_PREFIX;

/// 兩個孩子能湊起來玩的最大距離（世界格單位）——比集會鐘召集範圍小得多，要真的站在一塊，
/// 不是「同一條路上」就算。
pub const PLAYMATE_RANGE: f32 = 4.0;

/// 一筆「此刻是孩子、手邊沒正事」的快照，供配對用。
#[derive(Debug, Clone)]
pub struct PlaymateActor {
    pub id: String,
    pub name: &'static str,
    pub x: f32,
    pub z: f32,
}

/// 一對湊起來玩的孩子（`a`/`b` 依 id 排序求確定）。
#[derive(Debug, Clone, PartialEq)]
pub struct PlaymatePair {
    pub a_id: String,
    pub a_name: &'static str,
    pub b_id: String,
    pub b_name: &'static str,
}

/// 把一群「此刻可以玩耍的孩子」兩兩配成玩伴。
///
/// 規則（鏡像 `pet_play::detect` 的貪婪配對，求確定可重現）：
/// - 依 id 排序逐一替尚未入對者找「範圍內、最近」的另一個未入對者配成一對；每人至多入一對。
/// - 結果依 `(a_id, b_id)` 排序回傳——同一組輸入永遠得到同一組配對，可測。
pub fn find_playmates(actors: &[PlaymateActor]) -> Vec<PlaymatePair> {
    let mut idx: Vec<usize> = (0..actors.len()).collect();
    idx.sort_by(|&i, &j| actors[i].id.cmp(&actors[j].id));

    let mut taken = vec![false; actors.len()];
    let mut pairs: Vec<PlaymatePair> = Vec::new();

    for a_pos in 0..idx.len() {
        let ai = idx[a_pos];
        if taken[ai] {
            continue;
        }
        let a = &actors[ai];
        let mut best: Option<(usize, f32)> = None;
        for b_pos in (a_pos + 1)..idx.len() {
            let bi = idx[b_pos];
            if taken[bi] {
                continue;
            }
            let b = &actors[bi];
            let dx = a.x - b.x;
            let dz = a.z - b.z;
            let dist = (dx * dx + dz * dz).sqrt();
            if dist <= PLAYMATE_RANGE {
                if best.map_or(true, |(_, bd)| dist < bd) {
                    best = Some((bi, dist));
                }
            }
        }
        if let Some((bi, _)) = best {
            taken[ai] = true;
            taken[bi] = true;
            let (a_id, b_id) = if actors[ai].id <= actors[bi].id {
                (ai, bi)
            } else {
                (bi, ai)
            };
            pairs.push(PlaymatePair {
                a_id: actors[a_id].id.clone(),
                a_name: actors[a_id].name,
                b_id: actors[b_id].id.clone(),
                b_name: actors[b_id].name,
            });
        }
    }

    pairs.sort_by(|p, q| (p.a_id.as_str(), p.b_id.as_str()).cmp(&(q.a_id.as_str(), q.b_id.as_str())));
    pairs
}

/// 一起玩的台詞池——兩人共享同一場小遊戲，各自的泡泡點名對方。`{}` 由呼叫端填入對方名字。
const PLAYMATE_LINES: [&str; 6] = [
    "拉著{}的手一起追同一隻蝴蝶，兩人笑成一團！",
    "跟{}手拉手轉起圈圈，轉到兩人都站不穩、笑倒在地上。",
    "跟{}輪流踢著同一顆小石子，繞了大半個院子。",
    "跟{}比賽跳格子，兩人都不肯認輸。",
    "跟{}一起蹲在小花旁看了好久，時不時偷笑對看一眼。",
    "跟{}玩起躲貓貓，數到十就到處找對方。",
];

/// 依 `pick` 確定性選一句共玩台詞，填入對方名字，以 [`PLAY_PREFIX`] 起頭。
pub fn playmate_line(partner_name: &str, pick: usize) -> String {
    format!(
        "{PLAY_PREFIX}{}",
        PLAYMATE_LINES[pick % PLAYMATE_LINES.len()].replace("{}", partner_name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(id: &str, name: &'static str, x: f32, z: f32) -> PlaymateActor {
        PlaymateActor { id: id.to_string(), name, x, z }
    }

    #[test]
    fn empty_input_yields_no_pairs() {
        assert!(find_playmates(&[]).is_empty());
    }

    #[test]
    fn single_child_yields_no_pairs() {
        let actors = vec![actor("a", "露娜", 0.0, 0.0)];
        assert!(find_playmates(&actors).is_empty());
    }

    #[test]
    fn two_children_in_range_pair_up() {
        let actors = vec![actor("a", "露娜", 0.0, 0.0), actor("b", "諾娃", 2.0, 0.0)];
        let pairs = find_playmates(&actors);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].a_name, "露娜");
        assert_eq!(pairs[0].b_name, "諾娃");
    }

    #[test]
    fn two_children_out_of_range_do_not_pair() {
        let actors = vec![actor("a", "露娜", 0.0, 0.0), actor("b", "諾娃", 50.0, 0.0)];
        assert!(find_playmates(&actors).is_empty());
    }

    #[test]
    fn exactly_at_range_boundary_pairs() {
        let actors = vec![actor("a", "露娜", 0.0, 0.0), actor("b", "諾娃", PLAYMATE_RANGE, 0.0)];
        assert_eq!(find_playmates(&actors).len(), 1);
    }

    #[test]
    fn just_beyond_range_boundary_does_not_pair() {
        let actors = vec![actor("a", "露娜", 0.0, 0.0), actor("b", "諾娃", PLAYMATE_RANGE + 0.01, 0.0)];
        assert!(find_playmates(&actors).is_empty());
    }

    #[test]
    fn four_children_two_close_pairs_no_overlap() {
        let actors = vec![
            actor("a", "露娜", 0.0, 0.0),
            actor("b", "諾娃", 1.0, 0.0),
            actor("c", "奧瑞", 100.0, 0.0),
            actor("d", "米拉", 101.0, 0.0),
        ];
        let pairs = find_playmates(&actors);
        assert_eq!(pairs.len(), 2);
        let mut ids: Vec<&str> = pairs.iter().flat_map(|p| [p.a_id.as_str(), p.b_id.as_str()]).collect();
        ids.sort();
        assert_eq!(ids, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn odd_one_out_stays_unpaired() {
        let actors = vec![
            actor("a", "露娜", 0.0, 0.0),
            actor("b", "諾娃", 1.0, 0.0),
            actor("c", "奧瑞", 2.0, 0.0),
        ];
        let pairs = find_playmates(&actors);
        assert_eq!(pairs.len(), 1, "三人只湊得出一對，剩一人落單");
    }

    #[test]
    fn pairing_is_deterministic_regardless_of_input_order() {
        let a = actor("a", "露娜", 0.0, 0.0);
        let b = actor("b", "諾娃", 1.0, 0.0);
        let forward = find_playmates(&[a.clone(), b.clone()]);
        let backward = find_playmates(&[b, a]);
        assert_eq!(forward, backward);
    }

    #[test]
    fn playmate_line_prefixed_and_mentions_partner() {
        for pick in 0..PLAYMATE_LINES.len() * 2 {
            let line = playmate_line("諾娃", pick);
            assert!(line.starts_with(PLAY_PREFIX));
            assert!(line.contains("諾娃"));
        }
    }

    #[test]
    fn playmate_line_pool_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for line in PLAYMATE_LINES {
            assert!(seen.insert(line), "台詞池不該有重複句");
        }
    }

    #[test]
    fn playmate_line_pick_cycles_through_pool() {
        let n = PLAYMATE_LINES.len();
        assert_eq!(playmate_line("諾娃", 0), playmate_line("諾娃", n));
        assert_ne!(playmate_line("諾娃", 0), playmate_line("諾娃", 1));
    }
}
