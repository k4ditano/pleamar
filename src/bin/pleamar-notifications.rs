#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "windows")]
    if pleamar::run_notification_broker().is_err() {std::process::exit(1);}
    #[cfg(not(target_os = "windows"))]
    {eprintln!("The notification activation broker is Windows-only.");std::process::exit(1);}
}
