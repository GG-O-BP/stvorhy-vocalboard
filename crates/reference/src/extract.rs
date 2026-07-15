//! 레퍼런스 피치 추출 (§8): 보컬 스템 → SwiftF0 청크 추출 → 후처리
//! (confidence·스템 RMS 게이팅, 갭 브리징, 노트 세그멘테이션) → F0C1 프레임.

use serde::Serialize;
use vocalboard_codec::F0Frame;
use vocalboard_dsp::engine::InferenceEngine;
use vocalboard_dsp::gate::rms_dbfs;
use vocalboard_dsp::pitch::f0_to_midi;
use vocalboard_dsp::{ANALYSIS_SR, FMAX, FMIN, HOP};

use crate::ReferenceError;

/// 추출 파라미터 (레퍼런스는 라이브보다 관대한 confidence, 별도 RMS 게이트).
#[derive(Debug, Clone, Copy)]
pub struct ExtractParams {
    /// SwiftF0 confidence 임계 (스템은 깨끗하므로 0.7).
    pub conf_threshold: f32,
    /// 스템 RMS 게이트 dBFS.
    pub gate_dbfs: f32,
    /// 브리징할 최대 무성 갭 (hop 수).
    pub max_bridge_hops: usize,
    /// 브리징 허용 피치 차 (semitone).
    pub bridge_max_semitones: f32,
}

impl Default for ExtractParams {
    fn default() -> Self {
        Self {
            conf_threshold: 0.7,
            gate_dbfs: -50.0,
            max_bridge_hops: 4,
            bridge_max_semitones: 1.0,
        }
    }
}

/// 청크 크기 (hop): 10초. 경계 열화 프레임은 겹침으로 대체한다.
const CHUNK_HOPS: usize = 625;
/// 청크 좌우 컨텍스트/폐기 폭 (hop).
const EDGE_HOPS: usize = 4;

/// 16k mono 보컬 스템에서 hop당 F0C1 프레임을 추출한다.
///
/// 엔진 호출은 [겹침 포함 청크] 단위: 내부 프레임만 채택해 경계 열화를
/// 피한다 (SwiftF0는 좌우 문맥이 짧으면 confidence가 떨어짐 — dsp 실측).
pub fn extract_frames(
    engine: &mut dyn InferenceEngine,
    stem_16k: &[f32],
    params: &ExtractParams,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<F0Frame>, ReferenceError> {
    let n_hops = stem_16k.len() / HOP;
    let mut frames = Vec::with_capacity(n_hops);
    let mut hop_idx = 0usize;

    while hop_idx < n_hops {
        let take = CHUNK_HOPS.min(n_hops - hop_idx);
        // 좌우 EDGE_HOPS 컨텍스트를 붙여 추론.
        let ctx_start_hop = hop_idx.saturating_sub(EDGE_HOPS);
        let ctx_end_hop = (hop_idx + take + EDGE_HOPS).min(n_hops);
        let audio = &stem_16k[ctx_start_hop * HOP..ctx_end_hop * HOP];
        let est = engine
            .infer(audio)
            .map_err(|e| ReferenceError::Inference(e.to_string()))?;
        let offset = hop_idx - ctx_start_hop;

        for k in 0..take {
            let hop_audio = &stem_16k[(hop_idx + k) * HOP..(hop_idx + k + 1) * HOP];
            let rms = rms_dbfs(hop_audio);
            let e = est.get(offset + k).copied().unwrap_or(
                vocalboard_dsp::engine::PitchEstimate { f0: 0.0, confidence: 0.0 },
            );
            let in_range = e.f0.is_finite() && e.f0 >= FMIN && e.f0 <= FMAX;
            let voiced = rms >= params.gate_dbfs && in_range && e.confidence >= params.conf_threshold;
            let midi = if voiced { f0_to_midi(e.f0) } else { 0.0 };
            frames.push(F0Frame::quantize(midi, voiced, e.confidence, rms));
        }
        hop_idx += take;
        progress((hop_idx as f32 / n_hops.max(1) as f32).min(1.0));
    }
    Ok(frames)
}

/// 갭 브리징: 양옆이 voiced이고 피치가 가까우면 짧은 무성 갭을 선형
/// 보간으로 메운다 (자음/breath로 인한 순간 결손 보정).
pub fn bridge_gaps(frames: &mut [F0Frame], params: &ExtractParams) {
    let n = frames.len();
    let mut i = 0usize;
    while i < n {
        if frames[i].voiced() {
            i += 1;
            continue;
        }
        // 무성 런 [i, j).
        let mut j = i;
        while j < n && !frames[j].voiced() {
            j += 1;
        }
        let gap = j - i;
        if i > 0 && j < n && gap <= params.max_bridge_hops {
            let a = frames[i - 1];
            let b = frames[j];
            let da = a.midi();
            let db = b.midi();
            if (da - db).abs() <= params.bridge_max_semitones {
                let conf = a.conf.min(b.conf) as f32 / 255.0;
                for (idx, k) in (i..j).enumerate() {
                    let t = (idx + 1) as f32 / (gap + 1) as f32;
                    let midi = da + (db - da) * t;
                    let rms = frames[k].rms_dbfs();
                    frames[k] = F0Frame::quantize(midi, true, conf, rms);
                }
            }
        }
        i = j;
    }
}

/// 노트 블록 (§8 노트 세그멘테이션 → tracks.notes_json).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Note {
    /// 시작 ms.
    pub s: u32,
    /// 끝 ms (exclusive).
    pub e: u32,
    /// 대표 midi (구간 중앙값).
    pub m: f32,
}

