//! 玩家異狀（status effect）的純邏輯層——戰鬥史上第一個「狀態效果」維度（ROADMAP 469）。
//!
//! 既有戰鬥只有「傷害交換 + 等級成長」：護甲減傷、格擋、翻滾、蓄力都圍著「這一下扣多少血」打轉，
//! `element_affinity` 是傷害倍率、`meal_buff` 是料理給的**正面**養成增益——全都沒有「持續性的負面狀態」。
//! 本層補上第一個 debuff：**中毒（Poison）**。乙太迷霧 / 孢子系敵人擊中玩家時注入毒素，之後即使
//! 走出敵人攻擊範圍，毒仍在體內持續流失生命，直到自然代謝或**回到城鎮加速解毒**。
//!
//! 與既有元素的定位區隔（守「不重複骨架」鐵律）：
//!   - 強化怪 / 兇名精英（`enemy_field` notorious、`monster_colony` Alpha）＝**敵人本身更強**；
//!     本層是**施加在玩家身上的狀態**，是兩個正交維度。
//!   - `meal_buff` ＝料理給的**正面**限時增益；中毒是戰鬥施加的**負面**限時流失，骨架相反。
//!   - 護甲 / 格擋 / 翻滾化解的是「當下那一擊」；中毒**穿透減傷鏈**（體內毒、護甲擋不住），
//!     逼玩家用「撤離 / 回鎮」這種空間決策來解，而非堆防禦數值——讓戰鬥多一層策略。
//!
//! 這層是純資料 + 純函式，無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試。延續 `combat.rs` /
//! `vitals.rs` 的慣例：純邏輯獨立可測，由 `game.rs` 接線餵呼叫。毒傷透過既有 `Vitals::take_damage`
//! 結算（穿甲＝接線層不套護甲）。中毒狀態**記憶體前置、不持久化、零 migration、重啟清零**
//!（比照 `guard_shield` / `dodging`：暫態戰鬥狀態本就不該存檔，登出再登入仍中毒並不合理）。

use serde::{Deserialize, Serialize};

use crate::combat::EnemyKind;

/// 毒傷的結算間隔（秒）：每滿一秒造成一次毒傷，讀起來像規律的脈動流失。
pub const POISON_DMG_INTERVAL_SECS: f32 = 1.0;

/// 每次毒傷的點數。刻意只有 1——療癒向世界，中毒是「該撤了」的提醒而非處刑；
/// 穿甲但極溫和（8 秒滿毒最多累積 8 點，遠低於多數玩家血量），給張力不勸退。
pub const POISON_DMG_PER_TICK: u32 = 1;

/// 中毒剩餘秒數的上限：持續待在毒區會疊到這裡封頂，避免無限累積；走開後自然流失。
pub const POISON_MAX_SECS: f32 = 8.0;

/// 被毒襲一次注入的中毒秒數。反覆挨毒會延長（封頂 `POISON_MAX_SECS`）。
pub const POISON_HIT_SECS: f32 = 4.0;

/// 城鎮內的解毒加速倍率：站在主城安全圈內，中毒以 4 倍速率代謝消解
///（＝「打不過先回鎮」這條最自然的求生路線真的有效，呼應療癒向引導）。
pub const POISON_TOWN_CLEANSE_MULT: f32 = 4.0;

/// 主城解毒安全圈半徑（像素）。落在此圈內＝享受加速解毒。
/// 取一個涵蓋城鎮核心的範圍，讓玩家「跑進城」就明顯感覺毒退得快。
pub const TOWN_CLEANSE_RADIUS: f32 = 12.0 * 32.0; // 12 格

/// 這種敵人擊中玩家時是否施毒、施加多少秒。
///
/// 只有「乙太迷霧 / 孢子 / 幽靈」系的敵人帶毒——牠們的攻擊本就是飄散的霧氣與孢子，
/// 中毒在主題上自洽；機械 / 礦石 / 甲殼系（廢鐵無人機、晶石傀儡、珊瑚蟹等）不施毒。
/// 回 `None` ＝該敵人不帶毒。純查表、確定性、極好測。
pub fn poison_on_hit(kind: EnemyKind) -> Option<f32> {
    match kind {
        // 迷途乙太靈：野化乙太散出的霧氣帶毒（新手區也會碰到，故毒極溫和、用來教學「中毒」這件事）。
        EnemyKind::EtherWisp => Some(POISON_HIT_SECS),
        // 蕈菇潛行者：孢子凝聚而成，揮擊噴出致幻孢子。
        EnemyKind::MushroomStalker => Some(POISON_HIT_SECS),
        // 虛空幽靈：宇宙深淵的黑暗能量侵蝕。
        EnemyKind::VoidPhantom => Some(POISON_HIT_SECS),
        // 霧醚幻靈：乙太迷霧深處的幻靈，整身都是侵蝕性迷霧。
        EnemyKind::AetherSpecter => Some(POISON_HIT_SECS),
        _ => None,
    }
}

