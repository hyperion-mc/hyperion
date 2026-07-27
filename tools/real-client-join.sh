#!/usr/bin/env bash
# Join a server with the real Minecraft client and report whether the player
# reached the world. A scripted client proves packets parse; only this proves
# a real client can use them.
#
#   real-client-join.sh <host:port> [instance] [dwell_s] [server_log]
#
# Verdict rules. The client logs a failure loudly and a success not at all, so
# failure is read from the client and success is read from both sides: no
# failure line, the client still running after the dwell, and, when a server
# log is given, a join recorded on it during the run.
set -euo pipefail

addr="${1:?usage: real-client-join.sh <host:port> [instance] [dwell_s] [server_log]}"
instance="${2:-26.2}"
dwell="${3:-60}"
server_log="${4:-}"

prism="/Applications/Prism Launcher.app/Contents/MacOS/prismlauncher"
log="$HOME/Library/Application Support/PrismLauncher/instances/$instance/minecraft/logs/latest.log"

server_before=0
if [ -n "$server_log" ]; then
  server_before=$(grep -c "joined the world" "$server_log" || true)
fi

rm -f "$log"
"$prism" -l "$instance" -s "$addr" -o AgentBot >"/tmp/prism-$instance.out" 2>&1 &
launcher=$!

failed=""
for _ in $(seq 1 "$dwell"); do
  if [ -f "$log" ] && grep -qE 'Client disconnected with reason|Missing tag|Registry Loading' "$log"; then
    failed=yes
    break
  fi
  sleep 1
done

alive=no
pgrep -f "Prism Launcher: $instance\"" >/dev/null && alive=yes

server_after=$server_before
if [ -n "$server_log" ]; then
  server_after=$(grep -c "joined the world" "$server_log" || true)
fi

# Match the instance exactly. A bare KnotClient pattern also matches a client
# the operator is playing in, and killing that has already been one keystroke away.
# The launcher hands off to the game and exits, so by the time we get here it
# is usually already gone; its "no such process" goes to the launcher log
# rather than to the operator reading the verdict.
kill "$launcher" >>"/tmp/prism-$instance.out" 2>&1 || true
pkill -f "Prism Launcher: $instance\"" >>"/tmp/prism-$instance.out" 2>&1 || true

if [ -n "$failed" ]; then
  echo "FAILED $addr"
  grep -m3 -E 'disconnected with reason|Missing tag|Caused by' "$log" || true
  exit 1
fi
if [ "$alive" != yes ]; then
  echo "FAILED $addr: the client exited during the session"
  exit 1
fi
if [ -n "$server_log" ] && [ "$server_after" -le "$server_before" ]; then
  echo "FAILED $addr: no join recorded on the server during the session"
  exit 1
fi
echo "JOINED $addr, still in the world after ${dwell}s"
[ -n "$server_log" ] && echo "server recorded $((server_after - server_before)) join(s)"
exit 0
