//! 乙太方界·指令→任務 + 整地技能 v1（「居民真的照玩家的話做事」的地基）。
//!
//! **架構分層（同 voxel_skills 的鐵律）**：LLM 腦＝高層「做什麼／為什麼」；
//! 本模組＝低層「怎麼做」——全是零 LLM、零鎖、零 async 的純邏輯：
//! 偵測玩家的整地指令、整地任務資料模型、整地技能（逐格把地表帶到同一高度）。
//! 鎖／廣播／世界寫入／持久化觸發全留在 `voxel_ws.rs`。
//!
//! 這是「指令→可執行任務」的第一刀：玩家對居民說「幫我把這裡整平」，
//! 居民**真的走過去、把一塊地剷平/填平到同一高度**，而不再只是誠實地說做不到——
//! 因為合理大小的整地她現在真的做得到（答應是誠實的，不是空頭支票）。

use crate::voxel::{self, Block, WorldDelta, BASE_HEIGHT};

// ── 整地任務參數（v1 刻意保守：小而完整，別搞太大拖垮 tick）─────────────────────

/// 整地半徑（格）：以中心為原點，向四方各延伸這麼多格 → (2r+1)² 柱。
/// v1 固定 4 → 9×9 = 81 柱，是「一小塊地」的合理大小，居民一個人做得到。
pub const LEVEL_RADIUS: i32 = 4;

/// 每個 tick 處理幾柱（分批整地，別一次全改、免卡頓；比照採集/建造的節奏）。
/// 81 柱 ÷ 5 ≈ 17 個 tick（10Hz ≈ 1.7 秒）整完一塊——看得到在做事、又不炸 tick。
pub const LEVEL_COLUMNS_PER_STEP: usize = 5;

/// 削平時往「目標高度之上」最多掃幾格（把高地/樹幹削掉的上界）。
pub const LEVEL_MAX_UP: i32 = 20;

/// 填平時往「目標高度之下」最多填幾格（把窪地/坑填起的下界）。
pub const LEVEL_MAX_DOWN: i32 = 20;

/// 視為「已抵達工地、可開始整地」的水平距離＝半徑 + 這個餘裕（格）。
/// 站在工地中心附近即可作業（居民在已整平處/邊緣動手，沿用可逃精神不自困）。
pub const LEVEL_ARRIVE_MARGIN: f32 = 2.0;

/// 任務逾時（秒）：走不到工地/整不完就放棄（避免卡死任務永不釋放）。給得寬鬆，
/// 因為玩家通常站在居民附近下令，正常情況遠在此之前就整完了。
pub const LEVEL_DEADLINE_SECS: f32 = 180.0;

// ── 玩家指令偵測（純函式、確定性、可測、零 LLM）──────────────────────────────

/// 「整地意圖」關鍵詞：玩家這句話像在叫居民把一塊地弄平就命中。
/// 刻意收斂——一般閒聊不含這些詞，不會誤觸發。
const LEVEL_TOKENS: &[&str] = &[
    "整平", "整地", "推平", "剷平", "鏟平", "夷平", "弄平", "挖平",
    "填平", "壓平", "弄成平地", "推成平地", "清出一塊地", "清一塊地",
    "清出塊地", "整出一塊平地", "鏟一塊地", "剷一塊地",
];

/// 「大範圍」暗示詞：出現這些＝玩家想要的整地超出居民一個人的能力（該誠實婉拒）。
/// 與 `voxel_ws::detect_over_scope` 的 SCALE_HINTS 同一組語意（此處另存一份，
/// 讓本模組保持純粹自足、可獨立測試）。
const OVERSIZE_HINTS: &[&str] = &[
    "100", "百格", "大片", "大範圍", "整片", "整塊", "一大片",
    "一整片", "全部的地", "所有的地", "這一帶", "附近全", "整座", "整個世界",
];

/// 偵測：這句玩家的話是否在叫居民「整平一塊地」。命中任一整地意圖詞即算。
/// 純函式、確定性、可測——不誤觸發一般聊天。
pub fn detect_level_command(text: &str) -> bool {
    LEVEL_TOKENS.iter().any(|t| text.contains(t))
}

/// 偵測：這句整地請求是否「太大」（超出居民一個人能力，該走誠實婉拒而非答應）。
/// 命中任一大範圍暗示詞即算。純函式、可測。
pub fn is_oversized_level(text: &str) -> bool {
    OVERSIZE_HINTS.iter().any(|t| text.contains(t))
}

