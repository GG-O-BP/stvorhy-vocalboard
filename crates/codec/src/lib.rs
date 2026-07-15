//! F0C1 프레임 코덱 (스펙 §5).
//!
//! 4B/frame: `u16 midi_cents`(MIDI×100, 0=unvoiced), `u8 conf`(×255),
//! `u8 rms`(clamp(round(dBFS)+96, 0, 96), 복원 = 값−96).
//! 타임스탬프는 인덱스×hop_ms로 암시 — 결번 없음이 전제.
//! 저장 시 zstd. 같은 바이너리 레이아웃이 `.f0raw` 저널 파일에도 쓰인다
//! (frame_count=UNKNOWN, 꼬리 부분 프레임 잘림 허용).

use thiserror::Error;

pub mod preview;

/// 저장 계층이 sessions.codec / tracks.pitch_codec 에 기록하는 식별자.
pub const CODEC_NAME: &str = "f0c1+zstd";

pub const MAGIC: [u8; 4] = *b"F0C1";
pub const VERSION: u8 = 1;
pub const HOP_MS: u8 = 16;
pub const RMS_OFFSET: u8 = 96;
/// 저널(.f0raw) 모드: frame_count 미확정. 디코더는 남은 길이에서 개수를 유도한다.
pub const FRAME_COUNT_UNKNOWN: u32 = u32::MAX;

pub const HEADER_LEN: usize = 20;
pub const FRAME_LEN: usize = 4;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("buffer too short: {0} bytes")]
    TooShort(usize),
    #[error("bad magic")]
    BadMagic,
    #[error("unsupported version {0}")]
    UnsupportedVersion(u8),
    #[error("frame count mismatch: header {header}, payload {payload}")]
    CountMismatch { header: u32, payload: usize },
    #[error("zstd: {0}")]
    Zstd(#[from] std::io::Error),
}

/// F0C1 헤더: 버전, hop_ms, 시작 시각, rms_offset (스펙 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct F0c1Header {
    pub version: u8,
    pub hop_ms: u8,
    pub rms_offset: u8,
    /// 세션/트랙 시작 시각 (unix epoch ms). 트랙 피치처럼 무의미하면 0.
    pub start_unix_ms: i64,
    /// 프레임 수. 저널 파일은 [`FRAME_COUNT_UNKNOWN`].
    pub frame_count: u32,
}

impl F0c1Header {
    pub fn new(start_unix_ms: i64, frame_count: u32) -> Self {
        Self {
            version: VERSION,
            hop_ms: HOP_MS,
            rms_offset: RMS_OFFSET,
            start_unix_ms,
            frame_count,
        }
    }
}

/// 4바이트 양자화 프레임.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct F0Frame {
    /// MIDI×100. 0 = unvoiced.
    pub midi_cents: u16,
    /// confidence×255.
    pub conf: u8,
    /// clamp(round(dBFS)+96, 0, 96).
    pub rms: u8,
}

impl F0Frame {
    /// PitchFrame 값 도메인에서 양자화한다. voiced=false면 midi는 무시된다
    /// (F0C1은 unvoiced 피치 추정치를 보존하지 않는다 — 0=unvoiced 인코딩).
    pub fn quantize(midi: f32, voiced: bool, confidence: f32, rms_dbfs: f32) -> Self {
        let midi_cents = if voiced {
            // voiced인데 0으로 양자화되면 unvoiced와 충돌하므로 1로 클램프.
            ((midi * 100.0).round() as i64).clamp(1, u16::MAX as i64) as u16
        } else {
            0
        };
        Self {
            midi_cents,
            conf: ((confidence * 255.0).round() as i64).clamp(0, 255) as u8,
            rms: (rms_dbfs.round() as i64 + RMS_OFFSET as i64).clamp(0, RMS_OFFSET as i64) as u8,
        }
    }

    pub fn voiced(&self) -> bool {
        self.midi_cents != 0
    }

    pub fn midi(&self) -> f32 {
        self.midi_cents as f32 / 100.0
    }

    pub fn confidence(&self) -> f32 {
        self.conf as f32 / 255.0
    }

    pub fn rms_dbfs(&self) -> f32 {
        self.rms as f32 - RMS_OFFSET as f32
    }
}

