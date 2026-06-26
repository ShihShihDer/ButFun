//! 居民搭話模板（ROADMAP 118）。
//!
//! 路人居民（見 `resident_npc.rs`）預設零 LLM——本模組提供純模板對話與思想泡泡，
//! 讓居民有生命感、玩家可搭話，完全不花 LLM 額度。
//!
//! 架構：`ResidentPersona` × `ResidentContext` → 靜態字串切片，確定性選取（依 seed 取模）。

use crate::resident_npc::ResidentPersona;
use crate::daynight::Phase;
use crate::weather::WeatherType;
use crate::season::Season;

/// 居民思想泡泡 + 對話所需的世界上下文。
#[derive(Debug, Clone)]
pub struct ResidentContext {
    pub phase: Phase,
    pub weather: WeatherType,
}

/// 玩家攀談時，居民話語所反映的「此刻城鎮」上下文（ROADMAP 360）。
///
/// 與 `ResidentContext`（只供思想泡泡，僅看時段／天氣）刻意分開：攀談是玩家**主動**
/// 觸發的互動，理應讓居民把當下整個世界（季節／天氣／繁榮／生態警戒）都納進嘴邊——
/// 讓滿城路人從「千篇一律的罐頭」變成「活在當下世界裡的人」。純資料、零 LLM。
#[derive(Debug, Clone, Copy)]
pub struct TownTalkContext {
    /// 目前日夜時段。
    pub phase: Phase,
    /// 目前天氣。
    pub weather: WeatherType,
    /// 目前季節。
    pub season: Season,
    /// 城鎮繁榮等級（沿用 `ResidentManager::prosperity_level`）。
    pub prosperity_level: u8,
    /// 是否正逢生態警戒（生態壓力過高、怪物逼近城鎮）。
    pub eco_alarmed: bool,
}

/// 生態壓力達此值（0～100）即視為「生態警戒」，居民攀談時會流露不安。
/// 刻意對齊 ROADMAP 172 清剿委託的發布門檻（≥60），讓「居民開始擔心」與
/// 「全服清剿委託發布」是同一道生態紅線、世界感受一致。
pub const ECO_ALARM_PRESSURE: f32 = 60.0;

/// 繁榮等級達此值即視為「興旺」，居民攀談時偶爾流露對城鎮的自豪。
pub const PROSPERITY_PROUD_LEVEL: u8 = 3;

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

// ── 鄰里熟識度招呼模板（ROADMAP 557） ──────────────────────────────────────────
// 反覆碰上的同一對居民會「處出交情」，招呼依熟識階層升級：
// 點頭之交＝叫得出名字、語氣漸熟；老鄰居＝見面格外親、嘮得更深。

/// 點頭之交向對方招呼（帶對方名字，語氣比初識熟一些）
static GREET_ACQUAINTANCE: &[&str] = &[
    "{other}，又碰上啦！",
    "嘿{other}，最近常見到你呢。",
    "{other}，這幾天還順心吧？",
    "又見面了，{other}！",
    "{other}，老照面囉，哈哈。",
];

/// 老鄰居向對方招呼（帶對方名字，見面格外親）
static GREET_FRIEND: &[&str] = &[
    "🤝 {other}！老地方又碰上啦，今天可好？",
    "🤝 哎呀{other}，就愛跟你嘮兩句。",
    "🤝 {other}，老鄰居，過來坐坐！",
    "🤝 可算遇上你了，{other}，最近怎樣？",
    "🤝 {other}，見著你就高興，走兩步一塊兒？",
];

/// 點頭之交的回應
static REPLY_ACQUAINTANCE: &[&str] = &[
    "是啊，挺巧的！",
    "你也常在這兒呢，哈。",
    "還順，謝你問候！",
    "嗯，碰著你就親切！",
    "彼此彼此，回頭聊！",
];

/// 老鄰居的回應
static REPLY_FRIEND: &[&str] = &[
    "🤝 可不是嘛，跟你說話最舒坦了！",
    "🤝 哈，老鄰居就是不一樣！",
    "🤝 走啊，一塊兒溜達溜達！",
    "🤝 有你這鄰居，日子都暖些。",
    "🤝 好嘞，回頭上你那兒坐！",
];

// ── 老鄰居結伴同行招呼（ROADMAP 558）─────────────────────────────────────────────
/// 老鄰居起意結伴時、領路者向對方發出的邀約（帶對方名字）。
static STROLL_INVITE: &[&str] = &[
    "🚶 {other}，難得碰上，陪我繞村子走兩圈唄？",
    "🚶 走，{other}，咱倆一塊兒溜達溜達！",
    "🚶 {other}，正悶得慌，跟我串串門子去？",
    "🚶 來嘛{other}，老地方走一遭，邊走邊嘮！",
];

/// 被邀約的老鄰居應約一起走。
static STROLL_ACCEPT: &[&str] = &[
    "🚶 成啊，走著！",
    "🚶 好咧，正想活動活動腿腳！",
    "🚶 跟你走哪兒都樂意～",
    "🚶 那還等啥，前頭帶路！",
];

// ── 老鄰居鬧彆扭與和好（ROADMAP 559）─────────────────────────────────────────────
/// 老鄰居偶爾拌嘴鬧彆扭時、先開口那位的話（帶對方名字，語氣是熟人間的小慪氣、非惡意）。
static TIFF_OPEN: &[&str] = &[
    "😤 {other}，上回那事我可還記著呢，哼！",
    "😤 跟你說過多少遍了{other}，怎麼又這樣！",
    "😤 哼，{other}，今兒個我不想搭理你。",
    "😤 {other}你呀你，真是叫人沒法子。",
];

/// 鬧彆扭時、被慪那位的回嘴（一樣是老鄰居間的小彆扭，扭頭就走）。
static TIFF_REPLY: &[&str] = &[
    "😤 哼，不理就不理，誰稀罕！",
    "😤 你才是呢，氣死我了！",
    "😤 算了算了，各走各的！",
    "😤 好好好，你最有理，行了吧！",
];

/// 鬧過彆扭的老鄰居再碰上、主動和好那位的話（帶對方名字，台階遞出去）。
static MAKEUP_OPEN: &[&str] = &[
    "😌 {other}，前兒個是我話重了，別往心裡去哈。",
    "😌 哎{other}，老鄰居哪有隔夜仇，來握個手？",
    "😌 {other}，氣也消了，還是你這老夥計處得來。",
    "😌 別彆扭啦{other}，我給你賠個不是。",
];

/// 和好時、另一位順著台階下的回應（重修舊好、反而更親）。
static MAKEUP_REPLY: &[&str] = &[
    "🤝 嗨，我也沒真生氣，過去就過去了！",
    "🤝 早該和好啦，走，喝茶去！",
    "🤝 還是你敞亮，這事兒翻篇兒！",
    "🤝 老鄰居嘛，哪能為這點事傷和氣。",
];

