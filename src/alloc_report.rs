//! What a scene holds in memory, by part (`PLEAMAR_MEMORY=1`, pleamar#153).
//!
//! With the `alloc-report` feature (off by default: it counts every allocation),
//! the bytes live in Rust's heap are counted, and said at a few moments: the scene
//! read, the first frame, and the scene at rest a few seconds later. Without the
//! feature only the render's own parts are said, by their sizes. What the card's
//! driver, fontconfig or the Luau VM's C side take is not in either.

#[cfg(feature = "alloc-report")]
mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    pub struct Counting(pub AtomicUsize);

    // Only atomics here: allocating, locking or printing would recurse.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let p = unsafe { System.alloc(layout) };
            if !p.is_null() {
                self.0.fetch_add(layout.size(), Relaxed);
            }
            p
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let p = unsafe { System.alloc_zeroed(layout) };
            if !p.is_null() {
                self.0.fetch_add(layout.size(), Relaxed);
            }
            p
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            self.0.fetch_sub(layout.size(), Relaxed);
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let p = unsafe { System.realloc(ptr, layout, size) };
            if !p.is_null() {
                if size >= layout.size() {
                    self.0.fetch_add(size - layout.size(), Relaxed);
                } else {
                    self.0.fetch_sub(layout.size() - size, Relaxed);
                }
            }
            p
        }
    }

    #[global_allocator]
    pub static LIVE: Counting = Counting(AtomicUsize::new(0));
}

/// Whether it is asked for.
pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PLEAMAR_MEMORY").is_some_and(|v| !v.is_empty() && v != "0"))
}

/// Bytes live in Rust's heap now (with the feature; none without it).
pub fn live() -> Option<usize> {
    #[cfg(feature = "alloc-report")]
    return Some(counting::LIVE.0.load(std::sync::atomic::Ordering::Relaxed));
    #[cfg(not(feature = "alloc-report"))]
    None
}

/// «memory · WHEN: N MB live in the heap» (with the feature), and the parts given.
pub fn say(when: &str, parts: &[(&str, usize)]) {
    if !on() {
        return;
    }
    let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
    let live = live().map_or(String::new(), |b| format!(": {:.1} MB live in the heap", mb(b)));
    println!("memory · {when}{live}");
    let total: usize = parts.iter().map(|p| p.1).sum();
    for (name, bytes) in parts.iter().filter(|p| p.1 > 0) {
        println!("memory ·   {name}: {:.2} MB", mb(*bytes));
    }
    if !parts.is_empty() {
        println!("memory ·   (those: {:.1} MB)", mb(total));
    }
}

/// The heap a vector holds (its capacity, not what is in it).
pub fn vec<T>(v: &[T], capacity: usize) -> usize {
    let _ = v;
    capacity * std::mem::size_of::<T>()
}
