//! 使用者帳號模型(provider 無關)。
//!
//! 內部以 UUID 為主鍵；外部登入(目前只有 Google)用 `(provider, external_id)`
//! 連結到內部 user。行程內維護一份 `by_id` / `by_external` 的記憶體索引當權威來源,
//! 讓 `get` / `find_or_create` 在登入熱路徑上同步、不被 DB 往返卡住;耐久層在索引後面
//! 可抽換(沿 `inventory_store.rs` 同一套 0-E 結構):
//!   - `Postgres`：設了 `DATABASE_URL` 時,啟動載回全部帳號、變更時非同步 upsert(正式上線走這條)。
//!   - `Jsonl`：沒設 `DATABASE_URL`(本機 `cargo run`)時 append 寫穿 `data/users.jsonl`。
//!   - `Memory`：測試用,不碰磁碟也不碰 DB。
//!
//! 延續其他 store 的設計權衡(DB 為主、JSONL 補洞;寫入失敗只記 log 不中斷登入;載入時
//! 一律過 sanitizer 驗壞值)。帳號是「已登入」狀態的根,位置／背包／農地都以這個穩定 id 為鍵。

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use sqlx::Row;
use uuid::Uuid;

/// 無 `DATABASE_URL` 時的退回持久化檔(執行期產生、已 gitignore)。接 Postgres 後這個檔
/// 退為「遷移種子」：啟動時 DB 還沒有的帳號仍會從這裡補回索引並一次性 upsert 進 DB,
/// 讓換版(從 JSONL 版切到 Postgres 版)不會把既有帳號丟掉。
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
    /// 帽型選項（ROADMAP 98 捏臉）：0~4，0 = 預設頂帽。
    #[serde(default)]
    pub hair_style: u8,
    /// 膚色選項（ROADMAP 98 捏臉）：0~4，0 = 預設古銅金。
    #[serde(default)]
    pub skin_tone: u8,
    /// 護目鏡鏡片色（ROADMAP 98 捏臉）：0~4，0 = 預設藍。
    #[serde(default)]
    pub goggle_color: u8,
}

/// 索引後面的耐久層。
#[derive(Clone)]
enum Backend {
    /// 測試用：不載入、不寫。只在 `#[cfg(test)]` 的 `in_memory()` 建構,故非測試建置標 allow。
    #[cfg_attr(not(test), allow(dead_code))]
    Memory,
    /// 沒設 `DATABASE_URL`：append 寫穿到此 JSONL 檔。
    Jsonl(&'static str),
    /// 設了 `DATABASE_URL`：啟動載回全部帳號、變更時 upsert 到 `users` 表。
    Postgres(PgPool),
}

/// 使用者儲存層的內部狀態。可被多執行緒共用。
#[derive(Clone)]
pub struct UserStore {
    inner: Arc<Mutex<Inner>>,
    backend: Backend,
}

struct Inner {
    by_id: HashMap<Uuid, User>,
    /// (provider, external_id) -> user_id
    by_external: HashMap<(String, String), Uuid>,
}

impl UserStore {
    /// 無 DB 模式(測試、本機 `cargo run`)：索引從 JSONL 載入,變更時 append 寫穿 JSONL。
    pub fn new() -> Self {
        let (by_id, by_external) = index_users(load_from_disk());
        Self {
            inner: Arc::new(Mutex::new(Inner { by_id, by_external })),
            backend: Backend::Jsonl(STORE_PATH),
        }
    }

