//! AI 導演層＋獸潮攻城（ROADMAP 44）。
//!
//! 純規則導演（**不放 LLM 進遊戲迴圈**）：每 5~10 分鐘輪替觸發一次獸潮，
//! 怪群聚集在主城四個城門外緩衝區叫陣（保護圈照舊進不來）。
//! 全服廣播倒數 30 秒 → 衝擊開始（120 秒）→ 玩家打退 6 隻以上 → 全服獎勵；
//! 時間耗盡則廣播獸潮退去。
//!
//! 導演硬邊界：
//! - 所有怪物注入點確認在 `town_protected_at` 以外。
//! - 事件頻率：最短冷卻 300 秒（勝利後），最長觸發間隔 600 秒（Idle）。
//! - 兇名總數上限由 `enemy_field` 的 `level_cap` 把守，導演不再疊加。

use serde::{Deserialize, Serialize};

use crate::combat::EnemyKind;

// ─── 常數 ──────────────────────────────────────────────────────────────────

/// 廣播倒數秒數（玩家準備時間）。
pub const HORDE_ANNOUNCE_SECS: f32 = 30.0;
/// 攻城持續秒數（超時獸潮退去）。
pub const HORDE_SIEGE_SECS: f32 = 120.0;
/// 獸潮結束後下次觸發的最短冷卻（勝利/退去都一樣）。
pub const HORDE_COOLDOWN_SECS: f32 = 300.0;
/// 每次觸發所需的 Idle 倒數（固定 5 分鐘）。
pub const HORDE_INTERVAL_SECS: f32 = 300.0;
/// 每波注入的怪物數量。
pub const HORDE_WAVE_SIZE: usize = 8;
/// 打退所需的最低斬殺數（6/8 = 75%）。
pub const HORDE_VICTORY_KILLS: u32 = 6;
/// 勝利後全服每人獎勵乙太。
pub const HORDE_VICTORY_ETHER: u32 = 20;
/// 擊殺算入獸潮的最大距離（像素）。
pub const HORDE_KILL_RADIUS: f32 = 650.0;
/// 注入波次的散佈半徑（像素）。
pub const HORDE_SCATTER_RADIUS: f32 = 220.0;

// ─── 攻城點定義 ─────────────────────────────────────────────────────────────

/// 主城：`cgx=73, cgy=71, half_tiles=34`，保護圈 = half_tiles+8 = 42。
/// 攻城點距中心 47 格（47 > 42）確保在保護圈外，有 5 格緩衝區。
const TOWN_CGX: f32 = 73.0;
const TOWN_CGY: f32 = 71.0;
const TILE_PX: f32 = 32.0;
/// 攻城點距城鎮中心格數：需 > half_tiles(34) + 8(保護) + ceil(SCATTER/32)(散佈) ≈ 42 + 7 = 49。
/// 設 55 給足安全邊距（55 - 220/32 ≈ 48.1 > 42）。
const SITE_DIST: f32 = 55.0; // 格

/// 主城四個城門外的攻城點（世界像素座標）。
pub const SIEGE_SITES: [(f32, f32); 4] = [
    (TOWN_CGX * TILE_PX, (TOWN_CGY - SITE_DIST) * TILE_PX), // 北城門外
    (TOWN_CGX * TILE_PX, (TOWN_CGY + SITE_DIST) * TILE_PX), // 南城門外
    ((TOWN_CGX + SITE_DIST) * TILE_PX, TOWN_CGY * TILE_PX), // 東城門外
    ((TOWN_CGX - SITE_DIST) * TILE_PX, TOWN_CGY * TILE_PX), // 西城門外
];

/// 攻城點名稱（對應 SIEGE_SITES 索引）。
pub const SIEGE_LABELS: [&str; 4] = ["北城門外", "南城門外", "東城門外", "西城門外"];

