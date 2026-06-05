//! 地塊配置幾何（Phase 0-G-O1「農地改 per-player 擁有」的純邏輯地基，第一塊）。
//!
//! 目前世界只有 `field.rs` 那一塊全域共享農地。O1 要改成「每個玩家擁有自己的一塊地、
//! 空間分開、互不踩」——而要分開放，第一個缺的就是「第 N 個玩家的地該擺在世界的哪裡」。
//! 這層只管這件事：給一個地塊序號（0,1,2,…），算出它在世界中的左上角座標，
//! 讓家園區從中心**一圈一圈往外排**、彼此不重疊。同樣是純資料 + 純函式，無 IO、
//! 不碰 WebSocket / 遊戲迴圈 / `Field` 本身，便於自動測試。之後接上：
//!   - O1 接線：`Field` 帶上自己的 origin（不再用 `field.rs` 的全域常數），
//!     玩家進場 `assign` 一個序號、`plot_origin` 決定他那塊地的位置；`cell_at` /
//!     `within_field_reach` 改吃該 origin；伺服器 Farm 動作驗證「這塊地的序號屬於你」。
//!   - O2 地圖擴張：序號只增不減地往外長，正是「家園隨玩家數往外擴」與多星球願景的前奏。
//!
//! 設計取捨：
//!   - **序號 0 的地塊正好落在現有 `field.rs` 全域農地的位置**（`FIELD_ORIGIN`），
//!     讓接線時「第一個玩家」的地與今天看到的那塊無縫接續，不平白位移既有玩家的視野。
//!   - 用標準方塊螺旋（中心→右→上→左→下，一圈一圈）排序號：序號各不相同 ⇒ 整數格座標
//!     各不相同 ⇒ 任兩塊地至少差一個 stride（≥ 地塊footprint）⇒ **保證不重疊**，
//!     不必另存已用座標、也沒有浮點累積誤差。離中心越近序號越小，貼合「先來的玩家在家園核心」。
//!   - 不在這層擋世界邊界：地塊往外排遲早會超出目前 2000×2000 世界，但「把世界長大」正是
//!     O2 的工作；這層只給確定性的幾何，邊界與擴張交給接線/O2 決定。

use crate::field::{FIELD_COLS, FIELD_ORIGIN_X, FIELD_ORIGIN_Y, FIELD_ROWS, TILE_SIZE};

/// 相鄰地塊之間留的縫（世界像素）。留一格寬，讓玩家一眼分得出「這是另一塊地」、
/// 也給之後在縫裡擺柵欄 / 小路的空間。
pub const PLOT_GAP: f32 = TILE_SIZE;

/// 一塊地的footprint寬高（世界像素）＝農地格線尺寸（與 `field.rs` 同一套常數，單一真實來源）。
pub const PLOT_WIDTH: f32 = FIELD_COLS as f32 * TILE_SIZE;
pub const PLOT_HEIGHT: f32 = FIELD_ROWS as f32 * TILE_SIZE;

/// 相鄰地塊左上角的間距（footprint + 縫）。橫向 / 縱向各一套。
/// stride ≥ footprint 是「整數格座標不同 ⇒ 地塊不重疊」這個保證的關鍵前提。
pub const PLOT_STRIDE_X: f32 = PLOT_WIDTH + PLOT_GAP;
pub const PLOT_STRIDE_Y: f32 = PLOT_HEIGHT + PLOT_GAP;

/// 第 `index` 塊地在「螺旋格」中的整數座標（中心為 (0,0)，一圈一圈往外）。
///
/// 用標準方塊螺旋逐格走 `index` 步：起手朝上、碰到圈角就右轉，繞出
/// (0,0)→(1,0)→(1,1)→(0,1)→(-1,1)→(-1,0)→(-1,-1)→(0,-1)→(1,-1)→(2,-1)… 的外擴螺旋。
/// `index` 是玩家數量級的小數字，O(index) 走訪完全夠用且零狀態、純確定性。
fn spiral_grid(index: usize) -> (i32, i32) {
    let (mut x, mut y) = (0i32, 0i32);
    // 朝向：起手朝上（dy = -1）。碰到目前外圈的角就順時針右轉。
    let (mut dx, mut dy) = (0i32, -1i32);
    let mut result = (0, 0);
    for _ in 0..=index {
        result = (x, y);
        // 走到正方形外圈的角時轉向（Ulam 螺旋的標準轉角條件）。
        if x == y || (x < 0 && x == -y) || (x > 0 && x == 1 - y) {
            let t = dx;
            dx = -dy;
            dy = t;
        }
        x += dx;
        y += dy;
    }
    result
}

