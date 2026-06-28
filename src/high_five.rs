//! 玩家擊掌（ROADMAP 339）——兩名玩家靠近、各自比出擊掌，伺服器把他們配成一對，
//! 在兩人之間「啪」地迸出一道擊掌特效。這是第一條把「玩家↔玩家」**雙向同步**連起來的線。
//!
//! 338 表情是**單向廣播**（一個人比、全服看得到）；擊掌不同——它需要**兩個真人各自出手、
//! 又恰好站得夠近**才成立，是城鎮社交弧外延到「真人之間一起完成一個動作」的下一拍。
//!
//! 設計取捨：
//! - **純配對、零持久化**：玩家比擊掌時，`ws.rs` 只在該玩家身上點亮一個短暫的「擊掌意願」
//!   倒數（`high_five_offer`，記憶體前置、不入快照、不持久化、零 migration）。配對與廣播
//!   都在 `game.rs` 每幀做：把當下「還在比擊掌」的玩家依距離兩兩配對、迸出特效、清掉意願。
//! - **意願有時效**：意願維持 `OFFER_TICKS` 幀（約 1.5 秒），讓兩人不必分毫不差同時按——
//!   一前一後在這個時窗內各比一次就能對上。過了沒人接，意願自然淡掉、不殘留。
//! - **同區才配**：只有**同一星球、且同在室外**的玩家才配得起來（不同星球座標會撞、室內外
//!   空間不同）——配對函式吃 `zone` 字串，只在 `zone` 相同時才比距離。
//! - **純函式可測**：核心配對 `match_pairs` 是純函式（吃一串擊掌意願、吐配對結果），
//!   貪婪地讓每個人和「同區、範圍內、最近」的另一個人成對、每人至多入一對，結果確定可測。

use uuid::Uuid;

/// 擊掌意願維持的幀數（`game.rs` 每幀遞減）。約 2 秒（TICK_HZ=30）——夠兩人一前一後
/// 在這個時窗內各比一次就對得上，又短到過了就淡掉、不會和不相干的後來者誤配。
/// 隨 TICK_HZ 調整：30Hz × 2s = 60 幀；若再調 TICK_HZ，此常數需同步更新為 TICK_HZ * 2。
pub const OFFER_TICKS: u16 = 60;

/// 兩名玩家能擊掌的最大距離（像素，世界座標）。要靠得夠近、像真的伸手能擊到掌，
/// 才迸特效——隔半個畫面互比是各自的表情（338），不是擊掌。
pub const HIGH_FIVE_RANGE: f32 = 64.0;

/// 擊掌特效在前端顯示的秒數（「啪！」迸發＋火花上飄淡出的總時長）。短而俏皮。
pub const HIGH_FIVE_DISPLAY_SECS: u32 = 3;

/// 一筆「正在比擊掌」的玩家意願快照。`zone` = 同區判定鍵（如星球名）；只有 `zone` 相同的
/// 兩人才可能配成對。座標為世界像素座標。
#[derive(Debug, Clone)]
pub struct Offer {
    pub id: Uuid,
    pub zone: String,
    pub x: f32,
    pub y: f32,
}

