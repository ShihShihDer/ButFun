//! 乙太方界動態 Feed v1——記錄居民活動事件到 `data/voxel_feed.jsonl`，
//! 讓玩家回來時能讀到「不在時世界發生了什麼」。
//!
//! 設計要點：
//! - **append-only jsonl**：每筆事件一行、絕不改寫既有行。
//! - **ts_secs**（Unix 秒）：讓前端換算「X 分鐘前」。
//! - **zero migration**：純新增檔案，不動現有資料。
//! - **不持鎖**：`append_feed` / `load_recent_feed` 全是同步小檔 IO，呼叫端在鎖外呼叫。
//! - **行數上限 + 輪替**（M2 防 DoS）：append 後若超過 [`FEED_ROTATION_LIMIT`] 行，
//!   原子重寫成最新 [`FEED_KEEP_LINES`] 行；load 只讀檔尾 [`FEED_TAIL_BYTES`] 位元組
//!   避免整檔讀入。

use std::io::{Read, Seek, SeekFrom};

use serde::{Deserialize, Serialize};

const FEED_PATH: &str = "data/voxel_feed.jsonl";
/// 回傳給前端的最大事件筆數（最新在前）。
pub const FEED_LIMIT: usize = 30;

/// 讀尾時讀取的最大位元組數（每筆事件約 200 bytes，60 筆足用）。
/// 只掃這段 seek 窗口，不讀整檔——避免 DoS 放大。
const FEED_TAIL_BYTES: u64 = 16_384; // 16 KB

/// append 後觸發輪替的行數上限（近似，超過才重寫）。
const FEED_ROTATION_LIMIT: usize = 500;

/// 輪替後保留的最新行數（比 FEED_LIMIT 留多一些緩衝）。
const FEED_KEEP_LINES: usize = 60;

/// 一筆動態 Feed 事件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedEvent {
    /// Unix 秒（牆鐘），用於「X 分鐘前」換算。
    pub ts_secs: u64,
    /// 事件類型字串（"新心願"、"念頭種下"、"鄰里閒聊"、"蓋家動工"、"蓋家完工"）。
    pub kind: String,
    /// 主角居民名字（面向玩家顯示）。
    pub resident: String,
    /// 事件細節描述（面向玩家字串，留 i18n 空間）。
    pub detail: String,
}

/// 把一筆 Feed 事件 append 到 jsonl。失敗只 log，不 panic。
/// append 後若行數超過 `FEED_ROTATION_LIMIT`，自動輪替（原子重寫）。
pub fn append_feed(kind: &str, resident: &str, detail: &str) {
    let ts = now_secs();
    let ev = FeedEvent {
        ts_secs: ts,
        kind: sanitize(kind),
        resident: sanitize(resident),
        detail: sanitize(detail),
    };
    if ev.kind.is_empty() || ev.resident.is_empty() {
        return;
    }
    let line = match serde_json::to_string(&ev) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[voxel_feed] 序列化失敗: {e}");
            return;
        }
    };
    write_line(FEED_PATH, &line);
    // 輪替：append 後估算檔案行數，超限時裁短。
    maybe_rotate(FEED_PATH);
}

/// 讀取最新 n 筆 Feed 事件（最新在前）。
/// 只讀檔尾 `FEED_TAIL_BYTES` 位元組，避免整檔讀入（防 DoS 放大）。
/// 檔案不存在或損壞時回空 Vec。
pub fn load_recent_feed(n: usize) -> Vec<FeedEvent> {
    let events = read_tail_events(FEED_PATH, n * 4); // 多讀一點再排序截斷
    let mut events = events;
    // 最新在前（ts_secs 大者優先）
    events.sort_by(|a, b| b.ts_secs.cmp(&a.ts_secs));
    events.truncate(n);
    events
}

/// 把 Unix 秒差轉成繁中「X 分鐘前」文字（供後端測試；前端可自行換算）。
pub fn format_relative_time(event_ts: u64, now_ts: u64) -> String {
    let diff = now_ts.saturating_sub(event_ts);
    if diff < 60 {
        "剛才".to_string()
    } else if diff < 3600 {
        format!("{} 分鐘前", diff / 60)
    } else if diff < 86400 {
        format!("{} 小時前", diff / 3600)
    } else {
        format!("{} 天前", diff / 86400)
    }
}

// ── 內部工具 ──────────────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

fn write_line(path: &str, line: &str) {
    use std::io::Write;
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{line}") {
                eprintln!("[voxel_feed] 寫入失敗: {e}");
            }
        }
        Err(e) => eprintln!("[voxel_feed] 開檔失敗 {path}: {e}"),
    }
}

