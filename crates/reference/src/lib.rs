//! 레퍼런스 트랙 파이프라인 (스펙 §1·§3, Phase 3.5):
//! 임포트 → 디코드 → 보컬 분리(이중 모드) → SwiftF0 추출 → 후처리 →
//! 노트 세그멘테이션. 모델은 온디맨드 다운로드(재개+체크섬).

use thiserror::Error;

pub mod decode;
pub mod download;
pub mod extract;
pub mod flac;
pub mod separate;

#[derive(Debug, Error)]
pub enum ReferenceError {
    /// 사용자에게 그대로 보여줄 안내 오류 (HE-AAC/DRM 등).
    #[error("{0}")]
    User(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("dsp: {0}")]
    Dsp(#[from] vocalboard_dsp::DspError),
    #[error("codec: {0}")]
    Codec(#[from] vocalboard_codec::CodecError),
    #[error("inference: {0}")]
    Inference(String),
    #[error("download: {0}")]
    Download(String),
    #[error("{0}")]
    Other(String),
}
