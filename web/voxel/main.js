// ============================================================
// ButFun Voxel 前端（AI 生態世界 voxel 基底·切片①）
// ============================================================
// 後端權威、前端只渲染：收伺服器串來的 chunk（方塊資料）→ 合併 mesh（面剔除）→
// 玩家能走在地形上（重力 + voxel 逐軸 AABB 碰撞）→ 第三人稱鏡頭跟隨 + 鍵盤/觸控。
//
// 效能鐵律：**一個 chunk 一個合併 BufferGeometry**（面剔除去掉看不見的內面），
// 絕不每方塊一個 mesh（記取 #614 教訓）。
//
// 全隔離：只連 /voxel/ws、用 voxel 自己的 JSON 協定，不碰現有 2D/3D 任何東西。
// 不抄外部碼；全繁中註解；node --check 過。

import * as THREE from "three";

// ── 常數（與後端 voxel.rs 對齊）──────────────────────────────────────────────
const CHUNK = 16; // 一 chunk 邊長（方塊數），與 voxel::CHUNK 一致
// 方塊型別（對齊 Block enum）
const AIR = 0, GRASS = 1, DIRT = 2, STONE = 3, SAND = 4, WOOD = 5, LEAVES = 6, WATER = 7;
// 合成台 v1（ROADMAP 658）——玩家合成而得，不自然生成
const PLANK = 8, STONE_BRICK = 9, GLASS = 10;
// 種田 v1（ROADMAP 659）——農地狀態方塊 + 種子物品
const FARM_SOIL = 11, FARM_SOIL_SEEDED = 12, WHEAT_MATURE = 13;
const SEEDS = 14; // 純物品（無對應方塊），從葉片/收割掉落
// 工作台 v1（ROADMAP 665）——玩家合成+放置，互動開 3×3 合成格
const WORKBENCH = 15;
// 熔爐 v1（ROADMAP 666）——工作台合成+放置，互動開冶煉面板
const FURNACE = 16;
// 拋光石 v1（ROADMAP 666）——熔爐冶煉所得，精緻灰石建材
const SMOOTH_STONE = 17;
// 麵包 v1（ROADMAP 668）——純物品，收割小麥→合麵包；18=小麥顆粒,19=麵包
const WHEAT = 18, BREAD = 19;
// 深層礦石 v1（ROADMAP 682）——地底石層採掘所得
const COAL_ORE = 20; // 煤礦——最淺的礦石，y ≤ 3 的石層有機率生成
const IRON_ORE = 21; // 鐵礦——更深更稀少，y ≤ 1 的石層有機率生成
// 鐵錠 v1（ROADMAP 683）——熔爐冶煉所得（1 鐵礦 + 1 煤礦 → 2 鐵錠）
const IRON_INGOT = 22;
// 鐵磚 v1（ROADMAP 684）——工作台合成（4 鐵錠 → 1 鐵磚）
const IRON_BLOCK = 23;
// 流動水（水流動模擬）——來源水 WATER=7 是 level 0/無限；24..=30 是流動 level 1..=7。
// 非實心（可穿越、水面渲染同來源水），玩家不可放置（伺服器模擬維護的狀態方塊）。
const WATER_FLOW_BASE = 24, WATER_FLOW_MAX_LVL = 7;
// 任一方塊 id 是否為「水」（來源或流動）——渲染與碰撞都把兩者當水看待。
function isWaterId(b) { return b === WATER || b === HOT_SPRING_WATER || (b >= WATER_FLOW_BASE && b < WATER_FLOW_BASE + WATER_FLOW_MAX_LVL); }
// 火把 v1（ROADMAP 685）——背包合成（1 木頭 + 1 煤礦 → 4 火把）；橘黃光源，礦坑標記用
const TORCH = 31;
// 鎬具 v1（ROADMAP 687）——純物品，不可放置；提升石/礦採集速度
const PICKAXE_WOOD = 32, PICKAXE_STONE = 33, PICKAXE_IRON = 34;
// 梯子 v1（ROADMAP 688）——可放置；玩家進入方格後自動抓握、取消重力可垂直攀爬
const LADDER = 35;
// 斧頭 v1（ROADMAP 689）——純物品，不可放置；持斧砍木頭/葉片/木板大幅加速
const AXE_WOOD = 36, AXE_STONE = 37, AXE_IRON = 38;
// 鏟子 v1（ROADMAP 690）——純物品，不可放置；持鏟挖草地/泥土/沙地/農田大幅加速
const SHOVEL_WOOD = 39, SHOVEL_STONE = 40, SHOVEL_IRON = 41;
// 箱子 v1（ROADMAP 692）——工作台合成（8 木板 → 1 箱子）；放置後右鍵互動開儲物面板
const CHEST = 42;
// 木門 v1（ROADMAP 693）——背包 2×2 合成（4 木板 → 2 門）；右鍵切換開/關，DoorOpen 非實心可穿越
const DOOR_CLOSED = 43, DOOR_OPEN = 44;
// 床 v1——背包 2×2 合成（3 木板 + 3 葉片 → 1 床）；右鍵夜晚睡覺跳過黑夜到隔天黎明
const BED = 45;
// 第二種作物 v1——種田系統第一次有兩種作物可選：胡蘿蔔比小麥快熟（60s/水耕30s vs 90s/45s）。
// 胡蘿蔔幼苗/成熟胡蘿蔔為伺服器狀態方塊；種子/收成為純物品，從草地(GRASS)破壞額外掉落種子。
const CARROT_SEEDED = 46, CARROT_MATURE = 47;
const CARROT_SEEDS = 48, CARROT = 49;
// 第三種作物 v1——馬鈴薯比小麥慢熟但收成量大（120s/水耕60s，收成×2）。
// 馬鈴薯幼苗/成熟馬鈴薯為伺服器狀態方塊；種子/收成為純物品，從泥土(DIRT)破壞額外掉落種子（與胡蘿蔔取自草地區隔）。
const POTATO_SEEDED = 50, POTATO_MATURE = 51;
const POTATO_SEEDS = 52, POTATO = 53;
// 仙人掌 v1（生物群系第一刀）——沙漠群系程序生成，2格高；採集後可放置
const CACTUS = 54;
// 雪 v1（生物群系第二刀）——雪原群系地表覆蓋（取代草）；採集後可放置，白色建材
const SNOW = 55;
// 冰晶 v1（雪原冰晶採集）——雪原群系地表稀疏珍寶，1格高；採集後可放置，送居民有珍愛反應
const ICE_CRYSTAL = 56;
// 冰晶燈 v1（冰晶合成）——背包 2×2：1 冰晶 + 2 玻璃 → 1 冰晶燈；泛冷藍幽光的裝飾燈，蓋冰屋的建造回報
const ICE_LANTERN = 57;
// 乙太礦 v1（乙太礦脈）——世界最深層（y≤0）極稀有生成，青藍寶礦；採集後可放置，合成乙太燈的核心材料
const AETHER_ORE = 58;
// 乙太燈 v1（乙太礦脈）——工作台 3×3：1 乙太礦 + 4 玻璃 → 1 乙太燈；散發清冷青藍光的高階光源（真實動態光照）
const AETHER_LAMP = 59;
// 垂釣 v1（ROADMAP 734）：釣竿(60)/小魚(61)/乙太魚(62) 皆純物品，住背包不可放置
const FISHING_ROD = 60, FISH = 61, AETHER_FISH = 62;
// 烤魚 v1：生魚(61)在熔爐烤成烤魚(63)，居民最愛的美味贈禮（純物品不放置）
const COOKED_FISH = 63;
// 烤地薯 v1：生馬鈴薯(53)在熔爐烤成烤地薯(64)，居民最愛的美味贈禮（純物品不放置）
const BAKED_POTATO = 64;
// 野菜暖湯 v1（ROADMAP 778）：胡蘿蔔(49)+馬鈴薯(53)+小麥(18) 在工作台煮成暖湯(67)，
// 乙太方界第一道「多食材料理」，居民最珍視的餽贈（純物品不放置）
const STEW = 67;
// 乙太煙火 v1（ROADMAP 785）——工作台：1 乙太礦(58)+2 煤礦(20)+2 沙(4) → 3 乙太煙火(68)；
// 純物品不可放置，朝夜空施放（firework_launch）即消耗，火花在頭頂綻放、附近居民抬頭歡呼。
const FIREWORK = 68;
// 乙太沃肥 v1（ROADMAP 789）——工作台：3 雜草(1)+2 泥土(2) → 2 乙太沃肥(69)；純物品不可放置，
// 手持對準一株幼苗(12/46/50)一撒，作物生長計時往前跳一截（fertilize）——玩家主動催熟農業的動詞。
const FERTILIZER = 69;
// 乙太營火 v1（自主提案切片）——工作台：3 石頭(3)+2 木頭(5)+1 煤礦(20) → 1 營火(70)；
// 可放置的發光方塊，放下即向四周散出溫暖橘光照亮營地，入夜後吸引路過居民駐足圍暖。
const CAMPFIRE = 70;
// 植樹造林 v1（ROADMAP 738）——砍天然樹葉有機率掉樹苗(65)，種在土地上約 150 秒長成一株樹。
// 樹苗既是背包物品也是可放置方塊（item_id == block_id），是玩家第一個可再生木材來源。
const SAPLING = 65;
// 告示牌 v1（ROADMAP 740）——2 木板合成，放置後右鍵寫一行短字，浮在牌上人人看得見。
// 既是背包物品也是可放置方塊（item_id == block_id）。
const SIGN = 66;
// 水桶 v1（自主提案切片）——工作台：3 鐵錠 → 1 空水桶(71)；純物品不可放置。
// 手持空水桶對準水源＝舀水（→滿水桶72）；手持滿水桶對準空格＝倒出一格永久來源水（→空水桶）。
// 接上既有水流模擬與鄰水加速種田，玩家第一次能親手搬水、把荒地改造成綠洲水田。
const BUCKET = 71, WATER_BUCKET = 72;
// 鋤頭 v1（自主提案切片）——背包：2 木頭 + 1 木板 → 1 木鋤頭(73)；純物品不可放置。
// 手持鋤頭對準草地／泥土＝就地開墾成農田土（省去挖土搬工作台再放回的繞路）；與水桶成對。
const HOE = 73;
// 集會鐘 v1（自主提案切片）——工作台：4 鐵錠(22) + 1 木頭(5) → 1 集會鐘(74)；可放置的方塊。
// 放下後右鍵敲響即向四周傳出鐘聲，附近閒著的居民循聲朝你走來聚集——玩家第一次能像村長一樣
// 主動把村民召到身邊。既是背包物品也是可放置方塊（item_id == block_id）。
const BELL = 74;
// 莓果叢 v1（自主提案切片 806）——背包 2×2：樹苗(65) + 種子(14)×2 → 1 莓果叢苗(75)。
// 種在土地上約 100 秒結果（→ 結果的莓果叢76），採收得莓果(77)×2 後回退成苗、再結果——
// 世界第一種多年生、可反覆採收、不必重種的作物。莓果叢苗既是背包物品也是可放置方塊（item_id==block_id）；
// 結果的莓果叢是伺服器狀態方塊（玩家不放置）；莓果是純物品（採收掉落、可餽贈居民）。
const BERRY_BUSH = 75, BERRY_BUSH_RIPE = 76, BERRY = 77;
// 莓果醬 v1（自主提案切片 808）——熔爐煨煮：莓果(77)×3 → 1 莓果醬(78)。
// 乙太方界第一種「甜點」熟食：可自己享用（EDIBLE_FOODS）或餽贈居民（居民對甜食格外雀躍）。純物品不可放置。
const JAM = 78;
// 木長椅 v1（自主提案切片）——背包 2×2 合成：木頭(5)×2 + 木板(8)×2 → 1 木長椅(79)。
// 玩家擺在世界裡的家具方塊；白天路過閒著的居民會停下腳步坐上去歇一會兒。可放置、破壞回收自身。
const BENCH = 79;
// 雞舍生蛋 v1（自主提案切片）——工作台：木頭(5)×4 + 葉片(6)×2 → 1 空雞舍(80)。
// 世界第一種「動物產物」資源節點：放置後靜候一段時間會生蛋（→有蛋雞舍81），破壞收下蛋(82)
// 就地回退成空雞舍繼續孵，像莓果叢一樣可反覆利用、不必重蓋。空雞舍既是背包物品也是可放置
// 方塊（item_id == block_id）；有蛋雞舍是伺服器狀態方塊（玩家不放置）；蛋是純物品（可餽贈居民）。
const COOP = 80, COOP_READY = 81, EGG = 82;
// 漂流瓶 v1（自主提案切片 825）——背包 2×2 合成：玻璃(10)×2 → 1 空玻璃瓶(83)。
// 世界第一種「玩家↔玩家」的痕跡：對準水面丟下、寫上一句瓶中信；純物品不可放置。
const BOTTLE = 83;
// 建築藍圖 v1（自主提案切片）——工作台合成：五張藍圖各對應一種既有建物，送給居民後直接
// 改寫她的心願成你指定的建物種類（不再只能靠猜關鍵詞碰運氣）；純物品不可放置。
const BLUEPRINT_HOUSE = 84, BLUEPRINT_WELL = 85, BLUEPRINT_TOWER = 86,
      BLUEPRINT_GARDEN = 87, BLUEPRINT_PAVILION = 88;
// 染色建材 v1（自主提案切片）——背包 2×2 合成：2 沙 + 1 礦物 → 2 陶磚。建造近 200 刀以來，
// 純建材幾乎全是灰棕色系，本刀用天然礦物（鐵礦鏽紅/煤礦炭黑/雪原純白/乙太礦青藍）給沙子
// 染色，燒出世界第一批彩色建材。皆可放置、破壞回收自身。
const TERRACOTTA_RED = 89, TERRACOTTA_BLACK = 90, TERRACOTTA_WHITE = 91, TERRACOTTA_BLUE = 92;
// 溫泉遺跡 v1（世界第二種可探索地標，自主提案切片，接續古代遺跡）——伺服器維護的世界生成
// 方塊，玩家不可放置/破壞（同來源水）；當作一種水處理（isWaterId／collision/游泳），
// 只是上色走暖橘泉水而非冷藍，一眼與普通水分開。
const HOT_SPRING_WATER = 93;
// 野花 v1（自主提案切片）——草原／森林群系疏落生長的三色野花，世界第一種「花」；
// 可採、可放置（比照仙人掌/冰晶），送給居民換來世界第一句「收到花」的心動道謝。
const WILDFLOWER_RED = 94, WILDFLOWER_YELLOW = 95, WILDFLOWER_BLUE = 96;
// 居民教你一道獨門配方 v1（自主提案切片，ROADMAP 849）——與某位居民感情深厚時她會主動教你
// 的獨門配方：1 石頭 + 1 紅花 → 護身符(97)。純物品、不可放置。跟一般配方不同，這道配方要
// 先被居民教過（伺服器權威判定好感度）才能合成，見 `knownRecipes` + `matchBagRecipe`。
const AMULET = 97;
// 乙太幣 v1（ROADMAP 873，自主提案切片）——玩家↔玩家至今只有以物易物，沒有一種任何人
// 都想要的通用媒介；把常見原礦鑄成一枚可攜帶的信物，自由市集就能標價收付。純物品、
// 不可放置，98 是 97（護身符）之後的首個空號。
const COIN = 98;
// 驅影之劍 v1（ROADMAP 887，自主提案切片）——世界第一批武器，握在手上驅散夜之暗影更快：
// 木劍一擊抵兩下、石/鐵劍一擊即散，鐵劍還雙倍乙太礦。純物品、不可放置，99~101 是首個空號。
const SWORD_WOOD = 99, SWORD_STONE = 100, SWORD_IRON = 101;
// 方塊顏色（程序生成、純色；不用任何外部美術資產）
const COLOR = {
  [GRASS]:             [0.36, 0.66, 0.27],
  [DIRT]:              [0.55, 0.40, 0.26],
  [STONE]:             [0.50, 0.50, 0.52],
  [SAND]:              [0.85, 0.78, 0.55],
  [WOOD]:              [0.45, 0.31, 0.18],
  [LEAVES]:            [0.27, 0.55, 0.27],
  [WATER]:             [0.20, 0.45, 0.85],
  // 合成方塊：比自然原料更精緻（淺色調）
  [PLANK]:             [0.78, 0.62, 0.42], // 木板——淺棕，比原木明亮
  [STONE_BRICK]:       [0.62, 0.59, 0.56], // 石磚——均勻灰，比原石精緻
  [GLASS]:             [0.82, 0.93, 0.98], // 玻璃——淡藍，像磨砂玻璃
  // 種田 v1：農地三態——顏色漸層暗示成長進度
  [FARM_SOIL]:         [0.38, 0.24, 0.12], // 農田土——深棕，耕過的泥土
  [FARM_SOIL_SEEDED]:  [0.32, 0.42, 0.20], // 幼苗——帶綠的深色，種子萌芽中
  [WHEAT_MATURE]:      [0.88, 0.76, 0.22], // 成熟小麥——金黃色，可收割
  // 工作台 v1
  [WORKBENCH]:         [0.62, 0.40, 0.18], // 工作台——深琥珀棕，木製工作台感
  // 熔爐 v1
  [FURNACE]:           [0.36, 0.26, 0.20], // 熔爐——暗灰棕，燻黑石材爐
  [SMOOTH_STONE]:      [0.72, 0.72, 0.74], // 拋光石——明亮冷灰，精煉石材感
  // 深層礦石 v1（ROADMAP 682）——石灰底+礦石紋理感
  [COAL_ORE]:          [0.35, 0.33, 0.32], // 煤礦——深灰帶黑，石中夾黑炭
  [IRON_ORE]:          [0.66, 0.44, 0.28], // 鐵礦——帶鏽橙的石，鐵質感
  // 鐵錠 v1（ROADMAP 683）——閃亮銀灰，精煉金屬感
  [IRON_INGOT]:        [0.76, 0.76, 0.82], // 鐵錠——明亮銀灰，冶煉後的光澤金屬
  // 鐵磚 v1（ROADMAP 684）——壓縮精煉，比鐵錠更亮更飽和
  [IRON_BLOCK]:        [0.88, 0.88, 0.94], // 鐵磚——高亮銀白帶藍，光潔金屬塊感
  // 火把 v1（ROADMAP 685）——橘黃火焰感，點亮礦坑隧道
  [TORCH]:             [1.00, 0.61, 0.05], // 火把——暖橘黃，燃燒火焰的光感
  // 梯子 v1（ROADMAP 688）——暖木棕，比木板略深；放置後可垂直攀爬
  [LADDER]:            [0.62, 0.42, 0.20], // 梯子——深暖棕，木製梯架感
  // 鎬具 v1（ROADMAP 687）——工具物品，不渲染為世界方塊（只在 UI 顯示）
  [PICKAXE_WOOD]:  [0.60, 0.44, 0.26], // 木鎬——深棕木柄感
  [PICKAXE_STONE]: [0.58, 0.58, 0.60], // 石鎬——冷灰石質感
  [PICKAXE_IRON]:  [0.80, 0.82, 0.90], // 鐵鎬——明亮銀藍，精煉金屬感
  // 斧頭 v1（ROADMAP 689）——暖木棕/冷石灰/亮金屬，與鎬具色系相近但偏暖
  [AXE_WOOD]:  [0.70, 0.48, 0.22], // 木斧——暖棕，比木鎬淺一階（新磨的木刃）
  [AXE_STONE]: [0.55, 0.52, 0.48], // 石斧——微暖灰，比石鎬偏赭（石刃較粗礦）
  [AXE_IRON]:  [0.82, 0.78, 0.74], // 鐵斧——偏暖銀，比鐵鎬少一分冷藍（寬刃感）
  // 鏟子 v1（ROADMAP 690）——泥土色系，土木感；比斧頭更飽和偏赭（鏟面是鏟土的顏色）
  [SHOVEL_WOOD]:  [0.65, 0.45, 0.18], // 木鏟——赭棕，新刻木鏟頭感
  [SHOVEL_STONE]: [0.50, 0.46, 0.42], // 石鏟——灰赭，石材鏟頭（比石斧偏紅）
  [SHOVEL_IRON]:  [0.78, 0.80, 0.84], // 鐵鏟——冷銀，鐵製鏟面反光
  // 箱子 v1（ROADMAP 692）——暖棕木箱感，比工作台淺、比梯子亮；金屬鉚釘感
  [CHEST]:        [0.72, 0.52, 0.28], // 箱子——中暖棕，木箱+鐵釘視覺
  // 木門 v1（ROADMAP 693）——關閉=深暖棕厚實；開啟=淡杏白，一眼分辨可穿越
  [DOOR_CLOSED]:  [0.58, 0.36, 0.14], // 木門（關）——深暖棕，實心大門感
  [DOOR_OPEN]:    [0.85, 0.72, 0.55], // 木門（開）——淡杏白，半透感，可穿越
  // 床 v1——暖紅被褥感，一眼認出是家具而非建材
  [BED]:          [0.72, 0.30, 0.28], // 床——暖磚紅，被褥的溫暖感
  // 第二種作物 v1（胡蘿蔔）——橘色調對比小麥的金黃，一眼分辨兩種作物
  [CARROT_SEEDED]: [0.30, 0.44, 0.22], // 胡蘿蔔幼苗——帶綠的深色，種子萌芽中
  [CARROT_MATURE]:  [0.90, 0.52, 0.16], // 成熟胡蘿蔔——飽和橘色，可收割
  // 第三種作物 v1（馬鈴薯）——土黃偏棕，與小麥金黃、胡蘿蔔橘都能一眼分辨
  [POTATO_SEEDED]: [0.32, 0.30, 0.18], // 馬鈴薯幼苗——暗土黃，埋在土裡的感覺
  [POTATO_MATURE]:  [0.78, 0.64, 0.38], // 成熟馬鈴薯——土黃偏棕，可收割
  // 生物群系第一刀
  [CACTUS]:         [0.25, 0.58, 0.22], // 仙人掌——飽和深綠，沙漠中一眼認出
  // 生物群系第二刀
  [SNOW]:           [0.94, 0.96, 0.99], // 雪——近純白帶極淡藍，一眼認出的覆雪地表
  [ICE_CRYSTAL]:    [0.55, 0.82, 0.95], // 冰晶——飽和冰藍，在雪白地表上一眼認出的閃亮珍寶
  // 冰晶燈 v1（冰晶合成）——比冰晶更亮更泛白的冷藍幽光，像封在玻璃裡發光的冰
  [ICE_LANTERN]:    [0.70, 0.92, 1.00], // 冰晶燈——高亮冷藍白，泛著幽光的裝飾燈（比照火把純亮色作法）
  // 乙太礦脈 v1
  [AETHER_ORE]:     [0.28, 0.62, 0.78], // 乙太礦——深青藍寶礦，埋在最深灰石層裡的一脈幽光
  [AETHER_LAMP]:    [0.55, 0.90, 1.00], // 乙太燈——高亮清冷青藍，散發真實動態光照的高階明燈
  [FISHING_ROD]:    [0.62, 0.44, 0.24], // 釣竿——木褐色（背包圖示用；純物品不放置）
  [FISH]:           [0.70, 0.78, 0.82], // 小魚——銀灰帶青
  [AETHER_FISH]:    [0.40, 0.82, 0.98], // 乙太魚——青藍幽光，呼應乙太礦系
  [COOKED_FISH]:    [0.80, 0.52, 0.30], // 烤魚——烤成金褐帶焦香的暖棕，一看就是熟食
  [BAKED_POTATO]:   [0.72, 0.55, 0.32], // 烤地薯——烤到焦香的暖土褐，比生馬鈴薯更深更熟
  // 野菜暖湯 v1（ROADMAP 778）——胡蘿蔔橘×小麥金×菜綠拌成的暖橘紅濃湯色，一眼是熱騰騰的一鍋料理
  [STEW]:           [0.86, 0.42, 0.20],
  // 乙太煙火 v1（ROADMAP 785）——青藍底閃著金火花，背包圖示用（純物品不放置）
  [FIREWORK]:       [0.36, 0.58, 0.95],
  // 乙太沃肥 v1（ROADMAP 789）——腐草漚肥的深潤棕綠，一眼是漚熟的沃土肥料（純物品不放置）
  [FERTILIZER]:     [0.42, 0.46, 0.24],
  // 植樹造林 v1（ROADMAP 738）——嫩黃綠，比草地/樹葉更亮更嫩，一眼認出是剛種下的小苗
  [SAPLING]:        [0.52, 0.74, 0.30], // 樹苗——鮮嫩黃綠，抽芽中的幼苗感
  [SIGN]:           [0.62, 0.44, 0.25], // 告示牌——溫潤木牌棕（比木板稍深），一看就是塊立起來的木牌
  // 乙太營火 v1（自主提案切片）——炙熱的橘紅火堆色，比火把更飽和鮮亮，一眼是燃燒的營火（發光方塊）
  [CAMPFIRE]:       [0.95, 0.42, 0.12],
  // 水桶 v1（自主提案切片）——空桶是冷灰的鐵皮桶色；滿桶帶一泓水藍，一眼分得出裝了水（純物品不放置）
  [BUCKET]:         [0.58, 0.60, 0.64],
  [WATER_BUCKET]:   [0.28, 0.52, 0.82],
  // 鋤頭 v1（自主提案切片）——木柄土棕色，一眼是把農具（純物品不放置，只在物品欄顯示）
  [HOE]:            [0.62, 0.44, 0.24],
  // 集會鐘 v1（自主提案切片）——溫潤的青銅金鐘色，比鐵磚更暖更亮，一眼是掛著的鑄鐘（可放置方塊）
  [BELL]:           [0.80, 0.62, 0.24],
  // 莓果叢 v1（自主提案切片 806）——未結果叢是純樸的灌木綠（比樹葉深、偏墨綠）；結果的叢
  // 綴上飽和莓紅，一眼分辨「可以採了」；莓果是鮮甜的深莓紅（背包圖示用，純物品不放置）。
  [BERRY_BUSH]:      [0.22, 0.44, 0.22], // 莓果叢苗——墨綠灌木，尚未結果
  [BERRY_BUSH_RIPE]: [0.55, 0.16, 0.30], // 結果的莓果叢——綴滿莓紅，可採收
  [BENCH]:           [0.62, 0.44, 0.25], // 木長椅——溫潤木色，坐得下的家具
  [BERRY]:           [0.72, 0.14, 0.32], // 莓果——鮮甜深莓紅
  [JAM]:             [0.60, 0.10, 0.26], // 莓果醬——比生莓果更濃稠暗沉的熬煮莓紅（罐裝甜點感）
  // 雞舍生蛋 v1（自主提案切片）——空雞舍是溫潤的木架棕（比長椅稍深）；有蛋雞舍多一分暖黃，
  // 一眼分辨「可以收了」；蛋是溫潤的米白（背包圖示用，純物品不放置）。
  [COOP]:            [0.58, 0.42, 0.26], // 空雞舍——木架棕
  [COOP_READY]:      [0.80, 0.68, 0.32], // 有蛋雞舍——暖黃，一眼看出可收成
  [EGG]:             [0.92, 0.88, 0.76], // 蛋——溫潤米白
  [BOTTLE]:          [0.78, 0.90, 0.86], // 空玻璃瓶——淡青綠玻璃感（背包圖示用，純物品不放置）
  // 建築藍圖 v1：五張藍圖各自淺色系區隔，對應建物意象（背包圖示用，純物品不放置）
  [BLUEPRINT_HOUSE]:    [0.90, 0.75, 0.55], // 小屋藍圖——暖木黃
  [BLUEPRINT_WELL]:     [0.68, 0.80, 0.88], // 水井藍圖——石藍
  [BLUEPRINT_TOWER]:    [0.72, 0.70, 0.68], // 瞭望台藍圖——灰石
  [BLUEPRINT_GARDEN]:   [0.70, 0.88, 0.62], // 花圃藍圖——嫩綠
  [BLUEPRINT_PAVILION]: [0.92, 0.68, 0.42], // 涼亭藍圖——溫橘
  // 染色建材 v1（自主提案切片）——世界第一批彩色建材，四色皆飽和鮮明，一眼與灰棕建材分開。
  [TERRACOTTA_RED]:   [0.72, 0.24, 0.16], // 紅陶磚——鐵鏽紅，飽和磚紅
  [TERRACOTTA_BLACK]: [0.16, 0.15, 0.16], // 黑陶磚——深炭黑
  [TERRACOTTA_WHITE]: [0.92, 0.90, 0.86], // 白陶磚——溫潤米白（比雪更暖，一眼分辨建材與地表）
  [TERRACOTTA_BLUE]:  [0.18, 0.42, 0.72], // 青陶磚——沉穩靛藍，呼應乙太礦色系但更飽和內斂
  // 野花 v1（自主提案切片）——世界第一批「花」，三色皆鮮明飽和，一眼與草地/樹葉區分。
  [WILDFLOWER_RED]:    [0.86, 0.20, 0.28], // 紅花——飽和玫瑰紅
  [WILDFLOWER_YELLOW]: [0.95, 0.80, 0.18], // 黃花——明豔向日葵黃
  [WILDFLOWER_BLUE]:   [0.30, 0.46, 0.88], // 藍花——清亮矢車菊藍
  // 護身符 v1（居民教你一道獨門配方，自主提案切片）——溫潤琥珀色，一眼與其餘物品分開。
  [AMULET]: [0.78, 0.58, 0.22],
  // 乙太幣 v1（ROADMAP 873，自主提案切片）——溫暖亮金色，一眼認出「這是錢」。
  [COIN]: [0.95, 0.78, 0.20],
  // 驅影之劍 v1（ROADMAP 887，自主提案切片）——刃色與鎬/斧同一材質色系（木棕/石灰/鐵銀），
  // 一眼認出是同一階材料鍛的武器。
  [SWORD_WOOD]:  [0.64, 0.46, 0.26], // 木劍——深棕木刃
  [SWORD_STONE]: [0.62, 0.62, 0.64], // 石劍——冷灰石刃
  [SWORD_IRON]:  [0.86, 0.88, 0.94], // 鐵劍——明亮銀白，精煉金屬鋒芒
};

// ── 裝飾植物十字貼片渲染 v2 ─────────────────────────────────────────────
// 維護者玩到時把一顆「藍色方塊」納悶成積木、打掉才發現是小花——根因是每種方塊都被
// 畫成平色立方體。這批「本該是插在地上的細植物」改走十字貼片（cross-billboard）：
// 兩片交叉的直立四邊形（X 字形），比整格窄、貼地長，一眼就是「一小株植物」而非方塊。
// v2 精緻化（維護者回報 v1 花偏大偏粗、像大植物）：沿高度分段收放寬度、疊莖/花萼/花冠/
// 花心多段頂點色，讓野花嬌小可辨、紅/黃/藍更好分（詳見 emitCross）。
// 只改長相，碰撞/採集/放置沿用後端語意不動。
const CROSS_PLANTS = new Set([
  WILDFLOWER_RED, WILDFLOWER_YELLOW, WILDFLOWER_BLUE, // 三色野花——本刀主角
  SAPLING,                                            // 樹苗——抽芽幼苗
  BERRY_BUSH, BERRY_BUSH_RIPE,                         // 莓果叢苗／結果莓果叢
]);
// 莖色——統一的柔綠，作為花萼/葉叢的基底色，讓花冠的花色與地面/彼此更好分辨。
const STEM_COLOR = [0.24, 0.5, 0.22];

const DEBUG = location.search.includes("debug");
// 觸控裝置偵測（用於顯示精簡 HUD 文字 + 啟用搖桿/跳鈕/放置鈕）
const isTouch = "ontouchstart" in window || navigator.maxTouchPoints > 0;
const hudEl = document.getElementById("hud");
const dbgEl = document.getElementById("dbg");
const clockEl = document.getElementById("clock"); // 時段指示器（ROADMAP 896）
const seasonEl = document.getElementById("season"); // 季節指示器（ROADMAP 897）

// ── 操作設定 v1（麥塊 Bedrock 式：準心+按鈕防誤觸、靈敏度、慣用手、自動跳、預設人稱）──
// 全部存 localStorage、重載保留；合理預設。純前端顯示/手感層，不動遊戲規則或後端。
const SETTINGS_LS_KEY = "butfun.voxel.settings.v1";
// 觸控操作模式：
//   "crosshair" = 準心+按鈕（防誤觸主打）：拖曳只轉視角，挖/放各有專屬按鈕，絕不誤觸。
//   "tap"       = 點擊互動（舊模式，習慣者可切回）：輕點世界＝挖/對話。
const SETTINGS_DEFAULTS = {
  touchMode: "crosshair",                    // 手機預設準心+按鈕（解決誤觸）
  sensitivity: 1.0,                          // 視角靈敏度倍率（觸控拖曳＋滑鼠＋右搖桿）0.3~2.5
  btnScale: 1.0,                             // 觸控按鈕大小倍率 0.7~1.6
  btnOpacity: 1.0,                           // 觸控按鈕透明度 0.35~1.0
  handed: "right",                           // 慣用手："right"＝搖桿左·按鈕右；"left"＝左右對調
  autoJump: true,                            // 自動跳躍（撞一格自動踏上；預設開＝維持既有踏階手感）
  viewDefault: isTouch ? "third" : "first",  // 預設人稱（桌機第一、手機第三，沿用既有成熟手感）
};
// 從 localStorage 還原設定；缺欄位／壞資料一律 fallback 到預設（向後相容、永不 throw）。
function loadSettings() {
  const s = { ...SETTINGS_DEFAULTS };
  try {
    const raw = localStorage.getItem(SETTINGS_LS_KEY);
    if (raw) {
      const o = JSON.parse(raw);
      if (o && typeof o === "object") {
        if (o.touchMode === "crosshair" || o.touchMode === "tap") s.touchMode = o.touchMode;
        if (typeof o.sensitivity === "number" && isFinite(o.sensitivity)) s.sensitivity = Math.max(0.3, Math.min(2.5, o.sensitivity));
        if (typeof o.btnScale === "number" && isFinite(o.btnScale)) s.btnScale = Math.max(0.7, Math.min(1.6, o.btnScale));
        if (typeof o.btnOpacity === "number" && isFinite(o.btnOpacity)) s.btnOpacity = Math.max(0.35, Math.min(1.0, o.btnOpacity));
        if (o.handed === "left" || o.handed === "right") s.handed = o.handed;
        if (typeof o.autoJump === "boolean") s.autoJump = o.autoJump;
        if (o.viewDefault === "first" || o.viewDefault === "third") s.viewDefault = o.viewDefault;
      }
    }
  } catch (_) { /* localStorage 不可用或壞資料：用預設 */ }
  return s;
}
const settings = loadSettings();
function saveSettings() {
  try { localStorage.setItem(SETTINGS_LS_KEY, JSON.stringify(settings)); } catch (_) { /* 無痕/禁 storage：忽略 */ }
}

// 後端版本戳記：?debug=1 時 fetch /version，把「後端 commit / build 時間」顯示在 dbg HUD，
// 與前端內容雜湊（window.__BUILD__）並列 → 前後端版本都對得上 origin/main = 全上線了，一眼看出。
// 堵死「舊 binary 靜默上線、沒人發現」。失敗（端點不存在/離線）一律安全靜默，不影響遊戲。
let backendVersion = null;
if (DEBUG) {
  fetch("/version", { cache: "no-store" })
    .then((r) => (r.ok ? r.json() : null))
    .then((v) => { if (v) backendVersion = v; })
    .catch(() => {});
}
const errEl = document.getElementById("err");
function showErr(msg) { if (errEl) { errEl.textContent = msg; errEl.style.display = "block"; } }

// ── Three.js 場景 ──────────────────────────────────────────────────────────
const app = document.getElementById("app");
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x87b7e0);
scene.fog = new THREE.Fog(0x87b7e0, 40, 120);

const camera = new THREE.PerspectiveCamera(70, window.innerWidth / window.innerHeight, 0.1, 1000);

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
renderer.setSize(window.innerWidth, window.innerHeight);
app.appendChild(renderer.domElement);

// 半球光（天空/地面）給全向環境光（保證永不全黑），加一盞方向光做陰影感。
// hemi 存起來以便晝夜循環 v1 動態調整強度。
const hemi = new THREE.HemisphereLight(0xcfe8ff, 0x6b7a55, 1.15);
scene.add(hemi);
const sun = new THREE.DirectionalLight(0xfff3da, 0.65);
sun.position.set(40, 80, 25);
scene.add(sun);

// ── 晝夜循環 v1 ─────────────────────────────────────────────────────────────
// time_of_day：0.0=午夜、0.25=黎明、0.5=正午、0.75=黃昏、1.0=午夜（循環）。
// 由伺服器每幀廣播，前端只負責渲染（天空色/太陽位置/光強度）。
let worldTime = 0.42; // 預設白天，伺服器推播後更新

// 天空關鍵幀：[time, skyHex, sunColorHex, sunIntensity, hemiIntensity]
// 每兩個鄰近幀之間做線性插值。
const SKY_KEYS = [
  [0.00, 0x060d1a, 0x1a2d45, 0.03, 0.30], // 深夜
  [0.18, 0x0d1b30, 0x1a2d45, 0.05, 0.40], // 深夜末
  [0.22, 0xd4603a, 0xd4603a, 0.30, 0.65], // 黎明前橙紅
  [0.30, 0xf0a060, 0xf0c060, 0.50, 0.90], // 清晨金黃
  [0.38, 0x87b7e0, 0xfff3da, 0.65, 1.15], // 白晝湛藍
  [0.62, 0x87b7e0, 0xfff3da, 0.65, 1.15], // 白晝湛藍（延續）
  [0.70, 0xf08040, 0xff8c40, 0.45, 0.90], // 傍晚橙
  [0.80, 0xc04020, 0xff6020, 0.18, 0.55], // 黃昏深紅
  [0.88, 0x1a0d20, 0x1a2d45, 0.04, 0.35], // 入夜過渡
  [1.00, 0x060d1a, 0x1a2d45, 0.03, 0.30], // 深夜（循環對齊 t=0）
];

function _hc(hex) {
  return [(hex >> 16 & 0xff) / 255, (hex >> 8 & 0xff) / 255, (hex & 0xff) / 255];
}

// ── 時段指示器 v1（ROADMAP 896）─────────────────────────────────────────────
// 多位居民（玩家建議箱）反映：黃昏/夜晚只能靠天空亮度猜，沒有明確的「當前時段」提示。
// 本函式把 worldTime(0..1) 對映成四個時段，邊界刻意對齊 SKY_KEYS 的天色轉折——
// 徽章顯示的時段永遠和玩家眼前的天色一致（黎明橙紅升起=黎明、天藍=白天、夕紅=黃昏、繁星=夜晚）。
// 純函式、確定性、零副作用，供 HUD 徽章與 QA 共用。
const CLOCK_PHASES = {
  dawn:  { icon: "🌅", name: "黎明", cls: "cl-dawn" },
  day:   { icon: "☀️", name: "白天", cls: "cl-day" },
  dusk:  { icon: "🌆", name: "黃昏", cls: "cl-dusk" },
  night: { icon: "🌙", name: "夜晚", cls: "cl-night" },
};
function voxelClockPhase(t) {
  // 夾進 [0,1) 後判段（容錯 NaN/越界，指示器永不空白）。
  let x = Number.isFinite(t) ? t - Math.floor(t) : 0.5;
  if (x >= 0.22 && x < 0.38) return CLOCK_PHASES.dawn;  // 黎明前橙紅→清晨金黃（SKY 0.22~0.38）
  if (x >= 0.38 && x < 0.70) return CLOCK_PHASES.day;   // 白晝湛藍（SKY 0.38~0.70）
  if (x >= 0.70 && x < 0.88) return CLOCK_PHASES.dusk;  // 傍晚橙→黃昏深紅→入夜（SKY 0.70~0.88）
  return CLOCK_PHASES.night;                            // 深夜（SKY 0.88~1.0、0.0~0.22）
}
// worldTime(0..1) → 概略 24 小時制 "HH:MM"（t=0=00:00、0.25=06:00、0.5=12:00、0.75=18:00）。
function voxelClockTime(t) {
  const x = Number.isFinite(t) ? t - Math.floor(t) : 0.5;
  const total = Math.floor(x * 1440) % 1440; // 一天 1440 分
  const h = Math.floor(total / 60);
  const m = total % 60;
  return `${h < 10 ? "0" : ""}${h}:${m < 10 ? "0" : ""}${m}`;
}
// 供 QA 對映驗證（純函式，無渲染副作用）。
if (typeof window !== "undefined") { window.__voxelClockPhase = voxelClockPhase; window.__voxelClockTime = voxelClockTime; }

let _clockCls = ""; // 記住目前套用的時段 class，只在跨時段時才改 DOM（省重繪）
function updateClock() {
  if (!clockEl) return;
  const ph = voxelClockPhase(worldTime);
  if (ph.cls !== _clockCls) {
    clockEl.className = ph.cls;
    clockEl.querySelector(".cl-icon").textContent = ph.icon;
    clockEl.querySelector(".cl-name").textContent = ph.name;
    _clockCls = ph.cls;
  }
  clockEl.querySelector(".cl-time").textContent = voxelClockTime(worldTime);
}

// ── 季節指示器 v1（ROADMAP 897）─────────────────────────────────────────────
// 季節（voxel_season）至今只靠「刻意極輕」的天地染色暗示，玩家幾乎看不出現在是哪一季，
// 但季節其實驅動居民換季反應/心情/城鎮動態。本徽章把「現在是春/夏/秋/冬＋這一季第幾天」
// 明白寫在時段徽章下方，補齊「此刻（時段）→今日（季內第幾天）→這個季節」的完整時間層次。
// 季節字串 → 徽章圖示/名稱/底色 class（名稱在前端 hardcode，比照時段徽章的 i18n 慣例）。
const SEASON_BADGE = {
  spring: { icon: "🌸", name: "春天", cls: "se-spring" },
  summer: { icon: "🌻", name: "夏天", cls: "se-summer" },
  autumn: { icon: "🍂", name: "秋天", cls: "se-autumn" },
  winter: { icon: "❄️", name: "冬天", cls: "se-winter" },
};
let _seasonKey = ""; // 記住目前顯示的「季節+天數」，只在其變化時才改 DOM（省重繪）
function updateSeason() {
  if (!seasonEl) return;
  const s = SEASON_BADGE[worldSeason] || SEASON_BADGE.spring; // 容錯未知字串，永不空白
  const key = `${worldSeason}/${worldSeasonDay}`;
  if (key === _seasonKey) return;
  _seasonKey = key;
  seasonEl.className = s.cls;
  seasonEl.querySelector(".se-icon").textContent = s.icon;
  seasonEl.querySelector(".se-name").textContent = s.name;
  seasonEl.querySelector(".se-day").textContent = `第${worldSeasonDay}天`;
}

// 季節輪替 v1（ROADMAP 798）：各季節的天地染色 [r, g, b, 權重]（皆 0..1）。
// 依當前季節把插值後的天空/霧色往該季色調輕輕一混——權重刻意小，讓晝夜與晴雨變化仍讀得出，
// 只在氛圍上一眼分得出「換季了」。春天近乎原色（新綠本就是預設世界的模樣），
// 夏濃綠、秋暖橙、冬冷白。
const SEASON_TINT = {
  spring: [0.60, 0.85, 0.55, 0.06],
  summer: [0.50, 0.80, 0.45, 0.10],
  autumn: [0.88, 0.55, 0.28, 0.16],
  winter: [0.80, 0.86, 0.96, 0.18],
};

// ── 四季樹葉 v1（自主提案切片）───────────────────────────────────────────────
// 季節輪替(798)至今只把天空/霧/光「染色」，世界的「方塊」本身從不隨季節改變——森林一年到頭
// 都是同一片濃綠。本刀讓樹葉（世界最大片的植被）第一次隨四季換上不同顏色：春嫩綠、夏濃綠、
// 秋琥珀金、冬霜灰綠。是繼雨(700)/初雪(900)/彩虹(780)/流星(904)/晨霧(913) 一路只碰「天上落下或
// 掛著的東西」之後，環境軸第一次真的改變「地上長著的方塊」本身的模樣。
// 純視覺、零協議：前端用既有廣播的 worldSeason 本地決定樹葉基底色，換季時重建已載入的 chunk。
const SEASON_LEAF = {
  spring: [0.40, 0.66, 0.30], // 春——新葉嫩綠，比夏天更亮更黃嫩
  summer: [0.24, 0.52, 0.24], // 夏——濃蔭深綠，一年最鬱鬱蔥蔥（近似預設樹葉色）
  autumn: [0.82, 0.44, 0.14], // 秋——琥珀金橙，整片森林轉暖，本刀的主角
  winter: [0.56, 0.62, 0.58], // 冬——霜覆的冷灰綠，葉色褪盡、蒙上一層寒霜
};
// 當前季節的樹葉基底色（容錯未知字串 → 回夏天濃綠，永不回傳空值）。
function foliageLeafColor(season) {
  return SEASON_LEAF[season] || SEASON_LEAF.summer;
}
// 換季時把已載入的 chunk 全部標記重建，讓樹葉（與冬雪覆地）重新套上當季外觀。沿用每幀節流
// 的 dirty 佇列（rebuildChunk 迴圈每幀最多重建 4 塊），換季一次性重建成本分攤到數幀、不掉幀（FPS 鐵律）。
function remeshForSeason() {
  for (const k of chunks.keys()) dirty.add(k);
}

// ── 冬雪覆地 v1（ROADMAP 922）─────────────────────────────────────────────
// 四季樹葉（920）第一次讓「樹上的方塊」隨季節換色；但腳下那一大片草原一年到頭仍是同一片綠。
// 本刀補上環境軸最後、也最有感的一塊：冬天一到，露天的鬆軟地面（草／土／沙）頂面積上一層雪白，
// 整片大地一眼變成冬天。刻意只染「頂面」（`f.n[1]===1`）——雪是積在地表上、側面仍露出原本的
// 泥土綠，一眼就是麥塊那種「雪蓋在地上」的真實感，而非把整塊換成雪磚。
// 與既有元素 razor-sharp 區隔：920 四季樹葉＝樹葉方塊(LEAVES)換色；798 季節染色＝只染天空/霧；
// 900 初雪／snow＝天上飄落的粒子；snowman＝單一堆出來的物件；本刀＝地面方塊本身的「頂蓋」隨冬天鋪雪。
// 純視覺、零協議：前端依既有廣播的 worldSeason 本地決定，不新增任何 WS/HTTP 欄位、不動後端。
const SNOW_CAP = [0.93, 0.95, 0.99]; // 積雪頂蓋的雪白（略帶冷藍，冬日調性）
// 會積雪的鬆軟地面（自然裸露的地表）；刻意不含農田土（FARM_SOIL），免得雪蓋住玩家正在照顧的作物。
const SNOW_GROUND = new Set([GRASS, DIRT, SAND]);
// 冬天露天鬆軟地面的積雪頂蓋色；非冬天或非鬆軟地面回 null（不積雪）。
// 沿用 variedBlockColor 讓雪面帶極淡的自然起伏、不死白一片。
function winterSnowCapColor(season, blockId, wx, wy, wz) {
  if (season !== "winter" || !SNOW_GROUND.has(blockId)) return null;
  return variedBlockColor(SNOW_CAP, wx, wy, wz);
}

// 更新天空背景色、霧色、太陽方向/顏色、半球光強度。
function updateSkyAndLight(t) {
  // 找所在的插值段。
  let i = 0;
  while (i < SKY_KEYS.length - 2 && SKY_KEYS[i + 1][0] <= t) i++;
  const [t0, sky0, sun0, si0, hi0] = SKY_KEYS[i];
  const [t1, sky1, sun1, si1, hi1] = SKY_KEYS[i + 1];
  const f = t1 > t0 ? Math.max(0, Math.min(1, (t - t0) / (t1 - t0))) : 0;

  // 插值天空色並套用到背景+霧。
  const [sr0, sg0, sb0] = _hc(sky0);
  const [sr1, sg1, sb1] = _hc(sky1);
  let sr = sr0 + (sr1 - sr0) * f;
  let sg = sg0 + (sg1 - sg0) * f;
  let sb = sb0 + (sb1 - sb0) * f;
  // 下雨天氣 v1（ROADMAP 700）：天空/霧色往灰藍調混，讓下雨天一眼可辨。
  if (isRaining) {
    const [gr, gg, gb] = [0.42, 0.46, 0.52];
    const rw = 0.55;
    sr = sr + (gr - sr) * rw;
    sg = sg + (gg - sg) * rw;
    sb = sb + (gb - sb) * rw;
  }
  // 季節輪替 v1（ROADMAP 798）：依當前季節把天空/霧色往該季色調輕輕一染（權重小、晝夜仍讀得出），
  // 讓世界過了一季的氛圍變化一眼可辨。
  const st = SEASON_TINT[worldSeason];
  if (st) {
    sr = sr + (st[0] - sr) * st[3];
    sg = sg + (st[1] - sg) * st[3];
    sb = sb + (st[2] - sb) * st[3];
  }
  scene.background.setRGB(sr, sg, sb);
  scene.fog.color.setRGB(sr, sg, sb);

  // 插值太陽色與強度。
  const [ur0, ug0, ub0] = _hc(sun0);
  const [ur1, ug1, ub1] = _hc(sun1);
  sun.color.setRGB(ur0 + (ur1 - ur0) * f, ug0 + (ug1 - ug0) * f, ub0 + (ub1 - ub0) * f);
  sun.intensity = (si0 + (si1 - si0) * f) * (isRaining ? 0.6 : 1); // 下雨天陽光轉弱

  // 太陽軌跡：t=0.25 日出（東）、t=0.5 正午（頂）、t=0.75 日落（西）。
  const ang = (t - 0.25) * Math.PI * 2;
  sun.position.set(-Math.cos(ang) * 80, Math.sin(ang) * 80, 25);

  // 半球光強度。
  hemi.intensity = hi0 + (hi1 - hi0) * f;
}

// ── 下雨天氣 v1（ROADMAP 700）─────────────────────────────────────────────
// 伺服器機率式演變晴/雨並隨玩家快照廣播 raining:bool；前端只負責視覺：天空灰藍調 + 雨滴粒子。
// 宣告需在初始 updateSkyAndLight() 呼叫之前，避免其讀取 isRaining 時尚未初始化。
let isRaining = false;

// ── 季節輪替 v1（ROADMAP 798）───────────────────────────────────────────────
// 伺服器由世界累計日數推算當前季節、隨快照廣播 season:"spring"|"summer"|"autumn"|"winter"；
// 前端只負責視覺：換季時為整片天地（天空/霧/半球光）微微換上不同色調，讓「世界過了一季」一眼可辨。
// 宣告需在初始 updateSkyAndLight() 呼叫之前，避免其讀取 worldSeason 時尚未初始化。
let worldSeason = "spring";
// 季節指示器 v1（ROADMAP 897）：伺服器隨快照廣播 season_day（這一季第幾天，1-based）；HUD 徽章顯示用。
let worldSeasonDay = 1;

// 初始套用，讓進場就是白天而非等第一幀快照。
updateSkyAndLight(worldTime);

// 雨滴粒子：單一 THREE.Points（一次 draw call，效能鐵律——別用逐滴 mesh）。
// 座標系相對鏡頭：每幀把整片粒子雲平移到鏡頭上方，粒子本身只在小範圍內落下+重置高度循環。
const RAIN_COUNT = 400;
const RAIN_SPREAD = 30;   // 粒子雲水平範圍（格）
const RAIN_HEIGHT = 20;   // 粒子雲垂直範圍（格），落到底部就重置回頂部
const RAIN_FALL_SPEED = 24; // 格/秒
const rainPositions = new Float32Array(RAIN_COUNT * 3);
for (let i = 0; i < RAIN_COUNT; i++) {
  rainPositions[i * 3 + 0] = (Math.random() - 0.5) * RAIN_SPREAD;
  rainPositions[i * 3 + 1] = Math.random() * RAIN_HEIGHT;
  rainPositions[i * 3 + 2] = (Math.random() - 0.5) * RAIN_SPREAD;
}
const rainGeom = new THREE.BufferGeometry();
rainGeom.setAttribute("position", new THREE.BufferAttribute(rainPositions, 3));
const rainMat = new THREE.PointsMaterial({
  color: 0xaac4e0, size: 0.12, transparent: true, opacity: 0.55, depthWrite: false,
});
const rainPoints = new THREE.Points(rainGeom, rainMat);
rainPoints.visible = false;
scene.add(rainPoints);

// 冬季飄雪 v1（ROADMAP 900）：冬天下雨即飄雪——同一組雨滴粒子改渲染成白、較大、飄落更慢、
// 左右輕飄的雪花（伺服器不必新增協議：前端用既有廣播的 worldSeason==="winter" ＋ isRaining 本地判定）。
// 效能鐵律：仍是同一個 THREE.Points（一次 draw call），只切材質參數＋落速＋加一個 sin 橫飄，零額外物件。
const SNOW_FALL_SPEED = 4.5;  // 雪花飄落速度（格/秒）——遠慢於雨（24），飄逸感的關鍵
const SNOW_SWAY_AMPL = 6;     // 雪花左右輕飄幅度（格/秒），每顆相位錯開
let _snowSwayPhase = 0;       // 橫飄相位累計（僅一個全域 sin 輸入，成本可忽略）

// 每幀推進雨滴/雪花下落（純視覺，無碰撞）；不下雨時整組隱藏、零成本早退。
function updateRain(dt) {
  if (!isRaining) { rainPoints.visible = false; return; }
  const snowing = worldSeason === "winter"; // 冬季下雨 ⇒ 飄雪
  rainPoints.visible = true;
  // 依「雨 / 雪」切換材質外觀（賦值即生效，一次 draw call、零額外物件）。
  if (snowing) {
    rainMat.color.setHex(0xf4f8ff); rainMat.size = 0.28; rainMat.opacity = 0.85;
  } else {
    rainMat.color.setHex(0xaac4e0); rainMat.size = 0.12; rainMat.opacity = 0.55;
  }
  // 粒子雲整體跟著鏡頭水平移動，讓雨/雪看起來覆蓋玩家周遭而非固定世界座標。
  rainPoints.position.set(camera.position.x, camera.position.y + RAIN_HEIGHT / 2, camera.position.z);
  const fall = snowing ? SNOW_FALL_SPEED : RAIN_FALL_SPEED;
  const pos = rainGeom.attributes.position;
  _snowSwayPhase += dt;
  for (let i = 0; i < RAIN_COUNT; i++) {
    let y = pos.getY(i) - fall * dt;
    if (y < -RAIN_HEIGHT / 2) y += RAIN_HEIGHT; // 落到底部循環回頂部
    pos.setY(i, y);
    if (snowing) {
      // 雪花左右輕飄（每顆用索引錯開相位，看起來各自飄）；橫向也做環形循環維持粒子雲範圍。
      let x = pos.getX(i) + Math.sin(_snowSwayPhase * 1.5 + i) * SNOW_SWAY_AMPL * dt;
      if (x > RAIN_SPREAD / 2) x -= RAIN_SPREAD;
      else if (x < -RAIN_SPREAD / 2) x += RAIN_SPREAD;
      pos.setX(i, x);
    }
  }
  pos.needsUpdate = true;
}

// ── 繁星夜空 v1（ROADMAP 783）──────────────────────────────────────────────
// 夜空第一次掛滿繁星＋升起一輪明月,隨晝夜柔和淡入淡出。純視覺、零新協議:入夜程度全由既有
// 廣播的 time_of_day(worldTime)本地演算。效能鐵律:整片星場＝單一 THREE.Points(一次 draw call,
// 別用逐星 mesh),月球＝單一小球 mesh;白天整組隱藏、零成本早退。星場與月球每幀跟著鏡頭平移,
// 半徑夠大 ⇒ 看起來永遠掛在無限遠的天邊(視差可忽略)。
const STAR_COUNT = 700;      // 星星數(單一點雲,對 FPS 無感)
const STAR_RADIUS = 320;     // 星場球半徑(格):夠遠 ⇒ 隨鏡頭平移看起來固定在天上
const starPositions = new Float32Array(STAR_COUNT * 3);
for (let i = 0; i < STAR_COUNT; i++) {
  // 均勻散在球面上,再偏向上半球(y 抬高)——讓多數星星落在地平線之上的夜空,少數貼近天邊。
  const u = Math.random() * 2 - 1;             // cosθ ∈ [-1,1]
  const phi = Math.random() * Math.PI * 2;     // 方位角
  const r = Math.sqrt(Math.max(0, 1 - u * u));
  let y = u;
  y = y * 0.7 + 0.35;                            // 整體抬高:壓低地平線以下的星、集中在頭頂夜空
  starPositions[i * 3 + 0] = Math.cos(phi) * r * STAR_RADIUS;
  starPositions[i * 3 + 1] = y * STAR_RADIUS;
  starPositions[i * 3 + 2] = Math.sin(phi) * r * STAR_RADIUS;
}
const starGeom = new THREE.BufferGeometry();
starGeom.setAttribute("position", new THREE.BufferAttribute(starPositions, 3));
const starMat = new THREE.PointsMaterial({
  color: 0xffffff, size: 1.7, sizeAttenuation: false, // 螢幕像素尺寸(不隨距離縮),遠處星點清晰
  transparent: true, opacity: 0, depthWrite: false, fog: false,
});
const starPoints = new THREE.Points(starGeom, starMat);
starPoints.visible = false;
scene.add(starPoints);

// 明月:單一小球 mesh(MeshBasicMaterial 自發光、不吃光照,任何角度都圓),沿與太陽相對的軌跡掛在夜空。
const moonMat = new THREE.MeshBasicMaterial({
  color: 0xf5f2e0, transparent: true, opacity: 0, depthWrite: false, fog: false,
});
const moon = new THREE.Mesh(new THREE.SphereGeometry(7, 20, 20), moonMat);
moon.visible = false;
scene.add(moon);

// 入夜程度:1=深夜(繁星最盛)、0=白晝(無星),黎明/黃昏之間平滑過渡。與 SKY_KEYS 的深夜段對齊。
function nightFactor(t) {
  if (t < 0.16) return 1;                     // 深夜(午夜後)
  if (t < 0.26) return 1 - (t - 0.16) / 0.10; // 黎明:星星淡出
  if (t < 0.74) return 0;                     // 白晝:無星
  if (t < 0.86) return (t - 0.74) / 0.12;     // 黃昏:星星淡入
  return 1;                                   // 入夜(午夜前)
}

// 每幀更新星空:依 worldTime 算入夜程度,設星場/月球不透明度;夜裡把兩者跟著鏡頭平移到天邊。
// 下雨時繁星被雲遮去(淡掉),雨過天晴才重見星空。白天/無星時整組隱藏、零成本早退。
let starTwinkle = 0; // 星光微閃相位(廉價 sin,一次乘法)
function updateNightSky(dt) {
  let nf = nightFactor(worldTime);
  if (isRaining) nf *= 0.15; // 下雨:烏雲蔽星,只留極淡一層
  if (nf <= 0.001) { starPoints.visible = false; moon.visible = false; return; }
  starTwinkle += dt;
  const twinkle = 0.85 + 0.15 * Math.sin(starTwinkle * 1.6); // 整片星光輕微明滅(非逐星,零額外成本)
  starPoints.visible = true;
  starMat.opacity = nf * 0.9 * twinkle;
  starPoints.position.copy(camera.position); // 跟鏡頭平移 ⇒ 星星像掛在無限遠處
  moon.visible = true;
  moonMat.opacity = nf;
  // 月亮軌跡:與太陽相對(太陽在 t=0.5 正午最高,月亮在午夜最高)。ang 與 updateSkyAndLight 太陽同式再加 π。
  const ang = (worldTime - 0.25) * Math.PI * 2 + Math.PI;
  moon.position.set(
    camera.position.x - Math.cos(ang) * STAR_RADIUS * 0.85,
    camera.position.y + Math.sin(ang) * STAR_RADIUS * 0.85,
    camera.position.z + 25,
  );
}

// ── 雨後彩虹 v1（ROADMAP 780）─────────────────────────────────────────────
// 伺服器偵測「雨→晴」後,隨快照廣播 rainbow:bool(彩虹掛在天邊約一分鐘);前端只負責視覺:
// 一道七彩半弧掛在遠方天邊,雨停時柔和淡入、時間到再淡出。效能鐵律:七道半環 TorusGeometry
// 只在彩虹出現時可見(平時整組隱藏、零成本早退),半透明加法混合、depthWrite:false 融進天空。
let rainbowActive = false;       // 伺服器快照旗標:此刻是否該掛彩虹
let rainbowAlpha = 0;            // 本地平滑不透明度(向目標 ease,免得瞬間出現/消失)
const RAINBOW_PEAK_ALPHA = 0.55; // 彩虹最亮時的不透明度(半透明、不搶戲)
const RAINBOW_FADE_SPEED = 0.6;  // 淡入/淡出速度(alpha/秒)
const RAINBOW_RADIUS = 70;       // 半弧半徑(格),掛在遠方天邊
// 七彩(紅橙黃綠藍靛紫)由外而內排列,每道半環半徑遞減、共同組成一道彩虹。
const RAINBOW_COLORS = [0xff5a5a, 0xff9a3d, 0xffe14d, 0x5fd86b, 0x4db8ff, 0x6a72ff, 0xb96aff];
const rainbowGroup = new THREE.Group();
const rainbowMats = [];
for (let i = 0; i < RAINBOW_COLORS.length; i++) {
  // arc=Math.PI ⇒ 半圈(上半),恰好是一道跨越天際的彩虹弧。tube 細、分段適中即可。
  const geom = new THREE.TorusGeometry(RAINBOW_RADIUS - i * 0.9, 0.42, 6, 48, Math.PI);
  const mat = new THREE.MeshBasicMaterial({
    color: RAINBOW_COLORS[i], transparent: true, opacity: 0,
    depthWrite: false, blending: THREE.AdditiveBlending, fog: false,
  });
  rainbowMats.push(mat);
  rainbowGroup.add(new THREE.Mesh(geom, mat));
}
rainbowGroup.visible = false;
scene.add(rainbowGroup);

// 每幀更新彩虹:alpha 向目標 ease;可見時把整組移到鏡頭附近的遠方天邊、繞 Y 軸朝向鏡頭(僅偏航,
// 維持直立),讓玩家不論面朝哪都能望見這道弧。不亮時整組隱藏、零成本早退。
function updateRainbow(dt) {
  const target = rainbowActive ? RAINBOW_PEAK_ALPHA : 0;
  if (rainbowAlpha < target) rainbowAlpha = Math.min(target, rainbowAlpha + RAINBOW_FADE_SPEED * dt);
  else if (rainbowAlpha > target) rainbowAlpha = Math.max(target, rainbowAlpha - RAINBOW_FADE_SPEED * dt);
  if (rainbowAlpha <= 0.001) { rainbowGroup.visible = false; return; }
  rainbowGroup.visible = true;
  for (const mat of rainbowMats) mat.opacity = rainbowAlpha;
  // 用鏡頭前方的水平朝向決定彩虹掛在哪個方位:取鏡頭視線的水平分量,把彩虹放到那個方向的遠處天邊,
  // 讓玩家不論面朝哪都能望見這道弧(而非固定世界座標、只在某個朝向看得到)。
  const dir = new THREE.Vector3();
  camera.getWorldDirection(dir);
  dir.y = 0;
  if (dir.lengthSq() < 1e-6) dir.set(0, 0, -1);
  dir.normalize();
  rainbowGroup.position.set(
    camera.position.x + dir.x * RAINBOW_RADIUS * 1.4,
    camera.position.y + 6,
    camera.position.z + dir.z * RAINBOW_RADIUS * 1.4,
  );
  // 半弧(TorusGeometry arc=π)預設在 XY 平面、開口朝下;繞 Y 軸轉,讓弧面正對玩家視線(垂直於視線)。
  rainbowGroup.rotation.y = Math.atan2(dir.x, dir.z) + Math.PI;
}

// ── 流星許願 v1（ROADMAP 904）──────────────────────────────────────────────
// 伺服器夜裡低機率偵測到流星,隨快照廣播 meteor:bool(旗標亮約一個檢查窗);前端只負責視覺:
// 在「旗標由假轉真」的上升緣,於鏡頭前方高空生成一道加法混合的光痕,沿對角掠過天際、淡入再
// 拖尾淡出(約 1.2 秒),之後自行隱藏。效能鐵律:單一可重用 Mesh,平時隱藏、未播放時零成本早退,
// 不逐幀配置幾何/材質。與彩虹(白天)、繁星(靜止)區隔:這是夜空唯一「會動、會發生」的天象。
let meteorActive = false;      // 伺服器快照旗標:此刻是否剛劃過流星
let meteorWasActive = false;   // 上一幀旗標,偵測「假→真」上升緣(只在上升緣觸發一次動畫)
let meteorAnimT = -1;          // 動畫進度(秒);< 0 = 未在播放
const METEOR_DURATION = 1.2;   // 一道流星的視覺時長(秒)
const METEOR_LEN = 26;         // 光痕長度(格)
const METEOR_TRAVEL = 60;      // 劃過的位移距離(格)
const meteorMat = new THREE.MeshBasicMaterial({
  color: 0xfff3d0, transparent: true, opacity: 0,
  depthWrite: false, blending: THREE.AdditiveBlending, fog: false,
  side: THREE.DoubleSide, // 旋轉朝行進方向後兩面皆可能朝鏡頭,雙面才不會消失
});
// 光痕本體:細長平面(長軸沿本地 +X),之後整體旋轉對齊行進方向。
const meteorMesh = new THREE.Mesh(new THREE.PlaneGeometry(METEOR_LEN, 0.5), meteorMat);
meteorMesh.visible = false;
scene.add(meteorMesh);
const meteorFrom = new THREE.Vector3();
const meteorDir = new THREE.Vector3();
const _MET_XAXIS = new THREE.Vector3(1, 0, 0);
// 觸發一道流星:在鏡頭水平朝向的高空遠方設定起點/行進方向,開始播放動畫。
function triggerMeteor() {
  // 只在看得見星星的夜空才播(伺服器已有夜間門檻,這裡再保險一次,免得白天邊界瞬間閃現)。
  if (nightFactor(worldTime) <= 0.05) return;
  const dir = new THREE.Vector3();
  camera.getWorldDirection(dir);
  dir.y = 0;
  if (dir.lengthSq() < 1e-6) dir.set(0, 0, -1);
  dir.normalize();
  const side = new THREE.Vector3(-dir.z, 0, dir.x); // 鏡頭水平右向量
  // 起點:鏡頭前方遠處、偏左上方的高空。
  meteorFrom.set(
    camera.position.x + dir.x * 70 - side.x * 24,
    camera.position.y + 46,
    camera.position.z + dir.z * 70 - side.z * 24,
  );
  // 行進方向:向右斜下方掠過(主要沿 side、略微下降)。
  meteorDir.set(side.x, -0.35, side.z).normalize();
  // 光痕長軸(本地 +X)旋轉對齊行進方向。
  meteorMesh.quaternion.setFromUnitVectors(_MET_XAXIS, meteorDir);
  meteorAnimT = 0;
  meteorMesh.visible = true;
}
function updateMeteor(dt) {
  if (meteorActive && !meteorWasActive) triggerMeteor(); // 上升緣:播一道流星
  meteorWasActive = meteorActive;
  if (meteorAnimT < 0) return; // 未在播放:零成本早退
  meteorAnimT += dt;
  const p = meteorAnimT / METEOR_DURATION;
  if (p >= 1) { meteorAnimT = -1; meteorMesh.visible = false; meteorMat.opacity = 0; return; }
  meteorMesh.position.set(
    meteorFrom.x + meteorDir.x * METEOR_TRAVEL * p,
    meteorFrom.y + meteorDir.y * METEOR_TRAVEL * p,
    meteorFrom.z + meteorDir.z * METEOR_TRAVEL * p,
  );
  // 不透明度包絡:0→峰→0(sin),快速亮起再拖尾淡出。
  meteorMat.opacity = Math.sin(p * Math.PI) * 0.95;
}

// ── 乙太煙火 v1（ROADMAP 785）────────────────────────────────────────────────
// 玩家朝夜空施放的煙火:一束火花在施放者頭頂上方升空、綻放。伺服器廣播 firework{x,y,z,palette}
// 給全場,前端在該位置上方生成一朵綻放的火花點雲。效能鐵律:每束煙火＝單一 THREE.Points
// (一次 draw call),壽命約 2 秒後整束移除、釋放幾何;同時最多幾束(受伺服器每連線冷卻天然節流,
// 另設 FW_MAX_BURSTS 保險上限)。陣列空時 update 零成本早退。
const FW_PARTICLES = 90;          // 每束火花粒子數(單一點雲)
const FW_RISE_SECS = 0.55;        // 升空(咻)階段時長
const FW_BURST_SECS = 1.7;        // 綻放(砰)階段時長
const FW_RISE_HEIGHT = 16;        // 從施放者頭頂再往上升多少格才炸開
const FW_SPREAD = 7.5;            // 綻放速度尺度(格/秒)
const FW_GRAVITY = 5.0;           // 火花下墜加速度(格/秒^2)
const FW_MAX_BURSTS = 8;          // 同時最多幾束(保險上限,防極端洗版拖垮)
// 六組配色盤(對齊後端 firework_palette 的 PALETTE_COUNT=6);每束由伺服器選定索引,人人同色。
const FW_PALETTES = [
  [0xff5a5a, 0xffd24d, 0xffffff], // 暖紅金
  [0x4db8ff, 0x9fe8ff, 0xffffff], // 冰藍白(乙太系)
  [0x8a6aff, 0xd06aff, 0xffd24d], // 紫金
  [0x5fd86b, 0xd6ff7a, 0xffffff], // 翠綠
  [0xff8a3d, 0xffd24d, 0xff5a9a], // 橙桃
  [0x66ccff, 0xb96aff, 0xffffff], // 青紫(星夜呼應)
];
const fireworkBursts = []; // 進行中的煙火束

// 施放一束煙火:在世界座標 (x,y,z) 上方生成一朵先升空再綻放的火花。
function spawnFirework(x, y, z, palette) {
  if (fireworkBursts.length >= FW_MAX_BURSTS) {
    // 超出上限:回收最舊一束,避免無限累積(極端情況保險)。
    const old = fireworkBursts.shift();
    scene.remove(old.points);
    old.points.geometry.dispose();
  }
  const pal = FW_PALETTES[((palette % FW_PALETTES.length) + FW_PALETTES.length) % FW_PALETTES.length];
  const pos = new Float32Array(FW_PARTICLES * 3);
  const col = new Float32Array(FW_PARTICLES * 3);
  const vel = new Float32Array(FW_PARTICLES * 3); // 綻放後各粒子速度
  const originY = y + 2.0;          // 從施放者頭頂起
  const apexY = originY + FW_RISE_HEIGHT;
  const c = new THREE.Color();
  for (let i = 0; i < FW_PARTICLES; i++) {
    // 升空階段:全部聚在同一顆上升火種位置(update 每幀覆寫)。
    pos[i * 3] = x; pos[i * 3 + 1] = originY; pos[i * 3 + 2] = z;
    // 綻放速度:球面隨機方向 × 隨機速率(火花四散成球狀)。
    const th = Math.random() * Math.PI * 2;
    const ph = Math.acos(2 * Math.random() - 1);
    const sp = FW_SPREAD * (0.5 + Math.random() * 0.5);
    vel[i * 3]     = Math.sin(ph) * Math.cos(th) * sp;
    vel[i * 3 + 1] = Math.cos(ph) * sp;
    vel[i * 3 + 2] = Math.sin(ph) * Math.sin(th) * sp;
    c.setHex(pal[i % pal.length]);
    col[i * 3] = c.r; col[i * 3 + 1] = c.g; col[i * 3 + 2] = c.b;
  }
  const geom = new THREE.BufferGeometry();
  geom.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  geom.setAttribute("color", new THREE.BufferAttribute(col, 3));
  const mat = new THREE.PointsMaterial({
    size: 0.9, vertexColors: true, transparent: true, opacity: 1,
    depthWrite: false, blending: THREE.AdditiveBlending, fog: false, sizeAttenuation: true,
  });
  const points = new THREE.Points(geom, mat);
  points.frustumCulled = false; // 火花在頭頂遠處,別被視錐誤剔
  scene.add(points);
  fireworkBursts.push({ points, mat, vel, x, z, originY, apexY, age: 0 });
}

// 每幀推進所有煙火束:升空→綻放→淡出,壽命到就移除、釋放幾何。陣列空即零成本早退。
function updateFireworks(dt) {
  if (fireworkBursts.length === 0) return;
  const total = FW_RISE_SECS + FW_BURST_SECS;
  for (let b = fireworkBursts.length - 1; b >= 0; b--) {
    const fw = fireworkBursts[b];
    fw.age += dt;
    if (fw.age >= total) {
      scene.remove(fw.points);
      fw.points.geometry.dispose();
      fireworkBursts.splice(b, 1);
      continue;
    }
    const posAttr = fw.points.geometry.getAttribute("position");
    const arr = posAttr.array;
    if (fw.age < FW_RISE_SECS) {
      // 升空:火種從 originY 線性上升到 apexY,全部粒子同位置(一顆亮點往上竄)。
      const t = fw.age / FW_RISE_SECS;
      const cy = fw.originY + (fw.apexY - fw.originY) * t;
      for (let i = 0; i < FW_PARTICLES; i++) {
        arr[i * 3] = fw.x; arr[i * 3 + 1] = cy; arr[i * 3 + 2] = fw.z;
      }
      fw.mat.opacity = 1; fw.mat.size = 0.7;
    } else {
      // 綻放:各粒子從 apex 依速度飛散 + 重力下墜,隨時間淡出。
      const k = fw.age - FW_RISE_SECS;              // 綻放已過秒數(位置積分係數)
      for (let i = 0; i < FW_PARTICLES; i++) {
        arr[i * 3]     = fw.x + fw.vel[i * 3] * k;
        arr[i * 3 + 1] = fw.apexY + fw.vel[i * 3 + 1] * k - 0.5 * FW_GRAVITY * k * k;
        arr[i * 3 + 2] = fw.z + fw.vel[i * 3 + 2] * k;
      }
      const bf = k / FW_BURST_SECS;                 // 綻放進度 0..1
      fw.mat.opacity = Math.max(0, 1 - bf);         // 線性淡出
      fw.mat.size = 0.9 + bf * 0.6;                 // 火花微微變大再散去
    }
    posAttr.needsUpdate = true;
  }
}

// ── 集會鐘·鐘聲漣漪 v1（自主提案切片）─────────────────────────────────────────────
// 前端契約：任一玩家敲響集會鐘時伺服器廣播 bell_ring{x,y,z,ringer,count} 給全場，前端在鐘的位置
// 生成一圈往外擴散、淡出的水平聲波環（人人可見），呼應「鐘聲傳了出去、把村民召來」。效能鐵律：
// 每圈＝單一 THREE.Mesh（一次 draw call、共用環幾何、只縮放與調透明度），壽命約 1.6 秒後移除；
// 同時最多 BELL_MAX_RINGS 圈（敲鐘本就有 per-居民冷卻＋後端只在召到人時才廣播天然節流，另設保險上限）。
// 陣列空時 update 零成本早退。
const BELL_RING_SECS = 1.6;        // 一圈聲波從冒出到散盡的壽命（秒）
const BELL_RING_MAX_R = 9.0;       // 聲波擴散到的最大半徑（格）
const BELL_MAX_RINGS = 10;         // 同時最多幾圈（保險上限，防極端連環敲洗版）
// 共用一張薄環幾何（內外徑差＝環的粗細）；各圈只是同一幾何的不同縮放實例，省記憶體。
const bellRingGeom = new THREE.RingGeometry(0.82, 1.0, 40);
const bellRings = []; // 進行中的聲波環
function spawnBellRing(x, y, z) {
  if (bellRings.length >= BELL_MAX_RINGS) {
    // 超出上限：回收最舊一圈（極端情況保險，材質才是每圈自有、需釋放；幾何共用不釋放）。
    const old = bellRings.shift();
    scene.remove(old.mesh);
    old.mesh.material.dispose();
  }
  const mat = new THREE.MeshBasicMaterial({
    color: 0xffe08a, transparent: true, opacity: 0.75, side: THREE.DoubleSide,
    depthWrite: false, fog: false,
  });
  const mesh = new THREE.Mesh(bellRingGeom, mat);
  mesh.rotation.x = -Math.PI / 2;             // 攤平成水平環（躺在地面上擴散）
  mesh.position.set(x + 0.5, y + 0.6, z + 0.5); // 從鐘身中段略高處冒出
  mesh.frustumCulled = false;
  scene.add(mesh);
  bellRings.push({ mesh, mat, age: 0 });
}
// 每幀推進所有聲波環：半徑線性外擴、透明度淡出，壽命到就移除、釋放材質。陣列空即零成本早退。
function updateBellRings(dt) {
  if (bellRings.length === 0) return;
  for (let b = bellRings.length - 1; b >= 0; b--) {
    const rg = bellRings[b];
    rg.age += dt;
    if (rg.age >= BELL_RING_SECS) {
      scene.remove(rg.mesh);
      rg.mat.dispose();
      bellRings.splice(b, 1);
      continue;
    }
    const t = rg.age / BELL_RING_SECS;          // 0..1
    const r = 1.0 + (BELL_RING_MAX_R - 1.0) * t; // 半徑從 1 擴到最大
    rg.mesh.scale.set(r, r, r);
    rg.mat.opacity = 0.75 * (1 - t);            // 線性淡出
  }
}

// ── 居民哼歌·飄浮音符 v1（ROADMAP 788）──────────────────────────────────────────
// 前端契約：伺服器把哼歌台詞以音符符號「♪」起頭廣播進居民 say；前端偵測到 say 以 ♪ 起頭，就在
// 該居民頭頂生成一束緩緩上飄、淡出的音符——世界第一段可見的旋律。效能鐵律：每束＝單一 THREE.Points
// （一次 draw call、共用一張 ♪ 貼圖當點精靈），壽命約 2.6 秒後整束移除、釋放幾何；同時最多 HUM_MAX_BURSTS
// 束（居民哼歌本就長冷卻＋低機率天然節流，另設保險上限）。陣列空時 update 零成本早退。
const HUM_NOTES = 6;             // 每束音符粒子數（單一點雲）
const HUM_LIFE_SECS = 2.6;      // 一束音符壽命
const HUM_RISE_SPEED = 0.9;     // 上飄速度（格/秒）
const HUM_DRIFT = 0.5;          // 左右輕飄幅度尺度
const HUM_MAX_BURSTS = 6;       // 同時最多幾束（保險上限）
// ♪ 音符點精靈貼圖（共用一張、所有音符束共用，省記憶體）。柔和暖白，融進世界不刺眼。
function makeNoteTexture() {
  const s = 64;
  const cv = document.createElement("canvas");
  cv.width = cv.height = s;
  const ctx = cv.getContext("2d");
  ctx.clearRect(0, 0, s, s);
  ctx.font = "52px serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = "#fff3c4"; // 暖白偏金
  ctx.fillText("♪", s / 2, s / 2 + 4);
  const tex = new THREE.CanvasTexture(cv);
  tex.needsUpdate = true;
  return tex;
}
const NOTE_TEX = makeNoteTexture();
const humBursts = []; // 進行中的哼歌音符束

// 在世界座標 (x,y,z) 上方生成一束緩緩上飄的音符。
function spawnHumNotes(x, y, z) {
  if (humBursts.length >= HUM_MAX_BURSTS) {
    const old = humBursts.shift();
    scene.remove(old.points);
    old.points.geometry.dispose();
  }
  const pos = new Float32Array(HUM_NOTES * 3);
  const seed = new Float32Array(HUM_NOTES); // 各音符的相位（左右飄動錯開）
  const rise = new Float32Array(HUM_NOTES); // 各音符的上升速度倍率
  const originY = y + 2.4;                   // 從居民頭頂稍上方起
  for (let i = 0; i < HUM_NOTES; i++) {
    pos[i * 3] = x + (Math.random() - 0.5) * 0.6;
    pos[i * 3 + 1] = originY + Math.random() * 0.5;
    pos[i * 3 + 2] = z + (Math.random() - 0.5) * 0.6;
    seed[i] = Math.random() * Math.PI * 2;
    rise[i] = 0.7 + Math.random() * 0.6;
  }
  const geom = new THREE.BufferGeometry();
  geom.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  const mat = new THREE.PointsMaterial({
    size: 0.85, map: NOTE_TEX, transparent: true, opacity: 1,
    depthWrite: false, fog: true, sizeAttenuation: true, alphaTest: 0.1,
  });
  const points = new THREE.Points(geom, mat);
  points.frustumCulled = false;
  scene.add(points);
  humBursts.push({ points, mat, seed, rise, x, z, age: 0 });
}

// 每幀推進所有音符束：上飄＋左右輕飄＋淡出，壽命到就移除、釋放幾何。陣列空即零成本早退。
function updateHumNotes(dt) {
  if (humBursts.length === 0) return;
  for (let b = humBursts.length - 1; b >= 0; b--) {
    const hb = humBursts[b];
    hb.age += dt;
    if (hb.age >= HUM_LIFE_SECS) {
      scene.remove(hb.points);
      hb.points.geometry.dispose();
      humBursts.splice(b, 1);
      continue;
    }
    const posAttr = hb.points.geometry.getAttribute("position");
    const arr = posAttr.array;
    for (let i = 0; i < HUM_NOTES; i++) {
      arr[i * 3 + 1] += HUM_RISE_SPEED * hb.rise[i] * dt; // 緩緩上飄
      arr[i * 3] += Math.sin(hb.age * 2.2 + hb.seed[i]) * HUM_DRIFT * dt; // 左右輕飄
    }
    const t = hb.age / HUM_LIFE_SECS;
    hb.mat.opacity = t < 0.7 ? 1 : Math.max(0, 1 - (t - 0.7) / 0.3); // 後段淡出
    posAttr.needsUpdate = true;
  }
}

// ── 寵愛你的夥伴 v1（ROADMAP 899）：疼一下已馴服的小夥伴，頭頂浮起一串愛心 ──────────
// 沿用哼歌音符（788）同款「單一 THREE.Points 點雲＝一次 draw call、共用一張 💕 貼圖」手法：
// 上飄＋左右輕飄＋後段淡出，壽命約 1.8 秒後整束移除、釋放幾何；同時最多 HEART_MAX_BURSTS
// 束防洗版；陣列空零成本早退（守 FPS 鐵律）。純視覺回饋，撒嬌台詞由後端權威 pet_treat_ok 帶來。
const HEART_NOTES = 5;            // 每束愛心粒子數（單一點雲）
const HEART_LIFE_SECS = 1.8;     // 一束愛心壽命
const HEART_RISE_SPEED = 1.05;   // 上飄速度（格/秒）
const HEART_DRIFT = 0.45;        // 左右輕飄幅度尺度
const HEART_MAX_BURSTS = 6;      // 同時最多幾束（保險上限）
function makeHeartTexture() {
  const s = 64;
  const cv = document.createElement("canvas");
  cv.width = cv.height = s;
  const ctx = cv.getContext("2d");
  ctx.clearRect(0, 0, s, s);
  ctx.font = "48px serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("💕", s / 2, s / 2 + 2);
  const tex = new THREE.CanvasTexture(cv);
  tex.needsUpdate = true;
  return tex;
}
const HEART_TEX = makeHeartTexture();
const heartBursts = []; // 進行中的愛心束

// 在世界座標 (x,y,z) 上方生成一束緩緩上飄的愛心（疼夥伴時呼叫）。
function spawnHearts(x, y, z) {
  if (heartBursts.length >= HEART_MAX_BURSTS) {
    const old = heartBursts.shift();
    scene.remove(old.points);
    old.points.geometry.dispose();
  }
  const pos = new Float32Array(HEART_NOTES * 3);
  const seed = new Float32Array(HEART_NOTES); // 各愛心的相位（左右飄動錯開）
  const rise = new Float32Array(HEART_NOTES); // 各愛心的上升速度倍率
  const originY = y + 1.1;                     // 從小動物頭頂稍上方起（動物比居民矮）
  for (let i = 0; i < HEART_NOTES; i++) {
    pos[i * 3] = x + (Math.random() - 0.5) * 0.5;
    pos[i * 3 + 1] = originY + Math.random() * 0.4;
    pos[i * 3 + 2] = z + (Math.random() - 0.5) * 0.5;
    seed[i] = Math.random() * Math.PI * 2;
    rise[i] = 0.7 + Math.random() * 0.6;
  }
  const geom = new THREE.BufferGeometry();
  geom.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  const mat = new THREE.PointsMaterial({
    size: 0.7, map: HEART_TEX, transparent: true, opacity: 1,
    depthWrite: false, fog: true, sizeAttenuation: true, alphaTest: 0.1,
  });
  const points = new THREE.Points(geom, mat);
  points.frustumCulled = false;
  scene.add(points);
  heartBursts.push({ points, mat, seed, rise, age: 0 });
}

// 每幀推進所有愛心束：上飄＋左右輕飄＋淡出，壽命到就移除、釋放幾何。陣列空即零成本早退。
function updateHearts(dt) {
  if (heartBursts.length === 0) return;
  for (let b = heartBursts.length - 1; b >= 0; b--) {
    const hb = heartBursts[b];
    hb.age += dt;
    if (hb.age >= HEART_LIFE_SECS) {
      scene.remove(hb.points);
      hb.points.geometry.dispose();
      heartBursts.splice(b, 1);
      continue;
    }
    const posAttr = hb.points.geometry.getAttribute("position");
    const arr = posAttr.array;
    for (let i = 0; i < HEART_NOTES; i++) {
      arr[i * 3 + 1] += HEART_RISE_SPEED * hb.rise[i] * dt; // 緩緩上飄
      arr[i * 3] += Math.sin(hb.age * 2.4 + hb.seed[i]) * HEART_DRIFT * dt; // 左右輕飄
    }
    const t = hb.age / HEART_LIFE_SECS;
    hb.mat.opacity = t < 0.65 ? 1 : Math.max(0, 1 - (t - 0.65) / 0.35); // 後段淡出
    posAttr.needsUpdate = true;
  }
}

// ── 乙太沃肥 v1（ROADMAP 789）：施肥瞬間在幼苗上噴一小撮綠色沃肥火花 ─────────────
// 純視覺回饋（作物成熟與否由後端農地 tick 權威決定翻面）。單一 THREE.Points＝一次 draw call，
// 壽命短、上限 FERT_MAX_BURSTS 保險防洗版；陣列空零成本早退（守 FPS 鐵律）。
const FERT_SPARKS = 14;        // 每次施肥噴出的火花粒子數
const FERT_LIFE_SECS = 0.9;    // 一撮火花的壽命
const FERT_MAX_BURSTS = 6;     // 同時最多幾撮（防極端洗版）
const fertBursts = [];         // 進行中的施肥火花

function spawnFertSparkle(x, y, z) {
  if (fertBursts.length >= FERT_MAX_BURSTS) {
    const old = fertBursts.shift();
    scene.remove(old.points);
    old.points.geometry.dispose();
  }
  const pos = new Float32Array(FERT_SPARKS * 3);
  const vel = new Float32Array(FERT_SPARKS * 3);
  for (let i = 0; i < FERT_SPARKS; i++) {
    pos[i * 3] = x + 0.5; pos[i * 3 + 1] = y + 0.5; pos[i * 3 + 2] = z + 0.5;
    const a = Math.random() * Math.PI * 2;      // 水平隨機散開
    const sp = 0.3 + Math.random() * 0.4;
    vel[i * 3] = Math.cos(a) * sp;
    vel[i * 3 + 1] = 1.4 + Math.random() * 1.0; // 主要往上竄再落下
    vel[i * 3 + 2] = Math.sin(a) * sp;
  }
  const geom = new THREE.BufferGeometry();
  geom.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  const mat = new THREE.PointsMaterial({
    size: 0.22, color: 0x8fd44a, transparent: true, opacity: 1,
    depthWrite: false, fog: true, sizeAttenuation: true,
  });
  const points = new THREE.Points(geom, mat);
  points.frustumCulled = false;
  scene.add(points);
  fertBursts.push({ points, mat, vel, age: 0 });
}

// 每幀推進所有施肥火花：噴發＋重力下墜＋淡出，壽命到就移除、釋放幾何。陣列空即零成本早退。
function updateFertSparkle(dt) {
  if (fertBursts.length === 0) return;
  for (let b = fertBursts.length - 1; b >= 0; b--) {
    const fb = fertBursts[b];
    fb.age += dt;
    if (fb.age >= FERT_LIFE_SECS) {
      scene.remove(fb.points);
      fb.points.geometry.dispose();
      fertBursts.splice(b, 1);
      continue;
    }
    const posAttr = fb.points.geometry.getAttribute("position");
    const arr = posAttr.array;
    for (let i = 0; i < FERT_SPARKS; i++) {
      fb.vel[i * 3 + 1] -= 3.2 * dt; // 重力下墜
      arr[i * 3] += fb.vel[i * 3] * dt;
      arr[i * 3 + 1] += fb.vel[i * 3 + 1] * dt;
      arr[i * 3 + 2] += fb.vel[i * 3 + 2] * dt;
    }
    fb.mat.opacity = Math.max(0, 1 - fb.age / FERT_LIFE_SECS);
    posAttr.needsUpdate = true;
  }
}

// ── 世界環境氛圍 v1（ROADMAP 905）：環境微生物 ────────────────────────────────────
// 純前端視覺點綴，讓安靜的世界多一點活氣：白天草原/花叢旁飄幾隻蝴蝶、夜裡水邊/暗處有螢火蟲
// 微光（呼應暗影的光主題）。依「時段 × 玩家附近地表」自然生成，離開範圍或換時段即淡出。
// 非可互動實體、無後端權威（比照 firework/humNotes 的前端粒子作法）。
// 效能鐵律：兩型各一個「精靈池」（開場預建、之後只切 visible/位置/透明度，零配置churn）；
//   數量有硬上限、只在玩家附近生成、frustumCulled 讓背後的不進 draw；非該時段整池隱藏近零成本。
const AMB_BUTTERFLY_MAX = 7;    // 白天蝴蝶同時上限
const AMB_FIREFLY_MAX = 12;     // 夜間螢火蟲同時上限
const AMB_SPAWN_RANGE = 14;     // 生成半徑（格，僅玩家附近）
const AMB_DESPAWN_RANGE = 24;   // 錨點離玩家超過此距離就淡出回收
const AMB_SPAWN_INTERVAL = 0.55;// 生成評估節流（秒），非每幀掃地表
const AMB_FADE_SPEED = 1.6;     // 淡入/淡出速度（透明度/秒）——柔和不突兀

// 蝴蝶貼圖：🦋 emoji 一張共用（暖色、融進草原不刺眼）。
function makeButterflyTexture() {
  const s = 64;
  const cv = document.createElement("canvas");
  cv.width = cv.height = s;
  const ctx = cv.getContext("2d");
  ctx.clearRect(0, 0, s, s);
  ctx.font = "48px serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("🦋", s / 2, s / 2 + 2);
  const tex = new THREE.CanvasTexture(cv);
  tex.needsUpdate = true;
  return tex;
}
// 螢火蟲貼圖：柔和暖黃綠的徑向光暈（非 emoji），配加法混合＝夜裡一點溫暖微光。
function makeFireflyTexture() {
  const s = 64;
  const cv = document.createElement("canvas");
  cv.width = cv.height = s;
  const ctx = cv.getContext("2d");
  const g = ctx.createRadialGradient(s / 2, s / 2, 0, s / 2, s / 2, s / 2);
  g.addColorStop(0.0, "rgba(240,255,180,1)");   // 核心：亮暖黃綠
  g.addColorStop(0.35, "rgba(200,240,120,0.7)");
  g.addColorStop(1.0, "rgba(160,220,90,0)");    // 邊緣淡出
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, s, s);
  const tex = new THREE.CanvasTexture(cv);
  tex.needsUpdate = true;
  return tex;
}
const BUTTERFLY_TEX = makeButterflyTexture();
const FIREFLY_TEX = makeFireflyTexture();

// 預建一型精靈池：每隻一個 Sprite（自動 billboard），各自材質以便獨立淡入淡出。開場全隱藏。
function makeAmbientPool(n, tex, additive, baseScale, peakOp) {
  const arr = [];
  for (let i = 0; i < n; i++) {
    const mat = new THREE.SpriteMaterial({
      map: tex, transparent: true, opacity: 0, depthWrite: false, fog: true,
      ...(additive ? { blending: THREE.AdditiveBlending } : {}),
    });
    const sp = new THREE.Sprite(mat);
    sp.visible = false;
    // frustumCulled 預設 true：背後/視野外的不進 draw call，守 FPS。
    scene.add(sp);
    arr.push({
      sp, mat, alive: false, fade: 0, peakOp, baseScale,
      age: 0, ttl: 0, ax: 0, ay: 0, az: 0,
      phase: Math.random() * Math.PI * 2,   // 飄動相位（各隻錯開）
      scaleJit: 0.8 + Math.random() * 0.5,  // 大小微抖，避免整齊劃一
    });
  }
  return arr;
}
const butterflies = makeAmbientPool(AMB_BUTTERFLY_MAX, BUTTERFLY_TEX, false, 0.55, 0.95);
const fireflies = makeAmbientPool(AMB_FIREFLY_MAX, FIREFLY_TEX, true, 0.45, 0.9);

// 從玩家附近某 (wx,wz) 由上往下找地表：回傳第一個非空氣方塊的 {y,id}；未載入或找不到回 null。
function ambSurface(wx, wz) {
  const top = Math.floor(player.y) + 5;
  const bot = Math.floor(player.y) - 8;
  for (let wy = top; wy >= bot; wy--) {
    const b = getRaw(wx, wy, wz);
    if (b === -1) return null;   // chunk 未載入 → 放棄這次取樣
    if (b === AIR) continue;
    return { y: wy, id: b };
  }
  return null;
}

// 取一隻閒置精靈啟用：設定錨點、壽命、淡入起點。找不到閒置隻回 false。
function activateAmbient(pool, ax, ay, az, ttl) {
  for (const c of pool) {
    if (c.alive) continue;
    c.alive = true; c.fade = 0; c.age = 0; c.ttl = ttl;
    c.ax = ax; c.ay = ay; c.az = az;
    c.phase = Math.random() * Math.PI * 2;
    c.sp.visible = true;
    return true;
  }
  return false;
}

// 白天在花叢/草地上方生成一隻蝴蝶（偏好野花，草地機率減半以聚向花叢）。
function trySpawnButterfly() {
  let n = 0; for (const c of butterflies) if (c.alive) n++;
  if (n >= AMB_BUTTERFLY_MAX) return;
  const ang = Math.random() * Math.PI * 2;
  const rad = 5 + Math.random() * (AMB_SPAWN_RANGE - 5);
  const wx = Math.floor(player.x + Math.cos(ang) * rad);
  const wz = Math.floor(player.z + Math.sin(ang) * rad);
  const surf = ambSurface(wx, wz);
  if (!surf) return;
  const isFlower = surf.id === WILDFLOWER_RED || surf.id === WILDFLOWER_YELLOW || surf.id === WILDFLOWER_BLUE;
  if (!isFlower && surf.id !== GRASS) return;          // 只在花/草上方
  if (!isFlower && Math.random() < 0.5) return;         // 草地降半機率 → 花叢更熱鬧
  const ay = surf.y + 1.2 + Math.random() * 0.9;
  activateAmbient(butterflies, wx + 0.5, ay, wz + 0.5, 9 + Math.random() * 10);
}

// 夜間在水邊（優先）或暗處近地面生成一點螢火蟲微光。
function trySpawnFirefly() {
  let n = 0; for (const c of fireflies) if (c.alive) n++;
  if (n >= AMB_FIREFLY_MAX) return;
  const ang = Math.random() * Math.PI * 2;
  const rad = 4 + Math.random() * (AMB_SPAWN_RANGE - 4);
  const wx = Math.floor(player.x + Math.cos(ang) * rad);
  const wz = Math.floor(player.z + Math.sin(ang) * rad);
  const surf = ambSurface(wx, wz);
  if (!surf) return;
  const nearWater = isWaterId(surf.id);
  if (!nearWater && Math.random() < 0.55) return;       // 非水邊降低機率 → 多數聚水邊
  const ay = surf.y + 0.7 + Math.random() * (nearWater ? 1.0 : 1.3);
  activateAmbient(fireflies, wx + 0.5, ay, wz + 0.5, 8 + Math.random() * 12);
}

// 推進一隻精靈的移動與淡入淡出。wantAlive=此時段是否仍該存在。alive 消退時自動隱藏。
function stepAmbient(c, dt, wantAlive, isFirefly) {
  c.age += dt;
  const dx = c.ax - player.x, dz = c.az - player.z;
  const tooFar = dx * dx + dz * dz > AMB_DESPAWN_RANGE * AMB_DESPAWN_RANGE;
  const keep = wantAlive && c.age < c.ttl && !tooFar;
  // 淡入/淡出（柔和）
  c.fade += (keep ? 1 : -1) * AMB_FADE_SPEED * dt;
  if (c.fade <= 0) { c.fade = 0; c.alive = false; c.sp.visible = false; return; }
  if (c.fade > 1) c.fade = 1;
  const a = c.age, p = c.phase;
  let x, y, z, op = c.peakOp * c.fade;
  if (isFirefly) {
    // 螢火蟲：慢飄 + 明滅閃爍（加法混合下明滅＝一點忽明忽暗的暖光）。
    x = c.ax + Math.sin(a * 0.8 + p) * 0.6;
    y = c.ay + Math.sin(a * 0.6 + p * 1.3) * 0.4;
    z = c.az + Math.cos(a * 0.7 + p) * 0.6;
    op *= 0.45 + 0.55 * (0.5 + 0.5 * Math.sin(a * 2.2 + p)); // 明滅
  } else {
    // 蝴蝶：小幅快速振翅飄動 + 錨點緩慢平移（隨風悠悠）。
    c.ax += Math.cos(p) * 0.12 * dt;
    c.az += Math.sin(p) * 0.12 * dt;
    x = c.ax + Math.sin(a * 3.0 + p) * 0.45;
    y = c.ay + Math.sin(a * 5.0 + p) * 0.28;
    z = c.az + Math.cos(a * 2.6 + p) * 0.45;
  }
  c.sp.position.set(x, y, z);
  const sc = c.baseScale * c.scaleJit;
  c.sp.scale.set(sc, sc, sc);
  c.mat.opacity = op;
}

// 每幀推進環境微生物：依時段決定該出蝴蝶還是螢火蟲，節流評估生成，逐隻移動/淡出。
// 非任何生物的時段（如陰雨、正午與午夜交界）該池自然淡空、visible=false 近零成本。
let ambSpawnTimer = 0;
function updateAmbientLife(dt) {
  const nf = nightFactor(worldTime);
  const calm = !isRaining;                 // 陰雨天不出來（療癒、也省事）
  const dayOut = calm && nf < 0.25;        // 白天出蝴蝶
  const nightOut = calm && nf > 0.55;      // 夜裡出螢火蟲
  // 推進現有個體（時段不符即淡出回收）
  for (const c of butterflies) if (c.alive) stepAmbient(c, dt, dayOut, false);
  for (const c of fireflies) if (c.alive) stepAmbient(c, dt, nightOut, true);
  // 節流生成
  ambSpawnTimer -= dt;
  if (ambSpawnTimer <= 0) {
    ambSpawnTimer = AMB_SPAWN_INTERVAL;
    if (dayOut) trySpawnButterfly();
    else if (nightOut) trySpawnFirefly();
  }
}

// 方塊用 Lambert + 頂點色（每方塊上色），對光反應但靠半球光保底不黑。
// DoubleSide：切片① 求穩，避免任一面纏繞方向算錯被背面剔除成破洞/黑屏（perf 微讓步，之後可收回 FrontSide）。
const opaqueMat = new THREE.MeshLambertMaterial({ vertexColors: true, side: THREE.DoubleSide });

// ── 水體視覺升級 v1 ──────────────────────────────────────────────────────────
// 水 mesh 走頂點色（vertexColors: true），以便依流動等級染不同深淺。
// 來源水（WATER）：深藍不透明感；流動水 level 越高越淺色、更透明——一眼看出流向。
// 水面微動感：onBeforeCompile 注入 GLSL，頂面頂點依 time uniform 做輕微正弦波動。
// 水下氛圍：相機進入水方塊時，#underwaterOverlay 淡藍覆蓋層出現（CSS 即可、零 draw call）。

// 全域 time uniform——每幀在 update() 更新，共用給水面 shader。
const waterTime = { value: 0.0 };

// 水面動感 shader 注入：頂面頂點（normal.y > 0.9）沿 Y 軸輕微正弦偏移。
// 振幅 0.04 格，週期約 2.5 秒，水平位置也做微小 XZ offset 讓波紋更自然。
function makeWaterMat() {
  const mat = new THREE.MeshLambertMaterial({
    vertexColors: true,
    transparent: true,
    opacity: 0.60,
    side: THREE.DoubleSide,
  });
  // 避免 Three.js 快取到沒有 uTime uniform 的 program（每個水材質唯一 cache key）。
  mat.customProgramCacheKey = () => "butfun-water-wave-v1";
  mat.onBeforeCompile = (shader) => {
    shader.uniforms.uTime = waterTime;
    // 在 vertex shader 最前面宣告 uniform
    shader.vertexShader = "uniform float uTime;\n" + shader.vertexShader;
    // 在 #include <begin_vertex> 之後插入波動邏輯。
    // Three.js Lambert vertex shader 在 begin_vertex 時已有 position（attribute）
    // 和 objectNormal（已 decode 的法線），transformed 就是 begin_vertex 所建的工作座標。
    shader.vertexShader = shader.vertexShader.replace(
      "#include <begin_vertex>",
      `#include <begin_vertex>
      // 水面微動感：只對頂面（objectNormal.y > 0.9）的頂點做 Y 軸正弦偏移
      if (objectNormal.y > 0.9) {
        float wave = sin(position.x * 1.8 + uTime * 2.5) * 0.030
                   + sin(position.z * 1.6 + uTime * 1.9 + 1.2) * 0.025;
        transformed.y += wave;
      }
      `
    );
    // 記住 shader reference（uniforms 已含 uTime 指標，自動跟著 waterTime.value 更新）
    mat.userData.shader = shader;
  };
  return mat;
}
const waterMat = makeWaterMat();

// ── 火把/發光方塊 v1（ROADMAP 691 + 乙太礦脈 v1）───────────────────────────────
// 發光方塊（火把 31＝暖橘光、乙太燈 59＝清冷青藍光）放置後向周遭散發光；
// 手持發光方塊時鏡頭附近同樣有光。純前端、零後端、零協議、零 migration、零 LLM。

const TORCH_LIGHT_COLOR = 0xff8820;      // 火把——暖橘黃（比火把顏色稍橘，光感更暖）
const AETHER_LIGHT_COLOR = 0x66ccff;     // 乙太燈——清冷青藍（比火把冷、辨識度高）
const CAMPFIRE_LIGHT_COLOR = 0xff6a1e;   // 營火——炙熱橘紅（比火把更飽和暖烈，一堆真的在燒的火）

/** 此方塊是否為「發光方塊」（會被登記進光源池）。 */
function isLightBlock(b) { return b === TORCH || b === AETHER_LAMP || b === CAMPFIRE; }
/** 發光方塊的光色（不同方塊不同色調）。 */
function lightColorFor(b) {
  if (b === AETHER_LAMP) return AETHER_LIGHT_COLOR;
  if (b === CAMPFIRE) return CAMPFIRE_LIGHT_COLOR;
  return TORCH_LIGHT_COLOR;
}

// 追蹤世界中所有已知發光方塊座標（key="wx,wy,wz" → {wx,wy,wz,color}）。
const torchPositions = new Map();
function torchKey(wx, wy, wz) { return wx + "," + wy + "," + wz; }
function registerTorchBlock(wx, wy, wz, color = TORCH_LIGHT_COLOR) {
  torchPositions.set(torchKey(wx, wy, wz), { wx, wy, wz, color });
}
function unregisterTorchBlock(wx, wy, wz) {
  torchPositions.delete(torchKey(wx, wy, wz));
}
/** 掃描整個 chunk 找發光方塊並登記（chunk 載入時呼叫，讓重連後既有光源也有光）。 */
function scanChunkForTorches(cx, cy, cz) {
  const ch = chunks.get(ckey(cx, cy, cz));
  if (!ch) return;
  const bx = cx * CHUNK, by = cy * CHUNK, bz = cz * CHUNK;
  for (let ly = 0; ly < CHUNK; ly++)
    for (let lz = 0; lz < CHUNK; lz++)
      for (let lx = 0; lx < CHUNK; lx++) {
        const b = ch[lx + lz * CHUNK + ly * CHUNK * CHUNK];
        if (isLightBlock(b))
          registerTorchBlock(bx + lx, by + ly, bz + lz, lightColorFor(b));
      }
}

const TORCH_LIGHT_INTENSITY = 2.5;      // 亮度（影響照亮半徑內的方塊面）
const TORCH_LIGHT_DIST = 10;            // 光照衰減半徑（方塊單位）
const MAX_TORCH_POOL = 6;               // 效能上限：同時啟用的近旁火把光數量

// 放置火把光源池——預建好 PointLight 陣列、只移位、不動態 add/remove（效能穩定）。
const torchLightPool = [];
for (let _i = 0; _i < MAX_TORCH_POOL; _i++) {
  const pl = new THREE.PointLight(TORCH_LIGHT_COLOR, 0, TORCH_LIGHT_DIST);
  pl.visible = false;
  scene.add(pl);
  torchLightPool.push(pl);
}

// 手持火把光源——手拿火把時從鏡頭附近散出較暗的暖光（8 格半徑）。
const heldTorchLight = new THREE.PointLight(TORCH_LIGHT_COLOR, 0, 8);
scene.add(heldTorchLight);

let _torchRefreshTimer = 0; // 每 2 秒刷新一次近旁火把光（非每幀掃，降 CPU）

/**
 * 更新「放置火把」光源池：
 * 選距鏡頭最近的 MAX_TORCH_POOL 個登記火把 → 移過去點亮；其餘熄滅。
 * 每 2 秒呼叫一次（中頻，非每幀）。
 */
function updateNearbyTorchLights() {
  if (torchPositions.size === 0) {
    for (const pl of torchLightPool) { pl.visible = false; pl.intensity = 0; }
    return;
  }
  const cx = camera.position.x, cy = camera.position.y, cz = camera.position.z;
  const sorted = [];
  for (const { wx, wy, wz, color } of torchPositions.values()) {
    const dx = wx + 0.5 - cx, dy = wy + 0.5 - cy, dz = wz + 0.5 - cz;
    sorted.push({ wx, wy, wz, color, d2: dx * dx + dy * dy + dz * dz });
  }
  sorted.sort((a, b) => a.d2 - b.d2);
  for (let i = 0; i < torchLightPool.length; i++) {
    const pl = torchLightPool[i];
    if (i < sorted.length) {
      const t = sorted[i];
      pl.position.set(t.wx + 0.5, t.wy + 0.5, t.wz + 0.5);
      pl.color.setHex(t.color || TORCH_LIGHT_COLOR); // 依方塊種類上色（火把暖橘／乙太燈青藍）
      pl.intensity = TORCH_LIGHT_INTENSITY;
      pl.visible = true;
    } else {
      pl.intensity = 0;
      pl.visible = false;
    }
  }
}
// ── end 火把發光 v1 ───────────────────────────────────────────────────────────

// ── 世界資料：chunk 方塊 + mesh ─────────────────────────────────────────────
const chunks = new Map();      // "cx,cy,cz" -> Uint8Array(4096)
const meshes = new Map();      // "cx,cy,cz" -> { solid: Mesh|null, water: Mesh|null }
const dirty = new Set();       // 待重建 mesh 的 chunk key
const requested = new Set();   // 已向伺服器要過的 column "cx,cz"

function ckey(cx, cy, cz) { return cx + "," + cy + "," + cz; }

function b64ToBytes(b64) {
  const bin = atob(b64);
  const arr = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
  return arr;
}

// 任一世界座標的方塊原值：未載入回 -1，地心（y<0）回石頭（對齊後端基岩）。
function getRaw(wx, wy, wz) {
  if (wy < 0) return STONE;
  const cx = Math.floor(wx / CHUNK), cy = Math.floor(wy / CHUNK), cz = Math.floor(wz / CHUNK);
  const ch = chunks.get(ckey(cx, cy, cz));
  if (!ch) return -1;
  const lx = wx - cx * CHUNK, ly = wy - cy * CHUNK, lz = wz - cz * CHUNK;
  return ch[lx + lz * CHUNK + ly * CHUNK * CHUNK];
}

// 碰撞用：未載入(-1)視為空（不擋路、不卡人）；來源水/流動水與空氣皆不實心。
function solidCollide(wx, wy, wz) {
  const r = getRaw(wx, wy, wz);
  // 梯子（LADDER=35）、木門（開）（DOOR_OPEN=44）非實心——玩家可穿入；水與 AIR 同理不碰撞
  return r > 0 && !isWaterId(r) && r !== LADDER && r !== DOOR_OPEN;
}

// ── 六面定義（外向法線；用 DoubleSide 材質保險，避免纏繞方向把面剔成黑屏）──────
const FACES = [
  { n: [1, 0, 0],  v: [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]], d: [1, 0, 0] },
  { n: [-1, 0, 0], v: [[0, 0, 1], [0, 1, 1], [0, 1, 0], [0, 0, 0]], d: [-1, 0, 0] },
  { n: [0, 1, 0],  v: [[0, 1, 1], [1, 1, 1], [1, 1, 0], [0, 1, 0]], d: [0, 1, 0] },
  { n: [0, -1, 0], v: [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]], d: [0, -1, 0] },
  { n: [0, 0, 1],  v: [[1, 0, 1], [1, 1, 1], [0, 1, 1], [0, 0, 1]], d: [0, 0, 1] },
  { n: [0, 0, -1], v: [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]], d: [0, 0, -1] },
];

// 方塊色只在重建 mesh 時生成：世界座標相同就永遠得到相同微差，不進入每幀更新。
function variedBlockColor(c, wx, wy, wz) {
  let h = Math.imul(wx | 0, 73856093) ^ Math.imul(wy | 0, 19349663) ^ Math.imul(wz | 0, 83492791);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  const light = 0.94 + ((h >>> 0) / 4294967295) * 0.12; // 亮度約 ±6%
  const tint = (((h >>> 9) & 1023) / 1023 - 0.5) * 0.05; // 極淡冷暖色相差，避免髒灰
  return [
    Math.min(1, Math.max(0, c[0] * light + tint)),
    Math.min(1, Math.max(0, c[1] * light + tint * 0.25)),
    Math.min(1, Math.max(0, c[2] * light - tint * 0.7)),
  ];
}

// 固定方向明暗與場景光疊加：頂面柔亮、側面收斂、底面最深。
function faceShade(f) {
  return f.n[1] > 0 ? 1.07 : (f.n[1] < 0 ? 0.70 : 0.91);
}

// 不透明面是否該畫：相鄰是空氣、水或梯子（可視穿）才畫；未載入(-1)當作實心 → 不畫
//（避免世界邊緣冒出一面牆，等鄰塊串到再補）。
function faceVisibleOpaque(nx, ny, nz) {
  const r = getRaw(nx, ny, nz);
  if (r === -1) return false;
  // 梯子（LADDER=35）、木門（開）（DOOR_OPEN=44）是可穿越方塊，視覺上等同空氣
  return r === AIR || isWaterId(r) || r === LADDER || r === DOOR_OPEN;
}
// 水面可見性改由 rebuildChunk 內的水流分支就地判斷（含階梯落差牆），見 waterTopH/emitWaterFace。

// 重建一個 chunk 的合併 mesh（不透明 + 水各一個 geometry）。
function rebuildChunk(key) {
  const [cx, cy, cz] = key.split(",").map(Number);
  const ch = chunks.get(key);
  const old = meshes.get(key);
  if (old) {
    if (old.solid) { scene.remove(old.solid); old.solid.geometry.dispose(); }
    if (old.water) { scene.remove(old.water); old.water.geometry.dispose(); }
    meshes.delete(key);
  }
  if (!ch) return;

  const pos = [], norm = [], col = [], idx = [];
  const wpos = [], wnorm = [], wcol = [], widx = [];
  const baseX = cx * CHUNK, baseY = cy * CHUNK, baseZ = cz * CHUNK;

  for (let ly = 0; ly < CHUNK; ly++) {
    for (let lz = 0; lz < CHUNK; lz++) {
      for (let lx = 0; lx < CHUNK; lx++) {
        const b = ch[lx + lz * CHUNK + ly * CHUNK * CHUNK];
        if (b === AIR) continue;
        const wx = baseX + lx, wy = baseY + ly, wz = baseZ + lz;
        if (isWaterId(b)) {
          // 流動水視覺（麥塊做法）：依等級渲染成遞減高度，形成往低處流的階梯水面。
          // 來源水（WATER）維持滿格；流動水 level 1..7 越遠越矮。純視覺，不動後端水流邏輯。
          const topH = waterTopH(b);
          for (const f of FACES) {
            const nb = getRaw(wx + f.d[0], wy + f.d[1], wz + f.d[2]);
            if (f.n[1] === 1) {
              // 頂面：上方空氣才露出水面，畫在 topH（矮水面一眼看得出在漫）。
              if (nb === AIR) emitWaterFace(wpos, wnorm, wcol, widx, lx, ly, lz, f, topH, topH, b, wx, wy, wz);
            } else if (f.n[1] === -1) {
              // 底面：下方空氣才畫（避免內面）。
              if (nb === AIR) emitWaterFace(wpos, wnorm, wcol, widx, lx, ly, lz, f, 0, 0, b, wx, wy, wz);
            } else {
              // 側面：鄰空氣→整片側牆(0..topH)；鄰為較矮的水→畫階梯落差牆(鄰topH..topH)，
              // 讓「越流越低」的水階在側面也看得出來，不是兩塊水之間破洞。
              if (nb === AIR) {
                emitWaterFace(wpos, wnorm, wcol, widx, lx, ly, lz, f, topH, 0, b, wx, wy, wz);
              } else if (isWaterId(nb)) {
                const nH = waterTopH(nb);
                if (nH < topH - 1e-3) emitWaterFace(wpos, wnorm, wcol, widx, lx, ly, lz, f, topH, nH, b, wx, wy, wz);
              }
            }
          }
        } else if (CROSS_PLANTS.has(b) && !window.__qaCubePlants) {
          // 裝飾植物：走十字貼片（兩片交叉直立四邊形），一眼是「插在地上的一小株」而非方塊。
          // 併入不透明 mesh（opaqueMat 為 DoubleSide，兩面都畫，花不會半透明破洞）。
          // （window.__qaCubePlants 僅供 QA 對比截圖用，切回舊的整格立方體渲染。）
          emitCross(pos, norm, col, idx, lx, ly, lz, variedBlockColor(COLOR[b] || COLOR[STONE], wx, wy, wz), b);
        } else {
          // 四季樹葉 v1：樹葉方塊的基底色隨當前季節換色，其餘方塊沿用固定色。
          const base = b === LEAVES ? foliageLeafColor(worldSeason) : (COLOR[b] || COLOR[STONE]);
          const c = variedBlockColor(base, wx, wy, wz);
          // 冬雪覆地 v1（ROADMAP 922）：冬天露天的鬆軟地面頂面（f.n[1]===1）鋪一層雪白，側面維持原色。
          const snowCap = winterSnowCapColor(worldSeason, b, wx, wy, wz);
          for (const f of FACES) {
            if (!faceVisibleOpaque(wx + f.d[0], wy + f.d[1], wz + f.d[2])) continue;
            emitFace(pos, norm, col, idx, lx, ly, lz, f, (snowCap && f.n[1] === 1) ? snowCap : c);
          }
        }
      }
    }
  }

  const entry = { solid: null, water: null };
  if (idx.length) {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
    g.setAttribute("normal", new THREE.Float32BufferAttribute(norm, 3));
    g.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));
    g.setIndex(idx);
    const m = new THREE.Mesh(g, opaqueMat);
    m.position.set(baseX, baseY, baseZ);
    scene.add(m);
    entry.solid = m;
  }
  if (widx.length) {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.Float32BufferAttribute(wpos, 3));
    g.setAttribute("normal", new THREE.Float32BufferAttribute(wnorm, 3));
    g.setAttribute("color", new THREE.Float32BufferAttribute(wcol, 3));
    g.setIndex(widx);
    const m = new THREE.Mesh(g, waterMat);
    m.position.set(baseX, baseY, baseZ);
    scene.add(m);
    entry.water = m;
  }
  meshes.set(key, entry);
}

// 把一個面（4 頂點、2 三角）推進陣列。座標用 chunk 局部（mesh 自身有 position 偏移）。
function emitFace(pos, norm, col, idx, lx, ly, lz, f, c) {
  const start = pos.length / 3;
  const shade = faceShade(f);
  for (const v of f.v) {
    pos.push(lx + v[0], ly + v[1], lz + v[2]);
    norm.push(f.n[0], f.n[1], f.n[2]);
    if (col && c) col.push(Math.min(1, c[0] * shade), Math.min(1, c[1] * shade), Math.min(1, c[2] * shade));
  }
  idx.push(start, start + 1, start + 2, start, start + 2, start + 3);
}

// 色彩線性內插小工具：mix(a,b,t)=a→b 走 t（0..1）。用來把莖綠、花萼、花冠、花心
// 疊出層次頂點色（不引入外部圖檔，純程式生成的多段漸層）。
function mixCol(a, b, t) {
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
}

// 沿一條對角線推一段「梯形」四邊形：底邊在 yb、寬 halfB；頂邊在 yt、寬 halfT；
// 底/頂各自頂點色 cB/cT（做出漸層）。(ux,uz) 為該對角線的單位方向。
// 底頂寬可不同→把「細莖收窄底 → 鼓起的花冠 → 收成尖的花心」堆成一小株。
function emitCrossSeg(pos, norm, col, idx, cx, cz, ux, uz, yb, yt, halfB, halfT, cB, cT) {
  const start = pos.length / 3;
  // 左下、右下（底，寬 halfB）、右上、左上（頂，寬 halfT）
  pos.push(cx - ux * halfB, yb, cz - uz * halfB); norm.push(0, 1, 0); col.push(cB[0], cB[1], cB[2]);
  pos.push(cx + ux * halfB, yb, cz + uz * halfB); norm.push(0, 1, 0); col.push(cB[0], cB[1], cB[2]);
  pos.push(cx + ux * halfT, yt, cz + uz * halfT); norm.push(0, 1, 0); col.push(cT[0], cT[1], cT[2]);
  pos.push(cx - ux * halfT, yt, cz - uz * halfT); norm.push(0, 1, 0); col.push(cT[0], cT[1], cT[2]);
  idx.push(start, start + 1, start + 2, start, start + 2, start + 3);
}

// 十字貼片（cross-billboard）v2：把裝飾植物畫成兩片交叉的直立四邊形（俯視成 X）。
// v1 是一片填滿大半格的粗 X（寬 0.8、單純底綠頂花色），讀起來像大植物；v2 精緻化成
// 「一小株」——沿高度分段收放寬度、疊多段頂點色：
//   花（三色野花）：細莖(收窄底) → 綠花萼 → 鼓起的彩色花冠 → 收成尖的淺色花心，
//                  花冠最寬僅 ~0.52、總高 ~0.58，嬌小可辨、紅/黃/藍更好分。
//   苗（樹苗／莓果叢）：細莖 → 上寬的葉叢再收頂，像一株小苗而非填滿格的方塊。
// 法線一律朝上：讓花草固定吃頂光、不因側面背光而發黑。座標用 chunk 局部（mesh 有偏移）；
// 材質 opaqueMat 為 DoubleSide→兩面都畫，花不會半透明破洞。
function emitCross(pos, norm, col, idx, lx, ly, lz, topC, b) {
  const cx = lx + 0.5, cz = lz + 0.5, y0 = ly;
  const s = Math.SQRT1_2;                       // 對角線單位分量（1/√2）
  const dirs = [[s, s], [s, -s]];               // 兩條對角線方向
  const isFlower = (b === WILDFLOWER_RED || b === WILDFLOWER_YELLOW || b === WILDFLOWER_BLUE);

  // 依植物型別排出「分段梯形」表：[底y, 頂y, 底半寬, 頂半寬, 底色, 頂色]。
  const stem = STEM_COLOR;
  const stemDark = mixCol(stem, [0, 0, 0], 0.25);  // 莖底稍暗，貼地陰影感
  let segs;
  if (isFlower) {
    const crown = topC;                            // 花冠＝該野花的飽和色
    const calyx = mixCol(stem, crown, 0.35);       // 花萼＝莖綠帶一點花色，承接綠與彩
    const center = mixCol(crown, [1, 1, 1], 0.42); // 花心＝花色提亮，做出花瓣中心亮點
    segs = [
      // 細莖：底收窄(0.025)微張到 0.05，一株纖細的莖
      [y0 + 0.00, y0 + 0.30, 0.025, 0.05, stemDark, stem],
      // 花萼：從細莖張開，綠→花萼色
      [y0 + 0.28, y0 + 0.37, 0.05, 0.16, stem, calyx],
      // 花冠下半：鼓起到最寬(0.26→寬 0.52)，花萼→花冠
      [y0 + 0.36, y0 + 0.47, 0.15, 0.26, calyx, crown],
      // 花冠上半：收成尖端、提亮成花心，一朵有中心的小花
      [y0 + 0.47, y0 + 0.58, 0.26, 0.06, crown, center],
    ];
  } else {
    // 樹苗／莓果叢：細莖 → 上寬葉叢 → 收頂，一株小苗（莓果叢結果時 topC 偏紅＝綴果）
    const foliage = topC;
    segs = [
      [y0 + 0.00, y0 + 0.22, 0.03, 0.06, stemDark, stem],
      [y0 + 0.20, y0 + 0.55, 0.06, 0.24, stem, foliage],
      [y0 + 0.55, y0 + 0.70, 0.24, 0.05, foliage, foliage],
    ];
  }

  for (const [ux, uz] of dirs) {
    for (const [yb, yt, hB, hT, cB, cT] of segs) {
      emitCrossSeg(pos, norm, col, idx, cx, cz, ux, uz, yb, yt, hB, hT, cB, cT);
    }
  }
}

// 水面高度（0..1）：來源水滿格；流動水依 level 遞減，形成往低處的階梯。純視覺、不動後端。
function waterTopH(b) {
  if (b === WATER || b === HOT_SPRING_WATER) return 1.0; // 來源水／溫泉滿格
  const lvl = b - WATER_FLOW_BASE + 1;      // 1..7（越大＝離源越遠＝越矮）
  return Math.max(0.12, 1.0 - lvl * 0.11);  // level1≈0.89 … level7≈0.23
}

// 水體顏色：依流動等級深淺——來源水深藍，level 越高越淺越透明（一眼看出流向）。
// 回傳 [r, g, b]（0..1 線性），由 emitWaterFace 注入頂點色。
function waterColor(b) {
  if (b === HOT_SPRING_WATER) return [0.90, 0.56, 0.24]; // 溫泉：暖橘泉水，一眼與冷藍水分開
  if (b === WATER) return [0.13, 0.38, 0.80];   // 來源水：飽和深藍
  const lvl = b - WATER_FLOW_BASE + 1;            // 1..7
  const t = lvl / WATER_FLOW_MAX_LVL;             // 0..1（越大=越遠=越淡）
  // 從深藍（0.13,0.40,0.82）漸變到淺藍白（0.50,0.72,0.95）
  const r = 0.13 + t * (0.50 - 0.13);
  const g = 0.40 + t * (0.72 - 0.40);
  const bv = 0.82 + t * (0.95 - 0.82);
  return [r, g, bv];
}

// 推一個水面（4 頂點、2 三角）：頂邊在 yTop、底邊在 yBot（側面藉此畫出階梯落差牆）。
// wcol：水頂點色陣列；blockId：水方塊 id（決定深淺色）。
function emitWaterFace(pos, norm, col, idx, lx, ly, lz, f, yTop, yBot, blockId, wx, wy, wz) {
  const start = pos.length / 3;
  const c = variedBlockColor(waterColor(blockId), wx, wy, wz);
  const shade = faceShade(f);
  for (const v of f.v) {
    const y = v[1] === 1 ? yTop : yBot;
    pos.push(lx + v[0], ly + y, lz + v[2]);
    norm.push(f.n[0], f.n[1], f.n[2]);
    col.push(Math.min(1, c[0] * shade), Math.min(1, c[1] * shade), Math.min(1, c[2] * shade));
  }
  idx.push(start, start + 1, start + 2, start, start + 2, start + 3);
}

// ── 水下氛圍 v1 ──────────────────────────────────────────────────────────────
// 相機進入水方塊 → #underwaterOverlay（淡藍色覆蓋層）淡入；離水即淡出。
// 零 Three.js draw call，只改 DOM 元素 opacity。
const _underwaterEl = document.getElementById("underwaterOverlay");
let _isUnderwater = false;
function updateUnderwaterAtmosphere() {
  const cx = Math.floor(camera.position.x);
  const cy = Math.floor(camera.position.y);
  const cz = Math.floor(camera.position.z);
  const blockAtCamera = getRaw(cx, cy, cz);
  const underwater = isWaterId(blockAtCamera);
  if (underwater !== _isUnderwater) {
    _isUnderwater = underwater;
    if (_underwaterEl) {
      _underwaterEl.style.opacity = underwater ? "1" : "0";
    }
    // 水下微霧：略縮短 Three.js fog，回到地面立即還原（與 overlay 獨立，確保霧效無論如何生效）
    scene.fog.near = underwater ? 6 : 40;
    scene.fog.far  = underwater ? 18 : 120;
  }
}
// ── end 水下氛圍 v1 ───────────────────────────────────────────────────────────

// ── 晨霧 v1（ROADMAP 913）──────────────────────────────────────────────────────
// 每個清晨，整片世界浮起一層薄霧：縮短 Three.js fog 的能見度，遠處的屋與樹在霧裡朦朧成剪影，
// 日出後（太陽升高）再一點點散去。純前端、零後端、零協議、零 migration、零 LLM、零美術——
// 只用既有廣播的 time_of_day（worldTime）本地判定，複用場景既有的 scene.fog。
// 對齊後端 TimePhase::Dawn（time_of_day ∈ [0.20, 0.35)）：0.20 霧最濃、0.25 日出、0.35 散盡。
const MIST_DAWN_START = 0.20; // 清晨窗口起點（霧開始瀰漫），對齊後端 Dawn 起點
const MIST_DAWN_PEAK  = 0.22; // 日出前最濃
const MIST_DAWN_END   = 0.35; // 清晨窗口終點（霧散盡），對齊後端 Dawn 終點
// fog.near/far：無霧＝場景既有預設（40／120）；濃霧＝縮短到看不遠（12／50）。
const MIST_FOG_NEAR_CLEAR = 40,  MIST_FOG_NEAR_THICK = 12;
const MIST_FOG_FAR_CLEAR  = 120, MIST_FOG_FAR_THICK  = 50;

/** 依一日進度 t（0–1）算晨霧強度 [0,1]：清晨窗口外＝0（無霧）、窗口內先快速濃起再漸散。
 *  純函式、確定性、可單元測試；壞值（NaN/±Infinity/非數字）一律保守回 0（不起霧、永不爆）。 */
function dawnMistStrength(t) {
  if (typeof t !== "number" || !isFinite(t)) return 0;
  if (t <= MIST_DAWN_START || t >= MIST_DAWN_END) return 0; // 清晨窗口外：無霧
  if (t <= MIST_DAWN_PEAK) {
    // START→PEAK：霧快速濃起（線性上升到 1）。
    return (t - MIST_DAWN_START) / (MIST_DAWN_PEAK - MIST_DAWN_START);
  }
  // PEAK→END：日出後漸散（線性回落到 0）。
  return Math.max(0, 1 - (t - MIST_DAWN_PEAK) / (MIST_DAWN_END - MIST_DAWN_PEAK));
}

/** 每幀更新晨霧的 fog 能見度。水下另有自己的近霧（updateUnderwaterAtmosphere 主導），
 *  水下時整段跳過、由水下霧接管；地面則依晨霧強度縮短能見度（無霧時剛好還原成場景預設）。 */
function updateDawnMist() {
  if (_isUnderwater) return; // 水下：交給水下霧，不套晨霧
  const mist = dawnMistStrength(worldTime);
  scene.fog.near = MIST_FOG_NEAR_CLEAR - (MIST_FOG_NEAR_CLEAR - MIST_FOG_NEAR_THICK) * mist;
  scene.fog.far  = MIST_FOG_FAR_CLEAR  - (MIST_FOG_FAR_CLEAR  - MIST_FOG_FAR_THICK)  * mist;
}
// ── end 晨霧 v1 ───────────────────────────────────────────────────────────────

// 把一個 chunk 連同鄰塊標記為待重建（鄰塊也要重算面剔除）。
function markDirty(cx, cy, cz) {
  dirty.add(ckey(cx, cy, cz));
  dirty.add(ckey(cx + 1, cy, cz)); dirty.add(ckey(cx - 1, cy, cz));
  dirty.add(ckey(cx, cy + 1, cz)); dirty.add(ckey(cx, cy - 1, cz));
  dirty.add(ckey(cx, cy, cz + 1)); dirty.add(ckey(cx, cy, cz - 1));
}

// ── 玩家狀態（前端權威預測；位置同步回伺服器給別人看）──────────────────────
const player = { x: 0.5, y: 30, z: 0.5, vy: 0, grounded: false, yaw: 0 };
const PW = 0.3, PH = 1.7; // 半寬 / 身高

// 踏階視覺補間：物理 Y 瞬到位（碰撞正確、避免穿模），視覺 Y 平滑跟上（消除閃爍/瞬跳）。
// stepSmooth 是「視覺比物理落後多少格」，踏階時累積、每幀指數衰減歸零。
// 往下/重力完全不受影響（stepSmooth 只在踏階成功時累積，永遠 >= 0）。
let stepSmooth = 0;
// 衰減速率（格/秒）；可調：10 ≈ 0.3 秒內視覺追上物理，夠快看得出「走上去」、不拖泥帶水。
const STEP_SMOOTH_K = 10;
let myId = null;
let myName = "旅人";

// 登入綁定（比照 3D #821）：開頁查 /auth/me，登入者拿到帳號身分 → 進場以帳號名綁定
// 記憶/好感度/背包（換訪客名也認得你）。訪客仍可逛。身份真相由後端 cookie 決定，
// 這兩個變數只供 UI 與「入場名先正確」用。
let isLoggedIn = false;
let myAccountName = null;

// 好感度 v1（ROADMAP 656）：我與各居民的互動記憶筆數（連線後從 /voxel/affinity 拉取）。
// key = resident_id, value = count (0=陌生人, 1-2=相識, 3+=友人)
const myAffinity = new Map();

// ── 方塊小人 avatar（玩家 & 其他玩家共用造型）────────────────────────────────
// 造型：頭 + 軀幹 + 兩臂 + 兩腿，低多邊形方塊感（原創幾何，非任何外部遊戲資產；
// 「方塊人」是通用造型概念）。整體高度維持 PH=1.7、寬約 PW*2，只換視覺、碰撞盒不變。
// 原點在「身體中心」（對齊既有 mesh.position = y + PH/2、稱號牌 y = PH/2 + 0.35）。
// 手腳各掛在關節 pivot 上，繞 x 軸前後擺動＝走路動畫。幾何共用一份省記憶體，材質依配色建立。
const AV_HEAD_GEO = new THREE.BoxGeometry(0.42, 0.4, 0.42);
const AV_TORSO_GEO = new THREE.BoxGeometry(0.5, 0.55, 0.28);
const AV_LEG_GEO = new THREE.BoxGeometry(0.18, 0.75, 0.22);
const AV_ARM_GEO = new THREE.BoxGeometry(0.15, 0.6, 0.18);
const AV_C = PH / 2; // 身體中心相對腳底的高度（group 原點就在此）；下面用 feetY - AV_C 換算部位位置

// 建一具方塊小人。bodyColor=軀幹、headColor=頭、limbColor=手腳。回傳 group + 四肢 pivot 供動畫。
function buildAvatar(bodyColor, headColor, limbColor) {
  const group = new THREE.Group();
  const bodyMat = new THREE.MeshLambertMaterial({ color: bodyColor });
  const headMat = new THREE.MeshLambertMaterial({ color: headColor });
  const limbMat = new THREE.MeshLambertMaterial({ color: limbColor });

  // 軀幹：腳底起 0.75→1.30，中心在 feetY=1.025
  const torso = new THREE.Mesh(AV_TORSO_GEO, bodyMat);
  torso.position.y = 1.025 - AV_C;
  group.add(torso);
  // 頭：1.30→1.70，中心在 feetY=1.50（頭頂 1.70＝PH）
  const head = new THREE.Mesh(AV_HEAD_GEO, headMat);
  head.position.y = 1.50 - AV_C;
  group.add(head);

  // 四肢：pivot 掛在關節（髖/肩），部位往下垂、繞 pivot x 軸擺動＝抬腿擺臂
  function limb(geo, x, jointFeetY, len) {
    const pivot = new THREE.Group();
    pivot.position.set(x, jointFeetY - AV_C, 0);
    const m = new THREE.Mesh(geo, limbMat);
    m.position.y = -len / 2; // 讓部位頂端貼在關節上、往下垂
    pivot.add(m);
    group.add(pivot);
    return pivot;
  }
  // 腿：pivot 在髖部（feetY=0.75），腿長 0.75；臂：pivot 在肩部（feetY=1.30），臂長 0.6
  const legL = limb(AV_LEG_GEO, -0.12, 0.75, 0.75);
  const legR = limb(AV_LEG_GEO, 0.12, 0.75, 0.75);
  const armL = limb(AV_ARM_GEO, -0.325, 1.30, 0.6);
  const armR = limb(AV_ARM_GEO, 0.325, 1.30, 0.6);

  return { group, legL, legR, armL, armR, phase: 0 };
}

// 走路動畫：移動時手腳前後擺（腿與同側手臂反相＝自然擺臂）；靜止平滑回正。
// 純三角函數 + 每幀 dt，成本極低（守 FPS 鐵律）。
function animateAvatar(av, moving, dt) {
  if (moving) {
    av.phase += dt * 9.0;               // 擺動頻率
    const a = Math.sin(av.phase) * 0.7; // 擺幅（弧度）
    av.legL.rotation.x = a;  av.legR.rotation.x = -a;
    av.armL.rotation.x = -a; av.armR.rotation.x = a;
  } else {
    const k = Math.min(1, dt * 12);     // 平滑回正
    av.legL.rotation.x += -av.legL.rotation.x * k;
    av.legR.rotation.x += -av.legR.rotation.x * k;
    av.armL.rotation.x += -av.armL.rotation.x * k;
    av.armR.rotation.x += -av.armR.rotation.x * k;
  }
}

// ── 手持工具可見 v1（自主提案切片）───────────────────────────────────────────
// 玩家至今是隱形的操作者：工具/方塊做出來後除了 UI 圖示，世界裡誰都看不出你手上拿著
// 什麼——居民已有詳盡的視覺個性（住家色盤/建物/稱號），玩家自己反而毫無視覺存在感。
// 本刀補上：熱鍵選中的物品，用一顆貼色小方塊掛在慣用手（右臂）末端顯示出來，自己與
// 別的玩家都看得見——「你正拿著石鎬」「他手上是根釣竿」，操作第一次有了畫面。
// 共用幾何、每具身體各自一份材質（換手持物只換材質顏色，不重建 mesh，FPS 友善，
// 比照既有 digest 守門慣例：物品沒變就整幀不動）。
const HELD_ITEM_GEO = new THREE.BoxGeometry(0.22, 0.22, 0.22);
function attachHeldItem(av) {
  const mat = new THREE.MeshLambertMaterial({ color: 0xffffff });
  const mesh = new THREE.Mesh(HELD_ITEM_GEO, mat);
  // 掛在右臂 pivot（肩關節）下，貼近手臂末端、稍微前伸，像握在掌心；隨手臂擺動自然同動。
  mesh.position.set(0, -0.55, 0.14);
  mesh.visible = false;
  av.armR.add(mesh);
  av.heldMesh = mesh;
  av.heldId = null;
  return av;
}
// 依物品 id 查既有 COLOR 色盤（方塊/工具早已有色，見合成/採集系統）換色；查不到顏色的
// 物品（如食物/種子等未定義視覺色的雜項）就不顯示，避免突兀的預設方塊。
function setHeldItem(av, itemId) {
  if (!av || !av.heldMesh || av.heldId === itemId) return;
  av.heldId = itemId;
  const c = itemId ? COLOR[itemId] : null;
  if (!c) { av.heldMesh.visible = false; return; }
  av.heldMesh.material.color.setRGB(c[0], c[1], c[2]);
  av.heldMesh.visible = true;
}

// 玩家自己的身體（第三人稱可見）：金色系方塊小人，一眼認得是自己。
const myAvatar = attachHeldItem(buildAvatar(0xffcf6b, 0xffe0b0, 0xe6b866));
const bodyMesh = myAvatar.group; // 沿用 bodyMesh 名：.visible/.position/.rotation.y 都作用在 Group 上
let myTitleSprite = null; // 僅 QA 用（_qaSetMyTitle）：正式流程玩家自己不掛稱號牌
scene.add(bodyMesh);

// 其他玩家：id -> { mesh, av, bubble, lastSay }（bubble = 頭上對話泡泡，embodied 靠近說話 v1）
const others = new Map();
// 其他玩家配色：藍色系方塊小人（與自己金色一眼區分身分）。人數少，各自建一具成本可忽略。
const OTHER_PALETTE = { body: 0x8fd0ff, head: 0xcbe9ff, limb: 0x6fb4e6 };

// ── 乙太方界 AI 居民（切片③）────────────────────────────────────────────────
// 後端權威：居民的位置/名字/說的話都由 /voxel/ws 的 players 快照帶來，前端只渲染。
// FPS 鐵律（記取 #614/#820）：居民少（~4 位）、共用幾何/材質、頭頂名牌與泡泡用快取貼圖，
// 文字沒變就不重建貼圖；遠處（超過霧距）整個 group 隱藏，零渲染負擔。
const residents = new Map(); // id -> { group, label, bubble, lastName, lastSay }
// 居民配色（暖棕，與自己金色/別的玩家藍色一眼區分）。共用材質/幾何省記憶體。
const RES_BODY_MAT = new THREE.MeshLambertMaterial({ color: 0xd8b070 });
const RES_HEAD_MAT = new THREE.MeshLambertMaterial({ color: 0xe8c89a });
const RES_TORSO_GEO = new THREE.BoxGeometry(0.5, 1.0, 0.32);
const RES_HEAD_GEO = new THREE.BoxGeometry(0.42, 0.42, 0.42);
// 居民表情/肢體 v1：加一對細手臂做揮手/搔頭/縮身動作（共用幾何/材質，成本可忽略）。
const RES_ARM_GEO = new THREE.BoxGeometry(0.14, 0.55, 0.18);
const RES_VISIBLE_DIST = 110; // 超過此距離（接近霧盡頭）隱藏，省繪製
// 肢體動畫只在鏡頭附近的居民跑（FPS 鐵律）：超過此距離只保留位置更新、不算補間。
const RES_GESTURE_DIST = 42;
// 暗影害怕半徑（前端偵測）：居民附近有暗影小靈就縮身顫抖，無需後端新增旗標。
const RES_FEAR_DIST = 9;

// ── 野兔 v1（自主提案切片，ROADMAP 847）＋水中游魚 v1（ROADMAP 848）──────────
// 世界環境生物：純點綴、無名牌無泡泡。野兔=身體+兩隻耳朵、魚=身體+尾鰭，
// 依伺服器快照的 `kind` 欄位挑選對應模型。數量少、共用幾何/材質，與居民同款
// 「距離超過即整個 group 隱藏」省繪製（FPS 鐵律）。
const wildlifeEnts = new Map(); // id -> { group }
const RABBIT_BODY_MAT = new THREE.MeshLambertMaterial({ color: 0xc9a876 });
const RABBIT_EAR_MAT = new THREE.MeshLambertMaterial({ color: 0xdcc09a });
const RABBIT_BODY_GEO = new THREE.BoxGeometry(0.32, 0.24, 0.4);
const RABBIT_EAR_GEO = new THREE.BoxGeometry(0.06, 0.22, 0.06);
const FISH_BODY_MAT = new THREE.MeshLambertMaterial({ color: 0x4a90c2 });
const FISH_TAIL_MAT = new THREE.MeshLambertMaterial({ color: 0x3a7aa8 });
const FISH_BODY_GEO = new THREE.BoxGeometry(0.14, 0.14, 0.34);
const FISH_TAIL_GEO = new THREE.BoxGeometry(0.16, 0.16, 0.06);
// 放養雞 v1（自主提案切片，ROADMAP 870）：白身體 + 紅雞冠，體型介於兔子與魚之間。
const CHICKEN_BODY_MAT = new THREE.MeshLambertMaterial({ color: 0xf2ede2 });
const CHICKEN_COMB_MAT = new THREE.MeshLambertMaterial({ color: 0xc23b3b });
const CHICKEN_BODY_GEO = new THREE.BoxGeometry(0.3, 0.28, 0.36);
const CHICKEN_COMB_GEO = new THREE.BoxGeometry(0.1, 0.1, 0.1);
const WILDLIFE_VISIBLE_DIST = 60; // 兔子體型小，遠處看不清也不必畫，比居民更早隱藏

// ── 暗影生物 v1（怪物/抵禦第一刀·夜的張力）────────────────────────────────────
// 夜間暗處的漂浮小靈：半透明深色核心 + 微光邊（additive 外殼），輕輕上下浮動。
// 伺服器權威（位置/受擊/消散都由後端算），前端只渲染 + 把「準心挖擊」轉送 shadow_hit。
const shadowEnts = new Map(); // id -> { group, core, glow, baseY, phase, hitFlash }
const SHADOW_CORE_MAT = new THREE.MeshBasicMaterial({ color: 0x14102a, transparent: true, opacity: 0.78 });
const SHADOW_GLOW_MAT = new THREE.MeshBasicMaterial({
  color: 0x7a5fd0, transparent: true, opacity: 0.16, blending: THREE.AdditiveBlending, depthWrite: false,
});
const SHADOW_CORE_GEO = new THREE.IcosahedronGeometry(0.34, 1);
const SHADOW_GLOW_GEO = new THREE.IcosahedronGeometry(0.5, 1);
const SHADOW_VISIBLE_DIST = 70;
const SHADOW_PICK_REACH = 8; // 準心挑選距離（前端提示；真正打不打得到由伺服器 reach 複驗）

// 建一隻暗影小靈（深色核心 + 微光外殼；材質 clone 讓每隻可獨立變淡/閃白）。
function buildShadow(id) {
  const group = new THREE.Group();
  const core = new THREE.Mesh(SHADOW_CORE_GEO, SHADOW_CORE_MAT.clone());
  core.position.y = 0.45;
  group.add(core);
  const glow = new THREE.Mesh(SHADOW_GLOW_GEO, SHADOW_GLOW_MAT.clone());
  glow.position.y = 0.45;
  group.add(glow);
  group.userData.sid = id;
  scene.add(group);
  return { group, core, glow, baseY: 0, phase: Math.random() * Math.PI * 2, hitFlash: 0 };
}

// 依伺服器快照更新所有暗影（同構 updateWildlife）；受擊數越多越淡（「快散了」）。
// 首次見到暗影時提示玩法（一台裝置只提示一次，localStorage 記憶）。
function updateShadows(list) {
  const seen = new Set();
  for (const s of list) {
    seen.add(s.id);
    let ent = shadowEnts.get(s.id);
    if (!ent) {
      ent = buildShadow(s.id);
      shadowEnts.set(s.id, ent);
    }
    ent.group.position.x = s.x;
    ent.group.position.z = s.z;
    ent.baseY = s.y;
    // 受擊變淡：0 擊 0.78 → 2 擊 0.42（每擊 -0.18），微光邊也跟著弱。
    const fade = Math.max(0.3, 0.78 - (s.hits || 0) * 0.18);
    ent.core.material.opacity = fade;
    const dx = s.x - player.x, dz = s.z - player.z;
    ent.group.visible = (dx * dx + dz * dz) < (SHADOW_VISIBLE_DIST * SHADOW_VISIBLE_DIST);
  }
  for (const [id, ent] of shadowEnts) {
    if (!seen.has(id)) { scene.remove(ent.group); shadowEnts.delete(id); }
  }
  // 首夜提示：第一次親眼見到暗影時教玩法（光=庇護）。只提示一次、不洗版。
  if (list.length > 0 && !localStorage.getItem("bfShadowHint")) {
    localStorage.setItem("bfShadowHint", "1");
    showMsg("🌑 夜裡有暗影出沒…點亮燈火吧！火把與燈的光圈是庇護，牆內是安全的。");
    setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 5200);
  }
}

// 準心挑暗影（同款 raycast；命中回 id，否則 null）。伺服器仍會權威複驗 reach。
function pickShadow(clientX, clientY) {
  if (!shadowEnts.size) return null;
  const rect = renderer.domElement.getBoundingClientRect();
  const ndc = new THREE.Vector2(
    ((clientX - rect.left) / rect.width) * 2 - 1,
    -((clientY - rect.top) / rect.height) * 2 + 1
  );
  raycaster.setFromCamera(ndc, camera);
  const pickables = [];
  for (const ent of shadowEnts.values()) {
    if (ent.group.visible) ent.group.traverse((o) => { if (o.isMesh) pickables.push(o); });
  }
  if (!pickables.length) return null;
  const hits = raycaster.intersectObjects(pickables, false);
  if (!hits.length || hits[0].distance > SHADOW_PICK_REACH) return null;
  let obj = hits[0].object;
  while (obj && !(obj.userData && obj.userData.sid !== undefined)) obj = obj.parent;
  return obj && obj.userData && obj.userData.sid !== undefined ? obj.userData.sid : null;
}

// 消散輕煙：一顆快速放大並淡出的微光球（0.7 秒），溫柔的「散去」而非爆炸。
const shadowPuffs = []; // { mesh, t }
function spawnShadowPuff(x, y, z) {
  const mesh = new THREE.Mesh(SHADOW_GLOW_GEO, SHADOW_GLOW_MAT.clone());
  mesh.material.opacity = 0.5;
  mesh.position.set(x, y + 0.45, z);
  scene.add(mesh);
  shadowPuffs.push({ mesh, t: 0 });
}

// 每幀推進暗影氛圍：上下輕浮 + 微光呼吸 + 受擊閃白 + 輕煙淡出。數量 ≤6，成本極低。
function updateShadowFx(dt) {
  const now = performance.now() * 0.001;
  for (const ent of shadowEnts.values()) {
    ent.group.position.y = ent.baseY + Math.sin(now * 2.0 + ent.phase) * 0.12;
    const breathe = 1.0 + Math.sin(now * 3.1 + ent.phase) * 0.08;
    ent.glow.scale.setScalar(breathe);
    if (ent.hitFlash > 0) {
      ent.hitFlash -= dt;
      ent.core.material.color.setHex(0xcfc4ff); // 命中閃一下淡紫白
      if (ent.hitFlash <= 0) ent.core.material.color.setHex(0x14102a);
    }
  }
  for (let i = shadowPuffs.length - 1; i >= 0; i--) {
    const p = shadowPuffs[i];
    p.t += dt;
    const k = p.t / 0.7;
    if (k >= 1) { scene.remove(p.mesh); shadowPuffs.splice(i, 1); continue; }
    p.mesh.scale.setScalar(1 + k * 2.2);
    p.mesh.material.opacity = 0.5 * (1 - k);
  }
}

// 文字貼圖 sprite（名牌/泡泡共用工廠）。bubble=true 用柔色圓底（像在說話），否則白描邊名牌。
function makeTextSprite(text, bubble) {
  const canvas = document.createElement("canvas");
  canvas.width = 256; canvas.height = 64;
  const ctx = canvas.getContext("2d");
  ctx.font = "bold 26px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  let label = text;
  if (label.length > 16) label = label.slice(0, 15) + "…";
  if (bubble) {
    const tw = Math.min(248, ctx.measureText(label).width + 28);
    ctx.fillStyle = "rgba(245,248,255,0.92)";
    const bx = 128 - tw / 2, bw = tw, by = 14, bh = 38, rr = 10;
    ctx.beginPath();
    ctx.moveTo(bx + rr, by);
    ctx.arcTo(bx + bw, by, bx + bw, by + bh, rr);
    ctx.arcTo(bx + bw, by + bh, bx, by + bh, rr);
    ctx.arcTo(bx, by + bh, bx, by, rr);
    ctx.arcTo(bx, by, bx + bw, by, rr);
    ctx.closePath(); ctx.fill();
    ctx.fillStyle = "#243044";
    ctx.fillText(label, 128, by + bh / 2 + 1);
  } else {
    ctx.lineWidth = 5; ctx.strokeStyle = "rgba(0,0,0,0.8)";
    ctx.strokeText(label, 128, 32);
    ctx.fillStyle = "#fff7e6";
    ctx.fillText(label, 128, 32);
  }
  const tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false }));
  // 世界單位：方塊尺度，名牌約 2 寬 0.5 高，浮在頭頂。
  sprite.scale.set(2.4, 0.6, 1);
  return sprite;
}
function setSpriteText(sprite, text, bubble) {
  const fresh = makeTextSprite(text, bubble);
  if (sprite.material.map) sprite.material.map.dispose();
  sprite.material.map = fresh.material.map;
  sprite.material.needsUpdate = true;
}

// 特殊身分稱號牌（維護者的專屬身分）：金色「✦ 稱號 ✦」小標，穩定貼在他頭頂正上方一點點，
// 與一般玩家（本來就沒名牌）明顯區別。只在後端 title 字串為真時掛上（不信客戶端自報）。
// title＝稱號文字（如「引夢使者」「築夢工匠」）。
function makeTitleSprite(title) {
  const canvas = document.createElement("canvas");
  canvas.width = 256; canvas.height = 64;
  const ctx = canvas.getContext("2d");
  ctx.font = "bold 26px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  const label = "✦ " + title + " ✦";
  // 深色描邊讓金字在任何背景都清楚。
  ctx.lineWidth = 6; ctx.strokeStyle = "rgba(60,40,0,0.85)";
  ctx.strokeText(label, 128, 32);
  // 金色漸層字，與一般玩家 / 居民的暖白名牌區別。
  const grad = ctx.createLinearGradient(0, 16, 0, 48);
  grad.addColorStop(0, "#fff2b0");
  grad.addColorStop(0.5, "#ffd24d");
  grad.addColorStop(1, "#e8a400");
  ctx.fillStyle = grad;
  ctx.fillText(label, 128, 32);
  const tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false }));
  sprite.scale.set(2.6, 0.65, 1);
  // mesh 原點在身體中心（見 players 更新：mesh.position = p.y + PH/2）。頭頂在中心上方 PH/2；
  // 稱號牌貼頭頂正上方一點點（頭頂 + 0.35），跟著 mesh 每幀移動，不再飄在半空。
  sprite.position.y = PH / 2 + 0.35;
  return sprite;
}

// ── 告示牌 v1（ROADMAP 740）：牌面文字浮在世界裡，所有人看得見 ─────────────────────
// 文字內容以座標鍵記於 signTexts，實體 sprite 記於 signSprites（掛在世界固定位置，非跟人）。
const signTexts = new Map();   // "x,y,z" -> 文字
const signSprites = new Map(); // "x,y,z" -> THREE.Sprite

// 把一段文字切成最多 max 行、每行 per 字（用 Array.from 正確處理中日文等多位元組字）。
function wrapSignLines(text, per, max) {
  const chars = Array.from(text);
  const lines = [];
  for (let i = 0; i < chars.length && lines.length < max; i += per) {
    lines.push(chars.slice(i, i + per).join(""));
  }
  if (lines.length === 0) lines.push("");
  return lines;
}

// 產生一塊木牌樣式的文字 sprite（比名牌寬、可容兩行，深木底＋暖白字，一看就是塊牌子）。
function makeSignSprite(text) {
  const canvas = document.createElement("canvas");
  canvas.width = 256; canvas.height = 96;
  const ctx = canvas.getContext("2d");
  // 木牌底 + 邊框（純畫，不用外部素材）。
  ctx.fillStyle = "rgba(120,84,48,0.96)";
  ctx.fillRect(8, 12, 240, 72);
  ctx.lineWidth = 4; ctx.strokeStyle = "rgba(70,46,24,0.96)";
  ctx.strokeRect(8, 12, 240, 72);
  // 文字（最多兩行、每行約 12 字；置中）。
  ctx.font = "bold 26px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  ctx.fillStyle = "#fdf3e0";
  const lines = wrapSignLines(text, 12, 2);
  const lh = 30, startY = 48 - (lines.length - 1) * lh / 2;
  lines.forEach((ln, i) => ctx.fillText(ln, 128, startY + i * lh));
  const tex = new THREE.CanvasTexture(canvas); tex.anisotropy = 4;
  const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: true }));
  sprite.scale.set(2.0, 0.75, 1);
  return sprite;
}

// 套用一面牌子的文字（新增/改字/清除）。text 空字串＝移除牌面浮字（牌子被破壞或清空）。
function applySign(x, y, z, text) {
  const key = x + "," + y + "," + z;
  const clean = (text || "").trim();
  const old = signSprites.get(key);
  if (!clean) {
    signTexts.delete(key);
    if (old) { scene.remove(old); if (old.material.map) old.material.map.dispose(); old.material.dispose(); signSprites.delete(key); }
    return;
  }
  signTexts.set(key, clean);
  if (old) {
    // 改字：換掉貼圖即可，不重建 sprite。
    const fresh = makeSignSprite(clean);
    if (old.material.map) old.material.map.dispose();
    old.material.map = fresh.material.map;
    old.material.needsUpdate = true;
  } else {
    const sprite = makeSignSprite(clean);
    // 浮在牌子方塊正上方一點，讀起來像立在牌上。
    sprite.position.set(x + 0.5, y + 1.05, z + 0.5);
    scene.add(sprite);
    signSprites.set(key, sprite);
  }
}

// ── 漂流瓶 v1（自主提案切片 825）：世界上飄著瓶子的位置浮標 ─────────────────────────
// 只知道「這裡有一只瓶子」，內文絕不快取在前端——要真的右鍵撿起（read_bottle）才由伺服器
// 單播揭曉。所有瓶子共用同一張 emoji 貼圖（圖案固定不變），不必像告示牌逐面生畫布。
const bottleMarkers = new Map(); // "x,y,z" -> THREE.Sprite
let bottleMarkerTex = null;
function bottleMarkerTexture() {
  if (bottleMarkerTex) return bottleMarkerTex;
  const canvas = document.createElement("canvas");
  canvas.width = 64; canvas.height = 64;
  const ctx = canvas.getContext("2d");
  ctx.font = "40px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  ctx.fillText("🍾", 32, 34);
  bottleMarkerTex = new THREE.CanvasTexture(canvas);
  return bottleMarkerTex;
}
function addBottleMarker(x, y, z) {
  const key = x + "," + y + "," + z;
  if (bottleMarkers.has(key)) return;
  const sprite = new THREE.Sprite(
    new THREE.SpriteMaterial({ map: bottleMarkerTexture(), transparent: true, depthTest: true })
  );
  sprite.scale.set(0.6, 0.6, 1);
  sprite.position.set(x + 0.5, y + 1.05, z + 0.5);
  scene.add(sprite);
  bottleMarkers.set(key, sprite);
}
function removeBottleMarker(x, y, z) {
  const key = x + "," + y + "," + z;
  const sprite = bottleMarkers.get(key);
  if (sprite) { scene.remove(sprite); bottleMarkers.delete(key); } // 貼圖共用，不 dispose
}

// ── 掉落物 v1（自主提案切片 828）：世界上還沒被撿走的實體材料浮標 ───────────────────
// 玩家↔玩家至今僅有漂流瓶（非同步/文字）與並肩協作（被動加成）——本刀補上第一個主動的
// 實體資源轉手：按 Q 丟下一份手上材料，安靜留在原地；每顆掉落物用該材料自己的顏色畫成
// 小方塊貼圖＋數量標籤（不像瓶子共用單一圖案，貼圖依 id 用完即 dispose）。
const dropMarkers = new Map(); // id(number) -> THREE.Sprite
function makeDropSprite(itemId, count) {
  const canvas = document.createElement("canvas");
  canvas.width = 80; canvas.height = 80;
  const ctx = canvas.getContext("2d");
  const c = COLOR[itemId] || COLOR[STONE];
  ctx.fillStyle = `rgb(${(c[0] * 255) | 0},${(c[1] * 255) | 0},${(c[2] * 255) | 0})`;
  ctx.fillRect(18, 18, 44, 44);
  ctx.strokeStyle = "rgba(0,0,0,0.4)"; ctx.lineWidth = 3;
  ctx.strokeRect(18, 18, 44, 44);
  ctx.font = "bold 20px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  const label = "×" + count;
  ctx.lineWidth = 3; ctx.strokeStyle = "rgba(0,0,0,0.7)"; ctx.strokeText(label, 40, 66);
  ctx.fillStyle = "#fff"; ctx.fillText(label, 40, 66);
  const sprite = new THREE.Sprite(
    new THREE.SpriteMaterial({ map: new THREE.CanvasTexture(canvas), transparent: true, depthTest: true })
  );
  sprite.scale.set(0.5, 0.5, 1);
  return sprite;
}
function addDropMarker(id, x, y, z, itemId, count) {
  if (dropMarkers.has(id)) return;
  const sprite = makeDropSprite(itemId, count);
  sprite.position.set(x, y, z);
  scene.add(sprite);
  dropMarkers.set(id, sprite);
}
function removeDropMarker(id) {
  const sprite = dropMarkers.get(id);
  if (!sprite) return;
  scene.remove(sprite);
  if (sprite.material.map) sprite.material.map.dispose();
  sprite.material.dispose();
  dropMarkers.delete(id);
}

// ── 玩家自由市集 v1（自主提案切片 832）：世界上還在等人接手的交易攤浮標 ─────────────
// 玩家↔玩家至今僅有漂流瓶（文字/匿名）、並肩協作（被動加成）、掉落物（單向轉手）——
// 攤位是第一種「雙向議定」的以物易物：畫兩個顏色方塊（給出／要求）中間夾一個箭頭，
// 一眼看出「拿右邊換左邊」，貼圖依攤位內容生成、移除時 dispose（不像瓶子共用單一圖案）。
const stallMarkers = new Map(); // "x,y,z" -> THREE.Sprite
function makeStallSprite(giveItem, giveCount, wantItem, wantCount) {
  const canvas = document.createElement("canvas");
  canvas.width = 160; canvas.height = 80;
  const ctx = canvas.getContext("2d");
  ctx.fillStyle = "rgba(40,30,16,0.55)";
  ctx.fillRect(4, 4, 152, 72);
  ctx.strokeStyle = "rgba(255,235,180,0.9)"; ctx.lineWidth = 3;
  ctx.strokeRect(4, 4, 152, 72);
  const drawItem = (cx, itemId, count) => {
    const c = COLOR[itemId] || COLOR[STONE];
    ctx.fillStyle = `rgb(${(c[0] * 255) | 0},${(c[1] * 255) | 0},${(c[2] * 255) | 0})`;
    ctx.fillRect(cx - 22, 14, 44, 44);
    ctx.strokeStyle = "rgba(0,0,0,0.4)"; ctx.lineWidth = 2;
    ctx.strokeRect(cx - 22, 14, 44, 44);
    ctx.font = "bold 18px system-ui, sans-serif";
    ctx.textAlign = "center"; ctx.textBaseline = "middle";
    const label = "×" + count;
    ctx.lineWidth = 3; ctx.strokeStyle = "rgba(0,0,0,0.7)"; ctx.strokeText(label, cx, 62);
    ctx.fillStyle = "#fff"; ctx.fillText(label, cx, 62);
  };
  drawItem(112, giveItem, giveCount); // 攤主給出的（換到手上這份）
  ctx.font = "bold 24px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  ctx.fillStyle = "#ffe9b0";
  ctx.fillText("←", 80, 40);
  drawItem(48, wantItem, wantCount); // 攤主想換的（接手者要交出）
  const sprite = new THREE.Sprite(
    new THREE.SpriteMaterial({ map: new THREE.CanvasTexture(canvas), transparent: true, depthTest: true })
  );
  sprite.scale.set(1.1, 0.55, 1);
  return sprite;
}
function addStallMarker(x, y, z, giveItem, giveCount, wantItem, wantCount) {
  const key = x + "," + y + "," + z;
  if (stallMarkers.has(key)) return;
  const sprite = makeStallSprite(giveItem, giveCount, wantItem, wantCount);
  sprite.position.set(x + 0.5, y + 1.1, z + 0.5);
  scene.add(sprite);
  stallMarkers.set(key, sprite);
}
function removeStallMarker(x, y, z) {
  const key = x + "," + y + "," + z;
  const sprite = stallMarkers.get(key);
  if (!sprite) return;
  scene.remove(sprite);
  if (sprite.material.map) sprite.material.map.dispose();
  sprite.material.dispose();
  stallMarkers.delete(key);
}

// ── embodied 靠近說話 v1：自己頭上的對話泡泡（本地驅動，說話立即冒、計時消失）─────
// 不蓋住畫面、跟著角色在 3D 世界裡（「話活在世界裡」）。別人看到的版本走 players 廣播的 say。
const MY_BUBBLE_SECS = 6;
const myBubble = makeTextSprite("", true);
myBubble.visible = false;
scene.add(myBubble);
let myBubbleTimer = 0;
let myBubbleText = "";
function showMyBubble(text) {
  const t = (text || "").trim();
  if (!t) return;
  myBubbleText = t.slice(0, 60);
  setSpriteText(myBubble, myBubbleText, true);
  myBubble.visible = true;
  myBubbleTimer = MY_BUBBLE_SECS;
}

// 居民「夢想副標籤」sprite 工廠——名牌之下、小字 dim 暖色，顯示玩家種下的心願。
function makeDesireSprite(text) {
  const canvas = document.createElement("canvas");
  canvas.width = 256; canvas.height = 44;
  const ctx = canvas.getContext("2d");
  ctx.font = "italic 18px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  let label = text;
  if (label.length > 18) label = label.slice(0, 17) + "…";
  ctx.lineWidth = 4; ctx.strokeStyle = "rgba(0,0,0,0.55)";
  ctx.strokeText(label, 128, 22);
  ctx.fillStyle = "rgba(255,220,140,0.88)";
  ctx.fillText(label, 128, 22);
  const tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  const sprite = new THREE.Sprite(
    new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false })
  );
  sprite.scale.set(2.2, 0.48, 1);
  return sprite;
}
function setDesireText(sprite, text) {
  const fresh = makeDesireSprite(text);
  if (sprite.material.map) sprite.material.map.dispose();
  sprite.material.map = fresh.material.map;
  sprite.material.needsUpdate = true;
}

// ── 好感度指示燈（ROADMAP 656）──────────────────────────────────────────────

/** 依好感度計數回傳指示燈 emoji。0=無, 1-2=淡藍心(相識), 3+=金心(友人)。純函式、可測。 */
function affinityEmoji(count) {
  if (count <= 0) return "";
  if (count <= 2) return "💙";
  return "💛";
}

/** 製作好感度指示燈 sprite（小字 emoji，居名牌正上方）。 */
function makeAffinitySprite(emoji) {
  const canvas = document.createElement("canvas");
  canvas.width = 64; canvas.height = 40;
  const ctx = canvas.getContext("2d");
  ctx.font = "24px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  if (emoji) ctx.fillText(emoji, 32, 20);
  const tex = new THREE.CanvasTexture(canvas);
  const sprite = new THREE.Sprite(
    new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false })
  );
  sprite.scale.set(0.7, 0.44, 1);
  return sprite;
}

function setAffinityEmoji(sprite, emoji) {
  const fresh = makeAffinitySprite(emoji);
  if (sprite.material.map) sprite.material.map.dispose();
  sprite.material.map = fresh.material.map;
  sprite.material.needsUpdate = true;
}

/** 製作心情指示燈 sprite（心情 emoji，居名牌左側，ROADMAP 676）。 */
function makeMoodSprite(emoji) {
  const canvas = document.createElement("canvas");
  canvas.width = 64; canvas.height = 40;
  const ctx = canvas.getContext("2d");
  ctx.font = "24px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  if (emoji) ctx.fillText(emoji, 32, 20);
  const tex = new THREE.CanvasTexture(canvas);
  const sprite = new THREE.Sprite(
    new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false })
  );
  sprite.scale.set(0.7, 0.44, 1);
  return sprite;
}

function setMoodEmoji(sprite, emoji) {
  const fresh = makeMoodSprite(emoji);
  if (sprite.material.map) sprite.material.map.dispose();
  sprite.material.map = fresh.material.map;
  sprite.material.needsUpdate = true;
}

/** 從後端拉取玩家與各居民的好感度計數 → 更新 myAffinity Map。
 *  連線後取一次；每次對話後再更新，讓指示燈即時反映互動。零 LLM。 */
async function refreshAffinity() {
  if (!myName || myName === "旅人") return;
  try {
    const resp = await fetch(`/voxel/affinity?player=${encodeURIComponent(myName)}`);
    if (!resp.ok) return;
    const data = await resp.json();
    for (const [rid, count] of Object.entries(data)) {
      myAffinity.set(rid, typeof count === "number" ? count : 0);
    }
  } catch (_) { /* 網路問題忽略 */ }
}

// 建一位居民的可見實體（簡單 voxel 人形：軀幹 + 頭 + 名牌 + 夢想副標籤 + 泡泡）。
// group.userData.rid 記居民 id，供點選 raycast 反查「點到的是哪位居民」。
function buildResident(id, name) {
  const group = new THREE.Group();
  group.userData.rid = id;
  // body：軀幹 + 頭 + 手臂全掛在一個 body 子 group 上，讓「開心彈跳/害怕縮身」能整體
  //（含名牌以外的身體）位移縮放，不動到頭頂名牌/泡泡的座標基準。
  const body = new THREE.Group();
  group.add(body);
  const torso = new THREE.Mesh(RES_TORSO_GEO, RES_BODY_MAT);
  torso.position.y = 0.5; // 腳底在 group 原點，軀幹中心 0.5
  body.add(torso);
  // 頭掛在頸關節 pivot 上（頸部約 y=1.02），繞 x 軸微傾＝低頭/抬頭表情。
  const headPivot = new THREE.Group();
  headPivot.position.y = 1.02;
  body.add(headPivot);
  const head = new THREE.Mesh(RES_HEAD_GEO, RES_HEAD_MAT);
  head.position.y = 0.23; // 頭中心相對頸關節（1.02+0.23=1.25，與原本一致）
  headPivot.add(head);
  // 手臂：pivot 掛在肩部（y≈0.95），臂往下垂、繞 pivot x 軸擺動＝揮手/搔頭/垂手。
  function resArm(x) {
    const pivot = new THREE.Group();
    pivot.position.set(x, 0.95, 0);
    const m = new THREE.Mesh(RES_ARM_GEO, RES_BODY_MAT);
    m.position.y = -0.275; // 臂頂貼在肩上、往下垂
    pivot.add(m);
    body.add(pivot);
    return pivot;
  }
  const armL = resArm(-0.32);
  const armR = resArm(0.32);
  const label = makeTextSprite(name, false);
  label.position.y = 2.0;
  group.add(label);
  // 夢想副標籤：名牌下方，有心願時才顯示（玩家看到「我說的話種下了什麼」）。
  const desireLabel = makeDesireSprite("");
  desireLabel.position.y = 1.6;
  desireLabel.visible = false;
  group.add(desireLabel);
  const bubble = makeTextSprite("", true);
  bubble.position.y = 2.55;
  bubble.visible = false;
  group.add(bubble);
  // 好感度指示燈（ROADMAP 656）：有好感才顯示，偏置在名牌右側不覆蓋名字。
  const affinityIndicator = makeAffinitySprite("");
  affinityIndicator.position.set(0.85, 2.05, 0);
  affinityIndicator.visible = false;
  group.add(affinityIndicator);
  // 心情指示燈（ROADMAP 676）：伺服器廣播 mood emoji，偏置在名牌左側與好感度左右對稱。
  const moodIndicator = makeMoodSprite("");
  moodIndicator.position.set(-0.85, 2.05, 0);
  moodIndicator.visible = false;
  group.add(moodIndicator);
  scene.add(group);
  return {
    group, body, headPivot, armL, armR,
    label, desireLabel, bubble, affinityIndicator, moodIndicator,
    lastName: name, lastSay: "", lastDesire: "", lastAffinity: "", lastMood: "",
    // 肢體動畫狀態（居民表情/肢體 v1）：
    gestPhase: Math.random() * Math.PI * 2, // 起始相位打散，別讓全村同步呼吸
    mood: "",        // 當前心情 emoji（驅動彈跳/垂頭/搔頭）
    humming: false,  // 是否正哼歌（say 以 ♪ 起頭）→ 持續搖擺哼唱
    wavePulse: 0,    // 打招呼/道賀揮手的一次性脈衝（新泡泡出現時觸發，倒數歸零）
  };
}

// 依前端已知資訊（心情 emoji / 說話 / 附近暗影）程序化驅動居民的柔和肢體語言。
// 純三角函數補間、無新後端欄位；只算鏡頭附近的居民（FPS 鐵律）。療癒調性＝動作慢、小、可愛。
function animateResident(ent, dt, fearNear) {
  const b = ent.body, hp = ent.headPivot, aL = ent.armL, aR = ent.armR;
  // 距離門檻外：把身體回正到中性姿態一次即可，不再逐幀補間（省算）。
  const dx = ent.group.position.x - player.x, dz = ent.group.position.z - player.z;
  if (dx * dx + dz * dz > RES_GESTURE_DIST * RES_GESTURE_DIST) {
    b.position.y = 0; b.scale.set(1, 1, 1); b.rotation.z = 0;
    hp.rotation.x = 0; hp.rotation.z = 0; aL.rotation.x = 0; aL.rotation.z = 0;
    aR.rotation.x = 0; aR.rotation.z = 0;
    return;
  }
  ent.gestPhase += dt;
  const t = ent.gestPhase;
  const k = Math.min(1, dt * 6); // 姿態平滑係數（往目標補間，避免瞬跳）

  // 目標姿態（預設中性），依情緒疊加。
  let bobY = 0, sway = 0, crouch = 0, shiver = 0;
  let headTilt = 0, headDroop = 0;
  let armLx = 0, armRx = 0, armScratch = 0, waveAmt = 0;

  const mood = ent.mood;
  const asleep = mood === "💤";

  if (fearNear && !asleep) {
    // 害怕（附近有暗影）：縮身、微微後仰顫抖、雙手內收——最強、蓋過其他情緒。
    crouch = 0.16;
    shiver = Math.sin(t * 22) * 0.03;
    headDroop = -0.15; // 微微仰頭警戒
    armLx = 0.5; armRx = 0.5; // 雙手抬起護在身前
  } else if (asleep) {
    // 睡著：幾乎不動，只有極緩的呼吸起伏（頭微垂）。
    bobY = Math.sin(t * 1.2) * 0.012;
    headDroop = 0.28;
  } else if (ent.humming) {
    // 哼歌：左右輕搖 + 上下輕彈，最歡快可愛。
    sway = Math.sin(t * 3.2) * 0.11;
    bobY = Math.abs(Math.sin(t * 3.2)) * 0.05;
    headTilt = Math.sin(t * 3.2) * 0.08;
  } else if (mood === "😊" || mood === "🙂") {
    // 開心：輕輕上下彈跳（越開心越明顯），偶爾點頭。
    const amp = mood === "😊" ? 0.05 : 0.03;
    bobY = Math.abs(Math.sin(t * 2.4)) * amp;
    headTilt = Math.sin(t * 1.6) * 0.04;
  } else if (mood === "😔") {
    // 難過/寂寞：頭微垂、身體極緩慢地小幅搖，動作明顯變慢。
    headDroop = 0.22;
    sway = Math.sin(t * 1.0) * 0.03;
  } else if (mood === "🤔") {
    // 思考/發明：搔頭——右手抬到頭側來回蹭，頭跟著微側。
    armScratch = 1;
    armRx = 2.2 + Math.sin(t * 8) * 0.25; // 抬手到頭
    headTilt = 0.12;
  } else {
    // 中性（😐 或無 emoji）：極輕的自然呼吸起伏，維持「活著」的感覺。
    bobY = Math.sin(t * 1.5) * 0.012;
  }

  // 說話/道賀揮手：wavePulse 觸發時右手抬起來回揮（一次性、倒數歸零），蓋過上面的臂目標。
  if (ent.wavePulse > 0) {
    ent.wavePulse = Math.max(0, ent.wavePulse - dt);
    waveAmt = 1;
    armRx = 2.4;                 // 抬到肩上
    armScratch = 0;
  }

  // ── 把目標姿態平滑補間到實際 mesh ──
  b.position.y += ((bobY - crouch) - b.position.y) * k;
  b.rotation.z += ((sway + shiver) - b.rotation.z) * k;
  // 縮身時整體略壓扁（縮 y、微胖），害怕/難過收攏；平常回 1。
  const targetScaleY = 1 - crouch * 0.9;
  b.scale.y += (targetScaleY - b.scale.y) * k;
  hp.rotation.x += (headDroop - hp.rotation.x) * k;
  hp.rotation.z += (headTilt - hp.rotation.z) * k;
  // 揮手時右手繞 z 軸來回擺（招手感）；搔頭時右手在頭側小幅蹭。
  const waveZ = waveAmt ? Math.sin(t * 9) * 0.5 : (armScratch ? Math.sin(t * 8) * 0.15 : 0);
  aR.rotation.x += (armRx - aR.rotation.x) * k;
  aR.rotation.z += (waveZ - aR.rotation.z) * k;
  aL.rotation.x += (armLx - aL.rotation.x) * k;
  aL.rotation.z += (0 - aL.rotation.z) * k;
}

// 依伺服器快照更新所有居民（位置/朝向/名字/說的話）。新出現的就建、消失的就移除。
function updateResidents(list) {
  const seen = new Set();
  for (const r of list) {
    seen.add(r.id);
    let ent = residents.get(r.id);
    if (!ent) { ent = buildResident(r.id, r.name); residents.set(r.id, ent); }
    ent.group.position.set(r.x, r.y, r.z);
    ent.group.rotation.y = r.yaw || 0;
    if (r.name !== ent.lastName) { setSpriteText(ent.label, r.name, false); ent.lastName = r.name; }
    // 夢想副標籤：有 desire 就顯示「💭 心願文字」，沒有就隱藏。
    const desire = r.desire || "";
    if (desire !== ent.lastDesire) {
      ent.lastDesire = desire;
      if (desire) { setDesireText(ent.desireLabel, "💭 " + desire); ent.desireLabel.visible = true; }
      else { ent.desireLabel.visible = false; }
    }
    const say = r.say || "";
    if (say !== ent.lastSay) {
      ent.lastSay = say;
      if (say) {
        setSpriteText(ent.bubble, say, true); ent.bubble.visible = true;
        chatLogAppend("res", r.name, say, r.id); // 泡泡同步進聊天窗（去重會併掉截斷版）
        // 哼歌 v1（ROADMAP 788）：say 以音符「♪」起頭＝這是一段歌聲，於頭頂生成飄浮音符。
        const isHum = say.charAt(0) === "♪";
        if (isHum) spawnHumNotes(r.x, r.y, r.z);
        // 居民表情/肢體 v1：哼歌＝持續搖擺；一般說話＝觸發一次「打招呼／道賀」揮手脈衝。
        ent.humming = isHum;
        if (!isHum) ent.wavePulse = 1.4; // 揮手約 1.4 秒後收手
      }
      else { ent.bubble.visible = false; ent.humming = false; }
    }
    // 好感度指示燈（ROADMAP 656）：依 myAffinity 決定顯示哪種心型（sig 保護不重建貼圖）。
    const affCount = myAffinity.get(r.id) || 0;
    const emoji = affinityEmoji(affCount);
    if (emoji !== ent.lastAffinity) {
      ent.lastAffinity = emoji;
      if (emoji) { setAffinityEmoji(ent.affinityIndicator, emoji); ent.affinityIndicator.visible = true; }
      else { ent.affinityIndicator.visible = false; }
    }
    // 心情指示燈（ROADMAP 676）：伺服器動態計算並廣播 mood emoji，sig 保護不重建貼圖。
    const moodEmoji = r.mood || "";
    if (moodEmoji !== ent.lastMood) {
      ent.lastMood = moodEmoji;
      if (moodEmoji) { setMoodEmoji(ent.moodIndicator, moodEmoji); ent.moodIndicator.visible = true; }
      else { ent.moodIndicator.visible = false; }
    }
    ent.mood = moodEmoji; // 居民表情/肢體 v1：驅動彈跳/垂頭/搔頭的當前心情
    // 距離 LOD：遠到接近霧盡頭就整個隱藏（省繪製，不崩 FPS）。
    const dx = r.x - player.x, dz = r.z - player.z;
    ent.group.visible = (dx * dx + dz * dz) < (RES_VISIBLE_DIST * RES_VISIBLE_DIST);
  }
  for (const [id, ent] of residents) {
    if (!seen.has(id)) { scene.remove(ent.group); residents.delete(id); }
  }
}

// 每幀推進所有居民的肢體語言（居民表情/肢體 v1）。應在 rAF 迴圈呼叫。
// 害怕偵測純前端：居民附近（RES_FEAR_DIST 內）有可見暗影小靈就縮身顫抖，不需後端旗標。
function animateResidents(dt) {
  if (!residents.size) return;
  for (const ent of residents.values()) {
    if (!ent.group.visible) continue; // 遠處已隱藏的不算
    let fearNear = false;
    if (shadowEnts.size) {
      const rx = ent.group.position.x, rz = ent.group.position.z;
      for (const s of shadowEnts.values()) {
        const dx = s.group.position.x - rx, dz = s.group.position.z - rz;
        if (dx * dx + dz * dz < RES_FEAR_DIST * RES_FEAR_DIST) { fearNear = true; break; }
      }
    }
    animateResident(ent, dt, fearNear);
  }
}

// 建一隻野兔（純點綴：身體 + 兩隻立耳，無名牌無泡泡）。
function buildRabbit() {
  const group = new THREE.Group();
  const body = new THREE.Mesh(RABBIT_BODY_GEO, RABBIT_BODY_MAT);
  body.position.y = 0.14;
  group.add(body);
  const earL = new THREE.Mesh(RABBIT_EAR_GEO, RABBIT_EAR_MAT);
  earL.position.set(-0.08, 0.36, 0.05);
  group.add(earL);
  const earR = new THREE.Mesh(RABBIT_EAR_GEO, RABBIT_EAR_MAT);
  earR.position.set(0.08, 0.36, 0.05);
  group.add(earR);
  scene.add(group);
  return { group };
}

// 建一條魚（水中游魚 v1，ROADMAP 848：身體 + 尾鰭，無名牌無泡泡，比野兔更小更扁）。
function buildFish() {
  const group = new THREE.Group();
  const body = new THREE.Mesh(FISH_BODY_GEO, FISH_BODY_MAT);
  group.add(body);
  const tail = new THREE.Mesh(FISH_TAIL_GEO, FISH_TAIL_MAT);
  tail.position.set(0, 0, -0.2);
  group.add(tail);
  scene.add(group);
  return { group };
}

// 建一隻雞（放養雞 v1，自主提案切片 ROADMAP 870：身體 + 一小塊雞冠，無名牌無泡泡）。
function buildChicken() {
  const group = new THREE.Group();
  const body = new THREE.Mesh(CHICKEN_BODY_GEO, CHICKEN_BODY_MAT);
  body.position.y = 0.16;
  group.add(body);
  const comb = new THREE.Mesh(CHICKEN_COMB_GEO, CHICKEN_COMB_MAT);
  comb.position.set(0, 0.34, 0.14);
  group.add(comb);
  scene.add(group);
  return { group };
}

// 依伺服器快照更新所有環境生物（位置/朝向）。新出現的依 `kind` 建對應模型、
// 消失的就移除（同構 updateResidents）。
function updateWildlife(list) {
  const seen = new Set();
  for (const w of list) {
    seen.add(w.id);
    let ent = wildlifeEnts.get(w.id);
    if (!ent) {
      ent = w.kind === "fish" ? buildFish() : w.kind === "chicken" ? buildChicken() : buildRabbit();
      ent.group.userData.wid = w.id; // 餵野兔馴服 v1：raycast 命中後回查 id 用。
      ent.group.userData.wkind = w.kind; // 放養雞 v1：前端據此決定送 feed_wildlife 還是 feed_chicken。
      wildlifeEnts.set(w.id, ent);
    }
    ent.group.position.set(w.x, w.y, w.z);
    ent.group.rotation.y = w.yaw || 0;
    // 為馴服的動物取名 v1（ROADMAP 895）：記住是否已馴服（點一下能否取名，伺服器仍權威複驗），
    // 已命名的小夥伴頭頂掛上專屬名牌（懶建立、內容變更才重繪貼圖，守 FPS 鐵律）。
    ent.group.userData.wtamed = !!w.tamed;
    // 寵物指令「安置／召回」v1（ROADMAP 898）：記住命名與待命狀態（「再點一下」據此決定
    // 該送安置還是召回），已安置的小夥伴名牌旁掛 💤 待命標記，一眼看得出牠在等你。
    ent.petName = w.name || null;
    ent.wsettled = !!w.settled;
    if (w.name) {
      const labelStr = w.name + (w.settled ? " 💤" : "");
      if (!ent.nameLabel) {
        ent.nameLabel = makeTextSprite(labelStr, false);
        ent.nameLabel.position.y = 1.0; // 浮在小動物頭頂（軀幹矮，比居民名牌低）
        ent.nameLabel.scale.set(1.8, 0.45, 1); // 比居民名牌略小，配合小動物體型
        ent.group.add(ent.nameLabel);
        ent.labelText = labelStr;
      } else if (ent.labelText !== labelStr) {
        setSpriteText(ent.nameLabel, labelStr, false); // 內容變才重繪貼圖（守 FPS 鐵律）
        ent.labelText = labelStr;
      }
    }
    // 臨危依偎 v1（ROADMAP 903）：受驚依偎中的小夥伴頭頂掛一枚受驚表情（😨 等）。
    // 懶建立、可見性切換、內容變才重繪貼圖（守 FPS 鐵律，非受驚時只是隱藏、零重繪）。
    const emoteStr = w.emote || "";
    if (emoteStr) {
      if (!ent.emoteLabel) {
        ent.emoteLabel = makeTextSprite(emoteStr, false);
        ent.emoteLabel.position.y = 1.5; // 浮在名牌(1.0)之上，兩者不重疊
        ent.emoteLabel.scale.set(0.9, 0.9, 1);
        ent.group.add(ent.emoteLabel);
        ent.emoteText = emoteStr;
      } else {
        if (ent.emoteText !== emoteStr) { setSpriteText(ent.emoteLabel, emoteStr, false); ent.emoteText = emoteStr; }
        ent.emoteLabel.visible = true;
      }
    } else if (ent.emoteLabel) {
      ent.emoteLabel.visible = false;
    }
    const dx = w.x - player.x, dz = w.z - player.z;
    ent.group.visible = (dx * dx + dz * dz) < (WILDLIFE_VISIBLE_DIST * WILDLIFE_VISIBLE_DIST);
  }
  for (const [id, ent] of wildlifeEnts) {
    if (!seen.has(id)) {
      // 名牌貼圖回收，避免 GPU 記憶體洩漏（比照名牌工廠 setSpriteText 的 dispose 慣例）。
      if (ent.nameLabel && ent.nameLabel.material.map) ent.nameLabel.material.map.dispose();
      // 臨危依偎 v1（903）：受驚表情貼圖一併回收（同款 dispose 慣例）。
      if (ent.emoteLabel && ent.emoteLabel.material.map) ent.emoteLabel.material.map.dispose();
      scene.remove(ent.group);
      wildlifeEnts.delete(id);
    }
  }
}

// ── 點居民 → 對話（raycast 點選 + 直式對話框）────────────────────────────────
// 點到居民（在互動距離內）就開對話框；送出 → 後端以該居民人設呼 LLM → 回 talk 訊息。
const raycaster = new THREE.Raycaster();
const TALK_REACH = 16; // 可對話的最遠距離（方塊）：太遠的居民點不到，貼近「在你附近的人」
// 從螢幕座標 raycast 找命中的居民 id（命中且在 TALK_REACH 內才回 id，否則 null）。
function pickResident(clientX, clientY) {
  const rect = renderer.domElement.getBoundingClientRect();
  const ndc = new THREE.Vector2(
    ((clientX - rect.left) / rect.width) * 2 - 1,
    -((clientY - rect.top) / rect.height) * 2 + 1
  );
  raycaster.setFromCamera(ndc, camera);
  const pickables = [];
  for (const ent of residents.values()) {
    if (ent.group.visible) ent.group.traverse((o) => { if (o.isMesh) pickables.push(o); });
  }
  if (!pickables.length) return null;
  const hits = raycaster.intersectObjects(pickables, false);
  if (!hits.length || hits[0].distance > TALK_REACH) return null;
  // 沿父鏈往上找帶 rid 的 group。
  let obj = hits[0].object;
  while (obj && !(obj.userData && obj.userData.rid)) obj = obj.parent;
  return obj && obj.userData ? obj.userData.rid : null;
}

// 餵野兔馴服 v1（自主提案切片）：同款 raycast 挑選，改對象為 wildlife（野兔/魚/雞）實體。
// 只是「準心對到誰」的前端提示，伺服器仍會權威複驗真正的觸及範圍（vwild::TAME_REACH）。
// 放養雞 v1（ROADMAP 870）：回傳同時帶 `kind`，讓呼叫端決定送 feed_wildlife 還是 feed_chicken
// （對到魚或種類不符時，伺服器一律靜默拒絕，不影響手感）。
const WILDLIFE_PICK_REACH = 8;
function pickWildlife(clientX, clientY) {
  const rect = renderer.domElement.getBoundingClientRect();
  const ndc = new THREE.Vector2(
    ((clientX - rect.left) / rect.width) * 2 - 1,
    -((clientY - rect.top) / rect.height) * 2 + 1
  );
  raycaster.setFromCamera(ndc, camera);
  const pickables = [];
  for (const ent of wildlifeEnts.values()) {
    if (ent.group.visible) ent.group.traverse((o) => { if (o.isMesh) pickables.push(o); });
  }
  if (!pickables.length) return null;
  const hits = raycaster.intersectObjects(pickables, false);
  if (!hits.length || hits[0].distance > WILDLIFE_PICK_REACH) return null;
  let obj = hits[0].object;
  while (obj && !(obj.userData && obj.userData.wid)) obj = obj.parent;
  if (!obj || !obj.userData) return null;
  return { id: obj.userData.wid, kind: obj.userData.wkind };
}

// 為馴服的動物取名 v1（ROADMAP 895）：準心/輕點對到一隻「已馴服」的小夥伴 → 跳出取名輸入。
// 回傳 true 表示這一下已被命名流程接手（呼叫端不要再落到挖掘/挖擊）；false 表示沒對到動物。
// 伺服器仍會權威複驗（存在＋已馴服＋距離＋清洗名字），前端只是手感提示、不自報合法性。
// 點一下馴服的小夥伴：還沒取名 → 替牠取名（895）；已取名 → 在「跟著你」與「在這待命」
// 之間切換（寵物指令「安置／召回」v1，ROADMAP 898）。先取名建立羈絆，之後點牠就是指揮牠。
function tryPetInteract(clientX, clientY) {
  const pick = pickWildlife(clientX, clientY);
  if (!pick) return false;
  const ent = wildlifeEnts.get(pick.id);
  if (!ent || !ent.group.userData.wtamed) {
    showMsg("先餵食馴服牠，牠才願意讓你取名 🥕");
    return true; // 對到動物就接手這一下（別挖到牠背後的地）
  }
  // 已取名 → 送安置／召回切換（後端權威判定並回 pet_command_ok 帶當前狀態＋暖句）。
  if (ent.petName) {
    ws.send(JSON.stringify({ t: "pet_command", id: pick.id }));
    return true;
  }
  // 還沒取名 → 取名（取完名字，之後再點就是指揮牠待命／跟上）。
  const text = window.prompt("替這隻小夥伴取個名字吧（最多 12 字）", "");
  if (text !== null && text.trim() !== "") {
    ws.send(JSON.stringify({ t: "name_pet", id: pick.id, name: text }));
  }
  return true;
}

// 對話框 DOM + 狀態。
const chatEl = document.getElementById("chat");
const chatTitleEl = document.getElementById("chatTitle");
const chatLogEl = document.getElementById("chatLog");
const chatQuickEl = document.getElementById("chatQuick");
const chatInputEl = document.getElementById("chatInput");
const chatSendEl = document.getElementById("chatSend");
let chatRid = null;          // 目前對話的居民 id
let lastTalkReply = null;    // 最近一次居民回覆（QA 用）
let thinkingEl = null;       // 目前正在顯示的「思考中」動畫元素（null 代表沒有）

function appendMsg(kind, text) {
  if (!chatLogEl) return;
  const d = document.createElement("div");
  d.className = "msg " + kind;
  d.textContent = text;
  chatLogEl.appendChild(d);
  chatLogEl.scrollTop = chatLogEl.scrollHeight;
}

// 顯示「思考中」動畫指示器（居民收到訊息後立即顯示，等真回覆取代）。
// 若已有一個思考中元素（連發），先移除舊的再建新的。
function showThinking(name) {
  if (!chatLogEl) return;
  removeThinking(); // 清掉上一輪殘留
  const d = document.createElement("div");
  d.className = "msg thinking";
  // 顯示「居民名 思考中」並帶跳動點點（純 CSS animation，不用 JS timer）。
  const label = document.createElement("span");
  label.className = "thinking-label";
  label.textContent = (name || "居民") + " 思考中";
  const dots = document.createElement("span");
  dots.className = "thinking-dots";
  dots.setAttribute("aria-hidden", "true");
  d.appendChild(label);
  d.appendChild(dots);
  chatLogEl.appendChild(d);
  chatLogEl.scrollTop = chatLogEl.scrollHeight;
  thinkingEl = d;
}

// 移除「思考中」動畫元素（真回覆到了後呼叫）。
function removeThinking() {
  if (thinkingEl && thinkingEl.parentNode) {
    thinkingEl.parentNode.removeChild(thinkingEl);
  }
  thinkingEl = null;
}

// ── 麥塊式聊天記錄窗（左下常駐、可開合可捲）────────────────────────────────────
// 頭上泡泡臨場但難讀、長句被截、消失看不回；這裡把「範圍內所有說的話」完整存成一份
// 可讀可捲的 log（泡泡可截、log 不截）。凡是會觸發泡泡的訊息來源都經同一入口
// chatLogAppend()：自己 speak / 附近居民說話廣播(say) / 居民 talk 回覆 / 其他玩家說話。
//
// 【資安】訊息內容一律用 textContent 寫入（見下），絕不 innerHTML——別人訊息裡的
// <script>/HTML 只會顯示成純文字、不會在你畫面上執行（後端 sanitize_talk_text 只 trim/截長，
// 不濾 HTML，跳脫責任在前端顯示層）。
const CHATLOG_MAX = 50;         // 最多保留最近 N 條（超過砍最舊）
const CHATLOG_FADE_SECS = 5;    // 收合態：新訊息亮顯示幾秒後淡化不擋畫面
const CHATLOG_DEDUP_SECS = 6;   // 去重時間窗：同人、其一為另一前綴＝同一句（截斷泡泡↔完整回覆）
const CHATLOG_TEXT_CAP = 400;   // DOM 安全上限（> 後端最長回覆 300，實務不會截到真訊息）
const chatLogWinEl = document.getElementById("chatLogWin");
const chatLogBodyEl = document.getElementById("chatLogBody");
const chatLogHeadEl = document.getElementById("chatLogHead");
const chatLogToggleEl = document.getElementById("chatLogToggle");
let chatLogEntries = []; // [{ line, textEl, kind, speaker, text, t }]，最新在尾端
let chatLogExpanded = false;
let chatLogFadeTO = null;

// 居民/其他玩家 → 穩定顏色（同一位每次同色，一眼分辨誰在講）。純前端雜湊挑色盤。
const CHATLOG_PALETTE = ["#a0e8b0", "#f5b8d8", "#c8b0ff", "#ffc07a", "#7fe0d8", "#ffe08a", "#b8d8ff", "#e0a0f0"];
const chatLogColorCache = new Map();
function chatLogColorFor(key) {
  const k = key || "";
  let c = chatLogColorCache.get(k);
  if (c) return c;
  let h = 0;
  for (let i = 0; i < k.length; i++) h = (h * 31 + k.charCodeAt(i)) >>> 0;
  c = CHATLOG_PALETTE[h % CHATLOG_PALETTE.length];
  chatLogColorCache.set(k, c);
  return c;
}

// 亮起（新訊息或展開時）：清淡化、重設幾秒後再淡化的計時（用 setTimeout，不進 render 迴圈）。
function chatLogBright() {
  if (!chatLogWinEl) return;
  chatLogWinEl.classList.remove("faded");
  if (chatLogFadeTO) clearTimeout(chatLogFadeTO);
  chatLogFadeTO = setTimeout(() => {
    if (!chatLogExpanded && chatLogWinEl) chatLogWinEl.classList.add("faded");
  }, CHATLOG_FADE_SECS * 1000);
}

// 展開/收合：展開＝可捲動完整歷史（貼底）、收合＝只露最近幾行的精簡條。
function chatLogSetExpanded(on) {
  chatLogExpanded = !!on;
  if (!chatLogWinEl) return;
  chatLogWinEl.classList.toggle("expanded", chatLogExpanded);
  chatLogWinEl.classList.toggle("collapsed", !chatLogExpanded);
  if (chatLogToggleEl) chatLogToggleEl.textContent = chatLogExpanded ? "▼" : "▲";
  if (chatLogExpanded) {
    chatLogWinEl.classList.remove("faded");
    if (chatLogBodyEl) chatLogBodyEl.scrollTop = chatLogBodyEl.scrollHeight;
  } else {
    chatLogBright();
  }
}

/**
 * 把一句話 append 進聊天 log（完整、不截斷）。
 * kind: "self"（自己）/ "res"（居民）/ "other"（其他玩家）/ "sys"（系統）。
 * colorKey: 上色用的穩定鍵（居民 id / 玩家 id）；self/sys 用固定 CSS 色不需 colorKey。
 * 去重：同一句話可能同時走「截斷泡泡(say)」與「完整回覆(talk)」兩路 → 近期同 speaker、
 * 其一為另一前綴時視為同句，保留較長(完整)版、不重複成兩行。
 */
function chatLogAppend(kind, speaker, text, colorKey) {
  if (!chatLogBodyEl) return;
  let t = (text == null ? "" : String(text)).trim();
  if (!t) return;
  if (t.length > CHATLOG_TEXT_CAP) t = t.slice(0, CHATLOG_TEXT_CAP);
  const name = (speaker == null ? "" : String(speaker)).trim() || "？";
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : Date.now();

  // 去重：由尾往前掃時間窗內、同 kind+speaker 的既有行。
  for (let i = chatLogEntries.length - 1; i >= 0; i--) {
    const e = chatLogEntries[i];
    if (now - e.t > CHATLOG_DEDUP_SECS * 1000) break; // 更早的不用再比
    if (e.kind !== kind || e.speaker !== name) continue;
    if (e.text === t || e.text.startsWith(t) || t.startsWith(e.text)) {
      if (t.length > e.text.length) { e.text = t; e.textEl.textContent = t; } // 截斷→完整升級
      e.t = now;
      chatLogBright();
      return;
    }
  }

  // 新的一行：名字前綴 + 完整訊息，皆用 textContent（防 XSS）。
  const line = document.createElement("div");
  line.className = "clog-line clog-" + kind;
  const nameEl = document.createElement("span");
  nameEl.className = "clog-name";
  nameEl.textContent = name + "：";
  if (kind === "res" || kind === "other") nameEl.style.color = chatLogColorFor(colorKey || name);
  const textEl = document.createElement("span");
  textEl.className = "clog-text";
  textEl.textContent = t;
  line.appendChild(nameEl);
  line.appendChild(textEl);
  chatLogBodyEl.appendChild(line);
  chatLogEntries.push({ line, textEl, kind, speaker: name, text: t, t: now });

  // 只保留最近 N 條，超過砍最舊。
  while (chatLogEntries.length > CHATLOG_MAX) {
    const old = chatLogEntries.shift();
    if (old.line.parentNode) old.line.parentNode.removeChild(old.line);
  }
  if (chatLogExpanded) chatLogBodyEl.scrollTop = chatLogBodyEl.scrollHeight;
  chatLogBright();
}

if (chatLogHeadEl) chatLogHeadEl.addEventListener("click", () => chatLogSetExpanded(!chatLogExpanded));

// 開對話框（換對象就清空對話紀錄）。
function openChat(rid, name) {
  if (!chatEl) return;
  if (chatRid !== rid) {
    // 換居民：清空對話、移除思考中指示器、顯示「你走近了…」
    thinkingEl = null; // 舊元素連同 innerHTML 一起清掉，不用再 removeChild
    chatLogEl.innerHTML = "";
    appendMsg("sys", "你走近了 " + (name || "居民"));
  }
  chatRid = rid;
  chatTitleEl.textContent = name || "居民";
  releaseMouse(); // 桌機：開對話要放開滑鼠鎖定，游標才能打字
  chatEl.style.display = "flex";
  // 開定向對話 modal 時收起範圍說話列，避免兩者重疊。
  setSpeakBarShown(false);
  updateGiftBtn(); // 贈禮 v1：更新按鈕顯示哪件物品
  hideTradeOffer(); // 換居民時清掉舊交易提案（不同居民的提案不共用）
}
function closeChat() {
  if (chatEl) chatEl.style.display = "none";
  setSpeakBarShown(true); // 桌機恢復底部常駐輸入列；觸控維持收起（由「💬 說」鈕再開）
}

// 範圍說話輸入列顯隱：桌機常駐（用 inline display），觸控靠 .open class（預設收起、不佔位）。
function setSpeakBarShown(show) {
  const sb = document.getElementById("speakBar");
  if (!sb) return;
  if (document.body.classList.contains("touch")) {
    // 觸控：只負責「收起」（開 modal / 送出時）；展開一律由使用者點鈕，避免自動冒出壓拇指區。
    if (!show) sb.classList.remove("open");
  } else {
    sb.style.display = show ? "flex" : "none";
  }
}

// 送一句話給目前對話的居民（指定對象＝點居民 / 走近面對）。
function sendTalk(text) {
  const t = (text || "").trim();
  if (!t || !chatRid || !wsReady) return;
  ws.send(JSON.stringify({ t: "talk", resident_id: chatRid, text: t.slice(0, 200) }));
  appendMsg("me", "你：" + t);
  showMyBubble(t); // embodied：自己頭上也冒泡（話活在世界裡）
  chatLogAppend("self", myName || "你", t); // 自己說的也進左下聊天窗（完整）
}

// embodied 靠近說話 v1：範圍「說話」——不指定居民，伺服器挑半徑內最近/面對者回話，
// 其餘附近居民旁聽（進記憶、偶爾搭話）。不開 modal，回覆走世界裡的頭上泡泡。
function sendSpeak(text) {
  const t = (text || "").trim();
  if (!t || !wsReady) return;
  ws.send(JSON.stringify({ t: "talk", text: t.slice(0, 200) })); // 無 resident_id = 範圍說話
  showMyBubble(t); // 自己頭上立即冒泡（零延遲、不等伺服器來回）
  chatLogAppend("self", myName || "你", t); // 自己說的也進左下聊天窗（完整）
}

if (chatEl) {
  document.getElementById("chatClose").addEventListener("click", closeChat);
  chatSendEl.addEventListener("click", () => { sendTalk(chatInputEl.value); chatInputEl.value = ""; });
  chatInputEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { sendTalk(chatInputEl.value); chatInputEl.value = ""; e.preventDefault(); }
  });
  // 快捷句：不用打字也能互動（手機友善）。
  for (const q of ["你好！", "你在做什麼？", "這裡是哪裡？"]) {
    const b = document.createElement("div");
    b.className = "qbtn"; b.textContent = q;
    b.addEventListener("click", () => sendTalk(q));
    chatQuickEl.appendChild(b);
  }
  // 日記鈕：開啟當前對話居民的日記。
  const diaryBtnEl = document.getElementById("chatDiary");
  if (diaryBtnEl) diaryBtnEl.addEventListener("click", () => { if (chatRid) openDiary(chatRid); });
  // 贈禮鈕：送背包最多的一件給當前居民（ROADMAP 660）。
  const giftBtnEl = document.getElementById("chatGift");
  if (giftBtnEl) giftBtnEl.addEventListener("click", () => { if (chatRid) trySendGift(); });
  // 交易鈕：向當前居民請求以物易物（ROADMAP 670）。
  const tradeBtnEl = document.getElementById("chatTrade");
  if (tradeBtnEl) tradeBtnEl.addEventListener("click", () => { if (chatRid) tryRequestTrade(); });
  // 接受交易按鈕（用材料換，v1 原行為）。
  const tradeAcceptBtnEl = document.getElementById("tradeAcceptBtn");
  if (tradeAcceptBtnEl) tradeAcceptBtnEl.addEventListener("click", () => { if (chatRid) sendTradeAccept(chatRid, false); });
  // 付幣代替湊材料 v1（ROADMAP 874）：改直接付乙太幣成交。
  const tradeAcceptCoinBtnEl = document.getElementById("tradeAcceptCoinBtn");
  if (tradeAcceptCoinBtnEl) tradeAcceptCoinBtnEl.addEventListener("click", () => { if (chatRid) sendTradeAccept(chatRid, true); });
}

// ── 常駐說話輸入列（embodied 靠近說話 v1）─────────────────────────────────────
// 底部常駐輸入列：打字 → 範圍「說話」（不必先點居民）。手機/直式友善、不開 modal。
const speakInputEl = document.getElementById("speakInput");
const speakSendEl = document.getElementById("speakSend");
const speakBarEl = document.getElementById("speakBar");
const speakToggleEl = document.getElementById("speakToggle");
if (speakInputEl && speakSendEl) {
  const fireSpeak = () => {
    sendSpeak(speakInputEl.value);
    speakInputEl.value = "";
    speakInputEl.blur(); // 送完收鍵盤焦點，讓 WASD 等遊戲鍵恢復作用
    setSpeakBarShown(false); // 觸控：送完收起輸入列（桌機無影響，仍常駐）
  };
  speakSendEl.addEventListener("click", fireSpeak);
  speakInputEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { fireSpeak(); e.preventDefault(); }
  });
  // 觸控：失焦（點別處/收鍵盤）即收起輸入列，避免它常駐壓拇指區。
  // 延遲一拍讓「說」送出鈕的 click 先觸發（點鈕會先讓 input 失焦）。
  speakInputEl.addEventListener("blur", () => {
    if (document.body.classList.contains("touch")) {
      setTimeout(() => { if (document.activeElement !== speakInputEl) setSpeakBarShown(false); }, 160);
    }
  });
}
// 觸控「💬 說」鈕：展開/收合範圍說話輸入列（桌機此鈕以 CSS 隱藏、不觸發）。
if (speakToggleEl && speakBarEl && speakInputEl) {
  speakToggleEl.addEventListener("click", (e) => {
    e.preventDefault();
    const opening = !speakBarEl.classList.contains("open");
    speakBarEl.classList.toggle("open");
    if (opening) speakInputEl.focus(); // 展開即聚焦，鍵盤直接彈出可打字
  });
}

// ── 居民贈禮 v1（ROADMAP 660）────────────────────────────────────────────────
// 把採來的材料化作一份心意送給居民；居民記得你的照料，好感度 +2。

/// 不可作為禮物的 block_id（純 inventory 物品 / 不合語意送出）。
const GIFT_EXCLUDED = new Set([0, 7, 12, DOOR_OPEN, CARROT_SEEDED, POTATO_SEEDED]); // Air / Water / FarmSoilSeeded / DoorOpen / CarrotSeeded / PotatoSeeded（伺服器狀態，不可贈）

/**
 * 從背包（myInv: Map<blockId, count>）挑出最佳禮物。
 * 策略：選存量最多的可贈物品（最不稀缺）；同量以 blockId 小者優先（確定性）。
 * 空背包或無可贈物品回 null。
 * 確定性純函式，壞值（非 Map / 空 Map）安全回 null。
 * @param {Map<number,number>} inv
 * @returns {{ blockId: number, count: number }|null}
 */
function giftPickItem(inv) {
  if (!(inv instanceof Map)) return null;
  let best = null;
  for (const [bid, cnt] of inv) {
    if (GIFT_EXCLUDED.has(bid) || cnt <= 0) continue;
    if (!best || cnt > best.count || (cnt === best.count && bid < best.blockId)) {
      best = { blockId: bid, count: cnt };
    }
  }
  return best;
}

/** 更新「🎁 贈禮」按鈕顯示（呼叫於開對話框 / inv 改變後）。 */
function updateGiftBtn() {
  const el = document.getElementById("chatGift");
  if (!el) return;
  const pick = giftPickItem(myInv);
  if (!pick) {
    el.textContent = "🎁 贈禮";
    el.classList.add("gift-empty");
  } else {
    const iname = BLOCK_NAME[pick.blockId] || "物品";
    el.textContent = "🎁 贈" + iname;
    el.classList.remove("gift-empty");
  }
}

let lastGiftMs = 0; // 贈禮本地冷卻（防連按）

/** 執行贈禮：消耗最多的那件物品送給當前居民（ROADMAP 660）。 */
function trySendGift() {
  if (!wsReady || !chatRid) return;
  const now = Date.now();
  if (now - lastGiftMs < 1500) return; // 1.5 秒本地冷卻
  const pick = giftPickItem(myInv);
  if (!pick) {
    appendMsg("sys", "背包是空的，先去採集一些材料吧～");
    return;
  }
  lastGiftMs = now;
  ws.send(JSON.stringify({ t: "gift", resident_id: chatRid, item_id: pick.blockId }));
}

// ── 居民以物易物（ROADMAP 670）───────────────────────────────────────────────
// 玩家點「⇌ 交易」→ 伺服器回 trade_offer → 前端顯示提案橫幅 → 玩家點接受 → 伺服器執行交易。

// ── 吃東西回復飢餓 v1（玩家生存指標·溫和版）+ 親手煮的暖食自己也能享用 v1（779）──────
// 所有食物都能填飽肚子（對齊後端 voxel_player_stats::food_nutrition）：生穀/蔬果/魚
// ＋熟食（麵包/烤魚/烤地薯/野菜暖湯）＋加工（莓果醬）。熟食另有那份暖意 social 交織（後端判定）。
// 註：68=乙太煙火不是食物。
const EDIBLE_FOODS = new Set([
  WHEAT, BREAD, CARROT, POTATO, BAKED_POTATO,
  FISH, AETHER_FISH, COOKED_FISH, STEW, BERRY, JAM,
]);

/** 目前手持的方塊是否食物（手持食物時放置鈕會變「吃」）。 */
function heldIsFood() {
  const b = (typeof selectedBlock === "function") ? selectedBlock() : AIR;
  return EDIBLE_FOODS.has(b) && (myInv.get(b) || 0) > 0;
}

/** 挑要吃的食物：優先「手持的食物」（順手），否則背包裡存量最多者（同量取 id 小，確定性）。無則 null。 */
function eatPickItem(inv) {
  if (!(inv instanceof Map)) return null;
  // 手持食物優先——玩家選了那格就是想吃它。
  const held = (typeof selectedBlock === "function") ? selectedBlock() : AIR;
  if (EDIBLE_FOODS.has(held) && (inv.get(held) || 0) > 0) {
    return { blockId: held, count: inv.get(held) };
  }
  let best = null;
  for (const [bid, cnt] of inv) {
    if (!EDIBLE_FOODS.has(bid) || cnt <= 0) continue;
    if (!best || cnt > best.count || (cnt === best.count && bid < best.blockId)) {
      best = { blockId: bid, count: cnt };
    }
  }
  return best;
}

/** 更新「🍽 吃」按鈕：背包有任何食物才浮現，顯示要吃的是哪樣。 */
function updateEatBtn() {
  const el = document.getElementById("eatBtn");
  if (!el) return;
  const pick = eatPickItem(myInv);
  if (!pick) {
    el.style.display = "none";
  } else {
    el.style.display = "inline-flex";
    el.textContent = "🍽 吃" + (BLOCK_NAME[pick.blockId] || "食物");
  }
}

let lastEatMs = 0; // 享用本地冷卻（防連按）

/** 執行吃：吃下一份食物（伺服器權威驗證是否食物＋存量＋是否已飽）。 */
function tryEatDish() {
  if (!wsReady) return;
  const now = Date.now();
  if (now - lastEatMs < 1200) return; // 1.2 秒本地冷卻
  const pick = eatPickItem(myInv);
  if (!pick) { showMsg("背包裡沒有能吃的東西～去採點作物或煮頓飯吧"); return; }
  lastEatMs = now;
  ws.send(JSON.stringify({ t: "eat", item_id: pick.blockId }));
}

// ── 玩家生存指標 HUD（玩家生存指標 v1·溫和版）──────────────────────────────────
// 快捷欄正上方一條窄列：左半血、右半飢。純顯示——所有數值來自後端 player_stats（後端權威）。
let _lastStarvingMsgMs = 0; // 餓瘋提示的本地節流（別洗版）

/** 依 player_stats 訊息更新血/飢窄列（寬度百分比＋餓瘋樣式＋餓瘋一次性提示）。 */
function updateStatsHud(m) {
  const hp = document.getElementById("statHealthFill");
  const food = document.getElementById("statHungerFill");
  const hungerBox = document.getElementById("statHunger");
  if (hp && typeof m.health === "number" && typeof m.max_health === "number" && m.max_health > 0) {
    hp.style.width = Math.max(0, Math.min(100, (m.health / m.max_health) * 100)) + "%";
  }
  if (food && typeof m.hunger === "number" && typeof m.max_hunger === "number" && m.max_hunger > 0) {
    food.style.width = Math.max(0, Math.min(100, (m.hunger / m.max_hunger) * 100)) + "%";
  }
  if (hungerBox) hungerBox.classList.toggle("starving", !!m.starving);
  // 餓瘋（飢餓見底）：移動變慢，給一句溫和提示（本地節流 12 秒，別煩人）。
  if (m.starving) {
    const now = Date.now();
    if (now - _lastStarvingMsgMs > 12000) {
      _lastStarvingMsgMs = now;
      showMsg("肚子餓得走不動了……找點吃的吧🍽");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2600);
    }
  }
}

/** 受傷輕紅暈：淡入極輕微紅色再淡出（別嚇人）。 */
function flashDamage() {
  const el = document.getElementById("damageFlash");
  if (!el) return;
  el.classList.add("show");
  // 淡入後移除 .show → 走 CSS 的 0.5s 淡出。
  setTimeout(() => { el.classList.remove("show"); }, 120);
}

/** 溫柔重生：把相機/預測位置拉回重生點（後端已搬權威位置），顯示溫暖提示。背包不掉落。 */
function doGentleRespawn(x, y, z, message) {
  if (typeof x === "number" && typeof y === "number" && typeof z === "number") {
    player.x = x; player.y = y; player.z = z;
    player.vy = 0; player.grounded = false;
    // 重生點 chunk 可能還沒到、地表把人埋住，脫困一次頂出來（沿用出生瞬間慣例）。
    if (typeof unstuckIfNeeded === "function") { try { unstuckIfNeeded(); } catch (e) {} }
  }
  showMsg("🌙 " + (message || "你在溫暖的爐火邊醒來……"));
  setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 4000);
}

// ── 乙太煙火 v1（ROADMAP 785）─────────────────────────────────────────────────
/** 更新「🎆 施放煙火」按鈕：背包裡有乙太煙火才浮現，顯示剩餘束數。 */
function updateFireworkBtn() {
  const el = document.getElementById("fireworkBtn");
  if (!el) return;
  const cnt = (myInv instanceof Map ? myInv.get(FIREWORK) : 0) || 0;
  if (cnt <= 0) {
    el.style.display = "none";
  } else {
    el.style.display = "inline-flex";
    el.textContent = "🎆 施放煙火 ×" + cnt;
  }
}

let lastFireworkMs = 0; // 施放本地冷卻（防連按；伺服器另有權威冷卻）

/** 執行施放：朝夜空放一束煙火（伺服器權威驗證存量＋每連線冷卻）。 */
function tryLaunchFirework() {
  if (!wsReady) return;
  const now = Date.now();
  if (now - lastFireworkMs < 800) return; // 0.8 秒本地防連按（伺服器冷卻更長）
  const cnt = (myInv instanceof Map ? myInv.get(FIREWORK) : 0) || 0;
  if (cnt <= 0) { showMsg("背包裡沒有乙太煙火——在工作台用乙太礦＋煤礦＋沙做一束吧。"); return; }
  lastFireworkMs = now;
  ws.send(JSON.stringify({ t: "firework_launch" }));
}

let lastTradeMs = 0;     // 交易請求本地冷卻（防連按）
let pendingTradeRid = null; // 目前有開放提案的居民 id

/** 請求與當前居民交易（發 TradeRequest，等 trade_offer 回應）。 */
function tryRequestTrade() {
  if (!wsReady || !chatRid) return;
  const now = Date.now();
  if (now - lastTradeMs < 2000) return; // 2 秒冷卻
  lastTradeMs = now;
  hideTradeOffer(); // 清掉舊提案
  ws.send(JSON.stringify({ t: "trade_request", resident_id: chatRid }));
}

/** 接受指定居民的交易提案（發 TradeAccept）。payWithCoin=true 時改直接付乙太幣代替湊材料（ROADMAP 874）。 */
function sendTradeAccept(rid, payWithCoin) {
  if (!wsReady) return;
  ws.send(JSON.stringify({ t: "trade_accept", resident_id: rid, pay_with_coin: !!payWithCoin }));
  hideTradeOffer();
}

/** 顯示交易提案橫幅（trade_offer 到來時呼叫）。 */
function showTradeOffer(m) {
  const el = document.getElementById("tradeOffer");
  const textEl = document.getElementById("tradeOfferText");
  const coinBtn = document.getElementById("tradeAcceptCoinBtn");
  if (!el || !textEl) return;
  pendingTradeRid = m.resident_id;
  const offerLine = m.offer_count > 1
    ? `${m.offer_name}×${m.offer_count}`
    : m.offer_name;
  const wantLine = m.want_count > 1
    ? `${m.want_name}×${m.want_count}`
    : m.want_name;
  textEl.textContent = `${m.resident_name || "居民"} 提議：給你 ${offerLine}，換你的 ${wantLine}`;
  // 付幣代替湊材料 v1（ROADMAP 874）：不想湊 want_item 也可以直接付 coin_price 枚乙太幣，
  // 省得為了一單交易特地跑一趟採礦；伺服器仍會權威驗背包夠不夠，不夠會回 trade_fail。
  if (coinBtn && m.coin_price > 0) {
    coinBtn.textContent = `🪙 付${m.coin_price}枚乙太幣換`;
    coinBtn.style.display = "inline-block";
  } else if (coinBtn) {
    coinBtn.style.display = "none";
  }
  el.style.display = "flex";
}

/** 隱藏交易提案橫幅。 */
function hideTradeOffer() {
  const el = document.getElementById("tradeOffer");
  if (el) el.style.display = "none";
  pendingTradeRid = null;
}

// ── 居民日記面板（ROADMAP 650）────────────────────────────────────────────────
// 玩家可在聊天框點「📖 日記」看居民的記憶足跡與當前心願——把 AI 的內在生活變成可讀的故事。

const diaryEl = document.getElementById("diary");
const diaryTitleEl = document.getElementById("diaryTitle");
const diaryBodyEl = document.getElementById("diaryBody");

/** 開啟指定居民的日記面板（fetch /voxel/diary → 過濾 resident_id → 渲染）。 */
async function openDiary(rid) {
  if (!diaryEl || !diaryBodyEl) return;
  // 取居民顯示名（從 residents Map 讀）。
  const ent = residents.get(rid);
  const name = (ent && ent.lastName) || rid;
  if (diaryTitleEl) diaryTitleEl.textContent = name + " 的日記";
  diaryBodyEl.innerHTML = '<div class="diary-empty">載入中…</div>';
  diaryEl.style.display = "flex";
  try {
    // 居民察覺你翻過她的日記 v1：帶上 player+resident，讓伺服器記下「這位玩家翻過我的日記」
    // 的待發現旗標（只有點開單一居民日記面板才算——日記牆一覽全體不夾帶 resident，不觸發）。
    let url = "/voxel/diary";
    if (myName && myName !== "旅人") {
      url += `?player=${encodeURIComponent(myName)}&resident=${encodeURIComponent(rid)}`;
    }
    const resp = await fetch(url);
    if (!resp.ok) throw new Error("diary fetch failed: " + resp.status);
    const pages = await resp.json();
    const page = Array.isArray(pages) ? pages.find(p => p.resident_id === rid) : null;
    renderDiaryPage(page, name);
  } catch (e) {
    diaryBodyEl.innerHTML = '<div class="diary-empty">無法讀取日記。</div>';
  }
}

/** 把 DiaryPage 資料渲染進日記面板。 */
function renderDiaryPage(page, name) {
  if (!diaryBodyEl) return;
  diaryBodyEl.innerHTML = "";

  // 自我印象區塊（自我印象 v1，ROADMAP 770）：居民從累積記憶昇華出「我是個怎樣的人」的
  // 高階自我概念，放在日記頁最頂端——翻開日記第一眼就讀到「這位居民如何看待自己」。
  // 只在後端昇華出明顯主導領域時才有（純模板、無玩家原話，隱私已於後端把關）。
  if (page && page.self_image) {
    const selfSection = document.createElement("div");
    selfSection.className = "diary-self-image";
    selfSection.innerHTML =
      '<div class="diary-self-image-label">🪞 自我印象</div>' +
      '<div class="diary-self-image-text">' + escHtml(page.self_image) + '</div>';
    diaryBodyEl.appendChild(selfSection);
  }

  // 心願區塊。
  const desireSection = document.createElement("div");
  if (page && page.desire) {
    desireSection.innerHTML =
      '<div class="diary-desire-label">💭 當前心願</div>' +
      '<div class="diary-desire">「' + escHtml(page.desire) + '」</div>';
  } else {
    desireSection.innerHTML =
      '<div class="diary-no-desire">還沒有心願……等待旅人的話語種下第一顆夢想。</div>';
  }
  diaryBodyEl.appendChild(desireSection);

  // 記憶條目。
  const entries = (page && Array.isArray(page.entries)) ? page.entries : [];
  const memSection = document.createElement("div");
  const secTitle = document.createElement("div");
  secTitle.className = "diary-section-title";
  // 日記＝瞥見居民沒說出口的內心（第一人稱反思，不是聊天謄本）。
  secTitle.textContent = entries.length ? "📝 內心的迴響（最新在前）" : "📝 內心的迴響";
  memSection.appendChild(secTitle);
  if (entries.length === 0) {
    const empty = document.createElement("div");
    empty.className = "diary-empty";
    empty.textContent = name + " 的心湖還很平靜……來跟她說說話，留下些漣漪吧。";
    memSection.appendChild(empty);
  } else {
    for (const e of entries) {
      const row = document.createElement("div");
      row.className = "diary-entry";
      // 只渲染反思文字（無玩家名 / 無原話）——隱私邊界已在後端把關。
      row.innerHTML = '<span class="diary-entry-text">' + escHtml(e.text) + '</span>';
      memSection.appendChild(row);
    }
  }
  diaryBodyEl.appendChild(memSection);

  // 模糊印象區塊（記憶 v2 最小可行版）：更早以前被記憶 cap 淘汰的舊記憶，
  // 沒有直接消失、留下一句去識別化的殘影——只在真的有淡忘過才顯示。
  if (page && page.faint_impression) {
    const impSection = document.createElement("div");
    impSection.className = "diary-impression";
    impSection.textContent = page.faint_impression;
    diaryBodyEl.appendChild(impSection);
  }
}

/** 純函式：轉義 HTML 特殊字元，避免記憶摘要插入 XSS（防護邊界）。 */
function escHtml(str) {
  if (typeof str !== "string") return "";
  return str.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
            .replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

function closeDiary() { if (diaryEl) diaryEl.style.display = "none"; }

if (diaryEl) {
  const closeBtn = document.getElementById("diaryClose");
  if (closeBtn) closeBtn.addEventListener("click", closeDiary);
  // 點面板外（背景）也關閉。
  diaryEl.addEventListener("click", (e) => { if (e.target === diaryEl) closeDiary(); });
}

// 對外暴露日記相關方法給 QA 使用。
export const __diaryTest = {
  escHtml,
  renderDiaryPage: (page, name) => { renderDiaryPage(page, name); return diaryBodyEl && diaryBodyEl.innerHTML; },
};

// ── 居民日記牆（ROADMAP 674）────────────────────────────────────────────────────
// 玩家從任何地方都可翻閱所有居民的日記——不需走近、不需開聊天——一覽 AI 居民的內心世界。
const diaryWallEl = document.getElementById("diaryWall");
const diaryWallBodyEl = document.getElementById("diaryWallBody");
const diaryWallBtnEl = document.getElementById("diaryWallBtn");

/**
 * 把所有居民的 DiaryPage 渲染成日記牆卡片列表（純函式，操作 DOM）。
 * 每張卡片顯示居民名、當前心願摘要、內心反思條數，並附「📖 詳細日記」按鈕。
 * @param {Array<{resident_id:string,resident_name:string,desire?:string,entries:Array}>} pages
 */
export function renderDiaryWall(pages) {
  if (!diaryWallBodyEl) return;
  if (!pages || pages.length === 0) {
    diaryWallBodyEl.innerHTML = '<div class="dw-empty">還沒有居民日記。</div>';
    return;
  }
  diaryWallBodyEl.innerHTML = "";
  for (const page of pages) {
    const card = document.createElement("div");
    card.className = "dw-card";
    const name = escHtml(page.resident_name || page.resident_id);
    const desireHtml = page.desire
      ? '<div class="dw-desire-label">💭 當前心願</div>' +
        '<div class="dw-desire">「' + escHtml(page.desire) + '」</div>'
      : '<div class="dw-meta">還沒有心願……等旅人的話語種下第一顆夢想。</div>';
    const entryCount = Array.isArray(page.entries) ? page.entries.length : 0;
    const metaHtml = entryCount > 0
      ? '<div class="dw-meta">共 ' + entryCount + ' 段內心反思</div>'
      : '<div class="dw-meta">心湖還很平靜……</div>';
    card.innerHTML =
      '<div class="dw-name">' + name + '</div>' +
      desireHtml +
      metaHtml;
    // 「詳細日記」按鈕：點擊後關日記牆、開個別居民完整日記。
    const openBtn = document.createElement("button");
    openBtn.className = "dw-open";
    openBtn.textContent = "📖 詳細日記";
    const rid = page.resident_id;
    openBtn.addEventListener("click", () => { closeDiaryWall(); openDiary(rid); });
    card.appendChild(openBtn);
    diaryWallBodyEl.appendChild(card);
  }
}

let diaryWallVisible = false;

/** 開啟日記牆面板，從 /voxel/diary 抓取所有居民日記並渲染卡片列表。 */
async function openDiaryWall() {
  if (!diaryWallEl) return;
  diaryWallVisible = true;
  diaryWallEl.style.display = "flex";
  if (diaryWallBodyEl) diaryWallBodyEl.innerHTML = '<div class="dw-empty">載入中…</div>';
  try {
    const resp = await fetch("/voxel/diary");
    if (!resp.ok) throw new Error("diary fetch: " + resp.status);
    const pages = await resp.json();
    renderDiaryWall(Array.isArray(pages) ? pages : []);
  } catch (_e) {
    if (diaryWallBodyEl) diaryWallBodyEl.innerHTML = '<div class="dw-empty">無法讀取日記。</div>';
  }
}

/** 關閉日記牆面板。 */
function closeDiaryWall() {
  diaryWallVisible = false;
  if (diaryWallEl) diaryWallEl.style.display = "none";
}

// 綁定日記牆按鈕與關閉事件。
if (diaryWallBtnEl) diaryWallBtnEl.addEventListener("click", () => {
  diaryWallVisible ? closeDiaryWall() : openDiaryWall();
});
if (diaryWallEl) {
  const closeBtn = document.getElementById("diaryWallClose");
  if (closeBtn) closeBtn.addEventListener("click", closeDiaryWall);
  // 點面板外（背景遮罩）也關閉。
  diaryWallEl.addEventListener("click", (e) => { if (e.target === diaryWallEl) closeDiaryWall(); });
}

// ── 城鎮動態 Feed（ROADMAP 655）────────────────────────────────────────────────
const feedEl = document.getElementById("feed");
const feedBodyEl = document.getElementById("feedBody");
const feedBtnEl = document.getElementById("feedBtn");

/** 把 Unix 秒換算成「X 分鐘前」繁中字串。
 * @param {number} tsSecs 事件的 Unix 秒時戳。
 * @returns {string}
 */
export function formatRelativeTime(tsSecs) {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - tsSecs);
  if (diff < 60)     return "剛才";
  if (diff < 3600)   return `${Math.floor(diff / 60)} 分鐘前`;
  if (diff < 86400)  return `${Math.floor(diff / 3600)} 小時前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

/** 各事件類型對應的 emoji 提示。 */
const KIND_EMOJI = {
  "新心願":  "💭",
  "念頭種下": "✨",
  "鄰里閒聊": "💬",
  "蓋家動工": "🏗️",
  "蓋家完工": "🏠",
  "居民易物": "⇌",
  "重返":     "🪧",
  "漂流瓶":   "🍾",
};

/** 把後端回傳的 FeedEvent 陣列渲染到 #feedBody。
 * @param {Array<{ts_secs:number, kind:string, resident:string, detail:string}>} events
 */
export function renderFeed(events) {
  if (!feedBodyEl) return;
  if (!events || events.length === 0) {
    feedBodyEl.innerHTML = '<div class="feed-empty">還沒有動態……等居民開始活動後這裡就會熱鬧起來。</div>';
    return;
  }
  feedBodyEl.innerHTML = "";
  for (const ev of events) {
    const item = document.createElement("div");
    const kindSlug = (ev.kind || "").replace(/[^a-zA-Z一-鿿]/g, "");
    item.className = "feed-item kind-" + escHtml(ev.kind || "");
    const emoji = KIND_EMOJI[ev.kind] || "📌";
    item.innerHTML =
      '<div class="feed-who">' +
        '<span class="feed-who-kind">' + emoji + " " + escHtml(ev.kind || "") + "・" + escHtml(ev.resident || "") + '</span>' +
        '<span class="feed-who-time">' + formatRelativeTime(ev.ts_secs || 0) + '</span>' +
      '</div>' +
      '<div class="feed-detail">' + escHtml(ev.detail || "") + '</div>';
    feedBodyEl.appendChild(item);
  }
}

let feedVisible = false;
let feedRefreshTimer = null;

/** 開啟動態 Feed 面板並立刻抓取最新動態。 */
async function openFeed() {
  if (!feedEl) return;
  feedVisible = true;
  feedEl.style.display = "flex";
  await refreshFeed();
  // 每 30 秒自動刷新一次（面板開著時）。
  if (feedRefreshTimer) clearInterval(feedRefreshTimer);
  feedRefreshTimer = setInterval(() => { if (feedVisible) refreshFeed(); }, 30_000);
}

/** 關閉動態 Feed 面板。 */
function closeFeed() {
  feedVisible = false;
  if (feedEl) feedEl.style.display = "none";
  if (feedRefreshTimer) { clearInterval(feedRefreshTimer); feedRefreshTimer = null; }
}

/** 向後端抓最新 Feed 並重新渲染。 */
async function refreshFeed() {
  if (!feedBodyEl) return;
  try {
    const resp = await fetch("/voxel/feed");
    if (!resp.ok) throw new Error("feed fetch failed: " + resp.status);
    const events = await resp.json();
    renderFeed(events);
  } catch (err) {
    if (feedBodyEl) feedBodyEl.innerHTML = '<div class="feed-empty">無法讀取動態。</div>';
  }
}

// 綁定按鈕與關閉事件。
if (feedBtnEl) feedBtnEl.addEventListener("click", () => {
  feedVisible ? closeFeed() : openFeed();
});
if (feedEl) {
  const closeBtn = document.getElementById("feedClose");
  if (closeBtn) closeBtn.addEventListener("click", closeFeed);
}

// ── 居民羅盤（ROADMAP 705）──────────────────────────────────────────────────
// 居民散居世界四方（653：露娜在原點、諾娃/賽勒/奧瑞各在南/西/東 75 格）之後，
// 玩家只能憑印象亂走才找得到人。本面板讀伺服器早已廣播的居民即時座標（零新協議），
// 純前端算出「往哪個方向、還有多遠」，讓散居系統第一次真的方便被使用。
const compassEl = document.getElementById("compassPanel");
const compassBodyEl = document.getElementById("compassBody");
const compassBtnEl = document.getElementById("compassBtn");
const waypointBodyEl = document.getElementById("waypointBody");
const waypointAddBtnEl = document.getElementById("waypointAddBtn");

/** 世界座標下，從 (px,pz) 望向 (rx,rz) 的方位角（弧度）。
 * 與本引擎鏡頭朝向慣例同一套定義（`fwd(yaw) = (-sin(yaw), 0, -cos(yaw))`）：
 * 當 `worldBearing === yaw` 時，目標正好落在玩家正前方。
 * @returns {number} 方位角（弧度，未正規化）
 */
export function worldBearing(px, pz, rx, rz) {
  return Math.atan2(-(rx - px), -(rz - pz));
}

/** 目標相對玩家目前朝向（yaw）的螢幕旋轉角度（度，0~359）：
 * 0 = 正前方（箭頭朝上）、90 = 右方、180 = 正後方、270 = 左方。
 * 純函式、確定性，供羅盤箭頭 CSS `rotate()` 直接使用。
 * @returns {number} 0~359 的相對角度
 */
export function compassRelativeDeg(px, pz, rx, rz, yaw) {
  const rel = yaw - worldBearing(px, pz, rx, rz);
  const deg = (rel * 180 / Math.PI) % 360;
  return deg < 0 ? deg + 360 : deg;
}

let compassVisible = false;
let compassTimer = null;

// ── 個人路標（自主提案切片，ROADMAP 869）───────────────────────────────────
// 世界很大，玩家自己標記的地點（礦坑入口/看中的地基）走開幾步就再也找不回去；
// 本段讓玩家能在目前所站的位置插一支路標、取個短名字，跟居民座標並列在同一面板導航。
/** 這位玩家目前的路標快取：`[{label,x,y,z}]`，由 `/voxel/waypoints` 拉取 + `waypoint_sync` 即時更新。 */
let waypoints = [];

/** 向後端抓這位玩家目前的所有路標（開面板時先拉一份現況，之後靠 `waypoint_sync` 即時更新）。 */
async function refreshWaypoints() {
  if (!myName || myName === "旅人") { waypoints = []; return; }
  try {
    const resp = await fetch(`/voxel/waypoints?player=${encodeURIComponent(myName)}`);
    if (!resp.ok) throw new Error("waypoints fetch failed: " + resp.status);
    waypoints = await resp.json();
  } catch (err) {
    // 拉取失敗保留舊快取，不阻斷遊戲——下次開面板/重連再試。
  }
}

/** 彈窗問名字，在玩家目前所站位置插一支路標（座標由伺服器權威決定，不信任前端自報）。 */
function promptAddWaypoint() {
  const label = window.prompt("幫這個路標取個短名字（最多 12 字）：", "");
  if (label === null) return;
  const trimmed = label.trim();
  if (!trimmed) return;
  ws.send(JSON.stringify({ t: "set_waypoint", label: trimmed }));
}

/** 刪除指定名字的路標（伺服器確認後才會從 `waypoints` 快取移除，見 `waypoint_sync`）。 */
function removeWaypoint(label) {
  ws.send(JSON.stringify({ t: "remove_waypoint", label }));
}

// ── 雷達小地圖（ROADMAP 820）───────────────────────────────────────────────
// 705 羅盤只有文字列表（方向＋距離），玩家仍得自己在腦中拼出「大家散得多開」的
// 全貌。本刀在同一面板加一張圓形雷達畫布：玩家永遠朝上（螢幕正上方），居民依
// 相對方位與距離畫成色點——同一組數學（compassRelativeDeg），只是換成視覺呈現，
// 一眼看出「大家在哪一側、密集或分散」，與文字列表互補共存。
const compassRadarEl = document.getElementById("compassRadar");
const compassRadarCtx = compassRadarEl ? compassRadarEl.getContext("2d") : null;
/** 雷達涵蓋半徑（遊戲單位）；超出此距離的居民仍顯示正確方向，但夾在圓周邊緣（半透明標示「在範圍外」）。 */
const RADAR_RANGE_UNITS = 150;

/** 把「相對玩家朝向的角度(deg)＋距離(遊戲單位)」換算成雷達畫布上的相對座標（純函式，可測）。
 * 與 compassRelativeDeg 同一套慣例：0 度＝正前方（畫布正上方），順時針遞增。
 * @returns {{x:number,y:number,clamped:boolean}} 相對雷達圓心的偏移（px）；clamped=true 表示已超出範圍、夾在邊緣。
 */
export function radarPoint(dist, relDeg, rangeUnits, radiusPx) {
  const rad = relDeg * Math.PI / 180;
  const clamped = dist > rangeUnits;
  const r = clamped ? radiusPx : (dist / rangeUnits) * radiusPx;
  return { x: r * Math.sin(rad), y: -r * Math.cos(rad), clamped };
}

/** 重繪雷達畫布：背景圓＋兩圈距離刻度＋每位居民一個色點（顏色沿用聊天窗同款穩定色）＋中心固定朝上的玩家三角。 */
function renderCompassRadar() {
  if (!compassRadarCtx || !compassRadarEl) return;
  const ctx = compassRadarCtx;
  const w = compassRadarEl.width, h = compassRadarEl.height;
  const cx = w / 2, cy = h / 2;
  const outerR = Math.min(w, h) / 2 - 6;
  ctx.clearRect(0, 0, w, h);
  ctx.beginPath();
  ctx.arc(cx, cy, outerR, 0, Math.PI * 2);
  ctx.fillStyle = "rgba(10,14,22,0.85)";
  ctx.fill();
  ctx.strokeStyle = "rgba(200,184,255,0.25)";
  ctx.lineWidth = 1;
  ctx.stroke();
  for (const frac of [1 / 3, 2 / 3]) {
    ctx.beginPath();
    ctx.arc(cx, cy, outerR * frac, 0, Math.PI * 2);
    ctx.strokeStyle = "rgba(200,184,255,0.12)";
    ctx.stroke();
  }
  const dotR = outerR - 8;
  for (const [rid, ent] of residents.entries()) {
    const p = ent.group.position;
    const dx = p.x - player.x, dz = p.z - player.z;
    const dist = Math.hypot(dx, dz);
    const relDeg = compassRelativeDeg(player.x, player.z, p.x, p.z, player.yaw);
    const pt = radarPoint(dist, relDeg, RADAR_RANGE_UNITS, dotR);
    ctx.globalAlpha = pt.clamped ? 0.55 : 1;
    ctx.beginPath();
    ctx.arc(cx + pt.x, cy + pt.y, pt.clamped ? 4 : 5, 0, Math.PI * 2);
    ctx.fillStyle = chatLogColorFor(rid);
    ctx.fill();
    ctx.globalAlpha = 1;
  }
  // 個人路標（自主提案切片，ROADMAP 869）：金色小菱形，與居民色點一眼區隔。
  for (const w of waypoints) {
    const dx = w.x - player.x, dz = w.z - player.z;
    const dist = Math.hypot(dx, dz);
    const relDeg = compassRelativeDeg(player.x, player.z, w.x, w.z, player.yaw);
    const pt = radarPoint(dist, relDeg, RADAR_RANGE_UNITS, dotR);
    ctx.globalAlpha = pt.clamped ? 0.55 : 1;
    ctx.save();
    ctx.translate(cx + pt.x, cy + pt.y);
    ctx.rotate(Math.PI / 4);
    ctx.fillStyle = "#e8c95a";
    ctx.fillRect(-3.5, -3.5, 7, 7);
    ctx.restore();
    ctx.globalAlpha = 1;
  }
  ctx.beginPath();
  ctx.moveTo(cx, cy - 7);
  ctx.lineTo(cx - 5, cy + 5);
  ctx.lineTo(cx + 5, cy + 5);
  ctx.closePath();
  ctx.fillStyle = "#eaf2ff";
  ctx.fill();
}

/** 重新計算並渲染羅盤列表：依所有居民的即時座標算方位＋距離，離玩家近的排前面。 */
function renderCompassPanel() {
  renderCompassRadar();
  if (!compassBodyEl) return;
  const rows = [...residents.values()].map((ent) => {
    const p = ent.group.position;
    const dx = p.x - player.x, dz = p.z - player.z;
    return {
      name: ent.lastName,
      dist: Math.hypot(dx, dz),
      deg: compassRelativeDeg(player.x, player.z, p.x, p.z, player.yaw),
    };
  }).sort((a, b) => a.dist - b.dist);
  if (rows.length === 0) {
    compassBodyEl.innerHTML = '<div class="compass-empty">目前沒有居民座標可指引。</div>';
    return;
  }
  compassBodyEl.innerHTML = "";
  for (const row of rows) {
    const div = document.createElement("div");
    div.className = "compass-row";
    div.innerHTML =
      '<span class="compass-arrow" style="transform: rotate(' + row.deg.toFixed(0) + 'deg)">↑</span>' +
      '<span class="compass-name">' + escHtml(row.name) + '</span>' +
      '<span class="compass-dist">' + Math.round(row.dist) + ' 格</span>';
    compassBodyEl.appendChild(div);
  }
  renderWaypointList();
}

/** 渲染「我的路標」清單：跟居民列表同款方位箭頭＋距離，多一顆刪除鈕。 */
function renderWaypointList() {
  if (!waypointBodyEl) return;
  if (waypoints.length === 0) {
    waypointBodyEl.innerHTML = '<div class="compass-empty">還沒有路標，按上方「+ 插旗」在此處插一支。</div>';
    return;
  }
  waypointBodyEl.innerHTML = "";
  for (const w of waypoints) {
    const dx = w.x - player.x, dz = w.z - player.z;
    const dist = Math.hypot(dx, dz);
    const deg = compassRelativeDeg(player.x, player.z, w.x, w.z, player.yaw);
    const div = document.createElement("div");
    div.className = "compass-row waypoint-row";
    div.innerHTML =
      '<span class="compass-arrow" style="transform: rotate(' + deg.toFixed(0) + 'deg)">🚩</span>' +
      '<span class="compass-name">' + escHtml(w.label) + '</span>' +
      '<span class="compass-dist">' + Math.round(dist) + ' 格</span>' +
      '<span class="waypoint-del" data-label="' + escHtml(w.label) + '">✕</span>';
    waypointBodyEl.appendChild(div);
  }
  for (const btn of waypointBodyEl.querySelectorAll(".waypoint-del")) {
    btn.addEventListener("click", () => removeWaypoint(btn.dataset.label));
  }
}

/** 開啟居民羅盤面板，開始每 0.3 秒刷新一次方位（面板關閉時停止，不空耗）。 */
function openCompass() {
  if (!compassEl) return;
  compassVisible = true;
  compassEl.style.display = "flex";
  renderCompassPanel();
  refreshWaypoints().then(() => { if (compassVisible) renderCompassPanel(); });
  if (compassTimer) clearInterval(compassTimer);
  compassTimer = setInterval(() => { if (compassVisible) renderCompassPanel(); }, 300);
}

/** 關閉居民羅盤面板。 */
function closeCompass() {
  compassVisible = false;
  if (compassEl) compassEl.style.display = "none";
  if (compassTimer) { clearInterval(compassTimer); compassTimer = null; }
}

if (compassBtnEl) compassBtnEl.addEventListener("click", () => {
  compassVisible ? closeCompass() : openCompass();
});
if (compassEl) {
  const closeBtn = document.getElementById("compassClose");
  if (closeBtn) closeBtn.addEventListener("click", closeCompass);
}
if (waypointAddBtnEl) waypointAddBtnEl.addEventListener("click", promptAddWaypoint);

// ── 居民交情網（ROADMAP 708）────────────────────────────────────────────────
// 居民彼此拜訪（671）很久前就悄悄累積情誼（672：陌生→相識→老朋友），驅動問候語
// /八卦轉述（694）/互助蓋家（696），但這份資料只活在伺服器內部，玩家完全看不見
// 「這座小社會到底誰跟誰要好」。本面板讀新後端唯讀端點 `/voxel/relations`，把這
// 份隱形的社交網絡攤開給玩家看——跟羅盤（705）異曲同工：讓早已存在的系統第一次
// 被看見，而不是新造一套關係系統。
const relationsEl = document.getElementById("relationsPanel");
const relationsBodyEl = document.getElementById("relationsBody");
const relationsBtnEl = document.getElementById("relationsBtn");
const cliquesBodyEl = document.getElementById("cliquesBody");

const RELATION_TIER_ICON = { friend: "🤝", acquaintance: "🙂", stranger: "·" };
const RELATION_TIER_LABEL = { friend: "老朋友", acquaintance: "相識", stranger: "陌生" };
const RELATION_TIER_RANK = { friend: 2, acquaintance: 1, stranger: 0 };

/** 依情誼層級排序（老朋友優先），同層級依拜訪次數多到少排列。純函式、確定性、可測。
 * @param {Array<{a:string,b:string,tier:string,visits:number}>} rows
 * @returns {Array} 排序後的新陣列（不改動原陣列）
 */
export function sortRelationRows(rows) {
  return [...rows].sort((x, y) => {
    const r = (RELATION_TIER_RANK[y.tier] ?? 0) - (RELATION_TIER_RANK[x.tier] ?? 0);
    return r !== 0 ? r : (y.visits ?? 0) - (x.visits ?? 0);
  });
}

let relationsVisible = false;
let relationsRefreshTimer = null;

/** 重新渲染交情網列表。 */
function renderRelationsPanel(rows) {
  if (!relationsBodyEl) return;
  if (!rows || rows.length === 0) {
    relationsBodyEl.innerHTML = '<div class="relations-empty">目前沒有交情資料。</div>';
    return;
  }
  const sorted = sortRelationRows(rows);
  relationsBodyEl.innerHTML = "";
  for (const row of sorted) {
    const div = document.createElement("div");
    div.className = "relations-row tier-" + (row.tier || "stranger");
    // 居民戀愛心動 v1（ROADMAP 846）：戀人對在情誼層級之外多標一顆 ❤️，一眼認出誰跟誰在一起。
    const icon = row.sweetheart ? "❤️" : (RELATION_TIER_ICON[row.tier] || "·");
    const label = row.sweetheart ? "戀人" : (RELATION_TIER_LABEL[row.tier] || "陌生");
    div.innerHTML =
      '<span class="relations-icon">' + icon + '</span>' +
      '<span class="relations-names">' + escHtml(row.a) + ' ↔ ' + escHtml(row.b) + '</span>' +
      '<span class="relations-tier">' + label + '</span>';
    relationsBodyEl.appendChild(div);
  }
}

/**
 * 小圈子攤開（自主提案切片，接續 708 交情網 + 711 小圈子聚會）：711 早已判定「這幾位
 * 彼此皆為老朋友」來驅動小聚會，卻只在伺服器內部使用；本函式把同一份資料渲染成
 * 交情網面板頂端的一段小標籤，讓玩家第一次看見「這座小社會裡誰跟誰其實是一夥的」。
 * 沒有圈子時整段留白（CSS `:empty` 隱藏），不佔面板空間、不干擾原本的兩兩列表。
 * @param {Array<Array<string>>} cliques 每個元素是一組彼此皆為老朋友的居民名字。
 */
function renderCliquesSection(cliques) {
  if (!cliquesBodyEl) return;
  cliquesBodyEl.innerHTML = "";
  if (!cliques || cliques.length === 0) return;
  for (const members of cliques) {
    const div = document.createElement("div");
    div.className = "clique-row";
    div.innerHTML =
      '<span class="clique-icon">🤝</span>' +
      '<span class="clique-names">' + members.map(escHtml).join("・") + '</span>';
    cliquesBodyEl.appendChild(div);
  }
}

/** 向後端抓最新交情資料（兩兩情誼 + 小圈子）並重新渲染。 */
async function refreshRelations() {
  if (!relationsBodyEl) return;
  try {
    const resp = await fetch("/voxel/relations");
    if (!resp.ok) throw new Error("relations fetch failed: " + resp.status);
    const rows = await resp.json();
    renderRelationsPanel(rows);
  } catch (err) {
    relationsBodyEl.innerHTML = '<div class="relations-empty">無法讀取交情資料。</div>';
  }
  try {
    const cResp = await fetch("/voxel/cliques");
    if (cResp.ok) renderCliquesSection(await cResp.json());
  } catch (err) {
    // 小圈子是錦上添花的附加資訊，讀取失敗不影響主要的兩兩交情列表。
  }
}

/** 開啟居民交情網面板（情誼靠拜訪慢慢累積、變化很慢，30 秒刷新一次足夠，
 * 面板關閉時停止刷新，不背景空耗）。 */
function openRelations() {
  if (!relationsEl) return;
  relationsVisible = true;
  relationsEl.style.display = "flex";
  refreshRelations();
  if (relationsRefreshTimer) clearInterval(relationsRefreshTimer);
  relationsRefreshTimer = setInterval(() => { if (relationsVisible) refreshRelations(); }, 30_000);
}

/** 關閉居民交情網面板。 */
function closeRelations() {
  relationsVisible = false;
  if (relationsEl) relationsEl.style.display = "none";
  if (relationsRefreshTimer) { clearInterval(relationsRefreshTimer); relationsRefreshTimer = null; }
}

if (relationsBtnEl) relationsBtnEl.addEventListener("click", () => {
  relationsVisible ? closeRelations() : openRelations();
});
if (relationsEl) {
  const closeBtn = document.getElementById("relationsClose");
  if (closeBtn) closeBtn.addEventListener("click", closeRelations);
}

// ── 居民技能簿（ROADMAP 719）────────────────────────────────────────────────
// 技能發明（716）+ 傳授（717）至今只透過稍縱即逝的 Feed 文字曝光（「露娜教了我
// 『燒玻璃』這招！」播報一過就沒了），玩家從沒有任何管道能回頭查「這座小社會
// 現在誰會什麼」——這份資料只活在伺服器 `InventedSkillStore` 裡。跟 708 交情網
// 同一手法：讓早已存在的系統第一次被看見，而不是新造一套技能系統。
const skillsEl = document.getElementById("skillsPanel");
const skillsBodyEl = document.getElementById("skillsBody");
const skillsBtnEl = document.getElementById("skillsBtn");

/** 重新渲染技能簿列表。
 * 師承鏈可見（技能互教·北極星第四刀）：後端多回了 `lineage`（每筆技能的來歷——
 * 自己發明／承自XX（親子）／師承XX（教學）），非「自己發明」的在晶片上以淡字並列，
 * 村裡的知識系譜一眼看得出這手藝是誰傳給誰的。缺 `lineage`（舊後端）時照舊只顯名字。
 * @param {Array<{name:string, skills:string[], lineage?:Array<{name:string,origin:string}>}>} rows
 */
function renderSkillsPanel(rows) {
  if (!skillsBodyEl) return;
  if (!rows || rows.length === 0) {
    skillsBodyEl.innerHTML = '<div class="skills-empty">目前沒有技能資料。</div>';
    return;
  }
  skillsBodyEl.innerHTML = "";
  for (const row of rows) {
    const div = document.createElement("div");
    div.className = "skills-row";
    const skills = row.skills || [];
    const lineage = Array.isArray(row.lineage) ? row.lineage : null;
    const chips = skills.length > 0
      ? '<div class="skills-chips">' +
        skills.map((s, k) => {
          const origin = lineage && lineage[k] && lineage[k].origin && lineage[k].origin !== "自己發明"
            ? '<span class="skills-origin">·' + escHtml(lineage[k].origin) + '</span>'
            : "";
          return '<span class="skills-chip">' + escHtml(s) + origin + '</span>';
        }).join("") +
        '</div>'
      : '<div class="skills-none">尚未發明任何技能</div>';
    div.innerHTML = '<span class="skills-name">' + escHtml(row.name) + '</span>' + chips;
    skillsBodyEl.appendChild(div);
  }
}

/** 向後端抓最新技能資料並重新渲染。 */
async function refreshSkills() {
  if (!skillsBodyEl) return;
  try {
    const resp = await fetch("/voxel/skills");
    if (!resp.ok) throw new Error("skills fetch failed: " + resp.status);
    const rows = await resp.json();
    renderSkillsPanel(rows);
  } catch (err) {
    skillsBodyEl.innerHTML = '<div class="skills-empty">無法讀取技能資料。</div>';
  }
}

let skillsVisible = false;
let skillsRefreshTimer = null;

/** 開啟居民技能簿面板（技能發明頻率低，30 秒刷新一次足夠，面板關閉時停止刷新）。 */
function openSkills() {
  if (!skillsEl) return;
  skillsVisible = true;
  skillsEl.style.display = "flex";
  refreshSkills();
  if (skillsRefreshTimer) clearInterval(skillsRefreshTimer);
  skillsRefreshTimer = setInterval(() => { if (skillsVisible) refreshSkills(); }, 30_000);
}

/** 關閉居民技能簿面板。 */
function closeSkills() {
  skillsVisible = false;
  if (skillsEl) skillsEl.style.display = "none";
  if (skillsRefreshTimer) { clearInterval(skillsRefreshTimer); skillsRefreshTimer = null; }
}

if (skillsBtnEl) skillsBtnEl.addEventListener("click", () => {
  skillsVisible ? closeSkills() : openSkills();
});
if (skillsEl) {
  const closeBtn = document.getElementById("skillsClose");
  if (closeBtn) closeBtn.addEventListener("click", closeSkills);
}

// ── 玩家里程碑（ROADMAP 724）──────────────────────────────────────────────────
// 居民有技能簿（719）、交情網（708）可回頭翻閱自己的成長，玩家的療癒循環
// （採集→合成→蓋造→種田→贈禮→交易→熟識→安眠）至今卻沒有任何一處能回頭看看
// 「我走了多遠」。本面板純讀取既有 `/voxel/milestones` 資料，達成瞬間另由
// `milestone_unlocked` WS 訊息觸發慶祝提示（見下方 handler）。
const milesEl = document.getElementById("milestonesPanel");
const milesBodyEl = document.getElementById("milestonesBody");
const milesBtnEl = document.getElementById("milestonesBtn");

/** 重新渲染里程碑清單。
 * @param {Array<{id:string,name_zh:string,desc_zh:string,icon:string,earned:boolean}>} rows
 */
function renderMilestonesPanel(rows) {
  if (!milesBodyEl) return;
  if (!rows || rows.length === 0) {
    milesBodyEl.innerHTML = '<div class="skills-empty">目前沒有里程碑資料。</div>';
    return;
  }
  const earnedCount = rows.filter((r) => r.earned).length;
  let html = '<div class="miles-progress">已達成 ' + earnedCount + ' / ' + rows.length + '</div>';
  for (const row of rows) {
    html += '<div class="miles-row' + (row.earned ? ' miles-earned' : ' miles-locked') + '">' +
      '<span class="miles-icon">' + escHtml(row.icon || "🏅") + '</span>' +
      '<span class="miles-text"><span class="miles-name">' + escHtml(row.name_zh) + '</span>' +
      '<span class="miles-desc">' + escHtml(row.desc_zh) + '</span></span>' +
      '</div>';
  }
  milesBodyEl.innerHTML = html;
}

/** 向後端抓這位玩家最新的里程碑達成狀態並重新渲染。 */
async function refreshMilestones() {
  if (!milesBodyEl) return;
  try {
    const resp = await fetch(`/voxel/milestones?player=${encodeURIComponent(myName)}`);
    if (!resp.ok) throw new Error("milestones fetch failed: " + resp.status);
    const rows = await resp.json();
    renderMilestonesPanel(rows);
  } catch (err) {
    milesBodyEl.innerHTML = '<div class="skills-empty">無法讀取里程碑資料。</div>';
  }
}

let milesVisible = false;

/** 開啟玩家里程碑面板（低頻資料，開啟時抓一次即可，不必背景輪詢）。 */
function openMilestones() {
  if (!milesEl) return;
  milesVisible = true;
  milesEl.style.display = "flex";
  refreshMilestones();
}

/** 關閉玩家里程碑面板。 */
function closeMilestones() {
  milesVisible = false;
  if (milesEl) milesEl.style.display = "none";
}

if (milesBtnEl) milesBtnEl.addEventListener("click", () => {
  milesVisible ? closeMilestones() : openMilestones();
});
if (milesEl) {
  const closeBtn = document.getElementById("milestonesClose");
  if (closeBtn) closeBtn.addEventListener("click", closeMilestones);
}

// ── 探索紀事（自主提案切片，接續 838 古代遺跡／839 溫泉遺跡）───────────────────
// 838/839 讓世界第一次有了值得走遠尋訪的地標，但找到之後除了那一拍的驚喜什麼都沒留下——
// 玩家找過幾處、位置在哪，全無管道回頭翻閱。跟里程碑互補：里程碑記「有沒有做過」，
// 這裡記「在哪裡做到的」（可能多筆、每筆帶座標）。本面板純讀取既有 `/voxel/discoveries` 資料。
const discEl = document.getElementById("discPanel");
const discBodyEl = document.getElementById("discBody");
const discBtnEl = document.getElementById("discBtn");

/** 重新渲染探索紀事清單。
 * @param {{items:Array<{kind:string,label:string,icon:string,x:number,y:number,z:number}>,ruins:number,springs:number,outposts:number}} data
 */
function renderDiscoveryPanel(data) {
  if (!discBodyEl) return;
  const items = (data && data.items) || [];
  if (items.length === 0) {
    discBodyEl.innerHTML = '<div class="skills-empty">還沒有探索紀事——走遠去找找古代遺跡、溫泉，或循著居民的足跡找到她的邊陲營地吧。</div>';
    return;
  }
  const ruins = (data && data.ruins) || 0;
  const springs = (data && data.springs) || 0;
  const outposts = (data && data.outposts) || 0;
  let html = '<div class="disc-progress">🏛️ 遺跡 ' + ruins + ' 處 · ♨️ 溫泉 ' + springs + ' 處 · ⛺ 邊陲營地 ' + outposts + ' 處</div>';
  for (const it of items) {
    html += '<div class="disc-row">' +
      '<span class="disc-icon">' + escHtml(it.icon || "📍") + '</span>' +
      '<span class="disc-text"><span class="disc-name">' + escHtml(it.label) + '</span>' +
      '<span class="disc-coord">(' + it.x + ', ' + it.z + ')</span></span>' +
      '</div>';
  }
  discBodyEl.innerHTML = html;
}

/** 向後端抓這位玩家最新的探索紀事並重新渲染。 */
async function refreshDiscoveries() {
  if (!discBodyEl) return;
  try {
    const resp = await fetch(`/voxel/discoveries?player=${encodeURIComponent(myName)}`);
    if (!resp.ok) throw new Error("discoveries fetch failed: " + resp.status);
    const data = await resp.json();
    renderDiscoveryPanel(data);
  } catch (err) {
    discBodyEl.innerHTML = '<div class="skills-empty">無法讀取探索紀事。</div>';
  }
}

let discVisible = false;

/** 開啟探索紀事面板（低頻資料，開啟時抓一次即可，不必背景輪詢）。 */
function openDiscoveries() {
  if (!discEl) return;
  discVisible = true;
  discEl.style.display = "flex";
  refreshDiscoveries();
}

/** 關閉探索紀事面板。 */
function closeDiscoveries() {
  discVisible = false;
  if (discEl) discEl.style.display = "none";
}

if (discBtnEl) discBtnEl.addEventListener("click", () => {
  discVisible ? closeDiscoveries() : openDiscoveries();
});
if (discEl) {
  const closeBtn = document.getElementById("discClose");
  if (closeBtn) closeBtn.addEventListener("click", closeDiscoveries);
}

// 伺服器對同一種 `landmark_notes` 訊息有兩個時機都會送：①第一次發現地標（該問要不要
// 留言）②剛寫完留言後的回顯（只該更新顯示、不該再問一次——否則會無限循環跳窗）。兩者
// wire 格式相同，靠這個旗標分辨「這則是不是我剛寫完留言換來的回音」。
let landmarkNoteJustSent = false;

// ── 地標旅人留言（自主提案切片，ROADMAP 862，接續 838/839/840）────────────────────
// 840 探索紀事是私人足跡；本段接住伺服器在「第一次發現這處地標」那一刻主動推來的既有
// 留言簿——先秀給玩家看先前旅人留了什麼話，再問要不要也留一句給後來的人。真正的驗證/
// 去重/內容審查都在伺服器 `LeaveLandmarkNote` 那一刀，這裡純粹是問與顯示。
function showLandmarkNotes(kind, x, y, z, notes) {
  const icon = kind === "ruin" ? "🏛️" : "♨️";
  if (landmarkNoteJustSent) {
    landmarkNoteJustSent = false;
    showMsg(icon + " 留言已收錄進地標留言簿了！");
    return;
  }
  if (notes && notes.length > 0) {
    const preview = notes.slice(0, 2).map((n) => "「" + n.text + "」——" + n.player).join(" ／ ");
    showMsg(icon + " 先前旅人留言：" + preview);
  }
  const text = window.prompt(icon + " 要留一句話給後來的旅人嗎？（最多 60 字，留空可跳過）", "");
  if (text !== null && text.trim() !== "") {
    landmarkNoteJustSent = true;
    ws.send(JSON.stringify({ t: "leave_landmark_note", x, y, z, text }));
  }
}

// ── 玩家熟練度（自主提案切片，ROADMAP 842）──────────────────────────────────────
// 玩家里程碑（724）補上了「回頭看看走了多遠」的一次性徽章牆，但徽章是二元的（做過一次
// 沒），一次解鎖後就靜止——玩家日復一日反覆採集/耕種/垂釣，除了徽章牆上早已勾滿的勾勾，
// 持續投入本身從沒有任何看得見的累積成長。本面板顯示⛏️採集／🌾耕種／🎣垂釣三條連續
// 熟練度的經驗條，練到 Lv.5 起解鎖產出加成（後端已在背後生效，前端只忠實呈現進度）。
const masteryEl = document.getElementById("masteryPanel");
const masteryBodyEl = document.getElementById("masteryBody");
const masteryBtnEl = document.getElementById("masteryBtn");

/** 重新渲染熟練度清單。
 * @param {Array<{kind:string,name_zh:string,icon:string,xp:number,level:number,title:string,next_level_xp:number,bonus_unlocked:boolean}>} rows
 */
function renderMasteryPanel(rows) {
  if (!masteryBodyEl) return;
  if (!rows || rows.length === 0) {
    masteryBodyEl.innerHTML = '<div class="skills-empty">還沒有熟練度紀錄——去採集/耕種/垂釣練練手吧。</div>';
    return;
  }
  let html = "";
  for (const r of rows) {
    // 每級固定 60 xp（比照後端 LEVEL_XP_STEP）：本級起點 = level*60，滿級（10）恆滿條。
    const levelFloorXp = r.level * 60;
    const pct = r.level >= 10 ? 100 : Math.max(0, Math.min(100, Math.round(((r.xp - levelFloorXp) / 60) * 100)));
    const bonusTag = r.bonus_unlocked ? " · 🎁加成中" : "";
    html += '<div class="mastery-row">' +
      '<div class="mastery-row-head">' +
      '<span class="mastery-name">' + escHtml(r.icon || "") + " " + escHtml(r.name_zh) + '</span>' +
      '<span class="mastery-level">Lv.' + r.level + " " + escHtml(r.title) + bonusTag + '</span>' +
      '</div>' +
      '<div class="mastery-bar-track"><div class="mastery-bar-fill" style="width:' + pct + '%"></div></div>' +
      '<div class="mastery-xp">' + r.xp + ' / ' + r.next_level_xp + ' xp</div>' +
      '</div>';
  }
  masteryBodyEl.innerHTML = html;
}

/** 向後端抓這位玩家最新的熟練度並重新渲染。 */
async function refreshMastery() {
  if (!masteryBodyEl) return;
  try {
    const resp = await fetch(`/voxel/mastery?player=${encodeURIComponent(myName)}`);
    if (!resp.ok) throw new Error("mastery fetch failed: " + resp.status);
    const rows = await resp.json();
    renderMasteryPanel(rows);
  } catch (err) {
    masteryBodyEl.innerHTML = '<div class="skills-empty">無法讀取熟練度資料。</div>';
  }
}

let masteryVisible = false;

/** 開啟熟練度面板（低頻資料，開啟時抓一次即可，不必背景輪詢）。 */
function openMastery() {
  if (!masteryEl) return;
  masteryVisible = true;
  masteryEl.style.display = "flex";
  refreshMastery();
}

/** 關閉熟練度面板。 */
function closeMastery() {
  masteryVisible = false;
  if (masteryEl) masteryEl.style.display = "none";
}

if (masteryBtnEl) masteryBtnEl.addEventListener("click", () => {
  masteryVisible ? closeMastery() : openMastery();
});
if (masteryEl) {
  const closeBtn = document.getElementById("masteryClose");
  if (closeBtn) closeBtn.addEventListener("click", closeMastery);
}

// ── 村莊地圖（自主提案切片，ROADMAP 837）────────────────────────────────────────
// 村莊系統（835）早把居民的家收攏成中央廣場＋十字主路＋沿路地塊的實體佈局，玩家也
// 走在真正鋪好的石板路上——但那份佈局只活在腳下：玩家從沒有任何管道一眼看到「村子
// 多大、廣場在哪、誰住哪塊地」，只能靠雙腳丈量。跟羅盤（705雷達820）異曲同工：
// 讓早已存在的系統第一次被看見。與雷達的關鍵區隔——雷達永遠以玩家為中心、只畫「居民
// 目前位置」（會走動）；地圖以村莊中心為固定原點、畫的是「地塊佈局」這種不隨居民走動
// 而變的**地理**（僅認領時才變），兩者互補、維度不同。
const mapEl = document.getElementById("mapPanel");
const mapCanvasEl = document.getElementById("villageMapCanvas");
const mapCtx = mapCanvasEl ? mapCanvasEl.getContext("2d") : null;
const mapBtnEl = document.getElementById("mapBtn");
/** 地圖涵蓋半徑（遊戲單位，以村莊中心為原點）：略大於最遠地塊距離（PLOT_FIRST_OFFSET 20 +
 * PLOT_STRIDE 22 * 2 ≈ 64），留邊界餘裕看得到主路延伸出去的方向。 */
const VILLAGE_MAP_RANGE_UNITS = 90;

/** 把「世界座標相對村莊中心的偏移」換算成地圖畫布上的相對座標（純函式、可測）。
 * 超出 rangeUnits 的點回傳 `clamped=true`，呼叫端可選擇跳過不畫（避免畫到畫布外）。
 * @returns {{x:number,y:number,clamped:boolean}}
 */
export function villageMapPoint(worldX, worldZ, centerX, centerZ, rangeUnits, radiusPx) {
  const dx = worldX - centerX, dz = worldZ - centerZ;
  const clamped = Math.abs(dx) > rangeUnits || Math.abs(dz) > rangeUnits;
  return { x: (dx / rangeUnits) * radiusPx, y: (dz / rangeUnits) * radiusPx, clamped };
}

let mapData = null; // 最近一次 /voxel/village-map 回應（{cx,cz,plaza_radius,road_reach,plots}）
let mapVisible = false;
let mapRedrawTimer = null;

/** 重繪村莊地圖畫布：十字主路 + 廣場方形 + 各地塊（已認領=金點+名字／空地=灰點）+ 玩家藍點。 */
function renderVillageMap() {
  if (!mapCtx || !mapCanvasEl) return;
  const ctx = mapCtx;
  const w = mapCanvasEl.width, h = mapCanvasEl.height;
  const cx = w / 2, cy = h / 2;
  const radiusPx = Math.min(w, h) / 2 - 10;
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = "rgba(10,14,22,0.85)";
  ctx.fillRect(0, 0, w, h);
  if (!mapData) return;
  // 十字主路（貫穿整張畫布，村莊主路實際延伸更遠，畫布邊界即代表「路還沒到頭」）。
  ctx.strokeStyle = "rgba(220,220,200,0.35)";
  ctx.lineWidth = 5;
  ctx.beginPath(); ctx.moveTo(0, cy); ctx.lineTo(w, cy); ctx.stroke();
  ctx.beginPath(); ctx.moveTo(cx, 0); ctx.lineTo(cx, h); ctx.stroke();
  // 中央廣場（正方形）。
  const plazaPx = (mapData.plaza_radius / VILLAGE_MAP_RANGE_UNITS) * radiusPx;
  ctx.fillStyle = "rgba(220,200,140,0.55)";
  ctx.fillRect(cx - plazaPx, cy - plazaPx, plazaPx * 2, plazaPx * 2);
  // 沿路地塊：已認領=金點+名字，空地=半透明灰點。
  ctx.textAlign = "center";
  ctx.font = "10px sans-serif";
  for (const p of mapData.plots) {
    const pt = villageMapPoint(p.cx, p.cz, mapData.cx, mapData.cz, VILLAGE_MAP_RANGE_UNITS, radiusPx);
    if (pt.clamped) continue; // 超出地圖顯示範圍的地塊不畫（範圍已涵蓋現行村莊規模）
    const px = cx + pt.x, py = cy + pt.y;
    ctx.beginPath();
    ctx.arc(px, py, p.resident ? 5 : 3, 0, Math.PI * 2);
    ctx.fillStyle = p.resident ? "#ffd479" : "rgba(255,255,255,0.35)";
    ctx.fill();
    if (p.resident) {
      ctx.fillStyle = "#eaf2ff";
      ctx.fillText(p.resident, px, py - 8);
    }
  }
  // 玩家目前位置（超出範圍就夾在邊緣，維持「你在那個方向」的直覺）。
  const pp = villageMapPoint(player.x, player.z, mapData.cx, mapData.cz, VILLAGE_MAP_RANGE_UNITS, radiusPx);
  const dist = Math.hypot(pp.x, pp.y) || 1;
  const ppx = pp.clamped ? cx + (pp.x / dist) * radiusPx : cx + pp.x;
  const ppy = pp.clamped ? cy + (pp.y / dist) * radiusPx : cy + pp.y;
  ctx.beginPath();
  ctx.arc(ppx, ppy, 5, 0, Math.PI * 2);
  ctx.fillStyle = "#8ab8ff";
  ctx.fill();
  ctx.strokeStyle = "#eaf2ff";
  ctx.lineWidth = 1.5;
  ctx.stroke();
}

/** 向後端抓最新村莊地圖資料（中心/廣場半徑/地塊認領）並重新繪製。 */
async function refreshVillageMap() {
  try {
    const resp = await fetch("/voxel/village-map");
    if (!resp.ok) throw new Error("village-map fetch failed: " + resp.status);
    mapData = await resp.json();
  } catch (err) {
    mapData = null;
  }
  renderVillageMap();
}

/** 開啟村莊地圖面板：抓一次地塊佈局（地塊認領變化很慢，30 秒刷新一次足夠），
 * 玩家位置則每 0.3 秒重繪一次（沿用既有資料、零額外請求，比照雷達的即時感）。 */
function openVillageMap() {
  if (!mapEl) return;
  mapVisible = true;
  mapEl.style.display = "flex";
  refreshVillageMap();
  if (mapRedrawTimer) clearInterval(mapRedrawTimer);
  mapRedrawTimer = setInterval(() => { if (mapVisible) renderVillageMap(); }, 300);
}

/** 關閉村莊地圖面板。 */
function closeVillageMap() {
  mapVisible = false;
  if (mapEl) mapEl.style.display = "none";
  if (mapRedrawTimer) { clearInterval(mapRedrawTimer); mapRedrawTimer = null; }
}

if (mapBtnEl) mapBtnEl.addEventListener("click", () => {
  mapVisible ? closeVillageMap() : openVillageMap();
});
if (mapEl) {
  const closeBtn = document.getElementById("mapClose");
  if (closeBtn) closeBtn.addEventListener("click", closeVillageMap);
}

// ── 準心選取 + 高亮外框（MCPE 風）──────────────────────────────────────────────
// 選中方塊的線框外框（略大一點點避免 z-fighting）。對準時顯示、沒對到時隱藏。
const highlight = new THREE.LineSegments(
  new THREE.EdgesGeometry(new THREE.BoxGeometry(1.002, 1.002, 1.002)),
  new THREE.LineBasicMaterial({ color: 0x101014, transparent: true, opacity: 0.9 })
);
highlight.visible = false;
scene.add(highlight);
// 目前準心對準的方塊：{ bx,by,bz（命中方塊）, nx,ny,nz（命中面法線，放置往這方向偏一格）}
let target = null;

// ── 快捷欄（選要放的方塊型別）+ 背包存量（採集 v1）───────────────────────────
// 種田 v1（ROADMAP 659）：加入農田土 + 種子（種子為純物品，特殊 Plant 動作）
// 快捷欄麥塊化：固定 9 格（麥塊就是 9 格），數字鍵 1-9 選格、手機點選。
// 完整物品清單移到「背包」面板，從背包點物品即可指派進當前選中的快捷欄格。
// 空格 = AIR(0)；每格內容持久化到 localStorage，重整後保留。
const HOTBAR_SIZE = 9;
const HOTBAR_LS_KEY = "butfun.voxel.hotbar.v1";
// 預設起手：草/土/石/木/木板 + 木鎬，其餘留空；開局不空白、也不再洗版。
const HOTBAR_DEFAULT = [GRASS, DIRT, STONE, WOOD, PLANK, PICKAXE_WOOD, AIR, AIR, AIR];
// 從 localStorage 還原上次的快捷欄指派；資料壞掉或不可用就 fallback 預設。
function loadHotbar() {
  try {
    const raw = localStorage.getItem(HOTBAR_LS_KEY);
    if (raw) {
      const arr = JSON.parse(raw);
      if (Array.isArray(arr) && arr.length === HOTBAR_SIZE &&
          arr.every((n) => Number.isInteger(n) && n >= 0)) {
        return arr;
      }
    }
  } catch (_) { /* localStorage 不可用或壞資料：用預設 */ }
  return HOTBAR_DEFAULT.slice();
}
const HOTBAR = loadHotbar();
// 把當前快捷欄指派寫回 localStorage（每次指派/變更後呼叫）。
function saveHotbar() {
  try { localStorage.setItem(HOTBAR_LS_KEY, JSON.stringify(HOTBAR)); } catch (_) { /* 忽略：無痕/禁 storage */ }
}
// 指派一個 block 到指定快捷欄格（麥塊互動：從背包點物品→放進當前格），存檔並重建 UI。
function assignToHotbar(slot, blockId) {
  if (slot < 0 || slot >= HOTBAR_SIZE) return;
  HOTBAR[slot] = blockId;
  saveHotbar();
  buildHotbar();
  updateInvHud();
}
const BLOCK_NAME = {
  [GRASS]: "草", [DIRT]: "土", [STONE]: "石", [WOOD]: "木", [SAND]: "沙", [LEAVES]: "葉",
  [PLANK]: "木板", [STONE_BRICK]: "石磚", [GLASS]: "玻璃",
  // 種田 v1
  [FARM_SOIL]: "農田土", [FARM_SOIL_SEEDED]: "幼苗", [WHEAT_MATURE]: "成熟小麥",
  [SEEDS]: "種子",
  // 工作台 v1
  [WORKBENCH]: "工作台",
  // 熔爐 v1
  [FURNACE]: "熔爐", [SMOOTH_STONE]: "拋光石",
  // 麵包 v1（ROADMAP 668）
  [WHEAT]: "小麥", [BREAD]: "麵包",
  // 深層礦石 v1（ROADMAP 682）
  [COAL_ORE]: "煤礦", [IRON_ORE]: "鐵礦",
  // 鐵錠 v1（ROADMAP 683）
  [IRON_INGOT]: "鐵錠",
  // 鐵磚 v1（ROADMAP 684）
  [IRON_BLOCK]: "鐵磚",
  // 火把 v1（ROADMAP 685）
  [TORCH]: "火把",
  // 鎬具 v1（ROADMAP 687）
  [PICKAXE_WOOD]: "木鎬", [PICKAXE_STONE]: "石鎬", [PICKAXE_IRON]: "鐵鎬",
  [AXE_WOOD]: "木斧", [AXE_STONE]: "石斧", [AXE_IRON]: "鐵斧",
  // 梯子 v1（ROADMAP 688）
  [LADDER]: "梯子",
  // 鏟子 v1（ROADMAP 690）
  [SHOVEL_WOOD]: "木鏟", [SHOVEL_STONE]: "石鏟", [SHOVEL_IRON]: "鐵鏟",
  // 箱子 v1（ROADMAP 692）
  [CHEST]: "箱子",
  // 木門 v1（ROADMAP 693）
  [DOOR_CLOSED]: "木門（關）", [DOOR_OPEN]: "木門（開）",
  // 床 v1
  [BED]: "床",
  // 第二種作物 v1
  [CARROT_SEEDED]: "胡蘿蔔幼苗", [CARROT_MATURE]: "成熟胡蘿蔔",
  [CARROT_SEEDS]: "胡蘿蔔種子", [CARROT]: "胡蘿蔔",
  // 第三種作物 v1
  [POTATO_SEEDED]: "馬鈴薯幼苗", [POTATO_MATURE]: "成熟馬鈴薯",
  [POTATO_SEEDS]: "馬鈴薯種子", [POTATO]: "馬鈴薯",
  // 生物群系第一刀/第二刀
  [CACTUS]: "仙人掌", [SNOW]: "雪", [ICE_CRYSTAL]: "冰晶",
  // 冰晶燈 v1（冰晶合成）
  [ICE_LANTERN]: "冰晶燈",
  // 乙太礦脈 v1
  [AETHER_ORE]: "乙太礦", [AETHER_LAMP]: "乙太燈",
  // 垂釣 v1（ROADMAP 734）
  [FISHING_ROD]: "釣竿", [FISH]: "小魚", [AETHER_FISH]: "乙太魚", [COOKED_FISH]: "烤魚",
  [BAKED_POTATO]: "烤地薯",
  // 野菜暖湯 v1（ROADMAP 778）
  [STEW]: "野菜暖湯",
  // 乙太煙火 v1（ROADMAP 785）
  [FIREWORK]: "乙太煙火",
  // 乙太沃肥 v1（ROADMAP 789）
  [FERTILIZER]: "乙太沃肥",
  // 水桶 v1（自主提案切片）
  [BUCKET]: "水桶", [WATER_BUCKET]: "滿水桶",
  // 鋤頭 v1（自主提案切片）
  [HOE]: "木鋤頭",
  // 集會鐘 v1（自主提案切片）
  [BELL]: "集會鐘",
  // 植樹造林 v1（ROADMAP 738）
  [SAPLING]: "樹苗",
  // 告示牌 v1（ROADMAP 740）
  [SIGN]: "告示牌",
  // 莓果叢 v1（自主提案切片 806）
  [BERRY_BUSH]: "莓果叢苗", [BERRY_BUSH_RIPE]: "結果的莓果叢", [BERRY]: "莓果",
  [BENCH]: "木長椅",
  [JAM]: "莓果醬",
  // 雞舍生蛋 v1（自主提案切片）
  [COOP]: "雞舍", [COOP_READY]: "有蛋的雞舍", [EGG]: "蛋",
  // 漂流瓶 v1（自主提案切片 825）
  [BOTTLE]: "空玻璃瓶",
  // 建築藍圖 v1（自主提案切片）
  [BLUEPRINT_HOUSE]: "小屋藍圖", [BLUEPRINT_WELL]: "水井藍圖", [BLUEPRINT_TOWER]: "瞭望台藍圖",
  [BLUEPRINT_GARDEN]: "花圃藍圖", [BLUEPRINT_PAVILION]: "涼亭藍圖",
  // 染色建材 v1（自主提案切片）
  [TERRACOTTA_RED]: "紅陶磚", [TERRACOTTA_BLACK]: "黑陶磚",
  [TERRACOTTA_WHITE]: "白陶磚", [TERRACOTTA_BLUE]: "青陶磚",
  // 野花 v1（自主提案切片）
  [WILDFLOWER_RED]: "紅花", [WILDFLOWER_YELLOW]: "黃花", [WILDFLOWER_BLUE]: "藍花",
  // 居民教你一道獨門配方 v1（自主提案切片，ROADMAP 849）
  [AMULET]: "護身符",
  // 乙太幣 v1（ROADMAP 873，自主提案切片）
  [COIN]: "乙太幣",
  // 驅影之劍 v1（ROADMAP 887，自主提案切片）
  [SWORD_WOOD]: "木劍", [SWORD_STONE]: "石劍", [SWORD_IRON]: "鐵劍",
};
let selectedSlot = 0; // HOTBAR 索引
// 垂釣 v1（ROADMAP 734）：釣線是否已在水裡（拋竿後、收竿前）。伺服器權威把關時機，
// 此旗標只讓「同一個放置動作」在拋竿／收竿之間切換，並驅動上鉤提示。
let fishPending = false;
let fishBiteTimer = null; // 上鉤提示的客戶端計時器（純 UX，伺服器仍是唯一時機裁判）
const hotbarEl = document.getElementById("hotbar");
// 本地材料存量（block_id → count）；由 inv_sync / inv_update 伺服器訊息維護。
const myInv = new Map();

/** 更新熱鍵欄的材料數量顯示（只改 .cnt 文字，不重建整個 DOM）。 */
function updateInvHud() {
  if (!hotbarEl) return;
  HOTBAR.forEach((b, i) => {
    const slot = hotbarEl.children[i];
    if (!slot) return;
    const cnt = slot.querySelector(".cnt");
    if (b === AIR) { if (cnt) cnt.textContent = ""; slot.classList.add("empty"); return; }
    const n = myInv.get(b) || 0;
    if (cnt) cnt.textContent = n > 0 ? "×" + n : "";
    slot.classList.toggle("empty", n === 0);
  });
}

function buildHotbar() {
  if (!hotbarEl) return;
  hotbarEl.innerHTML = "";
  HOTBAR.forEach((b, i) => {
    const isEmpty = (b === AIR); // 空格：只顯示格號，不放色塊/名稱
    const slot = document.createElement("div");
    slot.className = "slot" + (i === selectedSlot ? " sel" : "") + " empty";
    const sw = document.createElement("div");
    sw.className = "sw";
    if (isEmpty) {
      sw.style.background = "transparent";
    } else {
      const c = COLOR[b] || COLOR[STONE];
      sw.style.background = `rgb(${(c[0] * 255) | 0},${(c[1] * 255) | 0},${(c[2] * 255) | 0})`;
    }
    const lbl = document.createElement("div");
    lbl.textContent = isEmpty ? String(i + 1) : ((i + 1) + " " + (BLOCK_NAME[b] || "?"));
    const cnt = document.createElement("div");
    cnt.className = "cnt";
    slot.appendChild(sw); slot.appendChild(lbl); slot.appendChild(cnt);
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      // 麥塊互動：背包開著且手上拿著物品（bagPick）→ 指派到這一格；否則單純選格。
      if (bagPanelVisible() && bagPick !== 0) {
        assignToHotbar(i, bagPick);
        bagPick = 0;
        selectSlot(i);
        renderBagPanel();
      } else {
        selectSlot(i);
      }
    });
    hotbarEl.appendChild(slot);
  });
  updateInvHud(); // 重建後補上數量/空格樣式
}
function selectSlot(i) {
  selectedSlot = ((i % HOTBAR.length) + HOTBAR.length) % HOTBAR.length;
  for (let k = 0; k < hotbarEl.children.length; k++) {
    hotbarEl.children[k].classList.toggle("sel", k === selectedSlot);
  }
  // 手持食物時：放置鈕改標「吃」，提示放置＝吃（玩家生存指標 v1·溫和版）。
  refreshPlaceBtnLabel();
}
function selectedBlock() { return HOTBAR[selectedSlot]; }

/** 依手持物是否食物，把放置鈕標示成「吃」或「放置」（提示放置鈕此刻會吃）。 */
function refreshPlaceBtnLabel() {
  const pb = document.getElementById("place");
  if (!pb) return;
  pb.textContent = (typeof heldIsFood === "function" && heldIsFood()) ? "🍽 吃" : "放置";
}

// 掉落物 v1（自主提案切片 828）：丟下目前手上選取材料的 1 份到準星處（觸及範圍內，
// 後端 reach 權威複驗）。空格／背包沒有那個材料 → 靜默忽略，不送空包。
function dropSelectedItem() {
  if (!target || !wsReady) return;
  const item = selectedBlock();
  if (item === AIR) return;
  if ((myInv.get(item) || 0) < 1) return;
  ws.send(JSON.stringify({ t: "drop_item", x: target.bx, y: target.by, z: target.bz, item_id: item, count: 1 }));
}

// 玩家自由市集 v1（自主提案切片 832）：把手上選取的材料擺成一攤交易看板，問清楚想換的東西
// 後送出——擺在瞄準格的面外側（比照一般放置的偏移，攤子立在你腳下踏實的地面上）。
function openStallAtTarget() {
  if (!target || !wsReady) return;
  const giveItem = selectedBlock();
  if (giveItem === AIR) return;
  if ((myInv.get(giveItem) || 0) < 1) return;
  const wantName = (window.prompt("這攤想換什麼？（輸入物品中文名，如：木頭）", "") || "").trim();
  if (!wantName) return;
  const wantItem = Number(Object.keys(BLOCK_NAME).find((id) => BLOCK_NAME[id] === wantName));
  if (!wantItem || Number.isNaN(wantItem)) {
    showErr("不認得這個物品名字，換個名字試試？");
    setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    return;
  }
  if (wantItem === giveItem) {
    showErr("不能拿同一種東西換自己喔。");
    setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    return;
  }
  const px = target.bx + target.nx, py = target.by + target.ny, pz = target.bz + target.nz;
  ws.send(JSON.stringify({
    t: "stall_open", x: px, y: py, z: pz,
    give_item: giveItem, give_count: 1, want_item: wantItem, want_count: 1,
  }));
}
buildHotbar();
// 數字鍵 1-9 切快捷欄（麥塊固定 9 格）
addEventListener("keydown", (e) => {
  if (e.target && e.target.tagName === "INPUT") return; // 對話輸入中不搶鍵
  const n = parseInt(e.key, 10);
  if (n >= 1 && n <= HOTBAR.length) selectSlot(n - 1);
});

// 純函式：玩家 AABB 是否與任一梯子方塊重疊（攀爬判定）。
// 不依賴全域，可測；getBlock(bx,by,bz) 回傳方塊 id（未載入回 -1）。
function aabbHitsLadder(x, y, z, getBlock, pw, ph) {
  const x0 = Math.floor(x - pw), x1 = Math.floor(x + pw);
  const y0 = Math.floor(y), y1 = Math.floor(y + ph - 0.01);
  const z0 = Math.floor(z - pw), z1 = Math.floor(z + pw);
  for (let bx = x0; bx <= x1; bx++)
    for (let by = y0; by <= y1; by++)
      for (let bz = z0; bz <= z1; bz++)
        if (getBlock(bx, by, bz) === LADDER) return true;
  return false;
}

// 純函式：以「腳底在 y、半寬 pw、身高 ph」的 AABB，問 isSolid(bx,by,bz) 是否與任一實心格重疊。
// 不依賴全域（player/solidCollide 由呼叫端帶入），方便真瀏覽器 QA 直接餵假地形驗證。
function aabbHitsSolid(x, y, z, isSolid, pw, ph) {
  const x0 = Math.floor(x - pw), x1 = Math.floor(x + pw);
  const y0 = Math.floor(y), y1 = Math.floor(y + ph - 0.01);
  const z0 = Math.floor(z - pw), z1 = Math.floor(z + pw);
  for (let bx = x0; bx <= x1; bx++)
    for (let by = y0; by <= y1; by++)
      for (let bz = z0; bz <= z1; bz++)
        if (isSolid(bx, by, bz)) return true;
  return false;
}

// 純函式：脫困（depenetration）。若 (x,y,z) 的 AABB 卡在實心方塊內，沿 +Y 逐格上抬，
// 回傳第一個「不再重疊」的整數腳底高度（站到方塊頂上）。本來就沒卡 → 原值返回（不動）。
// 收斂穩定：找到第一個 clear 高度就停，不會持續往上彈；maxRise 防呆（理論上地表之上必有空氣）。
function unstuckY(x, y, z, isSolid, pw, ph, maxRise = 96) {
  if (!aabbHitsSolid(x, y, z, isSolid, pw, ph)) return y; // 沒卡：完全不干擾正常走路/重力/踏階
  let ny = Math.floor(y);
  for (let i = 0; i <= maxRise; i++) {
    if (!aabbHitsSolid(x, ny, z, isSolid, pw, ph)) return ny;
    ny += 1;
  }
  return y; // 極端情況找不到 clear：維持原值，至少不亂跳
}

// AABB 是否與任一實心方塊重疊（碰撞核心）；用上面的純函式，餵入玩家當前位置。
function overlaps() {
  return aabbHitsSolid(player.x, player.y, player.z, solidCollide, PW, PH);
}

// 脫困保險：玩家被實心方塊埋住時（出生瞬間 / 新 chunk 載入 / 從未載入區走入後補載），
// 把腳底上抬到第一個不重疊高度頂出來，避免永久卡死。只在「真的重疊實心」時作用，
// 平常走路/重力/踏階完全不觸發（unstuckY 開頭早退）。回傳是否有脫困。
function unstuckIfNeeded() {
  if (!overlaps()) return false;
  const ny = unstuckY(player.x, player.y, player.z, solidCollide, PW, PH);
  if (ny !== player.y) {
    player.y = ny;
    player.vy = 0;       // 清掉下墜速度，別把人又壓回方塊裡
    stepSmooth = 0;      // 視覺別殘留踏階補間
    player.grounded = false; // 讓重力把人輕輕放到地表，穩定收斂
    return true;
  }
  return false;
}

// 水平移動一軸：撞牆就回退；若站在地上，試著踏上 1 格高台階（讓走斜坡/小丘順暢）。
// 踏上台階時：物理 Y 瞬間到位（碰撞/重力繼續正確運作），同時累積 stepSmooth 讓視覺 Y
// 從原地平滑抬升（update() 每幀指數衰減），消除以往「瞬間彈跳一格」的閃爍感。
function moveAxis(axis, delta) {
  if (delta === 0) return;
  const prev = player[axis];
  player[axis] += delta;
  if (!overlaps()) return;
  // 自動跳躍設定關閉時，撞到一格高就直接擋住（需手動按跳）；開啟＝維持既有踏階手感。
  if (player.grounded && settings.autoJump) {
    const py = player.y;
    player.y += 1.05;
    if (!overlaps()) {
      // 踏上台階成功：物理 Y 已到位；累積視覺補間偏移（visualY 由 update() 平滑追上）
      stepSmooth += player.y - py;
      return;
    }
    player.y = py;
  }
  player[axis] = prev; // 完全擋住 → 回退
}

// ── Voxel raycast（自寫 DDA 體素行進；不抄外部碼）──────────────────────────────
// 從原點 (ox,oy,oz) 沿單位方向 (dx,dy,dz) 一格一格走，回傳第一個非空氣/非水的實心方塊，
// 連同「進入該方塊時跨過的面法線」(nx,ny,nz)——放置時往這方向偏一格即面外側。
const RAY_MAX = 6.0; // 觸及距離（與後端 REACH 對齊）
function raycastVoxel(ox, oy, oz, dx, dy, dz) {
  let bx = Math.floor(ox), by = Math.floor(oy), bz = Math.floor(oz);
  const stepX = dx > 0 ? 1 : -1, stepY = dy > 0 ? 1 : -1, stepZ = dz > 0 ? 1 : -1;
  const tDeltaX = dx !== 0 ? Math.abs(1 / dx) : Infinity;
  const tDeltaY = dy !== 0 ? Math.abs(1 / dy) : Infinity;
  const tDeltaZ = dz !== 0 ? Math.abs(1 / dz) : Infinity;
  // 到下一個格界的參數距離。
  const fx = dx > 0 ? (bx + 1 - ox) : (ox - bx);
  const fy = dy > 0 ? (by + 1 - oy) : (oy - by);
  const fz = dz > 0 ? (bz + 1 - oz) : (oz - bz);
  let tMaxX = dx !== 0 ? fx * tDeltaX : Infinity;
  let tMaxY = dy !== 0 ? fy * tDeltaY : Infinity;
  let tMaxZ = dz !== 0 ? fz * tDeltaZ : Infinity;
  let nx = 0, ny = 0, nz = 0, t = 0;
  for (let guard = 0; guard < 64; guard++) {
    const r = getRaw(bx, by, bz);
    if (r > 0 && !isWaterId(r)) return { bx, by, bz, nx, ny, nz }; // 穿過來源/流動水，命中實心才停
    if (tMaxX < tMaxY && tMaxX < tMaxZ) {
      bx += stepX; t = tMaxX; tMaxX += tDeltaX; nx = -stepX; ny = 0; nz = 0;
    } else if (tMaxY < tMaxZ) {
      by += stepY; t = tMaxY; tMaxY += tDeltaY; nx = 0; ny = -stepY; nz = 0;
    } else {
      bz += stepZ; t = tMaxZ; tMaxZ += tDeltaZ; nx = 0; ny = 0; nz = -stepZ;
    }
    if (t > RAY_MAX) break;
  }
  return null;
}

// 視線方向（含俯仰）：從鏡頭中心穿過準心的方向 = 鏡頭看向 target 的方向。
function viewDir() {
  // 與 update() 的鏡頭擺位一致：鏡頭在玩家後上方、看向玩家頭頂。
  const tx = player.x, ty = player.y + 1.3, tz = player.z;
  const dist = 6.0, cp = Math.cos(camPitch), sp = Math.sin(camPitch);
  const camx = tx + Math.sin(player.yaw) * dist * cp;
  const camy = ty + dist * sp;
  const camz = tz + Math.cos(player.yaw) * dist * cp;
  const d = new THREE.Vector3(tx - camx, ty - camy, tz - camz);
  d.normalize();
  return d;
}

// 更新準心對準的方塊（每幀算）：從玩家眼睛沿視線 raycast。
function updateTarget() {
  const dir = viewDir();
  const eye = { x: player.x, y: player.y + 1.5, z: player.z };
  target = raycastVoxel(eye.x, eye.y, eye.z, dir.x, dir.y, dir.z);
  if (target) {
    highlight.visible = true;
    highlight.position.set(target.bx + 0.5, target.by + 0.5, target.bz + 0.5);
  } else {
    highlight.visible = false;
  }
}

// 本地套用一個方塊更新（伺服器廣播 / 樂觀預測共用）：改 chunk 資料 + 標記受影響 chunk 重建。
// 只重建該 chunk（及鄰塊，邊界面剔除用），絕不整個世界重建（延續切片① FPS 鐵律）。
function setLocalBlock(wx, wy, wz, b) {
  const cx = Math.floor(wx / CHUNK), cy = Math.floor(wy / CHUNK), cz = Math.floor(wz / CHUNK);
  const ch = chunks.get(ckey(cx, cy, cz));
  if (!ch) return; // 該 chunk 還沒載入——之後串流會帶正確（含 delta）的版本。
  const lx = wx - cx * CHUNK, ly = wy - cy * CHUNK, lz = wz - cz * CHUNK;
  const old = ch[lx + lz * CHUNK + ly * CHUNK * CHUNK];
  // 發光方塊 v1：放發光方塊→登記；破壞→移除登記（讓光源池即時更新；火把暖橘／乙太燈青藍）。
  if (isLightBlock(old)) unregisterTorchBlock(wx, wy, wz);
  ch[lx + lz * CHUNK + ly * CHUNK * CHUNK] = b;
  if (isLightBlock(b)) registerTorchBlock(wx, wy, wz, lightColorFor(b));
  markDirty(cx, cy, cz); // markDirty 只標該 chunk + 6 鄰塊
}

// ── 採礦手感 v1（ROADMAP 687）─────────────────────────────────────────────────
// 桌機：按住左鍵持續挖，挖滿進度條後才真正送 break（與手感一致）。
// 行動裝置：輕點保持即時挖（MCPE v1；採礦進度條不影響手機操作體感）。

// 方塊硬度（秒）——土/草/沙最快，礦石最慢。
const BLOCK_HARDNESS = {
  [GRASS]: 0.35, [DIRT]: 0.35, [SAND]: 0.35, [LEAVES]: 0.25,
  [WOOD]: 0.75,
  [PLANK]: 0.9, [STONE_BRICK]: 1.8,
  [STONE]: 1.8, [SMOOTH_STONE]: 2.0,
  [COAL_ORE]: 2.5, [IRON_ORE]: 2.5,
  [IRON_INGOT]: 1.5, [IRON_BLOCK]: 2.0,
  [FARM_SOIL]: 0.4, [FARM_SOIL_SEEDED]: 0.4, [WHEAT_MATURE]: 0.2,
  [CARROT_SEEDED]: 0.4, [CARROT_MATURE]: 0.2,
  [POTATO_SEEDED]: 0.4, [POTATO_MATURE]: 0.2,
  [WORKBENCH]: 1.2, [FURNACE]: 1.5,
  [TORCH]: 0.1,
  [LADDER]: 0.4,  // 梯子——木製，輕鬆打破
  [CHEST]: 1.0,   // 箱子——木箱，中等硬度（含存量，需謹慎破壞）
  [DOOR_CLOSED]: 0.8,  // 木門（關）——木製，輕鬆打破
  [DOOR_OPEN]:   0.8,  // 木門（開）——同材質，可破壞
  [BED]: 0.6,  // 床——布料+木架，輕鬆打破
  [ICE_CRYSTAL]: 1.2,  // 冰晶——結晶偏脆但需點時間敲下，比礦石快、比石頭稍慢
  [AETHER_ORE]: 2.8,   // 乙太礦——世界最硬最深的礦，比煤/鐵礦更耐敲（需鎬加速）
  [AETHER_LAMP]: 0.3,  // 乙太燈——玻璃燈罩，輕鬆敲下回收
  [SAPLING]: 0.2,      // 樹苗——嫩苗一敲即落（比照作物幼苗），輕鬆回收重種
  [SIGN]: 0.5,         // 告示牌——一塊木牌，輕鬆敲下回收（文字一併消失）
  // 染色建材 v1（自主提案切片）——燒製陶磚，比照石磚硬度（同為精緻建材）
  [TERRACOTTA_RED]: 1.6, [TERRACOTTA_BLACK]: 1.6, [TERRACOTTA_WHITE]: 1.6, [TERRACOTTA_BLUE]: 1.6,
  // 野花 v1（自主提案切片）——一叢嬌嫩野花，比照樹苗一敲即落，輕鬆採下帶走。
  [WILDFLOWER_RED]: 0.2, [WILDFLOWER_YELLOW]: 0.2, [WILDFLOWER_BLUE]: 0.2,
};
function blockHardness(bid) { return BLOCK_HARDNESS[bid] ?? 1.0; }

// 鎬具加速倍率（持特定鎬對石/礦類方塊的速度倍數）。
function pickaxeBonus(bid) {
  const stoneTypes = [STONE, STONE_BRICK, SMOOTH_STONE, COAL_ORE, IRON_ORE, IRON_BLOCK, IRON_INGOT, WORKBENCH, FURNACE, AETHER_ORE,
    TERRACOTTA_RED, TERRACOTTA_BLACK, TERRACOTTA_WHITE, TERRACOTTA_BLUE];
  if (!stoneTypes.includes(bid)) return 1.0;
  if ((myInv.get(PICKAXE_IRON) || 0) > 0) return 6.0;   // 鐵鎬：最快
  if ((myInv.get(PICKAXE_STONE) || 0) > 0) return 4.0;  // 石鎬：快
  if ((myInv.get(PICKAXE_WOOD) || 0) > 0) return 2.5;   // 木鎬：普通加速
  return 1.0; // 空手：基礎速度
}

// 斧頭加速倍率（持特定斧對木材類方塊的速度倍數；ROADMAP 689）。
// 只對木頭/葉片/木板有效；鎬具類方塊回 1.0（斧頭不補石礦）。
export function axeBonus(bid) {
  const woodTypes = [WOOD, LEAVES, PLANK];
  if (!woodTypes.includes(bid)) return 1.0;
  if ((myInv.get(AXE_IRON) || 0) > 0) return 6.0;   // 鐵斧：最快
  if ((myInv.get(AXE_STONE) || 0) > 0) return 4.0;  // 石斧：快
  if ((myInv.get(AXE_WOOD) || 0) > 0) return 2.5;   // 木斧：普通加速
  return 1.0; // 空手：基礎速度
}

// 鏟子加速倍率（持特定鏟對軟土類方塊的速度倍數；ROADMAP 690）。
// 只對草地/泥土/沙地/農田有效；石/木類方塊回 1.0（鏟子不補硬材）。
export function shovelBonus(bid) {
  const softTypes = [GRASS, DIRT, SAND, FARM_SOIL, FARM_SOIL_SEEDED];
  if (!softTypes.includes(bid)) return 1.0;
  if ((myInv.get(SHOVEL_IRON) || 0) > 0) return 6.0;   // 鐵鏟：最快
  if ((myInv.get(SHOVEL_STONE) || 0) > 0) return 4.0;  // 石鏟：快
  if ((myInv.get(SHOVEL_WOOD) || 0) > 0) return 2.5;   // 木鏟：普通加速
  return 1.0; // 空手：基礎速度
}

// 採礦狀態（桌機按住左鍵期間維持）。
let mining = null; // { x, y, z, bid, progress, total } 或 null

// 進度條 DOM（在 crosshair 正下方渲染進度）。
const miningBarEl = document.getElementById("miningBar");
const miningBarFillEl = document.getElementById("miningBarFill");

function updateMiningBar(frac) {
  if (!miningBarEl) return;
  if (frac === null) {
    miningBarEl.style.display = "none";
    return;
  }
  miningBarEl.style.display = "block";
  if (miningBarFillEl) miningBarFillEl.style.width = Math.min(1, frac) * 100 + "%";
}

/** 開始對準心對準的方塊計時挖掘（桌機模式）。*/
function startMining() {
  if (!target || !wsReady) return;
  const bid = getRaw(target.bx, target.by, target.bz);
  const hardness = blockHardness(bid);
  // 鎬具補石/礦，斧頭補木材，鏟子補土/沙，三者互補不疊加（各自對其他類回 1.0）
  const bonus = pickaxeBonus(bid) * axeBonus(bid) * shovelBonus(bid);
  const total = hardness / bonus; // 實際需要幾秒
  mining = { x: target.bx, y: target.by, z: target.bz, bid, progress: 0, total };
  updateMiningBar(0);
}

/** 取消當前採礦計時（鬆開左鍵、切換目標時）。*/
function cancelMining() {
  mining = null;
  updateMiningBar(null);
}

/** 每幀推進採礦進度（dt 秒），完成時送 break 訊息。應在 requestAnimationFrame 迴圈呼叫。*/
function tickMining(dt) {
  if (!mining) return;
  // 挖掘持續中的判定：桌機按住左鍵、手機按住挖鈕、手把按住挖鍵，任一為真即續挖。
  const digHeld = isMouseDown || touchDigHeld || gamepadDigHeld;
  // 若準心目標改變（轉頭對準另一格），重置進度。
  if (!target || target.bx !== mining.x || target.by !== mining.y || target.bz !== mining.z) {
    cancelMining();
    if (target && digHeld) startMining();
    return;
  }
  mining.progress += dt;
  if (mining.progress >= mining.total) {
    // 進度滿：送 break，立刻開始下一塊（如果按著）。
    // 工欲善其事 v1（790）：附上手持物品 id；伺服器查背包確認是真工具才給採集加成。
    ws.send(JSON.stringify({ t: "break", x: mining.x, y: mining.y, z: mining.z, tool: selectedBlock() }));
    cancelMining();
    if (digHeld) startMining();
  } else {
    updateMiningBar(mining.progress / mining.total);
  }
}

// 破壞準心對準的方塊：送 break（伺服器驗證後廣播 → setLocalBlock 套用）。回傳被挖座標或 null。
// 注意：行動裝置仍走即時送 break（MCPE 快感體驗）；桌機走 startMining / tickMining。
function breakAtTarget() {
  if (!target || !wsReady) return null;
  const c = { x: target.bx, y: target.by, z: target.bz };
  // 工欲善其事 v1（790）：附上手持物品 id 讓伺服器判定採集加成（見上）。
  ws.send(JSON.stringify({ t: "break", x: c.x, y: c.y, z: c.z, tool: selectedBlock() }));
  return c;
}

// 桌機是否按住左鍵（追蹤採礦狀態用）。
let isMouseDown = false;
// 在準心方塊的「面外側」放一個方塊：座標 = 命中方塊 + 命中面法線。回傳放置座標或 null。
// 種田 v1 + 工作台 v1：特殊互動邏輯，再 fallback 到一般放置。
function placeAtTarget() {
  if (!target || !wsReady) return null;
  // 空的快捷欄格（AIR）：沒選任何方塊，靜默忽略（避免送出 place AIR 誤刪）。
  if (selectedBlock() === AIR) return null;
  // 玩家自由市集 v1（自主提案切片 832）：右鍵對準已有攤位（不論手上拿什麼）→ 互動它
  //   （你自己的攤位＝收攤退還；別人的攤位＝有你要換的東西就成交）。伺服器權威判定，
  //   最先於一般放置判斷（比照漂流瓶讀瓶）。
  if (stallMarkers.has(target.bx + "," + target.by + "," + target.bz)) {
    ws.send(JSON.stringify({ t: "stall_interact", x: target.bx, y: target.by, z: target.bz }));
    return null;
  }
  // 漂流瓶 v1（自主提案切片 825）：右鍵對準已有瓶子的水面（不論手上拿什麼）→ 撿起它。
  //   伺服器單播內文揭曉、全場同步移除浮標；一次性拾起，最先高於一般放置判斷。
  if (bottleMarkers.has(target.bx + "," + target.by + "," + target.bz)) {
    ws.send(JSON.stringify({ t: "read_bottle", x: target.bx, y: target.by, z: target.bz }));
    return null;
  }
  // 工作台互動：右鍵對準工作台方塊 → 開啟 3×3 合成面板（不放置新方塊）。
  if (getRaw(target.bx, target.by, target.bz) === WORKBENCH) {
    openWbPanel();
    return null;
  }
  // 熔爐互動：右鍵對準熔爐方塊 → 開啟冶煉面板（不放置新方塊）。
  if (getRaw(target.bx, target.by, target.bz) === FURNACE) {
    openFurnacePanel();
    return null;
  }
  // 箱子互動：右鍵對準箱子方塊 → 傳送 open_chest，伺服器回 chest_view 後開面板。
  if (getRaw(target.bx, target.by, target.bz) === CHEST) {
    openChestPanel(target.bx, target.by, target.bz);
    return null;
  }
  // 木門互動（ROADMAP 693）：右鍵對準門（開或關）→ 傳送 toggle_door，伺服器廣播新狀態。
  const _doorRaw = getRaw(target.bx, target.by, target.bz);
  if (_doorRaw === DOOR_CLOSED || _doorRaw === DOOR_OPEN) {
    ws.send(JSON.stringify({ t: "toggle_door", x: target.bx, y: target.by, z: target.bz }));
    return null;
  }
  // 告示牌互動（ROADMAP 740）：右鍵對準既有告示牌 → 跳出輸入框編輯牌面文字。
  if (getRaw(target.bx, target.by, target.bz) === SIGN) {
    promptSignEdit(target.bx, target.by, target.bz, false);
    return null;
  }
  // 集會鐘互動（集會鐘 v1，自主提案切片）：右鍵對準既有集會鐘 → 敲響它（不放置新方塊）。
  //   後端權威複驗觸及範圍＋目標仍是鐘，把附近閒著的居民召來；不論手持什麼都能敲。
  if (getRaw(target.bx, target.by, target.bz) === BELL) {
    ws.send(JSON.stringify({ t: "ring_bell", x: target.bx, y: target.by, z: target.bz }));
    return null;
  }
  // 床互動：右鍵對準床 → 傳送 sleep_in_bed，夜晚時伺服器把時鐘撥到隔天黎明。
  if (getRaw(target.bx, target.by, target.bz) === BED) {
    ws.send(JSON.stringify({ t: "sleep_in_bed", x: target.bx, y: target.by, z: target.bz }));
    return null;
  }
  // 垂釣 v1（ROADMAP 734）：手持釣竿時，放置動作＝拋竿／收竿。
  //   已有一竿在水裡 → 收竿（伺服器判時機：太早會被退回，繼續等）。
  //   還沒拋 → 對準水面才拋竿（非水面靜默忽略）。
  if (selectedBlock() === FISHING_ROD) {
    if (fishPending) {
      ws.send(JSON.stringify({ t: "fish_reel" }));
      return null;
    }
    // 複用既有 isWaterId（來源水＋流動水 24~30，與後端 is_water_block 一致；非水面靜默忽略）。
    if (isWaterId(getRaw(target.bx, target.by, target.bz))) {
      ws.send(JSON.stringify({ t: "fish_cast", x: target.bx, y: target.by, z: target.bz }));
    }
    return null;
  }
  // 乙太沃肥 v1（ROADMAP 789）：手持沃肥對準一株幼苗一撒即催熟。目標是作物方塊本身
  // （不偏移到面外側）；非幼苗（成熟作物/農田土/其他）靜默忽略——後端仍會權威複驗。
  if (selectedBlock() === FERTILIZER) {
    const hitRaw = getRaw(target.bx, target.by, target.bz);
    if (hitRaw === FARM_SOIL_SEEDED || hitRaw === CARROT_SEEDED || hitRaw === POTATO_SEEDED) {
      ws.send(JSON.stringify({ t: "fertilize", x: target.bx, y: target.by, z: target.bz }));
      return { x: target.bx, y: target.by, z: target.bz };
    }
    return null;
  }
  // 水桶 v1（自主提案切片）：手持空水桶對準水源 → 舀水（後端權威複驗目標為來源水）。
  //   複用釣竿同款 isWaterId 判定（來源水＋流動水），非水面靜默忽略；後端只認來源水才真的舀。
  if (selectedBlock() === BUCKET) {
    if (isWaterId(getRaw(target.bx, target.by, target.bz))) {
      ws.send(JSON.stringify({ t: "bucket_fill", x: target.bx, y: target.by, z: target.bz }));
    }
    return null;
  }
  // 水桶 v1：手持滿水桶 → 在命中方塊的面外側倒出一格永久來源水（比照一般放置的面偏移）。
  if (selectedBlock() === WATER_BUCKET) {
    const px = target.bx + target.nx, py = target.by + target.ny, pz = target.bz + target.nz;
    // 別把水倒進自己身體（避免整個人泡在水裡的突兀感；後端仍會權威複驗目標可倒）。
    if (px === Math.floor(player.x) && pz === Math.floor(player.z) &&
        (py === Math.floor(player.y) || py === Math.floor(player.y + 1))) return null;
    ws.send(JSON.stringify({ t: "bucket_pour", x: px, y: py, z: pz }));
    return { x: px, y: py, z: pz };
  }
  // 漂流瓶 v1（自主提案切片 825）：手持空玻璃瓶對準水面 → 跳出輸入框寫一句瓶中信丟下去。
  //   複用同款 isWaterId 判定；非水面靜默忽略（後端仍會權威複驗目標＋登入身分＋真的持有瓶子）。
  if (selectedBlock() === BOTTLE) {
    if (isWaterId(getRaw(target.bx, target.by, target.bz))) {
      const text = window.prompt("瓶中信要寫什麼？（最多 60 字，寫給某位路過水邊的陌生旅人）", "");
      if (text !== null && text.trim() !== "") {
        ws.send(JSON.stringify({ t: "throw_bottle", x: target.bx, y: target.by, z: target.bz, text }));
      }
    }
    return null;
  }
  // 鋤頭 v1（自主提案切片）：手持鋤頭對準草地／泥土 → 就地開墾成農田土。目標是命中方塊本身
  //   （不偏移到面外側，比照沃肥）；非草／土靜默忽略——後端仍會權威複驗目標可鋤＋背包真持有鋤頭。
  if (selectedBlock() === HOE) {
    const hitRaw = getRaw(target.bx, target.by, target.bz);
    if (hitRaw === GRASS || hitRaw === DIRT) {
      ws.send(JSON.stringify({ t: "hoe_till", x: target.bx, y: target.by, z: target.bz }));
      return { x: target.bx, y: target.by, z: target.bz };
    }
    return null;
  }
  // 放養雞 v1（自主提案切片，ROADMAP 870）：手持小麥種子、準心對準一隻雞 → 優先「餵」
  // 而非「種」；沒對準雞就落到下面種田分支正常種下種子，不改變既有種田手感。
  if (selectedBlock() === SEEDS) {
    const pick = pickWildlife(window.innerWidth / 2, window.innerHeight / 2);
    if (pick) {
      ws.send(JSON.stringify({ t: "feed_chicken", id: pick.id }));
      return null;
    }
  }
  // 種子的特殊種植動作：目標是農田土本身（不偏移到面外側）。
  // 第二種作物 v1：胡蘿蔔種子選中時種下胡蘿蔔；第三種作物 v1：馬鈴薯種子選中時種下馬鈴薯，
  // 皆附帶 seed 欄位讓伺服器分辨作物種類。
  if (selectedBlock() === SEEDS || selectedBlock() === CARROT_SEEDS || selectedBlock() === POTATO_SEEDS) {
    const hitRaw = getRaw(target.bx, target.by, target.bz);
    if (hitRaw === FARM_SOIL) {
      const sel = selectedBlock();
      const seed = sel === CARROT_SEEDS ? CARROT_SEEDS : sel === POTATO_SEEDS ? POTATO_SEEDS : undefined;
      ws.send(JSON.stringify({ t: "plant", x: target.bx, y: target.by, z: target.bz, seed }));
      return { x: target.bx, y: target.by, z: target.bz };
    }
    // 種子只能種在農田土上——其他方塊靜默忽略。
    return null;
  }
  // 餵野兔馴服 v1（自主提案切片）：手持胡蘿蔔、準心對準一隻野兔 → 優先「餵」而非「吃」；
  // 沒對準野兔就落到下面 heldIsFood 分支正常吃掉，不改變既有吃食物手感。
  if (selectedBlock() === CARROT) {
    const pick = pickWildlife(window.innerWidth / 2, window.innerHeight / 2);
    if (pick) {
      ws.send(JSON.stringify({ t: "feed_wildlife", id: pick.id }));
      return null;
    }
  }
  // 手持食物時「放置」＝「吃」（玩家生存指標 v1·溫和版）：右鍵/放置鈕在手持食物時直接吃掉，
  // 回復飢餓（後端權威驗證）。順手：不用另外找按鈕，拿著就能吃。
  if (heldIsFood()) { tryEatDish(); return null; }
  // 麵包 v1（ROADMAP 668）+ 胡蘿蔔（第二種作物 v1）+ 馬鈴薯（第三種作物 v1）+ 漁獲（垂釣 v1）：純物品，不可放置——靜默忽略。
  if (selectedBlock() === WHEAT || selectedBlock() === BREAD || selectedBlock() === CARROT || selectedBlock() === POTATO
      || selectedBlock() === FISH || selectedBlock() === AETHER_FISH) return null;
  // 一般放置：在命中方塊的面外側放置。
  const px = target.bx + target.nx, py = target.by + target.ny, pz = target.bz + target.nz;
  // 別把方塊放進自己身體（避免卡死）。
  if (px === Math.floor(player.x) && pz === Math.floor(player.z) &&
      (py === Math.floor(player.y) || py === Math.floor(player.y + 1))) return null;
  ws.send(JSON.stringify({ t: "place", x: px, y: py, z: pz, b: selectedBlock() }));
  // 告示牌 v1（ROADMAP 740）：剛放下一塊新牌子 → 立刻讓玩家寫上文字。
  // place 與 sign_set 走同一條 socket、伺服器循序處理，故 sign_set 到達時牌子已立好。
  if (selectedBlock() === SIGN) {
    promptSignEdit(px, py, pz, true);
  }
  return { x: px, y: py, z: pz };
}

// 告示牌 v1（ROADMAP 740）：跳出輸入框讓玩家寫／改牌面文字，送 sign_set 給伺服器。
// isNew=true 時預設空白（剛放下的新牌）；否則帶入目前牌面文字供編輯。取消（null）不送。
function promptSignEdit(x, y, z, isNew) {
  const key = x + "," + y + "," + z;
  const cur = isNew ? "" : (signTexts.get(key) || "");
  const input = window.prompt("告示牌文字（最多 30 字，留空可清除）：", cur);
  if (input === null) return;
  ws.send(JSON.stringify({ t: "sign_set", x, y, z, text: input }));
}

// ── 輸入 ───────────────────────────────────────────────────────────────────
const keys = {};
addEventListener("keydown", (e) => {
  if (e.target && e.target.tagName === "INPUT") return; // 對話輸入中不觸發移動
  keys[e.code] = true; if (e.code === "Space") e.preventDefault();
  // 麥塊式快捷鍵：F5 切第一/三人稱、E 開/關背包（開背包會釋放滑鼠鎖定）。
  if (e.code === "F5") { e.preventDefault(); toggleViewMode(); }
  if (e.code === "KeyE") {
    e.preventDefault();
    if (bagPanelVisible()) closeBagPanel(); else openBagPanel();
  }
  // 掉落物 v1（自主提案切片 828）：Q 對準地面丟下一份目前手上選取的材料。
  if (e.code === "KeyQ") { e.preventDefault(); dropSelectedItem(); }
  if (e.code === "KeyM") { e.preventDefault(); openStallAtTarget(); }
  // Esc：關操作設定面板（也讓瀏覽器解除滑鼠鎖定，兩者不衝突）。
  if (e.code === "Escape" && settingsPanelVisible()) closeSettingsPanel();
  // Esc：也收起 ☰ 主選單抽屜（若正開著）。
  if (e.code === "Escape" && typeof closeMenuDrawer === "function") closeMenuDrawer();
});
addEventListener("keyup", (e) => { keys[e.code] = false; });

// ── 視角模式 + 俯仰 ─────────────────────────────────────────────────────────
// camPitch：視線俯仰（0=水平、正=往下看、負=往上看），yaw+pitch 純前端相機視角，
// 後端玩家只有 yaw（pitch 不上傳、不影響移動/面向）。
let camPitch = 0.35;
// 視角模式：'first'=第一人稱（相機在眼睛、藏身體）、'third'=第三人稱（後上方跟隨、看得到身體）。
// 預設：由操作設定的 viewDefault 決定（合理預設＝桌機第一、手機第三，可在設定面板改）。
let viewMode = settings.viewDefault;
// 俯仰上下限：第一人稱可近乎正負直視（±83°）；第三人稱維持較窄，避免鏡頭鑽進地面。
function pitchLimits() { return viewMode === "first" ? [-1.45, 1.45] : [-0.2, 1.3]; }
function clampPitch() { const [lo, hi] = pitchLimits(); camPitch = Math.max(lo, Math.min(hi, camPitch)); }

// 切換第一/三人稱（F5 或 👁 鈕）：重新夾俯仰到該模式範圍、切身體可見性、更新鈕字。
function toggleViewMode() {
  viewMode = (viewMode === "first") ? "third" : "first";
  clampPitch();
  bodyMesh.visible = (viewMode !== "first"); // 第一人稱藏自己身體（每幀也會再設一次，冪等）
  const vb = document.getElementById("viewBtn");
  if (vb) vb.textContent = (viewMode === "first") ? "👁 第一人稱" : "🧍 第三人稱";
}

// 桌機滑鼠鎖定（pointer lock）：點畫面進入、Esc 離開（麥塊桌機手感）。
let pointerLocked = false;
const MOUSE_SENS = 0.0022;
// 有面板/對話開著時不進滑鼠鎖定（那些需要游標操作）。
function anyPanelOpen() {
  return bagPanelVisible() || wbPanelVisible() || furnacePanelVisible() || chestPanelVisible() ||
         settingsPanelVisible() ||
         (chatEl && chatEl.style.display === "flex");
}
// 釋放滑鼠鎖定（開面板/對話時呼叫，讓游標回來能點格子/打字）。
function releaseMouse() { if (pointerLocked) { try { document.exitPointerLock(); } catch (e) {} } }
document.addEventListener("pointerlockchange", () => {
  pointerLocked = (document.pointerLockElement === renderer.domElement);
});

if (!isTouch) {
  // 桌機：麥塊式滑鼠鎖定視角。沒鎖定時點畫面＝進入視角（此下不當破壞）；
  // 鎖定中：mousemove 轉 yaw+pitch、左鍵挖/點居民對話、右鍵放置，準心固定螢幕中心。
  renderer.domElement.addEventListener("mousedown", (e) => {
    if (!pointerLocked) {
      if (!anyPanelOpen()) renderer.domElement.requestPointerLock();
      return;
    }
    if (e.button === 2) { placeAtTarget(); return; }
    if (e.button !== 0) return;
    isMouseDown = true;
    // 鎖定中游標藏在螢幕中心 → 用中心點做居民 raycast / 破壞。
    const cx = window.innerWidth / 2, cy = window.innerHeight / 2;
    const rid = pickResident(cx, cy);
    if (rid) {
      const ent = residents.get(rid);
      openChat(rid, ent && ent.lastName);
    } else if (tryPetInteract(cx, cy)) {
      // 寵物互動：準心對到自己馴服的小夥伴 → 取名（895）或安置／召回（898）；優先於挖擊/採礦，不挖牠背後的地。
      isMouseDown = false;
    } else {
      // 暗影生物 v1：準心對到暗影 → 挖擊它（送 shadow_hit；打不打得到由伺服器權威複驗）。
      const sid = pickShadow(cx, cy);
      if (sid !== null) { ws.send(JSON.stringify({ t: "shadow_hit", id: sid })); return; }
      startMining(); // 採礦手感 v1：開始計時挖掘，而非立即 break
    }
  });
  document.addEventListener("mouseup", (e) => {
    if (e.button === 0) { isMouseDown = false; cancelMining(); }
  });
  document.addEventListener("mousemove", (e) => {
    if (!pointerLocked) return;
    const sens = MOUSE_SENS * settings.sensitivity; // 靈敏度設定即時生效
    player.yaw -= e.movementX * sens;
    camPitch += e.movementY * sens; // 滑鼠往下＝往下看（往下 movementY>0 → pitch 增）
    clampPitch();
  });
  // 右鍵放置：擋掉瀏覽器選單。
  renderer.domElement.addEventListener("contextmenu", (e) => e.preventDefault());
}

// 觸控搖桿（isTouch 常數已在頁首定義）
const touchEl = document.getElementById("touch");
let joyVec = { x: 0, y: 0 };
// 準心+按鈕模式的挖掘狀態（挖鈕按住期間維持）＋準心是否對到居民（挖鈕切「說話」用）。
let touchDigHeld = false;
let crosshairResident = null; // rid 或 null：準心對到的居民（每幀節流更新）
if (isTouch) {
  if (touchEl) touchEl.style.display = "block";
  // 標記觸控裝置：CSS 可據此把左下聊天窗抬到搖桿之上（橫式尤其重要，見 index.html）。
  document.body.classList.add("touch");
  const joy = document.getElementById("joy"), nub = document.getElementById("joyNub");
  let joyId = null, jcx = 0, jcy = 0;
  joy.addEventListener("touchstart", (e) => {
    const t = e.changedTouches[0]; joyId = t.identifier;
    const r = joy.getBoundingClientRect(); jcx = r.left + r.width / 2; jcy = r.top + r.height / 2;
    e.preventDefault();
  }, { passive: false });
  addEventListener("touchmove", (e) => {
    for (const t of e.changedTouches) {
      if (t.identifier !== joyId) continue;
      let dx = (t.clientX - jcx) / 50, dy = (t.clientY - jcy) / 50;
      dx = Math.max(-1, Math.min(1, dx)); dy = Math.max(-1, Math.min(1, dy));
      joyVec.x = dx; joyVec.y = dy;
      nub.style.left = (35 + dx * 30) + "px"; nub.style.top = (35 + dy * 30) + "px";
    }
  }, { passive: false });
  addEventListener("touchend", (e) => {
    for (const t of e.changedTouches) if (t.identifier === joyId) { joyId = null; joyVec = { x: 0, y: 0 }; nub.style.left = "35px"; nub.style.top = "35px"; }
  });
  // 視角轉動：在非搖桿區拖曳。靈敏度吃設定倍率。
  //   準心+按鈕模式：拖曳「只轉視角」，絕不觸發挖/放（防誤觸主打）——但輕點居民仍可開對話（無害、不動世界）。
  //   點擊互動模式（舊）：輕點世界＝挖（點到居民＝對話），沿用 MCPE 點破壞範式。
  let camId = null, cx0 = 0, cy0 = 0, camMoved = 0;
  renderer.domElement.addEventListener("touchstart", (e) => {
    const t = e.changedTouches[0]; camId = t.identifier; cx0 = t.clientX; cy0 = t.clientY; camMoved = 0;
  });
  renderer.domElement.addEventListener("touchmove", (e) => {
    for (const t of e.changedTouches) {
      if (t.identifier !== camId) continue;
      camMoved += Math.abs(t.clientX - cx0) + Math.abs(t.clientY - cy0);
      const sens = 0.006 * settings.sensitivity; // 觸控拖曳靈敏度吃設定倍率
      player.yaw -= (t.clientX - cx0) * sens;
      camPitch += (t.clientY - cy0) * sens; clampPitch(); // 夾到目前模式的俯仰範圍
      cx0 = t.clientX; cy0 = t.clientY;
    }
  }, { passive: false });
  renderer.domElement.addEventListener("touchend", (e) => {
    for (const t of e.changedTouches) {
      if (t.identifier !== camId) continue;
      if (camMoved < 8) {
        // 輕點：先看是否點到居民（開對話，兩種模式皆可——不動世界、無誤觸風險）。
        const rid = pickResident(t.clientX, t.clientY);
        if (rid) { const ent = residents.get(rid); openChat(rid, ent && ent.lastName); }
        else if (tryPetInteract(t.clientX, t.clientY)) {
          // 寵物互動：輕點到自己馴服的小夥伴 → 取名（895）或安置／召回（898），像點居民一樣直覺，不動世界。
        }
        else {
          // 暗影生物 v1：輕點到暗影 → 挖擊（兩種觸控模式皆可——像點居民一樣直覺、無誤觸風險）。
          const sid = pickShadow(t.clientX, t.clientY);
          if (sid !== null) ws.send(JSON.stringify({ t: "shadow_hit", id: sid }));
          // 點世界＝挖：只在「點擊互動」模式生效；準心+按鈕模式拖曳/輕點都不挖（改按挖鈕）。
          else if (settings.touchMode === "tap") breakAtTarget();
        }
      }
      camId = null;
    }
  });
  const jumpBtn = document.getElementById("jump");
  jumpBtn.addEventListener("touchstart", (e) => { tryJump(); e.preventDefault(); }, { passive: false });
  const placeBtn = document.getElementById("place");
  placeBtn.addEventListener("touchstart", (e) => { placeAtTarget(); e.preventDefault(); }, { passive: false });
  // 專屬「挖」按鈕（準心+按鈕模式核心）：長按對準心方塊計時挖掘（進度條），鬆手取消。
  //   若準心對到居民 → 改為互動（開對話），不挖（挖/互動分離、絕不混淆）。
  const digBtn = document.getElementById("dig");
  if (digBtn) {
    digBtn.addEventListener("touchstart", (e) => {
      e.preventDefault();
      if (crosshairResident) {
        const ent = residents.get(crosshairResident);
        openChat(crosshairResident, ent && ent.lastName);
        return;
      }
      touchDigHeld = true;
      startMining(); // 與桌機共用計時挖掘手感（進度滿才 break）
    }, { passive: false });
    const endDig = (e) => { touchDigHeld = false; cancelMining(); if (e) e.preventDefault(); };
    digBtn.addEventListener("touchend", endDig, { passive: false });
    digBtn.addEventListener("touchcancel", endDig, { passive: false });
  }
}

function tryJump() { if (player.grounded) { player.vy = 8.2; player.grounded = false; } }

// ═══════════════════════════════════════════════════════════════════════════
// 操作設定面板 + 手把（gamepad）v1
//   全部純前端顯示/手感層：改視角靈敏度、觸控按鈕外觀/擺位、慣用手、自動跳、預設人稱、
//   觸控模式（準心+按鈕 / 點擊互動），並用瀏覽器 Gamepad API 支援手把。不動遊戲規則/後端。
// ═══════════════════════════════════════════════════════════════════════════

// ── 套用觸控外觀（大小/透明度/慣用手）到 CSS 變數 + body class ──
// 用 CSS 變數與 class 驅動排版，避免每次都手改多個 inline style（也讓媒體查詢仍生效）。
function applyTouchStyle() {
  const root = document.documentElement;
  root.style.setProperty("--touch-btn-scale", String(settings.btnScale));
  root.style.setProperty("--touch-btn-opacity", String(settings.btnOpacity));
  // 慣用手：left 時把觸控層鏡像（搖桿↔按鈕左右對調），用 body class 切 CSS。
  document.body.classList.toggle("lefty", settings.handed === "left");
}

// ── 套用觸控操作模式（準心+按鈕 / 點擊互動）：只切「挖鈕」的顯示 ──
// 準心+按鈕模式才顯示專屬挖鈕；點擊互動模式隱藏挖鈕（改為輕點世界挖）。
function applyTouchMode() {
  const digBtn = document.getElementById("dig");
  if (digBtn) digBtn.style.display = (isTouch && settings.touchMode === "crosshair") ? "flex" : "none";
}

// ── 每幀（節流）更新準心對到的居民：準心+按鈕模式下讓挖鈕切成「說話」──
let _crosshairScanTimer = 0;
function updateCrosshairResident(dt) {
  if (!isTouch || settings.touchMode !== "crosshair") { crosshairResident = null; return; }
  _crosshairScanTimer -= dt;
  if (_crosshairScanTimer > 0) return;
  _crosshairScanTimer = 0.15; // 節流：每 0.15 秒掃一次（省 raycast，不影響手感）
  crosshairResident = pickResident(window.innerWidth / 2, window.innerHeight / 2);
  const digBtn = document.getElementById("dig");
  if (digBtn) {
    // 對到居民 → 顯示「💬 說話」；否則回復「⛏ 挖」。
    digBtn.textContent = crosshairResident ? "💬 說話" : "⛏ 挖";
    digBtn.classList.toggle("talk", !!crosshairResident);
  }
}

// ── 手把（Gamepad）v1：輪詢在既有 rAF 迴圈裡（不另開迴圈）──
// Xbox 佈局：A(0)跳、B(1)放、X(2)挖、Y(3)背包、LB(4)/RB(5)切快捷欄、LT(6)放/RT(7)挖；
// 左搖桿移動、右搖桿轉視角。偵測到手把即自動生效。核心 A/B 完整，其餘加分。
let gamepadConnected = false;
let gamepadName = "";
let gamepadDigHeld = false;       // 手把挖鍵是否按住（給 tickMining 續挖）
let gpMove = { x: 0, y: 0 };      // 手把左搖桿移動向量（餵進 update 的移動合成）
const _gpPrevBtn = [];            // 上一幀各按鈕是否按下（做邊緣偵測，避免連發）
const GP_DEADZONE = 0.18;
const GP_LOOK = 2.6;              // 右搖桿轉視角速率（rad/秒基準，再乘靈敏度）
function _gpAxis(v) { return Math.abs(v) < GP_DEADZONE ? 0 : v; } // 死區處理
function pollGamepad(dt) {
  let pads = [];
  try { pads = navigator.getGamepads ? navigator.getGamepads() : []; } catch (_) { pads = []; }
  let gp = null;
  for (const p of pads) if (p && p.connected) { gp = p; break; }
  if (!gp) { gamepadConnected = false; gpMove.x = 0; gpMove.y = 0; gamepadDigHeld = false; return; }
  gamepadConnected = true; gamepadName = gp.id || "手把";
  // 左搖桿 → 移動（x 右正、y 下正；update 內 -y 當前進，與觸控搖桿一致）
  gpMove.x = _gpAxis(gp.axes[0] || 0);
  gpMove.y = _gpAxis(gp.axes[1] || 0);
  // 右搖桿 → 轉視角（吃靈敏度倍率）
  const rx = _gpAxis(gp.axes[2] || 0), ry = _gpAxis(gp.axes[3] || 0);
  if (rx) player.yaw -= rx * GP_LOOK * dt * settings.sensitivity;
  if (ry) { camPitch += ry * GP_LOOK * dt * settings.sensitivity; clampPitch(); }
  // 按鈕：邊緣偵測（這幀按下、上幀沒按）→ 觸發一次。
  const btn = (i) => !!(gp.buttons[i] && gp.buttons[i].pressed);
  const pressed = (i) => btn(i) && !_gpPrevBtn[i];
  if (pressed(0)) tryJump();                                  // A：跳
  if (pressed(1) || pressed(6)) placeAtTarget();              // B / LT：放置
  if (pressed(3)) { if (bagPanelVisible()) closeBagPanel(); else openBagPanel(); } // Y：背包
  if (pressed(4)) selectSlot(selectedSlot - 1);              // LB：上一格
  if (pressed(5)) selectSlot(selectedSlot + 1);              // RB：下一格
  // X / RT：挖（按住續挖，與觸控挖鈕/滑鼠左鍵共用計時）。準心對到居民 → 改互動。
  const digNow = btn(2) || btn(7);
  if (digNow && !gamepadDigHeld) {
    const rid = pickResident(window.innerWidth / 2, window.innerHeight / 2);
    if (rid) { const ent = residents.get(rid); openChat(rid, ent && ent.lastName); }
    else { gamepadDigHeld = true; startMining(); }
  } else if (!digNow && gamepadDigHeld) {
    gamepadDigHeld = false; cancelMining();
  }
  // 記錄本幀按鈕狀態供下幀邊緣偵測。
  for (let i = 0; i < gp.buttons.length; i++) _gpPrevBtn[i] = btn(i);
}
addEventListener("gamepadconnected", (e) => {
  gamepadConnected = true; gamepadName = (e.gamepad && e.gamepad.id) || "手把";
  showMsg("🎮 已連手把：" + gamepadName);
  updateSettingsGamepadStatus();
});
addEventListener("gamepaddisconnected", () => {
  gamepadConnected = false; updateSettingsGamepadStatus();
});

// ── 設定面板 DOM 綁定 ──
const gearBtn = document.getElementById("gearBtn");
const settingsPanelEl = document.getElementById("settingsPanel");
function settingsPanelVisible() { return settingsPanelEl && settingsPanelEl.style.display === "flex"; }
function openSettingsPanel() {
  if (!settingsPanelEl) return;
  syncSettingsPanelUI();
  updateSettingsGamepadStatus();
  settingsPanelEl.style.display = "flex";
  releaseMouse(); // 開面板時鬆開滑鼠鎖定，游標可操作控制項
}
function closeSettingsPanel() { if (settingsPanelEl) settingsPanelEl.style.display = "none"; }
function updateSettingsGamepadStatus() {
  const el = document.getElementById("setGamepadStatus");
  if (!el) return;
  el.textContent = gamepadConnected ? ("已連手把：" + gamepadName) : "未偵測到手把";
}
// 把 settings 現值灌回面板控制項（開面板時呼叫，確保顯示與實際一致）。
function syncSettingsPanelUI() {
  const q = (id) => document.getElementById(id);
  const mode = q("setTouchMode"); if (mode) mode.value = settings.touchMode;
  const sens = q("setSensitivity"); if (sens) sens.value = String(settings.sensitivity);
  const sensV = q("setSensitivityVal"); if (sensV) sensV.textContent = settings.sensitivity.toFixed(2) + "×";
  const size = q("setBtnScale"); if (size) size.value = String(settings.btnScale);
  const sizeV = q("setBtnScaleVal"); if (sizeV) sizeV.textContent = settings.btnScale.toFixed(2) + "×";
  const op = q("setBtnOpacity"); if (op) op.value = String(settings.btnOpacity);
  const opV = q("setBtnOpacityVal"); if (opV) opV.textContent = Math.round(settings.btnOpacity * 100) + "%";
  const handed = q("setHanded"); if (handed) handed.value = settings.handed;
  const aj = q("setAutoJump"); if (aj) aj.checked = settings.autoJump;
  const vd = q("setViewDefault"); if (vd) vd.value = settings.viewDefault;
}
if (gearBtn) gearBtn.addEventListener("click", () => { if (settingsPanelVisible()) closeSettingsPanel(); else openSettingsPanel(); });

// ── ☰ 主選單抽屜（UI 響應式整理）──────────────────────────────────────────────
// 右側一排功能鈕（動態/日記牆/羅盤/交情/技能/成就）＋人稱/操作設定全收進抽屜，
// 常駐畫面只留最常用（背包/說話/挖/放置）。所有鈕仍是原本的 DOM 元素、原本的
// 事件監聽器照舊生效——這裡只管抽屜的開合，不改任何功能行為。
const menuBtnEl = document.getElementById("menuBtn");
const menuDrawerEl = document.getElementById("menuDrawer");
function menuDrawerOpen() { return menuDrawerEl && menuDrawerEl.classList.contains("open"); }
function openMenuDrawer() {
  if (!menuDrawerEl) return;
  menuDrawerEl.classList.add("open");
  if (menuBtnEl) menuBtnEl.classList.add("open");
}
function closeMenuDrawer() {
  if (menuDrawerEl) menuDrawerEl.classList.remove("open");
  if (menuBtnEl) menuBtnEl.classList.remove("open");
}
if (menuBtnEl) {
  menuBtnEl.addEventListener("click", (e) => {
    e.stopPropagation();
    menuDrawerOpen() ? closeMenuDrawer() : openMenuDrawer();
  });
}
if (menuDrawerEl) {
  // 點抽屜內任一功能鈕後收起抽屜——它開的面板（z-index 20）就不會被抽屜（z-index 21）擋住。
  // 各鈕自身的開面板/切人稱監聽器照樣先觸發，這裡只負責關抽屜。
  menuDrawerEl.addEventListener("click", (e) => {
    const item = e.target.closest("#feedBtn, #diaryWallBtn, #compassBtn, #relationsBtn, #skillsBtn, #milestonesBtn, #mapBtn, #discBtn, #masteryBtn, #viewBtn, #gearBtn");
    if (item) closeMenuDrawer();
  });
}
// 點抽屜與 ☰ 鈕以外的地方 → 收起抽屜（麥塊/一般選單直覺）。
document.addEventListener("click", (e) => {
  if (!menuDrawerOpen()) return;
  if (menuDrawerEl && menuDrawerEl.contains(e.target)) return;
  if (menuBtnEl && menuBtnEl.contains(e.target)) return;
  closeMenuDrawer();
});
{
  const q = (id) => document.getElementById(id);
  const closeBtn = q("settingsClose");
  if (closeBtn) closeBtn.addEventListener("click", closeSettingsPanel);
  const mode = q("setTouchMode");
  if (mode) mode.addEventListener("change", () => { settings.touchMode = mode.value === "tap" ? "tap" : "crosshair"; saveSettings(); applyTouchMode(); });
  const sens = q("setSensitivity");
  if (sens) sens.addEventListener("input", () => {
    settings.sensitivity = Math.max(0.3, Math.min(2.5, parseFloat(sens.value) || 1));
    const v = q("setSensitivityVal"); if (v) v.textContent = settings.sensitivity.toFixed(2) + "×";
    saveSettings();
  });
  const size = q("setBtnScale");
  if (size) size.addEventListener("input", () => {
    settings.btnScale = Math.max(0.7, Math.min(1.6, parseFloat(size.value) || 1));
    const v = q("setBtnScaleVal"); if (v) v.textContent = settings.btnScale.toFixed(2) + "×";
    applyTouchStyle(); saveSettings();
  });
  const op = q("setBtnOpacity");
  if (op) op.addEventListener("input", () => {
    settings.btnOpacity = Math.max(0.35, Math.min(1.0, parseFloat(op.value) || 1));
    const v = q("setBtnOpacityVal"); if (v) v.textContent = Math.round(settings.btnOpacity * 100) + "%";
    applyTouchStyle(); saveSettings();
  });
  const handed = q("setHanded");
  if (handed) handed.addEventListener("change", () => { settings.handed = handed.value === "left" ? "left" : "right"; applyTouchStyle(); saveSettings(); });
  const aj = q("setAutoJump");
  if (aj) aj.addEventListener("change", () => { settings.autoJump = !!aj.checked; saveSettings(); });
  const vd = q("setViewDefault");
  if (vd) vd.addEventListener("change", () => {
    settings.viewDefault = vd.value === "third" ? "third" : "first";
    // 立即套用到目前人稱（順手切換，所見即所得），並存檔當下次預設。
    if (viewMode !== settings.viewDefault) toggleViewMode();
    saveSettings();
  });
}
// 開頁即套用一次觸控外觀/模式（讓存檔的大小/透明度/慣用手/模式立刻生效）。
applyTouchStyle();
applyTouchMode();

// ── WebSocket（/voxel/ws）─────────────────────────────────────────────────
// 無痛自動重連：部署重啟（~2 秒）玩家幾乎無感，指數退避，安靜期不顯示嚇人橫幅。
let ws = null, wsReady = false;
let wsRetryDelay = 300;       // 指數退避首次延遲(ms)；每次斷線加倍，上限 8s
const WS_RETRY_MAX = 8000;    // 最大退避上限
let wsRetryTimer = null;      // 重連排程 handle（避免同時多個重連 timer）
let wsBannerTimer = null;     // 安靜期橫幅延遲 handle（斷線後 3s 才顯示大橫幅）
let wsIsReconnect = false;    // 標記此次 connect() 為重連（非首次進場）
let wsSavedPos = null;        // 斷線前玩家位置，重連後恢復（不被 server spawn 蓋掉）
function connect() {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  ws = new WebSocket(`${proto}://${location.host}/voxel/ws`);
  ws.onopen = () => {
    wsReady = true;
    wsRetryDelay = 300;  // 重置退避計時器
    // 取消安靜期橫幅排程（若在 3s 安靜期內重連成功，橫幅永遠不出現）
    if (wsBannerTimer) { clearTimeout(wsBannerTimer); wsBannerTimer = null; }
    // 若先前已顯示「重新連線中」提示，重連成功後立即隱藏
    if (errEl) errEl.style.display = "none";
    // 入場名：登入者用帳號名（伺服器仍以 cookie 為準覆蓋，這裡只是讓 HUD 先正確），
    // 訪客用 localStorage 暫存名。身份綁定的真相在後端 resolve_identity。
    let nm = "旅人";
    try { nm = (myAccountName || localStorage.getItem("butfun_name") || "旅人"); } catch (e) { nm = myAccountName || "旅人"; }
    ws.send(JSON.stringify({ t: "join", name: nm }));
  };
  ws.onmessage = (ev) => {
    let m; try { m = JSON.parse(ev.data); } catch (e) { return; }
    if (m.t === "welcome") {
      myId = m.id; myName = m.name || "旅人";
      // 帶稱號的維護者回歸 → 只給他看的溫暖招呼（後端 title 字串判定，引夢使者/築夢工匠…）。
      if (m.title) appendMsg("sys", "✦ " + m.title + "，你回來了——居民們一直記得你。");
      // 重連後恢復斷線前位置，不讓玩家被傳送回出生點；首次連線才套用伺服器 spawn。
      // 濫用防護：重連仍走既有 join 身分驗證（cookie 路徑），wsSavedPos 只影響前端位置，無新對外面。
      if (wsIsReconnect && wsSavedPos) {
        player.x = wsSavedPos.x; player.y = wsSavedPos.y; player.z = wsSavedPos.z;
      } else {
        player.x = m.spawn.x; player.y = m.spawn.y; player.z = m.spawn.z;
      }
      wsIsReconnect = false;
      wsSavedPos = null;
      // 出生瞬間先脫困一次（若出生 chunk 已到、地表把人埋住，立刻頂出來）。
      unstuckIfNeeded();
      // 好感度 v1：連線後立即拉取與各居民的好感度，讓指示燈盡快亮起。
      refreshAffinity();
      // 居民教你一道獨門配方 v1（自主提案切片 849）：連線後拉一次已學會的獨門配方。
      refreshKnownRecipes();
      // 個人路標 v1（自主提案切片 869）：連線後拉一次現有路標，供羅盤面板隨時顯示。
      refreshWaypoints();
    } else if (m.t === "chunks") {
      for (const c of m.chunks) {
        const key = ckey(c.cx, c.cy, c.cz);
        chunks.set(key, b64ToBytes(c.data));
        markDirty(c.cx, c.cy, c.cz);
        scanChunkForTorches(c.cx, c.cy, c.cz); // 火把發光 v1：掃描新 chunk 的既有火把
      }
      // chunk 載入後立刻脫困：若新載入的方塊把玩家埋住（出生／chunk 邊緣／下落最常見），
      // 同一則訊息就把人頂出來，不必等下一幀。沒卡則零成本早退。
      unstuckIfNeeded();
    } else if (m.t === "block") {
      // 伺服器權威方塊更新（破壞/放置）：本地套用 + 只重建受影響 chunk。
      setLocalBlock(m.x, m.y, m.z, m.b);
    } else if (m.t === "players") {
      const seen = new Set();
      for (const p of m.players) {
        if (p.id === myId) continue;
        seen.add(p.id);
        let ent = others.get(p.id);
        if (!ent) {
          // 其他玩家也是方塊小人（藍色系）；mesh＝avatar 的 group，沿用既有 position/rotation/add 邏輯。
          const av = attachHeldItem(buildAvatar(OTHER_PALETTE.body, OTHER_PALETTE.head, OTHER_PALETTE.limb));
          const mesh = av.group; scene.add(mesh);
          // 頭上對話泡泡（child of mesh，sprite 永遠面向鏡頭、不受 mesh 旋轉影響）。
          const bubble = makeTextSprite("", true);
          bubble.position.y = PH / 2 + 1.7; // mesh 原點在身體中心，泡泡浮到頭頂上方
          bubble.visible = false;
          mesh.add(bubble);
          // lastMoveT：最近一次位置有變化的時間戳，供 render 迴圈判斷「是否在走路」→ 擺手腳。
          ent = { mesh, av, bubble, lastSay: "", titleText: null, title: null, lastMoveT: 0 };
          others.set(p.id, ent);
        }
        // 特殊身分稱號牌：後端 title 字串（引夢使者/築夢工匠…）為真才掛（不信客戶端自報）；
        // 一般玩家 / 訪客 title 為 null，沒有名牌，不受影響。稱號變更即重建（換稱號也跟得上）。
        const ptitle = p.title || null;
        if (ptitle !== ent.titleText) {
          if (ent.title) { ent.mesh.remove(ent.title); ent.title = null; }
          if (ptitle) {
            ent.title = makeTitleSprite(ptitle);
            ent.mesh.add(ent.title);
          }
          ent.titleText = ptitle;
        }
        // 手持工具可見 v1：伺服器廣播的 held（純視覺 cosmetic，見協定註解）換色/顯隱。
        setHeldItem(ent.av, p.held || null);
        // 位置有明顯變化 → 記下時間戳，讓 render 迴圈判定「在走路」而擺手腳（快照間也持續動）。
        const moved = Math.hypot(p.x - ent.mesh.position.x, p.z - ent.mesh.position.z);
        if (moved > 0.002) ent.lastMoveT = performance.now();
        ent.mesh.position.set(p.x, p.y + PH / 2, p.z);
        ent.mesh.rotation.y = p.yaw || 0;
        // embodied：別人說話 → 頭上冒泡（你走過會「聽到」別人在聊，世界有人聲）。
        const say = p.say || "";
        if (say !== ent.lastSay) {
          ent.lastSay = say;
          if (say) {
            setSpriteText(ent.bubble, say, true); ent.bubble.visible = true;
            chatLogAppend("other", p.name || "旅人", say, p.id); // 其他玩家說話也進聊天窗
          }
          else { ent.bubble.visible = false; }
        }
      }
      for (const [id, ent] of others) if (!seen.has(id)) { scene.remove(ent.mesh); others.delete(id); }
      // 乙太方界 AI 居民（與玩家分開的陣列）：位置/名字/說的話。
      if (m.residents) updateResidents(m.residents);
      // 野兔 v1（自主提案切片，ROADMAP 847）：世界第一種環境生物，純位置/朝向。
      if (m.wildlife) updateWildlife(m.wildlife);
      // 暗影生物 v1：夜間暗處的漂浮小靈（伺服器權威；白天陣列為空自然全移除）。
      if (m.shadows) updateShadows(m.shadows);
      // 晝夜循環 v1：伺服器每幀帶 time_of_day(0.0–1.0)，前端據此更新天空/光照。
      // 下雨天氣 v1（ROADMAP 700）：raining 隨同一份快照送達，一併觸發天空重繪。
      let skyDirty = false;
      if (typeof m.time_of_day === "number") { worldTime = m.time_of_day; skyDirty = true; }
      if (typeof m.raining === "boolean" && m.raining !== isRaining) { isRaining = m.raining; skyDirty = true; }
      // 雨後彩虹 v1（ROADMAP 780）：rainbow 隨同一份快照送達，切換前端彩虹弧的淡入/淡出目標。
      if (typeof m.rainbow === "boolean") rainbowActive = m.rainbow;
      // 流星許願 v1（ROADMAP 904）：meteor 隨同一份快照送達，updateMeteor 於旗標「假→真」上升緣播一道光痕。
      if (typeof m.meteor === "boolean") meteorActive = m.meteor;
      // 季節輪替 v1（ROADMAP 798）：season 隨同一份快照送達，換季時整片天地換上不同色調。
      // window.__qaFreezeSeason 僅供 QA 凍結季節（不被伺服器快照覆寫）以截四季樹葉對照圖用；正常遊玩恆為 undefined。
      if (!window.__qaFreezeSeason && typeof m.season === "string" && m.season !== worldSeason) { worldSeason = m.season; skyDirty = true; remeshForSeason(); /* 四季樹葉 v1：換季重建樹葉顏色 */ }
      // 季節指示器 v1（ROADMAP 897）：season_day（這一季第幾天）隨同一份快照送達，HUD 徽章顯示用。
      if (typeof m.season_day === "number") worldSeasonDay = m.season_day;
      if (skyDirty) updateSkyAndLight(worldTime);
    } else if (m.t === "talk") {
      // 居民對話回覆（單播）：
      //   thinking:true → 立即佔位（後端一收到就送），顯示動畫「思考中」指示器，不當一般氣泡。
      //   thinking 不存在（預設 false）→ LLM 真回覆，移除思考中指示器，顯示正常回覆氣泡。
      if (m.thinking) {
        showThinking(m.name); // 顯示「露娜 思考中 ●●●」動畫
      } else {
        removeThinking();     // 真回覆到了，先移除「思考中」
        lastTalkReply = m.reply || "";
        appendMsg("npc", (m.name || "居民") + "：" + lastTalkReply);
        // 居民 talk 回覆的完整版也進左下聊天窗（去重會併掉稍後那條截 40 字的頭上泡泡 say）。
        chatLogAppend("res", m.name || "居民", lastTalkReply, m.resident_id);
        // 好感度 v1：對話後更新好感度（後端可能已累積新記憶），讓指示燈即時升燈。
        refreshAffinity();
      }
    } else if (m.t === "inv_sync") {
      // 採集 v1：連線後收到背包全量快照，重置本地存量。
      myInv.clear();
      for (const [bid, cnt] of (m.items || [])) {
        if (cnt > 0) myInv.set(bid, cnt);
      }
      updateInvHud();
      updateGiftBtn(); // 贈禮 v1：背包恢復後同步更新按鈕
      updateEatBtn();  // 吃 v1：背包恢復後同步更新吃鈕
      refreshPlaceBtnLabel(); // 手持食物 → 放置鈕標「吃」
      updateFireworkBtn(); // 乙太煙火 v1（785）：背包變動同步更新施放鈕
    } else if (m.t === "inv_update") {
      // 採集 v1：單一材料增減後的新存量（伺服器回傳 total，非 delta）。
      if (m.count > 0) myInv.set(m.block_id, m.count);
      else myInv.delete(m.block_id);
      updateInvHud();
      updateGiftBtn(); // 贈禮 v1：材料變動後同步更新按鈕
      updateEatBtn();  // 吃 v1：材料變動後同步更新吃鈕
      refreshPlaceBtnLabel(); // 手持食物存量變動 → 更新放置/吃鈕標示
      updateFireworkBtn(); // 乙太煙火 v1（785）：背包變動同步更新施放鈕
      if (chestPanelVisible()) renderChestPanel(); // 箱子 v1：背包變動後同步更新箱子面板背包區
    } else if (m.t === "inv_denied") {
      // 採集 v1：放置材料不足，短暫提示。
      const bname = BLOCK_NAME[m.block_id] || "方塊";
      showErr("材料不足：" + bname + "（先去挖一些吧）");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "craft_ok") {
      // 合成成功（背包 2×2 或工作台 3×3）→ 清空對應格子 + 重繪面板。
      showMsg("合成成功：" + m.name_zh + " ×" + m.out_count + "！");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2200);
      bagGrid.fill(0); bagPick = 0;
      wbGrid.fill(0); wbPick = 0;
      if (bagPanelVisible()) renderBagPanel();
      if (wbPanelVisible()) renderWbPanel();
      if (furnacePanelVisible()) renderFurnacePanel();
    } else if (m.t === "craft_fail") {
      // 伺服器現在會依情況給不同 reason（材料不足／獨門配方未學會／工作台熔爐閘門等），
      // 過去這裡不論收到什麼一律顯示「材料不足」，讓其他 reason 文字被默默吞掉。
      showErr(m.reason || "材料不足，無法合成（先多採集一些）");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
      if (bagPanelVisible()) renderBagPanel();
      if (wbPanelVisible()) renderWbPanel();
      if (furnacePanelVisible()) renderFurnacePanel();
    } else if (m.t === "smelt_started") {
      // 熔爐煨煮 v1：熔爐冶煉不再瞬間——配料已入爐，過 m.secs 秒才熟成（回來自動入背包）。
      // 清空冶煉格 + 重繪面板（配料已被消耗，面板本就該歸零），提示先去忙別的。
      showMsg("🔥 " + (m.name_zh || "成品") + " 放進熔爐煨煮中…約 " + (m.secs || 0) + " 秒後熟成，先去忙別的吧");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2600);
      furnaceGrid.fill(0); furnacePick = 0;
      if (furnacePanelVisible()) renderFurnacePanel();
    } else if (m.t === "smelt_done") {
      // 熔爐煨煮 v1：某爐熟成——只有這爐的主人才顯示提示並更新背包（比照 return_gift 管線）。
      if (m.player === myName) {
        const iname = BLOCK_NAME[m.item_id] || m.item_name || "成品";
        if (m.count > 0) myInv.set(m.item_id, m.count);
        else myInv.delete(m.item_id);
        updateInvHud();
        updateGiftBtn();
        if (furnacePanelVisible()) renderFurnacePanel();
        showMsg("🔥 你的 " + iname + " ×" + (m.qty || 1) + " 煨好了！熱騰騰的，已放進背包");
        setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2600);
      }
    } else if (m.t === "plant_ok") {
      // 種田 v1 / 水耕農業 v1（ROADMAP 686）/ 第二/三種作物 v1：依作物種類 + 是否鄰近水源給不同提示。
      const plantMsg = m.carrot
        ? (m.irrigated ? "💧 水耕！胡蘿蔔將在 30 秒後成熟 🥕" : "已種下胡蘿蔔種子！等 60 秒就成熟 🥕")
        : m.potato
        ? (m.irrigated ? "💧 水耕！馬鈴薯將在 60 秒後成熟 🥔" : "已種下馬鈴薯種子！等 120 秒就成熟，收成量大 🥔")
        : (m.irrigated ? "💧 水耕！種子將在 45 秒後成熟 🌾" : "已種下種子！等 90 秒小麥就成熟 🌾");
      // 時令作物 v1（811）：種在該作物的時令季節時，後端附一句暖回饋（m.timely），接在種植提示後。
      showMsg(m.timely ? (plantMsg + "　" + m.timely) : plantMsg);
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, m.timely ? 3600 : 2500);
    } else if (m.t === "bounty") {
      // 時令豐收 v1（812）：在作物的時令季節收割成熟植株，額外多得一份果實——浮出當季鮮採回饋句
      //（背包已由 inv_update 更新）。與種植端的時令回饋（811）對成一對。
      showMsg(m.line || "🌾 當季鮮採，多收了一份！");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 3200);
    } else if (m.t === "plant_fail") {
      // 種田 v1：種植失敗（非農田土 / 沒種子 / 太遠），短暫提示。
      showErr("種植失敗：" + (m.reason || "未知原因"));
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "gift_ok") {
      // 贈禮 v1：送禮成功——居民道謝訊息顯示在對話框；更新贈禮鈕顯示。
      const iname = BLOCK_NAME[m.item_id] || m.item_name || "物品";
      appendMsg("sys", "✨ 你送出了 " + iname + " 給 " + (m.resident_name || "居民"));
      updateGiftBtn(); // 背包已更新，重算鈕
    } else if (m.t === "gift_fail") {
      // 贈禮 v1：送禮失敗（太遠 / 沒材料）。
      showErr(m.reason || "無法送禮");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "eat_ok") {
      // 吃東西回復飢餓 v1：熟食帶暖意回饋句（m.line）；生食沒有那份料理暖意，就給樸實的填飽提示。
      const iname = BLOCK_NAME[m.item_id] || m.item_name || "食物";
      showMsg(m.line ? ("🍲 " + m.line) : ("🍽 吃了" + iname + "，肚子暖了一點。"));
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 3000);
      updateEatBtn(); // 背包已由 inv_update 更新，重算吃鈕（吃完可能沒了）
      updateFireworkBtn(); // 乙太煙火 v1（785）：背包變動同步更新施放鈕
    } else if (m.t === "eat_fail") {
      // 吃 v1：吃不了（非食物 / 背包沒有 / 已飽）。
      showErr(m.reason || "現在沒法吃");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "player_stats") {
      // 玩家生存指標 v1（溫和版·後端權威）：更新快捷欄正上方的血/飢窄列。
      // 只有自己收得到（別人看不到你的條，減噪）。數值變化由 CSS width transition 平滑。
      updateStatsHud(m);
    } else if (m.t === "player_hurt") {
      // 受傷 v1：扣血時閃一層極輕微紅暈（別嚇人）。傷害量僅供視覺參考、血量以 player_stats 為準。
      flashDamage();
    } else if (m.t === "respawn") {
      // 溫柔重生 v1：血歸零 → 醒在村莊廣場/床邊。柔和淡出後把相機/預測位置拉回重生點，
      // 顯示一句溫暖提示；背包不掉落（後端保證）。
      doGentleRespawn(m.x, m.y, m.z, m.message);
    } else if (m.t === "hot_spring_enter") {
      // 溫泉遺跡 v1（自主提案切片）：剛踏進溫泉那一刻，浮出一句暖意提示（只給自己看）。
      showMsg(m.line || "暖流環繞全身，泡進溫泉舒服多了～");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2600);
    } else if (m.t === "shadow_puff") {
      // 暗影生物 v1：一隻暗影化成一縷輕煙（被擊散/誤入亮區/黎明）——放大淡出的微光球。
      spawnShadowPuff(m.x, m.y, m.z);
    } else if (m.t === "shadow_hit_ok") {
      // 暗影生物 v1：自己的挖擊命中——該暗影閃一下淡紫白（gone=true 時等 shadow_puff 收尾）。
      const ent = shadowEnts.get(m.id);
      if (ent) ent.hitFlash = 0.12;
    } else if (m.t === "siege") {
      // 暗潮之夜 v1：全村級的降臨（onset）／退去（cleared）橫幅——整句由伺服器給，i18n 集中在後端。
      // 降臨時橫幅多留一會兒（是一整夜的事，不像一般提示一閃即逝）。
      showMsg(m.msg || (m.phase === "onset" ? "🌑 暗潮之夜——暗影正湧向村莊！" : "🌅 暗潮退去了。"));
      if (m.phase === "onset") {
        const el = document.getElementById("msg");
        if (el) { clearTimeout(el._hideTimer); el._hideTimer = setTimeout(() => { el.style.display = "none"; }, 6000); }
      }
    } else if (m.t === "colony_discovered") {
      // 分村殖民 v1：走近一座此前沒發現過的野外村落——浮出它的名字與立村故事（只給自己看）。
      showMsg(m.line || "你發現了一座野外村落🏘️");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 4200);
    } else if (m.t === "colony_founded") {
      // 分村殖民 v1：世界某處剛奠下一座新村（人人可見的世界大事，稀有）——浮出立村捷報。
      showMsg("🏘️ 世界長出了新村落「" + (m.name || "野外村落") + "」——" + (m.story || ""));
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 5000);
    } else if (m.t === "firework") {
      // 乙太煙火 v1（785）：全場任一玩家施放的煙火——在該座標上方綻放一朵火花（人人可見）。
      spawnFirework(m.x, m.y, m.z, m.palette | 0);
    } else if (m.t === "bell_ring") {
      // 集會鐘 v1（自主提案切片）：全場任一玩家敲響的鐘——在該座標冒一圈往外擴散的聲波環（人人可見）。
      spawnBellRing(m.x | 0, m.y | 0, m.z | 0);
    } else if (m.t === "ring_ok") {
      // 集會鐘 v1：自己成功敲響——浮出「召來幾位居民」的回饋句。
      showMsg("🔔 鐘聲傳了出去，" + (m.count | 0) + " 位居民循聲趕來了～");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2600);
    } else if (m.t === "ring_none") {
      // 集會鐘 v1：敲響了但附近沒有聽得到又有空的居民（都睡了／遠行／剛應召過）。
      showMsg("🔔 鐘聲迴盪，可惜這會兒附近沒有能過來的居民。");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2600);
    } else if (m.t === "ring_fail") {
      // 集會鐘 v1：敲不到（離鐘太遠）。
      showErr(m.reason || "現在敲不到這座鐘");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "firework_ok") {
      // 乙太煙火 v1（785）：自己成功施放——浮出綻放回饋句。
      showMsg("🎆 " + (m.line || "煙火在夜空中綻放開來。"));
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2800);
      updateFireworkBtn(); // 背包已由 inv_update 更新，重算施放鈕（放完可能沒了）
    } else if (m.t === "firework_fail") {
      // 乙太煙火 v1（785）：放不了（冷卻中 / 背包沒有）。
      showErr(m.reason || "現在沒法施放煙火");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "fertilize_ok") {
      // 乙太沃肥 v1（789）：施肥成功——幼苗上噴一撮綠火花＋浮出回饋句（背包由 inv_update 更新）。
      spawnFertSparkle(m.x | 0, m.y | 0, m.z | 0);
      showMsg("🌱 " + (m.say || "撒下一撮沃肥，作物抽長了一截～"));
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2400);
    } else if (m.t === "tool_bonus") {
      // 工欲善其事 v1（790）：手持對的工具採集多收到一份材料——跳一句小回饋（背包由 inv_update 更新）。
      const iname = BLOCK_NAME[m.block_id] || "材料";
      showMsg("⛏️ 好工具！多採到 " + iname + " ×" + (m.count || 1));
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 1600);
    } else if (m.t === "coop_bonus") {
      // 並肩協作 v1（自主提案切片 827）：附近有其他真人玩家一起採集，默契多收到材料——
      // 跳一句小回饋（背包由 inv_update 更新）。
      showMsg(m.line || "🤝 並肩採集，默契多收了一份！");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 1800);
    } else if (m.t === "treasure") {
      // 深層寶藏 v1（自主提案切片）：天然礦脈裡的秘密驚喜，挖礦意外多得乙太幣——
      // 跳一句醒目的慶祝提示（背包由 inv_update 更新）。
      const iname = BLOCK_NAME[m.block_id] || "寶藏";
      showMsg("💎 挖到寶藏了！意外多得 " + iname + " ×" + (m.count || 1));
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 3200);
    } else if (m.t === "fertilize_fail") {
      // 乙太沃肥 v1（789）：施不了（太遠 / 非幼苗 / 背包沒有沃肥）。
      showErr(m.reason || "現在沒法施肥");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "bucket_ok") {
      // 水桶 v1（自主提案切片）：舀水／倒水成功——浮出回饋句（背包由 inv_update 更新，水格由 block 廣播更新）。
      showMsg(m.say || "🪣 水桶用了一下～");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2200);
    } else if (m.t === "bucket_fail") {
      // 水桶 v1：用不了（太遠 / 目標非水源或不可倒 / 背包沒有對應水桶）。
      showErr(m.reason || "現在沒法用水桶");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "hoe_ok") {
      // 鋤頭 v1（自主提案切片）：開墾成功——浮出回饋句（農田土由 block 廣播即時更新）。
      showMsg(m.say || "🪏 開墾好了一畦田～");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2200);
    } else if (m.t === "hoe_fail") {
      // 鋤頭 v1：開墾不了（太遠 / 目標非草地泥土 / 背包沒有鋤頭）。
      showErr(m.reason || "現在沒法開墾");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "feed_wildlife_ok") {
      // 餵野兔馴服 v1（自主提案切片）：馴服成功——浮出回饋句（背包由 inv_update 更新）。
      showMsg(m.say || "🥕 牠不再怕你了～");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2400);
    } else if (m.t === "feed_wildlife_fail") {
      // 餵野兔馴服 v1：餵不成（太遠／已經馴服過／背包沒有胡蘿蔔）。
      showErr(m.reason || "現在沒法餵食");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "feed_chicken_ok") {
      // 放養雞 v1（自主提案切片，ROADMAP 870）：馴服成功——浮出回饋句（背包由 inv_update 更新）。
      showMsg(m.say || "🌾 牠不再怕你了～");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2400);
    } else if (m.t === "feed_chicken_fail") {
      // 放養雞 v1：餵不成（太遠／已經馴服過／背包沒有種子）。
      showErr(m.reason || "現在沒法餵食");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "name_pet_ok") {
      // 為馴服的動物取名 v1（ROADMAP 895）：取名成功——浮出確認句（名牌由 players 廣播即時掛上）。
      showMsg(m.say || "🐾 牠有名字了～");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2600);
    } else if (m.t === "name_pet_fail") {
      // 為馴服的動物取名 v1：取不了名（太遠／還沒馴服／名字空白）。
      showErr(m.reason || "現在沒法取名");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "pet_treat_ok") {
      // 寵愛你的夥伴 v1（ROADMAP 899）：疼夥伴成功——浮出撒嬌句＋在牠頭頂綻一串愛心。
      showMsg(m.say || "💕 牠開心地蹭了蹭你～");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2600);
      // 依 id 回查該動物當前世界座標，就地噴愛心（找不到＝實體剛好離開視野，靜默略過）。
      const ent = m.id ? wildlifeEnts.get(m.id) : null;
      if (ent && ent.group) {
        const p = ent.group.position;
        spawnHearts(p.x, p.y, p.z);
      }
    } else if (m.t === "pet_command_ok") {
      // 寵物指令「安置／召回」v1（ROADMAP 898）：切換成功——浮出暖句（💤 待命標記由 players 廣播即時掛上/取下）。
      showMsg(m.say || (m.settled ? "🐾 牠乖乖在這兒待命～" : "🐾 牠又跟上你了～"));
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2600);
    } else if (m.t === "pet_command_fail") {
      // 寵物指令「安置／召回」v1：指揮不了（太遠／還沒馴服）。
      showErr(m.reason || "現在沒法指揮牠");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "return_gift") {
      // 居民回禮 v1（ROADMAP 667）：只有當事玩家才顯示提示並更新背包。
      if (m.player === myName) {
        const iname = BLOCK_NAME[m.item_id] || m.item_name || "物品";
        // 更新本地庫存（伺服器已累計新數量）。
        if (m.new_count > 0) myInv.set(m.item_id, m.new_count);
        updateInvHud();
        updateGiftBtn();
        // 顯示溫馨提示（比一般系統訊息更暖、附愛心）。
        appendMsg("sys", "💛 " + (m.resident_name || "居民") + " 把 " + iname + " ×" + m.qty + " 送給你了！");
      }
    } else if (m.t === "player_care") {
      // 居民關心你挨餓 v1（自主提案切片，ROADMAP 845）：只有當事玩家才顯示提示並更新背包。
      if (m.player === myName) {
        const iname = BLOCK_NAME[m.item_id] || m.item_name || "麵包";
        if (m.new_count > 0) myInv.set(m.item_id, m.new_count);
        updateInvHud();
        updateGiftBtn();
        appendMsg("sys", "🍞 " + (m.resident_name || "居民") + " 見你餓著肚子，遞了份 " + iname + " ×" + m.qty + " 給你！");
      }
    } else if (m.t === "night_guard") {
      // 守夜恩人 v1（自主提案切片，ROADMAP 888）：你替近旁居民驅散了逼近的暗影，只有當事玩家
      // 收到這則道謝提示（居民頭頂的道謝泡泡則全服可見，走既有 say 廣播）。
      if (m.player === myName) {
        appendMsg("sys", "🛡️ " + (m.resident_name || "居民") + " 感激你為ta驅散了逼近的暗影！");
      }
    } else if (m.t === "recipe_taught") {
      // 居民教你一道獨門配方 v1（自主提案切片，ROADMAP 849）：只有當事玩家才記到 knownRecipes
      // 並顯示提示；當下若正打開背包面板，重繪一次讓合成格立刻反映「學會了」。
      if (m.player === myName) {
        knownRecipes.add(m.recipe_id);
        if (bagPanelVisible()) renderBagPanel();
        appendMsg("sys", "📜 " + (m.resident_name || "居民") + " 教了你一道獨門配方：" + (m.name_zh || "護身符") + "！");
      }
    } else if (m.t === "fetch_delivered") {
      // 跑腿採集 v1（指令→任務第三刀）：只有下單的當事玩家才顯示提示並更新背包。
      if (m.player === myName) {
        const iname = BLOCK_NAME[m.item_id] || m.item_name || "物品";
        if (m.new_count > 0) myInv.set(m.item_id, m.new_count);
        updateInvHud();
        updateGiftBtn();
        appendMsg("sys", "📦 " + (m.resident_name || "居民") + "：" + (m.line || ("這是你要的" + iname + "！")));
      }
    } else if (m.t === "welcome_back") {
      // 久別重逢摘要 v1（ROADMAP 721）：只有自己看得到，離線期間世界發生的重要事件摘要。
      if (m.text) appendMsg("sys", m.text);
    } else if (m.t === "stall_sold_notice") {
      // 自由市集賣家通知 v1（自主提案切片，ROADMAP 864）：只有自己看得到，你不在時攤位成交的私訊。
      if (m.text) appendMsg("sys", m.text);
    } else if (m.t === "build_complete") {
      // 建物完工廣播 v1（ROADMAP 669）：全員可見，世界在長大。
      // 合力蓋家 v1（ROADMAP 834）：有協力者（additive helpers 欄位）就多提一句與誰合力。
      const who = m.resident || "居民";
      const what = m.kind || "建物";
      const helpers = Array.isArray(m.helpers) ? m.helpers : [];
      const credit = helpers.length ? "（與" + helpers.join("、") + "合力）" : "";
      appendMsg("sys", "🏗️ " + who + " 完成了「" + what + credit + "」的建造！走近去看看吧。");
    } else if (m.t === "wish_come_true") {
      // 心願真的成真 v1（ROADMAP 720）：這座建物是某位玩家的話種下的心願，全員可見；
      // 啟發者本人看到會格外有感（「我隨口說的話，真的被記住、被實現了」）。
      const who = m.resident || "居民";
      const what = m.kind || "建物";
      const player = m.player || "旅人";
      const mine = player === myName;
      appendMsg("sys", "🌟 " + who + " 因為" + (mine ? "你" : player) + "的一句話，把「" + what + "」蓋好了！");
    } else if (m.t === "item_wish_fulfilled") {
      // 送對禮物 v1（ROADMAP 722）：玩家送來的禮物正好是居民一直想要的東西，全員可見；
      // 送禮的當事人本人看到會格外有感（「我隨口送的東西，真的是她想要的」）。
      const who = m.resident || "居民";
      const item = m.item || "物品";
      const player = m.player || "旅人";
      const mine = player === myName;
      appendMsg("sys", "🎁 " + (mine ? "你" : player) + "送來的" + item + "，正是" + who + "一直想要的！");
    } else if (m.t === "trade_offer") {
      // 居民交易 v1（ROADMAP 670）：收到交易提案，顯示橫幅讓玩家確認。
      showTradeOffer(m);
      appendMsg("sys", "⇌ " + (m.resident_name || "居民") + " 想和你交易");
    } else if (m.t === "trade_done") {
      // 交易完成：顯示成功訊息，收起提案。
      hideTradeOffer();
      const got = m.got_count > 1 ? `${m.got_name}×${m.got_count}` : m.got_name;
      const gave = m.gave_count > 1 ? `${m.gave_name}×${m.gave_count}` : m.gave_name;
      appendMsg("sys", "✅ 交易成功！你給出了 " + gave + "，得到了 " + got);
      updateGiftBtn();
    } else if (m.t === "trade_fail") {
      // 交易失敗（太遠 / 沒材料 / 提案過期）。
      hideTradeOffer();
      showErr(m.reason || "交易失敗");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2200);
    } else if (m.t === "chest_view") {
      // 箱子 v1（ROADMAP 692）：伺服器回傳箱子內容，更新面板。
      _chestPos = { x: m.x, y: m.y, z: m.z };
      _chestItems = (m.items || []).slice();
      if (chestPanelEl) {
        chestPanelEl.style.display = "flex";
        renderChestPanel();
      }
    } else if (m.t === "chest_fail") {
      // 箱子操作失敗（數量不足等）。
      showErr(m.reason || "箱子操作失敗");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "sign_sync") {
      // 告示牌 v1（ROADMAP 740）：連線時一次收到世界上所有牌面文字，全部掛上。
      for (const s of (m.signs || [])) applySign(s.x, s.y, s.z, s.text || "");
    } else if (m.t === "sign") {
      // 告示牌 v1：某面牌子的文字變了（寫字/清空/破壞）——單面更新。
      applySign(m.x, m.y, m.z, m.text || "");
    } else if (m.t === "sleep_ok") {
      // 床 v1：睡覺成功——時鐘已跳到黎明（time_of_day 隨下一份 players 快照自動更新天色）。
      showMsg("😴 睡了一覺，天亮了！");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2200);
    } else if (m.t === "sleep_fail") {
      // 床 v1：白天/黎明/黃昏睡不著。
      showErr(m.reason || "睡不著");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "fish_cast_ok") {
      // 垂釣 v1（ROADMAP 734）：拋竿成功——浮標入水，等 m.wait 秒後提示上鉤（純 UX，伺服器裁定時機）。
      fishPending = true;
      showMsg(m.hint || "🎣 拋竿了，靜候魚兒上鉤…");
      if (fishBiteTimer) clearTimeout(fishBiteTimer);
      fishBiteTimer = setTimeout(() => {
        if (fishPending) showMsg("❗ 浮標一沉——有動靜！收竿！");
      }, Math.max(0, (m.wait || 4)) * 1000);
    } else if (m.t === "fish_too_early") {
      // 垂釣 v1：收竿太早，魚還沒上鉤——這竿保留，繼續等。
      showMsg(m.hint || "浮標還穩穩地浮著，再等一會兒…");
    } else if (m.t === "fish_catch") {
      // 垂釣 v1：釣起漁獲！背包已由 inv_update 更新；此處只揭曉。
      fishPending = false;
      if (fishBiteTimer) { clearTimeout(fishBiteTimer); fishBiteTimer = null; }
      showMsg(m.line || ("🎣 釣到了 " + (m.item_name || "魚") + "！"));
      updateGiftBtn();
    } else if (m.t === "fish_fail") {
      // 垂釣 v1：拋竿/收竿失敗（沒釣竿、非水面、太遠、還沒拋竿等）。
      fishPending = false;
      if (fishBiteTimer) { clearTimeout(fishBiteTimer); fishBiteTimer = null; }
      showErr(m.reason || "沒法釣魚");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "milestone_unlocked") {
      // 玩家里程碑 v1（ROADMAP 724）：只有自己看得到的私人慶祝提示；若面板剛好開著同步刷新。
      showMsg((m.icon || "🏅") + " 成就達成：" + (m.name_zh || "里程碑") + "！");
      if (milesVisible) refreshMilestones();
    } else if (m.t === "mastery_levelup") {
      // 玩家熟練度 v1（自主提案切片，ROADMAP 842）：私人慶祝提示；面板開著就同步刷新。
      showMsg(m.line || "熟練度升級了！");
      if (masteryVisible) refreshMastery();
    } else if (m.t === "mastery_bonus") {
      // 玩家熟練度 v1：練到 Lv.5 起的產出加成揭曉句（背包已由 inv_update 更新，此處只揭曉）。
      showMsg(m.line || "練出來的手法，多收了一份！");
    } else if (m.t === "landmark_notes") {
      // 地標旅人留言 v1（自主提案切片，ROADMAP 862）：第一次發現這處地標時伺服器主動推來
      // 留言簿——先看看先前旅人留下的話，再問要不要也留一句給後來的人。
      showLandmarkNotes(m.kind, m.x, m.y, m.z, m.notes || []);
    } else if (m.t === "landmark_note_fail") {
      // 地標旅人留言 v1：留言失敗（不是已知地標／需登入／內容審查未過／空白）。
      landmarkNoteJustSent = false;
      showErr(m.reason || "留言失敗");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "waypoint_sync") {
      // 個人路標 v1（自主提案切片，ROADMAP 869）：插旗/刪除成功後伺服器回傳最新完整清單。
      waypoints = m.items || [];
      if (compassVisible) renderCompassPanel();
    } else if (m.t === "waypoint_fail") {
      // 個人路標 v1：插旗/刪除失敗（空名稱／已達上限／找不到這支路標）。
      showErr(m.reason || "路標操作失敗");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "bottle_sync") {
      // 漂流瓶 v1（自主提案切片 825）：連線時一次收到世界上所有尚未被撿走的瓶子座標，全部掛上浮標。
      for (const b of (m.bottles || [])) addBottleMarker(b.x, b.y, b.z);
    } else if (m.t === "bottle_dropped") {
      // 漂流瓶 v1：有人丟了一只新瓶子——只知座標，內文絕不外流。
      addBottleMarker(m.x, m.y, m.z);
    } else if (m.t === "bottle_removed") {
      // 漂流瓶 v1：一只瓶子被撿走了，全場同步移除浮標。
      removeBottleMarker(m.x, m.y, m.z);
    } else if (m.t === "bottle_read") {
      // 漂流瓶 v1：撿起的瓶中信內文只單播給撿到的人——世界第一次有了陌生旅人留給你的話。
      showMsg("🍾 瓶中信：「" + (m.text || "") + "」");
    } else if (m.t === "bottle_throw_ok") {
      // 漂流瓶 v1：丟瓶成功的溫柔回饋。
      showMsg("🍾 瓶子漂走了，不知道會被誰撿到……");
    } else if (m.t === "bottle_fail") {
      // 漂流瓶 v1：丟瓶/撿瓶失敗（非水面、沒瓶子、需登入、已達上限等）。
      showErr(m.reason || "漂流瓶操作失敗");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "drop_sync") {
      // 掉落物 v1（自主提案切片 828）：連線時一次收到世界上所有還沒被撿走的掉落物，全部掛上浮標。
      for (const d of (m.items || [])) addDropMarker(d.id, d.x, d.y, d.z, d.item_id, d.count);
    } else if (m.t === "item_dropped") {
      // 掉落物 v1：有人丟下了一件材料——世界即時多一顆浮標。
      addDropMarker(m.id, m.x, m.y, m.z, m.item_id, m.count);
    } else if (m.t === "item_removed") {
      // 掉落物 v1：一件掉落物消失了（被撿走或逾時消散），全場同步移除浮標；
      // 撿走的那一份材料由既有 inv_update 補進撿到者的背包，不重造背包同步。
      removeDropMarker(m.id);
    } else if (m.t === "drop_fail") {
      // 掉落物 v1：丟東西失敗（材料不夠、地上掉落物已達上限等）。
      showErr(m.reason || "丟下材料失敗");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "stall_sync") {
      // 玩家自由市集 v1（自主提案切片 832）：連線時一次收到世界上所有還在等人接手的攤位。
      for (const s of (m.stalls || [])) addStallMarker(s.x, s.y, s.z, s.give_item, s.give_count, s.want_item, s.want_count);
    } else if (m.t === "stall_open") {
      // 玩家自由市集 v1：有人擺了個新攤位——世界即時多一個交易看板。
      addStallMarker(m.x, m.y, m.z, m.give_item, m.give_count, m.want_item, m.want_count);
    } else if (m.t === "stall_removed") {
      // 玩家自由市集 v1：一個攤位消失了（成交、收攤或逾時退還），全場同步移除浮標。
      removeStallMarker(m.x, m.y, m.z);
    } else if (m.t === "stall_trade_ok") {
      // 玩家自由市集 v1：接手成交成功——你換到了攤主給出的那份材料（背包已由 inv_update 更新）。
      showMsg("🏪 成交！你換到了 " + (BLOCK_NAME[m.got_item] || "材料") + " ×" + m.got_count);
    } else if (m.t === "stall_cancel_ok") {
      // 玩家自由市集 v1：收攤成功，材料已退回背包。
      showMsg("🏪 攤位收回了，材料退回背包。");
    } else if (m.t === "stall_fail") {
      // 玩家自由市集 v1：擺攤/接手/收攤失敗（位置不合法、材料不夠、攤位已被搶等）。
      showErr(m.reason || "市集操作失敗");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    }
  };
  ws.onclose = () => {
    wsReady = false;
    // 儲存斷線前位置（重連成功後恢復，不讓玩家回到出生點）。
    wsSavedPos = { x: player.x, y: player.y, z: player.z };
    wsIsReconnect = true;
    // 垂釣 v1：斷線時伺服器已清掉進行中的拋竿，前端旗標同步歸零，避免重連後卡在「以為還在釣」。
    fishPending = false;
    if (fishBiteTimer) { clearTimeout(fishBiteTimer); fishBiteTimer = null; }
    // 安靜期橫幅（3 秒）：快速重啟（部署重開 ~2s）不跳嚇人大橫幅；超時才顯示「重新連線中」。
    if (wsBannerTimer) { clearTimeout(wsBannerTimer); wsBannerTimer = null; }
    wsBannerTimer = setTimeout(() => {
      wsBannerTimer = null;
      if (!wsReady) showErr("重新連線中…");
    }, 3000);
    // 指數退避重連：300ms → 600ms → 1200ms → … 上限 8s，避免對剛重啟的伺服器瘋狂轟炸。
    if (wsRetryTimer) { clearTimeout(wsRetryTimer); wsRetryTimer = null; }
    wsRetryTimer = setTimeout(() => { wsRetryTimer = null; connect(); }, wsRetryDelay);
    wsRetryDelay = Math.min(wsRetryDelay * 2, WS_RETRY_MAX);
  };
  // onerror 後必跟 onclose，讓 onclose 統一處理重連；此處只記 console 不重複顯示橫幅。
  ws.onerror = (e) => { console.warn("[voxel] WS 錯誤，等待 onclose 重連", e && e.type); };
}
connect();

// ── Google 登入入口（比照 3D #821；沿用 2D/3D 同一套同源 cookie session）─────────────
// 右上角帳號 chip：訪客→「🔑 登入」（點了走既有 /auth/google/start）；登入→「帳號名 · 登出」。
// 登入後伺服器以 cookie 解出帳號 → join 自動綁帳號（記憶/好感度/背包跨 session 認得你）。
// 手機/直式友善：chip 小、靠右上、pointer-events 自理，不擋遊戲操作。
const acctEl = document.getElementById("acct");

function renderAccountChip() {
  if (!acctEl) return;
  acctEl.innerHTML = "";
  if (isLoggedIn) {
    const name = document.createElement("span");
    name.className = "acct-name";
    name.textContent = myAccountName || myName || "帳號";
    name.title = "已登入：" + (myAccountName || "");
    const out = document.createElement("button");
    out.type = "button";
    out.className = "acct-btn";
    out.textContent = "登出";
    out.title = "登出回到訪客";
    out.addEventListener("click", (e) => {
      if (e && e.preventDefault) e.preventDefault();
      // 清 session cookie 後重整：回訪客態（與 2D/3D 登出一致）。
      fetch("/auth/logout", { method: "POST", credentials: "same-origin" })
        .catch(() => {})
        .then(() => { try { location.reload(); } catch (_) { /* 測試 DOM 無 location */ } });
    });
    acctEl.appendChild(name);
    acctEl.appendChild(out);
  } else {
    const login = document.createElement("button");
    login.type = "button";
    login.className = "acct-btn acct-login";
    login.textContent = "🔑 登入";
    login.title = "用 Google 登入，讓居民記得你（記憶/好感度綁帳號）";
    login.addEventListener("click", (e) => {
      if (e && e.preventDefault) e.preventDefault();
      try { location.href = "/auth/google/start"; } catch (_) { /* 測試 DOM 無 location */ }
    });
    acctEl.appendChild(login);
  }
}

// 開頁查 /auth/me：已登入就點亮帳號 chip、記下帳號名（進場時帶上、伺服器仍以 cookie 為準）。
// OAuth 未設定／未登入回非 2xx → 維持訪客態（照常逛）。fetch 不可用（沙箱）就跳過。
renderAccountChip(); // 先以訪客態畫一次（/auth/me 命中再重畫）
if (typeof fetch === "function") {
  fetch("/auth/me", { credentials: "same-origin" })
    .then((r) => (r && r.ok ? r.json() : null))
    .then((me) => {
      if (me && me.id) {
        isLoggedIn = true;
        if (me.name) myAccountName = me.name;
        renderAccountChip();
      }
    })
    .catch(() => { /* 查不到就當訪客，不影響逛 */ });
}

// 走到哪、補要哪：請求玩家周邊半徑內、尚未載入也沒要過的 column。
let reqTimer = 0;
function streamChunks(dt) {
  reqTimer -= dt;
  if (!wsReady || reqTimer > 0) return;
  reqTimer = 0.25;
  const pcx = Math.floor(player.x / CHUNK), pcz = Math.floor(player.z / CHUNK);
  const R = 3;
  let sent = 0;
  for (let r = 0; r <= R && sent < 3; r++) {
    for (let dx = -r; dx <= r && sent < 3; dx++) {
      for (let dz = -r; dz <= r && sent < 3; dz++) {
        if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue; // 由近到遠的環
        const cx = pcx + dx, cz = pcz + dz, k = cx + "," + cz;
        if (requested.has(k)) continue;
        // 該 column 任一 cy 已載入就算有了
        if (chunks.has(ckey(cx, 0, cz)) || chunks.has(ckey(cx, 1, cz))) { requested.add(k); continue; }
        requested.add(k);
        ws.send(JSON.stringify({ t: "req", cx, cz }));
        sent++;
      }
    }
  }
}

// 位置上報（節流）
let sendTimer = 0;
function sendMove(dt) {
  sendTimer -= dt;
  if (!wsReady || sendTimer > 0) return;
  sendTimer = 0.1;
  // 手持工具可見 v1：順手把目前熱鍵選中的物品 id 帶上（0.1s 節流已有，零額外流量）。
  // 熱鍵格是「拿在手上要用的東西」，空格（undefined/0 之類非有效方塊）視為空手。
  const held = selectedBlock() || null;
  ws.send(JSON.stringify({ t: "move", x: player.x, y: player.y, z: player.z, yaw: player.yaw, held }));
}

// 自己的手持顯示：每幀即時反映熱鍵當前選中格，不等伺服器回音（別人看到的仍走
// broadcast held，容許 <=0.1s 的些微延遲，可接受）。
function updateMyHeldItem() { setHeldItem(myAvatar, selectedBlock() || null); }

// ── 主迴圈 ─────────────────────────────────────────────────────────────────
const SPEED = 5.0, GRAVITY = 24.0;
// 梯子攀爬速度（方塊/秒）；比走路略慢，強調「謹慎攀降」感。
const CLIMB_SPEED = 3.5;
let last = performance.now();
let frames = 0, fpsT = 0, fps = 0;
let dbgT = 0;

function update(dt) {
  // 手把輪詢（在既有 rAF 迴圈內，不另開迴圈）：先更新左搖桿移動/右搖桿視角/按鈕，
  // 讓下面的移動合成吃到本幀手把輸入（無延遲）。沒接手把時零成本早退。
  pollGamepad(dt);
  // 方向（相對鏡頭 yaw）
  const fwd = new THREE.Vector3(-Math.sin(player.yaw), 0, -Math.cos(player.yaw));
  const right = new THREE.Vector3(Math.cos(player.yaw), 0, -Math.sin(player.yaw));
  let mx = 0, mz = 0;
  if (keys["KeyW"] || keys["ArrowUp"]) mz += 1;
  if (keys["KeyS"] || keys["ArrowDown"]) mz -= 1;
  if (keys["KeyD"] || keys["ArrowRight"]) mx += 1;
  if (keys["KeyA"] || keys["ArrowLeft"]) mx -= 1;
  // 觸控搖桿（y 往上＝前進）
  mz += -joyVec.y; mx += joyVec.x;
  // 手把左搖桿（同觸控搖桿慣例：-y 前進、x 右移）
  mz += -gpMove.y; mx += gpMove.x;

  // ── 梯子攀爬 v1（ROADMAP 688）：進入梯子方格後取消重力、Space/跳鈕上爬、S/搖桿下降 ──
  const climbing = aabbHitsLadder(player.x, player.y, player.z, getRaw, PW, PH);
  if (climbing) {
    player.vy = 0;       // 取消重力累積
    player.grounded = false;
    // 上爬：Space（桌機）或搖桿向上（-joyVec.y > 0.2；y 軸向上為負）
    const climbUp = keys["Space"] || (-joyVec.y > 0.2);
    // 下降：S 鍵（桌機）或搖桿向下（joyVec.y > 0.2）
    const climbDown = keys["KeyS"] || keys["ArrowDown"] || (joyVec.y > 0.2);
    if (climbUp)        player.y += CLIMB_SPEED * dt;
    else if (climbDown) player.y -= CLIMB_SPEED * dt;
    // 水平仍可移動（側步可脫離梯子）
  } else {
    if ((keys["Space"]) && player.grounded) tryJump();
  }

  const dir = new THREE.Vector3();
  dir.addScaledVector(fwd, mz).addScaledVector(right, mx);
  if (dir.lengthSq() > 1e-4) {
    dir.normalize();
    moveAxis("x", dir.x * SPEED * dt);
    moveAxis("z", dir.z * SPEED * dt);
  }

  if (!climbing) {
    // 重力 + 垂直碰撞（只在非攀爬模式下套用）
    player.vy -= GRAVITY * dt;
    // 限制單幀垂直位移避免穿牆
    let dy = Math.max(-1.5, Math.min(1.5, player.vy * dt));
    const prevY = player.y;
    player.y += dy;
    if (overlaps()) {
      player.y = prevY;
      if (player.vy < 0) player.grounded = true;
      player.vy = 0;
    } else {
      if (player.vy < 0) player.grounded = false;
    }
    // 掉出世界保險：低於 -10 拉回出生高度
    if (player.y < -10) { player.y = 40; player.vy = 0; stepSmooth = 0; }
  }

  // 脫困保險（每幀）：若這幀結束後仍與實心方塊重疊（最常見：新 chunk 載入把人埋住、
  // 出生瞬間、走進未載入區後補載），把人頂出方塊外，避免永久卡死。沒卡就零成本早退。
  unstuckIfNeeded();

  // 踏階視覺補間衰減（frame-rate 無關的指數平滑）
  // stepSmooth > 0 → 視覺 Y 低於物理 Y；每幀靠近直到 < 0.005 格就吸附歸零。
  // 重力下落時 stepSmooth 保持 0，不影響往下的動態。
  if (stepSmooth > 0) {
    stepSmooth *= Math.exp(-STEP_SMOOTH_K * dt);
    if (stepSmooth < 0.005) stepSmooth = 0;
  }
  // visualY：bodyMesh 與鏡頭看向點用此值——踏階時從原地平滑升上去，消除瞬跳閃爍。
  const visualY = player.y - stepSmooth;

  // 玩家身體 + 朝向（用 visualY 避免角色瞬跳一格）。第一人稱藏自己身體（相機在眼睛裡）。
  bodyMesh.visible = (viewMode !== "first");
  bodyMesh.position.set(player.x, visualY + PH / 2, player.z);
  const meMoving = dir.lengthSq() > 1e-4;
  if (meMoving) bodyMesh.rotation.y = Math.atan2(dir.x, dir.z);
  // 走路動畫：有移動意圖就擺手腳，靜止回正（第一人稱藏起來時算了也不可見、成本可忽略）。
  animateAvatar(myAvatar, meMoving, dt);
  // 其他玩家：近 250ms 內位置有變化＝在走路，各自擺手腳（快照間也持續動、不卡頓）。
  const nowT = performance.now();
  for (const ent of others.values()) animateAvatar(ent.av, nowT - ent.lastMoveT < 250, dt);

  // embodied 靠近說話 v1：自己頭上的對話泡泡跟隨角色 + 倒數消失（話活在世界裡）。
  if (myBubbleTimer > 0) {
    myBubbleTimer -= dt;
    if (myBubbleTimer <= 0) { myBubble.visible = false; myBubbleText = ""; }
  }
  if (myBubble.visible) myBubble.position.set(player.x, visualY + PH + 0.85, player.z);

  if (viewMode === "first") {
    // 第一人稱：相機在眼睛高度，朝視線方向（yaw+pitch）看出去。
    const eye = new THREE.Vector3(player.x, visualY + 1.5, player.z);
    camera.position.copy(eye);
    const d = viewDir();
    camera.lookAt(eye.x + d.x, eye.y + d.y, eye.z + d.z);
  } else {
    // 第三人稱鏡頭跟隨（用 visualY 讓鏡頭也跟著平滑升，不突然跳）
    const lookTarget = new THREE.Vector3(player.x, visualY + 1.3, player.z);
    const dist = 6.0, cp = Math.cos(camPitch), sp = Math.sin(camPitch);
    camera.position.set(
      lookTarget.x + Math.sin(player.yaw) * dist * cp,
      lookTarget.y + dist * sp,
      lookTarget.z + Math.cos(player.yaw) * dist * cp
    );
    camera.lookAt(lookTarget);
  }

  // 準心對準的方塊（破壞/放置目標）+ 高亮外框。
  updateTarget();

  // 準心+按鈕模式：更新準心對到的居民（讓挖鈕切「說話」）。
  updateCrosshairResident(dt);

  // 採礦手感 v1（ROADMAP 687）：每幀推進挖掘進度（桌機左鍵／手機挖鈕／手把挖鍵共用計時）。
  tickMining(dt);

  // 發光方塊 v1（ROADMAP 691 + 乙太礦脈 v1）：手持發光方塊時在鏡頭附近亮對應色光。
  const heldSel = selectedBlock();
  const holdingLight = isLightBlock(heldSel) && (myInv.get(heldSel) || 0) > 0;
  if (holdingLight) {
    heldTorchLight.position.copy(camera.position);
    heldTorchLight.color.setHex(lightColorFor(heldSel)); // 火把暖橘／乙太燈青藍
    heldTorchLight.intensity = 1.8;
    heldTorchLight.visible = true;
  } else {
    heldTorchLight.intensity = 0;
    heldTorchLight.visible = false;
  }
  // 每 2 秒刷新近旁放置火把的光源池位置（非每幀掃，節省 CPU）。
  _torchRefreshTimer -= dt;
  if (_torchRefreshTimer <= 0) {
    _torchRefreshTimer = 2.0;
    updateNearbyTorchLights();
  }

  // 水體視覺升級 v1：累積 time uniform 驅動水面波動 shader。
  waterTime.value += dt;

  // 水下氛圍：相機所在方塊是否為水（每幀一次 getRaw 查詢，成本極低）。
  updateUnderwaterAtmosphere();
  updateDawnMist(); // 晨霧 v1（913）：清晨縮短 fog 能見度、日出後漸散（須在水下判定後，水下時讓位給水下霧）

  updateRain(dt);
  updateRainbow(dt);
  updateMeteor(dt); // 流星許願 v1（904）：上升緣播一道劃過夜空的光痕
  updateNightSky(dt);
  updateFireworks(dt); // 乙太煙火 v1（785）：推進進行中的煙火綻放
  updateShadowFx(dt);  // 暗影生物 v1：暗影浮動/微光呼吸/受擊閃白/輕煙淡出
  updateBellRings(dt); // 集會鐘 v1（自主提案切片）：推進進行中的鐘聲漣漪
  updateHumNotes(dt);  // 居民哼歌 v1（788）：推進頭頂飄浮音符
  animateResidents(dt); // 居民表情/肢體 v1：依心情/說話/附近暗影驅動柔和肢體語言
  updateHearts(dt);    // 寵愛你的夥伴 v1（899）：推進小夥伴頭頂的愛心
  updateFertSparkle(dt); // 乙太沃肥 v1（789）：推進施肥綠火花
  updateAmbientLife(dt); // 世界環境氛圍 v1（905）：白天蝴蝶／夜間螢火蟲環境微生物
  streamChunks(dt);
  sendMove(dt);
  updateMyHeldItem(); // 手持工具可見 v1：即時反映熱鍵切換，不等節流送出的回音

  // 每幀重建少量 dirty chunk（分攤成本）
  let built = 0;
  for (const key of dirty) {
    rebuildChunk(key);
    dirty.delete(key);
    if (++built >= 4) break;
  }
}

function safeRender() {
  renderer.render(scene, camera);
}

function loop() {
  const now = performance.now();
  let dt = (now - last) / 1000; last = now;
  if (dt > 0.1) dt = 0.1; // 分頁切回來別跳一大步
  try {
    update(dt);
    safeRender();
  } catch (e) {
    // render 一拋會永久停 rAF（畫面凍結）——抓住、印出、自我恢復（比照 3D safeRender 護網）。
    console.error("[voxel] 迴圈例外：", e);
    showErr("渲染例外（已自我恢復，見 console）：" + (e && e.message ? e.message : e));
  }

  // FPS / HUD
  frames++; fpsT += dt;
  if (fpsT >= 0.5) { fps = frames / fpsT; frames = 0; fpsT = 0; }
  dbgT += dt;
  if (dbgT >= 0.25) {
    dbgT = 0;
    updateClock(); // 時段指示器（ROADMAP 896）：每 0.25 秒依 worldTime 刷新徽章
    updateSeason(); // 季節指示器（ROADMAP 897）：同節拍依快照 season/season_day 刷新徽章
    // 觸控裝置顯示精簡文字，避免直式螢幕頂部 HUD 溢出
    hudEl.textContent = isTouch
      ? `乙太方界 · ${myName}\n${settings.touchMode === "crosshair" ? "拖曳看・⛏挖鈕・放置鈕" : "輕點挖・放置鈕放"}\nchunk:${chunks.size} 線上:${others.size + 1} 居民:${residents.size}`
      : `乙太方界 · ${myName}\nWASD移動·拖曳轉視角·空白跳\n左鍵/輕點挖·右鍵/放置鈕放·1-6選方塊\nchunk: ${chunks.size}　線上: ${others.size + 1}　居民: ${residents.size}`;
    if (DEBUG) {
      dbgEl.style.display = "block";
      // 後端版本：直式/手機友善，commit 與 build 時間各自一行。後端離線/未起時顯示「?」。
      const beCommit = backendVersion ? backendVersion.commit : "?";
      const beBuilt = backendVersion ? (backendVersion.built_at || "?") : "?";
      dbgEl.textContent =
        `FPS ${fps.toFixed(0)}\n` +
        `chunks ${chunks.size}  meshes ${meshes.size}\n` +
        `pos ${player.x.toFixed(1)},${player.y.toFixed(1)},${player.z.toFixed(1)}\n` +
        `grounded ${player.grounded}\n` +
        `build ${window.__BUILD__ || "?"}\n` +
        `後端 ${beCommit}\n` +
        `built ${beBuilt}`;
    }
  }
  requestAnimationFrame(loop);
}
requestAnimationFrame(loop);

addEventListener("resize", () => {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
});

// ── 背包 + 2×2 合成格 v1（ROADMAP 664）──────────────────────────────────────
// 前端配方表（對齊後端 voxel_craft::RECIPES，id/inputs/output 穩定契約）。
// 無順序合成（shapeless）：格子裡只要湊齊材料種類+數量即可，位置不限。
const RECIPES_JS = [
  { id: "plank",        name: "木板",   inputs: [[WOOD, 2]],  output_block: PLANK,       out_count: 4 },
  { id: "stone_brick",  name: "石磚",   inputs: [[STONE, 2]], output_block: STONE_BRICK, out_count: 2 },
  { id: "glass",        name: "玻璃",   inputs: [[SAND, 2]],  output_block: GLASS,       out_count: 1 },
  { id: "till",         name: "農田土", inputs: [[DIRT, 2]],  output_block: FARM_SOIL,   out_count: 2 },
  // 工作台 v1（ROADMAP 665）：4 木板 → 1 工作台（放滿 2×2 四格）。
  // 先前缺這條 → 玩家放 4 木板卻合不出工作台（前端配方表和後端 voxel_craft 脫節）。
  { id: "workbench",    name: "工作台", inputs: [[PLANK, 4]], output_block: WORKBENCH,   out_count: 1 },
  // 麵包 v1（ROADMAP 668）：3 小麥顆粒 → 1 麵包
  { id: "bread",        name: "麵包",   inputs: [[WHEAT, 3]], output_block: BREAD,       out_count: 1 },
  // 火把 v1（ROADMAP 685）：1 木頭 + 1 煤礦 → 4 火把
  { id: "torch",        name: "火把",   inputs: [[WOOD, 1], [COAL_ORE, 1]], output_block: TORCH, out_count: 4 },
  // 鎬具 v1（ROADMAP 687）：採石/採礦手感加速；剛好 2×2 四格
  { id: "wood_pickaxe",  name: "木鎬", inputs: [[WOOD, 3], [PLANK, 1]],  output_block: PICKAXE_WOOD,  out_count: 1 },
  { id: "stone_pickaxe", name: "石鎬", inputs: [[STONE, 3], [PLANK, 1]], output_block: PICKAXE_STONE, out_count: 1 },
  // 梯子 v1（ROADMAP 688）：3 木板 → 3 梯子（垂直攀爬，深礦上下自如）
  { id: "ladder", name: "梯子", inputs: [[PLANK, 3]], output_block: LADDER, out_count: 3 },
  // 斧頭 v1（ROADMAP 689）：砍木加速，和鎬具互補的工具線；剛好 2×2 四格
  { id: "wood_axe",  name: "木斧", inputs: [[WOOD, 3], [PLANK, 1]],  output_block: AXE_WOOD,  out_count: 1 },
  { id: "stone_axe", name: "石斧", inputs: [[STONE, 3], [PLANK, 1]], output_block: AXE_STONE, out_count: 1 },
  // 鏟子 v1（ROADMAP 690）：挖土/沙/草地加速，完成採集三件套（鎬+斧+鏟）
  { id: "wood_shovel",  name: "木鏟", inputs: [[WOOD, 1], [PLANK, 1]],  output_block: SHOVEL_WOOD,  out_count: 1 },
  { id: "stone_shovel", name: "石鏟", inputs: [[STONE, 1], [PLANK, 1]], output_block: SHOVEL_STONE, out_count: 1 },
  // 木門 v1（ROADMAP 693）：4 木板 → 2 門（填滿 2×2 格）
  { id: "door", name: "木門", inputs: [[PLANK, 4]], output_block: DOOR_CLOSED, out_count: 2 },
  // 床 v1：3 木板 + 3 葉片（當被褥）→ 1 床
  { id: "bed", name: "床", inputs: [[PLANK, 3], [LEAVES, 3]], output_block: BED, out_count: 1 },
  // 冰晶燈 v1（冰晶合成）：1 冰晶 + 2 玻璃 → 1 冰晶燈（雪原遠征的建造回報）
  { id: "ice_lantern", name: "冰晶燈", inputs: [[ICE_CRYSTAL, 1], [GLASS, 2]], output_block: ICE_LANTERN, out_count: 1 },
  // 告示牌 v1（ROADMAP 740）：2 木板 → 1 告示牌（唯一多重集，避開 4 木板＝工作台/木門的遮蔽）
  { id: "sign", name: "告示牌", inputs: [[PLANK, 2]], output_block: SIGN, out_count: 1 },
  // 水桶 v1（自主提案切片）：3 鐵錠 → 1 水桶（舀水引水灌溉乾田；鐵錠需熔爐，故不必再過工作台）
  { id: "bucket", name: "水桶", inputs: [[IRON_INGOT, 3]], output_block: BUCKET, out_count: 1 },
  // 鋤頭 v1（自主提案切片）：2 木頭 + 1 木板 → 1 木鋤頭（就地把草/土開墾成農田土；與水桶成對）
  { id: "hoe", name: "木鋤頭", inputs: [[WOOD, 2], [PLANK, 1]], output_block: HOE, out_count: 1 },
  // 莓果叢苗 v1（自主提案切片 806）：1 樹苗 + 2 種子 → 1 莓果叢苗（種下後多年生、可反覆採收，不必重種）
  { id: "berry_bush", name: "莓果叢苗", inputs: [[SAPLING, 1], [SEEDS, 2]], output_block: BERRY_BUSH, out_count: 1 },
  // 木長椅 v1（自主提案切片）：2 木頭 + 2 木板 → 1 木長椅（擺在世界裡→白天路過的居民會停下坐上去歇腳）
  { id: "bench", name: "木長椅", inputs: [[WOOD, 2], [PLANK, 2]], output_block: BENCH, out_count: 1 },
  // 漂流瓶 v1（自主提案切片 825）：2 玻璃 → 1 空玻璃瓶（對準水面丟下、寫上一句瓶中信）
  { id: "bottle", name: "空玻璃瓶", inputs: [[GLASS, 2]], output_block: BOTTLE, out_count: 1 },
  // 染色建材 v1（自主提案切片）：2 沙 + 1 礦物 → 2 陶磚（世界第一批彩色建材）
  { id: "terracotta_red",   name: "紅陶磚", inputs: [[SAND, 2], [IRON_ORE, 1]],  output_block: TERRACOTTA_RED,   out_count: 2 },
  { id: "terracotta_black", name: "黑陶磚", inputs: [[SAND, 2], [COAL_ORE, 1]],  output_block: TERRACOTTA_BLACK, out_count: 2 },
  { id: "terracotta_white", name: "白陶磚", inputs: [[SAND, 2], [SNOW, 1]],      output_block: TERRACOTTA_WHITE, out_count: 2 },
  { id: "terracotta_blue",  name: "青陶磚", inputs: [[SAND, 2], [AETHER_ORE, 1]], output_block: TERRACOTTA_BLUE,  out_count: 2 },
  // 護身符 v1（居民教你一道獨門配方，自主提案切片 849）：1 石頭 + 1 紅花 → 1 護身符。
  // `taught: true`——與其餘配方不同，湊對材料還不夠，要先被某位感情夠深的居民教過
  // （`knownRecipes` 由伺服器權威回報）才真正合成得出來，見 matchBagRecipe。
  { id: "amulet", name: "護身符", inputs: [[STONE, 1], [WILDFLOWER_RED, 1]], output_block: AMULET, out_count: 1, taught: true },
  // 乙太幣鑄造 v1（ROADMAP 873，自主提案切片）：三種最容易湊齊的原礦各開一條「賣出換幣」
  // 配方，統一 4 份原礦 → 1 枚乙太幣的匯率。鑄出的幣可拿去自由市集標價收付（832 已支援
  // 任意物品當 want_item，乙太幣天生就能直接掛上去，不必再改市集半行程式碼）。
  { id: "coin_from_wood",  name: "乙太幣（木頭）", inputs: [[WOOD, 4]],  output_block: COIN, out_count: 1 },
  { id: "coin_from_stone", name: "乙太幣（石頭）", inputs: [[STONE, 4]], output_block: COIN, out_count: 1 },
  { id: "coin_from_sand",  name: "乙太幣（沙）",   inputs: [[SAND, 4]],  output_block: COIN, out_count: 1 },
  // 驅影之劍 v1（ROADMAP 887，自主提案切片）——世界第一批武器，握在手上驅散夜之暗影更快。
  // 對齊後端 voxel_craft::RECIPES：木劍 {5:1,8:2}、石劍 {3:2,8:1}（鐵劍在工作台，見下）。
  { id: "wood_sword",  name: "木劍", inputs: [[WOOD, 1], [PLANK, 2]], output_block: SWORD_WOOD,  out_count: 1 },
  { id: "stone_sword", name: "石劍", inputs: [[STONE, 2], [PLANK, 1]], output_block: SWORD_STONE, out_count: 1 },
];

// ── 背包面板狀態 ──────────────────────────────────────────────────────────────
// bagGrid[0..3]：2×2 格子，0 代表空格，非零代表 block_id。
const bagGrid = [0, 0, 0, 0];
// 目前被「拿起」的 block_id（0 = 沒拿任何東西）。
let bagPick = 0;
// 居民教你一道獨門配方 v1（自主提案切片 849）：這位玩家已被居民教過的獨門配方 id 集合，
// 連線時向 `/voxel/known_recipes` 拉一次 + 收到 `recipe_taught` 廣播即時更新（見 refreshKnownRecipes）。
const knownRecipes = new Set();

/** 向後端抓這位玩家已被教過的獨門配方（居民教你一道獨門配方 v1，849）。 */
async function refreshKnownRecipes() {
  if (!myName || myName === "旅人") return;
  try {
    const resp = await fetch(`/voxel/known_recipes?player=${encodeURIComponent(myName)}`);
    if (!resp.ok) throw new Error("known_recipes fetch failed: " + resp.status);
    const rows = await resp.json();
    for (const r of rows) if (r.known) knownRecipes.add(r.id);
  } catch (err) {
    // 拉取失敗不阻斷遊戲——只是這次連線暫時看不到已學會的獨門配方，下次重連再試。
  }
}

const bagPanelEl = document.getElementById("bagPanel");
const bagInvGridEl = document.getElementById("bagInvGrid");
const bagGrid2x2El = document.getElementById("bagGrid2x2");
const bagResultEl  = document.getElementById("bagResultSlot");

function openBagPanel() {
  if (!bagPanelEl) return;
  // 四個合成/儲物面板共用同一個置中定位（手機直式尤其明顯疊在一起），
  // 彼此互斥才不會疊出無法操作的畫面——開一個之前先關掉其他三個。
  closeWbPanel();
  closeFurnacePanel();
  closeChestPanel();
  releaseMouse(); // 桌機：開面板要放開滑鼠鎖定，游標才能點格子
  bagPanelEl.style.display = "flex";
  renderBagPanel();
}
function closeBagPanel() {
  if (!bagPanelEl) return;
  bagPanelEl.style.display = "none";
  bagPick = 0; // 關面板時清除選取
}
function bagPanelVisible() {
  return bagPanelEl ? bagPanelEl.style.display === "flex" : false;
}

/**
 * matchBagRecipe——無順序配方比對（純函式，確定性）。
 * 統計格子裡的 block_id 出現次數，比對 RECIPES_JS，回傳 {recipe, canCraft} 或 null。
 * canCraft = 玩家實際背包材料足夠 ＋（若是獨門配方）已被居民教過（格子放入是「預覽意圖」，
 * 不實際扣除；居民教你一道獨門配方 v1，自主提案切片 849）。
 */
function matchBagRecipe() {
  const gridCounts = new Map();
  for (const bid of bagGrid) {
    if (bid !== 0) gridCounts.set(bid, (gridCounts.get(bid) || 0) + 1);
  }
  if (gridCounts.size === 0) return null;
  for (const r of RECIPES_JS) {
    const needed = new Map(r.inputs.map(([b, c]) => [b, c]));
    if (needed.size !== gridCounts.size) continue;
    let match = true;
    for (const [b, c] of needed) {
      if ((gridCounts.get(b) || 0) !== c) { match = false; break; }
    }
    if (!match) continue;
    const hasMaterials = r.inputs.every(([b, c]) => (myInv.get(b) || 0) >= c);
    const taughtOk = !r.taught || knownRecipes.has(r.id);
    return { recipe: r, canCraft: hasMaterials && taughtOk, taughtOk };
  }
  return null;
}

/** 顏色方塊 DOM（inline background swatch）。 */
function makeSwatchEl(blockId, cls) {
  const el = document.createElement("div");
  el.className = cls;
  const c = COLOR[blockId] || COLOR[STONE];
  el.style.background = `rgb(${(c[0]*255)|0},${(c[1]*255)|0},${(c[2]*255)|0})`;
  return el;
}

/** 渲染物品欄區域——列出背包內所有有數量的方塊，可點選「拿起」。 */
function renderBagInvGrid() {
  if (!bagInvGridEl) return;
  bagInvGridEl.innerHTML = "";
  const items = [...myInv.entries()].filter(([, cnt]) => cnt > 0);
  if (items.length === 0) {
    const emp = document.createElement("div");
    emp.style.cssText = "color:#506070;font-size:12px;font-style:italic;padding:6px 0";
    emp.textContent = "背包是空的，去挖一些方塊吧";
    bagInvGridEl.appendChild(emp);
    return;
  }
  items.forEach(([bid, cnt]) => {
    const slot = document.createElement("div");
    slot.className = "bag-inv-slot" + (bagPick === bid ? " picked" : "");
    slot.appendChild(makeSwatchEl(bid, "bag-inv-sw"));
    const name = document.createElement("div");
    name.className = "bag-inv-name";
    name.textContent = BLOCK_NAME[bid] || "?";
    const cntEl = document.createElement("div");
    cntEl.className = "bag-inv-cnt";
    cntEl.textContent = "×" + cnt;
    slot.appendChild(name); slot.appendChild(cntEl);
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      if (bagPick === bid) {
        bagPick = 0; // 再點同一格 → 放下（取消選取），不動快捷欄
      } else {
        bagPick = bid;
        // 麥塊互動：點背包物品 → 指派進當前選中的快捷欄格（也可再點某格改放）。
        assignToHotbar(selectedSlot, bid);
      }
      renderBagPanel();
    });
    bagInvGridEl.appendChild(slot);
  });
}

/** 渲染 2×2 合成格 + 結果格。 */
function renderBagCraftArea() {
  if (!bagGrid2x2El || !bagResultEl) return;
  bagGrid2x2El.innerHTML = "";
  for (let i = 0; i < 4; i++) {
    const bid = bagGrid[i];
    const slot = document.createElement("div");
    slot.className = "bag-grid-slot" + (bid !== 0 ? " filled" : "");
    if (bid !== 0) {
      slot.appendChild(makeSwatchEl(bid, "bag-grid-sw"));
      const lbl = document.createElement("div");
      lbl.className = "bag-grid-lbl";
      lbl.textContent = BLOCK_NAME[bid] || "?";
      slot.appendChild(lbl);
    }
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      if (bagPick !== 0 && bid === 0) {
        // 拿著東西 + 格子空 → 放入
        bagGrid[i] = bagPick;
        bagPick = 0;
      } else if (bagPick !== 0 && bid !== 0) {
        // 拿著東西 + 格子已有 → 交換
        bagGrid[i] = bagPick;
        bagPick = bid;
      } else if (bagPick === 0 && bid !== 0) {
        // 沒拿東西 + 格子有東西 → 拿起（格子清空）
        bagPick = bid;
        bagGrid[i] = 0;
      }
      renderBagPanel();
    });
    bagGrid2x2El.appendChild(slot);
  }
  // 結果格
  const match = matchBagRecipe();
  bagResultEl.className = ""; // 重設 class
  bagResultEl.innerHTML = "";
  if (match) {
    const r = match.recipe;
    bagResultEl.classList.add(match.canCraft ? "has-result" : "no-material");
    bagResultEl.appendChild(makeSwatchEl(r.output_block, "bag-res-sw"));
    const cnt = document.createElement("div");
    cnt.className = "bag-res-cnt";
    cnt.textContent = "×" + r.out_count;
    const name = document.createElement("div");
    name.className = "bag-res-name";
    name.textContent = r.name;
    bagResultEl.appendChild(cnt); bagResultEl.appendChild(name);
    if (!match.canCraft) {
      const warn = document.createElement("div");
      warn.style.cssText = "font-size:9px;color:#ff8060;margin-top:2px";
      // 居民教你一道獨門配方 v1（849）：材料足夠但沒學過 → 專屬提示，跟材料不足分開，
      // 讓玩家知道「不是缺材料，是還沒被教過」，去找感情夠深的居民多聊聊。
      warn.textContent = match.taughtOk ? "材料不足" : "你還沒學會這道配方";
      bagResultEl.appendChild(warn);
    }
  } else {
    // 沒有配方吻合：不洩題（維護者「合成應該自己摸索」）——只給「還沒湊對」的模糊訊號，
    // 讓玩家知道格子有反應、不是壞掉，但確切配方留給自己試 / 問居民（古代智慧口耳相傳）。
    const hasItems = bagGrid.some((b) => b !== 0);
    if (hasItems) {
      const hint = document.createElement("div");
      hint.style.cssText = "font-size:10px;color:#c0b090;text-align:center;line-height:1.3";
      hint.textContent = "……還湊不出什麼。";
      bagResultEl.appendChild(hint);
    }
  }
}

/** 渲染整個背包面板（物品欄 + 合成格）。 */
function renderBagPanel() {
  renderBagInvGrid();
  renderBagCraftArea();
}

// 結果格點擊：送出合成請求。
if (bagResultEl) bagResultEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  const match = matchBagRecipe();
  if (!match || !match.canCraft || !wsReady) return;
  ws.send(JSON.stringify({ t: "craft", recipe_id: match.recipe.id }));
});

// 清除合成格按鈕。
const bagClearBtnEl = document.getElementById("bagClearBtn");
if (bagClearBtnEl) bagClearBtnEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  bagGrid.fill(0);
  bagPick = 0;
  renderBagPanel();
});

// 視角切換鈕（👁）：第一/三人稱互換（等同 F5）。初始文字依預設模式設定。
const viewBtnEl = document.getElementById("viewBtn");
if (viewBtnEl) {
  viewBtnEl.textContent = (viewMode === "first") ? "👁 第一人稱" : "🧍 第三人稱";
  viewBtnEl.addEventListener("click", (e) => { toggleViewMode(); e.stopPropagation(); });
}

// 背包按鈕（🎒）開關面板。
const bagBtnEl = document.getElementById("bagBtn");
if (bagBtnEl) bagBtnEl.addEventListener("click", (e) => {
  if (bagPanelVisible()) { closeBagPanel(); } else { openBagPanel(); }
  e.stopPropagation();
});
// 享用鈕（親手煮的暖食自己也能享用 v1，779）：吃下一份自己煮的熟食。
const eatBtnEl = document.getElementById("eatBtn");
if (eatBtnEl) eatBtnEl.addEventListener("click", (e) => {
  tryEatDish();
  e.stopPropagation();
});
// 乙太煙火 v1（785）：施放鈕點擊 → 朝夜空放一束。
const fireworkBtnEl = document.getElementById("fireworkBtn");
if (fireworkBtnEl) fireworkBtnEl.addEventListener("click", (e) => {
  tryLaunchFirework();
  e.stopPropagation();
});
// 點面板外關閉。
document.addEventListener("pointerdown", (e) => {
  if (bagPanelVisible()) {
    if (!bagPanelEl.contains(e.target) && e.target !== bagBtnEl) closeBagPanel();
  }
});
// 關閉鈕（✕）。
const bagCloseEl = document.getElementById("bagClose");
if (bagCloseEl) bagCloseEl.addEventListener("click", closeBagPanel);

// ── 工作台 3×3 合成面板 v1（ROADMAP 665）─────────────────────────────────────
// 工作台配方表（對齊後端 voxel_craft::WORKBENCH_RECIPES）。
// 需要 5-6 個格子，超出 2×2，必須在工作台才能完成。
const WORKBENCH_RECIPES_JS = [
  { id: "plank_wb",       name: "木板（大量）",   inputs: [[WOOD, 6]],                output_block: PLANK,       out_count: 16 },
  { id: "stone_brick_wb", name: "石磚（大量）",   inputs: [[STONE, 6]],               output_block: STONE_BRICK, out_count: 10 },
  { id: "glass_wb",       name: "玻璃（大量）",   inputs: [[SAND, 6]],                output_block: GLASS,       out_count: 8  },
  { id: "stone_wood_mix", name: "混合石磚",       inputs: [[STONE, 3], [PLANK, 3]],   output_block: STONE_BRICK, out_count: 6  },
  { id: "farm_kit",       name: "農耕大包",       inputs: [[DIRT, 4], [WOOD, 2]],     output_block: FARM_SOIL,   out_count: 8  },
  { id: "iron_block",     name: "鐵磚",           inputs: [[IRON_INGOT, 6]],           output_block: IRON_BLOCK,  out_count: 2  },
  // 鐵鎬（ROADMAP 687）：3 鐵錠 + 2 木板 → 1 鐵鎬（5 格，需工作台）
  { id: "iron_pickaxe",   name: "鐵鎬",           inputs: [[IRON_INGOT, 3], [PLANK, 2]], output_block: PICKAXE_IRON, out_count: 1  },
  // 鐵斧（ROADMAP 689）：3 鐵錠 + 2 木板 → 1 鐵斧（5 格，需工作台；砍木材 6×）
  { id: "iron_axe",       name: "鐵斧",           inputs: [[IRON_INGOT, 3], [PLANK, 2]], output_block: AXE_IRON,     out_count: 1  },
  // 鐵鏟（ROADMAP 690）：2 鐵錠 + 3 木板 → 1 鐵鏟（5 格，需工作台；挖軟土 6×）
  { id: "iron_shovel",    name: "鐵鏟",           inputs: [[IRON_INGOT, 2], [PLANK, 3]], output_block: SHOVEL_IRON,  out_count: 1  },
  // 箱子 v1（ROADMAP 692）：8 木板 → 1 箱子（8 格，需工作台；放置後儲物）
  { id: "chest",          name: "箱子",           inputs: [[PLANK, 8]],                  output_block: CHEST,        out_count: 1  },
  // 乙太燈 v1（乙太礦脈）：1 乙太礦 + 4 玻璃 → 1 乙太燈（5 格，需工作台；散發真實青藍光）
  { id: "aether_lamp",    name: "乙太燈",         inputs: [[AETHER_ORE, 1], [GLASS, 4]], output_block: AETHER_LAMP,  out_count: 1  },
  // 野菜暖湯 v1（ROADMAP 778）：2 胡蘿蔔 + 2 馬鈴薯 + 1 小麥 → 1 暖湯（三種作物一大鍋，5 格需工作台）
  { id: "veggie_stew",    name: "野菜暖湯",       inputs: [[CARROT, 2], [POTATO, 2], [WHEAT, 1]], output_block: STEW, out_count: 1 },
  // 乙太煙火 v1（ROADMAP 785）：1 乙太礦 + 2 煤礦 + 2 沙 → 3 乙太煙火（工作台；朝夜空施放的慶祝道具）
  { id: "aether_firework", name: "乙太煙火",      inputs: [[AETHER_ORE, 1], [COAL_ORE, 2], [SAND, 2]], output_block: FIREWORK, out_count: 3 },
  // 乙太沃肥 v1（ROADMAP 789）：3 雜草 + 2 泥土 → 2 乙太沃肥（工作台；手持對準幼苗一撒即催熟一截）
  { id: "aether_fertilizer", name: "乙太沃肥",    inputs: [[GRASS, 3], [DIRT, 2]], output_block: FERTILIZER, out_count: 2 },
  // 乙太營火 v1（自主提案切片）：3 石頭 + 2 木頭 + 1 煤礦 → 1 營火（工作台；發光方塊，夜裡吸引居民圍暖）
  { id: "campfire",       name: "營火",           inputs: [[STONE, 3], [WOOD, 2], [COAL_ORE, 1]], output_block: CAMPFIRE, out_count: 1 },
  // 集會鐘 v1（自主提案切片）：4 鐵錠 + 1 木頭 → 1 集會鐘（工作台；放下後右鍵敲響召集附近居民）
  { id: "bell",           name: "集會鐘",         inputs: [[IRON_INGOT, 4], [WOOD, 1]], output_block: BELL, out_count: 1 },
  // 雞舍生蛋 v1（自主提案切片）：4 木頭 + 2 葉片 → 1 雞舍（工作台；放下後靜候一段時間會生蛋）
  { id: "coop",           name: "雞舍",           inputs: [[WOOD, 4], [LEAVES, 2]], output_block: COOP, out_count: 1 },
  // 建築藍圖 v1（自主提案切片）：五張藍圖各對應一種既有建物，送給居民後直接指定她蓋哪一種。
  { id: "blueprint_house",    name: "小屋藍圖",   inputs: [[PLANK, 4], [STONE, 2]],  output_block: BLUEPRINT_HOUSE,    out_count: 1 },
  { id: "blueprint_well",     name: "水井藍圖",   inputs: [[STONE_BRICK, 3], [GLASS, 2]], output_block: BLUEPRINT_WELL, out_count: 1 },
  { id: "blueprint_tower",    name: "瞭望台藍圖", inputs: [[STONE_BRICK, 5]],        output_block: BLUEPRINT_TOWER,    out_count: 1 },
  { id: "blueprint_garden",   name: "花圃藍圖",   inputs: [[LEAVES, 3], [SEEDS, 2]], output_block: BLUEPRINT_GARDEN,   out_count: 1 },
  { id: "blueprint_pavilion", name: "涼亭藍圖",   inputs: [[WOOD, 3], [TORCH, 2]],   output_block: BLUEPRINT_PAVILION, out_count: 1 },
  // 驅影之劍 v1（ROADMAP 887，自主提案切片）——精煉武器的頂點，一擊驅散暗影且雙倍乙太礦。
  // 對齊後端 voxel_craft::WORKBENCH_RECIPES：鐵劍 {5:2,22:3}（3 鐵錠鑄刃 + 2 木頭作握柄）。
  { id: "iron_sword",         name: "鐵劍",       inputs: [[IRON_INGOT, 3], [WOOD, 2]], output_block: SWORD_IRON, out_count: 1 },
];

// wbGrid[0..8]：3×3 共 9 格，0 代表空格，非零代表 block_id。
const wbGrid = new Array(9).fill(0);
// 目前被「拿起」的 block_id（0 = 沒拿）。
let wbPick = 0;

const wbPanelEl  = document.getElementById("wbPanel");
const wbInvGridEl = document.getElementById("wbInvGrid");
const wbGrid3x3El = document.getElementById("wbGrid3x3");
const wbResultEl  = document.getElementById("wbResultSlot");
const wbBtnEl     = document.getElementById("wbBtn");

function openWbPanel() {
  if (!wbPanelEl) return;
  closeBagPanel();
  closeFurnacePanel();
  closeChestPanel();
  releaseMouse(); // 桌機：開面板要放開滑鼠鎖定，游標才能拖放材料
  wbPanelEl.style.display = "flex";
  renderWbPanel();
}
function closeWbPanel() {
  if (!wbPanelEl) return;
  wbPanelEl.style.display = "none";
  wbPick = 0;
}
function wbPanelVisible() {
  return wbPanelEl ? wbPanelEl.style.display === "flex" : false;
}

/**
 * matchWbRecipe——無順序配方比對（純函式，確定性）。
 * 統計 wbGrid 裡的材料次數，比對 WORKBENCH_RECIPES_JS，回傳 {recipe, canCraft} 或 null。
 */
function matchWbRecipe() {
  const gridCounts = new Map();
  for (const bid of wbGrid) {
    if (bid !== 0) gridCounts.set(bid, (gridCounts.get(bid) || 0) + 1);
  }
  if (gridCounts.size === 0) return null;
  for (const r of WORKBENCH_RECIPES_JS) {
    const needed = new Map(r.inputs.map(([b, c]) => [b, c]));
    if (needed.size !== gridCounts.size) continue;
    let match = true;
    for (const [b, c] of needed) {
      if ((gridCounts.get(b) || 0) !== c) { match = false; break; }
    }
    if (!match) continue;
    const canCraft = r.inputs.every(([b, c]) => (myInv.get(b) || 0) >= c);
    return { recipe: r, canCraft };
  }
  return null;
}

/** 渲染工作台物品欄（共用 bag-inv-* CSS）。 */
function renderWbInvGrid() {
  if (!wbInvGridEl) return;
  wbInvGridEl.innerHTML = "";
  const items = [...myInv.entries()].filter(([, cnt]) => cnt > 0);
  if (items.length === 0) {
    const emp = document.createElement("div");
    emp.style.cssText = "color:#605040;font-size:12px;font-style:italic;padding:6px 0";
    emp.textContent = "背包是空的，去挖一些方塊吧";
    wbInvGridEl.appendChild(emp);
    return;
  }
  for (const [bid, cnt] of items) {
    const slot = document.createElement("div");
    slot.className = "bag-inv-slot" + (wbPick === bid ? " picked" : "");
    slot.appendChild(makeSwatchEl(bid, "bag-inv-sw"));
    const name = document.createElement("div");
    name.className = "bag-inv-name";
    name.textContent = BLOCK_NAME[bid] || "?";
    const cntEl = document.createElement("div");
    cntEl.className = "bag-inv-cnt";
    cntEl.textContent = "×" + cnt;
    slot.appendChild(name); slot.appendChild(cntEl);
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      wbPick = (wbPick === bid) ? 0 : bid;
      renderWbPanel();
    });
    wbInvGridEl.appendChild(slot);
  }
}

/** 渲染 3×3 合成格 + 結果格。 */
function renderWbCraftArea() {
  if (!wbGrid3x3El || !wbResultEl) return;
  wbGrid3x3El.innerHTML = "";
  for (let i = 0; i < 9; i++) {
    const bid = wbGrid[i];
    const slot = document.createElement("div");
    slot.className = "wb-grid-slot" + (bid !== 0 ? " filled" : "");
    if (bid !== 0) {
      slot.appendChild(makeSwatchEl(bid, "wb-grid-sw"));
      const lbl = document.createElement("div");
      lbl.className = "wb-grid-lbl";
      lbl.textContent = BLOCK_NAME[bid] || "?";
      slot.appendChild(lbl);
    }
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      if (wbPick !== 0 && bid === 0) {
        wbGrid[i] = wbPick; wbPick = 0;
      } else if (wbPick !== 0 && bid !== 0) {
        wbGrid[i] = wbPick; wbPick = bid;
      } else if (wbPick === 0 && bid !== 0) {
        wbPick = bid; wbGrid[i] = 0;
      }
      renderWbPanel();
    });
    wbGrid3x3El.appendChild(slot);
  }
  // 結果格
  wbResultEl.className = "";
  wbResultEl.innerHTML = "";
  const match = matchWbRecipe();
  if (match) {
    const r = match.recipe;
    wbResultEl.classList.add(match.canCraft ? "has-result" : "no-material");
    wbResultEl.appendChild(makeSwatchEl(r.output_block, "bag-res-sw"));
    const nm = document.createElement("div"); nm.className = "bag-res-name"; nm.textContent = r.name;
    const ct = document.createElement("div"); ct.className = "bag-res-cnt"; ct.textContent = "×" + r.out_count;
    wbResultEl.appendChild(nm); wbResultEl.appendChild(ct);
    if (!match.canCraft) {
      const warn = document.createElement("div");
      warn.style.cssText = "font-size:9px;color:#ff8060;margin-top:2px";
      warn.textContent = "材料不足";
      wbResultEl.appendChild(warn);
    }
  }
}

/** 渲染整個工作台面板（物品欄 + 合成格）。 */
function renderWbPanel() {
  renderWbInvGrid();
  renderWbCraftArea();
}

// 結果格點擊：送出工作台合成請求。
if (wbResultEl) wbResultEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  const match = matchWbRecipe();
  if (!match || !match.canCraft || !wsReady) return;
  ws.send(JSON.stringify({ t: "craft", recipe_id: match.recipe.id }));
});

// 清除合成格。
const wbClearBtnEl = document.getElementById("wbClearBtn");
if (wbClearBtnEl) wbClearBtnEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  wbGrid.fill(0); wbPick = 0;
  renderWbPanel();
});

// 工作台按鈕（🔨）點擊開啟面板（備用：也可以右鍵對準工作台方塊開啟）。
if (wbBtnEl) wbBtnEl.addEventListener("click", (e) => {
  if (wbPanelVisible()) { closeWbPanel(); } else { openWbPanel(); }
  e.stopPropagation();
});

// 關閉鈕（✕）。
const wbCloseEl = document.getElementById("wbClose");
if (wbCloseEl) wbCloseEl.addEventListener("click", closeWbPanel);

// 點面板外關閉。
document.addEventListener("pointerdown", (e) => {
  if (wbPanelVisible()) {
    if (wbPanelEl && !wbPanelEl.contains(e.target) && e.target !== wbBtnEl) closeWbPanel();
  }
});

// ── inv_update / inv_sync 後也刷新工作台面板（若已開啟）────────────────────
// 注意：inv_sync 和 inv_update handler 在 WS onmessage 裡，
//       craft_ok / craft_fail 的刷新邏輯已在 WS handler 那段處理過。

// ── 熔爐面板 v2（ROADMAP 712）──────────────────────────────────────────────────
// 改成跟背包 2×2 / 工作台 3×3 一致的「拿起→放入格子」格子式互動（原 v1 是按鈕清單，
// 跟另外兩層合成介面手感不一致；格數取 3——熔爐配方裡最耗格的 smelt_stone
// 需要「同一種材料×3」湊數量，比清單更貼近玩家已經上手的操作邏輯）。
const FURNACE_RECIPES_JS = [
  { id: "smelt_stone", name: "拋光石",       inputs: [[STONE, 3]],               output_block: SMOOTH_STONE, out_count: 3 },
  { id: "smelt_glass", name: "玻璃（冶煉）", inputs: [[SAND,  2]],               output_block: GLASS,        out_count: 3 },
  { id: "smelt_brick", name: "石磚（冶煉）", inputs: [[STONE, 2]],               output_block: STONE_BRICK,  out_count: 4 },
  // 鐵錠 v1（ROADMAP 683）：1 鐵礦 + 1 煤礦（燃料）→ 2 鐵錠
  { id: "smelt_iron",  name: "鐵錠",         inputs: [[IRON_ORE, 1], [COAL_ORE, 1]], output_block: IRON_INGOT, out_count: 2 },
  // 烤魚 v1：1 生小魚 → 1 烤魚（把垂釣漁獲烤成居民最愛的美味贈禮）
  { id: "smelt_fish",  name: "烤魚",         inputs: [[FISH, 1]],                    output_block: COOKED_FISH, out_count: 1 },
  { id: "smelt_potato",name: "烤地薯",       inputs: [[POTATO, 1]],                  output_block: BAKED_POTATO, out_count: 1 },
  // 莓果醬 v1（808）：3 莓果 → 1 莓果醬（把多年生莓園的莓果小火慢熬成甜點）
  { id: "smelt_jam",   name: "莓果醬",       inputs: [[BERRY, 3]],                   output_block: JAM,          out_count: 1 },
];

// furnaceGrid[0..2]：3 格輸入，0 代表空格，非零代表 block_id。
// 3 格而非 2 格：smelt_stone 單一配方就需要 3 顆石頭（同一材料佔滿 3 格才湊得出數量），
// 熔爐所有配方裡最多材料格數的就是它，故取 3 為格數上限（沿用背包/工作台「格數=最大配方所需」的設計）。
const furnaceGrid = [0, 0, 0];
// 目前被「拿起」的 block_id（0 = 沒拿）。
let furnacePick = 0;

const furnacePanelEl   = document.getElementById("furnacePanel");
const furnaceBtnEl     = document.getElementById("furnaceBtn");
const furnaceInvGridEl = document.getElementById("furnaceInvGrid");
const furnaceGridEl    = document.getElementById("furnaceGrid2");
const furnaceResultEl  = document.getElementById("furnaceResultSlot");

function openFurnacePanel() {
  if (!furnacePanelEl) return;
  closeBagPanel();
  closeWbPanel();
  closeChestPanel();
  releaseMouse(); // 桌機：開面板要放開滑鼠鎖定，游標才能點冶煉
  furnacePanelEl.style.display = "flex";
  renderFurnacePanel();
}
function closeFurnacePanel() {
  if (!furnacePanelEl) return;
  furnacePanelEl.style.display = "none";
  furnacePick = 0; // 關面板時清除選取
}
function furnacePanelVisible() {
  return furnacePanelEl ? furnacePanelEl.style.display === "flex" : false;
}

/**
 * matchFurnaceRecipe——無順序配方比對（純函式，確定性，與 matchBagRecipe/matchWbRecipe 同手法）。
 * 統計 furnaceGrid 裡的材料次數，比對 FURNACE_RECIPES_JS，回傳 {recipe, canCraft} 或 null。
 */
function matchFurnaceRecipe() {
  const gridCounts = new Map();
  for (const bid of furnaceGrid) {
    if (bid !== 0) gridCounts.set(bid, (gridCounts.get(bid) || 0) + 1);
  }
  if (gridCounts.size === 0) return null;
  for (const r of FURNACE_RECIPES_JS) {
    const needed = new Map(r.inputs.map(([b, c]) => [b, c]));
    if (needed.size !== gridCounts.size) continue;
    let match = true;
    for (const [b, c] of needed) {
      if ((gridCounts.get(b) || 0) !== c) { match = false; break; }
    }
    if (!match) continue;
    const canCraft = r.inputs.every(([b, c]) => (myInv.get(b) || 0) >= c);
    return { recipe: r, canCraft };
  }
  return null;
}

/** 渲染熔爐物品欄（點選拿起，與背包/工作台一致）。 */
function renderFurnaceInvGrid() {
  if (!furnaceInvGridEl) return;
  furnaceInvGridEl.innerHTML = "";
  const items = [...myInv.entries()].filter(([, cnt]) => cnt > 0);
  if (items.length === 0) {
    const emp = document.createElement("div");
    emp.style.cssText = "color:#605040;font-size:12px;font-style:italic;padding:6px 0";
    emp.textContent = "背包是空的，去挖一些方塊吧";
    furnaceInvGridEl.appendChild(emp);
    return;
  }
  for (const [bid, cnt] of items) {
    const slot = document.createElement("div");
    slot.className = "bag-inv-slot" + (furnacePick === bid ? " picked" : "");
    slot.appendChild(makeSwatchEl(bid, "bag-inv-sw"));
    const name = document.createElement("div");
    name.className = "bag-inv-name";
    name.textContent = BLOCK_NAME[bid] || "?";
    const cntEl = document.createElement("div");
    cntEl.className = "bag-inv-cnt";
    cntEl.textContent = "×" + cnt;
    slot.appendChild(name); slot.appendChild(cntEl);
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      furnacePick = (furnacePick === bid) ? 0 : bid;
      renderFurnacePanel();
    });
    furnaceInvGridEl.appendChild(slot);
  }
}

/** 渲染 3 格冶煉格 + 結果格。 */
function renderFurnaceCraftArea() {
  if (!furnaceGridEl || !furnaceResultEl) return;
  furnaceGridEl.innerHTML = "";
  for (let i = 0; i < 3; i++) {
    const bid = furnaceGrid[i];
    const slot = document.createElement("div");
    slot.className = "furnace-grid-slot" + (bid !== 0 ? " filled" : "");
    if (bid !== 0) {
      slot.appendChild(makeSwatchEl(bid, "furnace-grid-sw"));
      const lbl = document.createElement("div");
      lbl.className = "furnace-grid-lbl";
      lbl.textContent = BLOCK_NAME[bid] || "?";
      slot.appendChild(lbl);
    }
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      if (furnacePick !== 0 && bid === 0) {
        furnaceGrid[i] = furnacePick; furnacePick = 0;
      } else if (furnacePick !== 0 && bid !== 0) {
        furnaceGrid[i] = furnacePick; furnacePick = bid;
      } else if (furnacePick === 0 && bid !== 0) {
        furnacePick = bid; furnaceGrid[i] = 0;
      }
      renderFurnacePanel();
    });
    furnaceGridEl.appendChild(slot);
  }
  // 結果格
  furnaceResultEl.className = "";
  furnaceResultEl.innerHTML = "";
  const match = matchFurnaceRecipe();
  if (match) {
    const r = match.recipe;
    furnaceResultEl.classList.add(match.canCraft ? "has-result" : "no-material");
    furnaceResultEl.appendChild(makeSwatchEl(r.output_block, "bag-res-sw"));
    const nm = document.createElement("div"); nm.className = "bag-res-name"; nm.textContent = r.name;
    const ct = document.createElement("div"); ct.className = "bag-res-cnt"; ct.textContent = "×" + r.out_count;
    furnaceResultEl.appendChild(nm); furnaceResultEl.appendChild(ct);
    if (!match.canCraft) {
      const warn = document.createElement("div");
      warn.style.cssText = "font-size:9px;color:#ff8060;margin-top:2px";
      warn.textContent = "材料不足";
      furnaceResultEl.appendChild(warn);
    }
  }
}

/** 渲染整個熔爐面板（物品欄 + 冶煉格）。 */
function renderFurnacePanel() {
  renderFurnaceInvGrid();
  renderFurnaceCraftArea();
}

// 結果格點擊：送出冶煉請求。
if (furnaceResultEl) furnaceResultEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  const match = matchFurnaceRecipe();
  if (!match || !match.canCraft || !wsReady) return;
  ws.send(JSON.stringify({ t: "craft", recipe_id: match.recipe.id }));
});

// 清除冶煉格按鈕。
const furnaceClearBtnEl = document.getElementById("furnaceClearBtn");
if (furnaceClearBtnEl) furnaceClearBtnEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  furnaceGrid.fill(0);
  furnacePick = 0;
  renderFurnacePanel();
});

// 熔爐 HUD 按鈕（🔥）開閉面板。
if (furnaceBtnEl) furnaceBtnEl.addEventListener("click", (e) => {
  if (furnacePanelVisible()) { closeFurnacePanel(); } else { openFurnacePanel(); }
  e.stopPropagation();
});

// 關閉鈕（✕）。
const furnaceCloseEl = document.getElementById("furnaceClose");
if (furnaceCloseEl) furnaceCloseEl.addEventListener("click", closeFurnacePanel);

// 點面板外關閉。
document.addEventListener("pointerdown", (e) => {
  if (furnacePanelVisible()) {
    if (furnacePanelEl && !furnacePanelEl.contains(e.target) && e.target !== furnaceBtnEl) closeFurnacePanel();
  }
});

// ── 箱子面板 v1（ROADMAP 692）──────────────────────────────────────────────────
// 箱子世界座標（開啟中的箱子）；null = 沒有打開的箱子。
let _chestPos = null;
// 箱子當前內容（從 chest_view 伺服器訊息更新）：[{id, count}, ...]。
let _chestItems = [];

const chestPanelEl = document.getElementById("chestPanel");
const chestInvGridEl = document.getElementById("chestInvGrid");
const chestBoxGridEl = document.getElementById("chestBoxGrid");

function chestPanelVisible() {
  return chestPanelEl ? chestPanelEl.style.display === "flex" : false;
}

/** 開啟箱子面板：傳送 open_chest 請求，伺服器回 chest_view 後才真正渲染內容。 */
function openChestPanel(bx, by, bz) {
  if (!chestPanelEl) return;
  closeBagPanel();
  closeWbPanel();
  closeFurnacePanel();
  releaseMouse();
  _chestPos = { x: bx, y: by, z: bz };
  _chestItems = [];
  chestPanelEl.style.display = "flex";
  renderChestPanel();
  if (wsReady) ws.send(JSON.stringify({ t: "open_chest", x: bx, y: by, z: bz }));
}

function closeChestPanel() {
  if (!chestPanelEl) return;
  chestPanelEl.style.display = "none";
  _chestPos = null;
}

/**
 * 渲染箱子面板——分上下兩區：
 * - 上：箱子內容（_chestItems）；點某格→取出 1 個。
 * - 下：玩家背包（myInv）；點某格→放入 1 個（排除工具純物品）。
 */
function renderChestPanel() {
  if (!chestPanelEl) return;
  // 箱子內容區。
  if (chestBoxGridEl) {
    chestBoxGridEl.innerHTML = "";
    if (_chestItems.length === 0) {
      const empty = document.createElement("span");
      empty.className = "chest-empty-hint";
      empty.textContent = "（空箱子——從下方背包點格子存入物品）";
      chestBoxGridEl.appendChild(empty);
    } else {
      for (const { id, count } of _chestItems) {
        const sl = makeSwatchEl(id, "chest-box-slot");
        const cnt = document.createElement("span");
        cnt.className = "inv-count";
        cnt.textContent = "×" + count;
        sl.appendChild(cnt);
        sl.title = (BLOCK_NAME[id] || "方塊") + " ×" + count + "\n（點擊取出 1 個）";
        sl.addEventListener("click", () => {
          if (!_chestPos || !wsReady) return;
          ws.send(JSON.stringify({ t: "chest_take", x: _chestPos.x, y: _chestPos.y, z: _chestPos.z, item_id: id, count: 1 }));
        });
        chestBoxGridEl.appendChild(sl);
      }
    }
  }
  // 玩家背包區（只顯示可存入箱子的物品：排除種子 14、麵包 19 等純物品也可存，工具只存 item_id≤41）。
  if (chestInvGridEl) {
    chestInvGridEl.innerHTML = "";
    const EXCLUDE_FROM_CHEST = new Set([0]); // 只排 Air
    for (const [bid, cnt] of myInv) {
      if (EXCLUDE_FROM_CHEST.has(bid) || cnt <= 0) continue;
      const sl = makeSwatchEl(bid, "chest-inv-slot");
      const c = document.createElement("span");
      c.className = "inv-count";
      c.textContent = "×" + cnt;
      sl.appendChild(c);
      sl.title = (BLOCK_NAME[bid] || "物品") + " ×" + cnt + "\n（點擊存入 1 個）";
      sl.addEventListener("click", () => {
        if (!_chestPos || !wsReady) return;
        ws.send(JSON.stringify({ t: "chest_put", x: _chestPos.x, y: _chestPos.y, z: _chestPos.z, item_id: bid, count: 1 }));
      });
      chestInvGridEl.appendChild(sl);
    }
    if (chestInvGridEl.children.length === 0) {
      const hint = document.createElement("span");
      hint.className = "chest-empty-hint";
      hint.textContent = "（背包空的）";
      chestInvGridEl.appendChild(hint);
    }
  }
}

const chestCloseEl = document.getElementById("chestClose");
if (chestCloseEl) chestCloseEl.addEventListener("click", closeChestPanel);

// 點面板外關閉箱子面板。
document.addEventListener("pointerdown", (e) => {
  if (chestPanelVisible()) {
    if (chestPanelEl && !chestPanelEl.contains(e.target)) closeChestPanel();
  }
});

/** 簡短綠色提示（合成成功用；區別於 showErr 紅色錯誤）。 */
function showMsg(text) {
  const el = document.getElementById("msg");
  if (!el) return;
  el.textContent = text;
  el.style.display = "block";
  clearTimeout(el._hideTimer);
  el._hideTimer = setTimeout(() => { el.style.display = "none"; }, 3000);
}

// 對外暴露一點狀態，方便真瀏覽器 QA 讀數驗證。
window.__voxel = {
  get chunks() { return chunks.size; },
  get meshes() { return meshes.size; },
  get fps() { return fps; },
  get player() { return player; },
  // ── 時段指示器 v1 QA 用（ROADMAP 896）：讀徽章目前顯示、就地撥鐘（純視覺、無權威影響）──
  get clock() {
    return {
      cls: clockEl ? clockEl.className : "",
      name: clockEl ? (clockEl.querySelector(".cl-name")?.textContent || "") : "",
      icon: clockEl ? (clockEl.querySelector(".cl-icon")?.textContent || "") : "",
      time: clockEl ? (clockEl.querySelector(".cl-time")?.textContent || "") : "",
      worldTime,
    };
  },
  // 就地撥鐘：只動本地 worldTime＋天色＋徽章（不送任何訊息、不影響伺服器權威時間），供 QA 驗徽章對映。
  qaSetWorldTime(t) { worldTime = t; updateSkyAndLight(worldTime); updateClock(); return this.clock; },
  // ── 踏階平滑 QA 用：讀視覺 Y（平滑後）與補間偏移 ──
  get stepSmooth() { return stepSmooth; },
  get visualY() { return player.y - stepSmooth; },
  // ── 視角模式 QA 用（滑鼠鎖定視角 + 第一/三人稱切換）──
  get viewMode() { return viewMode; },
  get camPitch() { return camPitch; },
  get pointerLocked() { return pointerLocked; },
  get bodyVisible() { return bodyMesh.visible; },
  get playerId() { return myId; },
  // ── 手持工具可見 v1 QA 用（自主提案切片）：讀自己/指定其他玩家目前手持顯示狀態 ──
  get myHeld() { return { id: myAvatar.heldId, visible: myAvatar.heldMesh.visible }; },
  otherHeld(id) {
    const ent = others.get(id);
    return ent ? { id: ent.av.heldId, visible: ent.av.heldMesh.visible } : null;
  },
  setHeldItem(itemId) { setHeldItem(myAvatar, itemId); return this.myHeld; },
  // ── 暗影生物 v1 QA 用：讀暗影實體清單＋把視角轉向某座標（截圖/準心驗證）──
  get shadows() {
    return [...shadowEnts.entries()].map(([id, e]) => ({
      id, x: e.group.position.x, y: e.group.position.y, z: e.group.position.z, visible: e.group.visible,
    }));
  },
  get shadowPuffCount() { return shadowPuffs.length; },
  // 把視角轉向世界座標（等同玩家自己轉滑鼠，純視覺、無任何權威影響）。
  lookTowards(x, y, z) {
    const dx = x - player.x, dy = y - (player.y + 1.5), dz = z - player.z;
    player.yaw = Math.atan2(-dx, -dz);
    camPitch = -Math.atan2(dy, Math.hypot(dx, dz));
    clampPitch();
  },
  // 世界環境氛圍 v1（905）QA 用：讀此刻活著的蝴蝶／螢火蟲數＋各自位置（驗依時段真的生成、供取景）。
  get ambientCounts() {
    const bp = [], fp = [];
    for (const c of butterflies) if (c.alive) bp.push({ x: c.sp.position.x, y: c.sp.position.y, z: c.sp.position.z });
    for (const c of fireflies) if (c.alive) fp.push({ x: c.sp.position.x, y: c.sp.position.y, z: c.sp.position.z });
    return { butterflies: bp.length, fireflies: fp.length, butterflyPos: bp, fireflyPos: fp };
  },
  // 挖擊一隻暗影（headless QA 無法 pointer lock 時的替代路徑；與點擊送同一則訊息，
  // 打不打得到仍由伺服器權威驗 reach/節奏——玩家用 console 呼叫也佔不到任何便宜）。
  qaShadowHit(id) { ws.send(JSON.stringify({ t: "shadow_hit", id })); },
  // 玩家生存指標 QA 用：讀目前血/飢窄列的顯示寬度（%）＋餓瘋樣式，驗 HUD 真的隨 player_stats 動。
  get statsHud() {
    const hp = document.getElementById("statHealthFill");
    const food = document.getElementById("statHungerFill");
    const box = document.getElementById("statHunger");
    return {
      healthPct: hp ? parseFloat(hp.style.width) || 0 : -1,
      hungerPct: food ? parseFloat(food.style.width) || 0 : -1,
      starving: box ? box.classList.contains("starving") : false,
      placeLabel: document.getElementById("place")?.textContent || "",
    };
  },
  // 玩家生存指標 QA 用：直接餵一則 player_stats 給 HUD（驗顯示隨後端狀態動；正式一律走 ws）。
  _qaFeedStats(m) { updateStatsHud(m); return this.statsHud; },
  // 玩家生存指標 QA 用：觸發一次受傷紅暈。
  _qaFlashDamage() { flashDamage(); return true; },
  // 玩家生存指標 QA 用：觸發溫柔重生覆蓋（顯示暖心提示、拉回座標）。
  _qaRespawn(x, y, z, msg) { doGentleRespawn(x, y, z, msg); return true; },
  // 玩家生存指標 QA 用：走真實吃流程（發 eat 給伺服器；伺服器權威回 eat_ok/inv_update/player_stats）。
  _qaEat() { tryEatDish(); return true; },
  // 玩家生存指標 QA 用：授予食物並選到手上（把食物塞進一個空快捷格再選它），驗「手持食物→放置鈕變吃」。
  _qaGrantAndHold(itemId, count) {
    if (ws && ws.readyState === 1) ws.send(JSON.stringify({ t: "qa_grant", item_id: itemId, count: count || 1 }));
    // 找一個空格塞食物並選它。
    let slot = HOTBAR.indexOf(itemId);
    if (slot < 0) { slot = HOTBAR.indexOf(AIR); if (slot < 0) slot = 0; assignToHotbar(slot, itemId); buildHotbar(); }
    selectSlot(slot);
    return { slot, held: selectedBlock() };
  },
  // QA 用：把稱號牌掛到自己頭頂，驗證新 avatar 的頭頂貼合（正式流程稱號由後端 title 決定，不受影響）。
  _qaSetMyTitle(t) {
    if (myTitleSprite) { bodyMesh.remove(myTitleSprite); myTitleSprite = null; }
    if (t) { myTitleSprite = makeTitleSprite(t); bodyMesh.add(myTitleSprite); }
    return !!t;
  },
  toggleViewMode() { toggleViewMode(); return viewMode; },
  setCamPitch(p) { camPitch = p; clampPitch(); return camPitch; },
  setYaw(y) { player.yaw = y; return player.yaw; },
  get camPos() { return { x: camera.position.x, y: camera.position.y, z: camera.position.z }; },
  // 乙太方界 AI 居民（QA 用）：數量 + 位置/名字/說的話快照。
  get residentCount() { return residents.size; },
  residentInfo() {
    return [...residents.entries()].map(([id, e]) => ({
      id, name: e.lastName, say: e.lastSay,
      x: e.group.position.x, y: e.group.position.y, z: e.group.position.z,
      visible: e.group.visible,
    }));
  },
  // ── 對話 QA 用：列居民 id、直接對某居民送一句、讀最近回覆 ──
  residentIds() { return [...residents.keys()]; },
  talkTo(rid, text) {
    const ent = residents.get(rid);
    openChat(rid, ent && ent.lastName);
    sendTalk(text);
    return chatRid;
  },
  get lastTalkReply() { return lastTalkReply; },
  closeChat() { closeChat(); },
  // ── embodied 靠近說話 v1 QA 用：範圍說話 + 讀自己頭上泡泡狀態 ──
  speak(text) { sendSpeak(text); return myBubbleText; },
  get myBubbleText() { return myBubble.visible ? myBubbleText : ""; },
  get myBubbleVisible() { return myBubble.visible; },
  // ── 麥塊式聊天記錄窗 QA 用：注入一句、讀 log 內容/HTML、開合狀態 ──
  chatLogAppend(kind, speaker, text, colorKey) { chatLogAppend(kind, speaker, text, colorKey); },
  get chatLogLines() { return chatLogEntries.map((e) => ({ kind: e.kind, speaker: e.speaker, text: e.text })); },
  get chatLogHTML() { return chatLogBodyEl ? chatLogBodyEl.innerHTML : ""; },
  get chatLogExpanded() { return chatLogExpanded; },
  setChatLogExpanded(on) { chatLogSetExpanded(on); },
  get chatLogFaded() { return chatLogWinEl ? chatLogWinEl.classList.contains("faded") : false; },
  // ── 日記 QA 用（ROADMAP 650）──
  openDiary(rid) { return openDiary(rid); },
  closeDiary() { closeDiary(); },
  get diaryVisible() { return diaryEl ? diaryEl.style.display !== "none" : false; },
  // ── 日記牆 QA 用（ROADMAP 674）──
  openDiaryWall() { return openDiaryWall(); },
  closeDiaryWall() { closeDiaryWall(); },
  get diaryWallVisible() { return diaryWallVisible; },
  renderDiaryWall(p) { renderDiaryWall(p); return diaryWallBodyEl && diaryWallBodyEl.innerHTML; },
  // ── 動態 Feed QA 用（ROADMAP 655）──
  openFeed() { return openFeed(); },
  closeFeed() { closeFeed(); },
  get feedVisible() { return feedVisible; },
  renderFeed(ev) { renderFeed(ev); return feedBodyEl && feedBodyEl.innerHTML; },
  // ── 居民羅盤 QA 用（ROADMAP 705）──
  openCompass() { return openCompass(); },
  closeCompass() { closeCompass(); },
  get compassVisible() { return compassVisible; },
  renderCompassPanel() { renderCompassPanel(); return compassBodyEl && compassBodyEl.innerHTML; },
  worldBearing(px, pz, rx, rz) { return worldBearing(px, pz, rx, rz); },
  compassRelativeDeg(px, pz, rx, rz, yaw) { return compassRelativeDeg(px, pz, rx, rz, yaw); },
  // ── 雷達小地圖 QA 用（ROADMAP 820）──
  radarPoint(dist, relDeg, rangeUnits, radiusPx) { return radarPoint(dist, relDeg, rangeUnits, radiusPx); },
  // ── 居民交情網 QA 用（ROADMAP 708）──
  openRelations() { return openRelations(); },
  closeRelations() { closeRelations(); },
  get relationsVisible() { return relationsVisible; },
  refreshRelations() { return refreshRelations(); },
  renderRelationsPanel(rows) { renderRelationsPanel(rows); return relationsBodyEl && relationsBodyEl.innerHTML; },
  sortRelationRows(rows) { return sortRelationRows(rows); },
  // ── 小圈子攤開 QA 用（自主提案切片，接續 708+711）──
  renderCliquesSection(cliques) { renderCliquesSection(cliques); return cliquesBodyEl && cliquesBodyEl.innerHTML; },
  // ── 居民技能簿 QA 用（ROADMAP 719）──
  openSkills() { return openSkills(); },
  closeSkills() { closeSkills(); },
  get skillsVisible() { return skillsVisible; },
  refreshSkills() { return refreshSkills(); },
  renderSkillsPanel(rows) { renderSkillsPanel(rows); return skillsBodyEl && skillsBodyEl.innerHTML; },
  // ── 玩家里程碑 QA 用（ROADMAP 724）──
  openMilestones() { return openMilestones(); },
  closeMilestones() { closeMilestones(); },
  get milestonesVisible() { return milesVisible; },
  refreshMilestones() { return refreshMilestones(); },
  renderMilestonesPanel(rows) { renderMilestonesPanel(rows); return milesBodyEl && milesBodyEl.innerHTML; },
  // ── 村莊地圖 QA 用（自主提案切片，ROADMAP 837）──
  openVillageMap() { return openVillageMap(); },
  closeVillageMap() { closeVillageMap(); },
  get villageMapVisible() { return mapVisible; },
  refreshVillageMap() { return refreshVillageMap(); },
  renderVillageMap() { return renderVillageMap(); },
  villageMapPoint(worldX, worldZ, centerX, centerZ, rangeUnits, radiusPx) {
    return villageMapPoint(worldX, worldZ, centerX, centerZ, rangeUnits, radiusPx);
  },
  setVillageMapDataForTest(data) { mapData = data; },
  // ── 探索紀事 QA 用（自主提案切片，接續 838/839）──
  openDiscoveries() { return openDiscoveries(); },
  closeDiscoveries() { closeDiscoveries(); },
  get discoveriesVisible() { return discVisible; },
  refreshDiscoveries() { return refreshDiscoveries(); },
  renderDiscoveryPanel(data) { renderDiscoveryPanel(data); return discBodyEl && discBodyEl.innerHTML; },
  // ── 好感度 QA 用（ROADMAP 656）──
  affinityEmoji(count) { return affinityEmoji(count); },
  get myAffinity() { return Object.fromEntries(myAffinity); },
  refreshAffinity() { return refreshAffinity(); },
  // ── 採集背包 QA 用（ROADMAP 657）──
  get myInv() { return Object.fromEntries(myInv); },
  setInvForTest(bid, cnt) { if (cnt > 0) myInv.set(bid, cnt); else myInv.delete(bid); updateInvHud(); },
  updateInvHud() { updateInvHud(); },
  // ── 背包合成格 QA 用（ROADMAP 664）──
  get bagPanelVisible() { return bagPanelVisible(); },
  openBagPanel() { openBagPanel(); },
  closeBagPanel() { closeBagPanel(); },
  renderBagPanel() { renderBagPanel(); },
  get RECIPES_JS() { return RECIPES_JS; },
  get bagGrid() { return [...bagGrid]; },
  get bagPick() { return bagPick; },
  matchBagRecipe() { return matchBagRecipe(); },
  setBagGrid(slots) { slots.forEach((v, i) => { if (i < 4) bagGrid[i] = v; }); renderBagPanel(); },
  PLANK, STONE_BRICK, GLASS,
  // 種田 v1 常數 + QA 介面
  FARM_SOIL, FARM_SOIL_SEEDED, WHEAT_MATURE, SEEDS,
  // 第二種作物 v1 常數 QA 用
  CARROT_SEEDED, CARROT_MATURE, CARROT_SEEDS, CARROT,
  // 第三種作物 v1 常數 QA 用
  POTATO_SEEDED, POTATO_MATURE, POTATO_SEEDS, POTATO,
  // ── 工作台 3×3 QA 用（ROADMAP 665）──
  WORKBENCH,
  get wbPanelVisible() { return wbPanelVisible(); },
  openWbPanel() { openWbPanel(); },
  closeWbPanel() { closeWbPanel(); },
  renderWbPanel() { renderWbPanel(); },
  get WORKBENCH_RECIPES_JS() { return WORKBENCH_RECIPES_JS; },
  get wbGrid() { return [...wbGrid]; },
  get wbPick() { return wbPick; },
  matchWbRecipe() { return matchWbRecipe(); },
  setWbGrid(slots) { slots.forEach((v, i) => { if (i < 9) wbGrid[i] = v; }); renderWbPanel(); },
  // ── 熔爐 v1 QA 用（ROADMAP 666）──
  FURNACE, SMOOTH_STONE,
  get furnacePanelVisible() { return furnacePanelVisible(); },
  openFurnacePanel() { openFurnacePanel(); },
  closeFurnacePanel() { closeFurnacePanel(); },
  renderFurnacePanel() { renderFurnacePanel(); },
  get FURNACE_RECIPES_JS() { return FURNACE_RECIPES_JS; },
  // ── 熔爐冶煉格子化 QA 用（ROADMAP 712）──
  get furnaceGrid() { return [...furnaceGrid]; },
  get furnacePick() { return furnacePick; },
  matchFurnaceRecipe() { return matchFurnaceRecipe(); },
  setFurnaceGrid(slots) { slots.forEach((v, i) => { if (i < 3) furnaceGrid[i] = v; }); renderFurnacePanel(); },
  // ── 贈禮 v1 QA 介面（ROADMAP 660）──
  giftPickItem(inv) { return giftPickItem(inv); },
  updateGiftBtn() { updateGiftBtn(); },
  get giftBtnText() { const e = document.getElementById("chatGift"); return e ? e.textContent : ""; },
  get giftBtnEmpty() { const e = document.getElementById("chatGift"); return e ? e.classList.contains("gift-empty") : false; },
  GIFT_EXCLUDED: [...GIFT_EXCLUDED],
  // ── 麵包 v1 QA 用（ROADMAP 668）──
  WHEAT, BREAD,
  // ── 晝夜循環 v1 QA 用（ROADMAP 661）──
  get worldTime() { return worldTime; },
  updateSkyAndLight(t) { updateSkyAndLight(t); },
  get skyColor() { const c = scene.background; return { r: c.r, g: c.g, b: c.b }; },
  get sunIntensity() { return sun.intensity; },
  get hemiIntensity() { return hemi.intensity; },
  // ── 下雨天氣 v1 QA 用（ROADMAP 700）──
  get isRaining() { return isRaining; },
  set isRaining(v) { isRaining = !!v; updateSkyAndLight(worldTime); },
  get rainVisible() { return rainPoints.visible; },
  updateRain(dt) { updateRain(dt); },
  // ── 雨後彩虹 v1 QA 用（ROADMAP 780）──
  get rainbowActive() { return rainbowActive; },
  set rainbowActive(v) { rainbowActive = !!v; },
  get rainbowVisible() { return rainbowGroup.visible; },
  get rainbowAlpha() { return rainbowAlpha; },
  updateRainbow(dt) { updateRainbow(dt); },
  // ── 流星許願 v1 QA 用（ROADMAP 904）──
  get meteorActive() { return meteorActive; },
  set meteorActive(v) { meteorActive = !!v; },
  get meteorVisible() { return meteorMesh.visible; },
  get meteorOpacity() { return meteorMat.opacity; },
  triggerMeteor() { triggerMeteor(); return meteorMesh.visible; },
  updateMeteor(dt) { updateMeteor(dt); },
  // ── 四季樹葉 v1 QA 用（自主提案切片）──：切季節、讀當季樹葉色、就地重建驗證換色
  get worldSeason() { return worldSeason; },
  setSeason(s) { if (s !== worldSeason) { worldSeason = s; updateSkyAndLight(worldTime); remeshForSeason(); } return worldSeason; },
  foliageLeafColor(s) { return foliageLeafColor(s == null ? worldSeason : s); },
  // ── 真瀏覽器 QA 用：讀準心目標、讀方塊、觸發破壞/放置、選方塊 ──
  get target() { return target; },
  getBlock(x, y, z) { return getRaw(x, y, z); },
  // 純視覺 QA 用：就地寫一個本地方塊並重建 mesh（伺服器仍權威，串流會覆蓋回真值）。
  // 供「裝飾植物十字貼片」等渲染 QA 直接擺花草截圖，不必先湊背包/放置流程。
  _qaSetBlock(x, y, z, id) { setLocalBlock(x, y, z, id); return getRaw(x, y, z); },
  CROSS_PLANTS: [...CROSS_PLANTS],
  WILDFLOWER_RED, WILDFLOWER_YELLOW, WILDFLOWER_BLUE, BERRY_BUSH,
  // 純視覺 QA 用：切換「裝飾植物走舊的整格立方體」以便對比截圖，並就地重建所有已載入 chunk。
  _qaSetCubePlants(on) {
    window.__qaCubePlants = !!on;
    for (const k of chunks.keys()) { const [a, b, c] = k.split(",").map(Number); markDirty(a, b, c); }
    return window.__qaCubePlants;
  },
  doBreak() { return breakAtTarget(); },
  doPlace() { return placeAtTarget(); },
  selectSlotByBlock(b) {
    // 麥塊 9 格快捷欄：若該方塊已在欄上就選那格；不在就指派進當前格（QA/測試可選任意方塊）。
    let i = HOTBAR.indexOf(b);
    if (i < 0) { i = selectedSlot; assignToHotbar(i, b); }
    selectSlot(i);
    return selectedBlock();
  },
  // ── 快捷欄麥塊化 QA 用 ──
  get HOTBAR() { return [...HOTBAR]; },
  get HOTBAR_SIZE() { return HOTBAR_SIZE; },
  get selectedSlot() { return selectedSlot; },
  assignToHotbar(slot, blockId) { assignToHotbar(slot, blockId); return [...HOTBAR]; },
  // ── 流動水階梯視覺 QA 用：注入一塊測試 chunk + 讀水面高度，驗證階梯遞減 ──
  waterTopH(b) { return waterTopH(b); },
  injectChunkForTest(cx, cy, cz, bytes) {
    chunks.set(ckey(cx, cy, cz), Uint8Array.from(bytes));
    markDirty(cx, cy, cz);
    return chunks.get(ckey(cx, cy, cz)).length;
  },
  WATER, WATER_FLOW_BASE, CHUNK,
  // ── 脫困 / 碰撞 QA 用（修玩家卡地裡）──
  // 純函式：餵假地形（isSolid 回呼）即可驗證 AABB 重疊偵測與脫困上抬，不依賴真世界。
  aabbHitsSolid(x, y, z, isSolid, pw, ph) { return aabbHitsSolid(x, y, z, isSolid, pw == null ? PW : pw, ph == null ? PH : ph); },
  unstuckY(x, y, z, isSolid, pw, ph) { return unstuckY(x, y, z, isSolid, pw == null ? PW : pw, ph == null ? PH : ph); },
  PW, PH,
  get overlapping() { return overlaps(); },     // 當前玩家是否卡在實心方塊內
  unstuckNow() { return unstuckIfNeeded(); },    // 手動觸發一次脫困（回傳是否有動）
  // 直接設玩家位置（QA 模擬出生卡住／走進未載入區後補載的情境）。
  setPlayerPos(x, y, z) { player.x = x; player.y = y; player.z = z; player.vy = 0; },
  // ── 登入綁定 QA 用（比照 3D #821）──
  get isLoggedIn() { return isLoggedIn; },
  get myAccountName() { return myAccountName; },
  get myName() { return myName; },
  // ── 深層礦石 v1 QA 用（ROADMAP 682）──
  COAL_ORE, IRON_ORE,
  // ── 鐵錠 v1 QA 用（ROADMAP 683）──
  IRON_INGOT,
  // ── 鐵磚 v1 QA 用（ROADMAP 684）──
  IRON_BLOCK,
  // ── 火把 v1 QA 用（ROADMAP 685）──
  TORCH,
  // ── 鎬具 v1 QA 用（ROADMAP 687）──
  PICKAXE_WOOD, PICKAXE_STONE, PICKAXE_IRON,
  // ── 驅影之劍 v1 QA 用（ROADMAP 887）──
  SWORD_WOOD, SWORD_STONE, SWORD_IRON,
  blockHardness, pickaxeBonus,
  get mining() { return mining; },
  get isMouseDown() { return isMouseDown; },
  // ── 梯子 v1 QA 用（ROADMAP 688）──
  LADDER, CLIMB_SPEED,
  aabbHitsLadder(x, y, z, getBlock, pw, ph) {
    return aabbHitsLadder(x, y, z, getBlock, pw == null ? PW : pw, ph == null ? PH : ph);
  },
  // ── 斧頭 v1 QA 用（ROADMAP 689）──
  AXE_WOOD, AXE_STONE, AXE_IRON,
  axeBonus,
  // ── 鏟子 v1 QA 用（ROADMAP 690）──
  SHOVEL_WOOD, SHOVEL_STONE, SHOVEL_IRON,
  shovelBonus,
  // ── 火把發光 v1 QA 用（ROADMAP 691）──
  get torchCount() { return torchPositions.size; },
  get heldTorchActive() { return heldTorchLight.intensity > 0; },
  get activeTorchLights() { return torchLightPool.filter(pl => pl.visible).length; },
  registerTorchBlock,
  unregisterTorchBlock,
  updateNearbyTorchLights,
  // ── 箱子 v1 QA 用（ROADMAP 692）──
  CHEST,
  get chestPanelVisible() { return chestPanelVisible(); },
  openChestPanel(bx, by, bz) { openChestPanel(bx, by, bz); },
  closeChestPanel() { closeChestPanel(); },
  get chestPos() { return _chestPos; },
  get chestItems() { return [..._chestItems]; },
  renderChestPanel() { renderChestPanel(); },
  // ── 木門 v1 QA 用（ROADMAP 693）──
  DOOR_CLOSED, DOOR_OPEN,
  // ── 床 v1 QA 用 ──
  BED,
  // ── 告示牌 v1 QA 用（ROADMAP 740）──
  SIGN,
  applySign(x, y, z, text) { applySign(x, y, z, text); },
  get signTexts() { return Object.fromEntries(signTexts); },
  get signSpriteCount() { return signSprites.size; },
  wrapSignLines(text, per, max) { return wrapSignLines(text, per, max); },
  // ── 漂流瓶 v1 QA 用（自主提案切片 825）──
  BOTTLE,
  get bottleMarkerCount() { return bottleMarkers.size; },
  addBottleMarker(x, y, z) { addBottleMarker(x, y, z); },
  removeBottleMarker(x, y, z) { removeBottleMarker(x, y, z); },
  // ── 操作設定 + 手把 QA 用（操作大改：準心+按鈕防誤觸、設定面板、鍵盤/手把）──
  get settings() { return { ...settings }; },
  setSetting(key, val) {
    if (!(key in SETTINGS_DEFAULTS)) return null;
    settings[key] = val;
    saveSettings();
    // 套用副作用（與面板 change 事件一致）：外觀/模式/人稱即時生效。
    applyTouchStyle(); applyTouchMode();
    if (key === "viewDefault" && viewMode !== settings.viewDefault) toggleViewMode();
    return settings[key];
  },
  reloadSettings() { const s = loadSettings(); Object.assign(settings, s); applyTouchStyle(); applyTouchMode(); return { ...settings }; },
  get settingsPanelVisible() { return settingsPanelVisible(); },
  openSettingsPanel() { openSettingsPanel(); },
  closeSettingsPanel() { closeSettingsPanel(); },
  get crosshairResident() { return crosshairResident; },
  get touchDigHeld() { return touchDigHeld; },
  // 模擬按住/放開觸控挖鈕（QA 驗證「拖曳不挖、按鈕才挖」）。
  touchDigStart() { if (crosshairResident) { const e = residents.get(crosshairResident); openChat(crosshairResident, e && e.lastName); return "talk"; } touchDigHeld = true; startMining(); return "dig"; },
  touchDigEnd() { touchDigHeld = false; cancelMining(); },
  // 手把 QA 用：連線狀態/名稱、挖鍵狀態、注入一次移動向量。
  get gamepadConnected() { return gamepadConnected; },
  get gamepadName() { return gamepadName; },
  get gamepadDigHeld() { return gamepadDigHeld; },
  pollGamepad(dt) { pollGamepad(dt == null ? 0.016 : dt); return { connected: gamepadConnected, name: gamepadName, move: { ...gpMove } }; },
  // ── 個人路標 QA 用（自主提案切片，ROADMAP 869）──
  get waypoints() { return [...waypoints]; },
  setWaypoints(items) { waypoints = items; renderWaypointList(); },
  refreshWaypoints() { return refreshWaypoints(); },
  renderWaypointList() { renderWaypointList(); return waypointBodyEl && waypointBodyEl.innerHTML; },
  promptAddWaypoint() { return promptAddWaypoint(); },
  removeWaypoint(label) { removeWaypoint(label); },
  // ── 居民表情/肢體 v1 QA 用（純視覺、無權威影響）──
  // 讀某位居民當前的肢體姿態（身體位移/縮放/搖擺、頭傾、手臂角度），驗程序動畫真的動了。
  residentPose(rid) {
    const ent = rid ? residents.get(rid) : residents.values().next().value;
    if (!ent) return null;
    return {
      id: ent.group.userData.rid, mood: ent.mood, humming: ent.humming, wavePulse: ent.wavePulse,
      bodyY: ent.body.position.y, bodyScaleY: ent.body.scale.y, bodyRotZ: ent.body.rotation.z,
      headX: ent.headPivot.rotation.x, headZ: ent.headPivot.rotation.z,
      armRx: ent.armR.rotation.x, armRz: ent.armR.rotation.z,
      armLx: ent.armL.rotation.x, visible: ent.group.visible,
      x: ent.group.position.x, y: ent.group.position.y, z: ent.group.position.z,
    };
  },
  residentIds() { return [...residents.keys()]; },
  // 就地強制某位居民的情緒訊號（只動本地渲染狀態；下一則伺服器快照會覆寫回真值，
  // 玩家用 console 呼叫也佔不到任何便宜——這只是把「當下心情」提前塞給前端動畫）。
  qaSetResidentMood(rid, mood) { const e = residents.get(rid); if (e) { e.mood = mood; } return this.residentPose(rid); },
  qaSetResidentHumming(rid, on) { const e = residents.get(rid); if (e) { e.humming = !!on; } return this.residentPose(rid); },
  qaTriggerResidentWave(rid) { const e = residents.get(rid); if (e) { e.wavePulse = 1.4; } return this.residentPose(rid); },
};
