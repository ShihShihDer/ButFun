//! 乙太方界 AI 居民——純邏輯地基（切片③：靈魂搬進 voxel 新世界）。
//!
//! 這裡只放「與連線/鎖無關」的確定性純邏輯：居民在 voxel 世界裡的**物理**
//! （重力 + 逐軸 AABB 碰撞 + 自動踏階，比照玩家那套 voxel 碰撞）與**閒晃目標**選取。
//! 真正的 tick 驅動、廣播、無鎖 async 思考都在 `voxel_ws.rs`（嚴守 prod 死鎖鐵律：
//! 短鎖快照 → drop → spawn 思考 → 下一 tick 套用，絕不持鎖 await）。
//!
//! 全部抽成可測純函式，碰不到 hub / 連線 / LLM；不抄外部碼、繁中註解。

use crate::voxel::{self, WorldDelta, SEA_LEVEL};

/// 居民 AABB 半寬（與前端玩家 PW 一致）。
pub const RES_HALF_W: f32 = 0.3;
/// 居民身高（與前端玩家 PH 一致）。
pub const RES_HEIGHT: f32 = 1.7;
/// 居民行走速度（方塊/秒）。刻意比玩家(5.0)慢，走出閒適的「居民散步」感。
pub const RES_SPEED: f32 = 2.6;
/// 重力加速度（方塊/秒²），與前端一致。
pub const GRAVITY: f32 = 24.0;
/// 視為「已抵達目標」的水平距離門檻。
pub const ARRIVE_DIST: f32 = 0.6;

/// 居民的物理身體：腳底位置 (x,y,z) + 垂直速度 + 是否著地。
/// 與前端 `player` 同一套語意（y = 腳底），讓伺服器權威與前端渲染天然對齊。
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub vy: f32,
    pub grounded: bool,
}

impl Body {
    /// 在指定腳底座標建一個靜止身體。
    pub fn at(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z, vy: 0.0, grounded: false }
    }
}

/// 某世界座標是否為「會擋路」的實心方塊（套 delta overlay；水與空氣不擋）。
#[inline]
fn solid(world: &WorldDelta, wx: i32, wy: i32, wz: i32) -> bool {
    voxel::effective_block_at(world, wx, wy, wz).is_solid()
}

/// 居民 AABB（在腳底座標 (x,y,z)）是否與任一實心方塊重疊。
/// 鏡像前端 `overlaps()`：掃 AABB 覆蓋到的整數方塊格。純函式、可測。
pub fn overlaps(world: &WorldDelta, x: f32, y: f32, z: f32) -> bool {
    let x0 = (x - RES_HALF_W).floor() as i32;
    let x1 = (x + RES_HALF_W).floor() as i32;
    let y0 = y.floor() as i32;
    let y1 = (y + RES_HEIGHT - 0.01).floor() as i32;
    let z0 = (z - RES_HALF_W).floor() as i32;
    let z1 = (z + RES_HALF_W).floor() as i32;
    for bx in x0..=x1 {
        for by in y0..=y1 {
            for bz in z0..=z1 {
                if solid(world, bx, by, bz) {
                    return true;
                }
            }
        }
    }
    false
}

/// 沿單一水平軸移動（dx、dz 其中一個為 0）：撞牆就回退；若著地，試著踏上 1 格台階
/// （讓走斜坡/小丘順暢）。鏡像前端 `moveAxis`，逐軸分開呼叫 → 沿牆滑行不卡死。
fn move_axis(world: &WorldDelta, body: &mut Body, dx: f32, dz: f32) {
    if dx == 0.0 && dz == 0.0 {
        return;
    }
    let (px, pz) = (body.x, body.z);
    body.x += dx;
    body.z += dz;
    if !overlaps(world, body.x, body.y, body.z) {
        return;
    }
    if body.grounded {
        let py = body.y;
        body.y += 1.05;
        if !overlaps(world, body.x, body.y, body.z) {
            return; // 踏上台階成功（抬高 y，之後重力會落穩）
        }
        body.y = py;
    }
    // 完全擋住 → 回退這一軸。
    body.x = px;
    body.z = pz;
}

