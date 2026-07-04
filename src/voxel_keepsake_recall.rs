//! 乙太方界·居民路過你送的紀念物時，會駐足想起你 v1（voxel-keepsake-recall，自主提案切片 ROADMAP 784）。
//!
//! **缺口 / 為誰做**：keepsake（732）讓玩家送居民的可展示餽贈真的被擺進世界——她把你送的木頭／
//! 火把／冰晶燈放在腳邊當紀念物，你日後路過還看得到。但那塊方塊擺下去之後就**只是裝飾**：世界裡
//! 多了一塊磚，居民卻再也沒為它做過任何事。你送的心意在世界留了痕，卻沒在她的日常裡留下**回響**。
//!
//! 本切片把那塊紀念物接成一個活著的東西——**居民在自家附近閒晃、恰好路過她擺出的那件你送的紀念物時，
//! 偶爾會駐足下來，輕聲說一句「這是旅人送我的，我一直好好擺著」，睹物思人、想起你**；這份「又想起了你」
//! 也記進你們的交情、上動態牆。你送出的一份小禮，第一次不只在世界裡留下一塊方塊，還在她心裡一次次泛起
//! 漣漪——正中 PLAN_ETHERVOX 核心信念「**記憶要驅動行為、你的互動真的有後果**」：她記得這是誰送的
//! （keepsake 落地時寫下的那筆記憶＋腳邊那塊方塊），於是行為上真的為它停下腳步。
//!
//! **與既有系統的分界**：
//! - 不是 keepsake（732）本身——732 是「收到禮物→擺成方塊」的**一次性落地**；本切片是那塊方塊落地
//!   **之後**、居民日常路過時**反覆**觸發的睹物思人（記憶→持續行為）。
//! - 不是讀牌朝聖（751/pilgrimage）——那是居民**專程走向**心中某塊告示牌；本切片零新尋路，只在她
//!   **恰好經過**自家腳邊那塊紀念物時才觸發（紀念物就擺在她家搆得到的地方，閒晃自然會經過）。
//! - 不是望星邀約（783）——那是天象＋偏好驅動、把玩家**喚到身邊**；本切片是物件觸發的**獨自追憶**，
//!   玩家在旁就看得到、不在也照樣泛起（記進交情但不點名邀約）。
//!
//! **成本 / 濫用防護鐵律**：
//! - **純邏輯層**：靠近判定、觸發判定、追憶台詞／記憶／Feed 全為確定性純函式，零 LLM、零鎖、零 IO、
//!   可窮舉單元測試。鎖與副作用全在 `voxel_ws.rs`（短鎖即釋、不巢狀、記憶／Feed 走鎖外事件佇列，守
//!   prod 死鎖鐵律）。
//! - 台詞永不回放玩家原話——只嵌玩家**顯示名**與**紀念物名**（皆既有安全字串），無注入 / NSFW 風險。
//! - 每居民一份紀念物座標小佇列（上限 [`MAX_SPOTS`]、去重），純記憶體、重啟歸零、零 migration
//!   （那塊方塊本身仍由 keepsake 持久化在世界裡；重啟後追憶待下次擺紀念物才重新掛上，優雅退化）。
//! - 長冷卻（[`RECALL_COOLDOWN_SECS`]）＋極低機率（[`RECALL_CHANCE`]）＝睹物思人是偶爾的溫柔一拍、
//!   不洗版；各居民初始錯開。
//! - 面向玩家字串集中此處（i18n 友善）；繁中註解；不碰玩家資料表。

/// Feed 播報種類名稱（動態牆分類）。
pub const FEED_KIND: &str = "睹物思人";

/// 睹物思人冷卻（秒）：一次追憶後設此值，歸零前不再觸發——偶爾一拍才有感、不洗版。
pub const RECALL_COOLDOWN_SECS: f32 = 210.0;

/// 觸發半徑（格）：居民離她擺出的紀念物多近才算「路過、看到了」。紀念物就擺在她家搆得到處，
/// 閒晃自然會進出這個半徑。用距離平方比較，避免開根號。
pub const RECALL_NEAR_RADIUS: f32 = 2.6;

/// 每次「符合條件的 tick」真的駐足追憶的機率（極低）——配合長冷卻＝天然節流。
pub const RECALL_CHANCE: f32 = 0.05;

/// 每居民最多記幾件你送的紀念物座標（純記憶體佇列上限，防無限膨脹）。
pub const MAX_SPOTS: usize = 8;

