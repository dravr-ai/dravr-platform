#!/usr/bin/env bash
# ABOUTME: Fixture test for bin/dev-processes.sh — which process a dev script may stop, and whose a port is
# ABOUTME: Pins that a peer worktree's stack survives a stop and that a stranger's 200 is not read as our own
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Several worktrees of this repo run their own stack at once, so a name —
# "pierre-mcp-server", "expo start", "whatever holds 8081" — names all of them,
# and a 200 on a port says only that the port answered. The library under test
# decides both questions by identity instead, and these cases hold it to that: a
# stop reaches nothing but what this checkout recorded, and a health probe
# accepts nothing but the process this checkout started. Every case builds two
# fixture checkouts and acts as one of them; stand-in services are sleeps and
# throwaway HTTP servers, never a real build.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/../../bin/dev-processes.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() {
    echo "  ❌ $1"
    failures=$((failures + 1))
}

expect() { # label actual expected
    if [ "$2" = "$3" ]; then pass "$1"; else
        fail "$1"
        echo "      got:      $2"
        echo "      expected: $3"
    fi
}

alive() { kill -0 "$1" 2>/dev/null; }

echo "dev-processes.sh fixture tests"

if ! command -v python3 >/dev/null 2>&1; then
    fail "python3 is required to stand up a listener for the readiness cases"
    echo "❌ 1 dev-process-scope case(s) failed"
    exit 1
fi

# Two checkouts side by side, the situation the library exists for. `pwd -P`:
# mktemp lands under /var on macOS, a symlink to /private/var, and lsof reports
# the resolved side — an unresolved root would read a foreign process as ours.
root="$(mktemp -d)"
root="$(cd "$root" && pwd -P)"
a="$root/checkout-a"
b="$root/checkout-b"
serve_dir="$root/served"
mkdir -p "$a/logs" "$b/logs" "$serve_dir"
: >"$serve_dir/health"

started=()
cleanup() {
    local p
    for p in ${started[@]+"${started[@]}"}; do kill -9 "$p" 2>/dev/null || true; done
    rm -rf "$root"
}
trap cleanup EXIT

# Call one library function as a given checkout, from inside it. The
# DEV_PROJECT_ROOT / DEV_RUN_DIR seam is the whole point: the library is
# otherwise anchored to the checkout its own file lives in.
as() { # checkout fn args...
    local dir="$1"
    shift
    (
        cd "$dir" || exit 1
        export DEV_PROJECT_ROOT="$dir" DEV_RUN_DIR="$dir/logs"
        # shellcheck source=../../bin/dev-processes.sh
        . "$UNDER_TEST"
        "$@"
    )
}

# Same, for a snippet that needs several library calls (dev_spawn publishes its
# pid in a variable, so it cannot be reached through a single function call).
as_eval() { # checkout code
    local dir="$1" code="$2"
    (
        cd "$dir" || exit 1
        export DEV_PROJECT_ROOT="$dir" DEV_RUN_DIR="$dir/logs"
        # shellcheck source=../../bin/dev-processes.sh
        . "$UNDER_TEST"
        eval "$code"
    )
}

# A stand-in service running from a given checkout, recorded there under a name.
# `set -m` gives it a group of its own, exactly as dev_spawn does — a recorded
# pid that shares the test runner's group would have dev_stop signal the runner.
start_service() { # checkout name seconds -> sets SERVICE_PID
    local dir="$1" name="$2" secs="$3"
    set -m
    (cd "$dir" && exec sleep "$secs") &
    SERVICE_PID=$!
    disown "$SERVICE_PID" 2>/dev/null || true
    set +m
    started+=("$SERVICE_PID")
    as "$dir" dev_record "$name" "$SERVICE_PID" >/dev/null
}