    /// Postgres 模式(正式上線)：啟動時把 `users` 表全部載回索引,再用既有 JSONL 補齊
    /// DB 還沒有的帳號——並把這些補進來的帳號**一次性 upsert 進 DB**,讓換版(從 JSONL 版
    /// 切到 Postgres 版)時既有帳號不會因為「returning 玩家 find_or_create 命中即早回、永遠
    /// 不觸發寫入」而一直只活在 JSONL、最終隨 JSONL 淘汰丟失。已在 DB 的 id 以 DB 為準。
    /// 這個「DB 為主、JSONL 補洞」順序與 `InventoryStore::from_pool` 等一致。
    pub async fn from_pool(pool: PgPool) -> Self {
        let mut users = load_from_db(&pool).await;
        let known: std::collections::HashSet<Uuid> = users.iter().map(|u| u.id).collect();
        // JSONL 裡 DB 還沒有的帳號:補進記憶體,並一次性回填進 DB(冪等 upsert)。
        for u in load_from_disk() {
            if !known.contains(&u.id) {
                if let Err(e) = upsert_user(&pool, &u).await {
                    tracing::warn!(user_id = %u.id, "JSONL 帳號回填 Postgres 失敗(仍保留在記憶體):{e}");
                }
                users.push(u);
            }
        }
        let (by_id, by_external) = index_users(users);
        Self {
            inner: Arc::new(Mutex::new(Inner { by_id, by_external })),
            backend: Backend::Postgres(pool),
        }
    }

    /// 純記憶體版(測試用)：不載入、不寫磁碟、不碰 DB。
    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                by_id: HashMap::new(),
                by_external: HashMap::new(),
            })),
            backend: Backend::Memory,
        }
    }

    /// 用 provider+external_id 找;沒有就新建一個。回傳該 user。
    ///
    /// 記憶體索引的「查找或插入」在鎖內同步完成(避免兩個同 external_id 的登入競態各建一筆);
    /// 耐久寫入在**放開鎖之後** await(DB 是 async,不在持鎖路徑上 await——避免跨 await 持鎖)。
    /// returning 玩家命中索引即早回、不寫入(身分不變,沒東西要落地)。
    pub async fn find_or_create(
        &self,
        provider: &str,
        external_id: &str,
        email: Option<String>,
        name: &str,
    ) -> User {
        let user = {
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
                hair_style: 0,
                skin_tone: 0,
                goggle_color: 0,
            };
            inner.by_external.insert(key, user.id);
            inner.by_id.insert(user.id, user.clone());
            user
        };
        self.persist(&user).await;
        tracing::info!(user_id = %user.id, %provider, "新使用者建立: {}", user.name);
        user
    }

    pub fn get(&self, id: Uuid) -> Option<User> {
        self.inner.lock().unwrap().by_id.get(&id).cloned()
    }

    /// 依顯示名稱查找帳號（用於好友系統按名字加好友）。
    /// 若多個帳號同名，回傳任意一個；name 比對大小寫敏感（與登入顯示一致）。
    pub fn find_by_name(&self, name: &str) -> Option<User> {
        self.inner
            .lock()
            .unwrap()
            .by_id
            .values()
            .find(|u| u.name == name)
            .cloned()
    }

    /// 建一個 `provider="ai"` 的居民帳號(給 AI 自助註冊端點用)。每呼叫一次就是一個新居民
    /// (新 uuid、固定身分),其遊戲進度(位置/乙太)會比照其他登入玩家持久化。`external_id`
    /// 用 uuid 自身保證唯一(AI 居民沒有外部 provider)。`provider="ai"` 標記方便日後把這些
    /// 帳號轉成遊戲內 AI NPC。名字/物種一律過既有 sanitizer(與 Google 帳號同一道輸入邊界)。
    pub async fn create_ai(&self, name: &str, species: &str) -> User {
        let user = {
            let mut inner = self.inner.lock().unwrap();
            let id = Uuid::new_v4();
            let user = User {
                id,
                provider: "ai".to_string(),
                external_id: id.to_string(),
                email: None,
                name: sanitize_name(name),
                species: sanitize_species(species),
                created_at: now_millis(),
                hair_style: 0,
                skin_tone: 0,
                goggle_color: 0,
            };
            inner
                .by_external
                .insert(("ai".to_string(), user.external_id.clone()), user.id);
            inner.by_id.insert(user.id, user.clone());
            user
        };
        self.persist(&user).await;
        tracing::info!(user_id = %user.id, "新 AI 居民帳號建立: {}", user.name);
        user
    }

    /// 改顯示名:把該 user 的 `name` 換成清理後的新名,更新記憶體索引並落地到耐久層。
    /// 回傳更新後的 `User`;查無此人回 `None`。
    ///
    /// 耐久層用「同 `id` 覆蓋」語意落地:Postgres 走 `ON CONFLICT (id) DO UPDATE`;JSONL 走
    /// **append**(不重寫整檔)——舊行留著,載入時被後出現的同 `id` 行覆蓋(last-wins,見
    /// `index_users`),不做破壞性改寫 / 刪除。因 `ws.rs` 連線時即時讀 `UserStore`(authed 路徑
    /// `user.name`),改名後**重連**即生效,重啟也還在。名字一律過 `sanitize_name`(濾控制字元、
    /// 截 24 字、空退「拓荒者」),與帳號建立、訪客進場共用同一道公開輸入邊界。`provider` /
    /// `external_id`(登入比對鍵)不動,故 `by_external` 無需更新。
    pub async fn rename(&self, id: Uuid, new_name: &str) -> Option<User> {
        let user = {
            let mut inner = self.inner.lock().unwrap();
            let mut user = inner.by_id.get(&id)?.clone();
            user.name = sanitize_name(new_name);
            inner.by_id.insert(id, user.clone());
            user
        };
        self.persist(&user).await;
        tracing::info!(user_id = %id, "玩家改名為: {}", user.name);
        Some(user)
    }

    /// 更新外觀（ROADMAP 98 捏臉）：hair_style / skin_tone / goggle_color 各截到 0~4。
    /// 回傳更新後的 User；查無此人回 None。
    pub async fn update_appearance(
        &self,
        id: Uuid,
        hair_style: u8,
        skin_tone: u8,
        goggle_color: u8,
    ) -> Option<User> {
        let user = {
            let mut inner = self.inner.lock().unwrap();
            let mut user = inner.by_id.get(&id)?.clone();
            user.hair_style = hair_style.min(4);
            user.skin_tone = skin_tone.min(4);
            user.goggle_color = goggle_color.min(4);
            inner.by_id.insert(id, user.clone());
            user
        };
        self.persist(&user).await;
        Some(user)
    }

    /// 把一個帳號落地到耐久層(新建 / 改名共用)。Jsonl 模式 append 一行;Postgres 模式 upsert
    /// 同 `id` 列;Memory 無動作。寫入失敗只記 log、不中斷登入(索引仍是行程內權威,下次變更再試)。
    async fn persist(&self, user: &User) {
        match &self.backend {
            Backend::Memory => {}
            Backend::Jsonl(path) => append_to_disk_at(path, user),
            Backend::Postgres(pool) => {
                if let Err(e) = upsert_user(pool, user).await {
                    tracing::warn!(user_id = %user.id, "Postgres users upsert 失敗:{e}");
                }
            }
        }
    }
}

