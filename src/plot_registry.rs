//! 地塊歸屬登記（Phase 0-G-O1「農地改 per-player 擁有」的純邏輯地基，第二塊）。
//!
//! `plots.rs` 解了幾何問題——「第 N 塊地擺在世界的哪裡」。但 per-player 還缺另一半：
//! 「**哪個玩家擁有第幾塊地**」。這層只管這件事：玩家進場時分配一個尚未使用的地塊序號、
//! 之後重連拿回同一塊（同一玩家永遠是同一塊地），並提供「這塊地是不是你的」的歸屬判斷
//! ——正是接線時伺服器 Farm 動作要驗的「這塊地屬於你」。
//!
//! 設計取捨：
//!   - **序號單調遞增、只增不減**：第一個進場的玩家拿序號 0（`plots::plot_origin(0)` 正好
//!     對齊現有全域農地），之後每個新玩家拿下一個未用序號。離開的玩家保留其序號（不回收），
//!     貼合「先來的在家園核心、地圖往外長」與 O2「序號只增不減地往外排」的方向；也避免回收
//!     造成「同一塊地換人耕種、作物歸屬錯亂」。代價是從不回訪的玩家會占住序號，屬可接受
//!     （量級＝歷來玩家數，與 `positions.rs` 只記已登入玩家、不無界成長同理）。
//!   - **記憶體版**：連同 `positions.rs`／`field.rs` 的農地狀態，跨重啟持久化一律留待 0-E
//!     （那時把這張 user_id → 序號 表也存進 Postgres，returning 玩家重啟後仍拿回原地塊）。
//!   - 比照 `connections.rs`／`positions.rs` 做成可測的小 store；分配與歸屬判斷都是無 IO 的
//!     純粹方法，便於單元測試。
//!
//! 接線輪（標 `allow(dead_code)` 在此之前不刪）會把它接上：玩家進場 `assign` 取得序號 →
//! `plots::plot_origin(序號)` 決定他那塊 `Field` 的 origin；ws 的 `Farm` 動作先用 `owns`
//! 驗證「玩家操作的地塊序號屬於他自己」才放行，路過別人的地看得到、點不動。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

/// 記錄每個玩家 id 擁有的地塊序號。MVP：記憶體（跨重啟持久化留待 0-E）。
#[derive(Clone, Default)]
pub struct PlotRegistry {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// user_id -> 地塊序號（餵給 `plots::plot_origin`）。
    by_user: HashMap<Uuid, usize>,
    /// 下一個要發出去的未用序號（單調遞增、只增不減）。
    next: usize,
}

impl PlotRegistry {
    #[allow(dead_code)] // 接線輪在 `AppState` 建立 store 才有呼叫端；沿用本專案前置地基的慣例。
    pub fn new() -> Self {
        Self::default()
    }

    /// 取得 `user_id` 的地塊序號；還沒有就分配下一個未用序號並記住。
    /// 同一玩家重複呼叫（重連／多分頁）一律拿回**同一個**序號，不會多吃地塊。
    #[allow(dead_code)] // 接線輪（玩家進場分配地塊）才有呼叫端；沿用本專案前置地基的慣例。
    pub fn assign(&self, user_id: Uuid) -> usize {
        let mut inner = self.inner.lock().unwrap();
        if let Some(&idx) = inner.by_user.get(&user_id) {
            return idx;
        }
        let idx = inner.next;
        inner.next += 1;
        inner.by_user.insert(user_id, idx);
        idx
    }

    /// `user_id` 目前擁有的地塊序號；還沒分配過回 `None`。
    #[allow(dead_code)] // 同上，待接線（前端畫「我的地」、鏡頭定位用）。
    pub fn index_of(&self, user_id: Uuid) -> Option<usize> {
        self.inner.lock().unwrap().by_user.get(&user_id).copied()
    }

    /// 第 `index` 塊地是不是 `user_id` 擁有的——伺服器 Farm 動作驗地主用。
    /// 玩家還沒分配地塊（`index_of` 為 `None`）時對任何序號都回 `false`。
    #[allow(dead_code)] // 同上，待接線（ws `Farm` 動作驗「這塊地屬於你」）。
    pub fn owns(&self, user_id: Uuid, index: usize) -> bool {
        self.index_of(user_id) == Some(index)
    }

