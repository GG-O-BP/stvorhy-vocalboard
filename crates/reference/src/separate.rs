//! 보컬 분리 (스펙 §3: 기본 = MDX-Net 계열 경량 / 품질 = HTDemucs FT ONNX,
//! 곡당 1회 캐시).
//!
//! 두 그래프 계열을 모델 trait 뒤에 둔다:
//! - [`WaveModel`]: 파형 in/out (HTDemucs ONNX — STFT가 그래프 내부).
//!   세그먼트 + 선형 크로스페이드 overlap-add.
//! - [`SpectroModel`]: 스펙트로그램 in/out (UVR MDX: n_fft 7680, hop 1024,
//!   dim_f 3072, dim_t 256, 채널 [L_re, L_im, R_re, R_im]). STFT/iSTFT는
//!   여기서 수행(hann periodic, center 반사 패딩), 청크 hann 가중
//!   overlap-add.
//!
//! 실모델 없는 환경을 위해 Identity/Stub 구현이 파이프라인 수학을
//! 검증한다 (identity 모델 ⇒ 입력 복원).

use realfft::num_complex::Complex;
use realfft::RealFftPlanner;

use crate::download::{RemoteModel, HTDEMUCS_VOCALS, MDX_VOC_FT};
use crate::ReferenceError;

/// 분리 파이프라인 표준 샘플레이트.
pub const SEP_SR: u32 = 44_100;

#[derive(Debug, Clone, Default)]
pub struct StereoPcm {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl StereoPcm {
    pub fn len(&self) -> usize {
        self.left.len().min(self.right.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone)]
pub struct SeparatedStems {
    pub vocals: StereoPcm,
    pub accompaniment: StereoPcm,
}

/// 분리 모드 (설정 separation_quality).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SepMode {
    /// MDX Voc_FT (경량, 기본).
    Fast,
    /// HTDemucs FT vocals (품질).
    Quality,
}

impl SepMode {
    pub fn remote_model(&self) -> &'static RemoteModel {
        match self {
            SepMode::Fast => &MDX_VOC_FT,
            SepMode::Quality => &HTDEMUCS_VOCALS,
        }
    }

    /// tracks.sep_model 에 기록하는 식별자.
    pub fn id(&self) -> &'static str {
        match self {
            SepMode::Fast => "mdx:UVR-MDX-NET-Voc_FT",
            SepMode::Quality => "onnx:htdemucs_ft_vocals",
        }
    }
}

pub trait SeparationEngine: Send {
    fn separate(
        &mut self,
        mix: &StereoPcm,
        progress: &mut dyn FnMut(f32),
    ) -> Result<SeparatedStems, ReferenceError>;
}

// ─────────────────────────── 파형 경로 (HTDemucs) ───────────────────────────

/// 파형 세그먼트 → 보컬 파형 그래프.
pub trait WaveModel: Send {
    fn segment_len(&self) -> usize;
    /// (left, right) 각각 정확히 segment_len 길이.
    fn run(&mut self, left: &[f32], right: &[f32]) -> Result<(Vec<f32>, Vec<f32>), ReferenceError>;
}

/// 세그먼트 + 선형 크로스페이드 overlap-add (bag_infer.py 방식).
pub struct WaveformSeparator<M: WaveModel> {
    pub model: M,
    /// 세그먼트 겹침 비율 (기본 0.25).
    pub overlap: f32,
}

impl<M: WaveModel> WaveformSeparator<M> {
    pub fn new(model: M) -> Self {
        Self { model, overlap: 0.25 }
    }
}

