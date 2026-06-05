//! 騎乘登記表（Phase 1-E 蒸汽載具 MVP 的純邏輯地基之三）。
//!
//! `vehicle.rs` 解了「上車後車怎麼開」、`vehicle_field.rs` 解了「載具停哪、走近上哪一台」，
//! 但兩層都刻意把「**這台有沒有人騎、是誰在騎**」往外推給一張登記表——正如
//! `plots.rs`（幾何）與 `plot_registry.rs`（歸屬）把「地塊在哪」與「誰擁有」分屬兩層。
//! 這層就是那張表：玩家 id ↔ 載具序號的雙向對應，外加「一台車只准一個人騎」的權威判斷。
//!
//! 接線時的用法（標 `allow(dead_code)` 在此之前不刪）：
//!   - 玩家按上下車鍵 → ws 用 `vehicle_field::nearest_within_reach` 找到最近那台序號 →
//!     `board(user, index)`；回 `false`（那台已被別人騎）就不讓上、給前端回饋。
//!   - 上車後玩家的方向輸入導向 `vehicle_field::step_ridden(vehicle_of(user)?, ..)`。
//!   - 玩家按下車鍵、或**斷線 `cleanup` 時**呼叫 `disembark(user)` 放掉那台，
//!     讓它回到「沒人騎」可被下一個人上的狀態（斷線一定要叫，否則車被鬼佔住）。
//!
//! ## 與 `plot_registry.rs` 的關鍵差異：可釋放 vs 只增不減
//! 地塊歸屬是**持久、只增不減**的（離開仍保有自己的地，序號不回收）；騎乘卻是
//! **暫時、可釋放**的 session 狀態——下車或斷線就把車讓出來給別人騎。故這裡沒有
//! 「序號單調遞增」那套，改維護一條雙向不變式：
//!   - 一台載具至多一個騎士（`board` 對別人已騎的車回 `false`）。
//!   - 一個騎士至多騎一台載具（`board` 上新車前自動讓出舊車）。
//!
//! 兩張 `HashMap`（`by_user`／`by_vehicle`）在每個變動點同步維護，恆為彼此的反向。
//!
//! ## 不持久化（比照 `connections.rs`，而非 `plot_registry.rs`）
//! 騎乘是**連線存活期間**的 live 狀態：伺服器重啟時沒有任何連線存活，自然沒有人在騎，
//! 全部載具回到空閒（`vehicle_field` 仍從存檔還原它們**停放的位置**，只是都沒人騎）。
//! 故本層刻意**沒有 `from_saved` 載入入口**——沒有要跨重啟還原的東西，比照同為連線期
//! 暫態的 `connections.rs`（也無持久化）。這也避免無謂發明一條對不上語意的持久化路徑。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

/// 記錄「誰在騎哪一台載具」的雙向對應。MVP：記憶體（騎乘是連線期暫態，無需持久化）。
#[derive(Clone, Default)]
pub struct RideRegistry {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// 玩家 id -> 他正在騎的載具序號（每人至多一筆）。
    by_user: HashMap<Uuid, usize>,
    /// 載具序號 -> 騎它的玩家 id（每台至多一筆）。恆為 `by_user` 的反向。
    by_vehicle: HashMap<usize, Uuid>,
}

impl RideRegistry {
    #[allow(dead_code)] // 接線輪在 `AppState` 建立 store 才有呼叫端；沿用本專案前置地基的慣例。
    pub fn new() -> Self {
        Self::default()
    }

    /// `user` 嘗試上第 `index` 台載具。回 `true` 代表上車成功（之後 `vehicle_of` 會回該序號）。
    ///
    /// 權威判斷：
    ///   - 那台已被**別人**騎 → 回 `false`，不更動任何狀態（一台車只准一個人騎）。
    ///   - 那台已被**自己**騎（重複送上車意圖）→ 回 `true`，idempotent（不重複佔位）。
    ///   - 那台空著 → 成功；若該玩家原本騎著別台，先自動讓出舊車（一人至多騎一台），
    ///     再把自己記到新車上。
    #[allow(dead_code)] // 接線輪（ws 上車動作）才有呼叫端；沿用本專案前置地基的慣例。
    pub fn board(&self, user: Uuid, index: usize) -> bool {
        let mut inner = self.inner.lock().unwrap();
        // 那台已有騎士：是自己＝idempotent 成功；是別人＝拒絕、不動任何狀態。
        if let Some(&rider) = inner.by_vehicle.get(&index) {
            return rider == user;
        }
        // 那台空著。該玩家若原本騎別台，先讓出舊車（維持「一人至多一台」不變式）。
        if let Some(prev) = inner.by_user.insert(user, index) {
            inner.by_vehicle.remove(&prev);
        }
        inner.by_vehicle.insert(index, user);
        true
    }

    /// `user` 下車：放掉他正在騎的載具，回傳被釋放的載具序號（沒在騎則回 `None`）。
    /// 斷線 `cleanup` 也要呼叫，否則該玩家的車會被鬼佔住、沒人能再上。
    #[allow(dead_code)] // 接線輪（ws 下車／斷線清理）才有呼叫端；沿用本專案前置地基的慣例。
    pub fn disembark(&self, user: Uuid) -> Option<usize> {
        let mut inner = self.inner.lock().unwrap();
        let index = inner.by_user.remove(&user)?;
        inner.by_vehicle.remove(&index);
        Some(index)
    }

