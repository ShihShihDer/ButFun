#!/usr/bin/env bash
# ButFun 本機依賴漏洞掃描：cargo audit（Rust）+ npm audit（前端），比對權威漏洞庫（RustSec / npm advisories）。
# 有中招 → 寫進 butfun-coord/for_human.md + 推播。給排程（每週 timer）或 deploy 前跑。
# 設計：cargo-audit 沒裝就跳過該段（不擋）；只回報、不自動改。Dependabot 是 server 端主力，本檔是本機補強。
set -u
REPO="/home/shihshih/ButFun"
COORD="/home/shihshih/butfun-coord"
NOTIFY="$REPO/scripts/auto/notify.sh"
cd "$REPO" || exit 1

findings=""

# Rust（cargo audit）——真正的 prod 伺服器依賴。用 exit code 判定（有漏洞→非 0），
# 已評估接受的例外由 .cargo/audit.toml 白名單（rsa 打不到、spin yanked），故乾淨時 exit 0。
if command -v cargo-audit >/dev/null 2>&1 || cargo audit --version >/dev/null 2>&1; then
  cout="$(cargo audit --quiet 2>&1)"; crc=$?
  if [ "$crc" -ne 0 ] && printf '%s' "$cout" | grep -qiE 'vulnerabilit|RUSTSEC'; then
    findings+="### Rust（cargo audit）\n$(printf '%s' "$cout" | grep -iE 'Crate|Title|Severity|Solution|RUSTSEC|ID:|Dependency' | head -40)\n\n"
  fi
else
  echo "[security-scan] cargo-audit 未安裝，跳過 Rust 掃描（⚠️ 真正的 prod 依賴未稽核！請 cargo install cargo-audit --locked）"
fi

# 前端（npm audit）——純 QA/開發工具（ws/puppeteer-core），不上 prod。
# 改用 --json 拿「真實漏洞總數」分辨：乾淨(0) / 中招(>0) / audit 失敗(離線等，解析不出)。
# 失敗一律**不判中招**（舊版「只要不是 found 0 就中招」→ audit 一失敗就空手喊狼，是先前每週誤報的元凶）。
if [ -f package.json ]; then
  njson="$(npm audit --json 2>/dev/null)"
  ntotal="$(printf '%s' "$njson" | python3 -c 'import sys,json
try:
    d=json.load(sys.stdin); print(d.get("metadata",{}).get("vulnerabilities",{}).get("total",-1))
except Exception:
    print(-1)' 2>/dev/null || echo -1)"
  if [ "$ntotal" = "-1" ]; then
    echo "[security-scan] npm audit 無法解析（離線/失敗），不判中招（不喊狼）"
  elif [ "$ntotal" != "0" ]; then
    findings+="### 前端（npm audit）：$ntotal 個漏洞\n$(npm audit 2>&1 | grep -iE 'severity|advisory|fix|>=|<' | head -30)\n\n"
  fi
fi

ts="$(date '+%Y-%m-%d %H:%M')"
if [ -n "$findings" ]; then
  echo "[security-scan] 發現依賴漏洞,回報 + 推播"
  cd "$COORD" && git pull --rebase -q 2>/dev/null || true
  {
    printf '\n## [%s] security-scan → human | 依賴漏洞中招\n' "$ts"
    printf '本機 cargo/npm audit 掃到已知漏洞,請看下方並評估升級（Dependabot 應也已開修復 PR）：\n\n'
    printf '%b' "$findings"
  } >> "$COORD/for_human.md"
  git -C "$COORD" add -A && git -C "$COORD" commit -q -m "security-scan: 依賴漏洞回報 $ts" && git -C "$COORD" push -q 2>/dev/null || true
  "$NOTIFY" alert "依賴漏洞掃到中招,看 for_human.md" >/dev/null 2>&1 || true
else
  echo "[security-scan] $ts 乾淨,無已知依賴漏洞"
fi
