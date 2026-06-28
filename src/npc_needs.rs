//! NPC 需求驅力系統（ROADMAP 69：湧現派系第一塊）。
//!
//! 每個 NPC 有三個內心需求：安全感、歸屬感、繁榮感（0~100）。
//! 世界事件影響這些數值，數值每 DECAY_INTERVAL_SECS 秒緩慢向基線（50）回歸。
//! 對話時注入 system prompt，讓 NPC 語氣自然反映當下的情緒狀態。
//!
//! 設計鐵律：
//! - 純記憶體模式，重啟清零（=世界換季，NPC 重新出發）。
//! - 零 DB migration，零 WebSocket 依賴，純邏輯可獨立測試。
//! - LLM 仍只生成文字，需求數值只影響 system prompt 語境，碰不到遊戲狀態。

use std::collections::HashMap;

/// 需求值的中性基線——緩慢向此值回歸。
const BASELINE: i32 = 50;

/// 每次 `tick_decay_all` 向基線靠近的步長。
const DECAY_STEP: i32 = 1;

/// game.rs 呼叫 `tick_decay_all` 的週期（秒）。較慢的回歸讓情緒狀態有明顯持續性。
pub const DECAY_INTERVAL_SECS: u64 = 120; // 每 2 分鐘衰減一次

/// 需求偏低、足以讓居民「面露難色」並讓玩家可上前撫慰的門檻（ROADMAP 554）。
/// 取在「略感緊張／略有距離感／平凡度日」這一檔（< BASELINE）再低一些，
/// 確保只有**真的不好過**時才浮出煩惱、招來園丁，而非日常起伏。
pub const WORRY_THRESHOLD: i32 = 40;

/// 玩家上前「關心」一次（生面孔／交情尚淺時）把那份偏低的需求往上推的幅度（ROADMAP 554）。
/// 刻意一次推不滿——幾句關心才把人從谷底拉回平衡，讓「照料」是有過程的陪伴而非一鍵清空。
/// 同時是 [`comfort_amount`] 的最低一階（生面孔＝改版前幅度，向後相容）。
const COMFORT_STEP: i32 = 8;

/// 交情越深，撫慰越深入人心（ROADMAP 555）：把園丁的照料效力綁在與該居民「累積的交情」上。
/// `bond_tier_ord` ＝ 相熟層級的序（0 ＝ 生面孔、1 ＝ 點頭之交、2 ＝ 餐桌熟客，對齊
/// [`crate::lunch_regular::Familiarity`] 的順序），數值越大撫慰幅度越大；越界（>2）夾到最高一階。
/// 設計：居民對信得過的人更願意敞開心房，同一句關心落在老友心上，比落在生面孔心上更能撫平愁緒。
/// 純函式、確定性、可測；最低一階刻意等於 [`COMFORT_STEP`]（生面孔＝改版前，向後相容）。
pub fn comfort_amount(bond_tier_ord: u8) -> i32 {
    match bond_tier_ord {
        0 => COMFORT_STEP,      // 生面孔：8（＝改版前）
        1 => COMFORT_STEP + 4,  // 點頭之交：12
        _ => COMFORT_STEP + 8,  // 餐桌熟客（含越界）：16
    }
}

/// 故鄉茶棚每次「出爐熱茶」給全鎮 NPC 回暖的歸屬感幅度（ROADMAP 641，禱告驅動）。
/// 刻意小——這是日常的一盞熱茶（露娜祈願的「街角熱茶暖身、市集找到新朋友」），不是整場村慶；
/// 配合每 2 分鐘向基線回歸的衰減，長期讓全鎮歸屬感停在略高於平淡的「有點溫度」，而非一路衝頂。
const TEA_WARMTH: i32 = 4;

/// 三大需求之一，用來標出「此刻最該被撫平的那一件心事」（ROADMAP 554）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedKind {
    Safety,
    Belonging,
    Prosperity,
}

