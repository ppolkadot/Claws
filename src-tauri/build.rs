fn main() {
    // tauri-build embeds its manifest (Common Controls v6) only into app executables; test
    // binaries linking the mock runtime then fail to load with STATUS_ENTRYPOINT_NOT_FOUND
    // (TaskDialogIndirect). So the same manifest is linked into every target here instead.
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)).expect("tauri-build failed");

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        // Exactly that manifest: no linker-added UAC section.
        println!("cargo:rustc-link-arg=/MANIFESTUAC:NO");
        println!("cargo:rustc-link-arg=/WX");
    }
}
