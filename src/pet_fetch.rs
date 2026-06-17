//! 寵物逗玩接物（ROADMAP 345）——玩家朝面前丟出一個玩具，寵物便興奮地衝過去把它叼回來，
//! 再小跑回主人身邊。寵物這條維度的第一個**玩家主動互動**的玩法。
//!
//! 343 給了寵物身體（自己跟著主人跑）、344 讓兩隻寵物自己湊一塊玩耍——兩者都是寵物**自發**
//! 的行為，玩家只能旁觀。本切片第一次把「玩」這件事交到玩家手上：你按一下「🎾 逗寵物」、
//! 朝面前丟出玩具，寵物就立刻衝出去追、叼到後一路叼回你腳邊——一場你來我往的接物小遊戲。
//!
//! 設計取捨：
//! - **純判定、零狀態**：本模組只放純函式——丟出點（`throw_spot`）、朝目標推進一步（`chase_step`）。
//!   進行中的接物狀態（玩具座標＋階段）是 `Player` 上一個**記憶體前置、不持久化、零 migration**
//!   的 `Option<PetFetch>`，每 tick 由 `game.rs` 用本模組的純函式推進。
//! - **與 343/344 乾淨分層、不是配對特效**：接物由「玩家一個新動詞」觸發，是一段「衝去→叼回」的
//!   兩段歷程，不是 344「偵測站位配對→迸特效」的鏡像骨架；接物進行中優先於跟隨／玩耍。
//! - **零平衡風險**：純陪伴玩耍呈現，不送任何物品／乙太／戰力，與既有寵物被動加成、343 跟隨、
//!   344 玩耍都正交。丟玩具沒有任何代價（連線層限流防洗螢幕即可），玩具叼回即消失、不入背包。

/// 玩具丟出的距離（像素，世界座標）。落在主人面前約一個身位開外——夠遠才有「衝出去追」的
/// 感覺，又不至於遠到寵物得橫越半個畫面。
pub const THROW_DIST: f32 = 130.0;

/// 寵物接物時的衝刺速度（像素/秒）。比 343 跟隨速度（240）、衝刺（288）都快——逗玩時寵物
/// 玩得最起勁、巴巴地衝去叼玩具。
pub const FETCH_SPEED: f32 = 320.0;

/// 寵物「叼到玩具」的判定半徑（像素）。衝到玩具這麼近即視為叼住、轉入叼回階段。
pub const GRAB_REACH: f32 = 10.0;

/// 寵物「叼回主人身邊」的判定半徑（像素）。叼著玩具跑回主人這麼近即視為交差、接物結束。
/// 比 GRAB 大一些——回到腳邊歇下即可，不必精準貼上主人。
pub const RETURN_REACH: f32 = 26.0;

/// 接物中若寵物與主人距離超過此值（像素）就放棄這趟接物（讓 343 跟隨接手把寵物瞬移歸位）。
/// 防主人在接物途中換星球／重生／瞬移時，寵物傻傻地橫越世界去追老遠的玩具。
/// 對齊 `pet_follow::FOLLOW_SNAP`(600) 再留一點緩衝。
pub const ABORT_DIST: f32 = 700.0;

/// 接物的兩個階段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchPhase {
    /// 衝去追：寵物正衝向玩具落點。
    Chasing,
    /// 叼回來：寵物已叼到玩具，正叼著一路跑回主人身邊。
    Returning,
}

/// 一趟進行中的接物狀態（記憶體前置、不持久化）。`toy_*` 在追逐階段是玩具落點、
/// 在叼回階段每 tick 更新為寵物當前座標（玩具被叼著一起移動，前端據此畫出被叼著的玩具）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PetFetch {
    pub toy_x: f32,
    pub toy_y: f32,
    pub phase: FetchPhase,
}

/// 算出玩具的落點：自主人座標朝丟出方向 `(dx, dy)` 延伸 `THROW_DIST`。
///
/// `(dx, dy)` 是玩家面前的方向（可為任意長度，內部正規化成單位向量）；方向近乎零
/// （站著沒朝向）時退化為朝右丟（`(1, 0)`），求結果確定可測、絕不 NaN。純函式。
pub fn throw_spot(owner_x: f32, owner_y: f32, dx: f32, dy: f32) -> (f32, f32) {
    let len = (dx * dx + dy * dy).sqrt();
    let (nx, ny) = if len > 1e-4 {
        (dx / len, dy / len)
    } else {
        (1.0, 0.0) // 沒有明確方向時預設朝右，確定可測
    };
    (owner_x + nx * THROW_DIST, owner_y + ny * THROW_DIST)
}