/// 一個 NPC 的三大需求值（0~100）。
#[derive(Debug, Clone)]
pub struct NpcNeeds {
    /// 安全感：對外部威脅的主觀感受（低 = 極度不安，高 = 從容安心）。
    pub safety: i32,
    /// 歸屬感：與村落、玩家的連結感（低 = 疏離，高 = 充滿歸屬）。
    pub belonging: i32,
    /// 繁榮感：乙太/資源/生意的豐足感（低 = 拮据，高 = 豐饒慷慨）。
    pub prosperity: i32,
}

impl Default for NpcNeeds {
    fn default() -> Self {
        Self { safety: BASELINE, belonging: BASELINE, prosperity: BASELINE }
    }
}

impl NpcNeeds {
    fn clamp_all(&mut self) {
        self.safety = self.safety.clamp(0, 100);
        self.belonging = self.belonging.clamp(0, 100);
        self.prosperity = self.prosperity.clamp(0, 100);
    }

    /// 此刻**最該被撫平的那件心事**：三大需求中**低於 [`WORRY_THRESHOLD`]、且數值最低**的那一個。
    /// 全都還算安穩（皆 ≥ 門檻）→ `None`（這位居民現在不需要被關心）。
    /// 平手時依 安全感 ＞ 歸屬感 ＞ 繁榮感 的順序取（安危最要緊）。純函式、確定性。
    pub fn lowest_low_need(&self) -> Option<NeedKind> {
        let mut pick: Option<(NeedKind, i32)> = None;
        // 依優先序逐一比較：只有「更低」才取代，平手保留先到者（即上述優先序）。
        for (kind, val) in [
            (NeedKind::Safety, self.safety),
            (NeedKind::Belonging, self.belonging),
            (NeedKind::Prosperity, self.prosperity),
        ] {
            if val < WORRY_THRESHOLD && pick.map_or(true, |(_, best)| val < best) {
                pick = Some((kind, val));
            }
        }
        pick.map(|(k, _)| k)
    }

    /// 玩家上前關心，把指定的那件心事往上推 [`COMFORT_STEP`]（夾在 0~100）。
    /// 只動那一項，其餘不變；園丁的撫慰是把谷底慢慢拉回平衡。
    /// 維持生面孔／改版前幅度，等同 `comfort_by(need, COMFORT_STEP)`（向後相容）。
    pub fn comfort(&mut self, need: NeedKind) {
        self.comfort_by(need, COMFORT_STEP);
    }

    /// 同 [`comfort`](Self::comfort)，但撫慰幅度由呼叫端給定（ROADMAP 555）。
    /// 交情越深、`amount` 越大（見 [`comfort_amount`]）；只動目標那一項、其餘不變、夾在 0~100。
    pub fn comfort_by(&mut self, need: NeedKind, amount: i32) {
        match need {
            NeedKind::Safety => self.safety += amount,
            NeedKind::Belonging => self.belonging += amount,
            NeedKind::Prosperity => self.prosperity += amount,
        }
        self.clamp_all();
    }

    /// 每個值向基線靠近 DECAY_STEP 單位（高的降、低的升、恰好的不動）。
    pub fn decay_toward_baseline(&mut self) {
        if self.safety > BASELINE { self.safety -= DECAY_STEP; }
        else if self.safety < BASELINE { self.safety += DECAY_STEP; }

        if self.belonging > BASELINE { self.belonging -= DECAY_STEP; }
        else if self.belonging < BASELINE { self.belonging += DECAY_STEP; }

        if self.prosperity > BASELINE { self.prosperity -= DECAY_STEP; }
        else if self.prosperity < BASELINE { self.prosperity += DECAY_STEP; }
    }

