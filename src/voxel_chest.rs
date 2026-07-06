//! 乙太方界·箱子儲存系統 v1（ROADMAP 692）。
//!
//! 玩家合成箱子方塊後放置於世界，右鍵互動開啟共用儲存面板，可把多餘材料收進去——
//! 讓「採集→合成→建造基地+儲存」閉環第一次真正完整。
//!
//! **設計**：箱子存量以世界座標 `(wx, wy, wz)` 為鍵，內容是 `block_id → count` 的 HashMap。
//! 多位玩家共用同一箱子（先到先得，無鎖競爭由 WS handler 序列化 RwLock 解決）。
//! 箱子被破壞時，內容歸還破壞者（守護資料，不讓材料憑空消失）。
//!
//! **persist**：append-only JSONL（`data/voxel_chests.jsonl`），每筆事件記一行；
//! 重啟後 replay 重建現況（與背包持久化方式對齊）。
//!
//! 純邏輯層：零 async、零鎖、零 IO；鎖/IO/廣播全在 `voxel_ws.rs`。

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

/// 持久化路徑。
pub const CHEST_PATH: &str = "data/voxel_chests.jsonl";

/// 世界座標鍵（字串格式 "wx,wy,wz"，JSONL 序列化用）。
pub fn pos_key(wx: i32, wy: i32, wz: i32) -> String {
    format!("{wx},{wy},{wz}")
}

/// 一筆箱子事件（append-only JSONL 最小單元）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChestEntry {
    /// 箱子世界座標鍵。
    pub pos: String,
    /// 物品 id（`Block as u8` 或純物品 id）。
    pub item_id: u8,
    /// 正數 = 放入，負數 = 取出，0 = 不合法（skip）。
    pub delta: i32,
    /// 單調遞增序號（replay 時保序）。
    pub seq: u64,
}

/// 全局箱子 store：pos_key → (item_id → count)。
#[derive(Default)]
pub struct ChestStore {
    /// 存量 map：pos_key → HashMap<item_id, count>（只存非零值）。
    chests: HashMap<String, HashMap<u8, u32>>,
    /// 下一筆事件的序號。
    next_seq: u64,
}

impl ChestStore {
    /// 空 store（測試 / 首次啟動）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建 store（重啟後從 JSONL replay）。
    pub fn from_entries(entries: Vec<ChestEntry>) -> Self {
        let mut chests: HashMap<String, HashMap<u8, u32>> = HashMap::new();
        let mut max_seq = 0u64;
        for e in &entries {
            if e.delta == 0 { continue; }
            max_seq = max_seq.max(e.seq);
            let slot = chests.entry(e.pos.clone()).or_default();
            if e.delta > 0 {
                *slot.entry(e.item_id).or_insert(0) += e.delta as u32;
            } else {
                let sub = (-e.delta) as u32;
                let cur = slot.entry(e.item_id).or_insert(0);
                *cur = cur.saturating_sub(sub);
                if *cur == 0 { slot.remove(&e.item_id); }
            }
        }
        Self { chests, next_seq: max_seq + 1 }
    }

    /// 查詢箱子內容（回傳 `(item_id, count)` 向量，已按 item_id 排序，空箱子回空向量）。
    pub fn contents(&self, pos: &str) -> Vec<(u8, u32)> {
        let mut pairs: Vec<(u8, u32)> = self
            .chests
            .get(pos)
            .map(|m| m.iter().map(|(&id, &cnt)| (id, cnt)).collect())
            .unwrap_or_default();
        pairs.sort_by_key(|&(id, _)| id);
        pairs
    }

    /// 放入物品：把 `count` 個 `item_id` 加進 `pos` 的箱子，回傳持久化事件（供呼叫方 append）。
    /// 不限容量（v1 無上限，未來可加 MAX_CHEST_SLOTS 防濫用）。
    pub fn put(
        &mut self,
        pos: &str,
        item_id: u8,
        count: u32,
    ) -> ChestEntry {
        if count == 0 {
            return ChestEntry { pos: pos.to_string(), item_id, delta: 0, seq: 0 };
        }
        let slot = self.chests.entry(pos.to_string()).or_default();
        *slot.entry(item_id).or_insert(0) += count;
        let seq = self.next_seq;
        self.next_seq += 1;
        ChestEntry { pos: pos.to_string(), item_id, delta: count as i32, seq }
    }

