//! 主動技能系統（ROADMAP 45）。
//!
//! 五條熟練度各自在 Lv.5 解鎖一個主動技能；技能有冷卻時間，觸發後進入冷卻：
//!   - 戰吼 (Warcry)    ⚔️ warrior Lv.5：下次攻擊打中範圍內**所有**存活敵人（群攻），60 秒冷卻。
//!   - 豐饒術 (Bounty)  🌾 farmer  Lv.5：下次採集額外得 +3 個物品，120 秒冷卻。
//!   - 精密合成(Precision)🔧 artisan Lv.5：下次合成額外產出 +1 個成品，180 秒冷卻。
//!   - 風之步 (Gale)    🧭 explorer Lv.5：立即朝當前移動方向瞬移 256px，90 秒冷卻。
//!   - 議價術 (Haggle)  💰 merchant Lv.5：下次 NPC 賣出額外多得等額乙太（總收入 ×2），150 秒冷卻。

use serde::{Deserialize, Serialize};

/// 解鎖所需熟練度等級（Lv.5）。
pub const UNLOCK_LEVEL: u32 = 5;

/// 各技能冷卻時間（秒）。
pub const WARCRY_COOLDOWN:    f32 = 60.0;
pub const BOUNTY_COOLDOWN:    f32 = 120.0;
pub const PRECISION_COOLDOWN: f32 = 180.0;
pub const GALE_COOLDOWN:      f32 = 90.0;
pub const HAGGLE_COOLDOWN:    f32 = 150.0;

/// 風之步瞬移距離（像素）。
pub const GALE_DASH_PX: f32 = 256.0;

/// 豐饒術額外採集量。
pub const BOUNTY_BONUS_QTY: u32 = 3;

/// 技能種類枚舉（wire 序列化用 snake_case）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveSkillKind {
    Warcry,
    Bounty,
    Precision,
    Gale,
    Haggle,
}

impl ActiveSkillKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "warcry"    => Some(Self::Warcry),
            "bounty"    => Some(Self::Bounty),
            "precision" => Some(Self::Precision),
            "gale"      => Some(Self::Gale),
            "haggle"    => Some(Self::Haggle),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warcry    => "warcry",
            Self::Bounty    => "bounty",
            Self::Precision => "precision",
            Self::Gale      => "gale",
            Self::Haggle    => "haggle",
        }
    }

    pub fn cooldown_secs(self) -> f32 {
        match self {
            Self::Warcry    => WARCRY_COOLDOWN,
            Self::Bounty    => BOUNTY_COOLDOWN,
            Self::Precision => PRECISION_COOLDOWN,
            Self::Gale      => GALE_COOLDOWN,
            Self::Haggle    => HAGGLE_COOLDOWN,
        }
    }

    /// 判斷此技能對應的熟練度是否達解鎖等級。
    pub fn is_unlocked(self, masteries: &crate::class::Masteries) -> bool {
        let lv = match self {
            Self::Warcry    => masteries.warrior_level(),
            Self::Bounty    => masteries.farmer_level(),
            Self::Precision => masteries.artisan_level(),
            Self::Gale      => masteries.explorer_level(),
            Self::Haggle    => masteries.merchant_level(),
        };
        lv >= UNLOCK_LEVEL
    }

    /// 技能對應的職業名稱（用於錯誤訊息）。
    pub fn class_name(self) -> &'static str {
        match self {
            Self::Warcry    => "戰士",
            Self::Bounty    => "農夫",
            Self::Precision => "工匠",
            Self::Gale      => "探索者",
            Self::Haggle    => "商人",
        }
    }
}

/// 五技能的冷卻剩餘秒數（記憶體前置，重啟清空）。
#[derive(Debug, Clone, Default)]
pub struct SkillCooldowns {
    pub warcry:    f32,
    pub bounty:    f32,
    pub precision: f32,
    pub gale:      f32,
    pub haggle:    f32,
}

impl SkillCooldowns {
    pub fn get(&self, kind: ActiveSkillKind) -> f32 {
        match kind {
            ActiveSkillKind::Warcry    => self.warcry,
            ActiveSkillKind::Bounty    => self.bounty,
            ActiveSkillKind::Precision => self.precision,
            ActiveSkillKind::Gale      => self.gale,
            ActiveSkillKind::Haggle    => self.haggle,
        }
    }

