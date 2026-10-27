//! Real spectrum visualizer: OS monitor/loopback capture + FFT bins.
//!
//! Linux records the default sink's monitor source via PulseAudio, Windows
//! uses WASAPI loopback — either way the bars follow whatever is actually
//! playing (system audio, not Spotify specifically). Capture runs only
//! while the Music view is open; the render loop only ever reads the
//! latest smoothed snapshot.

use std::f32::consts::PI;
use std::sync::{
  atomic::{AtomicBool, Ordering},
  Arc, Mutex,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustfft::{num_complex::Complex, FftPlanner};

/// Number of bars rendered.
pub const VIZ_BINS: usize = 24;
/// FFT window: 2048 mono samples at 44.1kHz is ~46ms per chunk.
const VIZ_WINDOW: usize = 2048;
const VIZ_RATE: f32 = 44100.0;
/// Retry capture setup at most this often while the view stays open.
const VIZ_RETRY: Duration = Duration::from_secs(5);

// Render physics, ported from cliamp's classic peak / LED drivers:
// exponential body smoothing (fast attack, slow decay) plus ballistic
// peak caps (launch on rise, gravity fall, brief apex hold).
const BODY_RISE: f32 = 34.0;
const BODY_FALL: f32 = 10.0;
const CAP_LAUNCH_BASE: f32 = 0.8;
const CAP_LAUNCH_GAIN: f32 = 1.4;
const CAP_LAUNCH_MAX: f32 = 1.7;
const CAP_GRAVITY: f32 = 9.5;
const CAP_APEX_HOLD: f32 = 0.08;
const CAP_EPS: f32 = 0.01;
// Winamp LED matrix: chunkier body smoothing, quantized peak caps that
// hold at the apex then fall at constant speed.
const LED_RISE: f32 = 60.0;
const LED_FALL: f32 = 16.0;
const LED_PEAK_HOLD: f32 = 0.45;
const LED_PEAK_FALL: f32 = 0.55;
/// Fallback physics step when no previous frame timestamp exists.
const DEFAULT_DT: f32 = 1.0 / 30.0;

/// Visualizer render style, cycled with the `v` key. Bands-only modes
/// share the 24-bin FFT snapshot; scope/waveform modes (cliamp's Scope,
/// Wave, Stereo) need raw time-domain samples the capture path doesn't
/// deliver, so they are deliberately absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VizMode {
  Bars,
  Peak,
  Bricks,
  Columns,
  Ascii,
  Led,
  Mosaic,
  Off,
}

impl VizMode {
  pub fn name(self) -> &'static str {
    match self {
      VizMode::Bars => "Bars",
      VizMode::Peak => "Peak",
      VizMode::Bricks => "Bricks",
      VizMode::Columns => "Columns",
      VizMode::Ascii => "Ascii",
      VizMode::Led => "LED",
      VizMode::Mosaic => "Mosaic",
      VizMode::Off => "Off",
    }
  }

  pub fn cycle(self) -> VizMode {
    match self {
      VizMode::Bars => VizMode::Peak,
      VizMode::Peak => VizMode::Bricks,
      VizMode::Bricks => VizMode::Columns,
      VizMode::Columns => VizMode::Ascii,
      VizMode::Ascii => VizMode::Led,
      VizMode::Led => VizMode::Mosaic,
      VizMode::Mosaic => VizMode::Off,
      VizMode::Off => VizMode::Bars,
    }
  }

  pub fn from_str(s: &str) -> VizMode {
    match s.to_ascii_lowercase().as_str() {
      "peak" => VizMode::Peak,
      "bricks" => VizMode::Bricks,
      "columns" => VizMode::Columns,
      "ascii" => VizMode::Ascii,
      "led" => VizMode::Led,
      "mosaic" => VizMode::Mosaic,
      "off" => VizMode::Off,
      _ => VizMode::Bars,
    }
  }
}

/// One Mosaic heatmap tile: fixed band + ignition threshold, decaying value.
#[derive(Clone, Copy)]
struct MosaicCell {
  band: u8,
  thresh: f32,
  value: f32,
}

