//! 乙太方界·水中游魚 v1——世界第二種環境生物（自主提案切片，ROADMAP 848）。
//!
//! **真缺口**：野兔（847）讓陸地第一次「看得出有生機」，但世界的水域（湖泊/海灣，玩家早已
//! 靠水桶 794 舀水、靠釣竿垂釣的那片水）至今完全空蕩——除非魚上鉤那一刻，玩家從不曾**看見**
//! 水裡有魚。本模組讓水面下第一次有魚安靜地游動：一眼認出「這片水塘裡有魚」，也讓垂釣
//! 第一次有視覺線索（往魚多的地方甩竿），把「環境有生機」的軸線從陸地延伸進水域。
//!
//! **與野兔的刻意差異**（同軸線但不是重複）：野兔怕人、受驚逃跑；魚悠游不怕人（玩家隔著水面，
//! 不構成陸地上那種「逼近」的威脅感），純氛圍點綴——差異化的行為樹，而非換皮重貼。
//! 魚共用 `voxel_ws.rs` 的 `tick_wildlife`/`WildlifeAnimal`/`wildlife` hub 欄位與野兔同一套
//! 基礎設施（廣播/鎖序/暫態世界慣例），只是新增一個 `WildlifeKind::Fish` 分支——
//! 世界環境軸線第一次證明自己是可延伸的系統，不是野兔專屬的一次性特例。
//!
//! **刻意的範圍收斂**：純點綴、無 AI 大腦（零 LLM）、不影響釣魚機率/機制、無記憶、無持久化
//! （純記憶體，重啟於固定家域點重新生成，比照 `voxel_wildlife` 慣例）。
//!
//! ## 雨天魚群振奮 v1（自主提案切片，ROADMAP 1021）
//! 天氣（700/701/780/841/1020）至今碰過農地、居民、彩虹、垂釣機率、陸地野兔——唯獨水中
//! 游魚這一種環境生物，從沒對天氣有過任何反應：不管晴雨，魚永遠用同一個速度、同一圈
//! 半徑悠游，彷彿水面上下著雨這件事跟牠們毫無關係。**與 1020 野兔刻意方向相反**：野兔是
//! 陸地生物，淋雨不適、下雨時就地蜷縮躲避、動得更少；魚本就泡在水裡，雨滴打在水面淋不
//! 到牠們，反而更像現實裡的魚——下雨時游得更起勁、也更愛擠在一塊。世界第一次讓「陸」與
//! 「水」對同一場雨長出**方向相反**的兩種反應，而非同一招換皮重貼。也與 841（雨天垂釣，
//! 影響玩家拋竿收竿的**機率**、玩家看不見魚本身）刻意區隔：本節動的是魚**看得見的悠游
//! 動作**（速度＋聚攏半徑），兩者互不重疊。純加法：不新增任何持久化欄位，兩則純函式吃
//! 既有的 `raining` 快照即時算出，重啟／斷線零風險。

use crate::voxel::{self, SEA_LEVEL};

/// 魚悠游速度（方塊/秒）——比野兔閒晃（1.4）更慢，水阻感、不慌不忙。
pub const SWIM_SPEED: f32 = 0.9;
/// 魚閒晃半徑下限（方塊）：比野兔（1.5）更侷限，水塘通常不大，魚該待在塘裡打轉。
pub const WANDER_MIN_R: f32 = 0.8;
/// 魚閒晃半徑上限（方塊）。
pub const WANDER_MAX_R: f32 = 3.0;
/// 判定「這裡水夠深、魚游得起來」所需的最小水深（方塊）：地表到海平面至少差這麼多格，
/// 魚才有游動空間可看（太淺的水塘魚會貼著塘底，看起來像卡住）。
pub const MIN_WATER_DEPTH: i32 = 2;
/// 浮游高度與水面/塘底的最小餘裕（方塊）：避免魚的模型視覺上穿出水面或插進地形。
pub const CLEARANCE: f32 = 0.3;
/// 視為「已抵達閒晃目標」的水平距離門檻（比野兔的門檻稍緊，魚體型更小、閒晃半徑也更小）。
pub const ARRIVE_DIST: f32 = 0.3;

/// 這個世界座標（水平格）此刻是否「水夠深，適合魚游」。
/// 用 [`MIN_WATER_DEPTH`] 換算：地表高度需低於海平面至少這麼多格。
/// 純函式、確定性，供 `voxel_ws.rs` 挑選/複驗魚的閒晃目標時呼叫。
pub fn is_deep_water(wx: i32, wz: i32) -> bool {
    voxel::height_at(wx, wz) <= SEA_LEVEL - MIN_WATER_DEPTH
}

