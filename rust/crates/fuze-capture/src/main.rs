fn main() {
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some("--synthetic-readback") {
        if let Ok(report) = fuze_capture::native::synthetic_readback() {
            println!("{report}");
            return;
        }
    }
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some("--adapters") {
        match fuze_capture::adapter::enumerate() {
            Ok(adapters) => {
                println!("{}", serde_json::json!({"adapters": adapters}));
                return;
            }
            Err(_) => {}
        }
    }
    eprintln!("fuze-capture: unsupported-or-failed");
    std::process::exit(1);
}