    /// 從持久化的 `(user_id, 序號)` 對重建登記表——0-E 跨重啟載入入口。
    ///
    /// **關鍵不變式**：`next` 一律重建成「已用最大序號 + 1」。若天真載入把 `next`
    /// 設回 0，重啟後 `assign` 會把序號 0（或任何已發出的序號）再發給新玩家，
    /// 造成「同一塊地兩個地主、作物歸屬錯亂」——正是本模組「序號只增不減、不回收」
    /// 要防的災難。比照 `positions::spawn_at`／`field::from_tiles`／`daynight::at`
    /// 那條「每個存檔又重載的結構都在載入路徑驗證自身不變式」的硬化弧線。
    ///
    /// 重複的 `user_id` 取後見者（`HashMap` 覆蓋語意）；重複的**序號**不在這層擋
    /// （持久化端以 UNIQUE 保證唯一），這裡只負責 `next` 絕不回頭。空輸入＝全新登記表
    /// （`next` 為 0，第一個玩家仍拿序號 0、對齊現有全域農地）。
    #[allow(dead_code)] // 接 0-E 從 Postgres 載回時才有呼叫端；沿用本專案載入入口的前置慣例。
    pub fn from_saved(saved: impl IntoIterator<Item = (Uuid, usize)>) -> Self {
        let mut by_user = HashMap::new();
        let mut next = 0usize;
        for (user, index) in saved {
            // 已用最大序號 + 1：確保續發的序號永遠落在所有既有地塊之後，不重疊。
            next = next.max(index + 1);
            by_user.insert(user, index);
        }
        Self {
            inner: Arc::new(Mutex::new(Inner { by_user, next })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 第一個玩家拿序號 0（接線時對齊現有全域農地），後續玩家依序往外拿。
    #[test]
    fn assigns_sequential_indices_from_zero() {
        let reg = PlotRegistry::new();
        assert_eq!(reg.assign(Uuid::new_v4()), 0);
        assert_eq!(reg.assign(Uuid::new_v4()), 1);
        assert_eq!(reg.assign(Uuid::new_v4()), 2);
    }

    /// 同一玩家重複 `assign`（重連／多分頁）永遠拿回同一塊地，不會多吃序號。
    #[test]
    fn same_user_keeps_same_plot() {
        let reg = PlotRegistry::new();
        let id = Uuid::new_v4();
        let first = reg.assign(id);
        assert_eq!(reg.assign(id), first, "同玩家重連應拿回同一塊地");
        assert_eq!(reg.assign(id), first);
        // 中間插入別的玩家不影響原玩家的序號。
        let other = reg.assign(Uuid::new_v4());
        assert_ne!(other, first, "不同玩家不該分到同一塊");
        assert_eq!(reg.assign(id), first, "插入他人後原玩家仍是同一塊");
    }

    /// 不同玩家分到互異的序號（每人一塊、不重疊；對齊 `plots` 的互異保證）。
    #[test]
    fn distinct_users_get_distinct_indices() {
        let reg = PlotRegistry::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let idx = reg.assign(Uuid::new_v4());
            assert!(seen.insert(idx), "序號 {idx} 重複分配給了兩個玩家");
        }
    }

    /// 序號只增不減：玩家離開不回收，新玩家拿更後面的序號（不會把離開者的地塊讓出去）。
    /// （store 沒有移除入口，這裡以「持續分配、序號持續增長」鎖住此契約。）
    #[test]
    fn indices_only_grow() {
        let reg = PlotRegistry::new();
        let a = reg.assign(Uuid::new_v4());
        let b = reg.assign(Uuid::new_v4());
        let c = reg.assign(Uuid::new_v4());
        assert!(a < b && b < c, "序號應單調遞增：{a} < {b} < {c}");
    }

    /// `index_of`：分配前回 `None`，分配後回該序號。
    #[test]
    fn index_of_reflects_assignment() {
        let reg = PlotRegistry::new();
        let id = Uuid::new_v4();
        assert_eq!(reg.index_of(id), None, "分配前不該有地塊");
        let idx = reg.assign(id);
        assert_eq!(reg.index_of(id), Some(idx));
    }

    /// `owns`：只有自己分到的那塊回 `true`；別塊、未分配一律 `false`（驗地主的招牌契約）。
    #[test]
    fn owns_only_own_plot() {
        let reg = PlotRegistry::new();
        let owner = Uuid::new_v4();
        let stranger = Uuid::new_v4();
        let mine = reg.assign(owner);
        let theirs = reg.assign(stranger);

        assert!(reg.owns(owner, mine), "地主對自己的地該回 true");
        assert!(!reg.owns(owner, theirs), "不能聲稱擁有別人的地");
        assert!(!reg.owns(owner, 999), "不存在 / 沒分配的序號一律 false");

        // 從未分配地塊的玩家對任何序號都不算擁有。
        let nobody = Uuid::new_v4();
        assert!(!reg.owns(nobody, mine));
        assert!(!reg.owns(nobody, 0));
    }

    /// 空輸入＝全新登記表：`next` 為 0，第一個玩家仍拿序號 0（對齊現有全域農地）。
    #[test]
    fn from_saved_empty_behaves_like_new() {
        let reg = PlotRegistry::from_saved(std::iter::empty());
        assert_eq!(reg.assign(Uuid::new_v4()), 0);
    }

    /// 載回後保留每個玩家原本的地塊（returning 玩家重啟仍拿回同一塊）。
    #[test]
    fn from_saved_preserves_owners() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let reg = PlotRegistry::from_saved([(a, 0), (b, 3)]);
        assert_eq!(reg.index_of(a), Some(0));
        assert_eq!(reg.index_of(b), Some(3));
        assert!(reg.owns(b, 3));
        // 重連也拿回原序號、不重新分配。
        assert_eq!(reg.assign(a), 0);
    }

    /// 招牌不變式：續發的序號＝已用最大序號 + 1，**絕不**把已發出的序號再給新玩家。
    /// （天真載入把 `next` 設回 0 會讓新玩家撞上序號 0 的既有地主——本測試鎖死此災難不復現。）
    #[test]
    fn from_saved_resumes_after_max_index() {
        let owner0 = Uuid::new_v4();
        // 稀疏／跳號的歷史：最大序號是 10，續發必須從 11 起，而非填補 1..=9 的空洞。
        let reg = PlotRegistry::from_saved([(owner0, 0), (Uuid::new_v4(), 10)]);
        let fresh = reg.assign(Uuid::new_v4());
        assert_eq!(fresh, 11, "續發序號應為最大序號+1，不得回頭撞既有地塊");
        assert_ne!(fresh, 0);
        // 序號 0 的既有地主未被頂掉。
        assert!(reg.owns(owner0, 0), "載回後既有地主仍擁有自己的地");
        assert_eq!(reg.index_of(owner0), Some(0));
    }
}
