//! 水畔魚汛（ROADMAP 431）——伺服器權威、全服共享的魚群。
//!
//! 釣魚（ROADMAP 47／346 上鉤小遊戲）一直都在，但水面始終靜止、沒有任何
//! 線索告訴玩家「這裡有魚、可以下竿」——許多人根本不知道能釣魚。本模組把
//! 「水裡有魚」這件一直隱形的事**顯影**成水面上一群緩緩巡游的漣漪魚影：
//! 把世界切成與地名 locale 同尺度的分區，每塊分區恆有一處魚群，魚群中心隨
//! 一條全服共享的累積相位 `phase` 做緩慢的 Lissajous 巡游（兩軸不同頻率，
//! 不死板繞圈）。所有玩家眼裡同一塊水域的魚汛在同一個位置、同一個節拍漂移。
//!
//! 純邏輯層（無 IO、無 WebSocket、無 LLM、不碰生態查詢）：只把「世界座標＋相位」
//! 解析成魚群中心與「玩家是否循汛」。**要不要畫**（魚群中心須真落在水面上）由
//! 呼叫端各自用生態查詢決定——前端用 `biomeAt`、後端用 `world_core::biome_at`。
//!
//! 與世界風（430）同血脈：都是伺服器權威、全服共享、隨時間演化的世界狀態，
//! 但魚汛多了一個**誠實的小回報**——循著魚汛下竿，咬鉤等待略短（見
//! `BITE_HASTE`）；釣魚 5 秒冷卻仍主導產出節奏，故對經濟近乎零擾動，只是讓
//! 「讀懂水面、循汛下竿」這件事有手感上的回饋，而非寫死的數值膨脹。

use std::f32::consts::TAU;

/// 魚汛分區邊長（像素）：與地名 locale（ROADMAP 398／411）同尺度，每塊分區恆有一處魚群。
pub const CELL: f32 = 1536.0;

/// 魚群影響半徑（像素）：站在此半徑內的水邊下竿即視為「循汛」。
pub const SCHOOL_RADIUS: f32 = 132.0;

/// 魚群在所屬分區內漂移的幅度（像素）：留足邊距，使魚群恆落在自身分區內
/// （`CELL/2 − 半徑 − 邊距`），這樣「玩家循汛」只需查自身分區即夠、不必跨格。
const DRIFT: f32 = CELL * 0.5 - SCHOOL_RADIUS - 96.0;

/// 兩軸巡游週期（秒）：刻意取互質感的不同值，使中心走 Lissajous 曲線而非死板圓周；
/// `FISH_PERIOD` 取兩者最小公倍數（2100＝300×7＝420×5），相位回捲時兩軸恰各走完整數圈，
/// 故魚群位置在回捲處**連續、不跳位**。
const PERIOD_X: f32 = 300.0;
const PERIOD_Y: f32 = 420.0;

/// 全服共享相位的回捲週期（秒）：見上，取兩軸週期的最小公倍數確保回捲連續。
pub const FISH_PERIOD: f32 = 2100.0;

/// 兩軸角速度（弧度/秒）。
const W_X: f32 = TAU / PERIOD_X;
const W_Y: f32 = TAU / PERIOD_Y;

/// 循汛下竿的咬鉤加速係數：咬鉤等待時間乘以此值（愈小咬得愈快）。
/// 0.55＝循汛時等待約砍掉一半；但釣魚冷卻 5 秒不變、仍主導每分鐘下竿次數，
/// 故產出近乎零擾動——這只是手感回報，不是數值膨脹。
pub const BITE_HASTE: f32 = 0.55;

/// 世界座標所屬的魚汛分區索引。
pub fn cell_of(px: f32, py: f32) -> (i32, i32) {
    ((px / CELL).floor() as i32, (py / CELL).floor() as i32)
}

/// 分區 `(cx,cy)` 在相位 `phase` 時的魚群中心（世界座標）。
///
/// 確定性：每格以自身座標雜湊出一組固定的小相位偏移（小整數，Rust f32／JS f64
/// 算出來一致），使各格魚群彼此錯開、互不同步。`phase` 非有限時退回不漂移
/// （留在格心附近的固定點），永不產生 NaN。
pub fn school_point(cx: i32, cy: i32, phase: f32) -> (f32, f32) {
    let center_x = (cx as f32 + 0.5) * CELL;
    let center_y = (cy as f32 + 0.5) * CELL;
    let p = if phase.is_finite() { phase } else { 0.0 };
    // 每格固定的相位偏移：用座標的小範圍取模組出小整數弧度，Rust/JS 算得一致。
    let h = (cx.rem_euclid(8) * 3 + cy.rem_euclid(8) * 5) as f32;
    let off_x = h;
    let off_y = h * 0.5 + 1.0;
    let dx = (p * W_X + off_x).sin() * DRIFT;
    let dy = (p * W_Y + off_y).cos() * DRIFT;
    (center_x + dx, center_y + dy)
}

