//! 乙太方界·莓果叢 v1——多年生、可反覆採收的莓果叢（自主提案切片 ROADMAP 806）。
//!
//! **玩家有感**：乙太方界此前三種作物（小麥/胡蘿蔔/馬鈴薯）都是「一次性」的——
//! 種下→長成→採收後農田土就空了，要再吃得重種一輪。莓果叢是世界第一種**多年生**作物：
//! 種下一叢後會**反覆結果**——結果的莓果叢採收後不會消失，而是回退成未結果的莓果叢，
//! 靜候一段時間又結出新的一批莓果，可年復一年地採，不必重種。這補上療癒農業循環裡
//! 「一勞永逸的果園」那一味，讓玩家蓋起一片自己的莓園、走過去順手摘一把。
//!
//! **取得**：背包 2×2 合成——樹苗(65) + 種子(14)×2 → 莓果叢苗(75)（把樹苗與種子育成一叢
//! 會結果的灌木），兩種材料都是既有可再生資源（砍葉得樹苗/種子），無新掉落 RNG、無世界生成改動。
//!
//! **狀態機**（方塊 id 對齊後端 `Block`）：
//!   莓果叢苗(BUSH_UNRIPE_ID=75)  種在土地上（草/土/沙/雪/農田土之上）
//!     →[BERRY_GROW_SECS 後 tick_berry]→  結果的莓果叢(BUSH_RIPE_ID=76)
//!   採收：Break 結果的莓果叢(76) → 莓果(BERRY_ID=77)×2 ＋ **就地回退成莓果叢苗(75)** ＋ 重啟計時（多年生）。
//!   挖除：Break 莓果叢苗(75)（未結果） → 退還莓果叢苗(75)×1（自身），並清掉計時記錄。
//!
//! **純邏輯層**：`BerryStore`（記錄種下的莓果叢＋時間）＋成熟計時純函式，
//! 確定性、無副作用、全可測。鎖 / WS / IO 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。
//!
//! **成本鐵律**：零 LLM、零 migration、零協議破壞（走既有 place/break/inv_update 管線）。
//! `BerryStore` **純記憶體**（與 grove store 行為一致：重啟後未結果的莓果叢重置計時；
//! 已結果的莓果叢是 delta 方塊會持久、採收時重新登記計時，故最有價值的「結果」狀態不受重啟影響）。

use std::collections::HashMap;

/// 莓果叢苗（未結果）方塊 id——`Block::BerryBush`。既是背包可持有的物品（合成而得、可放置），
/// 也是種下後在世界裡的未結果灌木方塊（item_id == block_id，比照樹苗 65 的慣例）。
pub const BUSH_UNRIPE_ID: u8 = 75;

/// 結果的莓果叢方塊 id——`Block::BerryBushRipe`。伺服器維護的狀態方塊（玩家不能手動放置），
/// 由 `tick_berry` 從莓果叢苗長成，採收後回退成莓果叢苗。
pub const BUSH_RIPE_ID: u8 = 76;

/// 莓果物品 id（純 inventory 物品，無對應可放置方塊；採收結果的莓果叢時掉落）。
/// 一叢一次結 [`BERRY_YIELD`] 顆，是可反覆採收的療癒收穫，也可餽贈居民。
pub const BERRY_ID: u8 = 77;

/// 一叢結果一次的莓果產量。
pub const BERRY_YIELD: u32 = 2;

/// 莓果醬物品 id（純 inventory 物品，無對應可放置方塊；莓果醬 v1 ROADMAP 808）。
/// 把採到的莓果(77)放進熔爐小火慢熬而成——乙太方界第一種**甜點**熟食：
/// 不是填肚子的正餐，而是甜滋滋的療癒小食，玩家可自己享用（走 `voxel_meal::is_edible_dish`）
/// 或餽贈居民（走 `voxel_gift::is_food_gift`，居民對甜食格外雀躍）。
pub const JAM_ID: u8 = 78;

/// 莓果叢從「苗/剛採收」長到「結果」所需秒數（~100 秒）。
/// 比最快的胡蘿蔔(60s)慢、比最慢的馬鈴薯(120s)快——多年生的回報是「不必重種」，
/// 故單輪結果時間取中庸，不必最快。回退後同樣以這個秒數重新結果。
pub const BERRY_GROW_SECS: u64 = 100;

/// 一叢種下（或剛採收回退）的莓果叢記錄。
#[derive(Clone, Debug, PartialEq)]
pub struct Bush {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 種下/回退計時起算的 Unix 秒數（用來判斷是否結果）。
    pub planted_secs: u64,
}

/// 莓果叢 store（純記憶體，重啟後未結果的苗重置計時；已結果的叢是 delta 方塊會持久）。
#[derive(Default)]
pub struct BerryStore {
    bushes: HashMap<(i32, i32, i32), Bush>,
}