/// 主城安全圈內＝加速解毒。純函式（用 `expedition` 既有的主城中心常數），可測。
pub fn near_town_cleanse(px: f32, py: f32) -> bool {
    if !px.is_finite() || !py.is_finite() {
        return false;
    }
    let dx = px - crate::expedition::HOME_TOWN_X;
    let dy = py - crate::expedition::HOME_TOWN_Y;
    dx * dx + dy * dy <= TOWN_CLEANSE_RADIUS * TOWN_CLEANSE_RADIUS
}

/// 一名玩家身上的中毒狀態。`remaining` ＝中毒還會持續幾秒；`accum` ＝距下一次毒傷脈動的累積秒數。
///
/// 預設（`new` / `Default`）＝沒中毒（`remaining == 0`）。記憶體前置、不持久化、重啟清零。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Poison {
    /// 中毒剩餘秒數（0＝沒中毒）。
    remaining: f32,
    /// 距下一次毒傷脈動的累積秒數（[0, POISON_DMG_INTERVAL_SECS)）。
    accum: f32,
}

impl Poison {
    /// 沒中毒的初始狀態。
    pub fn new() -> Self {
        Self { remaining: 0.0, accum: 0.0 }
    }

    /// 是否正在中毒（前端據此畫毒泡、HUD 提示）。
    pub fn is_active(&self) -> bool {
        self.remaining > 0.0
    }

    /// 剩餘中毒秒數（供測試 / 除錯）。
    pub fn remaining(&self) -> f32 {
        self.remaining
    }

    /// 注入 `secs` 秒中毒：在現值上延長、封頂 `POISON_MAX_SECS`（反覆挨毒會疊到上限）。
    /// 非正 / 非有限 `secs` 一律忽略（保守、不引入 NaN）。
    pub fn apply(&mut self, secs: f32) {
        if !secs.is_finite() || secs <= 0.0 {
            return;
        }
        self.remaining = (self.remaining + secs).min(POISON_MAX_SECS);
    }

