fn main() {
    uptrakit_build_info::emit_enabled_features_env();

    let release_name = std::env::var("UPTRAKIT_RELEASE_NAME")
        .unwrap_or_else(|_| "uptrakit-controller".to_string());
    println!("cargo:rustc-env=UPTRAKIT_RELEASE_NAME={release_name}");
    println!("cargo:rerun-if-env-changed=UPTRAKIT_RELEASE_NAME");

    // When the embed-frontend feature is enabled, verify the frontend build
    // output exists. This gives a clear error at compile time rather than a
    // cryptic RustEmbed error about a missing folder.
    if std::env::var("CARGO_FEATURE_EMBED_FRONTEND").is_ok() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
        let index_path = std::path::Path::new(&manifest_dir)
            .join("../../../frontend/build/index.html")
            .canonicalize();

        if index_path.is_err() || !index_path.as_ref().is_ok_and(|p| p.is_file()) {
            panic!(
                "\n\n\
                 embed-frontend feature is enabled but frontend/build/index.html was not found.\n\
                 Build the frontend first: cd frontend && npm ci && npm run build\n\n"
            );
        }

        // Tell Cargo to re-run the build script if the frontend build output changes.
        println!("cargo::rerun-if-changed=../../../frontend/build");
    }
}
