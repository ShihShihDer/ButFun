//! 乙太方界·居民以物易物 v1——玩家向居民請求交易，居民提出以物換物條件（ROADMAP 670）。
//!
//! 純邏輯（交易提案生成、居民台詞、記憶摘要），無 WS / 鎖 / IO 細節。
//! 由 `voxel_ws.rs` 包進鎖後呼叫；確定性、可測、零 LLM。
//!
//! **交易流程**：
//! 1. 玩家點「⇌ 交易」→ TradeRequest → 伺服器生成提案 → 回 `trade_offer`。
//! 2. 玩家在 30 秒內點「接受」→ TradeAccept → 伺服器執行扣/給背包 → 回 `trade_done`。
//!    v2（自主提案切片，ROADMAP 874）起，扣的可以是提案要的原礦，**也可以直接付乙太幣**
//!    （`coin_price`，見下）——付幣時改扣 `voxel_craft::COIN_ID`，其餘流程不變。
//!    v3（自主提案切片，ROADMAP 958）起，`coin_price` 會依**最近付幣買走這項物品的次數**
//!    自動漲價（見 [`CoinDemandTracker`]）——874 當初刻意留白「之後要分級再擴充」，本刀補上。
//!
//! **各居民特長**（依 resident_id 字元和 % 4 決定，永遠確定性）：
//! - slot 0 → 種子 ↔ 木頭（農業，對應露娜）
//! - slot 1 → 石頭 ↔ 木頭（建築，對應諾娃）
//! - slot 2 → 木頭 ↔ 沙子（探索，對應賽勒）
//! - slot 3 → 玻璃 ↔ 石頭（煉製，對應奧瑞）
//!
//! **好感度影響比例**：
//! - 0（陌生）：玩家給 2 得 1（略不划算，反映居民不信任）
//! - 1–2（相識）：1:1 公平
//! - 3+（友人）：玩家給 1 得 2（划算，居民優待朋友）

/// 交易觸及範圍（方塊距離，水平 XZ 平面，與 GIFT_REACH 一致）。
pub const TRADE_REACH: f32 = 5.0;

/// 交易提案有效時間（秒）：玩家 30 秒內未接受則伺服器自動作廢。
pub const TRADE_OFFER_TTL: u64 = 30;

/// 付幣代替湊材料 v1（ROADMAP 874）的匯率：每 1 單位 `want_count` 折合這麼多枚乙太幣。
/// 基礎價一律同價（沙子/木頭/石頭/種子/玻璃），實際成交價再疊上 [`CoinDemandTracker`]
/// 依最近搶購熱度算出的漲價階（供需 v1，ROADMAP 958）。
pub const COIN_PRICE_PER_UNIT: u32 = 2;

// ── 供需驅動漲價 v1（自主提案切片，ROADMAP 958）───────────────────────────────
//
// **真缺口**：873/874 讓乙太幣成為玩家↔居民的通用貨幣，但下 `coin_price` 的匯率永遠死板
// （`want_count × COIN_PRICE_PER_UNIT`），程式碼自己當初就誠實留白「之後要分級再擴充」——
// 世界裡從沒有一種「大家最近搶著付幣買什麼，這樣東西就會漲價」的供需感，跟真正的市集毫無關係。
// 本刀補上：伺服器記住「最近」用幣買走每種物品的次數，買得越勤，下次開價越貴；一段時間沒人
// 搶購，價格會自己回落——供給與需求第一次在乙太方界裡自己拉扯。
//
// **換維度（非社交小反應重複）**：956/957 是「居民的個人特質→社交反應」；本刀是「玩家的消費
// 行為→市場價格」，完全不同的因果鏈（玩家→經濟 vs 居民→居民），也不碰 `voxel_craft_admire.rs`
// / `voxel_hobby.rs` 任何一行。

/// 每累積這麼多次「最近付幣買走同一種物品」，coin_price 就往上疊一階（漲一份 [`COIN_PRICE_PER_UNIT`]）。
pub const DEMAND_STEP: u32 = 3;

/// 漲價階數封頂（避免搶購到荒謬天價；封頂後基礎價最多變成 1+此值 倍）。
pub const MAX_DEMAND_TIER: u32 = 4;

