//! Developer-only binary, excluded from player packages. Public transfer/staging never launch apps.
//! Private-mode secrets enter only via anonymous stdin.
#[cfg(windows)]
fn run() -> Result<(), &'static str> {
    use fuze_worker_host::{update_channel, update_credential::Secret, update_validation};
    use std::{io::Read, path::Path};
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let Some(mode) = args.first().map(String::as_str) {
        if matches!(
            mode,
            "--transport-public" | "--resume-seed-public" | "--resume-public"
        ) {
            let transfer = mode == "--transport-public";
            if args.len() != if transfer { 4 } else { 2 } {
                return Err("validation-arguments");
            }
            let session = update_channel::Session::public();
            let report = if transfer {
                update_validation::transport(
                    session,
                    Path::new(&args[1]),
                    Path::new(&args[2]),
                    Path::new(&args[3]),
                )?
            } else {
                update_validation::resume_transport(
                    session,
                    Path::new(&args[1]),
                    mode == "--resume-seed-public",
                )?
            };
            println!("{report}");
            return Ok(());
        }
        if mode == "--stage-only" {
            if args.len() != 6 {
                return Err("validation-arguments");
            }
            let read_manifest = |path: &str| {
                serde_json::from_slice::<update_channel::Manifest>(&update_channel::bounded(
                    Path::new(path),
                    16384,
                )?)
                .map_err(|_| "validation-manifest")
            };
            let report = update_validation::staging(
                Path::new(&args[5]),
                &read_manifest(&args[1])?,
                Path::new(&args[2]),
                &read_manifest(&args[3])?,
                Path::new(&args[4]),
            )?;
            println!("{report}");
            return Ok(());
        }
    }
    let transport_only = args.first().is_some_and(|arg| arg == "--transport-private");
    let resume = args
        .first()
        .is_some_and(|arg| matches!(arg.as_str(), "--resume-seed-private" | "--resume-private"));
    if args.len()
        != if resume {
            2
        } else if transport_only {
            4
        } else {
            5
        }
    {
        return Err("validation-arguments");
    }
    let report = if args[0] == "--private" || transport_only || resume {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(513)
            .read_to_end(&mut bytes)
            .map_err(|_| "credential-input")?;
        let session = update_channel::Session::with_secret(Secret::new(bytes)?);
        if resume {
            update_validation::resume_transport(
                session,
                Path::new(&args[1]),
                args[0] == "--resume-seed-private",
            )?
        } else if transport_only {
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
        serde_json::json!({"in_place_replacement":true,"in_place_hidden_launch":true,"shortcuts":true,"uninstall":true,"local_setup":true,"setup_idempotent":true,"installed":true,"repeat_launch":true,"health_failure_rollback":true,
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
