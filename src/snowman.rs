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

use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// 全服同時存在的雪人上限——超過則堆雪靜默失敗，避免畫面被洗爆。
/// 雪人沒有逐座計時器（整個冬天都在），故上限比篝火寬鬆些，但仍有界。
pub const MAX_SNOWMEN: usize = 60;
/// 同一玩家兩次堆雪人的最短間隔（秒）——防止一個人連點鋪滿雪人。
pub const REBUILD_COOLDOWN_SECS: f32 = 6.0;
/// 雪人外觀樣式種類數——由 id 決定性取模，讓每座雪人圍巾／表情略有不同，堆起來各有個性。
pub const SNOWMAN_STYLES: u8 = 4;
/// 讚賞雪人的搆得著半徑（世界座標單位）——玩家須走近到這個距離內才能替雪人按讚，
/// 防止隔空讚賞。比照其他「走近才能互動」的世界物件半徑。
pub const CHEER_RADIUS: f32 = 80.0;

/// 替雪人按讚的結果（純邏輯，供 ws 層決定要不要送通知）。
#[derive(Debug, Clone, PartialEq)]
pub enum CheerOutcome {
    /// 成功讚賞：`cheers` = 該雪人最新累積愛心數；`builder_pid`/`builder_name` =
    /// 堆雪者（ws 層據此把暖心通知單播給堆雪者）。
    Ok {
        cheers: u16,
        builder_pid: Uuid,
        builder_name: String,
    },
    /// 找不到這座雪人（已融化／id 不存在）。
    NotFound,
    /// 太遠了，搆不著（不在 `CHEER_RADIUS` 內，或座標非有限值）。
    OutOfRange,
    /// 不能讚賞自己堆的雪人。
    Own,
    /// 已經讚賞過這座雪人了（一人一座一次）。
    AlreadyCheered,
}

