//! 遊戲內建議箱 —— 玩家回饋迴圈的伺服器端。
//!
//! 這直接服務「玩家在遊戲裡送建議 → 我收到 → 改版 → 發佈」的營運迴圈。
//! 建議同時存在記憶體（即時列出）與附加到耐久層（重啟後仍在）。建議是真實玩家資料,
//! 耐久層在記憶體 Vec 後面可抽換(沿 `users.rs` 同一套 0-E 結構):
//!   - `Postgres`：設了 `DATABASE_URL` 時,啟動載回全部建議、新增時非同步 insert(正式上線走這條)。
//!   - `Jsonl`：沒設 `DATABASE_URL`(本機 `cargo run`)時 append 寫穿 `data/suggestions.jsonl`。
//!   - `Memory`：測試用,不碰磁碟也不碰 DB。
//!
//! 延續其他 store 的設計權衡(DB 為主、JSONL 補洞;寫入失敗只記 log 不中斷送出;載入時
//! 一律過 sanitizer 驗壞值)。建議是 append-only、無更新語意,故沒有 upsert,只有 insert。

use std::collections::HashSet;
use std::io::Write;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use sqlx::Row;

/// 無 `DATABASE_URL` 時的退回持久化檔(執行期產生、已 gitignore)。接 Postgres 後這個檔
/// 退為「遷移種子」：啟動時 DB 還沒有的建議仍會從這裡補回記憶體並一次性 insert 進 DB,
/// 讓換版(從 JSONL 版切到 Postgres 版)不會把既有建議丟掉。
const LOG_PATH: &str = "data/suggestions.jsonl";

/// 建議署名最長字元數（與玩家名 `sanitize_name` 的上限一致）。
pub const MAX_FROM_CHARS: usize = 24;
/// 建議內容最長字元數。夠寫一整段心得，又擋掉「灌爆建議檔」的濫用 / 壞客戶端。
pub const MAX_TEXT_CHARS: usize = 1000;

/// 一則玩家建議。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub from: String,
    pub text: String,
    /// Unix 毫秒時間戳。
    pub at: u64,
}

/// 進來的建議（HTTP 請求 body）。
#[derive(Debug, Deserialize)]
pub struct NewSuggestion {
    #[serde(default = "anonymous")]
    pub from: String,
    pub text: String,
}

fn anonymous() -> String {
    "匿名拓荒者".to_string()
}

/// 把進來的署名 / 內容整理成要存下的 `Suggestion`：先濾控制字元、去頭尾空白、依「字元」
/// (非位元組,中文才不會被切壞)截到上限、空署名退回匿名。抽成純函式以便測試,把這條公開
/// endpoint 的輸入加固集中在一處(對齊聊天截 200 字、`sanitize_name` 截 24 字)。
///
/// 濾控制字元是必要的,且補齊先前只做 `trim`+`take` 的缺口:建議經公開未驗身的
/// `POST /api/suggestions` 進來、又會由公開的 `GET /api/suggestions` 回出,而我(維護者)
/// 多半直接在終端機讀 `data/suggestions.jsonl` 三角化——`ESC`(0x1B)等控制字元可被用來注入
/// ANSI 轉義、偽造或破壞顯示,`NUL` / `\r` 同理。比照 `sanitize_name` / `sanitize_chat`,
/// 控制字元先濾掉(不佔截斷額度)。兩欄差別:署名是單行身分欄位(對齊 `sanitize_name`),濾掉
/// 全部控制字元;內容是多行回饋(前端 `<textarea>`),保留換行 `\n` 讓玩家能分段,只濾掉換行
/// 以外的控制字元——換行存進 JSONL 會被 serde 轉義成 `\n`、不會把一筆紀錄拆成多行。
///
/// 清乾淨後內容變空(全空白 / 全控制字元)回 `None`,呼叫端據此不存——比照 `sanitize_chat`
/// 回 `Option` 的模式。這把「擋空建議」的判斷對齊到「實際會被存下的內容」這個單一真實
/// 來源:endpoint 先前只對 raw `text.trim()` 判空,而 `trim` 不濾控制字元,一則「全控制
/// 字元」的內容(如 `\0`/`ESC`,皆非空白)會通過 raw 檢查、卻在這裡被濾成空字串,仍寫進
/// JSONL 留下空建議垃圾紀錄。
fn sanitize(from: &str, text: &str, at: u64) -> Option<Suggestion> {
    let from: String = from
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_FROM_CHARS)
        .collect();
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_TEXT_CHARS)
        .collect();
    if text.is_empty() {
        return None;
    }
    Some(Suggestion {
        from: if from.is_empty() { anonymous() } else { from },
        text,
        at,
    })
}