/// 把寵物朝目標 `(tx, ty)` 推進一步，單幀位移受 `speed * dt` 上限約束、絕不越過目標。
///
/// 回傳 `(新 x, 新 y, 是否抵達)`：當寵物已在 `reach` 半徑內、或這一步剛好（或超過）能踩到
/// 目標時即視為抵達（後者直接落在目標點，不越過）。`dt` 夾在 `>= 0`，除法在 `dist > reach >= 0`
/// 的分支才做、`dist` 必為正，安全不 NaN。純函式、同入同出。
pub fn chase_step(
    px: f32,
    py: f32,
    tx: f32,
    ty: f32,
    dt: f32,
    speed: f32,
    reach: f32,
) -> (f32, f32, bool) {
    let dx = tx - px;
    let dy = ty - py;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist <= reach {
        return (px, py, true); // 已在判定半徑內，停下
    }
    let max_step = (speed * dt.max(0.0)).max(0.0);
    if dist <= max_step {
        return (tx, ty, true); // 一步即可踏到目標，直接落上去、不越過
    }
    let inv = max_step / dist; // dist > reach >= 0 且 dist > max_step >= 0 → 安全
    (px + dx * inv, py + dy * inv, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
        ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt()
    }

    #[test]
    fn throw_spot_straight_right() {
        let (x, y) = throw_spot(100.0, 50.0, 1.0, 0.0);
        assert!((x - (100.0 + THROW_DIST)).abs() < 1e-3);
        assert!((y - 50.0).abs() < 1e-3);
    }

    #[test]
    fn throw_spot_distance_is_throw_dist() {
        // 任意方向丟出，落點與主人的距離恆為 THROW_DIST。
        let (x, y) = throw_spot(0.0, 0.0, 3.0, 4.0); // 非單位向量
        assert!((dist(0.0, 0.0, x, y) - THROW_DIST).abs() < 1e-3, "落點距離應正規化為 THROW_DIST");
    }

    #[test]
    fn throw_spot_zero_dir_defaults_right() {
        // 沒有明確方向（站著沒朝向）→ 退化朝右，確定可測、不 NaN。
        let (x, y) = throw_spot(10.0, 20.0, 0.0, 0.0);
        assert!((x - (10.0 + THROW_DIST)).abs() < 1e-3);
        assert!((y - 20.0).abs() < 1e-3);
        assert!(x.is_finite() && y.is_finite());
    }

    #[test]
    fn throw_spot_diagonal_normalized() {
        let (x, y) = throw_spot(0.0, 0.0, 1.0, 1.0);
        let inv_sqrt2 = 1.0 / 2.0_f32.sqrt();
        assert!((x - inv_sqrt2 * THROW_DIST).abs() < 1e-2);
        assert!((y - inv_sqrt2 * THROW_DIST).abs() < 1e-2);
    }

    #[test]
    fn chase_within_reach_stops() {
        let (x, y, arrived) = chase_step(100.0, 100.0, 105.0, 100.0, 0.1, FETCH_SPEED, GRAB_REACH);
        assert!(arrived, "已在 reach 內應視為抵達");
        assert_eq!((x, y), (100.0, 100.0), "抵達後原地不動");
    }

    #[test]
    fn chase_one_step_snaps_to_target() {
        // 距離（5）大於 reach（2）但小於單幀最大位移 → 直接踏到目標、不越過、視為抵達。
        let (x, y, arrived) = chase_step(0.0, 0.0, 5.0, 0.0, 1.0, FETCH_SPEED, 2.0);
        assert!(arrived);
        assert_eq!((x, y), (5.0, 0.0), "一步到位應正好落在目標點");
    }

    #[test]
    fn chase_partial_step_moves_toward_capped() {
        // 距離遠大於單幀位移 → 只前進 speed*dt，朝目標方向，尚未抵達。
        let dt = 0.05;
        let (x, y, arrived) = chase_step(0.0, 0.0, 1000.0, 0.0, dt, FETCH_SPEED, GRAB_REACH);
        assert!(!arrived);
        assert!((x - FETCH_SPEED * dt).abs() < 1e-2, "單幀位移應受速度上限約束");
        assert!((y - 0.0).abs() < 1e-3, "方向應正確（純朝右）");
    }

    #[test]
    fn chase_reduces_distance() {
        let (x, y, _) = chase_step(0.0, 0.0, 500.0, 0.0, 0.05, FETCH_SPEED, GRAB_REACH);
        assert!(dist(x, y, 500.0, 0.0) < dist(0.0, 0.0, 500.0, 0.0), "每步距離應縮短");
    }

    #[test]
    fn chase_zero_dt_no_move() {
        let (x, y, arrived) = chase_step(0.0, 0.0, 500.0, 0.0, 0.0, FETCH_SPEED, GRAB_REACH);
        assert!(!arrived);
        assert_eq!((x, y), (0.0, 0.0), "dt=0 不移動");
    }

    #[test]
    fn chase_is_pure_same_in_same_out() {
        let a = chase_step(3.0, 7.0, 200.0, 90.0, 0.033, FETCH_SPEED, GRAB_REACH);
        let b = chase_step(3.0, 7.0, 200.0, 90.0, 0.033, FETCH_SPEED, GRAB_REACH);
        assert_eq!(a, b, "純函式：同輸入同輸出");
    }

    #[test]
    fn chase_direction_diagonal() {
        // 朝右下目標，x、y 都應朝目標方向前進。
        let (x, y, _) = chase_step(0.0, 0.0, 300.0, 300.0, 0.05, FETCH_SPEED, GRAB_REACH);
        assert!(x > 0.0 && y > 0.0, "對角線推進兩軸都應前進");
        assert!((x - y).abs() < 1e-2, "45 度方向兩軸位移應相等");
    }

    #[test]
    fn chase_converges_over_many_steps() {
        // 連續推進應單調收斂到目標（模擬整趟衝去追的歷程）。
        let (mut px, mut py) = (0.0_f32, 0.0_f32);
        let (tx, ty) = (400.0_f32, 0.0_f32);
        let mut last = dist(px, py, tx, ty);
        let mut arrived = false;
        for _ in 0..200 {
            let (nx, ny, a) = chase_step(px, py, tx, ty, 0.033, FETCH_SPEED, GRAB_REACH);
            let d = dist(nx, ny, tx, ty);
            assert!(d <= last + 1e-3, "距離應單調不增");
            last = d;
            px = nx;
            py = ny;
            if a {
                arrived = true;
                break;
            }
        }
        assert!(arrived, "有限步內應抵達目標");
    }

    #[test]
    fn return_reach_larger_than_grab() {
        // 叼回主人腳邊的判定比叼玩具寬鬆些（回到身邊歇下即可）。
        assert!(RETURN_REACH > GRAB_REACH);
    }
}
