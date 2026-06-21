//! 流星雨系統（ROADMAP 133）。
//!
//! 天文台竣工後，每 30 分鐘觸發一次流星雨，持續 5 分鐘。
//! 流星雨期間地面出現 6 個「星塵採集點」（DustNode），散落於城鎮中心周圍。
//! 玩家走近 80px 以內按採集，可獲得 ItemKind::StarDust×1。
//!
//! 成本紀律：純本機邏輯，**不呼叫任何 LLM**；零 migration，記憶體模式，重啟清零。

use std::collections::HashSet;
use uuid::Uuid;

/// 流星雨觸發間隔（秒）——30 分鐘。
pub const SHOWER_INTERVAL_SECS: f32 = 1800.0;
/// 首次觸發等待（秒）——天文台完工後 3 分鐘才首次觸發。
const FIRST_WAIT_SECS: f32 = 180.0;
/// 流星雨持續時間（秒）——5 分鐘。
pub const SHOWER_DURATION_SECS: f32 = 300.0;
/// 採集有效距離（像素）。
pub const COLLECT_REACH: f32 = 80.0;
/// 每次流星雨的星塵採集點數量。
pub const DUST_NODE_COUNT: usize = 6;

/// 城鎮中心座標（星塵節點分散於此周圍）。
/// 參考 state.rs 公共農地原點 (2200, 2200) + 農地寬高一半約 200px。
const TOWN_CENTER_X: f32 = 2400.0;
const TOWN_CENTER_Y: f32 = 2400.0;

/// 星塵節點偏移量（城鎮中心為圓心，六個固定散佈位置）。
const NODE_OFFSETS: [(f32, f32); DUST_NODE_COUNT] = [
    (-300.0, -200.0),
    (300.0, -250.0),
    (-150.0, 250.0),
    (200.0, 300.0),
    (-350.0, 100.0),
    (250.0, -100.0),
];

/// 一個星塵採集點。
#[derive(Debug, Clone)]
pub struct DustNode {
    pub id: u32,
    pub wx: f32,
    pub wy: f32,
    pub collected: bool,
    /// 是否為彩虹節點——每場流星雨恰好 1 個（ROADMAP 134）。
    pub is_rainbow: bool,
}

/// 流星雨 tick 的結果（給呼叫端決定要不要廣播什麼）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShowerTick {
    /// 本 tick 無事發生。
    None,
    /// 本 tick 新觸發了一場流星雨（用於廣播「流星雨降臨」公告）。
    Started,
    /// 本 tick 一場流星雨剛落幕；`wishes` = 本場累計的不重複許願人數
    ///（用於廣播「N 位旅人一同許下了願望」共願落幕公告；為 0 時呼叫端通常靜默）。
    Ended { wishes: u32 },
}

/// 流星雨狀態（純記憶體，重啟清零）。
pub struct MeteorShowerState {
    /// 距下次觸發的冷卻倒數（秒）。
    pub cooldown: f32,
    /// 活躍倒計時（秒）；0 = 無活躍流星雨。
    pub active_secs: f32,
    /// 當前採集點列表（含已採集的，供狀態判斷）。
    pub dust_nodes: Vec<DustNode>,
    /// 節點 ID 計數器（遞增，確保每輪節點 id 不重複）。
    pub node_counter: u32,
    /// 本場流星雨的許願者集合（ROADMAP 471 共願）。每場開場清空、落幕清空；
    /// 每人每場至多算一次（冪等），純記憶體、重啟清零。
    wishers: HashSet<Uuid>,
}

