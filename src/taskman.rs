//! The task manager's model: what a page's own counters say (`Performance.getMetrics`, per tab) and what the operating system
//! says about the processes this browser runs (read from `/proc`, per process). Pure parsing and arithmetic, tested; the window is
//! `app/taskman.rs`. docs/research/2026-11-21-task-manager.md.
//!
//! A tab's number is **main-thread CPU**: `TaskDuration` counts the seconds the page's main thread spent on tasks (script, layout,
//! style, painting set-up). It does not count web workers, the compositor, the network or the GPU, and Chromium shares a renderer
//! process between same-site tabs, so a process's CPU cannot be given to one tab: the process list is separate for that reason.

use serde_json::Value;
use std::collections::HashMap;

/// What one `Performance.getMetrics` answer says about a page.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TabMetrics {
    /// The page's own clock, in seconds (`Timestamp`): the wall time between two samples.
    pub timestamp: f64,
    /// Cumulative seconds of main-thread tasks (`TaskDuration`).
    pub task_secs: f64,
    pub js_used: f64,
    pub js_total: f64,
    pub nodes: u64,
    pub documents: u64,
    pub listeners: u64,
}

/// Read the JSON of a `Performance.getMetrics` result (`{"metrics":[{"name":..,"value":..}]}`). `None` without a clock.
pub fn parse_metrics(json: &str) -> Option<TabMetrics> {
    let v: Value = serde_json::from_str(json).ok()?;
    let mut m = TabMetrics::default();
    let mut clock = false;
    for entry in v.get("metrics")?.as_array()? {
        let (Some(name), Some(value)) = (entry.get("name").and_then(Value::as_str), entry.get("value").and_then(Value::as_f64)) else { continue };
        match name {
            "Timestamp" => {
                m.timestamp = value;
                clock = true;
            }
            "TaskDuration" => m.task_secs = value,
            "JSHeapUsedSize" => m.js_used = value,
            "JSHeapTotalSize" => m.js_total = value,
            "Nodes" => m.nodes = value as u64,
            "Documents" => m.documents = value as u64,
            "JSEventListeners" => m.listeners = value as u64,
            _ => {}
        }
    }
    clock.then_some(m)
}

/// The share of one CPU core the page's main thread used between two samples, in percent. `None` when the clock did not advance or
/// the counter went backwards (the page navigated and its counters started again): the new sample is then a fresh baseline.
pub fn tab_cpu_percent(prev: &TabMetrics, cur: &TabMetrics) -> Option<f64> {
    let wall = cur.timestamp - prev.timestamp;
    let busy = cur.task_secs - prev.task_secs;
    (wall > 0.05 && busy >= 0.0).then(|| (busy / wall * 100.0).min(100.0 * 8.0))
}

/// What a browser process is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Browser,
    Gpu,
    Network,
    Renderer,
    Utility,
    /// A program the browser started that is not Chromium (an extension's native messaging host).
    Other,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Browser => "Browser (windows, tabs, this program)",
            Kind::Gpu => "Graphics (GPU process)",
            Kind::Network => "Network service",
            Kind::Renderer => "Page renderer",
            Kind::Utility => "Utility",
            Kind::Other => "Helper program (extension native host)",
        }
    }
}

/// What a process is for, or `None` for the zygotes (forking helpers that do nothing of their own).
///
/// Chromium's `--type=` is only trustworthy for processes it starts itself. A renderer, the GPU process and most utilities are
/// *forked by a zygote* and keep the zygote's command line (`--type=zygote`), so they are told apart by the threads they run:
/// the GPU process has a GPU watchdog and a Viz compositor, a page renderer a `Compositor` thread.
pub fn classify(pid: u32, root: u32, ppid: u32, cmdline: &[String], threads: &[String]) -> Option<Kind> {
    let value = |name: &str| cmdline.iter().find_map(|a| a.strip_prefix(&format!("{name}=")));
    match value("--type") {
        None => Some(if pid == root { Kind::Browser } else { Kind::Other }),
        Some("zygote") if ppid == root => None,
        Some("zygote") => Some(kind_from_threads(threads)),
        Some("gpu-process") => Some(Kind::Gpu),
        Some("renderer") => Some(Kind::Renderer),
        Some("utility") => Some(if value("--utility-sub-type").is_some_and(|s| s.contains("NetworkService")) { Kind::Network } else { Kind::Utility }),
        Some(_) => Some(Kind::Utility),
    }
}

