//! 乙太方界·街頭手風琴 v1——世界第一次有玩家可親手操作的樂器，居民停下腳步聆聽鼓掌🪗🎶
//! （自主提案切片，ROADMAP 977；接續 974「遠征首領」/975「地底遺跡神殿」/976「蒸汽獨輪車」，
//! reviewer 明令「繼續開一塊新維度」）。
//!
//! **這一刀補的缺口（換維度，非同軸重複）**：乙太方界至今唯一與「音樂」沾邊的一筆是居民
//! 心情正好時自己哼起的歌（`voxel_humming`，ROADMAP 788）——那是居民**被動、獨自**的哼唱，
//! 玩家完全插不上手；集會鐘（`voxel_bell`）能把居民**召到**你身邊，但你到了之後什麼都做
//! 不了、只能乾站著。世界從沒有一件玩家能**主動操作、對著居民表演**的樂器——這正是
//! PLAN_ETHERVOX「玩家遊玩」主軸缺的一塊：不只是採集/合成/蓋造，也要有「秀給居民看」的樂趣。
//!
//! 本刀補上：工作台合成一把**街頭手風琴**，背著它站在村子裡按下演奏鍵，附近閒著的居民會
//! 停下手邊的事看向你、跟著拍子搖擺，並把「聽你演奏」記進交情——世界第一次由玩家**主動
//! 表演**、居民**當場駐足回應**。
//!
//! **與既有元素 razor-sharp 區隔**：
//! - 不是居民哼歌（`voxel_humming`）——那由居民**自己的心情**觸發、玩家旁聽不到不代表沒
//!   發生，純氛圍點綴；本刀由**玩家主動**按鍵觸發、有明確的**居民反應**（駐足+交情+記憶），
//!   玩家是表演的發起者而非旁觀者。
//! - 不是集會鐘（`voxel_bell`）——鐘聲**召喚居民走向你**（改變居民的移動目標）；手風琴
//!   **不移動任何人**，只讓已經在附近、閒著的居民原地駐足欣賞，觸發的是「反應」而非
//!   「位移」，兩者互補而非重複。
//! - 不是集體慶祝（煙火/生日）——那些是**事件驅動**的一次性慶典；手風琴是玩家**隨時可
//!   發起**的日常表演，沒有節慶前提。
//!
//! **純函式層**：本模組只有確定性純函式（範圍判定、反應台詞/記憶/動態牆文案），零 LLM、
//! 零鎖、零 IO、零 async，可單元測試。冷卻／掃描／廣播全留在 `voxel_ws.rs`（比照
//! `maybe_craft_admire` 的「靜態冷卻表 + 觸發時直接判定」慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：零 LLM（反應台詞/記憶/Feed 全確定性模板）、玩家觸發面只有一顆
//! 布林開關（`SetPerforming`）、伺服器權威驗真持有手風琴才准開演（比照騎乘/持劍驗證
//! 手法，不信客戶端自報）；每位居民 [`PERFORM_REACT_COOLDOWN_SECS`] 全域冷卻，狂按演奏鍵
//! 也拖不動同一位居民反覆駐足、不洗版泡泡/動態牆；不收玩家自由文字、不開對外端點。

/// 演奏能被聽見、觸發居民駐足反應的半徑（世界方塊，水平距離）——比集會鐘的召集半徑
/// （22）小得多：手風琴不召人，只讓「本來就在附近」的人聽見，不必老遠都跑來看你表演。
pub const PERFORM_RADIUS: f32 = 12.0;

/// 每位居民的駐足反應冷卻（秒）：一次反應過後這麼久才會再被同一場（或下一場）演奏打動，
/// **濫用防護主閘**——狂開關演奏也拖不動同一位居民反覆停下、不洗版泡泡/動態牆。
pub const PERFORM_REACT_COOLDOWN_SECS: u64 = 180;

