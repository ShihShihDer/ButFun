//! 城鎮大工程的持久化層（ROADMAP 131）。
//!
//! 存儲 `town_project` 進度與 `town_project_donations` 紀錄。

use uuid::Uuid;
use crate::town_project::{TownProjectState, TownProjectStatus};

#[allow(unused_imports)]
use sqlx::postgres::PgPool;

#[derive(Clone)]
enum Backend {
    Memory,
    Postgres(sqlx::postgres::PgPool),
}

/// 城鎮大工程的持久化 store。
#[derive(Clone)]
pub struct TownProjectStore {
    backend: Backend,
    loaded: Option<TownProjectState>,
}

impl TownProjectStore {
    pub fn new() -> Self {
        Self { backend: Backend::Memory, loaded: None }
    }

    pub async fn from_pool(pool: sqlx::postgres::PgPool) -> Self {
        let loaded = load_project_from_db(&pool).await;
        Self { backend: Backend::Postgres(pool), loaded }
    }

    pub fn saved_project(&self) -> Option<TownProjectState> {
        self.loaded.clone()
    }

    /// 更新工程進度（fire-and-forget）。
    pub fn save_progress(&self, state: TownProjectState) {
        match &self.backend {
            Backend::Memory => {}
            Backend::Postgres(pool) => {
                let pool = pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = upsert_project(&pool, &state).await {
                        tracing::error!(%e, "工程進度 upsert 失敗");
                    }
                });
            }
        }
    }

    /// 紀錄捐獻（fire-and-forget）。
    pub fn save_donation(&self, user_id: Uuid, project_id: String, ether: u32, wood: u32, stone: u32, crystal: u32, score_delta: u32) {
        match &self.backend {
            Backend::Memory => {}
            Backend::Postgres(pool) => {
                let pool = pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = update_donation(&pool, user_id, &project_id, ether, wood, stone, crystal, score_delta).await {
                        tracing::error!(%e, "捐獻紀錄更新失敗");
                    }
                });
            }
        }
    }

    pub async fn load_top_contributors(&self, project_id: &str) -> Vec<(Uuid, String, u32)> {
        match &self.backend {
            Backend::Memory => Vec::new(),
            Backend::Postgres(pool) => {
                let mut top_contributors = Vec::new();
                let contrib_rows = sqlx::query("SELECT d.user_id, u.name, d.total_score FROM town_project_donations d JOIN users u ON d.user_id = u.id WHERE d.project_id = $1 ORDER BY d.total_score DESC LIMIT 5")
                    .bind(project_id)
                    .fetch_all(pool)
                    .await;
                if let Ok(crows) = contrib_rows {
                    use sqlx::Row;
                    for cr in crows {
                        let uid: Uuid = cr.try_get("user_id").unwrap_or(Uuid::nil());
                        let name: String = cr.try_get("name").unwrap_or_else(|_| "Unknown".to_string());
                        let score: i32 = cr.try_get("total_score").unwrap_or(0);
                        top_contributors.push((uid, name, score as u32));
                    }
                }
                top_contributors
            }
        }
    }
}