// ── 主要 NPC 白天招呼模板（ROADMAP 244） ───────────────────────────────────────────

/// 主要 NPC 向路過居民主動招呼（帶居民名字）
static MAJOR_GREET_MERCHANT: &[&str] = &[
    "嘿，{other}！今天市場熱鬧，不去逛逛嗎？",
    "喲，{other}！剛進了批好貨，有空來看看！",
    "{other}，最近生意做得怎麼樣？",
];

static MAJOR_GREET_CHIEF: &[&str] = &[
    "{other}，今天村子氣氛不錯，辛苦了！",
    "看到大家都在努力，我這心裡就踏實。{other}，好樣的！",
    "嘿，{other}，最近生活上還有什麼需要幫忙的嗎？",
];

static MAJOR_GREET_WORKSHOP: &[&str] = &[
    "嗨，{other}！這工具用的還順手吧？不順隨時拿來修！",
    "看這腳步，今天幹勁十足啊，{other}！",
    "{other}，聽說你最近又採了不少好料？",
];

static MAJOR_GREET_TRAVELER: &[&str] = &[
    "你好啊，{other}！我是外地來的旅人，這兒風景真不錯。",
    "嘿，{other}！這城鎮比我想像中還有朝氣呢。",
    "打擾了，{other}，請問這附近有什麼有趣的故事嗎？",
];

static MAJOR_GREET_GENERIC: &[&str] = &[
    "嘿，{other}！忙著呢？",
    "喲，{other}，今天天氣真好！",
    "{other}，看到你真高興！",
];

/// 居民對主要 NPC 招呼的回應
static MAJOR_REPLY_TEMPLATES: &[&str] = &[
    "您好！今天確實不錯。",
    "嘿，剛好路過，您忙！",
    "是啊，日子越來越有盼頭了。",
    "哈哈，承您吉言！",
    "您太客氣了，回頭聊！",
];

/// 取得主要 NPC 向居民主動招呼的文字（帶居民名字）。
///
/// `major_id` 用於區分主要 NPC 身分；`other_name` 為居民顯示名；`seed` 供模板輪替。
pub fn get_major_npc_greet(major_id: &str, other_name: &str, seed: usize) -> String {
    let pool = match major_id {
        "merchant" => MAJOR_GREET_MERCHANT,
        "village_chief" => MAJOR_GREET_CHIEF,
        "workshop_npc" => MAJOR_GREET_WORKSHOP,
        id if id.starts_with("traveler") => MAJOR_GREET_TRAVELER,
        _ => MAJOR_GREET_GENERIC,
    };
    pool[seed % pool.len()].replace("{other}", other_name)
}

// ── 動態傳聞與八卦模板（ROADMAP 244 動態話題層） ───────────────────────────────────────────

/// 世界大事傳聞模板（帶大事內容與居民名）
static WORLD_GOSSIP_TEMPLATES: &[&str] = &[
    "嘿，{other}！你聽說了嗎？{event}，這消息傳得可真快！",
    "{other}，剛才城裡都在傳「{event}」，看來最近不安穩啊。",
    "喲，{other}！「{event}」這事兒，你怎麼看？",
    "剛剛聽人說起「{event}」，這世界變化真快，是吧，{other}？",
];

/// 社交八卦模板（帶對方名字與關係描述）
static SOCIAL_GOSSIP_TEMPLATES: &[&str] = &[
    "說起來，{other}，我覺得最近{target}對大家真是{desc}，挺有意思的。",
    "{other}，你有沒有覺得{target}最近有點變化？看他那樣，感覺是{desc}。",
    "嘿，{other}，私下跟你說，我對{target}可是{desc}，別傳出去啊！",
];

/// 取得動態主要 NPC 招呼文字（ROADMAP 244 動態話題層）。
///
/// 優先級：世界大事（WorldLog） > 社交八卦（NpcRelations） > 常規招呼（244）。
pub fn get_dynamic_major_npc_greet(
    major_id: &str,
    other_name: &str,
    seed: usize,
    world_events: &[String],
    relations: &[(String, String, i32)], // (target_name, affinity_desc, affinity_score)
) -> String {
    // 1. 優先聊世界大事（若最近有事）
    if !world_events.is_empty() {
        let event = &world_events[seed % world_events.len()];
        let template = WORLD_GOSSIP_TEMPLATES[seed % WORLD_GOSSIP_TEMPLATES.len()];
        return template.replace("{other}", other_name).replace("{event}", event);
    }

    // 2. 其次聊社交八卦（若有顯著關係）
    if !relations.is_empty() {
        let (target_name, desc, _) = &relations[seed % relations.len()];
        let template = SOCIAL_GOSSIP_TEMPLATES[seed % SOCIAL_GOSSIP_TEMPLATES.len()];
        return template
            .replace("{other}", other_name)
            .replace("{target}", target_name)
            .replace("{desc}", desc);
    }

    // 3. 最後退回常規招呼
    get_major_npc_greet(major_id, other_name, seed)
}

/// 取得居民對主要 NPC 招呼的回應文字。
pub fn get_major_npc_reply(seed: usize) -> &'static str {
    MAJOR_REPLY_TEMPLATES[seed % MAJOR_REPLY_TEMPLATES.len()]
}

/// 依鄰里熟識階層取得居民向鄰居招呼的文字（ROADMAP 557）。
///
/// 老鄰居／點頭之交叫得出名字、語氣更親；陌生則退回既有泛泛招呼。
pub fn get_neighbor_greet_tiered(
    other_name: &str,
    tier: crate::resident_bonds::NeighborTier,
    seed: usize,
) -> String {
    use crate::resident_bonds::NeighborTier::*;
    let pool: &[&str] = match tier {
        Friend => GREET_FRIEND,
        Acquaintance => GREET_ACQUAINTANCE,
        Stranger => GREET_TEMPLATES,
    };
    pool[seed % pool.len()].replace("{other}", other_name)
}

/// 老鄰居結伴同行時、領路者向對方發出的邀約文字（ROADMAP 558）。
pub fn get_neighbor_stroll_invite(other_name: &str, seed: usize) -> String {
    STROLL_INVITE[seed % STROLL_INVITE.len()].replace("{other}", other_name)
}

/// 老鄰居結伴同行時、被邀者的應約文字（ROADMAP 558）。
pub fn get_neighbor_stroll_accept(seed: usize) -> &'static str {
    STROLL_ACCEPT[seed % STROLL_ACCEPT.len()]
}

/// 老鄰居鬧彆扭時、先開口那位的話（帶對方名字）（ROADMAP 559）。
pub fn get_neighbor_tiff_open(other_name: &str, seed: usize) -> String {
    TIFF_OPEN[seed % TIFF_OPEN.len()].replace("{other}", other_name)
}