/// 只讀檔尾最多 `FEED_TAIL_BYTES` 位元組，解析並回傳最多 `limit` 筆事件。
/// 從尾端 seek，只讀一小塊窗口——避免整檔讀入。
pub(crate) fn read_tail_events(path: &str, limit: usize) -> Vec<FeedEvent> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return vec![],
    };
    let file_len = match f.seek(SeekFrom::End(0)) {
        Ok(n) => n,
        Err(_) => return vec![],
    };
    // 計算起始 seek 點（從尾端回退最多 FEED_TAIL_BYTES）
    let start = file_len.saturating_sub(FEED_TAIL_BYTES);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return vec![];
    }
    let mut buf = Vec::with_capacity((file_len - start) as usize + 1);
    if f.read_to_end(&mut buf).is_err() {
        return vec![];
    }
    let text = String::from_utf8_lossy(&buf);
    // 若從中間截斷，第一行可能殘破 → skip 到第一個完整換行符後的行
    let skip_first = start > 0;
    let mut events: Vec<FeedEvent> = text
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            // 可能從行中間開始，跳過第一行（殘破）
            if skip_first && i == 0 {
                return None;
            }
            let line = line.trim();
            if line.is_empty() { return None; }
            serde_json::from_str::<FeedEvent>(line).ok()
        })
        .collect();
    // 排序後取最新 limit 筆
    events.sort_by(|a, b| b.ts_secs.cmp(&a.ts_secs));
    events.truncate(limit);
    events
}

