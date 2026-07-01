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

/// 夜間遮蔽（回自己蓋的小屋）時的閒晃半徑：刻意遠比 [`HOME_RADIUS`] 小，
/// 讓居民緊靠在自家附近，而非在整個家域內遊走——玩家一眼就能看出「牠回家了」。
pub const SHELTER_WANDER_RADIUS: f32 = 2.5;

/// 此刻是否該回自己蓋的小屋遮蔽（夜間 或 下雨 + 已知小屋座標）。純函式、可測。
///
/// 只是「回家附近待著」的行為判斷，不含任何路徑/物理——呼叫端拿到 `true` 後，
/// 改用小屋座標當閒晃中心＋ [`SHELTER_WANDER_RADIUS`]，其餘走既有 `wander_center`/
/// `wander_target`，零新路徑邏輯。
/// ROADMAP 701：下雨天氣（700）接上這裡——白天下雨時，已蓋好小屋的居民也會回家避雨，
/// 不必等到夜間；雨停/天亮則恢復正常閒晃。
pub fn should_shelter(is_night: bool, raining: bool, has_house: bool) -> bool {
    (is_night || raining) && has_house
}

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

// ── 卡住偵測 + 脫困/送回（修：只救「真被困」的，不打斷正常採集/建造）──────────────
//
// 玩家端有前端 depenetration（unstuckY），但居民是後端權威移動、沒有等價脫困。
// 這裡補上脫困，但偵測要**精準**：早期版本只看「想動卻 6 秒沒位移」就救，會把
// 居民「採集時故意停在資源旁挖方塊」「走向採不到的資源時頂著障礙」這類**正常停頓**
// 誤判成卡住 → 每分鐘誤救數次、打斷採集。
//
// 修正後分兩件事一起成立才算「真被困」：
//   ① **正在導航**（朝閒晃/歸巢目標走，不是在執行採集/蓋造動作——動作有各自逾時）；
//   ② **幾何困住**（埋在實心方塊裡，或四面（含踏階）都爬不出去）。
// 只是單側被擋（例如頂著一塊資源、其他方向能走）→ 不算被困、不誤救。
// 全是確定性純函式、可測。

/// 連續「正在導航卻零進展、且幾何上真被困」累積多少秒視為卡住（觸發脫困/送回）。
pub const STUCK_SECS: f32 = 6.0;
/// 單 tick 水平位移小於此值視為「幾乎沒動」（還要同時 navigating + confined 才算卡住）。
pub const STUCK_MOVE_EPS: f32 = 0.02;
/// 往上脫困最多抬幾格找可站的地表空位（超過就改送回家）。
pub const UNSTUCK_MAX_LIFT: i32 = 6;

/// 幾何困住判定（純函式、可測）：居民此刻是否真的「爬不出去」。
/// ① 身體埋在實心方塊裡（被方塊覆蓋）→ 一定要救；
/// ② 否則朝四個水平方向各試探一步（含踏上一階），任一方向走得出去 → **不算**被困；
/// ③ 四向（含踏階）全部被擋 → 卡在爬不出的坑/箱裡。
/// 關鍵：採集時頂著一塊資源（只有單側被擋、其他方向是空地）→ 回 `false`，不誤救。
pub fn is_confined(world: &WorldDelta, body: &Body) -> bool {
    // 埋在實心方塊裡（被覆蓋）→ 一定要救。
    if overlaps(world, body.x, body.y, body.z) {
        return true;
    }
    // 步距取略大於 AABB 半寬，確保真的探進鄰格、不會原地打轉誤判。
    const PROBE: f32 = RES_HALF_W + 0.2;
    for (dx, dz) in [(PROBE, 0.0), (-PROBE, 0.0), (0.0, PROBE), (0.0, -PROBE)] {
        // 平移過去不撞實心 → 走得出去，不算被困。
        if !overlaps(world, body.x + dx, body.y, body.z + dz) {
            return false;
        }
        // 踏上一階再過去不撞實心 → 爬得出淺坑（≤1 格），也不算被困。
        if !overlaps(world, body.x + dx, body.y + 1.05, body.z + dz) {
            return false;
        }
    }
    // 四個方向（含踏階）都出不去 → 卡在爬不出的坑/箱裡。
    true
}