pub fn encode_header(h: &F0c1Header) -> [u8; HEADER_LEN] {
    let mut out = [0u8; HEADER_LEN];
    out[0..4].copy_from_slice(&MAGIC);
    out[4] = h.version;
    out[5] = h.hop_ms;
    out[6] = h.rms_offset;
    out[7] = 0; // reserved
    out[8..16].copy_from_slice(&h.start_unix_ms.to_le_bytes());
    out[16..20].copy_from_slice(&h.frame_count.to_le_bytes());
    out
}

pub fn encode_frame(f: &F0Frame) -> [u8; FRAME_LEN] {
    let mut out = [0u8; FRAME_LEN];
    out[0..2].copy_from_slice(&f.midi_cents.to_le_bytes());
    out[2] = f.conf;
    out[3] = f.rms;
    out
}

fn decode_frame(b: &[u8]) -> F0Frame {
    F0Frame {
        midi_cents: u16::from_le_bytes([b[0], b[1]]),
        conf: b[2],
        rms: b[3],
    }
}

/// 비압축 F0C1 인코딩. frame_count는 실제 프레임 수로 기록된다.
pub fn encode(start_unix_ms: i64, frames: &[F0Frame]) -> Vec<u8> {
    let header = F0c1Header::new(start_unix_ms, frames.len() as u32);
    let mut out = Vec::with_capacity(HEADER_LEN + frames.len() * FRAME_LEN);
    out.extend_from_slice(&encode_header(&header));
    for f in frames {
        out.extend_from_slice(&encode_frame(f));
    }
    out
}

pub fn decode_header(bytes: &[u8]) -> Result<F0c1Header, CodecError> {
    if bytes.len() < HEADER_LEN {
        return Err(CodecError::TooShort(bytes.len()));
    }
    if bytes[0..4] != MAGIC {
        return Err(CodecError::BadMagic);
    }
    let version = bytes[4];
    if version != VERSION {
        return Err(CodecError::UnsupportedVersion(version));
    }
    Ok(F0c1Header {
        version,
        hop_ms: bytes[5],
        rms_offset: bytes[6],
        start_unix_ms: i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        frame_count: u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
    })
}

/// 비압축 F0C1 디코딩.
///
/// frame_count가 [`FRAME_COUNT_UNKNOWN`]이면 저널 모드: 남은 길이에서 프레임
/// 수를 유도하고 꼬리의 부분 프레임(크래시 잘림)은 버린다. 확정 카운트인데
/// 페이로드가 모자라면 오류.
pub fn decode(bytes: &[u8]) -> Result<(F0c1Header, Vec<F0Frame>), CodecError> {
    let header = decode_header(bytes)?;
    let payload = &bytes[HEADER_LEN..];
    let available = payload.len() / FRAME_LEN;
    let count = if header.frame_count == FRAME_COUNT_UNKNOWN {
        available
    } else {
        let c = header.frame_count as usize;
        if available < c {
            return Err(CodecError::CountMismatch {
                header: header.frame_count,
                payload: available,
            });
        }
        c
    };
    let mut frames = Vec::with_capacity(count);
    for i in 0..count {
        frames.push(decode_frame(&payload[i * FRAME_LEN..i * FRAME_LEN + FRAME_LEN]));
    }
    Ok((header, frames))
}

const ZSTD_LEVEL: i32 = 3;

/// zstd 압축 포함 인코딩 (DB BLOB 저장용, codec = [`CODEC_NAME`]).
pub fn encode_compressed(start_unix_ms: i64, frames: &[F0Frame]) -> Result<Vec<u8>, CodecError> {
    Ok(zstd::bulk::compress(&encode(start_unix_ms, frames), ZSTD_LEVEL)?)
}

