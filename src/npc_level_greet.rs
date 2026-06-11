//! NPC 升等賀詞（ROADMAP 84：Wave 2 第十三塊）。
//!
//! 每次玩家升等時，凱爾長老送**私信**恭賀（只有本人看到）。
//! 若升到整十等級（10、20、30……），凱爾長老**全服廣播**里程碑宣告，
//! 讓全服玩家都知道有人達成重要進展，世界因而更有生命感。
//!
//! 成本紀律：
//! - 純罐頭訊息，**不**呼叫 LLM，零額外費用。
//! - 全服廣播設 `WORLD_BROADCAST_COOLDOWN_SECS` 全局冷卻，防止多人同時里程碑時刷屏。
//! - 零 migration，純記憶體模式，重啟清零，不破壞玩家資料。

/// 凱爾長老的顯示名稱。
pub const CHIEF_DISPLAY_NAME: &str = "凱爾長老";

/// 里程碑全服廣播的全局冷卻（秒）：避免多人同時達里程碑時連續廣播。
pub const WORLD_BROADCAST_COOLDOWN_SECS: f32 = 60.0;

/// 升等賀詞全域狀態（純記憶體，重啟清零）。
pub struct NpcLevelGreetState {
    /// 距下次允許全服廣播的倒數（秒）。
    world_cooldown: f32,
}

impl Default for NpcLevelGreetState {
    fn default() -> Self {
        Self { world_cooldown: 0.0 }
    }
}

impl NpcLevelGreetState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 推進時間倒數（每 tick 呼叫，dt 秒）。
    pub fn tick(&mut self, dt: f32) {
        if self.world_cooldown > 0.0 {
            self.world_cooldown = (self.world_cooldown - dt).max(0.0);
        }
    }

    /// 玩家升等時呼叫；回傳應採取的廣播動作。
    pub fn on_level_up(&mut self, player_name: &str, new_level: u32) -> LevelGreetAction {
        if is_milestone_level(new_level) && self.world_cooldown <= 0.0 {
            self.world_cooldown = WORLD_BROADCAST_COOLDOWN_SECS;
            LevelGreetAction::WorldBroadcast {
                message: world_broadcast_text(player_name, new_level),
            }
        } else {
            LevelGreetAction::DirectMessage {
                message: direct_message_text(player_name, new_level),
            }
        }
    }
}

/// 升等賀詞的廣播動作。
#[derive(Debug, Clone, PartialEq)]
pub enum LevelGreetAction {
    /// 全服廣播（里程碑等級：10、20、30……）。
    WorldBroadcast { message: String },
    /// 私信玩家（一般升等）。
    DirectMessage { message: String },
}

/// 判斷某等級是否為里程碑（整十且大於零）。
pub fn is_milestone_level(level: u32) -> bool {
    level > 0 && level % 10 == 0
}

/// 里程碑全服廣播文字（根據里程碑序號輪替 4 種語氣）。
pub fn world_broadcast_text(player_name: &str, level: u32) -> String {
    let idx = ((level / 10).saturating_sub(1)) as usize % 4;
    [
        format!("恭賀 {player_name} 晉升至 Lv.{level} 里程碑！在這片星域，你的成長有目共睹！"),
        format!("{player_name} 達到 Lv.{level}！願乙太之光指引更遠的前路！"),
        format!("了不起！{player_name} 突破了 Lv.{level}！這份毅力令人動容！"),
        format!("{player_name} 晉升 Lv.{level}！本星域的冒險史將記住你的名字！"),
    ][idx]
    .clone()
}