    pub fn set(&mut self, kind: ActiveSkillKind, secs: f32) {
        match kind {
            ActiveSkillKind::Warcry    => self.warcry    = secs,
            ActiveSkillKind::Bounty    => self.bounty    = secs,
            ActiveSkillKind::Precision => self.precision = secs,
            ActiveSkillKind::Gale      => self.gale      = secs,
            ActiveSkillKind::Haggle    => self.haggle    = secs,
        }
    }

    /// 推進所有冷卻（每 tick 呼叫）。
    pub fn tick(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        self.warcry    = (self.warcry    - dt).max(0.0);
        self.bounty    = (self.bounty    - dt).max(0.0);
        self.precision = (self.precision - dt).max(0.0);
        self.gale      = (self.gale      - dt).max(0.0);
        self.haggle    = (self.haggle    - dt).max(0.0);
    }

    /// 轉成 wire map（前端 HUD 倒數顯示用，0 = 可用）。
    pub fn as_wire_map(&self) -> std::collections::HashMap<String, u32> {
        let mut m = std::collections::HashMap::new();
        m.insert("warcry".into(),    self.warcry.ceil()    as u32);
        m.insert("bounty".into(),    self.bounty.ceil()    as u32);
        m.insert("precision".into(), self.precision.ceil() as u32);
        m.insert("gale".into(),      self.gale.ceil()      as u32);
        m.insert("haggle".into(),    self.haggle.ceil()    as u32);
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class::{Masteries, XP_PER_LEVEL};

    fn mastery_at_level(level: u32) -> Masteries {
        Masteries {
            warrior:  XP_PER_LEVEL * level,
            farmer:   XP_PER_LEVEL * level,
            artisan:  XP_PER_LEVEL * level,
            explorer: XP_PER_LEVEL * level,
            merchant: XP_PER_LEVEL * level,
        }
    }

    #[test]
    fn skill_locked_below_lv5() {
        let m = mastery_at_level(4);
        for kind in [ActiveSkillKind::Warcry, ActiveSkillKind::Bounty,
                     ActiveSkillKind::Precision, ActiveSkillKind::Gale, ActiveSkillKind::Haggle] {
            assert!(!kind.is_unlocked(&m), "{:?} 應鎖定在 Lv.4", kind);
        }
    }

    #[test]
    fn skill_unlocked_at_lv5() {
        let m = mastery_at_level(5);
        for kind in [ActiveSkillKind::Warcry, ActiveSkillKind::Bounty,
                     ActiveSkillKind::Precision, ActiveSkillKind::Gale, ActiveSkillKind::Haggle] {
            assert!(kind.is_unlocked(&m), "{:?} 應在 Lv.5 解鎖", kind);
        }
    }

    #[test]
    fn cooldown_tick_decrements() {
        let mut cd = SkillCooldowns::default();
        cd.set(ActiveSkillKind::Warcry, 60.0);
        cd.tick(1.0);
        assert!((cd.get(ActiveSkillKind::Warcry) - 59.0).abs() < 1e-4);
    }

    #[test]
    fn cooldown_does_not_go_negative() {
        let mut cd = SkillCooldowns::default();
        cd.set(ActiveSkillKind::Gale, 5.0);
        cd.tick(100.0);
        assert_eq!(cd.get(ActiveSkillKind::Gale), 0.0);
    }

    #[test]
    fn cooldown_wire_map_has_all_five_keys() {
        let cd = SkillCooldowns::default();
        let m = cd.as_wire_map();
        for k in ["warcry", "bounty", "precision", "gale", "haggle"] {
            assert!(m.contains_key(k), "wire map 缺少 key: {k}");
        }
    }

    #[test]
    fn from_str_round_trips() {
        for (s, expected) in [
            ("warcry",    ActiveSkillKind::Warcry),
            ("bounty",    ActiveSkillKind::Bounty),
            ("precision", ActiveSkillKind::Precision),
            ("gale",      ActiveSkillKind::Gale),
            ("haggle",    ActiveSkillKind::Haggle),
        ] {
            let got = ActiveSkillKind::from_str(s).expect("應解析成功");
            assert_eq!(got, expected);
            assert_eq!(got.as_str(), s);
        }
    }

    #[test]
    fn unknown_skill_returns_none() {
        assert!(ActiveSkillKind::from_str("teleport").is_none());
    }
}
