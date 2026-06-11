//! NPC 生命週期（回歸乙太）——AI NPC 成長第 8 步。
//!
//! 每個 NPC 都有壽命計時器。到了老年期，prompt 中注入「感到乙太回流、思考傳承」語境，
//! 讓 NPC 自然談起告別；壽命到時，引擎觸發「回歸乙太」事件：
//!   1. 廣播全服道別公告。
//!   2. 繼承人以新名字登場，繼承前任的記憶（DB 裡已有，以「前任口述歷史」框架注入 prompt）。
//!
//! 完全記憶體模式：重啟後 NPC 重新出生（「世界換季」），不需 migration、不破壞玩家資料。
//!
//! 設計守則（見 VISION_AI_EMERGENT_ECOSYSTEM.md）：
//! - 腦子自由、手有界。
//! - 損失有意義但不創傷（平靜回歸乙太，不是暴死）。
//! - 壽命預設 2 小時真實時間；可用 `BUTFUN_NPC_LIFESPAN_SECS` 覆寫（測試 / 演示用）。

use std::collections::HashMap;

/// NPC 壽命預設（秒，真實時間）。約 2 小時。
pub const LIFESPAN_SECS_DEFAULT: f64 = 7200.0;

/// 進入老年期的壽命分數（80% 壽命後進入老年，有預告、玩家來得及道別）。
pub const ELDER_FRACTION: f64 = 0.80;
/// 退休預告的壽命分數（90% 壽命後廣播「即將回歸」，給玩家最後互動機會）。
pub const RETIREMENT_ANNOUNCE_FRACTION: f64 = 0.90;

/// 取壽命設定（可用環境變數覆寫，方便演示 / 測試縮短）。
pub fn lifespan_secs() -> f64 {
    std::env::var("BUTFUN_NPC_LIFESPAN_SECS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(LIFESPAN_SECS_DEFAULT)
}

// ── 繼承人名池 ──────────────────────────────────────────────────────────────
// 每個 NPC 角色有三個備選名字（第 N 代 = names[generation % 3]）。
// 第 0 代名字與 npc_chat.rs 裡的預設名字相同。
fn successor_names(npc_id: &str) -> &'static [&'static str] {
    match npc_id {
        "merchant"         => &["薇拉", "梅塔", "艾娜"],
        "workshop_npc"     => &["老胡", "師傅阿鎮", "匠人托比"],
        "bounty_npc"       => &["蘭卡", "赤爪岳", "獵師梅德"],
        "expedition_npc"   => &["芙利亞", "探索家托倫", "地圖師萊拉"],
        "procurement_npc"  => &["吉爾", "採購員諾亞", "貿易商薩爾"],
        "farm_fair_npc"    => &["老農", "評審卡特", "農展師芙絲"],
        "village_chief"    => &["凱爾長老", "長老艾登", "守護者泰雅"],
        _                  => &["旅人"],
    }
}

// ── 老年 prompt 注入文字 ─────────────────────────────────────────────────────
/// 老年期注入 NPC system prompt 的語境（第一人稱）。
/// 讓 NPC 自然說出「感覺到乙太回流」、「思考傳承」等主題，不硬寫臺詞。
fn elder_prompt_snippet(npc_id: &str) -> &'static str {
    match npc_id {
        "merchant"         => "你最近感覺乙太在你身上慢慢回流，像是一種呼喚。你開始思考，哪位年輕的拓荒者有潛力繼承你的攤位與生意。",
        "workshop_npc"     => "你的老手骨感到乙太緩緩流動，知道該是傳授技藝、找個得意門生的時候了。",
        "bounty_npc"       => "這麼多年征戰，身上的傷越來越多說話。你開始覺得，是時候找個繼任者扛起告示板了。",
        "expedition_npc"   => "世界這麼大，你卻感到腳步漸漸沉重。你心裡知道，有些土地是要留給更年輕的探索者去踏的。",
        "procurement_npc"  => "你在星際貿易打滾了許多年，最近卻老是夢到回到故鄉乙太之海。也許是時候培植一個接棒的人了。",
        "farm_fair_npc"    => "這片土地上，你的雙手插了幾十年的秧。腰越來越不好，但眼睛還亮——只要能再看一次美好的豐收。",
        "village_chief"    => "你已見過大靜默前後的幾代人。乙太的呼喚越來越清晰，你心中掛念的，是這個村子日後由誰來守護。",
        _                  => "你感到乙太在你身上慢慢回流，有一種難言的平靜。",
    }
}

