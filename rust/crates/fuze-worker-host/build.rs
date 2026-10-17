#[path = "../../windows_icon.rs"]
mod windows_icon;
fn main() {
    windows_icon::embed("../../../public/fuze.ico");
}
