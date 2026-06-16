//! 寵物玩伴嬉戲（ROADMAP 344「寵物玩伴嬉戲」的純邏輯層）。
//!
//! ROADMAP 343 給了寵物身體——牠擁有自己的世界座標，會像隻黏人小夥伴跟著主人跑。本模組讓寵物
//! 在「跟隨」之上長出第一個**社交行為**：當兩名各有寵物的玩家靠在一塊（同星球、同在室外、站得夠近），
//! 他們的寵物會自己小跑到兩人之間湊作堆、開心地蹦跳玩耍一會兒；主人走遠、人潮散開，寵物便各自跑回
//! 主人身邊歇著。
//!
//! 這是**純由站位湧現**的互動——沒有新的玩家指令，玩家不必按任何鈕，只要兩個有寵物的人站近，
//! 寵物就自己湊起來玩。延續 `high_five.rs` / `pet_follow.rs` 的慣例：這層是純資料 ＋ 純函式、無 IO、
//! 不碰 WebSocket / 遊戲迴圈，由 `game.rs` 每 tick 餵呼叫。
//!
//! 主題是療癒的蒸汽龐克太空歌劇，寵物是被安撫馴服的乙太生靈——「夥伴之間也會彼此作伴玩耍」這件事
//! 本身就是溫柔的獲得感，與既有的被動加成、跟隨呈現都正交（加成管數值、跟隨管陪伴、嬉戲管社交），
//! 不送任何物品 / 乙太 / 戰力，零平衡風險。

use uuid::Uuid;

/// 兩名玩家的寵物能湊起來玩的最大「主人間距離」（像素，世界座標）。
/// 比擊掌（64）寬鬆些——擊掌要伸手碰得到，寵物玩耍只要兩個主人「在附近」就成立；
/// 但仍要明顯「站在一塊」，隔半個畫面不算。
pub const PLAY_RANGE: f32 = 150.0;

/// 玩耍時，每隻寵物的玩耍點相對兩主人中點、朝自己主人一側偏移的距離（像素）。
/// 兩隻寵物因此在中點兩側各據一方、相隔約 `2 * PLAY_GAP`，面對面玩耍而不重疊。
pub const PLAY_GAP: f32 = 16.0;

/// 寵物趕往玩耍點的速度（像素/秒）。比跟隨（240）略慢一點點——是輕快小跑去玩，不是衝刺。
pub const PLAY_SPEED: f32 = 220.0;

/// 寵物抵達玩耍點的判定半徑（像素）。進到這個圈內就算「到了」，原地蹦跳玩耍（彈跳由前端呈現）。
pub const PLAY_STOP: f32 = 8.0;

/// 一筆「有寵物、可參與玩耍」的玩家快照。`zone` = 同區判定鍵（如星球名）；只有 `zone` 相同的
/// 兩人寵物才可能湊起來玩。座標為主人的世界像素座標。
#[derive(Debug, Clone)]
pub struct PetActor {
    pub owner_id: Uuid,
    pub zone: String,
    pub owner_x: f32,
    pub owner_y: f32,
}

/// 一對湊起來玩耍的寵物。`a` / `b` 為兩位主人 id（恆 `a < b` 以求確定）；`spot_a` / `spot_b`
/// 分別是 a / b 的寵物該趕往的玩耍點（落在兩主人中點兩側、各偏向自己主人一些，面對面玩耍）。
#[derive(Debug, Clone, PartialEq)]
pub struct PlayPair {
    pub a: Uuid,
    pub b: Uuid,
    pub spot_a: (f32, f32),
    pub spot_b: (f32, f32),
}

