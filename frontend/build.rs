fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let index_path = std::path::Path::new(&manifest_dir)
        .join("build/index.html")
        .canonicalize();

    if index_path.is_err() || !index_path.as_ref().is_ok_and(|p| p.is_file()) {
        panic!(
            "\n\nfrontend/build/index.html not found.\n\
             Build the frontend first: cd frontend && npm ci && npm run build\n\n"
        );
    }

    println!("cargo::rerun-if-changed=build");
}
