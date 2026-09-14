#!/usr/bin/env bash
# Bring a tau2 company up end to end, and print where to watch it.
#
# One command from nothing to a running company with its role servers wired,
# its credential set, every layer verified, and a console URL. Everything it
# starts is logged under --logs and torn down by `tau2-up.sh down`.
#
#     scripts/tau2-up.sh                 # bring it up, print the console link
#     scripts/tau2-up.sh --task 0        # ...then run one tau2 task
#     scripts/tau2-up.sh down            # stop everything it started
#
# The role servers live in the `opencompany-tau2` checkout (--tau2), which
# vendors tau2-bench and needs its own venv; they are NOT in this repo. See
# companies/retail_co/README.md.
set -uo pipefail

DOMAIN=retail
BASE_PORT=8099
TAU2="${OC_TAU2:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../opencompany-tau2" 2>/dev/null && pwd)}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOME_DIR="${OC_RIG_HOME:-/tmp/tau2-rig}"
LOGS="${OC_LOGS:-/tmp/tau2-logs}"
CONSOLE=1
TASK=""
SEATS=(triage exchanges refunds cancellations amendments)

while [ $# -gt 0 ]; do
  case "$1" in
    down) DOWN=1 ;;
    --task) TASK="$2"; shift ;;
    --tasks) TASK="$2"; shift ;;
    --domain) DOMAIN="$2"; shift ;;
    --tau2) TAU2="$2"; shift ;;
    --port) BASE_PORT="$2"; shift ;;
    --logs) LOGS="$2"; shift ;;
    --no-console) CONSOLE=0 ;;
    -h|--help) sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

say() { printf '\033[1m%s\033[0m\n' "$*"; }
ok()  { printf '  \033[32m✓\033[0m %s\n' "$*"; }
bad() { printf '  \033[31m✗\033[0m %s\n' "$*"; }

stop_all() {
  say "stopping"
  pkill -f "tau2mcp.server" 2>/dev/null && ok "role servers" || true
  pkill -f "opencompany.*serve.*${DOMAIN}_co" 2>/dev/null || pkill -f "opencompany serve" 2>/dev/null && ok "company" || true
  pkill -f "vite.*--port" 2>/dev/null || true
  [ -f "$LOGS/console.pid" ] && kill "$(cat "$LOGS/console.pid")" 2>/dev/null && ok "console"
  rm -f "$LOGS/console.pid"
}

if [ "${DOWN:-0}" = 1 ]; then stop_all; exit 0; fi

mkdir -p "$LOGS"
[ -d "$TAU2" ] || { bad "no opencompany-tau2 checkout at $TAU2 — pass --tau2"; exit 1; }
PY="$TAU2/.venv/bin/python3"
[ -x "$PY" ] || { bad "no venv at $PY — run \`uv sync\` in $TAU2"; exit 1; }

# `CARGO_TARGET_DIR` first — a shared target dir is the usual setup when
# several worktrees of this repo are built side by side, and the binary is
# then nowhere near $REPO.
BIN="${OC_BIN:-}"
[ -x "${BIN:-}" ] || BIN="${CARGO_TARGET_DIR:+$CARGO_TARGET_DIR/debug/opencompany}"
[ -x "${BIN:-}" ] || BIN="$REPO/target/debug/opencompany"
[ -x "${BIN:-}" ] || BIN="$(ls -t "$REPO"/target*/debug/opencompany 2>/dev/null | head -1)"
[ -x "${BIN:-}" ] || { bad "no opencompany binary — cargo build --features openhuman,hivemind,mcp --bin opencompany"; exit 1; }

stop_all >/dev/null 2>&1
sleep 2

# 1. Role servers. Reseed the shared state FIRST: they seed it at boot, so
#    deleting it under a running server leaves every tool call erroring.
say "role servers ($DOMAIN)"
rm -f "$TAU2/.state/$DOMAIN.json" "$TAU2/.state/$DOMAIN.json.lock"
i=0
for seat in "${SEATS[@]}"; do
  ( cd "$TAU2" && nohup "$PY" -m tau2mcp.server --roles "roles/$DOMAIN.yaml" \
      --role "$seat" --http $((8801 + i)) > "$LOGS/$seat.log" 2>&1 & )
  i=$((i + 1))
