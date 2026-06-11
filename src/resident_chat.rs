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

// ── 居民隨機小事件模板（ROADMAP 122）──────────────────────────────────────────

/// 農夫隨機小事件（第三人稱敘事）。
static FARM_MINI_EVENTS: &[&str] = &[
    "🪨 {name}在翻土時挖到一塊形狀怪異的石頭，翻來覆去看了半天，最後輕輕放回田埂旁。",
    "🐛 {name}突然跳開一步——原來田裡藏了一隻肥嘟嘟的蟲子。緩了口氣，繼續耕作。",
    "🌿 {name}坐在田埂上擦汗，望著遠處的天邊出神，輕聲說了句「這片土地，真的不容易。」",
];

/// 市場客隨機小事件（第三人稱敘事）。
static MARKET_MINI_EVENTS: &[&str] = &[
    "🛍️ {name}在市場閒逛時停在一個攤位前，看到一件從沒見過的古怪玩意兒，問了半天價，最後搖搖頭走開。",
    "💬 {name}跟攤主聊起最近乙太行情，兩人一問一答，聊得挺起勁，圍觀的路人都聽得入神。",
    "🎁 {name}在市場角落發現有人擺了一捆晾乾的草藥，掂了掂重量，點頭說「不錯！」",
];

/// 廣場居民隨機小事件（第三人稱敘事）。
static SQUARE_MINI_EVENTS: &[&str] = &[
    "☀️ {name}找了塊向陽的大石頭坐下來曬太陽，眯起眼，臉上浮出滿足的微笑。",
    "🕊️ {name}注意到廣場石板縫裡冒出幾株小野花，蹲下來看了許久，沒有拔掉，悄悄站起來走開了。",
    "📣 {name}在廣場公告欄前站了一會兒，瞇眼認真讀著上面的字，邊讀邊點頭，嘴裡嘟噥著什麼。",
];

/// 遊走者隨機小事件（第三人稱敘事）。
static WANDER_MINI_EVENTS: &[&str] = &[
    "🗺️ {name}走到城鎮南邊角落，發現一面長滿青苔的老石牆，摸了摸，嘆了口氣，「這牆比我來得早多了。」",
    "🌙 {name}在小巷轉角停下腳步，仰頭望了望天色，喃喃說：「日子過得真快。」",
    "🔮 {name}在鎮邊偶然看到一道奇特的光影，左右張望了幾秒，最後搖搖頭繼續走，像什麼都沒發生。",
];

/// 取得居民隨機小事件文字（ROADMAP 122）。
///
/// 任何時段皆可廣播；`name` 嵌入文字；`seed` 供模板輪替。
pub fn get_mini_event(persona: ResidentPersona, name: &str, seed: usize) -> String {
    let pool: &[&str] = match persona {
        ResidentPersona::FarmWorker    => FARM_MINI_EVENTS,
        ResidentPersona::MarketBrowser => MARKET_MINI_EVENTS,
        ResidentPersona::TownSquare    => SQUARE_MINI_EVENTS,
        ResidentPersona::Wanderer      => WANDER_MINI_EVENTS,
    };
    pool[seed % pool.len()].replace("{name}", name)
}

// ── 居民主動搭話模板（ROADMAP 123）──────────────────────────────────────────

/// 農夫主動向玩家打招呼（帶居民名字與玩家名字）。
static FARM_PLAYER_GREET: &[&str] = &[
    "🌾 {name}從田埂抬起頭，看到{player}走近，笑著揮手：「{player}，你來了！農活真辛苦，來幫個忙？」",
    "🪣 {name}正在澆水，瞥見{player}，眼睛一亮：「{player}！最近有種田嗎？我這裡作物剛熟，要不要看看？」",
    "🌿 {name}擦了擦手，朝{player}點頭：「{player}，你懂農事嗎？今天的土壤狀態不錯，感謝天公啊。」",
];

/// 市場客主動向玩家打招呼。
static MARKET_PLAYER_GREET: &[&str] = &[
    "🛒 {name}在攤位旁回頭，瞧見{player}，立刻招手：「{player}！過來看看，我剛找到好東西！」",
    "💰 {name}比完價格，抬頭看到{player}笑道：「{player}，你買東西了嗎？薇拉那邊有些不錯的貨。」",
    "🏷️ {name}攔住路過的{player}：「{player}！你知不知道乙太最近漲了？要囤貨的話現在進場！」",
];

/// 廣場閒人主動向玩家打招呼。
static SQUARE_PLAYER_GREET: &[&str] = &[
    "☕ {name}悠哉地坐在石凳上，見到{player}就招手：「{player}！過來坐坐！廣場風景好，急什麼嘛。」",
    "🌸 {name}在廣場閒晃，碰見{player}笑道：「{player}，你也出來逛逛啊？今天人挺多的。」",
    "📖 {name}看到{player}，比了比旁邊的位置：「{player}，凱爾長老剛說了什麼，你有聽到嗎？」",
];

/// 遊走者主動向玩家打招呼。
static WANDER_PLAYER_GREET: &[&str] = &[
    "🧭 {name}從小巷轉出來，一眼看到{player}，點頭道：「{player}，你好啊！最近走了哪條路線？」",
    "🌙 {name}停下腳步望向{player}：「{player}！你不是也愛到處走嗎？城鎮東邊最近有奇怪的光，去看過嗎？」",
    "🗺️ {name}走近{player}低聲說：「{player}，告訴你個秘密——南邊有個角落沒人注意到，特別有趣。」",
];

