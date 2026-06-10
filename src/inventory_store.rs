//! 玩家背包的伺服器端記憶（Phase 0-E 持久化,沿 `positions.rs` 同一套抽換結構）。
//!
//! 採集、打怪掉落、農地收成三個來源都在灌背包,囤積卻撐不過 server 重啟（換版）——
//! 這層讓已登入玩家的背包跟著重連回來。行程內維護一份 `id -> Inventory` 的記憶體
//! cache 當權威來源,讓 `recall`/`remember` 保持**同步**、不污染 WebSocket / 遊戲迴圈
//! 的鎖序。耐久層在 cache 後面可抽換:
//!   - `Postgres`：設了 `DATABASE_URL` 時,啟動載回、定期非同步 upsert（正式上線走這條）。
//!   - `Jsonl`：沒設 `DATABASE_URL`（本機 `cargo run`）時寫穿 `data/inventories.jsonl`。
//!   - `Memory`：測試用,不碰磁碟也不碰 DB。
//!
//! 注意:只記「已登入」玩家（穩定 id）；訪客每次連線 id 隨機、記了也對不上,
//! 故不記,避免 cache 無界成長。延續 `PositionStore` 的每個設計權衡（DB 為主、JSONL
//! 補洞;flush 失敗只記 log 不中斷迴圈;載入時驗證壞值）。

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use sqlx::Row;
use uuid::Uuid;

use crate::equipment::EquipmentSlots;
use crate::inventory::Inventory;

/// 無 `DATABASE_URL` 時的退回持久化檔（執行期產生、已 gitignore）。接 Postgres 後這個檔
/// 退為「遷移種子」：啟動時 DB 沒有的 id 仍會從這裡補回 cache,讓換版不會把人的背包洗空。
const STORE_PATH: &str = "data/inventories.jsonl";

/// JSONL 一行紀錄：把 id、背包、裝備槽合起來序列化。
/// `equipment` 新增於 ROADMAP 36，舊版 JSONL 欄位缺席時用 `default` 補空槽。
#[derive(Serialize, Deserialize)]
struct DiskRow {
    id: Uuid,
    inv: Inventory,
    #[serde(default)]
    equipment: Option<EquipmentSlots>,
}

/// cache 後面的耐久層。
#[derive(Clone)]
enum Backend {
    /// 測試用：不載入、不寫。只在 `#[cfg(test)]` 的 `in_memory()` 建構,故非測試建置標 allow。
    #[cfg_attr(not(test), allow(dead_code))]
    Memory,
    /// 沒設 `DATABASE_URL`：寫穿到此 JSONL 檔。
    Jsonl(&'static str),
    /// 設了 `DATABASE_URL`：啟動載回、定期非同步 upsert 到 `inventories` 表。
    Postgres(PgPool),
}

/// 記住玩家背包與裝備槽的儲存層。記憶體 cache + 可抽換耐久層（Postgres / JSONL / 純記憶體）。
#[derive(Clone)]
pub struct InventoryStore {
    inner: Arc<RwLock<HashMap<Uuid, Inventory>>>,
    /// 裝備槽 cache（ROADMAP 36）。與背包共用同一個 inventories 表/JSONL 檔。
    equip: Arc<RwLock<HashMap<Uuid, EquipmentSlots>>>,
    backend: Backend,
}

impl Default for InventoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InventoryStore {
    /// 無 DB 模式：cache 從 JSONL 載入,之後寫穿 JSONL。
    pub fn new() -> Self {
        let (invs, equips) = load_from_disk(STORE_PATH);
        Self {
            inner: Arc::new(RwLock::new(invs)),
            equip: Arc::new(RwLock::new(equips)),
            backend: Backend::Jsonl(STORE_PATH),
        }
    }

    /// Postgres 模式（正式上線）：啟動時把 `inventories` 表載回 cache,再用既有 JSONL 補齊
    /// DB 還沒有的 id。DB 優先、JSONL 補洞（與 `PositionStore::from_pool` 相同策略）。
    pub async fn from_pool(pool: PgPool) -> Self {
        let (mut inv_cache, mut equip_cache) = load_inventories_from_db(&pool).await;
        let (disk_invs, disk_equips) = load_from_disk(STORE_PATH);
        for (id, inv) in disk_invs {
            inv_cache.entry(id).or_insert(inv);
        }
        for (id, eq) in disk_equips {
            equip_cache.entry(id).or_insert(eq);
        }
        Self {
            inner: Arc::new(RwLock::new(inv_cache)),
            equip: Arc::new(RwLock::new(equip_cache)),
            backend: Backend::Postgres(pool),
        }
    }

