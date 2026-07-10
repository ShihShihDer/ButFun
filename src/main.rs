//! ButFun — 蒸汽龐克太空歌劇療癒多人世界
//! Phase 0 權威伺服器骨架：靜態前端 + WebSocket 即時多人移動 + 遊戲內建議箱。
//!
//! 詳見 docs/GAME_DESIGN.md。

mod achievement;
mod active_skill;
mod affliction;
mod angler_bond;
mod auth;
mod class;
mod director;
mod combat;
mod compost;
mod guard;
mod dodge;
mod charged_strike;
mod wayfaring;
mod town_blocs;
mod town_share;
mod world_grove;
mod daily_recap;
mod craft_ceremony;
mod player_title;
mod activity_chain;
mod meditation;
mod busking;
mod busking_ensemble;
mod busking_repertoire;
mod kite;
mod firefly_lantern;
mod apiary;
mod companion_revive;
mod meal_buff;
mod meal_share;
mod dish_mastery;
mod onboarding;
mod idle_nudge;
mod town_project;
mod town_project_store;
mod visit_streak;
mod visit_streak_store;
mod welcome_kit;
mod welcome_kit_store;
mod observatory;
mod ether_surge;
mod meteor_shower;
mod night_aether_springs;
mod wandering_merchant;
mod wayposts;
mod bottle_drift;
mod campfire;
mod coop_build;
mod ship_repair;
mod snowman;
mod combat_mark;
mod connections;
mod equipment;
mod refinement;
mod crafting;
mod crop_demand;
mod crop_raid;
mod crop_rotation;
mod crop_variety;
mod crops;
mod daynight;
mod daynight_store;
mod db;
mod dynamic_price;
mod economy;
mod enemy_field;
mod field;
mod field_store;
mod field_thrive;
mod game;
mod gather;
mod gather_field;
mod guild;
mod market;
mod moon;
mod npc;
mod npc_chat;
mod npc_agent;
mod npc_agent_wire;
mod inventory;
mod inventory_store;
mod journey;
mod postcard;
mod postcard_mail;
mod tile_store;
mod tiles;
mod vehicle; // Phase 1-E 蒸汽載具 MVP·北極星「載具」垂直切片
mod vitals;
mod land_plot;
mod land_plot_store;
mod plot_registry;
mod plots;
mod daily_quest;
mod positions;
mod appearance;
mod profile;
mod protocol;
mod quest;
mod state;
mod suggestions;
mod tools;
mod users;
mod version;
mod world_event;
mod ws;
// AI 生態世界 voxel 基底（切片①）：全隔離的方塊世界，並行於現有世界、互不干涉。
mod voxel;
mod voxel_ws;
// 玩家生存指標第一階段（溫和版）：飢餓度＋血量的純邏輯（衰減/吃回復/溺跌傷害/回血/重生/持久化）。
mod voxel_player_stats;
// 乙太方界 AI 居民（切片③）：讓 AI 居民也活在 voxel 世界（純物理/閒晃邏輯）。
mod voxel_residents;
// 乙太方界·人口成長 v1：居民會出生、名冊持久化、新生兒繼承前輩技能（世代傳承、成本有界）。
mod voxel_roster;
// 乙太方界 AI 居民記憶系統 v1：對話歷史 + 持久記憶 + 回想（記得你、記得聊過什麼）。
mod voxel_memory;
// 乙太方界 AI 居民關係系統（切片：居民↔居民偶爾對話，湧現小社會第一顆種子）。
mod voxel_relations;
// 乙太方界 AI 居民渴望系統：玩家的話種下居民的夢想，記憶驅動行為 v1。
mod voxel_desires;
// 乙太方界·居民日記：把記憶格式化成人類可讀的生命故事（ROADMAP 650）。
mod voxel_diary;
// 乙太方界·居民蓋家系統：渴望化為方塊 v1——居民依心願自動蓋小屋/水井/塔/花圃（ROADMAP 652）。
mod voxel_building;
// 乙太方界·念頭播種閉環 v1：玩家說話讓附近居民「聽到」，零 LLM 空間廣播（ROADMAP 654）。
mod voxel_overhear;
// 乙太方界·城鎮動態 Feed v1：居民活動事件日誌，讓玩家回來時讀到「缺席時世界發生了什麼」（ROADMAP 655）。
mod voxel_feed;
// 乙太方界·村莊系統 v1：中央廣場+道路網+沿路地塊，居民新建築認領地塊沿路，一次性鋪路整理既有散落。
mod voxel_village;
// 乙太方界·採集背包 v1：挖方塊得材料、放置消耗存量，療癒「採集→蓋造」循環第一刀（ROADMAP 657）。
mod voxel_inventory;
// 乙太方界·合成台 v1：採集材料→合成木板/石磚/玻璃，「採集→合成→建造」循環（ROADMAP 658）。
mod voxel_craft;
// 乙太方界·種田 v1：撒種‧等待‧收割——療癒循環加入時間維度（ROADMAP 659）。
mod voxel_farm;
// 乙太方界·熔爐煨煮 v1：冶煉不再瞬間，放進生料需時間煨熟，回來取熱騰騰成品（自主提案）。
mod voxel_smelt;
// 乙太方界·居民贈禮 v1：把採來的材料化作一份心意送給居民（ROADMAP 660）。
mod voxel_gift;
// 乙太方界·互動有後果——居民回禮 v1：好感度達門檻時居民主動回贈小禮（ROADMAP 667）。
mod voxel_return_gift;
// 乙太方界·居民投你所好 v1：回禮時讀你說過的偏好記憶，特地挑一份呼應喜好的禮物（ROADMAP 730）。
mod voxel_preference;
// 乙太方界·暗影生物 v1：夜的張力純邏輯——夜間暗處生成、光=庇護、牆=屏障、挖擊反擊（怪物/抵禦第一刀，療癒調性）。
mod voxel_shadow;
// 乙太方界·晝夜循環 v1：世界時鐘純邏輯——一遊戲日 10 分鐘，廣播 time_of_day 給前端更新天空/光照（ROADMAP 661）。
mod voxel_time;
// 乙太方界·季節輪替 v1：累計遊戲日數 → 四季循環純邏輯；換季換世界色調＋居民抬頭反應＋城鎮動態（ROADMAP 798）。
mod voxel_season;
// 乙太方界·居民技能庫 v1：目標+記憶驅動 agency——蓋造不重複/有進展、採集技能（技能調用範本）。
mod voxel_skills;
// 乙太方界·技能發明 v1（真進化第一刀）：居民自己把基礎動作原語組合成解法，成功存成「自己的技能」、之後同處境零 LLM 重用。
mod voxel_invent;
// 乙太方界·建物完工廣播 v1：居民完成蓋造時 WS 廣播 + 慶賀泡泡，讓所有玩家看到世界在長大（ROADMAP 669）。
mod voxel_announce;
mod voxel_trade;
// 乙太方界·居民跨域探訪 v1：建物全蓋完的居民偶爾遠征拜訪鄰里（ROADMAP 671）。
mod voxel_visit;
// 乙太方界·居民情誼 v1：拜訪累積情誼層級（陌生→相識→老朋友），持久化跨重啟（ROADMAP 672）。
mod voxel_bonds;
// 乙太方界·老友情境問候 v1：好感度達老友時，居民說出記憶驅動的特定台詞（ROADMAP 675）。
mod voxel_fond_greeting;
// 乙太方界·居民心情指示 v1：根據情誼與記憶動態計算心情 emoji，廣播給前端（ROADMAP 676）。
mod voxel_mood;
// 乙太方界·孤獨尋伴 v1：Lonely 居民主動走向玩家尋求陪伴，玩家搭話後居民送木頭致謝（ROADMAP 678）。
mod voxel_comfort;
// 乙太方界·居民互相打氣 v1：Joyful/Content 居民主動走向 Lonely 同伴說暖心話，AI 社群自我支撐（ROADMAP 679）。
mod voxel_cheer;
// 乙太方界·箱子儲存 v1：合成箱子→放置→右鍵儲存材料（ROADMAP 692）。
mod voxel_chest;
// 乙太方界·居民回饋糧倉 v1：居民閒晃途中偶爾把手上多餘材料存回你已用過的箱子，箱子第一次雙向（自主提案切片）。
mod voxel_chest_contribute;
// 乙太方界·告示牌 v1：合成告示牌→放置→右鍵寫短字，浮在牌上人人看得見（ROADMAP 740）。
mod voxel_sign;
mod voxel_readsign; // 居民讀牌 v1：居民路過告示牌（740）時念出牌面、回應——玩家寫的字被 AI 看見
mod voxel_nameplate; // 居民立牌命名 v1：居民蓋完建物親手在門前立牌署名（749，741 讀牌的鏡像）
mod voxel_neighborsign; // 居民認得鄰居的家 v1：讀到別的居民立的銘牌時認出那是誰家、記成掛在該鄰居名下的記憶（750）
mod voxel_neighborvisit; // 居民登門找鄰居串門子 v1：朝聖地標其實是鄰居家牌時，抵達當成登門拜訪、情誼加溫（751）
mod voxel_hosted_visit; // 登門遇主人在家便當面迎客 v1：訪客抵達時那位鄰居正好在家，訪客見到本人、主人回招呼並記下迎客記憶（752）
mod voxel_callingcard; // 居民登門撲空留下心意 v1：主人不在家時訪客留下心意，主人回家後感應到「有人來找過我」並記憶（763）
mod voxel_player_home; // 居民認得「你」的家、會登門拜訪你 v1：把 749~763 的鄰里認家網第一次伸向玩家——你在家門前立牌署名，居民日後朝聖抵達時登門找你，碰上就暖招呼、撲空就留一句城鎮動態（自主提案切片，830）
mod voxel_savor; // 你送的食物她會細細享用 v1：食物餽贈長出第二拍——居民稍後在閒暇享用你送的食物，冒暖泡泡＋動態牆＋重新點亮心情（765）
mod voxel_meal; // 親手煮的暖食自己也能享用 v1：玩家吃下自己煮的料理得到暖意回饋，站在居民旁享用還會感染附近居民、深化交情（779）
mod voxel_self_image; // 居民自我印象 v1：從累積記憶昇華出「我漸漸成了村裡最愛蓋東西的人」的自我概念——顯示在日記頂端＋偶爾閒暇說出口＋動態牆（reflection→高階印象，770）
mod voxel_admire; // 居民會注意到你親手蓋的東西 v1：玩家在居民身邊接連堆起一片方塊（牆／屋／橋）時，居民停下來讚賞一句＋把「看著這位旅人親手蓋起了東西」記進心裡＋累積好感——建造第一次被居民看見（773）
mod voxel_farm_admire; // 居民注意到你悉心照料的農地 v1·773 讓居民注意到玩家連續放置方塊蓋東西，但種田（659~811）從頭到尾只有 753 居民主動幫忙照料、從沒有居民單純看見你在種田並由衷讚賞；本模組讓連續翻土/播種的農忙也能被身邊居民看見、記進心裡，用 farm 專屬台詞與 773 建造讚賞區隔（自主提案切片）
mod voxel_stall_notify; // 自由市集成交後，賣家離線也會知道 v1·832 讓攤位哪怕擺攤者離線也能兌現，但成交後擺攤者本人完全無感——材料默默記進背包、全域動態牆只是給別人看的村莊八卦；本模組比照 763 撲空留心意的精神，讓擺攤者下次登入時收到一則私訊「你不在時，某某接手了你的攤位」，經濟互動第一次也有了「錯過也讓你知道」的溫度（自主提案切片，ROADMAP 864）
mod voxel_structure_name; // 居民為你的建造作品取名字 v1：773 讚賞至今只講泛稱「你蓋的東西」，本模組讓居民第一次幫作品取名、下次路過認得出「就是這個」並喚出名字，自主提案切片
mod voxel_village_milestone; // 村莊集體里程碑 v1：既有里程碑全綁定單一玩家，本模組讓「被居民命名的地標數」跨過門檻時全村一起慶祝，第一個「大家一起達成」的集體事件，自主提案切片（ROADMAP 856）
mod voxel_monument; // 村莊中央紀念柱（村碑）v1：村莊每達一個集體里程碑（856），居民合力在廣場中心把一根中央村碑再拔高一段（石磚柱身＋乙太燈頂），集體成就第一次成為玩家眼見、夜裡發光的實體地標，自主提案切片（ROADMAP 885）
mod voxel_confide; // 居民會主動跟你聊起她的心事 v1：夠熟的居民靠近時偶爾主動把當前渴望當成心事對你說出口（被動日記→主動分享），並把「對你掏了心」記進記憶、加深情誼（781）
mod voxel_playerepithet; // 居民為你取一個名號 v1：把「關於某玩家」的累積記憶昇華成你在牠心中的角色名號（造物者／慷慨的人／老搭檔／常來的老友），打招呼改用名號稱呼你＋第一次安下名號時記動態牆——自我印象(770)的對外鏡像（世界如何看你）
mod voxel_witness; // 居民為鄰居圓夢而賀喜 v1：你送對禮物圓了某居民的心願時，身邊醒著的鄰居會看見、由衷道賀一句，圓夢者回謝，兩人情誼因這份共同喜悅升溫、各記一筆——小社會第一次為彼此的成就道賀（782）
mod voxel_friendtoken; // 居民為友誼立下信物 v1：兩位居民第一次成為老朋友時，作東者在家旁點起一盞「友誼的燈」作持久信物——小社會的關係網第一次從查表變成世界裡看得見的實體地標（自主提案切片）
mod voxel_request; // 居民會反過來拜託你幫個小忙 v1：夠面熟的居民偶爾主動向你討一樣好採集的基礎材料（木/石/煤/沙），你採來當禮物送到，她特別歡欣道謝並把「你在我開口時幫了我」記進記憶、加深情誼——採集第一次接上居民的需要（記憶→行為·你的互動有後果）
// 乙太方界·居民口耳相傳 v1：老朋友到訪時轉述見聞，記憶經朋友網絡流通（ROADMAP 694）。
mod voxel_gossip;
mod voxel_epithet_spread; // 你的名號口耳相傳 v1：居民把她為你取的名號說給老朋友聽，訪客從此「久仰」你、用名號招呼——774 名號透過小社會朋友網絡自己傳開（自主提案切片）
mod voxel_epithet_sign; // 居民把你掙得的名號刻成一塊牌立在自家門旁 v1：第一次為你安下名號時，在門旁立一塊「此地常客·造物者」告示牌——名號從口說變成世界裡永久可走近可讀的實體印記（keepsake(732) 之於禮物的鏡像·自主提案切片）
mod voxel_epithet_esteem; // 名號化為敬意 v1：被你贏得名號的居民，看見你在中距離時偶爾放下閒晃、特地走過來向你致意——你的名聲第一次改變居民的「行為」（記憶→行為·後果；鏡像 678 尋伴的走近但由敬重而非孤獨驅動·自主提案切片 ROADMAP 777）
mod voxel_stargaze; // 繁星夜空，居民夜裡邀你一起看星星 v1：夜空第一次掛滿繁星＋升起明月（前端純視覺），記得你愛看星星的居民會在星夜特地喚你到身邊同賞、記進交情——記憶驅動行為的浪漫一拍（自主提案切片 783）
mod voxel_firework; // 乙太煙火 v1：玩家把乙太礦＋煤礦合成煙火、朝夜空施放，火花在星空綻放、附近居民抬頭歡呼——玩家第一個主動點亮夜空、與居民共享的慶祝動作（自主提案切片 785）
mod voxel_compost; // 乙太沃肥 v1：把雜草＋泥土在工作台漚成沃肥，手持對準幼苗一撒即催熟一截（沿用農地 nudge_growth）——玩家第一個主動加速農業的動詞（自主提案切片 789）
mod voxel_tool; // 工欲善其事 v1：手持對的工具（鎬/斧/鏟）採集對應方塊，有機率多掉一份材料——工具第一次影響採集產出、療癒循環閉環（自主提案切片 790）
mod voxel_timely; // 時令作物 v1：每種作物有其「時令」季節（胡蘿蔔春/小麥夏/馬鈴薯秋），種在時令裡一種下就靠 nudge_growth 給一截 head-start——季節（798）第一次真的牽動玩法（自主提案切片 811）
mod voxel_bounty; // 時令豐收 v1：在作物的時令季節收割成熟植株 → 額外多得一份果實（與 811 對成「種在時令長得快／收在時令收得多」一對）（自主提案切片 812）
// 乙太方界·下雨天氣 v1：機率式晴/雨切換，下雨時農地視同水耕（ROADMAP 700）。
mod voxel_weather;
// 乙太方界·居民小圈子聚會 v1：互為老朋友的圈子偶爾相約碰面，小社會第一次被看見「聚在一起」（ROADMAP 711）。
mod voxel_clique;
// 乙太方界·居民偶爾小小拌嘴又和好 v1：老朋友到訪偶爾拌幾句嘴又和好，關係第一次有了真實的小摩擦（ROADMAP 715）。
mod voxel_quarrel;
mod voxel_tend; // 居民照料菜園 v1：對你有好感的居民路過你種下、還沒成熟的作物旁時順手幫忙照料（ROADMAP 753）
mod voxel_seedgift; // 居民種下你送的種子 v1：把你送的種子種進家旁的土裡、長成她自己的一畦菜園（ROADMAP 754）
mod voxel_giftgarden; // 居民收成你送的種子長成的菜園、把第一把收穫回贈給你 v1（ROADMAP 755）
mod voxel_teach;
// 乙太方界·居民夜晚睡覺 v1：深夜回到自家附近就躺下睡覺、頭頂冒 💤，天亮才醒來神清氣爽——世界第一次有了入夜就寢的作息（ROADMAP 739）。
mod voxel_sleep;
// 乙太方界·居民就寢反思 v1：入睡時回味今天最有感的一件事、冒個人化反思泡泡、昇華成「睡前反思」記憶並記進動態——記憶第一次驅動了「就寢」這個作息節拍（ROADMAP 744）。
mod voxel_bedtime;
// 乙太方界·居民會做夢 v1：熟睡中，一段深藏心底的珍貴往事不由自主浮成夢，冒「💤 夢見…」泡泡並記進動態——記憶連睡夢裡都在悄悄活著（ROADMAP 805）。
mod voxel_dream;
// 乙太方界·居民早上會把昨晚的夢說給你聽 v1：夜裡做過夢的居民，白天遇到你時偶爾主動把昨晚那個夢分享出來——夜的孤景第一次有了白天的回響與聽眾，並把這份親近記進記憶、加深情誼（ROADMAP 807）。
mod voxel_dreamshare;
// 乙太方界·居民晨間探友 v1：醒來讀昨晚的「睡前反思」記憶，若那份牽掛裡有另一位居民的名字，今天第一件事就是走去找他——記憶第一次真的改變了居民今天的去向（ROADMAP 745）。
mod voxel_morning;
// 乙太方界·居民晨間思念玩家 v1：醒來讀昨晚的「睡前反思」記憶，若那份牽掛裡有某位此刻在線玩家的名字，牠今天第一件事就是朝你走過來、抵達暖暖打招呼——記憶第一次把居民的腳步帶到了「你」面前（ROADMAP 746）。
mod voxel_daybreak;
// 乙太方界·居民久別重逢奔迎 v1：玩家久別歸來時，最惦記他的居民放下手邊的事奔向他、暖暖迎接（ROADMAP 747）。
mod voxel_reunion;
// 乙太方界·居民遠行探野 v1：Wanderer 人格居民偶爾獨自遠行到遠離主城的世界邊陲住上一陣子再返家——居民足跡第一次散進荒野（ROADMAP 756·item 7 散居維度）。
mod voxel_expedition;
// 乙太方界·久別重逢摘要 v1：玩家重連時把離線期間的重要事件彙整成一句私訊（ROADMAP 721）。
mod voxel_welcome;
// 乙太方界·居民互相以物易物 v1：交易特長系統（670）第一次接到居民與居民之間，小社會有了內部經濟（ROADMAP 723）。
mod voxel_resident_trade;
// 乙太方界·居民互贈分享採集所得 v1：老朋友到訪時主人把採集背包裡真的有的材料勻一份給訪客，小社會第一道真實物資流動（ROADMAP 748）。
mod voxel_share;
// 乙太方界·居民以物易物 v1：有餘料的居民路遇缺該料的老朋友，走近提議「拿我多的換你多的」，成交則雙方背包真的對換——小社會第一樁雙向互利的以物易物（ROADMAP 888）。
mod voxel_barter;
// 乙太方界·玩家里程碑 v1：把玩家自己的療癒循環第一次做成可回頭翻閱的成就徽章（ROADMAP 724）。
mod voxel_milestones;
// 乙太方界·居民為你的個人里程碑喝采 v1：里程碑此前只私訊玩家自己，本模組讓身邊閒著的
// 居民也為你的「第一次」由衷喝采、記進心裡（自主提案切片，接續 724/856）。
mod voxel_milestone_cheer;
// 乙太方界·玩家位置持久化 v1：登入帳號重整/重登回到上次位置（訪客不存、綁後端權威 email）。
mod voxel_player_pos;
mod voxel_keepsake; // ROADMAP 732 你送的心意她擺了出來·玩家餽贈化為紀念物擺進世界
mod voxel_keepsake_recall; // ROADMAP 784 居民路過你送的紀念物時駐足想起你·keepsake 落地後接成「記憶→睹物思人」的持續回響（自主提案切片）
mod voxel_humming; // ROADMAP 788 居民哼起歌來·心情正好（剛被互動點亮）時偶爾哼歌、頭頂飄音符，你在身邊時哼給你聽並記進交情——世界第一段旋律（自主提案切片）
mod voxel_campfire; // 乙太營火 v1·玩家蓋一處發光火堆，夜裡路過的居民駐足圍暖、心情變好，你也在旁時哼句暖語記進交情——玩家的建造第一次塑造居民的夜間社交場所（自主提案切片）
mod voxel_campfire_tale; // 圍著營火說故事 v1·夜裡兩位以上醒著的居民聚到同一座營火邊時，其中一位把心裡的一段往事講給夥伴聽，聆聽者記進社交記憶、兩人心情都亮一格——營火第一次成為居民之間的社交舞台（自主提案切片）
mod voxel_bucket; // 水桶 v1·用鐵錠打一只水桶，到湖邊舀水、回乾田倒出永久水源，接上既有水流模擬與鄰水加速種田——玩家第一次能親手搬水、把荒地改造成綠洲水田（自主提案切片）
mod voxel_hoe; // 鋤頭 v1·用木頭木板打一把鋤頭，走到草地/泥土上一鋤就地翻成農田土，省去挖土搬工作台再放回的繞路——與水桶成對（一管引水、一管開墾），親手把荒地開成田（自主提案切片）
mod voxel_fishing; // ROADMAP 734 垂釣 v1·對準水面拋竿等待收竿，釣起小魚/稀有乙太魚
mod voxel_grove; // ROADMAP 738 植樹造林 v1·砍葉得樹苗→種在土地上→靜候長成一株樹，可再生木材
mod voxel_bench; // 木長椅 v1·玩家合成一張長椅擺在世界裡，白天路過閒著的居民會停下腳步坐上去歇一會兒——玩家的建造第一次塑造居民的日間日常，與營火那條夜間線對成白天／夜晚一對（自主提案切片）
mod voxel_bench_chat; // 長椅並坐閒聊 v1·白天兩位相識以上的居民恰好都走到同一張長椅邊時，一位招呼另一位並肩坐下閒聊家常，兩人心情都亮一格、交情加溫——玩家的長椅第一次成為村子白天的社交角落，與圍火講故事那條夜間社交線對成白天／夜晚一對（自主提案切片）
mod voxel_bench_tiff; // 長椅拌嘴/和好 v1·交情已到老朋友的兩位居民偶爾在長椅邊為小事拌幾句嘴、彆扭一陣子，下次再碰上同一張長椅就會和好如初、交情反而更進一步——情誼帳本「熟識/幫過/吵過」第一次補上「吵過」這塊，居民關係第一次有負向摩擦與修復（自主提案切片）
mod voxel_anglerest; // 居民臨水垂釣 v1·白天閒著、恰好走到天然水體邊的居民，偶爾停下腳步對著水面靜靜垂一竿、釣起一尾小魚——把垂釣（734）模組早就埋下「居民的日記悄悄嚮往著釣魚」那份至今只寫在日記裡的嚮往第一次真的活出來（記憶/嚮往→行為，自主提案切片）
mod voxel_berry; // ROADMAP 806 莓果叢 v1·多年生可反覆採收的莓果叢：採收後回退再結果、不必重種（自主提案切片）
mod voxel_coop; // 雞舍生蛋 v1·世界第一種「動物產物」資源節點：放一座雞舍、靜候生蛋、收下就地回退繼續孵——與莓果叢對成植物/動物兩條可反覆採收的資源軸（自主提案切片）
mod voxel_raincover; // 雨天葉傘避雨 v1·下雨時閒著醒著的居民偶爾摘片闊葉當傘、停下腳步躲一會兒雨（設 wait_timer 駐足＝雨第一次改變居民「做什麼」而非只「說什麼」），你在近旁時招呼共避一葉傘、記進交情——環境×居民即時反應（自主提案切片）
mod voxel_homegaze; // 居民顧家駐足 v1·白天閒著醒著、恰好走到自家門前的居民偶爾停下腳步望著自己一手安頓下來的家、湧起踏實的歸屬感（設 wait_timer 駐足＝居民第一次對「一個地點（自家）」生出情感），你在近旁時把家的踏實記進交情——place-attachment/歸屬感（自主提案切片 816）
mod voxel_birthday; // 居民誕辰紀念 v1·世代傳承（人口成長）誕生的居民每滿一個乙太年（YEAR_SECS，與四季輪替同長）迎來一次誕辰紀念——回望來到這片天地多久、記得是誰生下自己便謝過父母，你在近旁時特地點名和你分享這一刻——世界第一次讓「時間／年歲」本身成為驅動行為的記憶（自主提案切片 819）
mod voxel_bell; // 集會鐘 v1·玩家用鐵錠鑄一座鐘、右鍵敲響，附近閒著的居民循聲朝你走來聚集、心情變好、記進交情——玩家第一次能像村長一樣主動把村民召到身邊（自主提案切片 796）
mod voxel_hunger; // 居民也會肚子餓 v1·居民第一個生理需求：餓意隨時間累積→回家吃存糧、你在她餓時餵食記得格外深（自主提案切片 799）
mod voxel_share_meal; // 飢餓時的守望相助 v1·餓著找吃的居民路遇交情已到相識以上的閒著鄰居，鄰居偶爾分她一口飯：餓意當場解、雙方各記暖記憶、情誼再加溫一格（自主提案切片 800）
mod voxel_gratitude; // 知恩圖報 v1·居民記得誰在牠餓時分過飯，換牠有餘力時優先回報那一口——連陌生人也還（打破 800 相識門檻），記憶對行為產生真實例外（自主提案切片 801）
mod voxel_ratelimit; // 對話 per-IP 速率限制 v1·上架前治安三件套①：per-connection 冷卻可被「開多條連線」繞過→以真實 IP 為鍵的 token bucket 設跨連線天花板，擋白嫖/燒爆免費 LLM（自主提案切片 802）
mod voxel_moderation; // 對話內容審查 v1·上架前治安三件套②：文字長度/速率合格但「內容」無閘→純邏輯樣式審查攔 prompt injection/越獄注入與明顯辱罵，超線者收溫柔提示、絕不觸發 LLM/廣播原文（自主提案切片 803）
mod voxel_frontier_visit; // 居民千里跋涉去邊陲探望遠行的夥伴 v1·留守主城的居民（露娜/賽勒）跟正在邊陲逗留的老朋友（奧瑞/諾娃）交情夠深時，偶爾放下手邊的事追去邊陲找她——散居（item 7）與情誼（item 4）第一次交織（自主提案切片 821）
mod voxel_illness; // 居民也會生病、鄰居與你的照顧讓她好轉快 v1·全庫唯一空白的「被照顧」情感深度——偶爾病倒、動作慢下來，靠自己休息會漸漸好轉，但交情夠的鄰居恰好路過陪伴、或你送她一碗暖湯，會好得更快（自主提案切片，本輪）
mod voxel_bottle; // 漂流瓶 v1·「玩家↔玩家」這條線在乙太方界至今完全空白——合成空玻璃瓶、寫上一句話丟進水裡，另一位路過水邊的玩家會撿起它、讀到陌生旅人的匿名留言；內容經治安三件套②審查、登入才能丟瓶、全局瓶數有上限，世界第一次有了玩家留給玩家的溫柔巧遇（自主提案切片 825）
mod voxel_blueprint; // 建築藍圖 v1·居民想蓋什麼至今全靠猜關鍵詞——合成一張藍圖送給居民，直接改寫她的心願成你指定的建物種類，完工時指名感謝你；沿用玩家聊天種願望的同一套 set_desire 機制，零新狀態機（自主提案切片）
mod voxel_coop_gather; // 並肩協作 v1·玩家↔玩家至今唯一互動是漂流瓶（825，非同步/匿名/一次性）——本刀補上第一個即時/同步協作：挖天然方塊時附近有其他真人玩家一起忙活，默契讓這塊多掉一點；沿用 790 工具加成同一張天然採集方塊適配表，不重立表也不疊加時令/工具加成（自主提案切片 827）
mod voxel_dropitem; // 掉落物 v1·玩家↔玩家至今僅有漂流瓶（825，非同步/文字）與並肩協作（827，被動加成）——本刀補上第一個主動的實體資源轉手：對著地面丟下手上一件材料，安靜留在原地，附近另一位玩家（或自己）走近時自動撿起（自主提案切片 828）
mod voxel_stall; // 玩家自由市集 v1·玩家↔玩家至今從沒有「雙向議定」的以物易物（670/723 都只到居民為止）——本刀補上：擺一個小攤，放上你願給的材料＋標明你想換的材料，escrow 扣下你的材料存進攤位本身，任何路過、身上有你要的東西的旅人都能上前成交，哪怕擺攤者早已離線；沒人接手可自行收回或逾時自動退還（自主提案切片 832）
mod voxel_frontier_find; // 玩家追到邊陲、巧遇正在遠行的居民 v1·821 讓留守居民追去邊陲找老朋友，但玩家從沒有這條路——即使真的一路追到邊陲撞見正在遠行的居民，牠的反應跟在主城相遇一模一樣；本刀讓居民第一次認出「你是特地追這麼遠來的」，冒出更驚喜的招呼、記進交情，散居（item 7）與玩家互動（item 3）第一次交織（自主提案切片）
mod voxel_discovery; // 探索紀事 v1·838 遺跡／839 溫泉找到之後除了當下的驚喜什麼都沒留下——玩家找過幾處、位置在哪，全無管道回顧；本模組補上持久化的探索足跡（座標＋種類），並替兩個系統補齊此前漏掉的里程碑徽章（自主提案切片）
mod voxel_landmark_note; // 地標旅人留言 v1·840 探索紀事是玩家私人視角（我去過哪裡），但同一處遺跡/溫泉被不同玩家造訪過，彼此完全看不到對方留下的痕跡；本模組讓每處地標第一次擁有一本共同的旅人留言簿，發現地標時能讀到先前旅人的話、也能留一句給後來的人（自主提案切片，ROADMAP 862）
mod voxel_mastery; // 玩家熟練度 v1·里程碑（724）只解決「做過一次沒」的二元徽章，持續投入本身沒有累積成長——本模組補上⛏️採集／🌾耕種／🎣垂釣三條連續熟練度，練到 Lv.5 起解鎖小額產出加成，讓反覆遊玩第一次有看得見的回饋曲線（自主提案切片，ROADMAP 842）
mod voxel_playercare; // 居民關心你挨餓 v1·至今「居民→玩家」的互動全是你先做了什麼（送禮/敲鐘/登門）居民才回應——本模組讓居民第一次主動注意到你此刻挨餓、走過來遞一份麵包、記進她心裡，把「你的互動有後果」第一次反過來（自主提案切片，ROADMAP 845）
mod voxel_romance; // 居民戀愛心動 v1·居民↔居民關係至今只有情誼（陌生→相識→老朋友）一條軸線——本模組補上一條全新的浪漫軸線：老朋友並肩坐在長椅上閒聊時偶爾擦出心動火花，締結成一對戀人（一生只有一位），小社會裡至今唯一空白的人性化羈絆（自主提案切片，ROADMAP 846）
mod voxel_lover_seek; // 戀人牽掛 v1·846 讓兩位老朋友締結成戀人，但成了戀人之後這份羈絆從沒有改變過任何行為——本模組讓「戀人」第一次真的影響行為：分開得夠遠、戀人醒著、過機率門檻，會放下手邊的事去找對方，重逢那一刻雙方各自留下一筆暖記憶（自主提案切片，ROADMAP 852）
mod voxel_wildlife; // 野兔 v1·乙太方界至今只有 4 位具名 AI 居民會走動，草地上從沒有任何環境生物——世界看得出「有人」卻看不出「有生機」；本模組補上世界第一種環境點綴生物：幾隻在村莊周圍草地上悠閒遊蕩、見到玩家靠近就受驚跳開的野兔，純點綴、無 AI 大腦、無戰鬥，讓乙太方界第一次不只是居民的舞台，也是一個活生生的世界（自主提案切片，ROADMAP 847）
mod voxel_fish; // 水中游魚 v1·野兔（847）讓陸地看得出有生機，但水域（湖泊/海灣）至今空蕩——除非上鉤，玩家從沒見過水裡有魚；本模組讓魚安靜地在水下游動，共用野兔的 wildlife hub 基礎設施只新增 Fish 分支，證明「世界環境」軸線可延伸而非野兔專屬特例，也給釣魚（794/841）第一次視覺線索（自主提案切片，ROADMAP 848）
mod voxel_player_recipe; // 居民教你一道獨門配方 v1·居民互相傳授技能（717）讓「本事」在居民↔居民朋友網絡流通，玩家的合成配方卻從第一天起全部開放，沒有任何「靠情誼解鎖新配方」的路；本模組讓居民↔玩家的關係第一次在配方上留下痕跡——好感深到門檻，她會主動教你一道獨門配方「護身符」，從此永久解鎖（自主提案切片，ROADMAP 849）
mod voxel_envy; // 居民見賢思齊 v1·居民的渴望至今只從對話/自我禱告/好奇心三個來源萌生，世界裡真實存在的事物（773/854 讓玩家蓋起、被居民命名記住的地標）從沒觸發過任何居民的心願；本模組補上第四個來源——居民路過一座已命名地標，偶爾心生嚮往，也想擁有一座自己的類似建物，世界第一次讓「環境本身」而非對話驅動居民的心願（自主提案切片，ROADMAP 858）
mod voxel_waypoint; // 個人路標 v1·世界很大（散居村莊+程序生成地形），玩家自己標記的地點（礦坑入口/看中的地基）走開幾步就再也找不回去，705 羅盤／820 雷達只導向居民；本模組讓玩家能在目前所站的位置插一支路標、取個短名字，之後在同一面板跟居民座標並列導航（自主提案切片，ROADMAP 869）
mod voxel_chicken; // 放養雞 v1·wildlife 系統至今只有野兔（847）一種陸地生物，雞舍（880）雖補上動物產物資源軸卻是不再互動的靜態方塊；本模組補上世界第二種可馴服動物——用種子馴服一隻在村莊周圍啄食的雞，牠會跟著你走，還會定期主動回饋你一顆蛋，陪伴與資源產出第一次疊在同一隻生物身上（自主提案切片，ROADMAP 870）
mod voxel_diary_peek; // 居民察覺你翻過她的日記 v1·日記（650）/日記牆（770）至今永遠單向——居民從沒發現自己被讀過；本模組讓居民在下次打招呼時有機率察覺「你翻過我的日記」，把「被凝視」第一次記進她對你的記憶，補齊日記/生命故事路線圖唯一的反饋缺口（自主提案切片，ROADMAP 871）
mod voxel_pet_admire; // 居民注意到你身邊跟著的馴服動物 v1·850/851 餵野兔馴服＋跟隨、870 放養雞讓玩家能馴服一隻動物並帶著牠散步，但這份羈絆從沒被居民感知過；本模組讓居民注意到你身邊跟著已馴服的兔子/雞，由衷讚賞這份陪伴，wildlife 系統與居民關係第一次交會（自主提案切片，ROADMAP 875）
mod voxel_proximity_teach; // 就地指導 v1·717 讓老朋友「登門到訪」時偶爾隨機教一手已學會的技能，但線上日誌顯示技能發明（716～867 真進化系列）更常見的一幕從沒被接住——居民對著同一個目標反覆想不出辦法、進退避冷卻，身邊卻可能正站著早就自己發明過同一樣東西的老朋友；本模組讓「答案就在旁邊」這件事真的發生：平常閒晃時剛好站得夠近，老朋友就會就地教會她正卡關的那個目標，不必等到下次登門到訪（自主提案切片）
mod voxel_treasure; // 深層寶藏 v1·790 工欲善其事只讓帶對工具多掉一份「同一種」素材，挖礦本身從沒有過真正「巧遇驚喜」的一刻；本模組在天然礦脈（非遺跡礦）裡藏進極稀有一小撮秘密寶藏，挖到時意外多得乙太幣——經濟（873）第一次有了「挖到的」而非只有「鑄出來的」貨幣來源（自主提案切片）
mod voxel_colony; // 分村殖民 v1·世界至今永遠只有一座主村——本模組讓成熟的主村派拓荒隊到遠方異群系奠下第二座「有名字、有立村故事」的野外村落殘核（小廣場＋水井＋燈＋立村碑），玩家遠行撞見可讀到來歷、記進探索紀事（自主提案切片，承 PLAN_ETHERVOX §7）
mod voxel_lovenest; // 戀人愛巢 v1·居民戀愛（846）至今只活在面板 ❤️ 與動態牆——本模組讓一對戀人在一起一陣子後在村邊合力蓋起一間亮著燈的小屋、門前立牌「X 與 Y 的愛巢」並登記進地標，戀愛第一次長成玩家走得到、夜裡發光的實體（自主提案切片，承 PLAN_ETHERVOX §4/§6）
mod pet;
mod pet_fetch;
mod pet_forage; // ROADMAP 484 寵物撈寶·把逗寵物接物接進羈絆→成長→回饋循環
mod pet_follow;
mod pet_greeting;
mod pet_personality;
mod pet_play;
mod fish_school;
mod fish_size;
mod fishing;
mod fishing_bite;
mod mining_vein;
mod prospecting; // ROADMAP 562 勘礦造詣·越掘越懂礦脈（採礦個人養成曲線）
mod cooking_steps;
mod aether_draw;
mod woodcutting;
mod skipstone; // ROADMAP 475 打水漂·水域第一個玩的動詞
mod skip_treasure; // ROADMAP 483 打水漂撈寶·把水漂接進經濟／風險循環
mod coop_labour; // ROADMAP 414 並肩協作·結伴勞動默契加成
mod constellation;
mod ancient_inscription;
mod field_guide;
mod terrain_atlas;
mod sky_codex;
mod ranching;
mod farm_crops;
mod star_crystal;
mod trade_route;
mod workshop;
mod bounty_board;
mod bounty_harvest;
mod expedition;
mod procurement;
mod farm_fair;
mod npc_lifecycle;
mod npc_schedule;
mod npc_memory_store;
mod npc_factions;
mod npc_gather;
mod npc_needs;
mod npc_proactive;
mod npc_relations;
mod village_chief;
mod traveler_npc;
mod boss_roar;
mod boss_ai;
mod boss_slam;
mod plaza_talk;
mod npc_dawn_call;
mod npc_dusk_call;
mod npc_noon_bell;
mod npc_night_watch;
mod daytime_talk;
mod lunch_chatter;
mod lunch_gift;
mod lunch_regular;
mod npc_bounty;
mod npc_defeat_reaction;
mod npc_level_greet;
mod npc_recognition;
mod npc_commission;
mod npc_expedition_boost;
mod npc_workshop_boost;
mod npc_treasury;
mod npc_deal;
mod npc_stock;
mod supply_chain;
mod world_log;
mod world_glimpse; // ROADMAP 445 世界此刻一瞥·登入畫面映出當下時辰/季節/天氣
mod player_log;
mod player_emote;
mod high_five;
mod emote_resonance;
mod player_cheer;
mod popularity_gathering;
mod weather;
mod wind; // ROADMAP 430 微風拂過微縮世界·世界級風場
mod rainbow;
mod reconcile;
mod return_hook;
mod region_name;
mod wayfinding;
mod social_dynamics;
mod soil_vitality;
mod friends;
mod party;
mod sprinkler;
mod village_well; // ROADMAP 640 禱告驅動·故鄉古井（應諾娃之禱，定時滋潤公田）
mod village_tea_stall; // ROADMAP 641 禱告驅動·故鄉茶棚（應露娜之禱，定時出爐熱茶溫暖全鎮）
mod resident_home; // ROADMAP 642 禱告驅動·居民木屋（應居民之禱，為他們蓋起溫暖的家）
mod harvest_festival; // ROADMAP 646 禱告驅動·豐收節慶典（應露娜之禱，廣場定期升起彩旗慶典）
mod field_spring;     // ROADMAP 647 禱告驅動·田邊清泉（應諾娃之禱，農田北坡天然清泉常流不息）
mod warehouse;
mod perishable;
mod home_interior;
mod home_furniture;
mod home_decor;
mod resident_npc;
mod resident_chat;
mod resident_bonds;
mod resident_care_back;
mod town_prosperity;
mod community_gathering;
mod season;
mod seasonal_harvest_award;
mod session_champions;
mod seasonal_nodes;
mod wildlife;
mod species_relations;
mod stat_points;
mod skill_mastery;
mod civic_vote;
mod town_memory;
mod invasion;
mod monster_colony;
mod eco_pressure;
mod eco_report;
mod eco_bounty;
mod eco_festival;
mod item_rarity;
mod element_affinity;
mod kill_streak;
mod weakpoint;
mod world_tally; // ROADMAP 495 今日世界戰報·廣場石板第一次有了「全服今天做了什麼」
mod world_tally_milestone; // ROADMAP 498 全服里程碑喝采——計數突破門檻時廣場 NPC 鼓舞一句
mod world_wonder; // ROADMAP 524 世界奇觀首探·五處隱藏秘境散落世界各角
mod world_boss;   // ROADMAP 525 世界守護者降臨·超強守護者周期現身荒野，協力擊敗全服皆獎
mod rain_regen;   // ROADMAP 496 草原細雨庇護·天氣首次影響戰鬥——細雨中戶外玩家緩緩回血
mod rain_harvest; // ROADMAP 502 雨天豐澤·細雨中收成每株多 +1 乙太
mod gold_rush;         // ROADMAP 521 黃金礦脈爭奪戰·每 30 分鐘週期性競技採礦事件
mod auction;           // ROADMAP 522 星際拍賣行·每 2 小時全服競標傳說遺物
mod fishing_contest;   // ROADMAP 523 萬尾釣魚大賽·每 45 分鐘全服釣魚競速，比總體長
mod monument;          // ROADMAP 526 旅人紀念碑·廣場石碑銘記守護者首殺/奇觀首探/釣魚冠軍/礦脈冠軍
mod guardian_blessing; // ROADMAP 533 守護者元素祝福·擊敗守護者的參戰玩家獲元素光環持續 2 小時

