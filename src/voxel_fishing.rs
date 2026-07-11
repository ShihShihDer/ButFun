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

// ── 稀有魚 v1（ROADMAP 939）：垂釣第一次有「哇」的驚喜與收藏感 ──────────────────
//
// 此前垂釣只有兩種下場：小魚（常見）或乙太魚（約 1/5 稀有）。稀有度單一、環境無關——
// 白天黑夜、深水淺灘、什麼群系，釣起乙太魚的機率全都一樣，釣魚缺少「今晚特別容易釣到
// 稀奇貨」的期待。本節補上兩種**環境限定**的稀有魚，讓時段／深水／群系第一次影響漁獲：
//   - 🌙 夜光魚（MOONFISH，111）：**夜裡**才咬鉤的月光魚，通體泛著珍珠般柔白微光。
//     只有深夜／入夜時段（`night == true`）才有機會釣起，白天絕不上鉤。
//   - 🌌 深海乙太魚（ABYSSAL_FISH，112）：**深水**裡潛藏的乙太方界最珍稀漁獲，全身流淌
//     著幽藍星芒。要對著夠深的水體（水面下連續數格皆水）下竿才有機會，是所有漁獲裡最稀有
//     的一種；雪原深湖的極寒水域（Snow 群系）更容易釣起。
// **稀有度階梯**：小魚（最常見）< 乙太魚（1/5）< 夜光魚（夜間限定）< 深海乙太魚（深水限定、最稀）。

/// 夜光魚物品 ID——夜裡才咬鉤的月光魚，通體柔白微光，比乙太魚更稀有的收藏珍寶。
/// 純物品（住背包不可放置），只在深夜／入夜時段垂釣才有機會釣起。
pub const MOONFISH_ID: u8 = 111;
/// 深海乙太魚物品 ID——深水裡潛藏的乙太方界最珍稀漁獲，全身流淌幽藍星芒。
/// 純物品（住背包不可放置），只在夠深的水體下竿才有機會，雪原極寒深湖更易上鉤。
pub const ABYSSAL_FISH_ID: u8 = 112;

/// 判定「深水」所需的水面下連續水深（格）——水面往下至少這麼多格都是水才算深水。
/// 河流／小水窪多半淺，唯有湖心／海域才夠深，深海乙太魚只在這種水域潛藏。
pub const DEEP_WATER_DEPTH: i32 = 3;

/// 一竿的環境情境——決定稀有魚是否可能上鉤、機率高低。純資料，由呼叫端從世界狀態組出。
///
/// 全欄位皆確定性快照（拋竿/收竿當下的天氣、時段、水深、群系），純函式據此抽漁獲，
/// 讓「今晚在雪原深湖釣魚」與「大白天在小溪釣魚」第一次會釣到不一樣的東西。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FishContext {
    /// 是否下雨（雨天魚更活躍，乙太魚機率提高——沿用雨天垂釣 v1）。
    pub raining: bool,
    /// 是否為夜晚（深夜／入夜）——夜光魚只在夜裡才咬鉤。
    pub night: bool,
    /// 下竿的水體是否夠深（水面下連續 `DEEP_WATER_DEPTH` 格皆水）——深海乙太魚只在深水潛藏。
    pub deep: bool,
    /// 下竿處是否為雪原群系（Snow）——極寒深湖讓深海乙太魚更容易上鉤。
    pub snow: bool,
}

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