free_port() { python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()'; }

# Source the shipped library with the seam unset, so the default that decides
# DEV_PROJECT_ROOT runs. `cd "$a"` first: the answer must come from where the
# file lives, never from where the caller stands.
without_override() { # code
    (
        cd "$a" || exit 1
        unset DEV_PROJECT_ROOT DEV_RUN_DIR
        # shellcheck source=../../bin/dev-processes.sh
        . "$UNDER_TEST"
        eval "$1"
    )
}

# --- 0: the default root is the checkout the library ships in ---
# Every other case here overrides DEV_PROJECT_ROOT and DEV_RUN_DIR to reach a
# fixture checkout, which leaves the one line that anchors the library to a
# checkout unexecuted. That line is load-bearing: a root one level too high is
# the parent directory holding every worktree, so all of them write the same
# logs/*.pid and each reads its neighbours' processes as its own — the exact
# cross-checkout kill this library exists to prevent. git is the independent
# answer to where the checkout is.
if repo_root="$(git -C "$(dirname "$UNDER_TEST")" rev-parse --show-toplevel 2>/dev/null)"; then
    repo_root="$(cd "$repo_root" && pwd -P)"
    expect "the default root is the checkout the library ships in" \
        "$(without_override 'echo "$DEV_PROJECT_ROOT"')" "$repo_root"
    expect "the default pid file lands in that checkout's logs/" \
        "$(without_override 'dev_pid_file pierre-server')" "$repo_root/logs/pierre-server.pid"
else
    fail "git can name the checkout the library ships in"
fi

wait_for_listener() { # port
    local _try
    for _try in $(seq 1 60); do
        [ -n "$(lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null | head -1)" ] && return 0
        sleep 0.5
    done
    return 1
}

wait_for_listener_count() { # port n
    local _try
    for _try in $(seq 1 60); do
        [ "$(lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null | sort -u | wc -l | tr -d ' ')" = "$2" ] && return 0
        sleep 0.5
    done
    return 1
}

# A listener sharing one port with another process, recorded in a checkout.
# SO_REUSEPORT reproduces on demand what the dual IPv4/IPv6 stack produces every
# day: two processes, one port, two rows in lsof.
shared_port_listener() { # checkout name port -> sets SERVICE_PID
    local dir="$1" name="$2" port="$3"
    set -m
    (cd "$dir" && exec python3 -c '
import socket, sys, time
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
s.bind(("127.0.0.1", int(sys.argv[1])))
s.listen(8)
time.sleep(120)
' "$port") &
    SERVICE_PID=$!
    disown "$SERVICE_PID" 2>/dev/null || true
    set +m
    started+=("$SERVICE_PID")
    as "$dir" dev_record "$name" "$SERVICE_PID" >/dev/null
}

# --- 1: a peer checkout's stack survives this checkout's stop ---
start_service "$b" pierre-server 40
b_pid="$SERVICE_PID"
start_service "$a" pierre-server 40
a_pid="$SERVICE_PID"
as "$a" dev_stop pierre-server "Pierre MCP Server" >/dev/null

if alive "$b_pid" && [ -e "$b/logs/pierre-server.pid" ]; then
    pass "a peer checkout's server and its pid file survive a stop"
else
    fail "a peer checkout's server and its pid file survive a stop"
fi
if alive "$a_pid"; then fail "this checkout's own server is stopped"; else
    pass "this checkout's own server is stopped"
fi

# --- 2: a recycled pid is never killed ---
start_service "$a" fixture 45
recycled="$SERVICE_PID"
printf '%s\n%s\n%s\n' "$recycled" "$(ps -p "$recycled" -o pgid= | tr -d ' ')" \
    'Thu Jan  1 00:00:00 2026' >"$a/logs/fixture.pid"
out="$(as "$a" dev_stop fixture "Dev Fixture API" 2>&1)"
if alive "$recycled"; then pass "a pid whose start stamp does not match is left alone"; else
    fail "a pid whose start stamp does not match is left alone"
fi
case "$out" in
    *recycled*) pass "the recycled pid is reported, not silently skipped" ;;
    *)
        fail "the recycled pid is reported, not silently skipped"
        echo "      got: $out"
        ;;
esac
if [ -e "$a/logs/fixture.pid" ]; then fail "the stale record is removed"; else
    pass "the stale record is removed"
fi
kill "$recycled" 2>/dev/null || true

# --- 3: a record whose process already exited is not an error ---
start_service "$a" sciotte 1
gone="$SERVICE_PID"
while alive "$gone"; do sleep 0.2; done
rc=0
out="$(as "$a" dev_stop sciotte "Sciotte scraper service" 2>&1)" || rc=$?
expect "stopping a dead record exits 0" "$rc" "0"
case "$out" in
    *"No recorded"*) pass "a dead record reports nothing to stop" ;;
    *)
        fail "a dead record reports nothing to stop"
        echo "      got: $out"
        ;;
esac
if [ -e "$a/logs/sciotte.pid" ]; then fail "a dead record's pid file is removed"; else
    pass "a dead record's pid file is removed"
fi

# --- 4: the whole process group dies, not just the supervisor ---
# bun and expo fork the process that holds the port, which is why the recorded
# pid alone is not enough and the group is what gets signalled.
sup="$(as_eval "$a" 'dev_spawn vite "$DEV_RUN_DIR/vite.log" bash -c "sleep 40 & wait"; echo "$DEV_SPAWNED_PID"')"
started+=("$sup")
sleep 0.5
grandchild="$(pgrep -P "$sup" | head -1)"
if [ -n "$grandchild" ]; then
    started+=("$grandchild")
    as "$a" dev_stop vite "Vite dev server" >/dev/null
    sleep 0.5
    if alive "$sup" || alive "$grandchild"; then
        fail "stopping a supervisor takes its forked worker with it"
    else
        pass "stopping a supervisor takes its forked worker with it"
    fi
