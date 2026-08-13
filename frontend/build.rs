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
    } else if std::env::var("PROFILE").as_deref() == Ok("debug") {
        // frontend/build/ is gitignored; compile-only debug contexts (CI
        // lint/test jobs, pre-push, local workspace builds) may legitimately
        // lack it — embed a stub and warn.
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
    } else {
        // A release-profile build produces a shippable binary, where a stub
        // would silently replace the real UI (the exact hazard: release-plz
        // binaries, docker images, `cargo install`). Fail-closed: anything
        // cargo ever reports other than "debug" (i.e. any profile not
        // inheriting `dev`) is treated as shippable.
        println!(
            "cargo::error=frontend/build/index.html missing in a release-profile \
             build — refusing to embed the stub UI. Run `npm run build` in \
             frontend/ (CI: ensure the frontend-build artifact was downloaded \
             to frontend/build/)."
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
