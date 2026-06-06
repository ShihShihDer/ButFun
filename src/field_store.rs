//! 玩家農地的伺服器端記憶（Phase 0-E 持久化，沿 `inventory_store.rs` 同一套抽換結構）。
//!
//! 採集/打怪/收成三來源灌背包已接 PG（0-E，見 inventory_store）；農地是同一個缺口的另一半
//! ——種田的進度（翻土／播種／澆水／成長）同樣撐不過 server 重啟（換版）。這層讓已登入玩家
//! 的整塊地跟著重啟回來，連帶把「哪個玩家擁有第幾塊地」（plot 序號）一起存好，重啟後序號
//! 歸屬不錯亂。
//!
//! 與背包的差異：農地存的是「(plot 序號, 整塊地序列化)」一對。序號不可省——`Field` 的 origin
//! 不入存檔（見 `field.rs`），載回時要靠序號用 `Field::reseated` 安置回正確位置；序號本身也餵
//! `PlotRegistry::from_saved` 重建歸屬、確保續發序號不撞既有地塊。故啟動時這個 store 同時
//! 餵養兩份權威狀態：`AppState::fields`（每塊地）與 `AppState::plots`（序號歸屬）。
//!
//! 行程內維護一份 `id -> (序號, Field)` 的記憶體 cache，耐久層可抽換（同 inventory_store）：
//!   - `Postgres`：設了 `DATABASE_URL` 時啟動載回、定期非同步 upsert（正式上線走這條）。
//!   - `Jsonl`：沒設 `DATABASE_URL`（本機 `cargo run`）時寫穿 `data/fields.jsonl`。
//!   - `Memory`：測試用，不碰磁碟也不碰 DB。
//!
//! 只記「已登入」玩家（穩定 id）；訪客不分地、自然不記。flush 失敗只記 log 不中斷遊戲迴圈；
//! 載入逐列過 `Field::reseated`（格數正確 + 每株作物健全）驗證，壞檔（格數錯、作物 NaN/Inf/
//! 負成長、被竄改）整列丟棄，不把壞值帶進世界。

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use sqlx::Row;
use uuid::Uuid;

use crate::field::Field;

/// 無 `DATABASE_URL` 時的退回持久化檔（執行期產生、已 gitignore）。接 Postgres 後這個檔
/// 退為「遷移種子」：啟動時 DB 沒有的 id 仍會從這裡補回 cache（同 inventory_store）。
const STORE_PATH: &str = "data/fields.jsonl";

/// cache 裡一塊地連同它的序號。序號決定 origin（載入時 reseat 用）與歸屬（餵 PlotRegistry）。
#[derive(Clone)]
struct Stored {
    plot_index: usize,
    field: Field,
}

/// JSONL 一行紀錄：id + 序號 + 整塊地序列化（`field` 形如 `{"tiles":[...]}`，origin 不入存檔）。
#[derive(Serialize, Deserialize)]
struct DiskRow {
    id: Uuid,
    plot_index: usize,
    field: Field,
}

