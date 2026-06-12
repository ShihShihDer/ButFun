//! 夜間乙太泉（ROADMAP 162）。
//!
//! 每當日夜循環由黃昏轉入夜晚時，在城外隨機散布 5 個「乙太泉」採集點。
//! 玩家走近 80px 以內互動，可獲得 +8 乙太（無需登入，登入者才真的加）。
//! 天亮（Night → Dawn）時，剩餘節點自動消失。
//!
//! 設計紀律：
//! - 純記憶體模式，重啟清零。零 migration，零 LLM。
//! - 純邏輯可獨立測試，不依賴 WebSocket / 遊戲迴圈。
//! - 節點座標固定（決定性，無偽隨機）——確保伺服器重啟後節點位置可重現。
//! - 玩家感知：夜幕降臨時全服廣播，「夜間探索有事做了」。

use crate::daynight::Phase;

/// 採集有效距離（像素）。
pub const COLLECT_REACH: f32 = 80.0;
/// 每夜出現的乙太泉數量。
pub const SPRING_COUNT: usize = 5;
/// 採集一個乙太泉獲得的乙太數。
pub const ETHER_REWARD: u32 = 8;

/// 5 個乙太泉的絕對世界座標。
/// 主城保護圈：以格子 (73,71) 為中心、chebyshev 半徑 34+8=42 格（42×32=1344px）。
/// 所有座標均經測試確認落在城鎮保護圈之外，鼓勵玩家夜間外出探索。
const SPRING_POSITIONS: [(f32, f32); SPRING_COUNT] = [
    (600.0, 2400.0),   // 城正西（gx=18, |18-73|=55 > 42）
    (3800.0, 2400.0),  // 城正東（gx=118, |118-73|=45 > 42）
    (2400.0, 600.0),   // 城正北（gy=18, |18-71|=53 > 42）
    (2400.0, 3800.0),  // 城正南（gy=118, |118-71|=47 > 42）
    (3800.0, 600.0),   // 城東北（兩軸均超出）
];

/// 一個乙太泉節點。
#[derive(Debug, Clone)]
pub struct SpringNode {
    /// 節點唯一 ID（用於前端送 CollectSpringNode）。
    pub id: u32,
    /// 世界座標 X。
    pub wx: f32,
    /// 世界座標 Y。
    pub wy: f32,
    /// 是否已被採集。
    pub collected: bool,
}

/// 夜間乙太泉系統狀態（純記憶體，重啟清零）。
pub struct NightAetherSprings {
    /// 是否目前有活躍的夜間乙太泉（夜晚期間）。
    pub active: bool,
    /// 當前乙太泉節點列表。
    pub nodes: Vec<SpringNode>,
    /// 上一個 tick 的日夜階段（用於偵測 Dusk→Night / Night→Dawn 轉換）。
    last_phase: Phase,
    /// 節點 ID 計數器（遞增，確保不重複）。
    node_counter: u32,
    /// 本夜是否已廣播過「全部採集完成」。
    pub all_collected_announced: bool,
}

impl NightAetherSprings {
    pub fn new() -> Self {
        Self {
            active: false,
            nodes: vec![],
            last_phase: Phase::Day,
            node_counter: 0,
            all_collected_announced: false,
        }
    }

    /// 是否所有節點都已被採集。
    pub fn all_collected(&self) -> bool {
        self.active && !self.nodes.is_empty() && self.nodes.iter().all(|n| n.collected)
    }

    /// 未被採集的節點（供快照廣播）。
    pub fn active_nodes(&self) -> impl Iterator<Item = &SpringNode> {
        self.nodes.iter().filter(|n| !n.collected)
    }

    /// 推進時間。回傳 `SpringsEvent`。
    ///
    /// - `None`：無特殊事件。
    /// - `Some(SpringsEvent::Activated)`：這個 tick 夜晚剛開始，節點已生成。
    /// - `Some(SpringsEvent::Deactivated)`：這個 tick 天亮，節點已清除。
    pub fn tick(&mut self, current_phase: Phase) -> Option<SpringsEvent> {
        let transition_to_night =
            (self.last_phase == Phase::Dusk || self.last_phase == Phase::Day)
            && current_phase == Phase::Night;
        let transition_to_dawn =
            self.last_phase == Phase::Night && current_phase == Phase::Dawn;

        self.last_phase = current_phase;

        if transition_to_night && !self.active {
            self.active = true;
            self.all_collected_announced = false;
            self.nodes = self.spawn_nodes();
            return Some(SpringsEvent::Activated);
        }

        if transition_to_dawn && self.active {
            self.active = false;
            self.nodes.clear();
            return Some(SpringsEvent::Deactivated);
        }

        None
    }

    /// 嘗試採集指定節點。玩家需在 COLLECT_REACH 範圍內。
    /// 回傳 `true` 表示成功採集，`false` 表示失敗（不在範圍/已採集/不存在）。
    pub fn try_collect(&mut self, node_id: u32, px: f32, py: f32) -> bool {
        if !self.active { return false; }
        if let Some(node) = self.nodes.iter_mut()
            .find(|n| n.id == node_id && !n.collected)
        {
            let dx = node.wx - px;
            let dy = node.wy - py;
            if dx * dx + dy * dy <= COLLECT_REACH * COLLECT_REACH {
                node.collected = true;
                return true;
            }
        }
        false
    }

