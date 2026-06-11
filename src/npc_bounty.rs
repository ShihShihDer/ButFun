//! NPC 自主懸賞令（ROADMAP 82：Wave 2 第十一塊）。
//!
//! 當兇名精英（notorious enemy，等級 ≥ base_level+3）存在，
//! 且獵手蘭卡的安全感低於閾值時，蘭卡自主發布通緝令到世界聊天頻道。
//! 玩家討伐通緝目標後可獲得懸賞乙太，並由蘭卡廣播討伐成功。
//!
//! 成本紀律：
//! - 公告冷卻 `ANNOUNCE_COOLDOWN_SECS`（15 分鐘）；同時最多一筆活躍懸賞。
//! - AI 生成走 Groq → ollama → 罐頭降級鏈；不啟用 LLM 也可正常運作。
//! - 純記憶體模式，重啟清零，零 migration，不破壞玩家資料。

/// 公告冷卻（秒）：前一則通緝令發出後至少等這麼久才再發。
pub const ANNOUNCE_COOLDOWN_SECS: f32 = 900.0; // 15 分鐘

/// 懸賞有效期（秒）：逾時自動失效。
pub const BOUNTY_DURATION_SECS: f32 = 1800.0; // 30 分鐘

/// 討伐懸賞目標的獎勵乙太。
pub const BOUNTY_REWARD: u32 = 25;

/// 蘭卡安全感低於此值時，才考慮發布通緝令。
pub const LANCA_SAFETY_THRESHOLD: i32 = 50;

/// 伺服器啟動後首次觸發的最短等待（秒），避免重啟立刻觸發。
const FIRST_ANNOUNCE_WAIT_SECS: f32 = 300.0; // 5 分鐘

/// 一筆活躍懸賞的資料。
#[derive(Debug, Clone)]
pub struct ActiveBounty {
    /// 通緝目標怪物種類名稱（如「晶石傀儡」）。
    pub kind_name: String,
    /// 發布時怪物的等級。
    pub level: u32,
    /// 獎勵乙太數量。
    pub reward_ether: u32,
    /// 剩餘有效期（秒），倒數至 0 則過期。
    pub lifetime: f32,
}

/// NPC 懸賞系統全域狀態（純記憶體，重啟清零）。
pub struct NpcBountyState {
    /// 當前活躍懸賞（同時最多一筆）。
    pub active: Option<ActiveBounty>,
    /// 距下次允許發布公告的倒數（秒）。
    announce_cooldown: f32,
}

impl Default for NpcBountyState {
    fn default() -> Self {
        Self {
            active: None,
            announce_cooldown: FIRST_ANNOUNCE_WAIT_SECS,
        }
    }
}

