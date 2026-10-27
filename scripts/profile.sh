#!/usr/bin/env bash
# profile.sh — wall-clock profiles for speedscope viewing.
#
# Complements (not replaces) the deterministic gates: iai Ir counts say WHAT
# regressed, criterion means say HOW MUCH, these profiles show WHERE time goes.
# Absolute times here are shape-accurate only: profile builds force frame
# pointers + debuginfo, which perturbs codegen slightly.
#
# Pipeline: perf record -F 99 -m 4 -e task-clock (software event: works at
# no root, no PMU) -> perf script -> profiles/*.perf.txt
# View (local only, never upload):  npx speedscope profiles/<name>.perf.txt
# (official speedscope recipe: perf script text is directly importable.)
#
# Why not samply: `samply record` needs PMU (blocked at paranoid=2), and
# `samply import` of perf.data leaves Rust frames as raw addresses while
# `perf script` resolves them — so samply adds nothing here.
#
# Sampling steps SKIP with exit 0 when perf is unavailable, so the CI
# profile job stays a no-gate artifact producer. Build failures are fatal.
set -u
OUT=profiles
mkdir -p "$OUT"

skip() { echo "profile.sh: SKIP ($1)"; echo "SKIP: $1" > "$OUT/SKIP.txt"; exit 0; }
need() { command -v "$1" >/dev/null 2>&1 || skip "missing tool: $1"; }
export CARGO_BUILD_JOBS="${SPTUNE_PROFILE_JOBS:-1}"
export CARGO_PROFILE_RELEASE_LTO="${SPTUNE_PROFILE_LTO:-thin}"
export CARGO_PROFILE_RELEASE_DEBUG=line-tables-only
# Release strips debuginfo by default (rustc -C strip wins over RUSTFLAGS
# debuginfo) — without this perf script emits raw addresses, no symbols.
export CARGO_PROFILE_RELEASE_STRIP=none
export RUSTFLAGS="${RUSTFLAGS:-} -C force-frame-pointers=yes"

guard_disk() { # release profile builds need room; fail soft, not frozen.
  local avail_kb
  avail_kb=$(df -k target 2>/dev/null | awk 'NR==2{print $4}')
  [ "${avail_kb:-0}" -gt 5242880 ] || skip "low disk for release profile build"
}

guard_mem() { # profile builds with jobs=1 still want ~1.5G; fail soft.
  local avail_kb
  avail_kb=$(awk '/MemAvailable/{print $2}' /proc/meminfo 2>/dev/null)
  [ "${avail_kb:-9999999}" -gt 1048576 ] || skip "low memory for release profile build"
}

guard_build() { # shared prelude for every mode that compiles.
  need perf; need cargo; need timeout
  if [ "${SPTUNE_PROFILE_NO_BUILD:-0}" != 1 ]; then
    guard_disk
    guard_mem
  fi
}

build_release() { # $1 = label; skipped when NO_BUILD=1 (uses existing binary).
  if [ "${SPTUNE_PROFILE_NO_BUILD:-0}" = 1 ]; then
    [ -x ./target/release/sptune ] || { echo "NO_BUILD=1 but no release binary" >&2; exit 1; }
    return 0
  fi
  guard_disk
  guard_mem
  nice -n 19 ionice -c3 cargo build --locked --release || exit 1
}

sample() { # <data> <txt> -- <cmd...>: perf record, then perf script to the
  # speedscope-importable text format (official recipe). perf resolves
  # symbols itself, so no converter needed.
  local data=$1 txt=$2; shift 2
  if ! perf record -F 99 -m 4 -e task-clock -g -o "$data" -- "$@"; then
    # A nonzero child exit (e.g. TUI with no TTY headless) still leaves
    # valid samples; only fail when perf itself produced nothing.
    [ -s "$data" ] || skip "perf record failed (paranoid level? try: echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid)"
  fi
  perf script -i "$data" > "$txt" || skip "perf script failed"
  rm -f "$data" "$OUT/SKIP.txt"
  echo "wrote $txt — view with: npx speedscope $txt"
}

