//! 乙太方界·野兔 v1——世界第一種環境生物（自主提案切片，ROADMAP 847）。
//!
//! **真缺口**：乙太方界至今只有 4 位具名 AI 居民會在世界裡走動，草地與森林從未有過
//! 任何一絲野生的動態——世界看得出「有人住」，卻看不出「有生機」。本模組補上世界
//! 第一種環境生物：幾隻在村莊周圍悠閒遊蕩的野兔，見到玩家靠近就受驚跳開。
//!
//! **刻意的範圍收斂**：純點綴、無 AI 大腦（零 LLM）、無戰鬥、無記憶、無持久化
//! （純記憶體、重啟於固定家域點重新生成，比照既有 `drops`/`stalls` 世界暫態慣例）。
//! 這不是居民↔居民/居民↔玩家關係軸線的第 N 刀，是全新的「世界環境」軸線第一刀。
//!
//! **純邏輯層**：家域遊蕩沿用既有 [`crate::voxel_residents`] 的
//! `wander_target`/`wander_center`/`step_toward`/`gravity_step`/`dry_ground_spawn`，
//! 本模組只補野兔專屬的「受驚偵測」與「逃跑方向」兩個確定性純函式，
//! 零 LLM、零鎖、零 IO，鎖/連線/tick 驅動全在 `voxel_ws.rs`。
//!
//! **餵野兔馴服 v1（自主提案切片）**：847/848 讓世界第一次看得出有生機，但那份生機
//! 至今只能遠遠看——玩家從沒有一條路能真正「碰」到牠。本刀補上世界環境軸線與玩家互動
//! 軸線第一次的交會：手持胡蘿蔔靠近一隻野兔並餵食，牠就此**永遠不再怕你**。因為
//! [`FLEE_RADIUS`] 大於 [`TAME_REACH`]，這一刀是刻意的——要餵到牠，得先追上一隻正在
//! 受驚逃跑的兔子，第一次成功的餵食因此帶著「追上牠」的小小成就感。**850 v1 說明裡
//! 明講「刻意只做『不再逃跑』，不做跟隨/寵物/繁殖」——跟隨正是本刀要補的那一半。**
//!
//! **馴服兔子跟隨你 v1（自主提案切片，ROADMAP 851）**：馴服至今只讓兔子「原地不怕你」，
//! 牠依舊只在自己的家域打轉，追上牠的那份成就感沒有下文——馴服一隻兔子和沒馴服看起來
//! 幾乎沒兩樣（除了牠不逃）。本刀讓馴服真正產生看得見的羈絆：**已馴服的兔子只要你靠近，
//! 就會像隻小跟班一樣跟上你走**（[`FOLLOW_RADIUS`] 內起跟、[`FOLLOW_LOSE_RADIUS`] 外才
//! 走失遲滯 hysteresis，同 [`should_flee`] 手法），跟到 [`FOLLOW_STOP_DIST`] 就停下不再
//! 往你身上擠；你若越走越遠，牠會安心跟丟、回到原本的閒晃。**v1 刻意收斂**：不分玩家
//! 身份（任何靠近的玩家都能被跟）、不繁殖、不能召回/放開、無寵物 UI——第一次讓「馴服」
//! 這件事在世界裡真的看得出差異，就是最小、最有感的一步。
//!
//! **馴服兔子生寶寶 v1（自主提案切片，ROADMAP 855）**：850/851 明講 v1 刻意不做「繁殖」——
//! 但那正是「世界環境」軸線唯一還空著的一格：野兔/游魚至今是固定數量的點綴生物，
//! 世界本身從沒有「自己長大」過。本刀補上：兩隻已馴服的兔子只要湊得夠近
//! （[`BREED_RADIUS`] 內），隔一段夠久的節流時間（[`BREED_INTERVAL_SECS`]）就有機率
//! （[`BREED_CHANCE`]）誕生一隻小兔子——牠一出生就是**已馴服**的（跟父母一樣認得你、
//! 立刻跟著走），世界第一次因為「你馴服了牠們」而自己長出新的生命。**刻意收斂**：
//! 全域節流（不分哪一對，同一時間全世界至多生一隻）、population 天花板
//! （[`MAX_RABBITS`]）防止無限增長、寶寶落在雙親中點附近最近的乾地、純記憶體
//! （重啟歸零，比照 wildlife 系統既有慣例）——不做基因/外觀差異，最小、最有感的一步。