    /// 取出物品：從 `pos` 箱子取 `count` 個 `item_id`（不足時靜默 clamp 實際取到的數量）。
    /// 回傳 `(實際取出量, 持久化事件)`；實際取出量可能 < count（箱子不夠多）。
    pub fn take(
        &mut self,
        pos: &str,
        item_id: u8,
        count: u32,
    ) -> (u32, ChestEntry) {
        let slot = self.chests.entry(pos.to_string()).or_default();
        let avail = slot.get(&item_id).copied().unwrap_or(0);
        let actual = avail.min(count);
        if actual == 0 {
            return (0, ChestEntry { pos: pos.to_string(), item_id, delta: 0, seq: 0 });
        }
        let new_count = avail - actual;
        if new_count == 0 { slot.remove(&item_id); } else { slot.insert(item_id, new_count); }
        let seq = self.next_seq;
        self.next_seq += 1;
        (actual, ChestEntry { pos: pos.to_string(), item_id, delta: -(actual as i32), seq })
    }

    /// 清空指定位置的箱子，回傳其原本內容（箱子被破壞時歸還材料用）。
    pub fn clear(&mut self, pos: &str) -> Vec<(u8, u32)> {
        let contents = self.contents(pos);
        self.chests.remove(pos);
        contents
    }

    /// 乙太方界共用糧倉 v1：從 (ox,oz) 起，`max_radius`（XZ 距離）內找一個「存有食物」的箱子——
    /// 讓找不到熟作物可收的餓居民，還能走去任何一個玩家存了食物的箱子借一份。`food_ids` 依偏好序
    /// 傳入，命中箱子裡第一個存量 >0 的即算數；箱子彼此都沒存糧、或半徑內根本沒箱子 → `None`
    /// （誠實失敗，鏡像 `voxel_skills::find_nearest_ripe_crop` 找不到熟作物時的態度）。
    /// 純邏輯（只讀既有 map，零 IO/鎖；呼叫端已持有 `chest` 讀鎖），可測。
    pub fn nearest_food_chest(
        &self,
        ox: i32,
        oz: i32,
        max_radius: i32,
        food_ids: &[u8],
    ) -> Option<(i32, i32, i32, u8)> {
        let max_d2 = i64::from(max_radius) * i64::from(max_radius);
        let mut best: Option<(i32, i32, i32, u8, i64)> = None;
        for (pos, contents) in &self.chests {
            let Some((x, y, z)) = parse_pos_key(pos) else { continue };
            let dx = i64::from(x - ox);
            let dz = i64::from(z - oz);
            let d2 = dx * dx + dz * dz;
            if d2 > max_d2 {
                continue;
            }
            let Some(&fid) = food_ids
                .iter()
                .find(|id| contents.get(id).copied().unwrap_or(0) > 0)
            else {
                continue;
            };
            if best.is_none_or(|(_, _, _, _, best_d2)| d2 < best_d2) {
                best = Some((x, y, z, fid, d2));
            }
        }
        best.map(|(x, y, z, fid, _)| (x, y, z, fid))
    }
}

/// [`pos_key`] 的反函式：把 "wx,wy,wz" 字串解析回座標。格式不符（理論上不會，所有鍵皆由
/// `pos_key` 生成）時回 `None`，呼叫端安全跳過而非 panic。
fn parse_pos_key(pos: &str) -> Option<(i32, i32, i32)> {
    let mut it = pos.split(',');
    let x = it.next()?.parse().ok()?;
    let y = it.next()?.parse().ok()?;
    let z = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((x, y, z))
}

// ── 持久化 IO（在 voxel_ws.rs 的鎖外呼叫）────────────────────────────────────────────

/// 從磁碟載入所有箱子事件（啟動時呼叫一次）。
pub fn load_chests() -> Vec<ChestEntry> {
    let Ok(f) = fs::File::open(CHEST_PATH) else { return vec![]; };
    BufReader::new(f)
        .lines()
        .filter_map(|l| l.ok())
        .filter_map(|l| serde_json::from_str::<ChestEntry>(&l).ok())
        .collect()
}