    /// 組成可插入 system prompt 的短段落（自然流露，無需直說）。
    pub fn to_prompt_section(&self) -> String {
        fn safety_desc(v: i32) -> &'static str {
            if v < 30 { "極度不安、難掩緊張" }
            else if v < 50 { "略感緊張、偶爾透露顧慮" }
            else if v <= 70 { "尚算安穩" }
            else { "從容安心、語氣有餘裕" }
        }
        fn belonging_desc(v: i32) -> &'static str {
            if v < 30 { "疏離、渴望連結" }
            else if v < 50 { "略有距離感" }
            else if v <= 70 { "溫暖且有連結感" }
            else { "充滿歸屬感、熱情待人" }
        }
        fn prosperity_desc(v: i32) -> &'static str {
            if v < 30 { "資源匱乏、分享更謹慎" }
            else if v < 50 { "平凡度日" }
            else if v <= 70 { "小有盈餘、心情還不錯" }
            else { "豐饒慷慨" }
        }
        format!(
            "\n\n【你此刻的心情狀態（讓它自然流露在語氣中，無需直說）】安全感 {s}/100（{sd}）・歸屬感 {b}/100（{bd}）・繁榮感 {p}/100（{pd}）",
            s = self.safety, sd = safety_desc(self.safety),
            b = self.belonging, bd = belonging_desc(self.belonging),
            p = self.prosperity, pd = prosperity_desc(self.prosperity),
        )
    }
}

/// 觸發 NPC 需求調整的世界事件（與 npc_proactive::WorldEventKind 對應，但獨立定義避免循環依賴）。
#[derive(Debug, Clone, Copy)]
pub enum NeedsEvent {
    RiftOpened,
    HordeArriving,
    HordeRepelled,
    QuestCompleted,
    VillageFestival,
    EliteSlain,
}

/// 所有 NPC 需求狀態的容器（記憶體模式，重啟清零）。
#[derive(Default)]
pub struct NpcNeedsState {
    map: HashMap<String, NpcNeeds>,
}

impl NpcNeedsState {
    /// 初始化七大 NPC，各有反映其個性的起始需求值。
    pub fn new() -> Self {
        let mut s = Self::default();
        // 商人：偏重繁榮感（生意人天性）
        s.map.insert("merchant".into(), NpcNeeds { safety: 55, belonging: 50, prosperity: 65 });
        // 工匠：高安全感（熟悉工坊環境）
        s.map.insert("workshop_npc".into(), NpcNeeds { safety: 60, belonging: 55, prosperity: 45 });
        // 獵手：安全感偏低（習慣警覺），但不畏危險
        s.map.insert("bounty_npc".into(), NpcNeeds { safety: 45, belonging: 50, prosperity: 50 });
        // 探勘員：高歸屬感（熱愛這個世界）
        s.map.insert("expedition_npc".into(), NpcNeeds { safety: 55, belonging: 65, prosperity: 45 });
        // 採購代理人：均衡（世故從容）
        s.map.insert("procurement_npc".into(), NpcNeeds { safety: 55, belonging: 50, prosperity: 55 });
        // 評審老農：高歸屬感（扎根土地的老農）
        s.map.insert("farm_fair_npc".into(), NpcNeeds { safety: 60, belonging: 65, prosperity: 50 });
        // 里長：高歸屬感（村落精神支柱）
        s.map.insert("village_chief".into(), NpcNeeds { safety: 55, belonging: 70, prosperity: 50 });
        s
    }

    /// 取得指定 NPC 的需求狀態複本（未知 NPC 回預設值）。
    pub fn get(&self, npc_id: &str) -> NpcNeeds {
        self.map.get(npc_id).cloned().unwrap_or_default()
    }

    /// 指定 NPC 此刻最該被撫平的那件心事（ROADMAP 554）；都安穩或未知 NPC → `None`。
    pub fn lowest_low_need(&self, npc_id: &str) -> Option<NeedKind> {
        self.map.get(npc_id).and_then(|n| n.lowest_low_need())
    }

    /// 玩家上前關心指定 NPC 的某件心事，把那份需求往上推（ROADMAP 554）。未知 NPC → 無動作。
    pub fn comfort(&mut self, npc_id: &str, need: NeedKind) {
        if let Some(n) = self.map.get_mut(npc_id) {
            n.comfort(need);
        }
    }

    /// 同 [`comfort`](Self::comfort)，但撫慰幅度由呼叫端給定（ROADMAP 555：交情越深、幅度越大）。
    /// 未知 NPC → 無動作。
    pub fn comfort_by(&mut self, npc_id: &str, need: NeedKind, amount: i32) {
        if let Some(n) = self.map.get_mut(npc_id) {
            n.comfort_by(need, amount);
        }
    }

