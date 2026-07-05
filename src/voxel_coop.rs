//! 乙太方界·雞舍生蛋 v1——世界第一種「動物產物」資源節點（自主提案切片）。
//!
//! **玩家有感**：乙太方界至今所有可採資源都來自「植物／礦物」——樹木、莊稼、礦脈——
//! 從沒有一種資源來自「飼養動物」。本刀補上這條至今完全空白的資源軸：玩家蓋一座雞舍，
//! 靜候一段時間雞舍會「生出」一顆蛋，破壞收下蛋、雞舍就地空出繼續孵下一顆——像莓果叢
//! （806）一樣可反覆利用、不必重蓋，讓「養一座雞舍」成為基地裡一處會持續回饋的角落。
//!
//! **取得**：工作台合成——木頭(5)×4 + 葉片(6)×2 → 雞舍(80)（木架撐頂、葉片鋪成溫暖的窩）。
//!
//! **狀態機**（方塊 id 對齊後端 `Block`）：
//!   雞舍(COOP_ID=80，空)  放置在世界任意合法位置
//!     →[COOP_LAY_SECS 後 tick_coop]→  雞舍(有蛋)(COOP_READY_ID=81)
//!   收蛋：Break 雞舍(有蛋)(81) → 蛋(EGG_ID=82)×1 ＋ **就地回退成雞舍(空)(80)** ＋ 重啟計時（可反覆）。
//!   拆除：Break 雞舍(空)(80)（尚未生蛋） → 退還雞舍(80)×1（自身），並清掉計時記錄。
//!
//! **純邏輯層**：`CoopStore`（記錄放置的雞舍座標＋時間）＋生蛋計時純函式，
//! 確定性、無副作用、全可測。鎖 / WS / IO 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。
//!
//! **成本鐵律**：零 LLM、零 migration、零協議破壞（走既有 place/break/inv_update 管線）。
//! `CoopStore` **純記憶體**（與莓果叢 store 行為一致：重啟後未生蛋的雞舍重置計時，比照
//! `voxel_berry::BerryStore` 既有精神，session-only 不额外持久化）。

use std::collections::HashMap;

/// 雞舍（空）方塊 id——`Block::Coop`。既是背包可持有的物品（合成而得、可放置），
/// 也是放置後在世界裡尚未生蛋的狀態（item_id == block_id，比照莓果叢苗 75 的慣例）。
pub const COOP_ID: u8 = 80;

/// 雞舍（有蛋）方塊 id——`Block::CoopReady`。伺服器維護的狀態方塊（玩家不能手動放置），
/// 由 `tick_coop` 從空雞舍長成，收蛋後回退成空雞舍。
pub const COOP_READY_ID: u8 = 81;

/// 蛋物品 id（純 inventory 物品，無對應可放置方塊；收下有蛋的雞舍時掉落）。
pub const EGG_ID: u8 = 82;

/// 一次收成的蛋產量。
pub const EGG_YIELD: u32 = 1;

/// 雞舍從「空／剛收蛋」到「生出蛋」所需秒數。比莓果叢（100s）快，蛋的產出節奏更緊湊，
/// 像養一窩會持續下蛋的雞，而非慢慢結果的果樹。
pub const COOP_LAY_SECS: u64 = 70;

/// 一座放置（或剛收蛋回退）的雞舍記錄。
#[derive(Clone, Debug, PartialEq)]
pub struct Coop {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 放置/回退計時起算的 Unix 秒數（用來判斷是否生蛋）。
    pub placed_secs: u64,
}

/// 雞舍 store（純記憶體，重啟後未生蛋的雞舍重置計時；已生蛋的雞舍是 delta 方塊會持久）。
#[derive(Default)]
pub struct CoopStore {
    coops: HashMap<(i32, i32, i32), Coop>,
}

