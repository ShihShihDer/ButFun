//! 乙太方界·垂釣 v1（釣魚）——對準水面拋竿，靜候片刻，收竿釣起水下的珍寶。
//!
//! **核心信念（玩家也要玩得爽）**：乙太方界至今的採集全是「挖方塊」——採礦、砍樹、
//! 收割農田，全是同一種「對準→敲掉」的手感。垂釣帶來**第一種截然不同的節奏**：
//! 不是敲，而是**等**。拋竿入水、浮標靜靜漂著，過幾秒魚兒上鉤，那一下收竿的期待與
//! 揭曉，是採礦給不了的療癒。水體（河、湖、海）遍佈世界卻至今只是背景——垂釣讓它們
//! 第一次成為「有東西可撈」的資源節點。而居民的日記早就悄悄嚮往著釣魚（見
//! `voxel_diary` 的 `Theme::Fishing`「想去釣魚」「水面下藏著什麼樣的安靜」）——
//! 這一刀讓玩家能真的替她們把那份嚮往活出來。
//!
//! **兩步驟真釣魚（伺服器權威、防作弊）**：①`FishCast`（拋竿）——驗手持釣竿、對準水面、
//! 觸及範圍內，記下一個 `ready_at`（3~7 秒後上鉤，隨機有變化＝真有「等」的味道）；
//! ②`FishReel`（收竿）——太早收竿撲空（浮標還沒沉，再等等）、時機到才釣起漁獲。
//! 節奏由伺服器計時把關，前端只呈現。
//!
//! **漁獲**：多數是「小魚」（食物贈禮，居民愛吃）；偶爾（約 1/5）釣起稀有的「乙太魚」——
//! 通體泛著青藍幽光的乙太方界原生魚，是可炫耀、可珍藏、可餽贈的珍寶。
//!
//! **純邏輯層**：本模組只有確定性純函式（漁獲抽選、上鉤秒數、水體判定、台詞），
//! 零 LLM、零鎖、零 async、零 IO、可單元測試。連線 / 鎖 / 背包寫入 / 廣播 / 持久化
//! 觸發全留在 `voxel_ws.rs`（沿用採集/贈禮的短鎖循序慣例，守 prod 死鎖鐵律）。
//!
//! ## 雨天垂釣 v1（自主提案切片，ROADMAP 841）
//! 天氣系統（700/701/780）至今只碰過農地灌溉／居民對話／彩虹視覺，垂釣完全沒接過
//! 天氣——不管晴雨，浮標永遠等一樣久、稀有魚永遠一樣難釣。這一刀補上：**下雨天魚兒
//! 更活躍**——上鉤等得更快、釣起乙太魚的機率也更高，讓「今天在下雨」第一次也成為
//! 垂釣玩家會留意、會特地選在雨天出門釣魚的理由。**只獎不罰**（守療癒優先鐵律）：
//! 晴天照舊是原本的 3~7 秒／1/5 機率，雨天只有加成、沒有懲罰。

/// 釣竿物品 ID（32~41 已被鎬/斧/鏟佔用；60 是首個空號）。純物品，住背包不可放置。
pub const FISHING_ROD_ID: u8 = 60;
/// 小魚物品 ID——最常見的漁獲，食物贈禮（居民愛吃）。
pub const FISH_ID: u8 = 61;
/// 乙太魚物品 ID——稀有漁獲，通體青藍幽光的乙太方界原生魚，可珍藏/餽贈。
pub const AETHER_FISH_ID: u8 = 62;
/// 烤魚物品 ID——把生的小魚放進熔爐烤出的噴香佳餚，居民最愛的美味贈禮。
/// 純物品（住背包不可放置），由熔爐配方 `smelt_fish`（生魚→烤魚）產出。
pub const COOKED_FISH_ID: u8 = 63;

/// 上鉤等待秒數的下界（收竿最快也要等這麼久）。
pub const BITE_MIN_SECS: u64 = 3;
/// 上鉤等待秒數的上界（最久等到這麼久，隨機落在 [MIN, MAX] 之間＝每次都不一樣）。
pub const BITE_MAX_SECS: u64 = 7;

