//! 乙太方界·你的名號口耳相傳 v1（自主提案切片·PLAN_ETHERVOX 路線圖④「居民↔居民關係／小社會
//! 湧現」＋核心信念「記憶要驅動**行為**、不只聊天」）。
//!
//! 774（`voxel_playerepithet`）讓**單一**居民把「關於某位玩家」的累積作為昇華成一個**名號**
//! （造物者／慷慨的人／老搭檔／常來的老友），並用它稱呼你。但那個名號至今只活在**那一位**居民
//! 心裡——她的朋友對你一無所知，你在整個小社會裡的「名聲」從沒流動過。
//!
//! 本模組把 774 換到**新軸**：讓名號**在居民之間口耳相傳**。承接既有「口耳相傳」機制
//! （`voxel_gossip`，694：老朋友到訪時主人把見聞轉述給訪客）——當一位居民（主人）已在心裡為某位
//! 玩家安下名號，老朋友來訪時她會**說起你**：「你知道那位『造物者』嗎……」。訪客從此**久仰**你，
//! 心裡記下這個名號＋一筆社交記憶。日後你頭一次撞見這位素未深交的居民，她卻已認得你，用一句
//! 「久仰」的招呼喊出你的名號——**你的名聲，第一次透過小社會自己傳開了。**
//!
//! 這是「記憶→行為」的社會化延伸：一位居民累積的印象，經由**朋友網絡**變成**另一位**居民
//! 怎麼稱呼你。玩家第一次能親眼見證「大家都聽說過我」這種湧現的口碑。
//!
//! **與 774 `voxel_playerepithet` 的分界（刻意區隔，非重複）**：
//! - 774 是**第一手**：某居民用**她自己**累積的記憶昇華名號、用名號招呼你（affinity ≥ 老友門檻）。
//! - 本模組是**第二手（傳聞）**：居民**沒有**跟你深交（affinity 不到門檻、無法自行昇華），純粹**聽
//!   相熟的朋友說起**才認得你——招呼語刻意帶「久仰／傳聞中」的口吻，與第一手名號招呼區分。
//!   一旦這位居民日後真的跟你處成老友、自己昇華出名號，第一手自然接管（見 `voxel_ws` 招呼接線）。
//!
//! **成本 / 濫用防護鐵律**（沿用 774 同守則）：全程**零 LLM**、**零新輸入面**、**零新端點**、
//! **零持久化 / 零 migration**（傳聞名號表純記憶體，重啟歸零，比照 774 `coined_epithets`）。輸出
//! **永遠是固定模板**（角色 → 確定性稱呼＋牽線者名），**絕不回放任何記憶原文或玩家原話**；只有本就
//! 會出現在招呼／動態牆的**玩家顯示名**與**居民名**會嵌入。
//!
//! 純邏輯層：挑可傳的名號、組招呼／Feed／記憶文字皆為**確定性純函式**，零 IO / 零鎖 / 零 async /
//! 零 LLM，窮舉可測。鎖與副作用（讀主人名號、寫訪客傳聞、冒泡泡、記動態）全在 `voxel_ws.rs`。
//! 不抄外部碼；繁中註解。

use std::collections::HashMap;

use crate::voxel_bonds::BondTier;
use crate::voxel_playerepithet::PlayerRole;

/// 老朋友到訪時，主人把某位玩家的名號說給訪客聽的機率。
/// 略低於見聞八卦（`voxel_gossip::gossip_chance`=0.35）——名號傳聞更難得、更有份量，
/// 稀少才有「原來大家都聽說過我」的驚喜，不淪為背景雜訊。只有「老朋友」夠熟才會這樣說起你。
pub fn share_chance(tier: BondTier) -> f32 {
    match tier {
        BondTier::Friend => 0.25,
        _ => 0.0,
    }
}

/// 一則「聽說來的名號」——某居民沒跟你深交，只從相熟的朋友口中聽過你是個怎樣的人。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hearsay {
    /// 聽來的角色名號（造物者／慷慨的人／…）。
    pub role: PlayerRole,
    /// 從哪位居民口中聽來（供招呼時點名牽線者，讓口碑有來處）。
    pub from: String,
}