/// 條件都滿足後這位居民是否真的會停下反應：閒著（非睡/非遠行等）+ 冷卻已過 + 距離在
/// 半徑內。純函式、確定性、可測。
pub fn perform_reaction_triggers(dist_sq: f32, idle: bool, cooldown_ok: bool) -> bool {
    idle && cooldown_ok && dist_sq <= PERFORM_RADIUS * PERFORM_RADIUS
}

/// 居民駐足聆聽時冒出的反應泡泡（四句輪替，確定性）。
pub fn perform_react_line(pick: usize) -> &'static str {
    const POOL: &[&str] = &[
        "🪗 這曲子真好聽，忍不住停下腳步～",
        "🎶 手不自覺跟著拍子晃了起來",
        "🎵 好久沒聽過這麼歡快的調子了",
        "👏 演奏得真好，值得為你鼓個掌！",
    ];
    POOL[pick % POOL.len()]
}

/// 聆聽者掛在演奏者名下的暖記憶（episodic，累積交情）。`player` 空 → 泛稱，仍不留空洞。
pub fn perform_memory_line(player: &str) -> String {
    if player.is_empty() {
        "有位旅人在村子裡彈起了手風琴，我停下腳步聽了好一會兒。".to_string()
    } else {
        format!("{player}在村子裡彈起了手風琴，我停下腳步聽了好一會兒，心裡暖暖的。")
    }
}

/// 城鎮動態牆一行：讓不在場/回來的玩家也讀到「有人開了一場街頭演奏」。`count` = 這次
/// 吸引了幾位居民駐足聆聽。
pub fn perform_feed_line(player: &str, count: usize) -> String {
    let who = if player.is_empty() { "有位旅人" } else { player };
    if count <= 1 {
        format!("{who}在村子裡彈起了手風琴，一位居民停下腳步聽得入神。")
    } else {
        format!("{who}在村子裡彈起了手風琴，{count}位居民停下腳步聽得入神。")
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perform_reaction_requires_idle() {
        assert!(!perform_reaction_triggers(1.0, false, true), "睡著/忙碌不該反應");
    }

    #[test]
    fn perform_reaction_requires_cooldown_ok() {
        assert!(!perform_reaction_triggers(1.0, true, false), "冷卻中不該反應");
    }

    #[test]
    fn perform_reaction_requires_within_radius() {
        let just_outside = PERFORM_RADIUS * PERFORM_RADIUS + 1.0;
        assert!(!perform_reaction_triggers(just_outside, true, true), "超出半徑不該反應");
    }

    #[test]
    fn perform_reaction_triggers_when_all_conditions_met() {
        assert!(perform_reaction_triggers(1.0, true, true));
    }

    #[test]
    fn perform_reaction_boundary_at_radius_inclusive() {
        let edge = PERFORM_RADIUS * PERFORM_RADIUS;
        assert!(perform_reaction_triggers(edge, true, true), "剛好等於半徑應觸發（<=）");
    }

    #[test]
    fn perform_react_line_non_empty_and_all_distinct() {
        let lines: std::collections::HashSet<_> = (0..4).map(perform_react_line).collect();
        assert_eq!(lines.len(), 4, "四句應各不相同");
        for pick in [0usize, 1, 2, 3, 99] {
            assert!(!perform_react_line(pick).is_empty());
        }
    }

    #[test]
    fn perform_memory_line_embeds_player_name() {
        assert!(perform_memory_line("露娜").contains("露娜"));
    }

    #[test]
    fn perform_memory_line_empty_player_falls_back_to_generic() {
        let line = perform_memory_line("");
        assert!(!line.is_empty());
        assert!(line.contains("有位旅人"));
    }

    #[test]
    fn perform_feed_line_mentions_count_when_plural() {
        let many = perform_feed_line("旅人", 3);
        assert!(many.contains('3'));
    }

    #[test]
    fn perform_feed_line_singular_reads_naturally() {
        let one = perform_feed_line("旅人", 1);
        assert!(one.contains("一位居民"));
    }

    #[test]
    fn perform_feed_line_empty_player_falls_back() {
        let line = perform_feed_line("", 2);
        assert!(line.contains("有位旅人"));
        assert!(!line.is_empty());
    }
}
