use std::process::Command;

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn main() {
    // Bake in where this build came from, so the app can update itself from the same clone.
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    println!("cargo:rustc-env=PDF_TUTOR_SOURCE={}", source.display());
    println!("cargo:rustc-env=PDF_TUTOR_COMMIT={}", git(&["rev-parse", "HEAD"]));
    println!("cargo:rustc-env=PDF_TUTOR_COMMIT_DATE={}", git(&["log", "-1", "--format=%cI"]));
    for p in ["../.git/HEAD", "../.git/refs/heads", "../.git/packed-refs"] {
        println!("cargo:rerun-if-changed={p}");
    }
    tauri_build::build()
}
