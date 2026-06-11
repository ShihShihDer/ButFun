//! 怪物王咆哮（ROADMAP 75）。
//!
//! 菁英精英（notorious enemy，等級 ≥ base_level+3）偶爾在世界頻道發出 AI 生成的威脅話語。
//!
//! 成本紀律：
//! - 每隻菁英最多每 `ROAR_COOLDOWN_SECS` 秒咆哮一次。
//! - 全局獨立 Semaphore（容量 1），防止多 boss 同時燒 LLM。
//! - 降級鏈：Groq → ollama → 罐頭（同 npc_chat 模式）。
//! - `BUTFUN_NPC_LLM=1` 未設定時直接回罐頭，不呼叫任何外部 API。

use std::collections::{HashMap, HashSet};

/// 每隻菁英咆哮的冷卻（秒）。
pub const ROAR_COOLDOWN_SECS: f32 = 180.0; // 3 分鐘

/// 伺服器啟動後首次咆哮的最短等待（秒），避免剛啟動就馬上觸發。
const FIRST_ROAR_WAIT_SECS: f32 = 60.0;

/// boss_roar 專屬 Semaphore 容量：同時最多 1 個 AI 咆哮呼叫。
pub const MAX_CONCURRENT_ROARS: usize = 1;

/// 一個菁英精英的咆哮狀態（純記憶體，重啟清零）。
pub struct BossRoarState {
    /// enemy_id → 距下次咆哮倒數（秒）。0 以下即觸發。
    cooldowns: HashMap<(i32, i32, usize), f32>,
}

/// 觸發一次咆哮所需的上下文資訊。
#[derive(Debug, Clone)]
pub struct RoarCandidate {
    pub id: (i32, i32, usize),
    /// 怪物中文種類名稱（來自 `EnemyKind::display_name()`）。
    pub kind_name: &'static str,
    pub level: u32,
}

impl BossRoarState {
    pub fn new() -> Self {
        Self {
            cooldowns: HashMap::new(),
        }
    }

