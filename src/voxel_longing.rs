//! 乙太方界·居民惦記離開的你 v1（voxel-longing，自主提案切片）。
//!
//! **真缺口 / 為誰做**：記憶→行為這條線一路做到了「你久別歸來那一刻」——久別重逢摘要（721，
//! `voxel_welcome`）在你回來時告訴你「世界發生了什麼」，久別重逢奔迎（747，`voxel_reunion`）
//! 讓最惦記你的居民在你回來那刻放下手邊的事奔來迎你。但這兩刀都**綁在你回來的那一瞬間**：只要
//! 你人不在，居民對你的記憶再厚，也從不曾因為你「離開了」而做出任何反應——世界對你的思念，永遠
//! 要等你回來才被說出口。你不在的那段時間裡，那位跟你交情最深的居民，其實一句話都沒為你說過。
//!
//! 本切片把這一環補上、正對著 `docs/PLAN_ETHERVOX.md` 核心信念「**記憶要驅動行為，不只聊天**」的
//! 另一面：**當你離線夠久，對你記憶最厚的那位居民會在某個醒著的時刻，主動想念起你**——望著你常來
//! 的方向念叨一句「好一陣子沒看到你了，真有點想念呢……」，這份想念上城鎮動態牆、也記進她與你的
//! 記憶。於是——①此刻在線的**別的**玩家，讀動態牆會看見這個小村對缺席的旅人仍有牽掛（小社會的
//! 溫度不靠你在場才亮）；②等**你**回來，久別重逢摘要裡就多了一行「露娜在你不在時，好幾次提起想
//! 念你」——你的離開，第一次也在這個世界裡留下了回響。記憶不只在你回來時被讀出，更在你缺席時**驅動
//! 居民主動做了一件事**。
//!
//! **與既有系統 razor-sharp 區隔（非同軸換皮）**：
//! - **奔迎（747）**＝觸發在「你回來那一刻」、居民**朝你走動**迎接；本刀＝觸發在「你離開夠久、仍
//!   未歸來」時、居民**獨自念叨**想你（零尋路、你根本不在場也照樣發生）。方向正好相反：一個迎歸、
//!   一個送別後的思念。
//! - **久別重逢摘要（721）**＝你回來時把「世界發生的既有事件」彙整給你看；本刀是**製造**一件全新的
//!   事件（居民的主動想念），它本身之後才會被 721 的摘要讀到。前者是讀，後者是寫。
//! - **晨間思念 / 夢（746）**＝居民在**天亮那一刻**因夢到你而念你（天象驅動、每天清晨）；本刀由**你
//!   的離線時長**驅動（跟一天中的時辰無關），一次離線只念一回（想念是稀有而有份量的，不是每天清晨
//!   的例行公事）。
//!
//! **成本 / 濫用防護鐵律**：
//! - **純邏輯層**：入列 / 取消 / 到期判定 / 想念台詞 / 記憶 / Feed 皆為確定性純函式，**零 LLM、零鎖、
//!   零 IO、零 async、零 migration**，可窮舉單元測試。佇列為 `VoxelHub` 純記憶體欄位（比照
//!   `last_seen` / `reunion_seek` 等世界暫態，重啟歸零）。鎖與副作用全在 `voxel_ws.rs`（短鎖循序
//!   即釋、不巢狀、記憶／Feed 走既有機制，守 prod 死鎖鐵律）。
//! - 台詞永不回放玩家原話——只嵌玩家**顯示名**（既有安全字串），無注入 / NSFW 風險。玩家無從主動
//!   觸發或洗版（純由伺服器端的離線時長 + 既有好感度門檻驅動；每位離開的玩家最多排一則待念，一念
//!   即清，天然有界）。

use std::collections::HashMap;

/// 玩家離線多久（秒）以上，居民才開始想念——太短的斷線重連不算「離開」，避免疲勞轟炸。
/// 刻意比奔迎的久別門檻（`voxel_reunion::REUNION_MIN_GAP_SECS` = 1800）短一截：想念發生在
/// 「你走了一陣子還沒回來」的當下，回來後若間隔更久，奔迎再各自獨立觸發，兩者互補不打架。
pub const LONGING_DELAY_SECS: u64 = 1200;

/// 居民要對你「記憶夠厚」（長期記憶筆數 ≥ 此值）才會想念你——沒交情的過客離開，居民不會沒來由
/// 惦記。與奔迎同門檻（3），語義一致：夠熟才會迎、也夠熟才會念。
pub const LONGING_AFFINITY: usize = 3;

/// 到期後多久（秒）內沒能把想念說出口（例如那位居民這段時間一直在睡），就默默作罷、清掉這則待念，
/// 不讓一則過時的想念一直卡在佇列裡等到天長地久。
pub const LONGING_EXPIRE_SECS: u64 = 3600;

/// Feed 事件種類名稱（面向玩家、集中可 i18n）。已登記進 `voxel_welcome` 的久別重逢摘要白名單，
/// 讓被想念的玩家回來時讀得到。
pub const FEED_KIND: &str = "居民想念";

/// 想念台詞（泡泡）字元上限，與其他社交泡泡台詞一致。
pub const SAY_MAX_CHARS: usize = 50;

