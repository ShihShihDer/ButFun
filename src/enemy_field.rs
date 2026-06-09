//! 敵人的世界佈置與自動鎖定（Phase 1-F 戰鬥 MVP「自動打怪」的純邏輯地基之二）。
//!
//! `combat.rs` 解了「一隻敵人怎麼被打、被打倒掉什麼、之後怎麼重生」；接線還缺另一半——
//! 「**敵人擺在世界哪裡、玩家走近時自動鎖定哪一隻**」。本層就是那塊純幾何 + 純互動。
//!
//! ③ 無限世界（切片 B）：改為區塊式確定性生成。

use std::collections::HashMap;
use world_core::{chunk_key, CHUNK_SIZE};

use crate::combat::{Enemy, EnemyKind};
use crate::inventory::ItemKind;
use crate::positions::is_in_safe_zone;

/// 每區塊平均生成的敵人數。
const ENEMIES_PER_CHUNK: usize = 1;

/// 自動攻擊的伸手範圍：玩家走進敵人這個距離內就會自動出手。
pub const ATTACK_REACH: f32 = 64.0;

/// 敵人察覺玩家、開始追擊的半徑。
pub const AGGRO_RADIUS: f32 = 260.0;

/// 追擊速度（像素/秒）。
const CHASE_SPEED: f32 = 105.0;

/// 沒有玩家在附近時，敵人緩緩漂回自己的出生點。
const RETURN_SPEED: f32 = 48.0;

/// 世界裡一隻有座標的敵人。
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedEnemy {
    /// 唯一 ID，格式為 `(chunk_x, chunk_y, index_in_chunk)`。
    pub id: (i32, i32, usize),
    /// 世界座標 X。
    pub x: f32,
    /// 世界座標 Y。
    pub y: f32,
    /// 敵人本身（生命 / 重生狀態）。
    pub enemy: Enemy,
}

/// 散佈在世界裡的一整組敵人。
#[derive(Debug, Clone, PartialEq)]
pub struct EnemyField {
    chunks: HashMap<(i32, i32), Vec<PlacedEnemy>>,
}

