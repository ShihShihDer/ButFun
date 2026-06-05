# ButFun 美術素材整合手冊 v1

> 給接手的遊戲工程師。這份文件說明 Phase 0 像素素材的規格、索引、狀態機與「如何接回 `web/game.js`」。
> 隨附**可運作的參考實作**:`demo/game.js`(純 canvas、零相依),所有下面的程式片段都來自它。
> 互動展示:`ButFun 素材展示.html`(可走動、可耕作、3 色票即時切換、附 sprite sheet 下載)。

---

## 0. 檔案清單

| 檔案 | 尺寸 | 佈局 | 用途 |
|---|---|---|---|
| `assets/tileset_a.png` | 128×128 | 4 欄 × 4 列 | 地面 — **A 邊境星(綠野)** |
| `assets/tileset_b.png` | 128×128 | 4 欄 × 4 列 | 地面 — **B 黃昏苔原** |
| `assets/tileset_c.png` | 128×128 | 4 欄 × 4 列 | 地面 — **C 乙太霧星** |
| `assets/field.png` | 256×32 | 8 欄 × 1 列 | 田地 8 狀態 |
| `assets/fence.png` | 192×32 | 6 欄 × 1 列 | 蒸汽龐克柵欄 6 件 |
| `assets/player.png` | 128×128 | 4 欄 × 4 列 | 玩家(4 影格 × 4 方向) |
| `assets/icons.png` | 80×16 | 5 欄 × 1 列 | UI 圖示(16×16) |
| `assets/ship.png` | 80×56 | 單張 | 墜毀乙太飛船 |
| `assets/workshop.png` | 56×60 | 單張 | 生鏽乙太工坊 |
| `assets/tree.png` | 40×52 | 單張 | 邊境星樹木 |
| `assets/rock.png` | 30×24 | 單張 | 礦石巨岩 |

**通用規格**:每格 32×32、0px 間隙、透明背景、`image-rendering: pixelated`、繪製時 `ctx.imageSmoothingEnabled = false`(**禁止插值放大**,否則糊邊)。授權 **CC0 / Public Domain**,可商用、可放進開源 repo。

**色票(嚴格一致)**

```
草地深綠 #2d4d2a   草地亮綠 #3e6a3a   泥土暖黃 #7a5f3c   翻土深褐 #5b4636
黃銅金   #c9a24b   文字米白 #e8e0cf   深空底色 #0b0d12   乙太藍光 #7fe0e6
```

---

## 1. 地面 tileset(3 套色票 = 3 種地圖主題,可混搭)

**佈局**(欄 0–3 為變體,避免大面積機械重複):

| 列(sy) | 內容 | 欄 0–3 |
|---|---|---|
| 0 | 草地 | 4 變體 |
| 1 | 泥土路徑 | 4 變體 |
| 2 | **水面** | **4 影格動畫(微波)** |
| 3 | 石板 | 4 變體(欄2 有黃銅鉚釘、欄3 帶苔) |

**混搭做法**:引擎只認「一張 tileset」。每個 zone / 地圖在資料裡指定要用哪一套(`a` 綠野邊境星 / `b` 黃昏苔原 / `c` 乙太霧星),載入對應 PNG 即可——這正好對上 GDD「多星球＝主題化地圖」。展示頁右上角的色票切換鈕就是這個機制的示範。

```js
const TS = 32;
const TILESETS = { borderland:'tileset_a.png', dusk:'tileset_b.png', etherfog:'tileset_c.png' };
const ROW = { grass:0, dirt:1, water:2, stone:3 };

// base: 'grass'|'dirt'|'water'|'stone'  variant: 0..3  timeMs: 動畫時間
function drawTile(ctx, sheet, base, variant, dx, dy, timeMs){
  const sy = ROW[base] * TS;
  const sx = (base === 'water')
    ? (Math.floor(timeMs / 170) % 4) * TS   // 水面:4 影格、約 170ms/格
    : (variant & 3) * TS;                    // 其餘:0–3 變體
  ctx.drawImage(sheet, sx, sy, TS, TS, dx, dy, TS, TS);
}
```

---

## 2. 田地系統 `field.png`(8 狀態 + 狀態機)

**欄位對照**

| 欄 | 狀態 |
|---|---|
| 0 | 未翻土(乾自然地,與草地可區分) |
| 1 | 已翻土(深褐犁溝) |
| 2 | 已澆水(深褐泛濕反光) |
| 3 | 乙太作物 ① 種子 |
| 4 | ② 發芽 |
| 5 | ③ 成長 |
| 6 | ④ 成熟(發光金果) |
| 7 | 缺水提示 overlay(藍點,疊在上層) |

