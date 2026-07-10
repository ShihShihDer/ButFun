//! 乙太方界暗影生物（Shadow Wisp）——「夜的張力」純邏輯（怪物/抵禦第一刀）。
//!
//! 設計調性是**療癒世界的夜之張力**，不是恐怖獵殺：
//!   - 只在夜間、遠離村莊與光源的暗處生成（全圖同時最多 [`MAX_WISPS`] 隻）。
//!   - 緩慢漂向最近的玩家/居民；觸碰玩家**緩慢**扣血（1 點/2 秒，配合既有溫柔重生，絕不秒殺）。
//!   - **光＝庇護**：火把/冰晶燈/乙太燈/營火半徑 [`LIGHT_RADIUS`] 內是亮區，
//!     暗影不進入、誤入即化成一縷輕煙消散——玩家夜裡沿著有燈的路走是安全的。
//!   - **牆＝屏障**：暗影是實體、不能穿實心方塊（同居民那套 AABB 碰撞），躲進屋裡就安全。
//!   - 玩家可反擊：挖擊 [`HITS_TO_DISSIPATE`] 下消散、掉一枚乙太礦當溫柔獎勵。
//!   - 居民只會害怕（冒泡逃回家），**絕不掉血**——療癒底線。
//!
//! 這裡只放「與連線/鎖無關」的確定性純邏輯（生成條件/光照庇護判定/漂移物理/觸傷/擊散），
//! 全部抽成可測純函式；真正的 tick 驅動、廣播、傷害套用都在 `voxel_ws.rs`
//! （嚴守 prod 死鎖鐵律：短鎖快照 → drop → 純計算 → 短鎖套用，絕不持鎖 await）。
//! 零 LLM：暗影的移動是純確定性計算，一毛腦力預算都不花。

use crate::voxel::{self, Block, WorldDelta, CHUNK};
use crate::voxel_time::TimePhase;

// ── 調性參數（集中一處，日後平衡好調）───────────────────────────────────────────

/// 全圖同時存在的暗影上限：少量點綴夜色，不是怪物海。
pub const MAX_WISPS: usize = 6;
/// 暗影漂移速度（方塊/秒）：刻意比玩家(5.0)與居民(2.6)都慢——「緩慢漂近的影子」，
/// 走路就能拉開距離，張力來自氛圍而非追殺。
pub const WISP_SPEED: f32 = 1.6;
/// 暗影 AABB 半寬 / 高（比居民小一號的漂浮小靈）。
pub const WISP_HALF_W: f32 = 0.3;
pub const WISP_HEIGHT: f32 = 0.9;
/// 光照庇護半徑（方塊）：任一光源方塊（火把/冰晶燈/乙太燈/營火）中心此距離內為亮區。
pub const LIGHT_RADIUS: f32 = 8.0;
/// 觸碰判定距離（方塊，中心對中心）：貼到這麼近才算「碰到」玩家。
pub const TOUCH_RADIUS: f32 = 1.2;
/// 每次觸碰扣的血（半顆心制，1 = 半顆心）。
pub const TOUCH_DAMAGE: u32 = 1;
/// 觸碰扣血冷卻（秒）：同一位玩家至少隔這麼久才會再被扣——緩慢、溫柔的壓力。
pub const TOUCH_COOLDOWN_SECS: f32 = 2.0;
/// 玩家挖擊幾下暗影會消散。
pub const HITS_TO_DISSIPATE: u8 = 3;
/// 兩次有效挖擊之間的最短間隔（秒）：伺服器端節流，擋封包連發瞬殺（濫用防護）。
pub const HIT_MIN_INTERVAL_SECS: f32 = 0.25;
/// 生成點與目標玩家的距離區間（方塊）：在視野邊緣的暗處現身，不貼臉跳臉。
pub const SPAWN_MIN_DIST: f32 = 14.0;
pub const SPAWN_MAX_DIST: f32 = 26.0;
/// 生成檢查間隔（秒）與每次檢查的生成機率：慢慢冒、不是開夜就刷滿。
pub const SPAWN_INTERVAL_SECS: f32 = 3.0;
pub const SPAWN_CHANCE: f32 = 0.5;
/// 村莊庇護半徑（方塊）：村莊中心此距離內絕不生成（廣場的燈火與人氣是天然安全區）。
pub const VILLAGE_SAFE_RADIUS: f32 = 48.0;
/// 光源快取重掃間隔（秒）：低頻掃一次世界 delta 收集光源座標（成本與改動量成正比）。
pub const LIGHT_RESCAN_SECS: f32 = 5.0;
/// 居民害怕半徑（方塊）：暗影靠到這麼近，居民會冒害怕泡泡並折返回家。
pub const FEAR_RADIUS: f32 = 9.0;
/// 同一位居民兩次害怕反應的冷卻（秒）：避免整夜狂洗泡泡。
pub const FEAR_COOLDOWN_SECS: f32 = 45.0;
/// 消散/擊散掉落的溫柔獎勵：一枚乙太礦（Block::AetherOre = 58）。
pub const SHARD_ITEM_ID: u8 = Block::AetherOre as u8;
/// 暗影漂浮時腳底離地的目標高度（方塊）：略浮空的影子感。
pub const HOVER_ABOVE_GROUND: f32 = 0.6;
/// 暗影追蹤目標的最遠距離（方塊）：太遠的玩家/居民不追（原地徘徊），tick 便宜。
pub const CHASE_RANGE: f32 = 40.0;

