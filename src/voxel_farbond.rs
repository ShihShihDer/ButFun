//! 乙太方界·兩村相思 v1（voxel-farbond，自主提案切片 ROADMAP 945，
//! PLAN_ETHERVOX §2「記憶→行為」× §3「居民↔居民關係·小社會湧現」× §7「居民散佈世界各處住」的交會）。
//!
//! **真缺口 / 為誰做**：殖民地真居住（942/1210）讓拓荒者第一次真的**搬離主村**、遷去遠方的第二座
//! 村（風禾屯／草浪屯）落地生活；情誼系統（672，`voxel_bonds`）早已在居民之間織出「相識／老朋友」
//! 的關係網。但這兩條線在「搬走之後」從不相認——一位居民搬去殖民地，就算她在主村有一位交情最深的
//! 老朋友，那位老朋友也**從不曾因為她搬遠了而想起她**。搬家至今只在**動身那一刻**由主村上一則
//! 一次性的「主村想念」（`voxel_settle::village_miss_feed_line`，泛泛「大家都惦記著」、不點名、不
//! 往復），此後兩村之間再無任何牽掛的回音。散居把居民的家搬進了遠方的村落，卻沒有人為「隔著兩座
//! 村、仍惦記著那位老朋友」寫一句話。**遷居的代價——把摯友留在了另一座村——至今在世界裡沒有回響。**
//!
//! 本切片把這一環補上、正對 `docs/PLAN_ETHERVOX.md` 核心信念「**記憶要驅動行為，不只聊天**」：
//! **當一位居民與某位老朋友（`BondTier::Friend`）如今住在不同的聚落（一在主村、一在殖民地，或分屬
//! 兩座殖民地），她偶爾會在某個醒著的安靜時刻，望向遠方念叨起那位隔村的摯友**——「不知道搬去
//! 『風禾屯』的諾娃過得好不好……」，這份思念上城鎮動態牆、也記進她與那位朋友的長期記憶。因為兩邊
//! 都是居民、各自獨立地掃到對方，這份相思**天生是雙向的**：留在主村的露娜會念叨搬去風禾屯的諾娃，
//! 搬去風禾屯的諾娃也會在她那頭念叨主村的露娜——無需特別編派方向，小社會的溫度自然在兩座村之間往復
//! 流動。玩家無論身在哪座村，讀動態牆都會看見「兩村相隔、情誼仍在」。記憶不只在重逢時被讀出，更在
//! 分隔兩地的日子裡**驅動居民主動說出了一句惦念**。
//!
//! **與既有系統 razor-sharp 區隔（非同軸換皮）**：
//! - **居民惦記離開的你（`voxel_longing`）**＝居民想念**離線的玩家**（人類），由**你的離線時長**驅動；
//!   本刀＝居民想念**另一位搬到別村的居民**，由**兩人分屬不同聚落 + 交情門檻**驅動。對象一個是人、
//!   一個是居民，觸發條件完全不同。
//! - **主村想念（942，`voxel_settle::village_miss_feed_line`）**＝**搬家那一刻**主村上的**一次性**、
//!   **不點名**泛泛感言（「大家都惦記著」）；本刀＝搬家**之後長日子裡**、**特定一位老朋友**點名念叨
//!   **特定一位隔村摯友**，且**雙向往復**、可反覆發生（有冷卻、稀有而有份量）。一個是離別瞬間的集體
//!   感言、一個是分隔日久的個人相思。
//! - **邊陲探友（821，`voxel_frontier_visit`）**＝留守居民**真的跋涉到荒野**去找**正在遠行**的朋友
//!   （空間移動、對象是暫時遠行者）；本刀＝**零尋路**、居民**原地念叨**想念一位**已永久遷居**別村的
//!   朋友（不移動、對象是定居者）。一個是身體追過去、一個是心裡惦記著。
//! - **送行（902，`voxel_sendoff`）**＝**啟程那一刻**鄰居道珍重；本刀＝**啟程很久以後**仍隔村相思。
//!
//! **成本 / 濫用防護鐵律**：
//! - **純邏輯層**：挑思念對象 / 台詞 / 記憶 / Feed / 冷卻判定皆確定性純函式，**零 LLM、零鎖、零 IO、
//!   零 async、零 migration**，可窮舉單元測試。冷卻鐘 [`FarBondClock`] 為 `VoxelHub` 純記憶體欄位
//!   （比照 `longing_queue` 等世界暫態，重啟歸零）。鎖與副作用全在 `voxel_ws.rs`（短鎖循序即釋、
//!   不巢狀、記憶／Feed 走既有機制，守 prod 死鎖鐵律）。
//! - 台詞只嵌居民**顯示名**與**村落名**（皆既有安全字串，非玩家自由輸入），無注入 / NSFW 風險。
//!   玩家無從主動觸發或洗版（純由伺服器端的聚落歸屬 + 既有情誼門檻驅動；每位居民有冷卻，天然稀疏）。