use std::net::SocketAddr;

use axum::extract::State;
use axum::http::header;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use state::AppState;
use suggestions::NewSuggestion;

#[tokio::main]
async fn main() {
    // 部署自驗 CLI（非伺服器啟動路徑）：`butfun-server verify-version <expected> <actual>`。
    // 比對「該部署的目標 commit」與「跑著的 server 回報的 commit」，印出判定並以 exit code 表態，
    // 讓 scripts/deploy.sh 與後端共用同一份比對邏輯（單一事實，不在 bash 再抄一份）。
    //   exit 0 = 相符 / 2 = 不符 / 3 = 未知。放在最前面，避免初始化 tracing/DB。
    {
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).map(String::as_str) == Some("verify-version") {
            let expected = args.get(2).map(String::as_str).unwrap_or("");
            let actual = args.get(3).map(String::as_str).unwrap_or("");
            match version::verify(expected, actual) {
                version::Verdict::Match => {
                    println!("match commit={actual}");
                    std::process::exit(0);
                }
                version::Verdict::Mismatch => {
                    println!("mismatch expected={expected} actual={actual}");
                    std::process::exit(2);
                }
                version::Verdict::Unknown => {
                    println!("unknown expected={expected} actual={actual}");
                    std::process::exit(3);
                }
            }
        }
        // QA 輔助 CLI（非伺服器啟動路徑，比照 verify-version 慣例）：
        // `butfun-server dump-house <rid> <cx> <cz>` → 印出該居民在該錨點的家
        // （確定性樣式 + 方塊清單）JSON 後退出。供「居民搬新家」隔離實測腳本
        // 佈置與伺服器**逐塊一致**的散落舊家（同一份 house_blocks_at，零手抄）。
        if args.get(1).map(String::as_str) == Some("dump-house") {
            let rid = args.get(2).map(String::as_str).unwrap_or("vox_res_0");
            let cx: i32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
            let cz: i32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
            let cy = voxel_building::surface_y(cx, cz);
            let blocks = voxel_building::house_blocks_at(rid, cx, cy, cz);
            println!(
                "{}",
                serde_json::json!({ "resident": rid, "cx": cx, "cy": cy, "cz": cz, "blocks": blocks })
            );
            std::process::exit(0);
        }
    }

    // 開發/正式上線都從 .env 載入秘密(systemd 會用 EnvironmentFile,本機 cargo run 用 dotenvy)。
    let _ = dotenvy::dotenv();
    // 在啟動當下定錨 uptime 起點（LazyLock 首次存取才初始化，不在這摸一下會變成「第一次
    // 有人打 /api/status 才開始計時」）。
    let _ = *SERVER_START;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "butfun_server=info,tower_http=warn".into()),
        )
        .init();

    // Phase 0-E 跨重啟持久化：有 DATABASE_URL 就連 Postgres、套 migration、把玩家位置
    // 載回；沒設則退回 JSONL/記憶體模式（見 db.rs / positions.rs）。連得到但 migration 失敗
    // 視為設定錯誤、直接中止（不要默默跑沒持久化的記憶體模式,免得又像換版洗檔那樣丟資料）。
    // 位置、背包、農地共用同一個連線池（PgPool 內部是 Arc,clone 便宜）：三個 store 各自獨立
    // 載回 / flush,沒有寫入順序耦合（見 0002_inventories.sql / 0003_fields.sql 為何不設外鍵）。
    let (positions, inventories, fields, daynight_store, users, suggestions, tile_store, land_plot_store, npc_memory_store, friends, guilds, sprinkler_persist, sprinkler_preload, tp_store, visit_streaks, welcome_kits) =
        match db::connect()
            .await
            .expect("Postgres 連線或 migration 失敗")
        {
            Some(pool) => {
                tracing::info!(
                    "Postgres 已連線、migration 已套用；\
                     玩家位置/背包/農地/日夜時刻/帳號/建議/地形差異/NPC記憶/好友/公會/灑水器/工程走 DB 持久化"
                );
                let positions = positions::PositionStore::from_pool(pool.clone()).await;
                let inventories = inventory_store::InventoryStore::from_pool(pool.clone()).await;
                let fields = field_store::FieldStore::from_pool(pool.clone()).await;
                let daynight_store = daynight_store::DayNightStore::from_pool(pool.clone()).await;
                let users = users::UserStore::from_pool(pool.clone()).await;
                let suggestions = suggestions::SuggestionStore::from_pool(pool.clone()).await;
                let tile_store = tile_store::TileStore::from_pool(pool.clone()).await;
                let land_plot_store = land_plot_store::LandPlotStore::from_pool(pool.clone()).await;
                let npc_memory_store = npc_memory_store::NpcMemoryStore::from_pool(pool.clone()).await;
                let friends = friends::FriendStore::from_pool(pool.clone()).await;
                let guilds = guild::GuildStore::from_pool(pool.clone()).await;
                let tp_store = town_project_store::TownProjectStore::from_pool(pool.clone()).await;
                let visit_streaks = visit_streak_store::VisitStreakStore::from_pool(pool.clone()).await;
                let welcome_kits = welcome_kit_store::WelcomeKitStore::from_pool(pool.clone()).await;
                let (sp, sp_rows) = sprinkler::SprinklerPersist::from_pool(pool).await;
                (positions, inventories, fields, daynight_store, users, suggestions, tile_store, land_plot_store, npc_memory_store, friends, guilds, sp, sp_rows, tp_store, visit_streaks, welcome_kits)
            }
            None => {
                tracing::warn!(
                    "未設 DATABASE_URL；玩家位置/背包/農地/日夜時刻/帳號/建議/地形差異/NPC記憶/好友/公會/灑水器/工程走記憶體模式"
                );
                (
                    positions::PositionStore::new(),
                    inventory_store::InventoryStore::new(),
                    field_store::FieldStore::new(),
                    daynight_store::DayNightStore::new(),
                    users::UserStore::new(),
                    suggestions::SuggestionStore::new(),
                    tile_store::TileStore::new(),
                    land_plot_store::LandPlotStore::new(),
                    npc_memory_store::NpcMemoryStore::new(),
                    friends::FriendStore::new(),
                    guild::GuildStore::new(),
                    sprinkler::SprinklerPersist::new(),
                    vec![],
                    town_project_store::TownProjectStore::new(),
                    visit_streak_store::VisitStreakStore::new(),
                    welcome_kit_store::WelcomeKitStore::new(),
                )
            }
        };

    let app_state = AppState::with_stores(
        positions,
        inventories,
        fields,
        daynight_store,
        users,
        suggestions,
        tile_store,
        land_plot_store,
        npc_memory_store,
        friends,
        guilds,
        sprinkler_persist,
        sprinkler_preload,
        tp_store,
        visit_streaks,
        welcome_kits,
    );
    if app_state.auth.is_some() {
        tracing::info!("Google OAuth 已啟用(/auth/google/start)");
    } else {
        tracing::warn!("Google OAuth 未設定;走訪客模式(設好 GOOGLE_CLIENT_ID/SECRET/REDIRECT_URI/BUTFUN_SESSION_SECRET 即啟用)");
    }

    // 啟動權威遊戲迴圈。
    game::spawn(app_state.clone());

    // 乙太方界（voxel）AI 居民 tick 迴圈：讓居民活在新世界、會走動、偶爾冒話。
    // 全隔離（自己的 hub/協定），不碰 AppState；嚴守鎖紀律見 voxel_ws.rs。
    voxel_ws::spawn_residents();
    // 種田 v1：每 15 秒檢查農地成熟，自動將幼苗升成熟小麥並廣播。
    voxel_ws::spawn_farm_tick();

    let app = Router::new()
        .route("/healthz", get(health))
        // 版本戳記（堵死「舊 binary 靜默上線」）：回後端編譯期烤入的 git commit + build 時間，
        // 順帶前端 voxel 內容雜湊。輕量、無個資，給人也給腳本（deploy 自驗）讀。見 api_version。
        .route("/version", get(api_version))
        // （封存：2D 的 WebSocket /ws 已移除——前端 game.js 已封存、無人連線。
        //   ws::ws_handler 保留可復原，voxel 用獨立的 /voxel/ws。）
        // 只收建議（POST），刻意不開公開的 GET 清單：建議是玩家送回的回饋（含自選
        // 署名），維護者本就直接讀 `data/suggestions.jsonl` 三角化。先前
        // `GET /api/suggestions` 是未驗身公開端點，會把全部玩家建議整包吐給任何人，
        // 而前端從不消費它（`web/game.js` 只 POST）——等於線上一個沒人用卻能被任意
        // `curl` 撈走所有玩家回饋（含自填署名）的資料曝露點。移除以收口；日後若要做
        // 後台檢視，再走驗身（見 `SuggestionStore::list`）。
        .route("/api/suggestions", post(post_suggestion))
        // 官網（/site/）的伺服器狀態小工具：只吐「線上人數 + 開機秒數」兩個彙總數字，
        // 不含任何玩家身分/位置資訊（公開端點，最小揭露原則）。
        .route("/api/status", get(api_status))
        // 官網即時世界小窗：吐「故鄉星球玩家的去識別化座標 + 城鎮幾何」，讓官網畫
        // 俯瞰活地圖（看得到有人在動）。只回座標數字、不含任何玩家身分（最小揭露）。
        .route("/api/worldview", get(api_worldview))
        // （封存：2D 經濟儀表 /api/economy 已移除——前端無人消費。api_economy 保留可復原。）
        // 登入相關路由
        .merge(auth::auth_router())
        // 個人資料編輯(改顯示名)——需登入,見 profile.rs
        .merge(profile::profile_router())
        // 外觀自訂(捏臉)——需登入,見 appearance.rs
        .merge(appearance::appearance_router())
        // 封存舊世界 + 統一正門（2026-06-30 維護者「只剩乙太方界、進遊戲統一 /」）：
        // 「/」直接服務乙太方界——voxel 首頁的 <base href="/voxel/"> 讓相對資源(main.js)
        // 仍解析到 /voxel/，WS/fetch 全用絕對路徑不受影響。舊 3D 入口(/3d/、/play3d/)307轉回 /。
        // serve_index/serve_3d_index/serve_play3d_index + 2D/3D 前後端 + 玩家資料全保留可復原。
        .route("/", get(serve_voxel_index))
        .route("/index.html", get(serve_voxel_index))
        .route("/3d/", get(|| async { axum::response::Redirect::temporary("/") }))
        .route("/3d/index.html", get(|| async { axum::response::Redirect::temporary("/") }))
        .route("/play3d/", get(|| async { axum::response::Redirect::temporary("/") }))
        .route("/play3d/index.html", get(|| async { axum::response::Redirect::temporary("/") }))
        // AI 生態世界 voxel 基底（切片①）：新頁 /voxel/ + 獨立 WS /voxel/ws，全隔離、
        // additive，與現有 2D/3D 協定零交集（見 voxel.rs / voxel_ws.rs）。
        .route("/voxel/", get(serve_voxel_index))
        .route("/voxel/index.html", get(serve_voxel_index))
        // PWA（乙太方界可安裝 / 加到主畫面像 App）：
        //   - /manifest.webmanifest：Web App Manifest（實檔在 web/voxel/，這裡顯式服務到根路徑，
        //     讓 start_url=/ 的 manifest link 抓得到）。
        //   - /sw.js：Service Worker，以「根 scope」服務——no-cache（更新看得到）+
        //     `Service-Worker-Allowed: /`，讓它能同時控制 / 與 /voxel/ 兩個入口。
        //   - 圖示 /voxel/icons/* 由既有 fallback 靜態(ServeDir)服務（web/voxel/icons/）。
        .route("/manifest.webmanifest", get(serve_manifest))
        .route("/sw.js", get(serve_service_worker))
        .route("/voxel/ws", get(voxel_ws::voxel_ws_handler))
        // 乙太方界·居民日記（ROADMAP 650）：讀取記憶 + 心願格式化成日記頁 JSON。
        .route("/voxel/diary", get(voxel_ws::voxel_diary_handler))
        // 乙太方界·城鎮動態 Feed（ROADMAP 655）：最新 30 筆居民活動事件 JSON。
        .route("/voxel/feed", get(voxel_ws::voxel_feed_handler))
        // 乙太方界·好感度（ROADMAP 656）：玩家與各居民的互動記憶筆數（好感度計數）JSON。
        .route("/voxel/affinity", get(voxel_ws::voxel_affinity_handler))
        // 乙太方界·居民交情網（ROADMAP 708）：居民彼此兩兩情誼層級 JSON，讓小社會被玩家看見。
        .route("/voxel/relations", get(voxel_ws::voxel_relations_handler))
        // 乙太方界·小圈子攤開（自主提案切片，接續 708+711）：彼此皆為老朋友的圈子清單 JSON。
        .route("/voxel/cliques", get(voxel_ws::voxel_cliques_handler))
        // 乙太方界·居民技能簿（ROADMAP 719）：每位居民已發明/學會的技能名清單 JSON。
        .route("/voxel/skills", get(voxel_ws::voxel_skills_handler))
        // 乙太方界·玩家里程碑（ROADMAP 724）：玩家自己的成就徽章清單 JSON。
        .route("/voxel/milestones", get(voxel_ws::voxel_milestones_handler))
        // 乙太方界·村莊地圖（自主提案切片，ROADMAP 837）：中心/廣場/主路尺寸＋沿路地塊認領 JSON。
        .route("/voxel/village-map", get(voxel_ws::voxel_village_map_handler))
        // 乙太方界·探索紀事（自主提案切片，接續 838/839）：這位玩家找到過的地標座標清單 JSON。
        .route("/voxel/discoveries", get(voxel_ws::voxel_discoveries_handler))
        // 乙太方界·玩家熟練度（自主提案切片，ROADMAP 842）：三條熟練度目前經驗/等級/稱號 JSON。
        .route("/voxel/mastery", get(voxel_ws::voxel_mastery_handler))
        // 乙太方界·玩家獨門配方（自主提案切片，ROADMAP 849）：居民教過哪些獨門配方 JSON。
        .route("/voxel/known_recipes", get(voxel_ws::voxel_known_recipes_handler))
        // 乙太方界·個人路標（自主提案切片，ROADMAP 869）：這位玩家目前所有路標 JSON。
        .route("/voxel/waypoints", get(voxel_ws::voxel_waypoints_handler))
        // 其餘路徑（game.js、assets、wasm…）交給靜態前端（web/）。game.js 維持可
        // 快取——它的 URL 帶內容雜湊，內容一變 URL 就變，CF/瀏覽器自然抓新版。
        .fallback_service(ServeDir::new("web"))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state.clone());

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("無法綁定連接埠");
    tracing::info!("ButFun 伺服器啟動於 http://{addr}");

    // 優雅關機:收到 SIGTERM(deploy 重啟)或 Ctrl-C 時,先停收新連線,再把全部狀態最後
    // flush 一次,才退出。否則換版重啟會丟掉上次週期 flush 之後、線上玩家最多約 10 秒的進度
    // (見 game::flush_all)。flush 是冪等 upsert,多寫一次永遠安全。
    let flush_state = app_state.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("伺服器執行失敗");
    tracing::info!("收到關機訊號;退出前最後一次落地玩家狀態…");
    game::flush_all(&flush_state).await;
    tracing::info!("狀態已落地,伺服器關閉");
}