impl<M: WaveModel> SeparationEngine for WaveformSeparator<M> {
    fn separate(
        &mut self,
        mix: &StereoPcm,
        progress: &mut dyn FnMut(f32),
    ) -> Result<SeparatedStems, ReferenceError> {
        let n = mix.len();
        let seg = self.model.segment_len();
        let step = ((seg as f32) * (1.0 - self.overlap)) as usize;
        let step = step.max(1).min(seg);

        let mut voc_l = vec![0.0f32; n];
        let mut voc_r = vec![0.0f32; n];
        let mut weight = vec![0.0f32; n];

        let mut pos = 0usize;
        let total_chunks = n.div_ceil(step).max(1);
        let mut done_chunks = 0usize;
        loop {
            let end = (pos + seg).min(n);
            let mut l = vec![0.0f32; seg];
            let mut r = vec![0.0f32; seg];
            l[..end - pos].copy_from_slice(&mix.left[pos..end]);
            r[..end - pos].copy_from_slice(&mix.right[pos..end]);
            let (vl, vr) = self.model.run(&l, &r)?;

            // 선형 페이드 (경계 트랜지션 = seg*overlap).
            let transition = ((seg as f32) * self.overlap) as usize;
            for i in 0..(end - pos) {
                let mut w = 1.0f32;
                if transition > 0 {
                    if i < transition {
                        w = w.min((i + 1) as f32 / transition as f32);
                    }
                    if seg - i <= transition {
                        w = w.min((seg - i) as f32 / transition as f32);
                    }
                }
                voc_l[pos + i] += vl[i] * w;
                voc_r[pos + i] += vr[i] * w;
                weight[pos + i] += w;
            }
            done_chunks += 1;
            progress((done_chunks as f32 / total_chunks as f32).min(1.0));
            if end == n {
                break;
            }
            pos += step;
        }
        for i in 0..n {
            if weight[i] > 1e-9 {
                voc_l[i] /= weight[i];
                voc_r[i] /= weight[i];
            }
        }
        let accomp = StereoPcm {
            left: mix.left.iter().zip(&voc_l).map(|(m, v)| m - v).collect(),
            right: mix.right.iter().zip(&voc_r).map(|(m, v)| m - v).collect(),
        };
        Ok(SeparatedStems {
            vocals: StereoPcm { left: voc_l, right: voc_r },
            accompaniment: accomp,
        })
    }
}

/// HTDemucs FT vocals ONNX: 입력 "mix" [1,2,343980] → "stems" [1,4,2,343980]
/// (vocals = 소스 인덱스 3). 반주 = mix − vocals.
pub struct OrtWaveModel {
    session: ort::session::Session,
    segment: usize,
    input_name: String,
}

pub const DEMUCS_SEGMENT: usize = 343_980;
const DEMUCS_VOCALS_INDEX: usize = 3;

impl OrtWaveModel {
    pub fn from_file(path: &std::path::Path) -> Result<Self, ReferenceError> {
        let session = ort::session::Session::builder()
            .and_then(|mut b| Ok(b.commit_from_file(path)?))
            .map_err(|e: ort::Error| ReferenceError::Inference(e.to_string()))?;
        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| ReferenceError::Inference("no inputs".into()))?;
        Ok(Self { session, segment: DEMUCS_SEGMENT, input_name })
    }
}

impl WaveModel for OrtWaveModel {
    fn segment_len(&self) -> usize {
        self.segment
    }

    fn run(&mut self, left: &[f32], right: &[f32]) -> Result<(Vec<f32>, Vec<f32>), ReferenceError> {
        let n = self.segment;
        let mut data = Vec::with_capacity(2 * n);
        data.extend_from_slice(left);
        data.extend_from_slice(right);
        let tensor = ort::value::Tensor::from_array(([1usize, 2, n], data))
            .map_err(|e| ReferenceError::Inference(e.to_string()))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])
            .map_err(|e| ReferenceError::Inference(e.to_string()))?;
        let (shape, stems) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| ReferenceError::Inference(e.to_string()))?;
        let dims: Vec<i64> = shape.iter().copied().collect();
        if dims.len() != 4 || dims[2] != 2 || dims[3] as usize != n {
            return Err(ReferenceError::Inference(format!(
                "예상 밖 stems shape: {dims:?}"
            )));
        }
        let sources = dims[1] as usize;
        let src = DEMUCS_VOCALS_INDEX.min(sources - 1);
        let base = src * 2 * n;
        let vl = stems[base..base + n].to_vec();
        let vr = stems[base + n..base + 2 * n].to_vec();
        Ok((vl, vr))
    }
}

