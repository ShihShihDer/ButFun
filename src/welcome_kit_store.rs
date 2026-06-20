//! 新手見面禮（ROADMAP 444）的持久化層。
//!
//! 只記一件事：某帳號**有沒有領過**見面禮，keyed by user_id 的獨立小表，與 users 帳號表、
//! 玩家遊戲資料表完全解耦。禮包內容的純邏輯在 `welcome_kit.rs`。
//!
//! 核心是一個**原子 test-and-set**：`claim(uid)` 第一次回 `true`（這次該發禮），之後永遠回
//! `false`（已領過、別重發）。多重連線／重連競態下也只有一次回 `true`，靠 DB 的
//! `INSERT … ON CONFLICT DO NOTHING`（記憶體模式靠 `HashSet::insert`）保證冪等。
//!
//! 沒設 `DATABASE_URL` 時走行內記憶體 backend（一個 `Mutex<HashSet>`），dev 仍能正常發一次禮，
//! 只是重啟後「領過」記錄清零（重啟後同帳號會再領一次）；接上 Postgres 後跨重啟永久只領一次。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

#[allow(unused_imports)]
use sqlx::postgres::PgPool;

#[derive(Clone)]
enum Backend {
    /// 行內記憶體：領過見面禮的 user_id 集合。重啟清零。
    Memory(Arc<Mutex<HashSet<Uuid>>>),
    Postgres(sqlx::postgres::PgPool),
}

/// 見面禮的持久化 store。
#[derive(Clone)]
pub struct WelcomeKitStore {
    backend: Backend,
}

impl WelcomeKitStore {
    pub fn new() -> Self {
        Self { backend: Backend::Memory(Arc::new(Mutex::new(HashSet::new()))) }
    }

    pub async fn from_pool(pool: sqlx::postgres::PgPool) -> Self {
        Self { backend: Backend::Postgres(pool) }
    }

    /// 嘗試替 `uid` 領取見面禮：**第一次**回 `true`（呼叫端據此真的發禮），之後回 `false`。
    /// 原子 test-and-set，冪等——同帳號多重連線／重連最多只有一次回 `true`。
    /// DB 出錯時保守回 `false`（寧可漏發一次見面禮，也不要靠不住地重複鑄幣／重複塞背包）。
    pub async fn claim(&self, uid: Uuid) -> bool {
        match &self.backend {
            Backend::Memory(set) => {
                let mut s = set.lock().unwrap();
                // insert 回 true 代表這個 uid 之前不在集合裡＝第一次領。
                s.insert(uid)
            }
            Backend::Postgres(pool) => {
                match insert_claim(pool, uid).await {
                    // rows_affected == 1：這次真的插進去了＝第一次領。
                    Ok(inserted) => inserted,
                    Err(e) => {
                        tracing::error!(%e, "見面禮 claim 失敗，保守視為已領（不重發）");
                        false
                    }
                }
            }
        }
    }
}

impl Default for WelcomeKitStore {
    fn default() -> Self {
        Self::new()
    }
}

/// 嘗試插入「已領」記錄；回傳是否為**這次新插入**（true＝第一次領）。
/// 靠 `ON CONFLICT DO NOTHING`＋`rows_affected` 做原子冪等。
async fn insert_claim(pool: &sqlx::postgres::PgPool, uid: Uuid) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "INSERT INTO welcome_kits (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(uid)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_claim_is_once_only() {
        let store = WelcomeKitStore::new();
        let uid = Uuid::from_u128(1);
        // 第一次領得到，之後永遠領不到。
        assert!(store.claim(uid).await);
        assert!(!store.claim(uid).await);
        assert!(!store.claim(uid).await);
        // 不同帳號各自獨立。
        let other = Uuid::from_u128(2);
        assert!(store.claim(other).await);
        assert!(!store.claim(other).await);
    }
}
