//! NPC 個人記憶與送禮餘裕的持久化層（ROADMAP 60）。
//!
//! - Postgres 模式（設了 `DATABASE_URL`）：啟動時載入全部記憶與餘裕；
//!   每次對話後 fire-and-forget upsert。
//! - 記憶體模式（測試 / 本機無 DB）：重啟後清空，行為正確但不跨重啟。

use std::collections::HashMap;
use uuid::Uuid;
use crate::npc_chat::NpcRel;

#[derive(Clone)]
enum Backend {
    Memory,
    Postgres(sqlx::postgres::PgPool),
}

/// NPC 個人記憶與送禮餘裕的持久化 store（Clone 安全，內含 Arc）。
#[derive(Clone)]
pub struct NpcMemoryStore {
    backend: Backend,
    loaded_memory: Vec<(Uuid, String, NpcRel)>,
    loaded_stock: HashMap<String, u32>,
}

impl Default for NpcMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl NpcMemoryStore {
    /// 記憶體模式（測試 / 無 DB）。
    pub fn new() -> Self {
        Self {
            backend: Backend::Memory,
            loaded_memory: Vec::new(),
            loaded_stock: HashMap::new(),
        }
    }

    /// Postgres 模式：啟動時載入全部 npc_memory 與 npc_gift_stock。
    pub async fn from_pool(pool: sqlx::postgres::PgPool) -> Self {
        let loaded_memory = load_memory(&pool).await;
        let loaded_stock = load_gift_stock(&pool).await;
        Self { backend: Backend::Postgres(pool), loaded_memory, loaded_stock }
    }

    /// 取回啟動時載入的個人記憶，供重建 npc_memory HashMap。
    pub fn saved_memory(&self) -> Vec<(Uuid, String, NpcRel)> {
        self.loaded_memory.clone()
    }

    /// 取回啟動時載入的送禮餘裕，供重建 npc_gift_stock HashMap。
    pub fn saved_gift_stock(&self) -> HashMap<String, u32> {
        self.loaded_stock.clone()
    }

    /// 對話後立刻持久化某玩家對某 NPC 的關係（fire-and-forget；失敗只記 log）。
    pub fn save_rel(&self, player_id: Uuid, npc_id: String, rel: NpcRel) {
        match &self.backend {
            Backend::Memory => {}
            Backend::Postgres(pool) => {
                let pool = pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = upsert_rel(&pool, player_id, &npc_id, &rel).await {
                        tracing::error!(%e, %player_id, npc_id, "npc_memory upsert 失敗");
                    }
                });
            }
        }
    }

    /// NPC 餘裕變動後立刻持久化（fire-and-forget；失敗只記 log）。
    pub fn save_gift_stock(&self, npc_id: String, stock: u32) {
        match &self.backend {
            Backend::Memory => {}
            Backend::Postgres(pool) => {
                let pool = pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = upsert_gift_stock(&pool, &npc_id, stock).await {
                        tracing::error!(%e, npc_id, "npc_gift_stock upsert 失敗");
                    }
                });
            }
        }
    }
}

async fn load_memory(pool: &sqlx::postgres::PgPool) -> Vec<(Uuid, String, NpcRel)> {
    let rows = sqlx::query(
        "SELECT player_id, npc_id, impression, talks, gifted, sell_count, buy_count FROM npc_memory",
    )
    .fetch_all(pool)
    .await;
    match rows {
        Ok(rows) => rows
            .iter()
            .map(|r| {
                let player_id: Uuid = sqlx::Row::try_get(r, "player_id").unwrap_or(Uuid::nil());
                let npc_id: String = sqlx::Row::try_get(r, "npc_id").unwrap_or_default();
                let impression: String = sqlx::Row::try_get(r, "impression").unwrap_or_default();
                let talks: i32 = sqlx::Row::try_get(r, "talks").unwrap_or(0);
                let gifted: bool = sqlx::Row::try_get(r, "gifted").unwrap_or(false);
                let sell_count: i32 = sqlx::Row::try_get(r, "sell_count").unwrap_or(0);
                let buy_count: i32 = sqlx::Row::try_get(r, "buy_count").unwrap_or(0);
                (
                    player_id,
                    npc_id,
                    NpcRel {
                        impression,
                        talks: talks as u32,
                        gifted,
                        sell_count: sell_count.max(0) as u32,
                        buy_count: buy_count.max(0) as u32,
                    },
                )
            })
            .collect(),
        Err(e) => {
            tracing::error!(%e, "載入 npc_memory 失敗");
            Vec::new()
        }
    }
}

async fn load_gift_stock(pool: &sqlx::postgres::PgPool) -> HashMap<String, u32> {
    let rows = sqlx::query("SELECT npc_id, stock FROM npc_gift_stock")
        .fetch_all(pool)
        .await;
    match rows {
        Ok(rows) => rows
            .iter()
            .map(|r| {
                let npc_id: String = sqlx::Row::try_get(r, "npc_id").unwrap_or_default();
                let stock: i32 = sqlx::Row::try_get(r, "stock").unwrap_or(0);
                (npc_id, stock.max(0) as u32)
            })
            .collect(),
        Err(e) => {
            tracing::error!(%e, "載入 npc_gift_stock 失敗");
            HashMap::new()
        }
    }
}

async fn upsert_rel(
    pool: &sqlx::postgres::PgPool,
    player_id: Uuid,
    npc_id: &str,
    rel: &NpcRel,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO npc_memory (player_id, npc_id, impression, talks, gifted, sell_count, buy_count)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (player_id, npc_id) DO UPDATE
         SET impression  = EXCLUDED.impression,
             talks       = EXCLUDED.talks,
             gifted      = EXCLUDED.gifted,
             sell_count  = EXCLUDED.sell_count,
             buy_count   = EXCLUDED.buy_count",
    )
    .bind(player_id)
    .bind(npc_id)
    .bind(&rel.impression)
    .bind(rel.talks as i32)
    .bind(rel.gifted)
    .bind(rel.sell_count as i32)
    .bind(rel.buy_count as i32)
    .execute(pool)
    .await?;
    Ok(())
}

async fn upsert_gift_stock(
    pool: &sqlx::postgres::PgPool,
    npc_id: &str,
    stock: u32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO npc_gift_stock (npc_id, stock) VALUES ($1, $2)
         ON CONFLICT (npc_id) DO UPDATE SET stock = EXCLUDED.stock",
    )
    .bind(npc_id)
    .bind(stock as i32)
    .execute(pool)
    .await?;
    Ok(())
}
