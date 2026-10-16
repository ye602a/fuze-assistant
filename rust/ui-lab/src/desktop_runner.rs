//! One bounded renderer retry in a fresh process. No steady-state polling or game access here.
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Debug)]
pub struct GraphicsFault;
impl std::fmt::Display for GraphicsFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("desktop-graphics-failed")
    }
}
impl std::error::Error for GraphicsFault {}
pub fn graphics(_: impl std::fmt::Display) -> Box<dyn std::error::Error> {
    Box::new(GraphicsFault)
}
fn emit(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}
struct OwnedChild {
    child: Child,
    _job: fuze_worker_host::ChildJob,
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
#[derive(Default)]
struct Attempt {
    healthy: bool,
    graphics: bool,
    complete: bool,
    handoff: Option<bool>,
    smoke_ok: bool,
}

fn attempt(
    config: &Path,
    smoke: bool,
    software: bool,
    paused: bool,
    fault: &str,
    announce: &mut bool,
    managed: bool,
) -> Result<Attempt> {
    let job = fuze_worker_host::ChildJob::for_ui_session()?;
    let child = Command::new(std::env::current_exe()?)
        .arg("--render-session")
        .arg(config)
        .arg(if smoke { "offline" } else { "live" })
        .arg(if software { "software" } else { "gpu" })
        .arg(if paused { "paused" } else { "normal" })
        .arg(fault)
        .env("FUZE_MANAGED_HEALTH_PROTOCOL", "2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()?;
    let mut owned = OwnedChild { child, _job: job };
    owned._job.assign(&owned.child)?;
    // The child cannot initialize Slint or the host until job assignment succeeds.
    owned
        .child
        .stdin
        .take()
        .ok_or("desktop-session-input")?
        .write_all(b"F")?;
    let stdout = owned.child.stdout.take().ok_or("desktop-session-output")?;
    let (tx, rx) = mpsc::sync_channel(16);
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout.take(4096)).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut state = Attempt::default();
    let mut terminal_at = None;
    let outcome = (|| -> Result<()> {
        loop {
            // After readiness the reader blocks on the pipe; there is no idle polling.
            let timeout = if let Some(at) = terminal_at {
                Duration::from_secs(8).saturating_sub(Instant::now().duration_since(at))
            } else if state.healthy && !smoke {
                Duration::from_secs(86400)
            } else {
                Duration::from_secs(18).saturating_sub(started.elapsed())
            };
            let line = match rx.recv_timeout(timeout) {
                Ok(v) => v,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout)
                    if state.healthy && !smoke && terminal_at.is_none() =>
                {
                    continue
                }
                Err(_) => return Err("desktop-session-timeout".into()),
            };
            match line.as_str() {
                "FUZE_DESKTOP_READY_2" | "FUZE_DESKTOP_CLOSED_2" => (),
                "FUZE_DESKTOP_HEALTHY_2" => {
                    state.healthy = true;
                    if managed && !*announce {
                        // Commit readiness only after this attempt has survived its health interval.
                        emit("FUZE_DESKTOP_READY_2");
                        emit("FUZE_DESKTOP_HEALTHY_2");
                        *announce = true;
                    }
                }
                "FUZE_RENDER_GRAPHICS" => {
                    state.graphics = true;
                    terminal_at = Some(Instant::now());
                }
                "FUZE_RENDER_DONE" => {
                    state.complete = true;
                    terminal_at = Some(Instant::now());
                }
                "FUZE_RENDER_UPDATE_PUBLIC" | "FUZE_RENDER_UPDATE_PRIVATE" if !smoke => {
                    state.complete = true;
                    state.handoff = Some(line == "FUZE_RENDER_UPDATE_PRIVATE");
                    terminal_at = Some(Instant::now());
                }
                "FUZE_RENDER_FAILED" => return Err("desktop-session-failed".into()),
                _ if smoke => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                        state.smoke_ok = v["desktop_ready"] == true
                            && v["pause_resume_exit"] == true
                            && v["game_access"] == false
                            && v["hidden"] == true;
                    }
                }
                _ => (), // Never echo arbitrary child output or exception text.
            }
        }
        let end = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(status) = owned.child.try_wait()? {
                if state.graphics || status.success() && state.complete {
                    break;
                }
                return Err("desktop-session-exit".into());
            }
            if Instant::now() >= end {
                return Err("desktop-session-exit-timeout".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    })();
    // Close the process job before considering a replacement or update handoff.
    drop(owned);
    drop(rx);
    let _ = reader.join();
    outcome?;
    Ok(state)
}

