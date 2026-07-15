//! 캡처 시스템 (스펙 §2).
//!
//! cpal 입력 콜백(RT 스레드)은 **ringbuf push만** 한다 — 할당/락/로그/IPC
//! 금지. DSP 스레드가 SPSC ringbuf에서 꺼내 mixdown → PitchPipeline →
//! 62.5Hz `PitchFrame`을 싱크(Channel 등)로 방출한다.
//!
//! 스트림 소유는 캡처 스레드가 맡는다 (cpal::Stream은 !Send 가정이 안전).
//! 개발 편의용 시뮬레이터 입력(VOCALBOARD_SIM_INPUT=1)은 마이크 없는
//! 환경에서 파이프라인 E2E를 검증하기 위한 것으로, 비브라토 사인을
//! 실시간 페이스로 ringbuf에 공급한다.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;
use serde::Serialize;
use vocalboard_dsp::{InferenceEngine, PipelineParams, PitchFrame, PitchPipeline};

/// ringbuf 용량 (interleaved f32). 48k 스테레오 기준 ≈0.68초.
const RING_CAPACITY: usize = 1 << 16;

#[derive(Debug, Clone, Serialize)]
pub struct CaptureInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub simulated: bool,
}

/// DSP 스레드가 산출물을 넘길 대상 (Channel + 스토리지 mpsc 팬아웃).
pub trait FrameSink: Send + 'static {
    fn on_frame(&mut self, frame: &PitchFrame);
    /// 드라이 mono 입력 (mixdown 직후, HPF 이전, 입력 SR) — 녹음용.
    fn on_audio(&mut self, _mono: &[f32]) {}
}

impl<F: FnMut(&PitchFrame) + Send + 'static> FrameSink for F {
    fn on_frame(&mut self, frame: &PitchFrame) {
        self(frame)
    }
}

struct Shared {
    stop: AtomicBool,
    /// RT 콜백에서 링버퍼가 가득 차 버린 샘플 수 (진단용).
    overrun: AtomicU64,
    params: Mutex<PipelineParams>,
    params_dirty: AtomicBool,
    error: Mutex<Option<String>>,
}

pub struct Capture {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    pub info: CaptureInfo,
}

impl Capture {
    /// 파라미터를 실시간 갱신한다 (다음 hop부터 적용).
    pub fn set_params(&self, p: PipelineParams) {
        *self.shared.params.lock().unwrap() = p;
        self.shared.params_dirty.store(true, Ordering::Release);
    }

    pub fn stop(mut self) {
        self.shared.stop.store(true, Ordering::Release);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        // 진단: 오류/오버런 리포트 (RT 콜백에서는 로그 금지였던 것들).
        if let Some(e) = self.shared.error.lock().unwrap().take() {
            eprintln!("[capture] 스트림 오류 발생 이력: {e}");
        }
        let overruns = self.shared.overrun.load(Ordering::Relaxed);
        if overruns > 0 {
            eprintln!("[capture] ringbuf 오버런: {overruns} 샘플 유실");
        }
    }
}

/// 캡처 시작 전에 입력 SR/채널을 조회한다 (스토리지 Begin이 프레임보다
/// 먼저 가야 하므로 세션 메타 확정용).
pub fn probe() -> Result<CaptureInfo, String> {
    if std::env::var("VOCALBOARD_SIM_INPUT").is_ok_and(|v| v == "1") {
        return Ok(CaptureInfo { sample_rate: 48_000, channels: 1, simulated: true });
    }
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "입력 장치가 없습니다".to_string())?;
    let config = device
        .default_input_config()
        .map_err(|e| format!("입력 설정 조회 실패: {e}"))?;
    Ok(CaptureInfo {
        sample_rate: config.sample_rate(),
        channels: config.channels(),
        simulated: false,
    })
}

