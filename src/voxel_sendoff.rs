//! 乙太方界·為遠行的夥伴送行 v1（voxel-sendoff，
//! PLAN_ETHERVOX item 4「居民↔居民關係」× item 7「散居·遠行探野」，ROADMAP 902）。
//!
//! **真缺口**：散居·遠行（756~762，`voxel_expedition`）讓愛四處走的居民（奧瑞·漂泊／諾娃·尋地）
//! 偶爾放下手邊的事、獨自走進遠離主城的荒野邊陲住上一陣子；而情誼系統（`voxel_bonds`）早已在
//! 居民之間織出「相識／老朋友」的關係網。但這兩套系統在**啟程那一刻**從不相識——奧瑞轉身走向
//! 荒野時，就算身邊正站著一位跟牠交情深厚的老鄰居，那位鄰居也只是自顧自地做自己的事，一句話
//! 都不會說。居民的遠行至今永遠是**沒有人相送的孤獨啟程**：世界替牠在動態牆上寫一句「奧瑞往東方
//! 的邊陲遠行了」，卻沒有任何一位夥伴真的**因為牠要走、而停下來道一聲珍重**。
//!
//! **本刀**：把「遠行啟程」接上「居民情誼」——當一位居民啟程遠行的那一刻，若身邊剛好有一位
//! **醒著、與牠相識以上（Acquaintance／Friend）的老鄰居**，那位鄰居會停下手邊的事、朝遠行的
//! 夥伴道一句珍重再見；這份到村口送一程的心意讓兩人交情再深一分（`bonds` 記一次往來），送行者
//! 也把「今天為夥伴送了行」記進心裡（episodic，掛遠行夥伴名下），動態牆補一則讓不在場的玩家
//! 回來也讀得到。遠行第一次在**離開的那一刻**有人相送——小社會的溫度，不再只在重逢時亮起。
//!
//! **與既有元素的定位區隔**：
//! - **邊陲探友（821，`voxel_frontier_visit`）**走的是**抵達之後**、留守者**跋涉到荒野盡頭**去找
//!   正在那逗留的朋友（空間跨域的重聚）；本刀是**啟程那一刻**、就近的鄰居在**主城原地**送牠一程
//!   （時間點截然不同：一個是「出發」、一個是「已在遠方」），兩者一送一迎、互補而不重疊。
//! - **久別奔迎（747，`voxel_reunion`）**是居民朝**玩家**歸來奔去；本刀是居民為**另一位居民**的
//!   離開送行——一個朝人類、一個在居民之間，且方向相反（一迎歸來、一送離開）。
//!
//! **純邏輯層**：本檔全是零 IO／零鎖／零 LLM／零 async 的確定性純函式（送行者資格判定、道別台詞
//! 選句、記憶／動態牆文案），可獨立窮舉單元測試。挑「就近、醒著、相識以上」的送行者、say/記憶/
//! Feed 落地都在 `voxel_ws.rs`（沿用邊陲探友 821／見證圓夢 witness 的短鎖循序＋鎖外落地慣例）。
//!
//! **成本／安全紀律**：零 LLM（判定＋台詞皆確定性）、零 migration（不新增持久欄位，記憶走既有
//! episodic 層、bonds 走既有往來計數）、零協議破壞（不動任何 WS/HTTP 欄位，say/Feed/memory 皆
//! 既有管線）、零新美術、零前端改動、FPS 零影響（僅在極少數「啟程」瞬間掃 4 位居民一次）。
//! **濫用防護**：不收玩家輸入、不觸發 LLM、不開對外端點、不動帳號權限——純伺服器內部確定性反應，
//! 送行者與台詞皆由後端在啟程那刻算出，玩家無從觸發或洗版；台詞／記憶皆內建常數、無玩家可注入內容。

use crate::voxel_bonds::BondTier;

/// 送行者必須離啟程者多近（格）才會停下送行——太遠的居民看不見對方啟程、不會憑空知道要送行。
pub const SENDOFF_RADIUS: f32 = 16.0;

/// Feed 事件類型字串（面向玩家、集中可 i18n）。
pub const FEED_KIND: &str = "遠行送行";

/// 送行者朝啟程夥伴道別的泡泡池（確定性輪替、輪替有界、皆點出夥伴名字，其中一句點出方位）。
const FAREWELL_LINES: &[&str] = &[
    "{t}，路上小心，早點回來啊！",
    "又要往{b}去啦？替我看看那邊的風景，平安回來。",
    "{t}要遠行了……珍重，我在村裡等你消息。",
    "別走太久喔，{t}！回來記得跟我說說一路上的見聞。",
];