/// 從主人為各玩家安下的名號（`coined`）裡，挑一個可以轉述給這位訪客的。
///
/// 只挑訪客**還完全不認得**的玩家——訪客既沒自己昇華過名號（`visitor_coined`）、也還沒聽誰說起過
/// （`visitor_heard`）。這模擬「把一位新朋友介紹給你認識」：已經認得的人不必再介紹一次
/// （避免同一名號反覆灌記憶／洗 Feed）。多位候選時取玩家名排序最前者，**確定性**可測。
///
/// 回傳 `(玩家名, 名號角色)`；沒有可傳的 → `None`。純函式。
pub fn pick_to_share(
    host_coined: &HashMap<String, PlayerRole>,
    visitor_coined: &HashMap<String, PlayerRole>,
    visitor_heard: &HashMap<String, Hearsay>,
) -> Option<(String, PlayerRole)> {
    let mut names: Vec<&String> = host_coined
        .keys()
        .filter(|p| !visitor_coined.contains_key(*p) && !visitor_heard.contains_key(*p))
        .collect();
    names.sort();
    names
        .first()
        .map(|p| ((*p).clone(), host_coined[*p]))
}

/// 居民**只聽說過你**（第二手傳聞）時打招呼的一句（≤ 呼叫端再截 40 字）。
///
/// 刻意帶「久仰／傳聞中」的口吻，與 774 第一手名號招呼（`greeting_for_role`）區分——讓玩家聽得出
/// 「這位其實還不認識我，是聽別人說的」。`pick`：呼叫端傳入的確定性擾動（如座標 bits），讓幾句
/// 招呼輪替、不機械重複。**隱私鐵律**：只由角色與牽線者名決定，永不含記憶原文 / 玩家原話。
pub fn hearsay_greeting_line(hs: &Hearsay, pick: usize) -> String {
    let ep = hs.role.epithet();
    let from: String = hs.from.chars().take(12).collect();
    let from = if from.trim().is_empty() { "朋友".to_string() } else { from };
    let lines = [
        format!("你就是傳聞中的「{ep}」吧？久仰了～"),
        format!("聽{from}提起過你，你就是那位「{ep}」！"),
        format!("原來你就是大家口中的「{ep}」呀，總算見著了。"),
    ];
    lines[pick % lines.len()].clone()
}

/// 動態牆（Feed）一句：某訪客居民第一次從朋友口中記住某玩家的名號時記一筆。
/// Feed 的 `resident` 欄另帶訪客名，故文案**不重複**嵌訪客名；主人名與玩家名嵌入（皆本就公開）。
pub fn spread_feed_line(host_name: &str, player_name: &str, role: PlayerRole) -> String {
    let host: String = host_name.chars().take(12).collect();
    let host = if host.trim().is_empty() { "一位老友".to_string() } else { host };
    let p: String = player_name.chars().take(12).collect();
    let p = if p.trim().is_empty() { "那位旅人".to_string() } else { p };
    format!("聽{host}提起，記住了大家口中的「{}」——{p}。", role.epithet())
}