/// 雨天上鉤等待秒數的下界——魚更活躍，最快只要這麼久就上鉤。
pub const RAIN_BITE_MIN_SECS: u64 = 2;
/// 雨天上鉤等待秒數的上界——整體比晴天緊湊，等待感依然存在但更快。
pub const RAIN_BITE_MAX_SECS: u64 = 4;

/// 雨天稀有魚判定的模數：`roll % RAIN_RARE_CATCH_MOD == 0` → 乙太魚。
/// 平時是 1/5（`% 5`），雨天收窄成 1/3，稀有魚明顯更容易釣到、但仍非必得。
pub const RAIN_RARE_CATCH_MOD: u64 = 3;

/// 判斷某方塊 ID 是否為「水體」（可下竿）。
///
/// 來源水 `Water=7`（無限、level 0）與流動水 `WaterFlow1..7 = 24..=30`（離源遞減）都算——
/// 玩家對著任一片水面都能拋竿，河湖海皆可垂釣。
pub fn is_water_block(id: u8) -> bool {
    id == 7 || (24..=30).contains(&id)
}

/// 依 `roll` 抽選漁獲：約 1/5 機率釣起稀有乙太魚，其餘為小魚。
///
/// 確定性純函式（同一 `roll` 恆得同一結果）——`roll` 由伺服器用「時間 + 玩家 + 座標」
/// 合成，讓每次收竿的結果自然分散又可測。
pub fn pick_catch(roll: u64) -> u8 {
    pick_catch_for(roll, false)
}

/// 依 `roll` 抽選漁獲，`raining` 決定稀有機率門檻（雨天 1/3、晴天 1/5，只獎不罰）。
///
/// 確定性純函式（同一 `roll`＋`raining` 恆得同一結果）。
pub fn pick_catch_for(roll: u64, raining: bool) -> u8 {
    let hit = if raining {
        roll % RAIN_RARE_CATCH_MOD == 0
    } else {
        roll % 5 == 0
    };
    if hit {
        AETHER_FISH_ID
    } else {
        FISH_ID
    }
}

/// 依 `roll` 決定這一竿要等幾秒才上鉤，落在 `[BITE_MIN_SECS, BITE_MAX_SECS]`。
///
/// 隨機化上鉤時間＝每次拋竿的「等」都不太一樣，才有真釣魚的味道（不是固定倒數）。
pub fn bite_secs(roll: u64) -> u64 {
    bite_secs_for(roll, false)
}

/// 依 `roll` 決定這一竿要等幾秒才上鉤，`raining` 決定範圍（雨天更快，只獎不罰）。
///
/// 確定性純函式（同一 `roll`＋`raining` 恆得同一結果）。
pub fn bite_secs_for(roll: u64, raining: bool) -> u64 {
    let (min, max) = if raining {
        (RAIN_BITE_MIN_SECS, RAIN_BITE_MAX_SECS)
    } else {
        (BITE_MIN_SECS, BITE_MAX_SECS)
    };
    let span = max - min + 1;
    min + roll % span
}

/// 漁獲的中文名（自給自足，與 `voxel_gift::item_name_zh` 同步）。
pub fn fish_name_zh(id: u8) -> &'static str {
    match id {
        FISHING_ROD_ID => "釣竿",
        FISH_ID => "小魚",
        AETHER_FISH_ID => "乙太魚",
        COOKED_FISH_ID => "烤魚",
        _ => "漁獲",
    }
}

/// 拋竿成功後給玩家看的提示（前端顯示，帶等待的期待感）。
pub fn cast_hint() -> &'static str {
    cast_hint_for(false)
}