/// zstd 압축 BLOB 디코딩.
pub fn decode_compressed(blob: &[u8]) -> Result<(F0c1Header, Vec<F0Frame>), CodecError> {
    // 15KB/분 원본이므로 상한은 넉넉히: 24시간 = 21.6MB.
    let raw = zstd::bulk::decompress(blob, 32 << 20)?;
    decode(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 의존성 없는 결정적 의사난수 (xorshift32).
    struct Rng(u32);
    impl Rng {
        fn next(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }
        fn f32(&mut self) -> f32 {
            (self.next() >> 8) as f32 / (1 << 24) as f32
        }
    }

    fn random_frames(n: usize, seed: u32) -> Vec<F0Frame> {
        let mut rng = Rng(seed);
        (0..n)
            .map(|_| {
                let voiced = rng.f32() > 0.3;
                let midi = 30.0 + rng.f32() * 66.0; // G1..C7 부근
                let conf = rng.f32();
                let rms = -96.0 + rng.f32() * 96.0;
                F0Frame::quantize(midi, voiced, conf, rms)
            })
            .collect()
    }

    #[test]
    fn roundtrip_uncompressed() {
        let frames = random_frames(1000, 42);
        let start = 1_752_566_400_123_i64;
        let bytes = encode(start, &frames);
        assert_eq!(bytes.len(), HEADER_LEN + 4000);
        let (header, decoded) = decode(&bytes).unwrap();
        assert_eq!(header.version, VERSION);
        assert_eq!(header.hop_ms, 16);
        assert_eq!(header.rms_offset, 96);
        assert_eq!(header.start_unix_ms, start);
        assert_eq!(header.frame_count, 1000);
        assert_eq!(decoded, frames);
    }

    #[test]
    fn roundtrip_compressed() {
        let frames = random_frames(5000, 7);
        let blob = encode_compressed(99, &frames).unwrap();
        assert!(blob.len() < HEADER_LEN + 5000 * FRAME_LEN);
        let (header, decoded) = decode_compressed(&blob).unwrap();
        assert_eq!(header.start_unix_ms, 99);
        assert_eq!(decoded, frames);
    }

    #[test]
    fn journal_mode_tolerates_truncated_tail() {
        let frames = random_frames(10, 3);
        let header = F0c1Header::new(5, FRAME_COUNT_UNKNOWN);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_header(&header));
        for f in &frames {
            bytes.extend_from_slice(&encode_frame(f));
        }
        // 크래시로 마지막 프레임이 2바이트만 쓰였다고 가정.
        bytes.truncate(bytes.len() - 2);
        let (h, decoded) = decode(&bytes).unwrap();
        assert_eq!(h.frame_count, FRAME_COUNT_UNKNOWN);
        assert_eq!(decoded.len(), 9);
        assert_eq!(decoded, frames[..9]);
    }

    #[test]
    fn strict_mode_rejects_short_payload() {
        let frames = random_frames(10, 9);
        let mut bytes = encode(0, &frames);
        bytes.truncate(bytes.len() - FRAME_LEN);
        assert!(matches!(
            decode(&bytes),
            Err(CodecError::CountMismatch { header: 10, payload: 9 })
        ));
    }

    #[test]
    fn quantization_domains() {
        // unvoiced: midi 무시, rms는 실측 보존.
        let f = F0Frame::quantize(52.3, false, 0.85, -45.4);
        assert_eq!(f.midi_cents, 0);
        assert!(!f.voiced());
        assert_eq!(f.conf, 217); // round(0.85*255)
        assert_eq!(f.rms, 51); // round(-45.4)=-45 → -45+96
        assert_eq!(f.rms_dbfs(), -45.0);

        // voiced 정밀도: A4 = 69.00.
        let f = F0Frame::quantize(69.004, true, 1.0, 0.0);
        assert_eq!(f.midi_cents, 6900);
        assert_eq!(f.midi(), 69.0);
        assert_eq!(f.conf, 255);
        assert_eq!(f.rms, 96);
        assert_eq!(f.rms_dbfs(), 0.0);

        // rms 하한 클램프.
        let f = F0Frame::quantize(0.0, false, 0.0, -200.0);
        assert_eq!(f.rms, 0);
        assert_eq!(f.rms_dbfs(), -96.0);

        // rms 상한 클램프 (양수 dBFS는 0으로).
        let f = F0Frame::quantize(0.0, false, 0.0, 5.0);
        assert_eq!(f.rms, 96);

        // voiced인데 0 근처 midi면 0 충돌 회피.
        let f = F0Frame::quantize(0.0, true, 0.5, -30.0);
        assert_eq!(f.midi_cents, 1);
        assert!(f.voiced());
    }

    #[test]
    fn bad_inputs() {
        assert!(matches!(decode(&[0u8; 3]), Err(CodecError::TooShort(3))));
        let mut bytes = encode(0, &[]);
        bytes[0] = b'X';
        assert!(matches!(decode(&bytes), Err(CodecError::BadMagic)));
        let mut bytes = encode(0, &[]);
        bytes[4] = 9;
        assert!(matches!(decode(&bytes), Err(CodecError::UnsupportedVersion(9))));
    }
}