/// 等待關機訊號:Unix 上同時聽 SIGTERM(systemd/deploy 重啟用)與 Ctrl-C;
/// 非 Unix 只聽 Ctrl-C。任一觸發即返回,交還主流程做最後 flush。
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            // 裝不上 SIGTERM 處理器極罕見;退而只靠 Ctrl-C,別讓伺服器起不來。
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

async fn health() -> &'static str {
    "ok"
}

/// `/version`：後端版本戳記。回 `{commit, built_at, voxel}`：
///   - `commit`   = 編譯期烤進 binary 的 git short SHA（build.rs 注入；抓不到 git 時 "unknown"）。
///   - `built_at` = 編譯期 build 時間（UTC）。
///   - `voxel`    = 前端 voxel main.js 的內容雜湊（與 serve_voxel_index 注入 `__BUILD__` 同算法；
///                  讀不到就 null，不擋端點）。
///
/// 輕量、無個資。給人也給腳本讀：scripts/deploy.sh 自驗 curl 此端點取 `commit` 比對目標 commit
/// →「舊 binary 靜默上線」會被當場抓到；前端 ?debug=1 HUD fetch 此端點顯示「後端 <commit>」，
/// 與前端內容雜湊並列 → 前後端版本都對得上 origin/main = 全上線了，一眼看出。回 no-store（永遠新鮮）。
async fn api_version() -> impl IntoResponse {
    let voxel = std::fs::read("web/voxel/main.js")
        .ok()
        .map(|b| content_version12(&b));
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
            "commit": version::GIT_SHA,
            "built_at": version::BUILD_TIME,
            "voxel": voxel,
        })),
    )
}

