#!/usr/bin/env bash
# xezim cross-platform benchmark runner.
#
#   ./bench/run_bench.sh [-r REPS] [-o OUT.csv] [-b B1,B2,...] [-x XEZIM]
#
# Emits ONE csv with a row per (bench, variant, rep). Work is fixed by the
# designs, so compare ns_per_insn / items_per_sec — never wall alone.
set -uo pipefail

REPS=5
OUT="bench_$(hostname -s)_$(uname -m).csv"
BENCHES="B2,B3,B5"          # B1 (c910) is opt-in: needs the external RTL
XEZIM="./target/release/xezim"
GEN="$(dirname "$0")/gen"

while getopts "r:o:b:x:h" o; do
  case "$o" in
    r) REPS="$OPTARG" ;;
    o) OUT="$OPTARG" ;;
    b) BENCHES="$OPTARG" ;;
    x) XEZIM="$OPTARG" ;;
    h) sed -n '2,10p' "$0"; exit 0 ;;
  esac
done

have() { [[ ",$BENCHES," == *",$1,"* ]]; }

# ---- host identification (goes in every row, so CSVs from different
# ---- machines can simply be concatenated)
HOST=$(hostname -s)
ARCH=$(uname -m)
if [[ -r /proc/cpuinfo ]]; then
  CPU=$(awk -F: '/model name/{gsub(/^ +/,"",$2); print $2; exit}' /proc/cpuinfo)
  [[ -z "$CPU" ]] && CPU=$(awk -F: '/Model name/{gsub(/^ +/,"",$2); print $2; exit}' /proc/cpuinfo)
else
  CPU=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)
fi
CPU=${CPU:-unknown}
NCORE=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 0)
XVER=$("$XEZIM" --version 2>/dev/null | head -1 || echo unknown)

# ---- optional hardware counters. Only the GENERIC perf events are used:
# they are PMU-independent and the kernel maps them on Neoverse/Graviton the
# same as on x86, so the columns stay comparable across platforms. Anything
# arch-specific (LLC-load-misses etc.) is deliberately avoided.
PERF_EVENTS="task-clock,cycles,instructions,branches,branch-misses,cache-references,cache-misses"
PERF=""
if command -v perf >/dev/null 2>&1 && perf stat -x, -e "$PERF_EVENTS" true >/dev/null 2>&1; then
  PERF="yes"
else
  echo "note: perf counters unavailable (missing perf, or perf_event_paranoid too high)."
  echo "      benchmarks still run; the ipc/branch_miss/cache_miss columns will be 0."
fi

echo "host=$HOST arch=$ARCH cpu='$CPU' cores=$NCORE xezim='$XVER' reps=$REPS perf=${PERF:-no}"

echo "host,arch,cpu,cores,xezim,bench,variant,threads,rep,wall_ms,items,items_per_sec,ipc,branch_miss_pct,cache_miss_pct,hw_cycles,hw_instructions,insns,ns_per_insn,edges_fired,settle_ms,edges_ms,nba_ms,process_ms,fallbacks,work,work_units" > "$OUT"

# Microsecond clock: bash 5's EPOCHREALTIME, expanded INLINE. Reading it
# through a $( ) subshell returned a stale value (the dynamic variable is
# evaluated in the parent before the fork), which produced zero/negative
# intervals; `date +%s%N` per call was likewise flaky under load.

