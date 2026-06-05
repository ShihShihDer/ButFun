//! 採集節點撒佈幾何（Phase 1-A「採集節點與動作」的純邏輯地基，第二塊）。
//!
//! `gather.rs` 解了「一個節點怎麼被採、採空怎麼重生」；接線 Phase 1-A 還差另一半——
//! 「世界裡這些節點該撒在哪、各是什麼種類」。這層只管這件事：給出一組確定性的
//! 節點落點（座標 + 種類），讓樹 / 石 / 乙太礦散佈在世界各處、避開玩家出生與農莊核心，
//! 採集才有「走出去探索、找資源」的節奏。同樣是純資料 + 純函式，無 IO、不碰
//! WebSocket / 遊戲迴圈，便於自動測試。延續 `plots.rs`（地塊配置幾何）的前置慣例：
//! 純幾何先落地、標 `allow(dead_code)`，接線輪才有呼叫端。
//!
//! 之後接上：
//!   - Phase 1-A 接線：伺服器啟動時 `scatter_nodes()` 生出世界節點清單，每筆配一個
//!     `gather::ResourceNode::new(kind)`；隨快照把節點位置 / 種類 / 是否採空廣播給前端畫。
//!   - 遊戲迴圈：每 tick 對採空的節點 `tick(dt)` 推進重生。
//!
//! 設計取捨：
//!   - **完全確定性、無隨機 / 無狀態**：用座標雜湊在一張粗格上決定「這格放不放節點、
//!     放哪個種類、在格內哪個位置」。同一份程式每次跑出同一張節點圖——重啟後節點不亂跳，
//!     也讓持久化（接 0-E）有穩定的世界基準（沿用前端 `game.js` 草叢以座標雜湊撒佈的路數）。
//!   - **避開家園核心**：序號 0 的地塊（現有全域農地）四周留一圈淨空（`home_clearance`），
//!     節點不會生在玩家出生點 / 起始農莊上擋路。世界中心 (1000,1000) 落在序號 0 地塊內，
//!     故這一圈淨空同時罩住出生點。
//!   - **不在這層處理「節點 vs 之後往外長的 per-player 地塊」重疊**：地塊隨玩家數往外排
//!     是 O1/O2 的事；接線時若某塊新地壓到節點，由接線/O2 決定讓地或讓節點，這層只給
//!     確定性的世界底圖。

use crate::field::{FIELD_ORIGIN_X, FIELD_ORIGIN_Y, TILE_SIZE};
use crate::gather::NodeKind;
use crate::plots::{PLOT_HEIGHT, PLOT_WIDTH};
use crate::state::{WORLD_HEIGHT, WORLD_WIDTH};

/// 撒佈用的粗格行列數（世界切成 `GRID_COLS × GRID_ROWS` 格，每格至多一個節點）。
const GRID_COLS: u32 = 8;
const GRID_ROWS: u32 = 8;

/// 一格內「放節點」的機率（百分比）。其餘格留空，讓節點疏密自然、不鋪滿整張地。
const NODE_DENSITY_PCT: u64 = 55;

/// 節點距格邊的內縮（世界像素），避免落點貼在格線上、相鄰格節點靠太近。
const CELL_MARGIN: f32 = TILE_SIZE;

/// 家園核心（序號 0 地塊）四周額外留的淨空（世界像素）。節點不生在此圈內，
/// 不擋玩家出生點與起始農莊。
const HOME_CLEARANCE: f32 = 2.0 * TILE_SIZE;

/// 世界裡一個採集節點的落點：在哪、是什麼種類。接線時每筆配一個
/// `gather::ResourceNode::new(kind)` 成為實際可採的節點。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeSpawn {
    pub x: f32,
    pub y: f32,
    pub kind: NodeKind,
}

/// 家園淨空矩形 `(x, y, width, height)`：序號 0 地塊footprint往四周各擴 `HOME_CLEARANCE`。
/// 對外公開讓接線 / 測試共用單一真實來源，不各自重算一套。
#[allow(dead_code)] // 接線輪才有呼叫端；沿用本專案前置地基的慣例。
pub fn home_clearance_rect() -> (f32, f32, f32, f32) {
    (
        FIELD_ORIGIN_X - HOME_CLEARANCE,
        FIELD_ORIGIN_Y - HOME_CLEARANCE,
        PLOT_WIDTH + 2.0 * HOME_CLEARANCE,
        PLOT_HEIGHT + 2.0 * HOME_CLEARANCE,
    )
}

