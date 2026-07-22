#!/usr/bin/env bash
# Run the UVM testsuite across two machines in parallel:
#   - local workstation : shard 0  (WORKERS=8)
#   - remote UX430UA     : shard 1  (WORKERS=4)
# Both run concurrently; results are merged at the end.
#
# Also supports single-host modes (sync/build/run/pull) for incremental work.
#
# Usage:
#   scripts/remote_uvm.sh sync       # rsync source + prebuilt DPI .so to remote
#   scripts/remote_uvm.sh build      # cargo build --release on remote
#   scripts/remote_uvm.sh run        # run FULL suite on remote only (WORKERS=4)
#   scripts/remote_uvm.sh split      # HYBRID: 8 local + 4 remote, then merge
#   scripts/remote_uvm.sh all        # sync && build && split
set -euo pipefail

REMOTE=UX430UA
BASE=/home/tom/prog/git
LOCAL_XEZIM=$BASE/xezim
LOCAL_TESTS=$BASE/uvm-tests
REMOTE_BASE=$BASE   # mirror identical path so run_uvm.py's hardcoded paths work

LOCAL_WORKERS="${LOCAL_WORKERS:-8}"
REMOTE_WORKERS="${REMOTE_WORKERS:-4}"
DISPATCH_PORT="${DISPATCH_PORT:-7647}"

RSYNC_EXCLUDES=(
    --exclude 'target/'
    --exclude '.git/'
    --exclude '*.swp'
    --exclude '.nvim/'
    --exclude 'graphify-out/'
    --exclude '/xezim/uvm_status/results*.json'
    --exclude '/xezim/uvm_status/results*.jsonl'
)

sync_repo() {
    local src="$1" dst_parent="$2" name="$3"
    echo ">>> rsync $name"
    rsync -az --delete "${RSYNC_EXCLUDES[@]}" \
        "$src/" "$REMOTE:$dst_parent/$name/"
}

do_sync() {
    ssh -o BatchMode=yes "$REMOTE" "mkdir -p $REMOTE_BASE/xezim $REMOTE_BASE/uvm-tests"
    sync_repo "$LOCAL_XEZIM/xezim"      "$REMOTE_BASE/xezim" xezim
    sync_repo "$LOCAL_XEZIM/xezim-core" "$REMOTE_BASE/xezim" xezim-core
    sync_repo "$LOCAL_XEZIM/1800.2-2020.3.1" "$REMOTE_BASE/xezim" 1800.2-2020.3.1
    sync_repo "$LOCAL_TESTS" "$REMOTE_BASE" uvm-tests
    # uvm_status/ sits at the repo root (sibling of the crate dir).
    sync_repo "$LOCAL_XEZIM/uvm_status" "$REMOTE_BASE/xezim" uvm_status
    echo ">>> copy DPI .so"
    rsync -az "$LOCAL_XEZIM/xezim/uvm-2020.3.1.so" \
        "$REMOTE:$REMOTE_BASE/xezim/xezim/uvm-2020.3.1.so"
    echo ">>> sync done"
}

do_build() {
    echo ">>> remote cargo build --release"
    ssh -o BatchMode=yes "$REMOTE" \
        '. ~/.cargo/env && cd '"$REMOTE_BASE"'/xezim/xezim && cargo build --release 2>&1 | tail -3'
}

# Local shard (runs in background)
_run_local_shard() {
    local out_local="$LOCAL_XEZIM/uvm_status/results_local.json"
    local outl_local="$LOCAL_XEZIM/uvm_status/results_local.jsonl"
    cd "$LOCAL_XEZIM"
    SHARDS=2 SHARD=0 WORKERS="$LOCAL_WORKERS" \
        OUT="$outl_local" OUTJSON="$out_local" \
        timeout 3000 python3 uvm_status/run_uvm.py
}

# Remote shard (runs in background via ssh)
_run_remote_shard() {
    local out_remote_rel="uvm_status/results_remote.json"
    local outl_remote_rel="uvm_status/results_remote.jsonl"
    ssh -o BatchMode=yes "$REMOTE" \
        'cd '"$REMOTE_BASE"'/xezim && SHARDS=2 SHARD=1 WORKERS='"$REMOTE_WORKERS"' \
         OUT='"$outl_remote_rel"' OUTJSON='"$out_remote_rel"' \
         timeout 3000 python3 uvm_status/run_uvm.py'
}