pub struct VizState {
  /// Latest smoothed snapshot (0.0..=1.0) for the renderer.
  pub levels: [f32; VIZ_BINS],
  /// Active render style.
  pub mode: VizMode,
  /// Exponentially smoothed body shared by the bar modes.
  smooth: [f32; VIZ_BINS],
  /// Ballistic peak caps (Peak mode): position, velocity, apex hold left.
  cap_pos: [f32; VIZ_BINS],
  cap_vel: [f32; VIZ_BINS],
  cap_hold: [f32; VIZ_BINS],
  /// LED-matrix body + quantized peaks with timed holds.
  led_body: [f32; VIZ_BINS],
  led_peak: [f32; VIZ_BINS],
  led_hold: [f32; VIZ_BINS],
  /// Mosaic tile grid (row-major); rebuilt when the box resizes.
  mosaic: Vec<MosaicCell>,
  mosaic_rows: usize,
  mosaic_tiles: usize,
  last_tick: Option<Instant>,
  /// Capture source label for the corner debug readout (empty = none).
  pub source: String,
  worker: Option<VizWorker>,
  retry_at: Option<Instant>,
}

struct VizWorker {
  stop: Arc<AtomicBool>,
  levels: Arc<Mutex<[f32; VIZ_BINS]>>,
  thread: Option<JoinHandle<()>>,
  source: String,
}

impl VizState {
  pub fn new() -> Self {
    Self {
      levels: [0.0; VIZ_BINS],
      mode: VizMode::Bars,
      smooth: [0.0; VIZ_BINS],
      cap_pos: [0.0; VIZ_BINS],
      cap_vel: [0.0; VIZ_BINS],
      cap_hold: [0.0; VIZ_BINS],
      led_body: [0.0; VIZ_BINS],
      led_peak: [0.0; VIZ_BINS],
      led_hold: [0.0; VIZ_BINS],
      mosaic: Vec::new(),
      mosaic_rows: 0,
      mosaic_tiles: 0,
      last_tick: None,
      source: String::new(),
      worker: None,
      retry_at: None,
    }
  }

  /// Switch style, dropping transient physics so the new mode starts clean.
  pub fn set_mode(&mut self, mode: VizMode) {
    self.mode = mode;
    self.smooth = [0.0; VIZ_BINS];
    self.cap_pos = [0.0; VIZ_BINS];
    self.cap_vel = [0.0; VIZ_BINS];
    self.cap_hold = [0.0; VIZ_BINS];
    self.led_body = [0.0; VIZ_BINS];
    self.led_peak = [0.0; VIZ_BINS];
    self.led_hold = [0.0; VIZ_BINS];
    self.mosaic.clear();
    self.mosaic_rows = 0;
    self.mosaic_tiles = 0;
    self.last_tick = None;
  }

  /// Advance body smoothing, ballistic caps and LED physics by one frame.
  /// Called from the renderer; dt comes from the wall clock so any tick
  /// cadence animates correctly, clamped like cliamp's drivers.
  pub fn tick(&mut self) {
    let now = Instant::now();
    let mut dt = self
      .last_tick
      .map(|t| (now - t).as_secs_f32())
      .unwrap_or(DEFAULT_DT);
    if dt <= 0.0 || dt > 10.0 * DEFAULT_DT {
      dt = DEFAULT_DT;
    }
    self.last_tick = Some(now);
    self.advance(dt);
  }