/// 記憶體 Vec 後面的耐久層。
#[derive(Clone)]
enum Backend {
    /// 測試用：不載入、不寫。只在 `#[cfg(test)]` 的 `in_memory()` 建構,故非測試建置標 allow。
    #[cfg_attr(not(test), allow(dead_code))]
    Memory,
    /// 沒設 `DATABASE_URL`：append 寫穿到此 JSONL 檔。
    Jsonl(&'static str),
    /// 設了 `DATABASE_URL`：啟動載回全部建議、新增時 insert 到 `suggestions` 表。
    Postgres(PgPool),
}

/// 建議的存放處。可被複製（內部共享）。
#[derive(Clone)]
pub struct SuggestionStore {
    items: Arc<Mutex<Vec<Suggestion>>>,
    backend: Backend,
}

impl SuggestionStore {
    /// 無 DB 模式(測試、本機 `cargo run`)：記憶體從 JSONL 載入,新增時 append 寫穿 JSONL。
    pub fn new() -> Self {
        let items = load_from_disk();
        Self {
            items: Arc::new(Mutex::new(items)),
            backend: Backend::Jsonl(LOG_PATH),
        }
    }

    /// Postgres 模式(正式上線)：啟動時把 `suggestions` 表全部載回,再用既有 JSONL 補齊
    /// DB 還沒有的建議——並把這些補進來的建議**一次性 insert 進 DB**,讓換版(從 JSONL 版
    /// 切到 Postgres 版)時既有建議不會丟。建議無自然主鍵(append-only),回填用
    /// `(from, text, at)` 三元組去重保證冪等：已在 DB 的同內容同毫秒建議不會重插,重啟
    /// 多次也不會把 JSONL 的舊建議灌成重複列。這個「DB 為主、JSONL 補洞」順序與
    /// `UserStore::from_pool` 等一致。
    pub async fn from_pool(pool: PgPool) -> Self {
        let mut items = load_from_db(&pool).await;
        let known: HashSet<(String, String, u64)> = items
            .iter()
            .map(|s| (s.from.clone(), s.text.clone(), s.at))
            .collect();
        // JSONL 裡 DB 還沒有的建議:補進記憶體,並一次性回填進 DB。
        for s in load_from_disk() {
            if !known.contains(&(s.from.clone(), s.text.clone(), s.at)) {
                if let Err(e) = insert_suggestion(&pool, &s).await {
                    tracing::warn!("JSONL 建議回填 Postgres 失敗(仍保留在記憶體):{e}");
                }
                items.push(s);
            }
        }
        Self {
            items: Arc::new(Mutex::new(items)),
            backend: Backend::Postgres(pool),
        }
    }

    /// 純記憶體版(測試用)：不載入、不寫磁碟、不碰 DB。
    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            items: Arc::new(Mutex::new(Vec::new())),
            backend: Backend::Memory,
        }
    }

    /// 新增一則建議；清乾淨後內容變空(全空白 / 全控制字元)回 `None`、不存任何東西，
    /// 否則回存好的紀錄。把「擋空」收斂到實際會被存下的內容上，避免空建議垃圾進檔。
    ///
    /// 耐久寫入在**放開鎖之後** await(DB insert 是 async,不在持鎖路徑上 await——避免跨
    /// await 持 `Mutex`,沿 `UserStore::find_or_create` 的鎖／await 切分)。
    pub async fn add(&self, new: NewSuggestion) -> Option<Suggestion> {
        let suggestion = sanitize(&new.from, &new.text, now_millis())?;
        self.persist(&suggestion).await;
        {
            let mut items = self.items.lock().unwrap();
            items.push(suggestion.clone());
        }
        tracing::info!(from = %suggestion.from, "收到玩家建議：{}", suggestion.text);
        Some(suggestion)
    }

    /// 把一則建議落地到耐久層。Jsonl 模式 append 一行;Postgres 模式 insert 一列;Memory 無動作。
    /// 寫入失敗只記 log、不中斷送出(記憶體仍是行程內權威,GET 仍看得到)。
    async fn persist(&self, s: &Suggestion) {
        match &self.backend {
            Backend::Memory => {}
            Backend::Jsonl(path) => append_to_disk_at(path, s),
            Backend::Postgres(pool) => {
                if let Err(e) = insert_suggestion(pool, s).await {
                    tracing::warn!("Postgres suggestions insert 失敗:{e}");
                }
            }
        }
    }

    /// 列出所有建議（最新的在前）。
    ///
    /// 目前沒有呼叫端：先前的公開 `GET /api/suggestions` 已移除（資料曝露收口，見
    /// `main.rs` 路由註解）。刻意保留這個建材給「日後驗身後的後台檢視」，接上時
    /// 走驗身路由再呼叫即可——故標 `allow(dead_code)`，比照 `crops::is_loadable`
    /// 等「先備好、之後接線」的前置慣例。
    #[allow(dead_code)]
    pub fn list(&self) -> Vec<Suggestion> {
        let items = self.items.lock().unwrap();
        items.iter().rev().cloned().collect()
    }
}

