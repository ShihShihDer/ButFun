//! 雪季堆雪人（ROADMAP 478）——隆冬時節，玩家在腳下把雪堆成一個雪人，立在世界裡，
//! 附近所有玩家都看得見、上頭還標著是誰堆的；天一回暖（季節離開冬季）雪人就融化消失。
//!
//! 設計取捨（刻意換骨架、不複製既有套路）：
//! - 冬季此前只是「作物長最慢」的被動背景（季節倍率 ×0.5）＋前端飄雪粒子，從沒有一個
//!   專屬於冬天的動詞。本切片給寒冬第一個**簽名活動**：堆雪人。
//! - 與野營篝火（474）／稻草人（476）同為「在世界裡擺一座物件」，但骨架關鍵不同：
//!   篝火有固定燃燒倒數、稻草人常駐，**雪人沒有逐座計時器，壽命由季節驅動**——整個冬天
//!   都在，季節一離開冬季就整批融化（`tick` 的 `is_winter` 旗標）。這是遊戲第一個
//!   「生命週期綁定特定季節」的世界物件，也是冬季第一個自我表達動詞
//!   （承接放風箏 470／夜螢提燈 477「擺出一個全服看得見、署名的東西」這條表達線）。
//! - 純表現、零玩法增益：雪人不回血、不驅敵、不送物品，就只是冬天裡一座可愛、署名的地標。
//!
//! 成本／安全紀律：
//! - 純記憶體模式，重啟清零。零 migration、零 LLM、零持久化、不碰玩家存檔與經濟。
//! - 純邏輯可獨立測試（堆雪冷卻、全服上限、回暖融化、樣式決定性），不依賴 WebSocket／遊戲迴圈。
//! - 平衡分寸：堆雪有每人冷卻＋全服同時上限，避免一個人洗版鋪滿雪人。
//! - 冬季限定的判定（只有冬天能堆）由呼叫端（ws 層，季節狀態在那）把關；本模組只負責
//!   「回暖即融化」與堆雪的速率／上限，保持純邏輯、確定性、好測。

use std::collections::HashMap;
use uuid::Uuid;

/// 全服同時存在的雪人上限——超過則堆雪靜默失敗，避免畫面被洗爆。
/// 雪人沒有逐座計時器（整個冬天都在），故上限比篝火寬鬆些，但仍有界。
pub const MAX_SNOWMEN: usize = 60;
/// 同一玩家兩次堆雪人的最短間隔（秒）——防止一個人連點鋪滿雪人。
pub const REBUILD_COOLDOWN_SECS: f32 = 6.0;
/// 雪人外觀樣式種類數——由 id 決定性取模，讓每座雪人圍巾／表情略有不同，堆起來各有個性。
pub const SNOWMAN_STYLES: u8 = 4;

/// 一座雪人（純記憶體）。
#[derive(Debug, Clone)]
pub struct Snowman {
    pub id: u32,
    pub wx: f32,
    pub wy: f32,
    /// 堆雪人的玩家暱稱（面向玩家字串，前端在雪人上方顯示「❄️ XXX 堆的」）。
    pub builder: String,
    /// 外觀樣式（0..SNOWMAN_STYLES）——由 id 決定，前端據此換圍巾色／表情。
    pub style: u8,
}

/// 全服雪人狀態（純記憶體，重啟清零）。
pub struct SnowmanField {
    /// 目前立在世界裡的雪人。
    snowmen: Vec<Snowman>,
    /// 雪人 id 計數器（遞增，確保 id 不重複）。
    counter: u32,
    /// 每位玩家的堆雪冷卻剩餘秒數；>0 表示還在冷卻、暫不能再堆。
    cooldowns: HashMap<Uuid, f32>,
}

impl SnowmanField {
    pub fn new() -> Self {
        Self {
            snowmen: Vec::new(),
            counter: 0,
            cooldowns: HashMap::new(),
        }
    }

    /// 目前立著的雪人數量。
    pub fn len(&self) -> usize {
        self.snowmen.len()
    }

    /// 是否一座雪人都沒有。
    pub fn is_empty(&self) -> bool {
        self.snowmen.is_empty()
    }

    /// 目前所有雪人（供快照廣播給前端顯示）。
    pub fn active(&self) -> &[Snowman] {
        &self.snowmen
    }

    /// 推進時間（`dt` 秒）：
    /// - `is_winter == false`（季節已離開冬季）→ 全部雪人**融化清空**（不論 dt）。
    /// - 每位玩家的堆雪冷卻一併遞減、歸零清除（避免 map 無限長大）。
    pub fn tick(&mut self, dt: f32, is_winter: bool) {
        // 回暖即融化：天一暖，整批雪人化掉。冬季外永遠是空的。
        if !is_winter {
            self.snowmen.clear();
        }
        if dt <= 0.0 {
            return;
        }
        self.cooldowns.retain(|_, cd| {
            *cd -= dt;
            *cd > 0.0
        });
    }

    /// 嘗試替玩家 `pid`（暱稱 `builder`）在其權威座標 `(px, py)` 堆一座雪人。
    /// 回傳 `Some(id)` 表示成功（`id` = 新雪人編號）；`None` 表示失敗：
    /// 座標非有限值、該玩家仍在堆雪冷卻中、或全服雪人已達上限。
    /// 冬季限定由呼叫端把關（季節狀態在 ws 層）；本函式純邏輯、確定性。
    /// 呼叫端負責先讀玩家權威座標（防隔空堆雪）、出鎖後才廣播。
    pub fn build(&mut self, pid: Uuid, builder: String, px: f32, py: f32) -> Option<u32> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        // 堆雪冷卻中。
        if self.cooldowns.get(&pid).is_some_and(|&cd| cd > 0.0) {
            return None;
        }
        // 全服上限。
        if self.snowmen.len() >= MAX_SNOWMEN {
            return None;
        }
        let id = self.counter;
        self.counter = self.counter.wrapping_add(1);
        let style = (id % SNOWMAN_STYLES as u32) as u8;
        self.snowmen.push(Snowman {
            id,
            wx: px,
            wy: py,
            builder,
            style,
        });
        self.cooldowns.insert(pid, REBUILD_COOLDOWN_SECS);
        Some(id)
    }
}