pub fn run(config: &Path, smoke: bool, managed: bool, fault: &str) -> Result<serde_json::Value> {
    if !smoke && fault != "none" {
        return Err("desktop-live-fault-disabled".into());
    }
    let config = std::fs::canonicalize(config)?;
    let name = if smoke {
        format!("Local\\FuzeSlintSupervisorSmoke-{}", std::process::id())
    } else {
        "Local\\FuzeRustDesktopPreview".into()
    };
    let instance = crate::startup::Instance::acquire(&name)?;
    let mut announce = false;
    let mut software = !cfg!(feature = "gpu") || smoke && fault == "none";
    let mut paused = false;
    let mut attempts = 0;
    let finished = loop {
        attempts += 1;
        let state = attempt(
            &config,
            smoke,
            software,
            paused,
            fault,
            &mut announce,
            managed,
        )?;
        if state.graphics {
            if software {
                return Err("desktop-software-failed".into());
            }
            software = true;
            paused = state.healthy;
            continue;
        }
        if smoke && !state.smoke_ok && fault != "exit" {
            return Err("desktop-smoke-incomplete".into());
        }
        break state;
    };
    drop(instance);
    if let Some(private) = finished.handoff {
        // Spawn outside the rendering job, after both session and supervisor mutexes are gone.
        crate::maintenance_bridge::Handoff::prepare(private)?.launch()?;
    }
    if managed && !smoke && !announce {
        emit("FUZE_DESKTOP_CLOSED_2");
    }
    Ok(
        serde_json::json!({"desktop_ready":finished.healthy,"pause_resume_exit":smoke && finished.smoke_ok,
        "game_access":!smoke,"hidden":smoke,"attempts":attempts,"software":software,
        "recovered_paused":paused,"synthetic_fault":fault!="none" && fault!="actual-gpu"}),
    )
}

pub fn child(args: &[String]) -> Result<()> {
    if args.len() != 5 {
        return Err("desktop-session-arguments".into());
    }
    let smoke = match args[1].as_str() {
        "offline" => true,
        "live" => false,
        _ => return Err("desktop-session-mode".into()),
    };
    let software = match args[2].as_str() {
        "software" => true,
        "gpu" => false,
        _ => return Err("desktop-session-renderer".into()),
    };
    let paused = match args[3].as_str() {
        "paused" => true,
        "normal" => false,
        _ => return Err("desktop-session-resume".into()),
    };
    let fault = args[4].as_str();
    if !matches!(
        fault,
        "none" | "actual-gpu" | "gpu-start" | "gpu-runtime" | "both" | "ordinary" | "exit"
    ) || !smoke && fault != "none"
    {
        return Err("desktop-session-fault".into());
    }
    let mut gate = [0];
    std::io::stdin().read_exact(&mut gate)?;
    if gate != [b'F'] {
        return Err("desktop-session-gate".into());
    }
    let result = crate::startup::run_session(Path::new(&args[0]), smoke, software, paused, fault);
    match result {
        Ok(Some(handoff)) => emit(if handoff.private_channel() {
            "FUZE_RENDER_UPDATE_PRIVATE"
        } else {
            "FUZE_RENDER_UPDATE_PUBLIC"
        }),
        Ok(None) => emit("FUZE_RENDER_DONE"),
        Err(e) => {
            emit(if e.is::<GraphicsFault>() {
                "FUZE_RENDER_GRAPHICS"
            } else {
                "FUZE_RENDER_FAILED"
            });
            return Err("desktop-session-failed".into());
        }
    }
    Ok(())
}

pub fn review(config: &Path, output: &Path) -> Result<serde_json::Value> {
    std::fs::create_dir_all(output)?;
    let start = run(config, true, false, "gpu-start")?;
    let runtime = run(config, true, false, "gpu-runtime")?;
    let closed = run(config, true, false, "exit")?;
    if start["attempts"] != 2
        || start["software"] != true
        || runtime["attempts"] != 2
        || runtime["recovered_paused"] != true
        || closed["attempts"] != 1
        || closed["desktop_ready"] != false
    {
        return Err("recovery-transition".into());
    }
    if run(config, true, false, "both").is_ok() || run(config, true, false, "ordinary").is_ok() {
        return Err("recovery-fail-closed".into());
    }
    let report = serde_json::json!({"startup":start,"runtime":runtime,"software_failure_stops":true,
        "ordinary_error_not_retried":true,"normal_exit_not_restarted":true,"game_access":false,"system_input":false,
        "runtime_fault_uses_software_fixture":true,"actual_device_loss_verified":false});
    std::fs::write(
        output.join("renderer-recovery.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
