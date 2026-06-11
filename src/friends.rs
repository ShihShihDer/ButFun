//! 好友關係持久化 store（ROADMAP 96）。
//!
//! 單向 follow：A 加 B，A 的清單有 B；B 未加 A 則 B 清單不含 A。
//! - Postgres 模式：啟動時載入全部好友對；變更時 fire-and-forget INSERT/DELETE。
//! - 記憶體模式（無 DATABASE_URL / 測試）：重啟後清空，行為正確但不跨重啟。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
enum Backend {
    Memory,
    Postgres(sqlx::postgres::PgPool),
}

/// 好友關係持久化 store（Clone 安全，內含 Arc）。
#[derive(Clone)]
pub struct FriendStore {
    inner: Arc<Mutex<HashMap<Uuid, HashSet<Uuid>>>>,
    backend: Backend,
}

impl Default for FriendStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FriendStore {
    /// 記憶體模式（測試 / 無 DB）。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            backend: Backend::Memory,
        }
    }

    /// Postgres 模式：啟動時載入全部好友關係。
    pub async fn from_pool(pool: sqlx::postgres::PgPool) -> Self {
        let pairs = load_friends(&pool).await;
        let mut map: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
        for (user_id, friend_id) in pairs {
            map.entry(user_id).or_default().insert(friend_id);
        }
        Self {
            inner: Arc::new(Mutex::new(map)),
            backend: Backend::Postgres(pool),
        }
    }

    /// 加好友（已存在回 false，新增回 true）。
    pub fn add(&self, user_id: Uuid, friend_id: Uuid) -> bool {
        let inserted = {
            let mut lock = self.inner.lock().unwrap();
            lock.entry(user_id).or_default().insert(friend_id)
        };
        if inserted {
            if let Backend::Postgres(pool) = &self.backend {
                let pool = pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = insert_friend(&pool, user_id, friend_id).await {
                        tracing::error!(%e, %user_id, %friend_id, "friends insert 失敗");
                    }
                });
            }
        }
        inserted
    }

    /// 刪好友。
    pub fn remove(&self, user_id: Uuid, friend_id: Uuid) {
        let existed = {
            let mut lock = self.inner.lock().unwrap();
            lock.get_mut(&user_id)
                .map(|set| set.remove(&friend_id))
                .unwrap_or(false)
        };
        if existed {
            if let Backend::Postgres(pool) = &self.backend {
                let pool = pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = delete_friend(&pool, user_id, friend_id).await {
                        tracing::error!(%e, %user_id, %friend_id, "friends delete 失敗");
                    }
                });
            }
        }
    }

    /// 取得某玩家的好友 UUID 清單。
    pub fn get_friends(&self, user_id: Uuid) -> Vec<Uuid> {
        self.inner
            .lock()
            .unwrap()
            .get(&user_id)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// 是否已加為好友。
    pub fn is_friend(&self, user_id: Uuid, friend_id: Uuid) -> bool {
        self.inner
            .lock()
            .unwrap()
            .get(&user_id)
            .map(|set| set.contains(&friend_id))
            .unwrap_or(false)
    }
}

async fn load_friends(pool: &sqlx::postgres::PgPool) -> Vec<(Uuid, Uuid)> {
    let rows = sqlx::query("SELECT user_id, friend_id FROM friends")
        .fetch_all(pool)
        .await;
    match rows {
        Ok(rows) => rows
            .iter()
            .filter_map(|r| {
                let user_id: Uuid = sqlx::Row::try_get(r, "user_id").ok()?;
                let friend_id: Uuid = sqlx::Row::try_get(r, "friend_id").ok()?;
                Some((user_id, friend_id))
            })
            .collect(),
        Err(e) => {
            tracing::error!(%e, "載入 friends 失敗");
            Vec::new()
        }
    }
}

async fn insert_friend(
    pool: &sqlx::postgres::PgPool,
    user_id: Uuid,
    friend_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO friends (user_id, friend_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(friend_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn delete_friend(
    pool: &sqlx::postgres::PgPool,
    user_id: Uuid,
    friend_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM friends WHERE user_id = $1 AND friend_id = $2")
        .bind(user_id)
        .bind(friend_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_get() {
        let store = FriendStore::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert!(store.add(a, b));
        assert!(!store.add(a, b), "重複加應回 false");
        assert!(store.is_friend(a, b));
        assert!(!store.is_friend(b, a), "單向，b 沒加 a");
        let friends = store.get_friends(a);
        assert_eq!(friends, vec![b]);
    }

    #[test]
    fn remove_friend() {
        let store = FriendStore::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store.add(a, b);
        store.remove(a, b);
        assert!(!store.is_friend(a, b));
        assert!(store.get_friends(a).is_empty());
    }

    #[test]
    fn get_friends_empty_when_no_entry() {
        let store = FriendStore::new();
        let a = Uuid::new_v4();
        assert!(store.get_friends(a).is_empty());
    }
}
