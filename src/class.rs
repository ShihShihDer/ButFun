//! 職業兼修熟練度系統（ROADMAP 38）。
//!
//! 五條熟練度各自獨立累積，做什麼練什麼（取代舊版「單選一職業」）：
//!   - 戰士 (Warrior)   ⚔️：殺怪得 XP，等級 ≥1 時攻擊力 +5、最大 HP +10。
//!   - 農夫 (Farmer)    🌾：農地收割得 XP，等級 ≥1 時農地 +3 乙太/次、NPC 收購 +25%。
//!   - 工匠 (Artisan)   🔧：挖礦/合成得 XP，等級 ≥1 時合成每項素材 -1（最少 1）。
//!   - 探索者 (Explorer) 🧭：星際旅行得 XP，等級 ≥1 時旅行費 -10 乙太（最低 10）。
//!   - 商人 (Merchant)  💰：NPC 買賣得 XP，等級 ≥1 時 NPC 所有收購 +50%。
//!
//! 每條加成沿用舊職業數值，但五條可同時生效（兼修）。
//! 等級 = XP / XP_PER_LEVEL（= 10），無上限；頭銜取最高等級那條。

use serde::{Deserialize, Serialize};

/// 每 10 點 XP 升一級。
pub const XP_PER_LEVEL: u32 = 10;

/// 職業頭銜枚舉（用於 HUD pill 顯示與 wire 傳輸）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobClass {
    Warrior,
    Farmer,
    Artisan,
    Explorer,
    Merchant,
}

impl JobClass {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "warrior"  => Some(Self::Warrior),
            "farmer"   => Some(Self::Farmer),
            "artisan"  => Some(Self::Artisan),
            "explorer" => Some(Self::Explorer),
            "merchant" => Some(Self::Merchant),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warrior  => "warrior",
            Self::Farmer   => "farmer",
            Self::Artisan  => "artisan",
            Self::Explorer => "explorer",
            Self::Merchant => "merchant",
        }
    }
}

/// 五條熟練度的 XP 積累狀態。每條各自獨立成長，互不影響。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Masteries {
    pub warrior:  u32,
    pub farmer:   u32,
    pub artisan:  u32,
    pub explorer: u32,
    pub merchant: u32,
}

impl Masteries {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn warrior_level(&self)  -> u32 { self.warrior  / XP_PER_LEVEL }
    pub fn farmer_level(&self)   -> u32 { self.farmer   / XP_PER_LEVEL }
    pub fn artisan_level(&self)  -> u32 { self.artisan  / XP_PER_LEVEL }
    pub fn explorer_level(&self) -> u32 { self.explorer / XP_PER_LEVEL }
    pub fn merchant_level(&self) -> u32 { self.merchant / XP_PER_LEVEL }

    /// 增加戰士 XP；回傳 `true` 表示觸發了等級提升（供呼叫方決定是否更新 HP 上限）。
    pub fn gain_warrior(&mut self, xp: u32) -> bool {
        let before = self.warrior_level();
        self.warrior = self.warrior.saturating_add(xp);
        self.warrior_level() > before
    }

    pub fn gain_farmer(&mut self, xp: u32) -> bool {
        let before = self.farmer_level();
        self.farmer = self.farmer.saturating_add(xp);
        self.farmer_level() > before
    }

    pub fn gain_artisan(&mut self, xp: u32) -> bool {
        let before = self.artisan_level();
        self.artisan = self.artisan.saturating_add(xp);
        self.artisan_level() > before
    }

    pub fn gain_explorer(&mut self, xp: u32) -> bool {
        let before = self.explorer_level();
        self.explorer = self.explorer.saturating_add(xp);
        self.explorer_level() > before
    }

    pub fn gain_merchant(&mut self, xp: u32) -> bool {
        let before = self.merchant_level();
        self.merchant = self.merchant.saturating_add(xp);
        self.merchant_level() > before
    }

    /// 目前頭銜職業：等級最高的那條；同高優先順序 warrior > farmer > artisan > explorer > merchant。
    /// 全部為 0 時回 `None`（尚未解鎖任何熟練度）。
    pub fn title_class(&self) -> Option<JobClass> {
        let levels = [
            (self.warrior_level(),  JobClass::Warrior),
            (self.farmer_level(),   JobClass::Farmer),
            (self.artisan_level(),  JobClass::Artisan),
            (self.explorer_level(), JobClass::Explorer),
            (self.merchant_level(), JobClass::Merchant),
        ];
        // 找最大等級，同高時取第一個（warrior 優先）。
        let max_lv = levels.iter().map(|(lv, _)| *lv).max().unwrap_or(0);
        if max_lv == 0 {
            return None;
        }
        levels.iter().find(|(lv, _)| *lv == max_lv).map(|(_, cls)| *cls)
    }
}

