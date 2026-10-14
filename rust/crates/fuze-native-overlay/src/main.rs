#[cfg(windows)]
mod native;
#[cfg(windows)]
fn main() {
    if native::run().is_err() {
        eprintln!("fuze-native-overlay: prototype-validation-failed");
        std::process::exit(1);
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
    std::process::exit(1);
}