impl CoopStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 放置（或收蛋後回退重新計時）一座雞舍：記錄座標 + placed_secs。
    /// 重複登記同格 → 覆蓋（重置計時）。
    pub fn plant(&mut self, x: i32, y: i32, z: i32, now_secs: u64) -> Coop {
        let c = Coop { x, y, z, placed_secs: now_secs };
        self.coops.insert((x, y, z), c.clone());
        c
    }

    /// 移除雞舍記錄（被拆掉／生蛋後從計時清單移除，等收蛋再重新登記）。
    pub fn remove(&mut self, x: i32, y: i32, z: i32) {
        self.coops.remove(&(x, y, z));
    }

    /// 此座標是否有雞舍計時記錄。
    pub fn has(&self, x: i32, y: i32, z: i32) -> bool {
        self.coops.contains_key(&(x, y, z))
    }

    /// 回傳所有已生蛋（放置滿 [`COOP_LAY_SECS`]）的雞舍座標。
    pub fn ready_coops(&self, now_secs: u64) -> Vec<(i32, i32, i32)> {
        self.coops
            .iter()
            .filter(|(_, c)| now_secs >= c.placed_secs.saturating_add(COOP_LAY_SECS))
            .map(|(&coord, _)| coord)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct_and_stable() {
        // 三個 id 互異、且落在既有 id 空間之外（0..=79 皆已用），80 是首個空號。
        assert_eq!(COOP_ID, 80);
        assert_eq!(COOP_READY_ID, 81);
        assert_eq!(EGG_ID, 82);
        assert_ne!(COOP_ID, COOP_READY_ID);
        assert_ne!(COOP_READY_ID, EGG_ID);
    }

    #[test]
    fn plant_then_has() {
        let mut s = CoopStore::new();
        assert!(!s.has(1, 5, 2));
        s.plant(1, 5, 2, 1000);
        assert!(s.has(1, 5, 2));
    }

    #[test]
    fn remove_clears_record() {
        let mut s = CoopStore::new();
        s.plant(1, 5, 2, 1000);
        s.remove(1, 5, 2);
        assert!(!s.has(1, 5, 2), "移除後不應再有記錄");
    }

    #[test]
    fn replant_resets_timer() {
        // 同格重複登記 → 覆蓋計時（收蛋回退時就是這樣重啟計時）。
        let mut s = CoopStore::new();
        s.plant(0, 5, 0, 1000);
        assert!(s.ready_coops(1000 + COOP_LAY_SECS - 1).is_empty());
        assert_eq!(s.ready_coops(1000 + COOP_LAY_SECS), vec![(0, 5, 0)]);
        // 在滿秒數時收蛋 → 回退重新登記（now = 剛好滿秒數那刻）
        s.plant(0, 5, 0, 1000 + COOP_LAY_SECS);
        assert!(
            s.ready_coops(1000 + COOP_LAY_SECS * 2 - 1).is_empty(),
            "回退後重置計時、需再等一輪"
        );
        assert_eq!(s.ready_coops(1000 + COOP_LAY_SECS * 2), vec![(0, 5, 0)]);
    }

    #[test]
    fn ready_at_exact_boundary() {
        let mut s = CoopStore::new();
        s.plant(3, 5, 7, 0);
        assert!(s.ready_coops(COOP_LAY_SECS - 1).is_empty(), "差一秒不熟");
        assert_eq!(
            s.ready_coops(COOP_LAY_SECS),
            vec![(3, 5, 7)],
            "剛好滿秒數即生蛋"
        );
    }

    #[test]
    fn only_ready_coops_returned() {
        let mut s = CoopStore::new();
        s.plant(0, 5, 0, 0); // 0 秒放
        s.plant(1, 5, 0, 50); // 50 秒放
        // now = COOP_LAY_SECS：第一座滿了（熟），第二座只過 (COOP_LAY_SECS - 50) 秒（未熟）
        let ready = s.ready_coops(COOP_LAY_SECS);
        assert_eq!(ready, vec![(0, 5, 0)]);
    }

    #[test]
    fn saturating_add_never_overflows() {
        // placed_secs 極大時 saturating_add 飽和到 u64::MAX、不 panic：
        // 只要 now 還沒到盡頭，門檻是 MAX、必未生蛋（確定性、無溢位 panic）。
        let mut s = CoopStore::new();
        s.plant(0, 5, 0, u64::MAX);
        assert!(
            s.ready_coops(u64::MAX - 1).is_empty(),
            "起算點已在盡頭，尚未到 MAX 時不得生蛋、且不 panic"
        );
    }
}
