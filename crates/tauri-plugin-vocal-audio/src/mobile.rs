//! 모바일 구현: Kotlin/Swift 플러그인으로 위임.

use serde::de::DeserializeOwned;
use serde::Serialize;
use tauri::plugin::{PluginApi, PluginHandle};
use tauri::{AppHandle, Runtime};

use crate::models::*;
use crate::Result;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_vocal_audio);

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<VocalAudio<R>> {
    #[cfg(target_os = "android")]
    let handle = api
        .register_android_plugin("app.vocalboard.plugin", "VocalAudioPlugin")
        .map_err(|e| crate::Error::Other(e.to_string()))?;
    #[cfg(target_os = "ios")]
    let handle = api
        .register_ios_plugin(init_plugin_vocal_audio)
        .map_err(|e| crate::Error::Other(e.to_string()))?;
    Ok(VocalAudio(handle))
}

pub struct VocalAudio<R: Runtime>(PluginHandle<R>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeepScreenOnArgs {
    enabled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExcludeArgs {
    paths: Vec<String>,
}

impl<R: Runtime> VocalAudio<R> {
    pub fn check_permissions(&self) -> Result<PermissionStatus> {
        Ok(self.0.run_mobile_plugin("checkPermissions", ())?)
    }

    pub fn request_permissions(&self) -> Result<PermissionStatus> {
        Ok(self
            .0
            .run_mobile_plugin("requestPermissions", serde_json::json!({ "permissions": ["microphone"] }))?)
    }

    pub fn configure_session(&self, config: SessionConfig) -> Result<()> {
        Ok(self.0.run_mobile_plugin("configureSession", config)?)
    }

    pub fn release_session(&self) -> Result<()> {
        Ok(self.0.run_mobile_plugin("releaseSession", ())?)
    }

    pub fn set_keep_screen_on(&self, enabled: bool) -> Result<()> {
        Ok(self
            .0
            .run_mobile_plugin("setKeepScreenOn", KeepScreenOnArgs { enabled })?)
    }

    pub fn current_route(&self) -> Result<RouteInfo> {
        Ok(self.0.run_mobile_plugin("currentRoute", ())?)
    }

    pub fn start_foreground_service(&self) -> Result<()> {
        Ok(self.0.run_mobile_plugin("startForegroundService", ())?)
    }

    pub fn stop_foreground_service(&self) -> Result<()> {
        Ok(self.0.run_mobile_plugin("stopForegroundService", ())?)
    }

    pub fn exclude_from_backup(&self, paths: Vec<String>) -> Result<()> {
        Ok(self.0.run_mobile_plugin("excludeFromBackup", ExcludeArgs { paths })?)
    }
}