/// 訪客居民記進**自己**記憶庫的一筆社交記憶（主體＝主人名，比照 `voxel_gossip` 的轉述慣例，
/// 刻意**不**掛在玩家名下，以免污染玩家角色分類／自我印象統計）。供日記／生命故事有社交痕跡。
pub fn heard_memory_summary(host_name: &str, player_name: &str, role: PlayerRole) -> String {
    let host: String = host_name.chars().take(12).collect();
    let host = if host.trim().is_empty() { "一位老友".to_string() } else { host };
    let p: String = player_name.chars().take(12).collect();
    let p = if p.trim().is_empty() { "一位旅人".to_string() } else { p };
    format!("聽{host}說起一位旅人{p}，大家都喚作「{}」。", role.epithet())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coined(pairs: &[(&str, PlayerRole)]) -> HashMap<String, PlayerRole> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn share_chance_only_friends() {
        assert_eq!(share_chance(BondTier::Stranger), 0.0);
        assert_eq!(share_chance(BondTier::Acquaintance), 0.0);
        assert!(share_chance(BondTier::Friend) > 0.0);
        // 比八卦稀少（更難得、更有份量）。
        assert!(share_chance(BondTier::Friend) < crate::voxel_gossip::gossip_chance(BondTier::Friend));
    }

    #[test]
    fn pick_shares_unknown_player() {
        let host = coined(&[("阿光", PlayerRole::Maker)]);
        let got = pick_to_share(&host, &HashMap::new(), &HashMap::new());
        assert_eq!(got, Some(("阿光".to_string(), PlayerRole::Maker)));
    }

    #[test]
    fn pick_skips_already_coined_by_visitor() {
        // 訪客自己就認得這位玩家（第一手）→ 不必再聽傳聞。
        let host = coined(&[("阿光", PlayerRole::Maker)]);
        let visitor_coined = coined(&[("阿光", PlayerRole::Giver)]);
        assert_eq!(pick_to_share(&host, &visitor_coined, &HashMap::new()), None);
    }

    #[test]
    fn pick_skips_already_heard() {
        let host = coined(&[("阿光", PlayerRole::Maker)]);
        let mut heard = HashMap::new();
        heard.insert(
            "阿光".to_string(),
            Hearsay { role: PlayerRole::Trader, from: "露娜".to_string() },
        );
        assert_eq!(pick_to_share(&host, &HashMap::new(), &heard), None);
    }

    #[test]
    fn pick_is_deterministic_by_name_order() {
        // 多位候選 → 取玩家名排序最前者，確定性。
        let host = coined(&[
            ("小北", PlayerRole::Giver),
            ("大東", PlayerRole::Maker),
        ]);
        let got = pick_to_share(&host, &HashMap::new(), &HashMap::new());
        // 「大東」<「小北」（字元序），取「大東」。
        assert_eq!(got.map(|(p, _)| p), Some("大東".to_string()));
    }

    #[test]
    fn pick_none_when_host_has_no_epithets() {
        assert_eq!(pick_to_share(&HashMap::new(), &HashMap::new(), &HashMap::new()), None);
    }

    #[test]
    fn hearsay_greeting_non_empty_fits_bubble_and_mentions_epithet() {
        let hs = Hearsay { role: PlayerRole::Maker, from: "露娜".to_string() };
        for pick in 0..6 {
            let line = hearsay_greeting_line(&hs, pick);
            assert!(!line.is_empty());
            assert!(line.chars().count() <= 40, "招呼不可破泡泡框：{line}");
            assert!(line.contains("造物者"), "招呼要含名號：{line}");
        }
    }

    #[test]
    fn hearsay_greeting_rotates() {
        let hs = Hearsay { role: PlayerRole::Giver, from: "露娜".to_string() };
        let a = hearsay_greeting_line(&hs, 0);
        let b = hearsay_greeting_line(&hs, 1);
        assert_ne!(a, b, "不同 pick 應輪替出不同招呼");
    }

    #[test]
    fn hearsay_greeting_empty_from_is_safe() {
        let hs = Hearsay { role: PlayerRole::Companion, from: String::new() };
        let line = hearsay_greeting_line(&hs, 1);
        assert!(!line.is_empty());
        assert!(!line.contains("聽提起"), "空牽線者名不可露出破碎字串：{line}");
    }

    #[test]
    fn feed_and_memory_contain_names_and_epithet() {
        let feed = spread_feed_line("露娜", "阿光", PlayerRole::Maker);
        assert!(feed.contains("露娜") && feed.contains("阿光") && feed.contains("造物者"));
        let mem = heard_memory_summary("露娜", "阿光", PlayerRole::Trader);
        assert!(mem.contains("露娜") && mem.contains("阿光") && mem.contains("老搭檔"));
    }

    #[test]
    fn feed_and_memory_empty_names_are_safe() {
        let feed = spread_feed_line("", "", PlayerRole::Giver);
        assert!(!feed.is_empty() && feed.contains("慷慨的人"));
        let mem = heard_memory_summary("", "", PlayerRole::Companion);
        assert!(!mem.is_empty() && mem.contains("常來的老友"));
    }

    #[test]
    fn memory_subject_is_host_not_player_to_avoid_polluting_role_classification() {
        // 記憶文字不含任何 classify_role 會命中的「作為關鍵詞」——避免傳聞被誤算成玩家的第一手作為。
        let mem = heard_memory_summary("露娜", "阿光", PlayerRole::Maker);
        assert!(crate::voxel_playerepithet::classify_role(&mem).is_none(),
            "傳聞記憶不該被歸類成玩家作為：{mem}");
    }
}