/// 把 HTML 裡的 `game.js?v=...` 版本字串換成「game.js 內容的 sha256 前 12 個 hex 字元」。
///
/// 根治「前端部署後玩家約 4h 看不到新版」的快取 bug：原本 index.html 寫死
/// `game.js?v=20260610-leaderboard`（手動、卡在 6/10、沒人更新），而 `/` 走純
/// `ServeDir` 靜態 serve、URL 不隨 game.js 內容變→Cloudflare(HIT,max-age 14400)＋
/// 瀏覽器快取會持續送舊的 game.js。改成內容雜湊後，game.js 內容一變版本字串就變→
/// URL 一變→CF/瀏覽器自然抓新版，立刻到位。
///
/// 穩健替換：找每一處 `game.js?v=` 後面到下一個 `"` 為止那段，整段換成新雜湊；
/// 找不到 `game.js?v=` 就原樣返回（不硬塞）。抽成純函式好測。
/// 內容雜湊版本字串：sha256 前 6 bytes → 12 個 hex 字元。夠長到不碰撞、夠短到 URL 乾淨。
/// game.js／main.js 的 `?v=` 版本注入、`window.__BUILD__`、`/version` 端點全部共用同一算法。
fn content_version12(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn inject_gamejs_version(html: &str, gamejs: &[u8]) -> String {
    let version = content_version12(gamejs);

    let needle = "game.js?v=";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(pos) = rest.find(needle) {
        // 寫入 needle（含）之前的內容 + needle 本身。
        let after = pos + needle.len();
        out.push_str(&rest[..after]);
        // 從 needle 之後找下一個 `"`，那之間是舊版本字串，整段換成新雜湊。
        let tail = &rest[after..];
        match tail.find('"') {
            Some(q) => {
                out.push_str(&version);
                rest = &tail[q..]; // 保留 `"` 起繼續掃（可能有多處）
            }
            // 沒有結尾 `"`（理論上不會）：保守起見原樣接上、停止替換。
            None => {
                out.push_str(tail);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 啟動時算一次並快取的首頁 HTML（game.js 版本已換成內容雜湊）。
/// 用 `LazyLock` 確保只讀檔/算雜湊一次，避免每請求摸大檔。
/// server cwd＝repo 根，相對路徑 `web/game.js`、`web/index.html` 可讀；deploy 重啟
/// server → 每次部署自動重算雜湊。讀檔失敗時退回原樣 index.html（不 panic、不擋服務）。
static INDEX_HTML: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let html = match std::fs::read_to_string("web/index.html") {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("讀 web/index.html 失敗，serve_index 將回空白：{e}");
            return String::new();
        }
    };
    match std::fs::read("web/game.js") {
        Ok(gamejs) => {
            let injected = inject_gamejs_version(&html, &gamejs);
            tracing::info!("serve_index：已把 index.html 的 game.js 版本注入為內容雜湊");
            injected
        }
        // 讀不到 game.js（理論上不會）：退回原樣 index.html，至少首頁能出。
        Err(e) => {
            tracing::warn!("讀 web/game.js 失敗，index.html 沿用原版本字串：{e}");
            html
        }
    }
});

/// 首頁 handler：回「已注入 game.js 內容雜湊」的 index.html，並帶 no-cache 標頭。
/// HTML 永遠新鮮（極小、無快取）；game.js 的 URL 隨內容雜湊變，照舊可被快取。
#[allow(dead_code)] // 封存:2D/3D入口已轉址,handler保留可復原
async fn serve_index() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, must-revalidate"),
        ],
        INDEX_HTML.as_str(),
    )
}