/// 鬧彆扭時、被慪那位的回嘴（ROADMAP 559）。
pub fn get_neighbor_tiff_reply(seed: usize) -> &'static str {
    TIFF_REPLY[seed % TIFF_REPLY.len()]
}

/// 鬧過彆扭後再碰上、主動和好那位的話（帶對方名字）（ROADMAP 559）。
pub fn get_neighbor_makeup_open(other_name: &str, seed: usize) -> String {
    MAKEUP_OPEN[seed % MAKEUP_OPEN.len()].replace("{other}", other_name)
}

/// 和好時、順著台階下的回應（ROADMAP 559）。
pub fn get_neighbor_makeup_reply(seed: usize) -> &'static str {
    MAKEUP_REPLY[seed % MAKEUP_REPLY.len()]
}

/// 依鄰里熟識階層取得居民對招呼的回應文字（ROADMAP 557）。
pub fn get_neighbor_reply_tiered(
    tier: crate::resident_bonds::NeighborTier,
    seed: usize,
) -> &'static str {
    use crate::resident_bonds::NeighborTier::*;
    let pool: &[&str] = match tier {
        Friend => REPLY_FRIEND,
        Acquaintance => REPLY_ACQUAINTANCE,
        Stranger => REPLY_TEMPLATES,
    };
    pool[seed % pool.len()]
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

// ── 凱旋餘韻談資模板（ROADMAP 186）──────────────────────────────────────────
/// 歡慶（185）散場後的「餘韻期」裡，居民興奮談論剛被討伐的菁英首領。
/// 不分 persona（全城都在聊同一件大事）、第一人稱口語，沿用既有思想泡泡泡泡層渲染。
/// 面向玩家字串，將來在地化時集中替換。
static TRIUMPH_THOUGHTS: &[&str] = &[
    "聽說有人把那隻菁英首領給討伐了，真是英雄！",
    "城外那頭怪物王終於倒下了，今晚能睡個好覺了。",
    "我親眼看到牠頭頂的赤環滅了……太痛快了！",
    "孩子們都在學那位勇者揮劍的樣子呢，哈哈。",
    "首領一倒，野外的怪物應該會安分一陣子吧？",
    "得替英雄備杯熱茶，凱旋歸來總得犒賞一下。",
    "方才那聲歡呼，整座城都聽見了吧！",
    "我還在發抖呢，沒想到真有人能撂倒那種龐然大物。",
    "廣場上大家都在傳頌這場勝仗，氣氛真好。",
    "牠盤踞那麼久，今天總算有人替我們出了口氣。",
];

/// 取得凱旋餘韻談資（ROADMAP 186）：確定性依 seed 取模，必非空。
pub fn get_triumph_thought(seed: usize) -> &'static str {
    TRIUMPH_THOUGHTS[seed % TRIUMPH_THOUGHTS.len()]
}

// ── 凱旋英雄禮讚模板（ROADMAP 188）──────────────────────────────────────────
/// 餘韻期間（186），討伐菁英的英雄玩家本人走進城裡、靠近某位居民時，
/// 該居民停步轉身、對英雄本人專屬道謝（頭頂 🙏）——居民第一次認得「特定玩家的戰功」。
/// 不分 persona（全城都認得這位英雄）、帶居民名 {name} 與英雄名 {player}。
/// 面向玩家字串，將來在地化時集中替換。
static HERO_GRATITUDE: &[&str] = &[
    "🙏 {name}停下腳步向{player}深深一鞠躬：「{player}！討伐菁英首領的英雄就是你吧？真是太感謝了！」",
    "🙏 {name}快步迎上{player}：「就是你斬下那頭怪物王的！城裡上下都念著你的好呢。」",
    "🙏 {name}紅著眼眶握住{player}的手：「{player}，有你在，我們晚上才敢安心點燈——謝謝你。」",
    "🙏 {name}朝{player}豎起大拇指：「方才那場惡戰我都看見了！{player}，你是全城的英雄！」",
    "🙏 {name}從攤上抓了把果子塞給{player}：「英雄不嫌棄就收下吧，這是我們的一點心意。」",
    "🙏 {name}向{player}恭敬行禮：「首領一倒，野外總算太平了，這份恩情我們記著呢。」",
];

/// 取得凱旋英雄禮讚文字（ROADMAP 188）：確定性依 seed 取模，必非空。
///
/// `resident_name` 為居民顯示名；`hero_name` 為英雄玩家名；`seed` 供模板輪替。
pub fn get_hero_gratitude(resident_name: &str, hero_name: &str, seed: usize) -> String {
    HERO_GRATITUDE[seed % HERO_GRATITUDE.len()]
        .replace("{name}", resident_name)
        .replace("{player}", hero_name)
}

/// persona 本色的搭話池（不分世界情境的基底）。
fn persona_chat(persona: ResidentPersona) -> &'static [&'static str] {
    match persona {
        ResidentPersona::MarketBrowser => MARKET_CHAT,
        ResidentPersona::FarmWorker    => FARM_CHAT,
        ResidentPersona::TownSquare    => SQUARE_CHAT,
        ResidentPersona::Wanderer      => WANDER_CHAT,
    }
}

// ── 此刻城鎮搭話池（ROADMAP 360）─────────────────────────────────────────────
// 全城共感、不分 persona：季節／天氣／生態警戒／繁榮是「整座城鎮共同的當下」，
// 由誰來搭話都會聊到。第一人稱口語，沿用 NpcReply/NpcSpeech 泡泡層渲染。
// 面向玩家字串，將來在地化時集中替換。

/// 生態警戒：怪物逼近、生態壓力高漲時，居民流露不安（壓過一切閒聊，這是當下最要緊的事）。
static ECO_ALARM_CHAT: &[&str] = &[
    "你也感覺到了吧？城外那股不安的氣息……最近怪物多得嚇人。",
    "聽說生態亂了套，掠食者都往城邊靠了，出門可得當心。",
    "蘭卡正忙著張羅清剿的事呢，這陣子真不太平。",
    "夜裡都不大敢往城外走了，怪物的動靜越來越大。",
    "你是冒險者吧？要是能幫忙壓一壓城外那些怪物，全城都會謝你的。",
];

/// 雨天（草原細雨）。
static RAIN_CHAT: &[&str] = &[
    "這雨下得正好，田裡的作物都樂壞了吧。",
    "下雨天嘛，慢慢走、別滑著了。",
    "我躲在屋簷下跟你聊兩句，雨一停就接著忙。",
];

/// 沙塵 / 晶塵 / 海霧等惡劣天氣（非晴非雨）：眯眼、掩面、行色匆匆。
static HARSH_WEATHER_CHAT: &[&str] = &[
    "這天氣真折騰人，眯著眼都快看不清路了。",
    "風沙這麼大，咱們長話短說吧，回頭天晴了再好好聊。",
    "這種天最好待在屋裡，你還在外頭跑，辛苦啦。",
];

