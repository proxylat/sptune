#!/usr/bin/env bash
# Fair, /proc-based CPU & memory benchmark: sptune (+ soloist if running)
# vs Spotify Desktop (Electron). Sums the entire process tree for each side
# using smaps_rollup PSS (with VmRSS fallback) and exact /proc/<pid>/stat
# CPU tick deltas.
#
# Usage: ./scripts/bench_perf.sh [duration_secs]

set -euo pipefail

DURATION="${1:-15}"
CLK_TCK="$(getconf CLK_TCK)"

find_pids() {
  local mode="$1"
  case "$mode" in
    sptune)
      ps -eo pid=,comm= | awk '$2 == "sptune" || $2 == "soloist" { print $1 }'
      ;;
    spotify)
      ps -eo pid=,comm=,args= | awk '($2 == "spotify" || $3 ~ /\/spotify$/) && $0 !~ /sptune/ { print $1 }'
      ;;
  esac
}

# Returns: "<pss_kb> <rss_kb> <cpu_ticks> <procs> <threads>"
sample_group() {
  local mode="$1"
  local pss_sum=0 rss_sum=0 ticks_sum=0 procs=0 threads=0
  local pid val utime stime
  while read -r pid; do
    [[ -n "$pid" && -r "/proc/$pid/stat" ]] || continue
    procs=$((procs + 1))

    # Prefer PSS (proportional set size) for fair shared-library accounting
    if [[ -r "/proc/$pid/smaps_rollup" ]]; then
      val="$(awk '/^Pss:/{print $2; exit}' "/proc/$pid/smaps_rollup" 2>/dev/null || echo 0)"
      pss_sum=$((pss_sum + ${val:-0}))
    fi
    if [[ -r "/proc/$pid/status" ]]; then
      val="$(awk '/^VmRSS:/{print $2; exit}' "/proc/$pid/status" 2>/dev/null || echo 0)"
      rss_sum=$((rss_sum + ${val:-0}))
      [[ -r "/proc/$pid/smaps_rollup" ]] || pss_sum=$((pss_sum + ${val:-0}))
      val="$(awk '/^Threads:/{print $2; exit}' "/proc/$pid/status" 2>/dev/null || echo 0)"
      threads=$((threads + ${val:-0}))
    fi

    # Fields 14 (utime) and 15 (stime) after the closing ')' of comm
    read -r utime stime < <(
      awk '{
        sub(/^.*\) /, "");
        print $12, $13;
      }' "/proc/$pid/stat" 2>/dev/null || echo "0 0"
    )
    ticks_sum=$((ticks_sum + ${utime:-0} + ${stime:-0}))
  done < <(find_pids "$mode")

  echo "$pss_sum $rss_sum $ticks_sum $procs $threads"
}

echo "Sampling sptune (+ soloist) vs Spotify Electron for ${DURATION}s (1s interval)..."

declare -A prev_ticks sum_cpu max_cpu sum_pss max_pss sum_rss max_rss last_procs last_threads samples
for g in sptune spotify; do
  read -r _ _ t _ _ < <(sample_group "$g")
  prev_ticks[$g]="$t"
  sum_cpu[$g]=0; max_cpu[$g]=0
  sum_pss[$g]=0; max_pss[$g]=0
  sum_rss[$g]=0; max_rss[$g]=0
  last_procs[$g]=0; last_threads[$g]=0; samples[$g]=0
done

start_ns="$(date +%s%N)"
end_ns=$(( start_ns + DURATION * 1000000000 ))
prev_ns="$start_ns"

while (( $(date +%s%N) < end_ns )); do
  sleep 1
  now_ns="$(date +%s%N)"
  elapsed_ms=$(( (now_ns - prev_ns) / 1000000 ))
  (( elapsed_ms < 1 )) && elapsed_ms=1
  prev_ns="$now_ns"
  for g in sptune spotify; do
    read -r pss rss ticks procs threads < <(sample_group "$g")
    last_procs[$g]="$procs"
    last_threads[$g]="$threads"
    if (( procs > 0 )); then
      samples[$g]=$((samples[$g] + 1))
      dt=$((ticks - prev_ticks[$g]))
      (( dt < 0 )) && dt=0
      cpu_x10=$(( dt * 1000000 / (CLK_TCK * elapsed_ms) ))
      sum_cpu[$g]=$((sum_cpu[$g] + cpu_x10))
      (( cpu_x10 > max_cpu[$g] )) && max_cpu[$g]="$cpu_x10"
      sum_pss[$g]=$((sum_pss[$g] + pss))
      (( pss > max_pss[$g] )) && max_pss[$g]="$pss"
      sum_rss[$g]=$((sum_rss[$g] + rss))
      (( rss > max_rss[$g] )) && max_rss[$g]="$rss"
    fi
    prev_ticks[$g]="$ticks"
  done
done

echo
printf "%-18s | %-6s | %-7s | %-11s | %-11s | %-11s | %-11s\n" \
  "Target" "Procs" "Threads" "Avg CPU %" "Peak CPU %" "Avg PSS MB" "Avg RSS MB"
printf "%s\n" "-------------------+--------+---------+-------------+-------------+-------------+------------"

for g in sptune spotify; do
  n="${samples[$g]}"
  if (( n == 0 )); then
    printf "%-18s | %-6s | %-7s | %-11s | %-11s | %-11s | %-11s\n" \
      "$g" "0" "0" "not running" "-" "-" "-"
  else
    avg_cpu="$(awk -v s="${sum_cpu[$g]}" -v n="$n" 'BEGIN { printf "%.1f", (s / n) / 10.0 }')"
    peak_cpu="$(awk -v m="${max_cpu[$g]}" 'BEGIN { printf "%.1f", m / 10.0 }')"
    avg_pss="$(awk -v s="${sum_pss[$g]}" -v n="$n" 'BEGIN { printf "%.1f", (s / n) / 1024.0 }')"
    avg_rss="$(awk -v s="${sum_rss[$g]}" -v n="$n" 'BEGIN { printf "%.1f", (s / n) / 1024.0 }')"
    printf "%-18s | %-6s | %-7s | %-11s | %-11s | %-11s | %-11s\n" \
      "$g" "${last_procs[$g]}" "${last_threads[$g]}" "${avg_cpu}%" "${peak_cpu}%" "${avg_pss} MB" "${avg_rss} MB"
  fi
done