  /// Physics step with an explicit dt (unit-testable; `tick` supplies the
  /// wall-clock dt).
  fn advance(&mut self, dt: f32) {
    for i in 0..VIZ_BINS {
      let target = self.levels[i].clamp(0.0, 1.0);
      // Shared smoothed body (bar modes).
      let rate = if target > self.smooth[i] {
        BODY_RISE
      } else {
        BODY_FALL
      };
      self.smooth[i] += (target - self.smooth[i]) * (1.0 - (-rate * dt).exp());
      // Ballistic caps: launch when landed and the bar rises, integrate
      // gravity, hold briefly at the apex, land back on the body.
      let body = self.smooth[i];
      if self.cap_vel[i] == 0.0 && self.cap_pos[i] <= body + CAP_EPS && target > self.cap_pos[i] {
        let delta = target - self.cap_pos[i];
        self.cap_pos[i] = target;
        self.cap_vel[i] = (CAP_LAUNCH_BASE + CAP_LAUNCH_GAIN * delta).min(CAP_LAUNCH_MAX);
        self.cap_hold[i] = 0.0;
      }
      if self.cap_hold[i] > 0.0 {
        self.cap_hold[i] = (self.cap_hold[i] - dt).max(0.0);
      } else {
        let prev_vel = self.cap_vel[i];
        self.cap_pos[i] += self.cap_vel[i] * dt;
        self.cap_vel[i] -= CAP_GRAVITY * dt;
        if self.cap_pos[i] > 1.0 {
          self.cap_pos[i] = 1.0;
        }
        if prev_vel > 0.0 && self.cap_vel[i] <= 0.0 && self.cap_pos[i] > body + CAP_EPS {
          self.cap_vel[i] = 0.0;
          self.cap_hold[i] = CAP_APEX_HOLD;
        } else if self.cap_pos[i] <= body {
          self.cap_pos[i] = body;
          self.cap_vel[i] = 0.0;
          self.cap_hold[i] = 0.0;
        }
      }
      // LED matrix: snappier body, peaks snap then hold + linear fall.
      let rate = if target > self.led_body[i] {
        LED_RISE
      } else {
        LED_FALL
      };
      self.led_body[i] += (target - self.led_body[i]) * (1.0 - (-rate * dt).exp());
      if self.led_body[i] >= self.led_peak[i] {
        self.led_peak[i] = self.led_body[i];
        self.led_hold[i] = LED_PEAK_HOLD;
      } else if self.led_hold[i] > 0.0 {
        self.led_hold[i] = (self.led_hold[i] - dt).max(0.0);
      } else {
        self.led_peak[i] = self.led_body[i].max(self.led_peak[i] - LED_PEAK_FALL * dt);
      }
    }
  }

  /// Smoothed body value for bin `i`.
  pub fn smooth(&self, i: usize) -> f32 {
    self.smooth[i]
  }

  /// Ballistic cap position for bin `i`.
  pub fn cap(&self, i: usize) -> f32 {
    self.cap_pos[i]
  }

  /// LED body / peak pair for bin `i`.
  pub fn led(&self, i: usize) -> (f32, f32) {
    (self.led_body[i], self.led_peak[i])
  }

  /// Advance the Mosaic heatmap for a `rows` x `tiles` grid, rebuilding
  /// the wiring (row-biased band + per-cell threshold) on resize.
  /// Cells ignite when their band exceeds their threshold, else decay.
  pub fn mosaic_tick(&mut self, rows: usize, tiles: usize) {
    if rows != self.mosaic_rows || tiles != self.mosaic_tiles {
      self.mosaic_rows = rows;
      self.mosaic_tiles = tiles;
      self.mosaic.clear();
      // Deterministic LCG (same constants as cliamp) so the wiring is
      // stable for a given grid size.
      let mut rng: u64 = 0xC1AB1A1015D5;
      let mut next = move || {
        rng = rng
          .wrapping_mul(6364136223846793005)
          .wrapping_add(1442695040888963407);
        (rng >> 33) as usize
      };
      for r in 0..rows {
        let base = if rows > 1 {
          (rows - 1 - r) * (VIZ_BINS - 1) / (rows - 1)
        } else {
          VIZ_BINS / 2
        };
        for _ in 0..tiles {
          let band =
            (base as isize + next() as isize % 5 - 2).clamp(0, VIZ_BINS as isize - 1) as u8;
          let thresh = 0.04 + (next() % 1000) as f32 / 1000.0 * 0.74;
          self.mosaic.push(MosaicCell {
            band,
            thresh,
            value: 0.0,
          });
        }
      }
    }
    for cell in &mut self.mosaic {
      let level = self.smooth[cell.band as usize];
      if level > cell.thresh && level.min(1.05) > cell.value {
        cell.value = level.min(1.05);
      }
      cell.value *= 0.88;
      if cell.value < 0.001 {
        cell.value = 0.0;
      }
    }
  }

  /// Value of the Mosaic tile at (`row`, `tile`).
  pub fn mosaic_value(&self, row: usize, tile: usize) -> f32 {
    self
      .mosaic
      .get(row * self.mosaic_tiles + tile)
      .map(|c| c.value)
      .unwrap_or(0.0)
  }

