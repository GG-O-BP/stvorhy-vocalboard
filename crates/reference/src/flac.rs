//! 분리 스템 FLAC 저장 (§5: 스템은 FLAC로 저장).

use std::path::Path;

use flacenc::component::BitRepr;
use flacenc::error::Verify;

use crate::ReferenceError;

/// 스테레오 f32 → 16-bit FLAC 파일.
pub fn write_flac_stereo(
    path: &Path,
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> Result<(), ReferenceError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| ReferenceError::Other(format!("flac config: {e:?}")))?;
    let n = left.len().min(right.len());
    // flacenc 0.5.1의 '짧은 마지막 블록' 출력은 symphonia 0.6 파서가
    // UnexpectedEof로 거부한다 (실측: 블록 배수 길이는 정상). 블록 배수로
    // 제로 패딩해 회피 — 꼬리 무음 ≤ block_size-1 샘플 (~93ms @44.1k).
    let padded = n.div_ceil(config.block_size) * config.block_size;
    let mut interleaved = Vec::with_capacity(padded * 2);
    for i in 0..n {
        interleaved.push((left[i].clamp(-1.0, 1.0) * 32767.0) as i32);
        interleaved.push((right[i].clamp(-1.0, 1.0) * 32767.0) as i32);
    }
    interleaved.resize(padded * 2, 0);
    let source =
        flacenc::source::MemSource::from_samples(&interleaved, 2, 16, sample_rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| ReferenceError::Other(format!("flac encode: {e:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| ReferenceError::Other(format!("flac write: {e:?}")))?;
    std::fs::write(path, sink.as_slice())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flac_roundtrips_through_symphonia() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stem.flac");
        let n = 44_100usize;
        let w = 2.0 * std::f64::consts::PI * 330.0 / 44_100.0;
        let left: Vec<f32> = (0..n)
            .map(|i| 0.5 * ((w * i as f64) % (2.0 * std::f64::consts::PI)).sin() as f32)
            .collect();
        let right: Vec<f32> = left.iter().map(|v| v * 0.5).collect();
        write_flac_stereo(&path, &left, &right, 44_100).unwrap();

        let d = crate::decode::decode_file(&path).unwrap();
        assert_eq!(d.channels, 2);
        assert_eq!(d.sample_rate, 44_100);
        assert!((d.duration_ms() as i64 - 1000).abs() < 100, "{}", d.duration_ms());
        let (l2, r2) = d.to_stereo();
        // 16-bit 양자화 오차 내 일치.
        for i in (0..n).step_by(997) {
            assert!((l2[i] - left[i]).abs() < 2.0 / 32768.0 + 1e-4);
            assert!((r2[i] - right[i]).abs() < 2.0 / 32768.0 + 1e-4);
        }
    }
}
