//! 玩家最後狀態（位置 + 乙太）的伺服器端記憶（Phase 0-E 持久化）。
//!
//! 行程內維護一份 `id -> Saved` 的記憶體 cache 當權威來源,讓 `recall`/`remember` 保持
//! **同步**、不污染 WebSocket / 遊戲迴圈的鎖序。耐久層在 cache 後面可抽換：
//!   - `Postgres`：設了 `DATABASE_URL` 時，啟動載回、定期非同步 upsert（正式上線走這條）。
//!   - `Jsonl`：沒設 `DATABASE_URL`（本機 `cargo run`）時寫穿 `data/positions.jsonl`。
//!   - `Memory`：測試用，不碰磁碟也不碰 DB。
//!
//! 注意：只記「已登入」玩家（穩定 id）；訪客每次連線 id 隨機、記了也對不上,
//! 故不記,避免 cache 無界成長。

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use sqlx::Row;
use uuid::Uuid;

use crate::state::{WORLD_HEIGHT, WORLD_WIDTH};

/// 無 `DATABASE_URL` 時的退回持久化檔（執行期產生、已 gitignore）。接 Postgres 後這個檔
/// 退為「遷移種子」：啟動時 DB 沒有的 id 仍會從這裡補回 cache,讓換版不會把人洗回中央。
const STORE_PATH: &str = "data/positions.jsonl";

/// JSONL 一行紀錄：把 id 與 `Saved` 合起來序列化。
#[derive(Serialize, Deserialize)]
struct DiskRow {
    id: Uuid,
    x: f32,
    y: f32,
    ether: u32,
}

/// 玩家進場時的預設位置（地圖中央）。沒有歷史位置時用它。
pub fn default_spawn() -> (f32, f32) {
    (WORLD_WIDTH / 2.0, WORLD_HEIGHT / 2.0)
}

/// 依「是否有記住的歷史位置」決定進場座標。純函式，便於測試。
///
/// 契約：回傳的座標一定有限。這層刻意防住「載入被竄改/壞掉的
/// 持久化位置」——Postgres 的 `REAL` 欄位可能存進 `NaN`/`Inf`,
/// 不檢查就可能讓玩家座標變非有限。非有限一律退回地圖中央。
/// 在無限世界中，有限的「界外」座標應原樣保留。
pub fn spawn_at(recalled: Option<(f32, f32)>) -> (f32, f32) {
    match recalled {
        Some((x, y)) if x.is_finite() && y.is_finite() => (x, y),
        _ => default_spawn(),
    }
}

/// 某玩家離線時記下的最後狀態：位置 + 收成累積的乙太。
///
/// 載入時的防線沿用既有入口、不在此重複：位置一律經 `spawn_at` 驗證（非有限退回地圖中央、
/// 界外夾回邊界），`ether` 是 `u32`、型別本身就擋掉 `NaN` / `Inf` / 負值。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub x: f32,
    pub y: f32,
    pub ether: u32,
}