async fn load_project_from_db(pool: &sqlx::postgres::PgPool) -> Option<TownProjectState> {
    // 目前只抓取 'observatory'
    let row = sqlx::query("SELECT project_id, name, status, target_ether, current_ether, target_wood, current_wood, target_stone, current_stone, target_crystal, current_crystal FROM town_project WHERE project_id = 'observatory'")
        .fetch_optional(pool)
        .await;

    match row {
        Ok(Some(r)) => {
            use sqlx::Row;
            let status_str: String = r.try_get("status").unwrap_or_else(|_| "building".to_string());
            let status = match status_str.as_str() {
                "planning" => TownProjectStatus::Planning,
                "building" => TownProjectStatus::Building,
                "completed" => TownProjectStatus::Completed,
                _ => TownProjectStatus::Building,
            };

            // 抓取前五大貢獻者
            let mut top_contributors = Vec::new();
            let contrib_rows = sqlx::query("SELECT d.user_id, u.name, d.total_score FROM town_project_donations d JOIN users u ON d.user_id = u.id WHERE d.project_id = 'observatory' ORDER BY d.total_score DESC LIMIT 5")
                .fetch_all(pool)
                .await;
            if let Ok(crows) = contrib_rows {
                for cr in crows {
                    let uid: Uuid = cr.try_get("user_id").unwrap_or(Uuid::nil());
                    let name: String = cr.try_get("name").unwrap_or_else(|_| "Unknown".to_string());
                    let score: i32 = cr.try_get("total_score").unwrap_or(0);
                    top_contributors.push((uid, name, score as u32));
                }
            }

            Some(TownProjectState {
                project_id: r.try_get("project_id").unwrap_or_else(|_| "observatory".to_string()),
                name: r.try_get("name").unwrap_or_else(|_| "蒸汽天文台".to_string()),
                status,
                target_ether: r.try_get::<i32, _>("target_ether").unwrap_or(0) as u32,
                current_ether: r.try_get::<i32, _>("current_ether").unwrap_or(0) as u32,
                target_wood: r.try_get::<i32, _>("target_wood").unwrap_or(0) as u32,
                current_wood: r.try_get::<i32, _>("current_wood").unwrap_or(0) as u32,
                target_stone: r.try_get::<i32, _>("target_stone").unwrap_or(0) as u32,
                current_stone: r.try_get::<i32, _>("current_stone").unwrap_or(0) as u32,
                target_crystal: r.try_get::<i32, _>("target_crystal").unwrap_or(0) as u32,
                current_crystal: r.try_get::<i32, _>("current_crystal").unwrap_or(0) as u32,
                top_contributors,
            })
        }
        _ => None,
    }
}

async fn upsert_project(pool: &sqlx::postgres::PgPool, state: &TownProjectState) -> Result<(), sqlx::Error> {
    let status_str = match state.status {
        TownProjectStatus::Planning => "planning",
        TownProjectStatus::Building => "building",
        TownProjectStatus::Completed => "completed",
    };
    sqlx::query(
        "INSERT INTO town_project (project_id, name, status, target_ether, current_ether, target_wood, current_wood, target_stone, current_stone, target_crystal, current_crystal) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
         ON CONFLICT (project_id) DO UPDATE SET status = EXCLUDED.status, current_ether = EXCLUDED.current_ether, \
         current_wood = EXCLUDED.current_wood, current_stone = EXCLUDED.current_stone, current_crystal = EXCLUDED.current_crystal, \
         updated_at = CURRENT_TIMESTAMP"
    )
    .bind(&state.project_id)
    .bind(&state.name)
    .bind(status_str)
    .bind(state.target_ether as i32)
    .bind(state.current_ether as i32)
    .bind(state.target_wood as i32)
    .bind(state.current_wood as i32)
    .bind(state.target_stone as i32)
    .bind(state.current_stone as i32)
    .bind(state.target_crystal as i32)
    .bind(state.current_crystal as i32)
    .execute(pool)
    .await?;
    Ok(())
}

async fn update_donation(pool: &sqlx::postgres::PgPool, user_id: Uuid, project_id: &str, ether: u32, wood: u32, stone: u32, crystal: u32, score_delta: u32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO town_project_donations (user_id, project_id, ether_donated, wood_donated, stone_donated, crystal_donated, total_score) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (user_id, project_id) DO UPDATE SET \
         ether_donated = town_project_donations.ether_donated + EXCLUDED.ether_donated, \
         wood_donated = town_project_donations.wood_donated + EXCLUDED.wood_donated, \
         stone_donated = town_project_donations.stone_donated + EXCLUDED.stone_donated, \
         crystal_donated = town_project_donations.crystal_donated + EXCLUDED.crystal_donated, \
         total_score = town_project_donations.total_score + EXCLUDED.total_score"
    )
    .bind(user_id)
    .bind(project_id)
    .bind(ether as i32)
    .bind(wood as i32)
    .bind(stone as i32)
    .bind(crystal as i32)
    .bind(score_delta as i32)
    .execute(pool)
    .await?;
    Ok(())
}