else
    fail "the spawned supervisor forked a worker to reap"
fi

# --- 5: a process is resolved to a checkout by its directory ---
start_service "$a" expo 30
a_live="$SERVICE_PID"
start_service "$b" expo 30
b_live="$SERVICE_PID"
if as "$a" dev_pid_is_ours "$a_live"; then pass "a process running from this checkout is ours"; else
    fail "a process running from this checkout is ours"
fi
if as "$a" dev_pid_is_ours "$b_live"; then fail "a process running from a peer checkout is not ours"; else
    pass "a process running from a peer checkout is not ours"
fi
if as "$a" dev_pid_is_ours 1; then fail "a system process is not ours"; else
    pass "a system process is not ours"
fi

# --- 6: readiness rejects a stranger's 200 and accepts our own ---
port="$(free_port)"
http_pid="$(as_eval "$a" 'cd "$serve_dir" && dev_spawn http "$DEV_RUN_DIR/http.log" python3 -m http.server "$port" --bind 127.0.0.1; echo "$DEV_SPAWNED_PID"')"
started+=("$http_pid")
if ! wait_for_listener "$port"; then fail "the fixture HTTP server came up on $port"; fi

start_service "$a" stranger 30
stranger="$SERVICE_PID"
rc=0
out="$(as "$a" dev_wait_healthy "$port" "$stranger" /health 3 2>&1)" || rc=$?
if [ "$rc" -ne 0 ]; then pass "a 200 from another process is not our readiness"; else
    fail "a 200 from another process is not our readiness"
fi
case "$out" in
    *"$http_pid"*"$stranger"*) pass "the failure names the real listener and the pid we started" ;;
    *)
        fail "the failure names the real listener and the pid we started"
        echo "      got: $out"
        ;;
esac
rc=0
as "$a" dev_wait_healthy "$port" "$http_pid" /health 5 >/dev/null 2>&1 || rc=$?
expect "the process that does hold the port reads as healthy" "$rc" "0"
kill "$stranger" 2>/dev/null || true

# --- 7: a stop scoped to the backend leaves this checkout's frontend running ---
# The test-mobile-app skill drives --server-only to simulate a backend outage.
start_service "$a" pierre-server 40
srv="$SERVICE_PID"
start_service "$a" vite 40
web="$SERVICE_PID"
as "$a" dev_stop pierre-server "Pierre MCP Server" >/dev/null
if alive "$srv"; then fail "--server-only stops the backend"; else pass "--server-only stops the backend"; fi
if alive "$web" && [ -e "$a/logs/vite.pid" ]; then
    pass "--server-only leaves this checkout's frontend running"
else
    fail "--server-only leaves this checkout's frontend running"
fi
kill "$web" 2>/dev/null || true

# --- 8: stopping never takes a peer's port; starting does, and names it first ---
# ChefFamille's standing instruction is that a start owns the port it was asked
# for. That licenses killing the ONE listener, never a name pattern, and only
# after saying whose it is.
port="$(free_port)"
peer="$(as_eval "$b" 'dev_spawn peer "$DEV_RUN_DIR/peer.log" python3 -m http.server "$port" --bind 127.0.0.1; echo "$DEV_SPAWNED_PID"')"
started+=("$peer")
if ! wait_for_listener "$port"; then fail "the peer HTTP server came up on $port"; fi

rc=0
out="$(as "$a" dev_reclaim_port "$port" "Pierre MCP Server" 2>&1)" || rc=$?
expect "a stop reports a peer's port instead of taking it" "$rc" "1"
if alive "$peer"; then pass "a stop leaves the peer's listener running"; else
    fail "a stop leaves the peer's listener running"
fi
case "$out" in
    *"held by another checkout"*"$peer"*) pass "the stop names the peer's pid and directory" ;;
    *)
        fail "the stop names the peer's pid and directory"
        echo "      got: $out"
        ;;
esac

rc=0
out="$(as "$a" dev_take_port "$port" "Pierre MCP Server" 2>&1)" || rc=$?
expect "a start takes the port it was asked for" "$rc" "0"
case "$out" in
    *"taking it"*"$peer"*) pass "the start names whose stack it took before killing it" ;;
    *)
        fail "the start names whose stack it took before killing it"
        echo "      got: $out"
        ;;
esac
if alive "$peer"; then fail "the listener the start took is gone"; else pass "the listener the start took is gone"; fi
expect "the port is free afterwards" "$(as "$a" dev_port_listener "$port")" ""