/// 一座雪人（純記憶體）。
#[derive(Debug, Clone)]
pub struct Snowman {
    pub id: u32,
    pub wx: f32,
    pub wy: f32,
    /// 堆雪人的玩家暱稱（面向玩家字串，前端在雪人上方顯示「❄️ XXX 堆的」）。
    pub builder: String,
    /// 堆雪者的玩家 id（伺服器端用：擋自讚、被讚時找堆雪者單播道賀；不外送前端）。
    pub builder_pid: Uuid,
    /// 外觀樣式（0..SNOWMAN_STYLES）——由 id 決定，前端據此換圍巾色／表情。
    pub style: u8,
    /// 累積愛心數（ROADMAP 479 雪人讚賞）——附近玩家走近按讚就 +1，全服可見、隨快照廣播。
    pub cheers: u16,
    /// 已讚賞過這座雪人的玩家（伺服器端用：保證一人一座只能讚一次；不外送前端）。
    cheered_by: HashSet<Uuid>,
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
            builder_pid: pid,
            style,
            cheers: 0,
            cheered_by: HashSet::new(),
        });
        self.cooldowns.insert(pid, REBUILD_COOLDOWN_SECS);
        Some(id)
    }

    /// 替雪人 `snowman_id` 按一個讚——讚賞者 `by`（在權威座標 `(bx, by)`）須走近到
    /// `CHEER_RADIUS` 內、不是自己堆的、且還沒讚過這座。成功則愛心 +1 並記下讚賞者，
    /// 回傳含最新愛心數與堆雪者資訊的 `CheerOutcome::Ok`（ws 層據此把暖心通知送給堆雪者）。
    /// 純邏輯、確定性、壞值保守（非有限座標一律當搆不著）；呼叫端負責先讀讚賞者權威座標
    /// （防隔空讚賞）、出鎖後才送通知。
    pub fn cheer(&mut self, snowman_id: u32, by: Uuid, bx: f32, by_y: f32) -> CheerOutcome {
        let Some(s) = self.snowmen.iter_mut().find(|s| s.id == snowman_id) else {
            return CheerOutcome::NotFound;
        };
        // 不能讚自己堆的雪人。
        if s.builder_pid == by {
            return CheerOutcome::Own;
        }
        // 座標壞值或太遠都搆不著。
        if !bx.is_finite() || !by_y.is_finite() {
            return CheerOutcome::OutOfRange;
        }
        let dx = s.wx - bx;
        let dy = s.wy - by_y;
        if dx * dx + dy * dy > CHEER_RADIUS * CHEER_RADIUS {
            return CheerOutcome::OutOfRange;
        }
        // 一人一座只能讚一次。
        if !s.cheered_by.insert(by) {
            return CheerOutcome::AlreadyCheered;
        }
        s.cheers = s.cheers.saturating_add(1);
        CheerOutcome::Ok {
            cheers: s.cheers,
            builder_pid: s.builder_pid,
            builder_name: s.builder.clone(),
        }
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

    #[test]
    fn cheer_succeeds_for_nearby_other_player() {
        let mut f = SnowmanField::new();
        let id = f.build(pid(1), "小雪".into(), 100.0, 100.0).unwrap();
        // 另一位玩家走到雪人旁（搆得著）按讚。
        let out = f.cheer(id, pid(2), 110.0, 105.0);
        match out {
            CheerOutcome::Ok { cheers, builder_pid, builder_name } => {
                assert_eq!(cheers, 1, "首次讚賞愛心數應為 1");
                assert_eq!(builder_pid, pid(1), "應回報堆雪者 id");
                assert_eq!(builder_name, "小雪", "應回報堆雪者暱稱");
            }
            other => panic!("應讚賞成功，卻得到 {other:?}"),
        }
        assert_eq!(f.active()[0].cheers, 1, "雪人愛心數應已累加");
    }

    #[test]
    fn cheer_rejects_own_snowman() {
        let mut f = SnowmanField::new();
        let id = f.build(pid(1), "a".into(), 0.0, 0.0).unwrap();
        assert_eq!(f.cheer(id, pid(1), 0.0, 0.0), CheerOutcome::Own, "不能讚自己的雪人");
        assert_eq!(f.active()[0].cheers, 0, "自讚被擋，愛心數不動");
    }

    #[test]
    fn cheer_rejects_out_of_range_and_bad_coords() {
        let mut f = SnowmanField::new();
        let id = f.build(pid(1), "a".into(), 0.0, 0.0).unwrap();
        // 太遠（超過 CHEER_RADIUS）。
        assert_eq!(
            f.cheer(id, pid(2), CHEER_RADIUS + 10.0, 0.0),
            CheerOutcome::OutOfRange,
            "搆不著不能讚"
        );
        // 邊界內側恰好可讚。
        assert!(
            matches!(f.cheer(id, pid(2), CHEER_RADIUS - 1.0, 0.0), CheerOutcome::Ok { .. }),
            "半徑內應可讚"
        );
        // 壞座標保守當搆不著。
        assert_eq!(f.cheer(id, pid(3), f32::NAN, 0.0), CheerOutcome::OutOfRange, "NaN 座標搆不著");
        assert_eq!(f.cheer(id, pid(3), 0.0, f32::INFINITY), CheerOutcome::OutOfRange, "Inf 座標搆不著");
    }

    #[test]
    fn cheer_is_once_per_player_per_snowman() {
        let mut f = SnowmanField::new();
        let id = f.build(pid(1), "a".into(), 0.0, 0.0).unwrap();
        assert!(matches!(f.cheer(id, pid(2), 0.0, 0.0), CheerOutcome::Ok { cheers: 1, .. }));
        assert_eq!(f.cheer(id, pid(2), 0.0, 0.0), CheerOutcome::AlreadyCheered, "同人不能重複讚");
        // 不同玩家可各讚一次，累加。
        assert!(matches!(f.cheer(id, pid(3), 0.0, 0.0), CheerOutcome::Ok { cheers: 2, .. }));
        assert_eq!(f.active()[0].cheers, 2, "兩位玩家各讚一次 = 2");
    }

    #[test]
    fn cheer_unknown_snowman_is_not_found() {
        let mut f = SnowmanField::new();
        f.build(pid(1), "a".into(), 0.0, 0.0).unwrap();
        assert_eq!(f.cheer(9999, pid(2), 0.0, 0.0), CheerOutcome::NotFound, "不存在的雪人 id");
    }

    #[test]
    fn cheers_reset_when_snowmen_melt() {
        let mut f = SnowmanField::new();
        let id = f.build(pid(1), "a".into(), 0.0, 0.0).unwrap();
        f.cheer(id, pid(2), 0.0, 0.0);
        assert_eq!(f.active()[0].cheers, 1);
        // 回暖融化後重堆，是全新雪人、愛心歸零（換個沒在冷卻的玩家堆，避開堆雪冷卻）。
        f.tick(1.0, false);
        assert!(f.is_empty(), "回暖整批融化");
        let id2 = f.build(pid(7), "b".into(), 0.0, 0.0).unwrap();
        assert_eq!(f.active()[0].cheers, 0, "重堆的雪人愛心從零開始");
        assert_ne!(id, id2, "新雪人 id 與舊的不同");
    }
}