/// 野兔閒晃速度（方塊/秒）——比居民散步（2.6）更悠閒，符合小動物碎步的觀感。
pub const WANDER_SPEED: f32 = 1.4;
/// 受驚逃跑速度——明顯比閒晃快，一眼看得出「嚇到了」。
pub const FLEE_SPEED: f32 = 4.2;
/// 玩家進入這個距離內，野兔就會受驚逃跑（方塊）。
pub const FLEE_RADIUS: f32 = 4.0;
/// 已受驚時，玩家要遠到超過這個距離才安心恢復閒晃（比 [`FLEE_RADIUS`] 稍大，
/// 這道遲滯（hysteresis）避免野兔在臨界距離上受驚/平靜來回抖動）。
pub const CALM_RADIUS: f32 = 6.0;
/// 逃跑目標離當下位置的距離（方塊）。
pub const FLEE_DIST: f32 = 6.0;
/// 野兔閒晃半徑下限（方塊）——比居民 `HOME_RADIUS`（20）小得多，野兔活動範圍更侷限。
pub const WANDER_MIN_R: f32 = 1.5;
/// 野兔閒晃半徑上限（方塊）。
pub const WANDER_MAX_R: f32 = 6.0;

/// 餵食馴服的觸及範圍（方塊）——刻意小於 [`FLEE_RADIUS`]：要餵到牠就得先追上正在
/// 受驚逃跑的兔子，第一次成功馴服因此帶著「追上牠」的小小成就感。
pub const TAME_REACH: f32 = 3.0;

/// 判斷這次餵食是否能成功馴服：距離要夠近、且這隻兔子還沒被馴服過（馴服是一次性、
/// 永久的——重複餵已馴服的兔子不會有任何效果，避免玩家對著同一隻兔子洗馴服訊息）。
pub fn should_tame(already_tamed: bool, player_dist_sq: f32) -> bool {
    !already_tamed && player_dist_sq < TAME_REACH * TAME_REACH
}

/// 馴服成功那一刻的回饋句（確定性輪替，`pick` 由呼叫端提供隨機源）。
const TAME_LINES: [&str; 4] = [
    "🥕 牠湊近你的手心，安心地嚼了起來——牠好像不再那麼怕你了。",
    "🥕 牠豎起耳朵愣了一下，接著才小口小口啃了起來，眼神放鬆了不少。",
    "🥕 牠終於停下逃跑的腳步，就地啃起你遞出的胡蘿蔔。",
    "🥕 牠蹭了蹭你的手，往後看見你也不會再拔腿就跑了。",
];

/// 依 `pick` 取一句馴服回饋（越界安全取模，永不 panic）。
pub fn tame_line(pick: usize) -> &'static str {
    TAME_LINES[pick % TAME_LINES.len()]
}

/// 已馴服的兔子開始跟隨的距離（方塊）——比 [`FLEE_RADIUS`] 寬鬆許多：不必刻意逼近，
/// 平常靠近牠就會主動跟上。
pub const FOLLOW_RADIUS: f32 = 8.0;
/// 已在跟隨時，玩家要遠到超過這個距離才安心跟丟（遲滯，避免臨界距離上跟隨/走失來回抖動，
/// 手法同 [`should_flee`] 的 `FLEE_RADIUS`/`CALM_RADIUS` 兩段式門檻）。
pub const FOLLOW_LOSE_RADIUS: f32 = 14.0;
/// 跟隨速度——比閒晃（[`WANDER_SPEED`]）快一些才追得上你的腳步，但不到受驚逃跑那麼急。
pub const FOLLOW_SPEED: f32 = 2.4;
/// 跟到這個距離就停下，不再往玩家身上擠（方塊）。
pub const FOLLOW_STOP_DIST: f32 = 2.5;

/// 依「目前是否正在跟隨」+「與最近玩家的距離平方」，判斷這一 tick 該不該跟隨（或維持跟隨）。
///
/// 遲滯避免抖動：還沒跟上時要近到 [`FOLLOW_RADIUS`] 內才起跟；已在跟隨時要遠到
/// [`FOLLOW_LOSE_RADIUS`] 外才安心跟丟——與 [`should_flee`] 同一手法，只是換了一組半徑。
pub fn should_follow(currently_following: bool, nearest_player_dist_sq: f32) -> bool {
    let threshold = if currently_following { FOLLOW_LOSE_RADIUS } else { FOLLOW_RADIUS };
    nearest_player_dist_sq < threshold * threshold
}