# --- 9: a port with two listeners is resolved to both, not to an arbitrary one ---
# A port normally carries an IPv4 socket and an IPv6 one, and when it is
# contested those belong to different processes. Reading one of them decides
# ownership by whichever lsof happened to print first: our own live server reads
# as absent (so a start that succeeded reports failure) and a take kills one
# holder while the other keeps the port. Both listeners are asserted, so the
# case fails whichever order lsof returns them in.
port="$(free_port)"
shared_port_listener "$b" peer-shared "$port"
peer_shared="$SERVICE_PID"
shared_port_listener "$a" ours-shared "$port"
ours_shared="$SERVICE_PID"
if ! wait_for_listener_count "$port" 2; then
    fail "two processes hold the same port for the contested-port cases"
fi
expect "both holders of the port are listed" \
    "$(as "$a" dev_port_listeners "$port" | sort -u | wc -l | tr -d ' ')" "2"
if as "$a" dev_pid_owns_port "$ours_shared" "$port"; then
    pass "our listener owns the port even when it shares it"
else
    fail "our listener owns the port even when it shares it"
fi
if as "$a" dev_pid_owns_port "$peer_shared" "$port"; then
    pass "the peer's listener owns the port even when it shares it"
else
    fail "the peer's listener owns the port even when it shares it"
fi
start_service "$a" bystander 30
bystander="$SERVICE_PID"
if as "$a" dev_pid_owns_port "$bystander" "$port"; then
    fail "a process that is on neither socket does not own the port"
else
    pass "a process that is on neither socket does not own the port"
fi
kill "$bystander" 2>/dev/null || true

# --- 10: a stop reclaims our holder of a shared port and spares the peer's ---
rc=0
out="$(as "$a" dev_reclaim_port "$port" "Pierre MCP Server" 2>&1)" || rc=$?
expect "reclaiming a shared port reports the peer's half" "$rc" "1"
if alive "$peer_shared"; then pass "the peer's holder of the shared port survives"; else
    fail "the peer's holder of the shared port survives"
fi
sleep 0.5
if alive "$ours_shared"; then fail "our holder of the shared port is stopped"; else
    pass "our holder of the shared port is stopped"
fi
case "$out" in
    *"held by another checkout"*"$peer_shared"*) pass "the reclaim names the peer's surviving holder" ;;
    *)
        fail "the reclaim names the peer's surviving holder"
        echo "      got: $out"
        ;;
esac

# --- 11: a start clears every holder of the port it was asked for ---
shared_port_listener "$a" ours-shared "$port"
ours_shared="$SERVICE_PID"
if ! wait_for_listener_count "$port" 2; then
    fail "two processes hold the port again for the take case"
fi
rc=0
out="$(as "$a" dev_take_port "$port" "Pierre MCP Server" 2>&1)" || rc=$?
expect "a start takes a port off every process holding it" "$rc" "0"
expect "no listener is left on the port" "$(as "$a" dev_port_listeners "$port")" ""
if alive "$peer_shared" || alive "$ours_shared"; then
    fail "both holders of the taken port are gone"
else
    pass "both holders of the taken port are gone"
fi

# --- 12: a spawned service reads no terminal ---
# dev_spawn puts the job in a process group of its own, which makes it a
# BACKGROUND group of the controlling terminal; a process in one that calls
# tcsetattr on that terminal is stopped with SIGTTOU rather than run, and `npx
# expo start` and `bun run dev` both install raw-mode keypress handlers. Feeding
# the caller a stdin that carries data is what shows where the child's stdin
# points: it reads EOF at once, so none of that data reaches the log.
#
# This one case cannot go through as_eval. Bash turns job control off inside a
# subshell, and an asynchronous command started with job control off is given
# /dev/null for stdin by the shell itself — which would mask the very
# redirection under test. The probe below is a script calling dev_spawn from its
# top level, the shape bin/start-server.sh and the setup script have, where
# `set -m` takes effect and the job inherits whatever stdin the caller had.
cat >"$a/stdin-probe.sh" <<'PROBE'
#!/usr/bin/env bash
. "$1"
dev_spawn stdin-probe "$2" sh -c 'cat; echo EOF-REACHED'
wait "$DEV_SPAWNED_PID" 2>/dev/null || true
PROBE
chmod +x "$a/stdin-probe.sh"
probe_log="$a/logs/stdin-probe.log"
printf 'keystrokes-the-service-must-not-see\n' >"$a/keystrokes.txt"
(
    cd "$a" || exit 1
    export DEV_PROJECT_ROOT="$a" DEV_RUN_DIR="$a/logs"
    "$a/stdin-probe.sh" "$UNDER_TEST" "$probe_log" <"$a/keystrokes.txt"
) >/dev/null 2>&1
expect "a spawned service reads /dev/null, never the caller's stdin" \
    "$(cat "$probe_log" 2>/dev/null)" "EOF-REACHED"

echo ""
if [ "$failures" -ne 0 ]; then
    echo "❌ $failures dev-process-scope case(s) failed"
    exit 1
fi
echo "✅ all dev-process-scope cases passed"
