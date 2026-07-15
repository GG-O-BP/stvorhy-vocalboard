//! E2E 검증용 합성 곡 생성기: A4(440Hz) 멜로디 + 저역 반주 톤.
//! 사용: cargo run -p vocalboard-reference --example gen_song -- <out.wav> [secs]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).expect("usage: gen_song <out.wav> [secs]");
    let secs: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(15.0);
    let sr = 44_100u32;
    let n = (sr as f64 * secs) as usize;
    let two_pi = 2.0 * std::f64::consts::PI;

    // 멜로디: A4 → C5 → A4 → E4 반복 (2초 단위), "보컬" 역할.
    let melody = [440.0f64, 523.25, 440.0, 329.63];
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: sr,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(out, spec).unwrap();
    let mut phase_m = 0.0f64;
    let mut phase_b = 0.0f64;
    for i in 0..n {
        let t = i as f64 / sr as f64;
        let seg = ((t / 2.0) as usize) % melody.len();
        let f = melody[seg];
        phase_m = (phase_m + two_pi * f / sr as f64) % two_pi;
        phase_b = (phase_b + two_pi * 110.0 / sr as f64) % two_pi;
        // 보컬(멜로디+배음) + 반주(저역 톤).
        let vocal = 0.35 * phase_m.sin() + 0.12 * (2.0 * phase_m).sin();
        let bass = 0.15 * phase_b.sin();
        let v = ((vocal + bass) * 32767.0) as i16;
        w.write_sample(v).unwrap();
        w.write_sample(v).unwrap();
    }
    w.finalize().unwrap();
    println!("wrote {out} ({secs}s)");
}
