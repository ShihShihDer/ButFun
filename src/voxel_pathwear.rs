//! 乙太方界·居民踏出來的小徑 v1（自主提案切片，`docs/PLAN_ETHERVOX.md` §4 居民↔居民關係／小社會湧現）。
//!
//! **真缺口**：居民一路長出了作息與關係——他們在家、水井、工作點、彼此的門前之間
//! 日復一日往返（散心採集、互助蓋家、拜訪、遠行歸來……），但這些習慣性的路線至今
//! **不在世界裡留下任何痕跡**：你走進乙太方界，看不出「露娜每天都往水井走那一條」。
//! 小社會的地理（誰常去哪、村子的動線長什麼樣）只活在伺服器的座標裡，從沒長進玩家
//! 看得到的地面。近期環境軸讓「地上的方塊」隨季節換色（920 四季樹葉、922 冬雪覆地），
//! 但那是**天象**驅動的被動染色；還沒有一刀讓地面因**居民自己的行為**而改變。
//!
//! 本模組補上那一拍：**居民反覆走過同一片草地，久了會把草皮踩踏成一條泥土小徑**。
//! 你某天回到村裡，會發現露娜家到水井之間的草地被踏出了一條淡淡的褐色小路——那是
//! 這座小村自己走出來的動線，是「小社會湧現」第一次寫進世界的地面本身。
//!
//! **與既有系統 razor-sharp 區隔**：
//! - 與 920 四季樹葉／922 冬雪覆地＝**天象/季節**被動染整片同類方塊、且純前端本地決定；
//!   本刀＝**居民行為**主動、逐格、把草**換成另一種方塊**（草→泥），是伺服器權威的真世界變更。
//! - 與 732 keepsake（你送的心意她擺出來）＝**玩家送禮**觸發居民放一塊**紀念物**；
//!   本刀＝**沒有任何玩家介入**，純由居民自己的日常往返累積而生（湧現，非玩家指令）。
//! - 與居民「整地」（號召鋪平大地）＝玩家**下指令**、大範圍、瞬間；本刀＝**無人指揮**、
//!   細窄、極慢，是走出來的而非鋪出來的。
//!
//! **成本紀律（鐵律）**：零 LLM、零新方塊種類（泥土 `Dirt` 早就存在、早就會渲染，
//! 零新美術）、零新協議欄位（小徑走既有 `broadcast_block` 廣播、玩家端當普通方塊變更收）、
//! 零 migration（走既有 `append_world_block` append-only 持久化，重啟後小徑仍在）。
//! **FPS 零影響**：磨損計數只在低頻 tick（`~1.7Hz`）掃**少數幾位**居民的腳下格、且只在
//! 「踏進新格子」那一步才記一筆（站著不動不計），純 HashMap 加法；達門檻才偶爾放一塊土。
//!
//! **濫用防護**：本切片**不收玩家輸入、不觸發 LLM、不開對外端點、不動帳號權限**——
//! 小徑完全由伺服器權威的居民座標驅動，玩家無從自報或催發、也無從洗版。
//!
//! **刻意的邊界**：只有**天然草皮**（`Block::Grass`）會被踏成小徑；農田土、玩家/居民
//! 放下的方塊、沙地、已成型的小徑本身都不受影響（見 [`wears_into_path`]）——絕不踩壞
//! 玩家的作物或建材。只計**居民**的往返（v1 聚焦「村子自己的動線」；玩家個人足跡留待後續）。
//!
//! **純邏輯層**：本模組零 IO／零鎖／零 LLM／零 async，全是確定性純函式與一個記憶體
//! 計數器，可完整單元測試。取樣居民座標／讀地表方塊／放土／廣播／持久化全在
//! `voxel_ws.rs`（`tick_pathwear`，短鎖循序取放、不巢狀，守死鎖鐵律）。

use crate::voxel::Block;
use std::collections::HashMap;

/// 一格草皮要被「踏進」幾次才磨損成小徑。刻意設得不低：小徑是**日積月累**走出來的，
/// 常走的動線（家↔井↔工作點）會自然累積、偏僻處永遠不會無端變土。
pub const PATHWEAR_THRESHOLD: u16 = 28;