    /// 純記憶體版（測試用）：不載入、不寫磁碟、不碰 DB。
    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            equip: Arc::new(RwLock::new(HashMap::new())),
            backend: Backend::Memory,
        }
    }

    /// 取出某玩家上次離線時的背包（沒有就 None）。讀 cache,同步。
    pub fn recall(&self, id: Uuid) -> Option<Inventory> {
        self.inner.read().unwrap().get(&id).cloned()
    }

    /// 記住某玩家目前背包（更新 cache,同步）。Jsonl 模式順手寫穿磁碟。
    pub fn remember(&self, id: Uuid, inv: &Inventory) {
        self.inner.write().unwrap().insert(id, inv.clone());
        self.persist_jsonl();
    }

    /// 取出某玩家的裝備槽（無記錄回 `None` — 首次登入需 `auto_equip_best` 遷移）。
    pub fn recall_equipment(&self, id: Uuid) -> Option<EquipmentSlots> {
        self.equip.read().unwrap().get(&id).cloned()
    }

    /// 記住某玩家目前裝備槽（更新 cache,同步）。Jsonl 模式順手寫穿磁碟。
    pub fn remember_equipment(&self, id: Uuid, eq: &EquipmentSlots) {
        self.equip.write().unwrap().insert(id, eq.clone());
        self.persist_jsonl();
    }

    /// 批次記住多名玩家的背包（給遊戲迴圈定期快照線上玩家用）。
    pub fn remember_all<I: IntoIterator<Item = (Uuid, Inventory)>>(&self, items: I) {
        {
            let mut m = self.inner.write().unwrap();
            for (id, inv) in items {
                m.insert(id, inv);
            }
        }
        self.persist_jsonl();
    }

    /// 批次記住多名玩家的裝備槽（給遊戲迴圈定期快照用）。
    pub fn remember_all_equipment<I: IntoIterator<Item = (Uuid, EquipmentSlots)>>(&self, items: I) {
        {
            let mut m = self.equip.write().unwrap();
            for (id, eq) in items {
                m.insert(id, eq);
            }
        }
        self.persist_jsonl();
    }

    /// 把線上已登入玩家的背包批次 upsert 到 Postgres（遊戲迴圈每 ~10 秒呼叫）。
    pub async fn flush_online(&self, rows: &[(Uuid, Inventory)]) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        if let Err(e) = upsert_inv_rows(pool, rows).await {
            tracing::warn!("Postgres 背包 flush_online 失敗（下一輪再試）：{e}");
        }
    }

    /// 玩家離線時把其最後背包 upsert 到 Postgres。
    pub async fn flush_one(&self, id: Uuid, inv: &Inventory) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        let row = [(id, inv.clone())];
        if let Err(e) = upsert_inv_rows(pool, &row).await {
            tracing::warn!("Postgres 背包 flush_one 失敗：{e}");
        }
    }

    /// 批次 upsert 裝備槽到 Postgres（遊戲迴圈每 ~10 秒呼叫）。
    pub async fn flush_equipment_online(&self, rows: &[(Uuid, EquipmentSlots)]) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        if let Err(e) = upsert_equip_rows(pool, rows).await {
            tracing::warn!("Postgres 裝備槽 flush_equipment_online 失敗（下一輪再試）：{e}");
        }
    }

    /// 玩家離線時把其裝備槽 upsert 到 Postgres。
    pub async fn flush_equipment_one(&self, id: Uuid, eq: &EquipmentSlots) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        let row = [(id, eq.clone())];
        if let Err(e) = upsert_equip_rows(pool, &row).await {
            tracing::warn!("Postgres 裝備槽 flush_equipment_one 失敗：{e}");
        }
    }

    /// Jsonl 模式才寫：把整份 cache（背包＋裝備槽）快照覆寫到磁碟。
    fn persist_jsonl(&self) {
        let Backend::Jsonl(path) = self.backend else {
            return;
        };
        let rows: Vec<String> = {
            let inv_map = self.inner.read().unwrap();
            let eq_map = self.equip.read().unwrap();
            inv_map
                .iter()
                .filter_map(|(id, inv)| {
                    serde_json::to_string(&DiskRow {
                        id: *id,
                        inv: inv.clone(),
                        equipment: eq_map.get(id).cloned(),
                    })
                    .ok()
                })
                .collect()
        };
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = format!("{path}.tmp");
        if let Ok(mut f) = std::fs::File::create(&tmp) {
            let _ = f.write_all(rows.join("\n").as_bytes());
            let _ = f.write_all(b"\n");
            let _ = f.sync_all();
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// 批次 upsert 背包到 `inventories` 表。走 runtime query API（非 `query!`），build/test 免 live DB。
async fn upsert_inv_rows(pool: &PgPool, rows: &[(Uuid, Inventory)]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (id, inv) in rows {
        let Ok(json) = serde_json::to_string(inv) else { continue; };
        sqlx::query(
            "INSERT INTO inventories (player_id, items, updated_at) \
             VALUES ($1, $2, now()) \
             ON CONFLICT (player_id) DO UPDATE SET \
               items = EXCLUDED.items, updated_at = now()",
        )
        .bind(id)
        .bind(json)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// 批次 upsert 裝備槽到 `inventories` 表的 `equipment` 欄（ROADMAP 36）。
async fn upsert_equip_rows(pool: &PgPool, rows: &[(Uuid, EquipmentSlots)]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (id, eq) in rows {
        let Ok(json) = serde_json::to_string(eq) else { continue; };
        sqlx::query(
            "INSERT INTO inventories (player_id, items, equipment, updated_at) \
             VALUES ($1, '{}', $2, now()) \
             ON CONFLICT (player_id) DO UPDATE SET \
               equipment = EXCLUDED.equipment, updated_at = now()",
        )
        .bind(id)
        .bind(json)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// 啟動時把 `inventories` 表載回 cache（背包 + 裝備槽兩個 map）。
/// 壞背包（被竄改存檔）跳過；裝備槽欄位缺席（舊版 DB）回 `None`，之後由連線層自動遷移。
async fn load_inventories_from_db(
    pool: &PgPool,
) -> (HashMap<Uuid, Inventory>, HashMap<Uuid, EquipmentSlots>) {
    let mut inv_map = HashMap::new();
    let mut eq_map = HashMap::new();
    let rows = match sqlx::query("SELECT player_id, items, equipment FROM inventories")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("從 Postgres 載入 inventories 失敗（先以空 cache 起來）：{e}");
            return (inv_map, eq_map);
        }
    };
    for r in rows {
        let id: Uuid = r.get("player_id");
        let items: String = r.get("items");
        if let Ok(inv) = serde_json::from_str::<Inventory>(&items) {
            if inv.is_loadable() {
                inv_map.insert(id, inv);
            }
        }
        // 裝備槽欄位為 NULL（舊版 DB）或解析失敗時靜默跳過，連線層補自動裝備。
        let eq_json: Option<String> = r.try_get("equipment").ok().flatten();
        if let Some(json) = eq_json {
            if let Ok(eq) = serde_json::from_str::<EquipmentSlots>(&json) {
                eq_map.insert(id, eq);
            }
        }
    }
    (inv_map, eq_map)
}

fn load_from_disk(path: &str) -> (HashMap<Uuid, Inventory>, HashMap<Uuid, EquipmentSlots>) {
    let mut inv_map = HashMap::new();
    let mut eq_map = HashMap::new();
    if let Ok(contents) = std::fs::read_to_string(path) {
        for line in contents.lines() {
            if let Ok(r) = serde_json::from_str::<DiskRow>(line) {
                if r.inv.is_loadable() {
                    inv_map.insert(r.id, r.inv);
                }
                if let Some(eq) = r.equipment {
                    eq_map.insert(r.id, eq);
                }
            }
        }
    }
    (inv_map, eq_map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::ItemKind;

    fn inv_of(wood: u32, stone: u32) -> Inventory {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, wood);
        inv.add(ItemKind::Stone, stone);
        inv
    }

    #[test]
    fn recall_is_none_before_remember() {
        let store = InventoryStore::in_memory();
        assert_eq!(store.recall(Uuid::new_v4()), None);
    }

    #[test]
    fn remember_then_recall_round_trips() {
        let store = InventoryStore::in_memory();
        let id = Uuid::new_v4();
        let inv = inv_of(3, 2);
        store.remember(id, &inv);
        assert_eq!(store.recall(id), Some(inv));
    }

    #[test]
    fn remember_overwrites_previous_inventory() {
        let store = InventoryStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, &inv_of(1, 0));
        store.remember(id, &inv_of(5, 7));
        assert_eq!(store.recall(id), Some(inv_of(5, 7)));
    }

    #[test]
    fn remember_all_updates_many_at_once() {
        let store = InventoryStore::in_memory();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store.remember_all([(a, inv_of(2, 0)), (b, inv_of(0, 9))]);
        assert_eq!(store.recall(a), Some(inv_of(2, 0)));
        assert_eq!(store.recall(b), Some(inv_of(0, 9)));
    }

    #[test]
    fn stores_are_independent_per_player() {
        let store = InventoryStore::in_memory();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store.remember(a, &inv_of(1, 1));
        assert_eq!(store.recall(b), None);
        assert_eq!(store.recall(a), Some(inv_of(1, 1)));
    }

    #[test]
    fn empty_inventory_round_trips() {
        // 空背包也要能存回（避免「沒記錄」與「記錄為空」混淆）。
        let store = InventoryStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, &Inventory::new());
        assert_eq!(store.recall(id), Some(Inventory::new()));
    }

    #[test]
    fn disk_row_serde_round_trips() {
        let row = DiskRow {
            id: Uuid::new_v4(),
            inv: inv_of(7, 3),
            equipment: None,
        };
        let json = serde_json::to_string(&row).unwrap();
        let back: DiskRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, row.id);
        assert_eq!(back.inv, row.inv);
        assert_eq!(back.equipment, None);
    }

    #[test]
    fn disk_row_with_equipment_round_trips() {
        use crate::equipment::{EquipmentSlots, equip};
        let mut eq = EquipmentSlots::default();
        equip(&mut eq, ItemKind::CrystalBlade);
        let row = DiskRow {
            id: Uuid::new_v4(),
            inv: inv_of(2, 1),
            equipment: Some(eq.clone()),
        };
        let json = serde_json::to_string(&row).unwrap();
        let back: DiskRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.equipment, Some(eq));
    }

    #[test]
    fn corrupt_disk_row_is_skipped_on_load() {
        // load_from_disk 對壞背包（0 條目）應跳過,不帶進 cache。這裡直接驗 is_loadable 防線——
        // load_from_disk 走檔案 IO,以反序列化出的壞背包確認過濾條件與其一致。
        let corrupt = serde_json::from_str::<Inventory>(r#"{"items":{"wood":0}}"#).unwrap();
        assert!(!corrupt.is_loadable());
        let healthy = serde_json::from_str::<Inventory>(r#"{"items":{"wood":3}}"#).unwrap();
        assert!(healthy.is_loadable());
    }

    #[tokio::test]
    async fn flush_is_noop_without_postgres() {
        let store = InventoryStore::in_memory();
        let id = Uuid::new_v4();
        store.flush_online(&[(id, inv_of(1, 2))]).await;
        store.flush_one(id, &inv_of(1, 2)).await;
        let eq = crate::equipment::EquipmentSlots::default();
        store.flush_equipment_online(&[(id, eq.clone())]).await;
        store.flush_equipment_one(id, &eq).await;
        assert_eq!(store.recall(id), None);
        assert_eq!(store.recall_equipment(id), None);
    }

    #[test]
    fn remember_equipment_round_trips() {
        use crate::equipment::{EquipmentSlots, equip};
        let store = InventoryStore::in_memory();
        let id = Uuid::new_v4();
        let mut eq = EquipmentSlots::default();
        equip(&mut eq, ItemKind::CrystalBlade);
        store.remember_equipment(id, &eq);
        assert_eq!(store.recall_equipment(id), Some(eq));
    }
}