/// 把一串「有寵物的玩家」兩兩配成寵物玩伴。
///
/// 規則（鏡像 `high_five::match_pairs` 的貪婪配對，求確定可重現）：
/// - 只有**同 `zone`** 且兩主人**距離 ≤ `PLAY_RANGE`** 的兩人，寵物才可能湊起來玩。
/// - 依 id 排序逐一替尚未入對者找「同區、範圍內、最近」的另一個未入對者配成一對；每人至多入一對。
/// - 結果依 `(a, b)` 排序回傳——同一組輸入永遠得到同一組配對（不依賴 HashMap 迭代順序），可測。
///
/// 每對另算出兩隻寵物的玩耍點：取兩主人中點，各朝自己主人一側偏移 `PLAY_GAP`，使兩寵物在中點
/// 兩側面對面玩耍而不重疊；兩主人恰好重疊時退化為左右各偏一點（a 在左、b 在右，確定）。
pub fn detect(actors: &[PetActor]) -> Vec<PlayPair> {
    // 依 id 排序，讓「誰先挑」與挑中的結果都確定。
    let mut idx: Vec<usize> = (0..actors.len()).collect();
    idx.sort_by_key(|&i| actors[i].owner_id);

    let mut taken = vec![false; actors.len()];
    let mut pairs: Vec<PlayPair> = Vec::new();

    for a_pos in 0..idx.len() {
        let ai = idx[a_pos];
        if taken[ai] {
            continue;
        }
        let a = &actors[ai];
        // 替 a 找同區、範圍內、最近的未入對者 b。
        let mut best: Option<(usize, f32)> = None;
        for b_pos in (a_pos + 1)..idx.len() {
            let bi = idx[b_pos];
            if taken[bi] {
                continue;
            }
            let b = &actors[bi];
            if b.zone != a.zone {
                continue;
            }
            let dx = a.owner_x - b.owner_x;
            let dy = a.owner_y - b.owner_y;
            let d2 = dx * dx + dy * dy;
            if d2 > PLAY_RANGE * PLAY_RANGE {
                continue;
            }
            // 取最近的；同距離時取 id 較小者（idx 已排序，先掃到的即較小）以求確定。
            if best.map(|(_, bd2)| d2 < bd2).unwrap_or(true) {
                best = Some((bi, d2));
            }
        }
        if let Some((bi, _)) = best {
            taken[ai] = true;
            taken[bi] = true;
            let b = &actors[bi];
            // 恆讓 a 為較小 id，spot 隨之對應，結果確定。
            let (lo, hi) = if a.owner_id < b.owner_id { (a, b) } else { (b, a) };
            let (spot_lo, spot_hi) = play_spots(
                (lo.owner_x, lo.owner_y),
                (hi.owner_x, hi.owner_y),
            );
            pairs.push(PlayPair {
                a: lo.owner_id,
                b: hi.owner_id,
                spot_a: spot_lo,
                spot_b: spot_hi,
            });
        }
    }

    pairs.sort_by_key(|p| (p.a, p.b));
    pairs
}

/// 算出兩隻寵物的玩耍點：兩主人中點各朝自己一側偏移 `PLAY_GAP`，面對面而不重疊。
/// 兩主人重疊（距離 0）時退化為左右各偏一點（第一個在左、第二個在右）以求確定。
fn play_spots(owner_a: (f32, f32), owner_b: (f32, f32)) -> ((f32, f32), (f32, f32)) {
    let mx = (owner_a.0 + owner_b.0) * 0.5;
    let my = (owner_a.1 + owner_b.1) * 0.5;
    let dx = owner_a.0 - owner_b.0;
    let dy = owner_a.1 - owner_b.1;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist <= f32::EPSILON {
        // 兩主人重疊：a 偏左、b 偏右（確定）。
        return ((mx - PLAY_GAP, my), (mx + PLAY_GAP, my));
    }
    let inv = 1.0 / dist;
    let ux = dx * inv; // 由 b 指向 a 的單位向量
    let uy = dy * inv;
    (
        (mx + ux * PLAY_GAP, my + uy * PLAY_GAP), // a 偏向自己主人一側
        (mx - ux * PLAY_GAP, my - uy * PLAY_GAP), // b 偏向自己主人一側
    )
}