    /// 世界事件發生，調整所有相關 NPC 的需求值。
    pub fn apply_world_event(&mut self, event: NeedsEvent) {
        match event {
            NeedsEvent::RiftOpened => {
                for (id, n) in self.map.iter_mut() {
                    match id.as_str() {
                        "bounty_npc"    => { n.safety -= 15; n.belonging += 5; } // 獵手興奮又緊張
                        "merchant"      => { n.safety -= 12; }
                        "village_chief" => { n.safety -= 10; }
                        _               => { n.safety -= 5; }
                    }
                    n.clamp_all();
                }
            }
            NeedsEvent::HordeArriving => {
                for n in self.map.values_mut() {
                    n.safety -= 25; // 最大威脅，全員安全感大跌
                    n.clamp_all();
                }
            }
            NeedsEvent::HordeRepelled => {
                for (id, n) in self.map.iter_mut() {
                    n.safety += 20; // 社群勝利，安全感回升
                    match id.as_str() {
                        "village_chief" => { n.belonging += 15; n.prosperity += 5; }
                        "bounty_npc"    => { n.belonging += 12; }
                        _               => { n.belonging += 5; }
                    }
                    n.clamp_all();
                }
            }
            NeedsEvent::QuestCompleted => {
                for (id, n) in self.map.iter_mut() {
                    match id.as_str() {
                        "village_chief" => { n.belonging += 12; n.prosperity += 5; }
                        "farm_fair_npc" => { n.belonging += 10; }
                        _               => { n.belonging += 5; }
                    }
                    n.clamp_all();
                }
            }
            NeedsEvent::VillageFestival => {
                for (id, n) in self.map.iter_mut() {
                    match id.as_str() {
                        "village_chief"  => { n.belonging += 20; n.prosperity += 10; }
                        "farm_fair_npc"  => { n.belonging += 18; }
                        "expedition_npc" => { n.belonging += 15; }
                        "merchant"       => { n.belonging += 10; n.prosperity += 15; }
                        _                => { n.belonging += 8;  n.prosperity += 3; }
                    }
                    n.clamp_all();
                }
            }
            NeedsEvent::EliteSlain => {
                for (id, n) in self.map.iter_mut() {
                    n.safety += 8;
                    match id.as_str() {
                        "bounty_npc"    => { n.belonging += 15; }
                        "village_chief" => { n.belonging += 10; }
                        _               => { n.belonging += 3; }
                    }
                    n.clamp_all();
                }
            }
        }
    }

    /// 玩家與商人完成交易——提升商人繁榮感。
    /// `is_sell`=true 表示玩家賣給商人（商人收購）；false 表示玩家向商人購買。
    pub fn apply_trade(&mut self, is_sell: bool) {
        if let Some(n) = self.map.get_mut("merchant") {
            if is_sell { n.prosperity += 4; } else { n.prosperity += 2; }
            n.clamp_all();
        }
    }

    /// 故鄉茶棚出爐熱茶（ROADMAP 641，禱告驅動）：給全鎮 NPC 一小份**歸屬暖意**（小幅回暖
    /// 歸屬感，只動 belonging、不碰 safety/prosperity），夾在 0~100。應露娜「街角熱茶暖身、
    /// 市集找到新朋友」之禱——一盞熱茶把疏離的人心稍稍拉近。比整場村慶輕得多（見 [`TEA_WARMTH`]）。
    /// 回傳實際因此回暖（belonging 確有上升、非已封頂）的 NPC 數，供記錄／測試。純正向、確定性。
    pub fn warm_community(&mut self) -> u32 {
        let mut warmed = 0;
        for n in self.map.values_mut() {
            let before = n.belonging;
            n.belonging = (n.belonging + TEA_WARMTH).clamp(0, 100);
            if n.belonging > before {
                warmed += 1;
            }
        }
        warmed
    }