use std::collections::HashMap;

/// 一位居民要隔多久（秒）才會再念叨一次隔村的摯友——相思是稀有而有份量的，不是每次掃描都發作。
/// 刻意設得偏長（約 1.5 個現實小時），讓「兩村相思」在動態牆上是偶爾一現的溫柔、不洗版。
pub const FARBOND_MIN_INTERVAL_SECS: u64 = 5400;

/// 相思泡泡（台詞）字元上限，與其他社交泡泡台詞一致。
pub const SAY_MAX_CHARS: usize = 50;

/// Feed 事件種類名稱（面向玩家、集中可 i18n）。
pub const FEED_KIND: &str = "兩村相思";

/// 一位「住在某聚落的老朋友」候選：供 [`pick_missed_friend`] 從中挑出最惦念、且與自己不同村的那位。
#[derive(Debug, Clone, PartialEq)]
pub struct FarFriend {
    /// 朋友的顯示名（台詞 / 記憶 / Feed 用）。
    pub name: String,
    /// 朋友如今所屬聚落（主村＝0、殖民地＝seq+1）。
    pub settlement: u64,
    /// 兩人的來往次數（`bonds` 的 visit_count）——用來在多位隔村摯友中挑「最常來往、最惦念」的那位。
    pub visits: u32,
}

/// 從「我的所有 `Friend` 級老朋友」中，挑出**如今住在跟我不同聚落**、且我最惦念（來往次數最多）的
/// 那一位來想念，回傳其參考。
///
/// 規則：只考慮 `settlement != my_settlement`（同村的天天見得到、不算隔村相思）；在隔村者中取
/// `visits` **最多**者；同分取**名字字典序最小**者（穩定、確定性、與呼叫端掃描順序無關）；沒有任何
/// 隔村摯友 → `None`（大家都住同一村，無人可相思）。純函式、確定性、無 IO。
pub fn pick_missed_friend(my_settlement: u64, friends: &[FarFriend]) -> Option<&FarFriend> {
    friends
        .iter()
        .filter(|f| f.settlement != my_settlement)
        .max_by(|a, b| {
            // 先比來往次數（多者勝）；同分則名字字典序小者勝（故用 b.name vs a.name 反向比較）。
            a.visits
                .cmp(&b.visits)
                .then_with(|| b.name.cmp(&a.name))
        })
}

/// 居民望向遠方念叨隔村摯友的相思泡泡（面向玩家字串，留 i18n 空間）：點名朋友＋朋友現居的村落名、
/// 輪替三句不機械、不破泡泡框。`pick` 由呼叫端餵入任意 usize（如時間雜湊），本函式取模輪替。
pub fn farbond_say_line(friend: &str, place: &str, pick: usize) -> String {
    let lines = [
        format!("不知道搬去「{place}」的{friend}過得好不好……"),
        format!("好一陣子沒見到{friend}了，「{place}」離這兒可真遠。"),
        format!("{friend}在「{place}」還好嗎？隔著兩座村，也還是會想起。"),
    ];
    let mut s = lines[pick % lines.len()].clone();
    truncate_chars(&mut s, SAY_MAX_CHARS);
    s
}

/// 昇華成記憶的一句摘要（存進這位居民與那位隔村朋友的長期記憶，點名朋友與村落）。
/// 記下這筆會讓兩人的情誼記憶再厚一分——隔著兩座村的惦念，也是一種來往。
pub fn farbond_memory_summary(friend: &str, place: &str) -> String {
    format!("🏘️{friend}搬去了「{place}」，隔著兩座村，我還是常常想起這位老朋友。")
}

/// Feed 動態播報用的一句話（附在 actor＝念叨者居民名之後，故不重複念叨者名；點名朋友＋村落、
/// 第三人稱，讓在線玩家與日後回訪者都讀得懂「兩村相隔、情誼仍在」）。
pub fn farbond_feed_detail(friend: &str, place: &str) -> String {
    format!("望著遠方，念叨起搬去「{place}」的老朋友{friend}——兩村相隔，也還惦記著這份情誼")
}

