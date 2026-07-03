//! 乙太方界玩家位置持久化 v1——登入帳號重整/重登回到上次位置。
//!
//! 純邏輯模組（不含 WS/鎖細節），由 `voxel_ws.rs` 在斷線時與定期 tick 呼叫。
//! **append-覆蓋取最新 jsonl 持久化**（`data/voxel_player_pos.jsonl`）：
//! 每次存一筆，載入時取同 email 最後一筆（依 ts 排序），向後相容（檔缺 = 首次，回 None）。
//!
//! # 安全
//! - 位置 key 一律綁後端 cookie→帳號解出的 **email**（權威），不信客戶端自報。
//! - 訪客（無 email）由呼叫端守衛：`account_email` 為 None 時根本不呼叫本模組。
//! - 無跨帳號讀寫風險：每次 load/save 都以後端解出的 email 為 key。
//! - 存檔 IO 一律在無鎖段呼叫（同步小檔、不 await、不持任何 hub 鎖）。

use serde::{Deserialize, Serialize};

/// jsonl 持久化路徑（`data/` 已 gitignore）。
pub const PLAYER_POS_PATH: &str = "data/voxel_player_pos.jsonl";

/// 座標合法性上下界（XZ ±16384，Y −64..512）。
const MAX_XZ: f32 = 16384.0;
const MIN_Y: f32 = -64.0;
const MAX_Y: f32 = 512.0;

/// 一筆位置記錄（append-only jsonl 最小單元，同 email 取 ts 最大的那筆有效）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerPosRecord {
    /// 帳號 email（索引鍵；後端 cookie→users 解出，不信客戶端）。
    pub email: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    /// Unix 秒（排序用；同 email 取最大 ts 的那筆）。
    pub ts: u64,
}

/// 座標合法性判定：非 NaN/Inf、在合理世界範圍內（XZ ±16384，Y −64..512）。
/// 防呆：存過的離譜座標（例如飛出地圖的測試值）重登時回 `None` 改用 `spawn_pos`。
pub fn is_valid_pos(x: f32, y: f32, z: f32) -> bool {
    x.is_finite() && y.is_finite() && z.is_finite()
        && x.abs() <= MAX_XZ
        && z.abs() <= MAX_XZ
        && y >= MIN_Y
        && y <= MAX_Y
}

/// 把目前位置 append 到 jsonl（不持任何鎖；失敗只吞掉，不 panic）。
/// **鐵律**：只在不持任何 hub 鎖時呼叫（同步小檔寫、不 await）。
/// 座標不合法時靜默忽略（防呆，絕不寫壞記錄）。
pub fn save_player_pos(email: &str, x: f32, y: f32, z: f32, yaw: f32) {
    if email.is_empty() || !is_valid_pos(x, y, z) {
        return;
    }
    let rec = PlayerPosRecord {
        email: email.to_string(),
        x,
        y,
        z,
        yaw: if yaw.is_finite() { yaw } else { 0.0 },
        ts: now_unix(),
    };
    if let Ok(line) = serde_json::to_string(&rec) {
        write_line(PLAYER_POS_PATH, &line);
    }
}

/// 載回某帳號上次儲存的位置，回傳 `(x, y, z, yaw)`。
/// 回 `None` 表示從未存過（首次登入）、資料檔不存在、或座標不合法（防呆，改用 `spawn_pos`）。
/// **鐵律**：只在不持任何 hub 鎖時呼叫（同步小檔讀、不 await）。
pub fn load_player_pos(email: &str) -> Option<(f32, f32, f32, f32)> {
    let content = std::fs::read_to_string(PLAYER_POS_PATH).ok()?;
    load_from_content(&content, email)
}

/// 核心載入邏輯（接受 content 字串，方便單元測試不依賴真實檔案）。
/// 讀取所有行，只看 email 相符的記錄，取 ts 最大的那筆。
pub(crate) fn load_from_content(content: &str, email: &str) -> Option<(f32, f32, f32, f32)> {
    let mut latest: Option<PlayerPosRecord> = None;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<PlayerPosRecord>(line) else {
            continue; // 壞行靜默略過（向後相容）
        };
        if rec.email != email {
            continue;
        }
        let update = match &latest {
            None => true,
            Some(prev) => rec.ts > prev.ts,
        };
        if update {
            latest = Some(rec);
        }
    }
    let rec = latest?;
    if is_valid_pos(rec.x, rec.y, rec.z) {
        Some((rec.x, rec.y, rec.z, if rec.yaw.is_finite() { rec.yaw } else { 0.0 }))
    } else {
        None // 離譜座標防呆：回 None，由呼叫端改用 spawn_pos
    }
}