  /// Start capture if needed and refresh the snapshot. Cheap: one lock;
  /// a failed setup retries at most every few seconds.
  pub fn poll(&mut self) {
    // Reap a finished worker so a silently died thread respawns (with
    // backoff, not a hot loop).
    if let Some(w) = &self.worker {
      let dead = w.thread.as_ref().is_some_and(|t| t.is_finished());
      if dead {
        if let Some(mut w) = self.worker.take() {
          if let Some(t) = w.thread.take() {
            let _ = t.join();
          }
        }
        self.retry_at = Some(Instant::now() + VIZ_RETRY);
      }
    }
    if self.worker.is_none() && self.retry_at.is_none_or(|t| Instant::now() >= t) {
      match VizWorker::spawn() {
        Some(w) => self.worker = Some(w),
        None => self.retry_at = Some(Instant::now() + VIZ_RETRY),
      }
    }
    if let Some(w) = &self.worker {
      self.source = w.source.clone();
      if let Ok(levels) = w.levels.lock() {
        self.levels = *levels;
      }
    } else {
      self.source.clear();
    }
  }

  /// Stop capture (Music view left). The thread exits on its next chunk.
  pub fn stop(&mut self) {
    if let Some(mut w) = self.worker.take() {
      w.stop.store(true, Ordering::Relaxed);
      if let Some(t) = w.thread.take() {
        let _ = t.join();
      }
      self.levels = [0.0; VIZ_BINS];
      self.set_mode(self.mode);
      self.source.clear();
    }
    self.retry_at = None;
  }
}

impl VizWorker {
  fn spawn() -> Option<Self> {
    #[cfg(target_os = "linux")]
    return Self::spawn_linux();
    #[cfg(target_os = "windows")]
    return Self::spawn_windows();
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    return None;
  }

  #[cfg(target_os = "linux")]
  fn spawn_linux() -> Option<Self> {
    use libpulse_binding::{
      channelmap::Map as ChanMap,
      def::BufferAttr,
      sample::{Format, Spec},
      stream::Direction,
    };
    use libpulse_simple_binding::Simple;

    let monitor = monitor_source()?;
    let spec = Spec {
      format: Format::S16le,
      rate: VIZ_RATE as u32,
      channels: 1,
    };
    let attr = BufferAttr {
      maxlength: u32::MAX,
      tlength: u32::MAX,
      prebuf: u32::MAX,
      minreq: u32::MAX,
      fragsize: (VIZ_WINDOW * 2) as u32,
    };
    let simple = Simple::new(
      None,
      "sptune",
      Direction::Record,
      Some(&monitor),
      "visualizer",
      &spec,
      None::<&ChanMap>,
      Some(&attr),
    )
    .ok()?;
    let stop = Arc::new(AtomicBool::new(false));
    let levels = Arc::new(Mutex::new([0.0; VIZ_BINS]));
    let thread = std::thread::Builder::new()
      .name("sptune-viz".into())
      .spawn({
        let stop = Arc::clone(&stop);
        let levels = Arc::clone(&levels);
        move || capture_loop(simple, stop, levels)
      })
      .ok()?;
    Some(Self {
      stop,
      levels,
      thread: Some(thread),
      source: monitor,
    })
  }

  /// WASAPI loopback on the default render endpoint (system audio).
  /// Render endpoint + Capture direction + Shared = LOOPBACK flag inside
  /// `initialize_client`; autoconvert yields f32 stereo at 44.1kHz.
  #[cfg(target_os = "windows")]
  fn spawn_windows() -> Option<Self> {
    use wasapi::*;

    initialize_mta().ok()?;
    let enumerator = DeviceEnumerator::new().ok()?;
    let device = enumerator.get_default_device(&Direction::Render).ok()?;
    let mut audio_client = device.get_iaudioclient().ok()?;
    let format = WaveFormat::new(32, 32, &SampleType::Float, VIZ_RATE as usize, 2, None);
    let (_, min_period) = audio_client.get_device_period().ok()?;
    audio_client
      .initialize_client(
        &format,
        &Direction::Capture,
        &StreamMode::PollingShared {
          autoconvert: true,
          buffer_duration_hns: min_period,
        },
      )
      .ok()?;
    let capture = audio_client.get_audiocaptureclient().ok()?;
    audio_client.start_stream().ok()?;
    let stop = Arc::new(AtomicBool::new(false));
    let levels = Arc::new(Mutex::new([0.0; VIZ_BINS]));
    let thread = std::thread::Builder::new()
      .name("sptune-viz".into())
      .spawn({
        let stop = Arc::clone(&stop);
        let levels = Arc::clone(&levels);
        move || {
          capture_loop_windows(audio_client, capture, stop, levels);
        }
      })
      .ok()?;
    Some(Self {
      stop,
      levels,
      thread: Some(thread),
      source: "loopback".into(),
    })
  }
}

