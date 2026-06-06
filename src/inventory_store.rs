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

use crate::inventory::Inventory;

/// 無 `DATABASE_URL` 時的退回持久化檔（執行期產生、已 gitignore）。接 Postgres 後這個檔
/// 退為「遷移種子」：啟動時 DB 沒有的 id 仍會從這裡補回 cache,讓換版不會把人的背包洗空。
const STORE_PATH: &str = "data/inventories.jsonl";

/// JSONL 一行紀錄：把 id 與整個背包合起來序列化（`inv` 形如 `{"items":{"wood":3}}`）。
#[derive(Serialize, Deserialize)]
struct DiskRow {
    id: Uuid,
    inv: Inventory,
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

/// 記住玩家背包的儲存層。記憶體 cache + 可抽換耐久層（Postgres / JSONL / 純記憶體）。
#[derive(Clone)]
pub struct InventoryStore {
    inner: Arc<RwLock<HashMap<Uuid, Inventory>>>,
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
        Self {
            inner: Arc::new(RwLock::new(load_from_disk(STORE_PATH))),
            backend: Backend::Jsonl(STORE_PATH),
        }
    }

    /// Postgres 模式（正式上線）：啟動時把 `inventories` 表載回 cache,再用既有 JSONL 補齊
    /// DB 還沒有的 id。這個「DB 為主、JSONL 補洞」的順序與 `PositionStore::from_pool` 一致——
    /// 換版（從 JSONL 版切到 Postgres 版）時 DB 可能還空,若不從 JSONL 種回,所有人會被
    /// `recall` miss 洗成空背包。已在 DB 的 id 以 DB 為準,不被舊 JSONL 覆蓋。
    pub async fn from_pool(pool: PgPool) -> Self {
        let mut cache = load_inventories_from_db(&pool).await;
        for (id, inv) in load_from_disk(STORE_PATH) {
            cache.entry(id).or_insert(inv); // DB 沒有的才用 JSONL 補,DB 優先
        }
        Self {
            inner: Arc::new(RwLock::new(cache)),
            backend: Backend::Postgres(pool),
        }
    }

