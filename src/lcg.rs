use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

thread_local! {
  static STATE: Cell<u64> = Cell::new(
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_nanos() as u64)
      .unwrap_or(0x9E37_79B9_7F4A_7C15),
  );
}

pub fn rand_idx(max: usize) -> usize {
  if max <= 1 {
    return 0;
  }
  let mut x = STATE.with(|c| c.get());
  x ^= x >> 12;
  x ^= x << 25;
  x ^= x >> 27;
  STATE.with(|c| c.set(x));
  (x.wrapping_mul(0x2545_F491_4F6C_DD1D) as usize) % max
}

#[cfg(test)]
mod tests {
  use super::rand_idx;

  #[test]
  fn stays_in_range_and_varies() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..500 {
      let v = rand_idx(7);
      assert!(v < 7);
      assert_eq!(rand_idx(1), 0);
      seen.insert(v);
    }
    assert!(seen.len() > 1);
  }
}