/// 每波怪物組成：混合多種故鄉生態敵人，難度層次分明。
const HORDE_WAVE_KINDS: [EnemyKind; HORDE_WAVE_SIZE] = [
    EnemyKind::FlutterSprite,   // 脆弱（熱身）
    EnemyKind::FlutterSprite,   // 脆弱（熱身）
    EnemyKind::MushroomStalker, // 中等
    EnemyKind::MushroomStalker, // 中等
    EnemyKind::CrystalGolem,   // 較硬
    EnemyKind::CrystalGolem,   // 較硬
    EnemyKind::RuneGuardian,   // 硬
    EnemyKind::RuneGuardian,   // 硬
];

// ─── 型別 ───────────────────────────────────────────────────────────────────

/// 導演對 ws.rs / game.rs 發出的指令。
#[derive(Debug)]
pub enum DirectorCmd {
    /// 廣播「30 秒後獸潮」並注入第一波怪物到攻城點。
    AnnounceHorde {
        site_x:     f32,
        site_y:     f32,
        site_label: &'static str,
        /// 要注入的 (世界像素 x, y, 種類) 列表，呼叫方逐一 inject_event_enemy。
        wave:       Vec<(f32, f32, EnemyKind)>,
    },
    /// 廣播「攻城開始！」（announce_timer 到 0 後）。
    SiegeStart { site_label: &'static str },
    /// 玩家在時限內達成斬殺數，全服勝利。
    HordeVictory { site_label: &'static str, kills: u32 },
    /// 時限耗盡，獸潮自行退去。
    HordeRetreat { site_label: &'static str },
}

/// 導演對前端的快照欄位（序列化後隨 Snapshot 廣播）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HordeView {
    /// `"announcing"` | `"sieging"`
    pub phase:      String,
    pub site_x:     f32,
    pub site_y:     f32,
    pub site_label: String,
    pub secs_left:  u32,
}

/// 導演狀態機。
#[derive(Debug)]
enum HordePhase {
    /// 閒置等待下次觸發。
    Idle { cooldown: f32 },
    /// 廣播倒數中，還沒開始攻城。
    Announcing { secs_left: f32 },
    /// 攻城進行中。
    Sieging { secs_left: f32, kills: u32 },
}

pub struct DirectorState {
    phase:      HordePhase,
    site_index: usize,
}

impl DirectorState {
    pub fn new() -> Self {
        Self {
            phase:      HordePhase::Idle { cooldown: HORDE_INTERVAL_SECS },
            site_index: 0,
        }
    }

    fn current_site(&self) -> (f32, f32, &'static str) {
        let (sx, sy) = SIEGE_SITES[self.site_index];
        (sx, sy, SIEGE_LABELS[self.site_index])
    }

    /// 每幀呼叫一次（dt 秒）；回傳需要執行的指令列表（通常 0~1 個）。
    pub fn tick(&mut self, dt: f32) -> Vec<DirectorCmd> {
        let mut cmds = Vec::new();
        // 先在 match 外取好 site_index，避免不可變借用與可變借用衝突。
        let si = self.site_index;
        match &mut self.phase {
            HordePhase::Idle { cooldown } => {
                *cooldown -= dt;
                if *cooldown <= 0.0 {
                    let (sx, sy) = SIEGE_SITES[si];
                    let label = SIEGE_LABELS[si];
                    self.phase = HordePhase::Announcing { secs_left: HORDE_ANNOUNCE_SECS };
                    cmds.push(DirectorCmd::AnnounceHorde {
                        site_x:     sx,
                        site_y:     sy,
                        site_label: label,
                        wave:       wave_positions(sx, sy),
                    });
                }
            }
            HordePhase::Announcing { secs_left } => {
                *secs_left -= dt;
                if *secs_left <= 0.0 {
                    let label = SIEGE_LABELS[si];
                    self.phase = HordePhase::Sieging { secs_left: HORDE_SIEGE_SECS, kills: 0 };
                    cmds.push(DirectorCmd::SiegeStart { site_label: label });
                }
            }
            HordePhase::Sieging { secs_left, .. } => {
                *secs_left -= dt;
                if *secs_left <= 0.0 {
                    // 時間耗盡 → 退去。在 next_cycle 前先取出 label（避免 borrow 衝突）。
                    let label = SIEGE_LABELS[si];
                    self.next_cycle();
                    cmds.push(DirectorCmd::HordeRetreat { site_label: label });
                }
            }
        }
        cmds
    }

    /// 玩家在攻城點附近擊殺怪物時呼叫（傳入玩家座標即可，ATTACK_REACH 64px 誤差可接受）。
    /// 若達到勝利條件，回傳 `Some(HordeVictory)` 並立刻結算；否則回傳 `None`。
    pub fn register_kill_near_site(&mut self, kill_x: f32, kill_y: f32) -> Option<DirectorCmd> {
        if !matches!(self.phase, HordePhase::Sieging { .. }) { return None; }
        let si = self.site_index;
        let (sx, sy) = SIEGE_SITES[si];
        let dx = kill_x - sx;
        let dy = kill_y - sy;
        if dx * dx + dy * dy > HORDE_KILL_RADIUS * HORDE_KILL_RADIUS {
            return None;
        }
        // 在獨立 block 內操作 kills，讓可變借用在 block 結束後釋放。
        let (trigger_victory, kills_count) = if let HordePhase::Sieging { kills, .. } = &mut self.phase {
            *kills += 1;
            let k = *kills;
            (k >= HORDE_VICTORY_KILLS, k)
        } else {
            unreachable!()
        };
        if trigger_victory {
            let label = SIEGE_LABELS[si];
            self.next_cycle();
            Some(DirectorCmd::HordeVictory { site_label: label, kills: kills_count })
        } else {
            None
        }
    }

    /// 攻城結束（勝利或退去）：進入冷卻並輪換攻城點。
    fn next_cycle(&mut self) {
        self.phase = HordePhase::Idle { cooldown: HORDE_COOLDOWN_SECS };
        self.site_index = (self.site_index + 1) % SIEGE_SITES.len();
    }

    /// 回傳供快照廣播的視圖；`None` 表示目前無事件（玩家端不渲染）。
    pub fn view(&self) -> Option<HordeView> {
        let (sx, sy, label) = self.current_site();
        match &self.phase {
            HordePhase::Idle { .. } => None,
            HordePhase::Announcing { secs_left } => Some(HordeView {
                phase:      "announcing".to_string(),
                site_x:     sx,
                site_y:     sy,
                site_label: label.to_string(),
                secs_left:  secs_left.ceil() as u32,
            }),
            HordePhase::Sieging { secs_left, kills: _ } => Some(HordeView {
                phase:      "sieging".to_string(),
                site_x:     sx,
                site_y:     sy,
                site_label: label.to_string(),
                secs_left:  secs_left.ceil() as u32,
            }),
        }
    }
}

impl Default for DirectorState {
    fn default() -> Self { Self::new() }
}

/// 在攻城點周圍生成 8 個怪物的散佈位置，8 方向均勻分佈。
fn wave_positions(site_x: f32, site_y: f32) -> Vec<(f32, f32, EnemyKind)> {
    let n = HORDE_WAVE_SIZE;
    (0..n).map(|i| {
        let angle = (i as f32) / (n as f32) * std::f32::consts::TAU;
        let wx = site_x + HORDE_SCATTER_RADIUS * angle.cos();
        let wy = site_y + HORDE_SCATTER_RADIUS * angle.sin();
        (wx, wy, HORDE_WAVE_KINDS[i])
    }).collect()
}

// ─── 測試 ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn siege_sites_outside_protected_zone() {
        // 所有攻城點都必須在 town_protected_at 之外。
        for &(sx, sy) in &SIEGE_SITES {
            assert!(
                !world_core::town_protected_at(sx as f64, sy as f64),
                "攻城點 ({}, {}) 在城鎮保護圈內！必須改到圈外。",
                sx, sy
            );
        }
    }

