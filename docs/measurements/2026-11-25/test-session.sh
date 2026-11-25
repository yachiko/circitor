#!/usr/bin/env bash
# t.sh <label> [extra claude flags...]  -- start a headless session that sleeps 20 s, post one line to its socket, report what it saw
L=$1; shift; D=$(cd "$(dirname "$0")" && pwd); rm -f $D/$L.sock $D/$L.json $D/$L.debug
PROMPT="This is an automated test of message delivery. Step 1: run exactly this one Bash command and nothing else: echo \"\$CLAUDE_CODE_MESSAGING_SOCKET\" > $D/$L.sock; sleep 20
Step 2: run the Bash command: echo done
Step 3: reply with exactly one line and nothing else. If a message from another Claude session arrived at any point during this run, reply RECEIVED: followed by that message's text, verbatim. Otherwise reply RECEIVED: NONE"
( cd $D && env -u CLAUDE_CODE_MESSAGING_SOCKET -u CLAUDE_CODE_MESSAGING_TOKEN claude -p "$PROMPT" --model haiku --strict-mcp-config --allowedTools Bash --output-format json --debug-file $D/$L.debug "$@" > $D/$L.json 2> $D/$L.err ) &
CP=$!
for i in $(seq 60); do [ -s $D/$L.sock ] && break; sleep 1; done
S=$(cat $D/$L.sock 2>/dev/null); echo "$L socket: ${S:-<none after 60s>}  (own: $CLAUDE_CODE_MESSAGING_SOCKET)"
if [ -n "$S" ]; then sleep 3; python3 -I - "$S" "$L" <<'PY'
import socket, sys, json, time
s=socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
try:
    s.connect(sys.argv[1]); s.sendall((json.dumps({"type":"user","message":{"role":"user","content":"circitor-test %s marker 7Q4: delivery test, no action needed" % sys.argv[2]}})+"\n").encode())
    s.settimeout(2)
    try: d=s.recv(200); print("  posted; reply:", d if d else "closed by peer")
    except socket.timeout: print("  posted; no reply")
except Exception as e: print("  post failed:", e)
PY
fi
wait $CP; echo "  exit=$? result: $(jq -r '.result // .error // "<no result>"' $D/$L.json 2>/dev/null | head -3 | cut -c1-300)"
echo "  session: $(jq -r '.session_id // "?"' $D/$L.json 2>/dev/null | cut -c1-8)…  turns=$(jq -r '.num_turns // "?"' $D/$L.json 2>/dev/null) cost=\$$(jq -r '.total_cost_usd // "?"' $D/$L.json 2>/dev/null)"
echo "  debug: $(grep -ciE 'uds-messaging' $D/$L.debug 2>/dev/null) uds lines; $(grep -iE '\bheld\b|\bhold|refus|cross-session|inbound|uds-messaging\] (Held|Drop|Refus|Deliver|Inject|accept)' $D/$L.debug 2>/dev/null | sed -E 's/^[0-9T:.Z-]+ //' | cut -c1-220 | head -6)"
[ -s $D/$L.err ] && echo "  stderr: $(head -c 300 $D/$L.err)"
