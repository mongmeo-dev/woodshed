//! Opens the default (or given) output device with the smallest buffer it
//! allows, plays a few test clicks and prints the measured output latency.
//!
//! `cargo run --release --example latency_probe [device-id]`

use std::time::Duration;

use woodshed_lib::audio::{
    AudioEngine, ClickSource, Direction, EngineConfig, EventKind, OutputRequest, list_devices,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for device in list_devices(Direction::Output)? {
        println!(
            "output device: {} [{}]{}",
            device.name,
            device.id,
            if device.is_default { " (default)" } else { "" }
        );
    }

    let (mut engine, _notifications) = AudioEngine::new(EngineConfig::default());
    let request = OutputRequest {
        device_id: std::env::args().nth(1),
        target_buffer_frames: 64,
    };
    let info = engine.open_output(&request)?;
    println!(
        "opened: {} | {} Hz | {} ch | requested buffer {:?} | reported buffer {:?}",
        info.device_name,
        info.sample_rate,
        info.channels,
        info.requested_buffer_frames,
        info.buffer_frames
    );

    let click = engine
        .handle()
        .add_source(Box::new(ClickSource::new(1_200.0, 0.03)), 0.3)?;
    let start = engine.handle().frame_after(0.05);
    let spacing = engine.format().frames_for_secs(0.25);
    for beat in 0..4 {
        let kind = EventKind::Trigger {
            note: 0,
            velocity: 1.0,
        };
        engine
            .handle()
            .schedule(start + beat * spacing, click, kind)?;
    }

    match engine.wait_for_latency(Duration::from_secs(2)) {
        Some(latency) => println!(
            "measured: {} frames per callback | output latency {:.2} ms",
            latency.callback_frames,
            latency.millis()
        ),
        None => println!("measured: no callbacks within 2 s"),
    }
    std::thread::sleep(Duration::from_millis(1_200));
    println!("stats: {:?}", engine.handle().stats());
    Ok(())
}
