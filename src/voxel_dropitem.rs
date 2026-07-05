//! 掉落物 v1（自主提案切片 828）——玩家↔玩家之間第一次能真的「遞東西」給彼此：
//! 對著地面丟下手上的一件材料，它會安靜地留在原地，附近的另一位玩家（也可以是自己）
//! 走近時自動撿起。
//!
//! **真缺口**：玩家↔玩家至今只有兩種互動——漂流瓶（825，丟一句話，非同步/匿名/一次性）
//! 與並肩協作（827，被動的採集加成，你完全不用做任何動作）。世界裡從沒有一種「主動把
//! 手上一件實體材料親手交給另一位真人」的辦法：想幫朋友一把、分他幾塊石頭，除了各自跑去
//! 挖礦別無他法。本模組補上這條缺口——**主動的實體資源轉手**，與 825（文字/非同步）、
//! 827（被動加成、非轉手）皆是全新維度。
//!
//! 純邏輯模組（不含 WS/鎖/IO 細節），由 `voxel_ws.rs` 包進 `RwLock` 使用。純記憶體
//! （比照 `voxel_farm`/`voxel_bottle` 等世界暫態 store 的慣例，重啟後掉落物清空，不持久化——
//! 掉落物本就是「暫留在地上等人撿」的短命狀態，非玩家永久資產，遺失風險等同其餘純記憶體
//! store，可接受）。

use std::collections::HashMap;

/// 全局同時存在的掉落物數量上限，避免無限堆積（比照漂流瓶 `MAX_ACTIVE_BOTTLES`）。
pub const MAX_ACTIVE_DROPS: usize = 200;

/// 掉落物沒人撿走的話，這麼多秒後自動消失（避免世界被廢棄物品永久佔滿）。
pub const DESPAWN_SECS: u64 = 300;

/// 玩家（或其他玩家）走進這個距離內即自動撿起（世界單位，約半個身位）。
pub const PICKUP_RADIUS: f32 = 1.6;

/// 單次丟下最多這麼多個（避免一次丟出離譜天量，UI/心智負擔考量，非稀缺性限制）。
pub const MAX_DROP_COUNT: u32 = 64;

/// 一件掉落在世界裡、尚未被撿走的物品。
#[derive(Debug, Clone)]
pub struct DroppedItem {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub item_id: u8,
    pub count: u32,
    pub dropped_by: String,
    pub dropped_secs: u64,
}

/// 世界上所有掉落物（純記憶體）。
#[derive(Debug, Default)]
pub struct DropStore {
    items: HashMap<u64, DroppedItem>,
    next_id: u64,
}

impl DropStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 落地一件掉落物；已達全局上限時回 `None`（呼叫端應在真的扣掉背包**之前**先檢查上限，
    /// 別讓物品憑空消失——比照漂流瓶 `at_cap` 護欄慣例）。
    pub fn spawn(
        &mut self,
        x: f32,
        y: f32,
        z: f32,
        item_id: u8,
        count: u32,
        dropped_by: &str,
        now_secs: u64,
    ) -> Option<u64> {
        if self.items.len() >= MAX_ACTIVE_DROPS {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.insert(
            id,
            DroppedItem {
                id,
                x,
                y,
                z,
                item_id,
                count,
                dropped_by: dropped_by.to_string(),
                dropped_secs: now_secs,
            },
        );
        Some(id)
    }

    /// 撿走／清除一件掉落物，回傳它原本的內容供呼叫端廣播＋給背包。
    pub fn remove(&mut self, id: u64) -> Option<DroppedItem> {
        self.items.remove(&id)
    }

    /// 全部掉落物（依 id 排序，供連線同步）。
    pub fn all(&self) -> Vec<&DroppedItem> {
        let mut v: Vec<_> = self.items.values().collect();
        v.sort_by_key(|d| d.id);
        v
    }

    /// 找出離 `(px,py,pz)` 在拾取半徑內的最早一件掉落物（id 最小＝最先丟下，確定性排序，
    /// 平手時不會每次挑到不同的一件）。壞座標（NaN/inf）保守回 `None`，不誤撿。
    /// **允許撿回自己剛丟下的東西**——這是正常操作（例如丟錯格子想收回），非漏洞。
    pub fn nearest_in_range(&self, px: f32, py: f32, pz: f32) -> Option<u64> {
        if !px.is_finite() || !py.is_finite() || !pz.is_finite() {
            return None;
        }
        let r2 = PICKUP_RADIUS * PICKUP_RADIUS;
        self.items
            .values()
            .filter(|d| {
                let dx = d.x - px;
                let dy = d.y - py;
                let dz = d.z - pz;
                let dist2 = dx * dx + dy * dy + dz * dz;
                dist2.is_finite() && dist2 <= r2
            })
            .min_by_key(|d| d.id)
            .map(|d| d.id)
    }

    /// 清掉已經過期（超過 `DESPAWN_SECS` 沒被撿走）的掉落物，回傳被清掉的清單供呼叫端
    /// 廣播移除（消散不告知任何人是誰丟的，安靜地消失即可）。
    pub fn expire(&mut self, now_secs: u64) -> Vec<DroppedItem> {
        let expired_ids: Vec<u64> = self
            .items
            .iter()
            .filter(|(_, d)| now_secs.saturating_sub(d.dropped_secs) >= DESPAWN_SECS)
            .map(|(&id, _)| id)
            .collect();
        expired_ids
            .into_iter()
            .filter_map(|id| self.items.remove(&id))
            .collect()
    }
}

