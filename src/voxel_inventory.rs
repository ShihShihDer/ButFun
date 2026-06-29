//! 乙太方界玩家背包 v1——挖方塊得材料、放置消耗存量。
//!
//! 純邏輯模組（不含 WS/鎖/IO 細節），由 `voxel_ws.rs` 包進 `RwLock` 使用。
//! **append-only jsonl 持久化**（`data/voxel_inventory.jsonl`），重啟後存量還在。
//!
//! 設計：以玩家「顯示名」（`name` from Join）為索引鍵，與 voxel_memory / voxel_desires 一致。
//! Delta 格式 jsonl：每筆 `(player, block_id, delta, seq)`，replay 重建現狀。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// jsonl 持久化路徑。
pub const VOXEL_INV_PATH: &str = "data/voxel_inventory.jsonl";

/// 一筆背包事件（append-only jsonl 最小單元）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvEntry {
    /// 玩家顯示名（索引鍵）。
    pub player: String,
    /// 方塊 id（`voxel::Block as u8`）。
    pub block_id: u8,
    /// 正數=獲得，負數=消耗。
    pub delta: i32,
    /// 單調遞增序號（replay 順序保證）。
    pub seq: u64,
}

/// 一位玩家的材料存量（block_id → 數量；只存非零項）。
#[derive(Debug, Clone, Default)]
pub struct VoxelInventory {
    items: HashMap<u8, u32>,
}

impl VoxelInventory {
    /// 增加材料。
    pub fn add(&mut self, block_id: u8, count: u32) {
        *self.items.entry(block_id).or_insert(0) += count;
    }

    /// 嘗試消耗 count 個 block_id；不足時回 false 且不更動狀態。
    pub fn take(&mut self, block_id: u8, count: u32) -> bool {
        let cur = self.items.get(&block_id).copied().unwrap_or(0);
        if cur < count {
            return false;
        }
        let next = cur - count;
        if next == 0 {
            self.items.remove(&block_id);
        } else {
            self.items.insert(block_id, next);
        }
        true
    }

    /// 查詢數量（0 = 沒有）。
    pub fn count(&self, block_id: u8) -> u32 {
        self.items.get(&block_id).copied().unwrap_or(0)
    }

    /// 非零項列表，按 block_id 排序（UI / 序列化用）。
    pub fn pairs(&self) -> Vec<(u8, u32)> {
        let mut v: Vec<_> = self.items.iter().map(|(&k, &v)| (k, v)).collect();
        v.sort_by_key(|&(k, _)| k);
        v
    }
}

/// 全服背包 store（玩家顯示名 → VoxelInventory）。
/// 純資料結構，所有方法皆同步；由 `voxel_ws.rs` 包進 `RwLock` 使用。
#[derive(Debug, Default)]
pub struct InvStore {
    map: HashMap<String, VoxelInventory>,
    pub next_seq: u64,
}

impl InvStore {
    /// 給予材料，回傳已記錄的 InvEntry（呼叫端 append 落地）。
    pub fn give(&mut self, player: &str, block_id: u8, count: u32) -> InvEntry {
        self.map.entry(player.to_string()).or_default().add(block_id, count);
        let e = InvEntry {
            player: player.to_string(),
            block_id,
            delta: count as i32,
            seq: self.next_seq,
        };
        self.next_seq += 1;
        e
    }

    /// 嘗試消耗材料；不足時回 None（不更動狀態）；成功回 InvEntry（呼叫端 append 落地）。
    pub fn take(&mut self, player: &str, block_id: u8, count: u32) -> Option<InvEntry> {
        if !self.map.entry(player.to_string()).or_default().take(block_id, count) {
            return None;
        }
        let e = InvEntry {
            player: player.to_string(),
            block_id,
            delta: -(count as i32),
            seq: self.next_seq,
        };
        self.next_seq += 1;
        Some(e)
    }

    /// 查詢數量（不存在的玩家回 0）。
    pub fn count(&self, player: &str, block_id: u8) -> u32 {
        self.map.get(player).map_or(0, |inv| inv.count(block_id))
    }