/// 推進一步寵物趕往玩耍點：給定寵物當前座標、玩耍點、時間增量 `dt`（秒），回傳寵物新座標與
/// 「這一步是否在移動」（供前端判斷要不要播小跑彈跳）。
///
/// 純函式、無狀態、結果確定可重現，便於自動測試。手感鏡像 `pet_follow::follow_step`：到了玩耍點
/// （進入 `PLAY_STOP` 圈）就停下原地玩耍，否則朝玩耍點移動、單幀位移受 `PLAY_SPEED` 上限約束、
/// 且絕不越過玩耍點。
pub fn play_step(pet: (f32, f32), spot: (f32, f32), dt: f32) -> (f32, f32, bool) {
    let dx = spot.0 - pet.0;
    let dy = spot.1 - pet.1;
    let dist = (dx * dx + dy * dy).sqrt();

    // 已在玩耍點附近 → 原地玩耍（不再移動，彈跳由前端呈現）。
    if dist <= PLAY_STOP {
        return (pet.0, pet.1, false);
    }

    let step = (PLAY_SPEED * dt).min(dist).max(0.0);
    let inv = 1.0 / dist; // dist > PLAY_STOP > 0，除法安全
    let nx = pet.0 + dx * inv * step;
    let ny = pet.1 + dy * inv * step;
    (nx, ny, step > f32::EPSILON)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(id: u128, zone: &str, x: f32, y: f32) -> PetActor {
        PetActor { owner_id: Uuid::from_u128(id), zone: zone.to_string(), owner_x: x, owner_y: y }
    }

    fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
        let dx = a.0 - b.0;
        let dy = a.1 - b.1;
        (dx * dx + dy * dy).sqrt()
    }

    // ── detect 配對 ──────────────────────────────────────────────────────────

    #[test]
    fn no_actors_no_pairs() {
        assert!(detect(&[]).is_empty());
    }

    #[test]
    fn single_pet_owner_does_not_pair() {
        let actors = vec![actor(1, "home", 0.0, 0.0)];
        assert!(detect(&actors).is_empty(), "落單的寵物主人沒玩伴");
    }

    #[test]
    fn two_close_owners_pets_play() {
        let actors = vec![actor(1, "home", 0.0, 0.0), actor(2, "home", 40.0, 0.0)];
        let pairs = detect(&actors);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].a, Uuid::from_u128(1));
        assert_eq!(pairs[0].b, Uuid::from_u128(2));
    }

    #[test]
    fn two_far_owners_do_not_play() {
        // 距離 300 > PLAY_RANGE(150) → 寵物不湊堆。
        let actors = vec![actor(1, "home", 0.0, 0.0), actor(2, "home", 300.0, 0.0)];
        assert!(detect(&actors).is_empty());
    }

    #[test]
    fn just_inside_range_plays_just_outside_does_not() {
        let inside = vec![actor(1, "home", 0.0, 0.0), actor(2, "home", PLAY_RANGE - 0.1, 0.0)];
        assert_eq!(detect(&inside).len(), 1, "剛好在範圍內要湊堆");
        let outside = vec![actor(1, "home", 0.0, 0.0), actor(2, "home", PLAY_RANGE + 0.1, 0.0)];
        assert!(detect(&outside).is_empty(), "剛好超出範圍不湊堆");
    }

    #[test]
    fn different_zones_never_play() {
        // 同座標但不同星球 → 不湊堆。
        let actors = vec![actor(1, "home", 5.0, 5.0), actor(2, "verdant", 5.0, 5.0)];
        assert!(detect(&actors).is_empty());
    }

    #[test]
    fn three_owners_two_close_one_far_yields_one_pair() {
        let actors = vec![
            actor(1, "home", 0.0, 0.0),
            actor(2, "home", 30.0, 0.0),  // 與 1 近
            actor(3, "home", 800.0, 0.0), // 落單
        ];
        let pairs = detect(&actors);
        assert_eq!(pairs.len(), 1);
        assert_eq!((pairs[0].a, pairs[0].b), (Uuid::from_u128(1), Uuid::from_u128(2)));
    }

    #[test]
    fn each_owner_in_at_most_one_pair() {
        // 四人擠在一起：配成兩對、每人恰好入一對。
        let actors = vec![
            actor(1, "home", 0.0, 0.0),
            actor(2, "home", 20.0, 0.0),
            actor(3, "home", 40.0, 0.0),
            actor(4, "home", 60.0, 0.0),
        ];
        let pairs = detect(&actors);
        assert_eq!(pairs.len(), 2, "四個近距離寵物主人配成兩對");
        let mut seen = std::collections::HashSet::new();
        for p in &pairs {
            assert!(seen.insert(p.a), "同一人不可入兩對");
            assert!(seen.insert(p.b), "同一人不可入兩對");
        }
    }

    #[test]
    fn picks_nearest_partner() {
        // 1 同時夠近 2(距 100) 與 3(距 20)；應和最近的 3 配對，2 落單。
        let actors = vec![
            actor(1, "home", 0.0, 0.0),
            actor(2, "home", 100.0, 0.0),
            actor(3, "home", 20.0, 0.0),
        ];
        let pairs = detect(&actors);
        assert_eq!(pairs.len(), 1);
        assert_eq!((pairs[0].a, pairs[0].b), (Uuid::from_u128(1), Uuid::from_u128(3)));
    }

    #[test]
    fn result_is_deterministic_regardless_of_input_order() {
        let a = vec![
            actor(3, "home", 40.0, 0.0),
            actor(1, "home", 0.0, 0.0),
            actor(2, "home", 20.0, 0.0),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(detect(&a), detect(&b), "配對結果不該因輸入順序而變");
    }

    // ── 玩耍點 ───────────────────────────────────────────────────────────────

    #[test]
    fn play_spots_meet_near_midpoint_facing_each_other() {
        // 兩主人水平相隔 100，中點在 (50,0)；兩玩耍點應對稱落在中點兩側、相隔 2*PLAY_GAP。
        let actors = vec![actor(1, "home", 0.0, 0.0), actor(2, "home", 100.0, 0.0)];
        let pairs = detect(&actors);
        let p = &pairs[0];
        let mid = (50.0, 0.0);
        // a（id 1，在左主人側）的玩耍點偏左、b 偏右。
        assert!(p.spot_a.0 < mid.0, "a 的玩耍點應偏向自己主人（左）");
        assert!(p.spot_b.0 > mid.0, "b 的玩耍點應偏向自己主人（右）");
        // 兩玩耍點相隔約 2 * PLAY_GAP（面對面、不重疊）。
        assert!((dist(p.spot_a, p.spot_b) - 2.0 * PLAY_GAP).abs() < 0.01);
        // 兩玩耍點皆距中點 PLAY_GAP。
        assert!((dist(p.spot_a, mid) - PLAY_GAP).abs() < 0.01);
        assert!((dist(p.spot_b, mid) - PLAY_GAP).abs() < 0.01);
    }

    #[test]
    fn play_spots_handle_overlapping_owners() {
        // 兩主人完全重疊：玩耍點退化為左右各偏，不 NaN、不重疊。
        let actors = vec![actor(1, "home", 100.0, 100.0), actor(2, "home", 100.0, 100.0)];
        let pairs = detect(&actors);
        let p = &pairs[0];
        assert!(p.spot_a.0.is_finite() && p.spot_b.0.is_finite(), "不可 NaN");
        assert!(p.spot_a.0 < p.spot_b.0, "退化時 a 在左、b 在右");
        assert!(dist(p.spot_a, p.spot_b) > 0.0, "兩玩耍點不可重疊");
    }

    // ── play_step 趨近 ───────────────────────────────────────────────────────

    #[test]
    fn play_step_settles_within_stop_zone() {
        let pet = (100.0, 100.0);
        let spot = (104.0, 100.0); // 距離 4 < PLAY_STOP
        let (nx, ny, moving) = play_step(pet, spot, 0.1);
        assert_eq!((nx, ny), pet);
        assert!(!moving);
    }

    #[test]
    fn play_step_moves_toward_spot() {
        let pet = (0.0, 0.0);
        let spot = (200.0, 0.0);
        let before = dist(pet, spot);
        let (nx, ny, moving) = play_step(pet, spot, 0.1);
        assert!(moving);
        assert!(nx > 0.0, "應朝玩耍點（右）移動");
        assert!((ny - 0.0).abs() < 0.001);
        assert!(dist((nx, ny), spot) < before, "距離應縮短");
    }

    #[test]
    fn play_step_clamped_by_speed() {
        let pet = (0.0, 0.0);
        let spot = (500.0, 0.0);
        let dt = 0.05;
        let (nx, _ny, _moving) = play_step(pet, spot, dt);
        assert!((nx - PLAY_SPEED * dt).abs() < 0.01, "nx={nx}");
    }

    #[test]
    fn play_step_never_overshoots_spot() {
        let pet = (0.0, 0.0);
        let spot = (50.0, 0.0);
        let (nx, ny, _moving) = play_step(pet, spot, 100.0);
        // 巨大 dt：最多走到玩耍點上，絕不越過。
        assert!((dist((nx, ny), spot)) < 0.01, "應停在玩耍點上");
        assert!(nx <= spot.0 + 0.01, "不該越過玩耍點");
    }

    #[test]
    fn play_step_zero_dt_no_move() {
        let pet = (0.0, 0.0);
        let spot = (500.0, 0.0);
        let (nx, ny, moving) = play_step(pet, spot, 0.0);
        assert_eq!((nx, ny), pet);
        assert!(!moving);
    }

    #[test]
    fn play_step_deterministic_pure() {
        let pet = (12.0, 34.0);
        let spot = (456.0, 78.0);
        assert_eq!(play_step(pet, spot, 0.1), play_step(pet, spot, 0.1));
    }
}
