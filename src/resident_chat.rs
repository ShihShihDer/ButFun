//! 居民搭話模板（ROADMAP 118）。
//!
//! 路人居民（見 `resident_npc.rs`）預設零 LLM——本模組提供純模板對話與思想泡泡，
//! 讓居民有生命感、玩家可搭話，完全不花 LLM 額度。
//!
//! 架構：`ResidentPersona` × `ResidentContext` → 靜態字串切片，確定性選取（依 seed 取模）。

use crate::resident_npc::ResidentPersona;
use crate::daynight::Phase;
use crate::weather::WeatherType;

/// 居民思想泡泡 + 對話所需的世界上下文。
#[derive(Debug, Clone)]
pub struct ResidentContext {
    pub phase: Phase,
    pub weather: WeatherType,
}

// ── 思想泡泡模板（短句，顯示於 NpcSpeech 泡泡 5 秒）────────────────────────────

static MARKET_THOUGHTS: &[&str] = &[
    "木材收購價最近不太穩……",
    "薇拉那邊有新貨了嗎？",
    "帶多點資源來賣才划算。",
    "今天的乙太夠買把新鎬子了。",
    "這裡的商機不少呢。",
    "有好貨就不能猶豫。",
    "乙太是好東西，要多存點。",
    "市場熱鬧的時候最開心了。",
];

static MARKET_THOUGHTS_RAIN: &[&str] = &[
    "下雨天東西怕潮，要包好。",
    "雨天攤販少，反而好逛。",
    "雨裡走動，腳底都溼了。",
];

static MARKET_THOUGHTS_NIGHT: &[&str] = &[
    "夜市有種特別的氣息。",
    "天黑了要小心錢袋。",
    "夜晚的市集燈火通明挺好看。",
];

static FARM_THOUGHTS: &[&str] = &[
    "土壤今天感覺不錯。",
    "作物再過一會兒就熟了。",
    "腰有點酸，但看到收成就值了。",
    "雞今天下了幾顆蛋？",
    "農夫這條路雖辛苦，但踏實。",
    "灌水的時機最重要。",
    "有地就有糧，有糧就有乙太。",
    "按時施肥，作物才能長好。",
];

static FARM_THOUGHTS_RAIN: &[&str] = &[
    "下雨剛好，省得澆水了！",
    "雨水潤田，今天不用灑水器。",
    "大自然的恩賜啊。",
];

static FARM_THOUGHTS_NIGHT: &[&str] = &[
    "夜裡還有螢火蟲飛舞。",
    "累了一天，好好休息吧。",
    "明早再來看看作物長多少。",
];

static SQUARE_THOUGHTS: &[&str] = &[
    "廣場上的人氣越來越旺了。",
    "剛聽說有兇名怪物？要小心啊。",
    "這座村子真的越來越有意思。",
    "凱爾長老今天心情好像不錯。",
    "又有新的冒險者加入了呢。",
    "廣場是全村最舒服的地方。",
    "村子的繁榮靠大家一起努力。",
    "公告欄最近貼了新的任務。",
];

static SQUARE_THOUGHTS_RAIN: &[&str] = &[
    "躲到屋簷下，還好不算太濕。",
    "下雨天廣場格外安靜。",
    "雨中的廣場別有一番味道。",
];

static SQUARE_THOUGHTS_NIGHT: &[&str] = &[
    "夜晚的廣場燈光溫柔。",
    "星空很美，捨不得回去睡。",
    "蘭卡說夜裡要注意安全。",
];

static WANDER_THOUGHTS: &[&str] = &[
    "這座村子每個角落都有故事。",
    "今天走到哪兒算哪兒。",
    "到處逛逛，說不定有驚喜。",
    "這裡的人都很有意思。",
    "不知道下一個角落有什麼。",
    "走走走，見識見識世界。",
    "每次走動都有不同的發現。",
    "這座城鎮比我想像的大。",
];

static WANDER_THOUGHTS_RAIN: &[&str] = &[
    "雨中漫步也有種情調。",
    "先找個屋簷躲一下。",
    "溼透了，但心情不錯。",
];

static WANDER_THOUGHTS_NIGHT: &[&str] = &[
    "夜裡城鎮燈光很溫暖。",
    "不知道哪裡有好吃的夜宵。",
    "夜色深了，還是繼續走走。",
];

// ── 工作動態廣播模板（ROADMAP 120）──────────────────────────────────────────

/// 市場攤主工作動態（白天）
static MARKET_WORK_DAY: &[&str] = &[
    "🛒 {name}在市場攤位前整理貨物，叫賣聲此起彼落！",
    "🏪 {name}和顧客討價還價，笑聲不斷。",
    "💰 {name}盤點今日收入，臉上掛著滿意的笑。",
    "🧺 {name}把新到的貨仔細擺放好，準備迎接顧客。",
];

