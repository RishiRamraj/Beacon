//! Verifies the speech-dispatcher sink against a live daemon.
//!
//! Run with: cargo run -p beacon-output --example speak
use beacon_output::sink::{SpeechDispatcherSink, SpeechSink};
use beacon_output::{Arbiter, Config, Intent, Priority};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sink = SpeechDispatcherSink::connect()?;
    println!("connected to {}", sink.name());

    // Screen reader users routinely run far faster than the default.
    let rate: i8 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    sink.set_rate(rate)?;
    println!("rate {rate}");

    let mut arbiter = Arbiter::new(Config {
        verbosity: 3,
        ..Config::default()
    });
    let out = arbiter.resolve(
        vec![
            Intent::new("Entering Kakariko Village.", Priority::Navigation, "nav"),
            Intent::new("Chest to the north.", Priority::Interaction, "prox"),
        ],
        Duration::ZERO,
    );

    for u in &out {
        println!("speaking: {:?}", u.text);
        sink.speak(u)?;
        std::thread::sleep(Duration::from_millis(1200));
    }
    Ok(())
}