    /// 取某玩家的全部非零材料（UI / 背包同步用）。
    pub fn pairs(&self, player: &str) -> Vec<(u8, u32)> {
        self.map
            .get(player)
            .map(|inv| inv.pairs())
            .unwrap_or_default()
    }

    /// 由 jsonl 記錄列表重建狀態（啟動時 replay；壞 delta 嘗試 take 失敗靜默略過）。
    pub fn from_entries(entries: Vec<InvEntry>) -> Self {
        let mut store = InvStore::default();
        for e in &entries {
            if e.delta > 0 {
                store
                    .map
                    .entry(e.player.clone())
                    .or_default()
                    .add(e.block_id, e.delta as u32);
            } else if e.delta < 0 {
                // take 失敗（存量不足）靜默略過，保持一致性。
                store
                    .map
                    .entry(e.player.clone())
                    .or_default()
                    .take(e.block_id, (-e.delta) as u32);
            }
            if e.seq >= store.next_seq {
                store.next_seq = e.seq + 1;
            }
        }
        store
    }
}

// ── jsonl 持久化（比照 voxel_memory::append_memory：輕量同步小檔寫）─────────────

/// 把一筆 InvEntry append 到 jsonl（不持任何鎖；失敗只記 log、不 panic）。
pub fn append_inv(entry: &InvEntry) {
    let clean_player: String = entry
        .player
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let clean = InvEntry {
        player: clean_player,
        block_id: entry.block_id,
        delta: entry.delta,
        seq: entry.seq,
    };
    let Ok(val) = serde_json::to_value(&clean) else {
        return;
    };
    write_inv_line(VOXEL_INV_PATH, &val);
}

/// 從 jsonl 載回所有記錄（啟動時呼叫一次）。檔不存在 / 壞行皆容忍。
pub fn load_inventory() -> Vec<InvEntry> {
    let Ok(content) = std::fs::read_to_string(VOXEL_INV_PATH) else {
        return vec![];
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<InvEntry>(line).ok())
        .collect()
}

fn write_inv_line(path: &str, record: &serde_json::Value) {
    use std::io::Write;
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            if let Ok(line) = serde_json::to_string(record) {
                let _ = writeln!(f, "{}", line);
            }
        }
        Err(e) => eprintln!("[voxel_inventory] append 失敗: {e}"),
    }
}