/// 居民「答應整地」的回覆（誠實而願意——她現在真的做得到合理大小）。
/// 依 `pick` 選句增加變化；口吻溫暖、坦白會花點時間。純函式、可測、零 LLM。
pub fn accept_line(name: &str, pick: usize) -> String {
    const POOL: [&str; 4] = [
        "好，我這就過去把那塊地整平，會花點時間喔～",
        "交給我吧！我去把那塊地弄平，稍等我一下下～",
        "沒問題，我這就動身去整那塊地，整完再跟你說！",
        "好呀，我走過去把它剷平、填平到一樣高，做起來囉～",
    ];
    let _ = name; // 名字保留給未來想帶入口吻用；目前選句不依名字。
    POOL[pick % POOL.len()].to_string()
}

// ── 整地任務資料模型（純資料 + 純方法；hub 只存它、tick 推進它）─────────────────

/// 一個指向某居民的整地任務。中心 (cx,cz)、半徑、目標高度 target_y（該柱最高實心方塊 y），
/// cursor＝下一個要處理的柱索引（0..總柱數），deadline＝剩餘逾時秒數。
/// v1 純記憶體（重啟後任務消失可接受）；**地形改動本身走既有 world delta 持久化**。
#[derive(Clone, Debug, PartialEq)]
pub struct DirectedTask {
    /// 被指派的居民系統 id（"vox_res_0"…）。
    pub assignee: String,
    /// 下令的玩家身份鍵（供 Feed / 記憶記錄「是誰請她整的」）。
    pub requester: String,
    /// 整地中心世界座標（水平）。
    pub cx: i32,
    pub cz: i32,
    /// 半徑（格）：範圍是以中心為原點、向四方各延伸 radius 的正方形。
    pub radius: i32,
    /// 目標地表高度：整完後每柱最高實心方塊都落在這個 y。
    pub target_y: i32,
    /// 下一個要處理的柱索引（0..total_columns）。整完＝cursor 到達總柱數。
    pub cursor: usize,
    /// 剩餘逾時（秒）：每 tick 遞減，歸零仍未整完就放棄任務。
    pub deadline: f32,
}

impl DirectedTask {
    /// 建一個全新任務（cursor 從 0、deadline 滿格）。
    pub fn new(assignee: String, requester: String, cx: i32, cz: i32, radius: i32, target_y: i32) -> Self {
        Self {
            assignee,
            requester,
            cx,
            cz,
            radius,
            target_y,
            cursor: 0,
            deadline: LEVEL_DEADLINE_SECS,
        }
    }

    /// 範圍邊長（柱）：2r+1。
    fn side(&self) -> usize {
        (self.radius * 2 + 1).max(1) as usize
    }

    /// 總柱數＝邊長²。
    pub fn total_columns(&self) -> usize {
        let s = self.side();
        s * s
    }

    /// 任務是否已整完（cursor 掃過全部柱）。
    pub fn is_complete(&self) -> bool {
        self.cursor >= self.total_columns()
    }

    /// 進度百分比（0..100）。
    pub fn progress_pct(&self) -> u8 {
        let total = self.total_columns().max(1);
        ((self.cursor.min(total) * 100) / total) as u8
    }

    /// 第 idx 個柱的世界座標 (x,z)（列優先展開；idx 應 < total_columns）。
    pub fn column_at(&self, idx: usize) -> (i32, i32) {
        let s = self.side();
        let dx = (idx / s) as i32;
        let dz = (idx % s) as i32;
        (self.cx - self.radius + dx, self.cz - self.radius + dz)
    }
}

// ── 整地技能核心（確定性、零 LLM、可測）──────────────────────────────────────────

/// 找某 (x,z) 柱的「最高實心方塊」y（套 delta overlay；全空回 None）。用來定 target_y。
/// 由高往低掃（涵蓋正常地形峰值 + 建物餘裕）。純函式、可測。
pub fn ground_top(world: &WorldDelta, x: i32, z: i32) -> Option<i32> {
    let top = BASE_HEIGHT + LEVEL_MAX_UP; // 涵蓋地形峰值 + 上方餘裕
    (0..=top)
        .rev()
        .find(|&y| voxel::effective_block_at(world, x, y, z).is_solid())
}