#[allow(dead_code)]
impl EnemyField {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
        }
    }

    pub fn enemies(&self) -> Vec<PlacedEnemy> {
        self.chunks.values().flatten().cloned().collect()
    }

    pub fn ensure_chunks_around(&mut self, px: f32, py: f32, radius: f32) {
        let (cx_min, cy_min) = chunk_key(px - radius, py - radius);
        let (cx_max, cy_max) = chunk_key(px + radius, py + radius);

        for cy in cy_min..=cy_max {
            for cx in cx_min..=cx_max {
                self.chunks.entry((cx, cy)).or_insert_with(|| generate_chunk(cx, cy));
            }
        }
    }

    pub fn tick(&mut self, dt: f32) {
        for nodes in self.chunks.values_mut() {
            for placed in nodes {
                placed.enemy.tick(dt);
            }
        }
    }

    /// 推進敵人移動。`tile_solid(x, y)` 回傳該世界像素座標是否為實心地形格（C-3 碰撞，
    /// 傳 `|_, _| false` 可關閉、保留舊行為）。敵人撞牆會沿單軸滑行、不穿牆也不整個卡死。
    /// `is_night` 為 true 時，追擊速度乘以 1.4——夜間怪物更具侵略性，給玩家危機感。
    pub fn advance<F: Fn(f32, f32) -> bool>(
        &mut self,
        dt: f32,
        players: &[(f32, f32)],
        is_night: bool,
        tile_solid: F,
    ) {
        if dt <= 0.0 {
            return;
        }
        let aggro_sq = AGGRO_RADIUS * AGGRO_RADIUS;
        // 夜間追擊速度加成：讓玩家感受到夜裡的危機感。
        let night_mult = if is_night { 1.4_f32 } else { 1.0_f32 };

        // 收集所有需要移動的敵人
        let mut to_move = Vec::new();

        for (&(cx, cy), enemies) in self.chunks.iter_mut() {
            for (idx, placed) in enemies.iter_mut().enumerate() {
                if !placed.enemy.is_alive() {
                    continue;
                }

                let mut nearest: Option<(f32, f32, f32)> = None;
                for &(tx, ty) in players {
                    if !tx.is_finite() || !ty.is_finite() {
                        continue;
                    }
                    let dx = tx - placed.x;
                    let dy = ty - placed.y;
                    let d2 = dx * dx + dy * dy;
                    if d2 <= aggro_sq && nearest.is_none_or(|(_, _, b)| d2 < b) {
                        nearest = Some((tx, ty, d2));
                    }
                }

                let (target_x, target_y, speed) = match nearest {
                    Some((tx, ty, _)) => (tx, ty, CHASE_SPEED * night_mult),
                    None => {
                        let (hx, hy) = spawn_position(placed.id);
                        (hx, hy, RETURN_SPEED)
                    }
                };

                let dx = target_x - placed.x;
                let dy = target_y - placed.y;
                let dist = (dx * dx + dy * dy).sqrt();

                if dist > 2.0 {
                    let step = (speed * dt).min(dist);
                    let mvx = dx / dist * step;
                    let mvy = dy / dist * step;
                    // C-3 碰撞:不穿實心地形。先試整步,撞牆就沿單軸滑行(能繞牆、別整個卡死)。
                    if !tile_solid(placed.x + mvx, placed.y + mvy) {
                        placed.x += mvx;
                        placed.y += mvy;
                    } else {
                        if !tile_solid(placed.x + mvx, placed.y) {
                            placed.x += mvx;
                        }
                        if !tile_solid(placed.x, placed.y + mvy) {
                            placed.y += mvy;
                        }
                    }
                    
                    let new_key = chunk_key(placed.x, placed.y);
                    if new_key != (cx, cy) {
                        to_move.push(((cx, cy), idx, new_key));
                    }
                }
            }
        }

        // 處理跨區塊移動 (從後往前移以保持索引有效)
        // 同時更新 id 讓 (id.0, id.1) 永遠與實際所在區塊一致，
        // 避免 attack_nearest 用舊 chunk 座標找不到而 unwrap panic。
        to_move.sort_by_key(|&(_, idx, _)| std::cmp::Reverse(idx));
        for (old_key, idx, new_key) in to_move {
            // 防護:chunk 不在或索引失效就跳過,**絕不 unwrap**(別讓單一壞索引 panic 炸死整個遊戲迴圈)。
            let mut enemy = match self.chunks.get_mut(&old_key) {
                Some(src) if idx < src.len() => src.remove(idx),
                _ => continue,
            };
            let target = self.chunks.entry(new_key).or_default();
            let new_idx = target.len();
            enemy.id = (new_key.0, new_key.1, new_idx);
            target.push(enemy);
        }
    }

    pub fn attack_nearest(
        &mut self,
        px: f32,
        py: f32,
        power: u32,
    ) -> Option<(EnemyKind, Option<(ItemKind, u32)>)> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        
        self.ensure_chunks_around(px, py, ATTACK_REACH);

        let (cx, cy) = chunk_key(px, py);
        // 記住敵人「實際被找到的 chunk」——別從 id.0/id.1 推導 chunk：敵人會移動跨 chunk,
        // 其 id 內含的原始 chunk 欄位可能對不上現在所在的 chunk,事後重查就會 None。
        let mut best: Option<((i32, i32), (i32, i32, usize), f32)> = None; // (找到的 chunk, 敵人 id, dist²)
        let reach_sq = ATTACK_REACH * ATTACK_REACH;

        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(enemies) = self.chunks.get(&(cx + dx, cy + dy)) {
                    for placed in enemies {
                        if !placed.enemy.is_alive() {
                            continue;
                        }
                        let dist_x = placed.x - px;
                        let dist_y = placed.y - py;
                        let dist_sq = dist_x * dist_x + dist_y * dist_y;
                        if dist_sq <= reach_sq {
                            if best.as_ref().map_or(true, |(_, _, b)| dist_sq < *b) {
                                best = Some(((cx + dx, cy + dy), placed.id, dist_sq));
                            }
                        }
                    }
                }
            }
        }

        // 用「實際找到的 chunk」重查;查不到一律回 None——**絕不 unwrap**:None 一 unwrap 整個
        // 遊戲迴圈 panic 死掉、全服收不到快照(玩家進去只有場景沒角色),就是這次踩的雷。
        if let Some((found_chunk, id, _)) = best {
            if let Some(enemies) = self.chunks.get_mut(&found_chunk) {
                if let Some(placed) = enemies.iter_mut().find(|e| e.id == id) {
                    let kind = placed.enemy.kind();
                    let loot = placed.enemy.attack(power);
                    return Some((kind, loot));
                }
            }
            None
        } else {
            None
        }
    }

    pub fn threat_at(&self, px: f32, py: f32) -> u32 {
        if !px.is_finite() || !py.is_finite() {
            return 0;
        }
        
        let (cx, cy) = chunk_key(px, py);
        let reach_sq = ATTACK_REACH * ATTACK_REACH;
        let mut total = 0;

        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(enemies) = self.chunks.get(&(cx + dx, cy + dy)) {
                    for placed in enemies {
                        if !placed.enemy.is_alive() {
                            continue;
                        }
                        let dist_x = placed.x - px;
                        let dist_y = placed.y - py;
                        if dist_x * dist_x + dist_y * dist_y <= reach_sq {
                            total += placed.enemy.kind().threat();
                        }
                    }
                }
            }
        }
        total
    }

    pub fn from_saved(saved: Vec<Enemy>) -> Option<Self> {
        let mut field = Self::new();
        for (i, enemy) in saved.into_iter().enumerate() {
            if !enemy.is_loadable() { continue; }
            let id = (0, 0, i);
            let (x, y) = spawn_position(id);
            let key = chunk_key(x, y);
            field.chunks.entry(key).or_default().push(PlacedEnemy { x, y, enemy, id });
        }
        Some(field)
    }
}

