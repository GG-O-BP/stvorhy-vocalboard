use tauri::{command, AppHandle, Runtime};

use crate::models::*;
use crate::Result;
use crate::VocalAudioExt;

#[command]
pub(crate) async fn check_permissions<R: Runtime>(app: AppHandle<R>) -> Result<PermissionStatus> {
    app.vocal_audio().check_permissions()
}

#[command]
pub(crate) async fn request_permissions<R: Runtime>(app: AppHandle<R>) -> Result<PermissionStatus> {
    app.vocal_audio().request_permissions()
}

#[command]
pub(crate) async fn configure_session<R: Runtime>(
    app: AppHandle<R>,
    config: SessionConfig,
) -> Result<()> {
    app.vocal_audio().configure_session(config)
}

#[command]
pub(crate) async fn release_session<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    app.vocal_audio().release_session()
}

#[command]
pub(crate) async fn set_keep_screen_on<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<()> {
    app.vocal_audio().set_keep_screen_on(enabled)
}

#[command]
pub(crate) async fn current_route<R: Runtime>(app: AppHandle<R>) -> Result<RouteInfo> {
    app.vocal_audio().current_route()
}

#[command]
pub(crate) async fn start_foreground_service<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    app.vocal_audio().start_foreground_service()
}

#[command]
pub(crate) async fn stop_foreground_service<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    app.vocal_audio().stop_foreground_service()
}

#[command]
pub(crate) async fn exclude_from_backup<R: Runtime>(
    app: AppHandle<R>,
    paths: Vec<String>,
) -> Result<()> {
    app.vocal_audio().exclude_from_backup(paths)
}
