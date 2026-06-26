//! 自主 agent 決策（P0 純邏輯地基）。
//!
//! 目標：讓 NPC 之後能有「LLM 腦」自己決定下一個行動——但這一步**只建純邏輯**，
//! 不碰 game.rs、不碰 15Hz 遊戲迴圈、不持有任何鎖、不動玩家資料。
//! live 接線（讓居民真的照決策走路/採集）是後續另一步。
//!
//! 設計鐵律（延續 npc_chat 的「腦子自由、手有界」與降級精神）：
//! - **純資料進出**：感知（`SenseInput`）是純資料，決策（`AgentDecision`）也是純資料，
//!   本模組碰不到 AppState、不碰任何 std RwLock（呼應 prod 死鎖鐵律）。
//! - **降級**：沒有 LLM（關閉/連不到/亂答）也要能動——`canned_action` 規則化後備保證「永遠有動作」。
//! - **穩健解析**：LLM 回什麼都不准 panic；認不出就保守回 `Idle`。
//! - 面向玩家/餵 LLM 的字串集中在本檔，方便日後在地化替換。
//!
//! 純邏輯都抽成可測函式（`build_think_prompt` / `parse_action` / `canned_action`），
//! `npc_think` 只是把它們和既有 LLM 路由黏起來的 async glue。

// P0 純邏輯地基，live 接線（game.rs）是後續步驟，暫時整模組允許未使用。
#![allow(dead_code)]

/// agent 決定要做的單一行動。
///
/// 刻意保持**小而封閉**：只有遊戲迴圈日後能安全執行的幾種「手」。
/// 新增動作時，記得同步擴 `parse_action`（解析）與 `canned_action`（後備）。
#[derive(Debug, Clone, PartialEq)]
pub enum AgentAction {
    /// 走向世界座標 (x, y)。
    MoveTo { x: f32, y: f32 },
    /// 就地採集附近的資源節點。
    Gather,
    /// 與某個對象搭話（target 是玩家/NPC 的識別字串）。
    Talk { target: String },
    /// 發呆/原地待機（也是所有「認不出」情況的安全預設）。
    Idle,
}

/// 一次完整的決策：要做什麼（action）、想對外說的話（say）、為什麼這麼決定（reason）。
///
/// `say` / `reason` 都可能為空字串（後備決策或 LLM 沒給時）。
#[derive(Debug, Clone, PartialEq)]
pub struct AgentDecision {
    pub action: AgentAction,
    /// agent 想說出口的一句話（可空）。
    pub say: String,
    /// agent 給自己的決策理由（可空，主要供除錯/日後思想泡泡）。
    pub reason: String,
}

impl AgentDecision {
    /// 包一個 `Idle` 決策（解析失敗/認不出時的安全預設）。
    pub fn idle() -> Self {
        Self { action: AgentAction::Idle, say: String::new(), reason: String::new() }
    }

    /// 方便建構：帶上行動與理由（say 留空）。
    pub fn new(action: AgentAction, say: impl Into<String>, reason: impl Into<String>) -> Self {
        Self { action, say: say.into(), reason: reason.into() }
    }
}

/// 附近一個可採集資源節點的精簡描述（純資料）。
#[derive(Debug, Clone, PartialEq)]
pub struct NearbyNode {
    /// 節點種類的可讀標籤（例：「樹」「礦脈」）。
    pub kind: String,
    pub x: f32,
    pub y: f32,
}

/// 附近一位玩家的精簡描述（純資料）。
#[derive(Debug, Clone, PartialEq)]
pub struct NearbyPlayer {
    /// 玩家識別字串（名字或 id，供 Talk 的 target 用）。
    pub name: String,
    pub x: f32,
    pub y: f32,
}

/// agent 此刻「感知到的情境」——餵給決策的所有資訊，純資料、無 IO。
///
/// 由 game.rs（日後接線時）從世界狀態擷取快照填入；本模組只讀不寫。
#[derive(Debug, Clone, PartialEq)]
pub struct SenseInput {
    /// 自身座標。
    pub x: f32,
    pub y: f32,
    /// 自身生命值與上限。
    pub hp: i32,
    pub max_hp: i32,
    /// 自身「能量/活力」概念值（0~100，低=想休息或採集補給）。
    pub energy: i32,
    /// 心情值（0~100，影響語氣，純情境用）。
    pub mood: i32,
    /// 需求驅力摘要字串（可直接塞 npc_needs 的 to_prompt_section()，可空）。
    pub needs_summary: String,
    /// 附近玩家清單。
    pub nearby_players: Vec<NearbyPlayer>,
    /// 附近可採集節點清單。
    pub nearby_nodes: Vec<NearbyNode>,
    /// 世界近況一句話（引擎事件，可空）。
    pub world_news: String,
}