    /// `user` 目前在騎的載具序號（沒在騎回 `None`）——接線層據此把方向輸入導向那台。
    #[allow(dead_code)] // 接線輪（遊戲迴圈推進有人騎的載具、前端畫「我在騎」）才有呼叫端。
    pub fn vehicle_of(&self, user: Uuid) -> Option<usize> {
        self.inner.lock().unwrap().by_user.get(&user).copied()
    }

    /// 第 `index` 台載具的騎士 id（沒人騎回 `None`）。
    #[allow(dead_code)] // 接線輪（前端標示「這台被誰騎」、伺服器判斷）才有呼叫端。
    pub fn rider_of(&self, index: usize) -> Option<Uuid> {
        self.inner.lock().unwrap().by_vehicle.get(&index).copied()
    }

    /// 第 `index` 台載具是否已有人騎（`nearest_within_reach` 選到後，接線層先問這個再決定讓不讓上）。
    #[allow(dead_code)] // 接線輪才有呼叫端；沿用本專案前置地基的慣例。
    pub fn is_ridden(&self, index: usize) -> bool {
        self.inner.lock().unwrap().by_vehicle.contains_key(&index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空著的車可上，上車後雙向對應都建立。
    #[test]
    fn board_free_vehicle_succeeds() {
        let reg = RideRegistry::new();
        let user = Uuid::new_v4();
        assert!(reg.board(user, 0));
        assert_eq!(reg.vehicle_of(user), Some(0));
        assert_eq!(reg.rider_of(0), Some(user));
        assert!(reg.is_ridden(0));
    }

    /// 招牌不變式：一台車只准一個人騎——別人已騎著就拒絕，且不動既有騎士。
    #[test]
    fn board_taken_vehicle_rejects_other_user() {
        let reg = RideRegistry::new();
        let owner = Uuid::new_v4();
        let intruder = Uuid::new_v4();
        assert!(reg.board(owner, 2));
        assert!(!reg.board(intruder, 2), "別人已騎這台，不該讓上");
        // 既有騎士不被頂掉、闖入者也沒被記上。
        assert_eq!(reg.rider_of(2), Some(owner));
        assert_eq!(reg.vehicle_of(intruder), None);
    }

    /// 同一玩家重複送上車同一台＝idempotent（不報錯、不重複佔位）。
    #[test]
    fn board_same_user_same_vehicle_is_idempotent() {
        let reg = RideRegistry::new();
        let user = Uuid::new_v4();
        assert!(reg.board(user, 1));
        assert!(reg.board(user, 1), "重複上同一台應仍成功");
        assert_eq!(reg.vehicle_of(user), Some(1));
        assert_eq!(reg.rider_of(1), Some(user));
    }

    /// 一人至多騎一台：上新車自動讓出舊車（舊車回到沒人騎）。
    #[test]
    fn board_new_vehicle_releases_previous() {
        let reg = RideRegistry::new();
        let user = Uuid::new_v4();
        assert!(reg.board(user, 0));
        assert!(reg.board(user, 1), "換騎另一台應成功");
        assert_eq!(reg.vehicle_of(user), Some(1), "現在騎的是新車");
        assert!(!reg.is_ridden(0), "舊車已讓出、沒人騎");
        assert_eq!(reg.rider_of(0), None);
        assert_eq!(reg.rider_of(1), Some(user));
    }

    /// 下車放掉載具，回傳被釋放的序號；之後雙向對應都清掉、車可被別人上。
    #[test]
    fn disembark_frees_vehicle() {
        let reg = RideRegistry::new();
        let user = Uuid::new_v4();
        reg.board(user, 3);
        assert_eq!(reg.disembark(user), Some(3));
        assert_eq!(reg.vehicle_of(user), None);
        assert!(!reg.is_ridden(3));
        // 讓出後，別的玩家上得了同一台。
        let other = Uuid::new_v4();
        assert!(reg.board(other, 3));
        assert_eq!(reg.rider_of(3), Some(other));
    }

    /// 沒在騎的人下車是 no-op，回 `None`（斷線清理對沒上車的玩家呼叫也安全）。
    #[test]
    fn disembark_without_riding_is_none() {
        let reg = RideRegistry::new();
        assert_eq!(reg.disembark(Uuid::new_v4()), None);
    }

    /// 多名玩家各騎各的車，互不干擾。
    #[test]
    fn riders_are_independent() {
        let reg = RideRegistry::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert!(reg.board(a, 0));
        assert!(reg.board(b, 1));
        assert_eq!(reg.vehicle_of(a), Some(0));
        assert_eq!(reg.vehicle_of(b), Some(1));
        // a 下車不影響 b。
        reg.disembark(a);
        assert_eq!(reg.vehicle_of(b), Some(1));
        assert!(reg.is_ridden(1));
        assert!(!reg.is_ridden(0));
    }

    /// 未被任何人騎的載具：`rider_of` 為 `None`、`is_ridden` 為 `false`。
    #[test]
    fn empty_vehicle_has_no_rider() {
        let reg = RideRegistry::new();
        assert_eq!(reg.rider_of(0), None);
        assert!(!reg.is_ridden(0));
        // 上一台後，別台仍空。
        reg.board(Uuid::new_v4(), 0);
        assert!(!reg.is_ridden(1));
    }
}
