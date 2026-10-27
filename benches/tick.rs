//! Idle-tick cost: the hot path of the 20%-idle hunt.
//!
//! Locks `update_on_tick` to nanoseconds in the paused states. If it ever
//! grows past ~1µs/iter, the 5s idle heartbeat itself becomes the CPU floor.
//!
//! Quick occasional run (short measurement):
//!   SPTUNE_BENCH_QUICK=1 cargo bench --bench tick
//! Full:
//!   cargo bench --bench tick

use criterion::{criterion_group, criterion_main, Criterion};
use sptune::app::App;
use std::hint::black_box;
use std::time::Duration;

fn quick() -> bool {
  std::env::var("SPTUNE_BENCH_QUICK").is_ok()
}

fn measurement(c: &mut Criterion, name: &str) {
  let mut group = c.benchmark_group(name);
  if quick() {
    group.measurement_time(Duration::from_secs(1));
    group.warm_up_time(Duration::from_millis(200));
  }
  group.bench_function("tick_idle_paused", |b| {
    let mut app = App::default();
    b.iter(|| {
      let painted = app.update_on_tick();
      black_box(painted);
    });
  });
  group.bench_function("tick_fetching", |b| {
    let mut app = App::default();
    app.is_fetching_current_playback = true;
    b.iter(|| {
      let painted = app.update_on_tick();
      black_box(painted);
    });
  });
  group.bench_function("tick_seek_pending", |b| {
    let mut app = App::default();
    app.preview_seek(90_000);
    b.iter(|| {
      let painted = app.update_on_tick();
      black_box(painted);
    });
  });
  group.finish();
}

fn tick(c: &mut Criterion) {
  measurement(c, "tick");
}

criterion_group!(benches, tick);
criterion_main!(benches);