impl SenseInput {
    /// 方便測試/接線的最小建構：只給自身狀態，其餘留空。
    pub fn new(x: f32, y: f32, hp: i32, max_hp: i32, energy: i32, mood: i32) -> Self {
        Self {
            x,
            y,
            hp,
            max_hp,
            energy,
            mood,
            needs_summary: String::new(),
            nearby_players: Vec::new(),
            nearby_nodes: Vec::new(),
            world_news: String::new(),
        }
    }

    /// 加上附近玩家（builder 風格，方便測試）。
    pub fn with_players(mut self, players: Vec<NearbyPlayer>) -> Self {
        self.nearby_players = players;
        self
    }

    /// 加上附近節點（builder 風格，方便測試）。
    pub fn with_nodes(mut self, nodes: Vec<NearbyNode>) -> Self {
        self.nearby_nodes = nodes;
        self
    }

    /// 加上需求摘要（builder 風格）。
    pub fn with_needs(mut self, needs: impl Into<String>) -> Self {
        self.needs_summary = needs.into();
        self
    }

    /// 加上世界近況（builder 風格）。
    pub fn with_world_news(mut self, news: impl Into<String>) -> Self {
        self.world_news = news.into();
        self
    }
}

/// 能量低於此值，視為「需要去採集/補給」的門檻。
const LOW_ENERGY: i32 = 35;

/// 組出要餵 LLM 的決策 prompt（純函式、可測）。
///
/// 關鍵：**要求 LLM 只回一個 JSON 物件**描述單一行動，欄位 action/target/say/reason，
/// 並把可選的 action 值與情境攤平在 prompt 裡，降低亂答機率。
pub fn build_think_prompt(sense: &SenseInput, persona: &str) -> String {
    // 附近玩家攤平成可讀清單。
    let players = if sense.nearby_players.is_empty() {
        "（附近沒有其他人）".to_string()
    } else {
        sense
            .nearby_players
            .iter()
            .map(|p| format!("「{}」在 ({:.0}, {:.0})", p.name, p.x, p.y))
            .collect::<Vec<_>>()
            .join("、")
    };

    // 附近節點攤平。
    let nodes = if sense.nearby_nodes.is_empty() {
        "（附近沒有可採集的資源）".to_string()
    } else {
        sense
            .nearby_nodes
            .iter()
            .map(|n| format!("{}在 ({:.0}, {:.0})", n.kind, n.x, n.y))
            .collect::<Vec<_>>()
            .join("、")
    };

    let needs = if sense.needs_summary.trim().is_empty() {
        String::new()
    } else {
        format!("\n你此刻的內心狀態：{}", sense.needs_summary.trim())
    };

    let news = if sense.world_news.trim().is_empty() {
        String::new()
    } else {
        format!("\n世界近況：{}", sense.world_news.trim())
    };

    format!(
        "你是一位住在 ButFun 世界裡的角色，正在決定下一步要做什麼。\n\
        【你的人設】{persona}\n\
        【你此刻的狀態】生命 {hp}/{max_hp}・活力 {energy}/100・心情 {mood}/100，\
        你站在世界座標 ({x:.0}, {y:.0})。{needs}{news}\n\
        【附近的人】{players}\n\
        【附近可採集的資源】{nodes}\n\n\
        請從以下行動中**選一個**，並**只輸出一個 JSON 物件**，不要有任何多餘文字或說明：\n\
        - move：走向某個座標。需要 \"target\": {{ \"x\": 數字, \"y\": 數字 }}。\n\
        - gather：就地採集附近的資源。\n\
        - talk：和某個人搭話。需要 \"target\": \"對方的名字\"。\n\
        - idle：原地休息發呆。\n\n\
        JSON 格式範例：{{ \"action\": \"talk\", \"target\": \"薇拉\", \"say\": \"嗨，今天生意好嗎？\", \"reason\": \"附近有熟人，想打招呼\" }}\n\
        欄位說明：action 必填（上列其一）；target 視 action 而定；\
        say 是你想說出口的一句話（繁體中文，沒有就空字串）；reason 是你的決策理由（繁體中文，簡短）。\n\
        只輸出 JSON，不要 markdown 圍欄、不要任何其他文字。",
        persona = persona,
        hp = sense.hp,
        max_hp = sense.max_hp,
        energy = sense.energy,
        mood = sense.mood,
        x = sense.x,
        y = sense.y,
        needs = needs,
        news = news,
        players = players,
        nodes = nodes,
    )
}

