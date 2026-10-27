use criterion::{criterion_group, criterion_main, Criterion};
use sptune::user_config::{
  theme_presets, BehaviorConfigString, KeyBindingsString, UserConfig, UserTheme,
};
use std::hint::black_box;

fn keybindings_fixture() -> KeyBindingsString {
  serde_json::from_value(serde_json::json!({
    "back": "z",
    "next_page": "ctrl-j",
    "previous_page": "ctrl-k",
  }))
  .unwrap()
}

fn theme_fixture() -> UserTheme {
  serde_json::from_value(serde_json::json!({
    "active": "Green",
    "banner": "29,185,84",
    "error_border": "Red",
    "selected": "White",
  }))
  .unwrap()
}

fn bench_load_keybindings(c: &mut Criterion) {
  c.bench_function("load_keybindings", |b| {
    b.iter_batched(
      || (UserConfig::new(), keybindings_fixture()),
      |(mut cfg, kb)| cfg.load_keybindings(black_box(kb)).unwrap(),
      criterion::BatchSize::SmallInput,
    );
  });
}

fn bench_load_theme(c: &mut Criterion) {
  c.bench_function("load_theme", |b| {
    b.iter_batched(
      || (UserConfig::new(), theme_fixture()),
      |(mut cfg, theme)| cfg.load_theme(black_box(theme)).unwrap(),
      criterion::BatchSize::SmallInput,
    );
  });
}

fn bench_load_behaviorconfig(c: &mut Criterion) {
  let behavior = BehaviorConfigString {
    seek_milliseconds: Some(5000),
    volume_increment: Some(5),
    tick_rate_milliseconds: Some(250),
    ..Default::default()
  };
  c.bench_function("load_behaviorconfig", |b| {
    b.iter_batched(
      UserConfig::new,
      |mut cfg| {
        cfg
          .load_behaviorconfig(black_box(behavior.clone()))
          .unwrap()
      },
      criterion::BatchSize::SmallInput,
    );
  });
}

fn bench_theme_presets(c: &mut Criterion) {
  c.bench_function("theme_presets", |b| b.iter(|| black_box(theme_presets())));
}

criterion_group!(
  benches,
  bench_load_keybindings,
  bench_load_theme,
  bench_load_behaviorconfig,
  bench_theme_presets,
);
criterion_main!(benches);
