use pleamar::{PlatformWindow, scene::{Cursor, Keyboard}};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

// A backend written against the original public interface knows only
// request_frame. Adding a capability must not silently disable that method.
struct ExistingWindow(Arc<AtomicBool>);
impl PlatformWindow for ExistingWindow {
    fn update_input_region(&self, _: &[[i32; 4]]) {}
    fn cursor(&self, _: Cursor) {}
    fn keyboard(&self, _: Keyboard) {}
    fn request_frame(&self) { self.0.store(true, Ordering::Relaxed); }
}

#[test]
fn existing_backends_keep_their_frame_callback_contract() {
    let requested = Arc::new(AtomicBool::new(false));
    let window: Box<dyn PlatformWindow> = Box::new(ExistingWindow(requested.clone()));
    if window.has_frame_callbacks() { window.request_frame(); }
    assert!(requested.load(Ordering::Relaxed), "an existing backend lost compositor pacing");
}