// ──────────────────────── 스펙트로그램 경로 (MDX) ────────────────────────

pub const MDX_N_FFT: usize = 7680;
pub const MDX_HOP: usize = 1024;
pub const MDX_DIM_F: usize = 3072;
pub const MDX_DIM_T: usize = 256;
/// 청크 원샘플 수: hop × (dim_t − 1).
pub const MDX_CHUNK: usize = MDX_HOP * (MDX_DIM_T - 1);
pub const MDX_COMPENSATE: f32 = 1.021;

/// [4][dim_f][dim_t] 스펙 → 같은 shape 보컬 스펙 추정 그래프.
pub trait SpectroModel: Send {
    fn run(&mut self, spec: &[f32]) -> Result<Vec<f32>, ReferenceError>;
}

/// UVR MDX ONNX ("input" [1,4,3072,256]).
pub struct OrtSpectroModel {
    session: ort::session::Session,
    input_name: String,
}

impl OrtSpectroModel {
    pub fn from_file(path: &std::path::Path) -> Result<Self, ReferenceError> {
        let session = ort::session::Session::builder()
            .and_then(|mut b| Ok(b.commit_from_file(path)?))
            .map_err(|e: ort::Error| ReferenceError::Inference(e.to_string()))?;
        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| ReferenceError::Inference("no inputs".into()))?;
        Ok(Self { session, input_name })
    }
}

impl SpectroModel for OrtSpectroModel {
    fn run(&mut self, spec: &[f32]) -> Result<Vec<f32>, ReferenceError> {
        let tensor =
            ort::value::Tensor::from_array(([1usize, 4, MDX_DIM_F, MDX_DIM_T], spec.to_vec()))
                .map_err(|e| ReferenceError::Inference(e.to_string()))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])
            .map_err(|e| ReferenceError::Inference(e.to_string()))?;
        let (_, out) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| ReferenceError::Inference(e.to_string()))?;
        Ok(out.to_vec())
    }
}

/// hann periodic 윈도.
fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos())
        .collect()
}

/// MDX STFT/iSTFT (torch.stft center=true 등가: n_fft/2 반사 패딩,
/// 프레임 = 1 + len/hop).
struct MdxStft {
    window: Vec<f32>,
    fft: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    ifft: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
}

impl MdxStft {
    fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        Self {
            window: hann(MDX_N_FFT),
            fft: planner.plan_fft_forward(MDX_N_FFT),
            ifft: planner.plan_fft_inverse(MDX_N_FFT),
        }
    }

    /// 채널 파형(chunk 길이) → 해당 채널의 (re, im) [dim_f][dim_t].
    fn forward_channel(&self, x: &[f32], re: &mut [f32], im: &mut [f32]) {
        let pad = MDX_N_FFT / 2;
        // 반사 패딩.
        let padded_len = x.len() + 2 * pad;
        let mut padded = vec![0.0f32; padded_len];
        padded[pad..pad + x.len()].copy_from_slice(x);
        for i in 0..pad {
            padded[pad - 1 - i] = x[(i + 1).min(x.len() - 1)];
            padded[pad + x.len() + i] = x[x.len() - 2 - i.min(x.len() - 2)];
        }
        let mut frame = vec![0.0f32; MDX_N_FFT];
        let mut spectrum = vec![Complex::default(); MDX_N_FFT / 2 + 1];
        for t in 0..MDX_DIM_T {
            let start = t * MDX_HOP;
            for i in 0..MDX_N_FFT {
                frame[i] = padded[start + i] * self.window[i];
            }
            self.fft.process(&mut frame, &mut spectrum).expect("fft");
            for f in 0..MDX_DIM_F {
                re[f * MDX_DIM_T + t] = spectrum[f].re;
                im[f * MDX_DIM_T + t] = spectrum[f].im;
            }
        }
    }

    /// (re, im) [dim_f][dim_t] → 채널 파형(chunk 길이). 윈도² OLA 정규화.
    fn inverse_channel(&self, re: &[f32], im: &[f32], out_len: usize) -> Vec<f32> {
        let pad = MDX_N_FFT / 2;
        let full = out_len + 2 * pad;
        let mut acc = vec![0.0f32; full];
        let mut norm = vec![0.0f32; full];
        let mut spectrum = vec![Complex::default(); MDX_N_FFT / 2 + 1];
        let mut frame = vec![0.0f32; MDX_N_FFT];
        for t in 0..MDX_DIM_T {
            for f in 0..(MDX_N_FFT / 2 + 1) {
                if f < MDX_DIM_F {
                    spectrum[f] = Complex::new(re[f * MDX_DIM_T + t], im[f * MDX_DIM_T + t]);
                } else {
                    spectrum[f] = Complex::default();
                }
            }
            // realfft 역변환 요건: DC/나이퀴스트 빈은 실수여야 한다.
            // 모델 출력 추정치는 이를 보장하지 않으므로 강제한다.
            spectrum[0].im = 0.0;
            spectrum[MDX_N_FFT / 2].im = 0.0;
            self.ifft.process(&mut spectrum, &mut frame).expect("ifft");
            let start = t * MDX_HOP;
            let scale = 1.0 / MDX_N_FFT as f32; // realfft 비정규화 보정
            for i in 0..MDX_N_FFT {
                acc[start + i] += frame[i] * scale * self.window[i];
                norm[start + i] += self.window[i] * self.window[i];
            }
        }
        (0..out_len)
            .map(|i| {
                let j = i + pad;
                if norm[j] > 1e-8 {
                    acc[j] / norm[j]
                } else {
                    0.0
                }
            })
            .collect()
    }
}