/// 套用重力 + 垂直碰撞（鏡像前端：限制單幀垂直位移避免穿牆，落地歸零、掉出世界拉回）。
pub fn gravity_step(world: &WorldDelta, body: &mut Body, dt: f32) {
    body.vy -= GRAVITY * dt;
    let dy = (body.vy * dt).clamp(-1.5, 1.5);
    let prev_y = body.y;
    body.y += dy;
    if overlaps(world, body.x, body.y, body.z) {
        body.y = prev_y;
        if body.vy < 0.0 {
            body.grounded = true;
        }
        body.vy = 0.0;
    } else if body.vy < 0.0 {
        body.grounded = false;
    }
    // 掉出世界保險：低於 -10 拉回高空，重力會重新落穩。
    if body.y < -10.0 {
        body.y = 40.0;
        body.vy = 0.0;
    }
}

/// 朝水平目標 (tx,tz) 走一步並套重力。回傳是否已抵達（水平距離 < `ARRIVE_DIST`）。
/// 已抵達就不再水平移動（只落重力），交回呼叫端挑下一個目標。
/// `speed` 由呼叫端傳入（一般為 `RES_SPEED`，夜間可乘以日夜作息乘數縮減）。
pub fn step_toward(world: &WorldDelta, body: &mut Body, tx: f32, tz: f32, dt: f32, speed: f32) -> bool {
    let dx = tx - body.x;
    let dz = tz - body.z;
    let dist = (dx * dx + dz * dz).sqrt();
    let reached = dist < ARRIVE_DIST;
    if !reached && dist > 1e-4 {
        let scale = (speed * dt) / dist;
        // 逐軸移動（沿牆滑行、台階自動踏上）。
        move_axis(world, body, dx * scale, 0.0);
        move_axis(world, body, 0.0, dz * scale);
    }
    gravity_step(world, body, dt);
    reached
}

/// 由水平位移量算朝向 yaw（與前端 `bodyMesh.rotation.y = atan2(dir.x, dir.z)` 對齊）。
/// 位移過小回 `None`（保留原朝向，避免抖動）。
pub fn yaw_from_move(dx: f32, dz: f32) -> Option<f32> {
    if dx * dx + dz * dz > 1e-6 {
        Some(dx.atan2(dz))
    } else {
        None
    }
}

/// 在 (cx,cz) 周圍依給定角度/半徑算出一個閒晃目標 (tx,tz)（純函式、可測）。
/// 隨機性由呼叫端提供（angle/radius），本函式只做幾何，方便單元測試釘住。
pub fn wander_target(cx: f32, cz: f32, angle: f32, radius: f32) -> (f32, f32) {
    (cx + angle.cos() * radius, cz + angle.sin() * radius)
}

/// 居民家域半徑（方塊）：超出此距離時歸巢（以家為閒晃中心），在內時自由閒晃。
pub const HOME_RADIUS: f32 = 20.0;

/// 取閒晃中心：若居民超出家域半徑，以家 (hx,hz) 為中心（引導歸巢），
/// 否則以當前位置 (cx,cz) 為中心（在家域內自由閒晃）。純函式、可測。
pub fn wander_center(cx: f32, cz: f32, hx: f32, hz: f32, home_radius: f32) -> (f32, f32) {
    let dx = cx - hx;
    let dz = cz - hz;
    if dx * dx + dz * dz > home_radius * home_radius {
        (hx, hz)
    } else {
        (cx, cz)
    }
}

/// 各居民家域中心的世界座標基準（i % 4）：4 位居民分散四方，玩家需探索才能遇到。
/// 露娜在原點（玩家出生附近），其餘三位分別在南/西/東方 75 格。純函式、可測。
pub fn resident_home_base(i: usize) -> (i32, i32) {
    match i % 4 {
        0 => (0, 0),    // 露娜：世界中心，出生就能遇
        1 => (0, 75),   // 諾娃：南方（農田、田園感）
        2 => (-75, 0),  // 賽勒：西方（海灣、探索感）
        _ => (75, 0),   // 奧瑞：東方（山林、遠足感）
    }
}