impl NpcBountyState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 每 tick 推進計時器，在條件成立時回傳「應廣播的懸賞」。
    ///
    /// - `dt`：本幀秒數
    /// - `notorious`：當前所有兇名精英的 `(kind_name, level)` 列表
    /// - `lanca_safety`：蘭卡當前安全感值（0~100）
    ///
    /// 回傳 `Some((kind_name, level))` 表示「發布此懸賞並廣播」，呼叫端負責廣播。
    pub fn tick(
        &mut self,
        dt: f32,
        notorious: &[(&'static str, u32)],
        lanca_safety: i32,
    ) -> Option<(String, u32)> {
        // 推進活躍懸賞的有效期倒數，過期則清除。
        if let Some(ref mut b) = self.active {
            b.lifetime -= dt;
            if b.lifetime <= 0.0 {
                self.active = None;
            }
        }

        // 推進公告冷卻倒數。
        if self.announce_cooldown > 0.0 {
            self.announce_cooldown -= dt;
        }

        // 觸發條件：有兇名精英 + 蘭卡安全感低 + 無活躍懸賞 + 冷卻結束。
        if lanca_safety >= LANCA_SAFETY_THRESHOLD
            || self.active.is_some()
            || self.announce_cooldown > 0.0
            || notorious.is_empty()
        {
            return None;
        }

        // 選等級最高的兇名精英作為通緝目標。
        let target = notorious.iter().max_by_key(|(_, lvl)| *lvl)?;
        let kind_name = target.0.to_string();
        let level = target.1;

        // 設定新懸賞並重置冷卻。
        self.active = Some(ActiveBounty {
            kind_name: kind_name.clone(),
            level,
            reward_ether: BOUNTY_REWARD,
            lifetime: BOUNTY_DURATION_SECS,
        });
        self.announce_cooldown = ANNOUNCE_COOLDOWN_SECS;

        Some((kind_name, level))
    }

    /// 玩家擊殺兇名精英時呼叫。若目標符合活躍懸賞，清除懸賞並回傳獎勵金額。
    ///
    /// - `killed_kind`：被擊殺的怪物種類名稱
    /// - `was_notorious`：此次擊殺時怪物是否為兇名狀態
    ///
    /// 回傳 `Some(reward_ether)` 表示「應給予此玩家懸賞獎勵」。
    pub fn on_notorious_killed(&mut self, killed_kind: &str, was_notorious: bool) -> Option<u32> {
        if !was_notorious {
            return None;
        }
        if let Some(ref b) = self.active {
            if b.kind_name == killed_kind {
                let reward = b.reward_ether;
                self.active = None;
                return Some(reward);
            }
        }
        None
    }
}

/// 建立 AI 通緝令的 system prompt。
pub fn build_system_prompt(kind_name: &str, level: u32) -> String {
    format!(
        "你是蒸汽龐克太空歌劇世界的獵手蘭卡，村落守護獵人，個性直接低調、充滿責任感。\
        目前有一隻 Lv.{level} 的兇名「{kind_name}」正在肆虐，威脅村落安全。\
        請以 30 字以內的繁體中文，以獵手口吻發出通緝令，提及懸賞 25 乙太並呼籲勇者出手。\
        只輸出那句公告，不加引號或前綴。"
    )
}

/// 罐頭通緝令（LLM 未啟用或呼叫失敗時的後備）。
pub fn canned_announcement(kind_name: &str, level: u32) -> String {
    format!(
        "一隻 Lv.{level} 的兇名{kind_name}正在橫行！討伐者賞 {BOUNTY_REWARD} 乙太，勇者請出手！"
    )
}

/// 呼叫 LLM 生成通緝令（Groq → ollama → 罐頭降級鏈）。
/// 為純 async 函式，由 game.rs 以 `tokio::spawn` 呼叫，不阻塞遊戲迴圈。
pub async fn generate_announcement(kind_name: &str, level: u32) -> String {
    let system = build_system_prompt(kind_name, level);
    let user = format!("現在發布這隻 Lv.{level} 兇名{kind_name}的通緝令。");
    match crate::npc_chat::raw_llm_call(&system, &user).await {
        Some(text) => {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                canned_announcement(kind_name, level)
            } else {
                trimmed
            }
        }
        None => canned_announcement(kind_name, level),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_state() -> NpcBountyState {
        let mut s = NpcBountyState::new();
        s.announce_cooldown = 0.0; // 跳過等待期，方便測試
        s
    }


    #[test]
    fn no_trigger_when_safety_high() {
        let mut s = ready_state();
        let notorious = vec![("晶石傀儡", 7u32)];
        // 安全感恰好等於閾值（>= 則不觸發）
        let result = s.tick(1.0, &notorious, LANCA_SAFETY_THRESHOLD);
        assert!(result.is_none(), "安全感達閾值時不應觸發懸賞");
    }

    #[test]
    fn no_trigger_during_cooldown() {
        let mut s = NpcBountyState::new();
        // 冷卻還剩 100 秒
        s.announce_cooldown = 100.0;
        let notorious = vec![("晶石傀儡", 7u32)];
        let result = s.tick(1.0, &notorious, 30);
        assert!(result.is_none(), "冷卻中不應觸發懸賞");
    }

    #[test]
    fn triggers_when_all_conditions_met() {
        let mut s = ready_state();
        let notorious = vec![("晶石傀儡", 7u32)];
        let result = s.tick(1.0, &notorious, 30);
        assert!(result.is_some(), "條件全部成立時應觸發懸賞");
        let (kind, level) = result.unwrap();
        assert_eq!(kind, "晶石傀儡");
        assert_eq!(level, 7);
    }

    #[test]
    fn active_bounty_blocks_new_announcement() {
        let mut s = ready_state();
        let notorious = vec![("晶石傀儡", 7u32)];
        let r1 = s.tick(1.0, &notorious, 30);
        assert!(r1.is_some(), "第一次應觸發");
        // 強制清零冷卻，但 active 仍在
        s.announce_cooldown = 0.0;
        let r2 = s.tick(1.0, &notorious, 30);
        assert!(r2.is_none(), "已有活躍懸賞時不應再觸發");
    }

    #[test]
    fn picks_highest_level_notorious() {
        let mut s = ready_state();
        let notorious = vec![("飄舞精靈", 4u32), ("晶石傀儡", 9u32), ("珊瑚蟹", 6u32)];
        let (kind, level) = s.tick(1.0, &notorious, 30).unwrap();
        assert_eq!(kind, "晶石傀儡", "應選等級最高的兇名精英");
        assert_eq!(level, 9);
    }

    #[test]
    fn bounty_expires_after_lifetime() {
        let mut s = ready_state();
        let notorious = vec![("蕈菇潛行者", 5u32)];
        let _ = s.tick(1.0, &notorious, 30);
        assert!(s.active.is_some());

        // 推進至超過有效期
        s.tick(BOUNTY_DURATION_SECS + 1.0, &[], 60);
        assert!(s.active.is_none(), "懸賞到期後應自動清除");
    }

    #[test]
    fn claim_rewards_on_matching_notorious_kill() {
        let mut s = ready_state();
        let notorious = vec![("符文守衛", 8u32)];
        let _ = s.tick(1.0, &notorious, 30);
        assert!(s.active.is_some());

        let reward = s.on_notorious_killed("符文守衛", true);
        assert_eq!(reward, Some(BOUNTY_REWARD), "討伐懸賞目標應得到獎勵");
        assert!(s.active.is_none(), "領取獎勵後懸賞應清除");
    }

    #[test]
    fn no_reward_for_non_notorious_kill() {
        let mut s = ready_state();
        let notorious = vec![("符文守衛", 8u32)];
        let _ = s.tick(1.0, &notorious, 30);

        // 同種類但非兇名狀態
        let reward = s.on_notorious_killed("符文守衛", false);
        assert!(reward.is_none(), "非兇名狀態不應觸發懸賞領取");
        assert!(s.active.is_some(), "懸賞應仍存在");
    }

    #[test]
    fn no_reward_for_wrong_kind() {
        let mut s = ready_state();
        let notorious = vec![("符文守衛", 8u32)];
        let _ = s.tick(1.0, &notorious, 30);

        // 擊殺不同種類
        let reward = s.on_notorious_killed("晶石傀儡", true);
        assert!(reward.is_none(), "擊殺非通緝目標種類不應得到獎勵");
        assert!(s.active.is_some(), "懸賞應仍存在");
    }

    #[test]
    fn build_system_prompt_contains_key_info() {
        let s = build_system_prompt("珊瑚蟹", 10);
        assert!(s.contains("珊瑚蟹"), "prompt 應包含怪物名稱");
        assert!(s.contains("10"), "prompt 應包含等級");
        assert!(s.contains("25"), "prompt 應包含賞金數字");
    }

    #[test]
    fn canned_announcement_contains_key_info() {
        let a = canned_announcement("晶石傀儡", 7);
        assert!(a.contains("晶石傀儡"), "罐頭公告應包含怪物名稱");
        assert!(a.contains("Lv.7"), "罐頭公告應包含等級");
        assert!(a.contains("25"), "罐頭公告應包含賞金");
    }
}