/// 夜晚 / 黃昏：燈火、歸途、夜談。
static NIGHT_CHAT: &[&str] = &[
    "這麼晚還在外頭啊？小心腳下，早點回去歇著。",
    "夜裡的城鎮安靜得很，跟你說兩句話倒也愜意。",
    "燈一盞盞亮起來了，我也該往家走了。",
];

/// 春季氛圍。
static SPRING_CHAT: &[&str] = &[
    "春天來了，到處都冒出新芽，看著就有精神。",
    "這個時節最適合下種了，農夫們都忙開了呢。",
    "春風一吹，整座城鎮都活了過來似的。",
];

/// 夏季氛圍。
static SUMMER_CHAT: &[&str] = &[
    "這大熱天的，記得多喝口水啊。",
    "夏天日頭毒，我都挑樹蔭底下走。",
    "天熱歸熱，作物倒是長得飛快。",
];

/// 秋季氛圍。
static AUTUMN_CHAT: &[&str] = &[
    "秋收的時節到了，倉裡的乙太總算寬裕些了。",
    "你看這滿城的金黃，秋天真是最美的時候。",
    "天涼了，添件衣裳吧，別著了風。",
];

/// 冬季氛圍。
static WINTER_CHAT: &[&str] = &[
    "這天寒地凍的，你還在外頭跑，真不容易。",
    "冬天田裡歇著了，大夥兒就圍著爐火聊天打發日子。",
    "下回見著我，記得我說過——冬天要把自己裹暖和了。",
];

/// 城鎮興旺時的自豪。
static PROSPERITY_PROUD_CHAT: &[&str] = &[
    "你瞧這城鎮多熱鬧！這些年總算興旺起來了。",
    "人氣越來越旺，新面孔也越來越多，住在這兒真好。",
    "大夥兒齊心，這座城鎮一天比一天有模有樣了。",
];

/// 季節對應的氛圍搭話池。
fn season_chat(season: Season) -> &'static [&'static str] {
    match season {
        Season::Spring => SPRING_CHAT,
        Season::Summer => SUMMER_CHAT,
        Season::Autumn => AUTUMN_CHAT,
        Season::Winter => WINTER_CHAT,
    }
}

/// 非晴朗天氣對應的搭話池；晴天回 `None`（交由季節／繁榮／persona 接手）。
fn weather_chat(weather: WeatherType) -> Option<&'static [&'static str]> {
    match weather {
        WeatherType::Clear => None,
        WeatherType::GrasslandRain => Some(RAIN_CHAT),
        // 沙塵 / 晶塵 / 海霧皆屬「惡劣天氣」，共用一組掩面匆匆的話。
        _ => Some(HARSH_WEATHER_CHAT),
    }
}

/// 取得居民對玩家搭話的回應文字（ROADMAP 360：反映此刻城鎮）。
///
/// 確定性依 `seed` 取模選句。挑句優先序＝「居民當下最掛心的事」：
/// 生態警戒（怪物逼近）＞ 惡劣天氣／雨 ＞ 入夜 ＞ 晴朗白天時，季節氛圍與 persona 本色
/// 交替（並在城鎮興旺時偶爾流露自豪）。如此季節／天氣／生態／繁榮每一條世界線都會在
/// 居民嘴邊現身，而 persona 本色（既有 MARKET/FARM/SQUARE/WANDER_CHAT）仍保留約半數的
/// 晴日攀談，不被淹沒。純函式、零 LLM、確定可測。
pub fn get_chat(persona: ResidentPersona, ctx: &TownTalkContext, seed: usize) -> &'static str {
    // 1. 生態警戒壓過一切——怪物逼近時，沒人有心情閒聊天氣。
    if ctx.eco_alarmed {
        return ECO_ALARM_CHAT[seed % ECO_ALARM_CHAT.len()];
    }
    // 2. 惡劣天氣／雨——最直接的當下體感。
    if let Some(pool) = weather_chat(ctx.weather) {
        return pool[seed % pool.len()];
    }
    // 3. 入夜（夜／黃昏）——歸途與燈火。
    if matches!(ctx.phase, Phase::Night | Phase::Dusk) {
        return NIGHT_CHAT[seed % NIGHT_CHAT.len()];
    }
    // 4. 晴朗白天：城鎮興旺時偶爾自豪一句（約 1/4）。
    if ctx.prosperity_level >= PROSPERITY_PROUD_LEVEL && seed % 4 == 0 {
        return PROSPERITY_PROUD_CHAT[seed % PROSPERITY_PROUD_CHAT.len()];
    }
    // 5. 其餘晴日：季節氛圍與 persona 本色交替（各約半數），兩種味道都嚐得到。
    if seed % 2 == 0 {
        let pool = season_chat(ctx.season);
        return pool[seed % pool.len()];
    }
    let pool = persona_chat(persona);
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

// ── 互助請求模板（ROADMAP 125）──────────────────────────────────────────────

/// 農夫求助語（第一人稱，帶居民名）。
static FARM_HELP_REQUEST: &[&str] = &[
    "🪣 {name}站在田邊皺著眉頭：「我的水桶裂了，誰能幫我找找看？」",
    "🌾 {name}抓著鋤頭喊道：「最近作物長不好，哪位好心人能陪我看看田地？」",
    "🐔 {name}急著張望說：「我的雞跑不見了！誰有沒有看到一隻橘紅色的雞？」",
];

/// 市場客求助語。
static MARKET_HELP_REQUEST: &[&str] = &[
    "🛒 {name}左右張望說：「我好像把錢袋落在市場這邊，誰幫我找一下？」",
    "📦 {name}苦著臉說：「這箱貨太重了，有沒有好心人幫我推一段路？」",
    "💬 {name}問周圍人：「今天的集市在哪裡開啊？我找了半天都摸不著頭緒。」",
];

/// 廣場居民求助語。
static SQUARE_HELP_REQUEST: &[&str] = &[
    "☕ {name}招招手說：「哎，有沒有人陪我聊幾句？今天心裡有點悶，想說說話。」",
    "📋 {name}比著公告欄說：「公告欄的字太小了，誰幫我念一下上面寫什麼？」",
    "🎵 {name}笑著說：「誰能教我哼那個最近流行的曲子？聽過但就是記不住旋律。」",
];

/// 遊走者求助語。
static WANDER_HELP_REQUEST: &[&str] = &[
    "🗺️ {name}拿著一張破舊地圖說：「這條路是不是繞遠了？誰知道怎麼走最快？」",
    "🔍 {name}苦著臉說：「我好像把東西落在城南角落，有沒有人跟我去找一下？」",
    "🌙 {name}小聲說：「聽說城外有個奇怪的地方，誰能陪我去探探？一個人怕怕的。」",
];

