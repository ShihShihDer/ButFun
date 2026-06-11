//! NPC 落敗反應（ROADMAP 83：Wave 2 第十二塊）。
//!
//! 當玩家被怪物擊倒（取得「倒地」狀態）時，
//! 凱爾長老或獵手蘭卡輪流在世界聊天頻道廣播慰問 / 警示，
//! 讓其他在線玩家也感受到世界對每次戰鬥都有所反應。
//!
//! 成本紀律：
//! - 全局冷卻 `REACTION_COOLDOWN_SECS`（2 分鐘）：防止多人同時倒地時連續廣播。
//! - Semaphore(1)：同時最多一個 LLM 呼叫。
//! - 降級鏈：Groq → ollama → 罐頭。
//! - `BUTFUN_NPC_LLM=1` 未設定時直接回罐頭，不呼叫任何外部 API。
//! - 純記憶體模式，重啟清零，零 migration，不破壞玩家資料。

/// 落敗反應冷卻（秒）：防止多玩家同時倒地時連續廣播。
pub const REACTION_COOLDOWN_SECS: f32 = 120.0; // 2 分鐘

/// 伺服器啟動後首次觸發的最短等待（秒），避免重啟立刻廣播。
const FIRST_REACTION_WAIT_SECS: f32 = 60.0; // 1 分鐘

/// 落敗反應專屬 Semaphore 容量。
pub const MAX_CONCURRENT_REACTIONS: usize = 1;

/// 凱爾長老顯示名稱。
pub const CHIEF_DISPLAY_NAME: &str = "凱爾長老";

/// 獵手蘭卡顯示名稱。
pub const RANKA_DISPLAY_NAME: &str = "獵手蘭卡";

/// 決定由哪個 NPC 廣播落敗反應。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReactionNpc {
    /// 凱爾長老：溫厚鼓勵。
    Chief,
    /// 獵手蘭卡：戰術警示。
    Lanka,
}

/// NPC 落敗反應全域狀態（純記憶體，重啟清零）。
pub struct NpcDefeatReactionState {
    /// 距下次允許觸發的倒數（秒）。
    cooldown: f32,
    /// 觸發計數：決定輪替 NPC（偶數→凱爾，奇數→蘭卡）。
    trigger_count: u32,
}

impl Default for NpcDefeatReactionState {
    fn default() -> Self {
        Self {
            cooldown: FIRST_REACTION_WAIT_SECS,
            trigger_count: 0,
        }
    }
}

impl NpcDefeatReactionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 推進時間倒數（秒）。每次觸發點呼叫一次（戰鬥判定每秒一次，dt=1.0）。
    pub fn tick(&mut self, dt: f32) {
        if self.cooldown > 0.0 {
            self.cooldown -= dt;
        }
    }

    /// 玩家倒地時呼叫；若冷卻已結束，回傳應廣播的 NPC 類型，呼叫端負責 AI 呼叫與廣播。
    pub fn on_player_downed(&mut self) -> Option<ReactionNpc> {
        if self.cooldown > 0.0 {
            return None;
        }
        self.cooldown = REACTION_COOLDOWN_SECS;
        let npc = if self.trigger_count % 2 == 0 {
            ReactionNpc::Chief
        } else {
            ReactionNpc::Lanka
        };
        self.trigger_count += 1;
        Some(npc)
    }
}

/// 建立凱爾長老慰問的 system prompt。
pub fn build_kyle_prompt(player_name: &str) -> String {
    format!(
        "你是蒸汽龐克太空歌劇世界中受人敬重的村長——凱爾長老。\
        一位名叫「{player_name}」的冒險者剛在征途中倒下了。\
        請用 25 字以內的繁體中文，以長老口吻說一句溫厚的慰問：\
        鼓勵他重振旗鼓，帶有些許智慧與關懷，略帶戲劇感。\
        只輸出那句話，不加引號或前綴。"
    )
}

/// 建立獵手蘭卡警示的 system prompt。
pub fn build_lanka_prompt(player_name: &str) -> String {
    format!(
        "你是蒸汽龐克太空歌劇世界的獵手蘭卡，村落守護獵人，個性直接低調、充滿責任感。\
        冒險者「{player_name}」剛被怪物擊倒。\
        請用 25 字以內的繁體中文，以獵手口吻對在線其他人提出警示：\
        點出敵情嚴峻、提醒各位保持謹慎。\
        只輸出那句話，不加引號或前綴。"
    )
}

/// 凱爾長老的罐頭慰問（LLM 未啟用或失敗時的後備）。
pub fn canned_kyle_reaction(player_name: &str) -> String {
    let idx = player_name.len() % 3;
    [
        format!("{player_name} 勇士倒下了，稍事休息，重振旗鼓再出發！"),
        format!("前方艱辛，{player_name} 落敗——但每次摔倒都是成長的養分。"),
        format!("願 {player_name} 早日復原，這塊土地需要你的守護。"),
    ][idx]
    .clone()
}

/// 獵手蘭卡的罐頭警示（LLM 未啟用或失敗時的後備）。
pub fn canned_lanka_reaction(player_name: &str) -> String {
    let idx = player_name.len() % 3;
    [
        format!("{player_name} 落敗了——那一帶的怪物今日格外兇悍，各位多加小心！"),
        format!("警報：{player_name} 被擊倒，敵情嚴峻，勿單獨深入！"),
        format!("{player_name} 倒下，各位注意安全，備好裝備再出戰。"),
    ][idx]
    .clone()
}