/// 把載入的使用者清單建成記憶體索引(`by_id` / `by_external`)。純函式以便測試。
///
/// **契約:同一個 `id` 後出現的行覆蓋先前的**(`by_id` 以最後一筆為準)——這正是
/// `rename` 靠 append 一筆同 `id`、新 `name` 的紀錄就能改名的基礎:重啟載入時後者勝出。
fn index_users(users: Vec<User>) -> (HashMap<Uuid, User>, HashMap<(String, String), Uuid>) {
    let mut by_id = HashMap::new();
    let mut by_external = HashMap::new();
    for u in users {
        by_external.insert((u.provider.clone(), u.external_id.clone()), u.id);
        by_id.insert(u.id, u);
    }
    (by_id, by_external)
}

impl Default for UserStore {
    fn default() -> Self {
        Self::new()
    }
}

/// 新玩家 / 訪客的預設物種。
pub const DEFAULT_SPECIES: &str = "terran";

/// 清理玩家輸入的名字：先濾掉控制字元（換行 / 歸位 / NUL 等）、去頭尾空白、以「字元」
/// 截到 24、空字串退回「拓荒者」。訪客進場（`ws.rs`）與帳號建立（這裡）共用，避免兩處規則漂移。
///
/// 濾控制字元是必要的：名字是單行身分欄位，卻會成為廣播給所有人的聊天 `from` 標籤與 HUD
/// 顯示名。聊天內容自己（`sanitize_chat`）已濾控制字元，名字若不濾，壞客戶端就能把換行 /
/// NUL 塞進名字、繞過聊天過濾，廣播出多行或破壞顯示／偽造介面的內容。與 `sanitize_chat`
/// 同一道公開輸入邊界。
pub fn sanitize_name(raw: &str) -> String {
    let s: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(24)
        .collect();
    if s.is_empty() {
        "拓荒者".to_string()
    } else {
        s
    }
}