/// 由 (ox,oz) 向外螺旋找第一處「水夠深」的水域，回傳魚的浮游座標 (x,y,z)
/// （鏡像 `voxel_residents::dry_ground_spawn` 找陸地的手法，這裡找水）。
///
/// 找不到就退回 (ox,oz)、y 取海平面下 1 格——地形起伏必然在合理範圍內產生窪地，
/// 這個退路只在極端巧合下才會用到，不會 panic、也不會讓魚憑空出現在天上。
pub fn wet_spot_spawn(ox: i32, oz: i32) -> (f32, f32, f32) {
    let (mut bx, mut bz, mut bh) = (ox, oz, voxel::height_at(ox, oz) - MIN_WATER_DEPTH);
    'search: for r in 0..48_i32 {
        for dx in -r..=r {
            for dz in -r..=r {
                if dx.abs().max(dz.abs()) != r {
                    continue;
                }
                let (x, z) = (ox + dx, oz + dz);
                let h = voxel::height_at(x, z);
                if h <= SEA_LEVEL - MIN_WATER_DEPTH {
                    bx = x;
                    bz = z;
                    bh = h;
                    break 'search;
                }
            }
        }
    }
    (bx as f32 + 0.5, mid_depth_y(bh), bz as f32 + 0.5)
}

/// 水深正中央的 y（地表到海平面之間取中點），讓魚離水面/塘底都有餘裕、不易穿模。
fn mid_depth_y(floor_h: i32) -> f32 {
    ((floor_h + 1) as f32 + SEA_LEVEL as f32) / 2.0
}

/// 把候選 y 夾回這一格當下的合法游動範圍內（地表之上、海平面之下，各留 [`CLEARANCE`] 餘裕）。
/// 供 tick 每格重新夾一次，容忍小範圍閒晃時地形深淺的細微起伏，不穿模。
pub fn clamp_swim_y(wx: i32, wz: i32, y: f32) -> f32 {
    let floor_h = voxel::height_at(wx, wz);
    let lo = floor_h as f32 + 1.0 + CLEARANCE;
    let hi = SEA_LEVEL as f32 - CLEARANCE;
    if lo > hi {
        // 水太淺塞不下餘裕（理論上不該挑中這種格，防禦性退回中點）。
        mid_depth_y(floor_h)
    } else {
        y.clamp(lo, hi)
    }
}

/// 純水平移動一步、無重力/無陸地碰撞（魚浮在水中，不需要陸地那套 AABB 碰撞——
/// 鏡像 `voxel_residents::step_toward` 的水平部分，但省去重力與實心方塊檢查）。
/// `speed` 由呼叫端傳入（晴天傳 [`SWIM_SPEED`]、雨天傳 [`effective_swim_speed`] 的結果），
/// 回傳新座標與是否已抵達（供呼叫端挑下一個閒晃目標）。純函式、確定性、可測。
pub fn swim_step(x: f32, z: f32, tx: f32, tz: f32, dt: f32, speed: f32) -> (f32, f32, bool) {
    let dx = tx - x;
    let dz = tz - z;
    let dist = (dx * dx + dz * dz).sqrt();
    if dist < ARRIVE_DIST {
        return (x, z, true);
    }
    let step = speed * dt;
    if step >= dist {
        (tx, tz, true)
    } else {
        (x + dx / dist * step, z + dz / dist * step, false)
    }
}

// ── 雨天魚群振奮 v1（自主提案切片，ROADMAP 1021）─────────────────────────────

/// 下雨時魚悠游速度的加成倍率——比平常更起勁地游動（陸地野兔遇雨變慢，水中魚遇雨反而更快，
/// 刻意方向相反）。
pub const RAIN_SWIM_SPEED_MULT: f32 = 1.6;

/// 依當前是否下雨，回傳這一刻魚該用的悠游速度（方塊/秒）。純函式、確定性，供 `voxel_ws.rs`
/// 每次呼叫 [`swim_step`] 前現算一次。
pub fn effective_swim_speed(raining: bool) -> f32 {
    if raining {
        SWIM_SPEED * RAIN_SWIM_SPEED_MULT
    } else {
        SWIM_SPEED
    }
}

/// 下雨時魚群閒晃的聚攏半徑上限（方塊）——比平常（[`WANDER_MAX_R`]）收斂許多，讓魚看起來
/// 擠得更緊，像被雨聲聚攏成一群，而非各自散開悠游。下限（[`WANDER_MIN_R`]）不變，魚不會被
/// 迫貼死不動。
pub const RAIN_WANDER_MAX_R: f32 = 1.6;

