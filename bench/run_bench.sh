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
BENCHES="B2,B3,B4,B5"          # B1 (c910) is opt-in: needs the external RTL
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

echo "host=$HOST arch=$ARCH cpu='$CPU' cores=$NCORE xezim='$XVER' reps=$REPS"

echo "host,arch,cpu,cores,xezim,bench,variant,threads,rep,wall_ms,items,items_per_sec,insns,ns_per_insn,edges_fired,settle_ms,edges_ms,nba_ms,process_ms,fallbacks,work,work_units" > "$OUT"

# Microsecond clock: bash 5's EPOCHREALTIME, expanded INLINE. Reading it
# through a $( ) subshell returned a stale value (the dynamic variable is
# evaluated in the parent before the fork), which produced zero/negative
# intervals; `date +%s%N` per call was likewise flaky under load.

# run <bench> <variant> <threads> <work_units> <file> [extra args...]
run() {
  local bench="$1" variant="$2" threads="$3" units="$4" file="$5"; shift 5
  for rep in $(seq 1 "$REPS"); do
    local t0 t1 wall log
    t0=${EPOCHREALTIME/[.,]/}
    log=$("$XEZIM" --threads "$threads" "$@" "$file" 2>&1)
    t1=${EPOCHREALTIME/[.,]/}
    wall=$(( (t1 - t0) / 1000 ))
    (( wall < 0 )) && wall=0

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

    # The primary rate for each bench: simulated cycles/sec (B2/B3/B4) or
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
    printf '%s,%s,"%s",%s,"%s",%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,"%s",%s\n' \
      "$HOST" "$ARCH" "$CPU" "$NCORE" "$XVER" \
      "$bench" "$variant" "$threads" "$rep" "$wall" \
      "$items" "$rate" \
      "${insns:-0}" "${nspi:-0}" "${edges:-0}" \
      "${settle:-0}" "${edg:-0}" "${nba:-0}" "${proc:-0}" "${fb:-0}" \
      "$work" "$units" >> "$OUT"
    printf '  %-16s %-12s t=%-2s rep%-2s %6s ms  %10s items/s  ns/insn=%s\n' \
      "$bench" "$variant" "$threads" "$rep" "$wall" "$rate" "${nspi:-n/a}"
  done
}

# ---------------------------------------------------------------- B2
if have B2; then
  echo "== B2 vm-dispatch (interpreter dispatch rate, cache-resident)"
  run B2 dispatch 1 4096 "$GEN/b2_vm_dispatch.sv" --max-time 500000
fi

# ---------------------------------------------------------------- B3
if have B3; then
  echo "== B3 mem-sweep (working set L1 -> DRAM)"
  for n in 10 12 14 16 18 20 22; do
    kib=$(( (1 << n) * 4 / 1024 ))
    run B3 "ws_${kib}KiB" 1 "$kib" "$GEN/b3_mem_sweep_${n}.sv" --max-time 250000
  done
fi

# ---------------------------------------------------------------- B4
if have B4; then
  echo "== B4 parallel-scaling (dispatcher sweep)"
  # NOTE: `--threads n` only offloads stdout writes — it is NOT parallel
  # simulation. Parallel edge dispatch is selected with XEZIM_DISPATCHER
  # (the default path already threads when a tick has enough independent
  # blocks), so the sweep is over the dispatcher, not over --threads.
  # This is the benchmark that measures xezim's parallelism, so it is also
  # the one most likely to expose a scaling limit in xezim rather than in
  # the hardware.
  ( unset XEZIM_DISPATCHER; run B4 "disp_default" 1 32 "$GEN/b4_parallel.sv" --max-time 250000 )
  for d in pdes perlp; do
    XEZIM_DISPATCHER="$d" run B4 "disp_${d}" 1 32 "$GEN/b4_parallel.sv" --max-time 250000
  done
  # Same design, more independent units: if the speedup is real it should grow
  # with available parallel work; if it is flat, the limit is the NBA merge.
  XEZIM_DISPATCHER=pdes run B4 "disp_pdes_wide" 1 128 "$GEN/b4_parallel_wide.sv" --max-time 250000
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