/// 一次脫困的結果（供呼叫端冒泡/記 feed/log）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rescue {
    /// 埋在實心方塊裡 → 往上頂到最近地表空位（depenetration）。
    LiftedUp,
    /// 頂不出來、或卡在爬不出的坑裡 → 送回家域出生地表。
    SentHome,
}

/// 更新「卡住計時」。只有三件事**同時**成立才累加 dt：
///   ① `navigating`：居民正朝某導航目標走（閒晃/歸巢），**不是**在執行採集/蓋造動作
///      （採集/蓋造的故意停頓有各自的逾時處理，不該被當成卡住）；
///   ② `confined`：此刻幾何上**真的被困**（見 [`is_confined`]）；
///   ③ 幾乎沒位移（`moved_dist < STUCK_MOVE_EPS`）。
/// 其餘情況（在動、原地歇息、執行動作中的停頓、只是單側被擋）一律歸零——不誤救。
/// 純函式、可測。
pub fn update_stuck_timer(prev: f32, moved_dist: f32, navigating: bool, confined: bool, dt: f32) -> f32 {
    if navigating && confined && moved_dist < STUCK_MOVE_EPS {
        prev + dt
    } else {
        0.0
    }
}

/// 嘗試把「埋在實心方塊裡」的居民往上頂到最近可站的地表（比照玩家 depenetration）。
/// 一開始就沒重疊（沒被埋）→ 直接回 `true`（無需脫困）。
/// 從目前腳底往上找最多 `max_lift` 格，第一個「整個身體不再重疊實心」的高度就站上去
/// （vy 歸零、交給重力落穩）。`max_lift` 內都頂不出來 → 回 `false`（交給送回家）。
pub fn try_unstuck_up(world: &WorldDelta, body: &mut Body, max_lift: i32) -> bool {
    if !overlaps(world, body.x, body.y, body.z) {
        return true;
    }
    let base = body.y.floor() as i32;
    for up in 1..=max_lift {
        let ny = (base + up) as f32;
        if !overlaps(world, body.x, ny, body.z) {
            body.y = ny;
            body.vy = 0.0;
            return true;
        }
    }
    false
}