    #[test]
    fn idle_transitions_to_announcing() {
        let mut d = DirectorState::new();
        let cmds = d.tick(HORDE_INTERVAL_SECS + 1.0);
        assert_eq!(cmds.len(), 1, "應觸發 AnnounceHorde");
        assert!(matches!(cmds[0], DirectorCmd::AnnounceHorde { .. }));
    }

    #[test]
    fn announcing_transitions_to_siege_start() {
        let mut d = DirectorState::new();
        d.tick(HORDE_INTERVAL_SECS + 1.0); // 觸發 Announce
        let cmds = d.tick(HORDE_ANNOUNCE_SECS + 1.0);
        assert!(cmds.iter().any(|c| matches!(c, DirectorCmd::SiegeStart { .. })));
    }

    #[test]
    fn siege_timeout_produces_retreat() {
        let mut d = DirectorState::new();
        d.tick(HORDE_INTERVAL_SECS + 1.0); // → Announcing
        d.tick(HORDE_ANNOUNCE_SECS + 1.0); // → Sieging
        let cmds = d.tick(HORDE_SIEGE_SECS + 1.0); // → Retreat
        assert!(cmds.iter().any(|c| matches!(c, DirectorCmd::HordeRetreat { .. })));
    }

    #[test]
    fn enough_kills_produce_victory() {
        let mut d = DirectorState::new();
        d.tick(HORDE_INTERVAL_SECS + 1.0); // → Announcing
        d.tick(HORDE_ANNOUNCE_SECS + 1.0); // → Sieging

        let (sx, sy, _) = d.current_site();
        // 前 5 次不勝利
        for _ in 0..(HORDE_VICTORY_KILLS - 1) {
            assert!(d.register_kill_near_site(sx, sy).is_none());
        }
        // 第 6 次勝利
        let result = d.register_kill_near_site(sx, sy);
        assert!(matches!(result, Some(DirectorCmd::HordeVictory { .. })));
    }