/// 玩家所在分區此刻的魚群中心（供「是否循汛」判定與前端就近渲染）。
pub fn school_near(px: f32, py: f32, phase: f32) -> (f32, f32) {
    let (cx, cy) = cell_of(px, py);
    school_point(cx, cy, phase)
}

/// 玩家是否站在自身分區魚群的影響半徑內（純距離判定；魚群是否真在水面上、
/// 玩家是否真在水邊由呼叫端另行以生態查詢／既有 `is_near_water` 把關）。
pub fn within_school(px: f32, py: f32, phase: f32) -> bool {
    let (sx, sy) = school_near(px, py, phase);
    let dx = px - sx;
    let dy = py - sy;
    dx * dx + dy * dy <= SCHOOL_RADIUS * SCHOOL_RADIUS
}

// ── 單元測試 ─────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    /// 魚群中心恆落在自身分區內（留邊不貼格界）——「循汛」只查自身分區的前提。
    #[test]
    fn school_stays_within_its_cell() {
        for cx in -3..3 {
            for cy in -3..3 {
                // 掃一整個相位週期，檢查中心永遠在分區界內。
                let mut phase = 0.0;
                while phase < FISH_PERIOD {
                    let (sx, sy) = school_point(cx, cy, phase);
                    let lo_x = cx as f32 * CELL;
                    let hi_x = (cx as f32 + 1.0) * CELL;
                    let lo_y = cy as f32 * CELL;
                    let hi_y = (cy as f32 + 1.0) * CELL;
                    assert!(
                        sx >= lo_x && sx <= hi_x && sy >= lo_y && sy <= hi_y,
                        "魚群中心 ({sx},{sy}) 跑出分區 ({cx},{cy}) 界 phase={phase}"
                    );
                    phase += 37.0;
                }
            }
        }
    }

    /// 相位回捲（+FISH_PERIOD）後魚群回到同一點——回捲連續、不跳位。
    #[test]
    fn school_is_continuous_across_period_wrap() {
        for &phase in &[0.0f32, 12.3, 199.0, 871.5, 2099.0] {
            let a = school_point(2, -1, phase);
            let b = school_point(2, -1, phase + FISH_PERIOD);
            assert!(
                (a.0 - b.0).abs() < 1e-2 && (a.1 - b.1).abs() < 1e-2,
                "回捲不連續：phase={phase} a={a:?} b={b:?}"
            );
        }
    }

    /// 確定性：同輸入同輸出。
    #[test]
    fn school_point_is_deterministic() {
        assert_eq!(school_point(1, 1, 100.0), school_point(1, 1, 100.0));
    }

    /// 不同分區的魚群不會同步在同一相對位置（相位偏移有效錯開）。
    #[test]
    fn neighbouring_cells_are_desynced() {
        let a = school_point(0, 0, 50.0);
        let b = school_point(1, 0, 50.0);
        // 扣掉格心差（CELL）後的「格內相對位移」不應相同。
        let ra = (a.0 - 0.5 * CELL, a.1 - 0.5 * CELL);
        let rb = (b.0 - 1.5 * CELL, b.1 - 0.5 * CELL);
        assert!(
            (ra.0 - rb.0).abs() > 1.0 || (ra.1 - rb.1).abs() > 1.0,
            "相鄰分區魚群相對位移竟相同：{ra:?} vs {rb:?}"
        );
    }

    /// 站在魚群中心＝循汛；遠在半徑外＝不循汛。
    #[test]
    fn within_school_boundary() {
        let phase = 321.0;
        let (sx, sy) = school_near(700.0, 700.0, phase);
        assert!(within_school(sx, sy, phase), "正在魚群中心應算循汛");
        // 把座標推到同一分區內、但離中心遠超半徑處（沿用同分區避免跨格）。
        let far_x = (sx + SCHOOL_RADIUS + 80.0).clamp(0.0, CELL - 1.0);
        assert!(
            !within_school(far_x, sy, phase),
            "離魚群中心夠遠不應算循汛"
        );
    }

    /// 壞相位（NaN/Inf）不 panic、不產生 NaN 座標。
    #[test]
    fn bad_phase_is_safe() {
        for &bad in &[f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let (sx, sy) = school_point(0, 0, bad);
            assert!(sx.is_finite() && sy.is_finite(), "壞相位產生了非有限座標");
        }
    }
}
