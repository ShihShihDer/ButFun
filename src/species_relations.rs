//! ROADMAP 144：人類↔物種湧現關係——跨物種好惡從互動長出。
//!
//! 每個野生動物物種對「人類」有一個態度值（0-100，50=中立）。
//! 玩家行為影響整個物種的態度：
//!   - 攻擊/殺死獵物物種 → 該物種敵意+（-12）
//!   - 殺死掠食者        → 被捕獵的獵物物種好感+（+10）
//!   - 餵食野生動物      → 該物種好感+（+15）
//! 態度值每 2 分鐘緩慢向 50 靠近 1 點（情緒有持續性但不永久）。
//!
//! 態度層級與行為影響：
//!   - 友善（≥65）：獵物不逃跑；偶爾送給玩家禮物（生態資源）。
//!   - 中立（45-64）：正常行為。
//!   - 警覺（25-44）：逃跑半徑增大。
//!   - 敵視（<25）：獵物主動攻擊玩家；掠食者更積極。
//!
//! 零 migration，記憶體模式，重啟清零（對應生態系「換季重生」哲學）。

use std::collections::HashMap;
use crate::wildlife::WildlifeKind;

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
