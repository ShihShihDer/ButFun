//! 乙太方界·眾力共築·乙太燈塔 v1（自主提案切片，接續 PLAN_ETHERVOX「玩家遊玩」節
//! 「進度/目標：療癒循環給人想一直玩下去的理由」，並承「交織點：你採集+合成+蓋造，
//! 居民在同個世界活著+蓋家，你們交易/互助/送禮——人類的樂趣與 AI 的生活交織」）。
//!
//! **真缺口**：世界至今每一座值得一看的建物——村碑（885 全村里程碑實體化）、殖民地
//! 奠基殘核（884）、居民個人長程夢想（917 lifeproject）、居民自己蓋的家——全部是
//! **居民蓋的**，玩家只能旁觀或送禮催化。反過來，玩家彼此之間也從沒有一件「大家一起
//! 出力才蓋得起來」的共同工程：玩家的採集/合成成果，此前只餵得進自己的背包、居民的
//! 心願、或箱子的私人儲藏，從沒有一項**跨玩家累積、任何人都看得到進度、蓋成後屬於
//! 全世界**的公共建設。本刀補上這一塊——一座固定佇立在村外的**乙太燈塔**地基，需要
//! 眾人陸續搬材料來才蓋得起來，蓋成的瞬間是全世界共同的成就。
//!
//! **與既有系統 razor-sharp 區隔**（避免同軸換皮）：
//! - `voxel_monument`（885 村碑）＝**居民**依「集體里程碑」自動疊高，玩家只能看；本刀＝
//!   **玩家**主動搬材料才會長，居民完全不參與。
//! - `voxel_lifeproject`（917）＝**單一居民**的個人跨天夢想；本刀＝**跨玩家**的公共工程。
//! - `voxel_colony`（884 分村殖民）＝居民自己遠征奠基、有名字有故事的**野外聚落**；
//!   本刀＝**主村旁固定一處**的建設，不是聚落、沒有居民遷入。
//! - 940「世界奇觀·乙太世界樹」＝**程序生成、天然、無來歷**的地標；本刀是**玩家蓋出來
//!   的人造建物**——故意不沿用「奇觀」一詞，避免與 940 的天然奇觀混淆。
//! - `voxel_chest_contribute`＝**居民**把採集品放進共用箱子（村莊糧倉）；本刀是**玩家**
//!   把材料獻給一座固定的建設工地，方向與角色都不同。
//!
//! **本刀範圍（v1，刻意有界）**：只做**建材募集 → 蓋成**這條主軸——固定選址（主村中心
//! 東方一段距離）、固定一份建材清單（石頭/木板/玻璃/鐵磚各若干）、任何玩家靠近後捐出
//! 背包裡的對應材料、進度全服可見、募齊即一次性蓋出燈塔並向全世界廣播。**不做**（留給
//! 未來）：多座公共工程輪替、蓋好後的燈塔具備遊戲機制效果（如夜間指路光柱）、可擴建。
//!
//! **純邏輯層鐵律**：本檔零 LLM、零鎖、零 async、零世界 IO——材料清單/進度計算/選址/
//! 燈塔藍圖/台詞全是確定性純函式，吃座標與數字吐座標與數字，方便單元測試釘死。真正的
//! 背包扣除/方塊落地/廣播/持久化都在 `voxel_ws.rs`，嚴守短鎖鐵律（比照村碑 885「golden
//! safe pattern」：`surface_y` 鎖外算 → `deltas` 寫鎖批次只在空氣格落子 → 鎖外廣播＋
//! append-only 落地）。
//!
//! **成本 / 濫用防護**：零 LLM（進度/台詞全走確定性模板）；捐獻是**扣自己背包**送進公共
//! 進度，無法無中生有（伺服器權威驗庫存，玩家自報無效）；同一材料捐超過剩餘需求會被
//! 自動夾住（不會浪費、也不會讓進度衝過 100%）；蓋成只會發生一次（`mark_completed`
//! 冪等），不會重複觸發世界廣播或重複放置方塊。

use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::voxel::Block;

/// 持久化路徑（`data/` 已 gitignore）。append-only：每筆捐獻＋落成事件各一行，
/// `WonderProgress::from_entries` 重播重建進度與貢獻榜（重啟後接續募集，不歸零）。
pub const LIGHTHOUSE_PATH: &str = "data/voxel_lighthouse.jsonl";