/// 脫困一位卡住的居民：① 若埋在實心方塊裡，先試往上頂到地表（LiftedUp）；
/// ② 沒被埋（卡在爬不出的坑裡）或頂不出來 → 送回家域 (hx,hz) 的出生地表（SentHome）。
/// 確定性、不碰鎖/IO（dry_ground_spawn 為純函式），守無鎖 await 鐵律。
pub fn rescue_resident(world: &WorldDelta, body: &mut Body, hx: f32, hz: f32, max_lift: i32) -> Rescue {
    if overlaps(world, body.x, body.y, body.z) && try_unstuck_up(world, body, max_lift) {
        return Rescue::LiftedUp;
    }
    *body = dry_ground_spawn(hx.floor() as i32, hz.floor() as i32);
    Rescue::SentHome
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

    // ── 卡住偵測 + 脫困/送回 ──────────────────────────────────────────────────

    /// 實測對比（`cargo test -- --nocapture` 看數字）：模擬 prod 真因——居民鎖定
    /// 一個「採不到的資源」（被牆/地形擋住），整段頂著障礙零位移。
    /// 舊規則（gather.is_some() ⇒ trying_to_move）會把這誤判成卡住、每分鐘狂救；
    /// 新規則（採集動作豁免 + 幾何困住判定）→ 0 次誤救。
    #[test]
    fn false_rescue_rate_before_vs_after_on_gather_pause() {
        let mut world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // +x 放一面 2 格高的牆當「採不到、頂著的資源」。
        for up in 0..=1 {
            voxel::set_block(&mut world, x + 1, h + 1 + up, z, Block::Stone);
        }
        let dt = 1.0 / 30.0;
        let secs = 600.0_f32; // 模擬 10 分鐘採集頂牆
        let ticks = (secs / dt) as u32;

        // 舊規則：採集中視為 trying_to_move、沒有幾何判定。
        let mut old_body = Body::at(x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        let mut old_stuck = 0.0_f32;
        let mut old_rescues = 0u32;
        // 新規則：採集動作豁免（navigating=false）。
        let mut new_body = Body::at(x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        let mut new_stuck = 0.0_f32;
        let mut new_rescues = 0u32;

        for _ in 0..ticks {
            // 舊規則 tick。
            let (px, pz) = (old_body.x, old_body.z);
            step_toward(&world, &mut old_body, x as f32 + 2.0, z as f32 + 0.5, dt, RES_SPEED);
            let moved = ((old_body.x - px).powi(2) + (old_body.z - pz).powi(2)).sqrt();
            // 舊邏輯：gather.is_some() ⇒ 視為 trying_to_move、無 confined 把關。
            old_stuck = if moved < STUCK_MOVE_EPS { old_stuck + dt } else { 0.0 };
            if old_stuck >= STUCK_SECS { old_rescues += 1; old_stuck = 0.0; }

            // 新規則 tick。
            let (px2, pz2) = (new_body.x, new_body.z);
            step_toward(&world, &mut new_body, x as f32 + 2.0, z as f32 + 0.5, dt, RES_SPEED);
            let moved2 = ((new_body.x - px2).powi(2) + (new_body.z - pz2).powi(2)).sqrt();
            let navigating = false; // 採集動作中 → 豁免
            let confined = navigating && is_confined(&world, &new_body);
            new_stuck = update_stuck_timer(new_stuck, moved2, navigating, confined, dt);
            if new_stuck >= STUCK_SECS { new_rescues += 1; new_stuck = 0.0; }
        }

        let per_min = |n: u32| n as f32 / (secs / 60.0);
        println!(
            "[脫困誤救對比] 採集頂牆 {secs}s：舊規則救={old_rescues}（{:.1}/分） 新規則救={new_rescues}（{:.1}/分）",
            per_min(old_rescues), per_min(new_rescues)
        );
        // 舊規則確實會周期性誤救（重現 prod 的「跟著採集冒出來」）。
        assert!(old_rescues > 0, "舊規則應重現採集誤救");
        // 新規則：採集停頓 0 誤救。
        assert_eq!(new_rescues, 0, "新規則採集停頓不該誤救：new_rescues={new_rescues}");
    }

    #[test]
    fn stuck_timer_accumulates_only_when_navigating_confined_and_still() {
        let dt = 1.0 / 30.0;
        // 正在導航 + 幾何被困 + 幾乎沒位移 → 累加。
        let t = update_stuck_timer(0.0, 0.0, true, true, dt);
        assert!((t - dt).abs() < 1e-6);
        // 連續累加數 tick。
        let t2 = update_stuck_timer(t, 0.001, true, true, dt);
        assert!(t2 > t);
    }

    #[test]
    fn stuck_timer_resets_when_moving_resting_or_not_confined() {
        let dt = 1.0 / 30.0;
        // 有在動（位移超過門檻）→ 歸零。
        assert_eq!(update_stuck_timer(5.0, 0.5, true, true, dt), 0.0);
        // 沒在導航（執行採集/蓋造動作中的停頓，或原地歇息）→ 歸零。
        assert_eq!(update_stuck_timer(5.0, 0.0, false, true, dt), 0.0);
        // 在導航、沒位移，但幾何上沒被困（只是單側被擋）→ 歸零（不誤救）。
        assert_eq!(update_stuck_timer(5.0, 0.0, true, false, dt), 0.0);
    }

    // ── 幾何困住判定 is_confined ───────────────────────────────────────────────

    #[test]
    fn is_confined_false_on_open_ground() {
        let world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // 站在開闊地表上方 → 四面都能走 → 不算被困。
        let body = Body::at(x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        assert!(!is_confined(&world, &body), "開闊地表不該判為被困");
    }

    #[test]
    fn is_confined_true_when_buried() {
        let world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // 腳底卡進地表方塊裡（被覆蓋）→ 一定算被困。
        let body = Body::at(x as f32 + 0.5, h as f32, z as f32 + 0.5);
        assert!(overlaps(&world, body.x, body.y, body.z), "前置：身體應被覆蓋");
        assert!(is_confined(&world, &body), "埋在實心裡應判為被困");
    }

    #[test]
    fn is_confined_false_when_only_one_side_blocked() {
        // 採集誤救真因的釘樁：居民頂著一塊資源（只有單側被擋、其他方向是空地）。
        // 舊邏輯會把這當卡住誤救；新邏輯幾何判定 → 不被困。
        let mut world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        let body = Body::at(x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        // 只在 +x 方向身體高度放一面牆（模擬頂著的資源/障礙）。
        for up in 0..=1 {
            voxel::set_block(&mut world, x + 1, h + 1 + up, z, Block::Stone);
        }
        assert!(!overlaps(&world, body.x, body.y, body.z), "前置：身體本身沒被覆蓋");
        assert!(!is_confined(&world, &body), "單側被擋、其他方向能走 → 不該判為被困");
    }

    #[test]
    fn is_confined_false_in_shallow_one_deep_pit() {
        // 只有 1 格高的牆圍著（淺坑）→ 踏一階就出得去 → 不算被困。
        let mut world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            voxel::set_block(&mut world, x + dx, h + 1, z + dz, Block::Stone);
        }
        let body = Body::at(x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        assert!(!is_confined(&world, &body), "1 格淺坑踏階出得去 → 不該判為被困");
    }

    #[test]
    fn is_confined_true_in_deep_walled_pit() {
        // 四面高牆（≥2 格）圍住、爬不出 → 真被困。
        let mut world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            for up in 1..=4 {
                voxel::set_block(&mut world, x + dx, h + up, z + dz, Block::Stone);
            }
        }
        let body = Body::at(x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        assert!(!overlaps(&world, body.x, body.y, body.z), "前置：站在坑底空氣裡");
        assert!(is_confined(&world, &body), "高牆深坑爬不出 → 應判為被困");
    }

    #[test]
    fn gathering_pause_against_resource_never_triggers_rescue() {
        // 端到端釘樁：居民走向「採不到的資源」頂著障礙、整段零位移——
        // 因為這是執行採集動作（navigating=false），卡住計時永不累加 → 不誤救。
        let mut world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // +x 放一面牆當「頂著的資源」。
        for up in 0..=1 {
            voxel::set_block(&mut world, x + 1, h + 1 + up, z, Block::Stone);
        }
        let mut body = Body::at(x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        let dt = 1.0 / 30.0;
        let mut stuck = 0.0_f32;
        let mut rescues = 0u32;
        // 模擬 20 秒：採集中（navigating=false），一直頂著牆零位移。
        for _ in 0..600 {
            let (px, pz) = (body.x, body.z);
            // 頂著資源：朝牆走（被擋、幾乎不動）。
            step_toward(&world, &mut body, x as f32 + 2.0, z as f32 + 0.5, dt, RES_SPEED);
            let moved = ((body.x - px).powi(2) + (body.z - pz).powi(2)).sqrt();
            let navigating = false; // 採集動作中，豁免脫困偵測
            let confined = navigating && is_confined(&world, &body);
            stuck = update_stuck_timer(stuck, moved, navigating, confined, dt);
            if stuck >= STUCK_SECS {
                rescues += 1;
                stuck = 0.0;
            }
        }
        assert_eq!(rescues, 0, "採集頂著資源的正常停頓不該觸發脫困：rescues={rescues}");
    }

    #[test]
    fn truly_trapped_resident_while_navigating_is_still_rescued() {
        // 對照組：真被困（四面高牆）+ 正在導航 → 6 秒後仍會觸發脫困（真卡住要救）。
        let mut world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            for up in 1..=4 {
                voxel::set_block(&mut world, x + dx, h + up, z + dz, Block::Stone);
            }
        }
        let mut body = Body::at(x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        let dt = 1.0 / 30.0;
        let mut stuck = 0.0_f32;
        let mut rescued = false;
        for _ in 0..300 {
            let (px, pz) = (body.x, body.z);
            // 正在導航：朝坑外某點走（被牆擋住零進展）。
            step_toward(&world, &mut body, x as f32 + 5.0, z as f32 + 0.5, dt, RES_SPEED);
            let moved = ((body.x - px).powi(2) + (body.z - pz).powi(2)).sqrt();
            let navigating = true;
            let confined = navigating && is_confined(&world, &body);
            stuck = update_stuck_timer(stuck, moved, navigating, confined, dt);
            if stuck >= STUCK_SECS {
                rescued = true;
                break;
            }
        }
        assert!(rescued, "真被困（高牆）且在導航 → 應在門檻時間內觸發脫困");
    }

    #[test]
    fn try_unstuck_up_noop_when_free() {
        let world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // 站在地表上方（沒被埋）→ 不需脫困、位置不變。
        let mut body = Body::at(x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        let y0 = body.y;
        assert!(try_unstuck_up(&world, &mut body, UNSTUCK_MAX_LIFT));
        assert_eq!(body.y, y0, "沒被埋不該移動");
    }

    #[test]
    fn try_unstuck_up_lifts_buried_body() {
        let world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // 腳底卡進地表方塊裡（y=h，身體與實心重疊）。
        let mut body = Body::at(x as f32 + 0.5, h as f32, z as f32 + 0.5);
        assert!(overlaps(&world, body.x, body.y, body.z), "前置：身體應卡在地裡");
        assert!(try_unstuck_up(&world, &mut body, UNSTUCK_MAX_LIFT), "應頂得出來");
        assert!(!overlaps(&world, body.x, body.y, body.z), "脫困後不應再重疊實心");
    }

    #[test]
    fn rescue_buried_resident_lifts_up() {
        let world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        let mut body = Body::at(x as f32 + 0.5, h as f32, z as f32 + 0.5);
        let r = rescue_resident(&world, &mut body, x as f32, z as f32, UNSTUCK_MAX_LIFT);
        assert_eq!(r, Rescue::LiftedUp, "埋在地裡應往上脫困");
        assert!(!overlaps(&world, body.x, body.y, body.z));
    }

    #[test]
    fn rescue_pit_trapped_resident_sends_home() {
        // 居民「沒被埋」但卡在爬不出的坑裡（站在坑底空氣中）→ 送回家域出生地表。
        let mut world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // 在 (x,z) 周圍築一圈高牆，圍出一個爬不出的坑；居民站在坑底（地表上方、沒被埋）。
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            for up in 1..=4 {
                voxel::set_block(&mut world, x + dx, h + up, z + dz, Block::Stone);
            }
        }
        let mut body = Body::at(x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        // 前置：身體本身沒卡進實心（站在坑底空氣裡）。
        assert!(!overlaps(&world, body.x, body.y, body.z));
        // 家域中心放在遠處平地（送回目標）。
        let (hx, hz) = land_point();
        let r = rescue_resident(&world, &mut body, hx as f32 + 50.0, hz as f32, UNSTUCK_MAX_LIFT);
        assert_eq!(r, Rescue::SentHome, "坑裡爬不出應送回家");
        // 送回後不卡在實心方塊裡。
        assert!(!overlaps(&world, body.x, body.y, body.z));
    }

    #[test]
    fn resident_home_base_wraps_modulo() {
        // i % 4 讓超過 4 的 index 循環，與 i<4 結果一致。
        for i in 0..4_usize {
            assert_eq!(resident_home_base(i), resident_home_base(i + 4));
        }
    }

    // ── should_shelter：夜間歸巢遮蔽判斷 ──────────────────────────────────────

    #[test]
    fn should_shelter_only_when_night_and_has_house() {
        assert!(should_shelter(true, false, true), "夜間 + 有小屋 → 該遮蔽");
        assert!(!should_shelter(false, false, true), "白天不下雨不遮蔽，就算有小屋");
        assert!(!should_shelter(true, false, false), "夜間但沒蓋過小屋 → 沒地方回，不遮蔽");
        assert!(!should_shelter(false, false, false), "白天且沒小屋 → 不遮蔽");
    }

    #[test]
    fn should_shelter_also_true_when_raining_daytime() {
        // ROADMAP 701：白天下雨 + 有小屋 → 也該遮蔽（不必等到夜間）。
        assert!(should_shelter(false, true, true), "白天下雨 + 有小屋 → 該遮蔽");
        assert!(!should_shelter(false, true, false), "白天下雨但沒小屋 → 沒地方躲，不遮蔽");
        assert!(should_shelter(true, true, true), "夜間+下雨+有小屋 → 仍該遮蔽（不重複計算/不衝突）");
    }

    #[test]
    fn shelter_wander_radius_is_smaller_than_home_radius() {
        // 遮蔽半徑該遠比家域半徑小，讓居民緊靠在自家附近（一眼看得出「回家了」）。
        assert!(SHELTER_WANDER_RADIUS > 0.0);
        assert!(SHELTER_WANDER_RADIUS < HOME_RADIUS);
    }
}
