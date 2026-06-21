//! 夜螢提燈（ROADMAP 477）。
//!
//! 背景：建議箱裡多位居民反覆反映「夜間只能等待與觀察、缺乏主動可做的小互動」；
//! 而 dev2review 也點名「自我表達」是全專案最少開發的維度。本切片把這兩個缺口接成一條：
//! 入夜後城外草間浮現一群群螢火蟲，玩家走近輕輕一捕即收進隨身提燈，提燈裡的螢火越多、
//! 身邊投下的柔光越亮——而這圈暖光**全服其他玩家都看得見**，成為夜裡一張認得出主人的名片。
//! 天亮時螢火飛回草間（提燈清空），是一夜一更新的溫和儀式，不囤積、不換錢、零經濟水龍頭。
//!
//! 設計取向（鏡像 ROADMAP 470 放風箏已獲准的範式）：這是「給被動氛圍現象（夜）第一個玩家動詞」
//! ＋「玩家可控、旁人可見的自我表達狀態」，純表現／社交、不回血、不嚇敵、不掉落、不給乙太，
//! 與近期 462/467/472 的「靠近回血」骨架、474 篝火的「嚇退野獸」骨架、戰利品掉落骨架皆不同。
//!
//! 設計紀律：
//! - 純記憶體模式，重啟清零。零 migration，零 LLM，零持久化，零經濟。
//! - 純邏輯可獨立測試，不依賴 WebSocket / 遊戲迴圈。
//! - 螢群座標固定（決定性，無偽隨機）——伺服器重啟後位置可重現。
//! - 玩家感知：夜幕降臨時全服廣播一句「螢火浮現」，夜間探索多一件溫柔的事可做。

use crate::daynight::Phase;

/// 捕螢有效距離（像素）。站進這個半徑、按互動鍵即可捕走一隻。
pub const CATCH_REACH: f32 = 70.0;
/// 每夜浮現的螢群數量。
pub const SWARM_COUNT: usize = 6;
/// 每個螢群可捕的螢火數（捕完即散）。
pub const SWARM_SIZE: u8 = 4;
/// 單人提燈最多裝幾隻螢火（封頂，超過不再增加；避免無上限堆數值）。
pub const LANTERN_MAX: u8 = 12;

/// 6 個螢群的絕對世界座標（皆經測試確認落在城鎮保護圈之外、且與夜間乙太泉座標不重疊，
/// 鼓勵玩家夜間外出、也讓兩種夜間活動散在不同方位）。
const SWARM_POSITIONS: [(f32, f32); SWARM_COUNT] = [
    (1200.0, 600.0),   // 城北偏西（y=600 < 安全圈，外）
    (3400.0, 600.0),   // 城北偏東（y 外）
    (1200.0, 3900.0),  // 城南偏西（y=3900 > 安全圈，外）
    (3400.0, 3900.0),  // 城南偏東（y 外）
    (600.0, 1500.0),   // 城西（x=600 < 安全圈，外）
    (4000.0, 2800.0),  // 城東（x=4000 > 安全圈，外）
];

/// 一個螢群節點。
#[derive(Debug, Clone)]
pub struct FireflySwarm {
    /// 螢群唯一 ID（用於前端送 CatchFirefly）。
    pub id: u32,
    /// 世界座標 X。
    pub wx: f32,
    /// 世界座標 Y。
    pub wy: f32,
    /// 還剩幾隻可捕（捕一隻 -1，歸 0 即散去）。
    pub remaining: u8,
}

/// 夜螢提燈系統狀態（純記憶體，重啟清零）。
pub struct FireflyLantern {
    /// 目前是否有活躍的螢群（夜晚期間）。
    pub active: bool,
    /// 當前螢群列表。
    pub swarms: Vec<FireflySwarm>,
    /// 上一個 tick 的日夜階段（用於偵測 Dusk→Night / Night→Dawn 轉換）。
    last_phase: Phase,
    /// 螢群 ID 計數器（遞增，跨夜不重複）。
    swarm_counter: u32,
}

impl FireflyLantern {
    pub fn new() -> Self {
        Self {
            active: false,
            swarms: vec![],
            last_phase: Phase::Day,
            swarm_counter: 0,
        }
    }

    /// 尚有螢火可捕的螢群（供快照廣播；捕光的不再廣播）。
    pub fn active_swarms(&self) -> impl Iterator<Item = &FireflySwarm> {
        self.swarms.iter().filter(|s| s.remaining > 0)
    }

