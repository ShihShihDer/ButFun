//! 喝采人氣（ROADMAP 341）——對附近玩家按一下「👏 喝采」，替他累積人氣；人氣到階，
//! 名牌上方永久亮起一枚人氣徽記，全世界擦肩而過都看得見誰最受歡迎。
//!
//! 338 表情單向、339 擊掌雙人、340 共鳴群體，三拍都是迸完就散的一次性特效——互動結束，
//! 彼此身上什麼都沒留下。喝采是這條線第一筆**會留下印記**的互動：它把社交從「即生即滅的
//! 特效」推進到「沉澱成看得見的人氣身份」。
//!
//! 設計取捨：
//! - **純判定、零狀態**：本模組只放純函式——挑對象（`pick_target`）、推導徽記（`popularity_for`）、
//!   防洗榜冷卻（`can_cheer`／`tick_cooldowns`）。意願倒數與冷卻表是 `Player` 上的記憶體欄位，
//!   人氣累積值 `cheers` 走既有 `positions` store 持久化（鏡像 336／337 的 atlas／skylog）。
//! - **挑最近的同區玩家**：喝采者一律用自己的權威座標，挑「同 `zone`／範圍內／最近」的另一名
//!   玩家當對象（同距取 id 較小求確定、跳過自己），防隔空喝采、結果確定可測。
//! - **零平衡風險**：人氣純社交身份，不換物品／乙太／戰力。刷榜只刷一個無實益的虛榮數字，
//!   且有「每對象 60s 冷卻」＋連線層限流雙重防濫用。

use std::collections::HashMap;
use uuid::Uuid;

/// 喝采意願維持的幀數（`game.rs` 每幀遞減）。約 1.5 秒（TICK_HZ≈20）——承載 ws→game 的
/// 喝采意圖，過了沒挑到對象就淡掉、不殘留。
pub const OFFER_TICKS: u16 = 30;

/// 能喝采的最大距離（像素，世界座標）。要靠得夠近、像真的在替身邊的人鼓掌，
/// 才算數——隔半個畫面互喊不是喝采。
pub const CHEER_RANGE: f32 = 72.0;

/// 對「同一個對象」再次喝采的冷卻幀數（≈60s）。你可以替全場每人各喝一次采，但不能對
/// 同一人連按刷數——這是防洗榜的核心（連線層限流是第二道）。
pub const CHEER_COOLDOWN: u16 = 1200;

/// 喝采特效在前端顯示的秒數（「👏 啪！」＋掌聲上飄淡出的總時長）。短而俏皮。
pub const CHEER_DISPLAY_SECS: u32 = 3;

/// 一名可被喝采的候選玩家快照。`zone` = 同區判定鍵（如星球名）；只有同 `zone` 的
/// 玩家才可能被挑中。座標為世界像素座標。
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: Uuid,
    pub zone: String,
    pub x: f32,
    pub y: f32,
}

/// 替一名喝采者挑出喝采對象。
///
/// 規則：從 `others` 裡挑「同 `zone`、距離 ≤ `CHEER_RANGE`、最近」的玩家；同距離時取 id
/// 較小者（求確定可測）；一律跳過喝采者自己（id 相同者）。都不符合回 `None`。
pub fn pick_target(
    giver_id: Uuid,
    giver_zone: &str,
    gx: f32,
    gy: f32,
    others: &[Candidate],
) -> Option<Uuid> {
    let mut best: Option<(Uuid, f32)> = None;
    for c in others {
        if c.id == giver_id || c.zone != giver_zone {
            continue;
        }
        let dx = c.x - gx;
        let dy = c.y - gy;
        let d2 = dx * dx + dy * dy;
        if d2 > CHEER_RANGE * CHEER_RANGE {
            continue;
        }
        // 取最近的；同距離時取 id 較小者以求確定。
        let better = match best {
            None => true,
            Some((bid, bd2)) => d2 < bd2 || (d2 == bd2 && c.id < bid),
        };
        if better {
            best = Some((c.id, d2));
        }
    }
    best.map(|(id, _)| id)
}

/// 一階人氣徽記：達到 `threshold` 點人氣即配戴，名牌上方顯示 `badge` + `title`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopularityTier {
    pub threshold: u64,
    pub badge: &'static str,
    pub title: &'static str,
}

/// 全部人氣階（**由高到低**排列：`popularity_for` 由上往下掃，回傳第一個已達門檻者）。
/// 門檻刻意拉開、越高越稀有——人氣是靠真人一次次喝采慢慢攢出來的。
pub const POPULARITY_TIERS: &[PopularityTier] = &[
    PopularityTier { threshold: 200, badge: "👑", title: "傳奇人物" },
    PopularityTier { threshold: 50,  badge: "🌟", title: "萬人迷" },
    PopularityTier { threshold: 10,  badge: "👏", title: "受歡迎" },
];

/// 依玩家累積人氣回傳其「配戴」的最高階人氣徽記（由高到低掃，第一個達門檻者勝）；
/// 未達最低門檻回 `None`。純函式，鏡像 335 蒐集稱號 `title_for`。
pub fn popularity_for(cheers: u64) -> Option<&'static PopularityTier> {
    POPULARITY_TIERS.iter().find(|t| cheers >= t.threshold)
}

/// 喝采者此刻能否對 `target` 計入一次人氣：對象不在冷卻表（或冷卻已歸零）才行。
/// 防同一對象被連按刷數。
pub fn can_cheer(cooldowns: &HashMap<Uuid, u16>, target: Uuid) -> bool {
    cooldowns.get(&target).copied().unwrap_or(0) == 0
}