// ── 道別公告 ────────────────────────────────────────────────────────────────
fn farewell_message(old_display: &str) -> String {
    format!(
        "✨ {}感謝大家這段時間的陪伴，她/他在乙太之光中安詳回歸——繼承者已接手，帶著前任的記憶與囑託重新出發。",
        old_display
    )
}

fn retirement_announcement(display: &str) -> String {
    format!(
        "🕯️ {}感到乙太的呼喚越來越近……她/他會在不久後回歸乙太，請把握與她/他交流的時間。",
        display
    )
}

fn arrival_message(new_display: &str, predecessor_display: &str) -> String {
    format!(
        "🌱 {}正式接任，帶著{}留下的記憶開始新的旅程。歡迎拜訪！",
        new_display, predecessor_display
    )
}

// ── 繼承人 prompt 注入文字（ROADMAP 116：傳說記憶漂移）─────────────────────
/// 新生代 NPC 進入對話時，注入「繼承自前任」的語境。
/// 第 1 代：記憶尚算清晰；第 2~3 代：開始神話化；第 4 代以上：幾乎是遠古傳說。
pub fn heir_context_snippet(predecessor_display: &str) -> String {
    heir_context_with_legend(predecessor_display, 1)
}

pub fn heir_context_with_legend(predecessor_display: &str, generation: u32) -> String {
    match generation {
        1 => format!(
            "你剛接任前任「{}」的職位，繼承了她/他留下的記憶片段。你對這些拓荒者略有印象，但細節模糊，像是聽了很久的口述傳說——有些事蹟被誇大，有些漸漸淡忘。你尊重前任，但也正在形成自己的個性與判斷。",
            predecessor_display
        ),
        2 | 3 => format!(
            "長輩說，很早以前有位叫「{}」的前輩奠定了這份職業的根基，留下許多膾炙人口的傳說。你只在故事裡聽說過她/他的事蹟，細節已如霧中花——但那份精神，你感覺自己還繼承了一些。",
            predecessor_display
        ),
        _ => format!(
            "族中老人偶爾說起，在很久很久以前的某個時代，有位叫「{}」的傳奇人物……那早已是幾代人以前的故事，真假難辨，但名字至今仍被提起，像是這份工作的守護傳說。",
            predecessor_display
        ),
    }
}

// ── 主結構 ──────────────────────────────────────────────────────────────────

/// 單一 NPC 的生命週期狀態。
#[derive(Clone)]
pub struct NpcLifecycleData {
    /// 累計存活秒數（真實時間）。
    pub age_secs: f64,
    /// 這一代的壽命（秒）；每代略有隨機擾動讓時間不一致，但實作上用固定值即可。
    pub lifespan_secs: f64,
    /// 目前世代的顯示名字。
    pub display_name: String,
    /// 世代計數（從 0 開始，回歸乙太後 +1）。
    pub generation: u32,
    /// 老年公告是否已廣播（防重複）。
    pub elder_announced: bool,
    /// 退休公告是否已廣播（防重複）。
    pub retirement_announced: bool,
    /// 繼承人語境（新生代剛接任時為 Some；首次對話後清空）。
    pub heir_context: Option<String>,
    /// 收徒後的徒弟名字（老年期由 game.rs 從居民列表選取並設入）。
    pub apprentice_name: Option<String>,
}

impl NpcLifecycleData {
    fn new(npc_id: &str) -> Self {
        let names = successor_names(npc_id);
        NpcLifecycleData {
            age_secs: 0.0,
            lifespan_secs: lifespan_secs(),
            display_name: names[0].to_string(),
            generation: 0,
            elder_announced: false,
            retirement_announced: false,
            heir_context: None,
            apprentice_name: None,
        }
    }