/// 세그멘테이션 파라미터 (swift-f0 저장소 segment_notes 기본값 준용).
#[derive(Debug, Clone, Copy)]
pub struct SegmentParams {
    /// 진행 중 노트 중앙값에서 이 이상 벗어나면 분할 (semitone).
    pub split_semitones: f32,
    /// 최소 노트 길이 (ms).
    pub min_note_ms: u32,
    /// 노트 내 허용 무성 그레이스 (ms).
    pub grace_ms: u32,
}

impl Default for SegmentParams {
    fn default() -> Self {
        Self { split_semitones: 0.8, min_note_ms: 50, grace_ms: 20 }
    }
}

/// voiced 프레임 열 → 노트 블록.
pub fn segment_notes(frames: &[F0Frame], p: &SegmentParams) -> Vec<Note> {
    let hop_ms = vocalboard_codec::HOP_MS as u32;
    let grace_hops = (p.grace_ms / hop_ms) as usize;
    let min_hops = (p.min_note_ms.div_ceil(hop_ms)) as usize;

    let mut notes = Vec::new();
    let mut cur: Vec<(usize, f32)> = Vec::new(); // (hop index, midi)
    let mut gap = 0usize;

    let median = |v: &mut Vec<f32>| -> f32 {
        v.sort_by(|a, b| a.total_cmp(b));
        let m = v.len();
        if m % 2 == 1 { v[m / 2] } else { (v[m / 2 - 1] + v[m / 2]) / 2.0 }
    };

    let flush = |cur: &mut Vec<(usize, f32)>, notes: &mut Vec<Note>| {
        if cur.len() >= min_hops.max(1) {
            let start = cur[0].0;
            let end = cur[cur.len() - 1].0 + 1;
            let mut midis: Vec<f32> = cur.iter().map(|(_, m)| *m).collect();
            notes.push(Note {
                s: start as u32 * hop_ms,
                e: end as u32 * hop_ms,
                m: median(&mut midis),
            });
        }
        cur.clear();
    };

    for (i, f) in frames.iter().enumerate() {
        if f.voiced() {
            let midi = f.midi();
            if !cur.is_empty() {
                let mut midis: Vec<f32> = cur.iter().map(|(_, m)| *m).collect();
                let med = median(&mut midis);
                if (midi - med).abs() > p.split_semitones {
                    flush(&mut cur, &mut notes);
                }
            }
            cur.push((i, midi));
            gap = 0;
        } else if !cur.is_empty() {
            gap += 1;
            if gap > grace_hops {
                flush(&mut cur, &mut notes);
                gap = 0;
            }
        }
    }
    flush(&mut cur, &mut notes);
    notes
}

