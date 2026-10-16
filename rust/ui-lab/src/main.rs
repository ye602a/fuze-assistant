mod direct;
mod slint_probe;
mod telemetry;
slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--review-native") if args.len() == 3 => {
            println!("{}", slint_probe::native_review(&args[1], std::path::Path::new(&args[2]))?);
        }
        Some("--review-direct") if args.len() == 2 => {
            let output = std::path::Path::new(&args[1]);
            std::fs::create_dir_all(output)?;
            println!("{}", direct::review(output)?);
        }
        Some("--review") if args.len() == 2 => {
            let output = std::path::Path::new(&args[1]);
            std::fs::create_dir_all(output)?;
            let slint = slint_probe::review(output)?;
            let direct = direct::review(output)?;
            let report = serde_json::json!({
                "schema": 1, "game_access": false, "system_input": false,
                "slint": slint, "direct2d": direct,
                "acceptance": "prototype_only",
                "manual_pending": ["real_ime", "mixed_monitor_dpi", "rdp_reconnect", "exclusive_fullscreen", "slint_gpu"]
            });
            std::fs::write(output.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
            println!("{}", report);
        }
        Some("--interactive") if args.len() <= 2 => {
            let renderer = args.get(1).map(String::as_str).unwrap_or("software");
            if !matches!(renderer, "software" | "skia") { return Err("unsupported renderer".into()); }
            if renderer == "skia" && !cfg!(feature = "gpu") { return Err("build with --features gpu first".into()); }
            slint::BackendSelector::new().backend_name("winit".into()).renderer_name(renderer.into()).select()?;
            let ui = Workbench::new()?;
            slint_probe::connect(&ui);
            ui.run()?;
        }
        Some("--overlay") if args.len() == 1 => direct::interactive()?,
        _ => println!("FUZE 界面原型：--review 输出目录（离屏/隐藏自有窗口）；--interactive [software|skia]；--overlay（需用户手动运行）。不访问游戏，不保存输入。"),
    }
    Ok(())
}