/// MDX 분리기: 청크 STFT → 그래프 → iSTFT → hann 가중 overlap-add.
pub struct MdxSeparator<M: SpectroModel> {
    pub model: M,
    pub compensate: f32,
    /// 청크 겹침 비율 (기본 0.25).
    pub overlap: f32,
    stft: MdxStft,
}

impl<M: SpectroModel> MdxSeparator<M> {
    pub fn new(model: M) -> Self {
        Self { model, compensate: MDX_COMPENSATE, overlap: 0.25, stft: MdxStft::new() }
    }

    fn process_chunk(
        &mut self,
        l: &[f32],
        r: &[f32],
    ) -> Result<(Vec<f32>, Vec<f32>), ReferenceError> {
        // 스펙 조립: [4][F][T] = [L_re, L_im, R_re, R_im].
        let plane = MDX_DIM_F * MDX_DIM_T;
        let mut spec = vec![0.0f32; 4 * plane];
        {
            let (a, rest) = spec.split_at_mut(plane);
            let (b, rest) = rest.split_at_mut(plane);
            let (c, d) = rest.split_at_mut(plane);
            self.stft.forward_channel(l, a, b);
            self.stft.forward_channel(r, c, d);
        }
        // 저역 3빈 제로 (audio-separator 관행).
        for ch in 0..4 {
            for f in 0..3 {
                for t in 0..MDX_DIM_T {
                    spec[ch * plane + f * MDX_DIM_T + t] = 0.0;
                }
            }
        }
        let est = self.model.run(&spec)?;
        if est.len() != spec.len() {
            return Err(ReferenceError::Inference(format!(
                "예상 밖 출력 크기 {} != {}",
                est.len(),
                spec.len()
            )));
        }
        let vl = self
            .stft
            .inverse_channel(&est[0..plane], &est[plane..2 * plane], MDX_CHUNK);
        let vr = self
            .stft
            .inverse_channel(&est[2 * plane..3 * plane], &est[3 * plane..4 * plane], MDX_CHUNK);
        Ok((vl, vr))
    }
}