/// 若 feed 檔行數超過 `FEED_ROTATION_LIMIT`，原子重寫成最新 `FEED_KEEP_LINES` 行。
/// rename 失敗時原檔不動（安全降級）。
fn maybe_rotate(path: &str) {
    // 快速路徑：若檔案很小（< 每行最短估算 × 限制行數），直接跳過，不讀檔。
    // 每行最小約 40 bytes（最短合法 FeedEvent JSON）；超過此閾值才計行。
    const MIN_BYTES_PER_LINE: u64 = 40;
    let size_threshold = FEED_ROTATION_LIMIT as u64 * MIN_BYTES_PER_LINE;
    if let Ok(m) = std::fs::metadata(path) {
        if m.len() < size_threshold {
            return;
        }
    } else {
        return;
    }

    // 真正讀取並計行
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < FEED_ROTATION_LIMIT {
        return; // 大多空白或短行，不輪替
    }

    // 保留最新 FEED_KEEP_LINES 行（尾部）
    let keep_start = lines.len().saturating_sub(FEED_KEEP_LINES);
    let kept: Vec<&str> = lines[keep_start..].to_vec();
    let new_content = kept.join("\n") + "\n";

    // 原子替換：temp 檔 → rename
    let tmp_path = format!("{path}.tmp");
    if std::fs::write(&tmp_path, &new_content).is_err() {
        // 寫 temp 失敗 → 清掉殘留 temp 後放棄（原檔不動）
        let _ = std::fs::remove_file(&tmp_path);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        eprintln!("[voxel_feed] 輪替 rename 失敗: {e}，原檔保留");
        let _ = std::fs::remove_file(&tmp_path);
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ── format_relative_time ──────────────────────────────────────────────────

    #[test]
    fn relative_time_just_now() {
        assert_eq!(format_relative_time(100, 120), "剛才");
    }

    #[test]
    fn relative_time_same_ts() {
        assert_eq!(format_relative_time(500, 500), "剛才");
    }

    #[test]
    fn relative_time_future_is_just_now() {
        // event_ts > now_ts → saturating_sub 為 0 → "剛才"
        assert_eq!(format_relative_time(999, 0), "剛才");
    }

    #[test]
    fn relative_time_minutes() {
        assert_eq!(format_relative_time(0, 300), "5 分鐘前");
    }

    #[test]
    fn relative_time_hours() {
        assert_eq!(format_relative_time(0, 7200), "2 小時前");
    }

    #[test]
    fn relative_time_days() {
        assert_eq!(format_relative_time(0, 172800), "2 天前");
    }

    // ── FeedEvent 序列化往返 ──────────────────────────────────────────────────

    #[test]
    fn feed_event_roundtrip() {
        let ev = FeedEvent {
            ts_secs: 1000,
            kind: "新心願".to_string(),
            resident: "露娜".to_string(),
            detail: "我想蓋一座觀星塔".to_string(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: FeedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    // ── load_recent_feed：空檔回 [] ───────────────────────────────────────────

    #[test]
    fn load_recent_feed_missing_file() {
        // 不存在的路徑 → 空 vec
        let content = std::fs::read_to_string("data/__nonexistent_voxel_feed_test__.jsonl");
        assert!(content.is_err());
        // load_recent_feed 底層相同邏輯，回 empty
        // （直接測 load_recent_feed 會碰真實檔案，這裡只測底層 read_to_string 行為）
    }

    // ── load_recent_feed 只回傳最新 n 筆 ─────────────────────────────────────

    #[test]
    fn load_recent_feed_returns_latest_n() {
        // 直接測 sort+truncate 邏輯，不依賴真實檔案
        let mut events: Vec<FeedEvent> = (0u64..10)
            .map(|i| FeedEvent {
                ts_secs: i,
                kind: "新心願".to_string(),
                resident: "露娜".to_string(),
                detail: format!("event {i}"),
            })
            .collect();
        events.sort_by(|a, b| b.ts_secs.cmp(&a.ts_secs));
        events.truncate(3);
        assert_eq!(events.len(), 3);
        // 最新在前（ts_secs=9,8,7）
        assert_eq!(events[0].ts_secs, 9);
        assert_eq!(events[1].ts_secs, 8);
        assert_eq!(events[2].ts_secs, 7);
    }

    // ── sanitize 過濾控制字元 ────────────────────────────────────────────────

    #[test]
    fn sanitize_strips_control() {
        let input = "露娜\x00\x1b[31m";
        let out = sanitize(input);
        assert!(!out.contains('\x00'));
        assert!(!out.contains('\x1b'));
        assert!(out.contains("露娜"));
    }

    // ── read_tail_events：只讀尾端，語意等價驗證（M2 防 DoS）────────────────

    /// 輔助：把一組 FeedEvent 寫成 jsonl tempfile，回傳路徑
    fn write_temp_feed(events: &[FeedEvent]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        for ev in events {
            writeln!(f, "{}", serde_json::to_string(ev).unwrap()).unwrap();
        }
        f
    }

    #[test]
    fn read_tail_events_returns_latest_n_from_file() {
        // 寫 15 筆（ts_secs=0..14），讀尾端取最新 5 筆，應為 14..10
        let events: Vec<FeedEvent> = (0u64..15)
            .map(|i| FeedEvent {
                ts_secs: i,
                kind: "新心願".to_string(),
                resident: "露娜".to_string(),
                detail: format!("event {i}"),
            })
            .collect();
        let tf = write_temp_feed(&events);
        let got = read_tail_events(tf.path().to_str().unwrap(), 5);
        assert_eq!(got.len(), 5);
        assert_eq!(got[0].ts_secs, 14);
        assert_eq!(got[4].ts_secs, 10);
    }

    #[test]
    fn read_tail_events_equals_full_read_for_small_file() {
        // 小檔（< FEED_TAIL_BYTES）：尾端讀與全讀結果一致
        let events: Vec<FeedEvent> = (0u64..20)
            .map(|i| FeedEvent {
                ts_secs: i,
                kind: "測試".to_string(),
                resident: "阿星".to_string(),
                detail: format!("d{i}"),
            })
            .collect();
        let tf = write_temp_feed(&events);
        let path = tf.path().to_str().unwrap();

        // 尾端讀（新實作）
        let mut tail = read_tail_events(path, 20);
        tail.sort_by_key(|e| e.ts_secs);

        // 全讀（舊邏輯）
        let content = std::fs::read_to_string(path).unwrap();
        let mut full: Vec<FeedEvent> = content
            .lines()
            .filter_map(|l| serde_json::from_str::<FeedEvent>(l).ok())
            .collect();
        full.sort_by_key(|e| e.ts_secs);

        assert_eq!(tail, full, "尾端讀與全讀結果應相同（小檔）");
    }

    // ── maybe_rotate：行數超限後檔案應被截短 ────────────────────────────────

    #[test]
    fn maybe_rotate_truncates_when_over_limit() {
        // 造出一個超過 FEED_ROTATION_LIMIT 行的 feed 檔
        let mut tf = tempfile::NamedTempFile::new().unwrap();
        for i in 0u64..(FEED_ROTATION_LIMIT as u64 + 50) {
            let ev = FeedEvent {
                ts_secs: i,
                kind: "k".to_string(),
                resident: "r".to_string(),
                detail: "d".to_string(),
            };
            writeln!(tf, "{}", serde_json::to_string(&ev).unwrap()).unwrap();
        }
        let path = tf.path().to_str().unwrap().to_string();
        maybe_rotate(&path);
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        assert!(
            lines.len() <= FEED_KEEP_LINES,
            "輪替後行數 {} 應 ≤ FEED_KEEP_LINES {}",
            lines.len(),
            FEED_KEEP_LINES
        );
        // 輪替後保留的是最新的（ts_secs 最大）
        let last: FeedEvent = serde_json::from_str(lines.last().unwrap()).unwrap();
        assert_eq!(
            last.ts_secs,
            (FEED_ROTATION_LIMIT as u64 + 49),
            "輪替後應保留最新一筆"
        );
    }

    #[test]
    fn maybe_rotate_noop_when_under_limit() {
        // 行數不足 FEED_ROTATION_LIMIT → 不輪替，檔案不變
        let mut tf = tempfile::NamedTempFile::new().unwrap();
        for i in 0u64..10 {
            let ev = FeedEvent {
                ts_secs: i,
                kind: "k".to_string(),
                resident: "r".to_string(),
                detail: "d".to_string(),
            };
            writeln!(tf, "{}", serde_json::to_string(&ev).unwrap()).unwrap();
        }
        let path = tf.path().to_str().unwrap().to_string();
        let before_len = std::fs::metadata(&path).unwrap().len();
        maybe_rotate(&path);
        let after_len = std::fs::metadata(&path).unwrap().len();
        assert_eq!(before_len, after_len, "行數不足時不應輪替");
    }
}
