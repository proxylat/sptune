#!/usr/bin/env bash
# One-bundle diagnostic for 429 / hang / high-memory reports.
# Read-only: never touches the live cache, only stats it.
# Output: diag-out/<yyyyMMdd-HHmmss>/SUMMARY.md + per-step logs.
# Light mode (--light): stdout only, skips the JSON cache scan (the slow
# part on big caches) — for quick "is it hot right now?" checks.
set -u
LIGHT=0
if [ "${1:-}" = "--light" ]; then LIGHT=1; fi
if [ "$LIGHT" = 1 ]; then
  echo "## process"
  pgrep -af 'sptune' || echo "no sptune process running"
  ps -o pid,%cpu,rss,etime,cmd -C sptune 2>/dev/null || true
  echo
  echo "## memory pressure"
  free -m 2>/dev/null | head -3 || true
  echo
  echo "## knobs (key names only -- values are secrets, never printed)"
  CFG="${XDG_CONFIG_HOME:-$HOME/.config}/sptune"
  ls -la "$CFG" 2>&1
  echo "playback_poll_secs default=5 clamp(1,30) (app.rs) | tick poll=250ms (events.rs)"
  echo "token bucket: 6.0/s burst 10, 429->Retry-After backoff (backend.rs)"
  exit 0
fi
OUT="diag-out/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
exec >"$OUT/SUMMARY.md" 2>"$OUT/errors.log"

echo "# sptune diag $(date -u +%FT%TZ)"
echo
echo "## versions"
rustc -V 2>&1; cargo -V 2>&1
git rev-parse --short HEAD 2>&1; git status --porcelain 2>&1 | head -20
echo
echo "## process"
pgrep -af 'sptune' || echo "no sptune process running"
ps -o pid,rss,vsz,etime,cmd -C sptune 2>/dev/null || true
echo
echo "## memory pressure"
free -m 2>/dev/null | head -3 || true
cat /sys/fs/cgroup/memory.current 2>/dev/null || cat /sys/fs/cgroup/memory/memory.usage_in_bytes 2>/dev/null || true
echo
echo "## config + caches (stats only, files untouched)"
CFG="${XDG_CONFIG_HOME:-$HOME/.config}/sptune"
ls -la "$CFG" 2>&1
python3 - "$CFG" <<'EOF' 2>&1
import json, os, sys
cfg = sys.argv[1]
def stat(name):
    p = os.path.join(cfg, name)
    try:
        size = os.path.getsize(p)
        d = json.load(open(p))
        if name == 'library_cache.json':
            print(f"{name}: {size}B playlists={len(d.get('playlists', []))}")
        elif name == 'playlist_cache_v2.json':
            n_items = sum(len(v.get('items', [])) if isinstance(v, dict) else 0 for v in d.values())
            print(f"{name}: {size}B cached_playlists={len(d)} total_items~{n_items}")
        elif isinstance(d, dict):
            print(f"{name}: {size}B keys={len(d)}")
        else:
            print(f"{name}: {size}B type={type(d).__name__} len={len(d)}")
    except FileNotFoundError:
        print(f"{name}: MISSING")
    except Exception as e:
        print(f"{name}: {size if 'size' in dir() else '?'}B UNREADABLE ({e})")
for f in ['library_cache.json', 'playlist_cache_v2.json', 'state.json', 'client.yml']:
    stat(f)
try:
    s = json.load(open(os.path.join(cfg, 'state.json')))
    print("state keys:", sorted(s.keys()))
except Exception as e:
    print("state:", e)
EOF
echo
echo "## knobs (key names only -- values are secrets, never printed)"
sed 's/:.*/: <redacted>/' "$CFG/client.yml" 2>/dev/null || echo "no client.yml"
echo "playback_poll_secs default=5 clamp(1,30) (app.rs) | tick poll=250ms (events.rs)"
echo "token bucket: 6.0/s burst 10, 429->Retry-After backoff (backend.rs)"
echo
echo "Raw logs: errors.log in this dir. Bundle: tar -czf $OUT.tar.gz $OUT"