// ─── 加成函式（取代舊版 Option<JobClass> 簽名）────────────────────────────────

/// 戰士熟練度：攻擊力加成（等級 ≥1 時 +5）。
pub fn combat_bonus(m: &Masteries) -> u32 {
    if m.warrior_level() >= 1 { 5 } else { 0 }
}

/// 戰士熟練度：最大 HP 加成（等級 ≥1 時 +10）。
pub fn hp_bonus(m: &Masteries) -> u32 {
    if m.warrior_level() >= 1 { 10 } else { 0 }
}

/// 農夫熟練度：農地收割額外乙太（等級 ≥1 時 +3）。
pub fn harvest_ether_bonus(m: &Masteries) -> u32 {
    if m.farmer_level() >= 1 { 3 } else { 0 }
}

/// 農夫 / 商人熟練度：NPC 收購乙太加成百分比。
/// 商人等級 ≥1 → 50%，農夫等級 ≥1 → 25%，取較大值（商人涵蓋農夫，不疊加）。
pub fn npc_sell_bonus_pct(m: &Masteries) -> u32 {
    if m.merchant_level() >= 1 { 50 }
    else if m.farmer_level() >= 1 { 25 }
    else { 0 }
}

/// 工匠熟練度：合成時每項素材的減量（等級 ≥1 時 -1，最少 1）。
pub fn crafting_reduction(m: &Masteries) -> u32 {
    if m.artisan_level() >= 1 { 1 } else { 0 }
}

/// 探索者熟練度：星際旅行費減少量（等級 ≥1 時 -10，最終費用不低於 10）。
pub fn travel_cost_reduction(m: &Masteries) -> u32 {
    if m.explorer_level() >= 1 { 10 } else { 0 }
}

/// 套用 NPC 收購加成後的乙太收益。
pub fn apply_npc_bonus(m: &Masteries, base_earned: u32) -> u32 {
    let pct = npc_sell_bonus_pct(m);
    if pct == 0 {
        return base_earned;
    }
    base_earned.saturating_add(base_earned.saturating_mul(pct) / 100)
}

