//! 城鎮大工程（ROADMAP 131）純邏輯：工程狀態、捐獻計算。
//!
//! 核心願景：讓玩家有共同奮鬥的長期目標，世界隨工程進度產生視覺變化。

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::inventory::ItemKind;

/// 工程狀態枚舉。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TownProjectStatus {
    Planning,
    Building,
    Completed,
}

/// 單個工程的進度與目標。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TownProjectState {
    pub project_id: String,
    pub name: String,
    pub status: TownProjectStatus,
    pub target_ether: u32,
    pub current_ether: u32,
    pub target_wood: u32,
    pub current_wood: u32,
    pub target_stone: u32,
    pub current_stone: u32,
    pub target_crystal: u32,
    pub current_crystal: u32,
    /// 前五大貢獻者 (player_id, name, score)
    pub top_contributors: Vec<(Uuid, String, u32)>,
}

impl TownProjectState {
    pub fn new_observatory() -> Self {
        Self {
            project_id: "observatory".to_string(),
            name: "蒸汽天文台".to_string(),
            status: TownProjectStatus::Building,
            target_ether: 10000,
            current_ether: 0,
            target_wood: 500,
            current_wood: 0,
            target_stone: 500,
            current_stone: 0,
            target_crystal: 200,
            current_crystal: 0,
            top_contributors: Vec::new(),
        }
    }

    /// 計算整體進度百分比 [0.0, 1.0]。
    pub fn progress_pct(&self) -> f32 {
        if self.status == TownProjectStatus::Completed { return 1.0; }
        let total_target = self.target_ether as f32 + (self.target_wood * 5) as f32 + (self.target_stone * 5) as f32 + (self.target_crystal * 15) as f32;
        let total_current = self.current_ether as f32 + (self.current_wood * 5) as f32 + (self.current_stone * 5) as f32 + (self.current_crystal * 15) as f32;
        if total_target <= 0.0 { return 1.0; }
        (total_current / total_target).min(1.0)
    }

    /// 接受捐獻。回傳增加的積分值。
    pub fn donate(&mut self, item: Option<ItemKind>, qty: u32) -> u32 {
        if self.status == TownProjectStatus::Completed { return 0; }
        match item {
            None => { // 捐乙太
                let can_take = self.target_ether.saturating_sub(self.current_ether).min(qty);
                self.current_ether += can_take;
                self.check_completion();
                can_take
            }
            Some(ItemKind::Wood) => {
                let can_take = self.target_wood.saturating_sub(self.current_wood).min(qty);
                self.current_wood += can_take;
                self.check_completion();
                can_take * 5
            }
            Some(ItemKind::Stone) => {
                let can_take = self.target_stone.saturating_sub(self.current_stone).min(qty);
                self.current_stone += can_take;
                self.check_completion();
                can_take * 5
            }
            Some(ItemKind::CrystalShard) | Some(ItemKind::StarCrystalShard) => {
                let can_take = self.target_crystal.saturating_sub(self.current_crystal).min(qty);
                self.current_crystal += can_take;
                self.check_completion();
                can_take * 15
            }
            _ => 0, // 不接受其他材料
        }
    }

    fn check_completion(&mut self) {
        if self.current_ether >= self.target_ether 
           && self.current_wood >= self.target_wood 
           && self.current_stone >= self.target_stone 
           && self.current_crystal >= self.target_crystal {
            self.status = TownProjectStatus::Completed;
        }
    }

    /// 更新前五大貢獻者名單（由 store 載入後填入）。
    pub fn update_contributors(&mut self, list: Vec<(Uuid, String, u32)>) {
        self.top_contributors = list;
    }

    pub fn view(&self) -> crate::protocol::TownProjectView {
        crate::protocol::TownProjectView {
            project_id: self.project_id.clone(),
            name: self.name.clone(),
            status: match self.status {
                TownProjectStatus::Planning => "planning".to_string(),
                TownProjectStatus::Building => "building".to_string(),
                TownProjectStatus::Completed => "completed".to_string(),
            },
            progress_pct: self.progress_pct(),
            current_ether: self.current_ether,
            target_ether: self.target_ether,
            current_wood: self.current_wood,
            target_wood: self.target_wood,
            current_stone: self.current_stone,
            target_stone: self.target_stone,
            current_crystal: self.current_crystal,
            target_crystal: self.target_crystal,
            top_contributors: self.top_contributors.iter().map(|(_, name, score)| {
                crate::protocol::ContributorView { name: name.clone(), score: *score }
            }).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_calculation() {
        let mut p = TownProjectState::new_observatory();
        assert_eq!(p.progress_pct(), 0.0);
        p.current_ether = 5000;
        assert!(p.progress_pct() > 0.0 && p.progress_pct() < 1.0);
        p.current_ether = 10000;
        p.current_wood = 500;
        p.current_stone = 500;
        p.current_crystal = 200;
        p.check_completion();
        assert_eq!(p.progress_pct(), 1.0);
        assert_eq!(p.status, TownProjectStatus::Completed);
    }

    #[test]
    fn donation_limits() {
        let mut p = TownProjectState::new_observatory();
        let score = p.donate(None, 12000);
        assert_eq!(score, 10000);
        assert_eq!(p.current_ether, 10000);
        
        let score_wood = p.donate(Some(ItemKind::Wood), 600);
        assert_eq!(score_wood, 500 * 5);
        assert_eq!(p.current_wood, 500);
    }
}
