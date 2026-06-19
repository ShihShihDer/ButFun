//! ROADMAP 411 遠遊見聞·初次踏足新天地——把 398 天地有名的「地名」長出一條探索玩法。
//!
//! 398（`region_name.rs`）讓世界第一次有了「地方感」：踏入一格 locale 就淡入一張地名卡。
//! 但那是**純呈現、零狀態**——卡片每次踏入都一樣，世界不記得你走過哪裡，探索本身沒有回報。
//! 本切片給探索／遠遊維度第一個**有回報的玩法**：世界開始**記得你這趟踏足過哪些地方**，
//! 每踏進一處本趟還沒去過的新 locale ＝一次「初次踏足」——地名卡綴上「✨初次踏足」、響一聲
//! 發現的清音、攢一點探索者熟練度，HUD 常駐一個「🧭 遠遊見聞 N 處」的足跡計數。
//! 直接回報「往外走、去沒去過的地方」這個無限世界北極星最核心的衝動。
//!
//! 全純記憶體、純函式、零持久化、零 migration、零 LLM。
//!
//! **換骨架（探索／遠遊維度，明確別於近期切片）**：不是 405 規劃預報、不是 406 作物品質、
//! 不是 407 料理熟練、不是 408／410 戰鬥防禦、不是 409 牧群羈絆——而是世界第一個**遠遊探索**
//! 的回報玩法，疊在 398 地名之上把「被動的地名卡」變成「主動的踏足發現」。也明確別於 54 古蹟探勘
//! （接令→指定生態深處採樣完成發獎的任務制）與 353 路標（玩家留言給後人）：本切片是**人人即有、
//! 隨走隨得**的「每到一處新地方」即時發現，不接令、不留物。
//!
//! **平衡（誠實交代，純療癒向、近零經濟擾動）**：初次踏足**只給少量探索者熟練度**（探索者主給
//! 星際旅行費折扣，非可交易資源、非戰力），且**本趟封頂**——只有頭 [`WAYFARE_XP_CAP_PER_SESSION`]
//! 次踏足計 XP，之後只增足跡計數不再給 XP，重登也得重新長途跋涉到沒去過的地方才攢得到，farming
//! 價值極低；不送任何物品／乙太／戰力，不碰戰鬥與經濟核心。locale 邊長 1536px（走過一格約 8~9 秒），
//! 「初次踏足」貨真價實是「你真的走到了一處新地方」。

use std::collections::HashSet;

/// 每次「初次踏足」攢的探索者熟練度 XP。刻意很小——這是療癒向的「謝你願意往外走」，不是成長核心。
/// （`class::XP_PER_LEVEL` = 10，故每次約 1/5 級。）
pub const WAYFARE_XP_PER_DISCOVERY: u32 = 2;

/// 本趟（本次連線）計 XP 的初次踏足上限：頭這麼多次給 XP，之後只增足跡計數不再給 XP。
/// 上限存在是為了徹底壓低「重登farming」誘因——上限內最多 [`WAYFARE_XP_PER_DISCOVERY`]×此值 XP。
pub const WAYFARE_XP_CAP_PER_SESSION: u32 = 6;

/// 足跡集合容量上界：一趟長時間遊玩理論上能走過很多 locale，設一個寬鬆上界護住記憶體
/// （超過即不再新增、足跡計數封頂於此）。1536px／格、180px/s ＝約 8.5 秒/格，4096 處遠超任何一趟。
pub const WAYFARE_VISITED_CAP: usize = 4096;

/// 一次踏入新 locale 的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WayfareOutcome {
    /// 本趟已踏足過這處——無事發生（不重複慶賀、不重複給 XP）。
    Known,
    /// 本趟第一次踏足這處——`tally` ＝至此本趟踏足過的不同地方數；`xp_reward` ＝這次攢的探索者 XP
    /// （可能為 0：本趟 XP 已封頂，仍是貨真價實的初次踏足，只是不再給 XP）。
    FirstFootfall { tally: u32, xp_reward: u32 },
}

/// 一趟遊玩的遠遊足跡（記憶體前置、重啟清空、零持久化）。
///
/// 只記「本趟踏足過哪些 locale id」與「已給過幾次 XP」——皆純記憶體，與 IO／鎖無關，方便單元測試。
/// 與 `dish_mastery::DishMastery`／`guard`／`dodge` 同脈絡的記憶體模式。
#[derive(Debug, Clone, Default)]
pub struct Wayfaring {
    /// 本趟踏足過的 locale id 集合（與 `region_name::Locale::id` 同型）。
    visited: HashSet<i64>,
    /// 本趟已發放 XP 的初次踏足次數（用於封頂，避免farming）。
    xp_grants: u32,
}

impl Wayfaring {
    /// 靜默記下一處 locale 為「已踏足」——不慶賀、不給 XP。
    /// 用於「進場首次定位」：起始所在地算你的起點、不該觸發發現慶賀，但要記下以免日後重回又當新發現。
    pub fn mark_seen(&mut self, locale_id: i64) {
        if self.visited.len() < WAYFARE_VISITED_CAP {
            self.visited.insert(locale_id);
        }
    }