/// 是否夠格當送行者：離得夠近 ∧ 醒著 ∧ 交情達「相識」以上（陌生人不會憑空送行）。
/// 純閘——「挑最近的一位合格鄰居」由呼叫端（ws）在快照上做。
pub fn qualifies_as_sender(dist_sq: f32, awake: bool, tier: BondTier) -> bool {
    awake && tier != BondTier::Stranger && dist_sq <= SENDOFF_RADIUS * SENDOFF_RADIUS
}

/// 送行者朝遠行夥伴道的一句珍重（確定性選句，`pick` 取模輪替、對任意 `pick` 都有界不 panic）。
pub fn farewell_bubble(traveler: &str, bearing: &str, pick: usize) -> String {
    let tmpl = FAREWELL_LINES[pick % FAREWELL_LINES.len()];
    tmpl.replace("{t}", traveler).replace("{b}", bearing)
}

/// 動態牆文案：讓不在場／回來的玩家也讀到「某位鄰居為某位夥伴的遠行送了行」。
/// 前端 Feed 標頭已顯示送行者名字＋類型，故此處只寫謂語、不重複送行者名。
pub fn sendoff_feed_line(traveler: &str, bearing: &str) -> String {
    format!("為往{bearing}的邊陲遠行的{traveler}送了行，到村口道一聲珍重")
}

/// 送行者把「今天為夥伴送行」昇華成的記憶摘要（episodic，掛遠行夥伴名下）。
pub fn sendoff_memory_line(traveler: &str, bearing: &str) -> String {
    format!("今天{traveler}啟程往{bearing}的邊陲遠行，我放下手邊的事，到村口送了牠一程。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 資格判定_近醒相識才夠格() {
        let r = SENDOFF_RADIUS;
        // 醒著＋老朋友＋在半徑內 → 夠格。
        assert!(qualifies_as_sender(0.0, true, BondTier::Friend));
        assert!(qualifies_as_sender(r * r, true, BondTier::Friend)); // 邊界（恰在半徑上）算數。
        // 相識也夠格（送行是比跋涉探友更輕的舉手之勞）。
        assert!(qualifies_as_sender(4.0, true, BondTier::Acquaintance));
        // 陌生人不會憑空送行。
        assert!(!qualifies_as_sender(0.0, true, BondTier::Stranger));
        // 睡著的鄰居不會起身送行。
        assert!(!qualifies_as_sender(0.0, false, BondTier::Friend));
        // 太遠（超出半徑）看不見啟程。
        assert!(!qualifies_as_sender(r * r + 1.0, true, BondTier::Friend));
    }

    #[test]
    fn 道別泡泡_非空且點名輪替有界() {
        // 四句循環、皆非空、佔位符皆被替換、彼此相異。
        let mut seen = std::collections::HashSet::new();
        let mut named = 0;
        for pick in 0..FAREWELL_LINES.len() {
            let s = farewell_bubble("奧瑞", "東方", pick);
            assert!(!s.is_empty());
            assert!(!s.contains("{t}") && !s.contains("{b}"), "佔位符該全被替換：{s}");
            if s.contains("奧瑞") {
                named += 1;
            }
            seen.insert(s);
        }
        assert_eq!(seen.len(), FAREWELL_LINES.len(), "每句應相異（輪替有變化）");
        // 多數句子直呼夥伴名字（其餘用對話式口吻帶到，如「又要往東方去啦」），確保足夠個人化。
        assert!(named >= FAREWELL_LINES.len() - 1, "應有多數句子直呼夥伴名字");
        // 任意大 pick 取模不 panic、且與同餘的句子一致（有界輪替）。
        assert_eq!(
            farewell_bubble("諾娃", "西方", 4),
            farewell_bubble("諾娃", "西方", 0)
        );
        assert_eq!(
            farewell_bubble("諾娃", "西方", 999),
            farewell_bubble("諾娃", "西方", 999 % FAREWELL_LINES.len())
        );
    }

    #[test]
    fn 方位_至少一句道別會帶到() {
        // 池中至少一句會把方位嵌進去（「往{b}去」那句），確保方位資訊有出口。
        let any_bearing = (0..FAREWELL_LINES.len())
            .any(|p| farewell_bubble("奧瑞", "北方的雪原", p).contains("北方的雪原"));
        assert!(any_bearing, "應至少有一句道別點出方位");
    }

    #[test]
    fn 動態牆與記憶_含名字與方位() {
        let feed = sendoff_feed_line("奧瑞", "東方");
        assert!(feed.contains("奧瑞") && feed.contains("東方"));
        assert!(!feed.is_empty());

        let mem = sendoff_memory_line("奧瑞", "東方");
        assert!(mem.contains("奧瑞") && mem.contains("東方"));
        // 記憶單行（不含換行，避免落地時破格／注入）。
        assert!(!mem.contains('\n'));
    }
}
