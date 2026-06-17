//! 鎮民互助分享（ROADMAP 369）。
//!
//! 故鄉七大 NPC 自 ROADMAP 69 起就各有三股內心需求（安全感／歸屬感／繁榮感），至今這些數值
//! 只默默調制 NPC 對話的語氣，**從不驅動任何看得見的行為**——一位手頭寬裕的鎮民與一位正捉襟見肘
//! 的鎮民，彼此之間從不發生任何事。本切片讓那份「繁榮感」第一次長出**鎮民彼此之間的行動**：
//! 當一位居民日子過得寬裕（繁榮感高），而另一位正過得拮据（繁榮感低）時，寬裕的那位會主動
//! 勻一份心意給拮据的鄰里——拮据者的繁榮感回升一截、寬裕者也分出一點，世界頻道飄來一句暖訊，
//! 玩家更會親眼看見一份心意化作小小光禮，自送禮者腳邊飄越廣場、落到受禮者手中。
//!
//! 這是北極星願景（`docs/VISION_AI_EMERGENT_ECOSYSTEM.md`：NPC 各有需求、自有經濟、彼此互動）
//! 的一小步——個體依**自身需求**自發地彼此牽動，不是設計者寫死的劇本。
//!
//! 設計鐵律（刻意與既有系統乾淨分工）：
//! - **走「需求」維度，不碰關係網**：選誰分享給誰，只看雙方的**繁榮感**落差（ROADMAP 69 的 needs），
//!   與 364～366 的好惡關係網（`npc_relations`）正交——那條管「誰跟誰親」，這條管「誰寬裕、誰拮据」。
//! - **療癒向、零失控**：軟門檻 + 落差門檻把分享限縮在「真有明顯貧富差」時才發生；分享只在
//!   七大 NPC 的內心需求值之間搬動，**不送任何玩家物品／乙太／戰力，零平衡風險、零經濟擾動**。
//! - **純邏輯可獨立測試**：`pick_share` 是純函式（吃繁榮感快照、回一樁分享事件或 None）；
//!   進行中的送禮手勢是 `TownShareState` 上一個記憶體前置、不持久化、零 migration 的計時器。
//! - 零 LLM、純查表 + 整數比較；面向玩家字串集中本檔，作為 i18n 替換點，繁中註解。

use crate::protocol::TownShareView;

/// 送禮者的繁榮感至少要到這個門檻，才算「寬裕到有餘力分享」。
const SURPLUS_THRESHOLD: i32 = 58;
/// 受禮者的繁榮感低於這個門檻，才算「拮据到值得幫襯」。
const NEED_THRESHOLD: i32 = 48;
/// 送禮者與受禮者的繁榮感落差至少要這麼大，分享才有意義（避免兩人差不多時也硬要分）。
const MIN_GAP: i32 = 12;

/// 一次分享，受禮者繁榮感回升的量。
pub const GIVE_AMOUNT: i32 = 6;
/// 一次分享，送禮者勻出的量（比受禮者得到的少——心意是會「增值」的，療癒向設計）。
pub const GIVER_COST: i32 = 3;

/// 送禮手勢（光禮飄越廣場）的持續秒數——前端據此把光禮從送禮者位置補間到受禮者位置。
pub const GESTURE_SECS: f32 = 3.0;

/// `pick_share` 的輸入：一位 NPC 的 id 與當前繁榮感。
#[derive(Debug, Clone, Copy)]
pub struct ShareCandidate {
    pub id: &'static str,
    pub prosperity: i32,
}

/// 一樁選定的分享事件（誰勻給誰、各動多少）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareEvent {
    pub giver: &'static str,
    pub receiver: &'static str,
    pub give: i32,
    pub cost: i32,
}