    /// 踏入一處 locale：本趟沒去過即「初次踏足」（攢 XP，封頂後只增足跡計數），去過則 `Known`。
    /// 冪等——同一 locale 第二次起一律 `Known`。
    pub fn discover(&mut self, locale_id: i64) -> WayfareOutcome {
        // 已踏足過＝無事發生。（容量已滿且此 id 不在集合內時也視為 Known，足跡計數封頂、不再長。）
        if self.visited.contains(&locale_id) {
            return WayfareOutcome::Known;
        }
        if self.visited.len() >= WAYFARE_VISITED_CAP {
            return WayfareOutcome::Known;
        }
        self.visited.insert(locale_id);
        // 本趟 XP 未封頂才給，否則仍是初次踏足、只是 xp_reward = 0。
        let xp_reward = if self.xp_grants < WAYFARE_XP_CAP_PER_SESSION {
            self.xp_grants += 1;
            WAYFARE_XP_PER_DISCOVERY
        } else {
            0
        };
        WayfareOutcome::FirstFootfall {
            tally: self.visited.len() as u32,
            xp_reward,
        }
    }

    /// 本趟至今踏足過的不同地方數——隨快照廣播給前端，HUD 畫「遠遊見聞 N 處」足跡計數。
    pub fn tally(&self) -> u32 {
        self.visited.len() as u32
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // 編譯期不變式：容量上界須容得下至少一次封頂量的踏足，否則封頂邏輯永遠摸不到。
    const _: () = assert!(WAYFARE_VISITED_CAP as u32 > WAYFARE_XP_CAP_PER_SESSION);

    #[test]
    fn first_visit_is_a_footfall_repeat_is_known() {
        let mut w = Wayfaring::default();
        match w.discover(7) {
            WayfareOutcome::FirstFootfall { tally, xp_reward } => {
                assert_eq!(tally, 1, "第一次踏足足跡＝1");
                assert_eq!(xp_reward, WAYFARE_XP_PER_DISCOVERY);
            }
            WayfareOutcome::Known => panic!("第一次踏足不該是 Known"),
        }
        // 同一處再踏＝無事發生。
        assert_eq!(w.discover(7), WayfareOutcome::Known);
        assert_eq!(w.tally(), 1, "重複踏足不增足跡");
    }

    #[test]
    fn distinct_places_grow_the_tally() {
        let mut w = Wayfaring::default();
        for (i, id) in [10, 20, 30].into_iter().enumerate() {
            match w.discover(id) {
                WayfareOutcome::FirstFootfall { tally, .. } => {
                    assert_eq!(tally as usize, i + 1, "足跡隨不同地方遞增");
                }
                WayfareOutcome::Known => panic!("各為不同地方、皆應初次踏足"),
            }
        }
        assert_eq!(w.tally(), 3);
    }

    #[test]
    fn mark_seen_records_without_reward() {
        let mut w = Wayfaring::default();
        w.mark_seen(99); // 起點：靜默記下、不給 XP
        assert_eq!(w.tally(), 1, "起點也算一處足跡");
        // 之後重回起點＝Known、不慶賀。
        assert_eq!(w.discover(99), WayfareOutcome::Known);
        // 起點不曾消耗 XP 額度——下一處新地方仍拿得到 XP。
        match w.discover(100) {
            WayfareOutcome::FirstFootfall { xp_reward, .. } => {
                assert_eq!(xp_reward, WAYFARE_XP_PER_DISCOVERY, "起點不該消耗 XP 額度");
            }
            WayfareOutcome::Known => panic!("新地方應初次踏足"),
        }
    }

    #[test]
    fn xp_caps_per_session_but_footfall_still_counts() {
        let mut w = Wayfaring::default();
        // 前 WAYFARE_XP_CAP_PER_SESSION 次給 XP。
        for id in 0..WAYFARE_XP_CAP_PER_SESSION as i64 {
            match w.discover(id) {
                WayfareOutcome::FirstFootfall { xp_reward, .. } => {
                    assert_eq!(xp_reward, WAYFARE_XP_PER_DISCOVERY);
                }
                WayfareOutcome::Known => panic!("額度內應給 XP"),
            }
        }
        // 封頂後仍是初次踏足、足跡照增，但 xp_reward = 0。
        match w.discover(999) {
            WayfareOutcome::FirstFootfall { tally, xp_reward } => {
                assert_eq!(xp_reward, 0, "封頂後不再給 XP");
                assert_eq!(tally, WAYFARE_XP_CAP_PER_SESSION + 1, "足跡仍照增");
            }
            WayfareOutcome::Known => panic!("封頂後仍是初次踏足、不是 Known"),
        }
    }

    #[test]
    fn visited_set_respects_capacity_bound() {
        let mut w = Wayfaring::default();
        // 灌到容量上界。
        for id in 0..WAYFARE_VISITED_CAP as i64 {
            w.mark_seen(id);
        }
        assert_eq!(w.tally() as usize, WAYFARE_VISITED_CAP);
        // 滿了之後新 id 一律 Known、足跡封頂於上界。
        assert_eq!(w.discover(i64::MAX), WayfareOutcome::Known);
        assert_eq!(w.tally() as usize, WAYFARE_VISITED_CAP);
    }
}