/// 目前開放的物種清單（與前端 index.html speciesInput 同步）。
pub const VALID_SPECIES: &[&str] = &["terran", "shilaya", "busi", "lumen"];

/// 清理玩家輸入的物種：先濾掉控制字元、去頭尾空白、再驗證是否為已開放物種；
/// 不合法或空字串皆退回預設物種 "terran"。
pub fn sanitize_species(raw: &str) -> String {
    let s: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string();
    if VALID_SPECIES.contains(&s.as_str()) {
        s
    } else {
        DEFAULT_SPECIES.to_string()
    }
}

/// 隨機角色名的形容詞池（材質／天象，呼應蒸汽龐克太空歌劇語彙）。
const CODENAME_ADJ: &[&str] = &[
    "黃銅", "霧鏽", "星塵", "發條", "蒸汽", "月光", "琥珀", "雲頂", "銅環", "微光", "漂浮", "齒輪",
];
/// 隨機角色名的名詞池（角色職）。
const CODENAME_NOUN: &[&str] = &[
    "拓荒者", "領航員", "技師", "夢行者", "旅人", "園丁", "信使", "觀星人", "拾荒者", "鐘錶匠",
];

/// 由 seed 決定一個隨機角色名，形如「黃銅領航員-417」。純函式以便測試。
///
/// 為什麼要這個：Google 登入會帶回真實姓名，過去直接拿來當顯示名（廣播成聊天 `from` /
/// HUD 名），等於把本名公開給所有玩家——隱私問題（玩家建議 at=1780631336007）。新帳號改
/// 配一個與主題相襯的隨機代號，玩家日後仍可自訂；既有帳號不受影響（`find_or_create` 命中
/// 即早回，根本不會走到產名）。尾碼數字降低撞名機率。
pub fn codename_from_seed(seed: u64) -> String {
    let adj = CODENAME_ADJ[(seed % CODENAME_ADJ.len() as u64) as usize];
    let noun =
        CODENAME_NOUN[((seed / CODENAME_ADJ.len() as u64) % CODENAME_NOUN.len() as u64) as usize];
    let combos = CODENAME_ADJ.len() as u64 * CODENAME_NOUN.len() as u64;
    let num = 100 + (seed / combos) % 900; // 100..=999
    format!("{adj}{noun}-{num}")
}

/// 抽一個隨機角色名給新帳號用（種子取自系統亂源）。
pub fn random_codename() -> String {
    use rand::Rng;
    codename_from_seed(rand::thread_rng().gen::<u64>())
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
        Ok(contents) => parse_and_sanitize(&contents),
        Err(_) => Vec::new(),
    }
}