/// 依當前是否下雨，回傳這一刻挑選新閒晃目標時該用的半徑上限（方塊）。純函式、確定性。
pub fn effective_wander_max_r(raining: bool) -> f32 {
    if raining {
        RAIN_WANDER_MAX_R
    } else {
        WANDER_MAX_R
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deep_water_matches_depth_threshold() {
        for (x, z) in [(0, 0), (5, 5), (-30, 12), (100, -40)] {
            assert_eq!(
                is_deep_water(x, z),
                voxel::height_at(x, z) <= SEA_LEVEL - MIN_WATER_DEPTH,
                "({x},{z}) 的深水判定應精確對齊「地表 <= 海平面 - 最小深度」"
            );
        }
    }

    #[test]
    fn wet_spot_spawn_lands_on_deep_water() {
        for (ox, oz) in [(20, 20), (-20, 20), (20, -20), (-20, -20), (0, 40), (40, 0)] {
            let (x, y, z) = wet_spot_spawn(ox, oz);
            let (rx, rz) = (x.floor() as i32, z.floor() as i32);
            assert!(
                is_deep_water(rx, rz),
                "({ox},{oz}) 附近找到的浮游點 ({rx},{rz}) 應落在夠深的水域"
            );
            let floor_h = voxel::height_at(rx, rz);
            assert!(y > floor_h as f32 && y < SEA_LEVEL as f32 + 1.0, "y={y} 應浮在水面下、塘底上");
        }
    }

    #[test]
    fn wet_spot_spawn_is_deterministic() {
        let a = wet_spot_spawn(20, 20);
        let b = wet_spot_spawn(20, 20);
        assert_eq!(a, b);
    }

    #[test]
    fn clamp_swim_y_keeps_within_bounds_with_clearance() {
        // 隨便給一個離譜高的 y，clamp 後應落回海平面以下、留有餘裕。
        let y = clamp_swim_y(0, 0, 999.0);
        assert!(y <= SEA_LEVEL as f32 - CLEARANCE + 1e-4);
    }

    #[test]
    fn clamp_swim_y_keeps_within_bounds_from_below() {
        let y = clamp_swim_y(0, 0, -999.0);
        let floor_h = voxel::height_at(0, 0);
        assert!(y >= floor_h as f32 + 1.0 + CLEARANCE - 1e-4);
    }

    #[test]
    fn mid_depth_y_is_between_floor_and_sea_level() {
        let y = mid_depth_y(0);
        assert!(y > 1.0 && y < SEA_LEVEL as f32 + 1.0);
    }

    #[test]
    fn swim_step_arrives_when_already_close() {
        let (x, z, reached) = swim_step(0.0, 0.0, 0.1, 0.0, 1.0, SWIM_SPEED);
        assert!(reached);
        assert_eq!((x, z), (0.0, 0.0)); // 已抵達不移動，交回呼叫端挑下一個目標
    }

    #[test]
    fn swim_step_moves_toward_target_without_overshoot() {
        let (x, z, reached) = swim_step(0.0, 0.0, 10.0, 0.0, 0.1, SWIM_SPEED);
        assert!(!reached);
        assert!((x - SWIM_SPEED * 0.1).abs() < 1e-4);
        assert_eq!(z, 0.0);
    }

    #[test]
    fn swim_step_snaps_to_target_when_dt_overshoots() {
        let (x, z, reached) = swim_step(0.0, 0.0, 0.5, 0.0, 10.0, SWIM_SPEED); // 大 dt，一步跨過終點
        assert!(reached);
        assert_eq!((x, z), (0.5, 0.0));
    }

    #[test]
    fn swim_step_diagonal_direction_is_normalized() {
        let (x, z, reached) = swim_step(0.0, 0.0, 3.0, 4.0, 1.0, SWIM_SPEED); // 3-4-5 三角形，方便驗證單位向量
        assert!(!reached);
        let moved = (x * x + z * z).sqrt();
        assert!((moved - SWIM_SPEED).abs() < 1e-3);
        assert!((x / z - 3.0 / 4.0).abs() < 1e-3);
    }

    #[test]
    fn swim_step_respects_custom_speed() {
        // 傳入雨天加成後的速度，位移量應依新速度而非固定的 SWIM_SPEED 縮放。
        let speed = effective_swim_speed(true);
        let (x, z, reached) = swim_step(0.0, 0.0, 10.0, 0.0, 0.1, speed);
        assert!(!reached);
        assert!((x - speed * 0.1).abs() < 1e-4);
        assert_eq!(z, 0.0);
    }

    #[test]
    fn effective_swim_speed_faster_when_raining() {
        assert_eq!(effective_swim_speed(true), SWIM_SPEED * RAIN_SWIM_SPEED_MULT);
        assert!(effective_swim_speed(true) > SWIM_SPEED);
    }

    #[test]
    fn effective_swim_speed_normal_when_dry() {
        assert_eq!(effective_swim_speed(false), SWIM_SPEED);
    }

    #[test]
    fn effective_wander_max_r_tighter_when_raining() {
        assert_eq!(effective_wander_max_r(true), RAIN_WANDER_MAX_R);
        assert!(effective_wander_max_r(true) < WANDER_MAX_R);
        // 收斂後的上限仍必須大於下限，選半徑的算式才不會產生負區間。
        assert!(RAIN_WANDER_MAX_R > WANDER_MIN_R);
    }

    #[test]
    fn effective_wander_max_r_normal_when_dry() {
        assert_eq!(effective_wander_max_r(false), WANDER_MAX_R);
    }
}