/// 每位居民的「上次相思時刻」冷卻鐘：純記憶體、確定性，控制相思稀疏發生（每位居民各自獨立冷卻）。
///
/// **相思成行 v1（947，`voxel_farvisit`）擴充**：除了上次念叨時刻，再累積「已念叨幾次」——
/// 念叨滿 [`crate::voxel_farvisit::EMBARK_AFTER_MISSES`] 次後，下一次相思時刻她就不再念叨、
/// 而是真的動身去看那位隔村的摯友（成行後 [`Self::reset_count`] 歸零，思念重新從頭累積）。
/// 計數以居民為鍵（非以「這對朋友」為鍵）——[`pick_missed_friend`] 是確定性挑選，同一位居民
/// 每次念叨的幾乎都是同一位最惦念的摯友，per-居民計數即近似 per-對計數，不另開一張表。
#[derive(Debug, Default)]
pub struct FarBondClock {
    /// 居民 id → （上次念叨的 unix 秒, 累積念叨次數）。
    last: HashMap<String, (u64, u32)>,
}

impl FarBondClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// 這位居民此刻是否「可以再相思一次」＝從沒念叨過、或距上次已滿 [`FARBOND_MIN_INTERVAL_SECS`]。
    /// 純讀、確定性。`now` 為 unix 秒。
    pub fn due(&self, resident_id: &str, now: u64) -> bool {
        match self.last.get(resident_id) {
            None => true,
            Some(&(last, _)) => now.saturating_sub(last) >= FARBOND_MIN_INTERVAL_SECS,
        }
    }

    /// 記下這位居民在 `now` 念叨了一次（重設其冷卻、念叨計數 +1）。
    pub fn mark(&mut self, resident_id: &str, now: u64) {
        let entry = self.last.entry(resident_id.to_string()).or_insert((0, 0));
        entry.0 = now;
        entry.1 = entry.1.saturating_add(1);
    }

    /// 這位居民已累積念叨了幾次（從沒念叨過＝0）。相思成行（947）用它判斷「思念滿了沒」。
    pub fn miss_count(&self, resident_id: &str) -> u32 {
        self.last.get(resident_id).map(|&(_, n)| n).unwrap_or(0)
    }

    /// 成行後歸零這位居民的念叨計數（保留上次時刻——回來後至少再隔一輪冷卻才會開始新的念叨）。
    pub fn reset_count(&mut self, resident_id: &str) {
        if let Some(entry) = self.last.get_mut(resident_id) {
            entry.1 = 0;
        }
    }

    /// 目前有冷卻記錄的居民數（測試 / 觀測用）。
    pub fn len(&self) -> usize {
        self.last.len()
    }

    pub fn is_empty(&self) -> bool {
        self.last.is_empty()
    }
}