do_split() {
    echo ">>> HYBRID run: $LOCAL_WORKERS local + $REMOTE_WORKERS remote (sharded)"
    local t0=$SECONDS
    # Launch both in background; capture exit codes.
    _run_local_shard  > "$LOCAL_XEZIM/uvm_status/split_local.log"  2>&1 &
    local pid_l=$!
    _run_remote_shard > "$LOCAL_XEZIM/uvm_status/split_remote.log" 2>&1 &
    local pid_r=$!
    echo "    local pid=$pid_l, remote pid=$pid_r"
    local rc_l=0 rc_r=0
    wait "$pid_l" || rc_l=$?
    wait "$pid_r" || rc_r=$?
    local dt=$(( SECONDS - t0 ))
    echo ">>> shards finished in ${dt}s (local rc=$rc_l, remote rc=$rc_r)"
    # Pull remote results back.
    rsync -az \
        "$REMOTE:$REMOTE_BASE/xezim/uvm_status/results_remote.json" \
        "$LOCAL_XEZIM/uvm_status/results_remote.json"
    rsync -az \
        "$REMOTE:$REMOTE_BASE/xezim/uvm_status/results_remote.jsonl" \
        "$LOCAL_XEZIM/uvm_status/results_remote.jsonl"
    echo ">>> merging results"
    python3 "$LOCAL_XEZIM/uvm_status/merge_results.py" \
        "$LOCAL_XEZIM/uvm_status/results_local.json" \
        "$LOCAL_XEZIM/uvm_status/results_remote.json" \
        -o "$LOCAL_XEZIM/uvm_status/results_merged.json"
}

do_dispatch() {
    echo ">>> DYNAMIC dispatch: $LOCAL_WORKERS local + $REMOTE_WORKERS remote"
    local t0=$SECONDS
    cd "$LOCAL_XEZIM"
    local outl="uvm_status/results.jsonl"
    local outj="uvm_status/results.json"
    local slog="$LOCAL_XEZIM/uvm_status/dispatch_server.log"
    local llog="$LOCAL_XEZIM/uvm_status/dispatch_local.log"
    local rlog="$LOCAL_XEZIM/uvm_status/dispatch_remote.log"
    rm -f "$outl" "$outj"
    # 1. coordinator (exits when all results in, or idle for ${IDLE_EXIT}s)
    python3 uvm_status/job_dispatch.py server --port "$DISPATCH_PORT" \
        --out "$outl" --out-json "$outj" > "$slog" 2>&1 &
    local pid_s=$!
    sleep 1   # let it bind
    # 2. local workers
    python3 uvm_status/job_dispatch.py worker --port "$DISPATCH_PORT" \
        --workers "$LOCAL_WORKERS" > "$llog" 2>&1 &
    local pid_l=$!
    # 3. remote workers via SSH reverse tunnel (remote localhost:PORT -> here)
    ssh -o BatchMode=yes -R "${DISPATCH_PORT}:127.0.0.1:${DISPATCH_PORT}" "$REMOTE" \
        "cd $REMOTE_BASE/xezim && python3 uvm_status/job_dispatch.py worker --port $DISPATCH_PORT --workers $REMOTE_WORKERS" \
        > "$rlog" 2>&1 &
    local pid_r=$!
    echo "    server pid=$pid_s, local pid=$pid_l, remote pid=$pid_r"
    # 4. the server knows when we are done — wait for it.
    wait "$pid_s"
    local rc_s=$?
    # workers self-exit once the queue is drained; clean up any stragglers.
    kill "$pid_l" "$pid_r" 2>/dev/null || true
    wait "$pid_l" 2>/dev/null || true
    wait "$pid_r" 2>/dev/null || true
    local dt=$(( SECONDS - t0 ))
    echo ">>> dispatch finished in ${dt}s (server rc=$rc_s)"
    echo "----- server summary -----"
    cat "$slog"
    if [ $rc_s -ne 0 ] || ! grep -q "TOTAL" "$slog"; then
        echo "----- local worker log (tail) -----"; tail -5 "$llog"
        echo "----- remote worker log (tail) -----"; tail -5 "$rlog"
    fi
}

do_run() {
    echo ">>> remote UVM suite (full, WORKERS=$REMOTE_WORKERS)"
    ssh -o BatchMode=yes "$REMOTE" \
        'cd '"$REMOTE_BASE"'/xezim && WORKERS='"$REMOTE_WORKERS"' OUT='"$REMOTE_BASE"'/xezim/uvm_status/results.json OUTJSON='"$REMOTE_BASE"'/xezim/uvm_status/results.json timeout 3000 python3 uvm_status/run_uvm.py 2>&1 | tail -8'
}

case "${1:-all}" in
    sync)  do_sync ;;
    build) do_build ;;
    run)   do_run ;;
    split) do_split ;;
    dispatch) do_dispatch ;;
    all)   do_sync; do_build; do_dispatch ;;
    *) echo "usage: $0 {sync|build|run|split|dispatch|all}"; exit 1 ;;
esac