/// 一件擺在世界裡的紀念物：座標 + 紀念物名 + 送禮玩家名（純記憶體，重啟歸零）。
#[derive(Clone, Debug, PartialEq)]
pub struct KeepsakeSpot {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 紀念物顯示名（如「木頭」「火把」）——既有安全字串。
    pub item: String,
    /// 送這份心意的玩家顯示名——既有安全字串。
    pub giver: String,
}

/// 把一件新擺出的紀念物記進居民的追憶佇列：同座標覆蓋（不重複堆疊），超過 `max` 丟最舊那件
/// （FIFO，保最近擺的）。純函式、可測。
pub fn remember_spot(spots: &mut Vec<KeepsakeSpot>, spot: KeepsakeSpot, max: usize) {
    // 同座標已有 → 更新內容（例如同一格重擺，理論上不會但求穩健）。
    if let Some(existing) = spots
        .iter_mut()
        .find(|s| s.x == spot.x && s.y == spot.y && s.z == spot.z)
    {
        *existing = spot;
        return;
    }
    spots.push(spot);
    // 超過上限就從最舊的丟起（保最近擺的那幾件）。
    while spots.len() > max {
        spots.remove(0);
    }
}

/// 找居民腳下位置附近、觸發半徑內最近的一件紀念物，回傳其索引。無則 `None`。
/// 只比水平距離（居民與紀念物大致同層），用平方比較。純函式、可測。
pub fn nearest_spot(spots: &[KeepsakeSpot], rx: f32, rz: f32, radius: f32) -> Option<usize> {
    let r2 = radius * radius;
    let mut best: Option<(usize, f32)> = None;
    for (i, s) in spots.iter().enumerate() {
        let dx = s.x as f32 + 0.5 - rx;
        let dz = s.z as f32 + 0.5 - rz;
        let d2 = dx * dx + dz * dz;
        if d2 <= r2 && best.map_or(true, |(_, bd)| d2 < bd) {
            best = Some((i, d2));
        }
    }
    best.map(|(i, _)| i)
}

/// 是否此刻駐足追憶：靠近某件紀念物 + 冷卻已過 + 過機率門檻，三者皆備才觸發。
/// say 是否為空、是否醒著由呼叫端在外層先確認（沿用 stargaze 慣例）。純函式、可測。
pub fn should_recall(near: bool, cooldown_ok: bool, roll: f32, threshold: f32) -> bool {
    near && cooldown_ok && roll < threshold
}

/// 睹物思人的泡泡台詞（獨自追憶、面向玩家）。只嵌玩家顯示名與紀念物名，永不回放原話；
/// 依 `pick` 確定性選句、截 40 字防泡泡溢框。
pub fn recall_line(giver: &str, item: &str, pick: usize) -> String {
    let pool: [&str; 4] = [
        "{g}送我的{i}還在這兒呢…每次看到就想起他。",
        "這份{i}是{g}的心意，我一直好好擺著。",
        "看到{g}送的{i}，心裡就暖暖的。",
        "{g}送的{i}擺在這，日子過得再忙也記得他。",
    ];
    clip(pool[pick % pool.len()].replace("{g}", giver).replace("{i}", item))
}

/// 睹物思人記憶摘要（掛在送禮玩家名下，供日記昇華成生命故事）。只嵌玩家名與紀念物名。
pub fn recall_memory_line(giver: &str, item: &str, pick: usize) -> String {
    let pool: [&str; 3] = [
        "我又看著{g}送的{i}，想起了他。",
        "{g}送的{i}我一直留著，今天又因它想起他。",
        "路過腳邊那件{g}送的{i}，心頭一暖。",
    ];
    clip(pool[pick % pool.len()].replace("{g}", giver).replace("{i}", item))
}

/// 動態牆播報句。
pub fn recall_feed_line(resident: &str, giver: &str, item: &str) -> String {
    clip(format!("{resident}駐足在{giver}送的{item}前，想起了他"))
}