impl<M: SpectroModel> SeparationEngine for MdxSeparator<M> {
    fn separate(
        &mut self,
        mix: &StereoPcm,
        progress: &mut dyn FnMut(f32),
    ) -> Result<SeparatedStems, ReferenceError> {
        let n = mix.len();
        let step = ((MDX_CHUNK as f32) * (1.0 - self.overlap)) as usize;
        let step = step.max(MDX_HOP);
        let window = hann(MDX_CHUNK);

        let mut voc_l = vec![0.0f32; n];
        let mut voc_r = vec![0.0f32; n];
        let mut weight = vec![0.0f32; n];

        let mut pos = 0usize;
        let total_chunks = n.div_ceil(step).max(1);
        let mut done = 0usize;
        loop {
            let end = (pos + MDX_CHUNK).min(n);
            let mut l = vec![0.0f32; MDX_CHUNK];
            let mut r = vec![0.0f32; MDX_CHUNK];
            l[..end - pos].copy_from_slice(&mix.left[pos..end]);
            r[..end - pos].copy_from_slice(&mix.right[pos..end]);
            let (vl, vr) = self.process_chunk(&l, &r)?;
            for i in 0..(end - pos) {
                // 단일 청크(전체가 한 청크에 들어오는 짧은 입력)는 평평한
                // 가중이어야 정규화가 성립한다.
                let w = if total_chunks == 1 { 1.0 } else { window[i].max(1e-3) };
                voc_l[pos + i] += vl[i] * w;
                voc_r[pos + i] += vr[i] * w;
                weight[pos + i] += w;
            }
            done += 1;
            progress((done as f32 / total_chunks as f32).min(1.0));
            if end == n {
                break;
            }
            pos += step;
        }
        for i in 0..n {
            if weight[i] > 1e-9 {
                voc_l[i] /= weight[i];
                voc_r[i] /= weight[i];
            }
        }
        let c = self.compensate;
        let accomp = StereoPcm {
            left: mix.left.iter().zip(&voc_l).map(|(m, v)| m - v * c).collect(),
            right: mix.right.iter().zip(&voc_r).map(|(m, v)| m - v * c).collect(),
        };
        Ok(SeparatedStems {
            vocals: StereoPcm { left: voc_l, right: voc_r },
            accompaniment: accomp,
        })
    }
}

// ─────────────────────────────── 스텁 ───────────────────────────────

/// 결정적 스텁: vocals = mix×0.5, accomp = mix×0.5.
/// 모델 없는 환경의 파이프라인/캐시/저장 검증용.
pub struct StubSeparationEngine;

pub const STUB_SEP_ID: &str = "stub:half";