/// 已在跟隨時，這一 tick 是否還要再往玩家的方向邁一步——跟到 [`FOLLOW_STOP_DIST`] 內
/// 就別再擠過去（純距離判定，供呼叫端決定要 `step_toward` 還是原地 `gravity_step`）。
pub fn should_close_follow_gap(player_dist_sq: f32) -> bool {
    player_dist_sq > FOLLOW_STOP_DIST * FOLLOW_STOP_DIST
}

/// 依「目前是否已受驚」+「與最近玩家的距離平方」，判斷這一 tick 該不該受驚（或維持受驚）。
///
/// 用遲滯避免抖動：平靜時要近到 [`FLEE_RADIUS`] 內才受驚；已受驚時要遠到
/// [`CALM_RADIUS`] 外才平靜下來——兩段式門檻讓「快靠近/快遠離」的邊界不反覆橫跳。
pub fn should_flee(currently_fleeing: bool, nearest_player_dist_sq: f32) -> bool {
    let threshold = if currently_fleeing { CALM_RADIUS } else { FLEE_RADIUS };
    nearest_player_dist_sq < threshold * threshold
}

/// 兔群數量天花板（世界初始 6 隻 + 最多再生 6 隻）——馴服兔子生寶寶 v1 防止無限增長。
pub const MAX_RABBITS: usize = 12;
/// 兩隻已馴服的兔子要湊到多近才算「在一起」、有機會生寶寶（方塊）。
pub const BREED_RADIUS: f32 = 3.0;
/// 全域生育節流：至少間隔這麼久才會再檢查一次生育（秒）——比照人口成長 v1 的
/// elapsed 節流手法，避免同一對兔子黏在一起就無限連生。
pub const BREED_INTERVAL_SECS: f32 = 90.0;
/// 節流窗口到了、且找得到湊近的一對時，這次判定的生育機率。
pub const BREED_CHANCE: f32 = 0.35;

/// 判斷這一輪節流窗口是否該誕生一隻小兔子（純函式、可測）：
/// 兔群數未達天花板 + 距上次生育夠久 + 機率骰命中。
pub fn should_breed(current_rabbit_count: usize, elapsed_since_last: f32, roll: f32) -> bool {
    current_rabbit_count < MAX_RABBITS
        && elapsed_since_last >= BREED_INTERVAL_SECS
        && roll < BREED_CHANCE
}

/// 在目前所有已馴服兔子的座標（`(索引, x, z)`）裡，找出第一對距離在 [`BREED_RADIUS`]
/// 內的親代、回傳兩者的索引。純函式、零隨機、O(n²) 但 n 極小（兔群天花板僅 12）。
pub fn find_breeding_pair(tamed_positions: &[(usize, f32, f32)]) -> Option<(usize, usize)> {
    for i in 0..tamed_positions.len() {
        for j in (i + 1)..tamed_positions.len() {
            let (ia, ax, az) = tamed_positions[i];
            let (ib, bx, bz) = tamed_positions[j];
            let dx = ax - bx;
            let dz = az - bz;
            if dx * dx + dz * dz <= BREED_RADIUS * BREED_RADIUS {
                return Some((ia, ib));
            }
        }
    }
    None
}

/// 由一對親代座標算出寶寶的落地點（兩者中點，純幾何、無隨機性）。
pub fn baby_spawn_point(ax: f32, az: f32, bx: f32, bz: f32) -> (f32, f32) {
    ((ax + bx) / 2.0, (az + bz) / 2.0)
}

/// 小兔子誕生那一刻的回饋句（確定性輪替，`pick` 由呼叫端提供隨機源）。
const BABY_LINES: [&str; 3] = [
    "🐇 草地上多了一隻毛茸茸的小兔子，正跌跌撞撞地跟著爸媽學走路。",
    "🐇 兩隻兔子依偎了一會兒，不知不覺間，身邊多了一隻怯生生的小兔子。",
    "🐇 一隻剛出生的小兔子睜開眼，第一眼就認出了你——牠也不怕你。",
];

/// 依 `pick` 取一句誕生回饋（越界安全取模，永不 panic）。
pub fn baby_line(pick: usize) -> &'static str {
    BABY_LINES[pick % BABY_LINES.len()]
}