/// 市場攤主工作動態（黎明）
static MARKET_WORK_DAWN: &[&str] = &[
    "🌄 {name}早早來到市場擺攤，是今天第一個開攤的攤主。",
    "☕ {name}喝了口熱茶，準備迎接一天的生意。",
];

/// 農夫工作動態（白天）
static FARM_WORK_DAY: &[&str] = &[
    "🌾 {name}揮著鋤頭翻土，田間傳來規律的勞動聲。",
    "💧 {name}正在為作物澆水，汗珠滴落泥土。",
    "🌱 {name}仔細查看作物長勢，臉上露出滿意的笑容。",
    "🐔 {name}順手餵了雞，順道查看一下蛋籃。",
];

/// 農夫工作動態（黎明）
static FARM_WORK_DAWN: &[&str] = &[
    "🌅 {name}迎著晨曦出門，鋤頭扛在肩上，開始一天的農活。",
    "🌿 {name}在晨霧中播下種子，輕聲哼著歌。",
];

/// 廣場居民工作動態（白天）
static SQUARE_WORK_DAY: &[&str] = &[
    "☕ {name}在廣場老樹下喝茶，和鄰居聊得正起勁。",
    "📋 {name}在公告欄前張望，看看有沒有新任務。",
    "🎵 {name}哼著小曲穿過廣場，招呼著認識的人。",
    "💬 {name}與幾位街坊鄰居圍坐一圈，話家常說得熱鬧。",
];

/// 廣場居民工作動態（黃昏）
static SQUARE_WORK_DUSK: &[&str] = &[
    "🌅 {name}站在廣場看著夕陽，感嘆今天又是美好的一天。",
    "🌇 {name}在黃昏的光暈中閒坐，與旁人分享今日見聞。",
];

/// 遊走居民工作動態（白天）
static WANDER_WORK_DAY: &[&str] = &[
    "🚶 {name}悠閒地在城裡四處走動，笑著跟每個人打招呼。",
    "🔍 {name}東看看西瞧瞧，四處打聽城裡的新鮮事。",
    "📦 {name}幫了某位攤主搬了幾箱貨，換了一小包乾糧。",
];

/// 取得居民工作動態廣播文字（ROADMAP 120）。
///
/// 在對應 persona 的工作時段廣播；夜晚或非工作時段回傳 `None`（不廣播）。
/// `name` 嵌入至文字中；`seed` 供模板輪替。
pub fn get_work_action(persona: ResidentPersona, phase: Phase, name: &str, seed: usize) -> Option<String> {
    let template = match (persona, phase) {
        (ResidentPersona::MarketBrowser, Phase::Day)  => MARKET_WORK_DAY[seed % MARKET_WORK_DAY.len()],
        (ResidentPersona::MarketBrowser, Phase::Dawn) => MARKET_WORK_DAWN[seed % MARKET_WORK_DAWN.len()],
        (ResidentPersona::FarmWorker,    Phase::Day)  => FARM_WORK_DAY[seed % FARM_WORK_DAY.len()],
        (ResidentPersona::FarmWorker,    Phase::Dawn) => FARM_WORK_DAWN[seed % FARM_WORK_DAWN.len()],
        (ResidentPersona::TownSquare,    Phase::Day)  => SQUARE_WORK_DAY[seed % SQUARE_WORK_DAY.len()],
        (ResidentPersona::TownSquare,    Phase::Dusk) => SQUARE_WORK_DUSK[seed % SQUARE_WORK_DUSK.len()],
        (ResidentPersona::Wanderer,      Phase::Day)  => WANDER_WORK_DAY[seed % WANDER_WORK_DAY.len()],
        _ => return None,
    };
    Some(template.replace("{name}", name))
}

// ── 搭話回應模板（稍長，顯示為 NpcReply）─────────────────────────────────────

static MARKET_CHAT: &[&str] = &[
    "你好啊！今天在市場走走，看看有沒有好貨。",
    "薇拉那邊的收購價最近有變嗎？記得多去看看。",
    "在這裡做買賣，要眼明手快。",
    "你是冒險者吧？多採資源來賣，很划算的！",
    "今天人氣不錯，市場熱鬧著呢。",
];

static FARM_CHAT: &[&str] = &[
    "哎呀，農活兒多著呢，忙都忙不完。",
    "你有種田嗎？買塊農田地塊，真的值得！",
    "今天雞蛋收了不少，高興啊。",
    "農夫這條路雖辛苦，但很踏實。",
    "作物要按時澆水，不然長不好啊。",
];

