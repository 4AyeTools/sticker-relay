fn main() {
    let is_release = std::env::var("PROFILE").is_ok_and(|profile| profile == "release");
    let has_custom_protocol = std::env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_some();

    if is_release && !has_custom_protocol {
        panic!(
            "release builds require the `custom-protocol` feature; otherwise the app tries to open the development localhost URL"
        );
    }

    tauri_build::build()
}