/// The kind of a zygote's child from the names of its threads.
pub fn kind_from_threads(threads: &[String]) -> Kind {
    let has = |name: &str| threads.iter().any(|t| t == name);
    if has("GpuWatchdog") || has("VizCompositorTh") {
        Kind::Gpu
    } else if has("Compositor") {
        Kind::Renderer
    } else {
        Kind::Utility
    }
}

/// One process, as read from `/proc`.
#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub kind: Kind,
    /// User and system time in clock ticks, cumulative.
    pub ticks: u64,
    /// Memory in bytes: proportional set size (shared pages such as the CEF library are split between the processes that map them),
    /// else the resident size.
    pub bytes: u64,
}

/// `(parent pid, cumulative ticks)` from the text of `/proc/PID/stat`. The command name is in parentheses and may hold spaces.
pub fn parse_stat(text: &str) -> Option<(u32, u64)> {
    let rest = text.get(text.rfind(')')? + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After the name: state, ppid, ..., utime (field 14 overall), stime (15).
    let ppid = fields.get(1)?.parse().ok()?;
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some((ppid, utime + stime))
}

/// Bytes from a `/proc` file of `Key:   123 kB` lines (`Pss` of `smaps_rollup`, `VmRSS` of `status`).
pub fn parse_kb(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':')?.split_whitespace().next()?.parse::<u64>().ok()).map(|kb| kb * 1024)
}

/// The command line of `/proc/PID/cmdline` (NUL separated).
pub fn split_cmdline(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect()
}

/// Every process that descends from `root` (this program), classified, from a `/proc`-shaped reader. `read(pid, file)` gives a file's
/// bytes (`threads` is the thread names, one per line). Split from `scan` so it is tested with made-up processes.
pub fn scan_with(root: u32, pids: &[u32], read: &dyn Fn(u32, &str) -> Option<Vec<u8>>) -> Vec<Proc> {
    let text = |pid: u32, file: &str| read(pid, file).map(|b| String::from_utf8_lossy(&b).into_owned());
    let stats: HashMap<u32, (u32, u64)> = pids.iter().filter_map(|p| Some((*p, parse_stat(&text(*p, "stat")?)?))).collect();
    let descends = |mut pid: u32| {
        for _ in 0..16 {
            if pid == root {
                return true;
            }
            match stats.get(&pid) {
                Some((ppid, _)) if *ppid != pid && *ppid != 0 => pid = *ppid,
                _ => return false,
            }
        }
        false
    };
    let mut out = Vec::new();
    for pid in pids.iter().copied().filter(|p| descends(*p)) {
        let cmdline = read(pid, "cmdline").map(|c| split_cmdline(&c)).unwrap_or_default();
        let threads: Vec<String> = text(pid, "threads").map(|t| t.lines().map(str::to_owned).collect()).unwrap_or_default();
        let Some(kind) = classify(pid, root, stats[&pid].0, &cmdline, &threads) else { continue };
        let bytes = text(pid, "smaps_rollup").and_then(|t| parse_kb(&t, "Pss")).or_else(|| text(pid, "status").and_then(|t| parse_kb(&t, "VmRSS"))).unwrap_or(0);
        out.push(Proc { pid, kind, ticks: stats[&pid].1, bytes });
    }
    out
}

/// This program's processes now, read from `/proc`.
pub fn scan() -> Vec<Proc> {
    let pids: Vec<u32> = std::fs::read_dir("/proc").into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect();
    scan_with(std::process::id(), &pids, &|pid, file| {
        if file == "threads" {
            let names: String = std::fs::read_dir(format!("/proc/{pid}/task")).ok()?.flatten().filter_map(|t| std::fs::read_to_string(t.path().join("comm")).ok()).collect();
            return Some(names.into_bytes());
        }
        std::fs::read(format!("/proc/{pid}/{file}")).ok()
    })
}

