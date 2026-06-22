//! 戰鬥記跡（ROADMAP 499）——擊敗敵人的地點留下短暫可見的記號。
//!
//! 玩家擊倒敵人後，在戰場位置留下一個「記跡」（⚔️ 符號），其他旅人走近
//! 時會浮現「XX 擊倒了 YY，N 分前」的提示文字，讓路過的人知道附近有人打過仗。
//!
//! 設計取捨：
//! - 純記憶體，重啟清零，零持久化，零 migration。
//! - 每個記跡存活 `MARK_LIFESPAN_SECS` 秒（5 分鐘）後自動消失。
//! - 全局上限 `MAX_MARKS`（20 條），超出時丟棄最舊的，永保最新鮮的記跡。
//! - 純邏輯層，無 IO，可完整單元測試。

use crate::protocol::CombatMarkView;

/// 記跡存活上限（秒）。5 分鐘——夠讓後來的旅人看到、又不會永久汙染地圖。
pub const MARK_LIFESPAN_SECS: f32 = 300.0;

/// 全局記跡上限。超出時丟棄最舊的（新鮮記跡優先）。
pub const MAX_MARKS: usize = 20;

/// 單一戰鬥記跡。
#[derive(Debug, Clone)]
pub struct CombatMark {
    /// 擊殺發生的世界座標 X（像素）。
    pub wx: f32,
    /// 擊殺發生的世界座標 Y（像素）。
    pub wy: f32,
    /// 擊殺者的顯示名稱（截為 12 字以防過長）。
    pub killer: String,
    /// 被擊殺怪物的中文名稱（來自 `EnemyKind::display_name`）。
    pub enemy_name: &'static str,
    /// 此記跡已存在的秒數（由 `tick` 累積），到達 `MARK_LIFESPAN_SECS` 後失效。
    pub age_secs: f32,
}

/// 全局戰鬥記跡集合（純記憶體，重啟清零）。
pub struct CombatMarkState {
    marks: Vec<CombatMark>,
}

impl CombatMarkState {
    pub fn new() -> Self {
        Self { marks: Vec::new() }
    }

    /// 新增一筆記跡，同時清理過期的並守住 `MAX_MARKS` 上限。
    /// `wx`/`wy` 是敵人被擊倒的世界座標，`killer` 是擊殺者名稱，
    /// `enemy_name` 是怪物中文名（靜態字串，來自 `EnemyKind::display_name`）。
    pub fn add(&mut self, wx: f32, wy: f32, killer: &str, enemy_name: &'static str) {
        // 先清掉已過期的，避免帶著過期記跡超出上限。
        self.marks.retain(|m| m.age_secs < MARK_LIFESPAN_SECS);
        // 名稱截 12 字，防止超長玩家名破版面。
        let killer_short: String = killer.chars().take(12).collect();
        // 新記跡插到最前（index 0 = 最新），超出上限就截掉尾部（最舊）。
        self.marks.insert(0, CombatMark {
            wx,
            wy,
            killer: killer_short,
            enemy_name,
            age_secs: 0.0,
        });
        if self.marks.len() > MAX_MARKS {
            self.marks.truncate(MAX_MARKS);
        }
    }

    /// 推進所有記跡的年齡一幀，並移除已過期的。`dt` 非正有限值時提前退回。
    pub fn tick(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        for m in &mut self.marks {
            m.age_secs += dt;
        }
        self.marks.retain(|m| m.age_secs < MARK_LIFESPAN_SECS);
    }

    /// 傳回目前所有活躍記跡的廣播快照（用於 WorldState 廣播）。
    pub fn view(&self) -> Vec<CombatMarkView> {
        self.marks
            .iter()
            .map(|m| CombatMarkView {
                wx: m.wx,
                wy: m.wy,
                killer: m.killer.clone(),
                enemy_name: m.enemy_name.to_string(),
                age_secs: m.age_secs.max(0.0) as u32,
            })
            .collect()
    }

    /// 目前活躍記跡數量（用於測試）。
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.marks.len()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 單元測試
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(state: &mut CombatMarkState, wx: f32, wy: f32) {
        state.add(wx, wy, "旅人甲", "銹蝕巡邏機");
    }

    #[test]
    fn add_single_mark_and_view() {
        let mut s = CombatMarkState::new();
        mk(&mut s, 100.0, 200.0);
        let v = s.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].wx, 100.0);
        assert_eq!(v[0].wy, 200.0);
        assert_eq!(v[0].killer, "旅人甲");
        assert_eq!(v[0].enemy_name, "銹蝕巡邏機");
        assert_eq!(v[0].age_secs, 0);
    }

    #[test]
    fn tick_ages_marks_and_expires_them() {
        let mut s = CombatMarkState::new();
        mk(&mut s, 0.0, 0.0);
        // 推進到快要過期但還沒到
        s.tick(MARK_LIFESPAN_SECS - 1.0);
        assert_eq!(s.len(), 1, "距過期 1 秒仍應存在");
        // 再推一秒讓它過期
        s.tick(1.5);
        assert_eq!(s.len(), 0, "超過存活期後應清除");
    }

    #[test]
    fn newest_mark_is_at_front() {
        let mut s = CombatMarkState::new();
        s.add(1.0, 1.0, "A", "銹蝕巡邏機");
        s.add(2.0, 2.0, "B", "迷途乙太靈");
        // 第二筆（最新）應在 index 0
        let v = s.view();
        assert_eq!(v[0].killer, "B", "最新記跡應排最前");
        assert_eq!(v[1].killer, "A");
    }

    #[test]
    fn cap_at_max_marks_drops_oldest() {
        let mut s = CombatMarkState::new();
        // 添加比上限多一筆
        for i in 0..=(MAX_MARKS as u32) {
            s.add(i as f32, 0.0, "X", "銹蝕巡邏機");
        }
        assert_eq!(s.len(), MAX_MARKS, "不可超出 MAX_MARKS");
    }

    #[test]
    fn killer_name_truncated_at_12_chars() {
        let mut s = CombatMarkState::new();
        let long = "A".repeat(20); // 20 個字
        s.add(0.0, 0.0, &long, "銹蝕巡邏機");
        let v = s.view();
        // 截 12 字（每個字 1 char）
        assert_eq!(v[0].killer.chars().count(), 12);
    }

    #[test]
    fn tick_bad_dt_is_noop() {
        let mut s = CombatMarkState::new();
        mk(&mut s, 0.0, 0.0);
        s.tick(0.0);
        s.tick(-1.0);
        s.tick(f32::NAN);
        s.tick(f32::INFINITY);
        // 壞值不推進年齡，記跡不消失
        assert_eq!(s.len(), 1, "壞 dt 不應修改狀態");
    }

    #[test]
    fn view_age_secs_reflects_elapsed() {
        let mut s = CombatMarkState::new();
        mk(&mut s, 0.0, 0.0);
        s.tick(65.0);
        let v = s.view();
        assert_eq!(v[0].age_secs, 65, "age_secs 應反映 tick 累積");
    }

    #[test]
    fn add_after_partial_expiry_removes_old_first() {
        let mut s = CombatMarkState::new();
        // 先加一筆，讓它快到期
        s.add(0.0, 0.0, "舊", "飄舞精靈");
        s.tick(MARK_LIFESPAN_SECS - 0.1);
        // 再加新的
        s.add(1.0, 1.0, "新", "晶石傀儡");
        s.tick(0.2); // 讓舊的過期
        // 舊的應消失，只剩新的
        let v = s.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].killer, "新");
    }
}
