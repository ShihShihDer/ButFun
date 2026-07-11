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

/// 一條已成型的小徑，要連續「多少次取樣都沒有任何居民再踏上」才會被草重新長回來（癒合）。
/// 刻意設得比磨成路的門檻大得多：小徑不會因居民偶爾改道就立刻消失，得是**真的長期無人問津**
/// 才慢慢褪回草地。以 `voxel_ws.rs` 的取樣頻率（10Hz 下每 6 tick ≈ 1.7Hz）估算，約當
/// 「村子仍有活動、但這條路整整十來分鐘沒人走」才癒合。閒置計數只在「村裡有人在走動」的
/// 取樣才推進（全村皆睡的深夜不推進），所以是按「活躍時段的荒廢」而非牆上時鐘計。
pub const HEAL_IDLE_SAMPLES: u16 = 1024;

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

/// 一條無人再走的小徑癒合後換回的方塊——重新長回天然草皮（自然、早已渲染、零新美術）。
#[inline]
pub fn healed_block() -> Block {
    Block::Grass
}

/// 一格小徑此刻是否還是「我們當初鋪下的那塊泥土」——只有仍是泥土才可癒合回草。
/// 若玩家/居民已在這格放了別的方塊（或它已被改成農田、蓋了建材…），就不是我們的路了，
/// 不該擅自改動——絕不覆蓋玩家對這格做過的事。
#[inline]
pub fn heals_back_to_grass(surface: Block) -> bool {
    matches!(surface, Block::Dirt)
}

/// 全村草皮磨損計數器（純記憶體、重啟歸零——已成型的小徑本身已是持久化的泥土方塊，
/// 重啟後仍在；重啟時歸零的只是「還沒踏成小徑的半途計數」，沿用 `last_seen` 慣例）。
#[derive(Default)]
pub struct PathWear {
    /// (格x, 格z) → 累計被踏進次數。達門檻轉成小徑後即移除該鍵（之後那格已是泥、不再計數）。
    steps: HashMap<(i32, i32), u16>,
    /// 已成型的小徑格 (格x, 格z) → 它的「連續無人踏上」閒置計數與其方塊 y。達 [`HEAL_IDLE_SAMPLES`]
    /// 即待癒合（草長回來）並移除該鍵。有人再踏上會被 [`refresh_worn`](Self::refresh_worn) 歸零，
    /// 常走的動線因此永遠不會癒合——只有真的長期荒廢的路才褪回草地。
    worn: HashMap<(i32, i32), WornCell>,
}

/// 一格已成型小徑的癒合追蹤：閒置了幾次取樣、以及它的方塊 y（癒合時要放回同一格）。
#[derive(Clone, Copy)]
struct WornCell {
    idle: u16,
    y: i32,
}

impl PathWear {
    pub fn new() -> Self {
        Self { steps: HashMap::new(), worn: HashMap::new() }
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

    /// 登記一格「剛磨成的小徑」進入癒合追蹤（閒置從 0 起算，記下它的方塊 y）。
    /// 呼叫端應在**確實把該格草皮換成泥土之後**才登記。安全閥：追蹤中的小徑已達
    /// [`MAX_TRACKED_CELLS`] 且該格是新格時忽略（不追蹤其癒合、亦不無界增長）——
    /// 未被追蹤的小徑就永久留著，不影響玩家。
    pub fn note_worn(&mut self, cx: i32, cz: i32, y: i32) {
        let key = (cx, cz);
        if !self.worn.contains_key(&key) && self.worn.len() >= MAX_TRACKED_CELLS {
            return;
        }
        self.worn.insert(key, WornCell { idle: 0, y });
    }

    /// 有居民再度踏上這一格：若它是條追蹤中的小徑，把閒置計數歸零（這條路仍在用、不該癒合）。
    /// 不是小徑格則無事發生。
    pub fn refresh_worn(&mut self, cx: i32, cz: i32) {
        if let Some(w) = self.worn.get_mut(&(cx, cz)) {
            w.idle = 0;
        }
    }

    /// 推進一次「所有追蹤中小徑」的閒置計數（每次取樣呼叫一次）：各格閒置 +1，
    /// 達 [`HEAL_IDLE_SAMPLES`] 的取出待癒合（回傳其 `(格x, 格z, 方塊y)`）並停止追蹤。
    /// 回傳的格子由呼叫端再確認「此刻仍是我們鋪的泥土」（見 [`heals_back_to_grass`]）才放草。
    pub fn advance_idle(&mut self) -> Vec<(i32, i32, i32)> {
        let mut healed = Vec::new();
        self.worn.retain(|&(cx, cz), w| {
            w.idle = w.idle.saturating_add(1);
            if w.idle >= HEAL_IDLE_SAMPLES {
                healed.push((cx, cz, w.y));
                false // 取出後停止追蹤
            } else {
                true
            }
        });
        healed
    }

    /// 目前正在追蹤癒合的小徑格數（供測試與觀測，非熱路徑）。
    pub fn worn_cells(&self) -> usize {
        self.worn.len()
    }
}

/// 全村第一次踏出小徑時，上一則溫柔的城鎮動態（每個行程至多一次，見 `voxel_ws.rs` 的
/// 一次性旗標）。措辭刻意不宣稱「史上第一條」（重啟後計數歸零可能再觸發），只描述當下所見。
pub fn first_path_feed_line() -> &'static str {
    "居民日復一日往返，草地上被悄悄踏出了一條小徑。"
}

