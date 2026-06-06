//! 日夜時鐘的持久化層（Phase 0-E，沿 `positions.rs`／`inventory_store.rs` 同一套抽換結構）。
//!
//! 日夜循環（`daynight.rs`）目前活在 `AppState.daynight` 的記憶體裡，每次伺服器重啟（換版）
//! 都歸零回破曉——0-G 全程反覆標注「仍待：日夜時刻持久化（接 0-E）」。這層讓世界時刻撐得過
//! 重啟：啟動時把上次存的時刻載回去種給 `AppState` 的時鐘，遊戲迴圈再定期把當下時刻落地。
//!
//! 與位置／背包／農地不同：日夜是**單一全域時鐘**、不分玩家，故這個 store 沒有 per-id 的
//! cache map——權威時鐘就是 `AppState.daynight`，這層只負責「啟動載入一次」與「定期 flush」。
//! 耐久層在後面可抽換（延續其他 store 的設計權衡：DB 為主、JSONL 補洞；flush 失敗只記 log
//! 不中斷遊戲迴圈；載入時一律經 `DayNight::at` 驗壞值）：
//!   - `Postgres`：設了 `DATABASE_URL` 時，啟動載回、定期非同步 upsert（正式上線走這條）。
//!   - `Jsonl`：沒設 `DATABASE_URL`（本機 `cargo run`）時寫穿 `data/daynight.jsonl`。
//!   - `Memory`：測試用，不碰磁碟也不碰 DB。

use std::io::Write;

use sqlx::postgres::PgPool;
use sqlx::Row;

use crate::daynight::DayNight;

/// 無 `DATABASE_URL` 時的退回持久化檔（執行期產生、已 gitignore）。接 Postgres 後這個檔
/// 退為「遷移種子」：啟動時 DB 還沒有時間列就從這裡補回，讓換版不會把世界時刻洗回破曉。
const STORE_PATH: &str = "data/daynight.jsonl";

/// singleton 列的固定主鍵（見 `migrations/0004_daynight.sql` 的 `CHECK (id = 1)`）。
const SINGLETON_ID: i32 = 1;

/// cache 後面的耐久層。
#[derive(Clone)]
enum Backend {
    /// 測試用：不載入、不寫。只在 `#[cfg(test)]` 的 `in_memory()` 建構，故非測試建置標 allow。
    #[cfg_attr(not(test), allow(dead_code))]
    Memory,
    /// 沒設 `DATABASE_URL`：寫穿到此 JSONL 檔。
    Jsonl(&'static str),
    /// 設了 `DATABASE_URL`：啟動載回、定期非同步 upsert 到 `daynight` 表（singleton 一列）。
    Postgres(PgPool),
}

/// 記住世界日夜時刻的儲存層。載入時把時刻種給 `AppState` 的權威時鐘，之後定期 flush。
#[derive(Clone)]
pub struct DayNightStore {
    backend: Backend,
    /// 啟動時載入的時刻（供 `AppState::with_stores` 種回權威 `DayNight`）。沒有存檔時為破曉。
    loaded: DayNight,
}

impl Default for DayNightStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DayNightStore {
    /// 無 DB 模式：從 JSONL 載入上次時刻（沒有就破曉），之後 flush 寫穿 JSONL。
    pub fn new() -> Self {
        Self {
            backend: Backend::Jsonl(STORE_PATH),
            loaded: load_from_disk(STORE_PATH).unwrap_or_default(),
        }
    }

    /// Postgres 模式（正式上線）：啟動時把 `daynight` 表的 singleton 列載回；DB 還沒有列時用
    /// 既有 JSONL 補（換版從 JSONL 版切到 Postgres 版時 DB 還空，不從 JSONL 種回會洗回破曉）。
    /// 這個「DB 為主、JSONL 補洞」的順序與 `PositionStore::from_pool` 等一致。
    pub async fn from_pool(pool: PgPool) -> Self {
        let loaded = match load_from_db(&pool).await {
            Some(dn) => dn,
            None => load_from_disk(STORE_PATH).unwrap_or_default(),
        };
        Self {
            backend: Backend::Postgres(pool),
            loaded,
        }
    }

    /// 純記憶體版（測試用）：不載入、不寫磁碟、不碰 DB；種子為破曉。
    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            backend: Backend::Memory,
            loaded: DayNight::new(),
        }
    }

    /// 啟動時載入的時刻，供 `AppState` 種回權威時鐘。
    pub fn loaded(&self) -> DayNight {
        self.loaded
    }

    /// 把當下世界時刻落地（遊戲迴圈每 ~10 秒呼叫一次）。Jsonl 模式寫穿磁碟；Postgres 模式
    /// upsert singleton 列；Memory 無動作。失敗只記 log、不中斷遊戲迴圈（下一輪再試；權威
    /// 時鐘仍在 `AppState.daynight` 的記憶體裡）。
    pub async fn flush(&self, dn: &DayNight) {
        match &self.backend {
            Backend::Memory => {}
            Backend::Jsonl(path) => persist_jsonl(path, dn),
            Backend::Postgres(pool) => {
                if let Err(e) = upsert_elapsed(pool, dn.elapsed()).await {
                    tracing::warn!("Postgres 日夜時刻 flush 失敗（下一輪再試）：{e}");
                }
            }
        }
    }
}

