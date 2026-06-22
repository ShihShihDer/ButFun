//! ROADMAP 495「今日世界戰報」——廣場石板第一次有了「全服今天做了什麼」。
//!
//! 自伺服器啟動（或下次重置）起，累計四個全伺服器行動數：採集次數、收穫次數、擊殺次數、
//! 今日登入人次。每次有玩家做了對應動作，就在這裡 +1。資料純存記憶體、伺服器重啟歸零——
//! 不需要持久化，重置就是「新的一天又從零開始」的戲感。
//!
//! 純邏輯（無 I/O、無亂數、確定可測）；前端透過 Snapshot `world_tally` 欄位讀取。

use crate::protocol::WorldTallyView;

/// 全伺服器今日行動計數器。每個 `record_*` 方法在對應事件觸發後呼叫。
#[derive(Debug, Default)]
pub struct WorldTally {
    /// 採集成功次數（撿木頭、撿礦石等）。
    gathers: u64,
    /// 農地收穫次數（乙太田或任何 FarmOutcome::Harvested）。
    harvests: u64,
    /// 擊殺敵人次數（任何 kill 事件）。
    kills: u64,
    /// 今日登入/連線人次（每次新 WebSocket 連上且帶帳號 token）。
    players_today: u64,
}

impl WorldTally {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_gather(&mut self) {
        self.gathers = self.gathers.saturating_add(1);
    }

    pub fn record_harvest(&mut self) {
        self.harvests = self.harvests.saturating_add(1);
    }

    pub fn record_kill(&mut self) {
        self.kills = self.kills.saturating_add(1);
    }

    pub fn record_player_login(&mut self) {
        self.players_today = self.players_today.saturating_add(1);
    }

    pub fn view(&self) -> WorldTallyView {
        WorldTallyView {
            gathers: self.gathers,
            harvests: self.harvests,
            kills: self.kills,
            players_today: self.players_today,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_zero() {
        let t = WorldTally::new();
        let v = t.view();
        assert_eq!(v.gathers, 0);
        assert_eq!(v.harvests, 0);
        assert_eq!(v.kills, 0);
        assert_eq!(v.players_today, 0);
    }

    #[test]
    fn record_gather_increments() {
        let mut t = WorldTally::new();
        t.record_gather();
        t.record_gather();
        assert_eq!(t.view().gathers, 2);
    }

    #[test]
    fn record_harvest_increments() {
        let mut t = WorldTally::new();
        t.record_harvest();
        assert_eq!(t.view().harvests, 1);
    }

    #[test]
    fn record_kill_increments() {
        let mut t = WorldTally::new();
        t.record_kill();
        t.record_kill();
        t.record_kill();
        assert_eq!(t.view().kills, 3);
    }

    #[test]
    fn record_player_login_increments() {
        let mut t = WorldTally::new();
        t.record_player_login();
        assert_eq!(t.view().players_today, 1);
    }

    #[test]
    fn counters_are_independent() {
        let mut t = WorldTally::new();
        t.record_gather();
        t.record_harvest();
        t.record_kill();
        t.record_player_login();
        let v = t.view();
        assert_eq!(v.gathers, 1);
        assert_eq!(v.harvests, 1);
        assert_eq!(v.kills, 1);
        assert_eq!(v.players_today, 1);
    }

    #[test]
    fn saturating_add_does_not_overflow() {
        let mut t = WorldTally::new();
        t.gathers = u64::MAX;
        t.record_gather();
        assert_eq!(t.view().gathers, u64::MAX);
    }

    #[test]
    fn view_reflects_all_fields() {
        let mut t = WorldTally::new();
        for _ in 0..5 { t.record_gather(); }
        for _ in 0..3 { t.record_harvest(); }
        for _ in 0..7 { t.record_kill(); }
        for _ in 0..2 { t.record_player_login(); }
        let v = t.view();
        assert_eq!(v.gathers, 5);
        assert_eq!(v.harvests, 3);
        assert_eq!(v.kills, 7);
        assert_eq!(v.players_today, 2);
    }

    #[test]
    fn default_same_as_new() {
        let d: WorldTally = WorldTally::default();
        let n = WorldTally::new();
        assert_eq!(d.view().gathers, n.view().gathers);
        assert_eq!(d.view().kills, n.view().kills);
    }
}
