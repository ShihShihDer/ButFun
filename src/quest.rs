//! 全服社群探索任務（ROADMAP 27）：三條任務讓所有玩家共同推進，完成時全員分潤乙太。
//!
//! 每輪固定三條任務（擊殺 / 採集 / 探索星球各一），全員共享進度；
//! 三條全完或計時 120 分鐘後自動換下一輪。

use crate::combat::EnemyKind;
use crate::inventory::ItemKind;

/// 每條任務完成後，所有在線玩家各得的乙太獎勵。
pub const QUEST_COMPLETE_REWARD: u32 = 15;

/// 每輪任務自動重置的時限（秒）：120 分鐘。
pub const QUEST_RESET_SECS: f32 = 7_200.0;

/// 一條任務的目標種類。
#[derive(Debug, Clone, PartialEq)]
pub enum QuestTarget {
    Kill { kind: EnemyKind, count: u32 },
    Gather { item: ItemKind, count: u32 },
    /// planet 對應 PLANET_* 常數字串。
    Travel { planet: &'static str },
}

/// 一條正在進行中的社群任務。
#[derive(Debug, Clone)]
pub struct CommunityQuest {
    pub target: QuestTarget,
    pub progress: u32,
    pub completed: bool,
    /// 前端顯示用的說明（繁中）。
    pub description: String,
}

impl CommunityQuest {
    fn new(target: QuestTarget, desc: &str) -> Self {
        Self { target, progress: 0, completed: false, description: desc.to_string() }
    }

    pub fn goal(&self) -> u32 {
        match &self.target {
            QuestTarget::Kill { count, .. } => *count,
            QuestTarget::Gather { count, .. } => *count,
            QuestTarget::Travel { .. } => 1,
        }
    }

    /// 推進進度；回傳 true 代表此次推進讓任務剛完成。
    pub fn advance(&mut self, amount: u32) -> bool {
        if self.completed { return false; }
        let goal = self.goal();
        self.progress = self.progress.saturating_add(amount).min(goal);
        if self.progress >= goal {
            self.completed = true;
            true
        } else {
            false
        }
    }
}

/// 三條任務 + 計時器的整體狀態。
pub struct QuestState {
    pub quests: [CommunityQuest; 3],
    pub reset_timer: f32,
    bucket: u32,
}

impl QuestState {
    pub fn new() -> Self {
        Self { quests: Self::generate(0), reset_timer: QUEST_RESET_SECS, bucket: 0 }
    }

    /// 依 bucket 循環決定本輪三條任務（擊殺 / 採集 / 探索星球）。
    fn generate(bucket: u32) -> [CommunityQuest; 3] {
        match bucket % 6 {
            0 => [
                CommunityQuest::new(QuestTarget::Kill { kind: EnemyKind::EtherWisp, count: 8 }, "擊敗 8 隻迷途乙太靈"),
                CommunityQuest::new(QuestTarget::Gather { item: ItemKind::Wood, count: 15 }, "採集 15 個木材"),
                CommunityQuest::new(QuestTarget::Travel { planet: "verdant" }, "星際探索：翠幽星"),
            ],
            1 => [
                CommunityQuest::new(QuestTarget::Kill { kind: EnemyKind::MushroomStalker, count: 5 }, "擊敗 5 隻蕈菇潛行者"),
                CommunityQuest::new(QuestTarget::Gather { item: ItemKind::CrystalShard, count: 6 }, "採集 6 個晶石碎片"),
                CommunityQuest::new(QuestTarget::Travel { planet: "crimson" }, "星際探索：赤焰星"),
            ],
            2 => [
                CommunityQuest::new(QuestTarget::Kill { kind: EnemyKind::CrystalGolem, count: 3 }, "擊敗 3 隻晶石傀儡"),
                CommunityQuest::new(QuestTarget::Gather { item: ItemKind::MushroomSpore, count: 8 }, "採集 8 個蕈菇孢子"),
                CommunityQuest::new(QuestTarget::Travel { planet: "void" }, "星際探索：虛空星"),
            ],
            3 => [
                CommunityQuest::new(QuestTarget::Kill { kind: EnemyKind::RuneGuardian, count: 4 }, "擊敗 4 隻符文守衛"),
                CommunityQuest::new(QuestTarget::Gather { item: ItemKind::AncientFragment, count: 5 }, "採集 5 個古代碎片"),
                CommunityQuest::new(QuestTarget::Travel { planet: "aether" }, "星際探索：霧醚星"),
            ],
            4 => [
                CommunityQuest::new(QuestTarget::Kill { kind: EnemyKind::CoralCrab, count: 3 }, "擊敗 3 隻珊瑚蟹"),
                CommunityQuest::new(QuestTarget::Gather { item: ItemKind::DeepSeaPearl, count: 4 }, "採集 4 個深海珍珠"),
                CommunityQuest::new(QuestTarget::Travel { planet: "origin" }, "星際探索：星源星"),
            ],
            _ => [
                CommunityQuest::new(QuestTarget::Kill { kind: EnemyKind::RiftGuardian, count: 1 }, "擊敗裂縫守護者"),
                CommunityQuest::new(QuestTarget::Gather { item: ItemKind::JadeShard, count: 6 }, "採集 6 個翠幽碎片"),
                CommunityQuest::new(QuestTarget::Travel { planet: "verdant" }, "星際探索：翠幽星"),
            ],
        }
    }

