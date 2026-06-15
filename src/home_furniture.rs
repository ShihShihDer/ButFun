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

use crate::home_interior::is_floor_cell;

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

/// 一件已擺放的家具：種類 + 室內地板格座標 (col, row)。
/// ROADMAP 323：家具從「一串看不見的被動加成」升級成「擺在室內具體格子、進房看得到的佈置」。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PlacedFurniture {
    pub kind: FurnitureKind,
    /// 室內地板格座標（1..=6，去掉外圍石磚牆）。
    pub col: u8,
    pub row: u8,
}

/// 前端顯示用的家具快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FurnitureView {
    /// 家具索引（移除時用）。
    pub idx: usize,
    pub kind: String,
    pub emoji: &'static str,
    pub label: &'static str,
    pub effect: &'static str,
    /// 室內地板格座標——前端據此把家具畫在玩家擺放的那一格（ROADMAP 323）。
    pub col: u8,
    pub row: u8,
}

/// 某個住家的家具列表。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HomeFurnishings {
    /// 已放置的家具（最多 MAX_FURNITURE 件），各自帶室內格座標。
    items: Vec<PlacedFurniture>,
    /// 居家風格主題（ROADMAP 325）：決定室內地板/牆面色調。
    /// serde default → 木屋，舊快照（無此欄位）反序列化後維持原始外觀，向後相容。
    #[serde(default)]
    style: crate::home_interior::HomeStyle,
}

impl HomeFurnishings {
    /// 嘗試在地板格 (col, row) 擺放一件家具。成功回 `true`；下列任一情況回 `false`：
    /// 已達上限／同種類已存在／目標格非地板格／目標格已被其他家具占用。
    /// 每種家具只能放一件（語意唯一：各有獨特被動效果）。
    pub fn place(&mut self, kind: FurnitureKind, col: u8, row: u8) -> bool {
        if self.items.len() >= MAX_FURNITURE
            || self.items.iter().any(|f| f.kind == kind)
            || !is_floor_cell(col, row)
            || self.items.iter().any(|f| f.col == col && f.row == row)
        {
            return false;
        }
        self.items.push(PlacedFurniture { kind, col, row });
        true
    }

    /// 移除指定索引的家具，回傳被移除的種類；索引越界回 `None`。
    pub fn remove(&mut self, idx: usize) -> Option<FurnitureKind> {
        if idx >= self.items.len() {
            return None;
        }
        Some(self.items.remove(idx).kind)
    }

    /// 某種家具是否已擺放。
    fn has_kind(&self, kind: FurnitureKind) -> bool {
        self.items.iter().any(|f| f.kind == kind)
    }

    /// 是否有蒸汽床鋪。
    pub fn has_bed(&self) -> bool {
        self.has_kind(FurnitureKind::SteamBed)
    }

    /// 是否有乙太寶箱。
    pub fn has_chest(&self) -> bool {
        self.has_kind(FurnitureKind::AetherChest)
    }

    /// 是否有乙太花盆。
    pub fn has_plant(&self) -> bool {
        self.has_kind(FurnitureKind::EtherPlant)
    }

    /// 是否有星魂燈。
    pub fn has_lantern(&self) -> bool {
        self.has_kind(FurnitureKind::StarLantern)
    }

    /// 是否有古代擺件。
    pub fn has_deco(&self) -> bool {
        self.has_kind(FurnitureKind::AncientDeco)
    }

    /// 目前已放幾件。
    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// 當前居家風格主題（ROADMAP 325）。
    pub fn style(&self) -> crate::home_interior::HomeStyle {
        self.style
    }

    /// 循環切換到下一個居家風格，回傳切換後的新風格。
    pub fn cycle_style(&mut self) -> crate::home_interior::HomeStyle {
        self.style = self.style.next();
        self.style
    }

