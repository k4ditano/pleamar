//! `pleamar --report`: when someone says «it stutters on my machine», what we
//! need from it, in one file they can send us.
//!
//! Every scene running is asked to measure its frames (`probe`, see
//! `probe.rs`) while the machine is watched from outside: how busy the CPU
//! and the card are, how hot, how fast the cores go, and which processes
//! took the CPU meanwhile. Then it is all written down in Markdown, with a
//! verdict per scene and what might explain it. Nothing personal goes in it:
//! no window titles, no files, no user or host names.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub fn run(args: Vec<String>, extra: Option<String>) -> i32 {
    let mut seconds = 30u64;
    let mut out: Option<String> = None;
    let mut named = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seconds" => seconds = it.next().and_then(|s| s.parse().ok()).unwrap_or(30).clamp(5, 300),
            "--out" => out = it.next(),
            s if s.starts_with("--") => {
                eprintln!("report · I don't know '{s}': --seconds N, --out FILE");
                return 2;
            }
            _ => named.push(a),
        }
    }
    let scenes = if named.is_empty() { crate::platform::running_scenes() } else { named };
    if scenes.is_empty() {
        eprintln!("report · there is no scene running here to measure (is this terminal inside the session?)");
        return 1;
    }

    // Measuring, all at once.
    let mut measuring = Vec::new();
    let mut old = Vec::new();
    for s in &scenes {
        match crate::platform::ask(s, "probe start", Duration::from_secs(2)) {
            Ok(a) if a.starts_with("measuring") => measuring.push(s.clone()),
            Ok(_) => old.push(s.clone()),
            Err(e) => eprintln!("report · {s}: {e}"),
        }
    }
    if measuring.is_empty() {
        eprintln!("report · no scene could measure: they run a pleamar older than this one ({}). Start them again after updating", old.join(", "));
        return 1;
    }
    eprintln!("report · measuring {} for {seconds} s. Use the desktop as usual meanwhile —", measuring.join(", "));
    eprintln!("         move windows, switch workspaces, open and close things— above all what feels slow.");

    let cpu0 = cpu_times();
    let procs0 = process_times();
    let mut watch = Watch::default();
    let start = Instant::now();
    let until = start + Duration::from_secs(seconds);
    let mut said = u64::MAX;
    while Instant::now() < until {
        watch.sample();
        let left = until.saturating_duration_since(Instant::now()).as_secs();
        if left != said {
            said = left;
            eprint!("\r         {left:>3} s left ");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    eprintln!("\r         done.          ");
    let wall = start.elapsed().as_secs_f64();
    let cpu1 = cpu_times();
    let procs1 = process_times();

    let mut answers = Vec::new();
    for s in &measuring {
        let a = crate::platform::ask(s, "probe report", Duration::from_secs(4)).unwrap_or_else(|e| format!("? {e}"));
        answers.push((s.clone(), a));
    }

    // ── what is written ──
    let mut md = String::new();
    let mut hints: Vec<String> = Vec::new();
    md.push_str(&format!("# pleamar report · {}\n\n", stamp("+%Y-%m-%d %H:%M")));
    md.push_str(&format!("pleamar {} · measured {seconds} s · {}\n\n", env!("CARGO_PKG_VERSION"), measuring.join(", ")));

    md.push_str("## Verdict\n\n");
    let mut verdicts = Vec::new();
    for (s, a) in &answers {
        let first = a.lines().next().unwrap_or("?").to_owned();
        md.push_str(&format!("- **{s}:** {first}\n"));
        verdicts.push(format!("{s}: {first}"));
        if a.contains("SOFTWARE") {
            hints.push(format!("{s} is painted by the CPU (llvmpipe/lavapipe): the GPU's Vulkan driver is missing or failed to load (vulkan-radeon / vulkan-intel / nvidia-utils)"));
        }
    }
    for s in &old {
        md.push_str(&format!("- **{s}:** not measured: it runs an older pleamar (start it again after updating)\n"));
    }

    // The machine.
    let mut machine = Vec::new();
    let mut line = |k: &str, v: String| machine.push(format!("- **{k}:** {v}"));
    line("System", os_release().unwrap_or_else(|| "?".into()));
    if let Some(kernel) = read("/proc/sys/kernel/osrelease") { line("Kernel", kernel); }
    let threads = std::thread::available_parallelism().map_or(0, |n| n.get());
    line("CPU", format!("{} · {threads} threads", cpu_model().unwrap_or_else(|| "?".into())));
    let governor = read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor");
    let driver = read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver");
    let epp = read("/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference");
    let profile = read("/sys/firmware/acpi/platform_profile");
    if governor.is_some() || driver.is_some() { line(
        "CPU frequency policy",
        format!(
            "{} governor · {} driver{}{}",
            governor.as_deref().unwrap_or("?"),
            driver.as_deref().unwrap_or("?"),
            epp.as_ref().map_or(String::new(), |e| format!(" · EPP {e}")),
            profile.as_ref().map_or(String::new(), |p| format!(" · platform profile {p}"))
        ),
    ); }
    if epp.as_deref() == Some("power") || matches!(profile.as_deref(), Some("low-power" | "quiet" | "cool")) {
        hints.push("the CPU is set to save power (EPP «power» or a low-power platform profile): try the balanced or performance profile".into());
    } else if governor.as_deref() == Some("powersave") && !driver.as_deref().is_some_and(|d| d.ends_with("-epp") || d == "intel_pstate") {
        hints.push("the CPU governor is «powersave» with a driver that then keeps the cores slow: try «schedutil» or «performance»".into());
    }
    #[cfg(not(target_os = "windows"))]
    if let Some(m) = meminfo() {
        line("Memory", format!("{:.1} GB · {:.1} GB available · swap in use {:.1} GB", m.0, m.1, m.2));
        if m.1 < m.0 * 0.08 {
            hints.push("memory was nearly full: the system may have been swapping".into());
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some((total, available)) = crate::platform::windows_diagnostics::memory() {
            line("Memory", format!("{total:.1} GB · {available:.1} GB available"));
            if available < total * 0.08 { hints.push("memory was nearly full".into()); }
        }
        line("Session", "Windows desktop · Win32 / DirectComposition".into());
        line("Unavailable counters", "system process ranking, CPU/GPU temperatures, core clocks and system GPU load; GPU and monitor details are reported by each scene".into());
    }
    #[cfg(not(target_os = "windows"))]
    line(
        "Session",
        format!(
            "{} · {}",
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "?".into()),
            std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "?".into())
        ),
    );
    // Their values only if they are not paths: those carry the user's name.
    let vars: Vec<String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("PLEAMAR_") && k != "PLEAMAR_SOCKETS" && k != "PLEAMAR_SOCKET_DIR")
        .map(|(k, v)| report_variable(&k, &v))
        .collect();
    if !vars.is_empty() {
        line("pleamar variables", vars.join(" "));
    }
    for g in gpus() {
        line("GPU", g);
    }

    // What happened while measuring.
    let mut during = Vec::new();
    if let (Some(a), Some(b)) = (cpu0, cpu1) {
        let (busy, total) = (b.0.saturating_sub(a.0) as f64, b.1.saturating_sub(a.1) as f64);
        if total > 0.0 {
            let pct = busy / total * 100.0;
            during.push(format!("- **CPU busy (whole system):** {pct:.0}%"));
            if pct > 85.0 {
                hints.push(format!("the CPU was {pct:.0}% busy while measuring: something else was taking it (see the processes)"));
            }
        }
    }
    if let Some(l) = read("/proc/loadavg") {
        during.push(format!("- **Load average:** {}", l.split(' ').take(3).collect::<Vec<_>>().join(" ")));
    }
    if !watch.mhz.is_empty() {
        let (min, max) = watch.mhz.iter().fold((f64::MAX, 0f64), |a, v| (a.0.min(*v), a.1.max(*v)));
        during.push(format!("- **Core clock (mean of the cores):** {:.0} MHz on average · {min:.0}–{max:.0}", mean(&watch.mhz)));
    }
    if !watch.cpu_temp.is_empty() {
        let max = watch.cpu_temp.iter().cloned().fold(0.0, f64::max);
        during.push(format!("- **CPU temperature:** {:.0} °C on average · {max:.0} at most", mean(&watch.cpu_temp)));
        if max >= 90.0 {
            hints.push(format!("the CPU reached {max:.0} °C: it may be slowing itself down to cool (thermal throttling)"));
        }
    }
    for (card, v) in &watch.gpu_busy {
        let max = v.iter().cloned().fold(0.0, f64::max);
        during.push(format!("- **GPU busy ({card}):** {:.0}% on average · {max:.0}% at most", mean(v)));
        if mean(v) > 90.0 {
            hints.push(format!("the GPU ({card}) was {:.0}% busy on average: it is the limit", mean(v)));
        }
    }
    for (card, v) in &watch.gpu_temp {
        during.push(format!("- **GPU temperature ({card}):** {:.0} °C on average · {:.0} at most", mean(v), v.iter().cloned().fold(0.0, f64::max)));
    }
    // Who took the CPU: the five that took the most.
    let mut took: Vec<(String, f64)> = procs1
        .iter()
        .filter_map(|(pid, (name, t))| {
            let before = procs0.get(pid).map_or(0, |p| p.1);
            (t >= &before).then(|| (name.clone(), (t - before) as f64 / ticks_per_second() / wall * 100.0))
        })
        .collect();
    // By program, not by process: a browser is forty of them.
    let mut by_name: HashMap<String, f64> = HashMap::new();
    for (n, pct) in took.drain(..) {
        *by_name.entry(n).or_default() += pct;
    }
    let mut took: Vec<(String, f64)> = by_name.into_iter().filter(|(_, p)| *p >= 1.0).collect();
    took.sort_by(|a, b| b.1.total_cmp(&a.1));
    if !took.is_empty() {
        during.push(format!(
            "- **Most CPU (% of a core):** {}",
            took.iter().take(6).map(|(n, p)| format!("{n} {p:.0}%")).collect::<Vec<_>>().join(" · ")
        ));
    }

    if !hints.is_empty() {
        md.push_str("\n## What might explain it\n\n");
        for h in &hints {
            md.push_str(&format!("- {h}\n"));
        }
    }
    md.push_str("\n## The machine\n\n");
    md.push_str(&machine.join("\n"));
    md.push_str("\n\n## While measuring\n\n");
    md.push_str(&during.join("\n"));
    md.push('\n');
    if let Some(e) = &extra {
        md.push('\n');
        md.push_str(e.trim_end());
        md.push('\n');
    }
    for (s, a) in &answers {
        md.push_str(&format!("\n## {s}\n\n"));
        md.push_str(a.split_once('\n').map_or("", |x| x.1).trim_end());
        md.push('\n');
    }

    let path = out.unwrap_or_else(|| {
        let home_var = if cfg!(target_os = "windows") { "USERPROFILE" } else { "HOME" };
        let home = std::env::var(home_var).unwrap_or_else(|_| ".".into());
        format!("{home}/pleamar-report-{}.md", stamp("+%Y-%m-%d-%H%M"))
    });
    if let Err(e) = std::fs::write(&path, &md) {
        eprintln!("report · {path}: {e}");
        print!("{md}");
        return 1;
    }
    println!();
    for v in &verdicts {
        println!("  {v}");
    }
    for h in &hints {
        println!("  · {h}");
    }
    println!("\nreport · written to {path}");
    println!("         Send it to us: attach it to an issue at https://github.com/k4ditano/pleamar/issues");
    println!("         or drop it in the Discord. It holds no personal data: numbers, the card and the monitors.");
    0
}

