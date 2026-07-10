//! 乙太方界·對話 per-IP 速率限制（上架前治安三件套 ①）。
//!
//! **真缺口**：`voxel_ws` 的對話（Talk）分支會觸發免費 LLM 回應，原本只有一道
//! **per-connection** 冷卻（`talk_cooldown_ok`，區域變數 `last_talk`）。這道冷卻
//! 可被「**同一個人開一堆 WebSocket 連線**」輕易繞過——每條連線各有自己的冷卻計時，
//! 一個腳本開 50 條連線就等於 50 倍的對話速率，可白嫖／燒爆免費 LLM 額度、洗版世界。
//!
//! 本模組補上一道**跨連線、以真實 IP 為鍵**的天花板：一個 [token bucket]（漏桶的對偶），
//! 每個 IP 一只桶，突發上限 [`TALK_BUCKET_CAP`] 個 token、每秒回補 [`TALK_REFILL_PER_SEC`]
//! 個——正常單人閒聊（每 4 秒一句＝15/分）遠在限額內，開幾十條連線猛灌的腳本則被壓到
//! 每 IP ~36/分的硬頂，無論開幾條連線都逃不掉。
//!
//! **設計取捨**：
//! - **純邏輯、時鐘注入**：`allow(ip, now_ms)` 由呼叫端傳入時間，模組內不讀時鐘 → 確定性、可測。
//! - **記憶體有界**：桶數超過 [`MAX_TRACKED_IPS`] 先清掉「已回滿」（久未活動）的桶，仍滿才整個清空。
//! - **時鐘回退安全**：`now < last` 以 `saturating_sub` 視為零回補，不會 panic 或倒扣。
//! - **信任邊界**：真實 IP 取自 Cloudflare `cf-connecting-ip`（tunnel 後才可信，退而求其次
//!   `x-forwarded-for`）——與既有建議箱 per-IP 限流（main.rs `suggest_rate_ok`）同一套姿態。
//!   origin 只經 CF tunnel 對外（未公開直連），故無法靠偽造標頭繞過（見 PR 濫用防護說明）。
//!
//! 這裡只放確定性純邏輯；鎖／連線／取 IP 都在 `voxel_ws.rs`。不抄外部碼；繁中註解。

use std::collections::HashMap;

/// 每個 IP 的突發上限：桶最多裝這麼多 token（＝一口氣最多這麼多則對話）。
pub const TALK_BUCKET_CAP: f64 = 12.0;
/// 每個 IP 的持續回補速率：每秒補這麼多 token（0.6/秒 ≈ 36 則/分鐘的長期上限）。
pub const TALK_REFILL_PER_SEC: f64 = 0.6;
/// 追蹤的 IP 桶數硬上限，防 map 無限長大（配合 [`IpTalkLimiter::prune_full`]）。
const MAX_TRACKED_IPS: usize = 20_000;

/// 單一 IP 的漏桶狀態。
#[derive(Clone, Copy, Debug)]
struct Bucket {
    /// 目前桶內剩餘 token（0.0 ~ [`TALK_BUCKET_CAP`]）。
    tokens: f64,
    /// 上次更新的時間戳（毫秒，由呼叫端注入）。
    last_ms: u64,
}

/// per-IP 對話速率限制器：每個 IP 一只 token bucket。
#[derive(Default)]
pub struct IpTalkLimiter {
    buckets: HashMap<String, Bucket>,
}

