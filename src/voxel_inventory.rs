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

    /// 移除某玩家的記憶體背包 entry（M5 訪客斷線清理用）。回傳是否真的移走了一筆。
    /// **只動記憶體**——jsonl 是 append-only、不在此刪（登入玩家重登會由 jsonl replay 復原，
    /// 故只該對「無持久化價值的訪客」呼叫；呼叫端負責這個判斷）。
    pub fn remove_player(&mut self, player: &str) -> bool {
        self.map.remove(player).is_some()
    }

    /// 目前記憶體中追蹤的玩家背包數（有界性檢查／測試用）。
    pub fn tracked_players(&self) -> usize {
        self.map.len()
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

// ── Compaction（M4 防磁碟無界膨脹）─────────────────────────────────────────
//
// voxel_inventory.jsonl 是「最終狀態型」append-only log：每筆 delta 累加出現狀。
// 壓縮策略：把現狀展開成「每 (player, block_id) 一筆正 delta」最小序列，
// 原子重寫替換原檔（temp → rename）。
// 鐵律：
//   1. replay(原始 entries) 與 replay(compact entries) 產生的 InvStore 現狀完全等價。
//   2. rename 前原檔不動；rename 失敗保住原檔。
//   3. 向後相容：新舊格式都能讀（serde default），不 drop 欄位。

/// 把 `path` 對應的 inventory jsonl 壓縮成最小現狀序列（原子 rename 替換原檔）。
/// 失敗時只記 log、保住原檔、不 panic。
/// 呼叫時機：伺服器啟動時（replay 完成後）或定期排程（鎖外呼叫）。
pub fn compact_inventory(path: &str) {
    let entries = load_inventory_from(path);
    if entries.is_empty() {
        return; // 空檔不必壓縮
    }
    let store = InvStore::from_entries(entries);
    // 展開成最小現狀 entries（每 player 每 block_id 一筆正 delta）
    let mut seq = 0u64;
    let mut minimal: Vec<InvEntry> = Vec::new();
    let mut players: Vec<String> = store.map.keys().cloned().collect();
    players.sort(); // 確定性順序，方便測試
    for player in &players {
        let mut pairs = store.map[player].pairs();
        pairs.sort_by_key(|&(k, _)| k);
        for (block_id, count) in pairs {
            if count > 0 {
                minimal.push(InvEntry {
                    player: player.clone(),
                    block_id,
                    delta: count as i32,
                    seq,
                });
                seq += 1;
            }
        }
    }

    // 序列化成 jsonl 字串
    let mut content = String::new();
    for e in &minimal {
        match serde_json::to_string(e) {
            Ok(line) => {
                content.push_str(&line);
                content.push('\n');
            }
            Err(err) => {
                eprintln!("[voxel_inventory] compact 序列化失敗: {err}，放棄");
                return;
            }
        }
    }

    atomic_write(path, &content, "[voxel_inventory]");
}

/// 從指定路徑讀回所有 InvEntry（供 compact 使用；壞行略過）。
fn load_inventory_from(path: &str) -> Vec<InvEntry> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec![];
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<InvEntry>(line).ok())
        .collect()
}

/// 原子寫：先寫 temp 再 rename 替換目標，失敗時清 temp、保住原檔。
fn atomic_write(path: &str, content: &str, log_prefix: &str) {
    let tmp = format!("{path}.compact.tmp");
    if let Err(e) = std::fs::write(&tmp, content) {
        eprintln!("{log_prefix} 寫 temp 失敗: {e}，放棄 compact");
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        eprintln!("{log_prefix} rename 失敗: {e}，原檔保留");
        let _ = std::fs::remove_file(&tmp);
    }
}

