//! The render's clock. As a rule it is the system's (`Instant::now`); when a
//! scene is filmed (`--film`) it is a virtual one, which only moves when the
//! render says a frame is done, by exactly one frame: the same scene and the
//! same script give the same frames on any machine, however long each takes
//! to paint, and a minute of animation is made as fast as it can be painted.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Where the virtual clock started, if there is one.
static BASE: OnceLock<Instant> = OnceLock::new();
/// How far it has gone from there, in nanoseconds.
static GONE: AtomicU64 = AtomicU64::new(0);

/// From now on time is virtual, and starts here.
pub fn make_virtual() {
    let _ = BASE.set(Instant::now());
}

pub fn is_virtual() -> bool {
    BASE.get().is_some()
}

/// What time it is for the render.
pub fn now() -> Instant {
    match BASE.get() {
        Some(base) => *base + Duration::from_nanos(GONE.load(Ordering::Acquire)),
        None => Instant::now(),
    }
}

/// How long since then, on the render's clock (never negative: a moment of
/// the system's clock after the virtual one gives zero).
pub fn since(then: Instant) -> Duration {
    now().saturating_duration_since(then)
}

/// How long the virtual clock has run.
pub fn gone() -> Duration {
    Duration::from_nanos(GONE.load(Ordering::Acquire))
}

/// The virtual clock moves on by that much (nothing if it is the system's).
pub fn advance(by: Duration) {
    if is_virtual() {
        GONE.fetch_add(by.as_nanos() as u64, Ordering::AcqRel);
    }
}