    fn is_elder(&self) -> bool {
        self.age_secs >= self.lifespan_secs * ELDER_FRACTION
    }

    fn should_announce_retirement(&self) -> bool {
        !self.retirement_announced && self.age_secs >= self.lifespan_secs * RETIREMENT_ANNOUNCE_FRACTION
    }

    fn should_announce_elder(&self) -> bool {
        !self.elder_announced && self.is_elder()
    }

    fn should_retire(&self) -> bool {
        self.age_secs >= self.lifespan_secs
    }

    /// 執行世代交替：重設計時器、更新顯示名。
    fn retire_and_spawn_heir(&mut self, npc_id: &str) -> (String, String) {
        let old_display = self.display_name.clone();
        let old_generation = self.generation;
        self.generation = self.generation.wrapping_add(1);
        let names = successor_names(npc_id);
        let new_display = names[self.generation as usize % names.len()].to_string();
        self.display_name = new_display.clone();
        self.age_secs = 0.0;
        self.lifespan_secs = lifespan_secs();
        self.elder_announced = false;
        self.retirement_announced = false;
        self.apprentice_name = None; // 繼承人自起全新，不帶前任徒弟關係
        // 傳說記憶漂移：世代越深，記憶越像傳說（ROADMAP 116）
        self.heir_context = Some(heir_context_with_legend(&old_display, old_generation + 1));
        (old_display, new_display)
    }
}

/// 引擎觸發的生命週期事件，由 game.rs 轉成廣播與 world_log 紀錄。
pub enum LifecycleEvent {
    /// NPC 進入老年期（80% 壽命）。game.rs 應從居民中選徒弟並呼叫 set_apprentice()。
    ElderPhase { npc_id: String, display: String },
    /// NPC 即將回歸乙太（90% 壽命），廣播告別倒數公告。
    RetirementSoon { npc_id: String, display: String, msg: String },
    /// NPC 正式回歸乙太，繼承人登場。廣播雙則公告 + 記入 world_log。
    RetiredToEther {
        npc_id: String,
        old_display: String,
        new_display: String,
        farewell_msg: String,
        arrival_msg: String,
    },
}

/// 所有 NPC 的生命週期管理器。
pub struct NpcLifecycle {
    npcs: HashMap<String, NpcLifecycleData>,
}

impl NpcLifecycle {
    /// 初始化所有 NPC（從 npc_chat::NPCS 建立）。
    pub fn new() -> Self {
        let mut npcs = HashMap::new();
        for persona in crate::npc_chat::NPCS {
            npcs.insert(persona.id.to_string(), NpcLifecycleData::new(persona.id));
        }
        NpcLifecycle { npcs }
    }

    /// 推進所有 NPC 時鐘，回傳本 tick 觸發的生命週期事件。
    pub fn tick(&mut self, dt_secs: f64) -> Vec<LifecycleEvent> {
        let mut events = Vec::new();

        // 先收集需要操作的 NPC IDs（避免在迭代中借用衝突）
        let npc_ids: Vec<String> = self.npcs.keys().cloned().collect();

        for npc_id in npc_ids {
            let Some(data) = self.npcs.get_mut(&npc_id) else { continue };
            data.age_secs += dt_secs;

            if data.should_announce_elder() {
                data.elder_announced = true;
                events.push(LifecycleEvent::ElderPhase {
                    npc_id: npc_id.clone(),
                    display: data.display_name.clone(),
                });
            }

            if data.should_announce_retirement() {
                data.retirement_announced = true;
                let msg = retirement_announcement(&data.display_name);
                events.push(LifecycleEvent::RetirementSoon {
                    npc_id: npc_id.clone(),
                    display: data.display_name.clone(),
                    msg,
                });
            }

            if data.should_retire() {
                let (old_display, new_display) = data.retire_and_spawn_heir(&npc_id);
                let farewell_msg = farewell_message(&old_display);
                let arrival_msg = arrival_message(&new_display, &old_display);
                events.push(LifecycleEvent::RetiredToEther {
                    npc_id: npc_id.clone(),
                    old_display,
                    new_display,
                    farewell_msg,
                    arrival_msg,
                });
            }
        }

        events
    }

