//! 旅行商人系統（ROADMAP 135 / 136）。
//!
//! 每 2 小時從城外來一位神秘旅行商人，帶著其他生態域的稀有物品，
//! 停留 10 分鐘，限時交易。玩家近身（TRADE_REACH 像素內）可開交易面板，
//! 用乙太購買只有旅行商人才帶來的稀有物品（每次來訪庫存獨立重置）。
//!
//! ROADMAP 136：每次到訪同時附帶 2 張「限時委託」——一張採集令、一張狩獵令。
//! 玩家可接取並在商人停留期間完成，獲得乙太獎勵 + 旅商特供稀有物品。
//! 商人離去時未完成的委託一律失效（無懲罰）。
//!
//! 成本紀律：純本機邏輯，**不呼叫任何 LLM**；零 migration，記憶體模式，重啟清零。

use crate::combat::EnemyKind;
use crate::inventory::ItemKind;

/// 首次拜訪等待（秒）——伺服器啟動後 5 分鐘。
pub const FIRST_WAIT_SECS: f32 = 300.0;
/// 拜訪間隔（秒）——2 小時。
pub const VISIT_INTERVAL_SECS: f32 = 7200.0;
/// 停留時間（秒）——10 分鐘。
pub const STAY_SECS: f32 = 600.0;
/// 交易有效距離（像素）——玩家走進這個範圍才能開面板。
pub const TRADE_REACH: f32 = 100.0;
/// 旅行商人站立位置（城鎮廣場北緣；遠離一般商人避免混淆）。
pub const WANDERER_X: f32 = 2380.0;
pub const WANDERER_Y: f32 = 2150.0;

// ── ROADMAP 136：限時委託 ─────────────────────────────────────────────────────

/// 委託任務類型（採集指定物品 / 擊殺指定怪種）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MerchantQuestKind {
    Gather { item: ItemKind, required: u32 },
    Kill   { enemy: EnemyKind, required: u32 },
}

/// 玩家接取並進行中的委託（或已完成 / 未接取的）。
#[derive(Debug, Clone)]
pub struct MerchantQuest {
    pub id:            u8,
    pub name:          &'static str,
    pub description:   &'static str,
    pub kind:          MerchantQuestKind,
    /// 完成後的乙太獎勵。
    pub reward_ether:  u32,
    /// 完成後的稀有物品獎勵（旅商特供，不從目錄扣庫存）。
    pub reward_item:   ItemKind,
    pub reward_qty:    u32,
    /// 是否已被玩家接取。
    pub accepted:      bool,
    /// 目前進度（擊殺 / 採集數量）。
    pub progress:      u32,
    /// 是否已完成（獎勵已發放）。
    pub completed:     bool,
}

impl MerchantQuest {
    /// 目前需達到的目標數量。
    pub fn required(&self) -> u32 {
        match &self.kind {
            MerchantQuestKind::Gather { required, .. } => *required,
            MerchantQuestKind::Kill   { required, .. } => *required,
        }
    }

    /// 委託是否進行中（已接取且未完成）。
    pub fn is_active(&self) -> bool {
        self.accepted && !self.completed
    }
}

/// 每次到訪提供的 2 張靜態委託（採集令 + 狩獵令）。
///
/// 採集令：採集 3 個星晶碎片 → 12 乙太 + 星塵×3（方便合成星光護符）。
/// 狩獵令：擊殺 2 隻晶石傀儡 → 18 乙太 + 彩虹星塵×1（流星雨稀有物，另一條取得路）。
const MERCHANT_QUEST_DEFS: &[(&str, &str, MerchantQuestKind, u32, ItemKind, u32)] = &[
    (
        "採集令",
        "為旅行商人採集 3 個星晶碎片，獲得 12 乙太與稀有星塵",
        MerchantQuestKind::Gather { item: ItemKind::StarCrystalShard, required: 3 },
        12,
        ItemKind::StarDust,
        3,
    ),
    (
        "狩獵令",
        "替旅行商人獵取 2 隻晶石傀儡，獲得 18 乙太與彩虹星塵",
        MerchantQuestKind::Kill { enemy: EnemyKind::CrystalGolem, required: 2 },
        18,
        ItemKind::RainbowStarDust,
        1,
    ),
];

