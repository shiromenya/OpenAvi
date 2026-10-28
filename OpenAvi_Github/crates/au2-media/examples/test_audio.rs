use std::sync::Arc;
use std::time::Duration;
use au2_media::{AudioEngine, AudioTrackItem};

fn main() {
    println!("=== Testing Audio Output on Windows ===");
    let engine = AudioEngine::new();

    let sample_rate = 44100;
    let total_samples = sample_rate * 2;
    let mut samples = Vec::with_capacity(total_samples * 2);
    for i in 0..total_samples {
        let t = i as f64 / sample_rate as f64;
        let v = (t * 440.0 * 2.0 * std::f64::consts::PI).sin();
        let s = (v * 16000.0) as i16;
        samples.push(s); // L
        samples.push(s); // R
    }

    let pcm = Arc::new(samples);
    let track = AudioTrackItem {
        audio: pcm,
        start_frame: 0,
        end_frame: 60,
        offset_sec: 0.0,
        volume: 1.0,
    };

    println!("Setting active track (2 seconds of 440Hz tone)...");
    engine.set_active_tracks(vec![track]);
    println!("Starting playback...");
    engine.sync_playback(true, 0, 30.0);

    for sec in 1..=3 {
        std::thread::sleep(Duration::from_secs(1));
        println!("Elapsed: {} sec...", sec);
        engine.sync_playback(true, sec * 30, 30.0);
    }

    engine.stop();
    println!("Audio test completed.");
}