/// Clock ticks per second (`_SC_CLK_TCK`, 100 on Linux).
pub fn ticks_per_second() -> f64 {
    // SAFETY: sysconf has no preconditions.
    let t = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if t > 0 { t as f64 } else { 100.0 }
}

/// A process's CPU between two samples, in percent of one core.
pub fn proc_cpu_percent(prev_ticks: u64, cur_ticks: u64, wall_secs: f64, ticks_per_second: f64) -> f64 {
    if wall_secs <= 0.0 {
        return 0.0;
    }
    cur_ticks.saturating_sub(prev_ticks) as f64 / ticks_per_second / wall_secs * 100.0
}

/// "340 MB", "1.2 GB".
pub fn bytes_text(bytes: f64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes >= 1024.0 * MB {
        format!("{:.1} GB", bytes / (1024.0 * MB))
    } else if bytes >= 10.0 * MB {
        format!("{:.0} MB", bytes / MB)
    } else {
        format!("{:.1} MB", bytes / MB)
    }
}

/// "12%", "0.4%" (small values keep a decimal so a quiet tab does not read as zero).
pub fn percent_text(p: f64) -> String {
    if p >= 9.5 { format!("{p:.0}%") } else if p >= 0.05 { format!("{p:.1}%") } else { "0%".to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const METRICS: &str = r#"{"metrics":[{"name":"Timestamp","value":100.5},{"name":"Documents","value":3},{"name":"Nodes","value":1200},{"name":"JSEventListeners","value":88},{"name":"JSHeapUsedSize","value":5242880},{"name":"JSHeapTotalSize","value":8388608},{"name":"TaskDuration","value":2.25}]}"#;

    #[test]
    fn metrics_are_read_by_name() {
        let m = parse_metrics(METRICS).unwrap();
        assert_eq!((m.timestamp, m.task_secs, m.js_used, m.nodes, m.documents, m.listeners), (100.5, 2.25, 5242880.0, 1200, 3, 88));
        assert!(parse_metrics(r#"{"metrics":[{"name":"Nodes","value":1}]}"#).is_none(), "no clock, no sample");
        assert!(parse_metrics("not json").is_none());
    }

    #[test]
    fn tab_cpu_is_the_busy_share_of_the_wall_time_and_a_reset_is_a_baseline() {
        let a = TabMetrics { timestamp: 10.0, task_secs: 1.0, ..Default::default() };
        let b = TabMetrics { timestamp: 12.0, task_secs: 1.5, ..Default::default() };
        assert_eq!(tab_cpu_percent(&a, &b), Some(25.0));
        let reset = TabMetrics { timestamp: 14.0, task_secs: 0.1, ..Default::default() };
        assert_eq!(tab_cpu_percent(&b, &reset), None, "the counter went backwards: the page navigated");
        assert_eq!(tab_cpu_percent(&a, &a), None, "the clock did not advance");
    }

    #[test]
    fn processes_are_told_apart_by_their_command_line_and_threads() {
        let cmd = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
        let t = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert_eq!(classify(10, 10, 1, &cmd("browsah --profile=home"), &[]), Some(Kind::Browser));
        assert_eq!(classify(30, 10, 10, &cmd("x --type=gpu-process"), &[]), Some(Kind::Gpu));
        assert_eq!(classify(30, 10, 10, &cmd("x --type=utility --utility-sub-type=network.mojom.NetworkService"), &[]), Some(Kind::Network));
        assert_eq!(classify(30, 10, 10, &cmd("x --type=utility --utility-sub-type=audio.mojom.AudioService"), &[]), Some(Kind::Utility));
        assert_eq!(classify(31, 10, 10, &cmd("/opt/1Password/1Password-BrowserSupport chrome-extension://x/"), &[]), Some(Kind::Other));
        // A zygote itself is skipped; what it forks keeps `--type=zygote` and is read from its threads.
        assert_eq!(classify(11, 10, 10, &cmd("x --type=zygote --no-sandbox"), &[]), None);
        assert_eq!(classify(12, 10, 11, &cmd("x --type=zygote"), &t(&["browsah", "Compositor", "Chrome_ChildIOT"])), Some(Kind::Renderer));
        assert_eq!(classify(13, 10, 11, &cmd("x --type=zygote"), &t(&["GpuWatchdog", "VizCompositorTh"])), Some(Kind::Gpu));
        assert_eq!(classify(14, 10, 11, &cmd("x --type=zygote"), &t(&["browsah", "ThreadPoolForeg"])), Some(Kind::Utility));
    }

    #[test]
    fn stat_and_memory_files_are_parsed() {
        // The name holds a space and a parenthesis: fields are counted from the last ")".
        let stat = "123 (my (odd) name) S 45 123 123 0 -1 4194560 100 0 0 0 700 300 0 0 20 0 5 0 99 1000 2000";
        assert_eq!(parse_stat(stat), Some((45, 1000)));
        assert_eq!(parse_kb("Rss:       100 kB\nPss:       2048 kB\n", "Pss"), Some(2048 * 1024));
        assert_eq!(parse_kb("VmRSS:\t  512 kB\n", "VmRSS"), Some(512 * 1024));
        assert_eq!(split_cmdline(b"a\0--type=gpu-process\0"), vec!["a".to_owned(), "--type=gpu-process".to_owned()]);
    }

    #[test]
    fn only_our_descendants_are_listed_and_zygotes_left_out() {
        let files: HashMap<(u32, &str), String> = HashMap::from([
            ((12, "threads"), "browsah\nCompositor\n".into()),
            ((10, "stat"), "10 (browsah) S 1 0 0 0 0 0 0 0 0 0 100 50 0 0 0 0 1 0 0 0 0".into()),
            ((10, "cmdline"), "browsah\0--profile=x\0".into()),
            ((10, "smaps_rollup"), "Pss: 1000 kB\n".into()),
            ((11, "stat"), "11 (browsah) S 10 0 0 0 0 0 0 0 0 0 5 5 0 0 0 0 1 0 0 0 0".into()),
            ((11, "cmdline"), "browsah\0--type=zygote\0".into()),
            ((12, "stat"), "12 (browsah) S 11 0 0 0 0 0 0 0 0 0 40 20 0 0 0 0 1 0 0 0 0".into()),
            ((12, "cmdline"), "browsah\0--type=renderer\0".into()),
            ((12, "status"), "VmRSS: 300 kB\n".into()),
            ((20, "stat"), "20 (other) S 1 0 0 0 0 0 0 0 0 0 1 1 0 0 0 0 1 0 0 0 0".into()),
            ((20, "cmdline"), "other\0--type=renderer\0".into()),
        ]);
        let read = |pid: u32, file: &str| files.get(&(pid, file)).map(|s| s.clone().into_bytes());
        let found = scan_with(10, &[10, 11, 12, 20], &read);
        assert_eq!(found, vec![Proc { pid: 10, kind: Kind::Browser, ticks: 150, bytes: 1000 * 1024 }, Proc { pid: 12, kind: Kind::Renderer, ticks: 60, bytes: 300 * 1024 }]);
    }

    #[test]
    fn cpu_and_sizes_read_well() {
        assert_eq!(proc_cpu_percent(100, 150, 1.0, 100.0), 50.0);
        assert_eq!(proc_cpu_percent(200, 100, 1.0, 100.0), 0.0, "a replaced process never gives a negative share");
        assert_eq!(bytes_text(340.0 * 1048576.0), "340 MB");
        assert_eq!(bytes_text(1.5 * 1073741824.0), "1.5 GB");
        assert_eq!(bytes_text(2.5 * 1048576.0), "2.5 MB");
        assert_eq!((percent_text(0.0), percent_text(0.4), percent_text(12.3)), ("0%".to_owned(), "0.4%".to_owned(), "12%".to_owned()));
    }
}