/// 建立本次到訪的委託清單（2 張，各自獨立）。
fn build_quests() -> Vec<MerchantQuest> {
    MERCHANT_QUEST_DEFS
        .iter()
        .enumerate()
        .map(|(i, (name, desc, kind, ether, item, qty))| MerchantQuest {
            id:           (i as u8) + 1,
            name,
            description:  desc,
            kind:         kind.clone(),
            reward_ether: *ether,
            reward_item:  *item,
            reward_qty:   *qty,
            accepted:     false,
            progress:     0,
            completed:    false,
        })
        .collect()
}

/// 供快照廣播用的委託摘要（可序列化、無循環引用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct MerchantQuestView {
    pub id:           u8,
    pub name:         &'static str,
    pub description:  &'static str,
    pub required:     u32,
    pub progress:     u32,
    pub accepted:     bool,
    pub completed:    bool,
    pub reward_ether: u32,
    /// 獎勵物品的 snake_case 字串（前端 ITEM_LOOK 對應鍵）。
    pub reward_item:  String,
    pub reward_qty:   u32,
}

// ── ROADMAP 135：交易目錄 ─────────────────────────────────────────────────────

/// 旅行商人一個商品條目。
#[derive(Debug, Clone)]
pub struct WanderingItem {
    pub item: ItemKind,
    /// 每次來訪可售數量上限（售完即缺貨）。
    pub stock: u32,
    /// 乙太單價。
    pub price_ether: u32,
    /// 本次來訪已售出數量（到訪重置為 0）。
    pub sold: u32,
}

impl WanderingItem {
    fn new(item: ItemKind, stock: u32, price_ether: u32) -> Self {
        Self { item, stock, price_ether, sold: 0 }
    }

    pub fn remaining(&self) -> u32 {
        self.stock.saturating_sub(self.sold)
    }
}

/// 旅行商人狀態（純記憶體，重啟清零）。
pub struct WanderingMerchantState {
    /// 距下次到訪的冷卻倒數（秒）。
    pub cooldown: f32,
    /// 在場倒計時（秒）；0 = 商人不在城鎮。
    pub active_secs: f32,
    /// 本次來訪商品目錄（到訪時填入，離去後清空）。
    pub catalog: Vec<WanderingItem>,
    /// 本次來訪的限時委託（ROADMAP 136；到訪時填入，離去後清空）。
    pub quests: Vec<MerchantQuest>,
}