done
sleep 12
i=0
for seat in "${SEATS[@]}"; do
  n=$(curl -s -X POST "http://127.0.0.1:$((8801 + i))/mcp" \
        -H 'content-type: application/json' -H 'accept: application/json, text/event-stream' \
        -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' 2>/dev/null \
      | grep -oE '"name":"[a-z_]+"' | wc -l | tr -d ' ')
  [ "${n:-0}" -gt 0 ] && ok "$seat :$((8801 + i)) — $n tools" || bad "$seat :$((8801 + i)) — no answer (see $LOGS/$seat.log)"
  i=$((i + 1))
done

# 2. The company.
say "company"
rm -rf "$HOME_DIR"; mkdir -p "$HOME_DIR"
RUST_LOG="${RUST_LOG:-opencompany::hivemind=debug,opencompany::server::operator=info,opencompany=info}" \
OPENCOMPANY_BIND="127.0.0.1:$BASE_PORT" \
  nohup "$BIN" serve --company "$REPO/companies/${DOMAIN}_co" --home "$HOME_DIR" \
  > "$LOGS/serve.log" 2>&1 &
for _ in $(seq 1 40); do
  [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "http://127.0.0.1:$BASE_PORT/healthz" 2>/dev/null)" = "200" ] && break
  sleep 3
done
if [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "http://127.0.0.1:$BASE_PORT/healthz")" = "200" ]; then
  ok "serving on 127.0.0.1:$BASE_PORT  (log: $LOGS/serve.log)"
else
  bad "the company did not come up — see $LOGS/serve.log"; tail -5 "$LOGS/serve.log"; exit 1
fi

SCOPE="http://127.0.0.1:$BASE_PORT/api/v1/companies/${DOMAIN}-co"

# 3. Credential. The models table goes WITH the key: an omitted `models` is
#    stored as an empty map that shadows the manifest's, and the next turn asks
#    the provider for a model nobody chose.
say "credential"
if [ -n "${OPENROUTER_API_KEY:-}" ]; then
  M=deepseek/deepseek-v4-flash
  curl -s -o /dev/null -X PUT "$SCOPE/inference" -H 'content-type: application/json' \
    -d "{\"provider\":\"openrouter\",\"base_url\":\"https://openrouter.ai/api/v1\",\"key\":\"$OPENROUTER_API_KEY\",\"models\":{\"chat-v1\":\"$M\",\"reasoning-v1\":\"$M\",\"agentic-v1\":\"$M\",\"vision-v1\":\"$M\"}}"
  ok "set from \$OPENROUTER_API_KEY, every tier on $M"
else
  bad "\$OPENROUTER_API_KEY is unset — set it, or configure Inference in the console"
fi

# 4. Wire the servers and verify every layer before spending a model call.
say "preflight"
python3 "$REPO/scripts/tau2-sim.py" --domain "$DOMAIN" --check \
  --base "http://127.0.0.1:$BASE_PORT" --tau2 "$TAU2" || true

# 5. The console: a Vite dev server proxying /api at this host. It picks its
#    own port, so the URL is read back out of its output rather than assumed.
if [ "$CONSOLE" = 1 ] && [ -d "$REPO/frontend/node_modules" ]; then
  say "console"
  ( cd "$REPO/frontend" && OC_API_TARGET="http://127.0.0.1:$BASE_PORT" \
      nohup npm run dev > "$LOGS/console.log" 2>&1 & echo $! > "$LOGS/console.pid" )
  URL=""
  for _ in $(seq 1 30); do
    URL=$(grep -oE 'http://(localhost|127\.0\.0\.1):[0-9]+' "$LOGS/console.log" 2>/dev/null | head -1)
    [ -n "$URL" ] && break
    sleep 2
  done
  [ -n "$URL" ] && ok "$URL" || bad "console did not report a URL — see $LOGS/console.log"
elif [ "$CONSOLE" = 1 ]; then
  say "console"
  bad "frontend/node_modules missing — run \`npm install\` in frontend/ to watch in a browser"
fi

echo
say "watch it"
echo "  console   ${URL:-(not started)}"
echo "  api       http://127.0.0.1:$BASE_PORT/api/v1/companies/${DOMAIN}-co"
echo "  host log  tail -f $LOGS/serve.log"
echo "  turns     ls $HOME_DIR/harness/${DOMAIN}-co/*/workspace/sessions/*/*.md"
echo
echo "  run a task:  python3 scripts/tau2-sim.py --domain $DOMAIN --task 0 --base http://127.0.0.1:$BASE_PORT --tau2 $TAU2"
echo "  stop:        scripts/tau2-up.sh down"

if [ -n "$TASK" ]; then
  echo
  say "task $TASK"
  # No --turns here: the runner's own default is the max, and a cap set from
  # this side silently fails every task whose policy wants a confirmation.
  python3 "$REPO/scripts/tau2-sim.py" --domain "$DOMAIN" --tasks "$TASK" --settle 600 --quiet 90 \
    --base "http://127.0.0.1:$BASE_PORT" --tau2 "$TAU2" --out "$LOGS/run.json"
  echo "  record: $LOGS/run.json"
fi