/// 第 `index` 塊地左上角在世界中的座標（像素）。序號 0 對齊現有全域農地位置。
#[allow(dead_code)] // 接線輪（玩家進場分配地塊）才有呼叫端；沿用本專案前置地基的慣例。
pub fn plot_origin(index: usize) -> (f32, f32) {
    let (gx, gy) = spiral_grid(index);
    (
        FIELD_ORIGIN_X + gx as f32 * PLOT_STRIDE_X,
        FIELD_ORIGIN_Y + gy as f32 * PLOT_STRIDE_Y,
    )
}

/// 第 `index` 塊地的footprint矩形 `(x, y, width, height)`（像素）。
/// 給接線 / O2 做世界邊界、鏡頭、碰撞與「點到哪塊地」用。
#[allow(dead_code)] // 同上，待接線。
pub fn plot_bounds(index: usize) -> (f32, f32, f32, f32) {
    let (x, y) = plot_origin(index);
    (x, y, PLOT_WIDTH, PLOT_HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 螺旋格座標要照標準方塊螺旋一步步外擴（鎖住前幾個座標，擋演算法被改壞）。
    #[test]
    fn spiral_grid_walks_a_square_spiral() {
        let expected = [
            (0, 0),
            (1, 0),
            (1, 1),
            (0, 1),
            (-1, 1),
            (-1, 0),
            (-1, -1),
            (0, -1),
            (1, -1),
            (2, -1),
        ];
        for (i, want) in expected.iter().enumerate() {
            assert_eq!(spiral_grid(i), *want, "螺旋格第 {i} 步座標不對");
        }
    }

    /// 序號 0 的地塊必須正好落在現有全域農地位置（接線時第一個玩家無縫接續）。
    #[test]
    fn plot_zero_aligns_with_global_field() {
        assert_eq!(plot_origin(0), (FIELD_ORIGIN_X, FIELD_ORIGIN_Y));
    }

    /// 同一個序號永遠給同一個位置（純確定性，沒有隨機 / 狀態）。
    #[test]
    fn plot_origin_is_deterministic() {
        for i in 0..30 {
            assert_eq!(plot_origin(i), plot_origin(i), "序號 {i} 不確定");
        }
    }

    /// 不同序號給不同位置（不會兩個玩家被分到同一塊地）。
    #[test]
    fn distinct_indices_give_distinct_origins() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..60 {
            let (x, y) = plot_origin(i);
            // f32 不能進 HashSet，轉成 bit 表示當鍵（同值同 bit）。
            assert!(
                seen.insert((x.to_bits(), y.to_bits())),
                "序號 {i} 的位置與前面某塊重複了"
            );
        }
    }

    /// 招牌保證：任兩塊地的footprint矩形都不重疊（每人一塊、互不踩）。
    #[test]
    fn no_two_plots_overlap() {
        let bounds: Vec<_> = (0..60).map(plot_bounds).collect();
        for (i, &(ax, ay, aw, ah)) in bounds.iter().enumerate() {
            for (j, &(bx, by, bw, bh)) in bounds.iter().enumerate() {
                if i >= j {
                    continue;
                }
                // 兩矩形重疊 ⇔ 在 x 與 y 兩軸上都有交疊。任一軸分離即不重疊。
                let overlap_x = ax < bx + bw && bx < ax + aw;
                let overlap_y = ay < by + bh && by < ay + ah;
                assert!(
                    !(overlap_x && overlap_y),
                    "地塊 {i} 與 {j} 重疊了：{:?} vs {:?}",
                    (ax, ay, aw, ah),
                    (bx, by, bw, bh)
                );
            }
        }
    }

    /// 相鄰地塊之間留得下一個縫（不會貼死在一起，看得出是兩塊地）。
    /// 用 runtime 的 `plot_origin` 量實際位移，而非直接斷言常數。
    #[test]
    fn adjacent_plots_leave_a_visible_gap() {
        // 序號 1 在序號 0 正右邊一個 stride；兩塊地之間的縫＝位移 - footprint 寬。
        let (x0, _) = plot_origin(0);
        let (x1, _) = plot_origin(1);
        let gap = (x1 - x0) - PLOT_WIDTH;
        assert!(gap > 0.0, "相鄰地塊沒留縫（gap = {gap}）");
    }
}