/// 캡처를 시작한다. `engine`/`sink`는 DSP 스레드로 이동한다.
pub fn start(
    engine: Box<dyn InferenceEngine>,
    params: PipelineParams,
    sink: Box<dyn FrameSink>,
) -> Result<Capture, String> {
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        overrun: AtomicU64::new(0),
        params: Mutex::new(params),
        params_dirty: AtomicBool::new(false),
        error: Mutex::new(None),
    });

    let simulated = std::env::var("VOCALBOARD_SIM_INPUT").is_ok_and(|v| v == "1");
    let rb = HeapRb::<f32>::new(RING_CAPACITY);
    let (producer, consumer) = rb.split();

    let (info, capture_thread) = if simulated {
        let info = CaptureInfo { sample_rate: 48_000, channels: 1, simulated: true };
        let t = spawn_sim_source(producer, shared.clone());
        (info, t)
    } else {
        let (info, t) = spawn_cpal_source(producer, shared.clone())?;
        (info, t)
    };

    let dsp_thread = spawn_dsp_thread(
        consumer,
        shared.clone(),
        info.sample_rate,
        info.channels,
        engine,
        params,
        sink,
    )?;

    Ok(Capture {
        shared,
        threads: vec![capture_thread, dsp_thread],
        info,
    })
}

type Prod = ringbuf::HeapProd<f32>;
type Cons = ringbuf::HeapCons<f32>;

/// cpal 입력 스트림을 소유하는 스레드를 띄운다. 스트림 구축 성공/실패는
/// 동기적으로 확인한다 (채널로 결과 회신).
fn spawn_cpal_source(
    mut producer: Prod,
    shared: Arc<Shared>,
) -> Result<(CaptureInfo, JoinHandle<()>), String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<CaptureInfo, String>>();

    let shared_thread = shared.clone();
    let t = std::thread::Builder::new()
        .name("vocalboard-capture".into())
        .spawn(move || {
            let shared = shared_thread;
            let host = cpal::default_host();
            let Some(device) = host.default_input_device() else {
                let _ = ready_tx.send(Err("입력 장치가 없습니다".into()));
                return;
            };
            let config = match device.default_input_config() {
                Ok(c) => c,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("입력 설정 조회 실패: {e}")));
                    return;
                }
            };
            let sample_rate = config.sample_rate();
            let channels = config.channels();
            let sample_format = config.sample_format();
            let stream_config: cpal::StreamConfig = config.into();

            let err_shared = shared.clone();
            let err_cb = move |e: cpal::StreamError| {
                // 오류 경로 (데이터 콜백 아님).
                *err_shared.error.lock().unwrap() = Some(e.to_string());
            };

            macro_rules! build {
                ($t:ty, $conv:expr) => {{
                    let over = shared.clone();
                    device.build_input_stream(
                        &stream_config,
                        move |data: &[$t], _| {
                            // RT 콜백: ringbuf push만.
                            let pushed = producer.push_iter(data.iter().map($conv));
                            let dropped = data.len() - pushed;
                            if dropped > 0 {
                                over.overrun.fetch_add(dropped as u64, Ordering::Relaxed);
                            }
                        },
                        err_cb,
                        None,
                    )
                }};
            }

            let stream = match sample_format {
                cpal::SampleFormat::F32 => build!(f32, |s: &f32| *s),
                cpal::SampleFormat::I16 => build!(i16, |s: &i16| *s as f32 / 32768.0),
                cpal::SampleFormat::U16 => build!(u16, |s: &u16| (*s as f32 - 32768.0) / 32768.0),
                other => {
                    let _ = ready_tx.send(Err(format!("미지원 샘플 포맷: {other:?}")));
                    return;
                }
            };
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("입력 스트림 구축 실패: {e}")));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(format!("스트림 시작 실패: {e}")));
                return;
            }
            let _ = ready_tx.send(Ok(CaptureInfo { sample_rate, channels, simulated: false }));

            // 스트림 생존 유지. stop 신호까지 대기.
            while !shared.stop.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(20));
            }
            drop(stream);
        })
        .map_err(|e| e.to_string())?;

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(info)) => Ok((info, t)),
        Ok(Err(e)) => {
            let _ = t.join();
            Err(e)
        }
        Err(_) => {
            shared.stop.store(true, Ordering::Release);
            let _ = t.join();
            Err("입력 스트림 초기화 시간 초과".into())
        }
    }
}

