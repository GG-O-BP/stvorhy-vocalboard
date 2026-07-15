//! 데스크톱(개발·검증용) 스텁 구현.
//!
//! 권한은 항상 granted, 라우트는 판별 불가(None) — UI는 이 경우 사용자
//! 확인(헤드폰 사용 체크)을 요구한다 (스피커 에코 오염 가드레일).

use serde::de::DeserializeOwned;
use tauri::plugin::PluginApi;
use tauri::{AppHandle, Runtime};

use crate::models::*;
use crate::Result;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<VocalAudio<R>> {
    Ok(VocalAudio(app.clone()))
}

pub struct VocalAudio<R: Runtime>(AppHandle<R>);

impl<R: Runtime> VocalAudio<R> {
    pub fn check_permissions(&self) -> Result<PermissionStatus> {
        Ok(PermissionStatus {
            microphone: PermissionState::Granted,
        })
    }

    pub fn request_permissions(&self) -> Result<PermissionStatus> {
        self.check_permissions()
    }

    pub fn configure_session(&self, _config: SessionConfig) -> Result<()> {
        Ok(())
    }

    pub fn release_session(&self) -> Result<()> {
        Ok(())
    }

    pub fn set_keep_screen_on(&self, _enabled: bool) -> Result<()> {
        Ok(())
    }

    pub fn current_route(&self) -> Result<RouteInfo> {
        Ok(RouteInfo {
            headphones: None,
            description: "desktop-unknown".into(),
        })
    }

    pub fn start_foreground_service(&self) -> Result<()> {
        Ok(())
    }

    pub fn stop_foreground_service(&self) -> Result<()> {
        Ok(())
    }

    pub fn exclude_from_backup(&self, _paths: Vec<String>) -> Result<()> {
        Ok(())
    }
}