/// 從七大 NPC 的繁榮感快照中，挑出一樁「寬裕者勻給拮据者」的分享——純函式、確定性、只讀。
///
/// 規則：送禮者＝繁榮感最高且 ≥ `SURPLUS_THRESHOLD` 者；受禮者＝繁榮感最低且 ≤ `NEED_THRESHOLD` 者；
/// 兩者須不同、且落差 ≥ `MIN_GAP`。`last_pair`（上一次的送禮者→受禮者）若與本次相同則跳過，
/// 避免同一對來回反覆刷頻。平手時取 `candidates` 次序最前者（確定可重現）。
pub fn pick_share(
    candidates: &[ShareCandidate],
    last_pair: Option<(&str, &str)>,
) -> Option<ShareEvent> {
    // 送禮者：繁榮感最高者（嚴格大於才取代 → 平手取次序最前）。
    let giver = candidates
        .iter()
        .copied()
        .fold(None::<ShareCandidate>, |best, c| match best {
            Some(b) if c.prosperity > b.prosperity => Some(c),
            Some(b) => Some(b),
            None => Some(c),
        })?;
    // 受禮者：繁榮感最低者（嚴格小於才取代 → 平手取次序最前）。
    let receiver = candidates
        .iter()
        .copied()
        .fold(None::<ShareCandidate>, |best, c| match best {
            Some(b) if c.prosperity < b.prosperity => Some(c),
            Some(b) => Some(b),
            None => Some(c),
        })?;

    if giver.id == receiver.id {
        return None;
    }
    if giver.prosperity < SURPLUS_THRESHOLD || receiver.prosperity > NEED_THRESHOLD {
        return None;
    }
    if giver.prosperity - receiver.prosperity < MIN_GAP {
        return None;
    }
    if last_pair == Some((giver.id, receiver.id)) {
        return None;
    }

    Some(ShareEvent {
        giver: giver.id,
        receiver: receiver.id,
        give: GIVE_AMOUNT,
        cost: GIVER_COST,
    })
}

/// 組出一句世界頻道暖訊（面向玩家、i18n 替換點）。名稱由呼叫端鏡像 id→顯示名後傳入。
pub fn announce_text(giver_name: &str, receiver_name: &str) -> String {
    format!("🤝 {giver_name} 日子過得寬裕，勻了一份心意給手頭拮据的 {receiver_name}——鎮上的暖意，悄悄流動著。")
}

/// 進行中的送禮手勢（記憶體前置、不持久化、零 migration）。
#[derive(Debug, Clone)]
struct ActiveGesture {
    giver: String,
    receiver: String,
    /// 已經過的秒數（0 → `GESTURE_SECS`）。
    elapsed: f32,
}

/// 鎮民互助分享的執行期狀態：防反覆刷頻的 `last_pair` + 進行中的送禮手勢。
/// 純記憶體，重啟清零（與 npc_needs 同樣是記憶體模式）。
#[derive(Default)]
pub struct TownShareState {
    last_pair: Option<(String, String)>,
    active: Option<ActiveGesture>,
}