/// `decay_all` 每隔這麼多個 [`spawn_farm_tick`](crate::voxel_ws::spawn_farm_tick) 的 15 秒節拍
/// 才真正退燒一次（= 2 分鐘）。ROADMAP 958 review 撞見的落地雷：一開始掛在每個 15 秒 tick 上，
/// 半衰期只有 15 秒，玩家得在同一個 tick 內狂買 6 次才攢得到 tier 2、45 秒內又蒸發，世界感受
/// 不會發生——改成分鐘級退燒，讓「連買幾次、漲價撐一陣子」這件事在正常遊玩節奏下真的看得到。
pub const DEMAND_DECAY_EVERY_N_TICKS: u32 = 8;

/// 全域「最近付幣買走各物品的次數」追蹤（供需 v1，ROADMAP 958）。
///
/// 純記憶體（比照 `pending_trades`/`voxel_stall` 世界暫態慣例，重啟即清空、零 migration）；
/// 每次成功「付幣」成交 `record_purchase` 一次，伺服器背景每 [`DEMAND_DECAY_EVERY_N_TICKS`]
/// 個 15 秒節拍 tick `decay_all` 一次讓熱度慢慢降溫——搶購退燒後價格自己回落，供需雙向都能感受到。
#[derive(Debug, Default, Clone)]
pub struct CoinDemandTracker {
    /// 物品 id → 最近熱度計數。只收付幣成交，不含以物易物（那條路完全不動幣）。
    recent_buys: std::collections::HashMap<u8, u32>,
}

impl CoinDemandTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 記一次「剛用幣買走這項物品」（熱度 +1）。
    pub fn record_purchase(&mut self, item_id: u8) {
        let c = self.recent_buys.entry(item_id).or_insert(0);
        *c = c.saturating_add(1);
    }

    /// 熱度隨時間退燒：每個物品的計數減半（無條件捨去），歸零的項目移除，避免無限膨脹。
    pub fn decay_all(&mut self) {
        self.recent_buys.retain(|_, c| {
            *c /= 2;
            *c > 0
        });
    }

    /// 這項物品目前漲價幾階（0 = 沒特別搶購，維持基礎價）。
    pub fn tier_for(&self, item_id: u8) -> u32 {
        let c = self.recent_buys.get(&item_id).copied().unwrap_or(0);
        (c / DEMAND_STEP).min(MAX_DEMAND_TIER)
    }
}

/// 居民的交易提案：居民提供的物品 ↔ 玩家需要提供的物品。
#[derive(Clone, Debug)]
pub struct TradeOffer {
    /// 居民提供的物品 id。
    pub offer_item: u8,
    /// 居民提供的數量。
    pub offer_count: u32,
    /// 居民想要的物品 id（玩家需給出）。
    pub want_item: u8,
    /// 居民想要的數量（玩家需給出）。
    pub want_count: u32,
    /// 付幣代替湊材料 v1（ROADMAP 874）：玩家也可以不湊 `want_item`，改直接付這麼多枚
    /// 乙太幣（`voxel_craft::COIN_ID`）成交——省得為了一單交易特地去採礦。
    pub coin_price: u32,
}

/// 依 resident_id 決定交易特長 slot（0..4，永遠確定性）。
pub fn resident_trade_slot(resident_id: &str) -> usize {
    let sum: u64 = resident_id.bytes().map(|b| b as u64).sum();
    (sum % 4) as usize
}

/// 根據居民 ID 與玩家好感度生成交易提案（確定性純函式）。
///
/// `demand`：目前的供需追蹤（見 [`CoinDemandTracker`]）——`offer_item`（玩家掏幣要買的東西）
/// 最近被搶購越多次，`coin_price` 就疊得越貴；呼叫端只需短鎖 clone 一份快照傳入，
/// 本函式仍是零鎖、零 IO 的確定性純函式。
pub fn make_offer(resident_id: &str, affinity: usize, demand: &CoinDemandTracker) -> TradeOffer {
    let slot = resident_trade_slot(resident_id);
    // (offer_item, want_item)：居民提供 / 玩家給出
    let (offer_item, want_item): (u8, u8) = match slot {
        0 => (14, 5),  // 種子 ↔ 木頭
        1 => (3, 5),   // 石頭 ↔ 木頭
        2 => (5, 4),   // 木頭 ↔ 沙子
        _ => (10, 3),  // 玻璃 ↔ 石頭
    };
    let (offer_count, want_count): (u32, u32) = if affinity == 0 {
        (1, 2) // 陌生人：玩家給 2 得 1
    } else if affinity <= 2 {
        (1, 1) // 相識：公平 1:1
    } else {
        (2, 1) // 友人：玩家給 1 得 2
    };
    let tier = demand.tier_for(offer_item);
    let coin_price = want_count * COIN_PRICE_PER_UNIT * (1 + tier);
    TradeOffer { offer_item, offer_count, want_item, want_count, coin_price }
}

