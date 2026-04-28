fn main() {
    uptrakit_build_info::emit_enabled_features_env();

    let release_name = std::env::var("UPTRAKIT_RELEASE_NAME")
        .unwrap_or_else(|_| "uptrakit-controller".to_string());
    println!("cargo:rustc-env=UPTRAKIT_RELEASE_NAME={release_name}");
    println!("cargo:rerun-if-env-changed=UPTRAKIT_RELEASE_NAME");
}