/// 呼叫 LLM 生成 NPC 落敗反應（Groq → ollama → 罐頭降級鏈）。
/// 為純 async 函式，由 game.rs 以 `tokio::spawn` 呼叫，不阻塞遊戲迴圈。
pub async fn generate_reaction(npc: ReactionNpc, player_name: String) -> (ReactionNpc, String) {
    let (system, user) = match npc {
        ReactionNpc::Chief => (
            build_kyle_prompt(&player_name),
            format!("請對「{player_name}」倒地說一句慰問。"),
        ),
        ReactionNpc::Lanka => (
            build_lanka_prompt(&player_name),
            format!("「{player_name}」剛被怪物擊倒，請廣播警示。"),
        ),
    };
    let text = match crate::npc_chat::raw_llm_call(&system, &user).await {
        Some(t) => {
            let trimmed = t.trim().to_string();
            if trimmed.is_empty() {
                fallback(&npc, &player_name)
            } else {
                trimmed
            }
        }
        None => fallback(&npc, &player_name),
    };
    (npc, text)
}

fn fallback(npc: &ReactionNpc, player_name: &str) -> String {
    match npc {
        ReactionNpc::Chief => canned_kyle_reaction(player_name),
        ReactionNpc::Lanka => canned_lanka_reaction(player_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_state() -> NpcDefeatReactionState {
        let mut s = NpcDefeatReactionState::new();
        s.cooldown = 0.0; // 跳過等待期
        s
    }

    #[test]
    fn no_trigger_during_cooldown() {
        let s = NpcDefeatReactionState::new(); // 初始有 FIRST_REACTION_WAIT_SECS 冷卻
        // 冷卻未歸零，不觸發
        let mut s = s;
        let result = s.on_player_downed();
        assert!(result.is_none(), "冷卻中不應觸發落敗反應");
    }

    #[test]
    fn triggers_when_cooldown_zero() {
        let mut s = ready_state();
        let result = s.on_player_downed();
        assert!(result.is_some(), "冷卻歸零後應觸發落敗反應");
    }

    #[test]
    fn first_trigger_is_chief() {
        let mut s = ready_state();
        let npc = s.on_player_downed().unwrap();
        assert_eq!(npc, ReactionNpc::Chief, "第一次觸發應由凱爾長老廣播");
    }

    #[test]
    fn second_trigger_is_lanka() {
        let mut s = ready_state();
        let _ = s.on_player_downed();
        s.cooldown = 0.0; // 重置冷卻
        let npc = s.on_player_downed().unwrap();
        assert_eq!(npc, ReactionNpc::Lanka, "第二次觸發應由獵手蘭卡廣播");
    }

    #[test]
    fn alternates_between_npcs() {
        let mut s = ready_state();
        let mut results = Vec::new();
        for _ in 0..4 {
            if let Some(npc) = s.on_player_downed() {
                results.push(npc);
            }
            s.cooldown = 0.0; // 強制重置冷卻供測試
        }
        assert_eq!(results.len(), 4);
        assert_eq!(results[0], ReactionNpc::Chief);
        assert_eq!(results[1], ReactionNpc::Lanka);
        assert_eq!(results[2], ReactionNpc::Chief);
        assert_eq!(results[3], ReactionNpc::Lanka);
    }

    #[test]
    fn cooldown_resets_after_trigger() {
        let mut s = ready_state();
        let _ = s.on_player_downed();
        assert!(s.cooldown > 0.0, "觸發後應重置冷卻");
        let result = s.on_player_downed();
        assert!(result.is_none(), "冷卻重置後不應立即再觸發");
    }

    #[test]
    fn tick_decrements_cooldown() {
        let mut s = NpcDefeatReactionState::new();
        let initial = s.cooldown;
        s.tick(10.0);
        assert!(s.cooldown < initial, "tick 應減少冷卻倒數");
        assert!((s.cooldown - (initial - 10.0)).abs() < 0.001);
    }

    #[test]
    fn tick_does_not_go_below_zero() {
        let mut s = ready_state(); // cooldown = 0
        s.tick(99.0);
        assert!(s.cooldown <= 0.0, "冷卻已為 0 時 tick 不應繼續減");
    }

    #[test]
    fn build_kyle_prompt_contains_player_name() {
        let p = build_kyle_prompt("鋼鐵戰士");
        assert!(p.contains("鋼鐵戰士"), "prompt 應包含玩家名稱");
        assert!(p.contains("凱爾長老"), "prompt 應包含 NPC 名稱");
    }

    #[test]
    fn build_lanka_prompt_contains_player_name() {
        let p = build_lanka_prompt("鋼鐵戰士");
        assert!(p.contains("鋼鐵戰士"), "prompt 應包含玩家名稱");
        assert!(p.contains("蘭卡"), "prompt 應包含 NPC 名稱");
    }

    #[test]
    fn canned_kyle_contains_player_name() {
        let r = canned_kyle_reaction("星辰旅人");
        assert!(r.contains("星辰旅人"), "罐頭慰問應包含玩家名稱");
    }

    #[test]
    fn canned_lanka_contains_player_name() {
        let r = canned_lanka_reaction("星辰旅人");
        assert!(r.contains("星辰旅人"), "罐頭警示應包含玩家名稱");
    }
}
