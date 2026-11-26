# Benchmarks

Measured over 60s with music playing in the background while idle (`./scripts/bench_perf.sh 60`).

- **PSS** (Proportional Set Size): memory accounting for shared libraries proportionally.
- **RSS** (Resident Set Size): total physical memory mapped into the process.

## `sptune` + `soloist` (using Soloist as the playback device)

| Target | Procs | Threads | Avg CPU % | Peak CPU % | Avg PSS | Avg RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `sptune` + `soloist` | 3 | 29 | 1.4% | 2.9% | 85.3 MB | 113.2 MB |

## Spotify Desktop (Electron)

| Target | Procs | Threads | Avg CPU % | Peak CPU % | Avg PSS | Avg RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `spotify` | 9 | 112 | 8.4% | 135.5% | 917.5 MB | 1614.6 MB |

## `sptune` + Spotify Desktop (using Spotify as the playback device)

| Target | Procs | Threads | Avg CPU % | Peak CPU % | Avg PSS | Avg RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `sptune` | 1 | 8 | 0.4% | 1.9% | 24.2 MB | 28.3 MB |
| `spotify` | 9 | 113 | 3.0% | 59.5% | 874.6 MB | 1576.1 MB |

> Anyone can reproduce these benchmarks using [`scripts/bench_perf.sh`](scripts/bench_perf.sh).