/// 把 3D 頁 HTML 裡的 `main.js?v=...` 版本字串換成 main.js 內容的 sha256 前 12 hex，
/// 並在 `</head>` 前插入 `<script>window.__BUILD__="<hash>";</script>`，
/// 讓前端 JS 可在 `?debug=1` 的偵錯 HUD 裡顯示版本號，一眼確認是最新版。
///
/// 邏輯與 `inject_gamejs_version` 完全對稱：同樣算 sha256、同樣只換 `?v=` 後到 `"` 之間；
/// 差別僅在 needle 是 `main.js?v=`，並額外注入 `window.__BUILD__`。
/// 抽成純函式便於測試（不碰磁碟）。
fn inject_mainjs_version(html: &str, mainjs: &[u8]) -> String {
    let version = content_version12(mainjs);

    // 第一步：替換所有 main.js?v=<舊版> 為 main.js?v=<hash>（邏輯同 inject_gamejs_version）。
    let needle = "main.js?v=";
    let mut out = String::with_capacity(html.len() + 80);
    let mut rest = html;
    while let Some(pos) = rest.find(needle) {
        let after = pos + needle.len();
        out.push_str(&rest[..after]);
        let tail = &rest[after..];
        match tail.find('"') {
            Some(q) => {
                out.push_str(&version);
                rest = &tail[q..]; // 保留 `"` 繼續掃（避免漏掉多處）
            }
            None => {
                out.push_str(tail);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);

    // 第二步：在 </head> 前插入 window.__BUILD__，供前端偵錯 HUD 讀版本號。
    let build_tag = format!("<script>window.__BUILD__=\"{version}\";</script>");
    if let Some(pos) = out.find("</head>") {
        out.insert_str(pos, &build_tag);
    }
    out
}

/// `/3d/`、`/3d/index.html` 的 handler：每次請求即時讀檔並注入 main.js 內容雜湊版本。
///
/// 根治「前端改動後玩家看到舊版」的問題：
/// - 舊做法：啟動時讀 index.html 一次（LazyLock）+ 手動 `?v=N`
///   → 改前端不重啟看不到新版；手動版本號忘記改就永久卡舊版。
/// - 新做法：每次請求讀 index.html + 算 web/3d/main.js 的 sha256，
///   把 `main.js?v=<任何舊值>` 換成 `main.js?v=<雜湊>`，並注入 `window.__BUILD__`；
///   前端改了、雜湊就變、URL 就變、CF/瀏覽器就抓新版——無需重啟伺服器。
/// index.html 帶 no-cache，main.js 本體仍走 ServeDir 可被快取。
#[allow(dead_code)] // 封存:2D/3D入口已轉址,handler保留可復原
async fn serve_3d_index() -> impl IntoResponse {
    let html = match std::fs::read_to_string("web/3d/index.html") {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("讀 web/3d/index.html 失敗：{e}");
            String::new()
        }
    };
    let body = match std::fs::read("web/3d/main.js") {
        Ok(mainjs) => {
            tracing::debug!("serve_3d_index：已把 index.html 的 main.js 版本注入為內容雜湊");
            inject_mainjs_version(&html, &mainjs)
        }
        Err(e) => {
            tracing::warn!("讀 web/3d/main.js 失敗，index.html 沿用原版本字串：{e}");
            html
        }
    };
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, must-revalidate"),
        ],
        body,
    )
}

