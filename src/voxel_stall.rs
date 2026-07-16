//! 乙太方界·玩家自由市集 v1（自主提案切片 832）。
//!
//! **真缺口**：玩家↔玩家至今有三種互動——漂流瓶（825，非同步/文字/匿名）、並肩協作
//! （827，被動的採集加成）、掉落物（828，主動但單向的實體轉手）——但世界從沒有一種
//! **雙向議定**的以物易物：你想拿石頭換木頭，得先找到對方、講好、同時各自掏東西出來，
//! 目前唯一辦法是各自碰運氣丟掉落物、指望對方剛好也想這樣換。「以物易物」這件事，
//! 玩家↔居民（670）與居民↔居民（723）都早就有了，唯獨**玩家↔玩家**是空白。
//!
//! 本刀補上：在世界裡擺一個小攤——放上你願意給的一份材料、標明你想換的一份材料，
//! 任何路過、身上有你要的東西的旅人都能上前一手交錢一手交貨，**哪怕你早已下線**。
//!
//! **escrow 模型（比照掉落物慣例）**：擺攤當下就先扣下你要給的材料，存進攤位本身
//! （非你的背包）——保證攤位一定兌現得了承諾，接手的人不會撲空。沒人接手可隨時
//! 自己收回攤位，材料原路退回；擺攤逾時未接手也會自動收攤退還。
//!
//! **一格一攤**（比照告示牌/漂流瓶慣例）：鍵＝世界座標，同一個位置同時只能有一攤。
//!
//! 純邏輯模組（不含 WS/鎖/IO 細節），由 `voxel_ws.rs` 包進 `RwLock` 使用。純記憶體
//! （重啟後市集清空、已扣下的材料退不回——與漂流瓶/掉落物等其餘純記憶體 store 風險
//! 等同，可接受，零 migration）。

use std::collections::HashMap;

/// 全局同時存在的攤位數量上限，避免無限堆積（比照漂流瓶 `MAX_ACTIVE_BOTTLES`）。
pub const MAX_ACTIVE_STALLS: usize = 60;

/// 攤位逾時沒人接手，這麼多秒後自動收攤、材料退還擺攤者（比掉落物消散久得多——
/// 這是認真的交易提案而非隨手丟棄，給更長的時間讓真的有緣人路過看到）。
pub const STALL_TTL_SECS: u64 = 1800;

/// 單筆給出/要求數量的上限（UI/心智負擔考量，非稀缺性限制）。
pub const MAX_STALL_COUNT: u32 = 64;

/// 一個擺在世界裡、待人接手的交易攤。
#[derive(Debug, Clone)]
pub struct TradeStall {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 擺攤者願意給出的物品與數量（已從擺攤者背包扣下、存於此攤）。
    pub give_item: u8,
    pub give_count: u32,
    /// 擺攤者想換得的物品與數量（接手者需給出）。
    pub want_item: u8,
    pub want_count: u32,
    pub owner: String,
    pub owner_name: String,
    pub opened_secs: u64,
}

/// 世界上所有交易攤（純記憶體）。鍵＝座標，一格一攤。
#[derive(Debug, Default)]
pub struct StallStore {
    stalls: HashMap<(i32, i32, i32), TradeStall>,
}