    /// 取得目前世代的顯示名字（用於 NpcReply.display）。
    pub fn current_display(&self, npc_id: &str) -> &str {
        self.npcs
            .get(npc_id)
            .map(|d| d.display_name.as_str())
            .unwrap_or("")
    }

    /// 是否處於老年期（影響 system_prompt 注入）。
    pub fn is_elder(&self, npc_id: &str) -> bool {
        self.npcs.get(npc_id).map(|d| d.is_elder()).unwrap_or(false)
    }

    /// 老年期語境字串，空字串表示非老年。
    /// 若已設定徒弟，額外注入收徒資訊（ROADMAP 116）。
    pub fn elder_context(&self, npc_id: &str) -> String {
        if !self.is_elder(npc_id) {
            return String::new();
        }
        let base = format!("\n\n【生命感悟】{}", elder_prompt_snippet(npc_id));
        let apprentice_part = self.npcs
            .get(npc_id)
            .and_then(|d| d.apprentice_name.as_deref())
            .map(|name| format!("\n你已收 {} 為徒弟，正在將多年心得傾囊相授，盼其日後繼承你的職責。", name))
            .unwrap_or_default();
        base + &apprentice_part
    }

    /// 設定 AI NPC 的徒弟名字（老年期由 game.rs 從居民列表選取後呼叫）。
    pub fn set_apprentice(&mut self, npc_id: &str, apprentice: String) {
        if let Some(data) = self.npcs.get_mut(npc_id) {
            data.apprentice_name = Some(apprentice);
        }
    }

    /// 此 NPC 是否已收徒（避免重複廣播）。
    pub fn has_apprentice(&self, npc_id: &str) -> bool {
        self.npcs
            .get(npc_id)
            .and_then(|d| d.apprentice_name.as_ref())
            .is_some()
    }

    /// 繼承人語境（新生代剛登場時）；呼叫後清空，避免每次對話都重複注入。
    pub fn take_heir_context(&mut self, npc_id: &str) -> Option<String> {
        self.npcs.get_mut(npc_id).and_then(|d| d.heir_context.take())
    }
}

