#![cfg_attr(windows, windows_subsystem = "windows")]
//! Explicit, finite developer entry point. Never invoked by offline CI or the stable app.
#[cfg(windows)]
fn run() -> Result<(), &'static str> {
    use fuze_worker_host::controller::{Controller, Monitor};
    use std::time::{Duration, Instant};
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() {
        let exe = std::env::current_exe().map_err(|_| "config-path")?;
        args = vec![
            "--desktop".into(),
            exe.with_file_name("preview.json")
                .to_string_lossy()
                .into_owned(),
        ];
    }
    if args.len() == 2 && args[0] == "--prepare-data" {
        let versions =
            fuze_worker_host::data_update::refresh(std::path::Path::new(&args[1]), &|| false)?;
        println!(
            "{}",
            serde_json::json!({"data_ready":true,"versions":versions,"game_access":false})
        );
        return Ok(());
    }
    let desktop = args.len() == 2
        && [
            "--desktop",
            "--managed-desktop",
            "--desktop-smoke",
            "--managed-desktop-smoke",
        ]
        .contains(&args[0].as_str());
    let check = args.len() == 2 && args[0] == "--check-bundle";
    if !check && !desktop && (args.len() != 3 || args[0] != "--live-control") {
        return Err("usage-live-control-config-seconds");
    }
    let seconds = if check || desktop {
        0
    } else {
        args[2]
            .parse::<u64>()
            .ok()
            .filter(|v| (1..=3600).contains(v))
            .ok_or("duration-invalid")?
    };
    let config = fuze_worker_host::host_config::load(std::path::Path::new(&args[1]))?;
    if desktop {
        return fuze_worker_host::desktop::run(
            config,
            ["--desktop-smoke", "--managed-desktop-smoke"].contains(&args[0].as_str()),
            ["--managed-desktop", "--managed-desktop-smoke"].contains(&args[0].as_str()),
        );
    }
    if check {
        config.check_bundle()?;
        println!(
            "{{\"bundle_ready\":true,\"locales\":3,\"worker_reaped\":true,\"game_access\":false}}"
        );
        return Ok(());
    }
    let monitor = Monitor::start()?;
    let mut controller = Controller::new(config);
    let started = Instant::now();
    let mut last_error = None;
    let mut last_connection = None;
    while started.elapsed() < Duration::from_secs(seconds) {
        let connection = monitor.state();
        if connection != last_connection {
            eprintln!("fuze-lcu: {connection:?}"); // State contains fixed enums only.
            last_connection = connection;
        }
        if let Err(error) = controller.tick(&monitor) {
            // All controller errors are static categories; no credentials, IDs, images or paths.
            if last_error != Some(error) {
                eprintln!("fuze-host: {error}");
                last_error = Some(error);
            }
        }
        std::thread::sleep(controller.idle_wait());
    }
    drop(controller);
    drop(monitor);
    Ok(())
}
fn main() {
    #[cfg(windows)]
    if let Err(error) = run() {
        eprintln!("fuze-host: {error}");
        if std::env::args().len() == 1 || std::env::args().nth(1).as_deref() == Some("--desktop") {
            fuze_worker_host::desktop::startup_error(error);
        }
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    std::process::exit(1);
}