/// 44.1k 스테레오 보컬 스템 → 16k mono (추출 입력).
pub fn stem_to_16k(left: &[f32], right: &[f32], sr: u32) -> Result<Vec<f32>, ReferenceError> {
    let n = left.len().min(right.len());
    let mono: Vec<f32> = (0..n).map(|i| (left[i] + right[i]) * 0.5).collect();
    Ok(vocalboard_dsp::resample::resample_all(&mono, sr, ANALYSIS_SR)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vocalboard_dsp::AcfEngine;

    fn voiced(midi: f32) -> F0Frame {
        F0Frame::quantize(midi, true, 0.9, -20.0)
    }

    fn unvoiced() -> F0Frame {
        F0Frame::quantize(0.0, false, 0.1, -70.0)
    }

    #[test]
    fn extraction_gates_and_emits_every_hop() {
        // 2초 사인 + 1초 무음 @16k.
        let w = 2.0 * std::f64::consts::PI * 220.0 / 16_000.0;
        let mut x: Vec<f32> = (0..32_000)
            .map(|i| 0.3 * ((w * i as f64) % (2.0 * std::f64::consts::PI)).sin() as f32)
            .collect();
        x.extend(std::iter::repeat(0.0f32).take(16_000));
        let mut engine = AcfEngine::new();
        let mut prog = 0.0;
        let frames = extract_frames(
            &mut engine,
            &x,
            &ExtractParams { conf_threshold: 0.6, ..Default::default() },
            &mut |p| prog = p,
        )
        .unwrap();
        assert_eq!(frames.len(), 48_000 / 256);
        assert!((prog - 1.0).abs() < 1e-6);
        // 사인 구간 다수 voiced (ACF 기준선), 무음 구간 전부 unvoiced.
        let sine_part = &frames[4..120];
        let voiced_count = sine_part.iter().filter(|f| f.voiced()).count();
        assert!(voiced_count * 10 >= sine_part.len() * 8, "{voiced_count}/{}", sine_part.len());
        assert!(frames[130..].iter().all(|f| !f.voiced()));
    }

    #[test]
    fn bridging_fills_short_gaps_only() {
        let p = ExtractParams::default();
        // 짧은 갭 (3 hop, 피치 인접) → 메움.
        let mut a = vec![voiced(60.0); 5];
        a.extend([unvoiced(), unvoiced(), unvoiced()]);
        a.extend(vec![voiced(60.5); 5]);
        bridge_gaps(&mut a, &p);
        assert!(a.iter().all(|f| f.voiced()));
        // 보간 단조 증가.
        assert!(a[5].midi() > 60.0 && a[7].midi() < 60.5 + 1e-3);

        // 긴 갭 (6 hop) → 그대로.
        let mut b = vec![voiced(60.0); 3];
        b.extend(vec![unvoiced(); 6]);
        b.extend(vec![voiced(60.2); 3]);
        bridge_gaps(&mut b, &p);
        assert!(b[3..9].iter().all(|f| !f.voiced()));

        // 피치가 먼 갭 (3 hop, 5 semitones) → 그대로.
        let mut c = vec![voiced(60.0); 3];
        c.extend(vec![unvoiced(); 3]);
        c.extend(vec![voiced(65.0); 3]);
        bridge_gaps(&mut c, &p);
        assert!(c[3..6].iter().all(|f| !f.voiced()));
    }

    #[test]
    fn note_segmentation_splits_and_filters() {
        let p = SegmentParams::default();
        let mut frames = Vec::new();
        frames.extend(vec![voiced(60.0); 20]); // 320ms C4
        frames.push(unvoiced()); // 16ms grace 내
        frames.extend(vec![voiced(60.1); 10]); // 이어짐
        frames.extend(vec![voiced(64.0); 15]); // 도약 → 새 노트 E4
        frames.extend(vec![unvoiced(); 10]); // 종료
        frames.extend(vec![voiced(67.0); 2]); // 32ms — min_note 미달 탈락
        frames.extend(vec![unvoiced(); 5]);

        let notes = segment_notes(&frames, &p);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!((notes[0].m - 60.0).abs() < 0.2);
        assert_eq!(notes[0].s, 0);
        assert!(notes[0].e >= 20 * 16);
        assert!((notes[1].m - 64.0).abs() < 0.1);
        assert!(notes[1].s >= notes[0].e);
    }

    #[test]
    fn vibrato_stays_one_note() {
        let p = SegmentParams::default();
        // ±0.3 semitone 비브라토 → 분할 없음.
        let frames: Vec<F0Frame> = (0..100)
            .map(|i| voiced(60.0 + 0.3 * ((i as f32) * 0.4).sin()))
            .collect();
        let notes = segment_notes(&frames, &p);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!((notes[0].m - 60.0).abs() < 0.2);
    }

    #[test]
    fn stem_to_16k_resamples() {
        let y = stem_to_16k(&vec![0.1f32; 44_100], &vec![0.1f32; 44_100], 44_100).unwrap();
        assert!((y.len() as i64 - 16_000).abs() < 1024, "{}", y.len());
    }
}