/// 燈塔面向玩家的名稱（i18n 集中此處）。
pub const LIGHTHOUSE_NAME: &str = "乙太燈塔";

/// 選址：相對村莊中心的固定偏移（格）——村莊東側一段距離，不擠廣場、玩家散步可及。
pub const SITE_DX: i32 = 70;
pub const SITE_DZ: i32 = 0;

/// 捐獻觸及半徑（格）：比照 `voxel_gift::GIFT_REACH`（5.0）略寬——燈塔是固定大型建物，
/// 不像居民會移動，給一點容錯讓玩家不必精準站在正中心。
pub const CONTRIBUTE_REACH: f32 = 6.0;

/// 一種所需建材：物品/方塊 id（對齊 `voxel_gift::item_name_zh`）＋面向玩家名＋總需求量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LighthouseMaterial {
    pub item_id: u8,
    pub name: &'static str,
    pub needed: u32,
}

/// 固定建材清單（v1 刻意不做難度分岔，四種玩家平常採集/合成就會攢到的基礎建材）。
pub const MATERIALS: [LighthouseMaterial; 4] = [
    LighthouseMaterial { item_id: 3, name: "石頭", needed: 160 },
    LighthouseMaterial { item_id: 8, name: "木板", needed: 80 },
    LighthouseMaterial { item_id: 10, name: "玻璃", needed: 40 },
    LighthouseMaterial { item_id: 23, name: "鐵磚", needed: 20 },
];

/// 是否為燈塔會收的材料。
pub fn is_material(item_id: u8) -> bool {
    MATERIALS.iter().any(|m| m.item_id == item_id)
}

/// 某材料的總需求量（非收單材料回 0）。
pub fn needed(item_id: u8) -> u32 {
    MATERIALS.iter().find(|m| m.item_id == item_id).map(|m| m.needed).unwrap_or(0)
}

/// 一筆歷史事件（append-only 落地格式）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum LighthouseEvent {
    /// 一位玩家獻上一份材料。
    Contribution { player: String, item_id: u8, qty: u32, unix: u64 },
    /// 燈塔正式落成（全歷史至多一筆，重播時只認第一筆，天然冪等）。
    Completed { unix: u64 },
}

/// 募集進度（記憶體重建，唯讀查詢皆為純函式，寫入只在 `voxel_ws.rs` 短鎖內呼叫）。
#[derive(Default)]
pub struct LighthouseProgress {
    given: HashMap<u8, u32>,
    contributors: HashMap<String, u32>,
    completed_unix: Option<u64>,
}

impl LighthouseProgress {
    pub fn new() -> Self {
        Self::default()
    }

    /// 由歷史事件重建（重啟後 replay，累計進度不歸零；`Completed` 只認第一筆）。
    pub fn from_entries(entries: Vec<LighthouseEvent>) -> Self {
        let mut p = Self::new();
        for e in entries {
            match e {
                LighthouseEvent::Contribution { player, item_id, qty, .. } => {
                    *p.given.entry(item_id).or_insert(0) += qty;
                    *p.contributors.entry(player).or_insert(0) += qty;
                }
                LighthouseEvent::Completed { unix } => {
                    if p.completed_unix.is_none() {
                        p.completed_unix = Some(unix);
                    }
                }
            }
        }
        p
    }

    /// 某材料目前已募得的量。
    pub fn given(&self, item_id: u8) -> u32 {
        *self.given.get(&item_id).unwrap_or(&0)
    }

    /// 某材料還缺多少（非收單材料回 0；已募滿回 0）。
    pub fn remaining(&self, item_id: u8) -> u32 {
        needed(item_id).saturating_sub(self.given(item_id))
    }

    /// 套用一筆捐獻（純狀態變更，不含 IO/時間）：回傳實際採用的數量——已依「這項材料
    /// 還缺多少」自動夾住，呼叫端另外要依玩家背包實際持有量夾一次（本函式不知道背包）。
    /// `qty=0`、已完工、或非收單材料時回 0、狀態不變（不寫入 0 筆貢獻，貢獻榜乾淨）。
    pub fn apply_contribution(&mut self, player: &str, item_id: u8, qty: u32) -> u32 {
        if qty == 0 || self.completed() {
            return 0;
        }
        let applied = qty.min(self.remaining(item_id));
        if applied == 0 {
            return 0;
        }
        *self.given.entry(item_id).or_insert(0) += applied;
        *self.contributors.entry(player.to_string()).or_insert(0) += applied;
        applied
    }

