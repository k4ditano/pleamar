//! Runs the real runtime with counters for live Rust allocations, including
//! mlua's Rust-backed VM allocator. Native COM, direct C++/driver allocations
//! and allocator-reserved pages are not counted.
//! The normal pleamar executable does not use this allocator or sampling thread.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Tracked {
    bytes: AtomicUsize,
    blocks: AtomicUsize,
    peak: AtomicUsize,
}

impl Tracked {
    const fn new() -> Self {
        Self { bytes: AtomicUsize::new(0), blocks: AtomicUsize::new(0), peak: AtomicUsize::new(0) }
    }
    fn added(&self, size: usize) {
        let live = self.bytes.fetch_add(size, Relaxed).wrapping_add(size);
        self.peak.fetch_max(live, Relaxed);
    }
}

// Delegate every allocation and its original layout to System. Bookkeeping
// uses only atomics: allocating, locking or formatting here would recurse.
unsafe impl GlobalAlloc for Tracked {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() { self.added(layout.size()); self.blocks.fetch_add(1, Relaxed); }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() { self.added(layout.size()); self.blocks.fetch_add(1, Relaxed); }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.bytes.fetch_sub(layout.size(), Relaxed);
        self.blocks.fetch_sub(1, Relaxed);
        unsafe { System.dealloc(ptr, layout); }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(ptr, layout, size) };
        if !next.is_null() {
            if size >= layout.size() { self.added(size - layout.size()); }
            else { self.bytes.fetch_sub(layout.size() - size, Relaxed); }
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: Tracked = Tracked::new();

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|arg| arg == "--compile-check") {
        compile_check(&arguments[1..]);
        return;
    }
    std::thread::spawn(|| {
        #[cfg(target_os = "windows")]
        let inspect_native = std::env::var_os("PLEAMAR_PROFILE_NATIVE_HEAP").is_some();
        let start = std::time::Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let (bytes, blocks, peak) = (ALLOCATOR.bytes.load(Relaxed), ALLOCATOR.blocks.load(Relaxed), ALLOCATOR.peak.load(Relaxed));
            println!("allocation · {:.1} s · Rust live {} bytes in {} blocks · peak {} bytes", start.elapsed().as_secs_f64(), bytes, blocks, peak);
            #[cfg(target_os = "windows")]
            if inspect_native { native_heap::report(); }
        }
    });
    pleamar::run();
}

fn compile_check(arguments: &[String]) {
    let [path, count] = arguments else {
        eprintln!("usage: allocation-profile --compile-check SCENE ITERATIONS");
        std::process::exit(2);
    };
    let count: usize = count.parse().ok().filter(|n| (1..=1000).contains(n)).unwrap_or_else(|| {
        eprintln!("iterations must be between 1 and 1000");
        std::process::exit(2);
    });
    // Interned names intentionally live once per process. Warm those names and
    // lazy runtime state before measuring repeated compilation of identical input.
    let valid = pleamar::read_scene(path).is_ok();
    for _ in 0..2 { drop(pleamar::read_scene(path)); }
    let mut samples = Vec::with_capacity(count);
    let before = ALLOCATOR.bytes.load(Relaxed);
    for _ in 0..count {
        let result = pleamar::read_scene(path);
        assert_eq!(result.is_ok(), valid, "compilation changed without a source change");
        drop(result);
        samples.push(ALLOCATOR.bytes.load(Relaxed));
    }
    let after = ALLOCATOR.bytes.load(Relaxed);
    println!("{}", serde_json::json!({"valid": valid, "iterations": count,
        "before_bytes": before, "after_bytes": after, "samples": samples}));
    // This is requested live memory, not RSS, allocator caches or driver memory.
    // Allow a small fixed amount of lazy bookkeeping, never a per-reload budget.
    if after > before + 4096 { std::process::exit(1); }
}

#[cfg(target_os = "windows")]
mod native_heap {
    use windows::{core::{s, w, BOOL}, Win32::{Foundation::{GetLastError, HANDLE},
        System::{LibraryLoader::{GetModuleHandleW, GetProcAddress}, Memory::*}}};

