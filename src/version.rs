//! 版本戳記：集中暴露「編譯期烤進 binary 的 git SHA／build 時間」，並提供部署自驗用的
//! 純比對函式。`/version` 端點、debug HUD、scripts/deploy.sh 全部共用這一份事實——
//! 目的是堵死「舊 binary 靜默上線、沒人發現」。
//!
//! SHA／時間由 build.rs 於編譯期透過 `cargo:rustc-env=` 注入；抓不到 git 時為 "unknown"。

/// 編譯期 git short SHA（build.rs 注入；抓不到 git 時為 "unknown"）。
pub const GIT_SHA: &str = env!("BUTFUN_GIT_SHA");

/// 編譯期 build 時間（UTC，`YYYY-MM-DDTHH:MM:SSZ`；build.rs 注入；抓不到時為 "unknown"）。
pub const BUILD_TIME: &str = env!("BUTFUN_BUILD_TIME");

/// 部署自驗的版本比對結果。抽成純函式（不碰 IO）好測。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 跑著的 commit == 目標 commit → 上線到位、成功。
    Match,
    /// 兩者不同 → 舊 binary 靜默上線，必須回滾。
    Mismatch,
    /// 任一邊空或 "unknown"（binary 不含 git 戳記，或 /version 還沒起來）→ 無法判定。
    /// 呼叫端應 retry（可能只是還沒起來）；retry 用盡仍 Unknown 視為失敗、別當成功放行。
    Unknown,
}

/// 比對「該部署的目標 commit」(expected) 與「跑著的 server 回報的 commit」(actual)。
///
/// expected = 工作樹 `git rev-parse --short HEAD`；
/// actual   = `curl /version` 回傳 JSON 裡的 `commit`。
///
/// 兩邊約定都用 short SHA（同長度）→ 用 trim 後字串相等判定。
/// 任一邊空白或字面 "unknown" → `Unknown`（無從比對，不可貿然當成功）。
pub fn verify(expected: &str, actual: &str) -> Verdict {
    let e = expected.trim();
    let a = actual.trim();
    if e.is_empty() || a.is_empty() || e == "unknown" || a == "unknown" {
        return Verdict::Unknown;
    }
    if e == a {
        Verdict::Match
    } else {
        Verdict::Mismatch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 相符_目標commit等於跑著的commit() {
        assert_eq!(verify("abc1234", "abc1234"), Verdict::Match);
        // 前後空白應被 trim 後仍相符（curl 抓出來可能帶換行）。
        assert_eq!(verify(" abc1234 ", "abc1234\n"), Verdict::Match);
    }

    #[test]
    fn 不符_舊binary靜默上線會被當場抓到() {
        // 跑著的是舊 commit → Mismatch → deploy 應回滾。
        assert_eq!(verify("abc1234", "old9999"), Verdict::Mismatch);
    }

    #[test]
    fn unknown_任一邊未知或空都無法判定() {
        assert_eq!(verify("abc1234", "unknown"), Verdict::Unknown);
        assert_eq!(verify("unknown", "abc1234"), Verdict::Unknown);
        assert_eq!(verify("", "abc1234"), Verdict::Unknown);
        assert_eq!(verify("abc1234", "   "), Verdict::Unknown);
    }
}
