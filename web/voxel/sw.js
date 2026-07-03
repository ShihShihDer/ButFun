/* 乙太方界 Service Worker
 * 目標：讓「加到主畫面」像 App、離線也不白屏。
 *
 * 快取策略（沿用專案「內容雜湊 no-cache」精神，避免更新看不到）：
 *   - 導覽請求（開 App 殼）與 main.js：network-first —— 有網一律抓最新（版本一變立刻拿到），
 *     沒網才退快取，再沒有才退友善離線頁。絕不讓玩家卡在舊版。
 *   - 圖示 / manifest 等不常變的靜態：cache-first（快、省流量）。
 *   - WS /voxel/ws 與即時 API（/version、/auth/*、/voxel/diary…）：完全放行、不攔截、不快取。
 *   - 版本化快取：SW_VERSION 一改，activate 時清掉所有舊快取。
 *
 * scope：本檔以 /sw.js 服務（根範圍），能控制 / 與 /voxel/ 兩個入口。
 */

const SW_VERSION = "ethervox-pwa-v1";
const CACHE = `ethervox-shell-${SW_VERSION}`;

// App shell：離線時仍能載入的最小靜態集合。
const APP_SHELL = [
  "/voxel/main.js",
  "/manifest.webmanifest",
  "/voxel/icons/icon-192.png",
  "/voxel/icons/icon-512.png",
  "/voxel/icons/icon-maskable-192.png",
  "/voxel/icons/icon-maskable-512.png",
  "/voxel/icons/apple-touch-icon.png",
  "/voxel/icons/favicon-32.png",
];

// 即時端點：一律走網路、SW 不插手（離線時由前端自己顯示「需要連線」）。
const LIVE_PREFIXES = [
  "/voxel/ws",
  "/voxel/diary",
  "/voxel/feed",
  "/voxel/affinity",
  "/voxel/relations",
  "/voxel/skills",
  "/voxel/milestones",
  "/version",
  "/auth/",
  "/api/",
];

// 友善離線頁（沒網、又沒快取時的導覽退路）——不是白屏，配遊戲夜空色調。
const OFFLINE_HTML = `<!DOCTYPE html>
<html lang="zh-Hant"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>乙太方界 — 需要連線</title>
<style>
  html,body{margin:0;height:100%;background:#0e1830;color:#eaf2ff;
    font-family:system-ui,-apple-system,"PingFang TC","Noto Sans TC",sans-serif;}
  .wrap{position:fixed;inset:0;display:flex;flex-direction:column;
    align-items:center;justify-content:center;text-align:center;padding:24px;gap:14px;}
  .cube{width:88px;height:88px;filter:drop-shadow(0 0 18px rgba(79,214,200,.5));}
  h1{font-size:20px;margin:6px 0 0;font-weight:700;}
  p{font-size:14px;line-height:1.7;opacity:.8;max-width:22em;margin:0;}
  button{margin-top:10px;padding:10px 22px;font-size:15px;color:#0e1830;
    background:#4fd6c8;border:0;border-radius:10px;cursor:pointer;font-weight:700;}
  button:active{transform:scale(.97);}
</style></head><body>
<div class="wrap">
  <svg class="cube" viewBox="0 0 100 100" aria-hidden="true">
    <polygon points="50,10 90,32 50,54 10,32" fill="#7af0e0"/>
    <polygon points="10,32 50,54 50,96 10,74" fill="#2f9fb0"/>
    <polygon points="90,32 50,54 50,96 90,74" fill="#1f6f88"/>
  </svg>
  <h1>乙太方界暫時連不上</h1>
  <p>這個世界需要網路連線才能進入——AI 居民正在裡頭生活、蓋家、等你回來。<br>接上網路後再試一次。</p>
  <button onclick="location.reload()">重新連線</button>
</div></body></html>`;

self.addEventListener("install", (event) => {
  // 立刻接手，讓新版 SW 不必等所有分頁關閉。
  self.skipWaiting();
  event.waitUntil(
    caches.open(CACHE).then(async (cache) => {
      // 個別加入：任何一項失敗都不整批中止（韌性）。
      await Promise.all(
        APP_SHELL.map((url) =>
          cache.add(new Request(url, { cache: "reload" })).catch(() => {}),
        ),
      );
      // App 殼首頁：以 '/'（後端會注入 main.js 內容雜湊版本）存起來。
      await cache.add(new Request("/", { cache: "reload" })).catch(() => {});
    }),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(
        keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)),
      );
      await self.clients.claim();
    })(),
  );
});

// 前端可送 SKIP_WAITING 促使新版立即接管（配合更新提示）。
self.addEventListener("message", (event) => {
  if (event.data && event.data.type === "SKIP_WAITING") self.skipWaiting();
});

function isLive(pathname) {
  return LIVE_PREFIXES.some((p) => pathname === p || pathname.startsWith(p));
}

async function networkFirst(request, isNav) {
  try {
    const fresh = await fetch(request);
    // 存一份最新的到快取（供之後離線用）。只快取成功的同源回應。
    if (fresh && fresh.ok) {
      const cache = await caches.open(CACHE);
      cache.put(request, fresh.clone()).catch(() => {});
    }
    return fresh;
  } catch (_e) {
    // 沒網：退快取（main.js 用 ignoreSearch，因版本?v=會變）。
    const cache = await caches.open(CACHE);
    const cached =
      (await cache.match(request, { ignoreSearch: true })) ||
      (isNav ? await cache.match("/") : undefined);
    if (cached) return cached;
    if (isNav) {
      return new Response(OFFLINE_HTML, {
        headers: { "Content-Type": "text/html; charset=utf-8" },
        status: 200,
      });
    }
    return Response.error();
  }
}

async function cacheFirst(request) {
  const cache = await caches.open(CACHE);
  const cached = await cache.match(request, { ignoreSearch: true });
  if (cached) return cached;
  try {
    const fresh = await fetch(request);
    if (fresh && fresh.ok) cache.put(request, fresh.clone()).catch(() => {});
    return fresh;
  } catch (_e) {
    return Response.error();
  }
}

self.addEventListener("fetch", (event) => {
  const req = event.request;
  // 只處理同源 GET；WS/跨源/POST 一律放行。
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  // 即時端點：不攔截（含 WS 升級請求）。
  if (isLive(url.pathname)) return;

  const isNav = req.mode === "navigate";
  const isMain = url.pathname === "/voxel/main.js";
  if (isNav || isMain) {
    event.respondWith(networkFirst(req, isNav));
    return;
  }
  // 其他靜態（圖示 / manifest）：cache-first。
  event.respondWith(cacheFirst(req));
});