/// 農夫感謝語（帶居民名與玩家名）。
static FARM_HELP_THANKS: &[&str] = &[
    "🌾 {name}感激地笑著：「謝謝你，{player}！你真是個熱心人，這點乙太表示我的謝意！」",
    "💧 {name}抹了把汗說：「{player}！多虧你幫忙，這裡有點乙太，你別嫌棄啊！」",
    "🪴 {name}用力點頭：「{player}，太感謝了！田裡的事最怕麻煩人，你真好！」",
];

/// 市場客感謝語。
static MARKET_HELP_THANKS: &[&str] = &[
    "🛒 {name}大笑說：「{player}，你救了我！這點乙太是我的感謝，下次碰到好事告訴我！」",
    "💰 {name}掏出一把乙太：「{player}！做生意最重要是人情，這是給你的，下次再麻煩囉！」",
    "📦 {name}拍拍手：「{player}！真的太謝謝了！我不善言詞，就這點乙太心意！」",
];

/// 廣場居民感謝語。
static SQUARE_HELP_THANKS: &[&str] = &[
    "☕ {name}笑瞇瞇說：「{player}，聊了幾句感覺好多了！這點乙太謝謝你陪我！」",
    "🎵 {name}開心地說：「{player}！就是你這樣熱心的人讓村子溫暖，乙太拿去買點好東西！」",
    "🌸 {name}感動地點頭：「{player}，謝謝你願意停下來幫忙，這是我小小的心意！」",
];

/// 遊走者感謝語。
static WANDER_HELP_THANKS: &[&str] = &[
    "🗺️ {name}把乙太塞到{player}手裡：「{player}！多謝你願意陪我，這是旅途存下來的一點。」",
    "🔍 {name}笑著說：「{player}，你真是旅者同好！這點乙太當見面禮，下次再一起探索！」",
    "🌙 {name}低聲說：「{player}，謝謝你……城裡的人都很忙，你肯停下來真好。」",
];

/// 取得居民互助請求廣播文字（ROADMAP 125）。
///
/// `name` 嵌入文字；`seed` 供模板輪替。
pub fn get_help_request(persona: ResidentPersona, name: &str, seed: usize) -> String {
    let pool: &[&str] = match persona {
        ResidentPersona::FarmWorker    => FARM_HELP_REQUEST,
        ResidentPersona::MarketBrowser => MARKET_HELP_REQUEST,
        ResidentPersona::TownSquare    => SQUARE_HELP_REQUEST,
        ResidentPersona::Wanderer      => WANDER_HELP_REQUEST,
    };
    pool[seed % pool.len()].replace("{name}", name)
}

/// 取得居民被幫助後的感謝語（ROADMAP 125）。
///
/// `name` 為居民名，`player_name` 為玩家名；`seed` 供模板輪替。
pub fn get_help_thanks(persona: ResidentPersona, name: &str, player_name: &str, seed: usize) -> String {
    let pool: &[&str] = match persona {
        ResidentPersona::FarmWorker    => FARM_HELP_THANKS,
        ResidentPersona::MarketBrowser => MARKET_HELP_THANKS,
        ResidentPersona::TownSquare    => SQUARE_HELP_THANKS,
        ResidentPersona::Wanderer      => WANDER_HELP_THANKS,
    };
    pool[seed % pool.len()]
        .replace("{name}", name)
        .replace("{player}", player_name)
}

// ── 快樂廣播模板（ROADMAP 126）────────────────────────────────────────────────
// 當居民 happiness >= HAPPY_THRESHOLD 時，工作廣播改用這些更歡欣的文字。
// 4 persona × 3 條 = 12 條，語氣明顯更活潑。

static FARM_HAPPY_WORK: &[&str] = &[
    "🌻 {name} 哼著小調翻土，今天的土地格外鬆軟，心情也跟著鬆了！",
    "🌱 {name} 一邊撒種一邊微笑，感覺今年的收成一定特別好！",
    "🚿 {name} 澆水澆得特別起勁，看著綠芽冒出來真是滿足！",
];

static MARKET_HAPPY_WORK: &[&str] = &[
    "💛 {name} 笑著整理攤位，最近城裡氣氛真好，生意也跟著順！",
    "🎉 {name} 和顧客聊得興起，今天的交易特別順暢，心情大好！",
    "✨ {name} 挑貨挑得眼睛發亮，覺得今天一定有好東西進帳！",
];

static SQUARE_HAPPY_WORK: &[&str] = &[
    "☀️ {name} 坐在廣場曬太陽，感覺整個世界都亮了起來，真愜意！",
    "💬 {name} 和路過的鄰居說說笑笑，城鎮有大家真好！",
    "🌸 {name} 看著廣場花圃發呆，心裡暖暖的，說不出是為什麼。",
];

static WANDER_HAPPY_WORK: &[&str] = &[
    "🎶 {name} 悠悠晃過街角，隨口哼了段小曲，心情輕得像風！",
    "🌈 {name} 在城裡四處遊逛，到處都覺得順眼，連石板路都可愛！",
    "🍃 {name} 踩著輕快的步伐繞城一圈，今天到哪都有股說不清的愉快。",
];

/// 取得居民快樂狀態下的工作廣播（ROADMAP 126）。
///
/// 只在 `happiness >= HAPPINESS_HAPPY_THRESHOLD` 時呼叫；語氣比一般版本更歡欣。
pub fn get_happy_work_action(persona: ResidentPersona, name: &str, seed: usize) -> String {
    let pool: &[&str] = match persona {
        ResidentPersona::FarmWorker    => FARM_HAPPY_WORK,
        ResidentPersona::MarketBrowser => MARKET_HAPPY_WORK,
        ResidentPersona::TownSquare    => SQUARE_HAPPY_WORK,
        ResidentPersona::Wanderer      => WANDER_HAPPY_WORK,
    };
    pool[seed % pool.len()].replace("{name}", name)
}

/// 取得居民快樂值突破門檻時的世界聊天廣播（ROADMAP 126）。
///
/// 玩家在聊天欄看到這條，知道自己的幫助讓城鎮更溫暖了。
pub fn get_happiness_boost_chat(name: &str) -> String {
    format!("💛 {} 心情格外好，幹活都有勁！", name)
}

// ── 快樂小回饋模板（ROADMAP 127）──────────────────────────────────────────────
// 快樂居民（happiness ≥ 70）計時到期且有玩家在附近時，主動招待玩家 GIFT_ETHER 乙太。
// 4 persona × 3 條 = 12 條，語氣溫暖、有人情味。

static FARM_GIFT: &[&str] = &[
    "🎁 {name}從口袋掏出一小把乙太，悄悄遞給{player}：「你常在城裡走動，這點心意收著吧！」",
    "🌾 {name}停下手邊農活，笑著向{player}揮手：「{player}！最近城鎮氣氛真好，這點乙太算我請你的！」",
    "🪴 {name}把攢下的一點乙太塞進{player}手裡：「田裡的收成好，分你一點，一起開心！」",
];

