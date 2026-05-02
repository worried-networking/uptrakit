use std::fs;
use std::path::{Path, PathBuf};

#[expect(
    clippy::expect_used,
    reason = "build script — panicking on missing environment variables or I/O errors is the correct behaviour"
)]
fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));
    let embed_dir = out_dir.join("embed");

    if embed_dir.exists() {
        fs::remove_dir_all(&embed_dir).expect("clear embed dir");
    }
    fs::create_dir_all(&embed_dir).expect("create embed dir");

    let src_build = manifest_dir.join("build");
    let src_index = src_build.join("index.html");

    if src_index.is_file() {
        copy_dir_recursive(&src_build, &embed_dir).expect("copy build/ to OUT_DIR");
    } else {
        // release-plz git_only mode runs `cargo package --workspace` in a fresh
        // worktree where `frontend/build/` (gitignored) does not exist. Embed
        // a stub so the package verifies. Production builds always run
        // `npm run build` first, populating `frontend/build/` with real assets.
        let stub = embed_dir.join("index.html");
        fs::write(
            &stub,
            "<!doctype html><title>uptrakit-frontend stub</title>\
             <p>Frontend assets were not built. Run `npm run build` in frontend/.</p>",
        )
        .expect("write stub index.html");
        println!(
            "cargo::warning=frontend/build/ missing — embedded stub. \
             Run `npm run build` in frontend/ for real assets."
        );
    }

    println!("cargo::rerun-if-changed=build");
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            fs::create_dir_all(&target)?;
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}
