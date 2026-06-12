//! ROADMAP 155 住家家具系統——在室內空間擺放家具、獲得被動增益。
//!
//! 延伸 ROADMAP 111 住家內裝（「裝飾留後續切片」即此）。
//! 每位 FreeBuild 地塊地主可在室內放最多 5 件家具，各自帶被動加成：
//!   🛏️ 蒸汽床鋪   → 每 30 秒回血 2（脫離戰鬥時）
//!   📦 乙太寶箱   → 背包物品種類上限 +3
//!   🪴 乙太花盆   → 採集 EXP +8%
//!   🔮 星魂燈     → 夜間攻擊力 +2
//!   🏺 古代擺件   → NPC 收購 +10%
//!
//! 記憶體模式（重啟清空家具列表，效果仍可持續到材料不在為止，但因純記憶體 → 重啟即空，
//! 行為等同「進住家後才能再放」；不需 migration）。
//! 每件家具消耗一個對應 ItemKind；移除時退還到背包。

use serde::{Deserialize, Serialize};

/// 家具種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FurnitureKind {
    /// 🛏️ 蒸汽床鋪（Wood×4 + Stone×2）：每 30 秒回血 2。
    SteamBed,
    /// 📦 乙太寶箱（Wood×3 + Stone×4）：背包物品種類上限 +3。
    AetherChest,
    /// 🪴 乙太花盆（WildFlower×2 + Wood×2）：採集 EXP +8%。
    EtherPlant,
    /// 🔮 星魂燈（StarCrystalShard×2 + Stone×2）：夜間攻擊力 +2。
    StarLantern,
    /// 🏺 古代擺件（AncientFragment×2 + Stone×1）：NPC 收購 +10%。
    AncientDeco,
}

impl FurnitureKind {
    /// 對應顯示 emoji。
    pub fn emoji(self) -> &'static str {
        match self {
            Self::SteamBed    => "🛏️",
            Self::AetherChest => "📦",
            Self::EtherPlant  => "🪴",
            Self::StarLantern => "🔮",
            Self::AncientDeco => "🏺",
        }
    }

    /// 中文名稱。
    pub fn label(self) -> &'static str {
        match self {
            Self::SteamBed    => "蒸汽床鋪",
            Self::AetherChest => "乙太寶箱",
            Self::EtherPlant  => "乙太花盆",
            Self::StarLantern => "星魂燈",
            Self::AncientDeco => "古代擺件",
        }
    }

    /// 效果說明（i18n 佔位）。
    pub fn effect_desc(self) -> &'static str {
        match self {
            Self::SteamBed    => "每 30 秒回血 2",
            Self::AetherChest => "背包種類上限 +3",
            Self::EtherPlant  => "採集 EXP +8%",
            Self::StarLantern => "夜間攻擊力 +2",
            Self::AncientDeco => "NPC 收購 +10%",
        }
    }

    /// 從 snake_case 字串解析（前端送 `kind` 欄位用）。
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "steam_bed"    => Some(Self::SteamBed),
            "aether_chest" => Some(Self::AetherChest),
            "ether_plant"  => Some(Self::EtherPlant),
            "star_lantern" => Some(Self::StarLantern),
            "ancient_deco" => Some(Self::AncientDeco),
            _ => None,
        }
    }
}

/// 對應的 ItemKind snake_case 字串（供 crafting recipe id 對照）。
pub const BED_ITEM: &str = "steam_bed";
pub const CHEST_ITEM: &str = "aether_chest";
pub const PLANT_ITEM: &str = "ether_plant";
pub const LANTERN_ITEM: &str = "star_lantern";
pub const DECO_ITEM: &str = "ancient_deco";

/// 每個住家最多可放幾件家具。
pub const MAX_FURNITURE: usize = 5;

/// 床鋪回血間隔（秒）。
pub const BED_REGEN_INTERVAL_SECS: f32 = 30.0;

/// 床鋪每次回血量。
pub const BED_REGEN_HP: u32 = 2;

/// 夜間攻擊力加成。
pub const LANTERN_NIGHT_ATK_BONUS: i32 = 2;

/// NPC 收購加成百分比（整數，加在 earned 上再除 100）。
pub const DECO_NPC_BONUS_PCT: u32 = 10;

/// 採集 EXP 加成百分比。
pub const PLANT_GATHER_EXP_PCT: u32 = 8;

/// 背包種類上限加成。
pub const CHEST_CAPACITY_BONUS: usize = 3;

/// 前端顯示用的家具快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FurnitureView {
    /// 家具索引（移除時用）。
    pub idx: usize,
    pub kind: String,
    pub emoji: &'static str,
    pub label: &'static str,
    pub effect: &'static str,
}

/// 某個住家的家具列表。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HomeFurnishings {
    /// 已放置的家具（最多 MAX_FURNITURE 件）。
    items: Vec<FurnitureKind>,
}

impl HomeFurnishings {
    /// 嘗試放置一件家具。成功回 `true`；已達上限或同種類已存在回 `false`。
    /// 每種家具只能放一件（語意唯一：各有獨特被動效果）。
    pub fn place(&mut self, kind: FurnitureKind) -> bool {
        if self.items.len() >= MAX_FURNITURE || self.items.contains(&kind) {
            return false;
        }
        self.items.push(kind);
        true
    }

    /// 移除指定索引的家具，回傳被移除的種類；索引越界回 `None`。
    pub fn remove(&mut self, idx: usize) -> Option<FurnitureKind> {
        if idx >= self.items.len() {
            return None;
        }
        Some(self.items.remove(idx))
    }

    /// 是否有蒸汽床鋪。
    pub fn has_bed(&self) -> bool {
        self.items.contains(&FurnitureKind::SteamBed)
    }