/// 套用旅行費折扣後的實際費用（最低 10）。
pub fn apply_travel_discount(m: &Masteries, base_cost: u32) -> u32 {
    let reduction = travel_cost_reduction(m);
    base_cost.saturating_sub(reduction).max(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warrior_lv1() -> Masteries {
        Masteries { warrior: XP_PER_LEVEL, ..Default::default() }
    }

    fn farmer_lv1() -> Masteries {
        Masteries { farmer: XP_PER_LEVEL, ..Default::default() }
    }

    fn artisan_lv1() -> Masteries {
        Masteries { artisan: XP_PER_LEVEL, ..Default::default() }
    }

    fn explorer_lv1() -> Masteries {
        Masteries { explorer: XP_PER_LEVEL, ..Default::default() }
    }

    fn merchant_lv1() -> Masteries {
        Masteries { merchant: XP_PER_LEVEL, ..Default::default() }
    }

    #[test]
    fn level_zero_before_threshold() {
        let m = Masteries::default();
        assert_eq!(m.warrior_level(), 0);
        assert_eq!(m.farmer_level(), 0);
        assert_eq!(m.artisan_level(), 0);
        assert_eq!(m.explorer_level(), 0);
        assert_eq!(m.merchant_level(), 0);
    }

    #[test]
    fn level_one_at_threshold() {
        let m = warrior_lv1();
        assert_eq!(m.warrior_level(), 1);
        // 其他條仍 0
        assert_eq!(m.farmer_level(), 0);
    }

    #[test]
    fn gain_triggers_level_up() {
        let mut m = Masteries::default();
        // 前 9 點不升級
        for _ in 0..9 {
            let leveled = m.gain_warrior(1);
            assert!(!leveled);
        }
        // 第 10 點觸發升級
        assert!(m.gain_warrior(1));
        assert_eq!(m.warrior_level(), 1);
    }

    #[test]
    fn gain_no_level_up_returns_false() {
        let mut m = warrior_lv1();
        // 已升 1 級，繼續加 9 點不再升
        for _ in 0..9 {
            assert!(!m.gain_warrior(1));
        }
        // 第 10 點→ 升到 2 級
        assert!(m.gain_warrior(1));
        assert_eq!(m.warrior_level(), 2);
    }

    #[test]
    fn combat_bonus_only_with_warrior_lv1() {
        assert_eq!(combat_bonus(&Masteries::default()), 0);
        assert_eq!(combat_bonus(&warrior_lv1()), 5);
        // 其他熟練度不給攻擊加成
        assert_eq!(combat_bonus(&farmer_lv1()), 0);
    }

    #[test]
    fn hp_bonus_only_with_warrior_lv1() {
        assert_eq!(hp_bonus(&Masteries::default()), 0);
        assert_eq!(hp_bonus(&warrior_lv1()), 10);
    }

    #[test]
    fn harvest_bonus_with_farmer_lv1() {
        assert_eq!(harvest_ether_bonus(&Masteries::default()), 0);
        assert_eq!(harvest_ether_bonus(&farmer_lv1()), 3);
        assert_eq!(harvest_ether_bonus(&warrior_lv1()), 0);
    }

    #[test]
    fn npc_bonus_merchant_beats_farmer() {
        assert_eq!(npc_sell_bonus_pct(&merchant_lv1()), 50);
        assert_eq!(npc_sell_bonus_pct(&farmer_lv1()), 25);
        assert_eq!(npc_sell_bonus_pct(&Masteries::default()), 0);
    }

    #[test]
    fn npc_bonus_merchant_overrides_farmer_when_both_active() {
        // 商人 + 農夫同時 lv1 → 取商人的 50%（較大值）。
        let m = Masteries {
            warrior: 0,
            farmer: XP_PER_LEVEL,
            artisan: 0,
            explorer: 0,
            merchant: XP_PER_LEVEL,
        };
        assert_eq!(npc_sell_bonus_pct(&m), 50);
    }

    #[test]
    fn apply_npc_bonus_correct() {
        // 商人 +50%：10 → 15。
        assert_eq!(apply_npc_bonus(&merchant_lv1(), 10), 15);
        // 農夫 +25%：8 → 10（整數截斷）。
        assert_eq!(apply_npc_bonus(&farmer_lv1(), 8), 10);
        // 無熟練度：不變。
        assert_eq!(apply_npc_bonus(&Masteries::default(), 10), 10);
    }

    #[test]
    fn artisan_crafting_reduction() {
        assert_eq!(crafting_reduction(&Masteries::default()), 0);
        assert_eq!(crafting_reduction(&artisan_lv1()), 1);
        assert_eq!(crafting_reduction(&warrior_lv1()), 0);
    }

    #[test]
    fn explorer_travel_discount() {
        // 30 乙太旅費 → 20（折扣 10）。
        assert_eq!(apply_travel_discount(&explorer_lv1(), 30), 20);
        // 非探索者無折扣。
        assert_eq!(apply_travel_discount(&Masteries::default(), 30), 30);
        // 費用最低 10。
        assert_eq!(apply_travel_discount(&explorer_lv1(), 5), 10);
    }

    #[test]
    fn title_class_none_when_all_zero() {
        assert_eq!(Masteries::default().title_class(), None);
    }

    #[test]
    fn title_class_returns_highest() {
        let m = Masteries { warrior: 5, farmer: XP_PER_LEVEL, ..Default::default() };
        // warrior=0 級（5 XP < 10），farmer=1 級 → 頭銜是農夫。
        assert_eq!(m.title_class(), Some(JobClass::Farmer));
    }

    #[test]
    fn title_class_tiebreak_prefers_warrior() {
        // 兩條都 1 級 → warrior 優先。
        let m = Masteries { warrior: XP_PER_LEVEL, farmer: XP_PER_LEVEL, ..Default::default() };
        assert_eq!(m.title_class(), Some(JobClass::Warrior));
    }

    #[test]
    fn multiple_masteries_independent() {
        // 五條同時生效：攻擊、收割、合成、旅行、NPC 加成都有。
        let m = Masteries {
            warrior:  XP_PER_LEVEL,
            farmer:   XP_PER_LEVEL,
            artisan:  XP_PER_LEVEL,
            explorer: XP_PER_LEVEL,
            merchant: XP_PER_LEVEL,
        };
        assert_eq!(combat_bonus(&m), 5);
        assert_eq!(harvest_ether_bonus(&m), 3);
        assert_eq!(crafting_reduction(&m), 1);
        assert_eq!(travel_cost_reduction(&m), 10);
        assert_eq!(npc_sell_bonus_pct(&m), 50); // merchant 蓋過 farmer
    }

    #[test]
    fn jobclass_round_trips() {
        for cls in [
            JobClass::Warrior, JobClass::Farmer, JobClass::Artisan,
            JobClass::Explorer, JobClass::Merchant,
        ] {
            assert_eq!(JobClass::from_str(cls.as_str()), Some(cls));
        }
        assert_eq!(JobClass::from_str("unknown"), None);
    }
}