/// cache 後面的耐久層。
#[derive(Clone)]
enum Backend {
    /// 測試用：不載入、不寫。只在 `#[cfg(test)]` 的 `in_memory()` 建構,故非測試建置標 allow。
    #[cfg_attr(not(test), allow(dead_code))]
    Memory,
    /// 沒設 `DATABASE_URL`：寫穿到此 JSONL 檔。
    Jsonl(&'static str),
    /// 設了 `DATABASE_URL`：啟動載回、定期非同步 upsert 到 `players` 表。
    Postgres(PgPool),
}

/// 記住玩家最後狀態的儲存層。記憶體 cache + 可抽換耐久層（Postgres / JSONL / 純記憶體）。
#[derive(Clone)]
pub struct PositionStore {
    inner: Arc<RwLock<HashMap<Uuid, Saved>>>,
    backend: Backend,
}

impl Default for PositionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionStore {
    /// 無 DB 模式：cache 從 JSONL 載入,之後寫穿 JSONL。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(load_from_disk(STORE_PATH))),
            backend: Backend::Jsonl(STORE_PATH),
        }
    }

    /// Postgres 模式（正式上線）：啟動時把 `players` 表載回 cache,再用既有 JSONL 補齊
    /// DB 還沒有的 id。這個「DB 為主、JSONL 補洞」的順序很關鍵——換版（從 JSONL 版切到
    /// Postgres 版）時 DB 可能還是空的,若不從 JSONL 種回,所有人會被 `recall` miss 洗回
    /// 地圖中央、乙太歸零（正是要修掉的 bug）。已在 DB 的 id 以 DB 為準,不被舊 JSONL 覆蓋。
    pub async fn from_pool(pool: PgPool) -> Self {
        let mut cache = load_players_from_db(&pool).await;
        for (id, saved) in load_from_disk(STORE_PATH) {
            cache.entry(id).or_insert(saved); // DB 沒有的才用 JSONL 補,DB 優先
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

    /// 取出某玩家上次離線時的狀態（沒有就 None）。讀 cache,同步。
    pub fn recall(&self, id: Uuid) -> Option<Saved> {
        self.inner.read().unwrap().get(&id).copied()
    }

    /// 記住某玩家目前狀態（更新 cache,同步）。Jsonl 模式順手寫穿磁碟;Postgres 模式只動
    /// cache,耐久寫入交給非同步的 `flush_online`/`flush_one`（DB 是 async,不在同步路徑上 await）。
    pub fn remember(&self, id: Uuid, x: f32, y: f32, ether: u32) {
        self.inner.write().unwrap().insert(id, Saved { x, y, ether });
        self.persist_jsonl();
    }

    /// 批次記住多名玩家（給遊戲迴圈定期快照線上玩家用）：更新 cache 一次。
    pub fn remember_all<I: IntoIterator<Item = (Uuid, f32, f32, u32)>>(&self, items: I) {
        {
            let mut m = self.inner.write().unwrap();
            for (id, x, y, ether) in items {
                m.insert(id, Saved { x, y, ether });
            }
        }
        self.persist_jsonl();
    }

    /// 把線上已登入玩家批次 upsert 到 Postgres（遊戲迴圈每 ~10 秒呼叫）。非 Postgres 模式無動作。
    /// 失敗只記 log、不中斷遊戲迴圈（下一輪再試;cache 仍是行程內權威）。
    pub async fn flush_online(&self, rows: &[(Uuid, String, String, f32, f32, u32)]) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        if let Err(e) = upsert_rows(pool, rows).await {
            tracing::warn!("Postgres flush_online 失敗（下一輪再試）：{e}");
        }
    }

    /// 玩家離線時把其最後狀態 upsert 到 Postgres（補上「最後一次 10s flush 後到離線之間」的
    /// 移動,離線後就不再進線上快照了）。非 Postgres 模式無動作。
    pub async fn flush_one(&self, id: Uuid, name: &str, species: &str, x: f32, y: f32, ether: u32) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        let row = [(id, name.to_string(), species.to_string(), x, y, ether)];
        if let Err(e) = upsert_rows(pool, &row).await {
            tracing::warn!("Postgres flush_one 失敗：{e}");
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
                .filter_map(|(id, s)| {
                    serde_json::to_string(&DiskRow {
                        id: *id,
                        x: s.x,
                        y: s.y,
                        ether: s.ether,
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

/// 批次 upsert 到 `players` 表（一筆 transaction,要嘛全進要嘛全不進）。
/// 走 runtime query API（非 `query!` 巨集），故 build/test 不需 live DB。
async fn upsert_rows(
    pool: &PgPool,
    rows: &[(Uuid, String, String, f32, f32, u32)],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (id, name, species, x, y, ether) in rows {
        sqlx::query(
            "INSERT INTO players (id, name, species, x, y, ether, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, now()) \
             ON CONFLICT (id) DO UPDATE SET \
               name = EXCLUDED.name, species = EXCLUDED.species, \
               x = EXCLUDED.x, y = EXCLUDED.y, ether = EXCLUDED.ether, updated_at = now()",
        )
        .bind(id)
        .bind(name)
        .bind(species)
        .bind(x)
        .bind(y)
        .bind(*ether as i64) // ether 欄位是 BIGINT(i64);u32 一定塞得下、不會溢位
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// 啟動時把 `players` 表載回 cache（只取位置 + 乙太,recall 不需要 name/species）。
/// 位置一律過 `spawn_at` 驗證,DB 即使存進壞值也不會把玩家生到非法位置。
/// 載入失敗（DB 連線剛斷等）回空 map,讓伺服器仍能起來、之後再寫回。
async fn load_players_from_db(pool: &PgPool) -> HashMap<Uuid, Saved> {
    let mut map = HashMap::new();
    let rows = match sqlx::query("SELECT id, x, y, ether FROM players")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("從 Postgres 載入 players 失敗（先以空 cache 起來）：{e}");
            return map;
        }
    };
    for r in rows {
        let id: Uuid = r.get("id");
        let x: f32 = r.get("x");
        let y: f32 = r.get("y");
        let ether: i64 = r.get("ether");
        let (x, y) = spawn_at(Some((x, y)));
        map.insert(
            id,
            Saved {
                x,
                y,
                ether: ether.max(0) as u32, // BIGINT 理論可負;夾回非負再轉 u32
            },
        );
    }
    map
}

fn load_from_disk(path: &str) -> HashMap<Uuid, Saved> {
    let mut map = HashMap::new();
    if let Ok(contents) = std::fs::read_to_string(path) {
        for line in contents.lines() {
            if let Ok(r) = serde_json::from_str::<DiskRow>(line) {
                // 位置經 spawn_at 驗證（壞值退回中央/夾邊界）;ether 型別本身擋壞值。
                let (x, y) = spawn_at(Some((r.x, r.y)));
                map.insert(r.id, Saved { x, y, ether: r.ether });
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_falls_back_to_center_when_no_history() {
        assert_eq!(spawn_at(None), default_spawn());
    }

    #[test]
    fn spawn_uses_recalled_position() {
        assert_eq!(spawn_at(Some((123.0, 456.0))), (123.0, 456.0));
    }

    #[test]
    fn spawn_falls_back_to_center_on_non_finite() {
        // 壞掉的持久化座標（NaN/Inf）不該把玩家生到非有限位置。
        assert_eq!(spawn_at(Some((f32::NAN, 100.0))), default_spawn());
        assert_eq!(spawn_at(Some((100.0, f32::INFINITY))), default_spawn());
        assert_eq!(spawn_at(Some((f32::NEG_INFINITY, f32::NAN))), default_spawn());
    }

    #[test]
    fn spawn_preserves_out_of_bounds_finite_coordinates() {
        // 在無限世界中，界外的有限座標應原樣保留，不被夾回邊界。
        assert_eq!(spawn_at(Some((-50.0, -50.0))), (-50.0, -50.0));
        assert_eq!(
            spawn_at(Some((WORLD_WIDTH + 999.0, WORLD_HEIGHT + 999.0))),
            (WORLD_WIDTH + 999.0, WORLD_HEIGHT + 999.0)
        );
    }

    #[test]
    fn recall_is_none_before_remember() {
        let store = PositionStore::in_memory();
        assert_eq!(store.recall(Uuid::new_v4()), None);
    }

    #[test]
    fn remember_then_recall_round_trips() {
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, 10.0, 20.0, 5);
        assert_eq!(
            store.recall(id),
            Some(Saved {
                x: 10.0,
                y: 20.0,
                ether: 5
            })
        );
    }

    #[test]
    fn remember_overwrites_previous_state() {
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, 10.0, 20.0, 1);
        store.remember(id, 30.0, 40.0, 9);
        assert_eq!(
            store.recall(id),
            Some(Saved {
                x: 30.0,
                y: 40.0,
                ether: 9
            })
        );
    }

    #[test]
    fn recalled_ether_survives_round_trip() {
        // 收成的乙太要能跟著重連回來，不被歸零。
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, 0.0, 0.0, 42);
        assert_eq!(store.recall(id).map(|s| s.ether), Some(42));
    }

    #[test]
    fn saved_round_trips_through_serde() {
        // 持久化格式地基：玩家最後狀態序列化再讀回要一模一樣。
        let s = Saved {
            x: 123.5,
            y: 678.25,
            ether: 7,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Saved = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn loaded_bad_position_still_gated_by_spawn_at() {
        // 即使耐久層存進非有限座標，進場仍一律經 spawn_at 驗證、會退回地圖中央。
        // 有限的「界外」座標則原樣保留。
        let bad = Saved {
            x: f32::INFINITY,
            y: WORLD_HEIGHT + 9999.0,
            ether: 1,
        };
        let (x, y) = spawn_at(Some((bad.x, bad.y)));
        assert_eq!((x, y), default_spawn());

        let out_of_bounds = Saved {
            x: -100.0,
            y: WORLD_HEIGHT + 100.0,
            ether: 1,
        };
        let (x, y) = spawn_at(Some((out_of_bounds.x, out_of_bounds.y)));
        assert_eq!((x, y), (-100.0, WORLD_HEIGHT + 100.0));
    }

    #[test]
    fn stores_are_independent_per_player() {
        let store = PositionStore::in_memory();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store.remember(a, 1.0, 1.0, 3);
        assert_eq!(store.recall(b), None);
        assert_eq!(
            store.recall(a),
            Some(Saved {
                x: 1.0,
                y: 1.0,
                ether: 3
            })
        );
    }

    #[tokio::test]
    async fn flush_is_noop_without_postgres() {
        // 非 Postgres 模式（測試）下,flush_* 不該 panic、也不需 DB。
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        store
            .flush_online(&[(id, "阿巡".into(), "terran".into(), 1.0, 2.0, 3)])
            .await;
        store.flush_one(id, "阿巡", "terran", 1.0, 2.0, 3).await;
        // cache 不受 flush 影響（flush 只負責耐久寫出,不改 cache）。
        assert_eq!(store.recall(id), None);
    }
}