// ── 測試 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── InvStore::remove_player（M5 訪客斷線清理）────────────────

    #[test]
    fn remove_player_drops_in_memory_entry() {
        let mut store = InvStore::default();
        store.give("訪客A", 5, 3);
        store.give("訪客B", 5, 1);
        assert_eq!(store.tracked_players(), 2);
        assert_eq!(store.count("訪客A", 5), 3);
        // 移除訪客 A → 回 true、其背包歸零、只剩 B。
        assert!(store.remove_player("訪客A"));
        assert_eq!(store.count("訪客A", 5), 0);
        assert_eq!(store.tracked_players(), 1);
        assert_eq!(store.count("訪客B", 5), 1, "不影響其他玩家");
        // 再移除已不在的名字 → 回 false、no-op。
        assert!(!store.remove_player("訪客A"));
        assert_eq!(store.tracked_players(), 1);
    }

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

    // ── compact_inventory：A==B 等價驗證（M4 資料安全閘）─────────────────────

    /// 輔助：把多筆 InvEntry 序列化後寫到 tempfile，回傳路徑字串
    fn write_inv_tempfile(entries: &[InvEntry]) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        for e in entries {
            let val = serde_json::to_value(e).unwrap();
            writeln!(f, "{}", serde_json::to_string(&val).unwrap()).unwrap();
        }
        f
    }

    /// 比較兩個 InvStore 的現狀是否相同（僅比非空存量的玩家；pairs 排序後比對）。
    fn stores_equal(a: &InvStore, b: &InvStore) -> bool {
        // 只看「有非零存量」的玩家，跳過空 VoxelInventory 項目（compact 不寫出 count=0 的玩家）
        let mut players_a: Vec<String> = a
            .map
            .keys()
            .filter(|p| !a.pairs(p).is_empty())
            .cloned()
            .collect();
        let mut players_b: Vec<String> = b
            .map
            .keys()
            .filter(|p| !b.pairs(p).is_empty())
            .cloned()
            .collect();
        players_a.sort();
        players_b.sort();
        if players_a != players_b {
            return false;
        }
        for p in &players_a {
            if a.pairs(p) != b.pairs(p) {
                return false;
            }
        }
        true
    }

    #[test]
    fn compact_inventory_state_equivalent_simple() {
        // 原始 entries：多筆增減（有冗餘）
        let entries = vec![
            InvEntry { player: "alice".into(), block_id: 5, delta: 10, seq: 0 },
            InvEntry { player: "alice".into(), block_id: 5, delta: -3, seq: 1 },
            InvEntry { player: "alice".into(), block_id: 2, delta: 5,  seq: 2 },
            InvEntry { player: "bob".into(),   block_id: 5, delta: 4,  seq: 3 },
            InvEntry { player: "bob".into(),   block_id: 5, delta: -4, seq: 4 }, // bob.5 淨 = 0
        ];

        let tf = write_inv_tempfile(&entries);
        let path = tf.path().to_str().unwrap().to_string();

        // 狀態 A：直接 replay 原始 entries
        let store_a = InvStore::from_entries(load_inventory_from(&path));

        // 執行 compact
        compact_inventory(&path);

        // 狀態 B：replay compact 後的 entries
        let store_b = InvStore::from_entries(load_inventory_from(&path));

        assert!(stores_equal(&store_a, &store_b), "compact 前後現狀應完全等價（A==B）");
    }

    #[test]
    fn compact_inventory_zero_count_removed() {
        // 若某 block_id 被全部消耗（count=0），compact 後不應出現該項目
        let entries = vec![
            InvEntry { player: "carol".into(), block_id: 3, delta: 5, seq: 0 },
            InvEntry { player: "carol".into(), block_id: 3, delta: -5, seq: 1 }, // 淨=0
        ];
        let tf = write_inv_tempfile(&entries);
        let path = tf.path().to_str().unwrap().to_string();
        compact_inventory(&path);
        let after = load_inventory_from(&path);
        // carol.3 淨=0，不應出現在 compact 後的檔案
        assert!(after.iter().all(|e| !(e.player == "carol" && e.block_id == 3)));
    }

    #[test]
    fn compact_inventory_many_deltas_reduces_line_count() {
        // 反覆挖放同一方塊，compact 後行數應大幅減少
        let entries: Vec<InvEntry> = (0u64..200)
            .map(|i| InvEntry {
                player: "dave".into(),
                block_id: 1,
                delta: if i % 2 == 0 { 1 } else { -1 }, // 淨=0（偶數次）
                seq: i,
            })
            .collect();
        let tf = write_inv_tempfile(&entries);
        let path = tf.path().to_str().unwrap().to_string();

        let state_before = InvStore::from_entries(load_inventory_from(&path));
        compact_inventory(&path);
        let state_after = InvStore::from_entries(load_inventory_from(&path));

        assert!(stores_equal(&state_before, &state_after), "A==B");
        // 200 筆 → compact 後應 ≤ 1 行（dave.1 淨=0 → 不出現）
        let after_lines = load_inventory_from(&path);
        assert!(after_lines.len() <= 1, "compact 後行數應 ≤ 1，實際 {}", after_lines.len());
    }

    #[test]
    fn compact_inventory_preserves_multiple_players() {
        // 多玩家多物品，compact 後各玩家存量完整保留
        let entries = vec![
            InvEntry { player: "p1".into(), block_id: 1, delta: 10, seq: 0 },
            InvEntry { player: "p1".into(), block_id: 2, delta: 5,  seq: 1 },
            InvEntry { player: "p2".into(), block_id: 1, delta: 3,  seq: 2 },
            InvEntry { player: "p1".into(), block_id: 1, delta: -4, seq: 3 }, // p1.1 → 6
        ];
        let tf = write_inv_tempfile(&entries);
        let path = tf.path().to_str().unwrap().to_string();

        let store_a = InvStore::from_entries(load_inventory_from(&path));
        compact_inventory(&path);
        let store_b = InvStore::from_entries(load_inventory_from(&path));

        assert!(stores_equal(&store_a, &store_b), "A==B");
        assert_eq!(store_b.count("p1", 1), 6);
        assert_eq!(store_b.count("p1", 2), 5);
        assert_eq!(store_b.count("p2", 1), 3);
    }
}
