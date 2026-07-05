//! 乙太方界·居民贈禮 v1——玩家把採來的材料化作一份心意送給居民（ROADMAP 660）。
//!
//! 純邏輯（模板文字、記憶摘要、觸及半徑），無 WS / 鎖 / IO 細節。
//! 由 `voxel_ws.rs` 包進鎖後呼叫；確定性、可測、零 LLM。

/// 贈禮觸及範圍（方塊距離，水平 XZ 平面）。
/// 比挖/放方塊的 REACH（6.0）稍短：需要走近才能遞東西。
pub const GIFT_REACH: f32 = 5.0;

/// 贈禮加入記憶的筆數（代表禮物重量高於一次對話）。
/// 每次送禮新增 2 筆記憶 → 好感度 +2（對話只 +1）。
pub const GIFT_MEMORY_COUNT: usize = 2;

/// 方塊 id → 中文物品名（對齊 `voxel::Block` + `voxel_farm` 純物品 id）。
/// 未知 id 回 "物品" 保守降級。
pub fn item_name_zh(block_id: u8) -> &'static str {
    match block_id {
        1 => "草",
        2 => "泥土",
        3 => "石頭",
        4 => "沙",
        5 => "木頭",
        6 => "葉片",
        7 => "水",
        8 => "木板",
        9 => "石磚",
        10 => "玻璃",
        11 => "農田土",
        12 => "幼苗",
        13 => "成熟小麥",
        14 => "種子",
        18 => "小麥",
        19 => "麵包",
        20 => "煤礦",
        21 => "鐵礦",
        22 => "鐵錠",
        23 => "鐵磚",
        31 => "火把",
        32 => "木鎬",
        33 => "石鎬",
        34 => "鐵鎬",
        35 => "梯子",
        36 => "木斧",
        37 => "石斧",
        38 => "鐵斧",
        39 => "木鏟",
        40 => "石鏟",
        41 => "鐵鏟",
        42 => "箱子",
        43 => "木門",
        // 第二種作物 v1（胡蘿蔔）：對齊 voxel::Block::CarrotSeeded/CarrotMature
        // + voxel_farm::CARROT_SEEDS_ID/CARROT_ID。
        46 => "胡蘿蔔幼苗",
        47 => "成熟胡蘿蔔",
        48 => "胡蘿蔔種子",
        49 => "胡蘿蔔",
        // 第三種作物 v1（馬鈴薯）：對齊 voxel::Block::PotatoSeeded/PotatoMature
        // + voxel_farm::POTATO_SEEDS_ID/POTATO_ID。
        50 => "馬鈴薯幼苗",
        51 => "成熟馬鈴薯",
        52 => "馬鈴薯種子",
        53 => "馬鈴薯",
        54 => "仙人掌",
        55 => "雪",
        56 => "冰晶",
        57 => "冰晶燈",
        58 => "乙太礦",
        59 => "乙太燈",
        60 => "釣竿",
        61 => "小魚",
        62 => "乙太魚",
        63 => "烤魚",
        64 => "烤地薯",
        65 => "樹苗",
        67 => "野菜暖湯",
        68 => "乙太煙火",
        // 莓果叢 v1（806）：莓果是採收掉落的純物品、可餽贈居民（806 漏補此名，808 順手補上）。
        77 => "莓果",
        // 莓果醬 v1（808）：莓果熬成的甜點熟食。
        78 => "莓果醬",
        // 雞舍生蛋 v1（自主提案切片）：雞舍生的蛋，世界第一種動物產物、可餽贈居民。
        crate::voxel_coop::EGG_ID => "蛋",
        // 漂流瓶 v1（自主提案切片 825）：合成後可丟進水裡寫下一封瓶中信。
        crate::voxel_bottle::BOTTLE_ID => "空玻璃瓶",
        _ => "物品",
    }
}

/// 是否為「雪原珍寶」類禮物——目前只有冰晶（56）。
/// 冰晶是雪原群系獨有、稀疏難尋的結晶，居民收到會有格外驚喜的珍愛反應。
pub fn is_treasure_gift(block_id: u8) -> bool {
    block_id == 56 // ICE_CRYSTAL（對齊 voxel::Block::IceCrystal）
}