impl StallStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.stalls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stalls.is_empty()
    }

    pub fn has(&self, pos: (i32, i32, i32)) -> bool {
        self.stalls.contains_key(&pos)
    }

    /// 擺一個新攤位；已達全局上限或該座標已有攤位時回 `false`（呼叫端應在真的扣掉
    /// 背包**之前**先檢查，別讓材料憑空消失——比照漂流瓶 `at_cap` 護欄慣例）。
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &mut self,
        pos: (i32, i32, i32),
        give_item: u8,
        give_count: u32,
        want_item: u8,
        want_count: u32,
        owner: &str,
        now_secs: u64,
    ) -> bool {
        self.open_with_name(pos, give_item, give_count, want_item, want_count, owner, owner, now_secs)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn open_with_name(
        &mut self, pos: (i32, i32, i32), give_item: u8, give_count: u32,
        want_item: u8, want_count: u32, owner: &str, owner_name: &str, now_secs: u64,
    ) -> bool {
        if self.stalls.len() >= MAX_ACTIVE_STALLS || self.stalls.contains_key(&pos) {
            return false;
        }
        self.stalls.insert(
            pos,
            TradeStall {
                x: pos.0,
                y: pos.1,
                z: pos.2,
                give_item,
                give_count,
                want_item,
                want_count,
                owner: owner.to_string(),
                owner_name: owner_name.to_string(),
                opened_secs: now_secs,
            },
        );
        true
    }

    pub fn get(&self, pos: (i32, i32, i32)) -> Option<&TradeStall> {
        self.stalls.get(&pos)
    }

    /// 取走並移除指定攤位（呼叫端接手/收攤成功後生效；若中途驗證失敗，用 [`Self::put_back`] 回滾）。
    pub fn remove(&mut self, pos: (i32, i32, i32)) -> Option<TradeStall> {
        self.stalls.remove(&pos)
    }

    /// 交易/收攤驗證中途失敗時，把已取出的攤位放回去（絕不讓已扣下的材料憑空消失）。
    pub fn put_back(&mut self, stall: TradeStall) {
        self.stalls.insert((stall.x, stall.y, stall.z), stall);
    }

    /// 全部攤位（依座標排序，供連線同步）。
    pub fn all(&self) -> Vec<&TradeStall> {
        let mut v: Vec<_> = self.stalls.values().collect();
        v.sort_by_key(|s| (s.x, s.y, s.z));
        v
    }

    /// 清掉已經過期（超過 [`STALL_TTL_SECS`] 沒人接手）的攤位，回傳被清掉的清單供呼叫端
    /// 退還材料給擺攤者＋廣播移除。
    pub fn expire(&mut self, now_secs: u64) -> Vec<TradeStall> {
        let expired_keys: Vec<(i32, i32, i32)> = self
            .stalls
            .iter()
            .filter(|(_, s)| now_secs.saturating_sub(s.opened_secs) >= STALL_TTL_SECS)
            .map(|(&k, _)| k)
            .collect();
        expired_keys
            .into_iter()
            .filter_map(|k| self.stalls.remove(&k))
            .collect()
    }
}

/// 把玩家指定的給出/要求數量夾進 `[1, MAX_STALL_COUNT]`，確定性、絕不 panic。
pub fn clamp_stall_count(count: u32) -> u32 {
    count.clamp(1, MAX_STALL_COUNT)
}