/// 由兔子座標與（最近）玩家座標算出「逃離玩家」的目標點（純幾何、無隨機性、可測）。
///
/// 玩家與兔子恰好同座標（距離為 0，退化情況）時預設往 +x 方向逃，避免除以零。
pub fn flee_target(rx: f32, rz: f32, px: f32, pz: f32) -> (f32, f32) {
    let dx = rx - px;
    let dz = rz - pz;
    let dist = (dx * dx + dz * dz).sqrt();
    if dist < 1e-4 {
        (rx + FLEE_DIST, rz)
    } else {
        (rx + dx / dist * FLEE_DIST, rz + dz / dist * FLEE_DIST)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_flee_triggers_within_flee_radius_when_calm() {
        assert!(should_flee(false, 3.9 * 3.9));
        assert!(!should_flee(false, 4.1 * 4.1));
    }

    #[test]
    fn should_flee_hysteresis_keeps_fleeing_until_calm_radius() {
        // 已受驚：距離落在 flee~calm 之間仍算受驚（遲滯，不提早平靜）。
        assert!(should_flee(true, 5.0 * 5.0));
        // 遠到超過 calm 半徑才真正平靜。
        assert!(!should_flee(true, 6.1 * 6.1));
    }

    #[test]
    fn should_flee_exact_boundary_is_exclusive() {
        // 距離恰好等於門檻不算「進入」（< 而非 <=），邊界一致不誤觸。
        assert!(!should_flee(false, FLEE_RADIUS * FLEE_RADIUS));
        assert!(!should_flee(true, CALM_RADIUS * CALM_RADIUS));
    }

    #[test]
    fn flee_target_points_directly_away_from_player() {
        let (tx, tz) = flee_target(0.0, 0.0, 1.0, 0.0);
        assert!(tx < 0.0, "玩家在 +x，兔子該往 -x 逃，得到 tx={tx}");
        assert!(tz.abs() < 1e-4);
    }

    #[test]
    fn flee_target_scales_to_flee_dist() {
        let (tx, tz) = flee_target(0.0, 0.0, 0.0, 3.0);
        let dist = (tx * tx + tz * tz).sqrt();
        assert!((dist - FLEE_DIST).abs() < 1e-3);
    }

    #[test]
    fn flee_target_handles_zero_distance_without_panic() {
        let (tx, tz) = flee_target(5.0, 5.0, 5.0, 5.0);
        assert!((tx - (5.0 + FLEE_DIST)).abs() < 1e-4);
        assert!((tz - 5.0).abs() < 1e-4);
    }

    #[test]
    fn flee_target_diagonal_direction() {
        let (tx, tz) = flee_target(0.0, 0.0, 1.0, 1.0);
        assert!(tx < 0.0 && tz < 0.0, "玩家在右上方，兔子該往左下逃");
        // 對角逃跑距離仍應是 FLEE_DIST（正規化過的方向向量）。
        let dist = (tx * tx + tz * tz).sqrt();
        assert!((dist - FLEE_DIST).abs() < 1e-3);
    }

    #[test]
    fn should_tame_requires_close_enough() {
        assert!(should_tame(false, 2.9 * 2.9));
        assert!(!should_tame(false, 3.1 * 3.1));
    }

    #[test]
    fn should_tame_boundary_is_exclusive() {
        assert!(!should_tame(false, TAME_REACH * TAME_REACH));
    }

    #[test]
    fn should_tame_rejects_already_tamed_regardless_of_distance() {
        assert!(!should_tame(true, 0.0));
    }

    #[test]
    fn tame_reach_tighter_than_flee_radius() {
        // 刻意設計：要餵到牠，得先追上正在逃跑的兔子（見模組說明）。
        assert!(TAME_REACH < FLEE_RADIUS);
    }

    #[test]
    fn tame_line_picks_vary_and_stay_nonempty() {
        let seen: std::collections::HashSet<&str> =
            (0..TAME_LINES.len()).map(tame_line).collect();
        assert_eq!(seen.len(), TAME_LINES.len(), "四句應各不相同");
        for pick in 0..TAME_LINES.len() {
            assert!(!tame_line(pick).is_empty());
        }
    }

    #[test]
    fn tame_line_pick_wraps_without_panic() {
        // 越界 pick 應安全取模，不 panic。
        let _ = tame_line(usize::MAX);
    }

    #[test]
    fn should_follow_triggers_within_follow_radius_when_not_following() {
        assert!(should_follow(false, 7.9 * 7.9));
        assert!(!should_follow(false, 8.1 * 8.1));
    }

    #[test]
    fn should_follow_hysteresis_keeps_following_until_lose_radius() {
        // 已在跟隨：距離落在 follow~lose 之間仍算跟著（遲滯，不提早跟丟）。
        assert!(should_follow(true, 10.0 * 10.0));
        // 遠到超過走失半徑才真正跟丟。
        assert!(!should_follow(true, 14.1 * 14.1));
    }

    #[test]
    fn should_follow_exact_boundary_is_exclusive() {
        assert!(!should_follow(false, FOLLOW_RADIUS * FOLLOW_RADIUS));
        assert!(!should_follow(true, FOLLOW_LOSE_RADIUS * FOLLOW_LOSE_RADIUS));
    }

    #[test]
    fn follow_radius_tighter_than_lose_radius() {
        // 遲滯設計前提：起跟半徑必須小於走失半徑，否則兩段式門檻無意義。
        assert!(FOLLOW_RADIUS < FOLLOW_LOSE_RADIUS);
    }

    #[test]
    fn should_close_follow_gap_stops_within_stop_dist() {
        assert!(should_close_follow_gap(2.6 * 2.6));
        assert!(!should_close_follow_gap(2.4 * 2.4));
    }

    #[test]
    fn should_close_follow_gap_boundary_is_exclusive() {
        assert!(!should_close_follow_gap(FOLLOW_STOP_DIST * FOLLOW_STOP_DIST));
    }

    // ── 馴服兔子生寶寶 v1 ────────────────────────────────────────────────

    #[test]
    fn should_breed_requires_all_three_conditions() {
        assert!(should_breed(4, BREED_INTERVAL_SECS, 0.0));
        assert!(!should_breed(MAX_RABBITS, BREED_INTERVAL_SECS, 0.0), "到天花板不該再生");
        assert!(!should_breed(4, BREED_INTERVAL_SECS - 1.0, 0.0), "節流未到不該生");
        assert!(!should_breed(4, BREED_INTERVAL_SECS, BREED_CHANCE), "機率沒中不該生");
    }

    #[test]
    fn should_breed_boundary_is_inclusive_for_elapsed() {
        // elapsed 恰好等於節流秒數應可生（>= 而非 >）。
        assert!(should_breed(0, BREED_INTERVAL_SECS, 0.0));
    }

    #[test]
    fn should_breed_chance_boundary_is_exclusive() {
        assert!(should_breed(0, BREED_INTERVAL_SECS, BREED_CHANCE - 0.001));
        assert!(!should_breed(0, BREED_INTERVAL_SECS, BREED_CHANCE));
    }

    #[test]
    fn find_breeding_pair_finds_close_pair() {
        let positions = vec![(0usize, 0.0, 0.0), (2usize, 100.0, 100.0), (5usize, 1.0, 1.0)];
        let pair = find_breeding_pair(&positions);
        assert_eq!(pair, Some((0, 5)), "索引 0 與 5 距離夠近應配成一對");
    }

    #[test]
    fn find_breeding_pair_none_when_all_far_apart() {
        let positions = vec![(0usize, 0.0, 0.0), (1usize, 100.0, 0.0), (2usize, 0.0, 100.0)];
        assert_eq!(find_breeding_pair(&positions), None);
    }

    #[test]
    fn find_breeding_pair_none_when_fewer_than_two() {
        assert_eq!(find_breeding_pair(&[]), None);
        assert_eq!(find_breeding_pair(&[(0usize, 0.0, 0.0)]), None);
    }

    #[test]
    fn find_breeding_pair_boundary_is_inclusive() {
        // 恰好等於 BREED_RADIUS 應算「湊近」（<= 而非 <，與 should_tame 等距離判定刻意不同——
        // 這裡沒有「先受驚再馴服」那種需要嚴格小於的追逐設計，純粹「夠近就算」）。
        let positions = vec![(0usize, 0.0, 0.0), (1usize, BREED_RADIUS, 0.0)];
        assert_eq!(find_breeding_pair(&positions), Some((0, 1)));
    }

    #[test]
    fn baby_spawn_point_is_midpoint() {
        let (x, z) = baby_spawn_point(0.0, 0.0, 4.0, 2.0);
        assert!((x - 2.0).abs() < 1e-4);
        assert!((z - 1.0).abs() < 1e-4);
    }

    #[test]
    fn baby_line_picks_vary_and_stay_nonempty() {
        let seen: std::collections::HashSet<&str> =
            (0..BABY_LINES.len()).map(baby_line).collect();
        assert_eq!(seen.len(), BABY_LINES.len(), "三句應各不相同");
        for pick in 0..BABY_LINES.len() {
            assert!(!baby_line(pick).is_empty());
        }
    }

    #[test]
    fn baby_line_pick_wraps_without_panic() {
        let _ = baby_line(usize::MAX);
    }
}
