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

    pub fn advance(&mut self, dt: f32, players: &[(f32, f32)]) {
        if dt <= 0.0 {
            return;
        }
        let aggro_sq = AGGRO_RADIUS * AGGRO_RADIUS;
        
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
                    Some((tx, ty, _)) => (tx, ty, CHASE_SPEED),
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
                    placed.x += dx / dist * step;
                    placed.y += dy / dist * step;
                    
                    let new_key = chunk_key(placed.x, placed.y);
                    if new_key != (cx, cy) {
                        to_move.push(((cx, cy), idx, new_key));
                    }
                }
            }
        }

        // 處理跨區塊移動 (從後往前移以保持索引有效)
        to_move.sort_by_key(|&(_, idx, _)| std::cmp::Reverse(idx));
        for (old_key, idx, new_key) in to_move {
            let enemy = self.chunks.get_mut(&old_key).unwrap().remove(idx);
            self.chunks.entry(new_key).or_default().push(enemy);
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
        let mut best: Option<((i32, i32, usize), f32)> = None;
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
                            if best.as_ref().map_or(true, |(_, b)| dist_sq < *b) {
                                best = Some((placed.id, dist_sq));
                            }
                        }
                    }
                }
            }
        }

        if let Some((id, _)) = best {
            let key = (id.0, id.1);
            let enemies = self.chunks.get_mut(&key).unwrap();
            let placed = enemies.iter_mut().find(|e| e.id == id).unwrap();
            let kind = placed.enemy.kind();
            let loot = placed.enemy.attack(power);
            Some((kind, loot))
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

/// 依生態域決定敵人種類：自然靈境（草地/森林）孕育迷途乙太靈，
/// 廢棄機械殘骸（岩地/沙漠）藏著銹蝕巡邏機。
fn kind_for_biome(biome: world_core::Biome) -> EnemyKind {
    use world_core::Biome;
    match biome {
        Biome::Meadow | Biome::Forest => EnemyKind::EtherWisp,
        Biome::Rocky | Biome::Sand | Biome::Water => EnemyKind::ScrapDrone,
    }
}

fn generate_chunk(cx: i32, cy: i32) -> Vec<PlacedEnemy> {
    let mut enemies = Vec::new();
    for i in 0..ENEMIES_PER_CHUNK {
        let id = (cx, cy, i);
        let (x, y) = spawn_position(id);
        let biome = world_core::biome_at(x as f64, y as f64);
        let kind = kind_for_biome(biome);
        enemies.push(PlacedEnemy {
            id,
            x,
            y,
            enemy: Enemy::new(kind),
        });
    }
    enemies
}

/// 在區塊內找一個非水域的落點（所有生態域都能出現敵人）。
fn spawn_position(id: (i32, i32, usize)) -> (f32, f32) {
    let mut salt = 0;
    loop {
        let (x, y) = scatter_position(id, salt);
        let biome = world_core::biome_at(x as f64, y as f64);
        if biome != world_core::Biome::Water {
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
        let target_id = f.enemies()[0].id;
        
        // 把敵人瞬移到區塊邊界 (511, 256)
        {
            let nodes = f.chunks.get_mut(&(0,0)).unwrap();
            nodes[0].x = 511.0;
            nodes[0].y = 256.0;
        }
        
        // 玩家在 (520, 256) 誘敵 (在 AGGRO_RADIUS 260 內)
        let player = (520.0, 256.0);
        f.advance(1.0, &[player]);
        
        // 敵人應已移入 (1,0) 區塊
        assert!(f.chunks.get(&(1,0)).is_some());
        assert!(f.chunks.get(&(1,0)).unwrap().iter().any(|e| e.id == target_id));
        assert!(f.chunks.get(&(0,0)).unwrap().iter().all(|e| e.id != target_id));
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
}