impl WanderingMerchantState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_WAIT_SECS,
            active_secs: 0.0,
            catalog: vec![],
            quests: vec![],
        }
    }

    /// 旅行商人目前是否在城鎮。
    pub fn is_active(&self) -> bool {
        self.active_secs > 0.0
    }

    /// 剩餘整數秒（供快照廣播）；不在城鎮時回 0。
    pub fn remaining_secs(&self) -> u32 {
        self.active_secs.ceil() as u32
    }

    /// 前進 dt 秒。回傳 (arrived, departed)。
    pub fn tick(&mut self, dt: f32) -> (bool, bool) {
        if self.is_active() {
            self.active_secs -= dt;
            if self.active_secs <= 0.0 {
                self.active_secs = 0.0;
                self.catalog.clear();
                self.quests.clear();
                self.cooldown = VISIT_INTERVAL_SECS;
                return (false, true);
            }
            return (false, false);
        }

        self.cooldown -= dt;
        if self.cooldown <= 0.0 {
            self.active_secs = STAY_SECS;
            self.catalog = build_catalog();
            self.quests = build_quests();
            return (true, false);
        }
        (false, false)
    }

    /// 玩家接取指定委託。成功回 `Ok(quest_name)`，失敗回 `Err(原因)`。
    pub fn accept_quest(&mut self, quest_id: u8) -> Result<&'static str, &'static str> {
        if !self.is_active() {
            return Err("旅行商人不在城鎮");
        }
        let quest = self.quests.iter_mut()
            .find(|q| q.id == quest_id)
            .ok_or("找不到指定委託")?;
        if quest.accepted {
            return Err("已接取此委託");
        }
        if quest.completed {
            return Err("委託已完成");
        }
        quest.accepted = true;
        Ok(quest.name)
    }

    /// 玩家擊殺了一隻怪。若有進行中的狩獵令且匹配，更新進度。
    /// 回傳完成的委託資訊 `Some((name, reward_ether, reward_item, reward_qty))`，或 `None`。
    pub fn on_kill(&mut self, enemy: EnemyKind) -> Option<(u8, &'static str, u32, ItemKind, u32)> {
        for q in &mut self.quests {
            if !q.is_active() {
                continue;
            }
            if let MerchantQuestKind::Kill { enemy: target, required } = q.kind {
                if target == enemy {
                    q.progress += 1;
                    if q.progress >= required {
                        q.completed = true;
                        return Some((q.id, q.name, q.reward_ether, q.reward_item, q.reward_qty));
                    }
                }
            }
        }
        None
    }

    /// 玩家採集了物品。若有進行中的採集令且匹配，更新進度。
    /// 回傳完成的委託資訊 `Some((name, reward_ether, reward_item, reward_qty))`，或 `None`。
    pub fn on_gather(&mut self, item: ItemKind, qty: u32) -> Option<(u8, &'static str, u32, ItemKind, u32)> {
        for q in &mut self.quests {
            if !q.is_active() {
                continue;
            }
            if let MerchantQuestKind::Gather { item: target, required } = q.kind {
                if target == item {
                    q.progress = q.progress.saturating_add(qty).min(required);
                    if q.progress >= required {
                        q.completed = true;
                        return Some((q.id, q.name, q.reward_ether, q.reward_item, q.reward_qty));
                    }
                }
            }
        }
        None
    }

    /// 快照用：回傳委託摘要（id / 名稱 / 需求 / 進度 / 已接 / 已完 / 獎勵）。
    pub fn quest_views(&self) -> Vec<MerchantQuestView> {
        self.quests.iter().map(|q| {
            // 用 serde_json 取得 snake_case wire key（與前端 ITEM_LOOK 對應）。
            let reward_item = serde_json::to_string(&q.reward_item)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            MerchantQuestView {
                id:           q.id,
                name:         q.name,
                description:  q.description,
                required:     q.required(),
                progress:     q.progress,
                accepted:     q.accepted,
                completed:    q.completed,
                reward_ether: q.reward_ether,
                reward_item,
                reward_qty:   q.reward_qty,
            }
        }).collect()
    }

    /// 玩家購買 qty 單位 item。回傳 Ok(total_ether_cost) 或 Err(描述)。
    pub fn buy(&mut self, item: ItemKind, qty: u32) -> Result<u32, &'static str> {
        if !self.is_active() {
            return Err("旅行商人不在城鎮");
        }
        if qty == 0 {
            return Err("數量必須 >= 1");
        }
        let entry = self
            .catalog
            .iter_mut()
            .find(|e| e.item == item)
            .ok_or("旅行商人沒有這個商品")?;
        if qty > entry.remaining() {
            return Err("商品庫存不足");
        }
        let cost = entry.price_ether.saturating_mul(qty);
        entry.sold += qty;
        Ok(cost)
    }
}

