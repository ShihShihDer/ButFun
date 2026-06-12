//! NPC 主動世界評論（ROADMAP 68：AI NPC 成長第 10 步）。
//!
//! 重大世界事件發生時，相關 NPC 自動在聊天頻道表達看法——
//! 讓 NPC 從「被動回應玩家」進化到「主動關注世界」。
//!
//! 設計鐵律：
//! - 腦子自由、手有界：LLM 只生成文字；引擎管廣播與冷卻。
//! - 降級安全：LLM 未啟用 / 連不到 → 罐頭評論，遊戲不壞。
//! - 非同步：生成一律 tokio::spawn，永不阻塞 15Hz 迴圈。
//! - 防洗頻：每個 NPC 每 10 分鐘最多主動說一次話。

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 每個 NPC 主動評論的冷卻時間（秒）。防止 NPC 把聊天頻道洗爆。
pub const PROACTIVE_COOLDOWN_SECS: u64 = 600; // 10 分鐘

/// 世界事件類型（從 game.rs 傳入，觸發 NPC 主動評論）。
#[derive(Debug, Clone)]
pub enum WorldEventKind {
    /// 宇宙裂縫開啟，附位置描述。
    RiftOpened { desc: String },
    /// 獸潮集結攻城，附城門位置。
    HordeArriving { site: String },
    /// 獸潮被玩家打退。
    HordeRepelled { site: String },
    /// 全服社群任務完成。
    QuestComplete { name: String },
    /// 兇名精英被討伐。
    EliteSlain { name: String, slayer: String },
    /// 村落節慶開始（凱爾長老辦的）。
    VillageFestival,
    /// 巢穴 Alpha 首領湧現（ROADMAP 171）——附巢穴名稱與怪物種類。
    AlphaEmergent { colony_name: String, kind_name: String },
    /// Alpha 領地爭奪結算（ROADMAP 171）——附勝負巢穴名。
    AlphaClashResult { winner_colony: String, loser_colony: String },
}

impl WorldEventKind {
    /// 給 LLM / 罐頭降級用的事件文字描述。
    pub fn description(&self) -> String {
        match self {
            WorldEventKind::RiftOpened { desc } => {
                format!("宇宙裂縫在{}開啟，裂縫守護者現身", desc)
            }
            WorldEventKind::HordeArriving { site } => {
                format!("獸潮大軍聚集在{}城門外，即將發動衝擊", site)
            }
            WorldEventKind::HordeRepelled { site } => {
                format!("獸潮已被拓荒者在{}打退，村落暫時安全", site)
            }
            WorldEventKind::QuestComplete { name } => {
                format!("全服社群任務「{}」圓滿完成", name)
            }
            WorldEventKind::EliteSlain { name, slayer } => {
                format!("兇名精英「{}」被拓荒者「{}」英勇討伐", name, slayer)
            }
            WorldEventKind::VillageFestival => {
                "村落節慶開始！全村 EXP 加成 30%".to_string()
            }
            WorldEventKind::AlphaEmergent { colony_name, kind_name } => {
                format!("{colony_name} 的 {kind_name} 族群達到巔峰，Alpha 首領湧現稱霸領地")
            }
            WorldEventKind::AlphaClashResult { winner_colony, loser_colony } => {
                format!("{winner_colony} 的 Alpha 擊潰了 {loser_colony} 的 Alpha，稱霸野外區域")
            }
        }
    }

