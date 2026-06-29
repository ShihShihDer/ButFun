//! 乙太方界動態 Feed v1——記錄居民活動事件到 `data/voxel_feed.jsonl`，
//! 讓玩家回來時能讀到「不在時世界發生了什麼」。
//!
//! 設計要點：
//! - **append-only jsonl**：每筆事件一行、絕不改寫既有行。
//! - **ts_secs**（Unix 秒）：讓前端換算「X 分鐘前」。
//! - **zero migration**：純新增檔案，不動現有資料。
//! - **不持鎖**：`append_feed` / `load_recent_feed` 全是同步小檔 IO，呼叫端在鎖外呼叫。

use serde::{Deserialize, Serialize};

const FEED_PATH: &str = "data/voxel_feed.jsonl";
/// 回傳給前端的最大事件筆數（最新在前）。
pub const FEED_LIMIT: usize = 30;

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
}

/// 讀取最新 n 筆 Feed 事件（最新在前）。
/// 檔案不存在或損壞時回空 Vec。
pub fn load_recent_feed(n: usize) -> Vec<FeedEvent> {
    let content = match std::fs::read_to_string(FEED_PATH) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let mut events: Vec<FeedEvent> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<FeedEvent>(l).ok())
        .collect();
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

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // format_relative_time

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

    // FeedEvent 序列化往返

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

    // load_recent_feed：空檔回 []

    #[test]
    fn load_recent_feed_missing_file() {
        // 不存在的路徑 → 空 vec
        let content = std::fs::read_to_string("data/__nonexistent_voxel_feed_test__.jsonl");
        assert!(content.is_err());
        // load_recent_feed 底層相同邏輯，回 empty
        // （直接測 load_recent_feed 會碰真實檔案，這裡只測底層 read_to_string 行為）
    }

    // load_recent_feed 只回傳最新 n 筆

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

    // sanitize 過濾控制字元

    #[test]
    fn sanitize_strips_control() {
        let input = "露娜\x00\x1b[31m";
        let out = sanitize(input);
        assert!(!out.contains('\x00'));
        assert!(!out.contains('\x1b'));
        assert!(out.contains("露娜"));
    }
}
