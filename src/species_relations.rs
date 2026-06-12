//! ROADMAP 144：人類↔物種湧現關係——跨物種好惡從互動長出。
//! ROADMAP 163：怪物=物種 phase 1——怪物種類納入物種關係框架。
//!
//! 每個野生動物物種 / 怪物種類對「人類」有一個態度值（0-100，50=中立）。
//! 野生動物態度影響：
//!   - 攻擊/殺死獵物物種 → 該物種敵意+（-12）
//!   - 殺死掠食者        → 被捕獵的獵物物種好感+（+10）
//!   - 餵食野生動物      → 該物種好感+（+15）
//! 怪物物種態度影響：
//!   - 玩家殺死某種怪 → 該物種敬畏人類（+8）
//!   - 某種怪擊倒玩家 → 該物種更加囂張（-10）
//! 態度層級影響怪物 aggro 半徑：友善 ×0.35 / 中立 ×1.0 / 警覺 ×1.3 / 敵視 ×1.6。
//!
//! 零 migration，記憶體模式，重啟清零（對應生態系「換季重生」哲學）。

use std::collections::HashMap;
use crate::wildlife::WildlifeKind;
use crate::combat::EnemyKind;

// ─── 常數 ────────────────────────────────────────────────────────────────────

/// 殺死獵物物種的態度懲罰。
const KILL_PREY_PENALTY: i32 = -12;
/// 殺死掠食者後對應獵物物種的態度獎勵。
const KILL_PREDATOR_REWARD: i32 = 10;
/// 餵食野生動物的態度獎勵。
const FEED_REWARD: i32 = 15;
/// 態度每次自然衰減的幅度（每 DECAY_INTERVAL_SECS 秒靠近 50 一點）。
const DECAY_AMOUNT: i32 = 1;
/// 自然衰減的時間間隔（秒）。
pub const DECAY_INTERVAL_SECS: f32 = 120.0;
/// 友善閾值（≥ 此值 → 友善）。
pub const FRIENDLY_THRESHOLD: i32 = 65;
/// 敵視閾值（< 此值 → 敵視）。
pub const HOSTILE_THRESHOLD: i32 = 25;
/// 警覺閾值（< 此值且 ≥ HOSTILE → 警覺）。
pub const WARY_THRESHOLD: i32 = 45;
/// 態度層級改變時的廣播冷卻（秒）——防止層級反覆刷頻。
const TIER_BROADCAST_COOLDOWN: f32 = 60.0;
/// 餵食距離（像素）——玩家必須距野生動物多近才能餵。
pub const FEED_REACH: f32 = 100.0;
/// 攻擊野生動物的距離（像素）——同現有 ATTACK_REACH。
pub const ATTACK_WILDLIFE_REACH: f32 = 64.0;
/// 敵視野生動物造成的傷害（HP）。
pub const HOSTILE_WILDLIFE_DAMAGE: u32 = 2;

// ─── 所有物種清單 ─────────────────────────────────────────────────────────────

pub const ALL_KINDS: &[WildlifeKind] = &[
    WildlifeKind::WildBird,
    WildlifeKind::WildDeer,
    WildlifeKind::SmallCritter,
    WildlifeKind::WildWolf,
    WildlifeKind::WildFox,
];

// ─── 資料結構 ────────────────────────────────────────────────────────────────

/// 物種態度層級——決定野生動物的行為模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationTier {
    Friendly,
    Neutral,
    Wary,
    Hostile,
}

impl RelationTier {
    pub fn as_str(self) -> &'static str {
        match self {
            RelationTier::Friendly => "friendly",
            RelationTier::Neutral  => "neutral",
            RelationTier::Wary     => "wary",
            RelationTier::Hostile  => "hostile",
        }
    }
    pub fn display_zh(self) -> &'static str {
        match self {
            RelationTier::Friendly => "🟢 友善",
            RelationTier::Neutral  => "⚪ 中立",
            RelationTier::Wary     => "🟡 警覺",
            RelationTier::Hostile  => "🔴 敵視",
        }
    }
}