/// 方塊 / 物品 id → 中文名（對齊 voxel_gift::item_name_zh，獨立維護讓模組自給自足）。
pub fn item_name_zh(block_id: u8) -> &'static str {
    match block_id {
        1 => "草",
        2 => "泥土",
        3 => "石頭",
        4 => "沙子",
        5 => "木頭",
        6 => "葉片",
        7 => "水",
        8 => "木板",
        9 => "石磚",
        10 => "玻璃",
        11 => "農田土",
        12 => "幼苗",
        13 => "成熟小麥",
        14 => "種子",
        18 => "小麥",
        19 => "麵包",
        // 乙太幣（`voxel_craft::COIN_ID`，ROADMAP 873）：付幣代替湊材料 v1 起，交易台詞/
        // 成交回條也可能提到「乙太幣」，補進這份獨立維護的命名表。
        98 => "乙太幣",
        _ => "物品",
    }
}

/// 居民提出交易時冒出的台詞（確定性純函式，零 LLM）。
/// 依 offer_count vs want_count 選不同語氣（公平 / 划算 / 不划算）。
pub fn offer_say_line(offer: &TradeOffer) -> String {
    let oname = item_name_zh(offer.offer_item);
    let wname = item_name_zh(offer.want_item);
    if offer.offer_count == offer.want_count {
        format!("我這兒有{}，要不要換你的{}？1:1 公平！", oname, wname)
    } else if offer.offer_count > offer.want_count {
        // 居民提供更多（友人優待）：強調划算
        format!("給你{}個{}，換你{}個{}——你是我的朋友，划得來！",
            offer.offer_count, oname, offer.want_count, wname)
    } else {
        // 居民提供更少（陌生人不信任）：坦白說明條件
        format!("你給我{}個{}，我給你{}個{}，怎麼樣？",
            offer.want_count, wname, offer.offer_count, oname)
    }
}

/// 交易成功後居民說的話（確定性純函式）。
pub fn done_say_line(player_name: &str, got_name: &str) -> String {
    if player_name.is_empty() {
        format!("成交！{}給你了。", got_name)
    } else {
        format!("{}，成交！{}給你了，謝謝你。", player_name, got_name)
    }
}

/// 寫進居民記憶的摘要（1 筆，確定性純函式）。
pub fn trade_memory(player_name: &str, gave_name: &str, got_name: &str) -> String {
    format!("和{}以物易物：我給了{}，換來了{}，感覺不錯", player_name, gave_name, got_name)
}

/// 付幣代替湊材料 v1（ROADMAP 874）：付乙太幣成交時寫進居民記憶的摘要，語氣與純以物易物
/// 區隔開（點名「直接付了乙太幣」而非某種原礦，讓居民記得的是「省事」而非「以物易物」）。
pub fn trade_memory_coin(player_name: &str, coin_count: u32, got_name: &str) -> String {
    format!("和{}交易：他直接付了{}枚乙太幣，換走了{}，省事又乾脆", player_name, coin_count, got_name)
}