impl TownShareState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 上一次的送禮者→受禮者（供 `pick_share` 避免反覆刷同一對）。
    pub fn last_pair(&self) -> Option<(&str, &str)> {
        self.last_pair
            .as_ref()
            .map(|(g, r)| (g.as_str(), r.as_str()))
    }

    /// 一樁分享成立：記下這一對、並啟動一段飄越廣場的送禮手勢。
    pub fn begin(&mut self, giver: &str, receiver: &str) {
        self.last_pair = Some((giver.to_string(), receiver.to_string()));
        self.active = Some(ActiveGesture {
            giver: giver.to_string(),
            receiver: receiver.to_string(),
            elapsed: 0.0,
        });
    }

    /// 每 tick 推進送禮手勢；逾時即清除。非正/非有限 dt 不前進（防呆、守單調）。
    pub fn tick(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        if let Some(g) = self.active.as_mut() {
            g.elapsed += dt;
            if g.elapsed >= GESTURE_SECS {
                self.active = None;
            }
        }
    }

    /// 進行中的送禮手勢快照（供前端畫光禮）；無手勢時回 None。
    /// `t` 為手勢進度（0=剛從送禮者腳邊出發，1=抵達受禮者），前端據此在兩位 NPC 之間補間光禮位置。
    pub fn view(&self) -> Option<TownShareView> {
        self.active.as_ref().map(|g| TownShareView {
            giver: g.giver.clone(),
            receiver: g.receiver.clone(),
            t: (g.elapsed / GESTURE_SECS).clamp(0.0, 1.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &'static str, prosperity: i32) -> ShareCandidate {
        ShareCandidate { id, prosperity }
    }

    #[test]
    fn picks_richest_giver_and_poorest_receiver() {
        let cands = [
            c("merchant", 70),
            c("workshop_npc", 40),
            c("village_chief", 55),
        ];
        let ev = pick_share(&cands, None).expect("應選出一樁分享");
        assert_eq!(ev.giver, "merchant", "最寬裕者應是送禮者");
        assert_eq!(ev.receiver, "workshop_npc", "最拮据者應是受禮者");
        assert_eq!(ev.give, GIVE_AMOUNT);
        assert_eq!(ev.cost, GIVER_COST);
    }

    #[test]
    fn no_share_when_no_one_is_surplus() {
        // 最高者只有 55 < SURPLUS_THRESHOLD(58)：沒人寬裕到有餘力分享。
        let cands = [c("merchant", 55), c("workshop_npc", 40)];
        assert!(pick_share(&cands, None).is_none(), "無人寬裕時不應分享");
    }

    #[test]
    fn no_share_when_no_one_is_needy() {
        // 最低者 50 > NEED_THRESHOLD(48)：沒人拮据到值得幫襯。
        let cands = [c("merchant", 70), c("workshop_npc", 50)];
        assert!(pick_share(&cands, None).is_none(), "無人拮据時不應分享");
    }

    #[test]
    fn no_share_when_gap_too_small() {
        // 60 與 48：雖各自過/未達門檻，落差 12 == MIN_GAP 可分；改成 59 vs 48 落差 11 < 12 則否。
        let ok = [c("merchant", 60), c("workshop_npc", 48)];
        assert!(pick_share(&ok, None).is_some(), "落差達門檻應可分享");
        let small = [c("merchant", 59), c("workshop_npc", 48)];
        assert!(
            pick_share(&small, None).is_none(),
            "落差不足 MIN_GAP 時不應分享"
        );
    }

    #[test]
    fn skips_immediate_repeat_of_same_pair() {
        let cands = [c("merchant", 70), c("workshop_npc", 40)];
        // 上一次正是 merchant→workshop_npc：本次應跳過，避免反覆刷同一對。
        assert!(
            pick_share(&cands, Some(("merchant", "workshop_npc"))).is_none(),
            "與上一對相同時應跳過"
        );
        // 不同對（或無上一對）則照常選出。
        assert!(pick_share(&cands, Some(("village_chief", "merchant"))).is_some());
    }

    #[test]
    fn deterministic_tie_break_takes_earliest() {
        // 兩位並列最高、兩位並列最低：平手取 candidates 次序最前。
        let cands = [
            c("merchant", 70),
            c("bounty_npc", 70),
            c("workshop_npc", 40),
            c("expedition_npc", 40),
        ];
        let ev = pick_share(&cands, None).unwrap();
        assert_eq!(ev.giver, "merchant", "並列最高取次序最前");
        assert_eq!(ev.receiver, "workshop_npc", "並列最低取次序最前");
    }

    #[test]
    fn single_candidate_yields_nothing() {
        let cands = [c("merchant", 70)];
        assert!(
            pick_share(&cands, None).is_none(),
            "只有一位 NPC 不可能自己分給自己"
        );
        assert!(pick_share(&[], None).is_none(), "空名單回 None");
    }

    #[test]
    fn announce_text_names_both_parties() {
        let line = announce_text("商人薇拉", "工匠鐸恩");
        assert!(line.contains("商人薇拉"), "暖訊應點名送禮者");
        assert!(line.contains("工匠鐸恩"), "暖訊應點名受禮者");
    }

    #[test]
    fn gesture_lifecycle_runs_and_clears() {
        let mut st = TownShareState::new();
        assert!(st.view().is_none(), "初始無手勢");
        st.begin("merchant", "workshop_npc");
        assert_eq!(st.last_pair(), Some(("merchant", "workshop_npc")));
        let v = st.view().expect("begin 後應有手勢");
        assert_eq!(v.giver, "merchant");
        assert_eq!(v.receiver, "workshop_npc");
        assert!(v.t < 0.5, "剛出發進度應接近 0");
        // 推進到一半。
        st.tick(GESTURE_SECS * 0.5);
        let v = st.view().unwrap();
        assert!((v.t - 0.5).abs() < 0.05, "推進一半進度應約 0.5");
        // 推進到逾時 → 清除。
        st.tick(GESTURE_SECS);
        assert!(st.view().is_none(), "逾時後手勢應清除");
        // last_pair 仍保留（供下次防重）。
        assert_eq!(st.last_pair(), Some(("merchant", "workshop_npc")));
    }

    #[test]
    fn gesture_ignores_bad_dt() {
        let mut st = TownShareState::new();
        st.begin("merchant", "workshop_npc");
        st.tick(-1.0);
        st.tick(f32::NAN);
        let v = st.view().expect("壞 dt 不應推進或清除手勢");
        assert!(v.t < 0.05, "壞 dt 後進度仍應接近 0");
    }
}