/// Read chunks, FFT, fold into log-spaced bins with peak-decay smoothing.
#[cfg(target_os = "linux")]
fn capture_loop(
  simple: libpulse_simple_binding::Simple,
  stop: Arc<AtomicBool>,
  levels: Arc<Mutex<[f32; VIZ_BINS]>>,
) {
  let mut planner = FftPlanner::<f32>::new();
  let fft = planner.plan_fft_forward(VIZ_WINDOW);
  let hann: Vec<f32> = (0..VIZ_WINDOW)
    .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / VIZ_WINDOW as f32).cos()))
    .collect();
  let mut buf = vec![Complex::new(0.0, 0.0); VIZ_WINDOW];
  let mut raw = vec![0u8; VIZ_WINDOW * 2];
  let mut peaks = [0.0f32; VIZ_BINS];
  let mut errs = 0u32;
  while !stop.load(Ordering::Relaxed) {
    if simple.read(&mut raw).is_err() {
      // Transient underruns happen; only give up after a sustained failure
      // so one hiccup no longer kills the bars forever.
      errs += 1;
      if errs > 50 {
        break;
      }
      std::thread::sleep(Duration::from_millis(10));
      continue;
    }
    errs = 0;
    for (i, s) in raw.as_chunks::<2>().0.iter().enumerate() {
      let v = i16::from_le_bytes(*s) as f32 / 32768.0;
      buf[i] = Complex::new(v * hann[i], 0.0);
    }
    let snapshot = analyze(&mut buf, &fft, &mut peaks);
    if let Ok(mut l) = levels.lock() {
      *l = snapshot;
    }
  }
  // Whatever ended the loop, don't leave stale levels behind.
  if let Ok(mut l) = levels.lock() {
    *l = [0.0; VIZ_BINS];
  }
}

/// Windowed time-domain samples in, smoothed 0.0..=1.0 bin levels out.
/// Pure DSP shared by the Linux and Windows capture backends (unit-tested below).
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn analyze(
  buf: &mut [Complex<f32>],
  fft: &std::sync::Arc<dyn rustfft::Fft<f32>>,
  peaks: &mut [f32; VIZ_BINS],
) -> [f32; VIZ_BINS] {
  // Log-spaced bins, 30Hz..16kHz.
  fft.process(buf);
  let mut snapshot = [0.0f32; VIZ_BINS];
  for (b, level) in snapshot.iter_mut().enumerate() {
    let lo = bin_edge(b).min(VIZ_WINDOW / 2);
    let hi = bin_edge(b + 1).clamp(1, VIZ_WINDOW / 2 + 1);
    // Hann-windowed full-scale sine peaks near N/4 (single-sided).
    let mut mag: f32 = 0.0;
    for c in &buf[lo..hi] {
      mag = mag.max(c.norm());
    }
    let target = (mag / (VIZ_WINDOW as f32 * 0.25)).min(1.0);
    if target > peaks[b] {
      peaks[b] = target;
    } else {
      peaks[b] = (peaks[b] - 0.02).max(target).max(0.0);
    }
    *level = peaks[b];
  }
  snapshot
}

/// FFT-bin edge for log bin `b` (0..=VIZ_BINS).
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn bin_edge(b: usize) -> usize {
  (30.0f32 * (16000.0f32 / 30.0f32).powf(b as f32 / VIZ_BINS as f32) * VIZ_WINDOW as f32 / VIZ_RATE)
    as usize
}