// ── 測試 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 沒有任何搶購紀錄的供需追蹤（測試預設基準，等同 874 舊行為）。
    fn no_demand() -> CoinDemandTracker {
        CoinDemandTracker::new()
    }

    #[test]
    fn resident_trade_slot_in_range() {
        for id in ["luna", "nova", "sailer", "auri", "", "abc123", "居民露娜"] {
            let slot = resident_trade_slot(id);
            assert!(slot < 4, "slot={slot} 超出 0..4 (id={id})");
        }
    }

    #[test]
    fn resident_trade_slot_deterministic() {
        let s1 = resident_trade_slot("居民露娜-001");
        let s2 = resident_trade_slot("居民露娜-001");
        assert_eq!(s1, s2, "相同 id 應得相同 slot");
    }

    #[test]
    fn make_offer_stranger_unfavorable() {
        // 陌生人（affinity=0）：玩家給更多才能得到居民的物品
        for id in ["luna", "nova", "sailer", "auri"] {
            let offer = make_offer(id, 0, &no_demand());
            assert!(offer.want_count > offer.offer_count,
                "陌生人交易應不划算 id={id}");
        }
    }

    #[test]
    fn make_offer_acquaintance_fair() {
        // 相識（1–2）：1:1 公平
        for id in ["luna", "nova", "sailer", "auri"] {
            for affinity in [1usize, 2] {
                let offer = make_offer(id, affinity, &no_demand());
                assert_eq!(offer.offer_count, offer.want_count,
                    "相識應 1:1 id={id} affinity={affinity}");
            }
        }
    }

    #[test]
    fn make_offer_friend_favorable() {
        // 友人（3+）：玩家給更少、得到更多
        for id in ["luna", "nova", "sailer", "auri"] {
            for affinity in [3usize, 5, 10] {
                let offer = make_offer(id, affinity, &no_demand());
                assert!(offer.offer_count > offer.want_count,
                    "友人交易應划算 id={id} affinity={affinity}");
            }
        }
    }

    #[test]
    fn make_offer_items_nonzero_and_different() {
        let offer = make_offer("test-resident", 1, &no_demand());
        assert!(offer.offer_item > 0, "offer_item 應非 0（不是 Air）");
        assert!(offer.want_item > 0, "want_item 應非 0（不是 Air）");
        assert_ne!(offer.offer_item, offer.want_item,
            "提供與需求物品不應相同（同一物品沒意義）");
    }

    #[test]
    fn make_offer_counts_positive() {
        for affinity in [0usize, 1, 2, 3, 10] {
            let offer = make_offer("resident-x", affinity, &no_demand());
            assert!(offer.offer_count > 0, "affinity={affinity} offer_count 應>0");
            assert!(offer.want_count > 0, "affinity={affinity} want_count 應>0");
        }
    }

    #[test]
    fn offer_say_line_non_empty_no_braces() {
        for affinity in [0, 1, 2, 3] {
            let offer = make_offer("resident-y", affinity, &no_demand());
            let s = offer_say_line(&offer);
            assert!(!s.is_empty(), "affinity={affinity} 台詞不得空");
            assert!(!s.contains('{'), "affinity={affinity} 台詞含未替換佔位");
        }
    }

    #[test]
    fn offer_say_line_fair_contains_item_names() {
        // 公平 1:1：台詞應提到兩種物品名
        let offer = TradeOffer { offer_item: 5, offer_count: 1, want_item: 4, want_count: 1, coin_price: 2 };
        let s = offer_say_line(&offer);
        assert!(s.contains("木頭"), "公平台詞應含 offer_item 名（木頭）");
        assert!(s.contains("沙子"), "公平台詞應含 want_item 名（沙子）");
    }

    #[test]
    fn done_say_line_non_empty() {
        assert!(!done_say_line("旅人", "木頭").is_empty());
        assert!(!done_say_line("", "石頭").is_empty());
    }

    #[test]
    fn done_say_line_with_name_contains_name() {
        let s = done_say_line("小明", "玻璃");
        assert!(s.contains("小明"), "含玩家名的成交台詞應包含玩家名");
    }

    #[test]
    fn trade_memory_contains_all_parts() {
        let s = trade_memory("小美", "種子", "木頭");
        assert!(s.contains("小美"), "記憶應含玩家名");
        assert!(s.contains("種子"), "記憶應含給出物品名");
        assert!(s.contains("木頭"), "記憶應含換來物品名");
    }

    #[test]
    fn item_name_zh_known_ids() {
        assert_eq!(item_name_zh(3), "石頭");
        assert_eq!(item_name_zh(4), "沙子");
        assert_eq!(item_name_zh(5), "木頭");
        assert_eq!(item_name_zh(10), "玻璃");
        assert_eq!(item_name_zh(14), "種子");
    }

    #[test]
    fn item_name_zh_unknown_fallback() {
        assert_eq!(item_name_zh(200), "物品");
        assert_eq!(item_name_zh(0), "物品"); // Air 不交易
    }

    #[test]
    fn constants_sane() {
        assert!(TRADE_REACH > 0.0, "TRADE_REACH 應大於 0");
        assert!(TRADE_OFFER_TTL > 0, "TRADE_OFFER_TTL 應大於 0");
        assert!(COIN_PRICE_PER_UNIT > 0, "COIN_PRICE_PER_UNIT 應大於 0");
    }

    // ── 付幣代替湊材料 v1（ROADMAP 874）─────────────────────────────────────────

    #[test]
    fn coin_price_scales_with_want_count() {
        for id in ["luna", "nova", "sailer", "auri"] {
            for affinity in [0usize, 1, 3] {
                let offer = make_offer(id, affinity, &no_demand());
                assert_eq!(offer.coin_price, offer.want_count * COIN_PRICE_PER_UNIT,
                    "coin_price 應等於 want_count×匯率 id={id} affinity={affinity}");
            }
        }
    }

    #[test]
    fn coin_price_always_positive() {
        for affinity in [0usize, 1, 2, 3, 10] {
            let offer = make_offer("resident-z", affinity, &no_demand());
            assert!(offer.coin_price > 0, "affinity={affinity} coin_price 應>0");
        }
    }

    #[test]
    fn item_name_zh_coin() {
        assert_eq!(item_name_zh(98), "乙太幣");
    }

    #[test]
    fn trade_memory_coin_contains_all_parts() {
        let s = trade_memory_coin("小美", 4, "玻璃");
        assert!(s.contains("小美"), "記憶應含玩家名");
        assert!(s.contains('4'), "記憶應含付出的乙太幣數量");
        assert!(s.contains("乙太幣"), "記憶應點名付的是乙太幣");
        assert!(s.contains("玻璃"), "記憶應含換來物品名");
    }

    #[test]
    fn trade_memory_coin_no_braces() {
        let s = trade_memory_coin("", 2, "石頭");
        assert!(!s.contains('{'), "台詞含未替換佔位");
        assert!(!s.is_empty());
    }

    // ── 供需驅動漲價 v1（自主提案切片，ROADMAP 958）─────────────────────────────

    #[test]
    fn demand_tracker_starts_at_tier_zero() {
        let d = no_demand();
        assert_eq!(d.tier_for(10), 0, "沒有任何搶購紀錄應維持基礎價（tier 0）");
    }

    #[test]
    fn demand_tracker_tier_rises_with_purchases() {
        let mut d = no_demand();
        assert_eq!(d.tier_for(10), 0);
        for _ in 0..DEMAND_STEP {
            d.record_purchase(10);
        }
        assert_eq!(d.tier_for(10), 1, "累積滿一階購買次數應漲一階");
        for _ in 0..DEMAND_STEP {
            d.record_purchase(10);
        }
        assert_eq!(d.tier_for(10), 2, "再累積一階應再漲一階");
    }

    #[test]
    fn demand_tracker_tier_caps_at_max() {
        let mut d = no_demand();
        for _ in 0..(DEMAND_STEP * (MAX_DEMAND_TIER + 10)) {
            d.record_purchase(10);
        }
        assert_eq!(d.tier_for(10), MAX_DEMAND_TIER, "漲價階數不應超過封頂");
    }

    #[test]
    fn demand_tracker_is_per_item() {
        let mut d = no_demand();
        for _ in 0..(DEMAND_STEP * 2) {
            d.record_purchase(10); // 只搶購玻璃
        }
        assert_eq!(d.tier_for(10), 2, "玻璃應漲價");
        assert_eq!(d.tier_for(3), 0, "沒被搶購的石頭不應受影響");
    }

    #[test]
    fn demand_tracker_decay_lowers_tier_over_time() {
        let mut d = no_demand();
        for _ in 0..(DEMAND_STEP * 3) {
            d.record_purchase(10);
        }
        assert_eq!(d.tier_for(10), 3);
        d.decay_all();
        assert!(d.tier_for(10) < 3, "退燒一次後漲價階數應下降");
        // 持續退燒最終應完全回到基礎價（不會卡在某個殘值出不去）。
        for _ in 0..20 {
            d.decay_all();
        }
        assert_eq!(d.tier_for(10), 0, "退燒夠久應完全回到基礎價");
    }

    #[test]
    fn coin_price_rises_with_demand_tier() {
        // 找一個 offer_item=玻璃(10)（slot 3）的 id，不假設任何特定名字對應哪個 slot。
        let id = (0u32..1000)
            .map(|n| format!("resident-{n}"))
            .find(|id| resident_trade_slot(id) == 3)
            .expect("1000 個候選裡應找得到 slot == 3 的 id");
        let mut d = no_demand();
        let base_offer = make_offer(&id, 1, &d);
        assert_eq!(base_offer.offer_item, 10, "slot 3 的 offer_item 應為玻璃");
        for _ in 0..(DEMAND_STEP * 2) {
            d.record_purchase(10);
        }
        let hot_offer = make_offer(&id, 1, &d);
        assert!(hot_offer.coin_price > base_offer.coin_price,
            "被搶購的物品，同樣的 affinity 下 coin_price 應比沒人搶購時更貴");
        assert_eq!(hot_offer.coin_price, base_offer.coin_price * 3,
            "漲 2 階 = 基礎價的 (1+2) 倍");
    }

    #[test]
    fn demand_tier_survives_real_tick_cadence() {
        // 鎖住 review 撞見的落地雷：純函式窮舉 record_purchase N 次不受真實節拍限制，
        // 掩蓋了「decay_all 掛在太密的節拍上，熱度根本攢不起來」。這裡模擬真實
        // spawn_farm_tick 節奏——每個 tick 呼叫 record_purchase 若干次，並只在
        // 每 DEMAND_DECAY_EVERY_N_TICKS 個 tick 才呼叫一次 decay_all（跟 voxel_ws.rs
        // 的 coin_demand_ticks 計數器同步），斷言：玩家在一段正常遊玩節奏內連買
        // 幾次，tier 真的爬得上去、且不會在下一個 tick 就被腰斬回基礎價。
        let mut d = no_demand();
        let mut ticks: u32 = 0;
        // 一位玩家每個 tick 都買 1 次同一項物品，連買 DEMAND_STEP 次（正常遊玩節奏，
        // 不是「同一 tick 內狂點」）。
        for _ in 0..DEMAND_STEP {
            d.record_purchase(10);
            ticks += 1;
            if ticks % DEMAND_DECAY_EVERY_N_TICKS == 0 {
                d.decay_all();
            }
        }
        assert_eq!(d.tier_for(10), 1,
            "正常節奏連買 DEMAND_STEP 次應攢到 tier 1（不該被同期退燒抵消）");

        // 買完之後過一個 tick（還沒到退燒週期）：漲價應該還在，不會下一拍就蒸發。
        ticks += 1;
        if ticks % DEMAND_DECAY_EVERY_N_TICKS == 0 {
            d.decay_all();
        }
        assert_eq!(d.tier_for(10), 1, "剛漲價後一個 tick 內應該還撐得住，不會秒回基礎價");
    }

    #[test]
    fn coin_price_unaffected_when_demand_on_other_item() {
        // 找一個 offer_item 不是「石頭」(3) 的居民 id（種子/木頭/玻璃皆可）。
        let id = (0u32..1000)
            .map(|n| format!("resident-{n}"))
            .find(|id| resident_trade_slot(id) != 1)
            .expect("1000 個候選裡應找得到 slot != 1 的 id");
        let baseline = make_offer(&id, 1, &no_demand());

        let mut d = no_demand();
        // 搶購的是「石頭」（slot 1 的 offer_item），不該影響其他 slot 的報價。
        for _ in 0..(DEMAND_STEP * 3) {
            d.record_purchase(3);
        }
        let unaffected = make_offer(&id, 1, &d);
        assert_eq!(baseline.coin_price, unaffected.coin_price,
            "搶購石頭不應影響其他 offer_item 的報價");
    }
}