static SQUARE_CHAT: &[&str] = &[
    "歡迎歡迎！這村子最近越來越熱鬧了。",
    "聽說最近怪物橫行？城外要小心哦。",
    "凱爾長老說，只要大家努力，村子就能更好。",
    "廣場是全村最舒服的地方，我每天都來。",
    "你有看最近公告嗎？好多任務可以做！",
];

static WANDER_CHAT: &[&str] = &[
    "到處走走，見識見識，這就是我的生活。",
    "你說奇不奇，每天都有新鮮事發生。",
    "聽說星球上還有更多寶藏，你去過嗎？",
    "這個村子啊，故事多著呢，慢慢聽吧。",
    "今天天氣不錯，適合到處晃晃。",
];

// ── 鄰里打招呼模板（ROADMAP 121）──────────────────────────────────────────────

/// 主動打招呼（帶對方名字）
static GREET_TEMPLATES: &[&str] = &[
    "嘿，{other}！",
    "{other}，你好啊！",
    "哎，{other}，巧了！",
    "{other}，最近怎麼樣？",
    "{other}，這麼巧，碰上了！",
];

/// 對方回應
static REPLY_TEMPLATES: &[&str] = &[
    "嗯，還好！",
    "哈，真巧！",
    "還行啊，你呢？",
    "忙著呢，改天再聊！",
    "好啊好啊！",
];

/// 取得居民向鄰居主動招呼的文字（帶對方名字）。
///
/// `other_name` 為對方居民顯示名；`seed` 供模板輪替。
pub fn get_neighbor_greet(other_name: &str, seed: usize) -> String {
    GREET_TEMPLATES[seed % GREET_TEMPLATES.len()].replace("{other}", other_name)
}

/// 取得居民對招呼的回應文字。
pub fn get_neighbor_reply(seed: usize) -> &'static str {
    REPLY_TEMPLATES[seed % REPLY_TEMPLATES.len()]
}

/// 取得居民的思想泡泡文字。
///
/// `seed` 用居民 index 加上思想計數取模，確保每次展示略有不同。
pub fn get_thought(persona: ResidentPersona, ctx: &ResidentContext, seed: usize) -> &'static str {
    let (day_pool, rain_pool, night_pool): (&[&str], &[&str], &[&str]) = match persona {
        ResidentPersona::MarketBrowser => (MARKET_THOUGHTS, MARKET_THOUGHTS_RAIN, MARKET_THOUGHTS_NIGHT),
        ResidentPersona::FarmWorker    => (FARM_THOUGHTS, FARM_THOUGHTS_RAIN, FARM_THOUGHTS_NIGHT),
        ResidentPersona::TownSquare    => (SQUARE_THOUGHTS, SQUARE_THOUGHTS_RAIN, SQUARE_THOUGHTS_NIGHT),
        ResidentPersona::Wanderer      => (WANDER_THOUGHTS, WANDER_THOUGHTS_RAIN, WANDER_THOUGHTS_NIGHT),
    };
    let pool = match (&ctx.phase, &ctx.weather) {
        (Phase::Night, _) | (Phase::Dusk, _) => night_pool,
        (_, WeatherType::GrasslandRain) => rain_pool,
        _ => day_pool,
    };
    pool[seed % pool.len()]
}

