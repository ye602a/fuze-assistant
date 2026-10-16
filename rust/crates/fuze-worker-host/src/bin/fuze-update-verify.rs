//! Developer-only binary, excluded from player packages. Secret enters only via anonymous stdin.
#[cfg(windows)]
fn run() -> Result<(), &'static str> {
    use fuze_worker_host::{update_channel, update_credential::Secret, update_validation};
    use std::{io::Read, path::Path};
    let args: Vec<_> = std::env::args().skip(1).collect();
    let transport_only = args.first().is_some_and(|arg| arg == "--transport-private");
    if args.len() != if transport_only { 4 } else { 5 } {
        return Err("validation-arguments");
    }
    let report = if args[0] == "--private" || transport_only {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(513)
            .read_to_end(&mut bytes)
            .map_err(|_| "credential-input")?;
        let session = update_channel::Session::with_secret(Secret::new(bytes)?);
        if transport_only {
            update_validation::transport(
                session,
                Path::new(&args[1]),
                Path::new(&args[2]),
                Path::new(&args[3]),
            )?
        } else {
            update_validation::remote(
                session,
                Path::new(&args[1]),
                Path::new(&args[2]),
                Path::new(&args[3]),
                Path::new(&args[4]),
            )?
        }
    } else if args[0] == "--offline" {
        let manifest: update_channel::Manifest =
            serde_json::from_slice(&update_channel::bounded(Path::new(&args[1]), 16384)?)
                .map_err(|_| "validation-manifest")?;
        manifest.validate()?;
        update_validation::lifecycle(
            Path::new(&args[4]),
            Path::new(&args[3]),
            Path::new(&args[2]),
            &manifest,
        )?;
        serde_json::json!({"shortcuts":true,"uninstall":true,"local_setup":true,"setup_idempotent":true,"installed":true,"repeat_launch":true,"health_failure_rollback":true,
            "process_start_failure_rollback":true,"hidden_native_window":true,"game_access":false})
    } else {
        return Err("validation-arguments");
    };
    println!("{report}");
    Ok(())
}
fn main() {
    #[cfg(windows)]
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