    /// 生成 SPRING_COUNT 個節點，五方位各一個（決定性，可測試）。
    fn spawn_nodes(&mut self) -> Vec<SpringNode> {
        let nodes = SPRING_POSITIONS.iter().enumerate().map(|(i, &(wx, wy))| SpringNode {
            id: self.node_counter.wrapping_add(i as u32),
            wx,
            wy,
            collected: false,
        }).collect();
        self.node_counter = self.node_counter.wrapping_add(SPRING_COUNT as u32);
        nodes
    }
}

impl Default for NightAetherSprings {
    fn default() -> Self {
        Self::new()
    }
}

/// `tick()` 回傳的夜間乙太泉事件。
#[derive(Debug, Clone, PartialEq)]
pub enum SpringsEvent {
    /// 黃昏轉夜晚：乙太泉已生成，廣播通知全服。
    Activated,
    /// 夜晚轉黎明：乙太泉消失，安靜處理即可。
    Deactivated,
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_not_active() {
        let s = NightAetherSprings::new();
        assert!(!s.active);
        assert!(s.nodes.is_empty());
    }

    #[test]
    fn activates_on_dusk_to_night_transition() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        let ev = s.tick(Phase::Night);
        assert_eq!(ev, Some(SpringsEvent::Activated));
        assert!(s.active);
        assert_eq!(s.nodes.len(), SPRING_COUNT);
    }

    #[test]
    fn does_not_reactivate_if_already_active() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        // 再 tick 一次（仍是 Night → Night）
        let ev = s.tick(Phase::Night);
        assert_eq!(ev, None, "已啟動時不應再次觸發");
        assert_eq!(s.nodes.len(), SPRING_COUNT);
    }

    #[test]
    fn deactivates_on_night_to_dawn_transition() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let ev = s.tick(Phase::Dawn);
        assert_eq!(ev, Some(SpringsEvent::Deactivated));
        assert!(!s.active);
        assert!(s.nodes.is_empty());
    }

    #[test]
    fn no_event_during_steady_night() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let ev = s.tick(Phase::Night);
        assert_eq!(ev, None);
    }

    #[test]
    fn no_event_during_day() {
        let mut s = NightAetherSprings::new();
        s.tick(Phase::Dawn);
        let ev = s.tick(Phase::Day);
        assert_eq!(ev, None);
        assert!(!s.active);
    }

    #[test]
    fn nodes_have_unique_ids() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let ids: Vec<u32> = s.nodes.iter().map(|n| n.id).collect();
        let mut sorted = ids.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), SPRING_COUNT, "節點 ID 應全部不重複");
    }

    #[test]
    fn ids_differ_across_nights() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let ids_first: Vec<u32> = s.nodes.iter().map(|n| n.id).collect();
        s.tick(Phase::Dawn);
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let ids_second: Vec<u32> = s.nodes.iter().map(|n| n.id).collect();
        for id in &ids_second {
            assert!(!ids_first.contains(id), "第二晚節點 id 不應與第一晚重複");
        }
    }

    #[test]
    fn try_collect_succeeds_in_range() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let node = &s.nodes[0];
        let (wx, wy, id) = (node.wx, node.wy, node.id);
        let result = s.try_collect(id, wx + 10.0, wy + 10.0);
        assert!(result, "在範圍內應採集成功");
        assert!(s.nodes.iter().find(|n| n.id == id).unwrap().collected);
    }

    #[test]
    fn try_collect_fails_out_of_range() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let node = &s.nodes[0];
        let (wx, wy, id) = (node.wx, node.wy, node.id);
        let result = s.try_collect(id, wx + 200.0, wy + 200.0);
        assert!(!result, "超出範圍不應採集成功");
    }

    #[test]
    fn try_collect_fails_when_not_active() {
        let mut s = NightAetherSprings::new();
        let result = s.try_collect(0, 600.0, 2400.0);
        assert!(!result, "非夜間不應採集");
    }

    #[test]
    fn try_collect_already_collected_fails() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let node = &s.nodes[0];
        let (wx, wy, id) = (node.wx, node.wy, node.id);
        s.try_collect(id, wx, wy);
        let result = s.try_collect(id, wx, wy);
        assert!(!result, "同一節點不能重複採集");
    }

    #[test]
    fn active_nodes_excludes_collected() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let id = s.nodes[0].id;
        let wx = s.nodes[0].wx;
        let wy = s.nodes[0].wy;
        s.try_collect(id, wx, wy);
        let active: Vec<_> = s.active_nodes().collect();
        assert_eq!(active.len(), SPRING_COUNT - 1);
    }

    #[test]
    fn all_collected_check() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        assert!(!s.all_collected());
        for i in 0..SPRING_COUNT {
            let node = &s.nodes[i];
            let (wx, wy, id) = (node.wx, node.wy, node.id);
            s.try_collect(id, wx, wy);
        }
        assert!(s.all_collected());
    }

    #[test]
    fn spring_locations_outside_safe_zone() {
        // 驗證五個乙太泉節點位於城鎮保護圈之外。
        for &(wx, wy) in &SPRING_POSITIONS {
            assert!(
                !crate::positions::is_in_safe_zone(wx, wy),
                "乙太泉節點 ({wx},{wy}) 不應在城鎮保護圈內（鼓勵夜間外出探索）"
            );
        }
    }
}
