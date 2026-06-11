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

// ───── ROADMAP 43 狼群戰術常數 ─────

/// 同種怪響應狼群警報的半徑（像素）。
const PACK_AGGRO_RADIUS: f32 = 400.0;
/// 狼群共同目標持續秒數，過期後清零回正常邏輯。
const PACK_TARGET_DURATION: f32 = 6.0;
/// 殘血呼救加速計時器長度（秒）。
const FLEE_BOOST_DURATION: f32 = 3.5;
/// 呼救加速倍率（+28% 速度，讓救援感受明顯）。
const FLEE_BOOST_MULT: f32 = 1.28;
/// 包夾偏移半徑：各只怪從不同角度逼近，不疊在同一點。
const FLANK_RADIUS: f32 = 32.0;
/// 殘血閾值：HP 低於此比例視為殘血，改逃跑並呼救。
const LOW_HP_THRESHOLD: f32 = 0.25;
/// 兇名精英光環半徑：半徑內同種小怪受到傷害加成。
const NOTORIOUS_AURA_RADIUS: f32 = 240.0;
/// 兇名精英光環傷害加成比例（+15%）。
const NOTORIOUS_DAMAGE_BONUS: f32 = 0.15;

/// 世界裡一隻有座標的敵人。
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedEnemy {
    /// 唯一 ID，格式為 `(chunk_x, chunk_y, index_in_chunk)`。
    pub id: (i32, i32, usize),
    /// 世界座標 X。
    pub x: f32,
    /// 世界座標 Y。
    pub y: f32,
    /// 地理基準等級（確定性，由位置計算，不持久化；死後重置到此值）。
    pub base_level: u32,
    /// 當前等級（可因擊倒玩家成長，死後回 base_level；顯示與縮放皆用此值）。
    pub level: u32,
    /// 敵人本身（生命 / 重生狀態）。
    pub enemy: Enemy,
    /// 狼群共同目標（玩家位置）：同種被攻擊時廣播，計時結束清零（ROADMAP 43）。
    pub pack_target: Option<(f32, f32)>,
    /// pack_target 剩餘有效秒數。
    pub pack_target_timer: f32,
    /// 呼救加速剩餘秒數（殘血同種怪呼救時設定）。
    pub flee_boost_timer: f32,
    /// boss 撤退計時器：> 0 時此怪強制逃離玩家（ROADMAP 117）。
    pub retreat_timer: f32,
}

/// `level_up_nearest_killer` 的回傳值，供呼叫端決定是否廣播兇名精英通告。
#[derive(Debug)]
pub struct EnemyLevelUpResult {
    pub kind: crate::combat::EnemyKind,
    pub new_level: u32,
    /// true = 剛剛跨過「基準+3」門檻，本輪才成為兇名精英。
    pub newly_notorious: bool,
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
                // 狼群計時器倒數（ROADMAP 43）
                if placed.pack_target_timer > 0.0 {
                    placed.pack_target_timer = (placed.pack_target_timer - dt).max(0.0);
                    if placed.pack_target_timer <= 0.0 {
                        placed.pack_target = None;
                    }
                }
                if placed.flee_boost_timer > 0.0 {
                    placed.flee_boost_timer = (placed.flee_boost_timer - dt).max(0.0);
                }
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

        // 狼群前置掃描（ROADMAP 43）：先用不可變借用收集殘血怪的位置，
        // 作為「呼救信號」，主迴圈再設定附近同種怪的加速計時器。
        // 此借用在 collect() 後立即釋放，讓後續 iter_mut 可正常借用。
        let pack_sq = PACK_AGGRO_RADIUS * PACK_AGGRO_RADIUS;
        let flee_signals: Vec<(EnemyKind, f32, f32)> = self.chunks.values()
            .flat_map(|v| v.iter())
            .filter(|e| {
                if !e.enemy.is_alive() { return false; }
                let hp_ratio = e.enemy.remaining_hp() as f32 / e.enemy.max_hp().max(1) as f32;
                if hp_ratio >= LOW_HP_THRESHOLD { return false; }
                players.iter().any(|&(px, py)| {
                    if !px.is_finite() || !py.is_finite() { return false; }
                    let dx = px - e.x; let dy = py - e.y;
                    dx * dx + dy * dy <= aggro_sq
                })
            })
            .map(|e| (e.enemy.kind(), e.x, e.y))
            .collect();

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

                // 呼救加速（ROADMAP 43）：若附近有殘血同種怪正在逃跑，設定加速計時器。
                for &(sig_kind, sig_x, sig_y) in &flee_signals {
                    if sig_kind == placed.enemy.kind() {
                        let dx = sig_x - placed.x; let dy = sig_y - placed.y;
                        if dx * dx + dy * dy <= pack_sq {
                            placed.flee_boost_timer = FLEE_BOOST_DURATION;
                        }
                    }
                }
                let speed_boost = if placed.flee_boost_timer > 0.0 { FLEE_BOOST_MULT } else { 1.0_f32 };

                // 殘血判定（ROADMAP 43）：HP < 25% 且玩家在追擊範圍內 → 逃跑。
                let hp_ratio = placed.enemy.remaining_hp() as f32 / placed.enemy.max_hp().max(1) as f32;
                let is_fleeing = hp_ratio < LOW_HP_THRESHOLD && nearest.is_some();