    /// 推進時間。回傳 `FireflyEvent`：
    /// - `None`：無特殊事件。
    /// - `Some(Activated)`：這個 tick 夜晚剛開始，螢群已生成。
    /// - `Some(Deactivated)`：這個 tick 天亮，螢群已清除（呼叫端應一併清空各玩家提燈）。
    pub fn tick(&mut self, current_phase: Phase) -> Option<FireflyEvent> {
        let to_night = (self.last_phase == Phase::Dusk || self.last_phase == Phase::Day)
            && current_phase == Phase::Night;
        let to_dawn = self.last_phase == Phase::Night && current_phase == Phase::Dawn;

        self.last_phase = current_phase;

        if to_night && !self.active {
            self.active = true;
            self.swarms = self.spawn_swarms();
            return Some(FireflyEvent::Activated);
        }
        if to_dawn && self.active {
            self.active = false;
            self.swarms.clear();
            return Some(FireflyEvent::Deactivated);
        }
        None
    }

    /// 唯讀檢查：指定螢群目前是否可捕（夜間、存在、還有剩、玩家在 CATCH_REACH 內）。不改狀態。
    pub fn can_catch(&self, swarm_id: u32, px: f32, py: f32) -> bool {
        self.active
            && self.swarms.iter().any(|s| {
                s.id == swarm_id && s.remaining > 0 && within(s.wx, s.wy, px, py)
            })
    }

    /// 嘗試從指定螢群捕一隻螢火。玩家需在 CATCH_REACH 內、螢群尚有剩。
    /// 回傳 `true` 表示捕到一隻（呼叫端負責加進該玩家提燈、並夾在 LANTERN_MAX）。
    pub fn try_catch(&mut self, swarm_id: u32, px: f32, py: f32) -> bool {
        if !self.active {
            return false;
        }
        if let Some(s) = self
            .swarms
            .iter_mut()
            .find(|s| s.id == swarm_id && s.remaining > 0)
        {
            if within(s.wx, s.wy, px, py) {
                s.remaining -= 1;
                return true;
            }
        }
        false
    }

    /// 生成螢群（六方位各一，決定性可測試）。所有 id 取自同一遞增計數器，跨夜不重複。
    fn spawn_swarms(&mut self) -> Vec<FireflySwarm> {
        SWARM_POSITIONS
            .iter()
            .map(|&(wx, wy)| {
                let id = self.swarm_counter;
                self.swarm_counter = self.swarm_counter.wrapping_add(1);
                FireflySwarm {
                    id,
                    wx,
                    wy,
                    remaining: SWARM_SIZE,
                }
            })
            .collect()
    }
}

impl Default for FireflyLantern {
    fn default() -> Self {
        Self::new()
    }
}

/// 把一隻新捕的螢火加進提燈，封頂在 LANTERN_MAX。純函式，供 ws / 測試共用同一口徑。
pub fn add_to_lantern(current: u8, caught: bool) -> u8 {
    if caught {
        (current + 1).min(LANTERN_MAX)
    } else {
        current
    }
}

/// `tick()` 回傳的事件。
#[derive(Debug, Clone, PartialEq)]
pub enum FireflyEvent {
    /// 黃昏／白天轉夜晚：螢群已生成，廣播通知全服。
    Activated,
    /// 夜晚轉黎明：螢群消失，呼叫端應一併清空各玩家提燈。
    Deactivated,
}