/// Append 單筆事件（delta=0 的事件直接忽略）。
pub fn append_chest(entry: &ChestEntry) {
    if entry.delta == 0 { return; }
    let Ok(line) = serde_json::to_string(entry) else { return; };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(CHEST_PATH) else { return; };
    let _ = writeln!(f, "{line}");
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_contents() {
        let mut store = ChestStore::new();
        store.put("1,2,3", 8, 5); // 5 木板進箱
        let c = store.contents("1,2,3");
        assert_eq!(c, vec![(8, 5)]);
    }

    #[test]
    fn take_exact() {
        let mut store = ChestStore::new();
        store.put("1,2,3", 8, 10);
        let (got, _e) = store.take("1,2,3", 8, 4);
        assert_eq!(got, 4);
        assert_eq!(store.contents("1,2,3"), vec![(8, 6)]);
    }

    #[test]
    fn take_more_than_available_clamps() {
        let mut store = ChestStore::new();
        store.put("0,0,0", 3, 3);
        let (got, _e) = store.take("0,0,0", 3, 99);
        assert_eq!(got, 3); // 只拿到 3
        assert!(store.contents("0,0,0").is_empty());
    }

    #[test]
    fn take_nonexistent_returns_zero() {
        let mut store = ChestStore::new();
        let (got, _e) = store.take("9,9,9", 8, 1);
        assert_eq!(got, 0);
    }

    #[test]
    fn clear_returns_contents_and_empties() {
        let mut store = ChestStore::new();
        store.put("5,5,5", 8, 3);
        store.put("5,5,5", 9, 2);
        let returned = store.clear("5,5,5");
        // 排序後應有兩筆
        let mut r = returned.clone();
        r.sort_by_key(|&(id, _)| id);
        assert_eq!(r, vec![(8, 3), (9, 2)]);
        assert!(store.contents("5,5,5").is_empty());
    }

    #[test]
    fn from_entries_replay() {
        let entries = vec![
            ChestEntry { pos: "0,0,0".into(), item_id: 8, delta: 10, seq: 0 },
            ChestEntry { pos: "0,0,0".into(), item_id: 8, delta: -3, seq: 1 },
        ];
        let store = ChestStore::from_entries(entries);
        assert_eq!(store.contents("0,0,0"), vec![(8, 7)]);
        assert_eq!(store.next_seq, 2); // max_seq + 1
    }

    #[test]
    fn pos_key_format() {
        assert_eq!(pos_key(1, -2, 300), "1,-2,300");
    }

    #[test]
    fn parse_pos_key_roundtrip() {
        assert_eq!(parse_pos_key(&pos_key(1, -2, 300)), Some((1, -2, 300)));
        assert_eq!(parse_pos_key("garbage"), None);
        assert_eq!(parse_pos_key("1,2"), None);
        assert_eq!(parse_pos_key("1,2,3,4"), None);
    }

    #[test]
    fn nearest_food_chest_finds_within_radius() {
        let mut store = ChestStore::new();
        store.put("10,64,0", 19, 3); // 麵包
        let found = store.nearest_food_chest(0, 0, 16, &[19]);
        assert_eq!(found, Some((10, 64, 0, 19)));
    }

    #[test]
    fn nearest_food_chest_ignores_out_of_radius() {
        let mut store = ChestStore::new();
        store.put("100,64,0", 19, 3);
        assert_eq!(store.nearest_food_chest(0, 0, 16, &[19]), None);
    }

    #[test]
    fn nearest_food_chest_ignores_non_food_items() {
        let mut store = ChestStore::new();
        store.put("5,64,5", 8, 10); // 木板，非食物 id
        assert_eq!(store.nearest_food_chest(0, 0, 16, &[19, 77]), None);
    }

    #[test]
    fn nearest_food_chest_ignores_emptied_stock() {
        let mut store = ChestStore::new();
        store.put("5,64,5", 19, 2);
        let (_, e) = store.take("5,64,5", 19, 2); // 掏空
        assert_eq!(e.delta, -2);
        assert_eq!(store.nearest_food_chest(0, 0, 16, &[19]), None);
    }

    #[test]
    fn nearest_food_chest_picks_the_closest() {
        let mut store = ChestStore::new();
        store.put("10,64,0", 19, 1);
        store.put("3,64,0", 19, 1);
        let found = store.nearest_food_chest(0, 0, 16, &[19]);
        assert_eq!(found, Some((3, 64, 0, 19)));
    }

    #[test]
    fn nearest_food_chest_empty_store_returns_none() {
        let store = ChestStore::new();
        assert_eq!(store.nearest_food_chest(0, 0, 16, &[19, 77]), None);
    }
}
