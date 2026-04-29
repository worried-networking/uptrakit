use std::fs;
use std::path::Path;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let build_dir = Path::new(&manifest_dir).join("build");
    let index_path = build_dir.join("index.html");

    // release-plz git_only mode runs `cargo package --workspace` in a fresh
    // worktree where `frontend/build/` (gitignored) does not exist. Production
    // builds always run `npm run build` first, so a real build/ is present
    // and overrides the stub. The stub keeps cargo package verify working.
    if !index_path.is_file() {
        fs::create_dir_all(&build_dir).expect("failed to create stub build/");
        fs::write(
            &index_path,
            "<!doctype html><title>uptrakit-frontend stub</title>\
             <p>Frontend assets were not built. Run `npm run build` in frontend/.</p>",
        )
        .expect("failed to write stub index.html");
        println!(
            "cargo::warning=frontend/build/index.html missing — wrote stub. \
             Run `npm run build` in frontend/ for real assets."
        );
    }

    println!("cargo::rerun-if-changed=build");
}