/// 兩點是否落在捕螢半徑內（平方比較，免開根號）。
fn within(ax: f32, ay: f32, bx: f32, by: f32) -> bool {
    let dx = ax - bx;
    let dy = ay - by;
    dx * dx + dy * dy <= CATCH_REACH * CATCH_REACH
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_not_active() {
        let s = FireflyLantern::new();
        assert!(!s.active);
        assert!(s.swarms.is_empty());
    }

    #[test]
    fn activates_on_dusk_to_night() {
        let mut s = FireflyLantern::new();
        s.last_phase = Phase::Dusk;
        let ev = s.tick(Phase::Night);
        assert_eq!(ev, Some(FireflyEvent::Activated));
        assert!(s.active);
        assert_eq!(s.swarms.len(), SWARM_COUNT);
        assert!(s.swarms.iter().all(|sw| sw.remaining == SWARM_SIZE));
    }

    #[test]
    fn does_not_reactivate_when_active() {
        let mut s = FireflyLantern::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let ev = s.tick(Phase::Night);
        assert_eq!(ev, None, "已啟動不應再次觸發");
        assert_eq!(s.swarms.len(), SWARM_COUNT);
    }

    #[test]
    fn deactivates_on_night_to_dawn() {
        let mut s = FireflyLantern::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let ev = s.tick(Phase::Dawn);
        assert_eq!(ev, Some(FireflyEvent::Deactivated));
        assert!(!s.active);
        assert!(s.swarms.is_empty());
    }

    #[test]
    fn no_event_during_day() {
        let mut s = FireflyLantern::new();
        s.tick(Phase::Dawn);
        let ev = s.tick(Phase::Day);
        assert_eq!(ev, None);
        assert!(!s.active);
    }

    #[test]
    fn swarms_have_unique_ids() {
        let mut s = FireflyLantern::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let mut ids: Vec<u32> = s.swarms.iter().map(|sw| sw.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SWARM_COUNT);
    }

    #[test]
    fn ids_differ_across_nights() {
        let mut s = FireflyLantern::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let first: Vec<u32> = s.swarms.iter().map(|sw| sw.id).collect();
        s.tick(Phase::Dawn);
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        for sw in &s.swarms {
            assert!(!first.contains(&sw.id), "隔夜 id 不應重複");
        }
    }

    #[test]
    fn catch_decrements_and_succeeds_in_range() {
        let mut s = FireflyLantern::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let sw = &s.swarms[0];
        let (wx, wy, id) = (sw.wx, sw.wy, sw.id);
        assert!(s.try_catch(id, wx + 10.0, wy - 10.0));
        assert_eq!(
            s.swarms.iter().find(|x| x.id == id).unwrap().remaining,
            SWARM_SIZE - 1
        );
    }

    #[test]
    fn catch_fails_out_of_range() {
        let mut s = FireflyLantern::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let sw = &s.swarms[0];
        let (wx, wy, id) = (sw.wx, sw.wy, sw.id);
        assert!(!s.try_catch(id, wx + 500.0, wy));
        assert_eq!(
            s.swarms.iter().find(|x| x.id == id).unwrap().remaining,
            SWARM_SIZE,
            "超距不應消耗螢火"
        );
    }

    #[test]
    fn catch_fails_when_not_active() {
        let mut s = FireflyLantern::new();
        assert!(!s.try_catch(0, 1200.0, 600.0));
    }

    #[test]
    fn swarm_depletes_after_swarm_size_catches() {
        let mut s = FireflyLantern::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let sw = &s.swarms[0];
        let (wx, wy, id) = (sw.wx, sw.wy, sw.id);
        for _ in 0..SWARM_SIZE {
            assert!(s.try_catch(id, wx, wy));
        }
        // 捕光後再捕無效，且不再列入 active_swarms。
        assert!(!s.try_catch(id, wx, wy), "捕光的螢群不可再捕");
        assert!(
            s.active_swarms().all(|x| x.id != id),
            "捕光的螢群不應再廣播"
        );
        assert_eq!(s.active_swarms().count(), SWARM_COUNT - 1);
    }

    #[test]
    fn can_catch_is_readonly() {
        let mut s = FireflyLantern::new();
        s.last_phase = Phase::Dusk;
        s.tick(Phase::Night);
        let sw = &s.swarms[0];
        let (wx, wy, id) = (sw.wx, sw.wy, sw.id);
        assert!(s.can_catch(id, wx + 5.0, wy));
        assert!(s.can_catch(id, wx + 5.0, wy), "唯讀檢查不改狀態");
        assert_eq!(
            s.swarms.iter().find(|x| x.id == id).unwrap().remaining,
            SWARM_SIZE
        );
        assert!(!s.can_catch(id, wx + 999.0, wy), "超距不可捕");
        assert!(!s.can_catch(99999, wx, wy), "不存在的螢群不可捕");
    }

    #[test]
    fn lantern_add_caps_at_max() {
        let mut n = 0u8;
        for _ in 0..(LANTERN_MAX as usize + 5) {
            n = add_to_lantern(n, true);
        }
        assert_eq!(n, LANTERN_MAX, "提燈封頂在 LANTERN_MAX");
    }

    #[test]
    fn lantern_add_noop_when_not_caught() {
        assert_eq!(add_to_lantern(3, false), 3, "沒捕到不應增加");
        assert_eq!(add_to_lantern(0, true), 1);
    }

    #[test]
    fn swarms_outside_safe_zone() {
        for &(wx, wy) in &SWARM_POSITIONS {
            assert!(
                !crate::positions::is_in_safe_zone(wx, wy),
                "螢群 ({wx},{wy}) 不應在城鎮保護圈內（鼓勵夜間外出）"
            );
        }
    }

    #[test]
    fn swarm_positions_are_distinct() {
        // 六個螢群座標互不重複（散在不同方位）。
        let mut seen: Vec<(f32, f32)> = vec![];
        for &p in &SWARM_POSITIONS {
            assert!(!seen.contains(&p), "螢群座標不應自我重複");
            seen.push(p);
        }
    }
}