    /// 對此事件感興趣的 NPC ID 列表（按優先順序排列）。
    /// 每次只選第一個不在冷卻中的 NPC 說話，避免同時轟炸聊天頻道。
    pub fn reacting_npcs(&self) -> &'static [&'static str] {
        match self {
            WorldEventKind::RiftOpened { .. } => &["bounty_npc", "merchant"],
            WorldEventKind::HordeArriving { .. } => {
                &["village_chief", "bounty_npc", "merchant"]
            }
            WorldEventKind::HordeRepelled { .. } => &["village_chief", "bounty_npc"],
            WorldEventKind::QuestComplete { .. } => &["village_chief", "farm_fair_npc"],
            WorldEventKind::EliteSlain { .. } => &["bounty_npc", "village_chief"],
            WorldEventKind::VillageFestival => {
                &["village_chief", "farm_fair_npc", "expedition_npc"]
            }
            WorldEventKind::AlphaEmergent { .. } => &["bounty_npc", "village_chief"],
            WorldEventKind::AlphaClashResult { .. } => &["bounty_npc", "merchant"],
        }
    }
}

/// NPC 對特定世界事件的罐頭評論（LLM 未啟用或失敗時的降級）。
pub fn canned_reaction(npc_id: &str, event: &WorldEventKind) -> String {
    match (npc_id, event) {
        ("bounty_npc", WorldEventKind::RiftOpened { desc }) => {
            format!("〔獵手蘭卡〕裂縫在{}又開了！想博名聲的快去，別空手上陣。", desc)
        }
        ("merchant", WorldEventKind::RiftOpened { .. }) => {
            "〔薇拉〕裂縫守護者出現了！武器、藥水我這邊都有，出發前補好裝備。".to_string()
        }
        ("village_chief", WorldEventKind::HordeArriving { site }) => {
            format!("〔凱爾長老〕拓荒者們！{}的守衛需要你們——勇氣是我們最強的護盾。", site)
        }
        ("bounty_npc", WorldEventKind::HordeArriving { .. }) => {
            "〔獵手蘭卡〕獸潮！這不是演習。有實力的出去，沒把握的守好城牆。".to_string()
        }
        ("merchant", WorldEventKind::HordeArriving { .. }) => {
            "〔薇拉〕獸潮來了！大家快來補給——這時候要打仗了！".to_string()
        }
        ("village_chief", WorldEventKind::HordeRepelled { site }) => {
            format!(
                "〔凱爾長老〕{}的獸潮被打退了！英勇的拓荒者們，老朽代表全村向你們致謝。",
                site
            )
        }
        ("bounty_npc", WorldEventKind::HordeRepelled { .. }) => {
            "〔獵手蘭卡〕打退了！那就是平時苦練的意義。好樣的。".to_string()
        }
        ("village_chief", WorldEventKind::QuestComplete { name }) => {
            format!("〔凱爾長老〕「{}」完成了！每一份貢獻，老朽都記在心裡。", name)
        }
        ("farm_fair_npc", WorldEventKind::QuestComplete { .. }) => {
            "〔評審老農〕大夥合力完成任務了！這團結勁，比老頭我最好的收成還珍貴。".to_string()
        }
        ("bounty_npc", WorldEventKind::EliteSlain { name, slayer }) => {
            format!(
                "〔獵手蘭卡〕「{}」終於倒下了！「{}」，今天的獵人頭銜，你當之無愧。",
                name, slayer
            )
        }
        ("village_chief", WorldEventKind::EliteSlain { slayer, .. }) => {
            format!(
                "〔凱爾長老〕感謝「{}」為村落除了一患。你的事跡，將被後人傳頌。",
                slayer
            )
        }
        ("village_chief", WorldEventKind::VillageFestival) => {
            "〔凱爾長老〕節慶開始！讓喜悅充滿每條街道——你們都是這個村落的驕傲。".to_string()
        }
        ("farm_fair_npc", WorldEventKind::VillageFestival) => {
            "〔評審老農〕節慶！老頭去搬些新鮮農產品，慶典少不了好食材！".to_string()
        }
        ("expedition_npc", WorldEventKind::VillageFestival) => {
            "〔探勘員芙利亞〕節慶！我從遠方帶回的故事，今晚全說個夠！".to_string()
        }
        // ROADMAP 171：Alpha 湧現 NPC 反應
        ("bounty_npc", WorldEventKind::AlphaEmergent { colony_name, kind_name }) => {
            format!("〔獵手蘭卡〕{colony_name} 出現 Alpha 霸主了！{kind_name} 族群巔峰——有膽的去挑戰，獎勵豐厚。")
        }
        ("village_chief", WorldEventKind::AlphaEmergent { colony_name, .. }) => {
            format!("〔凱爾長老〕{colony_name} 地區霸主降臨！勇者們謹慎行事，也可趁機立功。")
        }
        // ROADMAP 171：Alpha 領地爭奪結果 NPC 反應
        ("bounty_npc", WorldEventKind::AlphaClashResult { winner_colony, loser_colony }) => {
            format!("〔獵手蘭卡〕{winner_colony} Alpha 稱霸！{loser_colony} 元氣大傷——殘血霸主此時最好打，把握機會！")
        }
        ("merchant", WorldEventKind::AlphaClashResult { winner_colony, .. }) => {
            format!("〔薇拉〕{winner_colony} Alpha 獲勝了，還在殘血！武器藥水我這裡備著，快去收割。")
        }
        _ => {
            let display = crate::npc_chat::find_npc(npc_id)
                .map(|n| n.display)
                .unwrap_or(npc_id);
            format!("〔{}〕「{}」——世界又不一樣了。", display, event.description())
        }
    }
}