/// 全村第一次有小徑因長期無人問津而癒合（草長回來）時，上一則溫柔的城鎮動態
/// （每個行程至多一次，見 `voxel_ws.rs` 的一次性旗標）。措辭只描述當下所見、不誇大。
pub fn first_heal_feed_line() -> &'static str {
    "一條久無人問津的小徑，被草悄悄長了回來。"
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

    // ── 小徑癒合（草長回來）v1 的測試 ─────────────────────────────────────────

    #[test]
    fn healed_block_is_grass_and_only_dirt_heals_back() {
        assert_eq!(healed_block(), Block::Grass);
        // 只有仍是我們鋪的泥土才可癒合回草；玩家改動過（放了別的方塊）就不動。
        assert!(heals_back_to_grass(Block::Dirt));
        assert!(!heals_back_to_grass(Block::Grass));
        assert!(!heals_back_to_grass(Block::Plank));
        assert!(!heals_back_to_grass(Block::FarmSoil));
        assert!(!heals_back_to_grass(Block::Sand));
    }

    #[test]
    fn worn_cell_heals_after_being_idle_long_enough() {
        let mut w = PathWear::new();
        w.note_worn(5, 7, 63);
        assert_eq!(w.worn_cells(), 1);
        // 前 HEAL_IDLE_SAMPLES-1 次取樣都還不該癒合。
        for _ in 0..(HEAL_IDLE_SAMPLES - 1) {
            assert!(w.advance_idle().is_empty());
        }
        // 第 HEAL_IDLE_SAMPLES 次取樣剛好達標、取出待癒合、並回報正確的 (x,z,y)。
        let healed = w.advance_idle();
        assert_eq!(healed, vec![(5, 7, 63)]);
        // 癒合後就不再追蹤這格了。
        assert_eq!(w.worn_cells(), 0);
    }

    #[test]
    fn stepping_on_a_worn_cell_resets_its_idle_and_keeps_it() {
        let mut w = PathWear::new();
        w.note_worn(1, 2, 60);
        // 閒置累積到快癒合了……
        for _ in 0..(HEAL_IDLE_SAMPLES - 1) {
            w.advance_idle();
        }
        // 有人又踏上這條路 → 閒置歸零，這條路留住。
        w.refresh_worn(1, 2);
        // 再一次取樣遠遠不到門檻（從 0 重新起算）。
        assert!(w.advance_idle().is_empty());
        assert_eq!(w.worn_cells(), 1);
    }

    #[test]
    fn refresh_on_a_non_worn_cell_is_noop() {
        let mut w = PathWear::new();
        // 對一格根本沒在追蹤的座標呼叫 refresh 不 panic、不憑空新增追蹤。
        w.refresh_worn(42, 42);
        assert_eq!(w.worn_cells(), 0);
    }

    #[test]
    fn worn_cells_accumulate_idle_independently() {
        let mut w = PathWear::new();
        w.note_worn(0, 0, 60);
        w.note_worn(10, 10, 61);
        // 只讓其中一格保持有人走（不斷 refresh），另一格放著荒廢。
        for _ in 0..HEAL_IDLE_SAMPLES {
            w.refresh_worn(0, 0);
            let healed = w.advance_idle();
            if !healed.is_empty() {
                // 先癒合的一定是荒廢那格 (10,10)，被走的 (0,0) 還在。
                assert_eq!(healed, vec![(10, 10, 61)]);
            }
        }
        assert_eq!(w.worn_cells(), 1); // (0,0) 仍在（一直有人走）
    }

    #[test]
    fn note_worn_updates_existing_cell_without_growing_count() {
        let mut w = PathWear::new();
        w.note_worn(3, 3, 60);
        for _ in 0..500 {
            w.advance_idle();
        }
        // 同一格重新磨成路（例如重新有人走成路）→ 覆寫、閒置歸零、不重複佔用。
        w.note_worn(3, 3, 60);
        assert_eq!(w.worn_cells(), 1);
        assert!(w.advance_idle().is_empty()); // 從 0 重新起算
    }

    #[test]
    fn worn_tracking_safety_cap_holds() {
        let mut w = PathWear::new();
        for i in 0..MAX_TRACKED_CELLS as i32 {
            w.note_worn(i, 0, 60);
        }
        assert_eq!(w.worn_cells(), MAX_TRACKED_CELLS);
        // 追蹤滿了：新的一格小徑不再納入癒合追蹤（永久留著，不無界增長）。
        w.note_worn(999_999, 999_999, 60);
        assert_eq!(w.worn_cells(), MAX_TRACKED_CELLS);
    }

    #[test]
    fn heal_feed_line_is_non_empty() {
        assert!(!first_heal_feed_line().is_empty());
    }
}
