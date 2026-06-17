//! 星海寄語 / 漂流瓶（ROADMAP 354）——把一句話封進瓶裡拋向星海，漂到某位素未謀面的旅人腳邊。
//!
//! 這是把「玩家↔玩家」做厚的第二條**非同步**互動線。353 探索者路標是**定點、廣播**
//! （你選好地點立牌、路過的所有人都讀得到「這裡有人來過」）；本切片換一種骨架——
//! **非定點、點對點、可回贈**：你不知道瓶子會漂到誰手裡，只知道某個遠方的陌生人會撈到、
//! 讀到你的話，並且可以回贈你一句。第一次有了「跨服素昧平生的兩人，互道一句溫暖」的連結感
//! （與既有「密語」不同——密語要雙方同時在線且知道對方名字；漂流瓶是非同步、匿名、隨機投遞）。
//!
//! 設計鐵律（與路標一脈相承）：
//! - **零自由文字**：拋瓶與回贈都只能從一組預設句子裡挑（wire key 白名單），徹底杜絕 XSS／審查，
//!   也天生 i18n 友善（後端只存 wire key、面向玩家的句子由前端對照）。
//! - **記憶體模式、有界、會過期**：不持久化（重啟清空）；漂流瓶有 TTL、沒人撈會沉沒；
//!   全服總量、每人漂海上的量、每人回贈信箱量都設上限；回贈也會過期，記憶體永遠有界。
//! - **純社交，零平衡風險**：不送任何物品／乙太／戰力，純呈現與留言。
//! - **純函式可測**：拋／撈／回贈／過期全是 `BottleSea` 上的純邏輯，與 IO／鎖無關，結果確定可重現。

use std::collections::HashMap;
use uuid::Uuid;

/// 預設訊息白名單：(wire key, 後端備援中文句)。
/// wire key 是穩定協議契約、不面向玩家；面向玩家的顯示句以前端鏡像為準（i18n 集中在前端），
/// 這裡的中文僅作後端報讀／日誌備援。拋瓶與回贈共用同一組。新增句子兩邊（此處 + 前端
/// `BOTTLE_MESSAGES`）要同步。
pub const PRESET_MESSAGES: &[(&str, &str)] = &[
    ("hello_stranger", "嗨，陌生的旅人，願你今天順心～"),
    ("not_alone", "在這片星海裡，你並不孤單"),
    ("keep_going", "辛苦了，再撐一下，你做得很好"),
    ("good_luck", "祝你旅途順利、滿載而歸"),
    ("take_a_break", "別太累了，記得歇口氣"),
    ("thanks_for_being_here", "謝謝你也在這個世界裡"),
    ("good_fortune", "我在遠方挖到了好東西，也分你一點好運"),
    ("smile", "對著螢幕笑一個吧 :)"),
];

/// 全服同時漂在海上的瓶子上限（量小、只送數字給前端，不做 AOI 也無壓力）。
pub const MAX_DRIFTING: usize = 64;
/// 每位玩家同時漂在海上的瓶子上限（拋超過就頂掉自己最舊的，防洗版）。
pub const MAX_PER_AUTHOR: usize = 3;
/// 一只漂流瓶的存在時長（秒）。約 30 分鐘沒人撈就沉沒。
pub const BOTTLE_TTL_SECS: f32 = 1800.0;
/// 每位玩家回贈信箱的上限（滿了頂掉最舊的，記憶體有界）。
pub const MAX_INBOX_PER_PLAYER: usize = 20;
/// 回贈在信箱裡的存在時長（秒）。約 1 小時沒領取就清掉（離線太久就錯過，記憶體不無限長）。
pub const INBOX_TTL_SECS: f32 = 3600.0;
/// 撈到瓶後可回贈的時間窗（秒）。超過就視為「沒回贈」，不再能對那只瓶回話。
pub const REPLY_WINDOW_SECS: f32 = 120.0;

/// 判斷一個 wire key 是否為合法預設訊息。
pub fn is_valid_message_key(key: &str) -> bool {
    PRESET_MESSAGES.iter().any(|(k, _)| *k == key)
}

/// 一只漂在海上、還沒被撈走的漂流瓶。
#[derive(Debug, Clone)]
pub struct Bottle {
    /// 瓶子唯一識別碼（海上遞增）。
    pub id: u64,
    /// 拋瓶玩家 id（撈瓶時排除自己拋的；回贈時寄回給他）。
    pub author_id: Uuid,
    /// 拋瓶玩家顯示名（快照用，不再回查 players）。
    pub author_name: String,
    /// 訊息 wire key（白名單內）。
    pub message_key: String,
    /// 剩餘漂流秒數，降到 0 即沉沒。
    pub remaining: f32,
}