/// 給出/要求的物品需為真實物品（非 Air=0）且彼此不同（同一物品互換沒有意義）。
pub fn valid_stall_items(give_item: u8, want_item: u8) -> bool {
    give_item != 0 && want_item != 0 && give_item != want_item
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_entry_and_len_increases() {
        let mut s = StallStore::new();
        assert!(s.open((1, 2, 3), 5, 2, 3, 1, "露娜", 100));
        assert_eq!(s.len(), 1);
        let st = s.get((1, 2, 3)).unwrap();
        assert_eq!(st.give_item, 5);
        assert_eq!(st.give_count, 2);
        assert_eq!(st.want_item, 3);
        assert_eq!(st.want_count, 1);
        assert_eq!(st.owner, "露娜");
    }

    #[test]
    fn open_fails_when_position_occupied() {
        let mut s = StallStore::new();
        assert!(s.open((0, 0, 0), 5, 1, 3, 1, "甲", 0));
        assert!(!s.open((0, 0, 0), 4, 1, 3, 1, "乙", 0), "同一座標不應同時有兩攤");
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn open_fails_at_cap() {
        let mut s = StallStore::new();
        for i in 0..MAX_ACTIVE_STALLS {
            assert!(s.open((i as i32, 0, 0), 5, 1, 3, 1, "旅人", 0));
        }
        assert_eq!(s.len(), MAX_ACTIVE_STALLS);
        assert!(!s.open((999, 0, 0), 5, 1, 3, 1, "旅人", 0));
        assert_eq!(s.len(), MAX_ACTIVE_STALLS, "已達上限不應多存一筆");
    }

    #[test]
    fn remove_returns_stall_and_clears_it() {
        let mut s = StallStore::new();
        s.open((0, 0, 0), 5, 1, 3, 1, "諾娃", 10);
        let removed = s.remove((0, 0, 0)).unwrap();
        assert_eq!(removed.give_item, 5);
        assert!(s.is_empty());
        assert!(s.remove((0, 0, 0)).is_none(), "重複移除該回 None");
    }

    #[test]
    fn put_back_restores_removed_stall() {
        let mut s = StallStore::new();
        s.open((1, 1, 1), 5, 2, 3, 1, "賽勒", 0);
        let removed = s.remove((1, 1, 1)).unwrap();
        assert!(s.is_empty());
        s.put_back(removed);
        assert_eq!(s.len(), 1);
        assert_eq!(s.get((1, 1, 1)).unwrap().owner, "賽勒");
    }

    #[test]
    fn expire_removes_old_keeps_fresh() {
        let mut s = StallStore::new();
        s.open((0, 0, 0), 5, 1, 3, 1, "旅人甲", 0);
        s.open((1, 0, 0), 5, 1, 3, 1, "旅人乙", 1000);
        let expired = s.expire(STALL_TTL_SECS);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].owner, "旅人甲");
        assert_eq!(s.len(), 1);
        assert!(s.get((1, 0, 0)).is_some());
    }

    #[test]
    fn expire_boundary_exactly_ttl_expires() {
        let mut s = StallStore::new();
        s.open((0, 0, 0), 5, 1, 3, 1, "旅人", 0);
        let expired = s.expire(STALL_TTL_SECS);
        assert_eq!(expired.len(), 1);
    }

    #[test]
    fn expire_empty_store_returns_empty() {
        let mut s = StallStore::new();
        assert!(s.expire(9999).is_empty());
    }

    #[test]
    fn all_sorted_by_position() {
        let mut s = StallStore::new();
        s.open((5, 0, 0), 5, 1, 3, 1, "甲", 0);
        s.open((1, 0, 0), 5, 1, 3, 1, "乙", 0);
        let positions: Vec<(i32, i32, i32)> = s.all().iter().map(|st| (st.x, st.y, st.z)).collect();
        assert_eq!(positions, vec![(1, 0, 0), (5, 0, 0)]);
    }

    #[test]
    fn clamp_stall_count_bounds() {
        assert_eq!(clamp_stall_count(0), 1);
        assert_eq!(clamp_stall_count(1), 1);
        assert_eq!(clamp_stall_count(10), 10);
        assert_eq!(clamp_stall_count(MAX_STALL_COUNT), MAX_STALL_COUNT);
        assert_eq!(clamp_stall_count(MAX_STALL_COUNT + 1000), MAX_STALL_COUNT);
    }

    #[test]
    fn valid_stall_items_rejects_air_and_same_item() {
        assert!(!valid_stall_items(0, 3), "給出 Air 不合法");
        assert!(!valid_stall_items(5, 0), "要求 Air 不合法");
        assert!(!valid_stall_items(5, 5), "同一物品互換沒有意義");
        assert!(valid_stall_items(5, 3), "不同真實物品應合法");
    }

    #[test]
    fn has_reflects_store_state() {
        let mut s = StallStore::new();
        assert!(!s.has((0, 0, 0)));
        s.open((0, 0, 0), 5, 1, 3, 1, "旅人", 0);
        assert!(s.has((0, 0, 0)));
    }
}