/// 每幀把一名玩家的喝采冷卻表整體遞減一格，歸零的對象移除（保持表小、近乎零成本）。
pub fn tick_cooldowns(cooldowns: &mut HashMap<Uuid, u16>) {
    if cooldowns.is_empty() {
        return;
    }
    cooldowns.retain(|_, ticks| {
        *ticks = ticks.saturating_sub(1);
        *ticks > 0
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: u128, zone: &str, x: f32, y: f32) -> Candidate {
        Candidate { id: Uuid::from_u128(id), zone: zone.to_string(), x, y }
    }
    fn uid(id: u128) -> Uuid {
        Uuid::from_u128(id)
    }

    #[test]
    fn no_candidates_no_target() {
        assert!(pick_target(uid(1), "home", 0.0, 0.0, &[]).is_none());
    }

    #[test]
    fn skips_self() {
        // 場上只有自己 → 沒對象（不能替自己喝采）。
        let others = vec![cand(1, "home", 0.0, 0.0)];
        assert!(pick_target(uid(1), "home", 0.0, 0.0, &others).is_none());
    }

    #[test]
    fn picks_nearby_player() {
        let others = vec![cand(2, "home", 10.0, 0.0)];
        assert_eq!(pick_target(uid(1), "home", 0.0, 0.0, &others), Some(uid(2)));
    }

    #[test]
    fn ignores_out_of_range() {
        // 距離 200 > CHEER_RANGE(72) → 挑不到。
        let others = vec![cand(2, "home", 200.0, 0.0)];
        assert!(pick_target(uid(1), "home", 0.0, 0.0, &others).is_none());
    }

    #[test]
    fn boundary_inside_picks_outside_does_not() {
        let inside = vec![cand(2, "home", CHEER_RANGE - 0.1, 0.0)];
        assert_eq!(pick_target(uid(1), "home", 0.0, 0.0, &inside), Some(uid(2)), "剛好在範圍內要挑得到");
        let outside = vec![cand(2, "home", CHEER_RANGE + 0.1, 0.0)];
        assert!(pick_target(uid(1), "home", 0.0, 0.0, &outside).is_none(), "剛好超出範圍挑不到");
    }

    #[test]
    fn picks_nearest_of_several() {
        let others = vec![
            cand(2, "home", 50.0, 0.0),
            cand(3, "home", 10.0, 0.0), // 最近
            cand(4, "home", 30.0, 0.0),
        ];
        assert_eq!(pick_target(uid(1), "home", 0.0, 0.0, &others), Some(uid(3)));
    }

    #[test]
    fn tie_distance_picks_smaller_id() {
        // 兩人同距離 → 取 id 較小者，結果確定。
        let others = vec![cand(3, "home", 10.0, 0.0), cand(2, "home", 10.0, 0.0)];
        assert_eq!(pick_target(uid(1), "home", 0.0, 0.0, &others), Some(uid(2)));
    }

    #[test]
    fn different_zone_not_picked() {
        // 同座標但不同星球 → 挑不到（不同星球世界座標會撞）。
        let others = vec![cand(2, "verdant", 0.0, 0.0)];
        assert!(pick_target(uid(1), "home", 0.0, 0.0, &others).is_none());
    }

    #[test]
    fn popularity_none_below_first_tier() {
        assert!(popularity_for(0).is_none());
        assert!(popularity_for(9).is_none(), "未達 10 不配戴任何徽記");
    }

    #[test]
    fn popularity_tiers_by_threshold() {
        assert_eq!(popularity_for(10).map(|t| t.title), Some("受歡迎"));
        assert_eq!(popularity_for(49).map(|t| t.title), Some("受歡迎"));
        assert_eq!(popularity_for(50).map(|t| t.title), Some("萬人迷"));
        assert_eq!(popularity_for(199).map(|t| t.title), Some("萬人迷"));
        assert_eq!(popularity_for(200).map(|t| t.title), Some("傳奇人物"));
        assert_eq!(popularity_for(10_000).map(|t| t.title), Some("傳奇人物"));
    }

    #[test]
    fn popularity_tiers_ordered_high_to_low() {
        // popularity_for 由上往下掃、第一個達標者勝，故須由高門檻到低排列。
        for w in POPULARITY_TIERS.windows(2) {
            assert!(w[0].threshold > w[1].threshold, "人氣階應由高門檻到低排列");
        }
    }

    #[test]
    fn cooldown_blocks_then_clears() {
        let mut cd: HashMap<Uuid, u16> = HashMap::new();
        assert!(can_cheer(&cd, uid(2)), "沒喝采過就能喝采");
        cd.insert(uid(2), 3);
        assert!(!can_cheer(&cd, uid(2)), "冷卻中不能對同一人再計數");
        assert!(can_cheer(&cd, uid(3)), "對別人不受影響");
        // 三幀後冷卻歸零、對象從表中移除。
        tick_cooldowns(&mut cd);
        tick_cooldowns(&mut cd);
        assert!(!can_cheer(&cd, uid(2)), "尚未到期仍在冷卻");
        tick_cooldowns(&mut cd);
        assert!(can_cheer(&cd, uid(2)), "冷卻過後可再喝采");
        assert!(cd.is_empty(), "歸零的對象應已從表中移除");
    }

    #[test]
    fn tick_empty_cooldowns_is_noop() {
        let mut cd: HashMap<Uuid, u16> = HashMap::new();
        tick_cooldowns(&mut cd);
        assert!(cd.is_empty());
    }
}
