fn main() {
    // Slint's generated component constructors need room even in unoptimized developer builds.
    // Reserve virtual address space only; Windows commits stack pages on demand.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }
    slint_build::compile("ui/workbench.slint").expect("compile isolated workbench");
}