/// What is watched from outside while the scenes measure, four times a second.
#[derive(Default)]
struct Watch {
    mhz: Vec<f64>,
    cpu_temp: Vec<f64>,
    gpu_busy: Vec<(String, Vec<f64>)>,
    gpu_temp: Vec<(String, Vec<f64>)>,
}

impl Watch {
    #[cfg(target_os = "windows")]
    fn sample(&mut self) {}

    #[cfg(not(target_os = "windows"))]
    fn sample(&mut self) {
        // The cores' clock, averaged.
        let mut sum = 0.0;
        let mut n = 0.0;
        for k in 0..1024 {
            let Some(v) = read(&format!("/sys/devices/system/cpu/cpu{k}/cpufreq/scaling_cur_freq")).and_then(|s| s.parse::<f64>().ok()) else { break };
            sum += v / 1000.0;
            n += 1.0;
        }
        if n > 0.0 {
            self.mhz.push(sum / n);
        }
        // Temperatures, from hwmon: the CPU's (k10temp, coretemp, zenpower) and each card's.
        let Ok(dir) = std::fs::read_dir("/sys/class/hwmon") else { return };
        for h in dir.filter_map(Result::ok).map(|e| e.path()) {
            let name = read(&h.join("name").to_string_lossy()).unwrap_or_default();
            let temp = read(&h.join("temp1_input").to_string_lossy()).and_then(|t| t.parse::<f64>().ok()).map(|t| t / 1000.0);
            match name.as_str() {
                "k10temp" | "coretemp" | "zenpower" | "cpu_thermal" => {
                    if let Some(t) = temp {
                        self.cpu_temp.push(t);
                    }
                }
                "amdgpu" | "nouveau" | "i915" | "xe" => {
                    // The card this hwmon belongs to, by its PCI address.
                    let card = std::fs::canonicalize(h.join("device")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or(name.clone());
                    if let Some(t) = temp {
                        push(&mut self.gpu_temp, &card, t);
                    }
                    if let Some(b) = read(&h.join("device/gpu_busy_percent").to_string_lossy()).and_then(|b| b.parse::<f64>().ok()) {
                        push(&mut self.gpu_busy, &card, b);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn push(list: &mut Vec<(String, Vec<f64>)>, key: &str, v: f64) {
    match list.iter_mut().find(|(k, _)| k == key) {
        Some((_, l)) => l.push(v),
        None => list.push((key.to_owned(), vec![v])),
    }
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

#[cfg(not(target_os = "windows"))]
fn os_release() -> Option<String> {
    let s = std::fs::read_to_string("/etc/os-release").ok()?;
    s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_owned())
}

#[cfg(target_os = "windows")]
fn os_release() -> Option<String> { Some("Windows".into()) }

#[cfg(target_os = "windows")]
fn cpu_model() -> Option<String> { std::env::var("PROCESSOR_IDENTIFIER").ok() }

#[cfg(not(target_os = "windows"))]
fn cpu_model() -> Option<String> {
    let s = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    s.lines().find_map(|l| l.strip_prefix("model name").map(|r| r.trim_start_matches([' ', '\t', ':']).to_owned()))
}

/// Total, available and swap in use, in GB.
#[cfg(not(target_os = "windows"))]
fn meminfo() -> Option<(f64, f64, f64)> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = |k: &str| s.lines().find_map(|l| l.strip_prefix(k)).and_then(|r| r.trim().trim_end_matches("kB").trim().parse::<f64>().ok()).map(|v| v / 1024.0 / 1024.0);
    Some((kb("MemTotal:")?, kb("MemAvailable:")?, kb("SwapTotal:").unwrap_or(0.0) - kb("SwapFree:").unwrap_or(0.0)))
}

/// Each card the kernel sees: its driver and its PCI ids (enough to know
/// which card it is without asking for anything else).
fn gpus() -> Vec<String> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return out };
    let mut cards: Vec<_> = dir.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("card") && !n.to_string_lossy().contains('-'))).collect();
    cards.sort();
    for c in cards {
        let dev = c.join("device");
        let driver = std::fs::read_link(dev.join("driver")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "?".into());
        let vendor = read(&dev.join("vendor").to_string_lossy()).unwrap_or_default();
        let device = read(&dev.join("device").to_string_lossy()).unwrap_or_default();
        let who = match vendor.as_str() {
            "0x1002" => "AMD",
            "0x10de" => "NVIDIA",
            "0x8086" => "Intel",
            _ => "",
        };
        let pci = std::fs::canonicalize(&dev).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        out.push(format!("{} · {who} {vendor}:{device} · driver {driver} · {pci}", c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    }
    if let Some(v) = read("/sys/module/nvidia/version") {
        out.push(format!("NVIDIA driver {v}"));
    }
    out
}

/// (busy, total) jiffies of the whole system.
#[cfg(not(target_os = "windows"))]
fn cpu_times() -> Option<(u64, u64)> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    let f: Vec<u64> = s.lines().next()?.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
    let total: u64 = f.iter().take(8).sum();
    let idle = f.get(3)? + f.get(4).unwrap_or(&0);
    Some((total - idle, total))
}

#[cfg(target_os = "windows")]
use crate::platform::windows_diagnostics::cpu_times;

/// Every process's name and the CPU it has used, in ticks.
fn process_times() -> HashMap<u32, (String, u64)> {
    let mut out = HashMap::new();
    let Ok(dir) = std::fs::read_dir("/proc") else { return out };
    for e in dir.filter_map(Result::ok) {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
        let (Some(a), Some(b)) = (stat.find('('), stat.rfind(')')) else { continue };
        let name = stat[a + 1..b].to_owned();
        let f: Vec<&str> = stat[b + 2..].split(' ').collect();
        let t = f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        out.insert(pid, (name, t));
    }
    out
}

fn ticks_per_second() -> f64 {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: asks a constant of the system.
        (unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64).max(1.0)
    }
    #[cfg(not(target_os = "linux"))]
    100.0
}

/// The same local timestamp on both systems, without an external utility.
fn stamp(format: &str) -> String {
    chrono::Local::now().format(format.trim_start_matches('+')).to_string()
}

fn report_variable(key: &str, value: &str) -> String {
    if value.contains(['/', '\\']) { format!("{key} (a path)") } else { format!("{key}={value}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_paths_are_redacted_on_both_systems() {
        assert_eq!(report_variable("PLEAMAR_CONFIG", r"C:\Users\Someone\config"), "PLEAMAR_CONFIG (a path)");
        assert_eq!(report_variable("PLEAMAR_CONFIG", "/home/someone/config"), "PLEAMAR_CONFIG (a path)");
        assert_eq!(report_variable("PLEAMAR_BACKEND", "dx12"), "PLEAMAR_BACKEND=dx12");
        assert!(chrono::NaiveDateTime::parse_from_str(&stamp("+%Y-%m-%d %H:%M"), "%Y-%m-%d %H:%M").is_ok());
    }
}