/// 一則回贈，投到原作者的信箱裡，等他上線／開面板領取。
#[derive(Debug, Clone)]
pub struct InboxReply {
    /// 回贈唯一識別碼。
    pub id: u64,
    /// 回贈者顯示名。
    pub from_name: String,
    /// 訊息 wire key（白名單內）。
    pub message_key: String,
    /// 剩餘存在秒數，降到 0 即清掉。
    pub remaining: f32,
}

/// 撈到瓶後、尚未回贈前的暫存（鍵＝撈瓶玩家 id）。記下原作者，讓回贈能精準寄回，
/// 且不必信任前端傳來的對象 id（防偽造投遞）。一段時間沒回贈就過期。
#[derive(Debug, Clone)]
struct Pending {
    /// 原作者 id（回贈寄回給他）。
    author_id: Uuid,
    /// 剩餘可回贈秒數。
    remaining: f32,
}

/// 全服星海。記憶體、有界、會過期。
#[derive(Debug)]
pub struct BottleSea {
    /// 漂在海上、待撈的瓶子（FIFO：前面較舊）。
    drifting: Vec<Bottle>,
    /// 各玩家的回贈信箱（author_id → 回贈列表）。
    inboxes: HashMap<Uuid, Vec<InboxReply>>,
    /// 各玩家「剛撈到、待回贈」的暫存（reader_id → Pending）。
    pending: HashMap<Uuid, Pending>,
    /// 遞增 id 來源（瓶子與回贈共用，確保全域唯一遞增）。
    next_id: u64,
}

impl BottleSea {
    pub fn new() -> Self {
        Self {
            drifting: Vec::new(),
            inboxes: HashMap::new(),
            pending: HashMap::new(),
            next_id: 1,
        }
    }

    /// 海上目前漂著的瓶子數（廣播給前端顯示「海上漂著 N 只瓶」）。
    pub fn drifting_count(&self) -> usize {
        self.drifting.len()
    }

    /// 某玩家信箱裡的回贈數。
    pub fn inbox_len(&self, player_id: Uuid) -> usize {
        self.inboxes.get(&player_id).map_or(0, |v| v.len())
    }