    /// 是否有乙太寶箱。
    pub fn has_chest(&self) -> bool {
        self.items.contains(&FurnitureKind::AetherChest)
    }

    /// 是否有乙太花盆。
    pub fn has_plant(&self) -> bool {
        self.items.contains(&FurnitureKind::EtherPlant)
    }

    /// 是否有星魂燈。
    pub fn has_lantern(&self) -> bool {
        self.items.contains(&FurnitureKind::StarLantern)
    }

    /// 是否有古代擺件。
    pub fn has_deco(&self) -> bool {
        self.items.contains(&FurnitureKind::AncientDeco)
    }

    /// 目前已放幾件。
    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// 產生前端顯示快照。
    pub fn views(&self) -> Vec<FurnitureView> {
        self.items.iter().enumerate().map(|(idx, &kind)| FurnitureView {
            idx,
            kind: format!("{kind:?}").chars().fold(String::new(), |mut s, c| {
                if c.is_uppercase() && !s.is_empty() { s.push('_'); }
                s.push(c.to_ascii_lowercase());
                s
            }),
            emoji: kind.emoji(),
            label: kind.label(),
            effect: kind.effect_desc(),
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_up_to_max() {
        let mut h = HomeFurnishings::default();
        // 五種各放一件，剛好達到上限
        assert!(h.place(FurnitureKind::SteamBed));
        assert!(h.place(FurnitureKind::AetherChest));
        assert!(h.place(FurnitureKind::EtherPlant));
        assert!(h.place(FurnitureKind::StarLantern));
        assert!(h.place(FurnitureKind::AncientDeco));
        assert_eq!(h.count(), MAX_FURNITURE);
        // 已達上限，任何新放置都應拒絕
        assert!(!h.place(FurnitureKind::SteamBed));
    }

    #[test]
    fn no_duplicate_kind() {
        let mut h = HomeFurnishings::default();
        assert!(h.place(FurnitureKind::SteamBed));
        // 同種類第二次放置應拒絕
        assert!(!h.place(FurnitureKind::SteamBed));
        assert_eq!(h.count(), 1);
        // 不同種類仍可放
        assert!(h.place(FurnitureKind::AetherChest));
        assert_eq!(h.count(), 2);
    }

    #[test]
    fn remove_returns_kind() {
        let mut h = HomeFurnishings::default();
        h.place(FurnitureKind::SteamBed);
        h.place(FurnitureKind::AetherChest);
        let removed = h.remove(0);
        assert_eq!(removed, Some(FurnitureKind::SteamBed));
        assert_eq!(h.count(), 1);
        // 移除後 AetherChest 的索引變 0
        assert_eq!(h.remove(0), Some(FurnitureKind::AetherChest));
        assert_eq!(h.count(), 0);
    }

    #[test]
    fn remove_out_of_bounds() {
        let mut h = HomeFurnishings::default();
        assert!(h.remove(0).is_none());
        h.place(FurnitureKind::StarLantern);
        assert!(h.remove(1).is_none());
    }

    #[test]
    fn has_checks() {
        let mut h = HomeFurnishings::default();
        assert!(!h.has_bed() && !h.has_chest() && !h.has_plant() && !h.has_lantern() && !h.has_deco());
        h.place(FurnitureKind::SteamBed);
        assert!(h.has_bed());
        h.place(FurnitureKind::AetherChest);
        assert!(h.has_chest());
        h.place(FurnitureKind::EtherPlant);
        assert!(h.has_plant());
        h.place(FurnitureKind::StarLantern);
        assert!(h.has_lantern());
        h.place(FurnitureKind::AncientDeco);
        assert!(h.has_deco());
    }

    #[test]
    fn has_cleared_after_remove() {
        let mut h = HomeFurnishings::default();
        h.place(FurnitureKind::SteamBed);
        assert!(h.has_bed());
        h.remove(0);
        assert!(!h.has_bed());
    }

    #[test]
    fn from_str_roundtrip() {
        assert_eq!(FurnitureKind::from_str("steam_bed"),    Some(FurnitureKind::SteamBed));
        assert_eq!(FurnitureKind::from_str("aether_chest"), Some(FurnitureKind::AetherChest));
        assert_eq!(FurnitureKind::from_str("ether_plant"),  Some(FurnitureKind::EtherPlant));
        assert_eq!(FurnitureKind::from_str("star_lantern"), Some(FurnitureKind::StarLantern));
        assert_eq!(FurnitureKind::from_str("ancient_deco"), Some(FurnitureKind::AncientDeco));
        assert!(FurnitureKind::from_str("unknown").is_none());
    }

    #[test]
    fn views_correct_count_and_idx() {
        let mut h = HomeFurnishings::default();
        h.place(FurnitureKind::SteamBed);
        h.place(FurnitureKind::StarLantern);
        let v = h.views();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].idx, 0);
        assert_eq!(v[1].idx, 1);
        assert_eq!(v[0].emoji, "🛏️");
        assert_eq!(v[1].emoji, "🔮");
    }

    #[test]
    fn constants_reasonable() {
        assert!(MAX_FURNITURE >= 5);
        assert!(BED_REGEN_INTERVAL_SECS > 0.0);
        assert!(BED_REGEN_HP > 0);
        assert!(LANTERN_NIGHT_ATK_BONUS > 0);
        assert!(DECO_NPC_BONUS_PCT > 0 && DECO_NPC_BONUS_PCT < 100);
        assert!(PLANT_GATHER_EXP_PCT > 0 && PLANT_GATHER_EXP_PCT < 100);
        assert!(CHEST_CAPACITY_BONUS > 0);
    }

    #[test]
    fn default_is_empty() {
        let h = HomeFurnishings::default();
        assert_eq!(h.count(), 0);
        assert!(h.views().is_empty());
    }
}