/// 首夜提示（Feed/聊天窗，一夜只出現一次）：告訴玩家「光=庇護」的玩法。
/// 面向玩家字串集中於此，i18n 友善。
pub const SHADOW_FEED_KIND: &str = "暗影";
pub const SHADOW_FEED_ACTOR: &str = "乙太方界";
pub const SHADOW_FEED_DETAIL: &str = "夜裡有暗影出沒…點亮燈火吧！沿著有燈的路走是安全的。";

/// 居民害怕台詞池（暗影靠近時冒泡）。
pub const FEAR_LINES: &[&str] = &[
    "有黑黑的東西…快回家！",
    "那個影子…我先回家躲躲！",
    "夜裡的影子好可怕，快回屋裡！",
];

/// 依 seed 從害怕台詞池挑一句（確定性、可測）。
pub fn fear_line(seed: usize) -> &'static str {
    FEAR_LINES[seed % FEAR_LINES.len()]
}

// ── 暗影本體 ─────────────────────────────────────────────────────────────────

/// 一隻暗影小靈的權威狀態（伺服器算，客戶端只渲染）。
#[derive(Clone, Debug)]
pub struct Wisp {
    pub id: u64,
    /// 腳底位置（與玩家/居民同語意：y = AABB 底）。
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// 已被挖擊的次數（達 [`HITS_TO_DISSIPATE`] 即消散）。
    pub hits: u8,
}

/// 暗影消散的原因（決定要不要掉溫柔獎勵）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DissipateReason {
    /// 被玩家挖擊擊散 → 掉一枚乙太礦。
    Killed,
    /// 誤入亮區化成輕煙 → 也掉一枚（把影子引到燈下也是一種溫柔的玩法）。
    Light,
    /// 黎明整批消散 → 不掉（避免每天清晨滿地道具洗版）。
    Dawn,
}

/// 此消散原因是否掉落溫柔獎勵（一枚乙太礦）。純函式、可測。
pub fn drops_shard(reason: DissipateReason) -> bool {
    matches!(reason, DissipateReason::Killed | DissipateReason::Light)
}

// ── 生成條件（夜 + 暗 + 遠村）────────────────────────────────────────────────

/// 暗影活動時段：入夜過渡（Evening）與深夜（Night）。黎明（Dawn）一到即整批消散。
pub fn is_shadow_time(phase: TimePhase) -> bool {
    matches!(phase, TimePhase::Night | TimePhase::Evening)
}