impl MeteorShowerState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_WAIT_SECS,
            active_secs: 0.0,
            dust_nodes: vec![],
            node_counter: 0,
            wishers: HashSet::new(),
        }
    }

    /// 本場目前的不重複許願人數（ROADMAP 471）。
    pub fn wish_count(&self) -> u32 {
        self.wishers.len() as u32
    }

    /// 嘗試替玩家 `pid` 向當前這場流星雨許願（ROADMAP 471）。
    /// 回傳 `Some(total)` 表示這是本場**新**許願（`total` = 含這次的累計許願人數）；
    /// `None` 表示沒在效（無流星雨）或本場已許過願（冪等、不重複計數）。
    /// 純邏輯、確定性，呼叫端負責全程同一把寫鎖內完成、出鎖後才廣播。
    pub fn make_wish(&mut self, pid: Uuid) -> Option<u32> {
        if !self.is_active() {
            return None;
        }
        if self.wishers.insert(pid) {
            Some(self.wishers.len() as u32)
        } else {
            None
        }
    }

    /// 是否有活躍流星雨。
    pub fn is_active(&self) -> bool {
        self.active_secs > 0.0
    }

    /// 活躍剩餘整數秒（供快照廣播）；無流星雨時回 0。
    pub fn remaining_secs(&self) -> u32 {
        self.active_secs.ceil() as u32
    }

    /// 未被採集的節點（供快照廣播給前端顯示）。
    pub fn active_nodes(&self) -> impl Iterator<Item = &DustNode> {
        self.dust_nodes.iter().filter(|n| !n.collected)
    }

    /// 推進時間（`dt` 秒）。`project_completed` = 天文台是否已完工。
    /// 回傳 `ShowerTick`：`Started` = 本 tick 新觸發一場、`Ended` = 本 tick 一場剛落幕
    ///（附本場共願人數）、`None` = 無事。
    ///（落幕與開場不會在同一 tick 同時發生：開場時冷卻就已重設為 30 分鐘 ≫ 5 分鐘場長。）
    pub fn tick(&mut self, dt: f32, project_completed: bool) -> ShowerTick {
        // 活躍倒計時。
        if self.active_secs > 0.0 {
            self.active_secs -= dt;
            if self.active_secs <= 0.0 {
                self.active_secs = 0.0;
                self.dust_nodes.clear();
                // 本場落幕：先結算共願人數再清空許願者集合，回報給呼叫端廣播。
                let wishes = self.wishers.len() as u32;
                self.wishers.clear();
                return ShowerTick::Ended { wishes };
            }
        }

        // 未完工時不觸發。
        if !project_completed {
            return ShowerTick::None;
        }

        self.cooldown -= dt;
        if self.cooldown > 0.0 {
            return ShowerTick::None;
        }

        // 觸發新一場流星雨（開場清空上一場殘留的許願者，以防萬一）。
        self.cooldown = SHOWER_INTERVAL_SECS;
        self.active_secs = SHOWER_DURATION_SECS;
        self.dust_nodes = self.spawn_nodes();
        self.wishers.clear();
        ShowerTick::Started
    }

    /// 嘗試採集指定節點（驗證距離）。
    /// 回傳 `Some(is_rainbow)` 表示採集成功（true = 彩虹節點），`None` 表示失敗。
    pub fn try_collect(&mut self, node_id: u32, px: f32, py: f32) -> Option<bool> {
        if let Some(node) = self.dust_nodes.iter_mut()
            .find(|n| n.id == node_id && !n.collected)
        {
            let dx = node.wx - px;
            let dy = node.wy - py;
            if dx * dx + dy * dy <= COLLECT_REACH * COLLECT_REACH {
                let is_rainbow = node.is_rainbow;
                node.collected = true;
                return Some(is_rainbow);
            }
        }
        None
    }

    /// 生成 DUST_NODE_COUNT 個節點，散落於城鎮中心周圍。
    /// 每場恰好 1 個彩虹節點，位置由場次計數決定性選取，無偽隨機。
    fn spawn_nodes(&mut self) -> Vec<DustNode> {
        // 用場次計數決定彩虹節點落在哪個 offset 位置（依序輪替）。
        let shower_idx = self.node_counter / DUST_NODE_COUNT as u32;
        let rainbow_idx = (shower_idx % DUST_NODE_COUNT as u32) as usize;
        let nodes = NODE_OFFSETS.iter().enumerate().map(|(i, (dx, dy))| DustNode {
            id: self.node_counter.wrapping_add(i as u32),
            wx: TOWN_CENTER_X + dx,
            wy: TOWN_CENTER_Y + dy,
            collected: false,
            is_rainbow: i == rainbow_idx,
        }).collect();
        self.node_counter = self.node_counter.wrapping_add(DUST_NODE_COUNT as u32);
        nodes
    }
}

