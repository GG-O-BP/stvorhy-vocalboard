//! SwiftF0 ONNX 추론 (ort 구현체).
//!
//! 모델 계약 (lars76/swift-f0, MIT): 입력 `input_audio` f32 (1, N) @16kHz,
//! N ≥ 256. STFT(frame 1024, hop 256, 대칭 패딩 384)는 그래프 내부에 있어
//! 전처리가 없다. 출력 0 = `pitch_hz` (1, N/256), 출력 1 = `confidence`.
//! 텐서 이름은 세션 메타데이터에서 동적으로 읽는다.

use std::path::Path;

use ort::session::Session;
use ort::value::Tensor;

use crate::engine::{InferenceEngine, PitchEstimate};
use crate::{DspError, HOP};

/// 다운로드 매니저가 검증하는 공식 모델 사본의 SHA-256.
pub const SWIFTF0_SHA256: &str = "7e2390db8379cd9e1e2b22828e55b45b57c8559e4c8335678c717dc245c18176";
pub const SWIFTF0_URL: &str =
    "https://raw.githubusercontent.com/lars76/swift-f0/main/swift_f0/model.onnx";

pub struct OrtEngine {
    session: Session,
    input_name: String,
    pitch_output: String,
    conf_output: String,
}

impl OrtEngine {
    /// intra-thread 1 + (피처 주입 시) 모바일 EP까지 구성한 SessionBuilder.
    /// 미지원 그래프는 ORT가 CPU로 폴백한다.
    fn configured_builder() -> Result<ort::session::builder::SessionBuilder, ort::Error> {
        Session::builder()
            .and_then(|b| Ok(b.with_intra_threads(1)?))
            .and_then(|b| {
                #[allow(unused_mut)]
                let mut eps: Vec<ort::ep::ExecutionProviderDispatch> = Vec::new();
                #[cfg(feature = "ep-coreml")]
                eps.push(ort::ep::CoreML::default().build());
                #[cfg(feature = "ep-nnapi")]
                eps.push(ort::ep::NNAPI::default().build());
                #[cfg(feature = "ep-xnnpack")]
                eps.push(ort::ep::XNNPACK::default().build());
                if eps.is_empty() {
                    Ok(b)
                } else {
                    Ok(b.with_execution_providers(eps)?)
                }
            })
    }

    /// commit된 Session에서 입출력 텐서 이름을 확정한다.
    fn from_session(session: Session) -> Result<Self, DspError> {
        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| DspError::Inference("model has no inputs".into()))?;
        let out_names: Vec<String> = session.outputs().iter().map(|o| o.name().to_string()).collect();
        if out_names.len() < 2 {
            return Err(DspError::Inference(format!(
                "expected 2 outputs (pitch, confidence), got {out_names:?}"
            )));
        }
        // 그래프 순서상 0=pitch, 1=confidence. 이름에 "conf"가 있으면 그것을 신뢰.
        let (pitch_output, conf_output) = if out_names[0].to_lowercase().contains("conf") {
            (out_names[1].clone(), out_names[0].clone())
        } else {
            (out_names[0].clone(), out_names[1].clone())
        };
        Ok(Self {
            session,
            input_name,
            pitch_output,
            conf_output,
        })
    }

    /// 파일 경로에서 로드 (데스크톱 dev·명시 오버라이드용).
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, DspError> {
        let mut builder =
            Self::configured_builder().map_err(|e: ort::Error| DspError::Inference(e.to_string()))?;
        let session = builder
            .commit_from_file(path.as_ref())
            .map_err(|e: ort::Error| DspError::Inference(e.to_string()))?;
        Self::from_session(session)
    }

    /// 메모리(바이트 슬라이스)에서 로드 — 앱에 임베드된 모델용(build.rs + include_bytes!).
    pub fn from_memory(model: &[u8]) -> Result<Self, DspError> {
        let mut builder =
            Self::configured_builder().map_err(|e: ort::Error| DspError::Inference(e.to_string()))?;
        let session = builder
            .commit_from_memory(model)
            .map_err(|e: ort::Error| DspError::Inference(e.to_string()))?;
        Self::from_session(session)
    }
}

impl InferenceEngine for OrtEngine {
    fn infer(&mut self, audio_16k: &[f32]) -> Result<Vec<PitchEstimate>, DspError> {
        let expected = audio_16k.len() / HOP;
        if expected == 0 {
            return Ok(Vec::new());
        }
        let tensor = Tensor::from_array(([1usize, audio_16k.len()], audio_16k.to_vec()))
            .map_err(|e| DspError::Inference(e.to_string()))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])
            .map_err(|e| DspError::Inference(e.to_string()))?;
        let (_, pitch) = outputs[self.pitch_output.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(|e| DspError::Inference(e.to_string()))?;
        let (_, conf) = outputs[self.conf_output.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(|e| DspError::Inference(e.to_string()))?;
        let n = pitch.len().min(conf.len()).min(expected);
        Ok((0..n)
            .map(|i| PitchEstimate {
                f0: pitch[i],
                confidence: conf[i].clamp(0.0, 1.0),
            })
            .collect())
    }
}