    /// 推進 `dt` 秒，回傳這段時間該造成的毒傷（穿甲，由接線層直接餵 `take_damage`）。
    ///
    /// - 沒中毒（`remaining <= 0`）或非正 `dt`：no-op，回 0（比照 `Vitals::tick` 擋非正 dt）。
    /// - 毒傷脈動用「中毒仍有效的真實時間」累積（`active_dt`），故城鎮加速解毒只讓毒**退得快**、
    ///   不會反而讓毒傷算多——加速作用在 `remaining` 的衰減、不作用在傷害節奏。
    /// - 衰減：城鎮內以 `POISON_TOWN_CLEANSE_MULT` 倍速消解；`remaining` 歸零即清空 `accum`。
    pub fn tick(&mut self, dt: f32, in_town: bool) -> u32 {
        if !dt.is_finite() || dt <= 0.0 || self.remaining <= 0.0 {
            return 0;
        }
        // 毒傷只在「中毒仍有效」的那段時間內累積（用未加速的真實剩餘時間夾）。
        let active_dt = dt.min(self.remaining);
        self.accum += active_dt;
        let mut dmg = 0u32;
        while self.accum >= POISON_DMG_INTERVAL_SECS {
            self.accum -= POISON_DMG_INTERVAL_SECS;
            dmg += POISON_DMG_PER_TICK;
        }
        // 衰減剩餘中毒時間（城鎮內加速）。
        let decay = dt * if in_town { POISON_TOWN_CLEANSE_MULT } else { 1.0 };
        self.remaining = (self.remaining - decay).max(0.0);
        if self.remaining <= 0.0 {
            self.accum = 0.0;
        }
        dmg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mist_and_spore_enemies_poison() {
        // 帶毒：乙太靈、蕈菇、虛空、霧醚。
        assert_eq!(poison_on_hit(EnemyKind::EtherWisp), Some(POISON_HIT_SECS));
        assert_eq!(poison_on_hit(EnemyKind::MushroomStalker), Some(POISON_HIT_SECS));
        assert_eq!(poison_on_hit(EnemyKind::VoidPhantom), Some(POISON_HIT_SECS));
        assert_eq!(poison_on_hit(EnemyKind::AetherSpecter), Some(POISON_HIT_SECS));
        // 不帶毒：機械 / 礦石 / 甲殼 / 一般守衛。
        assert_eq!(poison_on_hit(EnemyKind::ScrapDrone), None);
        assert_eq!(poison_on_hit(EnemyKind::CrystalGolem), None);
        assert_eq!(poison_on_hit(EnemyKind::CoralCrab), None);
        assert_eq!(poison_on_hit(EnemyKind::EtherOverlord), None);
    }

    #[test]
    fn fresh_poison_is_inactive() {
        let p = Poison::new();
        assert!(!p.is_active());
        assert_eq!(p.remaining(), 0.0);
        assert_eq!(Poison::default(), p);
    }

    #[test]
    fn apply_extends_and_caps() {
        let mut p = Poison::new();
        p.apply(POISON_HIT_SECS);
        assert!(p.is_active());
        assert_eq!(p.remaining(), POISON_HIT_SECS);
        // 反覆挨毒延長、但封頂 MAX。
        p.apply(POISON_HIT_SECS);
        assert_eq!(p.remaining(), POISON_MAX_SECS); // 4+4=8=cap
        p.apply(POISON_HIT_SECS);
        assert_eq!(p.remaining(), POISON_MAX_SECS); // 仍封頂
    }

    #[test]
    fn apply_ignores_bad_input() {
        let mut p = Poison::new();
        p.apply(0.0);
        p.apply(-1.0);
        p.apply(f32::NAN);
        p.apply(f32::INFINITY);
        assert!(!p.is_active());
    }

    #[test]
    fn tick_no_poison_is_noop() {
        let mut p = Poison::new();
        assert_eq!(p.tick(1.0, false), 0);
    }

    #[test]
    fn tick_non_positive_dt_is_noop() {
        let mut p = Poison::new();
        p.apply(POISON_HIT_SECS);
        assert_eq!(p.tick(0.0, false), 0);
        assert_eq!(p.tick(-1.0, false), 0);
        assert_eq!(p.tick(f32::NAN, false), 0);
        assert_eq!(p.remaining(), POISON_HIT_SECS); // 不被壞 dt 影響
    }

    #[test]
    fn tick_pulses_one_damage_per_second() {
        let mut p = Poison::new();
        p.apply(POISON_HIT_SECS); // 4 秒
        // 每滿一秒一點毒傷。
        assert_eq!(p.tick(0.5, false), 0); // 累積 0.5，未滿一秒
        assert_eq!(p.tick(0.5, false), POISON_DMG_PER_TICK); // 累積到 1.0，造成 1 點
        assert_eq!(p.tick(1.0, false), POISON_DMG_PER_TICK);
    }

    #[test]
    fn tick_decays_to_zero_in_wild() {
        let mut p = Poison::new();
        p.apply(POISON_HIT_SECS); // 4 秒
        let mut total = 0u32;
        // 野外：每秒衰減 1 秒，4 秒後解毒。
        for _ in 0..4 {
            total += p.tick(1.0, false);
        }
        assert!(!p.is_active());
        assert_eq!(p.remaining(), 0.0);
        // 4 秒中毒 → 4 次脈動 → 4 點毒傷。
        assert_eq!(total, 4 * POISON_DMG_PER_TICK);
    }

    #[test]
    fn town_cleanse_decays_faster() {
        let mut wild = Poison::new();
        wild.apply(POISON_MAX_SECS); // 8 秒
        let mut town = Poison::new();
        town.apply(POISON_MAX_SECS); // 8 秒
        // 同樣推進 2 秒：城鎮內衰減 4 倍。
        wild.tick(2.0, false);
        town.tick(2.0, true);
        assert_eq!(wild.remaining(), POISON_MAX_SECS - 2.0); // 野外 8-2=6
        assert_eq!(town.remaining(), POISON_MAX_SECS - 2.0 * POISON_TOWN_CLEANSE_MULT); // 城鎮 8-8=0
        assert!(!town.is_active());
    }

    #[test]
    fn town_cleanse_does_not_inflate_damage() {
        // 城鎮加速只讓毒退得快，毒傷節奏不被放大：2 秒內最多 2 點（用真實時間算脈動）。
        let mut town = Poison::new();
        town.apply(POISON_MAX_SECS);
        let dmg = town.tick(2.0, true);
        assert!(dmg <= 2, "城鎮 2 秒毒傷不應超過 2 點，實得 {dmg}");
    }

    #[test]
    fn near_town_cleanse_geometry() {
        // 主城中心圈內。
        assert!(near_town_cleanse(
            crate::expedition::HOME_TOWN_X,
            crate::expedition::HOME_TOWN_Y
        ));
        // 遠在野外。
        assert!(!near_town_cleanse(
            crate::expedition::HOME_TOWN_X + TOWN_CLEANSE_RADIUS + 100.0,
            crate::expedition::HOME_TOWN_Y
        ));
        // 壞座標保守回 false。
        assert!(!near_town_cleanse(f32::NAN, 0.0));
    }
}