/// upsert singleton 列（固定 `id = 1`）。走 runtime query API（非 `query!` 巨集），故 build/test
/// 不需 live DB。
async fn upsert_elapsed(pool: &PgPool, elapsed: f32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO daynight (id, elapsed, updated_at) \
         VALUES ($1, $2, now()) \
         ON CONFLICT (id) DO UPDATE SET \
           elapsed = EXCLUDED.elapsed, updated_at = now()",
    )
    .bind(SINGLETON_ID)
    .bind(elapsed)
    .execute(pool)
    .await?;
    Ok(())
}

/// 啟動時把 `daynight` 表的 singleton 列載回。一律經 `DayNight::at` 還原並驗證（非有限退回
/// 破曉、界外／負值繞回），壞值不會把時鐘帶成 NaN／界外。沒有列（首次／剛 migrate）或載入
/// 失敗（DB 連線剛斷等）回 `None`，呼叫端退回 JSONL 種子或破曉。
async fn load_from_db(pool: &PgPool) -> Option<DayNight> {
    let row = match sqlx::query("SELECT elapsed FROM daynight WHERE id = $1")
        .bind(SINGLETON_ID)
        .fetch_optional(pool)
        .await
    {
        Ok(row) => row?,
        Err(e) => {
            tracing::warn!("從 Postgres 載入 daynight 失敗（先以破曉起來）：{e}");
            return None;
        }
    };
    let elapsed: f32 = row.get("elapsed");
    Some(DayNight::at(elapsed))
}

/// Jsonl 模式才寫：把當下時刻序列化成一行覆寫到磁碟（先寫暫存再 rename，避免寫到一半被
/// 重啟而毀檔，比照 `inventory_store::persist_jsonl`）。
fn persist_jsonl(path: &str, dn: &DayNight) {
    let Ok(line) = serde_json::to_string(dn) else {
        return;
    };
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = format!("{path}.tmp");
    if let Ok(mut f) = std::fs::File::create(&tmp) {
        let _ = f.write_all(line.as_bytes());
        let _ = f.write_all(b"\n");
        let _ = f.sync_all();
        let _ = std::fs::rename(&tmp, path);
    }
}

/// 從 JSONL 載入上次時刻（單行）。反序列化本身已一律經 `DayNight::at` 守門（見 `daynight.rs`
/// 的手動 `Deserialize`），故壞值同樣安全。沒有檔／空檔／壞行回 `None`。
fn load_from_disk(path: &str) -> Option<DayNight> {
    let contents = std::fs::read_to_string(path).ok()?;
    let line = contents.lines().next()?;
    serde_json::from_str::<DayNight>(line).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daynight::DAY_LENGTH_SECS;

    #[test]
    fn loaded_is_dawn_for_fresh_memory_store() {
        // 沒有存檔的純記憶體 store，種子是破曉（fraction 0）。
        let store = DayNightStore::in_memory();
        assert_eq!(store.loaded().fraction(), 0.0);
    }

    #[tokio::test]
    async fn flush_is_noop_without_postgres() {
        // 純記憶體模式（測試）下，flush 不該 panic、也不需 DB。
        let store = DayNightStore::in_memory();
        let mut dn = DayNight::new();
        dn.advance(DAY_LENGTH_SECS * 0.3);
        store.flush(&dn).await; // 不 panic 即可
    }

    #[test]
    fn jsonl_round_trips_through_disk() {
        // 把一個「進行到一半」的時刻寫穿 JSONL，再讀回要落在同一階段／比例。
        // 用行程位址當唯一暫存路徑，避免與其他測試／真實檔互撞。
        let path: &'static str = Box::leak(
            format!("data/test_daynight_{:p}.jsonl", &DAY_LENGTH_SECS).into_boxed_str(),
        );
        let mut dn = DayNight::new();
        dn.advance(DAY_LENGTH_SECS * 0.42);
        persist_jsonl(path, &dn);

        let back = load_from_disk(path).expect("應讀得回剛寫的時刻");
        assert!((back.fraction() - dn.fraction()).abs() < 1e-4);
        assert_eq!(back.phase(), dn.phase());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_from_missing_disk_is_none() {
        // 沒有檔時回 None（呼叫端退回破曉），不 panic。
        assert!(load_from_disk("data/definitely_does_not_exist_daynight.jsonl").is_none());
    }
}
