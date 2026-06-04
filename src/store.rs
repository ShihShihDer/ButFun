//! 玩家位置持久化(Phase 0-E)。
//!
//! 把持久化藏在 `state.rs` 之後的抽換點:有 `DATABASE_URL` 就用 Postgres,沒有就
//! 退回純記憶體(本機開發 / 測試)。上層(ws / game)一律呼叫同一組 async API,
//! 不必知道底層是 DB 還是記憶體。之後農地 / 背包等也接這層。

use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use uuid::Uuid;

use crate::state::Player;

/// 從持久層載回的玩家位置(目前 0-E 只還原座標;之後背包 / 農地再擴欄)。
#[derive(Debug, Clone, Copy)]
pub struct PersistedPos {
    pub x: f32,
    pub y: f32,
}

/// 玩家持久化儲存層。`pool = None` 即純記憶體模式(無 `DATABASE_URL`)。
#[derive(Clone)]
pub struct PlayerStore {
    pool: Option<PgPool>,
}

impl PlayerStore {
    /// 純記憶體(不持久化)。Default 與測試用。
    pub fn memory() -> Self {
        Self { pool: None }
    }

    /// 讀 `DATABASE_URL`:有就連 Postgres 並建表;沒有就退回記憶體模式。
    /// 連線 / 建表失敗時記錄錯誤並退回記憶體,確保伺服器仍能起得來(不因 DB 掛了拖垮遊戲)。
    pub async fn connect() -> Self {
        let url = match std::env::var("DATABASE_URL") {
            Ok(u) if !u.trim().is_empty() => u,
            _ => {
                tracing::warn!("未設 DATABASE_URL;玩家位置走純記憶體模式(重啟不保留)");
                return Self::memory();
            }
        };
        match PgPoolOptions::new().max_connections(5).connect(&url).await {
            Ok(pool) => match run_migration(&pool).await {
                Ok(()) => {
                    tracing::info!("Postgres 玩家持久化已啟用");
                    Self { pool: Some(pool) }
                }
                Err(e) => {
                    tracing::error!("建立 players 表失敗,退回記憶體模式:{e}");
                    Self::memory()
                }
            },
            Err(e) => {
                tracing::error!("連線 Postgres 失敗,退回記憶體模式:{e}");
                Self::memory()
            }
        }
    }

    /// 是否有實際持久化(供啟動 log / 測試判斷)。
    pub fn is_persistent(&self) -> bool {
        self.pool.is_some()
    }

    /// 載入某玩家的持久位置;無 DB 或查無資料 → None。
    pub async fn load(&self, id: Uuid) -> Option<PersistedPos> {
        let pool = self.pool.as_ref()?;
        let row = sqlx::query("SELECT x, y FROM players WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| tracing::warn!("讀取玩家 {id} 失敗:{e}"))
            .ok()??;
        Some(PersistedPos {
            // DB 用 DOUBLE PRECISION(f64);遊戲內部用 f32。
            x: row.get::<f64, _>("x") as f32,
            y: row.get::<f64, _>("y") as f32,
        })
    }

    /// 寫回某玩家位置(upsert,同 id 覆蓋);無 DB 為 no-op。
    pub async fn save(&self, p: &Player) {
        let Some(pool) = self.pool.as_ref() else {
            return;
        };
        let res = sqlx::query(
            "INSERT INTO players (id, name, species, x, y, updated_at) \
             VALUES ($1, $2, $3, $4, $5, now()) \
             ON CONFLICT (id) DO UPDATE SET \
               name = EXCLUDED.name, species = EXCLUDED.species, \
               x = EXCLUDED.x, y = EXCLUDED.y, updated_at = now()",
        )
        .bind(p.id)
        .bind(&p.name)
        .bind(&p.species)
        .bind(p.x as f64)
        .bind(p.y as f64)
        .execute(pool)
        .await;
        if let Err(e) = res {
            tracing::warn!("寫回玩家 {} 失敗:{e}", p.id);
        }
    }
}

impl Default for PlayerStore {
    fn default() -> Self {
        Self::memory()
    }
}

/// 建立 `players` 表(向後相容:`IF NOT EXISTS`,不動既有資料)。
/// 之後農地 / 背包欄位用後續 migration 疊加,不 drop 既有欄位。
async fn run_migration(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS players (\
            id UUID PRIMARY KEY,\
            name TEXT NOT NULL,\
            species TEXT NOT NULL,\
            x DOUBLE PRECISION NOT NULL,\
            y DOUBLE PRECISION NOT NULL,\
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now()\
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

// ============= 純邏輯 / 契約測試(無 DB) =============
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_is_not_persistent() {
        assert!(!PlayerStore::memory().is_persistent());
        assert!(!PlayerStore::default().is_persistent());
    }

    #[tokio::test]
    async fn memory_load_returns_none() {
        let store = PlayerStore::memory();
        assert!(store.load(Uuid::new_v4()).await.is_none());
    }

    #[tokio::test]
    async fn memory_save_is_noop() {
        // 記憶體模式 save 不該 panic、不需 DB,直接無聲返回。
        let store = PlayerStore::memory();
        let p = Player {
            id: Uuid::new_v4(),
            name: "測試".into(),
            species: "terran".into(),
            x: 1.0,
            y: 2.0,
            input: Default::default(),
        };
        store.save(&p).await; // 不 panic 即通過
    }
}
