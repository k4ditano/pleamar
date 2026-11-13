//! What a compiled scene holds in Rust's heap, by size of allocation (pleamar#153):
//! `scene-memory SCENE`. Says the bytes live after reading it, the sizes that hold most
//! of them, and how big the scene's types are. Only Rust's heap is counted.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering::Relaxed};

const CLASSES: usize = 4097;
struct Histogram {
    live: AtomicUsize,
    /// Live bytes and blocks by size (the last class: 4096 bytes and more).
    bytes: [AtomicIsize; CLASSES],
    blocks: [AtomicIsize; CLASSES],
}

impl Histogram {
    fn class(size: usize) -> usize {
        size.min(CLASSES - 1)
    }
    fn add(&self, size: usize) {
        self.live.fetch_add(size, Relaxed);
        self.bytes[Self::class(size)].fetch_add(size as isize, Relaxed);
        self.blocks[Self::class(size)].fetch_add(1, Relaxed);
    }
    fn sub(&self, size: usize) {
        self.live.fetch_sub(size, Relaxed);
        self.bytes[Self::class(size)].fetch_sub(size as isize, Relaxed);
        self.blocks[Self::class(size)].fetch_sub(1, Relaxed);
    }
}

unsafe impl GlobalAlloc for Histogram {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            self.add(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            self.add(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.sub(layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, size) };
        if !p.is_null() {
            self.sub(layout.size());
            self.add(size);
        }
        p
    }
}

#[global_allocator]
static HEAP: Histogram = Histogram { live: AtomicUsize::new(0), bytes: [const { AtomicIsize::new(0) }; CLASSES], blocks: [const { AtomicIsize::new(0) }; CLASSES] };

fn main() {
    let path = std::env::args().nth(1).expect("scene-memory SCENE");
    let snapshot = || (0..CLASSES).map(|k| (HEAP.bytes[k].load(Relaxed), HEAP.blocks[k].load(Relaxed))).collect::<Vec<_>>();
    let before = HEAP.live.load(Relaxed);
    let at_start = snapshot();
    let scene = pleamar::read_scene(&path).unwrap_or_else(|e| panic!("{e}"));
    let after = HEAP.live.load(Relaxed);
    let now = snapshot();
    let mb = |b: f64| b / (1024.0 * 1024.0);
    let s = pleamar::scene_counts(&scene);
    println!("{path}: {} properties, {} instructions, {} rules, {} zones, {} facts, {} texts", s.0, s.1, s.2, s.3, s.4, s.5);
    println!("live after reading it: {:.1} MB ({:.0} bytes a property, {:.0} an instruction)", mb((after - before) as f64), (after - before) as f64 / s.0.max(1) as f64, (after - before) as f64 / s.1.max(1) as f64);
    let mut by: Vec<(usize, isize, isize)> = (0..CLASSES).map(|k| (k, now[k].0 - at_start[k].0, now[k].1 - at_start[k].1)).filter(|c| c.1 > 0).collect();
    by.sort_by_key(|c| -c.1);
    println!("the sizes holding most (size: blocks, MB):");
    for (k, bytes, blocks) in by.iter().take(18) {
        println!("  {}{k} bytes: {blocks} blocks, {:.2} MB", if *k == CLASSES - 1 { "≥" } else { "" }, mb(*bytes as f64));
    }
    for (name, size) in pleamar::scene_type_sizes() {
        println!("size_of {name}: {size}");
    }
    drop(scene);
}