impl Default for SnowmanField {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn new_field_is_empty() {
        let f = SnowmanField::new();
        assert!(f.is_empty());
        assert_eq!(f.len(), 0);
        assert!(f.active().is_empty());
    }

    #[test]
    fn build_adds_a_snowman_with_builder_and_style() {
        let mut f = SnowmanField::new();
        let id = f.build(pid(1), "小雪".to_string(), 100.0, 200.0);
        assert!(id.is_some(), "首次堆雪人應成功");
        assert_eq!(f.len(), 1);
        let s = &f.active()[0];
        assert_eq!(s.wx, 100.0);
        assert_eq!(s.wy, 200.0);
        assert_eq!(s.builder, "小雪");
        assert!(s.style < SNOWMAN_STYLES, "樣式須落在合法範圍");
    }

    #[test]
    fn build_rejects_non_finite_position() {
        let mut f = SnowmanField::new();
        assert_eq!(f.build(pid(1), "a".into(), f32::NAN, 0.0), None, "NaN 座標應拒絕");
        assert_eq!(f.build(pid(1), "a".into(), 0.0, f32::INFINITY), None, "Inf 座標應拒絕");
        assert!(f.is_empty(), "非有限座標不應產生雪人");
    }

    #[test]
    fn build_is_rate_limited_per_player() {
        let mut f = SnowmanField::new();
        assert!(f.build(pid(1), "a".into(), 0.0, 0.0).is_some(), "首次堆雪成功");
        assert_eq!(f.build(pid(1), "a".into(), 50.0, 50.0), None, "冷卻中同一人不能再堆");
        assert_eq!(f.len(), 1);
        // 另一位玩家不受別人冷卻影響。
        assert!(f.build(pid(2), "b".into(), 0.0, 0.0).is_some(), "別的玩家可各自堆雪");
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn cooldown_clears_after_waiting() {
        let mut f = SnowmanField::new();
        f.build(pid(1), "a".into(), 0.0, 0.0);
        f.tick(REBUILD_COOLDOWN_SECS - 1.0, true);
        assert_eq!(f.build(pid(1), "a".into(), 0.0, 0.0), None, "冷卻未滿不能再堆");
        f.tick(2.0, true);
        assert!(f.build(pid(1), "a".into(), 0.0, 0.0).is_some(), "冷卻走完後可再堆");
    }

    #[test]
    fn thaw_melts_all_snowmen() {
        let mut f = SnowmanField::new();
        f.build(pid(1), "a".into(), 0.0, 0.0);
        f.build(pid(2), "b".into(), 10.0, 10.0);
        assert_eq!(f.len(), 2);
        // 冬天裡推進時間：雪人仍在。
        f.tick(1.0, true);
        assert_eq!(f.len(), 2, "冬季內雪人不融化");
        // 季節離開冬季：整批融化。
        f.tick(1.0, false);
        assert!(f.is_empty(), "回暖後雪人應全部融化清空");
    }

    #[test]
    fn cannot_build_logic_is_caller_gated_but_field_stays_consistent() {
        // 本模組不認得季節；非冬季時呼叫端不會呼叫 build，但即使誤呼叫，
        // 下一個 tick(_, false) 也會把它清掉——確保不殘留。
        let mut f = SnowmanField::new();
        f.build(pid(1), "a".into(), 0.0, 0.0);
        f.tick(0.5, false);
        assert!(f.is_empty(), "回暖 tick 應清掉任何殘留雪人");
    }

    #[test]
    fn global_cap_blocks_extra_snowmen() {
        let mut f = SnowmanField::new();
        for i in 0..MAX_SNOWMEN {
            assert!(f.build(pid(i as u128), "a".into(), i as f32, 0.0).is_some());
        }
        assert_eq!(f.len(), MAX_SNOWMEN);
        assert_eq!(
            f.build(pid(9999), "a".into(), 0.0, 0.0),
            None,
            "達全服上限後再堆雪應失敗"
        );
        assert_eq!(f.len(), MAX_SNOWMEN);
    }

    #[test]
    fn ids_and_styles_are_deterministic() {
        let mut f = SnowmanField::new();
        let a = f.build(pid(1), "a".into(), 0.0, 0.0).unwrap();
        let b = f.build(pid(2), "b".into(), 0.0, 0.0).unwrap();
        assert_ne!(a, b, "不同雪人 id 不應重複");
        // 樣式由 id 取模決定，確定可重現。
        assert_eq!(f.active()[0].style, (a % SNOWMAN_STYLES as u32) as u8);
        assert_eq!(f.active()[1].style, (b % SNOWMAN_STYLES as u32) as u8);
    }

    #[test]
    fn tick_zero_dt_keeps_winter_snowmen() {
        let mut f = SnowmanField::new();
        f.build(pid(1), "a".into(), 0.0, 0.0);
        f.tick(0.0, true);
        f.tick(-5.0, true);
        assert_eq!(f.len(), 1, "壞 dt 在冬季不應影響雪人");
    }
}