/// 磨損計數器上限：避免罕見情況下記憶體無界增長（村子動線就那幾條，遠低於此）。
/// 超過則不再登記新格（既有格仍照常累積、照常磨損），純安全閥。
pub const MAX_TRACKED_CELLS: usize = 4096;

/// 把居民的浮點腳下座標落到整數格。
#[inline]
pub fn ground_cell(x: f32, z: f32) -> (i32, i32) {
    (x.floor() as i32, z.floor() as i32)
}

/// 這一步是否「踏進了一個新格子」（相對上一次取樣所在的格）。
/// 站著不動（同一格）不算一步——小徑是**走**出來的，不是站出來的，
/// 避免卡住/發呆的居民在腳下磨出一塊泥。
#[inline]
pub fn stepped_into(prev: Option<(i32, i32)>, now: (i32, i32)) -> bool {
    prev != Some(now)
}

/// 哪種地表方塊會被走成小徑——只有**天然草皮**。其餘（泥土/沙/農田/建材/水…）皆不受影響：
/// 既保護玩家的作物與建材，也讓已成型的小徑（本身已是泥土）不再重複觸發。
#[inline]
pub fn wears_into_path(surface: Block) -> bool {
    matches!(surface, Block::Grass)
}

/// 草皮踏成小徑後換上的方塊——沿用既有泥土（自然、早已渲染、零新美術）。
#[inline]
pub fn worn_block() -> Block {
    Block::Dirt
}

/// 全村草皮磨損計數器（純記憶體、重啟歸零——已成型的小徑本身已是持久化的泥土方塊，
/// 重啟後仍在；重啟時歸零的只是「還沒踏成小徑的半途計數」，沿用 `last_seen` 慣例）。
#[derive(Default)]
pub struct PathWear {
    /// (格x, 格z) → 累計被踏進次數。達門檻轉成小徑後即移除該鍵（之後那格已是泥、不再計數）。
    steps: HashMap<(i32, i32), u16>,
}

impl PathWear {
    pub fn new() -> Self {
        Self { steps: HashMap::new() }
    }

    /// 記錄「有居民踏進了 (cx,cz) 這一格」一次。
    ///
    /// 回傳 `true` 表示這一步讓該格累計達到 [`PATHWEAR_THRESHOLD`]、該磨成小徑了
    /// ——呼叫端須再自行確認「該格地表此刻真的是天然草皮」（見 [`wears_into_path`]）
    /// 才放土，並在放成後呼叫 [`clear`](Self::clear) 收掉這格的計數。
    ///
    /// 安全閥：計數器已達 [`MAX_TRACKED_CELLS`] 且該格是新格時，直接忽略（回 `false`），
    /// 不讓記憶體無界增長。
    pub fn record_step(&mut self, cx: i32, cz: i32) -> bool {
        let key = (cx, cz);
        if !self.steps.contains_key(&key) && self.steps.len() >= MAX_TRACKED_CELLS {
            return false; // 安全閥：不登記新格
        }
        let e = self.steps.entry(key).or_insert(0);
        *e = e.saturating_add(1);
        *e >= PATHWEAR_THRESHOLD
    }

    /// 該格已磨成小徑（或不再需要追蹤），清掉它的計數。冪等（本就沒有也無妨）。
    pub fn clear(&mut self, cx: i32, cz: i32) {
        self.steps.remove(&(cx, cz));
    }

    /// 目前正在累積磨損的格子數（供測試與觀測，非熱路徑）。
    pub fn tracked_cells(&self) -> usize {
        self.steps.len()
    }
}

/// 全村第一次踏出小徑時，上一則溫柔的城鎮動態（每個行程至多一次，見 `voxel_ws.rs` 的
/// 一次性旗標）。措辭刻意不宣稱「史上第一條」（重啟後計數歸零可能再觸發），只描述當下所見。
pub fn first_path_feed_line() -> &'static str {
    "居民日復一日往返，草地上被悄悄踏出了一條小徑。"
}

