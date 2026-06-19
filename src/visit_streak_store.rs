//! 連日歸鄉·歸鄉印記（ROADMAP 397）的持久化層。
//!
//! 為每位登入玩家存「上次回訪是哪一天」與「連續回訪天數」，keyed by user_id 的獨立小表，
//! 與 users 帳號表、玩家遊戲資料表完全解耦。純邏輯在 `visit_streak.rs`。
//!
//! 沒設 `DATABASE_URL` 時走行內記憶體 backend（一個 `Mutex<HashMap>`），dev 仍能正常推進印記，
//! 只是重啟清零；接上 Postgres 後跨重啟保留。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::visit_streak::{advance_streak, StreakOutcome};

#[allow(unused_imports)]
use sqlx::postgres::PgPool;

#[derive(Clone)]
enum Backend {
    /// 行內記憶體：user_id → (last_visit_day, streak)。重啟清零。
    Memory(Arc<Mutex<HashMap<Uuid, (i64, u32)>>>),
    Postgres(sqlx::postgres::PgPool),
}

/// 歸鄉印記的持久化 store。
#[derive(Clone)]
pub struct VisitStreakStore {
    backend: Backend,
}

impl VisitStreakStore {
    pub fn new() -> Self {
        Self { backend: Backend::Memory(Arc::new(Mutex::new(HashMap::new()))) }
    }

    pub async fn from_pool(pool: sqlx::postgres::PgPool) -> Self {
        Self { backend: Backend::Postgres(pool) }
    }

    /// 推進一位玩家在 `today`（UTC 曆日序）的歸鄉印記，回傳結果。
    ///
    /// 讀目前記錄 → 純函式 `advance_streak` 算 → 真的前進了才寫回新的 (today, streak)。
    /// 同一天重複呼叫不會重複前進（純邏輯把關），多重連線競態最壞只是重算一次、結果一致。
    pub async fn advance(&self, uid: Uuid, today: i64) -> StreakOutcome {
        match &self.backend {
            Backend::Memory(map) => {
                let mut m = map.lock().unwrap();
                let prev = m.get(&uid).copied();
                let (prev_day, prev_streak) = match prev {
                    Some((d, s)) => (Some(d), s),
                    None => (None, 0),
                };
                let out = advance_streak(prev_day, prev_streak, today);
                if out.advanced {
                    m.insert(uid, (today, out.streak));
                }
                out
            }
            Backend::Postgres(pool) => {
                let prev = load_record(pool, uid).await;
                let (prev_day, prev_streak) = match prev {
                    Some((d, s)) => (Some(d), s),
                    None => (None, 0),
                };
                let out = advance_streak(prev_day, prev_streak, today);
                if out.advanced {
                    if let Err(e) = upsert_record(pool, uid, today, out.streak).await {
                        tracing::error!(%e, "歸鄉印記 upsert 失敗");
                    }
                }
                out
            }
        }
    }
}

impl Default for VisitStreakStore {
    fn default() -> Self {
        Self::new()
    }
}

async fn load_record(pool: &sqlx::postgres::PgPool, uid: Uuid) -> Option<(i64, u32)> {
    let row = sqlx::query("SELECT last_visit_day, streak FROM visit_streaks WHERE user_id = $1")
        .bind(uid)
        .fetch_optional(pool)
        .await;
    match row {
        Ok(Some(r)) => {
            use sqlx::Row;
            let day: i64 = r.try_get("last_visit_day").unwrap_or(0);
            let streak: i32 = r.try_get("streak").unwrap_or(0);
            Some((day, streak.max(0) as u32))
        }
        _ => None,
    }
}

async fn upsert_record(
    pool: &sqlx::postgres::PgPool,
    uid: Uuid,
    today: i64,
    streak: u32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO visit_streaks (user_id, last_visit_day, streak) VALUES ($1, $2, $3) \
         ON CONFLICT (user_id) DO UPDATE SET last_visit_day = EXCLUDED.last_visit_day, \
         streak = EXCLUDED.streak, updated_at = now()",
    )
    .bind(uid)
    .bind(today)
    .bind(streak as i32)
    .execute(pool)
    .await?;
    Ok(())
}