/// 개발용 시뮬레이터 소스: A4 비브라토(±30c, 5.5Hz) + 2/3배음, 실시간 페이스.
fn spawn_sim_source(mut producer: Prod, shared: Arc<Shared>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("vocalboard-sim-input".into())
        .spawn(move || {
            let sr = 48_000.0f64;
            let two_pi = 2.0 * std::f64::consts::PI;
            let mut phase = 0.0f64;
            let mut vib_phase = 0.0f64;
            let mut i: u64 = 0;
            let block = 480; // 10ms
            while !shared.stop.load(Ordering::Acquire) {
                let mut buf = [0.0f32; 480];
                for s in buf.iter_mut() {
                    // 5.5Hz 비브라토 ±30 cents.
                    let vib = (vib_phase).sin() * (30.0 / 1200.0) * std::f64::consts::LN_2;
                    let f = 440.0 * vib.exp();
                    phase = (phase + two_pi * f / sr) % two_pi;
                    vib_phase = (vib_phase + two_pi * 5.5 / sr) % two_pi;
                    let v = 0.35 * phase.sin() + 0.18 * (2.0 * phase).sin() + 0.08 * (3.0 * phase).sin();
                    // 4초 주기로 2초 발성 / 2초 무음 (게이트 경로 검증).
                    let gate_on = (i / (sr as u64 * 2)) % 2 == 0;
                    *s = if gate_on { (v * 0.5) as f32 } else { 0.0 };
                    i += 1;
                }
                let pushed = producer.push_slice(&buf);
                if pushed < block {
                    shared
                        .overrun
                        .fetch_add((block - pushed) as u64, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })
        .expect("spawn sim source")
}

fn spawn_dsp_thread(
    mut consumer: Cons,
    shared: Arc<Shared>,
    sample_rate: u32,
    channels: u16,
    engine: Box<dyn InferenceEngine>,
    params: PipelineParams,
    mut sink: Box<dyn FrameSink>,
) -> Result<JoinHandle<()>, String> {
    let mut pipeline =
        PitchPipeline::new(sample_rate, engine, params).map_err(|e| e.to_string())?;
    let ch = channels.max(1) as usize;

    std::thread::Builder::new()
        .name("vocalboard-dsp".into())
        .spawn(move || {
            let mut raw = vec![0.0f32; 8192];
            let mut pending: Vec<f32> = Vec::with_capacity(8192); // 채널 경계 잔여
            let mut mono: Vec<f32> = Vec::with_capacity(8192);
            let mut frames: Vec<PitchFrame> = Vec::with_capacity(16);

            loop {
                if shared.stop.load(Ordering::Acquire) {
                    break;
                }
                if shared.params_dirty.swap(false, Ordering::AcqRel) {
                    pipeline.set_params(*shared.params.lock().unwrap());
                }
                let n = consumer.pop_slice(&mut raw);
                if n == 0 {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                pending.extend_from_slice(&raw[..n]);
                let whole = pending.len() / ch * ch;
                mono.clear();
                for frame in pending[..whole].chunks_exact(ch) {
                    mono.push(frame.iter().sum::<f32>() / ch as f32);
                }
                pending.drain(..whole);
                if !mono.is_empty() {
                    sink.on_audio(&mono);
                }

                frames.clear();
                if let Err(e) = pipeline.process(&mono, &mut frames) {
                    *shared.error.lock().unwrap() = Some(e.to_string());
                    break;
                }
                for f in &frames {
                    sink.on_frame(f);
                }
            }
        })
        .map_err(|e| e.to_string())
}