    /// 產生前端顯示快照。
    pub fn views(&self) -> Vec<FurnitureView> {
        self.items.iter().enumerate().map(|(idx, f)| FurnitureView {
            idx,
            kind: format!("{:?}", f.kind).chars().fold(String::new(), |mut s, c| {
                if c.is_uppercase() && !s.is_empty() { s.push('_'); }
                s.push(c.to_ascii_lowercase());
                s
            }),
            emoji: f.kind.emoji(),
            label: f.kind.label(),
            effect: f.kind.effect_desc(),
            col: f.col,
            row: f.row,
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_up_to_max() {
        let mut h = HomeFurnishings::default();
        // 五種各放在不同格，剛好達到上限
        assert!(h.place(FurnitureKind::SteamBed, 1, 1));
        assert!(h.place(FurnitureKind::AetherChest, 2, 1));
        assert!(h.place(FurnitureKind::EtherPlant, 3, 1));
        assert!(h.place(FurnitureKind::StarLantern, 4, 1));
        assert!(h.place(FurnitureKind::AncientDeco, 5, 1));
        assert_eq!(h.count(), MAX_FURNITURE);
        // 已達上限，任何新放置都應拒絕（即便目標格仍空）
        assert!(!h.place(FurnitureKind::SteamBed, 6, 1));
    }

    #[test]
    fn no_duplicate_kind() {
        let mut h = HomeFurnishings::default();
        assert!(h.place(FurnitureKind::SteamBed, 1, 1));
        // 同種類第二次放置應拒絕（即便換到別格）
        assert!(!h.place(FurnitureKind::SteamBed, 2, 1));
        assert_eq!(h.count(), 1);
        // 不同種類仍可放
        assert!(h.place(FurnitureKind::AetherChest, 2, 1));
        assert_eq!(h.count(), 2);
    }

    #[test]
    fn place_rejects_non_floor_cell() {
        let mut h = HomeFurnishings::default();
        // 外圍石磚牆格（0 或 7）不可擺放
        assert!(!h.place(FurnitureKind::SteamBed, 0, 1));
        assert!(!h.place(FurnitureKind::SteamBed, 1, 7));
        assert_eq!(h.count(), 0);
        // 地板格可擺放
        assert!(h.place(FurnitureKind::SteamBed, 1, 1));
        assert_eq!(h.count(), 1);
    }

    #[test]
    fn place_rejects_occupied_cell() {
        let mut h = HomeFurnishings::default();
        assert!(h.place(FurnitureKind::SteamBed, 3, 3));
        // 同一格已被占用，另一種家具不能疊上去
        assert!(!h.place(FurnitureKind::AetherChest, 3, 3));
        assert_eq!(h.count(), 1);
        // 換到空格就可以
        assert!(h.place(FurnitureKind::AetherChest, 3, 4));
        assert_eq!(h.count(), 2);
    }

    #[test]
    fn remove_returns_kind() {
        let mut h = HomeFurnishings::default();
        h.place(FurnitureKind::SteamBed, 1, 1);
        h.place(FurnitureKind::AetherChest, 2, 1);
        let removed = h.remove(0);
        assert_eq!(removed, Some(FurnitureKind::SteamBed));
        assert_eq!(h.count(), 1);
        // 移除後 AetherChest 的索引變 0
        assert_eq!(h.remove(0), Some(FurnitureKind::AetherChest));
        assert_eq!(h.count(), 0);
    }

    #[test]
    fn remove_then_replace_elsewhere() {
        // 「移動」＝移除退背包後改擺他處：移除後同種家具可重放到別格。
        let mut h = HomeFurnishings::default();
        assert!(h.place(FurnitureKind::SteamBed, 1, 1));
        assert_eq!(h.remove(0), Some(FurnitureKind::SteamBed));
        assert!(h.place(FurnitureKind::SteamBed, 5, 5));
        let v = h.views();
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].col, v[0].row), (5, 5));
    }

    #[test]
    fn remove_out_of_bounds() {
        let mut h = HomeFurnishings::default();
        assert!(h.remove(0).is_none());
        h.place(FurnitureKind::StarLantern, 1, 1);
        assert!(h.remove(1).is_none());
    }

    #[test]
    fn has_checks() {
        let mut h = HomeFurnishings::default();
        assert!(!h.has_bed() && !h.has_chest() && !h.has_plant() && !h.has_lantern() && !h.has_deco());
        h.place(FurnitureKind::SteamBed, 1, 1);
        assert!(h.has_bed());
        h.place(FurnitureKind::AetherChest, 2, 1);
        assert!(h.has_chest());
        h.place(FurnitureKind::EtherPlant, 3, 1);
        assert!(h.has_plant());
        h.place(FurnitureKind::StarLantern, 4, 1);
        assert!(h.has_lantern());
        h.place(FurnitureKind::AncientDeco, 5, 1);
        assert!(h.has_deco());
    }

    #[test]
    fn has_cleared_after_remove() {
        let mut h = HomeFurnishings::default();
        h.place(FurnitureKind::SteamBed, 1, 1);
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
        h.place(FurnitureKind::SteamBed, 2, 3);
        h.place(FurnitureKind::StarLantern, 5, 6);
        let v = h.views();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].idx, 0);
        assert_eq!(v[1].idx, 1);
        assert_eq!(v[0].emoji, "🛏️");
        assert_eq!(v[1].emoji, "🔮");
        // 快照帶回各家具的室內格座標，前端據此畫在玩家擺放的那一格。
        assert_eq!((v[0].col, v[0].row), (2, 3));
        assert_eq!((v[1].col, v[1].row), (5, 6));
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