/// 這次生成檢查是否允許再生一隻（夜間 + 未達上限 + 機率擲中）。純函式、可測。
pub fn can_spawn(count: usize, shadow_time: bool, roll: f32) -> bool {
    shadow_time && count < MAX_WISPS && roll < SPAWN_CHANCE
}

/// 依角度/距離在目標玩家周圍算一個候選生成點（幾何純函式，隨機性由呼叫端給）。
pub fn spawn_candidate(px: f32, pz: f32, angle: f32, dist: f32) -> (f32, f32) {
    (px + angle.cos() * dist, pz + angle.sin() * dist)
}

/// 候選點是否離村莊中心夠遠（村莊庇護：廣場一帶永不生成）。純函式、可測。
pub fn far_from_village(x: f32, z: f32, vcx: f32, vcz: f32) -> bool {
    let dx = x - vcx;
    let dz = z - vcz;
    dx * dx + dz * dz > VILLAGE_SAFE_RADIUS * VILLAGE_SAFE_RADIUS
}

/// 某點是否在任一光源的庇護半徑內（亮區）。光源座標取方塊中心。純函式、可測。
pub fn is_lit(x: f32, y: f32, z: f32, lights: &[(i32, i32, i32)]) -> bool {
    let r2 = LIGHT_RADIUS * LIGHT_RADIUS;
    lights.iter().any(|&(lx, ly, lz)| {
        let dx = (lx as f32 + 0.5) - x;
        let dy = (ly as f32 + 0.5) - (y + WISP_HEIGHT * 0.5);
        let dz = (lz as f32 + 0.5) - z;
        dx * dx + dy * dy + dz * dz <= r2
    })
}

/// 是否為「會發光庇護」的光源方塊（火把/冰晶燈/乙太燈/營火——與前端光暈同一組）。
pub fn is_light_block(b: Block) -> bool {
    matches!(b, Block::Torch | Block::IceLantern | Block::AetherLamp | Block::Campfire)
}

/// 掃世界 delta 收集所有光源方塊的世界座標（低頻呼叫，成本與「被改過的方塊數」成正比，
/// 非世界大小；玩家/居民放的燈都在 delta 層，程序地形本身無光源）。純函式、可測。
pub fn collect_lights(world: &WorldDelta) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for (coord, delta) in world {
        for (&li, &b) in delta {
            if is_light_block(b) {
                // local_index 反解：li = lx + lz*CHUNK + ly*CHUNK²（見 voxel::local_index）。
                let c = CHUNK as usize;
                let lx = (li % c) as i32;
                let lz = ((li / c) % c) as i32;
                let ly = (li / (c * c)) as i32;
                out.push((coord.cx * CHUNK + lx, coord.cy * CHUNK + ly, coord.cz * CHUNK + lz));
            }
        }
    }
    out
}

/// 求 (x,z) 柱在（含 delta）世界裡可站的地表 y（腳底高度）：從地形高度起算，
/// 若該處被 delta 蓋了實心（玩家建物）就往上找，最多找 24 格；再往上仍卡就回 None
/// （這柱不適合生成）。純函式、可測。
pub fn hover_spawn_y(world: &WorldDelta, x: f32, z: f32) -> Option<f32> {
    let base = voxel::height_at(x.floor() as i32, z.floor() as i32);
    for up in 1..=24_i32 {
        let y = (base + up) as f32 + HOVER_ABOVE_GROUND;
        if !overlaps(world, x, y, z) {
            return Some(y);
        }
    }
    None
}

// ── 漂移物理（實體、不穿牆、無重力）──────────────────────────────────────────

/// 暗影 AABB 是否與任一實心方塊重疊（鏡像居民 `overlaps`，尺寸較小）。純函式、可測。
pub fn overlaps(world: &WorldDelta, x: f32, y: f32, z: f32) -> bool {
    let x0 = (x - WISP_HALF_W).floor() as i32;
    let x1 = (x + WISP_HALF_W).floor() as i32;
    let y0 = y.floor() as i32;
    let y1 = (y + WISP_HEIGHT - 0.01).floor() as i32;
    let z0 = (z - WISP_HALF_W).floor() as i32;
    let z1 = (z + WISP_HALF_W).floor() as i32;
    for bx in x0..=x1 {
        for by in y0..=y1 {
            for bz in z0..=z1 {
                if voxel::effective_block_at(world, bx, by, bz).is_solid() {
                    return true;
                }
            }
        }
    }
    false
}