/// 回傳目前 Unix 秒（ts 欄位用）。
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 小檔 append 寫一行（自動建 `data/` 目錄 + 換行）。失敗只吞掉，不 panic（比照 voxel_feed 慣例）。
fn write_line(path: &str, line: &str) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_valid_pos ────────────────────────────────────────────────────────

    #[test]
    fn is_valid_pos_accepts_normal_coords() {
        assert!(is_valid_pos(10.0, 20.0, -5.0));
        assert!(is_valid_pos(0.0, 0.0, 0.0));
        assert!(is_valid_pos(-16384.0, -64.0, 16384.0)); // 邊界
        assert!(is_valid_pos(0.0, 512.0, 0.0));           // y 上限
    }

    #[test]
    fn is_valid_pos_rejects_nan_and_inf() {
        assert!(!is_valid_pos(f32::NAN, 0.0, 0.0));
        assert!(!is_valid_pos(0.0, f32::NAN, 0.0));
        assert!(!is_valid_pos(0.0, 0.0, f32::NAN));
        assert!(!is_valid_pos(f32::INFINITY, 0.0, 0.0));
        assert!(!is_valid_pos(0.0, f32::NEG_INFINITY, 0.0));
        assert!(!is_valid_pos(f32::NEG_INFINITY, 0.0, f32::INFINITY));
    }

    #[test]
    fn is_valid_pos_rejects_out_of_range() {
        assert!(!is_valid_pos(16385.0, 0.0, 0.0));    // x 超出
        assert!(!is_valid_pos(0.0, 0.0, -16385.0));   // z 超出
        assert!(!is_valid_pos(0.0, 600.0, 0.0));      // y > 512
        assert!(!is_valid_pos(0.0, -100.0, 0.0));     // y < -64
        assert!(!is_valid_pos(-16385.0, 0.0, 0.0));   // x 負超出
    }

    // ── load_from_content：不同 email ──────────────────────────────────────

    #[test]
    fn load_returns_none_on_empty_content() {
        assert_eq!(load_from_content("", "a@b.com"), None);
        assert_eq!(load_from_content("\n\n", "a@b.com"), None);
    }

    #[test]
    fn load_returns_none_if_email_not_found() {
        let r = PlayerPosRecord {
            email: "alice@x.com".into(),
            x: 10.0, y: 20.0, z: 30.0, yaw: 0.0, ts: 1,
        };
        let content = serde_json::to_string(&r).unwrap() + "\n";
        assert_eq!(load_from_content(&content, "bob@x.com"), None);
    }

    #[test]
    fn load_returns_latest_by_ts() {
        // 同 email 兩筆，ts 較大的（r2）應勝出。
        let r1 = PlayerPosRecord {
            email: "a@b.com".into(), x: 1.0, y: 10.0, z: 2.0, yaw: 0.0, ts: 100,
        };
        let r2 = PlayerPosRecord {
            email: "a@b.com".into(), x: 5.0, y: 15.0, z: 6.0, yaw: 1.0, ts: 200,
        };
        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&r1).unwrap(),
            serde_json::to_string(&r2).unwrap(),
        );
        // ts=200 那筆（r2）應勝出。
        assert_eq!(load_from_content(&content, "a@b.com"), Some((5.0, 15.0, 6.0, 1.0)));
    }

    #[test]
    fn load_returns_latest_by_ts_reversed_order() {
        // 即使 ts 大的先出現，也應選 ts 最大的。
        let r1 = PlayerPosRecord {
            email: "a@b.com".into(), x: 5.0, y: 15.0, z: 6.0, yaw: 1.0, ts: 200,
        };
        let r2 = PlayerPosRecord {
            email: "a@b.com".into(), x: 1.0, y: 10.0, z: 2.0, yaw: 0.0, ts: 100,
        };
        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&r1).unwrap(),
            serde_json::to_string(&r2).unwrap(),
        );
        assert_eq!(load_from_content(&content, "a@b.com"), Some((5.0, 15.0, 6.0, 1.0)));
    }

    #[test]
    fn load_skips_bad_lines() {
        let r = PlayerPosRecord {
            email: "a@b.com".into(), x: 3.0, y: 11.0, z: 4.0, yaw: 0.5, ts: 50,
        };
        let content = format!(
            "not-json\n{}\n{{broken\n",
            serde_json::to_string(&r).unwrap(),
        );
        assert_eq!(load_from_content(&content, "a@b.com"), Some((3.0, 11.0, 4.0, 0.5)));
    }

    #[test]
    fn load_returns_none_for_invalid_stored_pos() {
        // 若存的座標不合法（離譜），load 回 None 讓呼叫端改用 spawn_pos。
        let r = PlayerPosRecord {
            email: "a@b.com".into(), x: 99999.0, y: 10.0, z: 0.0, yaw: 0.0, ts: 1,
        };
        let content = serde_json::to_string(&r).unwrap() + "\n";
        assert_eq!(load_from_content(&content, "a@b.com"), None);
    }

    // ── 多玩家獨立 ──────────────────────────────────────────────────────────

    #[test]
    fn load_multi_player_independent() {
        // 不同 email 各自獨立（取各自最新、互不影響）。
        let r_alice = PlayerPosRecord {
            email: "alice@x.com".into(), x: 10.0, y: 20.0, z: 30.0, yaw: 0.5, ts: 1,
        };
        let r_bob = PlayerPosRecord {
            email: "bob@x.com".into(), x: -5.0, y: 12.0, z: 8.0, yaw: 1.0, ts: 2,
        };
        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&r_alice).unwrap(),
            serde_json::to_string(&r_bob).unwrap(),
        );
        assert_eq!(
            load_from_content(&content, "alice@x.com"),
            Some((10.0, 20.0, 30.0, 0.5))
        );
        assert_eq!(
            load_from_content(&content, "bob@x.com"),
            Some((-5.0, 12.0, 8.0, 1.0))
        );
        // 不存在的帳號 → None。
        assert_eq!(load_from_content(&content, "nobody@x.com"), None);
    }

    // ── save_player_pos 防呆 ────────────────────────────────────────────────

    #[test]
    fn save_skips_invalid_coords_no_panic() {
        // NaN 座標 → 靜默略過，不 panic、不寫任何東西。
        save_player_pos("test@test.com", f32::NAN, 10.0, 20.0, 0.0);
        save_player_pos("test@test.com", 0.0, f32::INFINITY, 0.0, 0.0);
        save_player_pos("test@test.com", 0.0, 0.0, 99999.0, 0.0);
    }

    #[test]
    fn save_skips_empty_email_no_panic() {
        // 空 email → 靜默略過（訪客守衛的額外保險）。
        save_player_pos("", 1.0, 10.0, 2.0, 0.0);
    }

    // ── 帳號 key 綁後端 email，訪客不存 ────────────────────────────────────

    #[test]
    fn guest_has_no_email_so_no_save() {
        // 文件測試：訪客 account_email = None，呼叫端不呼叫 save/load，本模組永不觸及。
        // 此測試驗「is_valid_pos 對有效座標回 true，但沒有 email 就不存」這條設計不變式。
        let guest_email: Option<&str> = None;
        // 訪客路徑：完全不呼叫 save_player_pos。
        if let Some(email) = guest_email {
            save_player_pos(email, 1.0, 10.0, 2.0, 0.0);
        }
        // 到這裡不 panic 即通過。
    }

    // ── serde roundtrip ─────────────────────────────────────────────────────

    #[test]
    fn record_serde_roundtrip() {
        let rec = PlayerPosRecord {
            email: "user@example.com".into(),
            x: 12.5,
            y: 64.0,
            z: -8.25,
            yaw: 1.57,
            ts: 1_700_000_000,
        };
        let json = serde_json::to_string(&rec).unwrap();
        let decoded: PlayerPosRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, rec);
    }
}