static MARKET_GIFT: &[&str] = &[
    "💛 {name}在攤位旁招招手：「{player}！今天生意順，這點乙太算是我的好彩頭，分你一份！」",
    "🛒 {name}把一小包乙太塞給{player}：「你對城裡的人都很好，這點小意思心意到了！」",
    "✨ {name}笑著說：「{player}，城鎮有你真好。拿著，買點你喜歡的東西！」",
];

static SQUARE_GIFT: &[&str] = &[
    "☀️ {name}從石凳上站起來，把一把乙太遞給{player}：「你在這裡讓廣場更熱鬧，感謝你！」",
    "🌸 {name}眼神柔和地看著{player}：「{player}，城鎮因為你更溫暖了。這點心意，請收下。」",
    "💬 {name}拍拍{player}的肩膀：「平時多虧你照顧大家，這點乙太是我的謝意，別嫌少！」",
];

static WANDER_GIFT: &[&str] = &[
    "🧭 {name}從包袱裡翻出一把乙太，笑著交給{player}：「旅人之間互相照應，拿著用！」",
    "🌙 {name}低聲對{player}說：「城裡有你真好。這點乙太，是我想說謝謝的方式。」",
    "🗺️ {name}停下腳步，認真地把乙太遞給{player}：「你對這城鎮的心意，大家都看見了。」",
];