    /// 是否所有材料皆已募滿（不代表已落成——落成要另外 `mark_completed`）。
    pub fn all_materials_ready(&self) -> bool {
        MATERIALS.iter().all(|m| self.given(m.item_id) >= m.needed)
    }

    /// 是否已落成。
    pub fn completed(&self) -> bool {
        self.completed_unix.is_some()
    }

    /// 標記落成（冪等：已標記過回 `false`，呼叫端據此判斷「這次呼叫是不是真的觸發了
    /// 落成」，只有頭一次回 `true` 時才該去放方塊＋世界廣播）。
    pub fn mark_completed(&mut self, unix: u64) -> bool {
        if self.completed_unix.is_some() {
            return false;
        }
        self.completed_unix = Some(unix);
        true
    }

    /// 總體完成度百分比（0.0..=100.0，跨全部材料加總、每項材料的溢額不計入分子避免超額洗）。
    pub fn pct_complete(&self) -> f32 {
        let (given_sum, need_sum) = MATERIALS.iter().fold((0u32, 0u32), |(g, n), m| {
            (g + self.given(m.item_id).min(m.needed), n + m.needed)
        });
        if need_sum == 0 {
            return 100.0;
        }
        (given_sum as f32 / need_sum as f32) * 100.0
    }

    /// 貢獻榜前 `n` 名（依累計獻材量遞減、同量依名字排序讓結果確定性可測）。
    pub fn top_contributors(&self, n: usize) -> Vec<(String, u32)> {
        let mut v: Vec<(String, u32)> = self.contributors.iter().map(|(k, q)| (k.clone(), *q)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(n);
        v
    }
}

/// 一筆捐獻的處理結果。呼叫端**只**依 `applied` 去扣玩家背包（絕不是呼叫前算好的 want）——
/// `applied` 才是「這一刻真正吃下多少」的權威值，`applied==0` 時背包完全不用碰。
pub struct ContributionOutcome {
    pub applied: u32,
    pub just_completed: bool,
}

/// 套用一筆捐獻的核心決策：在呼叫端持有的 `lighthouse` 寫鎖窗口內，依「當下」的 remaining
/// 權威地夾住 `want`、必要時一併觸發落成。刻意不含任何鎖獲取或 IO——鎖由呼叫端在外面拿好，
/// 這裡只管邏輯，讓「兩人幾乎同時捐獻、其中一人的 want 其實已經超額」這種交錯情境能在單元
/// 測試裡用真正的 `LighthouseProgress` 狀態驅動，而不必為接線層搭一套假的 mock。
///
/// 修的是這個 bug：舊接線層「先扣背包、再套進度」，兩把鎖之間隔著磁碟寫入與 `.await`，
/// 別的玩家的訊息插得進來，導致已扣掉背包的材料在套用時被夾成 0、憑空蒸發。現在接線層
/// （`voxel_ws.rs`）改成先呼叫這裡拿到權威 `applied`，才依 `applied` 扣背包，中間不再有
/// 可被插隊的縫。
pub fn resolve_contribution(
    lighthouse: &mut LighthouseProgress,
    player: &str,
    item_id: u8,
    want: u32,
    unix: u64,
) -> ContributionOutcome {
    let applied = lighthouse.apply_contribution(player, item_id, want);
    let just_completed =
        applied > 0 && lighthouse.all_materials_ready() && lighthouse.mark_completed(unix);
    ContributionOutcome { applied, just_completed }
}

// ── 選址（純函式）────────────────────────────────────────────────────────────────────
/// 燈塔工地座標：村莊中心固定偏移（見 [`SITE_DX`]/[`SITE_DZ`]）。純函式、確定性。
pub fn site_coords(vcx: i32, vcz: i32) -> (i32, i32) {
    (vcx + SITE_DX, vcz + SITE_DZ)
}

// ── 燈塔藍圖（純函式、確定性）────────────────────────────────────────────────────────
/// 產生燈塔完工瞬間要落下的**全部**方塊清單（絕對世界座標）。
///
/// - `cx, cz`：工地中心。
/// - `surface_y`：該格「地面正上方」的 y（`voxel_building::surface_y` 語意，即第一格空氣）。
///
/// 佈局：5×5 石磚地基 → 中央柱身（石磚／鐵磚交錯裝飾環）→ 玻璃燈籠室（十字窗）→
/// 頂端乙太燈塔燈。呼叫端只在 `cur == Air` 時落子（air-only，絕不覆蓋既有方塊，冪等）。
pub fn lighthouse_cells(cx: i32, cz: i32, surface_y: i32) -> Vec<(i32, i32, i32, Block)> {
    let mut cells = Vec::with_capacity(40);
    // 5×5 地基（-2..=2）。
    for dx in -2..=2 {
        for dz in -2..=2 {
            cells.push((cx + dx, surface_y, cz + dz, Block::StoneBrick));
        }
    }
    // 中央柱身，往上 5 格：石磚為主、鐵磚每隔一段點綴裝飾環。
    let pillar_blocks = [
        Block::StoneBrick,
        Block::StoneBrick,
        Block::IronBlock,
        Block::StoneBrick,
        Block::StoneBrick,
    ];
    for (i, blk) in pillar_blocks.iter().enumerate() {
        cells.push((cx, surface_y + 1 + i as i32, cz, *blk));
    }
    // 燈籠室：十字窗（東西南北四片玻璃）+ 中心支撐柱延續。
    let lantern_y = surface_y + 1 + pillar_blocks.len() as i32;
    cells.push((cx, lantern_y, cz, Block::StoneBrick));
    for (ddx, ddz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        cells.push((cx + ddx, lantern_y, cz + ddz, Block::Glass));
    }
    // 頂端燈塔燈：全世界最亮的一盞。
    cells.push((cx, lantern_y + 1, cz, Block::AetherLamp));
    cells
}

/// 燈塔從地面算起的總高（格，含地基那一層）。
pub fn total_height() -> i32 {
    // 地基(1) + 柱身(5) + 燈籠室(1) + 頂燈(1)。
    1 + 5 + 1 + 1
}

// ── 面向玩家的句子（純函式、i18n 集中於此）────────────────────────────────────────────
/// 單次捐獻成功後回給該玩家的訊息（不廣播，僅本人看到，作為即時反饋）。
pub fn contribute_ack_line(item_name: &str, applied: u32, remaining_after: u32) -> String {
    if remaining_after == 0 {
        format!("獻上了 {applied} 份{item_name}——這項材料已經湊齊了！")
    } else {
        format!("獻上了 {applied} 份{item_name}，這項材料還缺 {remaining_after} 份。")
    }
}

/// 某項材料剛好湊齊時的全服動態牆播報。
pub fn material_ready_feed_line(item_name: &str) -> String {
    format!("🗼 {LIGHTHOUSE_NAME}的{item_name}已經湊齊了，就差其他幾樣材料了！")
}

/// 燈塔正式落成的全服動態牆播報（含前幾名功臣）。
pub fn completion_feed_line(top: &[(String, u32)]) -> String {
    if top.is_empty() {
        return format!("🗼 眾人合力搬運的材料終於湊齊，{LIGHTHOUSE_NAME}正式落成！");
    }
    let names: Vec<String> = top.iter().map(|(n, q)| format!("{n}（{q}）")).collect();
    format!(
        "🗼 眾人合力搬運的材料終於湊齊，{LIGHTHOUSE_NAME}正式落成！功勞最大的旅人：{}。",
        names.join("、")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_material_only_matches_listed_ids() {
        assert!(is_material(3)); // 石頭
        assert!(is_material(8)); // 木板
        assert!(is_material(10)); // 玻璃
        assert!(is_material(23)); // 鐵磚
        assert!(!is_material(5)); // 木頭不收
        assert!(!is_material(0));
    }

    #[test]
    fn needed_matches_material_table_and_zero_for_unknown() {
        assert_eq!(needed(3), 160);
        assert_eq!(needed(23), 20);
        assert_eq!(needed(99), 0);
    }

    #[test]
    fn apply_contribution_accumulates_and_clamps_to_remaining() {
        let mut p = LighthouseProgress::new();
        assert_eq!(p.apply_contribution("小明", 3, 50), 50);
        assert_eq!(p.given(3), 50);
        assert_eq!(p.remaining(3), 110);
        // 一次獻超過剩餘需求，只採用剩餘的部分（夾住、不浪費也不超額）。
        assert_eq!(p.apply_contribution("小明", 3, 9999), 110);
        assert_eq!(p.given(3), 160);
        assert_eq!(p.remaining(3), 0);
        // 已募滿後再獻，回 0、狀態不變。
        assert_eq!(p.apply_contribution("小明", 3, 5), 0);
        assert_eq!(p.given(3), 160);
    }

    #[test]
    fn apply_contribution_rejects_zero_qty_and_non_material() {
        let mut p = LighthouseProgress::new();
        assert_eq!(p.apply_contribution("小明", 3, 0), 0);
        assert_eq!(p.apply_contribution("小明", 5, 10), 0); // 木頭不收
        assert!(p.contributors.is_empty());
    }

    #[test]
    fn apply_contribution_no_op_after_completed() {
        let mut p = LighthouseProgress::new();
        p.mark_completed(1000);
        assert_eq!(p.apply_contribution("小明", 3, 10), 0);
        assert_eq!(p.given(3), 0);
    }

    #[test]
    fn all_materials_ready_requires_every_material_filled() {
        let mut p = LighthouseProgress::new();
        for m in MATERIALS.iter() {
            assert!(!p.all_materials_ready());
            p.apply_contribution("眾人", m.item_id, m.needed);
        }
        assert!(p.all_materials_ready());
    }

    #[test]
    fn mark_completed_is_idempotent() {
        let mut p = LighthouseProgress::new();
        assert!(p.mark_completed(500));
        assert!(p.completed());
        assert!(!p.mark_completed(600), "已完工不該再被標記第二次");
    }

    /// 接線層修過的 race（PR #1248 review）：A、B 幾乎同時捐獻，兩人都是依同一個
    /// 「remaining_before」快照各自算出 want，此測試模擬接線層依序（同一把寫鎖序列化）
    /// 呼叫 apply_contribution 的結果——驗證「權威量」只能來自這個函式的回傳值，
    /// 不能信賴呼叫端自己先算好的 want。若接線層誤把 want 拿去扣背包（而非這裡的
    /// applied），B 的材料就會在「已扣背包、卻沒進度可拿」的窗口裡憑空消失。
    #[test]
    fn interleaved_contributions_second_caller_gets_authoritative_zero_not_stale_want() {
        let mut p = LighthouseProgress::new();
        let item = MATERIALS[0].item_id; // 石頭
        let needed = MATERIALS[0].needed;
        // 先墊到只剩 10 顆的缺口。
        p.apply_contribution("墊底", item, needed - 10);
        assert_eq!(p.remaining(item), 10);

        // A、B 同時讀到 remaining_before = 10，背包各有 10 顆，各自算出的 want 都是 10——
        // 兩人 want 加總（20）已經超過真正剩下的缺口（10），這正是舊接線層會出事的縫。
        let remaining_before = p.remaining(item);
        let want_a = remaining_before.min(10);
        let want_b = remaining_before.min(10);

        // 接線層現在改叫 resolve_contribution（voxel_ws.rs 實際呼叫的那個函式），
        // applied 才是唯一該拿去扣背包的量。
        let outcome_a = resolve_contribution(&mut p, "A", item, want_a, 1000);
        let outcome_b = resolve_contribution(&mut p, "B", item, want_b, 1000);

        assert_eq!(outcome_a.applied, 10);
        assert_eq!(
            outcome_b.applied, 0,
            "材料已被 A 補滿，B 的 want 不該被套用任何進度——接線層依此得知 B 的背包完全不用扣"
        );
        assert_eq!(p.remaining(item), 0, "募得總量不能超過需求（不能被兩人的 want 加總洗超）");
        // 貢獻榜上 B 完全沒有紀錄，因為 applied_b == 0（apply_contribution 對 0 是 no-op）。
        assert!(p.top_contributors(10).iter().all(|(who, _)| who != "B"));
    }

    #[test]
    fn pct_complete_tracks_aggregate_progress() {
        let mut p = LighthouseProgress::new();
        assert_eq!(p.pct_complete(), 0.0);
        for m in MATERIALS.iter() {
            p.apply_contribution("眾人", m.item_id, m.needed);
        }
        assert_eq!(p.pct_complete(), 100.0);
    }

    #[test]
    fn top_contributors_sorted_descending_with_stable_tiebreak() {
        let mut p = LighthouseProgress::new();
        p.apply_contribution("柳", 3, 30);
        p.apply_contribution("阿明", 3, 80);
        p.apply_contribution("小美", 8, 10);
        let top = p.top_contributors(2);
        assert_eq!(top, vec![("阿明".to_string(), 80), ("柳".to_string(), 30)]);
    }

    #[test]
    fn from_entries_replays_contributions_and_first_completed_only() {
        let entries = vec![
            LighthouseEvent::Contribution { player: "柳".into(), item_id: 3, qty: 40, unix: 10 },
            LighthouseEvent::Contribution { player: "柳".into(), item_id: 3, qty: 20, unix: 20 },
            LighthouseEvent::Completed { unix: 100 },
            LighthouseEvent::Completed { unix: 200 }, // 理論上不該再發生，重播仍只認第一筆
        ];
        let p = LighthouseProgress::from_entries(entries);
        assert_eq!(p.given(3), 60);
        assert_eq!(p.contributors.get("柳"), Some(&60));
        assert!(p.completed());
    }

    #[test]
    fn site_coords_offsets_from_village_center() {
        assert_eq!(site_coords(0, 0), (SITE_DX, SITE_DZ));
        assert_eq!(site_coords(100, -50), (100 + SITE_DX, -50 + SITE_DZ));
    }

    #[test]
    fn lighthouse_cells_base_is_five_by_five_stone_brick_at_surface() {
        let cells = lighthouse_cells(0, 0, 20);
        let base: Vec<_> = cells.iter().filter(|c| c.1 == 20).collect();
        assert_eq!(base.len(), 25);
        assert!(base.iter().all(|c| c.3 == Block::StoneBrick));
    }

    #[test]
    fn lighthouse_cells_top_is_aether_lamp_and_no_duplicate_coords() {
        let cells = lighthouse_cells(5, -3, 10);
        let top = cells.iter().max_by_key(|c| c.1).unwrap();
        assert_eq!(top.3, Block::AetherLamp);
        // total_height() 是「疊了幾層」，頂端 y 是 surface_y + (層數 - 1)（地基那層本身不佔額外高度）。
        assert_eq!(top.1, 10 + total_height() - 1);
        let mut seen = std::collections::HashSet::new();
        for c in &cells {
            assert!(seen.insert((c.0, c.1, c.2)), "座標 {:?} 被重複佔用", (c.0, c.1, c.2));
        }
    }

    #[test]
    fn lighthouse_cells_lantern_room_has_four_glass_windows() {
        let cells = lighthouse_cells(0, 0, 0);
        let glass_count = cells.iter().filter(|c| c.3 == Block::Glass).count();
        assert_eq!(glass_count, 4);
    }

    #[test]
    fn contribute_ack_line_mentions_remaining_or_ready() {
        let done = contribute_ack_line("石頭", 10, 0);
        assert!(done.contains("湊齊"));
        let more = contribute_ack_line("石頭", 10, 5);
        assert!(more.contains('5'));
        assert!(!done.contains('\n') && !more.contains('\n'));
    }

    #[test]
    fn completion_feed_line_lists_top_contributors_and_handles_empty() {
        let line = completion_feed_line(&[("柳".to_string(), 120), ("阿明".to_string(), 80)]);
        assert!(line.contains('柳') || line.contains("柳"));
        assert!(line.contains("阿明"));
        assert!(line.contains(LIGHTHOUSE_NAME));
        let empty_line = completion_feed_line(&[]);
        assert!(empty_line.contains(LIGHTHOUSE_NAME));
        assert!(!line.contains('\n') && !empty_line.contains('\n'));
    }
}

// ── 持久化（append-only jsonl，比照 voxel_colony 慣例）───────────────────────────────
/// 載回全部歷史事件（檔缺＝從沒開始募集 → 空清單）。
pub fn load_events() -> Vec<LighthouseEvent> {
    let Ok(f) = fs::File::open(LIGHTHOUSE_PATH) else { return vec![] };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<LighthouseEvent>(&l).ok())
        .collect()
}

/// Append 一筆事件到 jsonl。append-only、絕不覆寫/刪除既有行；失敗只記 log 不 panic。
pub fn append_event(event: &LighthouseEvent) {
    let Ok(line) = serde_json::to_string(event) else { return };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(LIGHTHOUSE_PATH) else {
        tracing::warn!("無法寫入乙太燈塔募集紀錄 {LIGHTHOUSE_PATH}");
        return;
    };
    let _ = writeln!(f, "{line}");
}