/// `/play3d/`、`/play3d/index.html` 的 handler：同 `serve_3d_index`，對 web/play3d/main.js 算雜湊。
#[allow(dead_code)] // 封存:2D/3D入口已轉址,handler保留可復原
async fn serve_play3d_index() -> impl IntoResponse {
    let html = match std::fs::read_to_string("web/play3d/index.html") {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("讀 web/play3d/index.html 失敗：{e}");
            String::new()
        }
    };
    let body = match std::fs::read("web/play3d/main.js") {
        Ok(mainjs) => {
            tracing::debug!("serve_play3d_index：已把 index.html 的 main.js 版本注入為內容雜湊");
            inject_mainjs_version(&html, &mainjs)
        }
        Err(e) => {
            tracing::warn!("讀 web/play3d/main.js 失敗，index.html 沿用原版本字串：{e}");
            html
        }
    };
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, must-revalidate"),
        ],
        body,
    )
}

/// `/voxel/`、`/voxel/index.html` 的 handler：同 `serve_3d_index`，對 web/voxel/main.js 算雜湊。
/// AI 生態世界 voxel 基底的新前端頁，與現有頁完全並行、互不影響。
async fn serve_voxel_index() -> impl IntoResponse {
    let html = match std::fs::read_to_string("web/voxel/index.html") {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("讀 web/voxel/index.html 失敗：{e}");
            String::new()
        }
    };
    let body = match std::fs::read("web/voxel/main.js") {
        Ok(mainjs) => {
            tracing::debug!("serve_voxel_index：已把 index.html 的 main.js 版本注入為內容雜湊");
            inject_mainjs_version(&html, &mainjs)
        }
        Err(e) => {
            tracing::warn!("讀 web/voxel/main.js 失敗，index.html 沿用原版本字串：{e}");
            html
        }
    };
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, must-revalidate"),
        ],
        body,
    )
}

