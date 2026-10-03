//! `pleamar --report`: a while of frames, measured one by one, for whoever
//! says «it stutters» to send us what it does on their machine.
//!
//! The render keeps nothing of this unless it is asked: `probe start` (over
//! the scene's socket) begins, and `probe report` answers with what it saw
//! and stops. Each frame keeps how long it took against how long the screen
//! gives, and where the round before it went —the frame that came late is
//! the one whose round took too long—. No window titles, no texts of the
//! scene: numbers, the card and the monitors.

use std::time::Instant;

/// The sections of a round, as the render's `sec` has them, and the wait for
/// its turn to paint at the end.
pub const SECTIONS: usize = 9;
const NAMES: [&str; SECTIONS] = ["input", "rules and springs", "rules and springs", "compose", "regions", "paint", "rest", "windows' frames", "waiting for the screen"];
/// What each one means, said to whoever reads the report.
const MEANING: [&str; SECTIONS] = [
    "reading input and messages",
    "the scene's rules and springs",
    "the scene's rules and springs",
    "composing the scene (working out what is drawn)",
    "working out what changed on screen",
    "painting: recording and sending the frame to the card",
    "the rest of the round",
    "copying the programs' windows to the card",
    "waiting for the screen or the card: the GPU is busy, or its driver is slow to hand back frames",
];

struct Frame {
    /// From the round before to this one: what the eye sees.
    ms: f32,
    /// What the screen gives, then.
    period: f32,
    /// Where the round that made this frame went.
    sections: [f32; SECTIONS],
    /// Its logic was busy and the render did not wait for it.
    blocked: bool,
}

pub struct Probe {
    since: Instant,
    cpu_since: Option<f64>,
    frames: Vec<Frame>,
    previous: Option<[f32; SECTIONS]>,
    pub window_frames: u32,
}

/// What the render knows about itself, for the report.
pub struct Context {
    pub card: String,
    pub card_memory: Option<f64>,
    pub monitors: Vec<String>,
    pub windows: usize,
    pub scene: String,
    /// The scene's own `rate:` (0: none, the monitor's).
    pub rate: u32,
}

impl Probe {
    pub fn new() -> Probe {
        Probe { since: Instant::now(), cpu_since: process_cpu_ms(), frames: Vec::new(), previous: None, window_frames: 0 }
    }

    /// A round has ended. `ms` is the frame the eye saw (see `Frame`); it
    /// counts only if the render was animating before it, not if it just woke
    /// up from resting —that wait is no stutter—.
    pub fn round(&mut self, counts: bool, ms: f32, period: f32, sections: [f32; SECTIONS], blocked: bool) {
        if let (true, Some(before)) = (counts, self.previous) {
            // At most five minutes of them: whoever forgets to ask for the report does not fill the memory.
            if self.frames.len() < 5 * 60 * 360 {
                self.frames.push(Frame { ms, period, sections: before, blocked });
            }
        }
        self.previous = Some(sections);
    }

    /// Resting, the round that comes after is not the continuation of this one.
    pub fn rest(&mut self) {
        self.previous = None;
    }

