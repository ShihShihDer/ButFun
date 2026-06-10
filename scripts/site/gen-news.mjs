// 從 git 歷史產生官網更新日誌（web/site/news.json）。零 token：
// 自走迴圈每合一個 PR（或直進 main 的小修），下次部署官網就自動長出一條更新——
// 「AI 自動更新官網」的最便宜實作。只收玩家看得到的 feat/fix/perf；docs/chore 等略過。
// 用法： node scripts/site/gen-news.mjs   （deploy.sh 會在每次上線時自動跑）
import { execSync } from "child_process";
import { writeFileSync, mkdirSync } from "fs";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

// %x1e 當紀錄分隔、%x1f 當欄位分隔（標題/內文可含任何字元，不能用換行切）。
const raw = execSync(
  "git log --first-parent -200 --pretty=format:%x1e%ad%x1f%s%x1f%b --date=short main",
  { cwd: REPO, encoding: "utf8" },
);

const items = [];
for (const rec of raw.split("\x1e")) {
  if (!rec.trim()) continue;
  const [date, subject = "", body = ""] = rec.split("\x1f");
  // GitHub 合併 commit 的主旨是「Merge pull request #N …」，PR 標題在 body 第一行；
  // 直進 main 的 commit 標題就是主旨本身。
  const isMerge = subject.startsWith("Merge ");
  const title = (isMerge ? body.split("\n")[0] : subject).trim();
  if (!title || title.startsWith("Merge ")) continue;
  const prMatch = subject.match(/#(\d+)/);
  const m = title.match(/^(feat|fix|perf|docs|chore|refactor|test|ci|style)(\(([^)]*)\))?\s*:\s*(.*)$/);
  const kind = m ? m[1] : "other";
  if (!["feat", "fix", "perf", "other"].includes(kind)) continue; // 玩家看不到的不上官網
  items.push({
    date,
    pr: prMatch ? Number(prMatch[1]) : null,
    kind,
    scope: (m && m[3]) || "",
    text: m ? m[4] : title,
  });
  if (items.length >= 40) break;
}

mkdirSync(join(REPO, "web", "site"), { recursive: true });
writeFileSync(
  join(REPO, "web", "site", "news.json"),
  JSON.stringify({ generated: new Date().toISOString().slice(0, 10), items }, null, 1),
);
console.log(`✅ news.json：${items.length} 條更新`);