/// cache 後面的耐久層（同 inventory_store 的三態）。
#[derive(Clone)]
enum Backend {
    /// 測試用：不載入、不寫。只在 `#[cfg(test)]` 的 `in_memory()` 建構，故非測試建置標 allow。
    #[cfg_attr(not(test), allow(dead_code))]
    Memory,
    /// 沒設 `DATABASE_URL`：寫穿到此 JSONL 檔。
    Jsonl(&'static str),
    /// 設了 `DATABASE_URL`：啟動載回、定期非同步 upsert 到 `fields` 表。
    Postgres(PgPool),
}

/// 記住玩家農地的儲存層。記憶體 cache + 可抽換耐久層（Postgres / JSONL / 純記憶體）。
#[derive(Clone)]
pub struct FieldStore {
    inner: Arc<RwLock<HashMap<Uuid, Stored>>>,
    backend: Backend,
}

impl Default for FieldStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FieldStore {
    /// 無 DB 模式：cache 從 JSONL 載入，之後寫穿 JSONL。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(load_from_disk(STORE_PATH))),
            backend: Backend::Jsonl(STORE_PATH),
        }
    }

    /// Postgres 模式（正式上線）：啟動時把 `fields` 表載回 cache，再用既有 JSONL 補齊 DB 還
    /// 沒有的 id。「DB 為主、JSONL 補洞」的順序與 `InventoryStore::from_pool` 一致——換版時
    /// DB 可能還空，不從 JSONL 種回會讓 returning 玩家重啟後丟掉整塊地。
    pub async fn from_pool(pool: PgPool) -> Self {
        let cache = merge_seed(load_fields_from_db(&pool).await, load_from_disk(STORE_PATH));
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

    /// 啟動時把所有持久化的農地交給 `AppState`：`id -> Field`（origin 已於載入時 reseat 好）。
    /// 重啟後直接灌進權威 `fields` map，returning 玩家重連看得到自己離線前的進度。
    pub fn loaded_fields(&self) -> HashMap<Uuid, Field> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .map(|(id, s)| (*id, s.field.clone()))
            .collect()
    }

    /// 啟動時把所有持久化的「(玩家, 序號)」交給 `PlotRegistry::from_saved` 重建歸屬，確保
    /// returning 玩家重連拿回原序號、且續發序號不撞既有地塊（見 plot_registry 的不變式）。
    pub fn saved_plots(&self) -> Vec<(Uuid, usize)> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .map(|(id, s)| (*id, s.plot_index))
            .collect()
    }

    /// 記住某玩家目前的農地（更新 cache，同步）。Jsonl 模式順手寫穿磁碟；Postgres 模式只動
    /// cache，耐久寫入交給非同步的 `flush_online`/`flush_one`（DB 是 async，不在同步路徑上 await）。
    pub fn remember(&self, id: Uuid, plot_index: usize, field: &Field) {
        self.inner.write().unwrap().insert(
            id,
            Stored {
                plot_index,
                field: field.clone(),
            },
        );
        self.persist_jsonl();
    }

    /// 批次記住多塊地（給遊戲迴圈定期快照用）：更新 cache 一次。
    pub fn remember_all<I: IntoIterator<Item = (Uuid, usize, Field)>>(&self, items: I) {
        {
            let mut m = self.inner.write().unwrap();
            for (id, plot_index, field) in items {
                m.insert(id, Stored { plot_index, field });
            }
        }
        self.persist_jsonl();
    }

    /// 把農地批次 upsert 到 Postgres（遊戲迴圈每 ~10 秒呼叫）。非 Postgres 模式無動作。
    /// 失敗只記 log、不中斷遊戲迴圈（下一輪再試；cache 仍是行程內權威）。
    pub async fn flush_online(&self, rows: &[(Uuid, usize, Field)]) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        if let Err(e) = upsert_rows(pool, rows).await {
            tracing::warn!("Postgres 農地 flush_online 失敗（下一輪再試）：{e}");
        }
    }

    /// 玩家離線時把其最後農地 upsert 到 Postgres（補上「最後一次 10s flush 後到離線之間」種/澆/
    /// 收的進度）。非 Postgres 模式無動作。
    pub async fn flush_one(&self, id: Uuid, plot_index: usize, field: &Field) {
        let Backend::Postgres(pool) = &self.backend else {
            return;
        };
        let row = [(id, plot_index, field.clone())];
        if let Err(e) = upsert_rows(pool, &row).await {
            tracing::warn!("Postgres 農地 flush_one 失敗：{e}");
        }
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
                        plot_index: s.plot_index,
                        field: s.field.clone(),
                    })
                    .ok()
                })
                .collect()
        };
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // 先寫暫存再 rename，避免寫到一半被重啟而毀檔（同 inventory_store）。
        let tmp = format!("{path}.tmp");
        if let Ok(mut f) = std::fs::File::create(&tmp) {
            let _ = f.write_all(rows.join("\n").as_bytes());
            let _ = f.write_all(b"\n");
            let _ = f.sync_all();
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// 批次 upsert 到 `fields` 表（一筆 transaction，要嘛全進要嘛全不進）。tiles 存整塊地序列化後
/// 的 JSON 字串。走 runtime query API（非 `query!` 巨集），故 build/test 不需 live DB。
async fn upsert_rows(pool: &PgPool, rows: &[(Uuid, usize, Field)]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (id, plot_index, field) in rows {
        // 序列化農地；理論上不會失敗，萬一失敗就跳過這筆，不毀整批。
        let Ok(json) = serde_json::to_string(field) else {
            continue;
        };
        sqlx::query(
            "INSERT INTO fields (player_id, plot_index, tiles, updated_at) \
             VALUES ($1, $2, $3, now()) \
             ON CONFLICT (player_id) DO UPDATE SET \
               plot_index = EXCLUDED.plot_index, tiles = EXCLUDED.tiles, updated_at = now()",
        )
        .bind(id)
        .bind(*plot_index as i32)
        .bind(json)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// 啟動時把 `fields` 表載回 cache。每列過 `Field::reseated` 驗證（格數正確 + 作物健全），
/// 並由序號重建 origin；壞檔/被竄改的存檔一律跳過，不把壞值帶進世界。載入失敗（DB 連線剛斷
/// 等）回空 map，讓伺服器仍能起來、之後再寫回。
async fn load_fields_from_db(pool: &PgPool) -> HashMap<Uuid, Stored> {
    let mut map = HashMap::new();
    let rows = match sqlx::query("SELECT player_id, plot_index, tiles FROM fields")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("從 Postgres 載入 fields 失敗（先以空 cache 起來）：{e}");
            return map;
        }
    };
    for r in rows {
        let id: Uuid = r.get("player_id");
        let plot_index = r.get::<i32, _>("plot_index").max(0) as usize;
        let tiles: String = r.get("tiles");
        if let Ok(raw) = serde_json::from_str::<Field>(&tiles) {
            if let Some(field) = raw.reseated(plot_index) {
                map.insert(id, Stored { plot_index, field });
            }
        }
    }
    map
}

/// 把 JSONL 種子併入從 DB 載回的 cache：**DB 為主**。DB 已有的玩家不被種子覆蓋；種子裡 DB 還
/// 沒有的玩家才補進來,且其序號不得撞到任何已被占用的序號（DB 的、或先補進的種子列），撞到就跳過。
///
/// 為何需要這道防線:`PlotRegistry::from_saved` 的招牌不變式是「一塊地一個地主」,它倚賴「持久化
/// 端保證序號唯一」。Postgres 端有 `fields_plot_index_key` UNIQUE 約束撐著,但 JSONL 種子**沒有**
/// ——壞檔/手改、或換版遷移窗口裡 DB 與舊種子各記了同一序號的不同玩家,天真合併會讓兩個 id 帶同一
/// 序號流進 `loaded_fields()`／`saved_plots()`,造成「同一塊地兩個地主、作物歸屬錯亂」。延續本專案
/// `field::reseated`／`users::parse_and_sanitize` 那條「存檔又重載一律在載入路徑驗證不變式」的硬化弧線。
///
/// 種子 `HashMap` 迭代序不定,故先依 `(序號, id)` 排序,讓撞號時保留誰具決定性（不依賴雜湊隨機序）。
fn merge_seed(
    mut db: HashMap<Uuid, Stored>,
    seed: HashMap<Uuid, Stored>,
) -> HashMap<Uuid, Stored> {
    let mut used: HashSet<usize> = db.values().map(|s| s.plot_index).collect();
    let mut rows: Vec<(Uuid, Stored)> = seed.into_iter().collect();
    rows.sort_by_key(|(id, s)| (s.plot_index, *id));
    for (id, stored) in rows {
        if db.contains_key(&id) {
            continue; // DB 已有此玩家,以 DB 為準（不被舊種子覆蓋）。
        }
        if !used.insert(stored.plot_index) {
            tracing::warn!(
                "JSONL 補洞跳過 player {id} 的地塊:序號 {} 已被占用（避免同一塊地兩個地主）",
                stored.plot_index
            );
            continue;
        }
        db.insert(id, stored);
    }
    db
}

fn load_from_disk(path: &str) -> HashMap<Uuid, Stored> {
    let mut map = HashMap::new();
    // 守住「一塊地一個地主」:JSONL 沒有 DB 的 UNIQUE 約束,壞檔/手改可能讓兩列不同 id 撞同一序號。
    // 依檔案順序先到先得,後到的撞號列跳過（同一 id 更新同序號仍放行）。
    let mut owner_of_index: HashMap<usize, Uuid> = HashMap::new();
    if let Ok(contents) = std::fs::read_to_string(path) {
        for line in contents.lines() {
            if let Ok(r) = serde_json::from_str::<DiskRow>(line) {
                // 同 DB 載入：經 reseated 安置回序號的 origin 並驗證；壞檔（格數/作物）一律跳過。
                let Some(field) = r.field.reseated(r.plot_index) else {
                    continue;
                };
                if let Some(&owner) = owner_of_index.get(&r.plot_index) {
                    if owner != r.id {
                        tracing::warn!(
                            "載入 fields.jsonl 跳過 player {} 的地塊:序號 {} 已屬 {}（壞檔/手改撞號,避免重複地主）",
                            r.id,
                            r.plot_index,
                            owner
                        );
                        continue;
                    }
                }
                owner_of_index.insert(r.plot_index, r.id);
                map.insert(
                    r.id,
                    Stored {
                        plot_index: r.plot_index,
                        field,
                    },
                );
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 蓋一塊種到一半的地（序號 `index`），用來驗持久化進出。
    fn growing_field(index: usize) -> Field {
        use crate::crops::SPROUT_AT;
        let mut f = Field::for_plot(index);
        f.till(0, 0);
        f.plant(0, 0);
        f.water(0, 0);
        f.tick(SPROUT_AT + 1.0);
        f
    }

    #[test]
    fn empty_before_remember() {
        let store = FieldStore::in_memory();
        assert!(store.loaded_fields().is_empty());
        assert!(store.saved_plots().is_empty());
    }

    #[test]
    fn remember_reflected_in_loaded_fields_and_plots() {
        let store = FieldStore::in_memory();
        let id = Uuid::new_v4();
        let f = growing_field(2);
        store.remember(id, 2, &f);

        let loaded = store.loaded_fields();
        assert_eq!(loaded.get(&id), Some(&f));
        assert_eq!(store.saved_plots(), vec![(id, 2)]);
    }

    #[test]
    fn remember_overwrites_previous() {
        let store = FieldStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, 1, &Field::for_plot(1));
        let grown = growing_field(1);
        store.remember(id, 1, &grown);
        assert_eq!(store.loaded_fields().get(&id), Some(&grown));
    }

    #[test]
    fn remember_all_updates_many_at_once() {
        let store = FieldStore::in_memory();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store.remember_all([(a, 0, Field::for_plot(0)), (b, 3, growing_field(3))]);
        let loaded = store.loaded_fields();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get(&a), Some(&Field::for_plot(0)));
        assert_eq!(loaded.get(&b), Some(&growing_field(3)));
        let mut plots = store.saved_plots();
        plots.sort_by_key(|(_, idx)| *idx);
        assert_eq!(plots, vec![(a, 0), (b, 3)]);
    }

    #[test]
    fn disk_row_serde_round_trips_field_and_index() {
        // JSONL/DB 共用的序列化格式地基：id + 序號 + 整塊地序列化再讀回（origin 經 reseat 由
        // 序號重建）要一模一樣，尤其是長到中段的作物。
        let id = Uuid::new_v4();
        let f = growing_field(4);
        let row = DiskRow {
            id,
            plot_index: 4,
            field: f.clone(),
        };
        let json = serde_json::to_string(&row).unwrap();
        let back: DiskRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, id);
        assert_eq!(back.plot_index, 4);
        // serde 還原的 field origin 退回 (0,0)；reseat 回序號 4 後整塊一致。
        assert_eq!(back.field.reseated(4), Some(f));
    }

    fn stored(index: usize) -> Stored {
        Stored {
            plot_index: index,
            field: Field::for_plot(index),
        }
    }

    #[test]
    fn merge_seed_db_wins_for_same_player() {
        // DB 已有此玩家:種子裡同 id 的舊紀錄不得覆蓋 DB（即使序號不同）。
        let id = Uuid::new_v4();
        let db = HashMap::from([(id, stored(0))]);
        let seed = HashMap::from([(id, stored(5))]);
        let merged = merge_seed(db, seed);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged.get(&id).unwrap().plot_index, 0, "DB 序號為準");
    }

    #[test]
    fn merge_seed_fills_only_new_players() {
        // 種子裡 DB 還沒有、且序號不撞的玩家才補進來（換版遷移:DB 還空時從 JSONL 種回）。
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let db = HashMap::from([(a, stored(0))]);
        let seed = HashMap::from([(b, stored(1))]);
        let merged = merge_seed(db, seed);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.get(&a).unwrap().plot_index, 0);
        assert_eq!(merged.get(&b).unwrap().plot_index, 1);
    }

    #[test]
    fn merge_seed_skips_seed_row_colliding_with_db_index() {
        // 招牌防線:種子裡某玩家的序號已被 DB 的別人占用 → 跳過,避免「同一塊地兩個地主」。
        let db_owner = Uuid::new_v4();
        let seed_owner = Uuid::new_v4();
        let fresh = Uuid::new_v4();
        let db = HashMap::from([(db_owner, stored(0))]);
        let seed = HashMap::from([(seed_owner, stored(0)), (fresh, stored(1))]);
        let merged = merge_seed(db, seed);
        // 序號 0 仍只屬 DB 的地主;撞號的種子列被丟,不撞號的 fresh 照補。
        assert_eq!(merged.get(&db_owner).unwrap().plot_index, 0);
        assert!(!merged.contains_key(&seed_owner), "撞 DB 序號的種子列應跳過");
        assert_eq!(merged.get(&fresh).unwrap().plot_index, 1);
        // 每個序號至多一個地主（招牌不變式）。
        let indices: Vec<usize> = merged.values().map(|s| s.plot_index).collect();
        let unique: HashSet<usize> = indices.iter().copied().collect();
        assert_eq!(indices.len(), unique.len(), "不得有兩個地主共用同一序號");
    }

    #[test]
    fn merge_seed_dedups_within_seed_deterministically() {
        // DB 空、種子裡兩個不同 id 撞同一序號:保留依 (序號,id) 排序的前者,且結果具決定性。
        let mut lo = Uuid::new_v4();
        let mut hi = Uuid::new_v4();
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        let seed = HashMap::from([(lo, stored(0)), (hi, stored(0))]);
        let merged = merge_seed(HashMap::new(), seed);
        assert_eq!(merged.len(), 1, "撞號只留一個地主");
        assert!(merged.contains_key(&lo), "依 (序號,id) 排序保留較小 id");
    }

    #[test]
    fn load_from_disk_skips_duplicate_plot_index() {
        // 壞檔/手改:兩列不同 id 撞同一序號。載入後該序號只能有一個地主（先到先得）。
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("butfun_fieldstore_dup_{a}.jsonl"));
        let path = path.to_str().unwrap();
        let rows = [
            DiskRow {
                id: a,
                plot_index: 0,
                field: Field::for_plot(0),
            },
            DiskRow {
                id: b,
                plot_index: 0, // 撞號:同序號不同玩家
                field: Field::for_plot(0),
            },
        ];
        let body: Vec<String> = rows.iter().map(|r| serde_json::to_string(r).unwrap()).collect();
        std::fs::write(path, body.join("\n")).unwrap();

        let map = load_from_disk(path);
        let _ = std::fs::remove_file(path);

        assert_eq!(map.len(), 1, "撞號的第二列應被跳過");
        assert!(map.contains_key(&a), "先到的 a 保住序號 0");
        assert!(!map.contains_key(&b), "後到撞號的 b 被丟");
    }

    #[tokio::test]
    async fn flush_is_noop_without_postgres() {
        // 非 Postgres 模式（測試）下，flush_* 不該 panic、也不需 DB。
        let store = FieldStore::in_memory();
        let id = Uuid::new_v4();
        let f = growing_field(0);
        store.flush_online(&[(id, 0, f.clone())]).await;
        store.flush_one(id, 0, &f).await;
        // cache 不受 flush 影響（flush 只負責耐久寫出，不改 cache）。
        assert!(store.loaded_fields().is_empty());
    }
}