/// 私信文字（非里程碑升等，根據玩家名稱長度輪替 5 種語氣）。
pub fn direct_message_text(player_name: &str, level: u32) -> String {
    let idx = player_name.len() % 5;
    [
        format!("恭喜升到 Lv.{level}，{player_name}！繼續加油，你做得很棒！"),
        format!("Lv.{level} 了，{player_name}！每一步都讓這個世界更豐盛。"),
        format!("{player_name} 升至 Lv.{level}，旅途遠未結束——星辰在前方等著！"),
        format!("又升一級了，{player_name}！Lv.{level}，繼續往前走吧。"),
        format!("不錯，{player_name}！達到 Lv.{level} 了，乙太祝福你的前路！"),
    ][idx]
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_state() -> NpcLevelGreetState {
        NpcLevelGreetState::new() // world_cooldown = 0，可立即廣播
    }

    #[test]
    fn is_milestone_zero_is_not_milestone() {
        assert!(!is_milestone_level(0), "等級 0 不是里程碑");
    }

    #[test]
    fn is_milestone_ten_is_milestone() {
        assert!(is_milestone_level(10), "等級 10 應是里程碑");
    }

    #[test]
    fn is_milestone_five_is_not_milestone() {
        assert!(!is_milestone_level(5), "等級 5 不是整十里程碑");
    }

    #[test]
    fn is_milestone_twenty_is_milestone() {
        assert!(is_milestone_level(20), "等級 20 應是里程碑");
    }

    #[test]
    fn is_milestone_thirty_is_milestone() {
        assert!(is_milestone_level(30), "等級 30 應是里程碑");
    }

    #[test]
    fn milestone_level_triggers_world_broadcast() {
        let mut s = ready_state();
        let action = s.on_level_up("星辰旅人", 10);
        assert!(
            matches!(action, LevelGreetAction::WorldBroadcast { .. }),
            "整十等級應觸發全服廣播"
        );
    }

    #[test]
    fn non_milestone_level_triggers_direct_message() {
        let mut s = ready_state();
        let action = s.on_level_up("星辰旅人", 7);
        assert!(
            matches!(action, LevelGreetAction::DirectMessage { .. }),
            "非里程碑等級應觸發私信"
        );
    }

    #[test]
    fn milestone_during_cooldown_falls_back_to_direct() {
        let mut s = ready_state();
        // 先觸發一次里程碑，設入冷卻
        let _ = s.on_level_up("玩家甲", 10);
        // 冷卻中，另一玩家也達里程碑 → 應降級為私信
        let action = s.on_level_up("玩家乙", 20);
        assert!(
            matches!(action, LevelGreetAction::DirectMessage { .. }),
            "全服廣播冷卻中，里程碑應降級為私信"
        );
    }

    #[test]
    fn cooldown_resets_after_world_broadcast() {
        let mut s = ready_state();
        let _ = s.on_level_up("玩家甲", 10);
        assert!(s.world_cooldown > 0.0, "里程碑廣播後應設入冷卻");
    }

    #[test]
    fn tick_decrements_cooldown() {
        let mut s = NpcLevelGreetState {
            world_cooldown: 60.0,
        };
        s.tick(10.0);
        assert!((s.world_cooldown - 50.0).abs() < 0.001, "tick 應減少冷卻");
    }

    #[test]
    fn tick_does_not_go_below_zero() {
        let mut s = NpcLevelGreetState {
            world_cooldown: 5.0,
        };
        s.tick(100.0);
        assert!(s.world_cooldown <= 0.0, "tick 不應使冷卻低於 0");
    }

    #[test]
    fn world_broadcast_text_contains_player_name() {
        let text = world_broadcast_text("鋼鐵戰士", 10);
        assert!(text.contains("鋼鐵戰士"), "廣播文字應包含玩家名稱");
        assert!(text.contains("10"), "廣播文字應包含等級數字");
    }

    #[test]
    fn direct_message_text_contains_player_name() {
        let text = direct_message_text("鋼鐵戰士", 7);
        assert!(text.contains("鋼鐵戰士"), "私信文字應包含玩家名稱");
        assert!(text.contains("7"), "私信文字應包含等級數字");
    }

    #[test]
    fn world_broadcast_varies_by_milestone_index() {
        let t10 = world_broadcast_text("X", 10);
        let t20 = world_broadcast_text("X", 20);
        let t30 = world_broadcast_text("X", 30);
        let t40 = world_broadcast_text("X", 40);
        // 四個連續里程碑應有不同文字（輪替 4 種語氣）
        assert_ne!(t10, t20, "里程碑 10 與 20 應有不同語氣");
        assert_ne!(t20, t30, "里程碑 20 與 30 應有不同語氣");
        assert_ne!(t30, t40, "里程碑 30 與 40 應有不同語氣");
        // 第 5 個里程碑（50）回到第 1 種語氣，but 含不同等級數字，開頭相同
        let t50 = world_broadcast_text("X", 50);
        // 確認是同一種模板（都以「恭賀」開頭）
        assert!(t10.starts_with("恭賀"), "Lv.10 應用第一種模板（恭賀…）");
        assert!(t50.starts_with("恭賀"), "Lv.50 應輪回到第一種模板（恭賀…）");
    }

    #[test]
    fn after_cooldown_expires_milestone_broadcasts_again() {
        let mut s = ready_state();
        let _ = s.on_level_up("玩家甲", 10);
        s.world_cooldown = 0.0; // 模擬冷卻結束
        let action = s.on_level_up("玩家乙", 20);
        assert!(
            matches!(action, LevelGreetAction::WorldBroadcast { .. }),
            "冷卻結束後里程碑應再次觸發全服廣播"
        );
    }
}