**互動狀態機**(玩家在附近時點格子):

```
0 未翻土 ──翻土──▶ 1 已翻土
1 已翻土 ──播種──▶ 種子(stage=2, watered=false)
2/3/4 且未澆水 ──澆水──▶ watered=true(開始計時成長)
   watered 時 growth += dt;滿 GROW_TIME → stage++、watered=false(需再澆水)
5 成熟 ──收成──▶ +乙太,回到 1 已翻土(可再種)
```

```js
// f = { stage:0..5, watered:bool, growth:number }
function fieldColumn(f){
  if(f.stage === 0) return 0;                  // 未翻土
  if(f.stage === 1) return f.watered ? 2 : 1;  // 翻土 / 澆水
  return [,, 3,4,5,6][f.stage];                // 種子/發芽/成長/成熟
}
function drawField(ctx, sheet, f, dx, dy){
  ctx.drawImage(sheet, fieldColumn(f)*TS, 0, TS,TS, dx,dy, TS,TS);
  if(f.stage >= 2 && f.stage <= 4 && !f.watered)        // 缺水提示
    ctx.drawImage(sheet, 7*TS, 0, TS,TS, dx,dy, TS,TS);
}
```

**成熟發光**:除了 sprite 自帶的金光,再對 stage 5 疊一層 additive 徑向漸層(`globalCompositeOperation='lighter'`),夜晚加強——見 `demo/game.js` 的 render 區段。

---

## 3. 柵欄 `fence.png`(6 件 + 旋轉自動接圖)

| 欄 | 件 |
|---|---|
| 0 | 水平 rail |
| 1 | 垂直 rail |
| 2 | 角(連 東+南) |
| 3 | 角(連 西+南) |
| 4 | 丁字(連 東+西+南) |
| 5 | 十字 |

**最省事的 autotiler**:角一律用 **index 2** 靠旋轉處理四個方向;十字用 5;直線用 0 / 1。圍田時在底邊留 1–2 格缺口當柵門(gate),並把該格設為可通行。

```js
// neighbors = {n,e,s,w} 皆為「該方向也有柵欄」的 bool
function drawFence(ctx, sheet, tx, ty, {n,e,s,w}){
  const cnt = n+e+s+w; let piece = 0, rot = 0;
  if(cnt >= 3)        piece = 5;                         // 丁/十字 → 5(或用 4 旋轉)
  else if(e && w)     piece = 0;                          // 水平
  else if(n && s)     piece = 1;                          // 垂直
  else {              piece = 2;                          // 角:用 2 旋轉
    if(e&&s) rot=0; else if(s&&w) rot=90; else if(w&&n) rot=180; else rot=270; }
  ctx.save();
  ctx.translate(tx*TS + TS/2, ty*TS + TS/2);
  ctx.rotate(rot * Math.PI/180);
  ctx.drawImage(sheet, piece*TS, 0, TS,TS, -TS/2,-TS/2, TS,TS);
  ctx.restore();
}
```

---

## 4. 玩家 `player.png`(4 方向 × 4 影格)

- **列(sy) = 方向**:0 下、1 左、2 右、3 上
- **欄(sx) = 走路影格** 0–3;**站立用影格 0**
- 走路:`frame = Math.floor(animTimer) % 4`,移動時 `animTimer += dt * 8`
- `dy` 以**腳底**為基準,繪製時往上扣一格再 +6(腳貼地)

```js
function drawPlayer(ctx, sheet, p, footX, footY){
  ctx.drawImage(sheet, p.frame*TS, p.dir*TS, TS,TS,
                Math.round(footX - TS/2), Math.round(footY - TS + 6), TS,TS);
}
```

**其他玩家 / NPC 變色**(免另外畫圖,用同一張上色):

```js
function tintSheet(playerImg, color){
  const c = document.createElement('canvas'); c.width = c.height = 128;
  const x = c.getContext('2d'); x.imageSmoothingEnabled = false;
  x.drawImage(playerImg, 0, 0);
  x.globalCompositeOperation = 'source-atop';
  x.globalAlpha = 0.32; x.fillStyle = color; x.fillRect(0, 0, 128, 128);
  return c; // 拿這張當該 NPC 的 sheet
}
```

> ⚠️ 這是 **placeholder**。GDD 規劃「全自由創角器」會把身體部位分開上色、程式組裝;屆時這張整體 sheet 會被分件素材取代。先用它把流程跑起來即可。

---

## 5. UI 圖示 `icons.png`(16×16)

欄 0–4 依序:`建議 💡` / `田地 🌱` / `設定 ⚙️` / `背包 📦` / `載具 🚗`。黃銅線稿、透明背景,可直接當按鈕 icon 或 `<img>`。

