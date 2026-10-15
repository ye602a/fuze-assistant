//! Two-second self/owned-worker counters. No process enumeration, game handles or frame-loop sampling.
use std::time::{Duration, Instant};
use windows_sys::Win32::{
    Foundation::{FILETIME, HANDLE},
    System::{ProcessStatus::*, Threading::*},
};
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub sampled: Option<String>,
    pub cpu: Option<f64>,
    pub memory_mb: Option<f64>,
}
pub(crate) fn process(handle: HANDLE) -> Option<(u64, u64)> {
    unsafe {
        let mut created: FILETIME = std::mem::zeroed();
        let mut ended = created;
        let mut kernel = created;
        let mut user = created;
        if GetProcessTimes(handle, &mut created, &mut ended, &mut kernel, &mut user) == 0 {
            return None;
        }
        let ticks = |v: FILETIME| ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64;
        let mut memory: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        memory.cb = std::mem::size_of_val(&memory) as u32;
        if GetProcessMemoryInfo(
            handle,
            &mut memory,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ) == 0
        {
            return None;
        }
        Some((ticks(kernel) + ticks(user), memory.WorkingSetSize as u64))
    }
}
#[derive(Default)]
pub struct Sampler {
    previous: Option<(Instant, u64, Option<u32>)>,
    attempt: Option<Instant>,
    pub latest: Snapshot,
}
impl Sampler {
    pub fn due(&self) -> bool {
        self.attempt
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(2))
    }
    pub fn sample(&mut self, worker: Option<(u32, u64, u64)>) {
        if !self.due() {
            return;
        }
        self.attempt = Some(Instant::now());
        let Some((host_cpu, host_memory)) = process(unsafe { GetCurrentProcess() }) else {
            self.latest = Default::default();
            return;
        };
        let (pid, cpu, memory) = worker
            .map(|(pid, cpu, memory)| (Some(pid), cpu, memory))
            .unwrap_or_default();
        let sum = host_cpu + cpu;
        let now = Instant::now();
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1) as f64;
        self.latest = Snapshot {
            sampled: Some(clock_ago(0)),
            cpu: self
                .previous
                .filter(|(_, old, old_pid)| *old <= sum && *old_pid == pid)
                .map(|(at, old, _)| {
                    (((sum - old) as f64 / 10_000_000.0) / (now - at).as_secs_f64() / cores * 100.0)
                        .clamp(0.0, 100.0)
                }),
            memory_mb: Some((host_memory + memory) as f64 / 1048576.0),
        };
        self.previous = Some((now, sum, pid));
    }
}

pub fn clock_ago(age: u64) -> String {
    let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t);
    }
    let seconds =
        (u64::from(t.wHour) * 3600 + u64::from(t.wMinute) * 60 + u64::from(t.wSecond) + 86400
            - age.min(86400))
            % 86400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
}