/// 把一串擊掌意願兩兩配對。
///
/// 規則：
/// - 只有**同 `zone`** 且**距離 ≤ `HIGH_FIVE_RANGE`** 的兩人才可能成對。
/// - 貪婪配對：依 id 排序求確定性，逐一替尚未入對的人找「同區、範圍內、最近」的另一個未入對者
///   配成一對；每人至多入一對（已入對者不再參與後續配對）。
/// - 回傳每對以 `(較小 id, 較大 id)` 表示、整串依此排序——同一組意願永遠得到同一組配對，可測。
///
/// 自我配對不可能發生（id 唯一、且跳過自己）。一個落單的人這幀不成對、留待下幀（意願還在的話）。
pub fn match_pairs(offers: &[Offer]) -> Vec<(Uuid, Uuid)> {
    // 依 id 排序，讓「誰先挑」與挑中的結果都確定（不依賴 HashMap 迭代順序）。
    let mut idx: Vec<usize> = (0..offers.len()).collect();
    idx.sort_by_key(|&i| offers[i].id);

    let mut taken = vec![false; offers.len()];
    let mut pairs: Vec<(Uuid, Uuid)> = Vec::new();

    for a_pos in 0..idx.len() {
        let ai = idx[a_pos];
        if taken[ai] {
            continue;
        }
        // 替 a 找同區、範圍內、最近的未入對者 b。
        let a = &offers[ai];
        let mut best: Option<(usize, f32)> = None;
        for b_pos in (a_pos + 1)..idx.len() {
            let bi = idx[b_pos];
            if taken[bi] {
                continue;
            }
            let b = &offers[bi];
            if b.zone != a.zone {
                continue;
            }
            let dx = a.x - b.x;
            let dy = a.y - b.y;
            let d2 = dx * dx + dy * dy;
            if d2 > HIGH_FIVE_RANGE * HIGH_FIVE_RANGE {
                continue;
            }
            // 取最近的；同距離時取 id 較小的（idx 已排序，先掃到的即較小）以求確定。
            if best.map(|(_, bd2)| d2 < bd2).unwrap_or(true) {
                best = Some((bi, d2));
            }
        }
        if let Some((bi, _)) = best {
            taken[ai] = true;
            taken[bi] = true;
            let (lo, hi) = if a.id < offers[bi].id {
                (a.id, offers[bi].id)
            } else {
                (offers[bi].id, a.id)
            };
            pairs.push((lo, hi));
        }
    }

    pairs.sort();
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn off(id: u128, zone: &str, x: f32, y: f32) -> Offer {
        Offer { id: Uuid::from_u128(id), zone: zone.to_string(), x, y }
    }

    #[test]
    fn no_offers_no_pairs() {
        assert!(match_pairs(&[]).is_empty());
    }

    #[test]
    fn single_offer_does_not_pair() {
        let offers = vec![off(1, "home", 0.0, 0.0)];
        assert!(match_pairs(&offers).is_empty(), "落單的人不該和自己配對");
    }

    #[test]
    fn two_close_players_pair() {
        let offers = vec![off(1, "home", 0.0, 0.0), off(2, "home", 10.0, 0.0)];
        let pairs = match_pairs(&offers);
        assert_eq!(pairs, vec![(Uuid::from_u128(1), Uuid::from_u128(2))]);
    }

    #[test]
    fn two_far_players_do_not_pair() {
        // 距離 200 > HIGH_FIVE_RANGE(64) → 不成對。
        let offers = vec![off(1, "home", 0.0, 0.0), off(2, "home", 200.0, 0.0)];
        assert!(match_pairs(&offers).is_empty());
    }

    #[test]
    fn just_inside_range_pairs_just_outside_does_not() {
        let inside = vec![off(1, "home", 0.0, 0.0), off(2, "home", HIGH_FIVE_RANGE - 0.1, 0.0)];
        assert_eq!(match_pairs(&inside).len(), 1, "剛好在範圍內要成對");
        let outside = vec![off(1, "home", 0.0, 0.0), off(2, "home", HIGH_FIVE_RANGE + 0.1, 0.0)];
        assert!(match_pairs(&outside).is_empty(), "剛好超出範圍不成對");
    }

    #[test]
    fn different_zones_never_pair() {
        // 同座標但不同星球 → 不成對（不同星球世界座標會撞）。
        let offers = vec![off(1, "home", 5.0, 5.0), off(2, "verdant", 5.0, 5.0)];
        assert!(match_pairs(&offers).is_empty());
    }

    #[test]
    fn three_players_two_close_one_far_yields_one_pair() {
        let offers = vec![
            off(1, "home", 0.0, 0.0),
            off(2, "home", 8.0, 0.0),   // 與 1 近
            off(3, "home", 500.0, 0.0), // 落單
        ];
        let pairs = match_pairs(&offers);
        assert_eq!(pairs, vec![(Uuid::from_u128(1), Uuid::from_u128(2))]);
    }

    #[test]
    fn each_player_in_at_most_one_pair() {
        // 四人擠在一起：應配成兩對、每人恰好入一對，沒人重複。
        let offers = vec![
            off(1, "home", 0.0, 0.0),
            off(2, "home", 4.0, 0.0),
            off(3, "home", 8.0, 0.0),
            off(4, "home", 12.0, 0.0),
        ];
        let pairs = match_pairs(&offers);
        assert_eq!(pairs.len(), 2, "四個近距離玩家配成兩對");
        let mut seen = std::collections::HashSet::new();
        for (a, b) in &pairs {
            assert!(seen.insert(*a), "同一人不可入兩對");
            assert!(seen.insert(*b), "同一人不可入兩對");
        }
    }

    #[test]
    fn picks_nearest_partner() {
        // 1 同時夠近 2(距 50) 與 3(距 10)；應和最近的 3 配對，2 落單。
        let offers = vec![
            off(1, "home", 0.0, 0.0),
            off(2, "home", 50.0, 0.0),
            off(3, "home", 10.0, 0.0),
        ];
        let pairs = match_pairs(&offers);
        assert_eq!(pairs, vec![(Uuid::from_u128(1), Uuid::from_u128(3))]);
    }

    #[test]
    fn result_is_deterministic_regardless_of_input_order() {
        let a = vec![
            off(3, "home", 8.0, 0.0),
            off(1, "home", 0.0, 0.0),
            off(2, "home", 4.0, 0.0),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(match_pairs(&a), match_pairs(&b), "配對結果不該因輸入順序而變");
    }
}