    /// 拋一只漂流瓶。回 `Some(新瓶 id)` 成功；`None` 表示訊息 key 不合法。
    /// 容量規則：先頂掉自己最舊的（若已達 `MAX_PER_AUTHOR`），再頂掉全服最舊的（若已達 `MAX_DRIFTING`）。
    pub fn cast(
        &mut self,
        author_id: Uuid,
        author_name: impl Into<String>,
        message_key: &str,
    ) -> Option<u64> {
        if !is_valid_message_key(message_key) {
            return None;
        }
        // 同一玩家漂在海上的瓶子已達上限 → 頂掉他自己最舊的那只（Vec 前面的較舊）。
        while self.drifting.iter().filter(|b| b.author_id == author_id).count() >= MAX_PER_AUTHOR {
            if let Some(idx) = self.drifting.iter().position(|b| b.author_id == author_id) {
                self.drifting.remove(idx);
            } else {
                break;
            }
        }
        // 全服已達上限 → 頂掉全服最舊的那只（位置 0）。
        while self.drifting.len() >= MAX_DRIFTING {
            self.drifting.remove(0);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.drifting.push(Bottle {
            id,
            author_id,
            author_name: author_name.into(),
            message_key: message_key.to_string(),
            remaining: BOTTLE_TTL_SECS,
        });
        Some(id)
    }

    /// 撈一只瓶：回傳海上最舊的、**非自己拋的**那只（撈走＝離開海面）。
    /// 同時把「待回贈」暫存記在 `reader_id` 名下，使接下來的 `reply` 能精準寄回原作者。
    /// 回 `None` 表示海上沒有可撈的瓶（空海，或只剩自己拋的）。
    pub fn draw_for(&mut self, reader_id: Uuid) -> Option<Bottle> {
        let idx = self.drifting.iter().position(|b| b.author_id != reader_id)?;
        let bottle = self.drifting.remove(idx);
        self.pending.insert(
            reader_id,
            Pending { author_id: bottle.author_id, remaining: REPLY_WINDOW_SECS },
        );
        Some(bottle)
    }

    /// 回贈一句給「剛撈到的那只瓶」的作者。回 `Some((原作者 id, 投入信箱的回贈))` 表示成功路由；
    /// `None` 表示沒有可回贈對象（沒撈過／回贈窗已過）或 key 不合法。
    pub fn reply(
        &mut self,
        reader_id: Uuid,
        replier_name: impl Into<String>,
        message_key: &str,
    ) -> Option<(Uuid, InboxReply)> {
        if !is_valid_message_key(message_key) {
            return None;
        }
        let pending = self.pending.remove(&reader_id)?;
        // 雙保險：不回贈給自己（draw_for 已排除自己拋的瓶，這裡再擋一次）。
        if pending.author_id == reader_id {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let reply = InboxReply {
            id,
            from_name: replier_name.into(),
            message_key: message_key.to_string(),
            remaining: INBOX_TTL_SECS,
        };
        let inbox = self.inboxes.entry(pending.author_id).or_default();
        // 信箱上限：滿了頂掉最舊的。
        while inbox.len() >= MAX_INBOX_PER_PLAYER {
            inbox.remove(0);
        }
        inbox.push(reply.clone());
        Some((pending.author_id, reply))
    }

    /// 領取並清空某玩家的回贈信箱（連線時直送、或回贈寄達在線玩家時即送）。
    /// 回傳該玩家當下所有回贈（依投入順序）。空信箱回空 Vec。
    pub fn take_inbox(&mut self, player_id: Uuid) -> Vec<InboxReply> {
        self.inboxes.remove(&player_id).unwrap_or_default()
    }

    /// 推進一個 tick：漂流瓶 / 回贈信箱 / 待回贈暫存全部遞減、移除過期者。
    /// 回傳「漂在海上的瓶子數量是否變動」（讓呼叫端決定要不要重新廣播海上數量）。
    pub fn tick(&mut self, dt: f32) -> bool {
        let before = self.drifting.len();
        for b in &mut self.drifting {
            b.remaining -= dt;
        }
        self.drifting.retain(|b| b.remaining > 0.0);
        // 回贈過期清理。
        for inbox in self.inboxes.values_mut() {
            for r in inbox.iter_mut() {
                r.remaining -= dt;
            }
            inbox.retain(|r| r.remaining > 0.0);
        }
        self.inboxes.retain(|_, v| !v.is_empty());
        // 待回贈暫存過期清理。
        for p in self.pending.values_mut() {
            p.remaining -= dt;
        }
        self.pending.retain(|_, p| p.remaining > 0.0);
        self.drifting.len() != before
    }
}

impl Default for BottleSea {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn valid_keys_only() {
        assert!(is_valid_message_key("hello_stranger"));
        assert!(is_valid_message_key("smile"));
        assert!(!is_valid_message_key("definitely_not_a_key"));
        assert!(!is_valid_message_key(""));
    }

    #[test]
    fn cast_rejects_unknown_key() {
        let mut sea = BottleSea::new();
        assert_eq!(sea.cast(pid(1), "阿光", "bogus"), None);
        assert_eq!(sea.drifting_count(), 0);
    }

    #[test]
    fn cast_accepts_and_assigns_increasing_ids() {
        let mut sea = BottleSea::new();
        let a = sea.cast(pid(1), "阿光", "hello_stranger").unwrap();
        let b = sea.cast(pid(2), "小美", "smile").unwrap();
        assert!(b > a, "id 應遞增");
        assert_eq!(sea.drifting_count(), 2);
    }

    #[test]
    fn per_author_cap_evicts_own_oldest() {
        let mut sea = BottleSea::new();
        let p = pid(1);
        let first = sea.cast(p, "阿光", "hello_stranger").unwrap();
        sea.cast(p, "阿光", "smile").unwrap();
        sea.cast(p, "阿光", "good_luck").unwrap();
        // 第 4 只：頂掉自己最舊的（first），仍維持 MAX_PER_AUTHOR 只。
        sea.cast(p, "阿光", "keep_going").unwrap();
        assert_eq!(sea.drifting.iter().filter(|b| b.author_id == p).count(), MAX_PER_AUTHOR);
        assert!(sea.drifting.iter().all(|b| b.id != first), "自己最舊的應被頂掉");
    }

    #[test]
    fn global_cap_evicts_oldest() {
        let mut sea = BottleSea::new();
        for i in 0..MAX_DRIFTING {
            sea.cast(pid(i as u8), "玩家", "hello_stranger").unwrap();
        }
        assert_eq!(sea.drifting_count(), MAX_DRIFTING);
        let oldest = sea.drifting[0].id;
        sea.cast(pid(200), "新人", "smile").unwrap();
        assert_eq!(sea.drifting_count(), MAX_DRIFTING, "總量維持上限");
        assert!(sea.drifting.iter().all(|b| b.id != oldest), "全服最舊的應被頂掉");
    }

    #[test]
    fn draw_skips_own_bottle() {
        let mut sea = BottleSea::new();
        let me = pid(1);
        sea.cast(me, "阿光", "hello_stranger").unwrap();
        // 海上只有自己拋的瓶 → 撈不到。
        assert!(sea.draw_for(me).is_none());
        // 別人拋一只 → 我撈得到，且撈到的是別人的。
        sea.cast(pid(2), "小美", "smile").unwrap();
        let got = sea.draw_for(me).unwrap();
        assert_eq!(got.author_name, "小美");
        // 撈走後海上只剩我自己那只。
        assert_eq!(sea.drifting_count(), 1);
    }

    #[test]
    fn draw_returns_oldest_first() {
        let mut sea = BottleSea::new();
        let me = pid(9);
        let oldest = sea.cast(pid(1), "甲", "hello_stranger").unwrap();
        sea.cast(pid(2), "乙", "smile").unwrap();
        let got = sea.draw_for(me).unwrap();
        assert_eq!(got.id, oldest, "應先撈到最舊的瓶");
    }

    #[test]
    fn draw_on_empty_sea_is_none() {
        let mut sea = BottleSea::new();
        assert!(sea.draw_for(pid(1)).is_none());
    }

    #[test]
    fn reply_routes_to_author_inbox() {
        let mut sea = BottleSea::new();
        let author = pid(1);
        let reader = pid(2);
        sea.cast(author, "阿光", "hello_stranger").unwrap();
        sea.draw_for(reader).unwrap();
        let (to, reply) = sea.reply(reader, "小美", "smile").unwrap();
        assert_eq!(to, author, "回贈應寄回原作者");
        assert_eq!(reply.from_name, "小美");
        assert_eq!(sea.inbox_len(author), 1);
        // 領取後信箱清空。
        let drained = sea.take_inbox(author);
        assert_eq!(drained.len(), 1);
        assert_eq!(sea.inbox_len(author), 0);
    }

    #[test]
    fn reply_without_drawing_fails() {
        let mut sea = BottleSea::new();
        // 沒撈過任何瓶 → 無從回贈。
        assert!(sea.reply(pid(2), "小美", "smile").is_none());
    }

    #[test]
    fn reply_rejects_unknown_key() {
        let mut sea = BottleSea::new();
        sea.cast(pid(1), "阿光", "hello_stranger").unwrap();
        sea.draw_for(pid(2)).unwrap();
        assert!(sea.reply(pid(2), "小美", "bogus").is_none());
        // 失敗的回贈不該耗掉 pending 以外的東西，也沒投進任何信箱。
        assert_eq!(sea.inbox_len(pid(1)), 0);
    }

    #[test]
    fn reply_only_once_per_draw() {
        let mut sea = BottleSea::new();
        sea.cast(pid(1), "阿光", "hello_stranger").unwrap();
        sea.draw_for(pid(2)).unwrap();
        assert!(sea.reply(pid(2), "小美", "smile").is_some());
        // 同一次撈瓶只能回贈一次（pending 已被取走）。
        assert!(sea.reply(pid(2), "小美", "good_luck").is_none());
        assert_eq!(sea.inbox_len(pid(1)), 1);
    }

    #[test]
    fn reply_window_expires() {
        let mut sea = BottleSea::new();
        sea.cast(pid(1), "阿光", "hello_stranger").unwrap();
        sea.draw_for(pid(2)).unwrap();
        // 撈到後拖過回贈窗 → 不能再回贈。
        sea.tick(REPLY_WINDOW_SECS + 1.0);
        assert!(sea.reply(pid(2), "小美", "smile").is_none());
    }

    #[test]
    fn inbox_cap_evicts_oldest() {
        let mut sea = BottleSea::new();
        let author = pid(1);
        // 塞滿信箱再多塞一封：每封都重新撈一只新瓶再回贈。
        for i in 0..(MAX_INBOX_PER_PLAYER + 3) {
            sea.cast(author, "阿光", "hello_stranger").unwrap();
            let reader = pid(100 + i as u8);
            sea.draw_for(reader).unwrap();
            sea.reply(reader, "回信人", "smile").unwrap();
        }
        assert_eq!(sea.inbox_len(author), MAX_INBOX_PER_PLAYER, "信箱維持上限");
    }

    #[test]
    fn bottle_ttl_sinks_and_reports_change() {
        let mut sea = BottleSea::new();
        sea.cast(pid(1), "阿光", "hello_stranger").unwrap();
        // 還沒沉沒：tick 不回報變動。
        assert!(!sea.tick(1.0));
        assert_eq!(sea.drifting_count(), 1);
        // 一次推進超過 TTL：沉沒、回報變動。
        assert!(sea.tick(BOTTLE_TTL_SECS));
        assert_eq!(sea.drifting_count(), 0);
        // 空海再 tick：無變動。
        assert!(!sea.tick(1.0));
    }

    #[test]
    fn inbox_ttl_expires() {
        let mut sea = BottleSea::new();
        let author = pid(1);
        sea.cast(author, "阿光", "hello_stranger").unwrap();
        sea.draw_for(pid(2)).unwrap();
        sea.reply(pid(2), "小美", "smile").unwrap();
        assert_eq!(sea.inbox_len(author), 1);
        // 回贈擱太久沒領 → 過期清掉。
        sea.tick(INBOX_TTL_SECS + 1.0);
        assert_eq!(sea.inbox_len(author), 0);
    }
}
