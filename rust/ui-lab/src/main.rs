mod art_cache;
mod champion_actions;
mod champion_art;
mod champion_bridge;
mod closure_review;
#[cfg(feature = "ui-audit")]
mod control_review;
mod desktop_runner;
mod diagnostic_bridge;
mod direct;
mod feedback;
mod feedback_bridge;
mod font_probe;
mod game_windows;
mod host_bridge;
mod host_review;
mod lifecycle;
mod maintenance_bridge;
mod maintenance_review;
mod manual_profile;
mod motion;
mod native_shell;
mod overlay_review;
mod public_art;
mod record_bridge;
#[cfg(feature = "gpu")]
mod skia_surface_probe;
mod slint_probe;
mod stage_bridge;
mod startup;
mod status_view;
mod telemetry;
mod unbound_bridge;
slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        #[cfg(feature = "ui-audit")]
        Some("--review-controls") if args.len()==2 => println!("{}",control_review::run(std::path::Path::new(&args[1]))?),
        Some("--desktop" | "--managed-desktop" | "--desktop-smoke" | "--managed-desktop-smoke") if args.len()==2 => {
            let smoke=args[0].ends_with("smoke");
            let report=desktop_runner::run(std::path::Path::new(&args[1]),smoke,args[0].starts_with("--managed"),"none")?;
            if smoke { println!("{report}"); }
        },
        Some("--render-session") => desktop_runner::child(&args[1..])?,
        Some("--review-recovery") if args.len()==3 => println!("{}",desktop_runner::review(std::path::Path::new(&args[1]),std::path::Path::new(&args[2]))?),
        Some("--review-host") if args.len() == 3 => println!("{}", host_review::run(std::path::Path::new(&args[1]), std::path::Path::new(&args[2]))?),
        Some("--host-offline") if args.len() == 2 => {
            slint::BackendSelector::new().backend_name("winit".into()).renderer_name(if cfg!(feature="gpu") {"skia"} else {"software"}.into()).with_winit_window_attributes_hook(|a|a.with_active(false)).select()?;
            let ui = Workbench::new()?;
            slint_probe::connect(&ui);
            let binding = host_bridge::attach(&ui, host_bridge::offline_config(std::path::Path::new(&args[1])), true)?;
            let _lifecycle = lifecycle::attach(&ui,binding.session.clone(),true,true)?;
            let _maintenance = maintenance_bridge::attach(&ui,binding.session.clone(),true,None)?;
            ui.show()?;
            slint::run_event_loop_until_quit()?;
        }
        Some("--manual-profile") if args.len()==3 => manual_profile::run(&args[1],std::path::Path::new(&args[2]))?,
        #[cfg(feature="gpu")]
        Some("--review-skia-surface") if args.len()==3 => {
            println!("{}",skia_surface_probe::review(&args[1],std::path::Path::new(&args[2]))?);
        }
        Some("--profile-fonts") if args.len() == 4 => {
            if !matches!(args[2].as_str(), "file" | "resident") {return Err("unknown font mode".into());}
            let font=font_probe::prepare(args[2]=="resident")?;
            let mut result=slint_probe::native_review(&args[1],std::path::Path::new(&args[3]))?;
            result["font_probe"]=font;
            std::fs::write(std::path::Path::new(&args[3]).join("font-profile.json"),serde_json::to_vec_pretty(&result)?)?;
            println!("{}",result);
        }
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
            let renderer = args.get(1).map(String::as_str).unwrap_or(if cfg!(feature="gpu") {"skia"} else {"software"});
            if !matches!(renderer, "software" | "skia" | "skia-software") { return Err("unsupported renderer".into()); }
            if renderer != "software" && !cfg!(feature = "gpu") { return Err("build with --features gpu first".into()); }
            motion::configure(renderer == "skia");
            slint::BackendSelector::new().backend_name("winit".into()).renderer_name(renderer.into()).select()?;
            let ui = Workbench::new()?;
            slint_probe::connect(&ui);
            ui.run()?;
        }
        Some("--overlay") if args.len() == 1 => direct::interactive()?,
        _ => println!("FUZE 开发入口：--review 输出目录（离线/隐藏自有窗口）；--desktop-smoke 配置文件（隔离启动检查）；--interactive [software|skia]（离线预览）；--desktop 配置文件（真实宿主，需空闲时主动运行）。阶段验收前不替换稳定版。"),
    }
    Ok(())
}