# run <bench> <variant> <threads> <work_units> <file> [extra args...]
run() {
  local bench="$1" variant="$2" threads="$3" units="$4" file="$5"; shift 5
  for rep in $(seq 1 "$REPS"); do
    local t0 t1 wall log
    local perf_csv=""
    t0=${EPOCHREALTIME/[.,]/}
    if [[ -n "$PERF" ]]; then
      perf_csv=$(mktemp)
      log=$(perf stat -x, -o "$perf_csv" -e "$PERF_EVENTS" \
              "$XEZIM" --threads "$threads" "$@" "$file" 2>&1)
    else
      log=$("$XEZIM" --threads "$threads" "$@" "$file" 2>&1)
    fi
    t1=${EPOCHREALTIME/[.,]/}
    wall=$(( (t1 - t0) / 1000 ))
    (( wall < 0 )) && wall=0

    # Derive IPC and the two miss RATES (not raw counts): rates are what stay
    # meaningful when the machines run different clock speeds and core counts.
    local hw_cyc=0 hw_ins=0 hw_br=0 hw_brm=0 hw_cref=0 hw_cmis=0
    local ipc=0 brmiss=0 cmiss=0
    if [[ -n "$perf_csv" && -r "$perf_csv" ]]; then
      pget() { awk -F, -v e="$1" '$3 ~ e && $1 ~ /^[0-9.]+$/ {print int($1); exit}' "$perf_csv"; }
      hw_cyc=$(pget '^cycles');            hw_ins=$(pget '^instructions')
      hw_br=$(pget '^branches');           hw_brm=$(pget '^branch-misses')
      hw_cref=$(pget '^cache-references'); hw_cmis=$(pget '^cache-misses')
      hw_cyc=${hw_cyc:-0}; hw_ins=${hw_ins:-0}; hw_br=${hw_br:-0}
      hw_brm=${hw_brm:-0}; hw_cref=${hw_cref:-0}; hw_cmis=${hw_cmis:-0}
      (( hw_cyc  > 0 )) && ipc=$(awk -v a="$hw_ins"  -v b="$hw_cyc"  'BEGIN{printf "%.3f", a/b}')
      (( hw_br   > 0 )) && brmiss=$(awk -v a="$hw_brm" -v b="$hw_br" 'BEGIN{printf "%.2f", 100*a/b}')
      (( hw_cref > 0 )) && cmiss=$(awk -v a="$hw_cmis" -v b="$hw_cref" 'BEGIN{printf "%.2f", 100*a/b}')
      rm -f "$perf_csv"
    fi

    # xezim's own counters: attribute a platform delta to a subsystem
    local insns nspi edges settle edg nba proc fb work
    insns=$(grep -oE 'insns=[0-9]+'            <<<"$log" | head -1 | cut -d= -f2)
    nspi=$( grep -oE 'ns_per_insn=[0-9.]+'     <<<"$log" | head -1 | cut -d= -f2)
    edges=$(grep -oE 'edges_fired=[0-9]+'      <<<"$log" | head -1 | cut -d= -f2)
    fb=$(   grep -oE 'fallbacks=[0-9]+'        <<<"$log" | head -1 | cut -d= -f2)
    settle=$(grep -oE 'settle=[0-9.]+ms'       <<<"$log" | head -1 | tr -dc '0-9.')
    edg=$(  grep -oE ' edges=[0-9.]+ms'        <<<"$log" | head -1 | tr -dc '0-9.')
    nba=$(  grep -oE ' nba=[0-9.]+ms'          <<<"$log" | head -1 | tr -dc '0-9.')
    proc=$( grep -oE ' process=[0-9.]+ms'      <<<"$log" | head -1 | tr -dc '0-9.')
    work=$( grep -oE 'BENCH_DONE.*'            <<<"$log" | head -1 | tr ',' ';')

    if [[ -z "$work" ]]; then
      echo "  !! $bench/$variant rep$rep produced no BENCH_DONE — skipping row" >&2
      echo "$log" | tail -3 >&2
      continue
    fi

    # The primary rate for each bench: simulated cycles/sec (B2/B3) or
    # randomizations/sec (B5). ns_per_insn only covers bytecode-executed
    # work, so it reads 0 for the solver benchmark.
    local items rate
    items=$(grep -oE '(cycles|randomizations)=[0-9]+' <<<"$log" | head -1 | cut -d= -f2)
    items=${items:-0}
    if (( wall > 0 && items > 0 )); then
      rate=$(( items * 1000 / wall ))
    else
      rate=0
    fi
    printf '%s,%s,"%s",%s,"%s",%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,"%s",%s\n' \
      "$HOST" "$ARCH" "$CPU" "$NCORE" "$XVER" \
      "$bench" "$variant" "$threads" "$rep" "$wall" \
      "$items" "$rate" \
      "$ipc" "$brmiss" "$cmiss" "$hw_cyc" "$hw_ins" \
      "${insns:-0}" "${nspi:-0}" "${edges:-0}" \
      "${settle:-0}" "${edg:-0}" "${nba:-0}" "${proc:-0}" "${fb:-0}" \
      "$work" "$units" >> "$OUT"
    printf '  %-16s %-12s t=%-2s rep%-2s %6s ms  %10s items/s  ipc=%-5s brmiss=%-5s%% cmiss=%-5s%%\n' \
      "$bench" "$variant" "$threads" "$rep" "$wall" "$rate" "$ipc" "$brmiss" "$cmiss"
  done
}

