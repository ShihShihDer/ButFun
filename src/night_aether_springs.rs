//! 夜間乙太泉（ROADMAP 162；ROADMAP 362 滿月乙太潮）。
//!
//! 每當日夜循環由黃昏轉入夜晚時，在城外隨機散布 5 個「乙太泉」採集點。
//! 玩家走近 80px 以內互動，可獲得 +8 乙太（無需登入，登入者才真的加）。
//! 天亮（Night → Dawn）時，剩餘節點自動消失。
//!
//! ROADMAP 362：若該夜恰逢滿月（由 `crate::moon::is_full_moon` 這份權威月相判定——
//! 同一份月相早已驅動「滿月夜掠食者嗥月」），則在 5 口尋常泉之外，再多生 3 口「月華泉」，
//! 讓「月相」第一次對玩家有善意後果、把月相與夜間探索兩條線接起來。月華泉只是「多出來的泉眼」，
//! 採集與獎勵路徑與尋常泉完全一致（走 ROADMAP 350 汲泉小遊戲），不碰任何獎勵平衡。
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
/// ROADMAP 362：滿月夜額外湧現的「月華泉」數量（疊加在 SPRING_COUNT 之上）。
pub const MOONLIT_SPRING_COUNT: usize = 3;

/// 滿月夜額外的 3 口月華泉座標（與 SPRING_POSITIONS 不重疊，皆經測試確認落在城鎮保護圈外）。
const MOONLIT_POSITIONS: [(f32, f32); MOONLIT_SPRING_COUNT] = [
    (600.0, 600.0),    // 城西北（gx=18, gy=18，兩軸均超出保護圈）
    (600.0, 3800.0),   // 城西南（gx=18, gy=118，兩軸均超出）
    (3800.0, 3800.0),  // 城東南（gx=118, gy=118，兩軸均超出）
];

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
    /// ROADMAP 362：是否為滿月夜額外湧現的「月華泉」（僅供前端區隔渲染；採集/獎勵與尋常泉一致）。
    pub moonlit: bool,
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
    /// ROADMAP 362：今夜是否恰逢滿月（夜晚開始時定格，天亮清零）。供廣播文案切換用。
    pub moonlit_tonight: bool,
}