/// 拋竿成功後給玩家看的提示，`raining` 為真時明講雨天魚更活躍——
/// 讓玩家看得懂「這竿為什麼等得比平常快」，而不只是默默變快。
pub fn cast_hint_for(raining: bool) -> &'static str {
    if raining {
        "🎣🌧️ 拋竿了——雨天魚兒特別活躍，浮標應該很快就有動靜…"
    } else {
        "🎣 拋竿了——浮標靜靜漂在水面，靜候魚兒上鉤…"
    }
}

/// 太早收竿（魚還沒上鉤）給玩家看的提示。
pub fn too_early_hint() -> &'static str {
    "浮標還穩穩地浮著，別急，再等一會兒…"
}

/// 釣起漁獲後給玩家看的一句話（依是否稀有分岔語氣）。
pub fn catch_self_line(fish_id: u8) -> String {
    if fish_id == AETHER_FISH_ID {
        "✨ 收竿！一尾泛著青藍幽光的乙太魚破水而出——好稀有的漁獲！".to_string()
    } else {
        format!("🎣 收竿！釣起一尾活蹦亂跳的{}！", fish_name_zh(fish_id))
    }
}

/// 釣起漁獲後寫進世界動態 feed 的一行（讓不在場的人回來也讀得到誰在河邊釣到了什麼）。
pub fn catch_feed_line(player: &str, fish_id: u8) -> String {
    if fish_id == AETHER_FISH_ID {
        format!("{player} 在水邊釣起了一尾稀有的乙太魚，青藍幽光在掌心閃動 ✨")
    } else {
        format!("{player} 在水邊釣起了一尾{}", fish_name_zh(fish_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_blocks_are_castable_solids_are_not() {
        assert!(is_water_block(7), "來源水可下竿");
        for lvl in 24..=30 {
            assert!(is_water_block(lvl), "流動水 level {lvl} 可下竿");
        }
        // 陸地/建材不是水，不能對著它拋竿。
        for solid in [0u8, 1, 2, 3, 4, 5, 8, 9, 10, 23, 31] {
            assert!(!is_water_block(solid), "方塊 {solid} 不該可下竿");
        }
        // 流動水範圍邊界外（23=鐵磚、31=火把）不算水。
        assert!(!is_water_block(23));
        assert!(!is_water_block(31));
    }

    #[test]
    fn pick_catch_is_deterministic_and_both_outcomes_reachable() {
        // 同一 roll 恆得同一結果。
        assert_eq!(pick_catch(10), pick_catch(10));
        // roll % 5 == 0 → 乙太魚；否則小魚。
        assert_eq!(pick_catch(0), AETHER_FISH_ID);
        assert_eq!(pick_catch(5), AETHER_FISH_ID);
        assert_eq!(pick_catch(1), FISH_ID);
        assert_eq!(pick_catch(4), FISH_ID);
        // 掃一段連續 roll，兩種漁獲都要出現（分佈健康、不會永遠只釣到一種）。
        let mut common = 0;
        let mut rare = 0;
        for r in 0..100u64 {
            match pick_catch(r) {
                AETHER_FISH_ID => rare += 1,
                FISH_ID => common += 1,
                other => panic!("非預期漁獲 id {other}"),
            }
        }
        assert_eq!(rare, 20, "100 竿裡稀有魚恰好 1/5");
        assert_eq!(common, 80);
        assert!(common > rare, "小魚該比乙太魚常見");
    }

    #[test]
    fn bite_secs_always_within_bounds() {
        for r in 0..1000u64 {
            let s = bite_secs(r);
            assert!(
                (BITE_MIN_SECS..=BITE_MAX_SECS).contains(&s),
                "roll {r} 的上鉤秒數 {s} 落在範圍外"
            );
        }
        // 邊界都要搆得到（不是永遠固定一個值）。
        assert!((0..1000u64).map(bite_secs).any(|s| s == BITE_MIN_SECS));
        assert!((0..1000u64).map(bite_secs).any(|s| s == BITE_MAX_SECS));
    }

    #[test]
    fn names_and_lines_are_grounded() {
        assert_eq!(fish_name_zh(FISH_ID), "小魚");
        assert_eq!(fish_name_zh(AETHER_FISH_ID), "乙太魚");
        assert_eq!(fish_name_zh(COOKED_FISH_ID), "烤魚");
        assert_eq!(fish_name_zh(FISHING_ROD_ID), "釣竿");
        // 稀有與普通漁獲的自述與 feed 台詞要分岔、且點名漁獲/玩家。
        assert!(catch_self_line(AETHER_FISH_ID).contains("乙太魚"));
        assert!(catch_self_line(FISH_ID).contains("小魚"));
        let feed = catch_feed_line("露米", AETHER_FISH_ID);
        assert!(feed.contains("露米") && feed.contains("乙太魚"));
        assert!(catch_feed_line("諾亞", FISH_ID).contains("諾亞"));
    }

    #[test]
    fn rain_pick_catch_boosts_rare_chance_without_penalty() {
        // 同一 roll，晴天/雨天恆與各自模式的原函式一致（無回歸）。
        for r in 0..50u64 {
            assert_eq!(pick_catch(r), pick_catch_for(r, false));
        }
        // 雨天門檻收窄成 1/3，掃一段連續 roll 驗證比例確實提高、且仍非必得（只獎不罰）。
        let mut rain_rare = 0;
        let mut sun_rare = 0;
        for r in 0..300u64 {
            if pick_catch_for(r, true) == AETHER_FISH_ID {
                rain_rare += 1;
            }
            if pick_catch_for(r, false) == AETHER_FISH_ID {
                sun_rare += 1;
            }
        }
        assert_eq!(rain_rare, 100, "300 竿裡雨天稀有魚恰好 1/3");
        assert_eq!(sun_rare, 60, "300 竿裡晴天稀有魚恰好 1/5");
        assert!(rain_rare > sun_rare, "雨天稀有魚該比晴天更容易釣到");
        assert!(rain_rare < 300, "雨天仍非必得，只是機率提高");
    }

    #[test]
    fn rain_bite_secs_faster_but_still_bounded_and_no_regression() {
        // 同一 roll，晴天恆與原函式一致（無回歸）。
        for r in 0..50u64 {
            assert_eq!(bite_secs(r), bite_secs_for(r, false));
        }
        for r in 0..1000u64 {
            let s = bite_secs_for(r, true);
            assert!(
                (RAIN_BITE_MIN_SECS..=RAIN_BITE_MAX_SECS).contains(&s),
                "雨天 roll {r} 的上鉤秒數 {s} 落在範圍外"
            );
        }
        // 邊界都要搆得到。
        assert!((0..1000u64).map(|r| bite_secs_for(r, true)).any(|s| s == RAIN_BITE_MIN_SECS));
        assert!((0..1000u64).map(|r| bite_secs_for(r, true)).any(|s| s == RAIN_BITE_MAX_SECS));
        // 雨天上界比晴天下界還快——整體明顯更緊湊（只獎不罰，不會比晴天慢）。
        assert!(RAIN_BITE_MAX_SECS < BITE_MAX_SECS);
        assert!(RAIN_BITE_MAX_SECS <= BITE_MIN_SECS + 1);
    }

    #[test]
    fn cast_hint_mentions_rain_only_when_raining() {
        assert_eq!(cast_hint(), cast_hint_for(false));
        assert!(!cast_hint_for(false).contains("雨"));
        assert!(cast_hint_for(true).contains("雨"));
    }

    #[test]
    fn item_ids_do_not_collide() {
        // 釣魚四件物品 id 互不相同，且都在既有方塊 id（≤59）之上（避免撞方塊 enum）。
        let ids = [FISHING_ROD_ID, FISH_ID, AETHER_FISH_ID, COOKED_FISH_ID];
        for (i, a) in ids.iter().enumerate() {
            assert!(*a >= 60, "釣魚物品 id {a} 應 ≥60，避開既有方塊");
            for b in &ids[i + 1..] {
                assert_ne!(a, b, "釣魚物品 id 不可重複");
            }
        }
    }
}