impl Default for MeteorShowerState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_not_active() {
        let s = MeteorShowerState::new();
        assert!(!s.is_active());
        assert_eq!(s.remaining_secs(), 0);
    }

    #[test]
    fn does_not_trigger_if_not_completed() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        assert_eq!(s.tick(1.0, false), ShowerTick::None, "未完工時不應觸發流星雨");
    }

    #[test]
    fn does_not_trigger_before_cooldown_expires() {
        let mut s = MeteorShowerState { cooldown: 100.0, ..MeteorShowerState::new() };
        assert_eq!(s.tick(1.0, true), ShowerTick::None, "冷卻未結束時不觸發");
    }

    #[test]
    fn triggers_when_cooldown_expires_and_completed() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        assert_eq!(s.tick(0.1, true), ShowerTick::Started, "冷卻結束且完工後應觸發");
        assert!(s.is_active());
        assert_eq!(s.dust_nodes.len(), DUST_NODE_COUNT);
        assert_eq!(s.remaining_secs(), SHOWER_DURATION_SECS.ceil() as u32);
    }

    #[test]
    fn nodes_have_unique_ids_across_triggers() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        let ids_first: Vec<u32> = s.dust_nodes.iter().map(|n| n.id).collect();
        // 強制結束並再次觸發。
        s.active_secs = 0.0;
        s.dust_nodes.clear();
        s.cooldown = -1.0;
        s.tick(0.1, true);
        let ids_second: Vec<u32> = s.dust_nodes.iter().map(|n| n.id).collect();
        for id in &ids_second {
            assert!(!ids_first.contains(id), "第二輪節點 id 不應與第一輪重複");
        }
    }

    #[test]
    fn try_collect_succeeds_when_in_range() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        let node = &s.dust_nodes[0];
        let (wx, wy, id) = (node.wx, node.wy, node.id);
        let result = s.try_collect(id, wx + 10.0, wy + 10.0);
        assert!(result.is_some(), "在範圍內應成功採集");
        assert!(s.dust_nodes.iter().find(|n| n.id == id).unwrap().collected, "節點應標為已採集");
    }

    #[test]
    fn try_collect_fails_when_out_of_range() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        let node = &s.dust_nodes[0];
        let (wx, wy, id) = (node.wx, node.wy, node.id);
        let result = s.try_collect(id, wx + 200.0, wy + 200.0);
        assert!(result.is_none(), "超出範圍不應成功採集");
    }

    #[test]
    fn exactly_one_rainbow_node_per_shower() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        let rainbow_count = s.dust_nodes.iter().filter(|n| n.is_rainbow).count();
        assert_eq!(rainbow_count, 1, "每場流星雨恰好 1 個彩虹節點");
    }

    #[test]
    fn rainbow_node_cycles_across_showers() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        let mut rainbow_positions: Vec<usize> = vec![];
        for _ in 0..DUST_NODE_COUNT {
            s.tick(0.1, true);
            let pos = s.dust_nodes.iter().position(|n| n.is_rainbow).unwrap();
            rainbow_positions.push(pos);
            // 強制結束並再次觸發。
            s.active_secs = 0.0;
            s.dust_nodes.clear();
            s.cooldown = -1.0;
        }
        // 各場彩虹節點位置應覆蓋 0..DUST_NODE_COUNT 的所有索引（輪替）。
        let mut sorted = rainbow_positions.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), DUST_NODE_COUNT, "彩虹節點應輪替覆蓋所有位置");
    }

    #[test]
    fn active_nodes_excludes_collected() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        let id = s.dust_nodes[0].id;
        let wx = s.dust_nodes[0].wx;
        let wy = s.dust_nodes[0].wy;
        s.try_collect(id, wx, wy);
        let active: Vec<_> = s.active_nodes().collect();
        assert_eq!(active.len(), DUST_NODE_COUNT - 1, "已採集節點不應在 active_nodes 中");
    }

    #[test]
    fn nodes_cleared_when_shower_ends() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        assert_eq!(s.dust_nodes.len(), DUST_NODE_COUNT);
        // 快進超過持續時間。
        s.tick(SHOWER_DURATION_SECS + 1.0, true);
        assert!(!s.is_active());
        assert!(s.dust_nodes.is_empty(), "流星雨結束後節點應清空");
    }

    #[test]
    fn resets_cooldown_after_trigger() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        assert!(s.cooldown > 0.0, "觸發後冷卻應重設為 SHOWER_INTERVAL_SECS");
    }

    // ── ROADMAP 471 共願 ─────────────────────────────────────────────────────

    #[test]
    fn wish_fails_when_no_shower() {
        let mut s = MeteorShowerState::new();
        assert!(!s.is_active());
        assert_eq!(s.make_wish(Uuid::from_u128(1)), None, "無流星雨時許願應失敗");
        assert_eq!(s.wish_count(), 0);
    }

    #[test]
    fn wish_succeeds_during_shower_and_counts() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        assert_eq!(s.make_wish(Uuid::from_u128(1)), Some(1), "首位許願者計 1");
        assert_eq!(s.make_wish(Uuid::from_u128(2)), Some(2), "第二位許願者計 2");
        assert_eq!(s.wish_count(), 2);
    }

    #[test]
    fn wish_is_idempotent_per_player_per_shower() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        assert_eq!(s.make_wish(Uuid::from_u128(7)), Some(1));
        assert_eq!(s.make_wish(Uuid::from_u128(7)), None, "同一人本場重複許願不再計數");
        assert_eq!(s.wish_count(), 1, "重複許願不應膨脹人數");
    }

    #[test]
    fn shower_end_reports_wish_count_then_clears() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        s.make_wish(Uuid::from_u128(1));
        s.make_wish(Uuid::from_u128(2));
        s.make_wish(Uuid::from_u128(3));
        // 快進超過場長 → 落幕應回報 3 人共願。
        let ended = s.tick(SHOWER_DURATION_SECS + 1.0, true);
        assert_eq!(ended, ShowerTick::Ended { wishes: 3 }, "落幕應回報本場共願人數");
        assert_eq!(s.wish_count(), 0, "落幕後許願者集合應清空");
    }

    #[test]
    fn new_shower_resets_wishers() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        s.make_wish(Uuid::from_u128(1));
        // 結束本場。
        s.tick(SHOWER_DURATION_SECS + 1.0, true);
        // 觸發新一場。
        s.cooldown = -1.0;
        assert_eq!(s.tick(0.1, true), ShowerTick::Started);
        assert_eq!(s.wish_count(), 0, "新一場流星雨許願者應歸零");
        // 上一場已許願的人在新一場可再次許願。
        assert_eq!(s.make_wish(Uuid::from_u128(1)), Some(1), "新場同一人可再許願");
    }

    #[test]
    fn wish_count_zero_at_shower_end_when_nobody_wished() {
        let mut s = MeteorShowerState { cooldown: -1.0, ..MeteorShowerState::new() };
        s.tick(0.1, true);
        let ended = s.tick(SHOWER_DURATION_SECS + 1.0, true);
        assert_eq!(ended, ShowerTick::Ended { wishes: 0 }, "無人許願時落幕回報 0");
    }
}
