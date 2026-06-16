//! 玩家最後狀態（位置 + 乙太）的伺服器端記憶（Phase 0-E 持久化）。
//!
//! 行程內維護一份 `id -> Saved` 的記憶體 cache 當權威來源,讓 `recall`/`remember` 保持
//! **同步**、不污染 WebSocket / 遊戲迴圈的鎖序。耐久層在 cache 後面可抽換：
//!   - `Postgres`：設了 `DATABASE_URL` 時，啟動載回、定期非同步 upsert（正式上線走這條）。
//!   - `Jsonl`：沒設 `DATABASE_URL`（本機 `cargo run`）時寫穿 `data/positions.jsonl`。
//!   - `Memory`：測試用，不碰磁碟也不碰 DB。
//!
//! 注意：只記「已登入」玩家（穩定 id）；訪客每次連線 id 隨機、記了也對不上,
//! 故不記,避免 cache 無界成長。

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use sqlx::Row;
use uuid::Uuid;

/// 無 `DATABASE_URL` 時的退回持久化檔（執行期產生、已 gitignore）。接 Postgres 後這個檔
/// 退為「遷移種子」：啟動時 DB 沒有的 id 仍會從這裡補回 cache,讓換版不會把人洗回中央。
const STORE_PATH: &str = "data/positions.jsonl";

/// JSONL 一行紀錄：把 id 與 `Saved` 合起來序列化。
#[derive(Serialize, Deserialize)]
struct DiskRow {
    id: Uuid,
    x: f32,
    y: f32,
    ether: u32,
    #[serde(default)]
    wallet_expansions: u32,
    #[serde(default)]
    exp: u32,
    #[serde(default)]
    mastery_warrior: u32,
    #[serde(default)]
    mastery_farmer: u32,
    #[serde(default)]
    mastery_artisan: u32,
    #[serde(default)]
    mastery_explorer: u32,
    #[serde(default)]
    mastery_merchant: u32,
    /// 屬性加點（ROADMAP 152）：五個欄位，舊存檔讀為 0。
    #[serde(default)]
    stat_unspent: u32,
    #[serde(default)]
    stat_hp: u32,
    #[serde(default)]
    stat_attack: u32,
    #[serde(default)]
    stat_speed: u32,
    #[serde(default)]
    stat_atk_speed: u32,
    /// 技能使用型熟練度（ROADMAP 153）：五條，舊存檔讀為 0。
    #[serde(default)]
    skill_use_warcry: u32,
    #[serde(default)]
    skill_use_bounty: u32,
    #[serde(default)]
    skill_use_precision: u32,
    #[serde(default)]
    skill_use_gale: u32,
    #[serde(default)]
    skill_use_haggle: u32,
    /// 生態圖鑑 bitmask（ROADMAP 333）。舊存檔無此欄位讀為 0。
    #[serde(default)]
    codex: u64,
}

/// 玩家進場時的預設位置——刻意生在「公共農地」正中央。沒有歷史位置(新玩家/全清後)時用它。
/// 動機:全清後大家 0 乙太,而種田要先有地、買地又要乙太 → 死結。讓新玩家一落地就站在
/// **可耕種的無主公共地**上,先種田攢第一桶乙太、再買自己的地(配「沒買的地一樣能種、
/// 買地是為了保護」的設計)。
pub fn default_spawn() -> (f32, f32) {
    use crate::field::{FIELD_COLS, FIELD_ROWS, TILE_SIZE};
    use crate::state::{PUB_FIELD_ORIGIN_X, PUB_FIELD_ORIGIN_Y};
    (
        PUB_FIELD_ORIGIN_X + FIELD_COLS as f32 * TILE_SIZE / 2.0,
        PUB_FIELD_ORIGIN_Y + FIELD_ROWS as f32 * TILE_SIZE / 2.0,
    )
}

/// 新手村安全半徑（像素）。圓心為 `default_spawn()`；圓內不生成敵人。
/// 640 約等於 1.25 個 CHUNK_SIZE（512），讓整個公共農地周圍有一圈緩衝。
pub const SAFE_SPAWN_RADIUS: f32 = 640.0;

