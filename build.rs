//! 版本戳記建置腳本：把「編譯當下的 git commit short SHA」與「build 時間（UTC）」
//! 透過 `cargo:rustc-env=` 注入成編譯期常數，烤進 binary（見 src/version.rs 讀取）。
//!
//! 目的：堵死「舊 binary 靜默上線、沒人發現」——有了烤進去的 SHA，`/version` 端點與
//! scripts/deploy.sh 自驗就能一眼確認「跑著的 binary == 哪個 commit」。
//!
//! 鐵律：抓不到 git（淺 clone／無 .git／沒裝 git）一律優雅退回 "unknown"，**絕不**讓 build 失敗。

use std::process::Command;

fn main() {
    // git short SHA；任何失敗（沒裝 git／非 repo／指令錯）都退回 "unknown"。
    let sha = git_short_sha().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUTFUN_GIT_SHA={sha}");

    // build 時間（UTC, RFC3339 風格 `YYYY-MM-DDTHH:MM:SSZ`）。
    // 允許用 SOURCE_DATE_EPOCH 覆寫，利於可重現建置；抓不到時間就退回 "unknown"。
    let built_at = build_time_utc().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUTFUN_BUILD_TIME={built_at}");

    // commit 一前進就要重跑此腳本（否則增量編譯下 binary 內的 SHA 會停在舊值）。
    // 監看 .git/HEAD 與當前分支 ref 檔；任一不存在就略過（不報錯）。
    for p in git_rerun_paths() {
        println!("cargo:rerun-if-changed={p}");
    }
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
}

/// 跑 `git rev-parse --short HEAD`，成功且非空才回 Some；其餘一律 None（呼叫端退回 "unknown"）。
fn git_short_sha() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// 取 build 時間（UTC）。優先讀 SOURCE_DATE_EPOCH（可重現建置）；否則用系統現在時間。
/// 任何環節失敗回 None（呼叫端退回 "unknown"），不讓 build 失敗。
fn build_time_utc() -> Option<String> {
    let secs: u64 = match std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.trim().parse().ok())
    {
        Some(e) => e,
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs(),
    };
    Some(format_utc(secs))
}

/// 把 Unix epoch 秒數格式化成 `YYYY-MM-DDTHH:MM:SSZ`（UTC）。
/// 用 Howard Hinnant 的 civil-from-days 演算法，零相依、不需 chrono。
fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // days = 自 1970-01-01 起的天數 → 西曆年月日（Hinnant civil_from_days）。
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    format!("{year:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// 要監看以觸發重編的檔案路徑（commit 變了就重跑 build.rs）。
/// 讀 .git/HEAD 找出當前 ref，連同 HEAD 一起回傳；非 repo 就回空（不報錯）。
fn git_rerun_paths() -> Vec<String> {
    let mut paths = Vec::new();
    let head = std::path::Path::new(".git/HEAD");
    if head.exists() {
        paths.push(".git/HEAD".to_string());
        // HEAD 內容形如 "ref: refs/heads/main" → 也監看那個 ref 檔（commit 一變它就變）。
        if let Ok(content) = std::fs::read_to_string(head) {
            if let Some(r) = content.strip_prefix("ref:") {
                let r = r.trim();
                if !r.is_empty() {
                    paths.push(format!(".git/{r}"));
                }
            }
        }
    }
    paths
}
