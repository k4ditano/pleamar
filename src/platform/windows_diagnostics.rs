//! Counters used by the performance report, without child processes or windows.
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes, GetSystemTimes};

fn ticks(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}

pub fn process_cpu_ms() -> Option<f64> {
    let (mut creation, mut exit, mut kernel, mut user) = Default::default();
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user).ok()?; }
    Some((ticks(kernel) + ticks(user)) as f64 / 10_000.0)
}

pub fn rss_mb() -> Option<f64> {
    let mut counters = PROCESS_MEMORY_COUNTERS { cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size_of::<PROCESS_MEMORY_COUNTERS>() as u32).ok()?; }
    Some(counters.WorkingSetSize as f64 / 1024.0 / 1024.0)
}

/// Kernel time includes idle time; subtract it once to obtain busy time.
pub fn cpu_times() -> Option<(u64, u64)> {
    let (mut idle, mut kernel, mut user) = Default::default();
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()?; }
    let total = ticks(kernel) + ticks(user);
    Some((total.saturating_sub(ticks(idle)), total))
}

pub fn memory() -> Option<(f64, f64)> {
    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut status).ok()?; }
    let gb = 1024.0 * 1024.0 * 1024.0;
    Some((status.ullTotalPhys as f64 / gb, status.ullAvailPhys as f64 / gb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_counters_are_available_and_monotonic() {
        let before = process_cpu_ms().unwrap();
        let system_before = cpu_times().unwrap();
        let mut value = 1u64;
        for i in 0..100_000 { value = std::hint::black_box(value.wrapping_mul(31).wrapping_add(i)); }
        assert!(process_cpu_ms().unwrap() >= before);
        let system_after = cpu_times().unwrap();
        assert!(system_after.0 >= system_before.0 && system_after.1 >= system_before.1);
        assert!(system_after.0 <= system_after.1);
        assert!(rss_mb().unwrap() > 0.0);
        let (total, available) = memory().unwrap();
        assert!(total > 0.0 && available >= 0.0 && available <= total);
    }
}