/// NPC 主動評論的冷卻追蹤器（記憶體模式，重啟清空）。
#[derive(Default)]
pub struct NpcProactiveCooldowns {
    last_sent: HashMap<String, Instant>,
}

impl NpcProactiveCooldowns {
    pub fn new() -> Self {
        Self::default()
    }

    /// 若 npc_id 的冷卻已過，記錄本次時間並回 true；仍在冷卻中回 false。
    pub fn check_and_mark(&mut self, npc_id: &str, now: Instant) -> bool {
        if let Some(&last) = self.last_sent.get(npc_id) {
            if now.duration_since(last) < Duration::from_secs(PROACTIVE_COOLDOWN_SECS) {
                return false;
            }
        }
        self.last_sent.insert(npc_id.to_string(), now);
        true
    }
}

/// 從事件候選 NPC 中選出第一個不在冷卻中的，回傳其靜態 id；無可選時回 None。
pub fn pick_reacting_npc(
    event: &WorldEventKind,
    cooldowns: &mut NpcProactiveCooldowns,
    now: Instant,
) -> Option<&'static str> {
    for &npc_id in event.reacting_npcs() {
        if cooldowns.check_and_mark(npc_id, now) {
            return Some(npc_id);
        }
    }
    None
}

/// 非同步生成 NPC 主動評論文字（LLM 或降級罐頭）。
/// 呼叫方（game.rs）以 tokio::spawn 包裝，不阻塞 15Hz 迴圈。
pub async fn generate_proactive_reaction(
    npc_id: &'static str,
    event: WorldEventKind,
) -> String {
    let fallback = canned_reaction(npc_id, &event);
    let Some(npc) = crate::npc_chat::find_npc(npc_id) else {
        return fallback;
    };
    let event_desc = event.description();
    let sys = format!(
        "你是 ButFun 世界裡的 NPC，{}。\n\
         ButFun 是蒸汽龐克太空歌劇療癒世界；城內有黃銅城牆保護、城外有危險也有探索機會。\n\
         你剛聽到一則世界消息：「{}」\n\
         請以你的口吻，用 25 字以內在村子聊天頻道說一句反應，以「〔{}〕」開頭。\
         直接輸出那句話，不加說明或引號。",
        npc.persona,
        event_desc,
        npc.display,
    );
    match crate::npc_chat::raw_llm_call(&sys, "").await {
        Some(t) if !t.is_empty() => t,
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canned_reaction_never_empty() {
        let events = vec![
            WorldEventKind::RiftOpened { desc: "東北方".to_string() },
            WorldEventKind::HordeArriving { site: "主城".to_string() },
            WorldEventKind::HordeRepelled { site: "主城".to_string() },
            WorldEventKind::QuestComplete { name: "狩獵任務".to_string() },
            WorldEventKind::EliteSlain {
                name: "兇名狼王".to_string(),
                slayer: "勇者".to_string(),
            },
            WorldEventKind::VillageFestival,
            WorldEventKind::AlphaEmergent {
                colony_name: "廢料無人機陣".to_string(),
                kind_name: "廢料無人機".to_string(),
            },
            WorldEventKind::AlphaClashResult {
                winner_colony: "水晶傀儡巢穴".to_string(),
                loser_colony: "蘑菇窟".to_string(),
            },
        ];
        for event in &events {
            for &npc_id in event.reacting_npcs() {
                let r = canned_reaction(npc_id, event);
                assert!(!r.is_empty(), "npc={npc_id} 事件={}", event.description());
            }
        }
    }

    #[test]
    fn cooldown_blocks_repeat_at_same_instant() {
        let mut cd = NpcProactiveCooldowns::new();
        let now = Instant::now();
        assert!(cd.check_and_mark("bounty_npc", now));
        assert!(!cd.check_and_mark("bounty_npc", now), "同一瞬間第二次應被擋住");
    }

    #[test]
    fn cooldown_allows_after_expiry() {
        let mut cd = NpcProactiveCooldowns::new();
        let t0 = Instant::now();
        cd.check_and_mark("bounty_npc", t0);
        let t1 = t0 + Duration::from_secs(PROACTIVE_COOLDOWN_SECS + 1);
        assert!(cd.check_and_mark("bounty_npc", t1), "冷卻過後應可再發言");
    }

    #[test]
    fn pick_reacting_npc_skips_cooled_down() {
        let mut cd = NpcProactiveCooldowns::new();
        let now = Instant::now();
        let event = WorldEventKind::RiftOpened { desc: "東方".to_string() };
        // bounty_npc 先進入冷卻
        cd.check_and_mark("bounty_npc", now);
        // 第二次選 → bounty_npc 被擋，應選到 merchant
        let picked = pick_reacting_npc(&event, &mut cd, now);
        assert_eq!(picked, Some("merchant"), "第一個被擋後應選到下一個 merchant");
    }

    #[test]
    fn pick_reacting_npc_returns_none_when_all_cooled() {
        let mut cd = NpcProactiveCooldowns::new();
        let now = Instant::now();
        let event = WorldEventKind::RiftOpened { desc: "西方".to_string() };
        for &npc_id in event.reacting_npcs() {
            cd.check_and_mark(npc_id, now);
        }
        assert!(
            pick_reacting_npc(&event, &mut cd, now).is_none(),
            "全員冷卻中應回 None"
        );
    }

    #[test]
    fn description_not_empty_for_all_variants() {
        let events = vec![
            WorldEventKind::RiftOpened { desc: "X".to_string() },
            WorldEventKind::HordeArriving { site: "Y".to_string() },
            WorldEventKind::HordeRepelled { site: "Z".to_string() },
            WorldEventKind::QuestComplete { name: "N".to_string() },
            WorldEventKind::EliteSlain { name: "A".to_string(), slayer: "B".to_string() },
            WorldEventKind::VillageFestival,
            WorldEventKind::AlphaEmergent { colony_name: "C".to_string(), kind_name: "K".to_string() },
            WorldEventKind::AlphaClashResult { winner_colony: "W".to_string(), loser_colony: "L".to_string() },
        ];
        for e in events {
            assert!(!e.description().is_empty(), "description 不應為空");
        }
    }

    #[test]
    fn cooldown_constant_is_sane() {
        assert!(PROACTIVE_COOLDOWN_SECS >= 60, "至少 1 分鐘冷卻，防洗頻");
        assert!(PROACTIVE_COOLDOWN_SECS <= 3600, "不超過 1 小時，保持活躍感");
    }
}