impl Default for SuggestionStore {
    fn default() -> Self {
        Self::new()
    }
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn load_from_disk() -> Vec<Suggestion> {
    match std::fs::read_to_string(LOG_PATH) {
        Ok(contents) => parse_and_sanitize(&contents),
        Err(_) => Vec::new(),
    }
}

/// 把 JSONL 檔內容解析成建議清單，並讓每則**再過一次 `sanitize`**（載入防線）。純函式以便測試。
///
/// 為什麼載入也要過濾：控制字元過濾原本只加在**寫入**路徑（`add`/`sanitize`），但建議是
/// 「存檔又重載」的持久化結構——`data/suggestions.jsonl` 裡可能有**那道硬化 landing 之前**
/// 寫進的舊行，或被手動編輯 / 損毀的行。這些行會由公開的 `GET /api/suggestions` 原樣回出、
/// 又被維護者直接在終端機讀來三角化，殘留的 `ESC`(0x1B) / `NUL` / `\r` 仍能注入 ANSI 轉義、
/// 偽造或破壞顯示。讓讀路徑也走同一個 sanitizer，輸出就用「實際會被存下的乾淨內容」當單一
/// 真實來源，不論磁碟上那行是何時、被什麼寫進去的——延續 `field.rs::from_tiles` /
/// `Crop::is_loadable` / `positions::spawn_at` 在**載入時**驗證壞持久化資料的防線脈絡。
///
/// 刻意**不改寫 / 不刪除**磁碟上的檔（不破壞玩家資料）：只過濾載進記憶體、回給 GET 的內容。
/// 解析失敗的行照舊跳過；清乾淨後變空的行（全控制字元 / 全空白）也丟掉，比照寫入路徑「空建議不存」。
fn parse_and_sanitize(contents: &str) -> Vec<Suggestion> {
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<Suggestion>(line).ok())
        .filter_map(|s| sanitize(&s.from, &s.text, s.at))
        .collect()
}

fn append_to_disk_at(path: &str, s: &Suggestion) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut file) => {
            if let Ok(line) = serde_json::to_string(s) {
                let _ = writeln!(file, "{line}");
            }
        }
        Err(e) => tracing::warn!("無法寫入建議檔 {path}: {e}"),
    }
}

/// insert 一則建議到 `suggestions` 表。走 runtime query API(非 `query!` 巨集),故 build/test
/// 不需 live DB。`id` 由 BIGSERIAL 自動配;`at` 是 Unix 毫秒(對齊 `Suggestion.at: u64`)。
async fn insert_suggestion(pool: &PgPool, s: &Suggestion) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO suggestions (from_name, text, at) VALUES ($1, $2, $3)")
        .bind(&s.from)
        .bind(&s.text)
        .bind(s.at as i64)
        .execute(pool)
        .await?;
    Ok(())
}