/// 取得居民對玩家搭話的回應文字。
pub fn get_chat(persona: ResidentPersona, seed: usize) -> &'static str {
    let pool: &[&str] = match persona {
        ResidentPersona::MarketBrowser => MARKET_CHAT,
        ResidentPersona::FarmWorker    => FARM_CHAT,
        ResidentPersona::TownSquare    => SQUARE_CHAT,
        ResidentPersona::Wanderer      => WANDER_CHAT,
    };
    pool[seed % pool.len()]
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_day() -> ResidentContext {
        ResidentContext { phase: Phase::Day, weather: WeatherType::Clear }
    }
    fn ctx_night() -> ResidentContext {
        ResidentContext { phase: Phase::Night, weather: WeatherType::Clear }
    }
    fn ctx_rain() -> ResidentContext {
        ResidentContext { phase: Phase::Day, weather: WeatherType::GrasslandRain }
    }

    #[test]
    fn all_personas_return_nonempty_thought_day() {
        let ctx = ctx_day();
        for persona in [
            ResidentPersona::MarketBrowser,
            ResidentPersona::FarmWorker,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let t = get_thought(persona, &ctx, 0);
            assert!(!t.is_empty(), "persona {:?} returned empty thought", persona);
        }
    }

    #[test]
    fn all_personas_return_nonempty_thought_night() {
        let ctx = ctx_night();
        for persona in [
            ResidentPersona::MarketBrowser,
            ResidentPersona::FarmWorker,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let t = get_thought(persona, &ctx, 1);
            assert!(!t.is_empty());
        }
    }

    #[test]
    fn all_personas_return_nonempty_thought_rain() {
        let ctx = ctx_rain();
        for persona in [
            ResidentPersona::MarketBrowser,
            ResidentPersona::FarmWorker,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let t = get_thought(persona, &ctx, 2);
            assert!(!t.is_empty());
        }
    }

    #[test]
    fn thought_seed_wraps_around_without_panic() {
        let ctx = ctx_day();
        let t = get_thought(ResidentPersona::FarmWorker, &ctx, 9999);
        assert!(!t.is_empty());
    }

    #[test]
    fn chat_returns_nonempty_for_all_personas() {
        for (persona, seed) in [
            (ResidentPersona::MarketBrowser, 0usize),
            (ResidentPersona::FarmWorker, 1),
            (ResidentPersona::TownSquare, 2),
            (ResidentPersona::Wanderer, 3),
        ] {
            let r = get_chat(persona, seed);
            assert!(!r.is_empty(), "persona {:?} chat empty", persona);
        }
    }

    #[test]
    fn dusk_uses_night_pool() {
        let ctx = ResidentContext { phase: Phase::Dusk, weather: WeatherType::Clear };
        let t = get_thought(ResidentPersona::TownSquare, &ctx, 0);
        assert!(SQUARE_THOUGHTS_NIGHT.contains(&t), "dusk should use night pool, got: {t}");
    }

    #[test]
    fn chat_seed_wraps_around() {
        let r = get_chat(ResidentPersona::MarketBrowser, 9999);
        assert!(!r.is_empty());
    }

    #[test]
    fn rain_overrides_day_pool() {
        let ctx = ctx_rain();
        let t = get_thought(ResidentPersona::MarketBrowser, &ctx, 0);
        assert!(MARKET_THOUGHTS_RAIN.contains(&t), "rain should use rain pool, got: {t}");
    }

    #[test]
    fn work_action_day_contains_name() {
        for persona in [
            ResidentPersona::MarketBrowser,
            ResidentPersona::FarmWorker,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let result = get_work_action(persona, Phase::Day, "小花", 0);
            assert!(result.is_some(), "Day 時段 {:?} 應有工作廣播", persona);
            assert!(result.unwrap().contains("小花"), "廣播文字應包含姓名");
        }
    }

    #[test]
    fn work_action_dawn_works_for_farm_and_market() {
        for persona in [ResidentPersona::MarketBrowser, ResidentPersona::FarmWorker] {
            let r = get_work_action(persona, Phase::Dawn, "阿土", 0);
            assert!(r.is_some(), "{:?} 黎明應有工作廣播", persona);
        }
        // Wanderer 黎明不廣播
        assert!(get_work_action(ResidentPersona::Wanderer, Phase::Dawn, "阿水", 0).is_none());
    }

    #[test]
    fn work_action_dusk_only_for_square() {
        let r = get_work_action(ResidentPersona::TownSquare, Phase::Dusk, "梅子", 0);
        assert!(r.is_some(), "TownSquare 黃昏應有工作廣播");
        // 其他 persona 黃昏不廣播
        assert!(get_work_action(ResidentPersona::FarmWorker, Phase::Dusk, "老根", 0).is_none());
    }

    #[test]
    fn work_action_night_always_none() {
        for persona in [
            ResidentPersona::MarketBrowser,
            ResidentPersona::FarmWorker,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            assert!(
                get_work_action(persona, Phase::Night, "任何人", 0).is_none(),
                "夜晚 {:?} 不應廣播工作動態",
                persona,
            );
        }
    }

    #[test]
    fn work_action_seed_wraps_without_panic() {
        let r = get_work_action(ResidentPersona::FarmWorker, Phase::Day, "大牛", 9999);
        assert!(r.is_some());
    }

    #[test]
    fn neighbor_greet_contains_other_name() {
        let result = get_neighbor_greet("阿土", 0);
        assert!(result.contains("阿土"), "招呼文字應包含對方名字");
    }

    #[test]
    fn neighbor_greet_seed_wraps_without_panic() {
        let r = get_neighbor_greet("梅子", 9999);
        assert!(!r.is_empty());
    }

    #[test]
    fn neighbor_reply_nonempty_for_all_seeds() {
        for seed in [0, 1, 2, 3, 4, 9999] {
            assert!(!get_neighbor_reply(seed).is_empty());
        }
    }
}