    /// What it saw. The first line is the verdict, for the summary; the rest,
    /// the details, in Markdown.
    pub fn report(&self, c: &Context) -> String {
        let seconds = self.since.elapsed().as_secs_f64().max(0.001);
        let mut out = String::new();
        let n = self.frames.len();
        let period = if n == 0 { 16.7 } else { median(self.frames.iter().map(|f| f.period).collect()) };
        let mut ms: Vec<f32> = self.frames.iter().map(|f| f.ms).collect();
        ms.sort_by(f32::total_cmp);
        let at = |q: f64| if ms.is_empty() { 0.0 } else { ms[((ms.len() - 1) as f64 * q).round() as usize] };
        // Going steadily slower than the screen is not stuttering: something
        // paces it (the scene's `rate:`, or the compositor's frame notices).
        // Then late is late against that pace, not the screen's.
        let steady = n >= 30 && at(0.5) > period * 1.4 && at(0.95) < at(0.5) * 1.2;
        let pace = |f: &Frame| if steady { at(0.5) } else { f.period };
        let late: Vec<&Frame> = self.frames.iter().filter(|f| f.ms > pace(f) * 1.5).collect();
        let dropped = self.frames.iter().filter(|f| f.ms > pace(f) * 2.5).count();
        let late_pct = if n == 0 { 0.0 } else { late.len() as f64 * 100.0 / n as f64 };
        let animating: f64 = self.frames.iter().map(|f| f.ms as f64).sum::<f64>() / 1000.0;

        // Where the late ones went: the section that took most, on average.
        let mut cause = String::new();
        if !late.is_empty() {
            let mut sum = [0f64; SECTIONS];
            for f in &late {
                for (s, v) in sum.iter_mut().zip(f.sections) {
                    *s += v as f64;
                }
            }
            sum[1] += sum[2];
            sum[2] = 0.0;
            let mean_ms = late.iter().map(|f| f.ms as f64).sum::<f64>() / late.len() as f64;
            let worked: f64 = sum.iter().sum::<f64>() / late.len() as f64;
            let (k, most) = sum.iter().enumerate().fold((0, 0.0), |a, (k, v)| if *v > a.1 { (k, *v) } else { a });
            cause = if worked < mean_ms * 0.4 {
                format!("mostly outside its own work ({worked:.1} of {mean_ms:.1} ms accounted for): the process was not given the CPU in time, or something outside the loop held it")
            } else {
                format!("mostly {} ({:.1} ms of {mean_ms:.1} on average)", MEANING[k], most / late.len() as f64)
            };
        }

        let verdict = if n < 30 {
            "barely animated while measuring: nothing to judge (move windows, open things, while it measures)".to_owned()
        } else if steady && late_pct < 1.0 && c.rate > 0 && (at(0.5) - 1000.0 / c.rate as f32).abs() < 1.5 {
            format!("smooth · {:.0} fps, the scene's own `rate: {}` (the screen gives {:.0} Hz)", 1000.0 / at(0.5).max(0.1), c.rate, 1000.0 / period)
        } else if steady && late_pct < 1.0 {
            format!("steady at {:.0} fps, below the screen's {:.0} Hz: something paces it slower (the scene's `rate:`, or the compositor's frame notices)", 1000.0 / at(0.5).max(0.1), 1000.0 / period)
        } else if late_pct < 1.0 && at(0.99) < pace(&self.frames[0]) * 1.6 {
            format!("smooth · {:.0} fps against {:.0} Hz", 1000.0 / at(0.5).max(0.1), 1000.0 / period)
        } else if late_pct < 5.0 {
            format!("some stutter · {late_pct:.1}% of the frames late, the worst {:.0} ms · {cause}", at(1.0))
        } else {
            format!("struggling · {late_pct:.1}% of the frames late, the worst {:.0} ms · {cause}", at(1.0))
        };
        out.push_str(&verdict);
        out.push('\n');

        let line = |out: &mut String, k: &str, v: String| out.push_str(&format!("- **{k}:** {v}\n"));
        line(&mut out, "Scene", c.scene.clone());
        line(&mut out, "Card", c.card.clone());
        if let Some(m) = c.card_memory {
            line(&mut out, "Card memory in use", format!("{m:.0} MB"));
        }
        for m in &c.monitors {
            line(&mut out, "Surface", m.clone());
        }
        if c.windows > 0 || self.window_frames > 0 {
            line(&mut out, "Programs' windows", format!("{} · {:.0} frames of theirs a second", c.windows, self.window_frames as f64 / seconds));
        }
        line(&mut out, "Measured", format!("{seconds:.0} s · animating {:.0}% of it · {n} frames", (animating / seconds * 100.0).min(100.0)));
        if n > 0 {
            line(&mut out, "Frame time", format!("median {:.1} · p95 {:.1} · p99 {:.1} · worst {:.1} ms (the screen gives {period:.1})", at(0.5), at(0.95), at(0.99), at(1.0)));
            let against = if steady { format!("its own pace ({:.1} ms)", at(0.5)) } else { "the screen's".into() };
            line(&mut out, "Late frames", format!("{} ({late_pct:.1}%) over 1.5 periods of {against} · {dropped} over 2.5 (a frame or more lost)", late.len()));
            let blocked = self.frames.iter().filter(|f| f.blocked).count();
            if blocked > 0 {
                line(&mut out, "With its logic busy", format!("{blocked} frames"));
            }
        }
        if let (Some(a), Some(b)) = (self.cpu_since, process_cpu_ms()) {
            line(&mut out, "Process CPU", format!("{:.0}% of a core", (b - a) / 10.0 / seconds));
        }
        if let Some(rss) = rss_mb() {
            line(&mut out, "Memory (RSS)", format!("{rss:.0} MB"));
        }
        if n > 0 {
            // Where a round goes, on average, and in the late ones.
            let mean = |frames: &[&Frame], k: usize| frames.iter().map(|f| f.sections[k] as f64).sum::<f64>() / frames.len().max(1) as f64;
            let all: Vec<&Frame> = self.frames.iter().collect();
            out.push_str("\n| per round (ms) | on average | in the late ones |\n|---|---|---|\n");
            for k in [0, 1, 7, 3, 4, 5, 6, 8] {
                let (a, l) = if k == 1 { (mean(&all, 1) + mean(&all, 2), mean(&late, 1) + mean(&late, 2)) } else { (mean(&all, k), mean(&late, k)) };
                out.push_str(&format!("| {} | {a:.2} | {} |\n", NAMES[k], if late.is_empty() { "—".to_owned() } else { format!("{l:.2}") }));
            }
            // The worst five, whole.
            let mut worst: Vec<&Frame> = self.frames.iter().collect();
            worst.sort_by(|a, b| b.ms.total_cmp(&a.ms));
            out.push_str("\nThe worst frames:\n");
            for f in worst.iter().take(5) {
                let s = f.sections;
                out.push_str(&format!(
                    "- {:.0} ms · input {:.1} · rules {:.1} · windows {:.1} · compose {:.1} · regions {:.1} · paint {:.1} · rest {:.1} · waiting {:.1}{}\n",
                    f.ms, s[0], s[1] + s[2], s[7], s[3], s[4], s[5], s[6], s[8], if f.blocked { " · logic busy" } else { "" }
                ));
            }
        }
        out
    }
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// The CPU the whole process has used, in milliseconds.
#[cfg(not(target_os = "windows"))]
fn process_cpu_ms() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // After the name, which may have spaces, between parentheses.
    let rest = &stat[stat.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split(' ').collect();
    let ticks: f64 = f.get(11)?.parse::<f64>().ok()? + f.get(12)?.parse::<f64>().ok()?;
    #[cfg(target_os = "linux")]
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    #[cfg(not(target_os = "linux"))]
    let hz: f64 = 100.0;
    Some(ticks * 1000.0 / hz.max(1.0))
}

#[cfg(not(target_os = "windows"))]
fn rss_mb() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: f64 = s.lines().find_map(|l| l.strip_prefix("VmRSS:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
    Some(kb / 1024.0)
}

#[cfg(target_os = "windows")]
use crate::platform::windows_diagnostics::{process_cpu_ms, rss_mb};