/// WASAPI loopback reader: stereo f32 frames → mono → shared `analyze`.
/// `audio_client` is only held to stop the stream on exit.
#[cfg(target_os = "windows")]
fn capture_loop_windows(
  audio_client: wasapi::AudioClient,
  capture: wasapi::AudioCaptureClient,
  stop: Arc<AtomicBool>,
  levels: Arc<Mutex<[f32; VIZ_BINS]>>,
) {
  use std::collections::VecDeque;

  let mut planner = FftPlanner::<f32>::new();
  let fft = planner.plan_fft_forward(VIZ_WINDOW);
  let hann: Vec<f32> = (0..VIZ_WINDOW)
    .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / VIZ_WINDOW as f32).cos()))
    .collect();
  let mut buf = vec![Complex::new(0.0, 0.0); VIZ_WINDOW];
  let mut peaks = [0.0f32; VIZ_BINS];
  // f32 stereo frames: 8 bytes each.
  let mut queue: VecDeque<u8> = VecDeque::with_capacity(VIZ_WINDOW * 8 * 4);
  while !stop.load(Ordering::Relaxed) {
    if capture.read_from_device_to_deque(&mut queue).is_err() {
      break;
    }
    if queue.len() < VIZ_WINDOW * 8 {
      std::thread::sleep(Duration::from_millis(10));
      continue;
    }
    // Drain::as_slice doesn't exist; linearize first, then drain.
    let contig = queue.make_contiguous();
    for (i, frame) in contig[..VIZ_WINDOW * 8]
      .as_chunks::<8>()
      .0
      .iter()
      .enumerate()
    {
      // Left channel only; loopback is system audio, mono is enough to see.
      let v = f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
      buf[i] = Complex::new(v * hann[i], 0.0);
    }
    queue.drain(..VIZ_WINDOW * 8);
    let snapshot = analyze(&mut buf, &fft, &mut peaks);
    if let Ok(mut l) = levels.lock() {
      *l = snapshot;
    }
  }
  let _ = audio_client.stop_stream();
}

/// Default sink's monitor source name, via one-shot introspection.
#[cfg(target_os = "linux")]
fn monitor_source() -> Option<String> {
  use libpulse_binding::callbacks::ListResult;
  use libpulse_binding::context::introspect::SinkInfo;
  use libpulse_binding::context::{introspect::ServerInfo, Context, FlagSet, State as CtxState};
  use libpulse_binding::mainloop::standard::{IterateResult, Mainloop};
  use libpulse_binding::operation::State as OpState;
  use std::cell::RefCell;
  use std::rc::Rc;

  let mut ml = Mainloop::new()?;
  let mut ctx = Context::new(&ml, "sptune-viz")?;
  ctx.connect(None, FlagSet::NOFLAGS, None).ok()?;
  loop {
    match ml.iterate(true) {
      IterateResult::Quit(_) | IterateResult::Err(_) => return None,
      _ => {}
    }
    match ctx.get_state() {
      CtxState::Ready => break,
      CtxState::Failed | CtxState::Terminated => return None,
      _ => {}
    }
  }
  let intro = ctx.introspect();
  let sink_name: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
  let op = intro.get_server_info({
    let sink_name = Rc::clone(&sink_name);
    move |info: &ServerInfo| {
      *sink_name.borrow_mut() = info.default_sink_name.as_ref().map(|s| s.to_string());
    }
  });
  loop {
    ml.iterate(true);
    if op.get_state() == OpState::Done {
      break;
    }
  }
  drop(op);
  let sink_name = sink_name.borrow().clone()?;
  let monitor: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
  let op = intro.get_sink_info_by_name(&sink_name, {
    let monitor = Rc::clone(&monitor);
    move |r: ListResult<&SinkInfo>| {
      if let ListResult::Item(info) = r {
        *monitor.borrow_mut() = info.monitor_source_name.as_ref().map(|s| s.to_string());
      }
    }
  });
  loop {
    ml.iterate(true);
    if op.get_state() == OpState::Done {
      break;
    }
  }
  drop(op);
  let out = monitor.borrow().as_ref().cloned();
  out
}

#[cfg(all(test, any(target_os = "linux", target_os = "windows")))]
mod tests {
  use super::*;

  /// Synthetic 440Hz sine must land in the bin whose edges contain it.
  #[test]
  fn sine_440_peaks_in_expected_bin() {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(VIZ_WINDOW);
    let mut buf: Vec<Complex<f32>> = (0..VIZ_WINDOW)
      .map(|i| {
        let t = i as f32 / VIZ_RATE;
        let v = (2.0 * PI * 440.0 * t).sin();
        let hann = 0.5 * (1.0 - (2.0 * PI * i as f32 / VIZ_WINDOW as f32).cos());
        Complex::new(v * hann, 0.0)
      })
      .collect();
    let mut peaks = [0.0f32; VIZ_BINS];
    let levels = analyze(&mut buf, &fft, &mut peaks);
    // Bin containing 440Hz: 24*ln(440/30)/ln(16000/30) ≈ 10.3 → bin 10.
    let hot = levels
      .iter()
      .enumerate()
      .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
      .unwrap()
      .0;
    assert_eq!(hot, 10, "440Hz landed in bin {hot}, levels: {levels:?}");
    assert!(levels[10] > 0.5, "bin 10 too quiet: {:?}", levels);
  }