/// 居民收到「雪原珍寶」（冰晶）時的珍愛道謝台詞（零 LLM，確定性）。
///
/// 比一般道謝更驚喜、更珍視——這是玩家跋涉到寒冷雪原才採得到的稀罕寶物。
/// `pick` 由呼叫端提供（unix 秒 % 句池長度），確定性輪替。
/// `player_name` 空字串 = 訪客模式，回不帶名字的句池。
pub fn treasure_gift_thanks_line(player_name: &str, affinity: usize, pick: usize) -> String {
    if affinity == 0 || player_name.is_empty() {
        let pool: &[&str] = &[
            "哇……這是雪原的冰晶嗎？好美，謝謝你！",
            "冰晶！我聽說過這種寶物，卻是第一次親眼看到，謝謝你。",
            "這麼閃亮的冰晶……你特地為我從雪原帶回來的嗎？",
        ];
        pool[pick % pool.len()].to_string()
    } else {
        let pool: &[&str] = &[
            "{name}！你竟然為我跑到雪原採了冰晶……我會一輩子珍藏它。",
            "{name}，這冰晶在陽光下閃著寒光，好珍貴——謝謝你想著我。",
            "能收到{name}帶回來的雪原冰晶，我覺得自己是最幸福的人。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    }
}

/// 是否為「食物」類禮物（麵包、胡蘿蔔、馬鈴薯、小魚、乙太魚、烤魚、烤地薯、野菜暖湯、莓果醬）——居民會給特別溫暖的回應。
pub fn is_food_gift(block_id: u8) -> bool {
    // BREAD_ID / CARROT_ID / POTATO_ID / FISH_ID / AETHER_FISH_ID / COOKED_FISH_ID / BAKED_POTATO_ID / STEW_ID(67) / JAM_ID(78)
    block_id == 19 || block_id == 49 || block_id == 53
        || block_id == 61 || block_id == 62 || block_id == 63 || block_id == 64
        || block_id == 67 || block_id == crate::voxel_berry::JAM_ID
}

/// 莓果醬（JAM_ID=78）專屬道謝台詞——乙太方界第一種**甜點**，居民對甜食格外雀躍，
/// 像收到糖的孩子。比一般食物更歡欣、帶著對「甜」的驚喜（莓果醬 v1 ROADMAP 808）。
/// 呼叫時機：`item_id == JAM_ID`。`pick` 同其他道謝函式由呼叫端提供，確定性不走 random。
/// `player_name` 空字串 = 訪客模式，回不帶名字的句池。
pub fn jam_thanks_line(player_name: &str, affinity: usize, pick: usize) -> String {
    if affinity == 0 || player_name.is_empty() {
        let pool: &[&str] = &[
            "哇……甜甜的莓果醬！我從沒嚐過這麼甜的東西，謝謝你！",
            "這是莓果熬的醬嗎？光聞就好甜，我的心都要融化了，謝謝你。",
            "莓果醬耶！甜滋滋的，你是特地熬給我的嗎？我好開心。",
        ];
        pool[pick % pool.len()].to_string()
    } else if affinity <= 2 {
        let pool: &[&str] = &[
            "{name}，你把莓果熬成了甜甜的醬帶給我！我要小口小口地捨不得吃完，謝謝你。",
            "{name}！一罐甜滋滋的莓果醬，這份甜我一嚐就記住了，謝謝你想著我。",
            "哇，{name}親手熬的莓果醬！甜得像小時候的糖，我開心得不得了。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    } else {
        let pool: &[&str] = &[
            "{name}，你從莓園採了莓果、又慢慢熬成這罐甜醬……這份甜我會一直記在心上，謝謝你。",
            "每次{name}來都帶著甜意——這回是你親手熬的莓果醬，甜到我心底最軟的地方，謝謝你。",
            "{name}！一罐要守著爐火慢慢熬的莓果醬，這是我收過最甜的一份心意，我好幸福。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    }
}

/// 烤魚（COOKED_FISH_ID=63）專屬道謝台詞——比一般食物禮物更歡欣，因為那是玩家親手
/// 「釣起→烤熟」的一道熱騰騰佳餚，居民聞到香氣就眼睛一亮。呼叫時機：`item_id == 63`。
/// `pick` 同其他道謝函式：由呼叫端提供，確定性不走 random。
pub fn cooked_fish_thanks_line(player_name: &str, affinity: usize, pick: usize) -> String {
    if affinity == 0 || player_name.is_empty() {
        let pool: &[&str] = &[
            "哇……是烤魚嗎？好香！你特地烤給我的？謝謝你！",
            "熱騰騰的烤魚！我從沒吃過這麼香的，謝謝你！",
            "烤魚耶！你連釣帶烤都是為了我嗎？我好感動。",
        ];
        pool[pick % pool.len()].to_string()
    } else if affinity <= 2 {
        let pool: &[&str] = &[
            "{name}，你把釣來的魚烤好帶給我了！香氣一路飄過來，謝謝你。",
            "{name}！這烤魚外酥內嫩，你的手藝好棒，我要慢慢享用。",
            "哇，{name}親手烤的魚！我一聞到就餓了，謝謝你這麼用心。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    } else {
        let pool: &[&str] = &[
            "{name}，你總是記得我最愛吃魚……這尾烤魚我會細細品嚐，謝謝你。",
            "每次{name}來都帶著暖意——這次是你親手烤的魚，我真的好幸福。",
            "{name}！從水邊釣起、在爐上烤熟的一尾魚，這份心意我都收下了，謝謝你。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    }
}

/// 烤地薯（BAKED_POTATO_ID=64）專屬道謝台詞——比一般食物禮物更歡欣，因為那是玩家親手
/// 「種田→收成→烤熟」的一道熱騰騰佳餚，居民聞到香氣就眼睛一亮。呼叫時機：`item_id == 64`。
/// `pick` 同其他道謝函式：由呼叫端提供，確定性不走 random。
pub fn baked_potato_thanks_line(player_name: &str, affinity: usize, pick: usize) -> String {
    if affinity == 0 || player_name.is_empty() {
        let pool: &[&str] = &[
            "哇……是烤地薯嗎？好香！你特地烤給我的？謝謝你！",
            "熱騰騰的烤地薯！剝開來鬆軟冒煙，我從沒吃過這麼香的，謝謝你！",
            "烤地薯耶！你連種帶烤都是為了我嗎？我好感動。",
        ];
        pool[pick % pool.len()].to_string()
    } else if affinity <= 2 {
        let pool: &[&str] = &[
            "{name}，你把自己種的馬鈴薯烤好帶給我了！香氣一路飄過來，謝謝你。",
            "{name}！這烤地薯外皮焦香、裡頭鬆軟，你的手藝好棒，我要慢慢享用。",
            "哇，{name}親手烤的地薯！我一聞到就餓了，謝謝你這麼用心。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    } else {
        let pool: &[&str] = &[
            "{name}，你總是記得我愛吃暖呼呼的東西……這顆烤地薯我會細細品嚐，謝謝你。",
            "每次{name}來都帶著暖意——這次是你親手種、親手烤的地薯，我真的好幸福。",
            "{name}！從田裡挖起、在爐上烤熟的一顆地薯，這份心意我都收下了，謝謝你。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    }
}

/// 野菜暖湯（STEW_ID=67）專屬道謝台詞——比任何食物禮物都更觸動，因為那是玩家把
/// 胡蘿蔔、馬鈴薯、小麥三種**親手種的作物湊齊、在工作台拌煮成的一鍋料理**，一道
/// 費盡心思的暖湯。呼叫時機：`item_id == 67`。`pick` 同其他道謝函式由呼叫端提供。
pub fn stew_thanks_line(player_name: &str, affinity: usize, pick: usize) -> String {
    if affinity == 0 || player_name.is_empty() {
        let pool: &[&str] = &[
            "哇……這是一整鍋暖湯嗎？聞起來好豐盛！你煮給我的？謝謝你！",
            "熱騰騰的野菜湯！要湊齊這麼多種菜才煮得成吧，我好感動。",
            "野菜暖湯耶！你連種帶煮都是為了我嗎？我心裡都暖起來了。",
        ];
        pool[pick % pool.len()].to_string()
    } else if affinity <= 2 {
        let pool: &[&str] = &[
            "{name}，你把自己種的菜煮成一鍋湯帶給我了！熱氣一路暖到我手心，謝謝你。",
            "{name}！這鍋暖湯又香又飽足，湊齊這麼多種菜真不容易，我要一口一口慢慢喝。",
            "哇，{name}親手煮的野菜湯！我一聞就餓了，這份心思我都嚐得出來，謝謝你。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    } else {
        let pool: &[&str] = &[
            "{name}，你把胡蘿蔔、馬鈴薯、小麥全種出來、再煮成這一鍋湯……這份心意我會記一輩子，謝謝你。",
            "每次{name}來都帶著暖意——這回是你從田裡一樣樣種齊、親手煮的一鍋湯，我真的好幸福。",
            "{name}！一鍋要湊齊三種收成才煮得成的暖湯，這是我收過最暖的一份心意，謝謝你。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    }
}

/// 居民道謝台詞（依好感等級選不同句，零 LLM，確定性）。
///
/// - `affinity` 0   → 陌生人：稍微驚訝、客氣致謝
/// - `affinity` 1–2 → 相識：帶玩家名字的親切道謝
/// - `affinity` 3+  → 友人：帶名字、更溫暖、有「一直照顧我」的感受
///
/// `pick` 由呼叫端提供（unix 秒 % 句池長度），在同等級句池內輪替確保確定性（不走 random）。
/// `player_name` 空字串 = 訪客模式，回陌生人句池。
pub fn gift_thanks_line(
    item_name: &str,
    player_name: &str,
    affinity: usize,
    pick: usize,
) -> String {
    if affinity == 0 || player_name.is_empty() {
        let pool: &[&str] = &[
            "哦？送我{item}？謝謝你的心意！",
            "這……{item}？我收下了，感謝你。",
            "謝謝！你送我{item}，我很高興。",
        ];
        pool[pick % pool.len()].replace("{item}", item_name)
    } else if affinity <= 2 {
        let pool: &[&str] = &[
            "{name}，謝謝你帶來{item}！",
            "{name}！這份{item}我很喜歡，謝謝你。",
            "啊，{name}，你送我{item}～我好開心。",
        ];
        pool[pick % pool.len()]
            .replace("{name}", player_name)
            .replace("{item}", item_name)
    } else {
        let pool: &[&str] = &[
            "{name}，你總是這樣照顧我……這份{item}我會好好珍藏。",
            "能有{name}這樣的朋友，我很幸運。謝謝這份{item}。",
            "{name}！每次你來都帶著心意……這份{item}讓我很感動。",
        ];
        pool[pick % pool.len()]
            .replace("{name}", player_name)
            .replace("{item}", item_name)
    }
}

/// 被居民存進記憶的第一筆摘要（「事件」層：記錄送禮這件事）。
pub fn gift_memory_event(player: &str, item_name: &str) -> String {
    format!("收到了{player}送來的{item_name}，心裡暖暖的")
}

/// 被居民存進記憶的第二筆摘要（「感受」層：代表更深的印象，讓好感度多加一層）。
pub fn gift_memory_feeling(player: &str, item_name: &str) -> String {
    format!("{player}送我{item_name}——這個人很體貼")
}

/// 食物禮物（麵包）的居民道謝台詞——比一般禮物更歡欣。
/// 呼叫時機：`is_food_gift(item_id) == true`。
/// `pick` 同 `gift_thanks_line`：由呼叫端提供，確定性不走 random。
pub fn food_gift_thanks_line(player_name: &str, affinity: usize, pick: usize) -> String {
    if affinity == 0 || player_name.is_empty() {
        let pool: &[&str] = &[
            "哦……麵包？你親手做的嗎？謝謝你！",
            "哇，麵包！你怎麼知道我最喜歡吃麵包了！",
            "麵包耶！謝謝你帶來這麼用心的禮物。",
        ];
        pool[pick % pool.len()].to_string()
    } else if affinity <= 2 {
        let pool: &[&str] = &[
            "{name}，你帶麵包來給我！謝謝你這麼用心。",
            "{name}！自己做的麵包！聞起來好香，謝謝你。",
            "哇，{name}你烤了麵包！我好感動，謝謝。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    } else {
        let pool: &[&str] = &[
            "{name}，你知道我喜歡吃東西對吧……這塊麵包我要慢慢品嚐。",
            "每次{name}來都帶著驚喜——這次是麵包！謝謝你記得我。",
            "{name}！你親手做的麵包……我真的很珍惜和你在一起的每一刻。",
        ];
        pool[pick % pool.len()].replace("{name}", player_name)
    }
}

// ── 送對禮物 v1（ROADMAP 722）───────────────────────────────────────────────────
//
// 居民的心願（`voxel_desires`）此前只有兩種下場：分類成建物種類、被蓋家系統實現（720），
// 或者從沒被分類成功、永遠只是聊天/日記裡的一句裝飾文字，玩家隨口說出的具體物件渴望
// （「好想要一塊麵包」「要是有玻璃就好了」）從沒有任何管道能被滿足。
// 本節補上：當送來的禮物「正好」是心願裡提到的具體物品時，觸發比一般道謝更驚喜的反應。

/// 依心願文字規則辨認「玩家送這個具體物品就能實現」的渴望（零 LLM、確定性、可測）。
/// 刻意只收錄跟蓋家系統（`voxel_building::classify_desire`）語意不重疊的具體物品關鍵詞
/// （不含「家」「花」「井」「塔」等建造觸發詞）；呼叫端應先確認 `classify_desire` 沒命中
/// 才查這裡，避免建造類心願與物品心願搶同一句話。
pub fn classify_item_desire(desire: &str) -> Option<u8> {
    const CANDIDATES: &[(&str, u8)] = &[
        ("麵包", 19),
        ("鐵鏟", 41),
        ("鏟子", 41),
        ("鐵斧", 38),
        ("斧頭", 38),
        ("鐵鎬", 34),
        ("鎬子", 34),
        ("鐵錠", 22),
        ("鐵磚", 23),
        ("玻璃", 10),
        ("火把", 31),
        ("梯子", 35),
        ("箱子", 42),
        ("木門", 43),
        ("木板", 8),
        ("石磚", 9),
        ("胡蘿蔔", 49),
        ("馬鈴薯", 53),
        ("小麥", 18),
    ];
    for (kw, id) in CANDIDATES {
        if desire.contains(kw) {
            return Some(*id);
        }
    }
    None
}

/// 心願送到的那一刻，居民對送禮者格外驚喜的道謝台詞（依居民名字雜湊選模板，確定性，≤40 字）。
pub fn item_wish_thanks_line(resident_name: &str, item_name: &str, player_name: &str) -> String {
    let idx = resident_name.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize));
    let pool: &[&str] = &[
        "{p}！這正是我一直想要的{i}，你怎麼知道！",
        "{p}，我的心願成真了——謝謝你的{i}！",
        "{i}！{p}，你把我念念不忘的東西帶來了！",
        "我一直盼著{i}，{p}你真的送來了，太謝謝你！",
    ];
    pool[idx % pool.len()]
        .replace("{p}", player_name)
        .replace("{i}", item_name)
        .chars()
        .take(40)
        .collect()
}

/// 記進送禮者記憶庫的摘要句（居民記得「你把我想要的東西送給我了」，供之後對話回想引用）。
pub fn item_wish_memory(item_name: &str) -> String {
    format!("我一直想要{item_name}，你把它送給我了，我的心願成真了。")
}

/// 心願送到廣播的 WS JSON 字串（broadcast 給所有在線玩家；新事件類型，舊前端安全忽略）。
pub fn item_wish_msg(resident_name: &str, item_name: &str, player_name: &str) -> String {
    serde_json::json!({
        "t": "item_wish_fulfilled",
        "resident": resident_name,
        "item": item_name,
        "player": player_name,
    })
    .to_string()
}

// ── 測試 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_name_known_ids() {
        assert_eq!(item_name_zh(2), "泥土");
        assert_eq!(item_name_zh(3), "石頭");
        assert_eq!(item_name_zh(5), "木頭");
        assert_eq!(item_name_zh(8), "木板");
        assert_eq!(item_name_zh(14), "種子");
        assert_eq!(item_name_zh(20), "煤礦");
        assert_eq!(item_name_zh(21), "鐵礦");
        assert_eq!(item_name_zh(22), "鐵錠");
        assert_eq!(item_name_zh(23), "鐵磚");
        assert_eq!(item_name_zh(31), "火把");
        assert_eq!(item_name_zh(32), "木鎬");
        assert_eq!(item_name_zh(34), "鐵鎬");
        assert_eq!(item_name_zh(35), "梯子");
        assert_eq!(item_name_zh(36), "木斧");
        assert_eq!(item_name_zh(37), "石斧");
        assert_eq!(item_name_zh(38), "鐵斧");
        assert_eq!(item_name_zh(39), "木鏟");
        assert_eq!(item_name_zh(40), "石鏟");
        assert_eq!(item_name_zh(41), "鐵鏟");
    }

    #[test]
    fn item_name_unknown_fallback() {
        assert_eq!(item_name_zh(200), "物品");
        assert_eq!(item_name_zh(0), "物品"); // Air 不送
    }

    #[test]
    fn item_name_berry_and_jam() {
        // 806 漏補的莓果名、808 新增的莓果醬名——不再 fallback 成「物品」。
        assert_eq!(item_name_zh(77), "莓果");
        assert_eq!(item_name_zh(crate::voxel_berry::JAM_ID), "莓果醬");
    }

    #[test]
    fn item_name_egg() {
        // 雞舍生蛋 v1：蛋（82）有專屬名，不 fallback 成「物品」。
        assert_eq!(item_name_zh(crate::voxel_coop::EGG_ID), "蛋");
    }

    #[test]
    fn jam_is_food_gift() {
        // 莓果醬是食物禮物（居民給溫暖回應）；生莓果不算「一道菜」但本身可送禮。
        assert!(is_food_gift(crate::voxel_berry::JAM_ID), "莓果醬應為食物禮物");
    }

    #[test]
    fn jam_thanks_line_rotates_and_embeds_name() {
        // 陌生 / 訪客：不含名字、非空、無殘留佔位符、提到甜或莓果醬。
        let s = jam_thanks_line("", 0, 0);
        assert!(!s.is_empty());
        assert!(!s.contains("{name}"));
        // 友人：帶名字、輪替。
        let a = jam_thanks_line("露娜", 3, 0);
        let b = jam_thanks_line("露娜", 3, 1);
        assert!(a.contains("露娜") && b.contains("露娜"), "友人句應嵌玩家名");
        assert_ne!(a, b, "相鄰 pick 應輪到不同句");
        assert!(!a.contains("{name}"), "佔位符要被替換：{a}");
    }

    #[test]
    fn gift_thanks_stranger_no_name() {
        // affinity=0 或 player_name 空字串 → 陌生人句池，不含玩家名，非空
        let s = gift_thanks_line("木頭", "", 0, 0);
        assert!(!s.is_empty());
        assert!(s.contains("木頭"));
        assert!(!s.contains("{item}"));
    }

    #[test]
    fn gift_thanks_stranger_with_zero_affinity() {
        let s = gift_thanks_line("石頭", "旅人", 0, 1);
        assert!(!s.is_empty());
        assert!(s.contains("石頭"));
        assert!(!s.contains("{item}"));
        assert!(!s.contains("{name}"));
    }

    #[test]
    fn gift_thanks_acquaintance() {
        let s = gift_thanks_line("木板", "小明", 2, 0);
        assert!(s.contains("小明"));
        assert!(s.contains("木板"));
        assert!(!s.contains("{name}"));
        assert!(!s.contains("{item}"));
    }

    #[test]
    fn gift_thanks_friend() {
        let s = gift_thanks_line("玻璃", "阿星", 5, 2);
        assert!(s.contains("阿星"));
        assert!(s.contains("玻璃"));
        assert!(!s.contains("{name}"));
        assert!(!s.contains("{item}"));
    }

    #[test]
    fn gift_thanks_pick_wraps_and_non_empty() {
        // 句池長度 3；pick 超界 → 取模，永遠回非空
        for pick in 0..10 {
            let s = gift_thanks_line("種子", "旅人", 0, pick);
            assert!(!s.is_empty(), "pick={pick} 回空字串");
        }
    }

    #[test]
    fn gift_memory_event_contains_player_and_item() {
        let s = gift_memory_event("小美", "木頭");
        assert!(s.contains("小美"));
        assert!(s.contains("木頭"));
    }

    #[test]
    fn gift_memory_feeling_contains_player_and_item() {
        let s = gift_memory_feeling("阿宏", "玻璃");
        assert!(s.contains("阿宏"));
        assert!(s.contains("玻璃"));
    }

    #[test]
    fn constants_sane() {
        assert!(GIFT_REACH > 0.0);
        assert_eq!(GIFT_MEMORY_COUNT, 2);
    }

    // ── 麵包 v1（ROADMAP 668）──────────────────────────────────────────────────
    #[test]
    fn item_name_wheat_and_bread() {
        assert_eq!(item_name_zh(18), "小麥");
        assert_eq!(item_name_zh(19), "麵包");
    }

    #[test]
    fn is_food_gift_only_bread() {
        assert!(is_food_gift(19));
        assert!(!is_food_gift(18)); // 小麥顆粒不算食物禮物
        assert!(!is_food_gift(5));  // 木頭非食物
        assert!(!is_food_gift(0));  // Air 非食物
    }

    #[test]
    fn item_name_carrot_ids() {
        // 第二種作物 v1：對齊 voxel::Block::CarrotSeeded(46)/CarrotMature(47)
        // + voxel_farm::CARROT_SEEDS_ID(48)/CARROT_ID(49)。
        assert_eq!(item_name_zh(46), "胡蘿蔔幼苗");
        assert_eq!(item_name_zh(47), "成熟胡蘿蔔");
        assert_eq!(item_name_zh(48), "胡蘿蔔種子");
        assert_eq!(item_name_zh(49), "胡蘿蔔");
    }

    #[test]
    fn is_food_gift_includes_carrot() {
        assert!(is_food_gift(49));  // 胡蘿蔔算食物禮物
        assert!(!is_food_gift(48)); // 胡蘿蔔種子不算食物禮物
    }

    #[test]
    fn item_name_potato_ids() {
        // 第三種作物 v1：對齊 voxel::Block::PotatoSeeded(50)/PotatoMature(51)
        // + voxel_farm::POTATO_SEEDS_ID(52)/POTATO_ID(53)。
        assert_eq!(item_name_zh(50), "馬鈴薯幼苗");
        assert_eq!(item_name_zh(51), "成熟馬鈴薯");
        assert_eq!(item_name_zh(52), "馬鈴薯種子");
        assert_eq!(item_name_zh(53), "馬鈴薯");
    }

    #[test]
    fn is_food_gift_includes_potato() {
        assert!(is_food_gift(53));  // 馬鈴薯算食物禮物
        assert!(!is_food_gift(52)); // 馬鈴薯種子不算食物禮物
    }

    #[test]
    fn item_name_and_food_gift_include_fish() {
        // 垂釣 v1（ROADMAP 734）：釣竿(60)/小魚(61)/乙太魚(62)。
        assert_eq!(item_name_zh(60), "釣竿");
        assert_eq!(item_name_zh(61), "小魚");
        assert_eq!(item_name_zh(62), "乙太魚");
        assert!(is_food_gift(61), "小魚算食物禮物");
        assert!(is_food_gift(62), "乙太魚算食物禮物");
        assert!(!is_food_gift(60), "釣竿是工具、非食物");
    }

    #[test]
    fn item_name_and_food_gift_include_cooked_fish() {
        // 烤魚 v1：生魚(61)在熔爐烤成烤魚(63)，是居民最愛的美味贈禮。
        assert_eq!(item_name_zh(63), "烤魚");
        assert!(is_food_gift(63), "烤魚算食物禮物");
    }

    #[test]
    fn cooked_fish_thanks_non_empty_no_placeholders() {
        // 所有好感等級、多個 pick 值，不得有未替換的 {name}/{item}，且都提到「魚」。
        for affinity in [0, 1, 2, 3, 5] {
            for pick in 0..4 {
                let s = cooked_fish_thanks_line("旅人", affinity, pick);
                assert!(!s.is_empty(), "affinity={affinity} pick={pick} 回空");
                assert!(!s.contains("{name}"), "affinity={affinity} pick={pick} 未替換 name");
                assert!(!s.contains("{item}"), "affinity={affinity} pick={pick} 出現 item 佔位");
                assert!(s.contains("魚"), "affinity={affinity} pick={pick} 烤魚道謝該提到魚");
            }
        }
    }

    #[test]
    fn cooked_fish_thanks_friend_contains_name() {
        let s = cooked_fish_thanks_line("小星", 5, 0);
        assert!(s.contains("小星"), "友人等級應含玩家名");
    }

    #[test]
    fn item_name_and_food_gift_include_baked_potato() {
        // 烤地薯 v1：生馬鈴薯(53)在熔爐烤成烤地薯(64)，是居民最愛的美味贈禮。
        assert_eq!(item_name_zh(64), "烤地薯");
        assert!(is_food_gift(64), "烤地薯算食物禮物");
    }

    #[test]
    fn baked_potato_thanks_non_empty_no_placeholders() {
        // 所有好感等級、多個 pick 值，不得有未替換的 {name}/{item}，且都提到「地薯」。
        for affinity in [0, 1, 2, 3, 5] {
            for pick in 0..4 {
                let s = baked_potato_thanks_line("旅人", affinity, pick);
                assert!(!s.is_empty(), "affinity={affinity} pick={pick} 回空");
                assert!(!s.contains("{name}"), "affinity={affinity} pick={pick} 未替換 name");
                assert!(!s.contains("{item}"), "affinity={affinity} pick={pick} 出現 item 佔位");
                assert!(s.contains("薯"), "affinity={affinity} pick={pick} 烤地薯道謝該提到薯");
            }
        }
    }

    #[test]
    fn baked_potato_thanks_friend_contains_name() {
        let s = baked_potato_thanks_line("小星", 5, 0);
        assert!(s.contains("小星"), "友人等級應含玩家名");
    }

    // ── 野菜暖湯 v1（ROADMAP 778）───────────────────────────────────────────────

    #[test]
    fn item_name_and_food_gift_include_stew() {
        // 野菜暖湯 v1：胡蘿蔔(49)+馬鈴薯(53)+小麥(18) 在工作台煮成暖湯(67)。
        assert_eq!(item_name_zh(67), "野菜暖湯");
        assert!(is_food_gift(67), "野菜暖湯算食物禮物");
    }

    #[test]
    fn stew_thanks_non_empty_no_placeholders() {
        // 所有好感等級、多個 pick 值，不得有未替換的 {name}/{item}，且都提到「湯」。
        for affinity in [0, 1, 2, 3, 5] {
            for pick in 0..4 {
                let s = stew_thanks_line("旅人", affinity, pick);
                assert!(!s.is_empty(), "affinity={affinity} pick={pick} 回空");
                assert!(!s.contains("{name}"), "affinity={affinity} pick={pick} 未替換 name");
                assert!(!s.contains("{item}"), "affinity={affinity} pick={pick} 出現 item 佔位");
                assert!(s.contains("湯"), "affinity={affinity} pick={pick} 暖湯道謝該提到湯");
            }
        }
    }

    #[test]
    fn stew_thanks_friend_contains_name() {
        let s = stew_thanks_line("小星", 5, 0);
        assert!(s.contains("小星"), "友人等級應含玩家名");
    }

    #[test]
    fn stew_thanks_visitor_has_no_name() {
        // 訪客模式（空名）不得洩出佔位或殘留名字語塊
        let s = stew_thanks_line("", 3, 1);
        assert!(!s.is_empty());
        assert!(!s.contains("{name}"), "空名不得殘留佔位符");
    }

    #[test]
    fn food_gift_thanks_non_empty_no_placeholders() {
        // 所有好感等級、多個 pick 值，不得有未替換的 {name}/{item}。
        for affinity in [0, 1, 2, 3, 5] {
            for pick in 0..4 {
                let s = food_gift_thanks_line("旅人", affinity, pick);
                assert!(!s.is_empty(), "affinity={affinity} pick={pick} 回空");
                assert!(!s.contains("{name}"), "affinity={affinity} pick={pick} 未替換 name");
                assert!(!s.contains("{item}"), "affinity={affinity} pick={pick} 出現 item 佔位");
            }
        }
    }

    #[test]
    fn food_gift_thanks_friend_contains_name() {
        let s = food_gift_thanks_line("小星", 5, 0);
        assert!(s.contains("小星"), "友人等級應含玩家名");
    }

    // ── 雪原冰晶採集 v1 ─────────────────────────────────────────────────────────

    #[test]
    fn is_treasure_gift_only_ice_crystal() {
        assert!(is_treasure_gift(56), "冰晶應為雪原珍寶");
        // 其餘常見禮物都不是珍寶（食物/建材/礦石）。
        for id in [19u8, 49, 53, 8, 9, 10, 20, 21, 54, 55] {
            assert!(!is_treasure_gift(id), "id={id} 不應被判為珍寶");
        }
    }

    #[test]
    fn item_name_ice_crystal() {
        assert_eq!(item_name_zh(56), "冰晶");
        assert_eq!(item_name_zh(54), "仙人掌");
        assert_eq!(item_name_zh(55), "雪");
    }

    #[test]
    fn treasure_gift_thanks_non_empty_no_placeholders() {
        // 所有好感等級、多個 pick、含訪客（空名）都不得留下未替換佔位或回空。
        for name in ["旅人", ""] {
            for affinity in [0, 1, 2, 3, 5] {
                for pick in 0..4 {
                    let s = treasure_gift_thanks_line(name, affinity, pick);
                    assert!(!s.is_empty(), "name={name} affinity={affinity} pick={pick} 回空");
                    assert!(!s.contains("{name}"), "name={name} affinity={affinity} pick={pick} 未替換 name");
                    assert!(s.contains("冰晶"), "珍寶道謝應提到冰晶");
                }
            }
        }
    }

    #[test]
    fn treasure_gift_thanks_friend_contains_name() {
        let s = treasure_gift_thanks_line("小星", 5, 0);
        assert!(s.contains("小星"), "友人等級應含玩家名");
    }

    #[test]
    fn treasure_gift_thanks_stranger_no_name_when_empty() {
        // 訪客（空名）不得把空字串塞進句子留下突兀空缺，且不含 {name}。
        let s = treasure_gift_thanks_line("", 0, 1);
        assert!(!s.contains("{name}"), "訪客句不應有佔位");
        assert!(!s.is_empty());
    }

    // ── 送對禮物 v1（ROADMAP 722）───────────────────────────────────────────────

    #[test]
    fn classify_item_desire_matches_known_keywords() {
        assert_eq!(classify_item_desire("好想要一塊麵包"), Some(19));
        assert_eq!(classify_item_desire("要是有玻璃就好了"), Some(10));
        assert_eq!(classify_item_desire("唉，好想要一張木板做的床呀"), Some(8));
        assert_eq!(classify_item_desire("我想要一把鐵鏟"), Some(41));
        assert_eq!(classify_item_desire("真希望有火把"), Some(31));
        assert_eq!(classify_item_desire("我想要馬鈴薯"), Some(53));
    }

    #[test]
    fn classify_item_desire_none_when_no_keyword() {
        assert!(classify_item_desire("我想蓋一座塔").is_none());
        assert!(classify_item_desire("你好呀，今天天氣真好").is_none());
        assert!(classify_item_desire("").is_none());
    }

    #[test]
    fn classify_item_desire_does_not_overlap_build_keywords() {
        // 確保物品關鍵詞不會誤撞蓋家系統的建造觸發詞（避免同句被兩套系統搶）。
        use crate::voxel_building::classify_desire;
        let item_keywords = [
            "麵包", "鐵鏟", "鏟子", "鐵斧", "斧頭", "鐵鎬", "鎬子", "鐵錠", "鐵磚",
            "玻璃", "火把", "梯子", "箱子", "木門", "木板", "石磚", "胡蘿蔔", "馬鈴薯", "小麥",
        ];
        for kw in item_keywords {
            assert!(
                classify_desire(kw).is_none(),
                "物品關鍵詞「{kw}」不應被蓋家系統誤判為建造心願"
            );
        }
    }

    #[test]
    fn item_wish_thanks_line_contains_item_and_player_within_limit() {
        for name in ["露娜", "諾娃", "賽勒", "奧瑞"] {
            let s = item_wish_thanks_line(name, "麵包", "旅人");
            assert!(s.contains("麵包"), "{name} 台詞應含物品名: {s}");
            assert!(s.contains("旅人"), "{name} 台詞應含玩家名: {s}");
            assert!(s.chars().count() <= 40, "台詞不超過40字：{}", s.chars().count());
            assert!(!s.contains("{p}") && !s.contains("{i}"), "不應留下未替換的佔位符: {s}");
        }
    }

    #[test]
    fn item_wish_memory_contains_item_name() {
        let s = item_wish_memory("玻璃");
        assert!(s.contains("玻璃"));
    }

    #[test]
    fn item_wish_msg_is_valid_json_with_fields() {
        let msg = item_wish_msg("露娜", "麵包", "旅人");
        let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["t"], "item_wish_fulfilled");
        assert_eq!(v["resident"], "露娜");
        assert_eq!(v["item"], "麵包");
        assert_eq!(v["player"], "旅人");
    }
}