    #[test]
    fn kill_far_from_site_not_counted() {
        let mut d = DirectorState::new();
        d.tick(HORDE_INTERVAL_SECS + 1.0);
        d.tick(HORDE_ANNOUNCE_SECS + 1.0);
        let result = d.register_kill_near_site(0.0, 0.0); // 原點離攻城點很遠
        // 若 SIEGE_SITES 全都 > KILL_RADIUS，才是 None；否則可能巧合 pass。
        // 此測試依賴攻城點不在原點附近（設計保證）。
        let (sx, sy, _) = d.current_site();
        let dist = ((sx * sx + sy * sy) as f64).sqrt() as f32;
        if dist > HORDE_KILL_RADIUS {
            assert!(result.is_none(), "距離 {} px，不應計入", dist);
        }
    }

    #[test]
    fn wave_positions_count_and_outside_protected() {
        let (sx, sy) = SIEGE_SITES[0];
        let wave = wave_positions(sx, sy);
        assert_eq!(wave.len(), HORDE_WAVE_SIZE);
        for &(wx, wy, _) in &wave {
            assert!(
                !world_core::town_protected_at(wx as f64, wy as f64),
                "波次位置 ({}, {}) 在城鎮保護圈內！",
                wx, wy
            );
        }
    }

    #[test]
    fn view_returns_none_when_idle() {
        let d = DirectorState::new();
        assert!(d.view().is_none());
    }

    #[test]
    fn view_returns_announcing_phase() {
        let mut d = DirectorState::new();
        d.tick(HORDE_INTERVAL_SECS + 1.0);
        let v = d.view().expect("Announcing 應有 view");
        assert_eq!(v.phase, "announcing");
    }
}