/// 城鎮動態的事件類型字串（面向玩家、集中可 i18n）。
pub const FEED_KIND: &str = "小徑";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_cell_floors_toward_negative_infinity() {
        assert_eq!(ground_cell(3.9, 7.1), (3, 7));
        assert_eq!(ground_cell(-0.1, -2.9), (-1, -3));
        assert_eq!(ground_cell(0.0, 0.0), (0, 0));
    }

    #[test]
    fn stepped_into_only_counts_new_cells() {
        assert!(stepped_into(None, (1, 1))); // 第一次取樣就算一步
        assert!(stepped_into(Some((1, 1)), (2, 1))); // 走到新格
        assert!(!stepped_into(Some((2, 1)), (2, 1))); // 原地不動不算
    }

    #[test]
    fn only_grass_wears_into_a_path() {
        assert!(wears_into_path(Block::Grass));
        // 已成型小徑本身是泥、農田/沙/建材皆不受影響（保護作物與建材、不重複觸發）。
        assert!(!wears_into_path(Block::Dirt));
        assert!(!wears_into_path(Block::Sand));
        assert!(!wears_into_path(Block::FarmSoil));
        assert!(!wears_into_path(Block::Plank));
        assert!(!wears_into_path(Block::Water));
    }

    #[test]
    fn worn_path_block_is_plain_dirt() {
        assert_eq!(worn_block(), Block::Dirt);
    }

    #[test]
    fn record_step_reaches_threshold_exactly_once_at_boundary() {
        let mut w = PathWear::new();
        // 前 THRESHOLD-1 步都還不夠。
        for _ in 0..(PATHWEAR_THRESHOLD - 1) {
            assert!(!w.record_step(5, 5));
        }
        // 第 THRESHOLD 步剛好達標。
        assert!(w.record_step(5, 5));
        assert_eq!(w.tracked_cells(), 1);
    }

    #[test]
    fn different_cells_accumulate_independently() {
        let mut w = PathWear::new();
        for _ in 0..PATHWEAR_THRESHOLD {
            w.record_step(1, 1);
        }
        // (1,1) 已達標，(2,2) 只走了一步，互不相干。
        assert!(w.record_step(2, 2) == false);
        assert_eq!(w.tracked_cells(), 2);
    }

    #[test]
    fn clear_removes_a_cell_and_is_idempotent() {
        let mut w = PathWear::new();
        w.record_step(9, 9);
        assert_eq!(w.tracked_cells(), 1);
        w.clear(9, 9);
        assert_eq!(w.tracked_cells(), 0);
        w.clear(9, 9); // 再清一次不出錯
        assert_eq!(w.tracked_cells(), 0);
    }

    #[test]
    fn cleared_cell_starts_over_from_zero() {
        let mut w = PathWear::new();
        for _ in 0..PATHWEAR_THRESHOLD {
            w.record_step(3, 3);
        }
        w.clear(3, 3);
        // 清掉後這格重新從 0 起算，一步遠遠不到門檻。
        assert!(!w.record_step(3, 3));
    }

    #[test]
    fn safety_cap_stops_registering_new_cells() {
        let mut w = PathWear::new();
        // 塞滿到上限（每格各一步）。
        for i in 0..MAX_TRACKED_CELLS as i32 {
            w.record_step(i, 0);
        }
        assert_eq!(w.tracked_cells(), MAX_TRACKED_CELLS);
        // 再來一個「新格」被安全閥擋下、不增長。
        assert!(!w.record_step(999_999, 999_999));
        assert_eq!(w.tracked_cells(), MAX_TRACKED_CELLS);
        // 但既有格仍可繼續累積（不被安全閥誤傷）。
        for _ in 0..PATHWEAR_THRESHOLD {
            w.record_step(0, 0);
        }
    }

    #[test]
    fn feed_line_is_non_empty_and_does_not_overclaim_first_ever() {
        let line = first_path_feed_line();
        assert!(!line.is_empty());
        // 不宣稱「史上第一」（重啟後可能再觸發），措辭只描述當下。
        assert!(!line.contains("第一條"));
    }
}
