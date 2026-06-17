//! 探索者路標（ROADMAP 353）——玩家在世界裡立一塊路標、留下一句話給後來的人。
//!
//! 這是把「玩家↔玩家」做厚的新一條線：338 表情、339 擊掌都是**同步、同地**的即時互動
//! （兩人得同時在場），本切片第一次做出**非同步、跨時空**的玩家互動——你立的路標在你
//! 離線後仍留在原地，別的玩家路過時讀得到「曾有人來過、留了話」。共享世界第一次有了
//! 別人留下的痕跡感。
//!
//! 設計鐵律：
//! - **零自由文字**：訊息只能從一組預設句子裡挑（wire key 白名單），徹底杜絕 XSS／審查風險，
//!   也天生 i18n 友善（後端只存 wire key、面向玩家的句子由前端對照）。
//! - **記憶體模式、有界、會過期**：路標不持久化（重啟清空）；每塊有 TTL 自動消失；
//!   全服總量與每人持有量都設上限，立第 (上限+1) 塊會頂掉自己最舊的那塊（像「移動立牌」）。
//! - **純社交／導航、零平衡風險**：路標不送任何物品／乙太／戰力，純呈現與留言。
//! - **純函式可測**：放置、過期、容量淘汰全是 `WaypostBoard` 上的純邏輯，與 IO／鎖無關。

use uuid::Uuid;

/// 預設訊息白名單：(wire key, 後端備援中文句)。
/// wire key 是穩定協議契約、不面向玩家；面向玩家的顯示句以前端鏡像為準（i18n 集中在前端），
/// 這裡的中文僅作後端報讀／日誌備援。新增句子兩邊（此處 + 前端 `WAYPOST_MESSAGES`）要同步。
pub const PRESET_MESSAGES: &[(&str, &str)] = &[
    ("hello", "路過打聲招呼～"),
    ("good_view", "此處景色宜人，值得停留"),
    ("watch_out", "小心野怪出沒"),
    ("good_fishing", "這附近魚很多"),
    ("rich_ore", "這裡礦藏不錯"),
    ("rest_here", "在這歇口氣吧"),
    ("this_way", "好東西在這個方向"),
    ("thanks", "謝謝你來到這個世界"),
];

/// 全服同時存在的路標上限（量小、廣播全送，不做 AOI 也無壓力）。
pub const MAX_WAYPOSTS: usize = 24;
/// 每位玩家同時持有的路標上限（立超過就頂掉自己最舊的，鼓勵移動而非洗版）。
pub const MAX_PER_PLAYER: usize = 3;
/// 一塊路標的存在時長（秒）。約 10 分鐘後自然消失。
pub const WAYPOST_TTL_SECS: f32 = 600.0;

/// 判斷一個 wire key 是否為合法預設訊息。
pub fn is_valid_message_key(key: &str) -> bool {
    PRESET_MESSAGES.iter().any(|(k, _)| *k == key)
}

/// 一塊路標。
#[derive(Debug, Clone)]
pub struct Waypost {
    /// 路標唯一識別碼（板上遞增，前端據此去重、做「首次發現」判定）。
    pub id: u64,
    /// 世界座標 X。
    pub x: f32,
    /// 世界座標 Y。
    pub y: f32,
    /// 立牌玩家 id。
    pub owner_id: Uuid,
    /// 立牌玩家顯示名（快照用，不再回查 players）。
    pub owner_name: String,
    /// 訊息 wire key（白名單內）。
    pub message_key: String,
    /// 剩餘存在秒數，降到 0 即消失。
    pub remaining: f32,
}

/// 全服路標板。記憶體、有界、會過期。
#[derive(Debug, Default)]
pub struct WaypostBoard {
    posts: Vec<Waypost>,
    next_id: u64,
}

impl WaypostBoard {
    pub fn new() -> Self {
        Self { posts: Vec::new(), next_id: 1 }
    }