// ── 純邏輯測試 ───────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    /// 建立縮短壽命的測試用 NPC。
    fn test_data() -> NpcLifecycleData {
        NpcLifecycleData {
            age_secs: 0.0,
            lifespan_secs: 100.0,
            display_name: "薇拉".to_string(),
            generation: 0,
            elder_announced: false,
            retirement_announced: false,
            heir_context: None,
            apprentice_name: None,
        }
    }

    #[test]
    fn elder_phase_triggers_at_correct_fraction() {
        let mut d = test_data();
        assert!(!d.is_elder(), "出生時不應是老年");
        d.age_secs = d.lifespan_secs * ELDER_FRACTION;
        assert!(d.is_elder(), "到達老年閾值後應進入老年期");
    }

    #[test]
    fn retirement_triggers_at_lifespan() {
        let mut d = test_data();
        d.age_secs = 99.9;
        assert!(!d.should_retire());
        d.age_secs = 100.0;
        assert!(d.should_retire());
    }

    #[test]
    fn retire_and_spawn_heir_increments_generation_and_resets_age() {
        let mut d = test_data();
        d.age_secs = 100.0;
        let (old, new) = d.retire_and_spawn_heir("merchant");
        assert_eq!(old, "薇拉");
        assert_ne!(new, old, "繼承人名字應與前任不同");
        assert_eq!(d.generation, 1);
        assert_eq!(d.age_secs, 0.0, "交替後年齡重設為 0");
        assert!(!d.elder_announced, "交替後標記清除");
    }

    #[test]
    fn heir_context_set_after_succession() {
        let mut d = test_data();
        d.retire_and_spawn_heir("merchant");
        assert!(d.heir_context.is_some(), "交替後應設繼承人語境");
        let ctx = d.heir_context.as_ref().unwrap();
        assert!(ctx.contains("薇拉"), "繼承人語境應提及前任名字");
    }

    #[test]
    fn generation_cycles_through_name_pool() {
        // merchant 有三個名字，第 3 代應回到第 0 代的名字。
        let names = successor_names("merchant");
        assert_eq!(names.len(), 3);
        let mut d = NpcLifecycleData::new("merchant");
        // 模擬三次退休
        for _ in 0..3 {
            d.retire_and_spawn_heir("merchant");
        }
        // generation == 3，3 % 3 == 0，回到第 0 個名字
        assert_eq!(d.display_name, names[0]);
    }

    #[test]
    fn lifecycle_manager_covers_all_npcs() {
        let lc = NpcLifecycle::new();
        for persona in crate::npc_chat::NPCS {
            let display = lc.current_display(persona.id);
            assert!(!display.is_empty(), "NPC {} 應有顯示名字", persona.id);
        }
    }

    #[test]
    fn tick_produces_events_at_milestones() {
        // 用縮短壽命直接建立受控測試
        let mut lc = NpcLifecycle::new();
        // 強制設定 merchant 到老年前一步
        let merchant = lc.npcs.get_mut("merchant").unwrap();
        merchant.lifespan_secs = 100.0;
        merchant.age_secs = 79.0; // 老年閾值是 80.0

        // tick 2 秒跨過老年閾值
        let events = lc.tick(2.0);
        let has_elder = events.iter().any(|e| matches!(e, LifecycleEvent::ElderPhase { npc_id, .. } if npc_id == "merchant"));
        assert!(has_elder, "應觸發 merchant 老年事件");
    }

    #[test]
    fn set_apprentice_shows_in_elder_context() {
        let mut lc = NpcLifecycle::new();
        // 強制進入老年
        let merchant = lc.npcs.get_mut("merchant").unwrap();
        merchant.lifespan_secs = 100.0;
        merchant.age_secs = 85.0;
        merchant.elder_announced = true;
        // 設定徒弟
        lc.set_apprentice("merchant", "阿花".to_string());
        assert!(lc.has_apprentice("merchant"), "should have apprentice");
        let ctx = lc.elder_context("merchant");
        assert!(ctx.contains("阿花"), "老年語境應包含徒弟名字");
    }

    #[test]
    fn apprentice_cleared_on_retirement() {
        let mut lc = NpcLifecycle::new();
        let merchant = lc.npcs.get_mut("merchant").unwrap();
        merchant.lifespan_secs = 100.0;
        merchant.age_secs = 100.0;
        merchant.apprentice_name = Some("阿花".to_string());
        // 執行退休
        let _events = lc.tick(0.1);
        // 繼承人不應繼承徒弟名字
        assert!(!lc.has_apprentice("merchant"), "退休後徒弟資料應清除");
    }

    #[test]
    fn heir_context_becomes_legend_over_generations() {
        // 第 1 代：直接繼承，提到「剛接任」
        let ctx1 = heir_context_with_legend("薇拉", 1);
        assert!(ctx1.contains("剛接任") || ctx1.contains("記憶片段"), "第 1 代語境應是直接繼承");
        // 第 2 代以上：傳說化，提到「長輩說」或「傳奇」
        let ctx2 = heir_context_with_legend("薇拉", 2);
        assert!(ctx2.contains("長輩說") || ctx2.contains("傳說"), "第 2 代語境應偏向傳說");
        // 第 4 代：更神話，提到「很久很久以前」
        let ctx4 = heir_context_with_legend("薇拉", 4);
        assert!(ctx4.contains("很久很久以前") || ctx4.contains("遠古"), "第 4 代語境應為神話");
    }

    #[test]
    fn retirement_announcement_not_repeated() {
        let mut d = test_data();
        d.age_secs = d.lifespan_secs * RETIREMENT_ANNOUNCE_FRACTION;
        assert!(d.should_announce_retirement());
        d.retirement_announced = true;
        assert!(!d.should_announce_retirement(), "已廣播後不應重複");
    }

    #[test]
    fn elder_context_non_empty_for_known_npcs() {
        let lc = NpcLifecycle::new();
        for persona in crate::npc_chat::NPCS {
            // 老年語境不為空
            let snippet = elder_prompt_snippet(persona.id);
            assert!(!snippet.is_empty(), "NPC {} 老年語境不得為空", persona.id);
        }
    }
}