    /// 鎮民互助分享（ROADMAP 369）：調整指定 NPC 的繁榮感（正=回升、負=勻出），夾在 0~100。
    /// 未知 NPC 不動（邊界安全）。
    pub fn adjust_prosperity(&mut self, npc_id: &str, delta: i32) {
        if let Some(n) = self.map.get_mut(npc_id) {
            n.prosperity += delta;
            n.clamp_all();
        }
    }

    /// 所有 NPC 需求向基線緩慢靠近（由 game.rs 每 DECAY_INTERVAL_SECS 呼叫一次）。
    pub fn tick_decay_all(&mut self) {
        for n in self.map.values_mut() {
            n.decay_toward_baseline();
        }
    }

    /// 取得指定 NPC 的心情 prompt 段落（未知 NPC 回空字串，不汙染 prompt）。
    pub fn to_prompt_section(&self, npc_id: &str) -> String {
        self.map.get(npc_id).map(|n| n.to_prompt_section()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_values_in_range() {
        let s = NpcNeedsState::new();
        for (id, n) in &s.map {
            assert!(n.safety >= 0 && n.safety <= 100, "{id} safety out of range");
            assert!(n.belonging >= 0 && n.belonging <= 100, "{id} belonging out of range");
            assert!(n.prosperity >= 0 && n.prosperity <= 100, "{id} prosperity out of range");
        }
    }

    #[test]
    fn all_seven_npcs_initialized() {
        let s = NpcNeedsState::new();
        for id in &["merchant", "workshop_npc", "bounty_npc", "expedition_npc",
                    "procurement_npc", "farm_fair_npc", "village_chief"] {
            assert!(s.map.contains_key(*id), "{id} 應有初始需求值");
        }
    }

    #[test]
    fn horde_arriving_drops_all_safety() {
        let mut s = NpcNeedsState::new();
        let prev = s.get("merchant").safety;
        s.apply_world_event(NeedsEvent::HordeArriving);
        assert!(s.get("merchant").safety < prev, "HordeArriving 應降安全感");
        assert!(s.get("village_chief").safety < 55, "里長安全感也應下降");
    }

    #[test]
    fn horde_repelled_raises_safety_and_belonging() {
        let mut s = NpcNeedsState::new();
        s.apply_world_event(NeedsEvent::HordeArriving);
        let after_arrive = s.get("merchant").safety;
        s.apply_world_event(NeedsEvent::HordeRepelled);
        assert!(s.get("merchant").safety > after_arrive, "打退後安全感應回升");
        assert!(s.get("village_chief").belonging > 70, "里長歸屬感應大升");
    }

    #[test]
    fn festival_raises_chief_belonging_and_merchant_prosperity() {
        let mut s = NpcNeedsState::new();
        let prev_b = s.get("village_chief").belonging;
        let prev_p = s.get("merchant").prosperity;
        s.apply_world_event(NeedsEvent::VillageFestival);
        assert!(s.get("village_chief").belonging > prev_b, "節慶應升里長歸屬感");
        assert!(s.get("merchant").prosperity > prev_p, "節慶應升商人繁榮感");
    }

    #[test]
    fn tea_stall_warms_belonging_only() {
        // 茶棚出爐：全鎮歸屬感小幅回暖，但 safety/prosperity 不動（只給一份歸屬暖意）。
        let mut s = NpcNeedsState::new();
        let prev_b = s.get("merchant").belonging;
        let prev_s = s.get("merchant").safety;
        let prev_p = s.get("merchant").prosperity;
        let warmed = s.warm_community();
        assert!(warmed >= 1, "至少有 NPC 因熱茶回暖歸屬感");
        assert_eq!(s.get("merchant").belonging, (prev_b + TEA_WARMTH).min(100), "歸屬感回暖 TEA_WARMTH");
        assert_eq!(s.get("merchant").safety, prev_s, "茶棚不動安全感");
        assert_eq!(s.get("merchant").prosperity, prev_p, "茶棚不動繁榮感");
    }

    #[test]
    fn tea_stall_warmth_clamps_at_ceiling() {
        // 已封頂的歸屬感不再上升，也不計入回暖數（邊界安全）。
        let mut s = NpcNeedsState::new();
        for _ in 0..40 {
            s.warm_community(); // 反覆出爐把全鎮歸屬感推到 100
        }
        assert_eq!(s.get("village_chief").belonging, 100, "反覆回暖應封頂於 100");
        let warmed = s.warm_community();
        assert_eq!(warmed, 0, "全員已封頂時不再有人回暖");
    }

    #[test]
    fn trade_sell_raises_merchant_prosperity() {
        let mut s = NpcNeedsState::new();
        let prev = s.get("merchant").prosperity;
        s.apply_trade(true);
        assert_eq!(s.get("merchant").prosperity, (prev + 4).min(100), "賣出 +4");
    }

    #[test]
    fn trade_buy_raises_merchant_prosperity() {
        let mut s = NpcNeedsState::new();
        let prev = s.get("merchant").prosperity;
        s.apply_trade(false);
        assert_eq!(s.get("merchant").prosperity, (prev + 2).min(100), "購買 +2");
    }

    #[test]
    fn decay_moves_toward_baseline() {
        let mut n = NpcNeeds { safety: 80, belonging: 20, prosperity: 50 };
        n.decay_toward_baseline();
        assert_eq!(n.safety, 79);
        assert_eq!(n.belonging, 21);
        assert_eq!(n.prosperity, 50, "基線值應不動");
    }

    #[test]
    fn clamping_prevents_out_of_range() {
        let mut s = NpcNeedsState::new();
        for _ in 0..30 { s.apply_world_event(NeedsEvent::HordeArriving); }
        assert!(s.get("merchant").safety >= 0, "安全感不應低於 0");
    }

    #[test]
    fn prompt_section_not_empty_for_known_npc() {
        let s = NpcNeedsState::new();
        let section = s.to_prompt_section("merchant");
        assert!(!section.is_empty());
        assert!(section.contains("安全感"));
        assert!(section.contains("歸屬感"));
        assert!(section.contains("繁榮感"));
    }

    #[test]
    fn prompt_section_empty_for_unknown_npc() {
        let s = NpcNeedsState::new();
        assert!(s.to_prompt_section("不存在").is_empty());
    }

    // ── ROADMAP 554：低需求偵測 + 玩家撫慰 ──────────────────────────────
    #[test]
    fn lowest_low_need_none_when_all_calm() {
        // 三大需求皆在門檻以上 → 不需要被關心。
        let n = NpcNeeds { safety: 50, belonging: 50, prosperity: 50 };
        assert_eq!(n.lowest_low_need(), None);
        // 恰在門檻（40）也算安穩（嚴格小於才算偏低）。
        let edge = NpcNeeds { safety: WORRY_THRESHOLD, belonging: WORRY_THRESHOLD, prosperity: WORRY_THRESHOLD };
        assert_eq!(edge.lowest_low_need(), None);
    }

    #[test]
    fn lowest_low_need_picks_single_low() {
        let n = NpcNeeds { safety: 60, belonging: 25, prosperity: 55 };
        assert_eq!(n.lowest_low_need(), Some(NeedKind::Belonging));
    }

    #[test]
    fn lowest_low_need_picks_the_lowest_among_several() {
        // 安全 35、繁榮 20 都偏低 → 取更低的繁榮。
        let n = NpcNeeds { safety: 35, belonging: 55, prosperity: 20 };
        assert_eq!(n.lowest_low_need(), Some(NeedKind::Prosperity));
    }

    #[test]
    fn lowest_low_need_tie_breaks_by_priority() {
        // 三者同低且平手 → 依 安全 ＞ 歸屬 ＞ 繁榮 取安全（安危最要緊）。
        let n = NpcNeeds { safety: 30, belonging: 30, prosperity: 30 };
        assert_eq!(n.lowest_low_need(), Some(NeedKind::Safety));
        // 歸屬與繁榮平手、安全已安穩 → 取歸屬。
        let m = NpcNeeds { safety: 60, belonging: 30, prosperity: 30 };
        assert_eq!(m.lowest_low_need(), Some(NeedKind::Belonging));
    }

    #[test]
    fn comfort_raises_only_target_need_and_clamps() {
        let mut n = NpcNeeds { safety: 30, belonging: 50, prosperity: 50 };
        n.comfort(NeedKind::Safety);
        assert_eq!(n.safety, 30 + COMFORT_STEP, "安全感應被推高一階");
        assert_eq!(n.belonging, 50, "其餘需求不動");
        assert_eq!(n.prosperity, 50, "其餘需求不動");
        // 封頂不溢出 100。
        let mut hi = NpcNeeds { safety: 98, belonging: 50, prosperity: 50 };
        hi.comfort(NeedKind::Safety);
        assert_eq!(hi.safety, 100);
    }

    #[test]
    fn state_comfort_lifts_npc_out_of_worry() {
        let mut s = NpcNeedsState::new();
        // 把獵手安全感打到谷底，確認他開始有心事、撫慰後逐步回升。
        for _ in 0..3 {
            s.apply_world_event(NeedsEvent::HordeArriving); // 每次 safety -25
        }
        let before = s.get("bounty_npc").safety;
        assert!(before < WORRY_THRESHOLD, "獸潮連襲後獵手該不安");
        assert_eq!(s.lowest_low_need("bounty_npc"), Some(NeedKind::Safety));
        s.comfort("bounty_npc", NeedKind::Safety);
        assert_eq!(s.get("bounty_npc").safety, (before + COMFORT_STEP).min(100));
    }

    #[test]
    fn state_comfort_unknown_npc_is_noop() {
        let mut s = NpcNeedsState::new();
        s.comfort("不存在", NeedKind::Safety); // 不 panic、無副作用
        assert_eq!(s.lowest_low_need("不存在"), None);
    }

    // ── ROADMAP 555：交情越深、撫慰越有效 ──────────────────────────────
    #[test]
    fn comfort_amount_is_monotonic_and_backcompat() {
        // 生面孔一階 ＝ 改版前幅度（向後相容）。
        assert_eq!(comfort_amount(0), COMFORT_STEP);
        // 嚴格遞增：交情越深、撫慰越深入人心。
        assert!(comfort_amount(1) > comfort_amount(0));
        assert!(comfort_amount(2) > comfort_amount(1));
        // 越界（>2）夾到最高一階、永不再長。
        assert_eq!(comfort_amount(7), comfort_amount(2));
        assert_eq!(comfort_amount(u8::MAX), comfort_amount(2));
    }

    #[test]
    fn comfort_by_uses_given_amount_and_clamps() {
        let mut n = NpcNeeds { safety: 30, belonging: 50, prosperity: 50 };
        // 老友級幅度（tier 2）把同一谷底拉得比生面孔更高。
        n.comfort_by(NeedKind::Safety, comfort_amount(2));
        assert_eq!(n.safety, 30 + comfort_amount(2), "撫慰幅度依交情放大");
        assert_eq!(n.belonging, 50, "其餘需求不動");
        assert_eq!(n.prosperity, 50, "其餘需求不動");
        // 封頂不溢出 100。
        let mut hi = NpcNeeds { safety: 95, belonging: 50, prosperity: 50 };
        hi.comfort_by(NeedKind::Safety, comfort_amount(2));
        assert_eq!(hi.safety, 100);
    }

    #[test]
    fn state_comfort_by_lifts_more_for_closer_bond() {
        // 同一份谷底：生面孔一句 vs 老友一句，老友把人拉得更高。
        let mut stranger = NpcNeedsState::new();
        let mut friend = NpcNeedsState::new();
        for _ in 0..3 {
            stranger.apply_world_event(NeedsEvent::HordeArriving);
            friend.apply_world_event(NeedsEvent::HordeArriving);
        }
        stranger.comfort_by("bounty_npc", NeedKind::Safety, comfort_amount(0));
        friend.comfort_by("bounty_npc", NeedKind::Safety, comfort_amount(2));
        assert!(
            friend.get("bounty_npc").safety > stranger.get("bounty_npc").safety,
            "老友的關心應比生面孔更能撫平愁緒"
        );
    }
}