```js
function drawIcon(ctx, sheet, index, dx, dy, size=16){
  ctx.drawImage(sheet, index*16, 0, 16, 16, dx, dy, size, size);
}
```

---

## 6. 場景物件 props

| 檔 | 錨點 | 備註 |
|---|---|---|
| `ship.png` | 底部中心 | 墜毀飛船,引擎有乙太餘光;占地約 3×2 格設為不可走 |
| `workshop.png` | 底部中心 | 生鏽工坊,煙囪冒蒸汽、乙太窗光;占地約 2×2 格不可走 |
| `tree.png` | 底部中心(樹幹) | 樹幹格不可走;可加微幅左右搖曳 |
| `rock.png` | 底部中心 | 含黃銅 / 乙太礦脈;整格不可走 |

**深度排序(painter's)**:所有「站在地上的東西」(柵欄 / 樹 / 石 / 建築 / 玩家 / NPC)依**腳底 y(baseY)**由小到大排序後再畫,前後遮擋才會對。

```js
entities.sort((a,b) => a.baseY - b.baseY).forEach(drawEntity);
```

**整體 render 順序**:① 地面 tiles → ② 田地 sprites(+缺水 overlay)→ ③ 深度排序的實體 → ④ 成熟作物 additive 發光 → ⑤ 日夜 overlay(螢幕空間)。

---

## 7. 日夜循環

伺服器驅動一天進度 `t ∈ [0,1)`;client 依 `t` 在全螢幕疊該主題的夜色(暗藍 rgba)+ vignette,黃昏再加一層暖色洗。每套色票各有自己的 `night` / `dusk` 色(見 `demo/game.js` 的 `PAL`)。

```js
const lerp = (a,b,t) => a + (b-a) * Math.max(0, Math.min(1, t));
function nightAlpha(t){                 // 0 = 白天,1 = 深夜
  if(t < 0.18) return lerp(0.55, 0, t/0.18);
  if(t < 0.5 ) return 0;
  if(t < 0.66) return lerp(0, 0.85, (t-0.5)/0.16);
  if(t < 0.9 ) return 0.85;
  return lerp(0.85, 0.55, (t-0.9)/0.1);
}
```

---

## 8. 接回現有 repo(`web/game.js`)

現況:`web/game.js` 的 `drawTile` / `drawField` 是**純色塊 placeholder**(README 已註明之後換 `drawImage`)。整合步驟:

1. 把 `assets/*.png` 放到 `web/assets/`。
2. 啟動前 **preload** 全部 sheet(`new Image()`,全部 `onload` 後再進迴圈)。
3. 用本文件的 `drawTile` / `drawField` / `drawFence` / `drawPlayer` 取代色塊版本。
4. 算繪分離:**權威伺服器只送狀態**(田地 stage/watered、玩家位置、日夜 t);client 只負責 `drawImage`——與現有架構完全一致,不動 protocol。
5. 渲染照第 6 節順序;像素設定 `imageSmoothingEnabled=false` + CSS `image-rendering:pixelated`。

> `demo/game.js` 是一份**已驗證可運作**的整端參考(地圖生成、相機、碰撞、耕作、日夜、NPC、發光);可直接對照搬移,或當作 `web/game.js` 的改寫範本。

---

## 9. 已知限制 / 待辦

- **角色為 placeholder**——之後換成分件創角素材。
- **三套色票目前只換「地表 tileset」**(占畫面最大宗)。若要把田地 / 角色 / 場景物件也做成同主題的整套色調,需再產一輪(便宜,告知即可)。
- 未含:音效、季節變化、載具 / 自動化 / 戰鬥素材(屬 Phase 1+)。
- 水面動畫為 4 影格平移;若要更細緻可加幀。

---

## 附錄 A:把這包送上 GitHub 新分支

> 我(美術 AI)沒有 repo 寫入權限,無法直接開分支。以下指令你或工程師在 repo 根目錄貼上即可(或在 repo 目錄開 Claude Code 請它代跑)。

```bash
# 在 ButFun repo 根目錄
git checkout -b art/phase0-pixel-assets

mkdir -p web/assets
cp /path/to/下載解壓/assets/*.png       web/assets/
cp /path/to/下載解壓/docs/ASSET_INTEGRATION.md docs/
# 參考實作(可選,放著對照用)
mkdir -p web/_reference && cp /path/to/下載解壓/demo/game.js web/_reference/asset-demo.js

git add web/assets docs/ASSET_INTEGRATION.md web/_reference
git commit -m "art: Phase 0 pixel-art sprite sheets + 整合手冊"
git push -u origin art/phase0-pixel-assets
```

之後在 GitHub 開 PR 即可。