/// 沿單一軸嘗試位移：撞實心就回退（**不**踏階——牆是屏障，暗影翻不過去）。
fn drift_axis(world: &WorldDelta, w: &mut Wisp, dx: f32, dy: f32, dz: f32) {
    if dx == 0.0 && dy == 0.0 && dz == 0.0 {
        return;
    }
    let (px, py, pz) = (w.x, w.y, w.z);
    w.x += dx;
    w.y += dy;
    w.z += dz;
    if overlaps(world, w.x, w.y, w.z) {
        w.x = px;
        w.y = py;
        w.z = pz;
    }
}

/// 朝目標腳底座標漂一步（逐軸位移＋碰撞，無重力：暗影浮著）。
/// 垂直方向朝「目標 y + 漂浮高度」緩慢靠攏，但同樣不穿實心。
pub fn drift_step(world: &WorldDelta, w: &mut Wisp, tx: f32, ty: f32, tz: f32, dt: f32) {
    let dx = tx - w.x;
    let dy = (ty + HOVER_ABOVE_GROUND) - w.y;
    let dz = tz - w.z;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if dist < 1e-4 {
        return;
    }
    let step = (WISP_SPEED * dt).min(dist);
    let scale = step / dist;
    drift_axis(world, w, dx * scale, 0.0, 0.0);
    drift_axis(world, w, 0.0, 0.0, dz * scale);
    drift_axis(world, w, 0.0, dy * scale, 0.0);
}

/// 從一批候選目標（玩家＋居民腳底座標）挑最近的（限 [`CHASE_RANGE`] 內）。
/// 都太遠回 None（暗影原地徘徊）。純函式、可測。
pub fn nearest_target(x: f32, z: f32, targets: &[(f32, f32, f32)]) -> Option<(f32, f32, f32)> {
    let mut best: Option<((f32, f32, f32), f32)> = None;
    for &(tx, ty, tz) in targets {
        let dx = tx - x;
        let dz = tz - z;
        let d2 = dx * dx + dz * dz;
        if d2 > CHASE_RANGE * CHASE_RANGE {
            continue;
        }
        if best.map_or(true, |(_, bd2)| d2 < bd2) {
            best = Some(((tx, ty, tz), d2));
        }
    }
    best.map(|(t, _)| t)
}

// ── 觸傷 / 擊散 / 害怕 ────────────────────────────────────────────────────────

/// 玩家（腳底座標）是否正被這隻暗影碰到（水平近 + 垂直帶重疊）。純函式、可測。
pub fn touching(px: f32, py: f32, pz: f32, w: &Wisp) -> bool {
    let dx = px - w.x;
    let dz = pz - w.z;
    if dx * dx + dz * dz > TOUCH_RADIUS * TOUCH_RADIUS {
        return false;
    }
    // 垂直帶：玩家身高帶（0..1.7）與暗影身高帶（0..WISP_HEIGHT）有交集（放 0.5 餘裕）。
    let player_h = 1.7_f32;
    py < w.y + WISP_HEIGHT + 0.5 && w.y < py + player_h + 0.5
}

/// 登記一次有效挖擊：回傳（新累計、是否就此消散）。純函式、可測。
pub fn register_hit(hits: u8) -> (u8, bool) {
    let n = hits.saturating_add(1);
    (n, n >= HITS_TO_DISSIPATE)
}