    /// 立一塊新路標。回傳 `Some(新路標 id)` 表示成功；`None` 表示訊息 key 不合法。
    /// 容量規則：先頂掉自己最舊的（若已達 `MAX_PER_PLAYER`），再頂掉全服最舊的（若已達 `MAX_WAYPOSTS`）。
    pub fn place(
        &mut self,
        owner_id: Uuid,
        owner_name: impl Into<String>,
        x: f32,
        y: f32,
        message_key: &str,
    ) -> Option<u64> {
        if !is_valid_message_key(message_key) {
            return None;
        }
        // 同一玩家已達上限 → 頂掉他自己最舊的那塊（Vec 前面的較舊）。
        while self.posts.iter().filter(|p| p.owner_id == owner_id).count() >= MAX_PER_PLAYER {
            if let Some(idx) = self.posts.iter().position(|p| p.owner_id == owner_id) {
                self.posts.remove(idx);
            } else {
                break;
            }
        }
        // 全服已達上限 → 頂掉全服最舊的那塊（位置 0）。
        while self.posts.len() >= MAX_WAYPOSTS {
            self.posts.remove(0);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.posts.push(Waypost {
            id,
            x,
            y,
            owner_id,
            owner_name: owner_name.into(),
            message_key: message_key.to_string(),
            remaining: WAYPOST_TTL_SECS,
        });
        Some(id)
    }

    /// 推進一個 tick：遞減所有路標剩餘時間、移除過期者。
    /// 回傳是否「有任何路標消失」（讓呼叫端決定要不要重新廣播一次路標列表）。
    pub fn tick(&mut self, dt: f32) -> bool {
        let before = self.posts.len();
        for p in &mut self.posts {
            p.remaining -= dt;
        }
        self.posts.retain(|p| p.remaining > 0.0);
        self.posts.len() != before
    }

    /// 目前所有路標（供廣播／前端渲染）。
    pub fn posts(&self) -> &[Waypost] {
        &self.posts
    }

    /// 目前路標數量。
    pub fn len(&self) -> usize {
        self.posts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.posts.is_empty()
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
        assert!(is_valid_message_key("hello"));
        assert!(is_valid_message_key("watch_out"));
        assert!(!is_valid_message_key("definitely_not_a_key"));
        assert!(!is_valid_message_key(""));
    }

    #[test]
    fn place_rejects_unknown_key() {
        let mut b = WaypostBoard::new();
        assert_eq!(b.place(pid(1), "阿光", 10.0, 20.0, "bogus"), None);
        assert!(b.is_empty());
    }

    #[test]
    fn place_accepts_and_assigns_increasing_ids() {
        let mut b = WaypostBoard::new();
        let a = b.place(pid(1), "阿光", 10.0, 20.0, "hello").unwrap();
        let c = b.place(pid(2), "小美", 30.0, 40.0, "good_view").unwrap();
        assert!(c > a, "id 應遞增");
        assert_eq!(b.len(), 2);
        let post = b.posts().iter().find(|p| p.id == a).unwrap();
        assert_eq!(post.owner_name, "阿光");
        assert_eq!(post.message_key, "hello");
        assert!((post.remaining - WAYPOST_TTL_SECS).abs() < 1e-3);
    }

    #[test]
    fn per_player_cap_evicts_own_oldest() {
        let mut b = WaypostBoard::new();
        let p = pid(1);
        let first = b.place(p, "阿光", 0.0, 0.0, "hello").unwrap();
        b.place(p, "阿光", 1.0, 0.0, "good_view").unwrap();
        b.place(p, "阿光", 2.0, 0.0, "watch_out").unwrap();
        // 第 4 塊：頂掉自己最舊的（first），仍維持 MAX_PER_PLAYER 塊。
        b.place(p, "阿光", 3.0, 0.0, "thanks").unwrap();
        assert_eq!(b.posts().iter().filter(|x| x.owner_id == p).count(), MAX_PER_PLAYER);
        assert!(b.posts().iter().all(|x| x.id != first), "自己最舊的應被頂掉");
    }

    #[test]
    fn per_player_cap_does_not_evict_others() {
        let mut b = WaypostBoard::new();
        let me = pid(1);
        let other = b.place(pid(2), "小美", 9.0, 9.0, "hello").unwrap();
        for i in 0..(MAX_PER_PLAYER + 2) {
            b.place(me, "阿光", i as f32, 0.0, "hello").unwrap();
        }
        assert!(b.posts().iter().any(|x| x.id == other), "別人的路標不該被我的淘汰碰到");
        assert_eq!(b.posts().iter().filter(|x| x.owner_id == me).count(), MAX_PER_PLAYER);
    }

    #[test]
    fn global_cap_evicts_oldest() {
        let mut b = WaypostBoard::new();
        // 用許多不同玩家塞滿（避開 per-player 上限干擾）。
        for i in 0..MAX_WAYPOSTS {
            b.place(pid(i as u8), "玩家", i as f32, 0.0, "hello").unwrap();
        }
        assert_eq!(b.len(), MAX_WAYPOSTS);
        let oldest_id = b.posts()[0].id;
        b.place(pid(200), "新人", 0.0, 0.0, "hello").unwrap();
        assert_eq!(b.len(), MAX_WAYPOSTS, "總量維持上限");
        assert!(b.posts().iter().all(|x| x.id != oldest_id), "全服最舊的應被頂掉");
    }

    #[test]
    fn tick_expires_and_reports_change() {
        let mut b = WaypostBoard::new();
        b.place(pid(1), "阿光", 0.0, 0.0, "hello").unwrap();
        // 還沒過期：tick 不應回報變動。
        assert!(!b.tick(1.0));
        assert_eq!(b.len(), 1);
        // 一次推進超過 TTL：過期、回報變動。
        assert!(b.tick(WAYPOST_TTL_SECS));
        assert!(b.is_empty());
        // 空板再 tick：無變動。
        assert!(!b.tick(1.0));
    }

    #[test]
    fn tick_only_expires_the_due_ones() {
        let mut b = WaypostBoard::new();
        let young = b.place(pid(1), "阿光", 0.0, 0.0, "hello").unwrap();
        // 讓第一塊先老一截。
        b.tick(WAYPOST_TTL_SECS - 5.0);
        let fresh = b.place(pid(2), "小美", 1.0, 1.0, "good_view").unwrap();
        // 再推進 10 秒：young 過期、fresh 仍在。
        assert!(b.tick(10.0));
        assert!(b.posts().iter().all(|p| p.id != young));
        assert!(b.posts().iter().any(|p| p.id == fresh));
    }
}