/// 把 JSONL 檔內容解析成使用者清單，並讓每筆的顯示用身分欄位（`name` / `species`）
/// **再過一次對應的 sanitizer**（載入防線）。純函式以便測試。
///
/// 為什麼載入也要過濾：`name` / `species` 是「存檔又重載」的持久化欄位，而 `name` 會成為
/// 已登入玩家進場後**廣播給所有人**的聊天 `from` 標籤與 HUD 顯示名（見 `ws.rs` 的 authed
/// 路徑 `name: user.name`），`species` 也會顯示。控制字元過濾原本只加在**寫入**路徑
/// （`find_or_create` 呼叫 `sanitize_name`），但 `data/users.jsonl` 裡可能有**那道硬化
/// landing 之前**（名字濾控制字元是後來才加的）寫進的舊行，或被手動編輯 / 損毀的行——
/// 殘留的 `NUL` / `ESC`(0x1B) / 換行 會原樣載進記憶體、再隨登入玩家廣播出去，注入 ANSI
/// 轉義偽造顯示、或廣播出多行內容。讓讀路徑也走同一個 sanitizer，輸出就用「實際會被存下
/// 的乾淨值」當單一真實來源，不論磁碟上那行是何時、被什麼寫進去的——延續
/// `suggestions::parse_and_sanitize` / `field::from_tiles` / `positions::spawn_at` 在**載入時**
/// 驗證壞持久化資料的防線脈絡（`users.jsonl` 是另一個「存檔又重載卻在載入路徑沒驗證」的結構）。
///
/// 刻意只清**顯示用**欄位（`name` / `species`）：`provider` / `external_id` 是登入比對鍵
/// （要與 OAuth 那邊送來的值逐字相符），動了會讓既有帳號對不上、形同丟失帳號，故不碰；
/// `email` 同理不顯示給其他玩家。也刻意**不改寫 / 不刪除**磁碟上的檔（不破壞玩家資料），
/// 只過濾載進記憶體的內容。解析失敗的行照舊跳過。
fn parse_and_sanitize(contents: &str) -> Vec<User> {
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<User>(line).ok())
        .map(|mut u| {
            u.name = sanitize_name(&u.name);
            u.species = sanitize_species(&u.species);
            u
        })
        .collect()
}

fn append_to_disk_at(path: &str, u: &User) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut file) => {
            if let Ok(line) = serde_json::to_string(u) {
                let _ = writeln!(file, "{line}");
            }
        }
        Err(e) => tracing::warn!("無法寫入 users 檔 {path}: {e}"),
    }
}

/// upsert 一個帳號到 `users` 表(同 `id` 覆蓋:新建即插入、改名即更新 name)。走 runtime query
/// API(非 `query!` 巨集),故 build/test 不需 live DB。`provider` / `external_id` 是登入比對鍵,
/// 一併寫入但衝突時不改(沿 `id` 為準);`updated_at` 每次更新為 now()。
async fn upsert_user(pool: &PgPool, u: &User) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO users (id, provider, external_id, email, name, species, created_at, updated_at, \
           hair_style, skin_tone, goggle_color) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now(), $8, $9, $10) \
         ON CONFLICT (id) DO UPDATE SET \
           email = EXCLUDED.email, name = EXCLUDED.name, species = EXCLUDED.species, \
           hair_style = EXCLUDED.hair_style, skin_tone = EXCLUDED.skin_tone, \
           goggle_color = EXCLUDED.goggle_color, updated_at = now()",
    )
    .bind(u.id)
    .bind(&u.provider)
    .bind(&u.external_id)
    .bind(&u.email)
    .bind(&u.name)
    .bind(&u.species)
    .bind(u.created_at as i64)
    .bind(u.hair_style as i16)
    .bind(u.skin_tone as i16)
    .bind(u.goggle_color as i16)
    .execute(pool)
    .await?;
    Ok(())
}