/// **整地技能·單柱**：把 (x,z) 柱的地表帶到 target_y，回傳「要改的方塊」清單（不套用）。
///
/// 規則（確定性）：
/// - 高於 target_y 的實心方塊 → 挖掉（設 Air）：削平高地、砍掉擋路的樹幹/樹冠。
/// - 低於 target_y 的空缺（非實心：空氣/水）→ 用土填：填平窪地/坑。
///   從 target_y 往下填，遇到既有實心地基就停（不無限往下挖填）。
///   最頂那格（target_y）用草皮（Grass）收面，其下用泥土（Dirt）。
/// - 已在 target_y 且其上為空 → 無改動（回空清單）。
///
/// 掃描以 [`LEVEL_MAX_UP`] / [`LEVEL_MAX_DOWN`] 為上下界，成本有界。純函式、可測。
pub fn level_column(world: &WorldDelta, x: i32, z: i32, target_y: i32) -> Vec<(i32, i32, i32, Block)> {
    let mut out = Vec::new();

    // ① 削平：target_y 之上的實心方塊全挖成空氣。
    for y in (target_y + 1)..=(target_y + LEVEL_MAX_UP) {
        if voxel::effective_block_at(world, x, y, z).is_solid() {
            out.push((x, y, z, Block::Air));
        }
    }

    // ② 填平：從 target_y 往下，遇到非實心（空氣/水）就填土；碰到既有實心地基就停。
    let bottom = (target_y - LEVEL_MAX_DOWN).max(0);
    for y in (bottom..=target_y).rev() {
        if voxel::effective_block_at(world, x, y, z).is_solid() {
            break; // 到達地基，下面不用再填
        }
        let fill = if y == target_y { Block::Grass } else { Block::Dirt };
        out.push((x, y, z, fill));
    }

    out
}

/// **整地技能·一批**：從 task.cursor 起處理至多 [`LEVEL_COLUMNS_PER_STEP`] 柱，
/// 回傳（要改的方塊清單, 下一個 cursor）。呼叫端套用方塊、寫回 cursor（見 voxel_ws）。
/// 不碰鎖/IO——世界寫入與持久化在呼叫端。純函式、可測。
pub fn level_step(world: &WorldDelta, task: &DirectedTask) -> (Vec<(i32, i32, i32, Block)>, usize) {
    let total = task.total_columns();
    let mut changes = Vec::new();
    let mut cursor = task.cursor;
    let mut processed = 0usize;
    while cursor < total && processed < LEVEL_COLUMNS_PER_STEP {
        let (x, z) = task.column_at(cursor);
        changes.extend(level_column(world, x, z, task.target_y));
        cursor += 1;
        processed += 1;
    }
    (changes, cursor)
}

// ── 安全：整地時別把居民自己埋了 ─────────────────────────────────────────────────

/// 居民 AABB 半寬（與 voxel_residents::RES_HALF_W 一致；此處另存一份保持模組自足）。
const BODY_HALF_W: f32 = 0.3;
/// 居民身高（與 voxel_residents::RES_HEIGHT 一致）。
const BODY_HEIGHT: f32 = 1.7;