/// 依態度值回傳對應層級。
pub fn tier_from_attitude(attitude: i32) -> RelationTier {
    if attitude >= FRIENDLY_THRESHOLD {
        RelationTier::Friendly
    } else if attitude >= WARY_THRESHOLD {
        RelationTier::Neutral
    } else if attitude >= HOSTILE_THRESHOLD {
        RelationTier::Wary
    } else {
        RelationTier::Hostile
    }
}

// ─── 事件 ────────────────────────────────────────────────────────────────────

pub enum SpeciesRelationEvent {
    /// 態度層級改變——應廣播至全服聊天。
    TierChanged {
        kind: WildlifeKind,
        new_tier: RelationTier,
    },
}

// ─── 主結構 ──────────────────────────────────────────────────────────────────

/// 追蹤所有物種對人類的整體態度（0-100，50=中立）。
pub struct SpeciesRelations {
    /// 物種 → 態度值。
    pub attitudes: HashMap<WildlifeKind, i32>,
    /// 自然衰減計時器（秒）。
    decay_timer: f32,
    /// 各物種的層級廣播冷卻（秒）——防止層級反覆刷頻。
    tier_cooldowns: HashMap<WildlifeKind, f32>,
    /// 各物種上次廣播的層級（用於偵測改變）。
    last_tiers: HashMap<WildlifeKind, RelationTier>,
}

impl SpeciesRelations {
    pub fn new() -> Self {
        let mut attitudes = HashMap::new();
        let mut tier_cooldowns = HashMap::new();
        let mut last_tiers = HashMap::new();
        for &kind in ALL_KINDS {
            attitudes.insert(kind, 50);
            tier_cooldowns.insert(kind, 0.0);
            last_tiers.insert(kind, RelationTier::Neutral);
        }
        Self {
            attitudes,
            decay_timer: 0.0,
            tier_cooldowns,
            last_tiers,
        }
    }

    /// 玩家攻擊/殺死獵物物種 → 該物種敵意+。
    pub fn on_kill_prey(&mut self, kind: WildlifeKind) {
        self.adjust(kind, KILL_PREY_PENALTY);
    }

    /// 玩家殺死掠食者 → 被獵物種獲得好感。
    /// WildWolf 獵 WildDeer；WildFox 獵 SmallCritter。
    pub fn on_kill_predator(&mut self, predator_kind: WildlifeKind) {
        if let Some(prey_kind) = predator_kind.hunts() {
            self.adjust(prey_kind, KILL_PREDATOR_REWARD);
        }
    }

    /// 玩家餵食野生動物 → 該物種好感+。
    pub fn on_feed(&mut self, kind: WildlifeKind) {
        self.adjust(kind, FEED_REWARD);
    }

    /// 取得物種目前態度值（0-100）。
    pub fn attitude(&self, kind: WildlifeKind) -> i32 {
        *self.attitudes.get(&kind).unwrap_or(&50)
    }

    /// 取得物種目前態度層級。
    pub fn tier(&self, kind: WildlifeKind) -> RelationTier {
        tier_from_attitude(self.attitude(kind))
    }

    /// 取得所有物種的態度視圖（供快照廣播）。
    pub fn views(&self) -> Vec<SpeciesAttitudeView> {
        ALL_KINDS.iter().map(|&kind| SpeciesAttitudeView {
            kind: kind.as_str().to_string(),
            name: kind.display_name().to_string(),
            attitude: self.attitude(kind),
            tier: self.tier(kind).as_str().to_string(),
        }).collect()
    }