/// 挖擊觸及驗證：玩家眼睛到暗影中心的距離平方 ≤ (REACH+1)²（沿用方塊互動同一套餘裕）。
/// **後端權威**：客戶端只自報「想打哪隻」，打不打得到由伺服器算。純函式、可測。
pub fn hit_in_reach(px: f32, py: f32, pz: f32, w: &Wisp) -> bool {
    let dx = w.x - px;
    let dy = (w.y + WISP_HEIGHT * 0.5) - (py + voxel::EYE_HEIGHT);
    let dz = w.z - pz;
    let max = voxel::REACH + 1.0;
    dx * dx + dy * dy + dz * dz <= max * max
}

/// 居民（腳底座標）附近是否有暗影靠得夠近（觸發害怕反應）。純函式、可測。
pub fn frightened_by(rx: f32, rz: f32, wisps: &[Wisp]) -> bool {
    wisps.iter().any(|w| {
        let dx = rx - w.x;
        let dz = rz - w.z;
        dx * dx + dz * dz <= FEAR_RADIUS * FEAR_RADIUS
    })
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::height_at;
    use crate::voxel::SEA_LEVEL;

    /// 找一塊高於海平面的陸地（給碰撞/漂移測試站穩）。
    fn land_point() -> (i32, i32) {
        for c in 0..3000 {
            if height_at(c, 0) > SEA_LEVEL + 3 {
                return (c, 0);
            }
        }
        (0, 0)
    }

    #[test]
    fn shadow_time_only_night_and_evening() {
        assert!(is_shadow_time(TimePhase::Night), "深夜是暗影時段");
        assert!(is_shadow_time(TimePhase::Evening), "入夜過渡也是暗影時段");
        assert!(!is_shadow_time(TimePhase::Dawn), "黎明一到就不再生成");
        assert!(!is_shadow_time(TimePhase::Day), "白天沒有暗影");
        assert!(!is_shadow_time(TimePhase::Dusk), "黃昏還沒入夜");
    }

    #[test]
    fn can_spawn_respects_cap_time_and_roll() {
        assert!(can_spawn(0, true, 0.0), "夜間、未達上限、擲中 → 可生");
        assert!(!can_spawn(MAX_WISPS, true, 0.0), "到上限絕不再生（數量上限）");
        assert!(!can_spawn(0, false, 0.0), "白天絕不生成");
        assert!(!can_spawn(0, true, 1.0), "沒擲中不生（慢慢冒）");
        assert!(can_spawn(MAX_WISPS - 1, true, SPAWN_CHANCE - 0.01), "上限前一隻仍可生");
    }

    #[test]
    fn spawn_candidate_geometry() {
        // 角度 0、距離 20 → 純 +x 方向。
        let (x, z) = spawn_candidate(10.0, -5.0, 0.0, 20.0);
        assert!((x - 30.0).abs() < 1e-3);
        assert!((z + 5.0).abs() < 1e-3);
    }

    #[test]
    fn village_safe_radius_blocks_spawn() {
        assert!(!far_from_village(0.0, 0.0, 0.0, 0.0), "村中心不生成");
        assert!(!far_from_village(VILLAGE_SAFE_RADIUS - 1.0, 0.0, 0.0, 0.0), "半徑內不生成");
        assert!(far_from_village(VILLAGE_SAFE_RADIUS + 1.0, 0.0, 0.0, 0.0), "半徑外才可生成");
    }

    #[test]
    fn light_radius_shelters() {
        let lights = vec![(0, 10, 0)];
        // 燈正旁邊 → 亮區。
        assert!(is_lit(1.0, 10.0, 1.0, &lights), "燈旁是亮區");
        // 半徑外 → 暗區。
        assert!(!is_lit(LIGHT_RADIUS + 2.0, 10.0, 0.0, &lights), "半徑外是暗區");
        // 沒有任何燈 → 全暗。
        assert!(!is_lit(0.0, 10.0, 0.0, &[]), "無燈全暗");
    }

    #[test]
    fn light_blocks_are_the_lamp_family() {
        assert!(is_light_block(Block::Torch), "火把發光");
        assert!(is_light_block(Block::IceLantern), "冰晶燈發光");
        assert!(is_light_block(Block::AetherLamp), "乙太燈發光");
        assert!(is_light_block(Block::Campfire), "營火發光");
        assert!(!is_light_block(Block::Stone), "石頭不發光");
        assert!(!is_light_block(Block::AetherOre), "乙太礦石不算庇護光源");
    }

    #[test]
    fn collect_lights_roundtrips_world_coords() {
        let mut world = WorldDelta::new();
        // 放兩盞燈（含負座標，驗 euclid 反解）＋一塊石頭（不該被收集）。
        voxel::set_block(&mut world, 5, 12, 7, Block::Torch);
        voxel::set_block(&mut world, -3, 20, -18, Block::AetherLamp);
        voxel::set_block(&mut world, 4, 12, 7, Block::Stone);
        let mut lights = collect_lights(&world);
        lights.sort();
        assert_eq!(lights, vec![(-3, 20, -18), (5, 12, 7)], "光源座標應精確反解回世界座標");
    }

    #[test]
    fn wisp_cannot_pass_solid_wall() {
        let mut world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // 在 +x 築一面 3 格高的牆。
        for up in 1..=3 {
            voxel::set_block(&mut world, x + 1, h + up, z, Block::Stone);
        }
        let mut w = Wisp { id: 1, x: x as f32 + 0.5, y: (h + 1) as f32 + HOVER_ABOVE_GROUND, z: z as f32 + 0.5, hits: 0 };
        let y0 = w.y;
        // 朝牆後的目標漂 10 秒——應被牆完全擋住（x 不越過牆、也不翻牆爬升）。
        for _ in 0..100 {
            drift_step(&world, &mut w, x as f32 + 5.0, (h + 1) as f32, z as f32 + 0.5, 0.1);
        }
        assert!(w.x < x as f32 + 1.0, "牆=屏障：暗影不能穿實心方塊 x={}", w.x);
        assert!((w.y - y0).abs() < 1.0, "暗影不該翻牆爬升 y={} y0={y0}", w.y);
    }

    #[test]
    fn wisp_drifts_toward_target_on_open_ground() {
        let world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        let mut w = Wisp { id: 1, x: x as f32 + 0.5, y: (h + 1) as f32 + HOVER_ABOVE_GROUND, z: z as f32 + 0.5, hits: 0 };
        let tx = x as f32 + 6.0;
        let d0 = (tx - w.x).abs();
        for _ in 0..30 {
            drift_step(&world, &mut w, tx, (h + 1) as f32, z as f32 + 0.5, 0.1);
        }
        let d1 = (tx - w.x).abs();
        assert!(d1 < d0, "開闊地應逐步漂近目標：{d0} → {d1}");
        // 速度上限守住（3 秒最多 WISP_SPEED*3 格）。
        assert!(d0 - d1 <= WISP_SPEED * 3.0 + 0.01, "漂移不得超速");
    }

    #[test]
    fn nearest_target_picks_closest_within_range() {
        let targets = vec![(10.0, 5.0, 0.0), (3.0, 5.0, 0.0), (100.0, 5.0, 0.0)];
        let got = nearest_target(0.0, 0.0, &targets);
        assert_eq!(got, Some((3.0, 5.0, 0.0)), "應挑最近的目標");
        // 全部超出追蹤範圍 → None（原地徘徊）。
        let far = vec![(CHASE_RANGE + 10.0, 5.0, 0.0)];
        assert_eq!(nearest_target(0.0, 0.0, &far), None, "太遠不追");
        assert_eq!(nearest_target(0.0, 0.0, &[]), None, "沒目標不追");
    }

    #[test]
    fn touch_requires_close_and_vertical_overlap() {
        let w = Wisp { id: 1, x: 0.0, y: 10.0, z: 0.0, hits: 0 };
        assert!(touching(0.5, 10.0, 0.5, &w), "貼身＝碰到");
        assert!(!touching(TOUCH_RADIUS + 1.0, 10.0, 0.0, &w), "水平太遠不算碰");
        assert!(!touching(0.0, 20.0, 0.0, &w), "頭頂高空不算碰（垂直帶不重疊）");
    }

    #[test]
    fn three_hits_dissipate() {
        let (h1, d1) = register_hit(0);
        assert_eq!((h1, d1), (1, false), "第一下還在");
        let (h2, d2) = register_hit(h1);
        assert_eq!((h2, d2), (2, false), "第二下還在");
        let (h3, d3) = register_hit(h2);
        assert_eq!((h3, d3), (3, true), "第三下消散");
        // saturating：不會 overflow。
        let (h4, d4) = register_hit(u8::MAX);
        assert_eq!((h4, d4), (u8::MAX, true));
    }

    #[test]
    fn hit_reach_is_server_authoritative() {
        let w = Wisp { id: 1, x: 0.0, y: 10.0, z: 0.0, hits: 0 };
        assert!(hit_in_reach(2.0, 9.0, 0.0, &w), "近身打得到");
        assert!(!hit_in_reach(30.0, 9.0, 0.0, &w), "隔半張地圖打不到（客戶端自報無效）");
    }

    #[test]
    fn shard_drops_on_kill_and_light_not_on_dawn() {
        assert!(drops_shard(DissipateReason::Killed), "擊散掉乙太礦");
        assert!(drops_shard(DissipateReason::Light), "引進燈下消散也掉（溫柔獎勵）");
        assert!(!drops_shard(DissipateReason::Dawn), "黎明整批消散不掉（防洗版）");
    }

    #[test]
    fn residents_fear_nearby_wisp() {
        let wisps = vec![Wisp { id: 1, x: 0.0, y: 10.0, z: 0.0, hits: 0 }];
        assert!(frightened_by(3.0, 0.0, &wisps), "暗影在害怕半徑內 → 居民害怕");
        assert!(!frightened_by(FEAR_RADIUS + 2.0, 0.0, &wisps), "夠遠不怕");
        assert!(!frightened_by(0.0, 0.0, &[]), "沒暗影不怕");
    }

    #[test]
    fn fear_line_deterministic_and_nonempty() {
        for s in 0..8 {
            assert!(!fear_line(s).is_empty());
        }
        assert_eq!(fear_line(0), fear_line(FEAR_LINES.len()), "seed 取模循環");
    }

    #[test]
    fn hover_spawn_y_clears_delta_buildings() {
        let mut world = WorldDelta::new();
        let (x, z) = land_point();
        let h = height_at(x, z);
        // 素地：地表上一格就能浮。
        let y0 = hover_spawn_y(&world, x as f32 + 0.5, z as f32 + 0.5);
        assert!(y0.is_some(), "素地應有生成高度");
        assert!(!overlaps(&world, x as f32 + 0.5, y0.unwrap(), z as f32 + 0.5), "生成點不卡實心");
        // 蓋 2 格高的建物 → 生成高度應抬升到建物之上、仍不卡實心。
        for up in 1..=2 {
            voxel::set_block(&mut world, x, h + up, z, Block::Stone);
        }
        let y1 = hover_spawn_y(&world, x as f32 + 0.5, z as f32 + 0.5);
        assert!(y1.is_some());
        assert!(y1.unwrap() > y0.unwrap(), "被建物墊高");
        assert!(!overlaps(&world, x as f32 + 0.5, y1.unwrap(), z as f32 + 0.5));
    }

    #[test]
    fn touch_damage_is_gentle_never_oneshot() {
        // 療癒底線釘樁：單次觸傷遠小於滿血，且冷卻 ≥ 2 秒——就算站著不動，
        // 從滿血到歸零也要 ≥ 20 秒（MAX_HEALTH=20、1 點/2 秒），玩家永遠來得及走開。
        assert!(TOUCH_DAMAGE <= 1, "觸傷必須溫柔（≤ 半顆心）");
        assert!(TOUCH_COOLDOWN_SECS >= 2.0, "觸傷冷卻必須 ≥ 2 秒");
        assert!(WISP_SPEED < 5.0, "暗影必須比玩家慢（走路就能拉開）");
    }
}
