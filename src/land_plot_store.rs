//! 城外地塊產權的持久化層（ROADMAP 34/35）。
//!
//! 只存「誰買了哪塊、用途是什麼」：`(plot_id INTEGER, owner_id UUID, purpose TEXT)` 的稀疏表。
//! - Postgres 模式（設了 `DATABASE_URL`）：起動載入、購買時立刻非同步 upsert。
//! - 記憶體模式（測試 / 本機無 DB）：重啟後清空（地塊未購狀態），行為正確但不跨重啟。

use uuid::Uuid;
use crate::land_plot::PlotPurpose;

#[cfg(not(test))]
use sqlx::postgres::PgPool;
#[cfg(test)]
#[allow(unused_imports)]
use sqlx::postgres::PgPool;

#[derive(Clone)]
enum Backend {
    Memory,
    Postgres(sqlx::postgres::PgPool),
}

/// 城外地塊產權的持久化 store（Clone 安全，內含 Arc）。
#[derive(Clone)]
pub struct LandPlotStore {
    backend: Backend,
    /// 啟動時從 DB 載入的歸屬紀錄，供重建 `LandPlotRegistry`（含用途）。
    loaded: Vec<(u32, Uuid, PlotPurpose)>,
}

impl Default for LandPlotStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LandPlotStore {
    /// 記憶體模式（測試 / 無 DB）。
    pub fn new() -> Self {
        Self { backend: Backend::Memory, loaded: Vec::new() }
    }

    /// Postgres 模式：啟動時把 `land_plots` 表所有紀錄載入（含 purpose）。
    pub async fn from_pool(pool: sqlx::postgres::PgPool) -> Self {
        let loaded = load_from_db(&pool).await;
        Self { backend: Backend::Postgres(pool), loaded }
    }

    /// 取回啟動時載入的歸屬紀錄，供重建 `LandPlotRegistry`（ROADMAP 35：含用途）。
    pub fn saved_ownerships(&self) -> Vec<(u32, Uuid, PlotPurpose)> {
        self.loaded.clone()
    }

    /// 玩家購買地塊後立刻持久化（fire-and-forget；失敗只記 log）。
    pub fn save_purchase(&self, plot_id: u32, owner_id: Uuid, purpose: PlotPurpose) {
        match &self.backend {
            Backend::Memory => {}
            Backend::Postgres(pool) => {
                let pool = pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = upsert_ownership(&pool, plot_id, owner_id, purpose).await {
                        tracing::error!(%e, plot_id, "地塊產權 upsert 失敗");
                    }
                });
            }
        }
    }
}

async fn load_from_db(pool: &sqlx::postgres::PgPool) -> Vec<(u32, Uuid, PlotPurpose)> {
    let rows = sqlx::query("SELECT plot_id, owner_id, purpose FROM land_plots")
        .fetch_all(pool)
        .await;
    match rows {
        Ok(rows) => rows.iter().map(|r| {
            let plot_id: i32 = sqlx::Row::try_get(r, "plot_id").unwrap_or(0);
            let owner_id: Uuid = sqlx::Row::try_get(r, "owner_id").unwrap_or(Uuid::nil());
            let purpose_str: String = sqlx::Row::try_get(r, "purpose").unwrap_or_default();
            let purpose = PlotPurpose::from_str(&purpose_str);
            (plot_id as u32, owner_id, purpose)
        }).collect(),
        Err(e) => {
            tracing::error!(%e, "載入地塊產權失敗");
            Vec::new()
        }
    }
}

async fn upsert_ownership(
    pool: &sqlx::postgres::PgPool,
    plot_id: u32,
    owner_id: Uuid,
    purpose: PlotPurpose,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO land_plots (plot_id, owner_id, purpose) VALUES ($1, $2, $3) \
         ON CONFLICT (plot_id) DO UPDATE SET owner_id = EXCLUDED.owner_id, purpose = EXCLUDED.purpose"
    )
    .bind(plot_id as i32)
    .bind(owner_id)
    .bind(purpose.as_str())
    .execute(pool)
    .await?;
    Ok(())
}