/// PWA Web App Manifest：實檔在 `web/voxel/manifest.webmanifest`，這裡顯式服務到根路徑
/// `/manifest.webmanifest`（index.html 的 `<link rel="manifest">` 用絕對根路徑抓）。
/// no-cache 讓調整 manifest（名稱/圖示/色）後玩家立刻拿到新版。
async fn serve_manifest() -> impl IntoResponse {
    match std::fs::read("web/voxel/manifest.webmanifest") {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/manifest+json; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache, must-revalidate"),
            ],
            bytes,
        ),
        Err(e) => {
            tracing::error!("讀 web/voxel/manifest.webmanifest 失敗：{e}");
            (
                StatusCode::NOT_FOUND,
                [
                    (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                    (header::CACHE_CONTROL, "no-cache"),
                ],
                b"manifest not found".to_vec(),
            )
        }
    }
}

/// PWA Service Worker：實檔在 `web/voxel/sw.js`，顯式服務到根路徑 `/sw.js`，取得「根 scope」。
/// 額外送 `Service-Worker-Allowed: /`（即使日後改到子路徑供應也允許控制根），並 no-cache
/// （sw 一改就更新，配合 sw 內 skipWaiting + 清舊快取，避免玩家卡舊版）。
async fn serve_service_worker() -> impl IntoResponse {
    let swa = header::HeaderName::from_static("service-worker-allowed");
    match std::fs::read("web/voxel/sw.js") {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache, must-revalidate"),
                (swa, "/"),
            ],
            bytes,
        ),
        Err(e) => {
            tracing::error!("讀 web/voxel/sw.js 失敗：{e}");
            (
                StatusCode::NOT_FOUND,
                [
                    (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                    (header::CACHE_CONTROL, "no-cache"),
                    (header::HeaderName::from_static("service-worker-allowed"), "/"),
                ],
                b"sw not found".to_vec(),
            )
        }
    }
}

/// 行程啟動時刻（算 uptime 用）。`LazyLock` 在 main 啟動早期第一次被讀到時定錨。
static SERVER_START: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