/// 把玩家指定的丟下數量夾進 `[1, MAX_DROP_COUNT]`（0 或負值輸入在呼叫端已是 u32 故只會是 0，
/// 夾成 1；離譜天量夾成上限），確定性、絕不 panic。
pub fn clamp_drop_count(count: u32) -> u32 {
    count.clamp(1, MAX_DROP_COUNT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_creates_entry_and_len_increases() {
        let mut s = DropStore::new();
        let id = s.spawn(1.0, 2.0, 3.0, 5, 3, "露娜", 100).unwrap();
        assert_eq!(s.len(), 1);
        let all = s.all();
        assert_eq!(all[0].id, id);
        assert_eq!(all[0].item_id, 5);
        assert_eq!(all[0].count, 3);
        assert_eq!(all[0].dropped_by, "露娜");
    }

    #[test]
    fn spawn_fails_at_cap() {
        let mut s = DropStore::new();
        for i in 0..MAX_ACTIVE_DROPS {
            assert!(s.spawn(i as f32, 0.0, 0.0, 5, 1, "旅人", 0).is_some());
        }
        assert_eq!(s.len(), MAX_ACTIVE_DROPS);
        assert!(s.spawn(999.0, 0.0, 0.0, 5, 1, "旅人", 0).is_none());
        assert_eq!(s.len(), MAX_ACTIVE_DROPS, "已達上限不應多存一筆");
    }

    #[test]
    fn remove_returns_item_and_clears_it() {
        let mut s = DropStore::new();
        let id = s.spawn(0.0, 0.0, 0.0, 8, 4, "諾娃", 10).unwrap();
        let removed = s.remove(id).unwrap();
        assert_eq!(removed.item_id, 8);
        assert!(s.is_empty());
        assert!(s.remove(id).is_none(), "重複移除該回 None");
    }

    #[test]
    fn nearest_in_range_finds_within_radius() {
        let mut s = DropStore::new();
        let id = s.spawn(10.0, 5.0, 10.0, 3, 1, "賽勒", 0).unwrap();
        assert_eq!(s.nearest_in_range(10.5, 5.0, 10.0), Some(id));
    }

    #[test]
    fn nearest_in_range_none_when_too_far() {
        let mut s = DropStore::new();
        s.spawn(0.0, 0.0, 0.0, 3, 1, "奧瑞", 0).unwrap();
        assert_eq!(s.nearest_in_range(50.0, 0.0, 0.0), None);
    }

    #[test]
    fn nearest_in_range_boundary_exactly_at_radius_counts() {
        let mut s = DropStore::new();
        let id = s.spawn(0.0, 0.0, 0.0, 3, 1, "旅人", 0).unwrap();
        assert_eq!(s.nearest_in_range(PICKUP_RADIUS, 0.0, 0.0), Some(id));
    }

    #[test]
    fn nearest_in_range_ties_pick_smallest_id_deterministically() {
        let mut s = DropStore::new();
        let id_a = s.spawn(1.0, 0.0, 0.0, 3, 1, "旅人甲", 0).unwrap();
        let _id_b = s.spawn(1.0, 0.0, 0.0, 3, 1, "旅人乙", 0).unwrap();
        // 兩件掉落物同座標（等距），確定性挑 id 較小（較早丟下）的那件。
        assert_eq!(s.nearest_in_range(1.0, 0.0, 0.0), Some(id_a));
    }

    #[test]
    fn nearest_in_range_rejects_bad_coords() {
        let mut s = DropStore::new();
        s.spawn(0.0, 0.0, 0.0, 3, 1, "旅人", 0).unwrap();
        assert_eq!(s.nearest_in_range(f32::NAN, 0.0, 0.0), None);
        assert_eq!(s.nearest_in_range(f32::INFINITY, 0.0, 0.0), None);
    }

    #[test]
    fn expire_removes_old_keeps_fresh() {
        let mut s = DropStore::new();
        let old = s.spawn(0.0, 0.0, 0.0, 3, 1, "旅人", 0).unwrap();
        let fresh = s.spawn(0.0, 0.0, 0.0, 3, 1, "旅人", 250).unwrap();
        let expired = s.expire(300);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, old);
        assert_eq!(s.len(), 1);
        assert!(s.all().iter().any(|d| d.id == fresh));
    }

    #[test]
    fn expire_boundary_exactly_despawn_secs_expires() {
        let mut s = DropStore::new();
        let id = s.spawn(0.0, 0.0, 0.0, 3, 1, "旅人", 0).unwrap();
        let expired = s.expire(DESPAWN_SECS);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, id);
    }

    #[test]
    fn expire_empty_store_returns_empty() {
        let mut s = DropStore::new();
        assert!(s.expire(9999).is_empty());
    }

    #[test]
    fn all_sorted_by_id() {
        let mut s = DropStore::new();
        let a = s.spawn(0.0, 0.0, 0.0, 1, 1, "甲", 0).unwrap();
        let b = s.spawn(0.0, 0.0, 0.0, 2, 1, "乙", 0).unwrap();
        let ids: Vec<u64> = s.all().iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![a, b]);
    }

    #[test]
    fn clamp_drop_count_bounds() {
        assert_eq!(clamp_drop_count(0), 1);
        assert_eq!(clamp_drop_count(1), 1);
        assert_eq!(clamp_drop_count(10), 10);
        assert_eq!(clamp_drop_count(MAX_DROP_COUNT), MAX_DROP_COUNT);
        assert_eq!(clamp_drop_count(MAX_DROP_COUNT + 1000), MAX_DROP_COUNT);
    }
}
