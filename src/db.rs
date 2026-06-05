//! Phase 0-E：Postgres 持久化的連線地基。
//!
//! 這層只負責「**有沒有 DB**、連得上就把 migration 套好、給出一個連線池」。
//! 沒設 `DATABASE_URL`（本機 `cargo run`、`cargo test`）時回 `None`，呼叫端退回現有
//! 記憶體模式——延續 BACKLOG 0-E「無 `DATABASE_URL` 時退回記憶體模式以利測試」。
//!
//! 刻意做成獨立、純加法的地基：本切片不碰任何玩家資料寫入路徑（`PositionStore`
//! 等 store 的接線留待後續一輪一個 store incremental 接，避免單一巨大 PR）。
//! 連線後立刻套用 `migrations/`（`sqlx::migrate!` 在編譯期把 migration 檔嵌進來，
//! 不需 live DB 才能 build；查詢一律走 runtime API、不用 `query!` 巨集，故測試免連線）。

use sqlx::postgres::{PgPool, PgPoolOptions};

/// 把原始 `DATABASE_URL` 值正規化成「要不要連 DB」的決定。純函式，便於測試。
///
/// 去頭尾空白後為空（沒設、或設成空字串／全空白）一律視為「不連 DB」回 `None`，
/// 避免拿一個空字串去連線而炸在啟動。非空才回 `Some(trimmed)`。
pub fn normalize_database_url(raw: Option<String>) -> Option<String> {
    let url = raw?;
    let trimmed = url.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 讀環境變數 `DATABASE_URL` 並正規化（見 `normalize_database_url`）。
fn database_url_from_env() -> Option<String> {
    normalize_database_url(std::env::var("DATABASE_URL").ok())
}

/// 把 libpq 風格的 Unix socket `DATABASE_URL` 轉成 sqlx 能解析的形式。純函式，便於測試。
///
/// 維護者 `.env` 用的是 libpq 慣例：`postgresql://user@/db?host=/var/run/postgresql`
/// （authority 的 host 留空、socket 目錄放在 `host=` 查詢參數）。psql / libpq 吃這個，
/// 但 sqlx 的 URL parser 會因 host 為空而報「empty host」。sqlx 認得的等價寫法是把 socket
/// 目錄 percent-encode 後放回 authority 的 host 位置：`postgresql://user@%2Fvar%2Frun%2Fpostgresql/db`。
///
/// 只在「authority host 為空、且 `host=` 指向一個絕對路徑 socket 目錄」時做這個搬移，
/// 其餘 URL（一般 TCP host、或已是 percent-encode socket 形式）一律原樣放行。
/// 這讓維護者照 libpq 慣例寫的既有 `.env` 不必改、systemd 與本機都直接能用。
fn to_sqlx_url(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };

    // 從 query 抽出 `host=/絕對路徑`（socket 目錄），其餘參數保留。
    let mut socket_dir: Option<&str> = None;
    let mut remaining: Vec<&str> = Vec::new();
    for pair in query.split('&') {
        match pair.strip_prefix("host=") {
            Some(v) if v.starts_with('/') && socket_dir.is_none() => socket_dir = Some(v),
            _ => remaining.push(pair),
        }
    }
    let Some(dir) = socket_dir else {
        return url.to_string();
    };

    // 定位 authority（`scheme://` 與下一個 `/` 之間），確認其 host 段確實為空才搬移。
    let Some(after_scheme) = base.find("://").map(|i| i + 3) else {
        return url.to_string();
    };
    let Some(auth_end) = base[after_scheme..].find('/').map(|i| after_scheme + i) else {
        return url.to_string();
    };
    let authority = &base[after_scheme..auth_end]; // "" 或 "user@" 或 "user:pw@host"
    let host_part = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if !host_part.is_empty() {
        return url.to_string(); // 已有實體 host（一般 TCP），不動
    }

    // host 為空 → 把 socket 目錄 percent-encode 後插進 host 位置（authority 為 "" 或以 "@" 結尾）。
    let encoded = dir.replace('/', "%2F");
    let rest_query = if remaining.is_empty() {
        String::new()
    } else {
        format!("?{}", remaining.join("&"))
    };
    format!(
        "{}{}{}{}{}",
        &base[..after_scheme],
        authority,
        encoded,
        &base[auth_end..],
        rest_query
    )
}

/// 嘗試建立 Postgres 連線池並套用 migration。
///
/// 回 `Some(pool)` 表示 DB 已就緒、migration 已套好，呼叫端可把 store 接上去；
/// 回 `None` 表示沒設 `DATABASE_URL`（退回記憶體模式）。連得到但 migration 失敗
/// 視為設定錯誤（schema 對不上不該默默跑記憶體），直接 `Err` 讓啟動方決定。
pub async fn connect() -> Result<Option<PgPool>, sqlx::Error> {
    let Some(url) = database_url_from_env() else {
        return Ok(None);
    };
    // 維護者 .env 走 libpq 慣例的 Unix socket 寫法，轉成 sqlx 能解析的等價形式。
    let url = to_sqlx_url(&url);

    // 連線池上限刻意保守：本機自架、玩家量小，5 條連線綽綽有餘。
    let pool = PgPoolOptions::new().max_connections(5).connect(&url).await?;

    // 套用 migrations/ 下的所有 migration（已套過的會被 sqlx 跳過，冪等）。
    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(Some(pool))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_url_means_no_db() {
        assert_eq!(normalize_database_url(None), None);
    }

    #[test]
    fn empty_or_whitespace_url_means_no_db() {
        // 空字串／全空白的 DATABASE_URL 視為沒設，不該拿去連線。
        assert_eq!(normalize_database_url(Some(String::new())), None);
        assert_eq!(normalize_database_url(Some("   ".to_string())), None);
        assert_eq!(normalize_database_url(Some("\t\n".to_string())), None);
    }

    #[test]
    fn real_url_is_trimmed_and_kept() {
        assert_eq!(
            normalize_database_url(Some("  postgresql:///butfun  ".to_string())),
            Some("postgresql:///butfun".to_string())
        );
    }

    #[test]
    fn libpq_socket_url_becomes_sqlx_form() {
        // 維護者 .env 的 libpq Unix socket 寫法 → sqlx 認得的 percent-encode host 形式。
        assert_eq!(
            to_sqlx_url("postgresql://shihshih@/butfun?host=/var/run/postgresql"),
            "postgresql://shihshih@%2Fvar%2Frun%2Fpostgresql/butfun"
        );
    }

    #[test]
    fn libpq_socket_url_keeps_other_query_params() {
        // 只搬 host=,其餘查詢參數原樣保留。
        assert_eq!(
            to_sqlx_url("postgresql://u@/db?host=/tmp&sslmode=disable"),
            "postgresql://u@%2Ftmp/db?sslmode=disable"
        );
    }

    #[test]
    fn tcp_url_is_left_untouched() {
        // 一般 TCP host 不該被動到。
        let tcp = "postgresql://u:pw@localhost:5432/db?sslmode=require";
        assert_eq!(to_sqlx_url(tcp), tcp);
    }

    #[test]
    fn url_without_query_is_left_untouched() {
        let u = "postgresql://u@localhost/db";
        assert_eq!(to_sqlx_url(u), u);
    }

    #[test]
    fn already_encoded_socket_url_is_left_untouched() {
        // 已是 sqlx percent-encode socket 形式（host 非空）就別重複處理。
        let u = "postgresql://u@%2Fvar%2Frun%2Fpostgresql/db";
        assert_eq!(to_sqlx_url(u), u);
    }
}
