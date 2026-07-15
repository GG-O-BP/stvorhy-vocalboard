//! 플러그인 IPC 페이로드. Kotlin/Swift 구현과 필드명(camelCase)을 동기 유지.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionState {
    Granted,
    Denied,
    Prompt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    pub microphone: PermissionState,
}

/// 오디오 세션 구성 요청 (iOS AVAudioSession / Android 오디오 포커스).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    /// 반주 재생 + 캡처 동시 모드 여부 (iOS .playAndRecord는 항상 사용,
    /// Android 포커스 종류 결정에 사용).
    pub playback: bool,
}

/// 현재 출력 라우트 요약.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteInfo {
    /// 헤드폰(유선/BT) 경유 여부. 판별 불가(데스크톱)는 None.
    pub headphones: Option<bool>,
    /// 사람이 읽을 라우트 설명 (예: "wired-headphones", "speaker").
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepScreenOnRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcludeFromBackupRequest {
    /// 백업 제외로 표시할 절대 경로 목록 (iOS: NSURLIsExcludedFromBackupKey.
    /// Android: 앱 매니페스트 backup rules로 처리 — 구현은 no-op 성공).
    pub paths: Vec<String>,
}