cmd=${1:-help}
case $cmd in
bench)
  # Bench mode: profile one criterion benchmark, e.g.
  #   scripts/profile.sh bench playlist_remove_item
  #   scripts/profile.sh bench "playlist_remove_item/585"
  need perf; need cargo
  filter=${2:?usage: profile.sh bench <criterion-filter>}
  cargo bench --no-run --locked --bench cache || exit 1
  bin=$(ls -t target/release/deps/cache-* | grep -v '\.' | head -1)
  [ -x "$bin" ] || { echo "no cache bench binary" >&2; exit 1; }
  safe=$(echo "$filter" | tr -c 'a-zA-Z0-9' '_')
  sample "$OUT/$safe.perf.data" "$OUT/$safe.perf.txt" \
    "$bin" "$filter" --measurement-time 3 --warm-up-time 1
  ;;
live)
  # Live mode, startup-only: launch the real binary briefly, no workload,
  # no scripted API hammering. Best under a local terminal; headless it
  # captures the init path up to the TTY/auth gate.
  guard_build
  build_release startup
  [ -x ./target/release/sptune ] || { echo "missing release binary" >&2; exit 1; }
  sample "$OUT/startup.perf.data" "$OUT/startup.perf.txt" \
    timeout -k 2 -s INT 8 ./target/release/sptune
  ;;
quick)
  # Quick mode (default occasional check): perf stat counters, no sampling
  # output, ~10s. Cheapest way to answer "is idle still hot?".
  #   scripts/profile.sh quick [secs]
  guard_build
  build_release quick
  [ -x ./target/release/sptune ] || { echo "missing release binary" >&2; exit 1; }
  secs=${2:-10}
  perf stat -e task-clock,cycles,instructions,context-switches \
    -- timeout -k 2 -s INT "$secs" ./target/release/sptune || true
  ;;
idle)
  # Idle mode: sampled profile of a quiescent run (the 20%-idle hunt).
  # Needs a terminal/credentials to get past startup; headless it still
  # profiles whatever the init path does. Default 20s.
  #   scripts/profile.sh idle [secs]
  guard_build
  build_release idle
  [ -x ./target/release/sptune ] || { echo "missing release binary" >&2; exit 1; }
  secs=${2:-20}
  sample "$OUT/idle.perf.data" "$OUT/idle.perf.txt" \
    timeout -k 2 -s INT "$secs" ./target/release/sptune
  ;;
flame)
  # Flame mode: collapse an existing <name>.perf.txt to SVG (needs inferno).
  #   scripts/profile.sh flame idle   # -> profiles/idle.svg
  name=${2:?usage: profile.sh flame <name>}
  txt="$OUT/$name.perf.txt"
  [ -s "$txt" ] || { echo "no profile text: $txt (run bench/live/idle first)" >&2; exit 1; }
  if command -v inferno-collapse-perf >/dev/null 2>&1; then
    collapse="inferno-collapse-perf"; flame="inferno-flamegraph"
  elif command -v cargo-inferno >/dev/null 2>&1; then
    collapse="cargo-inferno collapse-perf"; flame="cargo-inferno flamegraph"
  else
    skip "missing inferno (cargo install inferno)"
  fi
  # shellcheck disable=SC2086
  $collapse "$txt" | $flame > "$OUT/$name.svg" \
    || { echo "flamegraph failed" >&2; exit 1; }
  echo "wrote $OUT/$name.svg"
  ;;
view)
  # scripts/profile.sh view <name>  (requires network for npx on first run)
  need npx
  name=${2:?usage: profile.sh view <name>}
  exec npx -y speedscope "$OUT/$name.perf.txt"
  ;;
*)
  echo "usage: profile.sh bench <filter> | live | quick [secs] | idle [secs] | flame <name> | view <name>" >&2; exit 2
  ;;
esac