/// 官網狀態小工具用的公開彙總：線上人數 + 開機秒數。刻意不含玩家名單/位置等
/// 任何個體資訊（公開端點，最小揭露）。
async fn api_status(State(app): State<AppState>) -> impl IntoResponse {
    let online = app.players.read().map(|p| p.len()).unwrap_or(0);
    // ROADMAP 445：彙整「世界此刻」一瞥（時辰／季節／天氣），讓登入畫面映出當下世界。
    // 全是全域世界狀態（公開、本就互相可見），不含任何玩家身分／座標，守最小揭露。
    let phase = app
        .daynight
        .read()
        .map(|d| d.phase())
        .unwrap_or(crate::daynight::Phase::Day);
    let season = app
        .season
        .read()
        .map(|s| s.current)
        .unwrap_or(crate::season::Season::Spring);
    let weather = app
        .weather
        .read()
        .map(|w| w.weather_type)
        .unwrap_or(crate::weather::WeatherType::Clear);
    let glimpse = crate::world_glimpse::compose(phase, season, weather, online);
    Json(serde_json::json!({
        "online": online,
        "uptime_secs": SERVER_START.elapsed().as_secs(),
        "glimpse": {
            "theme": glimpse.theme,
            "headline": glimpse.headline,
            "subline": glimpse.subline,
        },
    }))
}

/// 經濟儀表（ROADMAP 108）：彙總商隊金庫與乙太流量資訊，供維護者調參用。
/// 只回彙總數字，不含玩家身分或個別玩家乙太（最小揭露原則）。
#[allow(dead_code)] // 封存:2D /api/economy路由已移除,handler保留
async fn api_economy(State(app): State<AppState>) -> impl IntoResponse {
    let snap = app.npc_treasury.read().unwrap().snapshot();
    let online = app.players.read().map(|p| p.len()).unwrap_or(0);
    // 線上玩家乙太總量（匿名加總，不含身分）
    let online_ether_total: u64 = app.players.read()
        .map(|p| p.values().map(|pl| pl.ether as u64).sum())
        .unwrap_or(0);
    let uptime_secs = SERVER_START.elapsed().as_secs();

    let treasury: serde_json::Value = {
        let mut m = serde_json::Map::new();
        for (name, balance, max) in &snap.merchants {
            m.insert(name.to_string(), serde_json::json!({ "balance": balance, "max": max }));
        }
        m.into()
    };

    let net = snap.lifetime_injected as i64
        - snap.lifetime_paid_to_players as i64
        - snap.lifetime_supply_cost as i64;

    Json(serde_json::json!({
        "treasury": treasury,
        "faucet": {
            "lifetime_injected": snap.lifetime_injected,
            "restock_interval_secs": crate::npc_treasury::RESTOCK_INTERVAL_SECS,
        },
        "drain": {
            "lifetime_paid_to_players": snap.lifetime_paid_to_players,
            "lifetime_supply_cost": snap.lifetime_supply_cost,
        },
        "net_ether_delta": net,
        "online_players": online,
        "online_ether_total": online_ether_total,
        "uptime_secs": uptime_secs,
    }))
}

/// 官網即時世界小窗的資料源。回故鄉星球（home）線上玩家的「去識別化座標」
/// （只有 x/y 數字，**不含 id / 名字 / 任何身分**——多人公開世界裡位置本就互相可見，
/// 這裡比照最小揭露只給點）＋ 城鎮幾何（世界像素的中心與半徑），讓官網畫俯瞰活地圖。
async fn api_worldview(State(app): State<AppState>) -> impl IntoResponse {
    let players: Vec<[f32; 2]> = app
        .players
        .read()
        .map(|m| {
            m.values()
                .filter(|p| p.planet == state::PLANET_HOME)
                .map(|p| [p.x, p.y])
                .collect()
        })
        .unwrap_or_default();
    let towns: Vec<serde_json::Value> = world_core::TOWNS
        .iter()
        .map(|t| {
            let px = (t.cgx as f32 + 0.5) * world_core::TILE_PX;
            let py = (t.cgy as f32 + 0.5) * world_core::TILE_PX;
            let half = t.half_tiles as f32 * world_core::TILE_PX;
            serde_json::json!({ "x": px, "y": py, "half": half, "name": t.name })
        })
        .collect();
    Json(serde_json::json!({ "players": players, "towns": towns }))
}

/// 收到一則玩家建議。內容清乾淨後若為空（全空白 / 全控制字元）回 400、不存——
/// 擋空的判斷下沉到 `add`（依實際會被存下的內容），不是只對 raw 輸入 `trim`。
/// 建議箱每 IP 速率限制（H3 安全強化）：防匿名腳本無限 POST 灌爆 suggestions 表 / 撐爆磁碟。
/// Cloudflare tunnel 後真實 IP 在 `CF-Connecting-IP`；近似計數（每分鐘窗、每 IP ≤ 3 則）。
fn suggest_rate_ok(ip: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static RL: OnceLock<Mutex<HashMap<String, (u64, u32)>>> = OnceLock::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let min = now / 60;
    let mut map = RL.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if map.len() > 20000 {
        map.clear(); // 防 map 無限長大
    }
    let e = map.entry(ip.to_string()).or_insert((min, 0));
    if e.0 != min {
        *e = (min, 0);
    }
    e.1 += 1;
    e.1 <= 3
}

async fn post_suggestion(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(new): Json<NewSuggestion>,
) -> impl IntoResponse {
    // H3：每 IP 速率限制。Cloudflare tunnel 後真實 IP 在 CF-Connecting-IP（退而求其次 X-Forwarded-For）。
    let ip = headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    if !suggest_rate_ok(&ip) {
        return (StatusCode::TOO_MANY_REQUESTS, "建議送太頻繁了，請稍後再試").into_response();
    }
    match app.suggestions.add(new).await {
        Some(saved) => (StatusCode::CREATED, Json(saved)).into_response(),
        None => (StatusCode::BAD_REQUEST, "建議內容不可為空").into_response(),
    }
}

// 註：刻意不再提供 `list_suggestions` HTTP handler——建議清單不對外公開（見上方路由註解）。

#[cfg(test)]
mod tests {
    use super::{inject_gamejs_version, inject_mainjs_version};

    /// sha256(content) 前 12 hex 字元——測試用的期望版本算法（與函式一致）。
    fn expected_version(gamejs: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(gamejs)
            .iter()
            .take(6)
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn 替換舊版本字串為內容雜湊() {
        let html = r#"<html><body><script src="game.js?v=20260610-leaderboard"></script></body></html>"#;
        let gamejs = b"console.log('hello butfun');";
        let out = inject_gamejs_version(html, gamejs);

        let ver = expected_version(gamejs);
        // 新版本字串應出現，舊的應消失。
        assert!(out.contains(&format!("game.js?v={ver}")), "應注入內容雜湊版本: {out}");
        assert!(!out.contains("20260610-leaderboard"), "舊版本字串應被換掉: {out}");
        // 雜湊取 12 個 hex 字元。
        assert_eq!(ver.len(), 12);
    }

    #[test]
    fn 雜湊隨內容變而變() {
        let html = r#"<script src="game.js?v=old"></script>"#;
        let a = inject_gamejs_version(html, b"version A");
        let b = inject_gamejs_version(html, b"version B");
        assert_ne!(a, b, "不同 game.js 內容應產生不同版本字串");

        // 同內容應穩定（同 HTML 同內容 → 同輸出）。
        let a2 = inject_gamejs_version(html, b"version A");
        assert_eq!(a, a2, "相同內容應產生相同版本字串");
    }

    #[test]
    fn 替換多處且保留其餘html() {
        let html = r#"<a href="game.js?v=x">a</a> mid <script src="game.js?v=y"></script>"#;
        let gamejs = b"abc";
        let out = inject_gamejs_version(html, gamejs);
        let ver = expected_version(gamejs);
        // 兩處都換成同一雜湊。
        let count = out.matches(&format!("game.js?v={ver}")).count();
        assert_eq!(count, 2, "兩處 game.js?v= 都應被替換: {out}");
        // 其餘文字（mid）原樣保留。
        assert!(out.contains(" mid "), "非版本內容應保留: {out}");
    }

    #[test]
    fn 沒有版本字串時原樣返回() {
        let html = "<html>no script here</html>";
        let out = inject_gamejs_version(html, b"whatever");
        assert_eq!(out, html, "沒有 game.js?v= 應原樣返回");
    }

    // ---- inject_mainjs_version 測試 ----

    #[test]
    fn mainjs_替換舊版本字串為內容雜湊() {
        let html = r#"<html><head></head><body><script type="module" src="main.js?v=17"></script></body></html>"#;
        let mainjs = b"console.log('butfun 3d');";
        let out = inject_mainjs_version(html, mainjs);
        let ver = expected_version(mainjs);
        // main.js?v= 應被換成內容雜湊。
        assert!(out.contains(&format!("main.js?v={ver}")), "應注入內容雜湊版本: {out}");
        assert!(!out.contains("?v=17"), "舊版本字串應被換掉: {out}");
        // window.__BUILD__ 應被注入。
        assert!(out.contains(&format!("window.__BUILD__=\"{ver}\"")), "應注入 window.__BUILD__: {out}");
        // 注入點在 </head> 之前。
        let build_pos = out.find("window.__BUILD__").expect("找不到 __BUILD__");
        let head_pos = out.find("</head>").expect("找不到 </head>");
        assert!(build_pos < head_pos, "__BUILD__ 應在 </head> 之前: {out}");
    }

    #[test]
    fn mainjs_雜湊隨內容變而變() {
        let html = r#"<html><head></head><body><script type="module" src="main.js?v=1"></script></body></html>"#;
        let a = inject_mainjs_version(html, b"version A");
        let b = inject_mainjs_version(html, b"version B");
        assert_ne!(a, b, "不同 main.js 內容應產生不同版本字串");
    }

    #[test]
    fn mainjs_沒有版本字串時仍注入build標籤() {
        // 沒有 main.js?v= 的 HTML：版本替換跳過，但 __BUILD__ 仍應注入（有 </head>）。
        let html = "<html><head></head><body>no script</body></html>";
        let out = inject_mainjs_version(html, b"js content");
        let ver = expected_version(b"js content");
        // 沒有 main.js?v= 可替換，原樣通過。
        assert!(!out.contains("main.js?v="), "沒有 main.js?v= 不應憑空插入: {out}");
        // __BUILD__ 仍應注入。
        assert!(out.contains(&format!("window.__BUILD__=\"{ver}\"")), "即使無 main.js?v= 也應注入 __BUILD__: {out}");
    }
}
