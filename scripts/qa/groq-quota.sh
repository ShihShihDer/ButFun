#!/bin/bash
# 查每把 Groq key 的剩餘額度（讀 Groq 回應的 x-ratelimit-* header）。
# 用法：bash scripts/qa/groq-quota.sh   （從含 .env 的 repo 根目錄跑，或 BUTFUN_ENV 指定）
# 每把 key 打一個極小請求（max_tokens:1）、只讀 header、不在意內容。
set -uo pipefail
ENVF="${BUTFUN_ENV:-.env}"
KEYS=$(grep '^GROQ_API_KEY=' "$ENVF" 2>/dev/null | sed 's/^GROQ_API_KEY=//' | tr ',' '\n')
[ -z "$KEYS" ] && { echo "找不到 GROQ_API_KEY（$ENVF）"; exit 1; }
i=0
printf "%-5s %-12s %-12s %-10s %s\n" "key" "剩token" "上限token" "剩請求" "token重置"
echo "------------------------------------------------------------"
while IFS= read -r KEY; do
  KEY=$(echo "$KEY" | tr -d ' ')
  [ -z "$KEY" ] && continue
  i=$((i+1))
  H=$(curl -sS -m12 -D - -o /dev/null \
    https://api.groq.com/openai/v1/chat/completions \
    -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
    -d '{"model":"llama-3.3-70b-versatile","messages":[{"role":"user","content":"hi"}],"max_tokens":1}' 2>/dev/null)
  g(){ echo "$H" | grep -i "^$1:" | tr -d '\r' | awk '{print $2}'; }
  rt=$(g x-ratelimit-remaining-tokens); lt=$(g x-ratelimit-limit-tokens)
  rr=$(g x-ratelimit-remaining-requests); rst=$(g x-ratelimit-reset-tokens)
  code=$(echo "$H" | head -1 | awk '{print $2}')
  [ -z "$rt" ] && rt="(HTTP $code)"
  printf "%-5s %-12s %-12s %-10s %s\n" "$i" "${rt:-?}" "${lt:-?}" "${rr:-?}" "${rst:-?}"
done <<< "$KEYS"
echo "------------------------------------------------------------"
echo "註：剩token 是「當下這分鐘/視窗」的剩餘；每日 TPD 上限爆掉時 remaining 會很小或 429。"