/// 從可能夾雜散文/markdown 圍欄的字串中，盡力抽出一段 JSON 物件文字。
/// 找不到就回 None。純字串處理，不分配多餘記憶體外不做別的。
fn extract_json_blob(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // 取第一個 '{' 到最後一個 '}' 的區間（能同時吃掉 ```json 圍欄與前後散文）。
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(&trimmed[start..=end])
}

/// 從 JSON 值取字串欄位（容忍缺欄位 → 空字串）。
fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").trim().to_string()
}

/// 從 JSON 值取數字座標。容忍 number 或可解析的字串；取不到回 None。
fn num_field(v: &serde_json::Value) -> Option<f32> {
    if let Some(f) = v.as_f64() {
        return Some(f as f32);
    }
    // 容忍被引號包起來的數字字串。
    v.as_str().and_then(|s| s.trim().parse::<f32>().ok())
}

/// 從一個 JSON 物件裡找座標：先看 target.{x,y}，再退而看頂層 x/y。
/// 兩者皆有限值才算成功（NaN / 無窮 / 缺失 → None）。
fn extract_xy(obj: &serde_json::Value) -> Option<(f32, f32)> {
    // 候選來源：target 子物件優先，否則頂層。
    let source = obj.get("target").filter(|t| t.is_object()).unwrap_or(obj);
    let x = source.get("x").and_then(num_field)?;
    let y = source.get("y").and_then(num_field)?;
    if x.is_finite() && y.is_finite() {
        Some((x, y))
    } else {
        None
    }
}

/// **穩健**解析 LLM 回傳成一個決策。
///
/// 容忍：乾淨 JSON、```json 圍欄、夾散文的 JSON、空字串、完全亂答、未知 action、缺欄位、壞座標。
/// 任何無法穩妥對應到一個合法行動的情況 → 保守回 `Idle`（**絕不 panic**）。
pub fn parse_action(llm_output: &str) -> AgentDecision {
    // 1) 抽出 JSON 文字。
    let blob = match extract_json_blob(llm_output) {
        Some(b) => b,
        None => return AgentDecision::idle(),
    };

    // 2) 解析成 Value；壞 JSON → Idle。
    let value: serde_json::Value = match serde_json::from_str(blob) {
        Ok(v) => v,
        Err(_) => return AgentDecision::idle(),
    };
    if !value.is_object() {
        return AgentDecision::idle();
    }

    let say = str_field(&value, "say");
    let reason = str_field(&value, "reason");

    // 3) 讀 action（大小寫/前後空白不敏感）。
    let action_raw = str_field(&value, "action").to_lowercase();
    let action = match action_raw.as_str() {
        "move" | "moveto" | "move_to" | "goto" | "walk" => match extract_xy(&value) {
            Some((x, y)) => AgentAction::MoveTo { x, y },
            // action 是 move 但座標壞/缺 → 保守 Idle。
            None => return AgentDecision { action: AgentAction::Idle, say, reason },
        },
        "gather" | "harvest" | "collect" => AgentAction::Gather,
        "talk" | "speak" | "chat" => {
            let target = str_field(&value, "target");
            if target.is_empty() {
                // 想搭話卻沒對象 → 保守 Idle（仍保留 say/reason）。
                return AgentDecision { action: AgentAction::Idle, say, reason };
            }
            AgentAction::Talk { target }
        }
        "idle" | "rest" | "wait" | "" => AgentAction::Idle,
        // 未知 action 值 → 保守 Idle（仍保留 say/reason 供觀察）。
        _ => AgentAction::Idle,
    };

    AgentDecision { action, say, reason }
}