    /// 每 tick 推進計時。若計時到期或三條全完則換下一輪，回傳 true。
    pub fn tick(&mut self, dt: f32) -> bool {
        self.reset_timer -= dt;
        let all_done = self.quests.iter().all(|q| q.completed);
        if self.reset_timer <= 0.0 || all_done {
            self.bucket = self.bucket.wrapping_add(1);
            self.quests = Self::generate(self.bucket);
            self.reset_timer = QUEST_RESET_SECS;
            return true;
        }
        false
    }

    /// 通知有敵人被打倒。回傳剛完成之任務的說明清單（供廣播用）。
    pub fn on_kill(&mut self, kind: EnemyKind) -> Vec<String> {
        let mut done = vec![];
        for q in self.quests.iter_mut() {
            if let QuestTarget::Kill { kind: k, .. } = q.target {
                if k == kind && q.advance(1) {
                    done.push(q.description.clone());
                }
            }
        }
        done
    }

    /// 通知有物品被採集。回傳剛完成之任務的說明清單。
    pub fn on_gather(&mut self, item: ItemKind) -> Vec<String> {
        let mut done = vec![];
        for q in self.quests.iter_mut() {
            if let QuestTarget::Gather { item: k, .. } = q.target {
                if k == item && q.advance(1) {
                    done.push(q.description.clone());
                }
            }
        }
        done
    }

    /// 通知有玩家旅行到星球。回傳剛完成之任務的說明清單。
    pub fn on_travel(&mut self, planet: &str) -> Vec<String> {
        let mut done = vec![];
        for q in self.quests.iter_mut() {
            if let QuestTarget::Travel { planet: p } = q.target {
                if p == planet && q.advance(1) {
                    done.push(q.description.clone());
                }
            }
        }
        done
    }

    /// 剩餘秒數（前端顯示倒計時用）。
    pub fn secs_remaining(&self) -> u32 {
        self.reset_timer.max(0.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 新狀態三條任務全未完成() {
        let s = QuestState::new();
        assert!(s.quests.iter().all(|q| !q.completed));
        assert!(s.quests.iter().all(|q| q.progress == 0));
    }

    #[test]
    fn 擊殺推進進度並可完成() {
        let mut s = QuestState::new();
        // bucket 0 擊殺任務：8 隻 EtherWisp
        let goal = s.quests[0].goal();
        for i in 0..goal {
            let done = s.on_kill(EnemyKind::EtherWisp);
            if i < goal - 1 { assert!(done.is_empty()); }
            else { assert!(!done.is_empty()); }
        }
        assert!(s.quests[0].completed);
    }

    #[test]
    fn 採集推進進度並可完成() {
        let mut s = QuestState::new();
        // bucket 0 採集任務：15 個 Wood
        let goal = s.quests[1].goal();
        for i in 0..goal {
            let done = s.on_gather(ItemKind::Wood);
            if i < goal - 1 { assert!(done.is_empty()); }
            else { assert!(!done.is_empty()); }
        }
        assert!(s.quests[1].completed);
    }

    #[test]
    fn 旅行完成星球任務() {
        let mut s = QuestState::new();
        // bucket 0 旅行任務：verdant
        let done = s.on_travel("verdant");
        assert!(!done.is_empty());
        assert!(s.quests[2].completed);
    }

    #[test]
    fn 計時到期後重置() {
        let mut s = QuestState::new();
        let old_bucket = s.bucket;
        let reset = s.tick(QUEST_RESET_SECS + 1.0);
        assert!(reset);
        assert_eq!(s.bucket, old_bucket + 1);
        assert!(s.quests.iter().all(|q| !q.completed));
    }

    #[test]
    fn 三條全完後提前重置() {
        let mut s = QuestState::new();
        // 強制完成三條
        s.quests[0].completed = true;
        s.quests[1].completed = true;
        s.quests[2].completed = true;
        let reset = s.tick(1.0);
        assert!(reset);
        assert!(s.quests.iter().all(|q| !q.completed));
    }

    #[test]
    fn 六輪後循環回第一輪() {
        let mut s = QuestState::new();
        for _ in 0..6 {
            s.bucket = s.bucket.wrapping_add(1);
        }
        let fresh = QuestState::generate(s.bucket);
        // bucket % 6 == 0，應和初始相同（比較說明字串）
        let first = QuestState::generate(0);
        assert_eq!(fresh[0].description, first[0].description);
    }
}