/// 一則「待送出的想念」：某位居民惦記著某位已離線玩家。
#[derive(Debug, Clone, PartialEq)]
pub struct LongingEntry {
    /// 想念者（居民 id）。
    pub resident_id: String,
    /// 想念者顯示名（Feed / 台詞用）。
    pub resident_name: String,
    /// 被想念的玩家顯示名。
    pub player_name: String,
    /// 幾秒後（unix 秒）到期、可以開始想念（＝離開時刻 + `LONGING_DELAY_SECS`）。
    pub due_at: u64,
    /// 逾此時刻（unix 秒）還沒說出口就作廢（＝`due_at` + `LONGING_EXPIRE_SECS`）。
    pub expire_at: u64,
}

/// 待送出的想念佇列：key＝被想念的玩家顯示名，每位離開的玩家最多排一則（重複離開會刷新覆蓋）。
#[derive(Debug, Default)]
pub struct LongingQueue {
    entries: HashMap<String, LongingEntry>,
}

impl LongingQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 玩家離線時登記一則待念：由對他記憶最厚的居民在 `LONGING_DELAY_SECS` 秒後想念。
    /// 空玩家名 / 空居民（訪客或匿名）不入列，避免無主的想念。同一玩家再次離開會覆蓋刷新。
    pub fn enqueue(
        &mut self,
        player_name: &str,
        resident_id: &str,
        resident_name: &str,
        now: u64,
    ) {
        if player_name.is_empty() || resident_id.is_empty() {
            return;
        }
        let due_at = now.saturating_add(LONGING_DELAY_SECS);
        self.entries.insert(
            player_name.to_string(),
            LongingEntry {
                resident_id: resident_id.to_string(),
                resident_name: resident_name.to_string(),
                player_name: player_name.to_string(),
                due_at,
                expire_at: due_at.saturating_add(LONGING_EXPIRE_SECS),
            },
        );
    }

    /// 玩家回來了（或已被想念過）：清掉這則待念。回傳是否真的移除了東西。
    pub fn cancel(&mut self, player_name: &str) -> bool {
        self.entries.remove(player_name).is_some()
    }

    /// 清掉所有已逾期（`now` > `expire_at`）還沒說出口的待念，回傳清掉幾則。
    pub fn prune_expired(&mut self, now: u64) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, e| now <= e.expire_at);
        before - self.entries.len()
    }

    /// 取出此刻「該想念了」（`due_at` ≤ `now` ≤ `expire_at`）的所有待念快照（複製，不移除）。
    /// 呼叫端據此逐則檢查「玩家仍離線、居民此刻醒著」後才真的說出口，說完再 [`cancel`](Self::cancel)。
    /// 順序不保證（HashMap），但每則獨立、彼此無關，故無妨。
    pub fn actionable(&self, now: u64) -> Vec<LongingEntry> {
        self.entries
            .values()
            .filter(|e| e.due_at <= now && now <= e.expire_at)
            .cloned()
            .collect()
    }

    /// 目前排隊中的待念數（測試 / 觀測用）。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 從各居民對這位離開玩家的好感度（長期記憶筆數，索引需與居民清單對齊）中，挑出**最惦記他**、
/// 且達 [`LONGING_AFFINITY`] 門檻的那位來想念，回傳其索引。
///
/// 規則同奔迎的挑人：取好感度**最高**者；同分取**索引最小**者（穩定、確定性）；最高者仍未達門檻
/// → `None`（沒人跟你熟到會惦記你，離開就離開了）。與奔迎不同的是——這裡**不必**把睡著的居民填 0：
/// 想念是登記一則稍後才送的待念，登記當下誰在睡無所謂，真正「說出口」時才在呼叫端確認醒著。
/// 純函式、確定性、無 IO。
pub fn most_bonded(affinities: &[usize]) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None; // (好感度, 索引)
    for (i, &aff) in affinities.iter().enumerate() {
        match best {
            Some((ba, _)) if aff <= ba => {} // 嚴格大於才更新 → 同分保留較小索引
            _ => best = Some((aff, i)),
        }
    }
    match best {
        Some((aff, i)) if aff >= LONGING_AFFINITY => Some(i),
        _ => None,
    }
}

/// 居民獨自念叨的想念泡泡（面向玩家字串，留 i18n 空間）：點名玩家、輪替三句不機械、不破泡泡框。
/// `pick` 由呼叫端餵入任意 usize（如座標 bits／時間雜湊），本函式取模輪替。
pub fn longing_say_line(player: &str, pick: usize) -> String {
    let lines = [
        format!("{player}好一陣子沒來了，真有點想念呢……"),
        format!("不知道{player}最近去哪了，這幾天總會想起。"),
        format!("好久沒看到{player}了，希望他一切都好。"),
    ];
    let mut s = lines[pick % lines.len()].clone();
    truncate_chars(&mut s, SAY_MAX_CHARS);
    s
}

