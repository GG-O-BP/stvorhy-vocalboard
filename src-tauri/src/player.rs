//! 재생 시스템 (§2 재생: cpal 출력 스트림 직접, 플레이헤드=소비 샘플 수,
//! Channel로 동기).
//!
//! 출력 콜백은 RT 규칙 준수: 프리롤된 Arc<Vec<f32>> 읽기 + 원자 카운터만.
//! 플레이헤드 이벤트는 별도 스레드가 20Hz로 Channel 송출한다.
//! Phase 3: 세션 녹음(WAV mono) 리뷰 재생. Phase 3.5: 스템 재생에 재사용.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;

/// 프리롤된 interleaved PCM 클립.
pub struct Clip {
    pub samples: Arc<Vec<f32>>,
    pub channels: u16,
    pub sample_rate: u32,
}

impl Clip {
    /// symphonia로 디코드 (WAV/FLAC/MP3/AAC-LC/OGG).
    pub fn from_file(path: &std::path::Path) -> Result<Self, String> {
        let d = vocalboard_reference::decode::decode_file(path).map_err(|e| e.to_string())?;
        Ok(Self {
            samples: Arc::new(d.samples),
            channels: d.channels.max(1),
            sample_rate: d.sample_rate,
        })
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn duration_ms(&self) -> u32 {
        (self.frames() as u64 * 1000 / self.sample_rate.max(1) as u64) as u32
    }
}

/// 플레이헤드 동기 이벤트 (프론트 오버레이 커서용).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PlayheadEvent {
    /// 재생 위치 ms.
    pub t: u32,
    pub playing: bool,
    pub done: bool,
    pub duration_ms: u32,
}

struct PlayerShared {
    /// 소비된 출력 프레임 수 (출력 SR 기준) — 플레이헤드의 단일 진실.
    pos: AtomicU64,
    playing: AtomicBool,
    stop: AtomicBool,
    total: u64,
    out_sr: u32,
}

pub struct Player {
    shared: Arc<PlayerShared>,
    threads: Vec<JoinHandle<()>>,
}

