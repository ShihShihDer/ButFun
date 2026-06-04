//! 使用者帳號模型(provider 無關)。
//!
//! 內部以 UUID 為主鍵；外部登入(目前只有 Google)用 `(provider, external_id)`
//! 連結到內部 user。資料以 JSONL 形式存在 `data/users.jsonl`,啟動時整檔讀進記憶體、
//! 變更時 append 一行 + 更新記憶體索引。之後接 Phase 0-E Postgres 時,把這層後面
//! 換成 `PgStore` 即可,不動上層。

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const STORE_PATH: &str = "data/users.jsonl";

/// 一個內部使用者帳號。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    /// 外部登入 provider 名稱,例 "google"。日後可加 "discord"、"apple"。
    pub provider: String,
    /// provider 那邊的唯一 ID,例 Google 的 `sub`。
    pub external_id: String,
    pub email: Option<String>,
    pub name: String,
    /// 玩家自選種族(MVP 預設 "terran")。
    pub species: String,
    /// Unix 毫秒。
    pub created_at: u64,
}

/// 使用者儲存層的內部狀態。可被多執行緒共用。
#[derive(Clone)]
pub struct UserStore {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    by_id: HashMap<Uuid, User>,
    /// (provider, external_id) -> user_id
    by_external: HashMap<(String, String), Uuid>,
}

impl UserStore {
    pub fn new() -> Self {
        let users = load_from_disk();
        let mut by_id = HashMap::new();
        let mut by_external = HashMap::new();
        for u in users {
            by_external.insert((u.provider.clone(), u.external_id.clone()), u.id);
            by_id.insert(u.id, u);
        }
        Self {
            inner: Arc::new(Mutex::new(Inner { by_id, by_external })),
        }
    }

    /// 用 provider+external_id 找;沒有就新建一個。回傳該 user。
    pub fn find_or_create(
        &self,
        provider: &str,
        external_id: &str,
        email: Option<String>,
        name: &str,
    ) -> User {
        let mut inner = self.inner.lock().unwrap();
        let key = (provider.to_string(), external_id.to_string());
        if let Some(uid) = inner.by_external.get(&key).copied() {
            if let Some(u) = inner.by_id.get(&uid) {
                return u.clone();
            }
        }
        let user = User {
            id: Uuid::new_v4(),
            provider: provider.to_string(),
            external_id: external_id.to_string(),
            email,
            name: sanitize_name(name),
            species: DEFAULT_SPECIES.to_string(),
            created_at: now_millis(),
        };
        append_to_disk(&user);
        inner.by_external.insert(key, user.id);
        inner.by_id.insert(user.id, user.clone());
        tracing::info!(user_id = %user.id, %provider, "新使用者建立: {}", user.name);
        user
    }

    pub fn get(&self, id: Uuid) -> Option<User> {
        self.inner.lock().unwrap().by_id.get(&id).cloned()
    }
}

impl Default for UserStore {
    fn default() -> Self {
        Self::new()
    }
}

/// 新玩家 / 訪客的預設物種。
pub const DEFAULT_SPECIES: &str = "terran";

/// 清理玩家輸入的名字：去頭尾空白、以「字元」截到 24、空字串退回「拓荒者」。
/// 訪客進場（`ws.rs`）與帳號建立（這裡）共用，避免兩處規則漂移。
pub fn sanitize_name(raw: &str) -> String {
    let s: String = raw.trim().chars().take(24).collect();
    if s.is_empty() {
        "拓荒者".to_string()
    } else {
        s
    }
}

/// 清理玩家輸入的物種：去頭尾空白、空字串退回預設物種。
pub fn sanitize_species(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        DEFAULT_SPECIES.to_string()
    } else {
        s.to_string()
    }
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn load_from_disk() -> Vec<User> {
    match std::fs::read_to_string(STORE_PATH) {
        Ok(contents) => contents
            .lines()
            .filter_map(|line| serde_json::from_str::<User>(line).ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn append_to_disk(u: &User) {
    if let Some(parent) = std::path::Path::new(STORE_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(STORE_PATH)
    {
        Ok(mut file) => {
            if let Ok(line) = serde_json::to_string(u) {
                let _ = writeln!(file, "{line}");
            }
        }
        Err(e) => tracing::warn!("無法寫入 users 檔 {STORE_PATH}: {e}"),
    }
}

// ============= 純邏輯單元測試(無 IO) =============
#[cfg(test)]
mod tests {
    use super::{sanitize_name, sanitize_species, DEFAULT_SPECIES};

    #[test]
    fn keeps_normal_name() {
        assert_eq!(sanitize_name("施育群"), "施育群");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(sanitize_name("  小明  "), "小明");
    }

    #[test]
    fn empty_or_whitespace_falls_back_to_default() {
        assert_eq!(sanitize_name(""), "拓荒者");
        assert_eq!(sanitize_name("   "), "拓荒者");
    }

    #[test]
    fn truncates_to_24_chars() {
        // 取 24 個「字元」(非位元組),確保多位元組字也以字元計。
        let long = "あ".repeat(50);
        let out = sanitize_name(&long);
        assert_eq!(out.chars().count(), 24);
    }

    #[test]
    fn counts_chars_not_bytes() {
        // 25 個中日文字應被截到 24 個字元(而非 24 bytes)。
        let name = "界".repeat(25);
        assert_eq!(sanitize_name(&name).chars().count(), 24);
    }

    #[test]
    fn species_keeps_normal_value() {
        assert_eq!(sanitize_species("celestial"), "celestial");
    }

    #[test]
    fn species_trims_whitespace() {
        assert_eq!(sanitize_species("  terran  "), "terran");
    }

    #[test]
    fn species_empty_or_whitespace_falls_back_to_default() {
        assert_eq!(sanitize_species(""), DEFAULT_SPECIES);
        assert_eq!(sanitize_species("   "), DEFAULT_SPECIES);
    }
}