/// 啟動時把 `users` 表全部載回。顯示用欄位(`name` / `species`)一律過 sanitizer,與 JSONL
/// 載入路徑(`parse_and_sanitize`)同一道防線——磁碟被竄改也不會把控制字元帶進廣播。
/// 載入失敗(DB 連線剛斷等)回空清單,讓伺服器仍能起來、之後再寫回。
async fn load_from_db(pool: &PgPool) -> Vec<User> {
    let rows = match sqlx::query(
        "SELECT id, provider, external_id, email, name, species, created_at, \
           COALESCE(hair_style,0) AS hair_style, \
           COALESCE(skin_tone,0) AS skin_tone, \
           COALESCE(goggle_color,0) AS goggle_color \
         FROM users",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("從 Postgres 載入 users 失敗(先以空索引起來):{e}");
            return Vec::new();
        }
    };
    rows.into_iter()
        .map(|r| {
            let created_at: i64 = r.get("created_at");
            let hair_style: i16 = r.get("hair_style");
            let skin_tone: i16 = r.get("skin_tone");
            let goggle_color: i16 = r.get("goggle_color");
            User {
                id: r.get("id"),
                provider: r.get("provider"),
                external_id: r.get("external_id"),
                email: r.get("email"),
                name: sanitize_name(&r.get::<String, _>("name")),
                species: sanitize_species(&r.get::<String, _>("species")),
                created_at: created_at as u64,
                hair_style: (hair_style as u8).min(4),
                skin_tone: (skin_tone as u8).min(4),
                goggle_color: (goggle_color as u8).min(4),
            }
        })
        .collect()
}

// ============= 純邏輯單元測試(無 IO) =============
#[cfg(test)]
mod tests {
    use super::{
        codename_from_seed, index_users, parse_and_sanitize, sanitize_name, sanitize_species, User,
        UserStore, DEFAULT_SPECIES,
    };
    use uuid::Uuid;

    // 測試用:組一個帶指定 id / name 的 User(其餘欄位填佔位值)。
    fn mk_user(id: Uuid, name: &str) -> User {
        User {
            id,
            provider: "google".to_string(),
            external_id: "sub".to_string(),
            email: None,
            name: name.to_string(),
            species: "terran".to_string(),
            created_at: 1,
            hair_style: 0,
            skin_tone: 0,
            goggle_color: 0,
        }
    }

    #[test]
    fn index_users_last_line_wins_for_same_id() {
        // rename 靠 append 一筆同 id、新 name 的紀錄,載入時後者勝出。鎖住這個契約——
        // 否則改名重啟後會復活舊名。
        let id = Uuid::parse_str("00000000-0000-0000-0000-0000000000aa").unwrap();
        let (by_id, by_external) = index_users(vec![mk_user(id, "舊名"), mk_user(id, "新名")]);
        assert_eq!(by_id.len(), 1, "同 id 應收斂成一筆");
        assert_eq!(by_id.get(&id).unwrap().name, "新名");
        // 登入比對鍵索引仍指向同一個 id(provider/external_id 不因改名變動)。
        assert_eq!(
            by_external.get(&("google".to_string(), "sub".to_string())),
            Some(&id)
        );
    }

    #[test]
    fn index_users_keeps_distinct_ids() {
        let a = Uuid::parse_str("00000000-0000-0000-0000-0000000000a1").unwrap();
        let b = Uuid::parse_str("00000000-0000-0000-0000-0000000000b2").unwrap();
        let mut ua = mk_user(a, "甲");
        ua.external_id = "sa".to_string();
        let mut ub = mk_user(b, "乙");
        ub.external_id = "sb".to_string();
        let (by_id, _) = index_users(vec![ua, ub]);
        assert_eq!(by_id.len(), 2);
        assert_eq!(by_id.get(&a).unwrap().name, "甲");
        assert_eq!(by_id.get(&b).unwrap().name, "乙");
    }

    #[test]
    fn keeps_normal_name() {
        assert_eq!(sanitize_name("施育群"), "施育群");
    }

    #[test]
    fn codename_has_expected_shape() {
        // 形如「<形容詞><名詞>-<100..=999>」,且尾碼在合法範圍。
        let name = codename_from_seed(0);
        let (head, num) = name.rsplit_once('-').expect("應含 '-' 尾碼");
        assert!(!head.is_empty());
        let n: u64 = num.parse().expect("尾碼應為數字");
        assert!((100..=999).contains(&n), "尾碼 {n} 不在 100..=999");
    }

