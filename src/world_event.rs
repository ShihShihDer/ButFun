//! 動態世界事件：宇宙裂縫（ROADMAP 26）。
//!
//! 每隔固定時間，在故鄉星球某個預設位置開啟宇宙裂縫，湧現裂縫守護者。
//! 開啟時透過聊天頻道廣播位置，玩家可衝去獵殺守護者取得裂縫碎片。

/// 宇宙裂縫的出現間隔（秒）。約 3 分鐘一次。
pub const RIFT_INTERVAL_SECS: f32 = 180.0;
/// 宇宙裂縫的存在時長（秒）。開啟後 2 分鐘消失（守護者被殺後自然重生計時超長，無礙）。
pub const RIFT_DURATION_SECS: f32 = 120.0;

/// 裂縫可出現的固定座標列表（全在故鄉新手村安全區外）。
/// 每次事件循環選下一個，讓玩家熟悉方向但不完全可預測。
pub const RIFT_LOCATIONS: &[(f32, f32)] = &[
    (3400.0, 1400.0), // 新手村東北方
    (1000.0, 3300.0), // 新手村西南方
    (3900.0, 3600.0), // 新手村東南方
];

/// 宇宙裂縫事件的世界狀態。由 `AppState` 持有並由遊戲迴圈每 tick 推進。
#[derive(Debug, Clone)]
pub struct WorldEvent {
    /// 距下次裂縫觸發的倒數（秒）。降到 0 時觸發新裂縫。
    timer: f32,
    /// 目前裂縫是否開啟中。
    pub active: bool,
    /// 裂縫位置 X（世界座標）。
    pub x: f32,
    /// 裂縫位置 Y（世界座標）。
    pub y: f32,
    /// 裂縫剩餘持續時間（秒）。降到 0 時關閉裂縫。
    pub remaining: f32,
    /// 循環索引：決定下一次裂縫出現在哪個位置。
    location_idx: usize,
}

impl WorldEvent {
    pub fn new() -> Self {
        Self {
            timer: RIFT_INTERVAL_SECS,
            active: false,
            x: 0.0,
            y: 0.0,
            remaining: 0.0,
            location_idx: 0,
        }
    }

    /// 推進事件計時器。回傳 `Some((x, y))` 表示這個 tick 觸發了新裂縫、需要在該座標注入守護者。
    pub fn tick(&mut self, dt: f32) -> Option<(f32, f32)> {
        if self.active {
            self.remaining -= dt;
            if self.remaining <= 0.0 {
                self.active = false;
                self.timer = RIFT_INTERVAL_SECS;
            }
            None
        } else {
            self.timer -= dt;
            if self.timer <= 0.0 {
                let (lx, ly) = RIFT_LOCATIONS[self.location_idx % RIFT_LOCATIONS.len()];
                self.location_idx += 1;
                self.active = true;
                self.x = lx;
                self.y = ly;
                self.remaining = RIFT_DURATION_SECS;
                self.timer = RIFT_INTERVAL_SECS;
                Some((lx, ly))
            } else {
                None
            }
        }
    }

    /// 回傳當前裂縫的快照視圖（若開啟中）。供協定層廣播給前端。
    pub fn view(&self) -> Option<WorldEventView> {
        if self.active {
            Some(WorldEventView {
                x: self.x,
                y: self.y,
                remaining_secs: self.remaining,
            })
        } else {
            None
        }
    }
}

impl Default for WorldEvent {
    fn default() -> Self {
        Self::new()
    }
}

/// 快照裡的世界事件視圖（供前端顯示裂縫位置 + 剩餘時間）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorldEventView {
    /// 裂縫中心 X（世界座標）。
    pub x: f32,
    /// 裂縫中心 Y（世界座標）。
    pub y: f32,
    /// 裂縫剩餘持續秒數（前端倒計時用）。
    pub remaining_secs: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 事件初始為未啟動() {
        let e = WorldEvent::new();
        assert!(!e.active);
        assert!(e.view().is_none());
    }

    #[test]
    fn 計時到達後觸發裂縫() {
        let mut e = WorldEvent::new();
        // 還差 1 秒不觸發
        let r = e.tick(RIFT_INTERVAL_SECS - 1.0);
        assert!(r.is_none());
        assert!(!e.active);
        // 再過 1 秒觸發
        let r = e.tick(1.0);
        assert!(r.is_some());
        assert!(e.active);
    }

    #[test]
    fn 裂縫持續後關閉() {
        let mut e = WorldEvent::new();
        e.tick(RIFT_INTERVAL_SECS); // 觸發
        assert!(e.active);
        e.tick(RIFT_DURATION_SECS); // 持續時間耗盡
        assert!(!e.active);
        assert!(e.view().is_none());
    }

    #[test]
    fn 位置循環選取() {
        let mut e = WorldEvent::new();
        // 第一次裂縫
        let r1 = e.tick(RIFT_INTERVAL_SECS).unwrap();
        assert_eq!(r1, RIFT_LOCATIONS[0]);
        // 關閉再觸發第二次
        e.tick(RIFT_DURATION_SECS);
        let r2 = e.tick(RIFT_INTERVAL_SECS).unwrap();
        assert_eq!(r2, RIFT_LOCATIONS[1]);
    }

    #[test]
    fn view_回傳剩餘秒數() {
        let mut e = WorldEvent::new();
        e.tick(RIFT_INTERVAL_SECS);
        let v = e.view().unwrap();
        assert!((v.remaining_secs - RIFT_DURATION_SECS).abs() < 0.1);
    }
}