impl Default for EnemyField {
    fn default() -> Self {
        Self::new()
    }
}

/// 依生態域決定敵人種類：每個生態域有專屬守護者，打倒後掉落該生態域特產，
/// 讓「戰鬥」成為「採礦挖掘」之外獲取特產的第二條路。
fn kind_for_biome(biome: world_core::Biome) -> EnemyKind {
    use world_core::Biome;
    match biome {
        // 草原——飄舞精靈守護野花叢，脆弱但溫和。
        Biome::Meadow => EnemyKind::FlutterSprite,
        // 森林——蕈菇潛行者潛伏在蕈菇洞，中等威脅。
        Biome::Forest => EnemyKind::MushroomStalker,
        // 岩地晶洞——晶石傀儡守衛晶洞，最堅硬的守門者。
        Biome::Rocky => EnemyKind::CrystalGolem,
        // 沙漠遺跡——古代符文守衛，沉睡千年被探索者驚醒。
        Biome::Sand => EnemyKind::RuneGuardian,
        // 水域珊瑚礁——珊瑚蟹藏身礁石之間，守著稀有珍珠。
        Biome::Water => EnemyKind::CoralCrab,
    }
}

fn generate_chunk(cx: i32, cy: i32) -> Vec<PlacedEnemy> {
    let mut enemies = Vec::new();
    // 星球判定：區塊中心 X ≥ VOID_ZONE_MIN_X 為虛空星；X ≥ VERDANT_ZONE_MIN_X 為翠幽星；
    // X ≤ ORIGIN_ZONE_MAX_X 為星源星（優先於霧醚星）；X ≤ AETHER_ZONE_MAX_X 為霧醚星（優先於赤焰星）；
    // X ≤ CRIMSON_ZONE_MAX_X 為赤焰星。
    // 虛空星優先（其 X 範圍包含翠幽星範圍）；星源星優先於霧醚星（更深的極西境）；霧醚星優先於赤焰星。
    let chunk_center_x = (cx as f64 + 0.5) * (world_core::CHUNK_SIZE as f64);
    let is_void    = chunk_center_x >= world_core::VOID_ZONE_MIN_X;
    let is_verdant = !is_void && chunk_center_x >= world_core::VERDANT_ZONE_MIN_X;
    let is_origin  = chunk_center_x <= world_core::ORIGIN_ZONE_MAX_X;
    let is_aether  = !is_origin && chunk_center_x <= world_core::AETHER_ZONE_MAX_X;
    let is_crimson = !is_origin && !is_aether && chunk_center_x <= world_core::CRIMSON_ZONE_MAX_X;
    for i in 0..ENEMIES_PER_CHUNK {
        let id = (cx, cy, i);
        let (x, y) = spawn_position(id);
        // 新手村安全區不生成敵人，讓新玩家有緩衝時間熟悉遊戲。
        if is_in_safe_zone(x, y) {
            continue;
        }
        let kind = if is_void {
            // 虛空星：一律生成虛空幽靈（整個虛空星都是宇宙深淵領域，無視地表生態域）。
            EnemyKind::VoidPhantom
        } else if is_verdant {
            // 翠幽星：一律生成翠幽魅影（整個翠幽星都是異星領域，無視地表生態域）。
            EnemyKind::JadeWraith
        } else if is_origin {
            // 星源星：一律生成源晶守護者（整個星源星都是宇宙源頭領域，無視地表生態域）。
            EnemyKind::OriginGuardian
        } else if is_aether {
            // 霧醚星：一律生成霧醚幻靈（整個霧醚星都是乙太迷霧領域，無視地表生態域）。
            EnemyKind::AetherSpecter
        } else if is_crimson {
            // 赤焰星：一律生成蒸汽構裝（整個赤焰星都是古代蒸汽文明領域，無視地表生態域）。
            EnemyKind::SteamConstruct
        } else {
            let biome = world_core::biome_at(x as f64, y as f64);
            kind_for_biome(biome)
        };
        enemies.push(PlacedEnemy {
            id,
            x,
            y,
            enemy: Enemy::new(kind),
        });
    }
    enemies
}