/// 是否在「城鎮保護圈」內（主城＋各星球據點，牆內＋牆外 8 格緩衝）。
/// 敵人不在此生成、也不踏入。實作在 world-core（圍牆城鎮幾何的單一真相，
/// 與前端 wasm 同一份）；舊的圓形判斷由主城方形範圍完整涵蓋。純函式可測試。
pub fn is_in_safe_zone(x: f32, y: f32) -> bool {
    world_core::town_protected_at(x as f64, y as f64)
}

/// 依「是否有記住的歷史位置」決定進場座標。純函式，便於測試。
///
/// 契約：回傳的座標一定有限。這層刻意防住「載入被竄改/壞掉的
/// 持久化位置」——Postgres 的 `REAL` 欄位可能存進 `NaN`/`Inf`,
/// 不檢查就可能讓玩家座標變非有限。非有限一律退回地圖中央。
/// 在無限世界中，有限的「界外」座標應原樣保留。
pub fn spawn_at(recalled: Option<(f32, f32)>) -> (f32, f32) {
    match recalled {
        Some((x, y)) if x.is_finite() && y.is_finite() => (x, y),
        _ => default_spawn(),
    }
}

/// 某玩家離線時記下的最後狀態：位置 + 乙太 + 農地擴張格數 + 經驗值 + 五條熟練度 + 屬性加點。
///
/// 載入時的防線沿用既有入口；`wallet_expansions` 是 `u32`、型別本身擋壞值，
/// 超上限由 `PlotWallet::is_loadable` 驗、不過退回 0（全新地起算）。
/// 熟練度欄位全部 `#[serde(default)]`，讓舊存檔安全讀為 0。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Saved {
    pub x: f32,
    pub y: f32,
    pub ether: u32,
    #[serde(default)]
    pub wallet_expansions: u32,
    /// 累積經驗值（ROADMAP 17 升級系統）。
    #[serde(default)]
    pub exp: u32,
    /// 五條熟練度 XP（ROADMAP 38）。
    #[serde(default)]
    pub masteries: crate::class::Masteries,
    /// 屬性加點分配（ROADMAP 152）。舊存檔讀為全 0。
    #[serde(default)]
    pub stats: crate::stat_points::StatPoints,
    /// 技能使用型熟練度（ROADMAP 153）。舊存檔讀為全 0。
    #[serde(default)]
    pub skill_masteries: crate::skill_mastery::SkillMasteries,
    /// 生態圖鑑已發現物種 bitmask（ROADMAP 333）。舊存檔讀為 0（圖鑑全空、重新蒐集）。
    #[serde(default)]
    pub codex: u64,
}