impl SeparationEngine for StubSeparationEngine {
    fn separate(
        &mut self,
        mix: &StereoPcm,
        progress: &mut dyn FnMut(f32),
    ) -> Result<SeparatedStems, ReferenceError> {
        progress(1.0);
        let half = |v: &Vec<f32>| v.iter().map(|s| s * 0.5).collect::<Vec<_>>();
        Ok(SeparatedStems {
            vocals: StereoPcm { left: half(&mix.left), right: half(&mix.right) },
            accompaniment: StereoPcm { left: half(&mix.left), right: half(&mix.right) },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, freq: f64) -> Vec<f32> {
        let w = 2.0 * std::f64::consts::PI * freq / SEP_SR as f64;
        (0..n)
            .map(|i| (0.4 * ((w * i as f64) % (2.0 * std::f64::consts::PI)).sin()) as f32)
            .collect()
    }

    #[test]
    fn stft_istft_roundtrip() {
        let stft = MdxStft::new();
        let x = tone(MDX_CHUNK, 441.0);
        let plane = MDX_DIM_F * MDX_DIM_T;
        let mut re = vec![0.0f32; plane];
        let mut im = vec![0.0f32; plane];
        stft.forward_channel(&x, &mut re, &mut im);
        let y = stft.inverse_channel(&re, &im, MDX_CHUNK);
        assert_eq!(y.len(), x.len());
        // dim_f(3072) 컷은 7.9kHz 초과 성분 제거와 같다 — 441Hz 톤은 보존.
        let lo = MDX_N_FFT;
        let hi = MDX_CHUNK - MDX_N_FFT;
        for i in (lo..hi).step_by(1009) {
            assert!((y[i] - x[i]).abs() < 1e-3, "i={i}: {} vs {}", y[i], x[i]);
        }
    }

    struct IdentitySpectro;
    impl SpectroModel for IdentitySpectro {
        fn run(&mut self, spec: &[f32]) -> Result<Vec<f32>, ReferenceError> {
            Ok(spec.to_vec())
        }
    }

    #[test]
    fn mdx_identity_model_reconstructs_mix() {
        let n = MDX_CHUNK + MDX_CHUNK / 2;
        let mix = StereoPcm { left: tone(n, 330.0), right: tone(n, 550.0) };
        let mut sep = MdxSeparator::new(IdentitySpectro);
        let mut last = 0.0;
        let stems = sep.separate(&mix, &mut |p| last = p).unwrap();
        assert!((last - 1.0).abs() < 1e-6);
        // identity 모델 ⇒ vocals ≈ mix (경계 제외).
        let lo = MDX_N_FFT;
        let hi = n - MDX_N_FFT;
        let mut max_err = 0.0f32;
        for i in lo..hi {
            max_err = max_err.max((stems.vocals.left[i] - mix.left[i]).abs());
            max_err = max_err.max((stems.vocals.right[i] - mix.right[i]).abs());
        }
        assert!(max_err < 5e-3, "max_err {max_err}");
        // accomp = mix − vocals×1.021 ≈ −0.021×mix.
        let i = n / 2;
        assert!((stems.accompaniment.left[i] + 0.021 * mix.left[i]).abs() < 5e-3);
    }

    /// 모델 출력이 DC 빈 허수부를 더럽혀도 iSTFT가 죽지 않아야 한다
    /// (실모델 회귀: realfft는 DC/나이퀴스트 허수부 0을 요구).
    struct DirtyDcSpectro;
    impl SpectroModel for DirtyDcSpectro {
        fn run(&mut self, spec: &[f32]) -> Result<Vec<f32>, ReferenceError> {
            let mut out = spec.to_vec();
            let plane = MDX_DIM_F * MDX_DIM_T;
            for ch in [1usize, 3] {
                // im 채널의 f=0 행을 오염.
                for t in 0..MDX_DIM_T {
                    out[ch * plane + t] = 0.37;
                }
            }
            Ok(out)
        }
    }

    #[test]
    fn mdx_tolerates_dirty_dc_bin() {
        let n = MDX_CHUNK;
        let mix = StereoPcm { left: tone(n, 440.0), right: tone(n, 440.0) };
        let mut sep = MdxSeparator::new(DirtyDcSpectro);
        let stems = sep.separate(&mix, &mut |_| {}).unwrap();
        assert_eq!(stems.vocals.left.len(), n);
        assert!(stems.vocals.left.iter().all(|v| v.is_finite()));
    }

    struct IdentityWave(usize);
    impl WaveModel for IdentityWave {
        fn segment_len(&self) -> usize {
            self.0
        }
        fn run(&mut self, l: &[f32], r: &[f32]) -> Result<(Vec<f32>, Vec<f32>), ReferenceError> {
            Ok((l.to_vec(), r.to_vec()))
        }
    }

    #[test]
    fn waveform_identity_model_reconstructs_mix() {
        let n = 50_000;
        let mix = StereoPcm { left: tone(n, 220.0), right: tone(n, 440.0) };
        let mut sep = WaveformSeparator::new(IdentityWave(16_384));
        let stems = sep.separate(&mix, &mut |_| {}).unwrap();
        for i in (0..n).step_by(487) {
            assert!(
                (stems.vocals.left[i] - mix.left[i]).abs() < 1e-4,
                "i={i}"
            );
            assert!(stems.accompaniment.left[i].abs() < 1e-4);
        }
    }

    #[test]
    fn stub_halves_mix() {
        let mix = StereoPcm { left: tone(1000, 440.0), right: tone(1000, 440.0) };
        let stems = StubSeparationEngine.separate(&mix, &mut |_| {}).unwrap();
        assert!((stems.vocals.left[500] - mix.left[500] * 0.5).abs() < 1e-7);
        assert!((stems.accompaniment.left[500] - mix.left[500] * 0.5).abs() < 1e-7);
    }
}