impl IpTalkLimiter {
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    /// 嘗試為 `ip` 花一個 token（放行一則對話）。
    ///
    /// 放行回 `true` 並扣一個 token；桶已見底回 `false`（不扣、不改變桶）。
    /// `now_ms` 由呼叫端注入的毫秒時間戳（純函式、可測，模組內不讀時鐘）。
    pub fn allow(&mut self, ip: &str, now_ms: u64) -> bool {
        // 記憶體有界：桶數觸頂先清「已回滿」的久置桶，仍滿才整桶清空（極端保底、寧可誤放不吃爆記憶體）。
        if self.buckets.len() >= MAX_TRACKED_IPS && !self.buckets.contains_key(ip) {
            self.prune_full(now_ms);
            if self.buckets.len() >= MAX_TRACKED_IPS {
                self.buckets.clear();
            }
        }
        let bucket = self.buckets.entry(ip.to_string()).or_insert(Bucket {
            tokens: TALK_BUCKET_CAP,
            last_ms: now_ms,
        });
        // 依經過時間回補 token（clamp 到上限）；時鐘回退（now < last）→ 零回補、不倒扣。
        let elapsed_ms = now_ms.saturating_sub(bucket.last_ms);
        bucket.tokens =
            (bucket.tokens + elapsed_ms as f64 / 1000.0 * TALK_REFILL_PER_SEC).min(TALK_BUCKET_CAP);
        bucket.last_ms = now_ms;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// 清掉 token 已回滿（久未活動、無限流必要）的桶，回收記憶體。
    fn prune_full(&mut self, now_ms: u64) {
        self.buckets.retain(|_, b| {
            let elapsed_ms = now_ms.saturating_sub(b.last_ms);
            let tokens =
                (b.tokens + elapsed_ms as f64 / 1000.0 * TALK_REFILL_PER_SEC).min(TALK_BUCKET_CAP);
            tokens < TALK_BUCKET_CAP
        });
    }

    /// 目前追蹤的 IP 桶數（測試／觀測用）。
    #[cfg(test)]
    pub fn tracked(&self) -> usize {
        self.buckets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全新 IP：可連續放行到突發上限，超過即擋。
    #[test]
    fn fresh_ip_allows_burst_up_to_cap() {
        let mut rl = IpTalkLimiter::new();
        let cap = TALK_BUCKET_CAP as usize;
        // 同一毫秒連打：前 cap 則全放，第 cap+1 則被擋。
        for i in 0..cap {
            assert!(rl.allow("1.2.3.4", 1000), "第 {i} 則應放行");
        }
        assert!(!rl.allow("1.2.3.4", 1000), "超過突發上限應被擋");
    }

    /// 時間經過會回補 token → 稍後又能說話。
    #[test]
    fn refills_over_time() {
        let mut rl = IpTalkLimiter::new();
        let cap = TALK_BUCKET_CAP as usize;
        for _ in 0..cap {
            assert!(rl.allow("ip", 0));
        }
        assert!(!rl.allow("ip", 0), "剛榨乾應被擋");
        // 過 2 秒 → 回補 2 * 0.6 = 1.2 個 token → 至少放行一則。
        assert!(rl.allow("ip", 2000), "回補後應放行");
    }

    /// 回補有上限：長時間閒置不會累積超過突發上限。
    #[test]
    fn refill_caps_at_burst_limit() {
        let mut rl = IpTalkLimiter::new();
        // 閒置一整天再回來：桶最多只補到 cap，不會爆量。
        let cap = TALK_BUCKET_CAP as usize;
        for _ in 0..cap {
            assert!(rl.allow("ip", 86_400_000));
        }
        assert!(!rl.allow("ip", 86_400_000), "回補封頂：不因久置而超過突發上限");
    }

    /// 不同 IP 各自獨立計量，互不影響（一個人洗版不會拖累別人）。
    #[test]
    fn different_ips_are_independent() {
        let mut rl = IpTalkLimiter::new();
        let cap = TALK_BUCKET_CAP as usize;
        for _ in 0..cap {
            assert!(rl.allow("attacker", 0));
        }
        assert!(!rl.allow("attacker", 0), "洗版者自己被擋");
        assert!(rl.allow("innocent", 0), "另一個 IP 不受影響");
    }

    /// 時鐘回退（now < last）不 panic、不倒扣、不異常放行。
    #[test]
    fn clock_going_backward_is_safe() {
        let mut rl = IpTalkLimiter::new();
        assert!(rl.allow("ip", 10_000));
        // 時間倒退：saturating_sub → 零回補，桶不變，仍照常判定。
        assert!(rl.allow("ip", 5_000));
        let cap = TALK_BUCKET_CAP as usize;
        for _ in 0..cap {
            let _ = rl.allow("ip", 5_000);
        }
        assert!(!rl.allow("ip", 5_000), "時鐘回退期間不會異常放行超額");
    }

    /// 空字串 IP 也當作一只獨立桶處理，不 panic。
    #[test]
    fn empty_ip_key_is_handled() {
        let mut rl = IpTalkLimiter::new();
        assert!(rl.allow("", 0));
        assert_eq!(rl.tracked(), 1);
    }

    /// 桶數受硬上限約束：大量不同 IP 湧入後，追蹤數不會無限長大。
    #[test]
    fn tracked_ips_are_bounded() {
        let mut rl = IpTalkLimiter::new();
        // 灌入超過上限數量的獨立 IP（各花一個 token、留下未滿桶），再全部回滿後觸發清理。
        for i in 0..(MAX_TRACKED_IPS + 50) {
            rl.allow(&format!("ip-{i}"), 0);
        }
        // 觸頂後應已清過一輪，桶數落在上限之內。
        assert!(rl.tracked() <= MAX_TRACKED_IPS, "追蹤桶數應受硬上限約束");
    }
}