    /// 每幀推進：自然衰減 + 層級改變偵測，回傳本幀事件。
    pub fn tick(&mut self, dt: f32) -> Vec<SpeciesRelationEvent> {
        let mut events = Vec::new();

        // 冷卻計時器倒數。
        for cd in self.tier_cooldowns.values_mut() {
            *cd = (*cd - dt).max(0.0);
        }

        // 自然衰減（每 DECAY_INTERVAL_SECS 秒向 50 靠近一點）。
        self.decay_timer += dt;
        if self.decay_timer >= DECAY_INTERVAL_SECS {
            self.decay_timer = 0.0;
            for &kind in ALL_KINDS {
                let v = self.attitudes.entry(kind).or_insert(50);
                if *v > 50 { *v -= DECAY_AMOUNT; }
                else if *v < 50 { *v += DECAY_AMOUNT; }
            }
        }

        // 偵測層級改變並發出事件（有冷卻）。
        for &kind in ALL_KINDS {
            let current_tier = self.tier(kind);
            let last = *self.last_tiers.get(&kind).unwrap_or(&RelationTier::Neutral);
            let cooldown = *self.tier_cooldowns.get(&kind).unwrap_or(&0.0);
            if current_tier != last && cooldown <= 0.0 {
                *self.last_tiers.entry(kind).or_insert(RelationTier::Neutral) = current_tier;
                *self.tier_cooldowns.entry(kind).or_insert(0.0) = TIER_BROADCAST_COOLDOWN;
                events.push(SpeciesRelationEvent::TierChanged { kind, new_tier: current_tier });
            }
        }

        events
    }

    // ── 私有輔助 ─────────────────────────────────────────────────────────────

    fn adjust(&mut self, kind: WildlifeKind, delta: i32) {
        let v = self.attitudes.entry(kind).or_insert(50);
        *v = (*v + delta).clamp(0, 100);
    }
}

/// 供快照廣播的物種態度視圖。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeciesAttitudeView {
    pub kind: String,
    pub name: String,
    pub attitude: i32,
    pub tier: String,
}