/// 啟動時把 `suggestions` 表全部載回(依 `at`、再 `id` 排序保住送出順序,讓 `list` 的 `rev()`
/// 仍是最新在前)。顯示用欄位(`from` / `text`)一律過 sanitizer,與 JSONL 載入路徑
/// (`parse_and_sanitize`)同一道防線——磁碟被竄改也不會把控制字元帶進 GET 輸出。清乾淨後
/// 變空的列丟掉(比照寫入路徑「空建議不存」)。載入失敗(DB 連線剛斷等)回空清單,讓伺服器
/// 仍能起來、之後再寫回。
async fn load_from_db(pool: &PgPool) -> Vec<Suggestion> {
    let rows = match sqlx::query("SELECT from_name, text, at FROM suggestions ORDER BY at ASC, id ASC")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("從 Postgres 載入 suggestions 失敗(先以空清單起來):{e}");
            return Vec::new();
        }
    };
    rows.into_iter()
        .filter_map(|r| {
            let at: i64 = r.get("at");
            sanitize(
                &r.get::<String, _>("from_name"),
                &r.get::<String, _>("text"),
                at as u64,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_keeps_normal_input() {
        let s = sanitize("  小明  ", "  希望有貓咪  ", 42).unwrap();
        assert_eq!(s.from, "小明");
        assert_eq!(s.text, "希望有貓咪");
        assert_eq!(s.at, 42);
    }

    #[test]
    fn empty_from_falls_back_to_anonymous() {
        assert_eq!(sanitize("", "有內容", 0).unwrap().from, anonymous());
        assert_eq!(sanitize("   ", "有內容", 0).unwrap().from, anonymous());
    }

    #[test]
    fn caps_from_by_chars() {
        let long = "字".repeat(MAX_FROM_CHARS + 10);
        let s = sanitize(&long, "x", 0).unwrap();
        assert_eq!(s.from.chars().count(), MAX_FROM_CHARS);
    }

    #[test]
    fn caps_text_by_chars_not_bytes() {
        // 全中文(每字多位元組):應以字元數截斷,不是位元組數。
        let long = "乙".repeat(MAX_TEXT_CHARS + 50);
        let s = sanitize("我", &long, 0).unwrap();
        assert_eq!(s.text.chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn keeps_text_at_exactly_the_cap() {
        let exact = "a".repeat(MAX_TEXT_CHARS);
        let s = sanitize("我", &exact, 0).unwrap();
        assert_eq!(s.text.chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn strips_control_chars_from_from() {
        // 署名是單行身分欄位：換行 / 歸位 / NUL / ESC / tab 全濾掉，
        // 不讓壞客戶端把多行或終端機轉義塞進回給公開 GET 的署名。
        let s = sanitize("小\n明\r\0\u{1b}\t", "有內容", 0).unwrap();
        assert_eq!(s.from, "小明");
    }

    #[test]
    fn keeps_newlines_in_text_but_strips_other_controls() {
        // 內容是多行回饋（textarea）：保留換行讓玩家分段，但濾掉換行以外的控制字元
        // （NUL / 歸位 / ESC——維護者多在終端機讀檔，ESC 可注入 ANSI 轉義偽造顯示）。
        let s = sanitize("我", "第一段\0\r\n第二段", 0).unwrap();
        assert_eq!(s.text, "第一段\n第二段");
    }

    #[test]
    fn stripping_esc_neutralizes_ansi_injection() {
        // 注入 ANSI 轉義靠的是 ESC（0x1B）控制位元組；濾掉它，殘留的 `[31m` 只是
        // 無害的字面文字，不再能在維護者的終端機著色 / 偽造顯示。
        let s = sanitize("我", "正常\u{1b}[31m紅字", 0).unwrap();
        assert!(!s.text.contains('\u{1b}'));
        assert_eq!(s.text, "正常[31m紅字");
    }

    #[test]
    fn control_or_whitespace_only_text_is_rejected() {
        // 清乾淨後變空的內容不該被存：全控制字元（`\0`/`ESC` 等非空白，會通過
        // endpoint 對 raw `text.trim()` 的判空）以及全空白，sanitize 一律回 None，
        // 呼叫端據此回 400、不寫進 JSONL 留下空建議垃圾紀錄。
        assert!(sanitize("我", "\0\r\u{1b}\t", 0).is_none());
        assert!(sanitize("我", "\0\u{1b}", 0).is_none());
        assert!(sanitize("我", "   ", 0).is_none());
        assert!(sanitize("我", "", 0).is_none());
        // 還有可見字元的內容仍照常存下。
        assert!(sanitize("我", "\0真有建議\u{1b}", 0).is_some());
    }

    #[test]
    fn load_path_strips_control_chars_from_legacy_lines() {
        // legacy lines may carry control bytes written before write-path hardening;
        // the load path must re-filter so GET output stays clean.
        let jsonl = "{\"from\":\"A\\u001bB\",\"text\":\"normal\\u001b[31mred\\u0000 \",\"at\":7}";
        let out = parse_and_sanitize(jsonl);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].from, "AB");
        assert_eq!(out[0].text, "normal[31mred");
        assert!(!out[0].text.contains('\u{1b}'));
        assert!(!out[0].text.contains('\0'));
        assert_eq!(out[0].at, 7);
    }

    #[test]
    fn load_path_drops_empty_after_sanitize() {
        // 內容清乾淨後變空的行（全控制字元 / 全空白）不該載進來當空建議垃圾。
        let jsonl = "{\"from\":\"我\",\"text\":\"\\u0000\\u001b\",\"at\":1}\n\
                     {\"from\":\"我\",\"text\":\"   \",\"at\":2}\n\
                     {\"from\":\"我\",\"text\":\"真有建議\",\"at\":3}";
        let out = parse_and_sanitize(jsonl);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "真有建議");
        assert_eq!(out[0].at, 3);
    }

    #[test]
    fn load_path_skips_malformed_json_but_keeps_valid() {
        // 損毀 / 非 JSON 的行跳過，合法的照常載入（沿用原本 filter_map 的容錯）。
        let jsonl = "這不是 json\n\
                     {\"from\":\"小明\",\"text\":\"希望有貓咪\",\"at\":42}\n\
                     {壞掉的";
        let out = parse_and_sanitize(jsonl);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].from, "小明");
        assert_eq!(out[0].text, "希望有貓咪");
    }

    #[test]
    fn load_path_preserves_clean_lines_unchanged() {
        // 已經乾淨的正常內容載入後一字不差（過 sanitizer 不該動到合法內容）。
        let jsonl = r#"{"from":"拓荒者","text":"多一點花","at":100}"#;
        let out = parse_and_sanitize(jsonl);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].from, "拓荒者");
        assert_eq!(out[0].text, "多一點花");
        assert_eq!(out[0].at, 100);
    }

    // ===== store 行為（純記憶體 backend，不碰磁碟 / DB）=====

    #[tokio::test]
    async fn add_stores_and_lists_newest_first() {
        // add 成功後該則建議進記憶體;list 最新在前(後加的先列)。純記憶體 backend 不碰磁碟/DB。
        let store = SuggestionStore::in_memory();
        let a = store
            .add(NewSuggestion {
                from: "甲".into(),
                text: "第一則".into(),
            })
            .await
            .expect("有內容應存下");
        let b = store
            .add(NewSuggestion {
                from: "乙".into(),
                text: "第二則".into(),
            })
            .await
            .expect("有內容應存下");
        // 兩則都過 sanitize 存下。
        assert_eq!(a.text, "第一則");
        assert_eq!(b.text, "第二則");
        let listed = store.list();
        assert_eq!(listed.len(), 2);
        // list 最新在前：後加的「第二則」排第一。
        assert_eq!(listed[0].text, "第二則");
        assert_eq!(listed[1].text, "第一則");
    }

    #[tokio::test]
    async fn add_rejects_empty_after_sanitize_and_stores_nothing() {
        // 清乾淨後變空的內容(全控制字元 / 全空白)回 None、不進記憶體,不留空建議垃圾。
        let store = SuggestionStore::in_memory();
        assert!(store
            .add(NewSuggestion {
                from: "我".into(),
                text: "\0\u{1b}\t".into(),
            })
            .await
            .is_none());
        assert!(store
            .add(NewSuggestion {
                from: "我".into(),
                text: "   ".into(),
            })
            .await
            .is_none());
        assert!(store.list().is_empty(), "被拒的建議不該進記憶體");
    }

    #[test]
    fn control_chars_do_not_count_toward_cap() {
        // 控制字元先濾掉、不佔截斷額度：夾在合法字元間的控制字元被移除後，
        // 仍保留滿額的可見字元。
        let mut raw = String::new();
        for _ in 0..MAX_FROM_CHARS {
            raw.push('字');
            raw.push('\0');
        }
        let s = sanitize(&raw, "x", 0).unwrap();
        assert_eq!(s.from.chars().count(), MAX_FROM_CHARS);
        assert!(!s.from.contains('\0'));
    }
}