/// 每次來訪的標準商品目錄——帶著其他生態域稀有物品，乙太售價略高於市面。
fn build_catalog() -> Vec<WanderingItem> {
    vec![
        // 裂縫碎片：通常只能在深層戰鬥取得，量少，旅商帶來 2 個
        WanderingItem::new(ItemKind::RiftShard, 2, 20),
        // 岩漿晶石：炎紅星才有，非探索者難以取得
        WanderingItem::new(ItemKind::LavaCrystal, 3, 14),
        // 翠幽碎片：翠幽星才有，旅商偶爾帶來
        WanderingItem::new(ItemKind::JadeShard, 3, 12),
        // 星晶碎片：流星雨少見掉落，旅商帶來穩定補貨渠道
        WanderingItem::new(ItemKind::StarCrystalShard, 5, 8),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_arrival_triggers_after_first_wait() {
        let mut s = WanderingMerchantState::new();
        let (arrived, _) = s.tick(FIRST_WAIT_SECS - 1.0);
        assert!(!arrived, "未到 FIRST_WAIT_SECS 不應到訪");
        assert!(!s.is_active());
        let (arrived, _) = s.tick(1.0);
        assert!(arrived, "剛好超過 FIRST_WAIT_SECS 應到訪");
        assert!(s.is_active());
        assert_eq!(s.catalog.len(), 4);
        assert_eq!(s.quests.len(), 2, "到訪時應帶 2 張委託");
    }

    #[test]
    fn stays_for_stay_secs_then_departs() {
        let mut s = WanderingMerchantState::new();
        s.cooldown = 0.1;
        s.tick(0.1); // 觸發到訪
        assert!(s.is_active());
        let (_, departed) = s.tick(STAY_SECS);
        assert!(departed, "應在 STAY_SECS 後離去");
        assert!(!s.is_active());
        assert!(s.catalog.is_empty(), "離去後商品目錄應清空");
        assert!(s.quests.is_empty(), "離去後委託應清空");
    }

    #[test]
    fn next_visit_resets_sold() {
        let mut s = WanderingMerchantState::new();
        s.cooldown = 0.1;
        s.tick(0.1);
        s.buy(ItemKind::RiftShard, 1).unwrap();
        s.tick(STAY_SECS); // 商人離去
        s.tick(VISIT_INTERVAL_SECS); // 觸發下一次到訪
        assert!(s.is_active());
        let entry = s.catalog.iter().find(|e| e.item == ItemKind::RiftShard).unwrap();
        assert_eq!(entry.sold, 0, "每次到訪 sold 應重置");
    }

    #[test]
    fn buy_deducts_stock_and_returns_cost() {
        let mut s = WanderingMerchantState::new();
        s.active_secs = STAY_SECS;
        s.catalog = build_catalog();
        let cost = s.buy(ItemKind::RiftShard, 1).unwrap();
        assert_eq!(cost, 20, "RiftShard 售價應為 20 乙太");
        let e = s.catalog.iter().find(|e| e.item == ItemKind::RiftShard).unwrap();
        assert_eq!(e.sold, 1);
        assert_eq!(e.remaining(), 1); // stock=2, sold=1
    }

    #[test]
    fn buy_multi_qty_correct_cost() {
        let mut s = WanderingMerchantState::new();
        s.active_secs = STAY_SECS;
        s.catalog = build_catalog();
        let cost = s.buy(ItemKind::StarCrystalShard, 3).unwrap();
        assert_eq!(cost, 24); // 8 * 3
    }

    #[test]
    fn buy_fails_when_inactive() {
        let mut s = WanderingMerchantState::new();
        assert!(s.buy(ItemKind::RiftShard, 1).is_err());
    }

    #[test]
    fn buy_fails_when_out_of_stock() {
        let mut s = WanderingMerchantState::new();
        s.active_secs = STAY_SECS;
        s.catalog = build_catalog();
        s.buy(ItemKind::RiftShard, 2).unwrap(); // 全買光 (stock=2)
        assert!(s.buy(ItemKind::RiftShard, 1).is_err(), "庫存耗盡後應拒絕購買");
    }

    #[test]
    fn buy_zero_qty_fails() {
        let mut s = WanderingMerchantState::new();
        s.active_secs = STAY_SECS;
        s.catalog = build_catalog();
        assert!(s.buy(ItemKind::RiftShard, 0).is_err());
    }

    #[test]
    fn remaining_secs_returns_ceil() {
        let mut s = WanderingMerchantState::new();
        s.active_secs = 9.3;
        assert_eq!(s.remaining_secs(), 10);
    }

    #[test]
    fn cooldown_resets_after_depart() {
        let mut s = WanderingMerchantState::new();
        s.cooldown = 0.1;
        s.tick(0.1);
        s.tick(STAY_SECS);
        // 冷卻應重置為 VISIT_INTERVAL_SECS
        assert!((s.cooldown - VISIT_INTERVAL_SECS).abs() < 1.0);
    }

    // ── ROADMAP 136：委託系統測試 ────────────────────────────────────────────

    fn make_active_state() -> WanderingMerchantState {
        let mut s = WanderingMerchantState::new();
        s.active_secs = STAY_SECS;
        s.catalog = build_catalog();
        s.quests = build_quests();
        s
    }

    #[test]
    fn build_quests_returns_two_quests() {
        let quests = build_quests();
        assert_eq!(quests.len(), 2);
        // 第一張為採集令，第二張為狩獵令
        assert!(matches!(quests[0].kind, MerchantQuestKind::Gather { .. }));
        assert!(matches!(quests[1].kind, MerchantQuestKind::Kill { .. }));
    }

    #[test]
    fn quests_start_not_accepted() {
        let quests = build_quests();
        for q in &quests {
            assert!(!q.accepted);
            assert!(!q.completed);
            assert_eq!(q.progress, 0);
        }
    }

    #[test]
    fn accept_quest_succeeds_when_active() {
        let mut s = make_active_state();
        let result = s.accept_quest(1);
        assert!(result.is_ok());
        assert!(s.quests[0].accepted);
    }

    #[test]
    fn accept_quest_fails_when_inactive() {
        let mut s = WanderingMerchantState::new();
        assert!(s.accept_quest(1).is_err());
    }

    #[test]
    fn accept_quest_fails_when_already_accepted() {
        let mut s = make_active_state();
        s.accept_quest(1).unwrap();
        let result = s.accept_quest(1);
        assert!(result.is_err());
    }

    #[test]
    fn on_gather_progresses_gather_quest() {
        let mut s = make_active_state();
        s.accept_quest(1).unwrap(); // 採集令 id=1
        let result = s.on_gather(ItemKind::StarCrystalShard, 2);
        assert!(result.is_none(), "還差 1 個，未完成");
        assert_eq!(s.quests[0].progress, 2);
    }

    #[test]
    fn on_gather_completes_quest_at_required() {
        let mut s = make_active_state();
        s.accept_quest(1).unwrap();
        s.on_gather(ItemKind::StarCrystalShard, 2);
        let completion = s.on_gather(ItemKind::StarCrystalShard, 1);
        assert!(completion.is_some(), "第 3 個應觸發完成");
        let (id, _name, ether, item, qty) = completion.unwrap();
        assert_eq!(id, 1);
        assert_eq!(ether, 12);
        assert_eq!(item, ItemKind::StarDust);
        assert_eq!(qty, 3);
        assert!(s.quests[0].completed);
    }

    #[test]
    fn on_gather_does_not_affect_wrong_item() {
        let mut s = make_active_state();
        s.accept_quest(1).unwrap();
        let result = s.on_gather(ItemKind::Wood, 5);
        assert!(result.is_none());
        assert_eq!(s.quests[0].progress, 0, "不匹配物品不計進度");
    }

    #[test]
    fn on_kill_progresses_kill_quest() {
        let mut s = make_active_state();
        s.accept_quest(2).unwrap(); // 狩獵令 id=2
        let result = s.on_kill(EnemyKind::CrystalGolem);
        assert!(result.is_none(), "還差 1 隻，未完成");
        assert_eq!(s.quests[1].progress, 1);
    }

    #[test]
    fn on_kill_completes_kill_quest() {
        let mut s = make_active_state();
        s.accept_quest(2).unwrap();
        s.on_kill(EnemyKind::CrystalGolem);
        let completion = s.on_kill(EnemyKind::CrystalGolem);
        assert!(completion.is_some(), "第 2 隻應觸發完成");
        let (id, _name, ether, item, qty) = completion.unwrap();
        assert_eq!(id, 2);
        assert_eq!(ether, 18);
        assert_eq!(item, ItemKind::RainbowStarDust);
        assert_eq!(qty, 1);
        assert!(s.quests[1].completed);
    }

    #[test]
    fn on_kill_does_not_affect_wrong_enemy() {
        let mut s = make_active_state();
        s.accept_quest(2).unwrap();
        let result = s.on_kill(EnemyKind::FlutterSprite);
        assert!(result.is_none());
        assert_eq!(s.quests[1].progress, 0, "不匹配敵人不計進度");
    }

    #[test]
    fn quest_views_serializes_reward_item_as_snake_case() {
        let s = make_active_state();
        let views = s.quest_views();
        // 採集令獎勵 StarDust → "star_dust"
        assert_eq!(views[0].reward_item, "star_dust");
        // 狩獵令獎勵 RainbowStarDust → "rainbow_star_dust"
        assert_eq!(views[1].reward_item, "rainbow_star_dust");
    }

    #[test]
    fn unaccepted_quest_on_kill_does_not_progress() {
        let mut s = make_active_state();
        // 不先接取，直接擊殺
        let result = s.on_kill(EnemyKind::CrystalGolem);
        assert!(result.is_none());
        assert_eq!(s.quests[1].progress, 0, "未接取的委託不計進度");
    }
}