  #[test]
  fn silence_stays_zero() {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(VIZ_WINDOW);
    let mut buf = vec![Complex::new(0.0, 0.0); VIZ_WINDOW];
    let mut peaks = [0.0f32; VIZ_BINS];
    let levels = analyze(&mut buf, &fft, &mut peaks);
    assert!(levels.iter().all(|&l| l == 0.0));
  }

  #[test]
  fn mode_cycle_covers_all_names() {
    let mut m = VizMode::Bars;
    let mut seen = vec![m.name()];
    for _ in 0..7 {
      m = m.cycle();
      seen.push(m.name());
    }
    assert_eq!(
      seen,
      vec!["Bars", "Peak", "Bricks", "Columns", "Ascii", "LED", "Mosaic", "Off"]
    );
    assert_eq!(m.cycle(), VizMode::Bars);
  }

  #[test]
  fn mode_from_str_falls_back_to_bars() {
    assert_eq!(VizMode::from_str("LED"), VizMode::Led);
    assert_eq!(VizMode::from_str("mosaic"), VizMode::Mosaic);
    assert_eq!(VizMode::from_str("nonsense"), VizMode::Bars);
  }

  #[test]
  fn tick_smooths_body_and_launches_caps() {
    let mut viz = VizState::new();
    viz.levels = [0.9; VIZ_BINS];
    viz.advance(1.0 / 30.0);
    // Fast attack moves the body most of the way in one 33ms frame.
    assert!(viz.smooth(0) > 0.5, "body too slow: {}", viz.smooth(0));
    // A landed cap launches upward off a rising bar.
    assert!(viz.cap(0) >= 0.9, "cap didn't launch: {}", viz.cap(0));
    // Silence decays the body back down.
    viz.levels = [0.0; VIZ_BINS];
    for _ in 0..120 {
      viz.advance(1.0 / 30.0);
    }
    assert!(viz.smooth(0) < 0.05, "body stuck: {}", viz.smooth(0));
  }

  #[test]
  fn led_peak_holds_then_falls() {
    let mut viz = VizState::new();
    viz.levels = [0.8; VIZ_BINS];
    viz.advance(1.0 / 30.0);
    let (_, peak) = viz.led(0);
    assert!(peak >= 0.5, "led peak never rose: {peak}");
    viz.levels = [0.0; VIZ_BINS];
    // Hold (0.45s) keeps the peak pinned for the first 10 frames...
    for _ in 0..10 {
      viz.advance(1.0 / 30.0);
    }
    let (_, held) = viz.led(0);
    assert!(held > 0.3, "led peak fell during hold: {held}");
  }

  #[test]
  fn mosaic_wires_grid_and_ignites() {
    let mut viz = VizState::new();
    viz.levels = [1.0; VIZ_BINS];
    viz.mosaic_tick(3, 5);
    for _ in 0..5 {
      viz.advance(1.0 / 30.0);
      viz.mosaic_tick(3, 5);
    }
    let mut lit = 0;
    for r in 0..3 {
      for t in 0..5 {
        if viz.mosaic_value(r, t) > 0.05 {
          lit += 1;
        }
      }
    }
    assert!(lit > 0, "no mosaic tile ignited on full-scale input");
    // Resize rebuilds the grid instead of indexing out of bounds.
    viz.mosaic_tick(2, 2);
    assert_eq!(viz.mosaic.len(), 4);
  }

  /// Live end-to-end proof (needs a real monitor source + an audible
  /// tone playing, e.g. `pw-play tone440.wav`): run with
  /// `cargo test -- --ignored live_monitor`. Hardware-dependent, so it
  /// never runs in CI — but it exercises the real capture path.
  #[test]
  #[ignore]
  fn live_monitor_reacts_to_tone() {
    let mut viz = VizState::new();
    let start = std::time::Instant::now();
    let mut hot = 0.0f32;
    while start.elapsed() < Duration::from_secs(4) {
      viz.poll();
      hot = hot.max(viz.levels[10]);
      std::thread::sleep(Duration::from_millis(100));
    }
    viz.stop();
    assert!(hot > 0.2, "bin 10 never went hot during 440Hz tone: {hot}");
  }
}