// ── 測試 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── VoxelInventory ────────────────────────────────────────

    #[test]
    fn add_and_count() {
        let mut inv = VoxelInventory::default();
        inv.add(5, 3);
        assert_eq!(inv.count(5), 3);
        assert_eq!(inv.count(3), 0); // 未加入的方塊回 0
    }

    #[test]
    fn take_success_reduces_count() {
        let mut inv = VoxelInventory::default();
        inv.add(3, 5);
        assert!(inv.take(3, 3));
        assert_eq!(inv.count(3), 2);
    }

    #[test]
    fn take_all_removes_entry() {
        let mut inv = VoxelInventory::default();
        inv.add(5, 1);
        assert!(inv.take(5, 1));
        assert_eq!(inv.count(5), 0);
        assert!(inv.pairs().is_empty()); // 非零才出現在 pairs
    }

    #[test]
    fn take_insufficient_fails_and_leaves_state_unchanged() {
        let mut inv = VoxelInventory::default();
        inv.add(3, 2);
        assert!(!inv.take(3, 3)); // 不足
        assert_eq!(inv.count(3), 2); // 未更動
    }

    #[test]
    fn take_empty_returns_false() {
        let mut inv = VoxelInventory::default();
        assert!(!inv.take(5, 1)); // 根本沒有
    }

    #[test]
    fn pairs_sorted_by_block_id() {
        let mut inv = VoxelInventory::default();
        inv.add(5, 10);
        inv.add(2, 3);
        inv.add(1, 7);
        let p = inv.pairs();
        assert_eq!(p[0].0, 1);
        assert_eq!(p[1].0, 2);
        assert_eq!(p[2].0, 5);
    }

    // ── InvStore ──────────────────────────────────────────────

    #[test]
    fn store_give_increases_count() {
        let mut store = InvStore::default();
        let e = store.give("alice", 5, 2);
        assert_eq!(e.delta, 2);
        assert_eq!(e.block_id, 5);
        assert_eq!(store.count("alice", 5), 2);
    }

    #[test]
    fn store_take_success() {
        let mut store = InvStore::default();
        store.give("bob", 3, 10);
        let e = store.take("bob", 3, 4);
        assert!(e.is_some());
        assert_eq!(e.unwrap().delta, -4);
        assert_eq!(store.count("bob", 3), 6);
    }

    #[test]
    fn store_take_insufficient_returns_none_and_state_unchanged() {
        let mut store = InvStore::default();
        store.give("carol", 3, 2);
        assert!(store.take("carol", 3, 5).is_none());
        assert_eq!(store.count("carol", 3), 2); // 未更動
    }

    #[test]
    fn store_seq_increments_per_operation() {
        let mut store = InvStore::default();
        let e1 = store.give("alice", 5, 1);
        let e2 = store.take("alice", 5, 1).unwrap();
        assert_eq!(e1.seq, 0);
        assert_eq!(e2.seq, 1);
    }

    #[test]
    fn store_different_players_independent() {
        let mut store = InvStore::default();
        store.give("alice", 5, 3);
        store.give("bob", 5, 7);
        assert_eq!(store.count("alice", 5), 3);
        assert_eq!(store.count("bob", 5), 7);
    }

    #[test]
    fn from_entries_replays_correctly() {
        let entries = vec![
            InvEntry { player: "alice".into(), block_id: 5, delta: 10, seq: 0 },
            InvEntry { player: "alice".into(), block_id: 5, delta: -3, seq: 1 },
            InvEntry { player: "bob".into(), block_id: 2, delta: 5, seq: 2 },
        ];
        let store = InvStore::from_entries(entries);
        assert_eq!(store.count("alice", 5), 7);
        assert_eq!(store.count("bob", 2), 5);
        assert_eq!(store.next_seq, 3);
    }

    #[test]
    fn from_entries_bad_take_silently_skipped() {
        // 嘗試消耗超過存量：take 失敗靜默略過，存量保持原值。
        let entries = vec![
            InvEntry { player: "alice".into(), block_id: 3, delta: 2, seq: 0 },
            InvEntry { player: "alice".into(), block_id: 3, delta: -5, seq: 1 },
        ];
        let store = InvStore::from_entries(entries);
        assert_eq!(store.count("alice", 3), 2); // 壞 take 未扣
    }

    #[test]
    fn jsonl_roundtrip() {
        let dir = std::env::temp_dir().join(format!("voxinv_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("voxel_inventory.jsonl");
        let pstr = path.to_str().unwrap();
        let _ = std::fs::remove_file(&path);

        let e = InvEntry { player: "alice".into(), block_id: 5, delta: 3, seq: 0 };
        let val = serde_json::to_value(&e).unwrap();
        write_inv_line(pstr, &val);

        let content = std::fs::read_to_string(&path).unwrap();
        let loaded: Vec<InvEntry> = content
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].player, "alice");
        assert_eq!(loaded[0].block_id, 5);
        assert_eq!(loaded[0].delta, 3);
    }

    #[test]
    fn jsonl_bad_line_skipped() {
        let dir = std::env::temp_dir().join(format!("voxinv_bad_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("voxel_inventory_bad.jsonl");
        let pstr = path.to_str().unwrap();
        let _ = std::fs::remove_file(&path);
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(pstr).unwrap();
        writeln!(f, "{{\"player\":\"ok\",\"block_id\":3,\"delta\":1,\"seq\":0}}").unwrap();
        writeln!(f, "壞行{{not json}}").unwrap();
        writeln!(f, "{{\"player\":\"ok2\",\"block_id\":2,\"delta\":4,\"seq\":1}}").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let loaded: Vec<InvEntry> =
            content.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        assert_eq!(loaded.len(), 2); // 壞行被略過
    }
}