/// 取得快樂居民招待玩家的訊息文字（ROADMAP 127）。
///
/// `name` 為居民名；`player_name` 為玩家名；`seed` 供模板輪替。
pub fn get_gift_message(persona: ResidentPersona, name: &str, player_name: &str, seed: usize) -> String {
    let pool: &[&str] = match persona {
        ResidentPersona::FarmWorker    => FARM_GIFT,
        ResidentPersona::MarketBrowser => MARKET_GIFT,
        ResidentPersona::TownSquare    => SQUARE_GIFT,
        ResidentPersona::Wanderer      => WANDER_GIFT,
    };
    pool[seed % pool.len()]
        .replace("{name}", name)
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

    fn talk_clear_day() -> TownTalkContext {
        TownTalkContext {
            phase: Phase::Day,
            weather: WeatherType::Clear,
            season: Season::Spring,
            prosperity_level: 0,
            eco_alarmed: false,
        }
    }

    #[test]
    fn chat_returns_nonempty_for_all_personas() {
        let ctx = talk_clear_day();
        for (persona, seed) in [
            (ResidentPersona::MarketBrowser, 0usize),
            (ResidentPersona::FarmWorker, 1),
            (ResidentPersona::TownSquare, 2),
            (ResidentPersona::Wanderer, 3),
        ] {
            let r = get_chat(persona, &ctx, seed);
            assert!(!r.is_empty(), "persona {:?} chat empty", persona);
        }
    }

    #[test]
    fn chat_eco_alarm_overrides_everything() {
        // 生態警戒時，不論 persona／季節／天氣／時段，一律回不安的警戒語。
        let ctx = TownTalkContext {
            phase: Phase::Day,
            weather: WeatherType::GrasslandRain, // 連雨天也被警戒壓過
            season: Season::Summer,
            prosperity_level: 9,
            eco_alarmed: true,
        };
        for persona in [
            ResidentPersona::MarketBrowser,
            ResidentPersona::FarmWorker,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            for seed in 0..ECO_ALARM_CHAT.len() + 3 {
                let r = get_chat(persona, &ctx, seed);
                assert!(ECO_ALARM_CHAT.contains(&r), "eco 警戒應回警戒語，got: {r}");
            }
        }
    }

    #[test]
    fn chat_rain_uses_rain_pool() {
        let ctx = TownTalkContext { weather: WeatherType::GrasslandRain, ..talk_clear_day() };
        for seed in 0..6 {
            let r = get_chat(ResidentPersona::FarmWorker, &ctx, seed);
            assert!(RAIN_CHAT.contains(&r), "雨天應回雨天語，got: {r}");
        }
    }

    #[test]
    fn chat_harsh_weather_uses_harsh_pool() {
        // 沙塵 / 晶塵 / 海霧皆走惡劣天氣池。
        for w in [
            WeatherType::DesertSandstorm,
            WeatherType::RockyCrystalDust,
            WeatherType::WaterSeaMist,
        ] {
            let ctx = TownTalkContext { weather: w, ..talk_clear_day() };
            let r = get_chat(ResidentPersona::Wanderer, &ctx, 1);
            assert!(HARSH_WEATHER_CHAT.contains(&r), "{w:?} 應回惡劣天氣語，got: {r}");
        }
    }

    #[test]
    fn chat_night_uses_night_pool() {
        // 晴朗夜／黃昏皆回夜談語。
        for phase in [Phase::Night, Phase::Dusk] {
            let ctx = TownTalkContext { phase, ..talk_clear_day() };
            let r = get_chat(ResidentPersona::TownSquare, &ctx, 2);
            assert!(NIGHT_CHAT.contains(&r), "{phase:?} 應回夜談語，got: {r}");
        }
    }

    #[test]
    fn chat_clear_day_surfaces_season_or_persona() {
        // 晴朗白天、無警戒、繁榮普通：seed 偶數→季節氛圍、奇數→persona 本色，兩種都嚐得到。
        let ctx = talk_clear_day(); // Spring
        let even = get_chat(ResidentPersona::MarketBrowser, &ctx, 0);
        assert!(SPRING_CHAT.contains(&even), "偶數 seed 晴日應回季節語，got: {even}");
        let odd = get_chat(ResidentPersona::MarketBrowser, &ctx, 1);
        assert!(MARKET_CHAT.contains(&odd), "奇數 seed 晴日應回 persona 本色，got: {odd}");
    }

    #[test]
    fn chat_all_seasons_have_lines() {
        for season in [Season::Spring, Season::Summer, Season::Autumn, Season::Winter] {
            let ctx = TownTalkContext { season, ..talk_clear_day() };
            // 偶數 seed 走季節池。
            let r = get_chat(ResidentPersona::Wanderer, &ctx, 0);
            assert!(season_chat(season).contains(&r), "{season:?} 季節語缺失，got: {r}");
        }
    }

    #[test]
    fn chat_prosperity_proud_appears_when_thriving() {
        // 興旺城鎮、晴朗白天、seed % 4 == 0 → 自豪語。
        let ctx = TownTalkContext { prosperity_level: PROSPERITY_PROUD_LEVEL, ..talk_clear_day() };
        let r = get_chat(ResidentPersona::TownSquare, &ctx, 0);
        assert!(PROSPERITY_PROUD_CHAT.contains(&r), "興旺時 seed%4==0 應回自豪語，got: {r}");
    }

    #[test]
    fn chat_is_deterministic() {
        let ctx = talk_clear_day();
        let a = get_chat(ResidentPersona::FarmWorker, &ctx, 7);
        let b = get_chat(ResidentPersona::FarmWorker, &ctx, 7);
        assert_eq!(a, b, "相同輸入應回相同句（確定性）");
    }

    #[test]
    fn dusk_uses_night_pool() {
        let ctx = ResidentContext { phase: Phase::Dusk, weather: WeatherType::Clear };
        let t = get_thought(ResidentPersona::TownSquare, &ctx, 0);
        assert!(SQUARE_THOUGHTS_NIGHT.contains(&t), "dusk should use night pool, got: {t}");
    }

    #[test]
    fn chat_seed_wraps_around() {
        // 各條世界線（警戒／天氣／夜／晴日）大 seed 取模皆不 panic、必非空。
        let bases = [
            talk_clear_day(),
            TownTalkContext { eco_alarmed: true, ..talk_clear_day() },
            TownTalkContext { weather: WeatherType::DesertSandstorm, ..talk_clear_day() },
            TownTalkContext { phase: Phase::Night, ..talk_clear_day() },
            TownTalkContext { prosperity_level: 9, ..talk_clear_day() },
        ];
        for ctx in bases {
            let r = get_chat(ResidentPersona::MarketBrowser, &ctx, 9999);
            assert!(!r.is_empty());
        }
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
        use crate::resident_bonds::NeighborTier;
        // 三個熟識階層的招呼都該叫得出對方名字
        for tier in [NeighborTier::Stranger, NeighborTier::Acquaintance, NeighborTier::Friend] {
            let result = get_neighbor_greet_tiered("阿土", tier, 0);
            assert!(result.contains("阿土"), "招呼文字應包含對方名字（{:?}）", tier);
        }
    }

    #[test]
    fn neighbor_greet_seed_wraps_without_panic() {
        use crate::resident_bonds::NeighborTier;
        let r = get_neighbor_greet_tiered("梅子", NeighborTier::Friend, 9999);
        assert!(!r.is_empty());
    }

    #[test]
    fn neighbor_reply_nonempty_for_all_seeds() {
        use crate::resident_bonds::NeighborTier;
        for tier in [NeighborTier::Stranger, NeighborTier::Acquaintance, NeighborTier::Friend] {
            for seed in [0, 1, 2, 3, 4, 9999] {
                assert!(!get_neighbor_reply_tiered(tier, seed).is_empty());
            }
        }
    }

    #[test]
    fn major_npc_greet_contains_name_and_varied() {
        // 驗證不同 NPC 身分是否有不同模板
        let merchant_greet = get_major_npc_greet("merchant", "阿土", 0);
        assert!(merchant_greet.contains("阿土"));
        
        let chief_greet = get_major_npc_greet("village_chief", "小花", 1);
        assert!(chief_greet.contains("小花"));

        let traveler_greet = get_major_npc_greet("traveler_1", "二柱", 0);
        assert!(traveler_greet.contains("二柱") && traveler_greet.contains("旅人"));
    }

    #[test]
    fn major_npc_reply_is_nonempty() {
        for seed in [0, 1, 2, 99] {
            assert!(!get_major_npc_reply(seed).is_empty());
        }
    }

    #[test]
    fn dynamic_major_greet_to_player_prioritizes_and_fills() {
        // ROADMAP 255：玩家搭話沿用同一支動態話題層（other_name = 玩家名）。
        // 1) 有世界大事時優先聊大事，且帶玩家名、帶事件、不殘留佔位符。
        let events = vec!["裂隙開啟".to_string()];
        let g = get_dynamic_major_npc_greet("merchant", "阿明", 0, &events, &[]);
        assert!(g.contains("阿明"), "應帶玩家名：{g}");
        assert!(g.contains("裂隙開啟"), "有大事時應聊大事：{g}");
        assert!(!g.contains('{'), "替換後不應殘留佔位符：{g}");

        // 2) 無大事但有顯著關係時退到八卦，帶玩家名與對象名。
        let rels = vec![("芙利亞".to_string(), "有點難相處".to_string(), 28)];
        let g2 = get_dynamic_major_npc_greet("merchant", "阿明", 1, &[], &rels);
        assert!(g2.contains("阿明"), "八卦也應帶玩家名：{g2}");
        assert!(g2.contains("芙利亞"), "八卦應提及對象：{g2}");
        assert!(!g2.contains('{'), "替換後不應殘留佔位符：{g2}");

        // 3) 皆無時退回日常寒暄，仍帶玩家名、非空、確定性。
        let g3 = get_dynamic_major_npc_greet("village_chief", "阿明", 2, &[], &[]);
        assert!(g3.contains("阿明") && !g3.is_empty(), "日常寒暄應帶玩家名：{g3}");
        assert_eq!(g3, get_dynamic_major_npc_greet("village_chief", "阿明", 2, &[], &[]));
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

    // ── ROADMAP 125 互助請求測試 ──────────────────────────────────────────────

    #[test]
    fn help_request_all_personas_nonempty() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_help_request(persona, "阿土", 0);
            assert!(!text.is_empty(), "persona {:?} 求助語不應為空", persona);
        }
    }

    #[test]
    fn help_request_contains_name() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_help_request(persona, "梅子", 0);
            assert!(text.contains("梅子"), "求助語應含居民名 '梅子'：{text}");
        }
    }

    #[test]
    fn help_request_seed_wraps_without_panic() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let _ = get_help_request(persona, "二柱", 9999);
        }
    }

    #[test]
    fn help_request_all_templates_have_name_placeholder() {
        for pool in [FARM_HELP_REQUEST, MARKET_HELP_REQUEST, SQUARE_HELP_REQUEST, WANDER_HELP_REQUEST] {
            for template in pool {
                assert!(template.contains("{name}"), "模板應含 {{name}}：{template}");
                let filled = template.replace("{name}", "測試");
                assert!(!filled.contains("{name}"), "替換後不應殘留佔位符");
            }
        }
    }

    #[test]
    fn help_thanks_all_personas_nonempty() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_help_thanks(persona, "阿土", "冒險者", 0);
            assert!(!text.is_empty(), "persona {:?} 感謝語不應為空", persona);
        }
    }

    #[test]
    fn help_thanks_contains_both_names() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_help_thanks(persona, "小花", "英雄甲", 0);
            assert!(text.contains("小花"), "感謝語應含居民名 '小花'：{text}");
            assert!(text.contains("英雄甲"), "感謝語應含玩家名 '英雄甲'：{text}");
        }
    }

    #[test]
    fn help_thanks_no_leftover_placeholders() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            for seed in 0..3 {
                let text = get_help_thanks(persona, "居民名", "玩家名", seed);
                assert!(!text.contains("{name}"), "不應殘留 {{name}}：{text}");
                assert!(!text.contains("{player}"), "不應殘留 {{player}}：{text}");
            }
        }
    }

    // ── 快樂廣播測試（ROADMAP 126）──────────────────────────────────────────────

    #[test]
    fn happy_work_action_all_personas_nonempty() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_happy_work_action(persona, "阿土", 0);
            assert!(!text.is_empty(), "快樂工作廣播不應為空（persona: {:?}）", persona);
        }
    }

    #[test]
    fn happy_work_action_contains_name() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_happy_work_action(persona, "梅子", 1);
            assert!(text.contains("梅子"), "快樂廣播應含居民名（persona: {:?}）：{text}", persona);
        }
    }

    #[test]
    fn happy_work_action_seed_wraps_without_panic() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let _ = get_happy_work_action(persona, "狗蛋", 9999);
        }
    }

    #[test]
    fn happy_work_all_templates_have_name_placeholder() {
        for pool in [FARM_HAPPY_WORK, MARKET_HAPPY_WORK, SQUARE_HAPPY_WORK, WANDER_HAPPY_WORK] {
            for template in pool {
                assert!(template.contains("{name}"), "快樂模板應含 {{name}}：{template}");
                let filled = template.replace("{name}", "測試");
                assert!(!filled.contains("{name}"), "替換後不應殘留佔位符：{filled}");
            }
        }
    }

    #[test]
    fn happiness_boost_chat_contains_name() {
        let msg = get_happiness_boost_chat("阿花");
        assert!(msg.contains("阿花"), "快樂廣播應含居民名：{msg}");
    }

    // ── ROADMAP 127 快樂小回饋測試 ───────────────────────────────────────────────

    #[test]
    fn gift_message_all_personas_nonempty() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_gift_message(persona, "阿土", "冒險者", 0);
            assert!(!text.is_empty(), "persona {:?} 招待訊息不應為空", persona);
        }
    }

    #[test]
    fn gift_message_contains_both_names() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let text = get_gift_message(persona, "梅子", "英雄甲", 0);
            assert!(text.contains("梅子"), "招待訊息應含居民名：{text}");
            assert!(text.contains("英雄甲"), "招待訊息應含玩家名：{text}");
        }
    }

    #[test]
    fn gift_message_no_leftover_placeholders() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            for seed in 0..3 {
                let text = get_gift_message(persona, "居民名", "玩家名", seed);
                assert!(!text.contains("{name}"), "不應殘留 {{name}}：{text}");
                assert!(!text.contains("{player}"), "不應殘留 {{player}}：{text}");
            }
        }
    }

    #[test]
    fn gift_message_seed_wraps_without_panic() {
        for persona in [
            ResidentPersona::FarmWorker,
            ResidentPersona::MarketBrowser,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let _ = get_gift_message(persona, "老根", "測試玩家", 9999);
        }
    }

    #[test]
    fn gift_message_all_templates_have_placeholders() {
        for pool in [FARM_GIFT, MARKET_GIFT, SQUARE_GIFT, WANDER_GIFT] {
            for template in pool {
                assert!(template.contains("{name}"), "禮物模板應含 {{name}}：{template}");
                assert!(template.contains("{player}"), "禮物模板應含 {{player}}：{template}");
                let filled = template.replace("{name}", "A").replace("{player}", "B");
                assert!(!filled.contains('{'), "替換後不應殘留佔位符：{filled}");
            }
        }
    }

    #[test]
    fn triumph_thought_is_deterministic_and_nonempty() {
        // 勝利談資（ROADMAP 186）：依 seed 取模、必非空、且 seed 環繞不 panic。
        for seed in [0usize, 1, 7, 9, 10, 99, usize::MAX] {
            let t = get_triumph_thought(seed);
            assert!(!t.is_empty(), "勝利談資不應為空（seed={seed}）");
        }
        // 同 seed 取兩次應一致（確定性）。
        assert_eq!(get_triumph_thought(3), get_triumph_thought(3));
    }

    #[test]
    fn hero_gratitude_fills_placeholders_and_is_deterministic() {
        // 凱旋英雄禮讚（ROADMAP 188）：依 seed 取模、必非空、替換後不殘留佔位符、含 🙏。
        for seed in [0usize, 1, 5, 6, 99, usize::MAX] {
            let s = get_hero_gratitude("艾拉", "勇者", seed);
            assert!(!s.is_empty(), "禮讚文字不應為空（seed={seed}）");
            assert!(!s.contains('{'), "替換後不應殘留佔位符：{s}");
            assert!(s.contains("艾拉"), "應含居民名：{s}");
            assert!(s.contains("勇者"), "應含英雄名：{s}");
            assert!(s.contains('🙏'), "禮讚應帶 🙏：{s}");
        }
        // 同 seed 取兩次應一致（確定性）。
        assert_eq!(get_hero_gratitude("A", "B", 2), get_hero_gratitude("A", "B", 2));
    }

    #[test]
    fn hero_gratitude_all_templates_have_placeholders() {
        for template in HERO_GRATITUDE {
            assert!(template.contains("{name}"), "禮讚模板應含 {{name}}：{template}");
            assert!(template.contains("{player}"), "禮讚模板應含 {{player}}：{template}");
        }
    }

    // ── ROADMAP 559：鬧彆扭與和好招呼語 ──────────────────────────────────────────

    #[test]
    fn tiff_and_makeup_lines_fill_and_are_deterministic() {
        for seed in [0usize, 1, 3, 7, 99, usize::MAX] {
            // 帶名字那兩句：替換後不殘留佔位符、含對方名字、非空。
            for s in [
                get_neighbor_tiff_open("阿福", seed),
                get_neighbor_makeup_open("阿福", seed),
            ] {
                assert!(!s.is_empty(), "彆扭/和好文字不應為空（seed={seed}）");
                assert!(!s.contains('{'), "替換後不應殘留佔位符：{s}");
                assert!(s.contains("阿福"), "應含對方名：{s}");
            }
            // 回應那兩句：非空。
            assert!(!get_neighbor_tiff_reply(seed).is_empty());
            assert!(!get_neighbor_makeup_reply(seed).is_empty());
        }
        // 確定性：同 seed 取兩次一致。
        assert_eq!(get_neighbor_tiff_open("甲", 2), get_neighbor_tiff_open("甲", 2));
        assert_eq!(get_neighbor_makeup_reply(5), get_neighbor_makeup_reply(5));
    }

    #[test]
    fn tiff_makeup_open_templates_have_placeholder() {
        for t in TIFF_OPEN.iter().chain(MAKEUP_OPEN.iter()) {
            assert!(t.contains("{other}"), "帶名招呼模板應含 {{other}}：{t}");
        }
    }
}
