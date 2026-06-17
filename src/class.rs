//! 職業兼修熟練度系統（ROADMAP 38，階梯榮銜 ROADMAP 351）。
//!
//! 五條熟練度各自獨立累積，做什麼練什麼（取代舊版「單選一職業」）：
//!   - 戰士 (Warrior)   ⚔️：殺怪得 XP，攻擊力／最大 HP 加成。
//!   - 農夫 (Farmer)    🌾：農地收割得 XP，農地額外乙太／NPC 收購加成。
//!   - 工匠 (Artisan)   🔧：挖礦/合成得 XP，合成每項素材減量（最少 1）。
//!   - 探索者 (Explorer) 🧭：星際旅行得 XP，旅行費折扣（最低 10）。
//!   - 商人 (Merchant)  💰：NPC 買賣得 XP，NPC 所有收購加成。
//!
//! 每條加成沿用舊職業數值，但五條可同時生效（兼修）。
//! 等級 = XP / XP_PER_LEVEL（= 10），無上限；頭銜取最高等級那條。
//!
//! ── 階梯榮銜（ROADMAP 351）─────────────────────────────────────────────────
//! 過去每條加成只看「等級 ≥1 與否」是個布林開關：練到一級就拿滿，往後升級再多
//! 也不再強化，導致一身熟練度練到後面**毫無回報**。本系統把每條加成改成隨「階級」
//! 階梯成長——等級越高、階級越高、加成越強：
//!   未入門(Lv0) → 學徒(Lv1–4) → 匠人(Lv5–9) → 師匠(Lv10–19) → 宗師(Lv20+)
//! **學徒階的數值刻意維持與舊版相同**（向後相容、低階玩家零變動，舊測試全綠），
//! 真正吃到苦練的回報是 Lv5／Lv10／Lv20 的階級躍升（匠人／師匠／宗師）。

use serde::{Deserialize, Serialize};

/// 每 10 點 XP 升一級。
pub const XP_PER_LEVEL: u32 = 10;

/// 熟練度階級（ROADMAP 351）：由等級推導，對應一條加成階梯的檔位（tier 0~4）。
/// 面向玩家字串集中在 `label()`（繁中）／`wire()`（i18n 鍵）兩處可替換。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MasteryRank {
    /// 未入門（Lv0，加成為 0）。
    Novice,
    /// 學徒（Lv1–4，加成 = 舊版數值，向後相容）。
    Apprentice,
    /// 匠人（Lv5–9）。
    Adept,
    /// 師匠（Lv10–19）。
    Master,
    /// 宗師（Lv20+）。
    Grandmaster,
}

impl MasteryRank {
    /// 由等級推導階級。門檻：1／5／10／20。
    pub fn from_level(level: u32) -> Self {
        match level {
            0 => Self::Novice,
            1..=4 => Self::Apprentice,
            5..=9 => Self::Adept,
            10..=19 => Self::Master,
            _ => Self::Grandmaster,
        }
    }

    /// 加成階梯檔位（0=未入門 … 4=宗師），用來索引各條加成表。
    pub fn tier(self) -> usize {
        match self {
            Self::Novice => 0,
            Self::Apprentice => 1,
            Self::Adept => 2,
            Self::Master => 3,
            Self::Grandmaster => 4,
        }
    }

    /// 面向玩家的繁中階級名（i18n 佔位：集中替換點）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Novice => "未入門",
            Self::Apprentice => "學徒",
            Self::Adept => "匠人",
            Self::Master => "師匠",
            Self::Grandmaster => "宗師",
        }
    }

    /// wire / i18n 鍵（snake_case，與 serde 表示一致）。
    pub fn wire(self) -> &'static str {
        match self {
            Self::Novice => "novice",
            Self::Apprentice => "apprentice",
            Self::Adept => "adept",
            Self::Master => "master",
            Self::Grandmaster => "grandmaster",
        }
    }

    /// 此階是否屬「高階」（師匠以上）——只有跨到高階才值得在世界頻道同慶，避免洗頻。
    pub fn is_high(self) -> bool {
        self >= Self::Master
    }
}

// ─── 各條加成的階梯表（index = MasteryRank::tier()，0=未入門 … 4=宗師）─────────
// 鐵律：tier 1（學徒）= 舊版「等級 ≥1」的數值，向後相容、低階玩家零變動。
// 真正的成長回報在 tier 2/3/4（匠人/師匠/宗師），需苦練到 Lv5/10/20 才吃得到。