/// 依字元（非位元組）截斷字串到上限，避免切壞多位元組中文。
fn truncate_chars(s: &mut String, max: usize) {
    if s.chars().count() > max {
        let truncated: String = s.chars().take(max).collect();
        *s = truncated;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ff(name: &str, settlement: u64, visits: u32) -> FarFriend {
        FarFriend {
            name: name.to_string(),
            settlement,
            visits,
        }
    }

    #[test]
    fn picks_cross_settlement_friend_only() {
        // 我在主村（0）；同村的朋友不算，只有搬去殖民地（1）的諾娃可相思。
        let friends = vec![ff("賽勒", 0, 9), ff("諾娃", 1, 3)];
        let picked = pick_missed_friend(0, &friends).unwrap();
        assert_eq!(picked.name, "諾娃");
    }

    #[test]
    fn none_when_all_same_settlement() {
        // 大家都住主村 → 無人可隔村相思。
        let friends = vec![ff("賽勒", 0, 9), ff("露娜", 0, 3)];
        assert!(pick_missed_friend(0, &friends).is_none());
        // 一位隔村朋友都沒有的空清單亦然。
        assert!(pick_missed_friend(0, &[]).is_none());
    }

    #[test]
    fn picks_most_visited_among_far_friends() {
        // 兩位都住殖民地（跟我主村不同），取來往最多的那位。
        let friends = vec![ff("諾娃", 1, 2), ff("奧瑞", 2, 7)];
        assert_eq!(pick_missed_friend(0, &friends).unwrap().name, "奧瑞");
    }

    #[test]
    fn tie_breaks_by_name_deterministically() {
        // 來往次數同分 → 取名字字典序最小者（確定性、與清單順序無關）。
        let a = vec![ff("諾娃", 1, 5), ff("奧瑞", 2, 5)];
        let b = vec![ff("奧瑞", 2, 5), ff("諾娃", 1, 5)];
        assert_eq!(pick_missed_friend(0, &a).unwrap().name, "奧瑞"); // "奧" < "諾"
        assert_eq!(pick_missed_friend(0, &b).unwrap().name, "奧瑞");
    }

    #[test]
    fn works_between_two_colonies() {
        // 我住殖民地 1；同在殖民地 1 的不算，只有住主村（0）與殖民地 2 的算隔村。
        let friends = vec![ff("同村人", 1, 9), ff("主村人", 0, 4), ff("他村人", 2, 6)];
        assert_eq!(pick_missed_friend(1, &friends).unwrap().name, "他村人");
    }

    #[test]
    fn say_line_rotates_names_friend_and_place() {
        for pick in 0..6 {
            let s = farbond_say_line("諾娃", "風禾屯", pick);
            assert!(s.contains("諾娃"));
            assert!(s.contains("風禾屯"));
            assert!(s.chars().count() <= SAY_MAX_CHARS);
        }
        // 三句循環：pick 0 與 3 同句、0 與 1 不同句。
        assert_eq!(
            farbond_say_line("諾娃", "風禾屯", 0),
            farbond_say_line("諾娃", "風禾屯", 3)
        );
        assert_ne!(
            farbond_say_line("諾娃", "風禾屯", 0),
            farbond_say_line("諾娃", "風禾屯", 1)
        );
    }

    #[test]
    fn memory_and_feed_name_friend_and_place_nonempty() {
        let mem = farbond_memory_summary("諾娃", "風禾屯");
        assert!(mem.contains("諾娃") && mem.contains("風禾屯") && !mem.is_empty());
        let feed = farbond_feed_detail("諾娃", "風禾屯");
        assert!(feed.contains("諾娃") && feed.contains("風禾屯") && !feed.is_empty());
        // Feed 明細不重複念叨者名（呼叫端會把 actor 名接在前面）——這裡只驗非空且點到朋友。
        assert!(!feed.contains('\n'), "Feed 明細須單行、防注入洗版");
    }

    #[test]
    fn feed_and_say_are_single_line() {
        assert!(!farbond_say_line("諾娃", "風禾屯", 0).contains('\n'));
        assert!(!farbond_feed_detail("諾娃", "風禾屯").contains('\n'));
    }

    #[test]
    fn clock_gates_by_interval() {
        let mut c = FarBondClock::new();
        assert!(c.due("r1", 10_000), "從沒念叨過 → 可以");
        c.mark("r1", 10_000);
        assert!(!c.due("r1", 10_000), "剛念叨完 → 冷卻中");
        assert!(
            !c.due("r1", 10_000 + FARBOND_MIN_INTERVAL_SECS - 1),
            "冷卻未滿 → 不可"
        );
        assert!(
            c.due("r1", 10_000 + FARBOND_MIN_INTERVAL_SECS),
            "冷卻正好滿 → 可以"
        );
        // 別的居民互不影響。
        assert!(c.due("r2", 10_000));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn clock_counts_misses_and_resets_for_embark() {
        // 相思成行（947）：每次 mark 念叨計數 +1；reset_count 歸零但保留冷卻時刻。
        let mut c = FarBondClock::new();
        assert_eq!(c.miss_count("r1"), 0, "從沒念叨過＝0");
        c.mark("r1", 10_000);
        c.mark("r1", 10_000 + FARBOND_MIN_INTERVAL_SECS);
        c.mark("r1", 10_000 + FARBOND_MIN_INTERVAL_SECS * 2);
        assert_eq!(c.miss_count("r1"), 3);
        assert_eq!(c.miss_count("r2"), 0, "別的居民互不影響");
        c.reset_count("r1");
        assert_eq!(c.miss_count("r1"), 0, "成行後思念歸零、重新累積");
        assert!(
            !c.due("r1", 10_000 + FARBOND_MIN_INTERVAL_SECS * 2),
            "歸零不清冷卻時刻——回來後至少再隔一輪冷卻才開始新的念叨"
        );
        // 對從沒念叨過的居民 reset 是安全的 no-op。
        c.reset_count("r9");
        assert_eq!(c.miss_count("r9"), 0);
    }

    #[test]
    fn clock_now_before_last_does_not_underflow() {
        // 防呆：時鐘倒退（now < last）也不 panic、視為冷卻中。
        let mut c = FarBondClock::new();
        c.mark("r1", 10_000);
        assert!(!c.due("r1", 9_000));
    }
}