/// 在區塊內找一個非水域且非實心的落點（所有生態域都能出現敵人）。
fn spawn_position(id: (i32, i32, usize)) -> (f32, f32) {
    let mut salt = 0;
    loop {
        let (x, y) = scatter_position(id, salt);
        let wx = x as f64;
        let wy = y as f64;
        let biome = world_core::biome_at(wx, wy);
        if biome != world_core::Biome::Water && world_core::tile_kind_at(wx, wy) == world_core::TileKind::Empty {
            return (x, y);
        }
        salt += 1;
        if salt > 40 { return (x, y); } // 防呆
    }
}

fn scatter_position(id: (i32, i32, usize), salt: u64) -> (f32, f32) {
    let mut s = (id.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    s = s.wrapping_add((id.1 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    s = s.wrapping_add(id.2 as u64);
    s = s.wrapping_add(salt.wrapping_mul(0x94D0_49BB_1331_11EB));
    
    let x = (id.0 as f32) * CHUNK_SIZE + hash01(s) * CHUNK_SIZE;
    let y = (id.1 as f32) * CHUNK_SIZE + hash01(s.wrapping_add(1)) * CHUNK_SIZE;
    (x, y)
}

fn hash01(n: u64) -> f32 {
    let mut z = n.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f32 / (1u64 << 53) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_field_is_empty() {
        let f = EnemyField::new();
        assert_eq!(f.enemies().len(), 0);
    }

    #[test]
    fn ensure_chunks_generates_enemies() {
        let mut f = EnemyField::new();
        f.ensure_chunks_around(0.0, 0.0, 100.0);
        assert!(f.enemies().len() >= ENEMIES_PER_CHUNK);
    }

    #[test]
    fn enemy_chases_player_and_crosses_chunk() {
        let mut f = EnemyField::new();
        // 生成 (0,0) 區塊，敵人座標約在 (0..512, 0..512)
        f.ensure_chunks_around(256.0, 256.0, 10.0);

        // 把敵人瞬移到區塊邊界 (511, 256)
        {
            let nodes = f.chunks.get_mut(&(0,0)).unwrap();
            nodes[0].x = 511.0;
            nodes[0].y = 256.0;
        }

        // 玩家在 (520, 256) 誘敵 (在 AGGRO_RADIUS 260 內)
        let player = (520.0, 256.0);
        f.advance(1.0, &[player], false, |_, _| false);

        // 敵人應已移入 (1,0) 區塊，且 id 隨之更新（id.0 == 1）
        let new_chunk = f.chunks.get(&(1,0)).expect("敵人應在 (1,0) 區塊");
        assert!(!new_chunk.is_empty());
        // 跨區塊後 id 必須與所在區塊一致（這是核心不變式）
        for e in new_chunk {
            assert_eq!(e.id.0, 1);
            assert_eq!(e.id.1, 0);
        }
        // 舊區塊不再有任何 id.0==0 且 id.1==0 的活著敵人
        let old_chunk = f.chunks.get(&(0,0)).expect("舊區塊仍應存在");
        assert!(old_chunk.is_empty());
    }

    #[test]
    fn enemy_blocked_by_solid_tile_does_not_pass_through() {
        // C-3:敵人撞到實心地形不該穿牆。
        let mut f = EnemyField::new();
        f.ensure_chunks_around(256.0, 256.0, 10.0);
        let ey = {
            let chunk = f.chunks.get_mut(&(0, 0)).unwrap();
            chunk[0].x = 200.0;
            chunk[0].y = 256.0;
            chunk[0].y
        };
        // 牆:x >= 240 一律實心。敵人被右邊玩家 (400,256) 誘往 +x,應被牆擋下、不穿過。
        f.advance(1.0, &[(400.0, 256.0)], false, |x, _y| x >= 240.0);
        let e = &f.chunks.get(&(0, 0)).unwrap()[0];
        assert!(e.x < 240.0, "敵人不該穿牆進實心格, x={}", e.x);
        assert_eq!(e.y, ey, "本例目標同 y,滑行時 y 不該漂");
    }

    #[test]
    fn attack_nearest_after_cross_chunk_does_not_panic() {
        // 重現 panic：敵人跨區塊後 attack_nearest 不應 unwrap 失敗
        let mut f = EnemyField::new();
        f.ensure_chunks_around(256.0, 256.0, 10.0);

        // 瞬移到邊界，讓 advance 把牠送進 (1,0)
        {
            let chunk = f.chunks.get_mut(&(0, 0)).unwrap();
            chunk[0].x = 511.0;
            chunk[0].y = 256.0;
        }
        f.advance(1.0, &[(520.0, 256.0)], false, |_, _| false);

        // 在新位置附近攻擊，不應 panic
        let result = f.attack_nearest(516.0, 256.0, 1);
        assert!(result.is_some());
    }

    #[test]
    fn attack_nearest_hits_enemy() {
        let mut f = EnemyField::new();
        f.ensure_chunks_around(0.0, 0.0, 100.0);
        let target = f.enemies()[0].clone();
        let got = f.attack_nearest(target.x, target.y, 1);
        assert!(got.is_some());
        assert_eq!(got.unwrap().0, target.enemy.kind());
    }

    #[test]
    fn night_enemies_chase_faster_than_day() {
        // 夜間（is_night=true）在同樣 dt 內追擊速度應比白天（is_night=false）快。
        // 直接驗算：以小 dt 讓速度×dt 遠小於目標距離，避免 min(step,dist) 截斷。
        // 白天 CHASE_SPEED=105，夜間 CHASE_SPEED*1.4=147，dt=0.1 => 10.5 vs 14.7 px。
        // 只要敵人在 AGGRO_RADIUS 內並離玩家夠遠（>147 px），兩者差距就能量出來。
        fn measure_chase(is_night: bool) -> f32 {
            let mut f = EnemyField::new();
            f.ensure_chunks_around(0.0, 0.0, CHUNK_SIZE + 10.0);
            let before_enemies = f.enemies();
            let before = before_enemies.iter().find(|e| e.enemy.is_alive()).expect("should have enemy");
            // 在 AGGRO_RADIUS(260) 內但距離 > 147（夜間速度×dt 的最大值）。
            let player = (before.x + 200.0, before.y);
            let bx = before.x;
            let by = before.y;
            f.advance(0.1, &[player], is_night, |_, _| false);
            // 在原位置附近找最近的存活敵人（用距離匹配，避免 enemies() 順序問題）。
            let after = f.enemies().into_iter().filter(|e| e.enemy.is_alive())
                .min_by(|a, b| {
                    let da = (a.x-bx).powi(2)+(a.y-by).powi(2);
                    let db = (b.x-bx).powi(2)+(b.y-by).powi(2);
                    da.partial_cmp(&db).unwrap()
                }).expect("still alive");
            let dx = after.x - bx;
            let dy = after.y - by;
            (dx * dx + dy * dy).sqrt()
        }
        let moved_day = measure_chase(false);
        let moved_night = measure_chase(true);
        assert!(
            moved_night > moved_day,
            "夜間移動距離（{moved_night:.2}）應大於白天（{moved_day:.2}）"
        );
    }
}