                // 目標與速度決策（ROADMAP 43 狼群包夾 / 殘血逃跑 / ROADMAP 117 boss 撤退）
                let (target_x, target_y, speed) = if placed.retreat_timer > 0.0 {
                    // ROADMAP 117 boss 撤退：強制逃離玩家，無視 HP 門檻。
                    placed.retreat_timer = (placed.retreat_timer - dt).max(0.0);
                    if let Some((tx, ty, _)) = nearest {
                        let fdx = placed.x - tx;
                        let fdy = placed.y - ty;
                        let fdist = (fdx * fdx + fdy * fdy).sqrt().max(1.0);
                        let flee_x = placed.x + fdx / fdist * 320.0;
                        let flee_y = placed.y + fdy / fdist * 320.0;
                        (flee_x, flee_y, CHASE_SPEED * 1.2 * night_mult)
                    } else {
                        let (hx, hy) = spawn_position(placed.id);
                        (hx, hy, RETURN_SPEED)
                    }
                } else if is_fleeing {
                    // 殘血逃跑：朝玩家反方向移動（is_fleeing 已確認 nearest.is_some()，
                    // 用 if let 保底而非 unwrap，防止競態條件炸死遊戲迴圈）。
                    if let Some((tx, ty, _)) = nearest {
                        let fdx = placed.x - tx;
                        let fdy = placed.y - ty;
                        let fdist = (fdx * fdx + fdy * fdy).sqrt().max(1.0);
                        let flee_x = placed.x + fdx / fdist * 300.0;
                        let flee_y = placed.y + fdy / fdist * 300.0;
                        (flee_x, flee_y, CHASE_SPEED * 0.85 * night_mult * speed_boost)
                    } else {
                        let (hx, hy) = spawn_position(placed.id);
                        (hx, hy, RETURN_SPEED)
                    }
                } else if placed.pack_target_timer > 0.0 {
                    if let Some((ptx, pty)) = placed.pack_target {
                        // 狼群包夾：用 ID 雜湊給每隻怪不同的進攻角度，不疊在同一點。
                        let id_hash = (placed.id.0 as u64)
                            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                            .wrapping_add((placed.id.1 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
                            .wrapping_add(placed.id.2 as u64);
                        let angle = (id_hash >> 11) as f32 / (1u64 << 53) as f32
                            * std::f32::consts::TAU;
                        let fx = ptx + FLANK_RADIUS * angle.cos();
                        let fy = pty + FLANK_RADIUS * angle.sin();
                        (fx, fy, CHASE_SPEED * night_mult * speed_boost)
                    } else {
                        match nearest {
                            Some((tx, ty, _)) => (tx, ty, CHASE_SPEED * night_mult * speed_boost),
                            None => { let (hx, hy) = spawn_position(placed.id); (hx, hy, RETURN_SPEED) }
                        }
                    }
                } else {
                    match nearest {
                        Some((tx, ty, _)) => (tx, ty, CHASE_SPEED * night_mult * speed_boost),
                        None => { let (hx, hy) = spawn_position(placed.id); (hx, hy, RETURN_SPEED) }
                    }
                };

                let dx = target_x - placed.x;
                let dy = target_y - placed.y;
                let dist = (dx * dx + dy * dy).sqrt();

                if dist > 2.0 {
                    let step = (speed * dt).min(dist);
                    let mvx = dx / dist * step;
                    let mvy = dy / dist * step;
                    // 城鎮保護圈是敵人的絕對禁區：不只不在裡面生成，追擊也不得踏入
                    // （含城門口緩衝——否則玩家一出城門就被圍毆，跟沒有城一樣）。
                    // 已在禁區內的（例如事件注入點失誤的個體）放行走出去，**但城內
                    // （牆圈以內）無條件禁入**——否則圈內出生的怪能一路漫遊穿過城門
                    // 進城打人（線上真實事故：裂縫守護者在城裡現身）。
                    let self_in_protected =
                        world_core::town_protected_at(placed.x as f64, placed.y as f64);
                    let blocked = |x: f32, y: f32| {
                        tile_solid(x, y)
                            || world_core::town_interior_at(x as f64, y as f64)
                            || (!self_in_protected
                                && world_core::town_protected_at(x as f64, y as f64))
                    };
                    // C-3 碰撞:不穿實心地形。先試整步,撞牆就沿單軸滑行(能繞牆、別整個卡死)。
                    if !blocked(placed.x + mvx, placed.y + mvy) {
                        placed.x += mvx;
                        placed.y += mvy;
                    } else {
                        if !blocked(placed.x + mvx, placed.y) {
                            placed.x += mvx;
                        }
                        if !blocked(placed.x, placed.y + mvy) {
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

    /// 回傳 `(種類, 等級, 擊殺前是否兇名精英, 掉落)`；等級用於呼叫端縮放 exp。
    /// 若敵人被打倒（掉落 = Some），其等級重置為 base_level、HP 上限也同步回撥。
    pub fn attack_nearest(
        &mut self,
        px: f32,
        py: f32,
        power: u32,
    ) -> Option<(EnemyKind, u32, bool, Option<(ItemKind, u32)>)> {
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
        let mut result: Option<(EnemyKind, u32, bool, Option<(ItemKind, u32)>)> = None;
        let mut pack_kind: Option<EnemyKind> = None;
        let mut trigger_flee_cry = false;

        if let Some((found_chunk, id, _)) = best {
            if let Some(enemies) = self.chunks.get_mut(&found_chunk) {
                if let Some(placed) = enemies.iter_mut().find(|e| e.id == id) {
                    let kind = placed.enemy.kind();
                    let pre_kill_level = placed.level;
                    let was_notorious = placed.level >= placed.base_level.saturating_add(3);
                    let loot = placed.enemy.attack(power);
                    if loot.is_some() {
                        // 被打倒：等級重置為地理基準值，HP 上限同步回撥（重生時滿基準血）。
                        placed.level = placed.base_level;
                        placed.enemy.reset_max_hp_to_base_level(placed.base_level);
                    }
                    // 收集狼群警報資料（借用釋放後才能掃描其他 chunk）（ROADMAP 43）
                    pack_kind = Some(kind);
                    if placed.enemy.is_alive() {
                        let hp_ratio = placed.enemy.remaining_hp() as f32
                            / placed.enemy.max_hp().max(1) as f32;
                        trigger_flee_cry = hp_ratio < LOW_HP_THRESHOLD;
                    }
                    result = Some((kind, pre_kill_level, was_notorious, loot));
                }
            }
        }

        // 狼群警報（ROADMAP 43）：攻擊事件廣播給附近同種怪，讓牠們包夾攻擊者。
        // 若被打怪殘血，同時設定附近同種的呼救加速計時器。
        if let Some(kind) = pack_kind {
            let pack_sq = PACK_AGGRO_RADIUS * PACK_AGGRO_RADIUS;
            for enemies in self.chunks.values_mut() {
                for e in enemies.iter_mut() {
                    if e.enemy.is_alive() && e.enemy.kind() == kind {
                        let dx = e.x - px;
                        let dy = e.y - py;
                        if dx * dx + dy * dy <= pack_sq {
                            e.pack_target = Some((px, py));
                            e.pack_target_timer = PACK_TARGET_DURATION;
                            if trigger_flee_cry {
                                e.flee_boost_timer = FLEE_BOOST_DURATION;
                            }
                        }
                    }
                }
            }
        }

        result
    }

    /// 戰吼（ROADMAP 45）：打中 ATTACK_REACH 內所有存活敵人，回傳全部戰利品清單。
    /// 與 `attack_nearest` 語意相同，差別在不只打最近的那隻。
    pub fn attack_all_in_reach(
        &mut self,
        px: f32,
        py: f32,
        power: u32,
    ) -> Vec<(EnemyKind, u32, bool, Option<(ItemKind, u32)>)> {
        if !px.is_finite() || !py.is_finite() {
            return Vec::new();
        }
        self.ensure_chunks_around(px, py, ATTACK_REACH);
        let (cx, cy) = chunk_key(px, py);
        let reach_sq = ATTACK_REACH * ATTACK_REACH;

        // 收集所有在範圍內且存活的敵人 id + 所在 chunk
        let mut targets: Vec<((i32, i32), (i32, i32, usize))> = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                let ck = (cx + dx, cy + dy);
                if let Some(enemies) = self.chunks.get(&ck) {
                    for placed in enemies {
                        if !placed.enemy.is_alive() { continue; }
                        let ddx = placed.x - px;
                        let ddy = placed.y - py;
                        if ddx * ddx + ddy * ddy <= reach_sq {
                            targets.push((ck, placed.id));
                        }
                    }
                }
            }
        }

        let mut results = Vec::new();
        for (ck, eid) in targets {
            let mut pack_flee = false;
            let mut pack_kind: Option<EnemyKind> = None;
            if let Some(enemies) = self.chunks.get_mut(&ck) {
                if let Some(placed) = enemies.iter_mut().find(|e| e.id == eid) {
                    let kind = placed.enemy.kind();
                    let pre_kill_level = placed.level;
                    let was_notorious = placed.level >= placed.base_level.saturating_add(3);
                    let loot = placed.enemy.attack(power);
                    if loot.is_some() {
                        placed.level = placed.base_level;
                        placed.enemy.reset_max_hp_to_base_level(placed.base_level);
                    }
                    pack_kind = Some(kind);
                    if placed.enemy.is_alive() {
                        let hp_ratio = placed.enemy.remaining_hp() as f32
                            / placed.enemy.max_hp().max(1) as f32;
                        pack_flee = hp_ratio < LOW_HP_THRESHOLD;
                    }
                    results.push((kind, pre_kill_level, was_notorious, loot));
                }
            }
            // 狼群警報（與 attack_nearest 一致）
            if let Some(kind) = pack_kind {
                let pack_sq = PACK_AGGRO_RADIUS * PACK_AGGRO_RADIUS;
                for enemies in self.chunks.values_mut() {
                    for e in enemies.iter_mut() {
                        if e.enemy.is_alive() && e.enemy.kind() == kind {
                            let dx = e.x - px;
                            let dy = e.y - py;
                            if dx * dx + dy * dy <= pack_sq {
                                e.pack_target = Some((px, py));
                                e.pack_target_timer = PACK_TARGET_DURATION;
                                if pack_flee {
                                    e.flee_boost_timer = FLEE_BOOST_DURATION;
                                }
                            }
                        }
                    }
                }
            }
        }
        results
    }

    /// 最近擊倒玩家的敵人升一級（ROADMAP 42）。
    /// 在玩家被打趴後呼叫，找 ATTACK_REACH 內最近的存活敵人，讓牠 +1 級（硬上限 base_level+5）。
    /// 若本次升級使其跨過「base_level+3」門檻，回傳含 newly_notorious=true 的結果供呼叫端廣播。
    pub fn level_up_nearest_killer(&mut self, px: f32, py: f32) -> Option<EnemyLevelUpResult> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        let (cx, cy) = chunk_key(px, py);
        let reach_sq = ATTACK_REACH * ATTACK_REACH;
        let mut best: Option<((i32, i32), (i32, i32, usize), f32)> = None;

        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(enemies) = self.chunks.get(&(cx + dx, cy + dy)) {
                    for placed in enemies {
                        if !placed.enemy.is_alive() { continue; }
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

        if let Some((found_chunk, id, _)) = best {
            if let Some(enemies) = self.chunks.get_mut(&found_chunk) {
                if let Some(placed) = enemies.iter_mut().find(|e| e.id == id) {
                    let was_notorious = placed.level >= placed.base_level.saturating_add(3);
                    let cap = placed.base_level.saturating_add(5);
                    placed.level = (placed.level + 1).min(cap);
                    let newly_notorious = !was_notorious && placed.level >= placed.base_level.saturating_add(3);
                    // 同步 HP 上限（等比例縮放當前血）
                    placed.enemy.update_max_hp_for_level(placed.level);
                    return Some(EnemyLevelUpResult {
                        kind: placed.enemy.kind(),
                        new_level: placed.level,
                        newly_notorious,
                    });
                }
            }
        }
        None
    }

    pub fn threat_at(&self, px: f32, py: f32) -> u32 {
        if !px.is_finite() || !py.is_finite() {
            return 0;
        }

        let (cx, cy) = chunk_key(px, py);
        let reach_sq = ATTACK_REACH * ATTACK_REACH;
        let aura_sq = NOTORIOUS_AURA_RADIUS * NOTORIOUS_AURA_RADIUS;
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
                            let base = crate::combat::scaled_threat(placed.enemy.kind().threat(), placed.level);
                            // 兇名精英光環（ROADMAP 43）：非精英的小怪若附近有同種精英，
                            // 受光環加持造成 +15% 傷害。精英本身不疊加自己的光環。
                            let is_notorious = placed.level >= placed.base_level.saturating_add(3);
                            let threat = if !is_notorious {
                                let has_aura = self.chunks.values()
                                    .flat_map(|v| v.iter())
                                    .any(|e| {
                                        e.enemy.is_alive()
                                            && e.enemy.kind() == placed.enemy.kind()
                                            && e.level >= e.base_level.saturating_add(3)
                                            && {
                                                let ex = e.x - placed.x;
                                                let ey = e.y - placed.y;
                                                ex * ex + ey * ey <= aura_sq
                                            }
                                    });
                                if has_aura {
                                    ((base as f32 * (1.0 + NOTORIOUS_DAMAGE_BONUS)).round() as u32)
                                        .max(base)
                                } else {
                                    base
                                }
                            } else {
                                base
                            };
                            total += threat;
                        }
                    }
                }
            }
        }
        total
    }

    /// 在指定世界座標注入一隻事件敵人（如宇宙裂縫守護者）。
    /// 使用 `index + RIFT_ID_OFFSET` 確保 ID 不與確定性生成的 ID 衝突。
    pub fn inject_event_enemy(&mut self, x: f32, y: f32, kind: EnemyKind) {
        const RIFT_ID_OFFSET: usize = 10000;
        let key = chunk_key(x, y);
        let chunk = self.chunks.entry(key).or_default();
        let idx = chunk.len() + RIFT_ID_OFFSET;
        let base_level = crate::combat::monster_level_at(x, y);
        chunk.push(PlacedEnemy {
            id: (key.0, key.1, idx),
            x,
            y,
            base_level,
            level: base_level,
            enemy: Enemy::new_leveled(kind, base_level),
            pack_target: None,
            pack_target_timer: 0.0,
            flee_boost_timer: 0.0,
            retreat_timer: 0.0,
        });
    }

    pub fn from_saved(saved: Vec<Enemy>) -> Option<Self> {
        let mut field = Self::new();
        for (i, enemy) in saved.into_iter().enumerate() {
            if !enemy.is_loadable() { continue; }
            let id = (0, 0, i);
            let (x, y) = spawn_position(id);
            let base_level = crate::combat::monster_level_at(x, y);
            let key = chunk_key(x, y);
            field.chunks.entry(key).or_default().push(PlacedEnemy {
                x, y, base_level, level: base_level, enemy, id,
                pack_target: None, pack_target_timer: 0.0, flee_boost_timer: 0.0,
                retreat_timer: 0.0,
            });
        }
        Some(field)
    }

    /// 嘗試馴化（ROADMAP 46）：在 `reach` 範圍內找最近的存活且 HP < 25% 的敵人，
    /// 從世界移除並回傳其種類。若無符合條件的敵人回 None。
    /// 呼叫端負責根據種類判斷是否可馴化（`pet::pet_from_enemy_kind`）並扣乙太。
    /// 嘗試馴化：找最近的瀕死（HP < 25%）敵人，只有在 `accept(kind)` 為 true 時才移除並回傳。
    /// 若 `accept` 回傳 false（不可馴化種類或乙太不足），敵人保持原樣不會消失。
    pub fn try_tame_nearest<F>(&mut self, px: f32, py: f32, reach: f32, accept: F) -> Option<EnemyKind>
    where
        F: Fn(EnemyKind) -> bool,
    {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        self.ensure_chunks_around(px, py, reach);
        let (cx, cy) = chunk_key(px, py);
        let reach_sq = reach * reach;
        // 找最近、存活、瀕死（HP < 25%）的敵人，記錄其 chunk key 與索引。
        let mut best: Option<((i32, i32), usize, EnemyKind)> = None;
        let mut best_dist_sq = f32::MAX;

        for dy in -1..=1 {
            for dx in -1..=1 {
                let ck = (cx + dx, cy + dy);
                if let Some(enemies) = self.chunks.get(&ck) {
                    for (idx, placed) in enemies.iter().enumerate() {
                        if !placed.enemy.is_alive() { continue; }
                        let hp_ratio = placed.enemy.remaining_hp() as f32
                            / placed.enemy.max_hp().max(1) as f32;
                        if hp_ratio >= LOW_HP_THRESHOLD { continue; }
                        let ddx = placed.x - px;
                        let ddy = placed.y - py;
                        let dist_sq = ddx * ddx + ddy * ddy;
                        if dist_sq <= reach_sq && dist_sq < best_dist_sq {
                            best_dist_sq = dist_sq;
                            best = Some((ck, idx, placed.enemy.kind()));
                        }
                    }
                }
            }
        }

        // 先確認種類可馴化且條件符合，才實際移除敵人，避免不可馴化種類無聲消失。
        if let Some((ck, idx, kind)) = best {
            if accept(kind) {
                if let Some(enemies) = self.chunks.get_mut(&ck) {
                    if idx < enemies.len() {
                        enemies.remove(idx);
                        return Some(kind);
                    }
                }
            }
        }
        None
    }

    /// 怪物王發布戰術指令（ROADMAP 117）：依 `BossTactic` 對附近的怪施加行為影響。
    ///
    /// - `Surround` / `FocusFire`：COMMAND_RADIUS 內所有活著的小怪設定 `pack_target`（利用現有
    ///   包夾機制，不同 id_hash 決定進攻角度 → 自然從四面夾擊）。
    ///   `FocusFire` 讓所有怪指向 boss 最近的玩家；`Surround` 各用自己最近的玩家。
    /// - `Retreat`：找到 boss 本身，設定 `retreat_timer` 讓他強制逃跑。
    /// - `Rally`：RALLY_RADIUS（600px）內**同種**怪全部設定 `pack_target`。
    pub fn broadcast_boss_command(
        &mut self,
        boss_id: (i32, i32, usize),
        boss_x: f32,
        boss_y: f32,
        tactic: &crate::boss_ai::BossTactic,
        players: &[(f32, f32)],
    ) {
        use crate::boss_ai::{BossTactic, COMMAND_RADIUS, RALLY_RADIUS, TACTIC_DURATION_SECS};

        match tactic {
            BossTactic::Surround | BossTactic::FocusFire => {
                // 集火時所有怪集中指向 boss 最近的玩家；包圍時各自找最近玩家。
                let focus_target: Option<(f32, f32)> = if matches!(tactic, BossTactic::FocusFire) {
                    players.iter()
                        .min_by(|(ax, ay), (bx, by)| {
                            let da = (ax - boss_x).powi(2) + (ay - boss_y).powi(2);
                            let db = (bx - boss_x).powi(2) + (by - boss_y).powi(2);
                            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .copied()
                } else {
                    None
                };

                let cmd_sq = COMMAND_RADIUS * COMMAND_RADIUS;
                for enemies in self.chunks.values_mut() {
                    for e in enemies.iter_mut() {
                        if !e.enemy.is_alive() || e.id == boss_id { continue; }
                        let dx = e.x - boss_x;
                        let dy = e.y - boss_y;
                        if dx * dx + dy * dy > cmd_sq { continue; }
                        let target = focus_target.or_else(|| {
                            players.iter()
                                .min_by(|(ax, ay), (bx, by)| {
                                    let da = (ax - e.x).powi(2) + (ay - e.y).powi(2);
                                    let db = (bx - e.x).powi(2) + (by - e.y).powi(2);
                                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                                })
                                .copied()
                        });
                        if let Some((px, py)) = target {
                            e.pack_target = Some((px, py));
                            e.pack_target_timer = TACTIC_DURATION_SECS;
                        }
                    }
                }
            }
            BossTactic::Retreat => {
                // 找到 boss 本身，設定撤退計時器。
                for enemies in self.chunks.values_mut() {
                    for e in enemies.iter_mut() {
                        if e.id == boss_id {
                            e.retreat_timer = TACTIC_DURATION_SECS;
                            return;
                        }
                    }
                }
            }
            BossTactic::Rally => {
                // 找到 boss 的種類，呼召同種怪湧向最近的玩家。
                let boss_kind = self.chunks.values()
                    .flat_map(|v| v.iter())
                    .find(|e| e.id == boss_id)
                    .map(|e| e.enemy.kind());
                let Some(kind) = boss_kind else { return };
                let nearest_player = players.iter()
                    .min_by(|(ax, ay), (bx, by)| {
                        let da = (ax - boss_x).powi(2) + (ay - boss_y).powi(2);
                        let db = (bx - boss_x).powi(2) + (by - boss_y).powi(2);
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .copied();
                let Some((px, py)) = nearest_player else { return };
                let rally_sq = RALLY_RADIUS * RALLY_RADIUS;
                for enemies in self.chunks.values_mut() {
                    for e in enemies.iter_mut() {
                        if !e.enemy.is_alive() { continue; }
                        if e.enemy.kind() != kind { continue; }
                        let dx = e.x - boss_x;
                        let dy = e.y - boss_y;
                        if dx * dx + dy * dy <= rally_sq {
                            e.pack_target = Some((px, py));
                            e.pack_target_timer = TACTIC_DURATION_SECS;
                        }
                    }
                }
            }
        }
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
        let base_level = crate::combat::monster_level_at(x, y);
        enemies.push(PlacedEnemy {
            id,
            x,
            y,
            base_level,
            level: base_level,
            enemy: Enemy::new_leveled(kind, base_level),
            pack_target: None,
            pack_target_timer: 0.0,
            flee_boost_timer: 0.0,
            retreat_timer: 0.0,
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
    fn enemy_inside_ring_cannot_enter_town_interior() {
        // 在保護圈內出生的怪（事件注入失誤等情況）放行走動讓牠能離開——但**城內
        // 無條件禁入**：把怪放在主城東門正前方（圈內、與門同列），玩家在城中心當餌，
        // 直線追擊路徑正對城門開口，也絕不准穿門進城。
        let east_wall_gx = 73 + 34;
        let ring_x = (east_wall_gx + 3) as f32 * 32.0 + 16.0; // 牆外 3 格＝保護圈內
        let gate_y = 71.5 * 32.0; // 與東門同列
        let mut f = EnemyField::new();
        f.ensure_chunks_around(6000.0, 6000.0, 10.0); // 圈外某 chunk 取得個體
        let key = f.chunks.keys().copied().next().expect("應有 chunk");
        {
            let chunk = f.chunks.get_mut(&key).unwrap();
            assert!(!chunk.is_empty(), "測試 chunk 應有敵人");
            chunk[0].x = ring_x;
            chunk[0].y = gate_y;
        }
        for _ in 0..150 {
            f.advance(1.0 / 15.0, &[(2344.0, gate_y)], true, |_x, _y| false);
        }
        for enemies in f.chunks.values() {
            for e in enemies {
                assert!(
                    !world_core::town_interior_at(e.x as f64, e.y as f64),
                    "圈內怪穿過城門進了城內：({}, {})",
                    e.x,
                    e.y
                );
            }
        }
    }

    #[test]
    fn enemy_never_enters_town_protected_zone() {
        // 圍牆城鎮：玩家在主城內當餌，城外敵人追擊也不得踏入保護圈（牆外 8 格緩衝）。
        // 把敵人放在主城東側保護圈邊界外，多 tick 朝城內玩家追，每步檢查都沒踏進去。
        let (tcx, tcy) = (2344.0_f32, 2272.0_f32); // 主城中心格(73,71)附近
        let edge_x = (73 + 34 + 8) as f32 * 32.0; // 保護圈外緣（格）→ px
        let mut f = EnemyField::new();
        f.ensure_chunks_around(edge_x + 300.0, tcy, 10.0);
        let key = chunk_key(edge_x + 300.0, tcy);
        let chunk = f.chunks.get_mut(&key).unwrap();
        chunk[0].x = edge_x + 300.0;
        chunk[0].y = tcy;
        // 追 10 秒（怪追速遠超過 300px/10s），無地形阻擋——唯一能擋牠的是保護圈。
        for _ in 0..150 {
            f.advance(1.0 / 15.0, &[(tcx, tcy)], true, |_x, _y| false);
        }
        for enemies in f.chunks.values() {
            for e in enemies {
                assert!(
                    !world_core::town_protected_at(e.x as f64, e.y as f64),
                    "敵人踏進了城鎮保護圈：({}, {})",
                    e.x,
                    e.y
                );
            }
        }
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

    // ───── ROADMAP 42 怪物成長生態測試 ─────

    #[test]
    fn enemy_level_resets_on_kill() {
        // 被玩家殺死後，等級應重置為 base_level。
        let mut f = EnemyField::new();
        f.ensure_chunks_around(6000.0, 6000.0, 10.0);
        // 手動調高等級
        let key = *f.chunks.keys().next().unwrap();
        {
            let chunk = f.chunks.get_mut(&key).unwrap();
            assert!(!chunk.is_empty());
            chunk[0].level = chunk[0].base_level + 4;
        }
        let (ex, ey, base) = {
            let e = &f.chunks[&key][0];
            (e.x, e.y, e.base_level)
        };
        // 用一萬點傷害確保一擊必殺
        let result = f.attack_nearest(ex, ey, 10000);
        assert!(result.is_some(), "應能攻擊到敵人");
        let (_, _, was_notorious, loot) = result.unwrap();
        assert!(loot.is_some(), "應有掉落（代表擊殺）");
        assert!(was_notorious, "等級為 base+4 應為兇名精英");
        // 擊殺後 level 應重置為 base_level
        let cur_level = f.chunks[&key][0].level;
        assert_eq!(cur_level, base, "擊殺後等級應回歸 base_level");
    }

    #[test]
    fn level_up_nearest_killer_increments_level() {
        // 模擬玩家被打趴，最近敵人升一級。
        let mut f = EnemyField::new();
        f.ensure_chunks_around(6000.0, 6000.0, 10.0);
        let key = *f.chunks.keys().next().unwrap();
        let (ex, ey, before_level) = {
            let e = &f.chunks[&key][0];
            (e.x, e.y, e.level)
        };
        let result = f.level_up_nearest_killer(ex, ey);
        assert!(result.is_some(), "應找到敵人升級");
        let after_level = f.chunks[&key][0].level;
        assert_eq!(after_level, before_level + 1, "敵人應升一級");
    }

    #[test]
    fn level_up_capped_at_base_plus_5() {
        // 等級上限：base_level + 5，超過不再升。
        let mut f = EnemyField::new();
        f.ensure_chunks_around(6000.0, 6000.0, 10.0);
        let key = *f.chunks.keys().next().unwrap();
        let base = f.chunks[&key][0].base_level;
        // 先將等級調至上限
        f.chunks.get_mut(&key).unwrap()[0].level = base + 5;
        let (ex, ey) = { let e = &f.chunks[&key][0]; (e.x, e.y) };
        f.level_up_nearest_killer(ex, ey);
        assert_eq!(f.chunks[&key][0].level, base + 5, "上限 base+5 不應再升");
    }

    #[test]
    fn newly_notorious_triggers_at_base_plus_3() {
        // 從 base+2 升到 base+3 時 newly_notorious 應為 true。
        let mut f = EnemyField::new();
        f.ensure_chunks_around(6000.0, 6000.0, 10.0);
        let key = *f.chunks.keys().next().unwrap();
        let base = f.chunks[&key][0].base_level;
        f.chunks.get_mut(&key).unwrap()[0].level = base + 2;
        let (ex, ey) = { let e = &f.chunks[&key][0]; (e.x, e.y) };
        let result = f.level_up_nearest_killer(ex, ey);
        let r = result.unwrap();
        assert_eq!(r.new_level, base + 3);
        assert!(r.newly_notorious, "跨過 base+3 門檻時 newly_notorious 應為 true");
    }

    #[test]
    fn attack_nearest_returns_notorious_status() {
        // attack_nearest 第三個元素反映擊殺前是否為兇名精英。
        let mut f = EnemyField::new();
        f.ensure_chunks_around(6000.0, 6000.0, 10.0);
        let key = *f.chunks.keys().next().unwrap();
        {
            let chunk = f.chunks.get_mut(&key).unwrap();
            chunk[0].level = chunk[0].base_level + 3;
        }
        let (ex, ey) = { let e = &f.chunks[&key][0]; (e.x, e.y) };
        let result = f.attack_nearest(ex, ey, 10000);
        let (_, _, was_notorious, loot) = result.unwrap();
        assert!(loot.is_some());
        assert!(was_notorious, "level == base+3 時應回傳 was_notorious=true");
    }

    // ───── ROADMAP 43 狼群戰術測試 ─────

    #[test]
    fn pack_aggro_triggered_on_attack() {
        // 攻擊一隻怪後，附近同種怪應收到 pack_target（ROADMAP 43）。
        let mut f = EnemyField::new();
        // 生成兩個相鄰區塊的怪，確保有同種同星球（世界中心：草原 FlutterSprite）
        f.ensure_chunks_around(6100.0, 6100.0, PACK_AGGRO_RADIUS + 200.0);
        // 找第一隻活著的怪當攻擊目標
        let target_pos = {
            f.chunks.values().flat_map(|v| v.iter())
                .find(|e| e.enemy.is_alive())
                .map(|e| (e.x, e.y))
                .expect("應有敵人")
        };
        let target_kind = {
            f.chunks.values().flat_map(|v| v.iter())
                .find(|e| e.enemy.is_alive() && e.x == target_pos.0 && e.y == target_pos.1)
                .map(|e| e.enemy.kind())
                .unwrap()
        };
        // 只打 1 點傷害，不擊殺，確保仍能廣播狼群警報
        let _result = f.attack_nearest(target_pos.0, target_pos.1, 1);
        // 附近同種怪應有 pack_target
        let any_pack = f.chunks.values().flat_map(|v| v.iter()).any(|e| {
            e.enemy.is_alive() && e.enemy.kind() == target_kind && e.pack_target.is_some()
        });
        assert!(any_pack, "攻擊後附近同種怪應收到 pack_target");
    }

    #[test]
    fn enemy_flees_when_low_hp() {
        // 殘血怪（HP < 25%）在玩家追擊範圍內應逃離，而非追向玩家（ROADMAP 43）。
        let mut f = EnemyField::new();
        f.ensure_chunks_around(6000.0, 6000.0, 10.0);
        let key = *f.chunks.keys().next().unwrap();
        // 取得敵人位置
        let (ex, ey) = { let e = &f.chunks[&key][0]; (e.x, e.y) };
        // 把 HP 降到 10%（殘血）——直接 attack 打到快死
        let max_hp = f.chunks[&key][0].enemy.max_hp();
        let damage = max_hp - max_hp / 10; // 留 10% HP
        f.attack_nearest(ex, ey, damage);
        assert!(f.chunks[&key][0].enemy.is_alive(), "怪應仍存活");

        // 玩家在怪的右側（ex + 150，在 AGGRO_RADIUS 260 內）
        let player_x = ex + 150.0;
        let player_y = ey;
        let before_x = f.chunks[&key][0].x;
        f.advance(1.0, &[(player_x, player_y)], false, |_, _| false);
        let after_x = f.chunks[&key].iter()
            .find(|e| e.enemy.is_alive())
            .map(|e| e.x)
            .unwrap_or(before_x);
        // 逃跑方向應遠離玩家（往左，after_x < before_x）
        assert!(
            after_x < before_x,
            "殘血怪應遠離玩家（逃跑），before_x={before_x:.1} after_x={after_x:.1}"
        );
    }

    #[test]
    fn flee_boost_timer_set_by_flee_signal() {
        // 殘血同種怪的呼救信號應讓附近怪獲得加速計時器（ROADMAP 43）。
        let mut f = EnemyField::new();
        f.ensure_chunks_around(6100.0, 6100.0, PACK_AGGRO_RADIUS + 200.0);
        // 找一隻 max_hp >= 5 的怪：唯有這類怪能「留 1 HP = HP < 25%」同時存活，
        // max_hp <= 4 的怪（FlutterSprite=3, EtherWisp=4）HP 25% < 1 → 只能死，不能進殘血狀態。
        let (first_kind, fx, fy) = match f.chunks.values().flat_map(|v| v.iter())
            .filter(|e| e.enemy.is_alive() && e.enemy.max_hp() >= 5)
            .map(|e| (e.enemy.kind(), e.x, e.y))
            .next() {
            Some(t) => t,
            None => return, // 區域內無可進入殘血狀態的怪，跳過測試
        };

        // 打到剩 1 HP（確保 hp_ratio = 1/max_hp < 25%，且仍存活）
        let max_hp = f.chunks.values().flat_map(|v| v.iter())
            .find(|e| e.x == fx && e.y == fy)
            .map(|e| e.enemy.max_hp()).unwrap_or(10);
        let damage = max_hp - 1; // 留 1 HP
        if damage > 0 { f.attack_nearest(fx, fy, damage); }

        // 在殘血怪旁邊放一個玩家，讓 advance 的前置掃描觸發呼救信號
        f.advance(0.05, &[(fx + 50.0, fy)], false, |_, _| false);

        // 殘血怪本身或附近同種怪應有 flee_boost_timer > 0
        let boosted = f.chunks.values().flat_map(|v| v.iter()).any(|e| {
            e.enemy.is_alive() && e.enemy.kind() == first_kind && e.flee_boost_timer > 0.0
        });
        assert!(boosted, "殘血怪呼救後，附近同種怪應有 flee_boost_timer > 0");
    }

    #[test]
    fn notorious_aura_increases_threat() {
        // 附近有兇名精英時，同種小怪造成的傷害應多 15%（ROADMAP 43）。
        let mut f = EnemyField::new();
        f.ensure_chunks_around(6000.0, 6000.0, 200.0);
        let key = *f.chunks.keys().next().unwrap();
        // 確保至少有一隻存活怪
        let e = &f.chunks[&key][0];
        assert!(e.enemy.is_alive());
        let (ex, ey) = (e.x, e.y);

        // 量基準威脅（無精英）
        let base_threat = f.threat_at(ex, ey);

        // 在同位置注入一隻同種兇名精英（level = base + 3）
        let kind = f.chunks[&key][0].enemy.kind();
        let base_level = f.chunks[&key][0].base_level;
        let chunk = f.chunks.get_mut(&key).unwrap();
        let idx = chunk.len() + 5000;
        chunk.push(PlacedEnemy {
            id: (key.0, key.1, idx),
            x: ex + 10.0, // 距小怪 10px，在光環範圍內
            y: ey,
            base_level,
            level: base_level + 3,
            enemy: crate::combat::Enemy::new_leveled(kind, base_level + 3),
            pack_target: None,
            pack_target_timer: 0.0,
            flee_boost_timer: 0.0,
            retreat_timer: 0.0,
        });

        // 精英本身也在 ATTACK_REACH 內（10px），會加入 threat；
        // 關注的是小怪受光環加成後總威脅應高於純基準。
        let aura_threat = f.threat_at(ex, ey);
        assert!(
            aura_threat > base_threat,
            "兇名精英光環應使附近同種怪威脅提升，base={base_threat} aura={aura_threat}"
        );
    }

    #[test]
    fn broadcast_surround_sets_pack_target_on_nearby_enemies() {
        // ROADMAP 117：broadcast_boss_command Surround 應對附近小怪設 pack_target。
        // 直接手動注入 boss + minion，確保座標在 COMMAND_RADIUS 內，不依賴區塊生成分佈。
        use crate::combat::{Enemy, EnemyKind};
        use world_core::chunk_key;

        let mut f = EnemyField::new();
        let boss_x = 7000.0_f32;
        let boss_y = 7000.0_f32;
        let boss_id = (13i32, 13i32, 0usize);
        let minion_id = (13i32, 13i32, 1usize);

        let key = chunk_key(boss_x, boss_y);
        f.chunks.entry(key).or_default().push(PlacedEnemy {
            id: boss_id, x: boss_x, y: boss_y,
            base_level: 1, level: 4,
            enemy: Enemy::new_leveled(EnemyKind::RuneGuardian, 4),
            pack_target: None, pack_target_timer: 0.0,
            flee_boost_timer: 0.0, retreat_timer: 0.0,
        });
        // minion 100px 旁邊，在 COMMAND_RADIUS(500px) 內。
        f.chunks.entry(key).or_default().push(PlacedEnemy {
            id: minion_id, x: boss_x + 100.0, y: boss_y,
            base_level: 1, level: 1,
            enemy: Enemy::new_leveled(EnemyKind::RuneGuardian, 1),
            pack_target: None, pack_target_timer: 0.0,
            flee_boost_timer: 0.0, retreat_timer: 0.0,
        });

        let players = vec![(boss_x + 300.0, boss_y)];
        f.broadcast_boss_command(boss_id, boss_x, boss_y, &crate::boss_ai::BossTactic::Surround, &players);

        let minion = f.chunks[&key].iter().find(|e| e.id == minion_id).unwrap();
        assert!(
            minion.pack_target.is_some(),
            "Surround 指令應對 COMMAND_RADIUS 內的小怪設定 pack_target"
        );
    }

    #[test]
    fn retreat_timer_causes_boss_to_flee() {
        // ROADMAP 117：retreat_timer > 0 時 boss 應遠離玩家而非追擊。
        let mut f = EnemyField::new();
        f.ensure_chunks_around(6200.0, 6200.0, 200.0);
        let key = *f.chunks.keys().next().unwrap();
        // 直接設定 retreat_timer。
        let (bx, by) = {
            let e = &mut f.chunks.get_mut(&key).unwrap()[0];
            e.retreat_timer = crate::boss_ai::TACTIC_DURATION_SECS;
            (e.x, e.y)
        };
        // 玩家在 boss 右方 200px。
        let player_pos = (bx + 200.0, by);
        let before_x = bx;
        f.advance(0.2, &[player_pos], false, |_, _| false);
        let after_x = f.chunks.values().flat_map(|v| v.iter()).next().map(|e| e.x).unwrap_or(bx);
        assert!(
            after_x < before_x + 1.0,
            "撤退計時器啟動時 boss 應遠離右方玩家，before={before_x:.1} after={after_x:.1}"
        );
    }
}