/// 昇華成記憶的一句摘要（存進這位居民與該玩家的長期記憶，點名玩家）。
/// 記下這筆會讓好感度（＝記憶筆數）再厚一分——你的缺席也讓她更惦記你，記憶→行為→更深的記憶。
pub fn longing_memory_summary(player: &str) -> String {
    format!("🕊️{player}好久沒來了，我心裡一直惦記著他。")
}

/// Feed 動態播報用的一句話（附在 actor＝居民名之後，故不重複居民名；點名玩家、第三人稱，
/// 讓在線的別人與回來的本人都讀得懂）。搭配 `voxel_welcome` 白名單，回來的玩家也讀得到。
pub fn longing_feed_detail(player: &str) -> String {
    format!("望著{player}常來的方向，念叨著好一陣子沒見到人了，語氣裡有些想念")
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

    fn q_with(now: u64) -> LongingQueue {
        let mut q = LongingQueue::new();
        q.enqueue("小明", "r1", "露娜", now);
        q
    }

    #[test]
    fn enqueue_sets_due_and_expire() {
        let q = q_with(1000);
        // due_at=2200、expire_at=5800，取一個落在窗內的 now 讀回這則待念。
        let acted = q.actionable(3000);
        let e = &acted[0];
        assert_eq!(e.player_name, "小明");
        assert_eq!(e.resident_id, "r1");
        assert_eq!(e.resident_name, "露娜");
        assert_eq!(e.due_at, 1000 + LONGING_DELAY_SECS);
        assert_eq!(e.expire_at, 1000 + LONGING_DELAY_SECS + LONGING_EXPIRE_SECS);
    }

    #[test]
    fn empty_player_or_resident_not_enqueued() {
        let mut q = LongingQueue::new();
        q.enqueue("", "r1", "露娜", 1000);
        q.enqueue("小明", "", "露娜", 1000);
        assert!(q.is_empty());
    }

    #[test]
    fn reenqueue_overwrites_same_player() {
        let mut q = q_with(1000);
        q.enqueue("小明", "r2", "諾娃", 5000); // 同玩家再次離開 → 覆蓋（due=6200、expire=9800）
        assert_eq!(q.len(), 1);
        let acted = q.actionable(7000);
        let e = &acted[0];
        assert_eq!(e.resident_id, "r2");
        assert_eq!(e.due_at, 5000 + LONGING_DELAY_SECS);
    }

    #[test]
    fn not_actionable_before_due() {
        let q = q_with(1000);
        // due_at = 1000+1200 = 2200；剛離開時（now=1000）與到期前一秒都不該行動。
        assert!(q.actionable(1000).is_empty());
        assert!(q.actionable(2199).is_empty());
        assert_eq!(q.actionable(2200).len(), 1); // 正好到期
    }

    #[test]
    fn not_actionable_after_expire() {
        let q = q_with(1000);
        // expire_at = 2200 + 3600 = 5800。
        assert_eq!(q.actionable(5800).len(), 1); // 正好逾期邊界仍算數
        assert!(q.actionable(5801).is_empty()); // 過了就不再行動
    }

    #[test]
    fn cancel_removes_entry() {
        let mut q = q_with(1000);
        assert!(q.cancel("小明"));
        assert!(q.is_empty());
        assert!(!q.cancel("小明")); // 再取消 no-op
    }

    #[test]
    fn prune_expired_drops_only_overdue() {
        let mut q = LongingQueue::new();
        q.enqueue("甲", "r1", "露娜", 1000); // expire_at = 5800
        q.enqueue("乙", "r2", "諾娃", 4000); // due=5200, expire=8800
        assert_eq!(q.prune_expired(6000), 1); // 甲逾期、乙未逾期
        assert_eq!(q.len(), 1);
        assert!(q.actionable(7000).iter().any(|e| e.player_name == "乙"));
    }

    #[test]
    fn most_bonded_picks_highest_above_threshold() {
        assert_eq!(most_bonded(&[1, 5, 3]), Some(1)); // 最高 5
        assert_eq!(most_bonded(&[LONGING_AFFINITY]), Some(0)); // 正好達門檻
        assert_eq!(most_bonded(&[LONGING_AFFINITY - 1, LONGING_AFFINITY - 1]), None); // 都不到
        assert_eq!(most_bonded(&[]), None);
    }

    #[test]
    fn most_bonded_ties_take_smallest_index() {
        assert_eq!(most_bonded(&[4, 4, 4]), Some(0));
    }

    #[test]
    fn say_line_rotates_and_names_player() {
        for pick in 0..6 {
            let s = longing_say_line("小明", pick);
            assert!(s.contains("小明"));
            assert!(s.chars().count() <= SAY_MAX_CHARS);
        }
        // 三句循環：pick 0 與 3 同句、0 與 1 不同句。
        assert_eq!(longing_say_line("小明", 0), longing_say_line("小明", 3));
        assert_ne!(longing_say_line("小明", 0), longing_say_line("小明", 1));
    }

    #[test]
    fn memory_and_feed_name_player_nonempty() {
        assert!(longing_memory_summary("小明").contains("小明"));
        assert!(!longing_memory_summary("小明").is_empty());
        assert!(longing_feed_detail("小明").contains("小明"));
        assert!(!longing_feed_detail("小明").is_empty());
    }
}