impl Player {
    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::Release);
    }

    pub fn resume(&self) {
        self.shared.playing.store(true, Ordering::Release);
    }

    pub fn seek_ms(&self, t_ms: u32) {
        let frame = (t_ms as u64) * self.shared.out_sr as u64 / 1000;
        self.shared.pos.store(frame.min(self.shared.total), Ordering::Release);
    }

    pub fn position_ms(&self) -> u32 {
        (self.shared.pos.load(Ordering::Acquire) * 1000 / self.shared.out_sr as u64) as u32
    }

    pub fn stop(mut self) {
        self.shared.stop.store(true, Ordering::Release);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// 클립 재생을 시작한다. `on_event`는 ~20Hz + 종료 시 1회 호출된다.
pub fn play(
    clip: Clip,
    start_ms: u32,
    on_event: Box<dyn Fn(PlayheadEvent) + Send + 'static>,
) -> Result<Player, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "출력 장치가 없습니다".to_string())?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("출력 설정 조회 실패: {e}"))?;
    let out_sr = config.sample_rate();
    let out_ch = config.channels().max(1) as usize;
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    // 장치 SR로 프리롤 리샘플 (오프라인, 재생 전 1회, 채널별).
    let clip_ch = clip.channels.max(1) as usize;
    let samples: Arc<Vec<f32>> = if clip.sample_rate == out_sr {
        clip.samples
    } else {
        let frames = clip.samples.len() / clip_ch;
        let mut per_ch: Vec<Vec<f32>> = Vec::with_capacity(clip_ch);
        for c in 0..clip_ch {
            let chan: Vec<f32> = (0..frames).map(|i| clip.samples[i * clip_ch + c]).collect();
            per_ch.push(
                vocalboard_dsp::resample::resample_all(&chan, clip.sample_rate, out_sr)
                    .map_err(|e| e.to_string())?,
            );
        }
        let out_frames = per_ch.iter().map(|v| v.len()).min().unwrap_or(0);
        let mut inter = Vec::with_capacity(out_frames * clip_ch);
        for i in 0..out_frames {
            for chan in &per_ch {
                inter.push(chan[i]);
            }
        }
        Arc::new(inter)
    };
    let total = (samples.len() / clip_ch) as u64;

    let shared = Arc::new(PlayerShared {
        pos: AtomicU64::new(((start_ms as u64) * out_sr as u64 / 1000).min(total)),
        playing: AtomicBool::new(true),
        stop: AtomicBool::new(false),
        total,
        out_sr,
    });

    // 스트림 소유 스레드 (capture와 동일 패턴: Stream !Send 안전).
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let stream_shared = shared.clone();
    let stream_samples = samples.clone();
    let owner = std::thread::Builder::new()
        .name("vocalboard-playback".into())
        .spawn(move || {
            let shared = stream_shared;
            let samples = stream_samples;

            macro_rules! build {
                ($t:ty, $conv:expr) => {{
                    let s = shared.clone();
                    let data_arc = samples.clone();
                    device.build_output_stream(
                        &stream_config,
                        move |out: &mut [$t], _| {
                            // RT 콜백: Arc 읽기 + 원자 연산만.
                            let playing = s.playing.load(Ordering::Acquire);
                            let mut pos = s.pos.load(Ordering::Acquire);
                            for frame in out.chunks_mut(out_ch) {
                                if playing && pos < s.total {
                                    let base = pos as usize * clip_ch;
                                    for (i, slot) in frame.iter_mut().enumerate() {
                                        // 클립 채널 부족분은 마지막 채널 복제.
                                        let c = i.min(clip_ch - 1);
                                        *slot = $conv(data_arc[base + c]);
                                    }
                                    pos += 1;
                                } else {
                                    for slot in frame.iter_mut() {
                                        *slot = $conv(0.0);
                                    }
                                }
                            }
                            if playing {
                                s.pos.store(pos, Ordering::Release);
                            }
                        },
                        |e| eprintln!("[player] 스트림 오류: {e}"),
                        None,
                    )
                }};
            }

            let stream = match sample_format {
                cpal::SampleFormat::F32 => build!(f32, |v: f32| v),
                cpal::SampleFormat::I16 => build!(i16, |v: f32| (v.clamp(-1.0, 1.0) * 32767.0) as i16),
                cpal::SampleFormat::U16 => {
                    build!(u16, |v: f32| ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * 65535.0) as u16)
                }
                other => {
                    let _ = ready_tx.send(Err(format!("미지원 출력 포맷: {other:?}")));
                    return;
                }
            };
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("출력 스트림 구축 실패: {e}")));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(format!("재생 시작 실패: {e}")));
                return;
            }
            let _ = ready_tx.send(Ok(()));
            while !shared.stop.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(20));
            }
            drop(stream);
        })
        .map_err(|e| e.to_string())?;

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = owner.join();
            return Err(e);
        }
        Err(_) => {
            shared.stop.store(true, Ordering::Release);
            let _ = owner.join();
            return Err("출력 스트림 초기화 시간 초과".into());
        }
    }

    // 플레이헤드 이벤트 스레드 (~20Hz).
    let emit_shared = shared.clone();
    let duration_ms = (total * 1000 / out_sr as u64) as u32;
    let emitter = std::thread::Builder::new()
        .name("vocalboard-playhead".into())
        .spawn(move || {
            let mut done_sent = false;
            loop {
                if emit_shared.stop.load(Ordering::Acquire) {
                    break;
                }
                let pos = emit_shared.pos.load(Ordering::Acquire);
                let done = pos >= emit_shared.total;
                let playing = emit_shared.playing.load(Ordering::Acquire) && !done;
                if done && !done_sent {
                    done_sent = true;
                    emit_shared.playing.store(false, Ordering::Release);
                }
                on_event(PlayheadEvent {
                    t: (pos * 1000 / emit_shared.out_sr as u64) as u32,
                    playing,
                    done,
                    duration_ms,
                });
                if done {
                    // 종료 이벤트는 보냈고, stop 대기만 남는다.
                    std::thread::sleep(Duration::from_millis(200));
                } else {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        })
        .map_err(|e| e.to_string())?;

    Ok(Player {
        shared,
        threads: vec![owner, emitter],
    })
}