/// 某座標是否落在家園淨空圈內（落在裡面的格就不放節點）。
fn in_home_clearance(x: f32, y: f32) -> bool {
    let (rx, ry, rw, rh) = home_clearance_rect();
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

/// 座標雜湊：把格座標 `(col, row)` 混成一個 64-bit 值，用來確定性地決定
/// 放不放、放哪種、格內落點。用 FNV-1a 起手再做兩輪 bit 混合，讓相鄰格結果夠分散。
fn hash_cell(col: u32, row: u32) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for v in [col, row] {
        h ^= v as u64;
        h = h.wrapping_mul(0x0000_0001_0000_01b3); // FNV prime
    }
    // 額外的 splitmix64 風格混合，避免低位規律。
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h
}

/// 由雜湊值取 `[0, 1)` 區間的小數（取一段位元），給格內 jitter 用。
fn frac_from(bits: u64) -> f32 {
    (bits % 1000) as f32 / 1000.0
}

/// 生出世界的採集節點落點清單。完全確定性：同樣的常數每次跑出同一張節點圖。
///
/// 走訪每一格 → 雜湊決定是否放節點 → 放的話在格內 jitter 出落點、依雜湊挑種類 →
/// 落在家園淨空圈內就跳過。座標保證落在世界範圍內（格與內縮都在界內算）。
#[allow(dead_code)] // 接線輪（伺服器啟動撒佈節點）才有呼叫端。
pub fn scatter_nodes() -> Vec<NodeSpawn> {
    let cell_w = WORLD_WIDTH / GRID_COLS as f32;
    let cell_h = WORLD_HEIGHT / GRID_ROWS as f32;
    let mut spawns = Vec::new();

    for row in 0..GRID_ROWS {
        for col in 0..GRID_COLS {
            let h = hash_cell(col, row);
            // 是否在這格放節點。
            if h % 100 >= NODE_DENSITY_PCT {
                continue;
            }
            // 格內 jitter：用不同位元段取 x / y 落點，留 `CELL_MARGIN` 內縮。
            let span_x = (cell_w - 2.0 * CELL_MARGIN).max(0.0);
            let span_y = (cell_h - 2.0 * CELL_MARGIN).max(0.0);
            let x = col as f32 * cell_w + CELL_MARGIN + frac_from(h >> 8) * span_x;
            let y = row as f32 * cell_h + CELL_MARGIN + frac_from(h >> 24) * span_y;

            if in_home_clearance(x, y) {
                continue;
            }

            // 依雜湊挑種類（取高位段，與放置 / jitter 用的位元錯開）。
            let kind = match (h >> 40) % 3 {
                0 => NodeKind::Tree,
                1 => NodeKind::Rock,
                _ => NodeKind::EtherOre,
            };
            spawns.push(NodeSpawn { x, y, kind });
        }
    }

    spawns
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 撒佈完全確定性：兩次呼叫得到一模一樣的節點圖（重啟後節點不亂跳）。
    #[test]
    fn scatter_is_deterministic() {
        assert_eq!(scatter_nodes(), scatter_nodes());
    }

    /// 至少要生出一些節點（密度設定不能讓世界空無一物）。
    #[test]
    fn scatter_produces_nodes() {
        assert!(!scatter_nodes().is_empty(), "世界沒生出任何採集節點");
    }

    /// 每個節點都落在世界範圍內（不會生到地圖外採不到）。
    #[test]
    fn all_nodes_within_world_bounds() {
        for n in scatter_nodes() {
            assert!(
                n.x >= 0.0 && n.x <= WORLD_WIDTH && n.y >= 0.0 && n.y <= WORLD_HEIGHT,
                "節點落到世界外：{:?}",
                n
            );
        }
    }

    /// 沒有節點生在家園淨空圈內（不擋玩家出生點 / 起始農莊）。
    #[test]
    fn no_node_in_home_clearance() {
        for n in scatter_nodes() {
            assert!(
                !in_home_clearance(n.x, n.y),
                "節點生在家園淨空圈內：{:?}",
                n
            );
        }
    }

    /// 世界中心（玩家出生點）落在家園淨空圈內——確認淨空確實罩住出生點。
    #[test]
    fn world_center_is_inside_home_clearance() {
        assert!(in_home_clearance(WORLD_WIDTH / 2.0, WORLD_HEIGHT / 2.0));
    }

    /// 三種節點都有撒出來（種類分配不會退化成只剩一種）。
    #[test]
    fn all_three_kinds_appear() {
        let spawns = scatter_nodes();
        for kind in [NodeKind::Tree, NodeKind::Rock, NodeKind::EtherOre] {
            assert!(
                spawns.iter().any(|n| n.kind == kind),
                "種類 {:?} 一個都沒撒出來",
                kind
            );
        }
    }

    /// 節點落點互異（同格至多一個、不同格落點不同，不會兩個節點疊在同一點）。
    #[test]
    fn node_positions_are_distinct() {
        let spawns = scatter_nodes();
        let mut seen = std::collections::HashSet::new();
        for n in &spawns {
            assert!(
                seen.insert((n.x.to_bits(), n.y.to_bits())),
                "兩個節點落在同一點：{:?}",
                n
            );
        }
    }
}