// ─── 測試 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_species_start_neutral() {
        let sr = SpeciesRelations::new();
        for &kind in ALL_KINDS {
            assert_eq!(sr.attitude(kind), 50);
            assert_eq!(sr.tier(kind), RelationTier::Neutral);
        }
    }

    #[test]
    fn kill_prey_reduces_attitude() {
        let mut sr = SpeciesRelations::new();
        sr.on_kill_prey(WildlifeKind::WildBird);
        assert_eq!(sr.attitude(WildlifeKind::WildBird), 50 + KILL_PREY_PENALTY);
    }

    #[test]
    fn kill_predator_raises_prey_attitude_wolf_deer() {
        let mut sr = SpeciesRelations::new();
        sr.on_kill_predator(WildlifeKind::WildWolf);
        assert_eq!(sr.attitude(WildlifeKind::WildDeer), 50 + KILL_PREDATOR_REWARD);
        // 狐狸不受影響
        assert_eq!(sr.attitude(WildlifeKind::WildFox), 50);
    }

    #[test]
    fn kill_predator_raises_prey_attitude_fox_critter() {
        let mut sr = SpeciesRelations::new();
        sr.on_kill_predator(WildlifeKind::WildFox);
        assert_eq!(sr.attitude(WildlifeKind::SmallCritter), 50 + KILL_PREDATOR_REWARD);
        assert_eq!(sr.attitude(WildlifeKind::WildDeer), 50);
    }

    #[test]
    fn feed_raises_attitude() {
        let mut sr = SpeciesRelations::new();
        sr.on_feed(WildlifeKind::WildDeer);
        assert_eq!(sr.attitude(WildlifeKind::WildDeer), 50 + FEED_REWARD);
    }

    #[test]
    fn attitude_clamps_to_0_100() {
        let mut sr = SpeciesRelations::new();
        for _ in 0..10 {
            sr.on_kill_prey(WildlifeKind::WildBird);
        }
        assert!(sr.attitude(WildlifeKind::WildBird) >= 0);
        for _ in 0..10 {
            sr.on_feed(WildlifeKind::WildBird);
        }
        assert!(sr.attitude(WildlifeKind::WildBird) <= 100);
    }

    #[test]
    fn tier_transitions_correctly() {
        assert_eq!(tier_from_attitude(65), RelationTier::Friendly);
        assert_eq!(tier_from_attitude(64), RelationTier::Neutral);
        assert_eq!(tier_from_attitude(45), RelationTier::Neutral);
        assert_eq!(tier_from_attitude(44), RelationTier::Wary);
        assert_eq!(tier_from_attitude(25), RelationTier::Wary);
        assert_eq!(tier_from_attitude(24), RelationTier::Hostile);
        assert_eq!(tier_from_attitude(0),  RelationTier::Hostile);
        assert_eq!(tier_from_attitude(100), RelationTier::Friendly);
    }

    #[test]
    fn kill_predator_has_no_effect_for_prey_kinds() {
        let mut sr = SpeciesRelations::new();
        sr.on_kill_predator(WildlifeKind::WildBird); // 野鳥不是掠食者，hunts() = None
        // 所有態度應維持 50
        for &kind in ALL_KINDS {
            assert_eq!(sr.attitude(kind), 50);
        }
    }

    #[test]
    fn tier_change_event_emitted_on_attitude_change() {
        let mut sr = SpeciesRelations::new();
        // 把野鹿拉到敵視層級（50 - 36 = 14 < 25）
        for _ in 0..3 {
            sr.on_kill_prey(WildlifeKind::WildDeer);
        }
        let events = sr.tick(1.0);
        assert!(events.iter().any(|e| matches!(e,
            SpeciesRelationEvent::TierChanged {
                kind: WildlifeKind::WildDeer,
                new_tier: RelationTier::Hostile
            }
        )));
    }

    #[test]
    fn tier_event_has_cooldown() {
        let mut sr = SpeciesRelations::new();
        // 讓鹿進入友善（50+30=80 ≥ 65）
        for _ in 0..2 {
            sr.on_feed(WildlifeKind::WildDeer);
        }
        let events1 = sr.tick(1.0);
        assert!(events1.iter().any(|e| matches!(e, SpeciesRelationEvent::TierChanged { .. })));
        // 馬上再 tick → 冷卻中，不應再發事件
        let events2 = sr.tick(1.0);
        assert!(!events2.iter().any(|e| matches!(e, SpeciesRelationEvent::TierChanged { .. })));
    }

    #[test]
    fn natural_decay_toward_neutral() {
        let mut sr = SpeciesRelations::new();
        // 把野鳥友善（50+30=80）
        sr.on_feed(WildlifeKind::WildBird);
        sr.on_feed(WildlifeKind::WildBird);
        let before = sr.attitude(WildlifeKind::WildBird);
        // 等超過 DECAY_INTERVAL_SECS
        sr.tick(DECAY_INTERVAL_SECS + 1.0);
        let after = sr.attitude(WildlifeKind::WildBird);
        assert!(after < before, "態度應向 50 靠近：before={before}, after={after}");
    }

    #[test]
    fn views_returns_all_five_species() {
        let sr = SpeciesRelations::new();
        let views = sr.views();
        assert_eq!(views.len(), 5);
        assert!(views.iter().any(|v| v.kind == "wild_bird"));
        assert!(views.iter().any(|v| v.kind == "wild_wolf"));
    }

    #[test]
    fn friendly_threshold_and_hostile_threshold_are_reasonable() {
        assert!(FRIENDLY_THRESHOLD > 50, "友善閾值應高於中立");
        assert!(HOSTILE_THRESHOLD < 50, "敵視閾值應低於中立");
        assert!(WARY_THRESHOLD < FRIENDLY_THRESHOLD);
        assert!(HOSTILE_THRESHOLD < WARY_THRESHOLD);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// ROADMAP 163：怪物=物種 phase 1——怪物種類的集體態度追蹤。
// ════════════════════════════════════════════════════════════════════════════

// ─── 怪物物種常數 ─────────────────────────────────────────────────────────────

/// 玩家擊殺怪物後該物種的態度獎勵（怪物學會敬畏人類）。
const MONSTER_KILL_REWARD: i32 = 8;
/// 怪物擊倒玩家後該物種的態度懲罰（怪物氣焰更盛）。
const MONSTER_PLAYER_KILL_PENALTY: i32 = -10;
/// 怪物態度自然衰減間隔（秒）——比野生動物慢，怪物記性更長。
pub const MONSTER_DECAY_INTERVAL_SECS: f32 = 180.0;

/// 所有 14 種怪物種類。
pub const ALL_MONSTER_KINDS: &[EnemyKind] = &[
    EnemyKind::ScrapDrone,
    EnemyKind::EtherWisp,
    EnemyKind::FlutterSprite,
    EnemyKind::MushroomStalker,
    EnemyKind::CrystalGolem,
    EnemyKind::RuneGuardian,
    EnemyKind::CoralCrab,
    EnemyKind::JadeWraith,
    EnemyKind::SteamConstruct,
    EnemyKind::VoidPhantom,
    EnemyKind::AetherSpecter,
    EnemyKind::OriginGuardian,
    EnemyKind::RiftGuardian,
    EnemyKind::EtherOverlord,
];

// ─── aggro 半徑倍率 ───────────────────────────────────────────────────────────

/// 依怪物態度層級取得 aggro 半徑倍率。
/// - 友善：×0.35（幾乎不主動追——玩家大量擊殺後怪物學會敬而遠之）
/// - 中立：×1.0（正常）
/// - 警覺：×1.3（稍大 aggro，略顯緊張）
/// - 敵視：×1.6（遠距離就衝來，玩家讓該種族吃了大虧）
pub fn aggro_multiplier_for_tier(tier: RelationTier) -> f32 {
    match tier {
        RelationTier::Friendly => 0.35,
        RelationTier::Neutral  => 1.0,
        RelationTier::Wary     => 1.3,
        RelationTier::Hostile  => 1.6,
    }
}

// ─── 事件 ────────────────────────────────────────────────────────────────────

pub enum MonsterRelationEvent {
    /// 某怪物種類的態度層級改變——應廣播至全服聊天。
    TierChanged {
        kind: EnemyKind,
        new_tier: RelationTier,
    },
}

// ─── 主結構 ──────────────────────────────────────────────────────────────────

/// 追蹤所有怪物種類對人類的整體態度（0-100，50=中立）。
/// 態度隨玩家殺怪/被怪殺而變動；每 MONSTER_DECAY_INTERVAL_SECS 秒向 50 靠近 1 點。
pub struct MonsterSpeciesRelations {
    pub attitudes: HashMap<EnemyKind, i32>,
    decay_timer: f32,
    tier_cooldowns: HashMap<EnemyKind, f32>,
    last_tiers: HashMap<EnemyKind, RelationTier>,
}

impl MonsterSpeciesRelations {
    pub fn new() -> Self {
        let mut attitudes = HashMap::new();
        let mut tier_cooldowns = HashMap::new();
        let mut last_tiers = HashMap::new();
        for &kind in ALL_MONSTER_KINDS {
            attitudes.insert(kind, 50);
            tier_cooldowns.insert(kind, 0.0);
            last_tiers.insert(kind, RelationTier::Neutral);
        }
        Self {
            attitudes,
            decay_timer: 0.0,
            tier_cooldowns,
            last_tiers,
        }
    }

    /// 玩家擊殺某種怪 → 該物種學會敬畏人類，態度+。
    pub fn on_player_kills_monster(&mut self, kind: EnemyKind) {
        self.adjust(kind, MONSTER_KILL_REWARD);
    }

    /// 某種怪擊倒玩家 → 該物種更加囂張，態度-。
    pub fn on_monster_kills_player(&mut self, kind: EnemyKind) {
        self.adjust(kind, MONSTER_PLAYER_KILL_PENALTY);
    }

    /// 取得怪物種類目前態度值（0-100）。
    pub fn attitude(&self, kind: EnemyKind) -> i32 {
        *self.attitudes.get(&kind).unwrap_or(&50)
    }

    /// 取得怪物種類目前態度層級。
    pub fn tier(&self, kind: EnemyKind) -> RelationTier {
        tier_from_attitude(self.attitude(kind))
    }

    /// 取得某種怪的 aggro 半徑倍率（供 enemy_field 使用）。
    pub fn aggro_multiplier(&self, kind: EnemyKind) -> f32 {
        aggro_multiplier_for_tier(self.tier(kind))
    }

    /// 建立所有怪物種類的 aggro 倍率快照（HashMap 格式，供 enemy_field 一次性更新）。
    pub fn aggro_multipliers_snapshot(&self) -> HashMap<EnemyKind, f32> {
        ALL_MONSTER_KINDS.iter()
            .map(|&kind| (kind, self.aggro_multiplier(kind)))
            .collect()
    }

    /// 取得所有怪物種類的態度視圖（供快照廣播）。
    pub fn views(&self) -> Vec<SpeciesAttitudeView> {
        ALL_MONSTER_KINDS.iter().map(|&kind| SpeciesAttitudeView {
            kind: kind.as_str().to_string(),
            name: kind.display_name().to_string(),
            attitude: self.attitude(kind),
            tier: self.tier(kind).as_str().to_string(),
        }).collect()
    }

    /// 每幀推進：自然衰減 + 層級改變偵測，回傳本幀事件。
    pub fn tick(&mut self, dt: f32) -> Vec<MonsterRelationEvent> {
        let mut events = Vec::new();

        for cd in self.tier_cooldowns.values_mut() {
            *cd = (*cd - dt).max(0.0);
        }

        self.decay_timer += dt;
        if self.decay_timer >= MONSTER_DECAY_INTERVAL_SECS {
            self.decay_timer = 0.0;
            for &kind in ALL_MONSTER_KINDS {
                let v = self.attitudes.entry(kind).or_insert(50);
                if *v > 50 { *v -= 1; }
                else if *v < 50 { *v += 1; }
            }
        }

        for &kind in ALL_MONSTER_KINDS {
            let current_tier = self.tier(kind);
            let last = *self.last_tiers.get(&kind).unwrap_or(&RelationTier::Neutral);
            let cooldown = *self.tier_cooldowns.get(&kind).unwrap_or(&0.0);
            if current_tier != last && cooldown <= 0.0 {
                *self.last_tiers.entry(kind).or_insert(RelationTier::Neutral) = current_tier;
                *self.tier_cooldowns.entry(kind).or_insert(0.0) = TIER_BROADCAST_COOLDOWN;
                events.push(MonsterRelationEvent::TierChanged { kind, new_tier: current_tier });
            }
        }

        events
    }

    fn adjust(&mut self, kind: EnemyKind, delta: i32) {
        let v = self.attitudes.entry(kind).or_insert(50);
        *v = (*v + delta).clamp(0, 100);
    }
}

// ─── 怪物物種測試 ────────────────────────────────────────────────────────────

#[cfg(test)]
mod monster_tests {
    use super::*;

    #[test]
    fn all_monster_kinds_start_neutral() {
        let ms = MonsterSpeciesRelations::new();
        for &kind in ALL_MONSTER_KINDS {
            assert_eq!(ms.attitude(kind), 50);
            assert_eq!(ms.tier(kind), RelationTier::Neutral);
        }
    }

    #[test]
    fn kill_monster_raises_attitude() {
        let mut ms = MonsterSpeciesRelations::new();
        ms.on_player_kills_monster(EnemyKind::ScrapDrone);
        assert_eq!(ms.attitude(EnemyKind::ScrapDrone), 50 + MONSTER_KILL_REWARD);
        // 其他種類不受影響
        assert_eq!(ms.attitude(EnemyKind::CrystalGolem), 50);
    }

    #[test]
    fn monster_kills_player_lowers_attitude() {
        let mut ms = MonsterSpeciesRelations::new();
        ms.on_monster_kills_player(EnemyKind::CrystalGolem);
        assert_eq!(ms.attitude(EnemyKind::CrystalGolem), 50 + MONSTER_PLAYER_KILL_PENALTY);
        assert_eq!(ms.attitude(EnemyKind::ScrapDrone), 50);
    }

    #[test]
    fn attitude_clamps_to_0_100() {
        let mut ms = MonsterSpeciesRelations::new();
        for _ in 0..15 { ms.on_player_kills_monster(EnemyKind::ScrapDrone); }
        assert!(ms.attitude(EnemyKind::ScrapDrone) <= 100);
        for _ in 0..15 { ms.on_monster_kills_player(EnemyKind::ScrapDrone); }
        assert!(ms.attitude(EnemyKind::ScrapDrone) >= 0);
    }

    #[test]
    fn friendly_tier_gives_small_aggro_multiplier() {
        let mut ms = MonsterSpeciesRelations::new();
        // 把某種怪拉到友善（50 + 8*3 = 74 ≥ 65）
        for _ in 0..3 { ms.on_player_kills_monster(EnemyKind::EtherWisp); }
        assert_eq!(ms.tier(EnemyKind::EtherWisp), RelationTier::Friendly);
        assert!(ms.aggro_multiplier(EnemyKind::EtherWisp) < 1.0, "友善物種 aggro 倍率應小於 1.0");
    }

    #[test]
    fn hostile_tier_gives_large_aggro_multiplier() {
        let mut ms = MonsterSpeciesRelations::new();
        // 把某種怪拉到敵視（50 - 10*3 = 20 < 25）
        for _ in 0..3 { ms.on_monster_kills_player(EnemyKind::CrystalGolem); }
        assert_eq!(ms.tier(EnemyKind::CrystalGolem), RelationTier::Hostile);
        assert!(ms.aggro_multiplier(EnemyKind::CrystalGolem) > 1.0, "敵視物種 aggro 倍率應大於 1.0");
    }

    #[test]
    fn aggro_multiplier_snapshot_covers_all_kinds() {
        let ms = MonsterSpeciesRelations::new();
        let snap = ms.aggro_multipliers_snapshot();
        assert_eq!(snap.len(), ALL_MONSTER_KINDS.len());
        for &kind in ALL_MONSTER_KINDS {
            assert!(snap.contains_key(&kind));
        }
    }

    #[test]
    fn monster_tier_event_emitted_on_change() {
        let mut ms = MonsterSpeciesRelations::new();
        // 把怪拉到友善
        for _ in 0..3 { ms.on_player_kills_monster(EnemyKind::RuneGuardian); }
        let events = ms.tick(1.0);
        assert!(events.iter().any(|e| matches!(e,
            MonsterRelationEvent::TierChanged { kind: EnemyKind::RuneGuardian, new_tier: RelationTier::Friendly }
        )));
    }

    #[test]
    fn monster_tier_event_has_cooldown() {
        let mut ms = MonsterSpeciesRelations::new();
        for _ in 0..3 { ms.on_player_kills_monster(EnemyKind::CoralCrab); }
        let e1 = ms.tick(1.0);
        assert!(e1.iter().any(|e| matches!(e, MonsterRelationEvent::TierChanged { .. })));
        let e2 = ms.tick(1.0);
        assert!(!e2.iter().any(|e| matches!(e, MonsterRelationEvent::TierChanged { .. })));
    }

    #[test]
    fn natural_decay_toward_neutral() {
        let mut ms = MonsterSpeciesRelations::new();
        for _ in 0..3 { ms.on_player_kills_monster(EnemyKind::JadeWraith); }
        let before = ms.attitude(EnemyKind::JadeWraith);
        ms.tick(MONSTER_DECAY_INTERVAL_SECS + 1.0);
        let after = ms.attitude(EnemyKind::JadeWraith);
        assert!(after < before, "態度應向 50 靠近：before={before}, after={after}");
    }

    #[test]
    fn views_returns_all_14_monster_kinds() {
        let ms = MonsterSpeciesRelations::new();
        let views = ms.views();
        assert_eq!(views.len(), 14);
        assert!(views.iter().any(|v| v.kind == "scrap_drone"));
        assert!(views.iter().any(|v| v.kind == "ether_overlord"));
    }

    #[test]
    fn aggro_multiplier_for_neutral_is_one() {
        assert_eq!(aggro_multiplier_for_tier(RelationTier::Neutral), 1.0);
    }

    #[test]
    fn all_monster_kinds_count() {
        assert_eq!(ALL_MONSTER_KINDS.len(), 14);
    }
}