/// 居民出生點：自 (ox,oz) 向外螺旋找第一塊「高於海平面的陸地」，站到地表上方
/// （確保不卡水/土裡；對齊 `voxel_ws::spawn_pos` 的找地策略）。找不到就退回 (ox,oz)。
pub fn dry_ground_spawn(ox: i32, oz: i32) -> Body {
    let (mut bx, mut bz, mut bh) = (ox, oz, voxel::height_at(ox, oz));
    'search: for r in 0..48_i32 {
        for dx in -r..=r {
            for dz in -r..=r {
                if dx.abs().max(dz.abs()) != r {
                    continue;
                }
                let (x, z) = (ox + dx, oz + dz);
                let h = voxel::height_at(x, z);
                if h > SEA_LEVEL + 1 {
                    bx = x;
                    bz = z;
                    bh = h;
                    break 'search;
                }
            }
        }
    }
    // 站在地表方塊「之上」：方塊 bh 頂面在 y=bh+1，多給 1 格餘裕讓重力落穩。
    Body::at(bx as f32 + 0.5, (bh + 2) as f32, bz as f32 + 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::{height_at, Block};

    /// 找一個明顯高於海平面的陸地點（給碰撞/重力測試一塊穩地）。
    fn land_point() -> (i32, i32) {
        for c in 0..3000 {
            if height_at(c, 0) > SEA_LEVEL + 3 {
                return (c, 0);
            }
        }
        (0, 0)
    }

    #[test]
    fn overlaps_ground_solid_air_empty() {
        let world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // 腳底正好在地表方塊內（y=h）→ 與實心地表重疊。
        assert!(overlaps(&world, x as f32 + 0.5, h as f32, z as f32 + 0.5));
        // 腳底站在地表之上一格（y=h+1）→ 身體在空氣裡、不重疊。
        assert!(!overlaps(&world, x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5));
    }

    #[test]
    fn gravity_settles_onto_ground() {
        let world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // 從地表上方數格落下，幾步後應穩穩著地、不再下沉、不穿進地裡。
        let mut body = Body::at(x as f32 + 0.5, (h + 4) as f32, z as f32 + 0.5);
        for _ in 0..120 {
            gravity_step(&world, &mut body, 1.0 / 30.0);
        }
        assert!(body.grounded, "落下後應著地");
        // 腳底應停在地表頂面附近（h+1），容一點碰撞餘裕。
        assert!(
            body.y >= h as f32 && body.y <= (h + 2) as f32,
            "腳底應停在地表頂面附近：y={} h={h}",
            body.y
        );
        // 站定後不應卡進實心方塊。
        assert!(!overlaps(&world, body.x, body.y, body.z));
    }

    #[test]
    fn step_toward_reaches_flat_target() {
        let world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        let mut body = Body::at(x as f32 + 0.5, (h + 2) as f32, z as f32 + 0.5);
        // 先落穩。
        for _ in 0..60 {
            gravity_step(&world, &mut body, 1.0 / 30.0);
        }
        // 目標放在腳邊一點點（同格附近，地形平緩）——數秒內應抵達。
        let (tx, tz) = (body.x + 0.4, body.z);
        let mut reached = false;
        for _ in 0..300 {
            if step_toward(&world, &mut body, tx, tz, 1.0 / 30.0, RES_SPEED) {
                reached = true;
                break;
            }
        }
        assert!(reached, "平地上應能走到近處目標");
        assert!(!overlaps(&world, body.x, body.y, body.z), "抵達後不應卡牆");
    }

    #[test]
    fn wander_target_geometry() {
        // 角度 0、半徑 5 → 純 +x 方向。
        let (tx, tz) = wander_target(10.0, 20.0, 0.0, 5.0);
        assert!((tx - 15.0).abs() < 1e-4);
        assert!((tz - 20.0).abs() < 1e-4);
        // 角度 π/2 → 純 +z 方向。
        let (tx2, tz2) = wander_target(0.0, 0.0, std::f32::consts::FRAC_PI_2, 4.0);
        assert!(tx2.abs() < 1e-3);
        assert!((tz2 - 4.0).abs() < 1e-3);
    }

    #[test]
    fn yaw_from_move_ignores_tiny() {
        assert_eq!(yaw_from_move(0.0, 0.0), None);
        // 純 +z（atan2(0, +) = 0）。
        assert_eq!(yaw_from_move(0.0, 1.0), Some(0.0));
        // 純 +x（atan2(+, 0) = π/2）。
        let y = yaw_from_move(1.0, 0.0).unwrap();
        assert!((y - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    }

    #[test]
    fn dry_ground_spawn_is_above_sea() {
        let body = dry_ground_spawn(0, 0);
        let h = height_at(body.x.floor() as i32, body.z.floor() as i32);
        assert!(h > SEA_LEVEL + 1, "出生點該在陸地：h={h}");
        assert!(body.y > h as f32, "出生點 Y 應在地表之上：y={} h={h}", body.y);
        // 出生身體不應一開始就卡進實心方塊。
        let world = WorldDelta::new();
        assert!(!overlaps(&world, body.x, body.y, body.z));
    }

    #[test]
    fn delta_block_blocks_resident() {
        // 在腳邊放一塊 delta 石頭 → 居民 AABB 應偵測到實心、被擋。
        let mut world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // 站在地表上方。
        let fx = x as f32 + 0.5;
        let fz = z as f32 + 0.5;
        assert!(!overlaps(&world, fx, (h + 1) as f32, fz));
        // 在身體所在格放石頭 → 重疊。
        voxel::set_block(&mut world, x, h + 1, z, Block::Stone);
        assert!(overlaps(&world, fx, (h + 1) as f32, fz));
    }

    #[test]
    fn wander_center_within_home_uses_current() {
        // 居民在家域半徑內 → 以當前位置為中心（自由閒晃）。
        let (cx, cz) = (5.0_f32, 5.0_f32);
        let (hx, hz) = (0.0_f32, 0.0_f32);
        let r = 20.0_f32;
        // 距家 ~7 格 < 20 → 回當前位置。
        let (wx, wz) = wander_center(cx, cz, hx, hz, r);
        assert!((wx - cx).abs() < 1e-4, "在家域內應回當前 x");
        assert!((wz - cz).abs() < 1e-4, "在家域內應回當前 z");
    }

    #[test]
    fn wander_center_outside_home_uses_home() {
        // 居民超出家域半徑 → 以家為中心（引導歸巢）。
        let (cx, cz) = (30.0_f32, 0.0_f32);
        let (hx, hz) = (0.0_f32, 0.0_f32);
        let r = 20.0_f32;
        // 距家 30 > 20 → 回家座標。
        let (wx, wz) = wander_center(cx, cz, hx, hz, r);
        assert!((wx - hx).abs() < 1e-4, "超出家域應回家 x");
        assert!((wz - hz).abs() < 1e-4, "超出家域應回家 z");
    }

    #[test]
    fn wander_center_at_exact_boundary_uses_current() {
        // 恰好在邊界上（距離 == home_radius）→ 不超出，用當前位置。
        let r = 20.0_f32;
        let (cx, cz) = (r, 0.0_f32); // 距家剛好 r
        let (hx, hz) = (0.0_f32, 0.0_f32);
        let (wx, wz) = wander_center(cx, cz, hx, hz, r);
        // 距離 == r，不 > r，故回當前。
        assert!((wx - cx).abs() < 1e-4, "邊界上應回當前 x");
        assert!((wz - cz).abs() < 1e-4, "邊界上應回當前 z");
    }

    #[test]
    fn resident_home_base_four_directions() {
        // 4 位居民的家基準各不相同、且至少 3 位不在原點。
        let homes: Vec<(i32, i32)> = (0..4).map(resident_home_base).collect();
        // 全部 4 個不全相同（有分散）。
        let unique: std::collections::HashSet<_> = homes.iter().collect();
        assert_eq!(unique.len(), 4, "4 位居民家基準應各不相同");
        // 除 i=0 外，其他 3 位應距原點至少 50 格。
        for (i, (hx, hz)) in homes.iter().enumerate().skip(1) {
            let d = ((hx * hx + hz * hz) as f32).sqrt();
            assert!(d >= 50.0, "居民 {i} 家基準距原點應 ≥ 50 格：d={d}");
        }
    }

    #[test]
    fn resident_home_base_wraps_modulo() {
        // i % 4 讓超過 4 的 index 循環，與 i<4 結果一致。
        for i in 0..4_usize {
            assert_eq!(resident_home_base(i), resident_home_base(i + 4));
        }
    }
}