impl NightAetherSprings {
    pub fn new() -> Self {
        Self {
            active: false,
            nodes: vec![],
            last_phase: Phase::Day,
            node_counter: 0,
            all_collected_announced: false,
            moonlit_tonight: false,
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

    /// 推進時間。`moon_full` 為今夜是否恰逢滿月（ROADMAP 362，由 game.rs 餵入權威月相）。回傳 `SpringsEvent`。
    ///
    /// - `None`：無特殊事件。
    /// - `Some(SpringsEvent::Activated)`：這個 tick 夜晚剛開始，節點已生成（滿月則含月華泉）。
    /// - `Some(SpringsEvent::Deactivated)`：這個 tick 天亮，節點已清除。
    pub fn tick(&mut self, current_phase: Phase, moon_full: bool) -> Option<SpringsEvent> {
        let transition_to_night =
            (self.last_phase == Phase::Dusk || self.last_phase == Phase::Day)
            && current_phase == Phase::Night;
        let transition_to_dawn =
            self.last_phase == Phase::Night && current_phase == Phase::Dawn;

        self.last_phase = current_phase;

        if transition_to_night && !self.active {
            self.active = true;
            self.all_collected_announced = false;
            self.moonlit_tonight = moon_full;
            self.nodes = self.spawn_nodes(moon_full);
            return Some(SpringsEvent::Activated);
        }

        if transition_to_dawn && self.active {
            self.active = false;
            self.moonlit_tonight = false;
            self.nodes.clear();
            return Some(SpringsEvent::Deactivated);
        }

        None
    }

    /// 唯讀檢查：指定節點目前是否「可採集」（夜間、存在、未採、玩家在 COLLECT_REACH 內）。
    /// 不改狀態——供 ROADMAP 350 汲取小遊戲在「開始汲取」時驗格而不立即採走（鎖定時才真採）。
    pub fn can_collect(&self, node_id: u32, px: f32, py: f32) -> bool {
        if !self.active { return false; }
        self.nodes.iter().any(|n| {
            n.id == node_id && !n.collected && {
                let dx = n.wx - px;
                let dy = n.wy - py;
                dx * dx + dy * dy <= COLLECT_REACH * COLLECT_REACH
            }
        })
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

    /// 生成節點：尋常 SPRING_COUNT 口（五方位各一），滿月夜再追加 MOONLIT_SPRING_COUNT 口月華泉
    /// （決定性，可測試）。所有節點 id 取自同一遞增計數器，確保跨泉、跨夜皆不重複。
    fn spawn_nodes(&mut self, moon_full: bool) -> Vec<SpringNode> {
        let mut nodes: Vec<SpringNode> = SPRING_POSITIONS.iter().map(|&(wx, wy)| {
            let id = self.node_counter;
            self.node_counter = self.node_counter.wrapping_add(1);
            SpringNode { id, wx, wy, collected: false, moonlit: false }
        }).collect();
        if moon_full {
            for &(wx, wy) in MOONLIT_POSITIONS.iter() {
                let id = self.node_counter;
                self.node_counter = self.node_counter.wrapping_add(1);
                nodes.push(SpringNode { id, wx, wy, collected: false, moonlit: true });
            }
        }
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
        let ev = s.tick(Phase::Night, false);
        assert_eq!(ev, Some(SpringsEvent::Activated));
        assert!(s.active);
        assert_eq!(s.nodes.len(), SPRING_COUNT);
    }

    #[test]
    fn does_not_reactivate_if_already_active() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, false);
        // 再 tick 一次（仍是 Night → Night）
        let ev = s.tick(Phase::Night, false);
        assert_eq!(ev, None, "已啟動時不應再次觸發");
        assert_eq!(s.nodes.len(), SPRING_COUNT);
    }

    #[test]
    fn deactivates_on_night_to_dawn_transition() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, false);
        let ev = s.tick(Phase::Dawn, false);
        assert_eq!(ev, Some(SpringsEvent::Deactivated));
        assert!(!s.active);
        assert!(s.nodes.is_empty());
    }

    #[test]
    fn no_event_during_steady_night() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, false);
        let ev = s.tick(Phase::Night, false);
        assert_eq!(ev, None);
    }

    #[test]
    fn no_event_during_day() {
        let mut s = NightAetherSprings::new();
        s.tick(Phase::Dawn, false);
        let ev = s.tick(Phase::Day, false);
        assert_eq!(ev, None);
        assert!(!s.active);
    }

    #[test]
    fn nodes_have_unique_ids() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, false);
        let ids: Vec<u32> = s.nodes.iter().map(|n| n.id).collect();
        let mut sorted = ids.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), SPRING_COUNT, "節點 ID 應全部不重複");
    }

    #[test]
    fn ids_differ_across_nights() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, false);
        let ids_first: Vec<u32> = s.nodes.iter().map(|n| n.id).collect();
        s.tick(Phase::Dawn, false);
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, false);
        let ids_second: Vec<u32> = s.nodes.iter().map(|n| n.id).collect();
        for id in &ids_second {
            assert!(!ids_first.contains(id), "第二晚節點 id 不應與第一晚重複");
        }
    }

    #[test]
    fn try_collect_succeeds_in_range() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, false);
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
        s.tick(Phase::Night, false);
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
        s.tick(Phase::Night, false);
        let node = &s.nodes[0];
        let (wx, wy, id) = (node.wx, node.wy, node.id);
        s.try_collect(id, wx, wy);
        let result = s.try_collect(id, wx, wy);
        assert!(!result, "同一節點不能重複採集");
    }

    #[test]
    fn can_collect_readonly_does_not_mutate() {
        // can_collect 只驗格、不採走（ROADMAP 350 開始汲取用）。
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, false);
        let node = &s.nodes[0];
        let (wx, wy, id) = (node.wx, node.wy, node.id);
        assert!(s.can_collect(id, wx + 5.0, wy), "範圍內應可採");
        assert!(s.can_collect(id, wx + 5.0, wy), "唯讀檢查不應改變狀態，仍可採");
        assert!(!s.nodes.iter().find(|n| n.id == id).unwrap().collected, "不應被採走");
        assert!(!s.can_collect(id, wx + 999.0, wy), "超出範圍不可採");
        assert!(!s.can_collect(99999, wx, wy), "不存在的節點不可採");
    }

    #[test]
    fn active_nodes_excludes_collected() {
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, false);
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
        s.tick(Phase::Night, false);
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

    // ─── ROADMAP 362：滿月乙太潮 ─────────────────────────────────────────────

    #[test]
    fn full_moon_spawns_extra_moonlit_springs() {
        // 滿月夜應在 5 口尋常泉之外多生 MOONLIT_SPRING_COUNT 口月華泉。
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        let ev = s.tick(Phase::Night, true);
        assert_eq!(ev, Some(SpringsEvent::Activated));
        assert_eq!(s.nodes.len(), SPRING_COUNT + MOONLIT_SPRING_COUNT);
        assert!(s.moonlit_tonight, "滿月夜 moonlit_tonight 應為真");
        let moonlit = s.nodes.iter().filter(|n| n.moonlit).count();
        assert_eq!(moonlit, MOONLIT_SPRING_COUNT, "恰好 MOONLIT_SPRING_COUNT 口標記為月華泉");
    }

    #[test]
    fn normal_night_has_no_moonlit_springs() {
        // 平夜不生月華泉，且 moonlit_tonight 為否。
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, false);
        assert_eq!(s.nodes.len(), SPRING_COUNT);
        assert!(!s.moonlit_tonight);
        assert!(s.nodes.iter().all(|n| !n.moonlit), "平夜任何泉都不該是月華泉");
    }

    #[test]
    fn moonlit_positions_outside_safe_zone() {
        // 三口月華泉座標皆須落在城鎮保護圈外（鼓勵夜間外出探索）。
        for &(wx, wy) in &MOONLIT_POSITIONS {
            assert!(
                !crate::positions::is_in_safe_zone(wx, wy),
                "月華泉節點 ({wx},{wy}) 不應在城鎮保護圈內"
            );
        }
    }

    #[test]
    fn moonlit_tonight_resets_on_dawn() {
        // 天亮後 moonlit_tonight 應歸零（與節點一併清除）。
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, true);
        assert!(s.moonlit_tonight);
        s.tick(Phase::Dawn, false);
        assert!(!s.moonlit_tonight, "天亮後 moonlit_tonight 應歸零");
        assert!(s.nodes.is_empty());
    }

    #[test]
    fn moonlit_and_normal_ids_all_unique() {
        // 月華泉與尋常泉取自同一遞增計數器，所有 id 須不重複。
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, true);
        let ids: Vec<u32> = s.nodes.iter().map(|n| n.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "月華泉與尋常泉 id 不應重複");
    }

    #[test]
    fn moonlit_spring_is_collectable() {
        // 月華泉採集行為與尋常泉一致（try_collect 在範圍內成功）。
        let mut s = NightAetherSprings::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night, true);
        let node = s.nodes.iter().find(|n| n.moonlit).expect("應有月華泉");
        let (wx, wy, id) = (node.wx, node.wy, node.id);
        assert!(s.try_collect(id, wx + 10.0, wy + 10.0), "月華泉在範圍內應可採集");
        assert!(s.nodes.iter().find(|n| n.id == id).unwrap().collected);
    }
}