    /// 純記憶體版（測試用）：不載入、不寫磁碟、不碰 DB。
    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            backend: Backend::Memory,
        }
    }

    /// 取出某玩家上次離線時的背包（沒有就 None）。讀 cache,同步。
    pub fn recall(&self, id: Uuid) -> Option<Inventory> {
        self.inner.read().unwrap().get(&id).cloned()
    }

    /// 記住某玩家目前背包（更新 cache,同步）。Jsonl 模式順手寫穿磁碟;Postgres 模式只動
    /// cache,耐久寫入交給非同步的 `flush_online`/`flush_one`（DB 是 async,不在同步路徑上 await）。
    pub fn remember(&self, id: Uuid, inv: &Inventory) {
        self.inner.write().unwrap().insert(id, inv.clone());
        self.persist_jsonl();
    }

    /// 批次記住多名玩家的背包（給遊戲迴圈定期快照線上玩家用）：更新 cache 一次。
    pub fn remember_all<I: IntoIterator<Item = (Uuid, Inventory)>>(&self, items: I) {
        {
            let mut m = self.inner.write().unwrap();
            for (id, inv) in items {
                m.insert(id, inv);
            }
        }
        self.persist_jsonl();
    }

    /// 把線上已登入玩家的背包批次 upsert 到 Postgres（遊戲迴圈每 ~10 秒呼叫）。非 Postgres 模式
    /// 無動作。失敗只記 log、不中斷遊戲迴圈（下一輪再試;cache 仍是行程內權威）。
    pub async fn flush_online(&self, rows: &[(Uuid, Inventory)]) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        if let Err(e) = upsert_rows(pool, rows).await {
            tracing::warn!("Postgres 背包 flush_online 失敗（下一輪再試）：{e}");
        }
    }

    /// 玩家離線時把其最後背包 upsert 到 Postgres（補上「最後一次 10s flush 後到離線之間」的
    /// 採集/合成）。非 Postgres 模式無動作。
    pub async fn flush_one(&self, id: Uuid, inv: &Inventory) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        let row = [(id, inv.clone())];
        if let Err(e) = upsert_rows(pool, &row).await {
            tracing::warn!("Postgres 背包 flush_one 失敗：{e}");
        }
    }

    /// Jsonl 模式才寫：把整份 cache 快照覆寫到磁碟。其餘模式無動作。
    fn persist_jsonl(&self) {
        let Backend::Jsonl(path) = self.backend else {
            return;
        };
        let rows: Vec<String> = {
            let m = self.inner.read().unwrap();
            m.iter()
                .filter_map(|(id, inv)| {
                    serde_json::to_string(&DiskRow {
                        id: *id,
                        inv: inv.clone(),
                    })
                    .ok()
                })
                .collect()
        };
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // 先寫暫存再 rename,避免寫到一半被重啟而毀檔。
        let tmp = format!("{path}.tmp");
        if let Ok(mut f) = std::fs::File::create(&tmp) {
            let _ = f.write_all(rows.join("\n").as_bytes());
            let _ = f.write_all(b"\n");
            let _ = f.sync_all();
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// 批次 upsert 到 `inventories` 表（一筆 transaction,要嘛全進要嘛全不進）。
/// items 存整個背包序列化後的 JSON 字串。走 runtime query API（非 `query!` 巨集），
/// 故 build/test 不需 live DB。
async fn upsert_rows(pool: &PgPool, rows: &[(Uuid, Inventory)]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (id, inv) in rows {
        // 序列化背包；理論上不會失敗（BTreeMap<ItemKind,u32>),萬一失敗就跳過這筆,不毀整批。
        let Ok(json) = serde_json::to_string(inv) else {
            continue;
        };
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

/// 啟動時把 `inventories` 表載回 cache。每筆過 `Inventory::is_loadable` 驗證,
/// 壞檔/被竄改的存檔（0 條目、超過堆疊上限、反序列化失敗）一律跳過,不把壞值帶進世界。
/// 載入失敗（DB 連線剛斷等）回空 map,讓伺服器仍能起來、之後再寫回。
async fn load_inventories_from_db(pool: &PgPool) -> HashMap<Uuid, Inventory> {
    let mut map = HashMap::new();
    let rows = match sqlx::query("SELECT player_id, items FROM inventories")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("從 Postgres 載入 inventories 失敗（先以空 cache 起來）：{e}");
            return map;
        }
    };
    for r in rows {
        let id: Uuid = r.get("player_id");
        let items: String = r.get("items");
        if let Ok(inv) = serde_json::from_str::<Inventory>(&items) {
            if inv.is_loadable() {
                map.insert(id, inv);
            }
        }
    }
    map
}

fn load_from_disk(path: &str) -> HashMap<Uuid, Inventory> {
    let mut map = HashMap::new();
    if let Ok(contents) = std::fs::read_to_string(path) {
        for line in contents.lines() {
            if let Ok(r) = serde_json::from_str::<DiskRow>(line) {
                // 同 DB 載入:壞背包（被竄改的存檔）一律跳過,不帶進世界。
                if r.inv.is_loadable() {
                    map.insert(r.id, r.inv);
                }
            }
        }
    }
    map
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
        // JSONL/DB 共用的序列化格式地基:id + 整個背包序列化再讀回要一模一樣。
        let row = DiskRow {
            id: Uuid::new_v4(),
            inv: inv_of(7, 3),
        };
        let json = serde_json::to_string(&row).unwrap();
        let back: DiskRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, row.id);
        assert_eq!(back.inv, row.inv);
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
        // 非 Postgres 模式（測試）下,flush_* 不該 panic、也不需 DB。
        let store = InventoryStore::in_memory();
        let id = Uuid::new_v4();
        store.flush_online(&[(id, inv_of(1, 2))]).await;
        store.flush_one(id, &inv_of(1, 2)).await;
        // cache 不受 flush 影響（flush 只負責耐久寫出,不改 cache）。
        assert_eq!(store.recall(id), None);
    }
}
