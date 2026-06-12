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

# Rust（cargo audit）
if command -v cargo-audit >/dev/null 2>&1 || cargo audit --version >/dev/null 2>&1; then
  out="$(cargo audit 2>&1)"
  if echo "$out" | grep -qiE 'Vulnerabilit(y|ies) found|error: [0-9]+ vulnerabilit'; then
    findings+="### Rust（cargo audit）\n$(echo "$out" | grep -iE 'ID|Crate|Title|Severity|Solution|vulnerabilit' | head -40)\n\n"
  fi
else
  echo "[security-scan] cargo-audit 未安裝，跳過 Rust 掃描"
fi

# 前端（npm audit）
if [ -f package.json ]; then
  nout="$(npm audit 2>&1)"
  if ! echo "$nout" | grep -qiE 'found 0 vulnerabilities'; then
    findings+="### 前端（npm audit）\n$(echo "$nout" | grep -iE 'vulnerabilit|severity|advisory|fix' | head -30)\n\n"
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
