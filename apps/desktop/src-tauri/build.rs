//! Build script: Tauri's codegen. The app's own commands are listed so that each one
//! needs an explicit permission in `capabilities/` (a test keeps the two in sync).

include!("src/command_names.rs");

fn main() -> Result<(), String> {
    // `frontendDist` must exist at compile time (clippy, tests): a stub until the UI is built.
    let dist = std::path::Path::new("../build");
    if !dist.exists() {
        std::fs::create_dir_all(dist).map_err(|e| e.to_string())?;
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><title>cloudit</title><p>The UI is not built: run make build-desktop.",
        )
        .map_err(|e| e.to_string())?;
    }
    println!("cargo:rerun-if-changed=src/command_names.rs");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .map_err(|e| e.to_string())
}