impl BerryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 種下（或採收後回退重新計時）一叢莓果叢：記錄座標 + planted_secs。
    /// 重複登記同格 → 覆蓋（重置計時）。
    pub fn plant(&mut self, x: i32, y: i32, z: i32, now_secs: u64) -> Bush {
        let b = Bush { x, y, z, planted_secs: now_secs };
        self.bushes.insert((x, y, z), b.clone());
        b
    }

    /// 移除莓果叢記錄（被挖掉 / 結果後從計時清單移除，等採收再重新登記）。
    pub fn remove(&mut self, x: i32, y: i32, z: i32) {
        self.bushes.remove(&(x, y, z));
    }

    /// 此座標是否有莓果叢計時記錄。
    pub fn has(&self, x: i32, y: i32, z: i32) -> bool {
        self.bushes.contains_key(&(x, y, z))
    }

    /// 回傳所有已結果（種下滿 [`BERRY_GROW_SECS`]）的莓果叢座標。
    pub fn ripe_bushes(&self, now_secs: u64) -> Vec<(i32, i32, i32)> {
        self.bushes
            .iter()
            .filter(|(_, b)| now_secs >= b.planted_secs.saturating_add(BERRY_GROW_SECS))
            .map(|(&coord, _)| coord)
            .collect()
    }
}

/// 莓果叢可種在哪些「土地」方塊之上（草/土/沙/雪/農田土），與樹苗一致。
/// 傳入的是「莓果叢腳下那格」的方塊 id。純函式，供 `voxel_ws` 種植前驗證。
pub fn is_plantable_ground(block_id: u8) -> bool {
    matches!(
        block_id,
        1  // Grass 草地
        | 2  // Dirt 泥土
        | 4  // Sand 沙地
        | 11 // FarmSoil 農田土
        | 55 // Snow 雪原地表
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct_and_stable() {
        // 三個 id 互異、且落在既有 id 空間之外（0..=74 皆已用），不撞號。
        assert_eq!(BUSH_UNRIPE_ID, 75);
        assert_eq!(BUSH_RIPE_ID, 76);
        assert_eq!(BERRY_ID, 77);
        assert_ne!(BUSH_UNRIPE_ID, BUSH_RIPE_ID);
        assert_ne!(BUSH_RIPE_ID, BERRY_ID);
    }

    #[test]
    fn plant_then_has() {
        let mut s = BerryStore::new();
        assert!(!s.has(1, 5, 2));
        s.plant(1, 5, 2, 1000);
        assert!(s.has(1, 5, 2));
    }

    #[test]
    fn remove_clears_record() {
        let mut s = BerryStore::new();
        s.plant(1, 5, 2, 1000);
        s.remove(1, 5, 2);
        assert!(!s.has(1, 5, 2), "移除後不應再有記錄");
    }

    #[test]
    fn replant_resets_timer() {
        // 同格重複登記 → 覆蓋計時（採收回退時就是這樣重啟計時）。
        let mut s = BerryStore::new();
        s.plant(0, 5, 0, 1000);
        // 1000 + 100 = 1100 之前不熟
        assert!(s.ripe_bushes(1099).is_empty());
        // 到 1100 熟
        assert_eq!(s.ripe_bushes(1100), vec![(0, 5, 0)]);
        // 在 1100 採收 → 回退重新登記（now=1100）
        s.plant(0, 5, 0, 1100);
        // 1100 + 100 = 1200 之前又不熟了（多年生：採收後要再等一輪）
        assert!(s.ripe_bushes(1199).is_empty(), "回退後重置計時、需再等一輪");
        assert_eq!(s.ripe_bushes(1200), vec![(0, 5, 0)]);
    }

    #[test]
    fn ripe_at_exact_boundary() {
        let mut s = BerryStore::new();
        s.plant(3, 5, 7, 0);
        assert!(s.ripe_bushes(BERRY_GROW_SECS - 1).is_empty(), "差一秒不熟");
        assert_eq!(
            s.ripe_bushes(BERRY_GROW_SECS),
            vec![(3, 5, 7)],
            "剛好滿秒數即結果"
        );
    }

    #[test]
    fn only_ripe_bushes_returned() {
        let mut s = BerryStore::new();
        s.plant(0, 5, 0, 0); // 0 秒種
        s.plant(1, 5, 0, 50); // 50 秒種
        // now = 100：第一叢滿 100 秒（熟），第二叢只 50 秒（未熟）
        let ripe = s.ripe_bushes(100);
        assert_eq!(ripe, vec![(0, 5, 0)]);
    }

    #[test]
    fn plantable_ground_matches_grove_lands() {
        // 草/土/沙/農田土/雪可種；石/木/水/空氣不可。
        assert!(is_plantable_ground(1)); // Grass
        assert!(is_plantable_ground(2)); // Dirt
        assert!(is_plantable_ground(4)); // Sand
        assert!(is_plantable_ground(11)); // FarmSoil
        assert!(is_plantable_ground(55)); // Snow
        assert!(!is_plantable_ground(3)); // Stone
        assert!(!is_plantable_ground(5)); // Wood
        assert!(!is_plantable_ground(7)); // Water
        assert!(!is_plantable_ground(0)); // Air
    }

    #[test]
    fn saturating_add_never_overflows() {
        // planted_secs 極大時 saturating_add 飽和到 u64::MAX、不 panic：
        // 只要 now 還沒到盡頭，門檻是 MAX、必未熟（確定性、無溢位 panic）。
        let mut s = BerryStore::new();
        s.plant(0, 5, 0, u64::MAX);
        assert!(
            s.ripe_bushes(u64::MAX - 1).is_empty(),
            "起算點已在盡頭，尚未到 MAX 時不得結果、且不 panic"
        );
    }
}