/// 依「水面往下連續幾格是水」判斷是否為深水（`depth >= DEEP_WATER_DEPTH`）。
///
/// 確定性純函式：呼叫端（`voxel_ws`）從水面往下逐格探 `is_water_block`、算出連續水深後
/// 傳進來即可，本函式不碰世界狀態、可單元測試。淺溪／小水窪深度不足 → 非深水（釣不到
/// 深海乙太魚）；湖心／海域夠深 → 深水。
pub fn is_deep_water(consecutive_depth: i32) -> bool {
    consecutive_depth >= DEEP_WATER_DEPTH
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

// ── 稀有魚抽選階梯（ROADMAP 939）─────────────────────────────────────────────────

/// 深海乙太魚基礎機率的模數：深水下竿時 `roll % ABYSSAL_CATCH_MOD == 0` → 深海乙太魚。
/// 1/12＝比乙太魚（1/5）更稀有，是所有漁獲裡最難得的一種。
pub const ABYSSAL_CATCH_MOD: u64 = 12;
/// 雪原深湖的深海乙太魚機率模數：極寒水域收窄成 1/7，比一般深水更容易上鉤（只獎不罰）。
pub const ABYSSAL_SNOW_CATCH_MOD: u64 = 7;
/// 夜光魚機率的模數：夜裡下竿時 `roll % MOONFISH_CATCH_MOD == 0` → 夜光魚。
/// 1/8＝介於乙太魚與深海乙太魚之間的稀有度，夜釣專屬的驚喜。
pub const MOONFISH_CATCH_MOD: u64 = 8;

/// 依環境情境 `ctx` 抽選漁獲——稀有度階梯的完整版（含夜光魚／深海乙太魚）。
///
/// 確定性純函式（同一 `roll`＋`ctx` 恆得同一結果）。抽選優先序（由稀到常，先中先得）：
///   1. **深海乙太魚**（僅深水）：最珍稀，雪原深湖機率更高。
///   2. **夜光魚**（僅夜晚）：夜釣專屬。
///   3. **乙太魚**（沿用 `pick_catch_for`，雨天加成）。
///   4. **小魚**（保底）。
/// 用不同「相位」的 roll（乘上互質常數再取模）判定各稀有魚，讓三種稀有判定彼此獨立、
/// 不會因為共用同一個 `% k` 而系統性地互相排擠或疊在同一批 roll 上。
pub fn pick_catch_ctx(roll: u64, ctx: FishContext) -> u8 {
    // 深海乙太魚：只在深水潛藏（非深水永遠釣不到）。雪原極寒深湖機率更高（只獎不罰）。
    if ctx.deep {
        let modulo = if ctx.snow { ABYSSAL_SNOW_CATCH_MOD } else { ABYSSAL_CATCH_MOD };
        if roll.wrapping_mul(2_654_435_761) % modulo == 0 {
            return ABYSSAL_FISH_ID;
        }
    }
    // 夜光魚：只在夜裡咬鉤（白天永遠釣不到）。
    if ctx.night && roll.wrapping_mul(40_503) % MOONFISH_CATCH_MOD == 0 {
        return MOONFISH_ID;
    }
    // 其餘沿用既有小魚／乙太魚抽選（雨天乙太魚機率加成，向後相容）。
    pick_catch_for(roll, ctx.raining)
}

/// 是否為「稀有」漁獲（乙太魚／夜光魚／深海乙太魚）——收竿時要不要跳「哇」驚喜提示。
/// 小魚是最常見的日常漁獲，不算驚喜。純函式、可測。
pub fn is_rare_catch(fish_id: u8) -> bool {
    matches!(fish_id, AETHER_FISH_ID | MOONFISH_ID | ABYSSAL_FISH_ID)
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
        MOONFISH_ID => "夜光魚",
        ABYSSAL_FISH_ID => "深海乙太魚",
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

/// 釣起漁獲後給玩家看的一句話（依稀有度分岔語氣，稀有魚各有專屬「哇」的驚喜句）。
pub fn catch_self_line(fish_id: u8) -> String {
    match fish_id {
        ABYSSAL_FISH_ID =>
            "🌌 收竿！深水裡竟拉起一尾流淌著幽藍星芒的深海乙太魚——傳說中最珍稀的漁獲！哇！！"
                .to_string(),
        MOONFISH_ID =>
            "🌙 收竿！月光下一尾泛著珍珠柔白微光的夜光魚破水而出——好美的夜釣驚喜！哇！"
                .to_string(),
        AETHER_FISH_ID =>
            "✨ 收竿！一尾泛著青藍幽光的乙太魚破水而出——好稀有的漁獲！".to_string(),
        _ => format!("🎣 收竿！釣起一尾活蹦亂跳的{}！", fish_name_zh(fish_id)),
    }
}

/// 釣起漁獲後寫進世界動態 feed 的一行（讓不在場的人回來也讀得到誰在河邊釣到了什麼）。
pub fn catch_feed_line(player: &str, fish_id: u8) -> String {
    match fish_id {
        ABYSSAL_FISH_ID => format!(
            "{player} 從深水裡釣起了一尾傳說中的深海乙太魚，幽藍星芒在掌心流轉 🌌"
        ),
        MOONFISH_ID => format!(
            "{player} 在月光下釣起了一尾夜光魚，珍珠般的柔白微光靜靜漾著 🌙"
        ),
        AETHER_FISH_ID => format!(
            "{player} 在水邊釣起了一尾稀有的乙太魚，青藍幽光在掌心閃動 ✨"
        ),
        _ => format!("{player} 在水邊釣起了一尾{}", fish_name_zh(fish_id)),
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
        // 釣魚全部物品 id 互不相同，且都在既有方塊 id（≤59）之上（避免撞方塊 enum）。
        // 稀有魚 111/112 接續已用到的 110（南瓜）之後，不撞既有任何物品/方塊。
        let ids = [
            FISHING_ROD_ID, FISH_ID, AETHER_FISH_ID, COOKED_FISH_ID,
            MOONFISH_ID, ABYSSAL_FISH_ID,
        ];
        for (i, a) in ids.iter().enumerate() {
            assert!(*a >= 60, "釣魚物品 id {a} 應 ≥60，避開既有方塊");
            for b in &ids[i + 1..] {
                assert_ne!(a, b, "釣魚物品 id 不可重複");
            }
        }
        // 稀有魚 id 接在既有最大號 110（南瓜）之後，從 111 起。
        assert_eq!(MOONFISH_ID, 111);
        assert_eq!(ABYSSAL_FISH_ID, 112);
    }

    /// 建一個測試用情境（預設晴天／白天／淺水／非雪原）。
    fn ctx(raining: bool, night: bool, deep: bool, snow: bool) -> FishContext {
        FishContext { raining, night, deep, snow }
    }

    #[test]
    fn deep_water_threshold() {
        // 未達門檻（淺溪／水窪）不算深水；達門檻起算深水。
        assert!(!is_deep_water(0));
        assert!(!is_deep_water(DEEP_WATER_DEPTH - 1));
        assert!(is_deep_water(DEEP_WATER_DEPTH));
        assert!(is_deep_water(DEEP_WATER_DEPTH + 5));
    }

    #[test]
    fn ctx_names_and_rarity_flags() {
        assert_eq!(fish_name_zh(MOONFISH_ID), "夜光魚");
        assert_eq!(fish_name_zh(ABYSSAL_FISH_ID), "深海乙太魚");
        // 稀有判定：三種稀有魚都算稀有，小魚不算。
        assert!(is_rare_catch(AETHER_FISH_ID));
        assert!(is_rare_catch(MOONFISH_ID));
        assert!(is_rare_catch(ABYSSAL_FISH_ID));
        assert!(!is_rare_catch(FISH_ID));
        assert!(!is_rare_catch(COOKED_FISH_ID));
    }

    #[test]
    fn moonfish_only_bites_at_night() {
        // 白天：掃一整段 roll 都不該釣起夜光魚（夜光魚白天絕不上鉤）。
        for r in 0..2000u64 {
            assert_ne!(
                pick_catch_ctx(r, ctx(false, false, false, false)),
                MOONFISH_ID,
                "白天不該釣起夜光魚（roll {r}）"
            );
        }
        // 夜晚：淺水非雪原時，夜光魚要真的出現過（不是永遠釣不到）。
        let got_moon = (0..2000u64)
            .any(|r| pick_catch_ctx(r, ctx(false, true, false, false)) == MOONFISH_ID);
        assert!(got_moon, "夜裡淺水應能釣起夜光魚");
    }

    #[test]
    fn abyssal_only_bites_in_deep_water() {
        // 淺水：掃一整段 roll 都不該釣起深海乙太魚（只在深水潛藏）。
        for r in 0..2000u64 {
            // 淺水（deep=false），連夜晚也不該出現深海乙太魚。
            assert_ne!(
                pick_catch_ctx(r, ctx(false, true, false, false)),
                ABYSSAL_FISH_ID,
                "淺水不該釣起深海乙太魚（roll {r}）"
            );
        }
        // 深水：深海乙太魚要真的出現過。
        let got_abyssal = (0..2000u64)
            .any(|r| pick_catch_ctx(r, ctx(false, false, true, false)) == ABYSSAL_FISH_ID);
        assert!(got_abyssal, "深水應能釣起深海乙太魚");
    }

    #[test]
    fn snow_deep_water_boosts_abyssal_without_penalty() {
        // 雪原深湖的深海乙太魚該比一般深水更容易釣到（只獎不罰）。
        let plain = (0..6000u64)
            .filter(|&r| pick_catch_ctx(r, ctx(false, false, true, false)) == ABYSSAL_FISH_ID)
            .count();
        let snow = (0..6000u64)
            .filter(|&r| pick_catch_ctx(r, ctx(false, false, true, true)) == ABYSSAL_FISH_ID)
            .count();
        assert!(plain > 0, "一般深水也要釣得到深海乙太魚");
        assert!(snow > plain, "雪原深湖的深海乙太魚該更容易釣到（{snow} > {plain}）");
        // 門檻常數本身也體現「雪原更容易」（模數更小＝機率更高）。
        assert!(ABYSSAL_SNOW_CATCH_MOD < ABYSSAL_CATCH_MOD);
    }

    #[test]
    fn ctx_is_deterministic_and_backward_compatible() {
        // 同一 roll＋ctx 恆得同一結果。
        let c = ctx(true, true, true, true);
        for r in 0..100u64 {
            assert_eq!(pick_catch_ctx(r, c), pick_catch_ctx(r, c));
        }
        // 晴天／白天／淺水／非雪原 → 完全退回既有小魚／乙太魚抽選（向後相容、無回歸）。
        let base = ctx(false, false, false, false);
        for r in 0..300u64 {
            assert_eq!(pick_catch_ctx(r, base), pick_catch_for(r, false));
        }
        // 雨天但淺水白天 → 仍是既有雨天抽選（雨天乙太魚加成，不牽動稀有魚）。
        let rainy = ctx(true, false, false, false);
        for r in 0..300u64 {
            assert_eq!(pick_catch_ctx(r, rainy), pick_catch_for(r, true));
        }
    }

    #[test]
    fn all_four_fish_reachable_and_small_still_common() {
        // 全開情境（雨夜深水雪原）：四種漁獲都要搆得到、且小魚仍是保底大宗。
        let full = ctx(true, true, true, true);
        let mut small = 0;
        let mut aether = 0;
        let mut moon = 0;
        let mut abyssal = 0;
        for r in 0..6000u64 {
            match pick_catch_ctx(r, full) {
                FISH_ID => small += 1,
                AETHER_FISH_ID => aether += 1,
                MOONFISH_ID => moon += 1,
                ABYSSAL_FISH_ID => abyssal += 1,
                other => panic!("非預期漁獲 id {other}"),
            }
        }
        assert!(small > 0 && aether > 0 && moon > 0 && abyssal > 0, "四種漁獲都要出現");
        // 小魚仍是最常見（療癒向：稀有魚是驚喜、不是常態），比每一種稀有魚都多。
        assert!(small > aether, "小魚該比乙太魚常見");
        assert!(small > moon, "小魚該比夜光魚常見");
        assert!(small > abyssal, "小魚該比深海乙太魚常見");
    }

    #[test]
    fn abyssal_is_rarest_in_plain_deep_water() {
        // 一般深水（非雪原、非夜、非雨）：深海乙太魚是最稀有的漁獲——比乙太魚都少。
        // （雪原深湖有加成會反超，故稀有度階梯以「一般深水」為準線衡量。）
        let plain_deep = FishContext { raining: false, night: false, deep: true, snow: false };
        let mut small = 0;
        let mut aether = 0;
        let mut abyssal = 0;
        for r in 0..6000u64 {
            match pick_catch_ctx(r, plain_deep) {
                FISH_ID => small += 1,
                AETHER_FISH_ID => aether += 1,
                ABYSSAL_FISH_ID => abyssal += 1,
                MOONFISH_ID => panic!("白天不該出現夜光魚"),
                other => panic!("非預期漁獲 id {other}"),
            }
        }
        assert!(abyssal > 0, "一般深水也要釣得到深海乙太魚");
        assert!(abyssal < aether, "深海乙太魚該比乙太魚更稀有（{abyssal} < {aether}）");
        assert!(small > aether, "小魚仍是最常見");
    }

    #[test]
    fn rare_catch_lines_are_distinct_and_grounded() {
        // 三種稀有魚的自述／feed 各有專屬語氣、點名漁獲。
        assert!(catch_self_line(ABYSSAL_FISH_ID).contains("深海乙太魚"));
        assert!(catch_self_line(MOONFISH_ID).contains("夜光魚"));
        assert!(catch_self_line(AETHER_FISH_ID).contains("乙太魚"));
        // 稀有魚自述帶「哇」的驚喜感、feed 點名玩家。
        assert!(catch_self_line(ABYSSAL_FISH_ID).contains("哇"));
        assert!(catch_self_line(MOONFISH_ID).contains("哇"));
        let feed_a = catch_feed_line("露米", ABYSSAL_FISH_ID);
        assert!(feed_a.contains("露米") && feed_a.contains("深海乙太魚"));
        let feed_m = catch_feed_line("諾亞", MOONFISH_ID);
        assert!(feed_m.contains("諾亞") && feed_m.contains("夜光魚"));
    }
}