# ---------------------------------------------------------------- B2
if have B2; then
  echo "== B2 vm-dispatch (interpreter rate, cache-resident)"
  # Two variants with the same footprint and the same amount of work; only the
  # PREDICTABILITY of the executed path differs. B2a's block order repeats every
  # cycle (the predictor learns it perfectly: ~0.04% miss on x86); B2b dispatches
  # on an LFSR so a different case arm and a different subset of blocks fires
  # each cycle. The DELTA between them is the indirect-branch-predictor cost —
  # which is the thing that actually differs between Zen, Golden Cove and Neoverse.
  run B2 dispatch_regular 1 4096 "$GEN/b2_vm_dispatch.sv" --max-time 500000

  # B2b under the SEQUENTIAL dispatcher: this is the real indirect-branch
  # measurement (the bytecode VM actually runs; `insns` is non-zero).
  XEZIM_NO_PARALLEL=1 run B2 dispatch_branchy 1 4096 "$GEN/b2b_vm_branchy.sv" --max-time 120000

  # Same design with the parallel edge path FORCED on. xezim now calibrates at
  # run time and (on machines where threading loses) picks sequential by itself,
  # so the parallel path has to be forced to be measured at all. The ratio
  # (branchy_par / branchy_seq) is a direct measure of thread fork/join + NBA
  # merge cost — which is exactly what differs between x86 and Graviton, and it
  # also tells you whether xezim's calibration made the right call on this host.
  XEZIM_FORCE_PARALLEL=1 run B2 dispatch_branchy_par 1 4096 "$GEN/b2b_vm_branchy.sv" --max-time 120000
fi

# ---------------------------------------------------------------- B3
if have B3; then
  echo "== B3 mem-sweep (working set L1 -> DRAM)"
  for n in 10 12 14 16 18 20 22; do
    kib=$(( (1 << n) * 4 / 1024 ))
    run B3 "ws_${kib}KiB" 1 "$kib" "$GEN/b3_mem_sweep_${n}.sv" --max-time 250000
  done
fi

# ---------------------------------------------------------------- B5
if have B5; then
  echo "== B5 constraint-rand (solver + PRNG throughput)"
  run B5 randomize 1 20000 "$GEN/b5_constraint_rand.sv" --max-time 1000000
fi

# ---------------------------------------------------------------- B1
if have B1; then
  echo "== B1 c910-hello (real RTL; requires simtest/xuantie_c910 setup)"
  if [[ -x simtest/xuantie_c910/run_c910_hello.sh ]]; then
    for rep in $(seq 1 "$REPS"); do
      t0=$(date +%s%N)
      log=$(simtest/xuantie_c910/run_c910_hello.sh 2>&1)
      t1=$(date +%s%N)
      wall=$(( (t1 - t0) / 1000000 ))
      nspi=$(grep -oE 'ns_per_insn=[0-9.]+' <<<"$log" | head -1 | cut -d= -f2)
      insns=$(grep -oE 'insns=[0-9]+' <<<"$log" | head -1 | cut -d= -f2)
      printf '%s,%s,"%s",%s,"%s",B1,c910_hello,1,%s,%s,%s,%s,0,0,0,0,0,0,"c910",1\n' \
        "$HOST" "$ARCH" "$CPU" "$NCORE" "$XVER" "$rep" "$wall" "${insns:-0}" "${nspi:-0}" >> "$OUT"
      printf '  %-16s %-12s rep%-2s %6s ms  ns/insn=%s\n' B1 c910_hello "$rep" "$wall" "${nspi:-n/a}"
    done
  else
    echo "  (skipped: simtest/xuantie_c910 not set up on this host)"
  fi
fi

echo
echo "wrote $OUT ($(( $(wc -l < "$OUT") - 1 )) rows)"
