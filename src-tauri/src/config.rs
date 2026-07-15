//! 앱 설정 (스펙 §3: tauri-plugin-store + `configure` 커맨드로 백엔드 동기화).
//!
//! 프론트가 store(settings.json)의 `config` 키에 전체 객체를 저장하고,
//! 변경 시 `configure`로 백엔드에 밀어넣는다. 백엔드는 시작 시 store에서
//! 초기값을 읽는다. 키 계약은 src/lib/settings.js 와 동기 유지.

use serde::{Deserialize, Serialize};
use vocalboard_dsp::PipelineParams;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// RMS 게이트 임계 dBFS (§4 기본 -45).
    pub gate_dbfs: f32,
    /// confidence 임계 (§4 기본 0.9).
    pub conf_threshold: f32,
    /// 세션 중 마이크 드라이 WAV 녹음 (Phase 2).
    pub recording_enabled: bool,
    /// 녹음 보존 개수 (Phase 2, 최근 N개).
    pub recording_keep_last: u32,
    /// 채점 옥타브 불변 모드 (Phase 3.5).
    pub octave_invariant: bool,
    /// 동기 재생 지연 캘리브레이션 ms (Phase 3.5).
    pub latency_calib_ms: i32,
    /// 보컬 분리 품질 모드 (false=고속 MDX, true=품질 HTDemucs, Phase 3.5).
    pub separation_quality: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            gate_dbfs: -45.0,
            conf_threshold: 0.9,
            recording_enabled: true,
            recording_keep_last: 20,
            octave_invariant: false,
            latency_calib_ms: 0,
            separation_quality: false,
        }
    }
}

impl AppConfig {
    pub fn pipeline_params(&self) -> PipelineParams {
        PipelineParams {
            conf_threshold: self.conf_threshold,
            gate_dbfs: self.gate_dbfs,
            ..PipelineParams::default()
        }
    }

    /// 값 범위 정합 (프론트 버그/수동 편집 방어).
    pub fn sanitized(mut self) -> Self {
        self.gate_dbfs = self.gate_dbfs.clamp(-96.0, 0.0);
        self.conf_threshold = self.conf_threshold.clamp(0.0, 1.0);
        self.recording_keep_last = self.recording_keep_last.clamp(1, 1000);
        self.latency_calib_ms = self.latency_calib_ms.clamp(-500, 500);
        self
    }
}
