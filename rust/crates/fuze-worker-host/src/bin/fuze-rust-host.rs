//! Explicit, finite developer entry point. Never invoked by offline CI or the stable app.
#[cfg(windows)]
fn run() -> Result<(), &'static str> {
    use fuze_worker_host::controller::{Config, Controller, Monitor};
    use std::{
        io::Read,
        path::PathBuf,
        time::{Duration, Instant},
    };
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        bundled_data: PathBuf,
        cache_data: Option<PathBuf>,
        locale: String,
        worker: PathBuf,
        ort: PathBuf,
        model: PathBuf,
        dictionary: PathBuf,
        backend: String,
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    let check = args.len() == 2 && args[0] == "--check-bundle";
    if !check && (args.len() != 3 || args[0] != "--live-control") {
        return Err("usage-live-control-config-seconds");
    }
    let seconds = if check {
        0
    } else {
        args[2]
            .parse::<u64>()
            .ok()
            .filter(|v| (1..=3600).contains(v))
            .ok_or("duration-invalid")?
    };
    let mut bytes = Vec::new();
    std::fs::File::open(&args[1])
        .map_err(|_| "config-read")?
        .take(16385)
        .read_to_end(&mut bytes)
        .map_err(|_| "config-read")?;
    if bytes.len() > 16384 {
        return Err("config-limit");
    }
    let input: Input = serde_json::from_slice(&bytes).map_err(|_| "config-invalid")?;
    let config_path = std::fs::canonicalize(&args[1]).map_err(|_| "config-read")?;
    let directory = config_path.parent().ok_or("config-path")?;
    let resolve = |path: PathBuf| {
        if path.is_absolute() {
            path
        } else {
            directory.join(path)
        }
    };
    let locale = match input.locale.as_str() {
        "auto" => None,
        "zh-CN" => Some(fuze_data::Locale::ZhCn),
        "zh-TW" => Some(fuze_data::Locale::ZhTw),
        "en-US" => Some(fuze_data::Locale::EnUs),
        _ => return Err("locale-invalid"),
    };
    if !["cpu", "auto", "dml", "dml-strict"].contains(&input.backend.as_str()) {
        return Err("backend-invalid");
    }
    let mut roots = vec![fuze_data::Root {
        path: resolve(input.bundled_data),
        source: fuze_data::Source::Bundled,
    }];
    if let Some(path) = input.cache_data {
        roots.push(fuze_data::Root {
            path: resolve(path),
            source: fuze_data::Source::Cache,
        });
    }
    let config = Config {
        roots,
        locale,
        worker: resolve(input.worker),
        ort: resolve(input.ort),
        model: resolve(input.model),
        dictionary: resolve(input.dictionary),
        backend: input.backend,
    };
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
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    std::process::exit(1);
}