    fn summary(heap: HANDLE) -> Result<HEAP_SUMMARY, u32> {
        type Read = unsafe extern "system" fn(HANDLE, u32, *mut HEAP_SUMMARY) -> BOOL;
        static READ: std::sync::OnceLock<Option<Read>> = std::sync::OnceLock::new();
        // This API starts at build 20348. Older Windows must still be able to
        // load the diagnostic and report that this extra counter is unavailable.
        let read = READ.get_or_init(|| unsafe {
            let module = GetModuleHandleW(w!("kernel32.dll")).ok()?;
            GetProcAddress(module, s!("HeapSummary")).map(|function| std::mem::transmute::<unsafe extern "system" fn() -> isize, Read>(function))
        }).ok_or(127u32)?;
        let mut result = HEAP_SUMMARY { cb: std::mem::size_of::<HEAP_SUMMARY>() as u32, ..Default::default() };
        if unsafe { read(heap, 0, &mut result) }.as_bool() { Ok(result) }
        else { Err(unsafe { GetLastError().0 }) }
    }

    pub fn report() {
        let started = std::time::Instant::now();
        let result = unsafe { GetProcessHeap() }.map_err(|e| e.code().0 as u32).and_then(summary);
        match result {
            Ok(value) => {
                // The default heap cannot disappear while the process runs.
                // Other heaps may be destroyed concurrently: count them without
                // retaining handles or walking an unowned heap.
                let heaps = unsafe { GetProcessHeaps(&mut []) };
                println!("allocation · default heap: {} allocated · {} committed · {} reserved bytes · {} process heaps · query {:.3} ms",
                    value.cbAllocated, value.cbCommitted, value.cbReserved, heaps, started.elapsed().as_secs_f64() * 1000.0);
            }
            Err(code) => println!("allocation · default heap unavailable: {code}"),
        }
    }

    #[cfg(test)]
    #[test]
    fn native_summary_observes_owned_allocation_and_free() {
        struct Owned(HANDLE);
        impl Drop for Owned { fn drop(&mut self) { unsafe { let _ = HeapDestroy(self.0); } } }
        unsafe {
            let heap = Owned(HeapCreate(HEAP_NONE, 0, 0).unwrap());
            let before = match summary(heap.0) {
                Err(127) => { eprintln!("NOT RUN: HeapSummary is unavailable on this Windows version"); return; }
                value => value.unwrap(),
            };
            let ptr = HeapAlloc(heap.0, HEAP_ZERO_MEMORY, 1 << 20);
            assert!(!ptr.is_null());
            let during = summary(heap.0).unwrap();
            assert!(during.cbAllocated >= before.cbAllocated + (1 << 20));
            HeapFree(heap.0, HEAP_NONE, Some(ptr)).unwrap();
            assert_eq!(summary(heap.0).unwrap().cbAllocated, before.cbAllocated);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_live_blocks_across_growth_shrink_and_free() {
        let tracked = Tracked::new();
        unsafe {
            let small = Layout::from_size_align(64, 16).unwrap();
            let large = Layout::from_size_align(512, 16).unwrap();
            let ptr = tracked.alloc_zeroed(small);
            assert!(!ptr.is_null());
            assert!(std::slice::from_raw_parts(ptr, 64).iter().all(|b| *b == 0));
            ptr.write(37);
            let ptr = tracked.realloc(ptr, small, 512);
            assert!(!ptr.is_null());
            assert_eq!(ptr.read(), 37);
            assert_eq!(tracked.bytes.load(Relaxed), 512);
            assert_eq!(tracked.blocks.load(Relaxed), 1);
            let ptr = tracked.realloc(ptr, large, 64);
            assert!(!ptr.is_null());
            assert_eq!(ptr.read(), 37);
            assert_eq!(tracked.bytes.load(Relaxed), 64);
            assert_eq!(tracked.blocks.load(Relaxed), 1);
            tracked.dealloc(ptr, small);
            let ptr = tracked.alloc(small);
            assert!(!ptr.is_null());
            tracked.dealloc(ptr, small);
            assert_eq!(tracked.bytes.load(Relaxed), 0);
            assert_eq!(tracked.blocks.load(Relaxed), 0);
            assert_eq!(tracked.peak.load(Relaxed), 512);
        }
    }
}