    /// 推進時間，回傳第一個冷卻歸零的菁英（最多一個，避免同幀觸發大量 LLM）。
    ///
    /// `notorious`：當前所有菁英精英的列表 `(id, kind_name, level)`。
    /// 函式會自動清理已消失的敵人（避免記憶體洩漏）。
    pub fn tick(
        &mut self,
        dt: f32,
        notorious: &[((i32, i32, usize), &'static str, u32)],
    ) -> Option<RoarCandidate> {
        // 清除已不存在的菁英，釋放 HashMap 空間。
        let active_ids: HashSet<_> = notorious.iter().map(|(id, _, _)| *id).collect();
        self.cooldowns.retain(|id, _| active_ids.contains(id));

        let mut candidate: Option<RoarCandidate> = None;
        for &(id, kind_name, level) in notorious {
            let timer = self.cooldowns.entry(id).or_insert(FIRST_ROAR_WAIT_SECS);
            *timer -= dt;
            if *timer <= 0.0 && candidate.is_none() {
                *timer = ROAR_COOLDOWN_SECS;
                candidate = Some(RoarCandidate { id, kind_name, level });
            }
        }
        candidate
    }
}

/// 建立 AI 咆哮的 system prompt。
pub fn build_system_prompt(kind_name: &str, level: u32) -> String {
    format!(
        "你是蒸汽龐克太空歌劇世界中一隻強大的「{kind_name}」，等級 {level}，是這片區域的菁英霸主。\
        你個性蠻橫、充滿威脅感，視人類為無謂的侵略者。\
        請用 25 字以內的繁體中文，以充滿怒氣的怪物口吻說一句怒吼、威嚇或挑釁的話。\
        只輸出那一句話，不要任何額外說明、引號或前綴。"
    )
}

/// 依種類回傳罐頭咆哮（LLM 未啟用或呼叫失敗時的後備）。
pub fn canned_roar(kind_name: &str) -> &'static str {
    match kind_name {
        "廢鐵無人機" => "入侵者！全體目標鎖定——消滅！",
        "乙太鬼火" => "你的乙太……將成為我的養料！",
        "飄舞精靈" => "消失！這片光芒不容褻瀆！",
        "蕈菇潛行者" => "你的恐懼氣息……讓我胃口大開！",
        "晶石傀儡" => "人類的血，是最好的結晶養料！",
        "符文守衛" => "擅闖禁地者，即刻封存於符文之中！",
        "珊瑚蟹" => "退後！這片礁石是我的王國！",
        "翠幽魅影" => "你們的靈魂……終將在翠幽中徘徊！",
        "蒸汽構裝" => "運算完畢：你們的滅亡率 100%！",
        "虛空幽靈" => "虛空吞噬一切——包括你們！",
        "霧醚幻靈" => "在霧中迷失吧，再也找不到出路！",
        "源晶守護者" => "褻瀆源晶者，萬劫不復！",
        "裂縫守護者" => "裂縫的力量——非你能承受！",
        _ => "膽敢踏入我的領地？今日你們必將後悔！",
    }
}

/// 呼叫 LLM 生成咆哮（Groq → ollama → 罐頭降級）。
/// 為純 async 函式，由 game.rs tokio::spawn 呼叫，不阻塞遊戲迴圈。
pub async fn generate_roar(kind_name: &str, level: u32, player_count: usize) -> String {
    let system = build_system_prompt(kind_name, level);
    let user = format!(
        "附近有 {player_count} 個人類玩家入侵你的領地，現在發出你的怒吼！"
    );
    match crate::npc_chat::raw_llm_call(&system, &user).await {
        Some(text) => {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                canned_roar(kind_name).to_string()
            } else {
                trimmed
            }
        }
        None => canned_roar(kind_name).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_roar_does_not_trigger_immediately() {
        let mut state = BossRoarState::new();
        let id = (0i32, 0i32, 0usize);
        let notorious = vec![(id, "晶石傀儡", 5u32)];

        // 剛出現：timer = FIRST_ROAR_WAIT_SECS，減 1 秒後仍未歸零。
        let r = state.tick(1.0, &notorious);
        assert!(r.is_none(), "首次出現 1 秒後不應觸發咆哮");
    }

    #[test]
    fn roar_triggers_after_wait() {
        let mut state = BossRoarState::new();
        let id = (1i32, 2i32, 0usize);
        let notorious = vec![(id, "符文守衛", 7u32)];

        // 推進超過 FIRST_ROAR_WAIT_SECS
        let r = state.tick(FIRST_ROAR_WAIT_SECS + 1.0, &notorious);
        assert!(r.is_some(), "等待時間過後應觸發咆哮");
        let c = r.unwrap();
        assert_eq!(c.id, id);
        assert_eq!(c.kind_name, "符文守衛");
        assert_eq!(c.level, 7);
    }

    #[test]
    fn roar_resets_cooldown_after_trigger() {
        let mut state = BossRoarState::new();
        let id = (0i32, 0i32, 1usize);
        let notorious = vec![(id, "虛空幽靈", 10u32)];

        // 觸發一次
        state.tick(FIRST_ROAR_WAIT_SECS + 1.0, &notorious);

        // 觸發後立刻再 tick 不應再觸發（timer 重置為 ROAR_COOLDOWN_SECS）
        let r = state.tick(1.0, &notorious);
        assert!(r.is_none(), "觸發後應重置冷卻，不立刻再觸發");
    }

    #[test]
    fn roar_cleans_up_gone_enemies() {
        let mut state = BossRoarState::new();
        let id = (5i32, 5i32, 0usize);
        let notorious = vec![(id, "蒸汽構裝", 12u32)];

        // 先記錄進 cooldowns
        state.tick(1.0, &notorious);
        assert_eq!(state.cooldowns.len(), 1);

        // 帶空列表 tick → 應清除
        state.tick(1.0, &[]);
        assert_eq!(state.cooldowns.len(), 0, "怪消失後應清除 cooldown");
    }

    #[test]
    fn canned_roar_all_kinds_non_empty() {
        let kinds = [
            "廢鐵無人機", "乙太鬼火", "飄舞精靈", "蕈菇潛行者",
            "晶石傀儡", "符文守衛", "珊瑚蟹", "翠幽魅影",
            "蒸汽構裝", "虛空幽靈", "霧醚幻靈", "源晶守護者", "裂縫守護者",
            "未知怪物",
        ];
        for k in &kinds {
            assert!(!canned_roar(k).is_empty(), "{k} 的罐頭咆哮不應為空");
        }
    }

    #[test]
    fn build_system_prompt_contains_kind_and_level() {
        let s = build_system_prompt("蕈菇潛行者", 9);
        assert!(s.contains("蕈菇潛行者"), "prompt 應包含種類名稱");
        assert!(s.contains('9'), "prompt 應包含等級數字");
    }
}