/// 取得居民主動向玩家打招呼的文字（ROADMAP 123）。
///
/// `resident_name` 為居民顯示名；`player_name` 為玩家名；`seed` 供模板輪替。
pub fn get_player_greeting(persona: ResidentPersona, resident_name: &str, player_name: &str, seed: usize) -> String {
    let pool: &[&str] = match persona {
        ResidentPersona::FarmWorker    => FARM_PLAYER_GREET,
        ResidentPersona::MarketBrowser => MARKET_PLAYER_GREET,
        ResidentPersona::TownSquare    => SQUARE_PLAYER_GREET,
        ResidentPersona::Wanderer      => WANDER_PLAYER_GREET,
    };
    pool[seed % pool.len()]
        .replace("{name}", resident_name)
        .replace("{player}", player_name)
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

    // ── ROADMAP 122 小事件測試 ────────────────────────────────────────────────

    #[test]
    fn mini_event_all_personas_contain_name() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_mini_event(persona, "小花", 0);
            assert!(text.contains("小花"), "persona {:?} 小事件應包含姓名，got: {text}", persona);
        }
    }

    #[test]
    fn mini_event_all_personas_nonempty() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_mini_event(persona, "阿土", 0);
            assert!(!text.is_empty(), "persona {:?} 小事件不應為空", persona);
        }
    }

    #[test]
    fn mini_event_seed_wraps_without_panic() {
        // seed 超出模板長度應以取模回捲，不 panic。
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let _ = get_mini_event(persona, "二柱", 9999);
        }
    }

    #[test]
    fn mini_event_different_seeds_produce_variety() {
        // 同 persona 不同 seed 至少能產出 ≥2 種不同文字（模板數 ≥ 3）。
        let texts: Vec<_> = (0..3).map(|s| get_mini_event(ResidentPersona::FarmWorker, "梅子", s)).collect();
        let unique: std::collections::HashSet<_> = texts.iter().collect();
        assert!(unique.len() >= 2, "FarmWorker 小事件至少應有 2 種不同模板");
    }

    #[test]
    fn mini_event_name_substitution_works() {
        let text = get_mini_event(ResidentPersona::Wanderer, "老根", 1);
        assert!(text.contains("老根"), "應正確替換姓名 '老根'");
        assert!(!text.contains("{name}"), "模板佔位符應已被替換");
    }

    #[test]
    fn mini_event_market_seed_1_contains_name() {
        let text = get_mini_event(ResidentPersona::MarketBrowser, "春花", 1);
        assert!(text.contains("春花"));
    }

    #[test]
    fn mini_event_square_seed_2_nonempty() {
        let text = get_mini_event(ResidentPersona::TownSquare, "阿水", 2);
        assert!(!text.is_empty());
    }

    #[test]
    fn mini_event_all_templates_contain_placeholder_filled() {
        // 確保所有模板都有 {name} 並正確被替換。
        for pool in [FARM_MINI_EVENTS, MARKET_MINI_EVENTS, SQUARE_MINI_EVENTS, WANDER_MINI_EVENTS] {
            for template in pool {
                assert!(template.contains("{name}"), "模板應含 {{name}} 佔位符: {template}");
                let filled = template.replace("{name}", "測試名");
                assert!(!filled.contains("{name}"), "替換後不應殘留 {{name}}: {filled}");
            }
        }
    }

    // ── ROADMAP 123 玩家搭話測試 ──────────────────────────────────────────────

    #[test]
    fn player_greeting_all_personas_nonempty() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_player_greeting(persona, "阿土", "冒險者甲", 0);
            assert!(!text.is_empty(), "persona {:?} 打招呼不應為空", persona);
        }
    }

    #[test]
    fn player_greeting_contains_both_names() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_player_greeting(persona, "小花", "英雄乙", 0);
            assert!(text.contains("小花"), "persona {:?} 應含居民名 '小花'：{text}", persona);
            assert!(text.contains("英雄乙"), "persona {:?} 應含玩家名 '英雄乙'：{text}", persona);
        }
    }

    #[test]
    fn player_greeting_no_leftover_placeholders() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            for seed in 0..3 {
                let text = get_player_greeting(persona, "居民名", "玩家名", seed);
                assert!(!text.contains("{name}"), "不應殘留 {{name}}：{text}");
                assert!(!text.contains("{player}"), "不應殘留 {{player}}：{text}");
            }
        }
    }

    #[test]
    fn player_greeting_seed_wraps_without_panic() {
        let _ = get_player_greeting(ResidentPersona::Wanderer, "老根", "測試玩家", 9999);
    }

    #[test]
    fn player_greeting_different_seeds_produce_variety() {
        // 同 persona 不同 seed 至少有 2 種不同模板（每個 persona 有 3 條）。
        let texts: Vec<_> = (0..3)
            .map(|s| get_player_greeting(ResidentPersona::FarmWorker, "小麥", "玩家X", s))
            .collect();
        let unique: std::collections::HashSet<_> = texts.iter().collect();
        assert!(unique.len() >= 2, "FarmWorker 打招呼至少應有 2 種不同模板");
    }

    #[test]
    fn player_greeting_all_templates_check() {
        // 確保所有 persona 的所有 3 條模板都含兩個佔位符且可正常替換。
        for pool in [FARM_PLAYER_GREET, MARKET_PLAYER_GREET, SQUARE_PLAYER_GREET, WANDER_PLAYER_GREET] {
            for template in pool {
                assert!(template.contains("{name}"), "模板應含 {{name}}：{template}");
                assert!(template.contains("{player}"), "模板應含 {{player}}：{template}");
                let filled = template.replace("{name}", "A").replace("{player}", "B");
                assert!(!filled.contains('{'), "替換後不應殘留佔位符：{filled}");
            }
        }
    }
}