    #[test]
    fn codename_survives_sanitize_unchanged() {
        // 隨機代號會被當顯示名存下,必須能原樣通過 sanitize_name(無控制字元、≤24 字元)。
        for seed in [0u64, 1, 42, 999, u64::MAX] {
            let name = codename_from_seed(seed);
            assert_eq!(sanitize_name(&name), name, "seed={seed} 被 sanitize 改動了");
            assert!(name.chars().count() <= 24);
        }
    }

    #[test]
    fn codename_varies_with_seed() {
        // 不同 seed 至少產生多種代號(否則撞名嚴重、失去意義)。
        let mut seen = std::collections::HashSet::new();
        for seed in 0..200u64 {
            seen.insert(codename_from_seed(seed));
        }
        assert!(seen.len() > 50, "代號變化太少: 僅 {}", seen.len());
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
    fn strips_control_chars_from_name() {
        // 換行 / 歸位 / NUL / tab 都該被濾掉——名字會成為廣播的聊天 from 標籤，
        // 不讓壞客戶端藉名字塞進多行或破壞顯示的內容。
        assert_eq!(sanitize_name("施\n育\r群\0老\t師"), "施育群老師");
    }

    #[test]
    fn control_only_name_falls_back_to_default() {
        // 清乾淨後變空（全是控制字元）→ 退回預設名，而非空字串。
        assert_eq!(sanitize_name("\n\r\0\t"), "拓荒者");
    }

    #[test]
    fn control_chars_filtered_before_truncation() {
        // 控制字元先被濾掉、不佔截斷的 24 字額度：24 個有效字 + 夾雜換行 → 全留下。
        let raw = "字\n".repeat(24); // 24 個「字」+ 24 個換行
        let out = sanitize_name(&raw);
        assert_eq!(out.chars().count(), 24);
        assert!(!out.contains('\n'));
    }

    #[test]
    fn species_keeps_valid_values() {
        // 四個已開放種族都應保留原值。
        assert_eq!(sanitize_species("terran"), "terran");
        assert_eq!(sanitize_species("shilaya"), "shilaya");
        assert_eq!(sanitize_species("busi"), "busi");
        assert_eq!(sanitize_species("lumen"), "lumen");
    }

    #[test]
    fn species_rejects_unknown_value() {
        // 非白名單的種族（包含舊測試的 "celestial"）一律退回預設物種。
        assert_eq!(sanitize_species("celestial"), DEFAULT_SPECIES);
        assert_eq!(sanitize_species("alien"), DEFAULT_SPECIES);
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

    #[test]
    fn species_strips_control_chars() {
        // 控制字元清除後若還是合法種族就保留，否則退回預設。
        assert_eq!(sanitize_species("ter\nr\0an"), "terran");
        assert_eq!(sanitize_species("\n\0\t"), DEFAULT_SPECIES);
    }

    // ===== 載入路徑（parse_and_sanitize）防線：對齊 suggestions::parse_and_sanitize =====

    #[test]
    fn load_path_strips_control_chars_from_name_and_species() {
        // name/species 是「存檔又重載」的顯示用欄位（name 會廣播成聊天 from / HUD 名）。
        // 硬化 landing 之前寫進、或被竄改的舊行殘留控制字元，載入時要被同一個 sanitizer
        // 濾掉，不讓它原樣載進記憶體再隨登入玩家廣播出去（ESC 可注入 ANSI 轉義偽造顯示）。
        let jsonl = "{\"id\":\"00000000-0000-0000-0000-000000000001\",\
                      \"provider\":\"google\",\"external_id\":\"sub-1\",\
                      \"email\":\"a@b.com\",\"name\":\"小\\u001b明\\u0000\",\
                      \"species\":\"ter\\nran\",\"created_at\":7}";
        let out = parse_and_sanitize(jsonl);
        assert_eq!(out.len(), 1);
        let u = &out[0];
        assert_eq!(u.name, "小明");
        assert_eq!(u.species, "terran");
        assert!(!u.name.contains('\u{1b}'));
        assert!(!u.name.contains('\0'));
        // 查找鍵 / 中介資料不被動到：provider/external_id 是登入比對鍵，動了會對不上帳號。
        assert_eq!(u.provider, "google");
        assert_eq!(u.external_id, "sub-1");
        assert_eq!(u.email.as_deref(), Some("a@b.com"));
        assert_eq!(
            u.id,
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
        );
        assert_eq!(u.created_at, 7);
    }

    #[test]
    fn load_path_control_only_name_falls_back_to_default() {
        // 名字 / 物種被竄改成全控制字元 → 退回預設（拓荒者 / terran），而非空字串。
        let jsonl = "{\"id\":\"00000000-0000-0000-0000-000000000002\",\
                      \"provider\":\"google\",\"external_id\":\"s2\",\
                      \"name\":\"\\u0000\\u001b\",\"species\":\"\\n\",\"created_at\":1}";
        let out = parse_and_sanitize(jsonl);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "拓荒者");
        assert_eq!(out[0].species, DEFAULT_SPECIES);
    }

    // ===== store 行為（純記憶體 backend，不碰磁碟 / DB）=====

    #[tokio::test]
    async fn find_or_create_returns_same_user_for_same_external_id() {
        // returning 玩家（同 provider+external_id）必須拿回同一個內部帳號，不是每次新建。
        let store = UserStore::in_memory();
        let a = store
            .find_or_create("google", "sub-1", Some("a@b.com".into()), "甲")
            .await;
        let b = store
            .find_or_create("google", "sub-1", None, "改了名也不該影響")
            .await;
        assert_eq!(a.id, b.id, "同 external_id 應命中既有帳號");
        assert_eq!(b.name, "甲", "命中既有帳號時不覆蓋既有顯示名");
    }

    #[tokio::test]
    async fn create_ai_makes_distinct_accounts() {
        // 每次 create_ai 都是一個新居民（新 uuid、provider=ai、external_id=自身 uuid）。
        let store = UserStore::in_memory();
        let a = store.create_ai("機器人", "terran").await;
        let b = store.create_ai("機器人", "terran").await;
        assert_ne!(a.id, b.id);
        assert_eq!(a.provider, "ai");
        assert_eq!(a.external_id, a.id.to_string());
        // 名字過 sanitizer 後可由 get 取回。
        assert_eq!(store.get(a.id).unwrap().name, "機器人");
    }

    #[tokio::test]
    async fn rename_updates_name_and_persists_in_index() {
        // 改名後 get 立即反映新名（過 sanitizer）；查無此人回 None。
        let store = UserStore::in_memory();
        let u = store
            .find_or_create("google", "sub-2", None, "舊名")
            .await;
        let renamed = store.rename(u.id, "  新名\n ").await.expect("應改名成功");
        assert_eq!(renamed.name, "新名", "新名應過 sanitize（去空白／控制字元）");
        assert_eq!(store.get(u.id).unwrap().name, "新名");
        assert!(store.rename(Uuid::new_v4(), "x").await.is_none(), "查無此人回 None");
    }

    #[test]
    fn load_path_keeps_clean_lines_and_skips_malformed() {
        // 已經乾淨的正常行載入後一字不差；損毀 / 非 JSON 的行跳過（沿用 filter_map 容錯）。
        let jsonl = "這不是 json\n\
                     {\"id\":\"00000000-0000-0000-0000-000000000003\",\"provider\":\"google\",\
                      \"external_id\":\"s3\",\"email\":null,\"name\":\"拓荒者\",\
                      \"species\":\"terran\",\"created_at\":100}\n\
                     {壞掉";
        let out = parse_and_sanitize(jsonl);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "拓荒者");
        assert_eq!(out[0].species, "terran");
        assert_eq!(out[0].external_id, "s3");
        assert_eq!(out[0].email, None);
        assert_eq!(out[0].created_at, 100);
    }
}