/// 泡泡／記憶／Feed 統一截字（≤40 字，防溢框）。
fn clip(line: String) -> String {
    line.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(x: i32, y: i32, z: i32, item: &str, giver: &str) -> KeepsakeSpot {
        KeepsakeSpot { x, y, z, item: item.into(), giver: giver.into() }
    }

    #[test]
    fn remember_dedups_same_coord_and_caps_fifo() {
        let mut spots = Vec::new();
        // 同座標重擺 → 覆蓋、不堆疊。
        remember_spot(&mut spots, spot(1, 2, 3, "木頭", "諾娃"), MAX_SPOTS);
        remember_spot(&mut spots, spot(1, 2, 3, "火把", "諾娃"), MAX_SPOTS);
        assert_eq!(spots.len(), 1);
        assert_eq!(spots[0].item, "火把");
        // 超過上限丟最舊。
        for i in 0..MAX_SPOTS as i32 {
            remember_spot(&mut spots, spot(10 + i, 0, 0, "木頭", "諾娃"), MAX_SPOTS);
        }
        assert_eq!(spots.len(), MAX_SPOTS);
        // 最舊那件（1,2,3）已被擠掉。
        assert!(!spots.iter().any(|s| s.x == 1 && s.y == 2 && s.z == 3));
    }

    #[test]
    fn nearest_spot_picks_closest_within_radius() {
        let spots = vec![
            spot(5, 0, 5, "木頭", "諾娃"),
            spot(0, 0, 0, "火把", "諾娃"),
        ];
        // 居民站在 (0.4, 0.4) 附近 → 最近的是 (0,0,0) 那件（含 +0.5 中心）。
        assert_eq!(nearest_spot(&spots, 0.4, 0.4, RECALL_NEAR_RADIUS), Some(1));
        // 都在半徑外 → None。
        assert_eq!(nearest_spot(&spots, 50.0, 50.0, RECALL_NEAR_RADIUS), None);
        // 空佇列 → None。
        assert_eq!(nearest_spot(&[], 0.5, 0.5, RECALL_NEAR_RADIUS), None);
    }

    #[test]
    fn nearest_spot_radius_boundary_inclusive() {
        let spots = vec![spot(0, 0, 0, "木頭", "諾娃")];
        // 紀念物中心 (0.5,0.5)；居民站在正好 radius 距離處應算「在範圍內」（<=）。
        let rx = 0.5 + RECALL_NEAR_RADIUS;
        assert_eq!(nearest_spot(&spots, rx, 0.5, RECALL_NEAR_RADIUS), Some(0));
        // 再遠一點點就出界。
        assert_eq!(nearest_spot(&spots, rx + 0.01, 0.5, RECALL_NEAR_RADIUS), None);
    }

    #[test]
    fn should_recall_needs_all_conditions() {
        assert!(should_recall(true, true, 0.01, RECALL_CHANCE));
        // 不靠近 → 否。
        assert!(!should_recall(false, true, 0.01, RECALL_CHANCE));
        // 冷卻未過 → 否。
        assert!(!should_recall(true, false, 0.01, RECALL_CHANCE));
        // 骰子過門檻 → 否。
        assert!(!should_recall(true, true, 0.9, RECALL_CHANCE));
    }

    #[test]
    fn should_recall_chance_boundary() {
        // roll 正好等於 threshold 不觸發（嚴格小於）。
        assert!(!should_recall(true, true, RECALL_CHANCE, RECALL_CHANCE));
        assert!(should_recall(true, true, RECALL_CHANCE - 0.001, RECALL_CHANCE));
    }

    #[test]
    fn lines_embed_names_and_clip_and_rotate() {
        for pick in 0..8 {
            let bubble = recall_line("諾娃", "冰晶燈", pick);
            assert!(bubble.contains("諾娃") && bubble.contains("冰晶燈"));
            assert!(bubble.chars().count() <= 40 && !bubble.is_empty());
            let mem = recall_memory_line("諾娃", "冰晶燈", pick);
            assert!(mem.contains("諾娃") && mem.contains("冰晶燈"));
            assert!(mem.chars().count() <= 40 && !mem.is_empty());
        }
        // pick 輪替換句。
        assert_ne!(recall_line("諾娃", "木頭", 0), recall_line("諾娃", "木頭", 1));
        let feed = recall_feed_line("露娜", "諾娃", "火把");
        assert!(feed.contains("露娜") && feed.contains("諾娃") && feed.contains("火把"));
        assert!(feed.chars().count() <= 40);
    }

    #[test]
    fn recall_line_never_empty_for_long_names() {
        // 超長玩家名不會截到破壞辨識或空字串。
        let long = "超級無敵長長長長長長長長長長長長長的旅人名字";
        let line = recall_line(long, "木頭", 0);
        assert!(!line.is_empty() && line.chars().count() <= 40);
    }
}