/// 判斷世界格 (bx,by,bz) 是否落在「腳底在 (px,py,pz) 的居民身體」佔用的方塊格內。
/// 用來在套用填塊時過濾掉「會把居民埋起來」的實心方塊（沿用可逃精神）。純函式、可測。
pub fn cell_in_body(bx: i32, by: i32, bz: i32, px: f32, py: f32, pz: f32) -> bool {
    let x0 = (px - BODY_HALF_W).floor() as i32;
    let x1 = (px + BODY_HALF_W).floor() as i32;
    let y0 = py.floor() as i32;
    let y1 = (py + BODY_HEIGHT - 0.01).floor() as i32;
    let z0 = (pz - BODY_HALF_W).floor() as i32;
    let z1 = (pz + BODY_HALF_W).floor() as i32;
    (x0..=x1).contains(&bx) && (y0..=y1).contains(&by) && (z0..=z1).contains(&bz)
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::height_at;

    // ── detect_level_command：該中 / 不誤觸發 ────────────────────────────────────

    #[test]
    fn detect_level_command_catches_leveling_intent() {
        assert!(detect_level_command("幫我把這裡整平"));
        assert!(detect_level_command("露娜，幫我把這裡整平"));
        assert!(detect_level_command("把這塊地推平"));
        assert!(detect_level_command("可以幫我剷平這塊地嗎"));
        assert!(detect_level_command("這裡夷平一下"));
        assert!(detect_level_command("幫我整地"));
        assert!(detect_level_command("清出一塊地給我"));
        assert!(detect_level_command("把這弄平"));
        assert!(detect_level_command("填平這個坑"));
    }

    #[test]
    fn detect_level_command_ignores_chitchat() {
        assert!(!detect_level_command("你好呀，今天天氣真好"));
        assert!(!detect_level_command("你在做什麼呀"));
        assert!(!detect_level_command("這片天地好漂亮"));
        assert!(!detect_level_command("玻璃怎麼合成"));
        assert!(!detect_level_command("你叫什麼名字"));
        assert!(!detect_level_command(""));
    }

    #[test]
    fn is_oversized_level_flags_big_requests() {
        // 大範圍 → 太大（該婉拒）。
        assert!(is_oversized_level("幫我把這附近100×100的地整平"));
        assert!(is_oversized_level("把這一大片整平"));
        assert!(is_oversized_level("夷平這整片土地"));
        assert!(is_oversized_level("把百格的地推平"));
        // 小範圍「這裡/這塊」→ 不算太大（居民做得到）。
        assert!(!is_oversized_level("幫我把這裡整平"));
        assert!(!is_oversized_level("把這塊地推平"));
        assert!(!is_oversized_level("整地"));
    }

    #[test]
    fn accept_line_is_warm_and_varied() {
        let a = accept_line("露娜", 0);
        let b = accept_line("露娜", 1);
        assert!(!a.is_empty());
        assert_ne!(a, b, "不同 pick 應可選到不同句");
    }

    // ── DirectedTask 模型 ────────────────────────────────────────────────────────

    #[test]
    fn directed_task_geometry_and_progress() {
        let t = DirectedTask::new("vox_res_0".into(), "濕濕的".into(), 10, 20, 4, 8);
        // 9×9 = 81 柱。
        assert_eq!(t.total_columns(), 81);
        assert!(!t.is_complete());
        assert_eq!(t.progress_pct(), 0);
        // 第 0 柱＝左下角 (cx-r, cz-r)。
        assert_eq!(t.column_at(0), (6, 16));
        // 最後一柱＝右上角 (cx+r, cz+r)。
        assert_eq!(t.column_at(80), (14, 24));
        // 每柱座標都在範圍內、且互不重複。
        let mut seen = std::collections::HashSet::new();
        for i in 0..t.total_columns() {
            let (x, z) = t.column_at(i);
            assert!((6..=14).contains(&x) && (16..=24).contains(&z));
            assert!(seen.insert((x, z)), "柱座標不應重複");
        }
        assert_eq!(seen.len(), 81);
    }

    #[test]
    fn directed_task_completes_when_cursor_reaches_end() {
        let mut t = DirectedTask::new("r".into(), "p".into(), 0, 0, 1, 5); // 3×3=9
        assert_eq!(t.total_columns(), 9);
        t.cursor = 9;
        assert!(t.is_complete());
        assert_eq!(t.progress_pct(), 100);
    }

    // ── level_column：削高 / 填低 / 已平不動 ─────────────────────────────────────

    /// 造一個「乾淨的單柱」：把 (x,z) 從 y=0..=top 全設實心 Stone（模擬一根實心柱到 top）。
    fn make_solid_column(world: &mut WorldDelta, x: i32, z: i32, top: i32) {
        // 清掉 top 之上一段（保守），再把 0..=top 設實心。
        for y in (top + 1)..(top + LEVEL_MAX_UP + 2) {
            voxel::set_block(world, x, y, z, Block::Air);
        }
        for y in 0..=top {
            voxel::set_block(world, x, y, z, Block::Stone);
        }
    }

    #[test]
    fn level_column_shaves_high_ground() {
        let mut world = WorldDelta::new();
        // 一根高柱（頂在 20），target 8 → 應把 9..=20 挖成空氣。
        make_solid_column(&mut world, 100, 100, 20);
        let changes = level_column(&world, 100, 100, 8);
        assert!(!changes.is_empty());
        // 全部改動都是「挖成空氣」且在 target 之上。
        for (_, y, _, b) in &changes {
            assert_eq!(*b, Block::Air);
            assert!(*y > 8);
        }
        // 套用後最高實心＝target_y。
        let mut w2 = world.clone();
        for (x, y, z, b) in changes {
            voxel::set_block(&mut w2, x, y, z, b);
        }
        assert_eq!(ground_top(&w2, 100, 100), Some(8));
    }

    #[test]
    fn level_column_fills_low_pit() {
        let mut world = WorldDelta::new();
        // 一根矮柱（頂在 3），target 8 → 應把 4..=8 填土（頂草、下泥）。
        make_solid_column(&mut world, 200, 200, 3);
        let changes = level_column(&world, 200, 200, 8);
        assert!(!changes.is_empty());
        for (_, y, _, b) in &changes {
            assert!(*y >= 4 && *y <= 8);
            assert!(b.is_solid());
        }
        let mut w2 = world.clone();
        for (x, y, z, b) in changes {
            voxel::set_block(&mut w2, x, y, z, b);
        }
        assert_eq!(ground_top(&w2, 200, 200), Some(8));
        // 頂面是草皮。
        assert_eq!(voxel::effective_block_at(&w2, 200, 8, 200), Block::Grass);
    }

    #[test]
    fn level_column_flat_is_noop() {
        let mut world = WorldDelta::new();
        make_solid_column(&mut world, 300, 300, 8);
        let changes = level_column(&world, 300, 300, 8);
        assert!(changes.is_empty(), "已在目標高度且上方為空 → 不需改動");
    }

    // ── level_step + 迴圈：凹凸地形 → 全平（核心「她真的整平了」證據）──────────────

    #[test]
    fn level_step_flattens_bumpy_region_to_target() {
        let mut world = WorldDelta::new();
        // 造一片 radius=3（7×7）凹凸地：每柱高度依座標波動（3..=15）。
        let (cx, cz, r): (i32, i32, i32) = (500, 500, 3);
        for dx in -r..=r {
            for dz in -r..=r {
                let x = cx + dx;
                let z = cz + dz;
                // 用簡單確定性公式造高低起伏。
                let h = 3 + ((dx.abs() * 2 + dz.abs() * 3) % 12);
                make_solid_column(&mut world, x, z, h);
            }
        }
        let target_y = 8;
        let mut task = DirectedTask::new("vox_res_0".into(), "濕濕的".into(), cx, cz, r, target_y);

        // 反覆 level_step、套用改動，直到任務完成——鏡像 production 的分批整地。
        let mut guard = 0;
        while !task.is_complete() {
            let (changes, next) = level_step(&world, &task);
            for (x, y, z, b) in changes {
                voxel::set_block(&mut world, x, y, z, b);
            }
            task.cursor = next;
            guard += 1;
            assert!(guard < 1000, "整地應在有限步內完成（cursor 每步前進）");
        }

        // 驗證：範圍內每一柱地表頂都恰好在 target_y，且其上為空氣（真的變平了）。
        for dx in -r..=r {
            for dz in -r..=r {
                let x = cx + dx;
                let z = cz + dz;
                assert_eq!(
                    ground_top(&world, x, z),
                    Some(target_y),
                    "柱 ({x},{z}) 應被整平到 {target_y}"
                );
                assert_eq!(
                    voxel::effective_block_at(&world, x, target_y + 1, z),
                    Block::Air,
                    "柱 ({x},{z}) 目標高度之上應為空氣"
                );
            }
        }
    }

    #[test]
    fn level_step_advances_cursor_in_bounded_batches() {
        let world = WorldDelta::new();
        let task = DirectedTask::new("r".into(), "p".into(), 0, 0, 4, 8);
        let (_changes, next) = level_step(&world, &task);
        // 一步至多前進 LEVEL_COLUMNS_PER_STEP 柱。
        assert_eq!(next, LEVEL_COLUMNS_PER_STEP.min(task.total_columns()));
    }

    // ── ground_top：吃 delta ────────────────────────────────────────────────────

    #[test]
    fn ground_top_reads_delta_overlay() {
        let mut world = WorldDelta::new();
        // 找一個陸地點，疊一塊 delta 石頭抬高地表頂。
        let (x, z) = (0, 0);
        let base = height_at(x, z);
        voxel::set_block(&mut world, x, base + 3, z, Block::Stone);
        assert_eq!(ground_top(&world, x, z), Some(base + 3));
    }

    // ── cell_in_body：安全過濾（別把居民埋了）────────────────────────────────────

    #[test]
    fn cell_in_body_detects_occupied_cells() {
        // 居民腳底在 (10.5, 8.0, 10.5)，身高 1.7 → 佔 y=8,9 兩層、x/z=10 一格。
        assert!(cell_in_body(10, 8, 10, 10.5, 8.0, 10.5));
        assert!(cell_in_body(10, 9, 10, 10.5, 8.0, 10.5));
        // 腳下那格（y=7）不在身體內。
        assert!(!cell_in_body(10, 7, 10, 10.5, 8.0, 10.5));
        // 頭頂上方（y=10）不在身體內。
        assert!(!cell_in_body(10, 10, 10, 10.5, 8.0, 10.5));
        // 隔壁柱不在身體內。
        assert!(!cell_in_body(11, 8, 10, 10.5, 8.0, 10.5));
    }
}