/// 戰士：攻擊力加成。
const WARRIOR_ATK: [u32; 5] = [0, 5, 8, 12, 18];
/// 戰士：最大 HP 加成。
const WARRIOR_HP: [u32; 5] = [0, 10, 18, 28, 42];
/// 農夫：農地收割額外乙太。
const FARMER_HARVEST: [u32; 5] = [0, 3, 4, 6, 9];
/// 工匠：合成每項素材減量（最少 1，由呼叫端夾住）。
const ARTISAN_REDUCE: [u32; 5] = [0, 1, 2, 3, 4];
/// 探索者：星際旅行費減少量（最終費用不低於 10，由呼叫端夾住）。
const EXPLORER_TRAVEL: [u32; 5] = [0, 10, 18, 28, 40];
/// 農夫：NPC 收購加成百分比階梯。
const FARMER_SELL_PCT: [u32; 5] = [0, 25, 32, 40, 50];
/// 商人：NPC 收購加成百分比階梯（涵蓋並高於農夫）。
const MERCHANT_SELL_PCT: [u32; 5] = [0, 50, 62, 78, 100];

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

    /// 五職固定順序，與 `Masteries::tier_snapshot()` 的索引對齊（晉階偵測用）。
    pub const ALL: [JobClass; 5] = [
        Self::Warrior, Self::Farmer, Self::Artisan, Self::Explorer, Self::Merchant,
    ];

    /// 面向玩家的繁中職業名（i18n 佔位：集中替換點）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Warrior  => "戰士",
            Self::Farmer   => "農夫",
            Self::Artisan  => "工匠",
            Self::Explorer => "探索者",
            Self::Merchant => "商人",
        }
    }

    /// 職業圖示（用於世界頻道晉階同慶與 HUD）。
    pub fn emoji(self) -> &'static str {
        match self {
            Self::Warrior  => "⚔️",
            Self::Farmer   => "🌾",
            Self::Artisan  => "🔧",
            Self::Explorer => "🧭",
            Self::Merchant => "💰",
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

    // ── 階級（ROADMAP 351）：由等級推導，驅動加成階梯與晉階同慶 ──────────────
    pub fn warrior_rank(&self)  -> MasteryRank { MasteryRank::from_level(self.warrior_level()) }
    pub fn farmer_rank(&self)   -> MasteryRank { MasteryRank::from_level(self.farmer_level()) }
    pub fn artisan_rank(&self)  -> MasteryRank { MasteryRank::from_level(self.artisan_level()) }
    pub fn explorer_rank(&self) -> MasteryRank { MasteryRank::from_level(self.explorer_level()) }
    pub fn merchant_rank(&self) -> MasteryRank { MasteryRank::from_level(self.merchant_level()) }

    /// 指定職業的目前階級。
    pub fn rank(&self, class: JobClass) -> MasteryRank {
        match class {
            JobClass::Warrior  => self.warrior_rank(),
            JobClass::Farmer   => self.farmer_rank(),
            JobClass::Artisan  => self.artisan_rank(),
            JobClass::Explorer => self.explorer_rank(),
            JobClass::Merchant => self.merchant_rank(),
        }
    }

    /// 五條當前階級的 tier 快照（順序 warrior／farmer／artisan／explorer／merchant），
    /// 給 `game.rs` 逐幀比對「是否剛跨階」用（晉階偵測，零持久化）。
    pub fn tier_snapshot(&self) -> [u8; 5] {
        [
            self.warrior_rank().tier()  as u8,
            self.farmer_rank().tier()   as u8,
            self.artisan_rank().tier()  as u8,
            self.explorer_rank().tier() as u8,
            self.merchant_rank().tier() as u8,
        ]
    }

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

// ─── 加成函式（ROADMAP 351：依階級階梯查表，學徒檔 = 舊版數值）────────────────

/// 戰士熟練度：攻擊力加成（學徒 +5／匠人 +8／師匠 +12／宗師 +18）。
pub fn combat_bonus(m: &Masteries) -> u32 {
    WARRIOR_ATK[m.warrior_rank().tier()]
}

/// 戰士熟練度：最大 HP 加成（學徒 +10／匠人 +18／師匠 +28／宗師 +42）。
pub fn hp_bonus(m: &Masteries) -> u32 {
    WARRIOR_HP[m.warrior_rank().tier()]
}

/// 農夫熟練度：農地收割額外乙太（學徒 +3／匠人 +4／師匠 +6／宗師 +9）。
pub fn harvest_ether_bonus(m: &Masteries) -> u32 {
    FARMER_HARVEST[m.farmer_rank().tier()]
}

/// 農夫 / 商人熟練度：NPC 收購乙太加成百分比。
/// 兩條各自依階級查表，取較大值（商人涵蓋農夫，不疊加）：
/// 農夫 25→32→40→50；商人 50→62→78→100。
pub fn npc_sell_bonus_pct(m: &Masteries) -> u32 {
    let farmer = FARMER_SELL_PCT[m.farmer_rank().tier()];
    let merchant = MERCHANT_SELL_PCT[m.merchant_rank().tier()];
    farmer.max(merchant)
}

/// 工匠熟練度：合成時每項素材的減量（學徒 -1／匠人 -2／師匠 -3／宗師 -4，最少留 1，由呼叫端夾住）。
pub fn crafting_reduction(m: &Masteries) -> u32 {
    ARTISAN_REDUCE[m.artisan_rank().tier()]
}

/// 探索者熟練度：星際旅行費減少量（學徒 -10／匠人 -18／師匠 -28／宗師 -40，最終費用不低於 10）。
pub fn travel_cost_reduction(m: &Masteries) -> u32 {
    EXPLORER_TRAVEL[m.explorer_rank().tier()]
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

/// 世界頻道晉階同慶台詞（ROADMAP 351）：玩家在某條熟練度躍升到高階（師匠／宗師）時
/// 全服報一聲。純查表、零 LLM；`{name}` 由 `name` 直接填入（i18n 集中替換點）。
pub fn rank_up_line(class: JobClass, rank: MasteryRank, name: &str) -> String {
    format!(
        "🏅 {emoji} {name} 晉升為「{cls}{rank}」！",
        emoji = class.emoji(),
        name = name,
        cls = class.label(),
        rank = rank.label(),
    )
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

    // ── ROADMAP 351 階梯榮銜 ─────────────────────────────────────────────────

    #[test]
    fn rank_thresholds_map_correctly() {
        assert_eq!(MasteryRank::from_level(0),  MasteryRank::Novice);
        assert_eq!(MasteryRank::from_level(1),  MasteryRank::Apprentice);
        assert_eq!(MasteryRank::from_level(4),  MasteryRank::Apprentice);
        assert_eq!(MasteryRank::from_level(5),  MasteryRank::Adept);
        assert_eq!(MasteryRank::from_level(9),  MasteryRank::Adept);
        assert_eq!(MasteryRank::from_level(10), MasteryRank::Master);
        assert_eq!(MasteryRank::from_level(19), MasteryRank::Master);
        assert_eq!(MasteryRank::from_level(20), MasteryRank::Grandmaster);
        assert_eq!(MasteryRank::from_level(999), MasteryRank::Grandmaster);
    }

    #[test]
    fn rank_tier_is_monotonic_and_bounded() {
        // tier 嚴格遞增 0→4，且都落在加成表索引範圍。
        let ranks = [
            MasteryRank::Novice, MasteryRank::Apprentice, MasteryRank::Adept,
            MasteryRank::Master, MasteryRank::Grandmaster,
        ];
        for (i, r) in ranks.iter().enumerate() {
            assert_eq!(r.tier(), i);
            assert!(r.tier() < 5);
        }
    }

    #[test]
    fn rank_high_only_master_and_above() {
        assert!(!MasteryRank::Novice.is_high());
        assert!(!MasteryRank::Apprentice.is_high());
        assert!(!MasteryRank::Adept.is_high());
        assert!(MasteryRank::Master.is_high());
        assert!(MasteryRank::Grandmaster.is_high());
    }

    #[test]
    fn apprentice_tier_matches_legacy_flat_values() {
        // 鐵律：學徒檔（Lv1）= 舊版「等級 ≥1」數值，向後相容、舊測試全綠。
        assert_eq!(combat_bonus(&warrior_lv1()), 5);
        assert_eq!(hp_bonus(&warrior_lv1()), 10);
        assert_eq!(harvest_ether_bonus(&farmer_lv1()), 3);
        assert_eq!(crafting_reduction(&artisan_lv1()), 1);
        assert_eq!(travel_cost_reduction(&explorer_lv1()), 10);
        assert_eq!(npc_sell_bonus_pct(&farmer_lv1()), 25);
        assert_eq!(npc_sell_bonus_pct(&merchant_lv1()), 50);
    }

    fn at_level(field: fn(&mut Masteries, u32), lv: u32) -> Masteries {
        let mut m = Masteries::default();
        field(&mut m, lv * XP_PER_LEVEL);
        m
    }

    #[test]
    fn warrior_bonus_scales_up_through_ranks() {
        let set_w = |m: &mut Masteries, xp: u32| m.warrior = xp;
        // 匠人(Lv5)／師匠(Lv10)／宗師(Lv20) 攻擊力逐階變強。
        assert_eq!(combat_bonus(&at_level(set_w, 5)), 8);
        assert_eq!(combat_bonus(&at_level(set_w, 10)), 12);
        assert_eq!(combat_bonus(&at_level(set_w, 20)), 18);
        // HP 同樣逐階變強，且嚴格遞增。
        assert!(hp_bonus(&at_level(set_w, 5)) > hp_bonus(&warrior_lv1()));
        assert!(hp_bonus(&at_level(set_w, 20)) > hp_bonus(&at_level(set_w, 10)));
    }

    #[test]
    fn economy_bonuses_scale_but_stay_bounded() {
        let set_f = |m: &mut Masteries, xp: u32| m.farmer = xp;
        let set_m = |m: &mut Masteries, xp: u32| m.merchant = xp;
        let set_e = |m: &mut Masteries, xp: u32| m.explorer = xp;
        // 農夫收割乙太逐階增。
        assert_eq!(harvest_ether_bonus(&at_level(set_f, 10)), 6);
        assert_eq!(harvest_ether_bonus(&at_level(set_f, 20)), 9);
        // 商人收購百分比逐階增、宗師封頂 100%。
        assert_eq!(npc_sell_bonus_pct(&at_level(set_m, 5)), 62);
        assert_eq!(npc_sell_bonus_pct(&at_level(set_m, 20)), 100);
        // 旅行折扣逐階增，但實際費用永不低於 10。
        assert_eq!(apply_travel_discount(&at_level(set_e, 20), 100), 60);
        assert_eq!(apply_travel_discount(&at_level(set_e, 20), 5), 10);
    }

    #[test]
    fn npc_bonus_takes_higher_of_farmer_or_merchant_per_tier() {
        // 農夫宗師(50%) vs 商人匠人(62%) → 取較大 62%。
        let m = Masteries { farmer: 20 * XP_PER_LEVEL, merchant: 5 * XP_PER_LEVEL, ..Default::default() };
        assert_eq!(npc_sell_bonus_pct(&m), 62);
    }

    #[test]
    fn tier_snapshot_reflects_each_track() {
        let m = Masteries {
            warrior:  1 * XP_PER_LEVEL,   // 學徒 tier1
            farmer:   5 * XP_PER_LEVEL,   // 匠人 tier2
            artisan:  10 * XP_PER_LEVEL,  // 師匠 tier3
            explorer: 20 * XP_PER_LEVEL,  // 宗師 tier4
            merchant: 0,                  // 未入門 tier0
        };
        assert_eq!(m.tier_snapshot(), [1, 2, 3, 4, 0]);
        // 順序與 JobClass::ALL 對齊。
        assert_eq!(JobClass::ALL, [
            JobClass::Warrior, JobClass::Farmer, JobClass::Artisan,
            JobClass::Explorer, JobClass::Merchant,
        ]);
    }

    #[test]
    fn rank_helper_matches_level() {
        let m = Masteries { artisan: 12 * XP_PER_LEVEL, ..Default::default() };
        assert_eq!(m.rank(JobClass::Artisan), MasteryRank::Master);
        assert_eq!(m.rank(JobClass::Warrior), MasteryRank::Novice);
    }

    #[test]
    fn rank_up_line_mentions_name_class_and_rank() {
        let line = rank_up_line(JobClass::Warrior, MasteryRank::Grandmaster, "阿吉");
        assert!(line.contains("阿吉"));
        assert!(line.contains("戰士"));
        assert!(line.contains("宗師"));
    }

    #[test]
    fn rank_wire_round_trips_with_serde() {
        // wire() 與 serde snake_case 一致（前端 i18n 鍵對得上）。
        for r in [
            MasteryRank::Novice, MasteryRank::Apprentice, MasteryRank::Adept,
            MasteryRank::Master, MasteryRank::Grandmaster,
        ] {
            let json = serde_json::to_string(&r).unwrap();
            assert_eq!(json, format!("\"{}\"", r.wire()));
        }
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