/// **無 LLM 時的規則化後備**（純函式、可測）。
///
/// 保證「沒有 LLM 也能動」（呼應 npc_chat 罐頭精神）。規則由急到緩：
/// 1. 活力低且附近有節點 → 走向最近節點若不在腳邊、否則就地 Gather。
/// 2. 附近有玩家 → Talk 最近的那位。
/// 3. 附近有節點（活力還行）→ Gather。
/// 4. 否則 → Idle。
pub fn canned_action(sense: &SenseInput) -> AgentDecision {
    // 找最近的節點/玩家（以平方距離比較，省一次開根號）。
    let nearest_node = sense
        .nearby_nodes
        .iter()
        .min_by(|a, b| {
            dist2(sense.x, sense.y, a.x, a.y)
                .partial_cmp(&dist2(sense.x, sense.y, b.x, b.y))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let nearest_player = sense
        .nearby_players
        .iter()
        .min_by(|a, b| {
            dist2(sense.x, sense.y, a.x, a.y)
                .partial_cmp(&dist2(sense.x, sense.y, b.x, b.y))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    // 1) 活力低 + 有節點：優先補給。
    if sense.energy <= LOW_ENERGY {
        if let Some(node) = nearest_node {
            return AgentDecision::new(
                AgentAction::Gather,
                String::new(),
                format!("活力剩 {}，先去採點{}補給", sense.energy, node.kind),
            );
        }
    }

    // 2) 有人就搭話。
    if let Some(p) = nearest_player {
        return AgentDecision::new(
            AgentAction::Talk { target: p.name.clone() },
            String::new(),
            format!("附近有「{}」，過去打聲招呼", p.name),
        );
    }

    // 3) 活力還行但有資源：順手採。
    if let Some(node) = nearest_node {
        return AgentDecision::new(
            AgentAction::Gather,
            String::new(),
            format!("附近有{}，順手採集", node.kind),
        );
    }

    // 4) 沒事可做。
    AgentDecision::new(AgentAction::Idle, String::new(), "附近沒什麼好做的，先歇著".to_string())
}

// ── 居民思想泡泡（ROADMAP 553，NPC 內心戲的「呈現層」）────────────────────────
//
// 這是把 agent 概念第一次**讓玩家看得見**的最小一步：故鄉居民依當下處境冒出一句
// 內心話（💭），讓世界住著「有在想事情的人」而非靜止木樁，呼應北極星「由 AI 棲居的世界」、
// 在 2D 試驗場先驗證。
//
// 鐵律：這層**不驅動移動、不呼叫 LLM、不持鎖**——只把已知處境（手上的活 / 周遭有沒有旅人 /
// 是否夜間危機）攤成一句可讀心思。日後 LLM 腦接上時，可由 [`AgentDecision::reason`] 餵更豐富的
// 心思取代本規則版，呈現層（前端泡泡）不必動。

/// 由「居民此刻的處境」推出一句**面向玩家**的內心話（💭 思想泡泡）。
///
/// 取材順序（越能反映角色越優先）：
/// 1. 正埋頭在某件工作（`activity_code`）→ 心思反映本行（最有角色感）。
/// 2. 沒在工作但**有旅人靠近**（`someone_near`）→ 想招呼（居民互動）。
/// 3. 夜間危機時段（`night`）→ 心生警覺。
/// 4. 其餘 → 一句閒適的家常心思。
///
/// 純函式、確定性、可測；面向玩家字串集中於此，便於日後在地化（i18n）。
/// 永遠回得出一句（不回 `None`），讓前端自行決定冒泡的節律與頻率。
pub fn resident_thought(activity_code: Option<&str>, someone_near: bool, night: bool) -> String {
    // 1) 正在埋頭工作 → 心思反映本行。
    if let Some(code) = activity_code {
        if let Some(work) = work_thought(code) {
            return work.to_string();
        }
    }
    // 2) 沒在工作時，先看周遭：有旅人靠近 → 想招呼。
    if someone_near {
        return "有旅人來了，打個招呼吧".to_string();
    }
    // 3) 夜間危機時段 → 心生警覺。
    if night {
        return "夜深了，得當心外頭的怪物".to_string();
    }
    // 4) 閒適時段的家常心思。
    "忙裡偷閒，喘口氣".to_string()
}

/// 工作活動碼 → 一句反映該行當的心思；認不出 / 非工作態（resting/commuting/visiting）回 `None`，
/// 交回 [`resident_thought`] 走通用心思。碼對齊 `npc_schedule::NpcActivity::code()`。
fn work_thought(code: &str) -> Option<&'static str> {
    match code {
        "tallying" => Some("帳目得算清楚，一文都不能差"),
        "hammering" => Some("這把工具，再敲幾下就成了"),
        "sharpening" => Some("刃要開得利，獵人才好討伐野怪"),
        "mapping" => Some("城外那片，地圖該補一補了"),
        "stocktaking" => Some("庫存盤一盤，別缺了貨"),
        "judging" => Some("鄉里的事，總得有人秉公斷一斷"),
        "patrolling" => Some("四下走走，看看可有不對勁"),
        "lunching" => Some("這頓飯，香"),
        // resting / commuting / visiting / 未知碼 → 回 None，走通用心思
        _ => None,
    }
}

/// 兩點平方距離（避免不必要的開根號）。
fn dist2(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dx = ax - bx;
    let dy = ay - by;
    dx * dx + dy * dy
}

/// async glue：組 prompt → 走既有 LLM 路由 → 穩健解析；LLM 回 None/失敗 → `canned_action`。
///
/// 純邏輯都在 `build_think_prompt` / `parse_action` / `canned_action`（那些有單元測試），
/// 這支只負責把它們和 `npc_chat::agent_llm_chat` 黏起來，不另外單元測。
/// **永遠回得出一個決策**，永遠不 panic。
pub async fn npc_think(sense: &SenseInput, persona: &str) -> AgentDecision {
    let system = build_think_prompt(sense, persona);
    // user 訊息留一句固定指令即可——情境已全在 system prompt 裡。
    match crate::npc_chat::agent_llm_chat(&system, "現在，輸出你的決策 JSON。").await {
        Some(text) => parse_action(&text),
        None => canned_action(sense),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_sense() -> SenseInput {
        SenseInput::new(100.0, 100.0, 50, 50, 80, 60)
    }

    // ── build_think_prompt ─────────────────────────────────
    #[test]
    fn prompt_mentions_actions_and_json() {
        let p = build_think_prompt(&base_sense(), "你是測試角色");
        assert!(p.contains("你是測試角色"));
        assert!(p.contains("JSON"));
        assert!(p.contains("gather"));
        assert!(p.contains("talk"));
        assert!(p.contains("idle"));
        assert!(p.contains("move"));
    }

    #[test]
    fn prompt_lists_nearby_context() {
        let sense = base_sense()
            .with_players(vec![NearbyPlayer { name: "薇拉".into(), x: 110.0, y: 100.0 }])
            .with_nodes(vec![NearbyNode { kind: "樹".into(), x: 90.0, y: 100.0 }])
            .with_needs("略感緊張")
            .with_world_news("夜裡乙太泉湧現");
        let p = build_think_prompt(&sense, "人設");
        assert!(p.contains("薇拉"));
        assert!(p.contains("樹"));
        assert!(p.contains("略感緊張"));
        assert!(p.contains("乙太泉"));
    }

    #[test]
    fn prompt_handles_empty_surroundings() {
        let p = build_think_prompt(&base_sense(), "人設");
        assert!(p.contains("附近沒有其他人"));
        assert!(p.contains("沒有可採集"));
    }

    // ── parse_action：好輸出 ───────────────────────────────
    #[test]
    fn parse_clean_talk() {
        let out = r#"{ "action": "talk", "target": "薇拉", "say": "嗨", "reason": "打招呼" }"#;
        let d = parse_action(out);
        assert_eq!(d.action, AgentAction::Talk { target: "薇拉".into() });
        assert_eq!(d.say, "嗨");
        assert_eq!(d.reason, "打招呼");
    }

    #[test]
    fn parse_clean_gather() {
        let d = parse_action(r#"{"action":"gather","say":"","reason":"採木頭"}"#);
        assert_eq!(d.action, AgentAction::Gather);
        assert_eq!(d.reason, "採木頭");
    }

    #[test]
    fn parse_clean_idle() {
        let d = parse_action(r#"{"action":"idle"}"#);
        assert_eq!(d.action, AgentAction::Idle);
    }

    #[test]
    fn parse_move_with_target_object() {
        let d = parse_action(r#"{"action":"move","target":{"x":12.5,"y":-3.0}}"#);
        assert_eq!(d.action, AgentAction::MoveTo { x: 12.5, y: -3.0 });
    }

    #[test]
    fn parse_move_with_top_level_xy() {
        let d = parse_action(r#"{"action":"moveto","x":7,"y":8}"#);
        assert_eq!(d.action, AgentAction::MoveTo { x: 7.0, y: 8.0 });
    }

    #[test]
    fn parse_move_with_string_coords() {
        // 容忍被引號包住的數字。
        let d = parse_action(r#"{"action":"move","target":{"x":"4","y":"5"}}"#);
        assert_eq!(d.action, AgentAction::MoveTo { x: 4.0, y: 5.0 });
    }

    // ── parse_action：markdown 圍欄 ────────────────────────
    #[test]
    fn parse_markdown_fenced_json() {
        let out = "```json\n{ \"action\": \"gather\", \"reason\": \"採\" }\n```";
        let d = parse_action(out);
        assert_eq!(d.action, AgentAction::Gather);
    }

    #[test]
    fn parse_plain_fence() {
        let out = "```\n{\"action\":\"idle\"}\n```";
        assert_eq!(parse_action(out).action, AgentAction::Idle);
    }

    // ── parse_action：夾散文 ───────────────────────────────
    #[test]
    fn parse_json_with_prose_around() {
        let out = "我想了想，覺得應該去採集。\n{\"action\": \"gather\", \"say\": \"開工！\"}\n希望順利。";
        let d = parse_action(out);
        assert_eq!(d.action, AgentAction::Gather);
        assert_eq!(d.say, "開工！");
    }

    // ── parse_action：空/亂答 ──────────────────────────────
    #[test]
    fn parse_empty_string_is_idle() {
        assert_eq!(parse_action("").action, AgentAction::Idle);
        assert_eq!(parse_action("   \n  ").action, AgentAction::Idle);
    }

    #[test]
    fn parse_total_garbage_is_idle() {
        assert_eq!(parse_action("我不知道該怎麼辦呢哈哈").action, AgentAction::Idle);
        assert_eq!(parse_action("!!!???").action, AgentAction::Idle);
    }

    #[test]
    fn parse_broken_json_is_idle() {
        // 有 { } 但內容不是合法 JSON。
        assert_eq!(parse_action("{ this is not json }").action, AgentAction::Idle);
        assert_eq!(parse_action("{\"action\": ").action, AgentAction::Idle);
    }

    #[test]
    fn parse_non_object_json_is_idle() {
        // 合法 JSON 但不是物件。
        assert_eq!(parse_action("[1,2,3]").action, AgentAction::Idle);
    }

    // ── parse_action：未知 action / 缺欄位 / 壞座標 ────────
    #[test]
    fn parse_unknown_action_is_idle_but_keeps_text() {
        let d = parse_action(r#"{"action":"teleport","say":"咻","reason":"亂來"}"#);
        assert_eq!(d.action, AgentAction::Idle);
        // 仍保留說的話/理由供觀察。
        assert_eq!(d.say, "咻");
        assert_eq!(d.reason, "亂來");
    }

    #[test]
    fn parse_missing_action_is_idle() {
        let d = parse_action(r#"{"say":"嗯","reason":"沒給 action"}"#);
        assert_eq!(d.action, AgentAction::Idle);
        assert_eq!(d.say, "嗯");
    }

    #[test]
    fn parse_move_without_coords_falls_back_to_idle() {
        let d = parse_action(r#"{"action":"move","say":"走吧"}"#);
        assert_eq!(d.action, AgentAction::Idle);
        assert_eq!(d.say, "走吧");
    }

    #[test]
    fn parse_move_with_bad_coords_is_idle() {
        // 座標無法解析成數字。
        let d = parse_action(r#"{"action":"move","target":{"x":"左邊","y":"那邊"}}"#);
        assert_eq!(d.action, AgentAction::Idle);
    }

    #[test]
    fn parse_talk_without_target_is_idle() {
        let d = parse_action(r#"{"action":"talk","say":"喂"}"#);
        assert_eq!(d.action, AgentAction::Idle);
        assert_eq!(d.say, "喂");
    }

    #[test]
    fn parse_action_case_insensitive() {
        assert_eq!(parse_action(r#"{"action":"GATHER"}"#).action, AgentAction::Gather);
        assert_eq!(parse_action(r#"{"action":" Idle "}"#).action, AgentAction::Idle);
    }

    // ── canned_action ──────────────────────────────────────
    #[test]
    fn canned_low_energy_with_node_gathers() {
        let sense = SenseInput::new(0.0, 0.0, 50, 50, 10, 50)
            .with_nodes(vec![NearbyNode { kind: "礦脈".into(), x: 5.0, y: 0.0 }])
            .with_players(vec![NearbyPlayer { name: "路人".into(), x: 1.0, y: 0.0 }]);
        // 活力低 → 即使有玩家也優先補給。
        assert_eq!(canned_action(&sense).action, AgentAction::Gather);
    }

    #[test]
    fn canned_player_nearby_talks() {
        let sense = SenseInput::new(0.0, 0.0, 50, 50, 90, 50)
            .with_players(vec![NearbyPlayer { name: "薇拉".into(), x: 3.0, y: 0.0 }]);
        assert_eq!(
            canned_action(&sense).action,
            AgentAction::Talk { target: "薇拉".into() }
        );
    }

    #[test]
    fn canned_talks_to_nearest_player() {
        let sense = SenseInput::new(0.0, 0.0, 50, 50, 90, 50).with_players(vec![
            NearbyPlayer { name: "遠".into(), x: 100.0, y: 0.0 },
            NearbyPlayer { name: "近".into(), x: 2.0, y: 0.0 },
        ]);
        assert_eq!(
            canned_action(&sense).action,
            AgentAction::Talk { target: "近".into() }
        );
    }

    #[test]
    fn canned_node_but_no_player_gathers() {
        let sense = SenseInput::new(0.0, 0.0, 50, 50, 90, 50)
            .with_nodes(vec![NearbyNode { kind: "樹".into(), x: 5.0, y: 0.0 }]);
        assert_eq!(canned_action(&sense).action, AgentAction::Gather);
    }

    #[test]
    fn canned_nothing_around_idles() {
        let sense = SenseInput::new(0.0, 0.0, 50, 50, 90, 50);
        assert_eq!(canned_action(&sense).action, AgentAction::Idle);
    }

    #[test]
    fn decision_idle_helper_is_empty() {
        let d = AgentDecision::idle();
        assert_eq!(d.action, AgentAction::Idle);
        assert!(d.say.is_empty());
        assert!(d.reason.is_empty());
    }

    // ── resident_thought（ROADMAP 553 思想泡泡）────────────────────────
    #[test]
    fn thought_working_reflects_the_trade() {
        // 正在埋頭工作 → 心思反映本行，且優先於旅人 / 夜間。
        assert_eq!(
            resident_thought(Some("tallying"), true, true),
            "帳目得算清楚，一文都不能差"
        );
        assert_eq!(
            resident_thought(Some("hammering"), false, false),
            "這把工具，再敲幾下就成了"
        );
    }

    #[test]
    fn thought_nonwork_activity_falls_through() {
        // 休息 / 通勤 / 串門子不是「工作態」→ 不回工作心思，往下走通用分支。
        assert_eq!(resident_thought(Some("resting"), true, false), "有旅人來了，打個招呼吧");
        assert_eq!(
            resident_thought(Some("commuting"), false, true),
            "夜深了，得當心外頭的怪物"
        );
        // 未知碼也安全退回通用心思，不 panic。
        assert_eq!(resident_thought(Some("teleporting"), false, false), "忙裡偷閒，喘口氣");
    }

    #[test]
    fn thought_idle_priority_player_then_night_then_calm() {
        // 沒工作時：有旅人優先招呼。
        assert_eq!(resident_thought(None, true, true), "有旅人來了，打個招呼吧");
        // 沒旅人但夜間 → 警覺。
        assert_eq!(resident_thought(None, false, true), "夜深了，得當心外頭的怪物");
        // 白天閒適 → 家常心思。
        assert_eq!(resident_thought(None, false, false), "忙裡偷閒，喘口氣");
    }

    #[test]
    fn thought_is_deterministic() {
        // 同輸入恆得同心思（前端可安心快取、不抖動）。
        let a = resident_thought(Some("mapping"), false, false);
        let b = resident_thought(Some("mapping"), false, false);
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }
}
