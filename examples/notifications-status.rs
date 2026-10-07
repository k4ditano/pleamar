//! Report capability/consent only; never print notification content or request access.
#[cfg(not(target_os = "windows"))]
fn main() { eprintln!("notifications-status requires Windows"); std::process::exit(1); }
#[cfg(target_os = "windows")]
fn main() {
    use windows::{ApplicationModel::Package, UI::Notifications::Management::UserNotificationListener,
        Win32::System::WinRT::*};
    unsafe { RoInitialize(RO_INIT_MULTITHREADED).unwrap(); }
    println!("identity: {:?}", Package::Current().and_then(|p| p.Id()?.Name()));
    println!("access: {:?}", UserNotificationListener::Current().and_then(|l| l.GetAccessStatus()));
    unsafe { RoUninitialize(); }
}
