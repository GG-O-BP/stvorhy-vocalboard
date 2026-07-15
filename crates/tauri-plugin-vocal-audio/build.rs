const COMMANDS: &[&str] = &[
    "check_permissions",
    "request_permissions",
    "configure_session",
    "release_session",
    "set_keep_screen_on",
    "current_route",
    "start_foreground_service",
    "stop_foreground_service",
    "exclude_from_backup",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