/// cache 後面的耐久層。
#[derive(Clone)]
enum Backend {
    /// 測試用：不載入、不寫。只在 `#[cfg(test)]` 的 `in_memory()` 建構,故非測試建置標 allow。
    #[cfg_attr(not(test), allow(dead_code))]
    Memory,
    /// 沒設 `DATABASE_URL`：寫穿到此 JSONL 檔。
    Jsonl(&'static str),
    /// 設了 `DATABASE_URL`：啟動載回、定期非同步 upsert 到 `players` 表。
    Postgres(PgPool),
}

/// 記住玩家最後狀態的儲存層。記憶體 cache + 可抽換耐久層（Postgres / JSONL / 純記憶體）。
#[derive(Clone)]
pub struct PositionStore {
    inner: Arc<RwLock<HashMap<Uuid, Saved>>>,
    backend: Backend,
}

impl Default for PositionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionStore {
    /// 無 DB 模式：cache 從 JSONL 載入,之後寫穿 JSONL。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(load_from_disk(STORE_PATH))),
            backend: Backend::Jsonl(STORE_PATH),
        }
    }

    /// Postgres 模式（正式上線）：啟動時把 `players` 表載回 cache,再用既有 JSONL 補齊
    /// DB 還沒有的 id。這個「DB 為主、JSONL 補洞」的順序很關鍵——換版（從 JSONL 版切到
    /// Postgres 版）時 DB 可能還是空的,若不從 JSONL 種回,所有人會被 `recall` miss 洗回
    /// 地圖中央、乙太歸零（正是要修掉的 bug）。已在 DB 的 id 以 DB 為準,不被舊 JSONL 覆蓋。
    pub async fn from_pool(pool: PgPool) -> Self {
        let mut cache = load_players_from_db(&pool).await;
        for (id, saved) in load_from_disk(STORE_PATH) {
            cache.entry(id).or_insert(saved); // DB 沒有的才用 JSONL 補,DB 優先
        }
        Self {
            inner: Arc::new(RwLock::new(cache)),
            backend: Backend::Postgres(pool),
        }
    }

    /// 純記憶體版（測試用）：不載入、不寫磁碟、不碰 DB。
    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            backend: Backend::Memory,
        }
    }

    /// 取出某玩家上次離線時的狀態（沒有就 None）。讀 cache,同步。
    pub fn recall(&self, id: Uuid) -> Option<Saved> {
        self.inner.read().unwrap().get(&id).copied()
    }

    /// 記住某玩家目前狀態（更新 cache,同步）。Jsonl 模式順手寫穿磁碟;Postgres 模式只動
    /// cache,耐久寫入交給非同步的 `flush_online`/`flush_one`。
    pub fn remember(&self, id: Uuid, x: f32, y: f32, ether: u32, wallet_expansions: u32, exp: u32, masteries: crate::class::Masteries, stats: crate::stat_points::StatPoints, skill_masteries: crate::skill_mastery::SkillMasteries, codex: u64) {
        self.inner.write().unwrap().insert(id, Saved { x, y, ether, wallet_expansions, exp, masteries, stats, skill_masteries, codex });
        self.persist_jsonl();
    }

    /// 批次記住多名玩家（給遊戲迴圈定期快照線上玩家用）：更新 cache 一次。
    pub fn remember_all<I: IntoIterator<Item = (Uuid, f32, f32, u32, u32, u32, crate::class::Masteries, crate::stat_points::StatPoints, crate::skill_mastery::SkillMasteries, u64)>>(&self, items: I) {
        {
            let mut m = self.inner.write().unwrap();
            for (id, x, y, ether, wallet_expansions, exp, masteries, stats, skill_masteries, codex) in items {
                m.insert(id, Saved { x, y, ether, wallet_expansions, exp, masteries, stats, skill_masteries, codex });
            }
        }
        self.persist_jsonl();
    }

    /// 把線上已登入玩家批次 upsert 到 Postgres（遊戲迴圈每 ~10 秒呼叫）。非 Postgres 模式無動作。
    /// 失敗只記 log、不中斷遊戲迴圈（下一輪再試;cache 仍是行程內權威）。
    pub async fn flush_online(&self, rows: &[OnlinePlayerRow]) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        if let Err(e) = upsert_rows(pool, rows).await {
            tracing::warn!("Postgres flush_online 失敗（下一輪再試）：{e}");
        }
    }

    /// 玩家離線時把其最後狀態 upsert 到 Postgres（補離線前最後進度）。非 Postgres 模式無動作。
    pub async fn flush_one(&self, id: Uuid, name: &str, species: &str, x: f32, y: f32, ether: u32, wallet_expansions: u32, exp: u32, masteries: crate::class::Masteries, stats: crate::stat_points::StatPoints, skill_masteries: crate::skill_mastery::SkillMasteries, codex: u64) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        let row = [(id, name.to_string(), species.to_string(), x, y, ether, wallet_expansions, exp, masteries, stats, skill_masteries, codex)];
        if let Err(e) = upsert_rows(pool, &row).await {
            tracing::warn!("Postgres flush_one 失敗：{e}");
        }
    }

    /// 排行榜：依等級（exp）降冪取前 N 名，回傳 `(name, level)`。
    /// Postgres 模式查 DB（含離線玩家）；其他模式回空向量（呼叫端自行用線上玩家補底）。
    pub async fn leaderboard_top_level(&self, limit: i64) -> Vec<(String, u32)> {
        let Backend::Postgres(pool) = &self.backend else { return vec![]; };
        let rows = sqlx::query(
            "SELECT name, COALESCE(exp, 0) AS exp FROM players ORDER BY exp DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        rows.into_iter()
            .map(|r| {
                let name: String = r.get("name");
                let exp: i64 = r.get("exp");
                let level = (exp.max(0) as u32) / 100;
                (name, level)
            })
            .collect()
    }

    /// 排行榜：依乙太降冪取前 N 名，回傳 `(name, ether)`。
    /// Postgres 模式查 DB（含離線玩家）；其他模式回空向量。
    pub async fn leaderboard_top_ether(&self, limit: i64) -> Vec<(String, u32)> {
        let Backend::Postgres(pool) = &self.backend else { return vec![]; };
        let rows = sqlx::query(
            "SELECT name, COALESCE(ether, 0) AS ether FROM players ORDER BY ether DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        rows.into_iter()
            .map(|r| {
                let name: String = r.get("name");
                let ether: i64 = r.get("ether");
                (name, ether.max(0) as u32)
            })
            .collect()
    }

    /// Jsonl 模式才寫：把整份 cache 快照覆寫到磁碟。其餘模式無動作。
    fn persist_jsonl(&self) {
        let Backend::Jsonl(path) = self.backend else {
            return;
        };
        let rows: Vec<String> = {
            let m = self.inner.read().unwrap();
            m.iter()
                .filter_map(|(id, s)| {
                    serde_json::to_string(&DiskRow {
                        id: *id,
                        x: s.x,
                        y: s.y,
                        ether: s.ether,
                        wallet_expansions: s.wallet_expansions,
                        exp: s.exp,
                        mastery_warrior:  s.masteries.warrior,
                        mastery_farmer:   s.masteries.farmer,
                        mastery_artisan:  s.masteries.artisan,
                        mastery_explorer: s.masteries.explorer,
                        mastery_merchant: s.masteries.merchant,
                        stat_unspent:   s.stats.unspent,
                        stat_hp:        s.stats.hp,
                        stat_attack:    s.stats.attack,
                        stat_speed:     s.stats.speed,
                        stat_atk_speed: s.stats.atk_speed,
                        skill_use_warcry:    s.skill_masteries.warcry,
                        skill_use_bounty:    s.skill_masteries.bounty,
                        skill_use_precision: s.skill_masteries.precision,
                        skill_use_gale:      s.skill_masteries.gale,
                        skill_use_haggle:    s.skill_masteries.haggle,
                        codex: s.codex,
                    })
                    .ok()
                })
                .collect()
        };
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // 先寫暫存再 rename,避免寫到一半被重啟而毀檔。
        let tmp = format!("{path}.tmp");
        if let Ok(mut f) = std::fs::File::create(&tmp) {
            let _ = f.write_all(rows.join("\n").as_bytes());
            let _ = f.write_all(b"\n");
            let _ = f.sync_all();
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// 線上玩家 upsert 列型別（含熟練度 + 屬性加點 + 技能熟練度）。
pub type OnlinePlayerRow = (Uuid, String, String, f32, f32, u32, u32, u32, crate::class::Masteries, crate::stat_points::StatPoints, crate::skill_mastery::SkillMasteries, u64);

/// 批次 upsert 到 `players` 表（一筆 transaction,要嘛全進要嘛全不進）。
/// 走 runtime query API（非 `query!` 巨集），故 build/test 不需 live DB。
async fn upsert_rows(
    pool: &PgPool,
    rows: &[OnlinePlayerRow],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (id, name, species, x, y, ether, wallet_expansions, exp, m, s, sk, codex) in rows {
        sqlx::query(
            "INSERT INTO players \
               (id, name, species, x, y, ether, wallet_expansions, exp, \
                mastery_warrior, mastery_farmer, mastery_artisan, mastery_explorer, mastery_merchant, \
                stat_unspent, stat_hp, stat_attack, stat_speed, stat_atk_speed, \
                skill_use_warcry, skill_use_bounty, skill_use_precision, skill_use_gale, skill_use_haggle, \
                codex, \
                updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, now()) \
             ON CONFLICT (id) DO UPDATE SET \
               name = EXCLUDED.name, species = EXCLUDED.species, \
               x = EXCLUDED.x, y = EXCLUDED.y, ether = EXCLUDED.ether, \
               wallet_expansions = EXCLUDED.wallet_expansions, exp = EXCLUDED.exp, \
               mastery_warrior  = EXCLUDED.mastery_warrior, \
               mastery_farmer   = EXCLUDED.mastery_farmer, \
               mastery_artisan  = EXCLUDED.mastery_artisan, \
               mastery_explorer = EXCLUDED.mastery_explorer, \
               mastery_merchant = EXCLUDED.mastery_merchant, \
               stat_unspent   = EXCLUDED.stat_unspent, \
               stat_hp        = EXCLUDED.stat_hp, \
               stat_attack    = EXCLUDED.stat_attack, \
               stat_speed     = EXCLUDED.stat_speed, \
               stat_atk_speed = EXCLUDED.stat_atk_speed, \
               skill_use_warcry    = EXCLUDED.skill_use_warcry, \
               skill_use_bounty    = EXCLUDED.skill_use_bounty, \
               skill_use_precision = EXCLUDED.skill_use_precision, \
               skill_use_gale      = EXCLUDED.skill_use_gale, \
               skill_use_haggle    = EXCLUDED.skill_use_haggle, \
               codex = EXCLUDED.codex, \
               updated_at = now()",
        )
        .bind(id)
        .bind(name)
        .bind(species)
        .bind(x)
        .bind(y)
        .bind(*ether as i64)
        .bind(*wallet_expansions as i64)
        .bind(*exp as i64)
        .bind(m.warrior  as i64)
        .bind(m.farmer   as i64)
        .bind(m.artisan  as i64)
        .bind(m.explorer as i64)
        .bind(m.merchant as i64)
        // 屬性點 5 欄為 INT4：以 i32 綁定，與欄位型別一致（避免 i64→INT4 隱式收斂的邊界風險）。
        .bind(s.unspent  as i32)
        .bind(s.hp       as i32)
        .bind(s.attack   as i32)
        .bind(s.speed    as i32)
        .bind(s.atk_speed as i32)
        // 技能熟練度 5 欄同為 INTEGER(INT4)，也必須 i32 綁定。
        .bind(sk.warcry    as i32)
        .bind(sk.bounty    as i32)
        .bind(sk.precision as i32)
        .bind(sk.gale      as i32)
        .bind(sk.haggle    as i32)
        // 生態圖鑑 bitmask（ROADMAP 333）：BIGINT 欄，以 i64 綁定（僅低位有值，安全）。
        .bind(*codex as i64)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// 啟動時把 `players` 表載回 cache（含五條熟練度 + 屬性加點，ROADMAP 38/152）。
/// 位置一律過 `spawn_at` 驗證,DB 即使存進壞值也不會把玩家生到非法位置。
/// 載入失敗（DB 連線剛斷等）回空 map,讓伺服器仍能起來、之後再寫回。
async fn load_players_from_db(pool: &PgPool) -> HashMap<Uuid, Saved> {
    let mut map = HashMap::new();
    let rows = match sqlx::query(
        "SELECT id, x, y, ether, wallet_expansions, COALESCE(exp, 0) AS exp, \
         COALESCE(mastery_warrior, 0)  AS mastery_warrior, \
         COALESCE(mastery_farmer,  0)  AS mastery_farmer, \
         COALESCE(mastery_artisan, 0)  AS mastery_artisan, \
         COALESCE(mastery_explorer,0)  AS mastery_explorer, \
         COALESCE(mastery_merchant,0)  AS mastery_merchant, \
         COALESCE(stat_unspent,   0)  AS stat_unspent, \
         COALESCE(stat_hp,        0)  AS stat_hp, \
         COALESCE(stat_attack,    0)  AS stat_attack, \
         COALESCE(stat_speed,     0)  AS stat_speed, \
         COALESCE(stat_atk_speed, 0)  AS stat_atk_speed, \
         COALESCE(skill_use_warcry,    0) AS skill_use_warcry, \
         COALESCE(skill_use_bounty,    0) AS skill_use_bounty, \
         COALESCE(skill_use_precision, 0) AS skill_use_precision, \
         COALESCE(skill_use_gale,      0) AS skill_use_gale, \
         COALESCE(skill_use_haggle,    0) AS skill_use_haggle, \
         COALESCE(codex, 0) AS codex \
         FROM players",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("從 Postgres 載入 players 失敗（先以空 cache 起來）：{e}");
            return map;
        }
    };
    for r in rows {
        let id: Uuid = r.get("id");
        let x: f32 = r.get("x");
        let y: f32 = r.get("y");
        let ether: i64 = r.get("ether");
        let wallet_expansions: i64 = r.get("wallet_expansions");
        let exp: i64 = r.get("exp");
        let mw: i64 = r.get("mastery_warrior");
        let mf: i64 = r.get("mastery_farmer");
        let ma: i64 = r.get("mastery_artisan");
        let me: i64 = r.get("mastery_explorer");
        let mm: i64 = r.get("mastery_merchant");
        // 屬性點 5 欄（migration 0024）建為 INTEGER(INT4)→必須以 i32 解碼，否則 sqlx ColumnDecode
        // 會 panic（Rust i64/INT8 與 SQL INT4 不相容），讀玩家位置時整個 server 開機就掛、crash loop。
        // 其他 i64 欄（ether/exp/masteries）的欄位本就是 BIGINT，故維持 i64。
        let su: i32 = r.get("stat_unspent");
        let sh: i32 = r.get("stat_hp");
        let sa: i32 = r.get("stat_attack");
        let ss: i32 = r.get("stat_speed");
        let sas: i32 = r.get("stat_atk_speed");
        // 技能熟練度 5 欄（migration 0025）同為 INTEGER(INT4)，同樣 i32 解碼。
        let sku_wc: i32 = r.get("skill_use_warcry");
        let sku_bo: i32 = r.get("skill_use_bounty");
        let sku_pr: i32 = r.get("skill_use_precision");
        let sku_ga: i32 = r.get("skill_use_gale");
        let sku_ha: i32 = r.get("skill_use_haggle");
        // 生態圖鑑 bitmask（ROADMAP 333，migration 0026）：BIGINT 欄，i64 解碼後位元重塑回 u64。
        let codex: i64 = r.get("codex");
        let (x, y) = spawn_at(Some((x, y)));
        map.insert(
            id,
            Saved {
                x,
                y,
                ether: ether.max(0) as u32,
                wallet_expansions: wallet_expansions.max(0) as u32,
                exp: exp.max(0) as u32,
                masteries: crate::class::Masteries {
                    warrior:  mw.max(0) as u32,
                    farmer:   mf.max(0) as u32,
                    artisan:  ma.max(0) as u32,
                    explorer: me.max(0) as u32,
                    merchant: mm.max(0) as u32,
                },
                stats: crate::stat_points::StatPoints {
                    unspent:   su.max(0) as u32,
                    hp:        sh.max(0) as u32,
                    attack:    sa.max(0) as u32,
                    speed:     ss.max(0) as u32,
                    atk_speed: sas.max(0) as u32,
                },
                skill_masteries: crate::skill_mastery::SkillMasteries {
                    warcry:    sku_wc.max(0) as u32,
                    bounty:    sku_bo.max(0) as u32,
                    precision: sku_pr.max(0) as u32,
                    gale:      sku_ga.max(0) as u32,
                    haggle:    sku_ha.max(0) as u32,
                },
                codex: codex as u64,
            },
        );
    }
    map
}

fn load_from_disk(path: &str) -> HashMap<Uuid, Saved> {
    let mut map = HashMap::new();
    if let Ok(contents) = std::fs::read_to_string(path) {
        for line in contents.lines() {
            if let Ok(r) = serde_json::from_str::<DiskRow>(line) {
                let (x, y) = spawn_at(Some((r.x, r.y)));
                map.insert(r.id, Saved {
                    x, y,
                    ether: r.ether,
                    wallet_expansions: r.wallet_expansions,
                    exp: r.exp,
                    masteries: crate::class::Masteries {
                        warrior:  r.mastery_warrior,
                        farmer:   r.mastery_farmer,
                        artisan:  r.mastery_artisan,
                        explorer: r.mastery_explorer,
                        merchant: r.mastery_merchant,
                    },
                    stats: crate::stat_points::StatPoints {
                        unspent:   r.stat_unspent,
                        hp:        r.stat_hp,
                        attack:    r.stat_attack,
                        speed:     r.stat_speed,
                        atk_speed: r.stat_atk_speed,
                    },
                    skill_masteries: crate::skill_mastery::SkillMasteries {
                        warcry:    r.skill_use_warcry,
                        bounty:    r.skill_use_bounty,
                        precision: r.skill_use_precision,
                        gale:      r.skill_use_gale,
                        haggle:    r.skill_use_haggle,
                    },
                    codex: r.codex,
                });
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{WORLD_HEIGHT, WORLD_WIDTH};

    #[test]
    fn world_core_safe_zone_matches_game() {
        // world-core 地形挖空圈必須對齊遊戲出生點 + 安全半徑,否則新手村會被埋 / 挖空錯位。
        let (cx, cy) = default_spawn();
        assert_eq!(world_core::SAFE_ZONE_CX as f32, cx);
        assert_eq!(world_core::SAFE_ZONE_CY as f32, cy);
        assert_eq!(world_core::SAFE_ZONE_RADIUS as f32, SAFE_SPAWN_RADIUS);
        // 出生點該是乾淨地（被挖空），玩家不會一進場就卡在土裡。
        assert_eq!(
            world_core::tile_kind_at(cx as f64, cy as f64),
            world_core::TileKind::Empty
        );
    }

    #[test]
    fn default_spawn_is_in_safe_zone() {
        let (cx, cy) = default_spawn();
        assert!(is_in_safe_zone(cx, cy), "新手村生成點本身必定在安全區內");
    }

    #[test]
    fn safe_zone_excludes_distant_point() {
        let (cx, cy) = default_spawn();
        // 主城保護圈＝牆內 34 格＋緩衝 8 格 ≈ 1344px（Chebyshev）。取明顯更遠的點驗證圈外。
        // （圍牆城鎮後保護圈比舊圓形 640px 大——這是刻意的：城變大、怪離城門更遠。）
        assert!(!is_in_safe_zone(cx + SAFE_SPAWN_RADIUS * 3.0, cy));
        assert!(!is_in_safe_zone(cx, cy + SAFE_SPAWN_RADIUS * 3.0));
    }

    #[test]
    fn safe_zone_edge_is_inside() {
        let (cx, cy) = default_spawn();
        // 剛好在半徑上的點算在安全區內（`<=`）。
        assert!(is_in_safe_zone(cx + SAFE_SPAWN_RADIUS, cy));
    }

    #[test]
    fn spawn_falls_back_to_center_when_no_history() {
        assert_eq!(spawn_at(None), default_spawn());
    }

    #[test]
    fn spawn_uses_recalled_position() {
        assert_eq!(spawn_at(Some((123.0, 456.0))), (123.0, 456.0));
    }

    #[test]
    fn spawn_falls_back_to_center_on_non_finite() {
        // 壞掉的持久化座標（NaN/Inf）不該把玩家生到非有限位置。
        assert_eq!(spawn_at(Some((f32::NAN, 100.0))), default_spawn());
        assert_eq!(spawn_at(Some((100.0, f32::INFINITY))), default_spawn());
        assert_eq!(spawn_at(Some((f32::NEG_INFINITY, f32::NAN))), default_spawn());
    }

    #[test]
    fn spawn_preserves_out_of_bounds_finite_coordinates() {
        // 在無限世界中，界外的有限座標應原樣保留，不被夾回邊界。
        assert_eq!(spawn_at(Some((-50.0, -50.0))), (-50.0, -50.0));
        assert_eq!(
            spawn_at(Some((WORLD_WIDTH + 999.0, WORLD_HEIGHT + 999.0))),
            (WORLD_WIDTH + 999.0, WORLD_HEIGHT + 999.0)
        );
    }

    #[test]
    fn recall_is_none_before_remember() {
        let store = PositionStore::in_memory();
        assert_eq!(store.recall(Uuid::new_v4()), None);
    }

    #[test]
    fn remember_then_recall_round_trips() {
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, 10.0, 20.0, 5, 0, 0, crate::class::Masteries::default(), crate::stat_points::StatPoints::default(), crate::skill_mastery::SkillMasteries::default(), 0);
        assert_eq!(
            store.recall(id),
            Some(Saved {
                x: 10.0,
                y: 20.0,
                ether: 5,
                wallet_expansions: 0,
                exp: 0,
                masteries: crate::class::Masteries::default(),
                stats: crate::stat_points::StatPoints::default(),
                skill_masteries: crate::skill_mastery::SkillMasteries::default(),
                codex: 0,
            })
        );
    }

    #[test]
    fn remember_overwrites_previous_state() {
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, 10.0, 20.0, 1, 0, 0, crate::class::Masteries::default(), crate::stat_points::StatPoints::default(), crate::skill_mastery::SkillMasteries::default(), 0);
        store.remember(id, 30.0, 40.0, 9, 2, 100, crate::class::Masteries::default(), crate::stat_points::StatPoints::default(), crate::skill_mastery::SkillMasteries::default(), 0);
        assert_eq!(
            store.recall(id),
            Some(Saved {
                x: 30.0,
                y: 40.0,
                ether: 9,
                wallet_expansions: 2,
                exp: 100,
                masteries: crate::class::Masteries::default(),
                stats: crate::stat_points::StatPoints::default(),
                skill_masteries: crate::skill_mastery::SkillMasteries::default(),
                codex: 0,
            })
        );
    }

    #[test]
    fn recalled_ether_survives_round_trip() {
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, 0.0, 0.0, 42, 3, 200, crate::class::Masteries::default(), crate::stat_points::StatPoints::default(), crate::skill_mastery::SkillMasteries::default(), 0);
        assert_eq!(store.recall(id).map(|s| s.ether), Some(42));
        assert_eq!(store.recall(id).map(|s| s.wallet_expansions), Some(3));
        assert_eq!(store.recall(id).map(|s| s.exp), Some(200));
    }

    #[test]
    fn masteries_persisted_and_recalled() {
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        let m = crate::class::Masteries { warrior: 15, farmer: 5, ..Default::default() };
        store.remember(id, 0.0, 0.0, 0, 0, 0, m, crate::stat_points::StatPoints::default(), crate::skill_mastery::SkillMasteries::default(), 0);
        assert_eq!(store.recall(id).map(|s| s.masteries), Some(m));
    }

    #[test]
    fn saved_round_trips_through_serde() {
        let s = Saved {
            x: 123.5,
            y: 678.25,
            ether: 7,
            wallet_expansions: 2,
            exp: 150,
            masteries: crate::class::Masteries { warrior: 10, ..Default::default() },
            stats: crate::stat_points::StatPoints::default(),
            skill_masteries: crate::skill_mastery::SkillMasteries::default(),
            codex: 0,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Saved = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn saved_defaults_masteries_when_missing_from_old_json() {
        // 舊存檔沒有 masteries 欄位 → 安全讀為全零。
        let json = r#"{"x":1.0,"y":2.0,"ether":5,"wallet_expansions":0,"exp":50}"#;
        let s: Saved = serde_json::from_str(json).unwrap();
        assert_eq!(s.masteries, crate::class::Masteries::default());
    }

    #[test]
    fn loaded_bad_position_still_gated_by_spawn_at() {
        let bad = Saved {
            x: f32::INFINITY,
            y: WORLD_HEIGHT + 9999.0,
            ether: 1,
            wallet_expansions: 0,
            exp: 0,
            masteries: crate::class::Masteries::default(),
            stats: crate::stat_points::StatPoints::default(),
            skill_masteries: crate::skill_mastery::SkillMasteries::default(),
            codex: 0,
        };
        let (x, y) = spawn_at(Some((bad.x, bad.y)));
        assert_eq!((x, y), default_spawn());

        let out_of_bounds = Saved {
            x: -100.0,
            y: WORLD_HEIGHT + 100.0,
            ether: 1,
            wallet_expansions: 0,
            exp: 0,
            masteries: crate::class::Masteries::default(),
            stats: crate::stat_points::StatPoints::default(),
            skill_masteries: crate::skill_mastery::SkillMasteries::default(),
            codex: 0,
        };
        let (x, y) = spawn_at(Some((out_of_bounds.x, out_of_bounds.y)));
        assert_eq!((x, y), (-100.0, WORLD_HEIGHT + 100.0));
    }

    #[test]
    fn stores_are_independent_per_player() {
        let store = PositionStore::in_memory();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store.remember(a, 1.0, 1.0, 3, 0, 0, crate::class::Masteries::default(), crate::stat_points::StatPoints::default(), crate::skill_mastery::SkillMasteries::default(), 0);
        assert_eq!(store.recall(b), None);
        assert_eq!(
            store.recall(a),
            Some(Saved {
                x: 1.0,
                y: 1.0,
                ether: 3,
                wallet_expansions: 0,
                exp: 0,
                masteries: crate::class::Masteries::default(),
                stats: crate::stat_points::StatPoints::default(),
                skill_masteries: crate::skill_mastery::SkillMasteries::default(),
                codex: 0,
            })
        );
    }

    #[tokio::test]
    async fn flush_is_noop_without_postgres() {
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        let m = crate::class::Masteries::default();
        let s = crate::stat_points::StatPoints::default();
        let sk = crate::skill_mastery::SkillMasteries::default();
        store
            .flush_online(&[(id, "阿巡".into(), "terran".into(), 1.0, 2.0, 3, 0, 0, m, s, sk, 0)])
            .await;
        store.flush_one(id, "阿巡", "terran", 1.0, 2.0, 3, 0, 0, m, s, sk, 0).await;
        assert_eq!(store.recall(id), None);
    }
}
