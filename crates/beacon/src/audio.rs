//! Audio output, and the clock the whole emulator runs on.
//!
//! Audio is what paces emulation. A dropped video frame is a visual hiccup; a
//! starved audio buffer is a click, and for a player navigating by sound a
//! click is indistinguishable from a cue. So the frame loop runs as fast as the
//! audio queue drains and no faster.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// How much audio to keep queued ahead of the output device.
///
/// Roughly 100 ms at 48 kHz stereo. Long enough to absorb a slow frame, short
/// enough that input does not feel detached from sound.
const TARGET_QUEUED_SAMPLES: usize = 48_000 * 2 / 10;

/// Above this the emulator is running ahead and should wait.
const HIGH_WATER: usize = TARGET_QUEUED_SAMPLES * 2;

/// Shared between the frame loop and the audio callback.
struct Shared {
    /// Interleaved stereo samples awaiting playback.
    queue: std::collections::VecDeque<f32>,
    /// Times the device wanted samples we did not have. Surfaced rather than
    /// hidden: underruns mean the machine cannot keep up, which is exactly the
    /// low-end-hardware question the design left open.
    underruns: u64,
}

pub struct Audio {
    shared: Arc<Mutex<Shared>>,
    // Held to keep the device alive; dropping this stops playback.
    _stream: cpal::Stream,
}

impl Audio {
    /// Opens the default output device at the emulator's sample rate.
    pub fn new(sample_rate: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;

        let config = device.default_output_config()?;
        let channels = config.channels() as usize;

        // The emulator produces stereo at a fixed rate. Rather than resample,
        // ask the device for the emulator's rate and let cpal pick the closest
        // supported configuration.
        let stream_config = cpal::StreamConfig {
            channels: config.channels(),
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let shared = Arc::new(Mutex::new(Shared {
            queue: std::collections::VecDeque::with_capacity(HIGH_WATER * 2),
            underruns: 0,
        }));

        let cb_shared = Arc::clone(&shared);
        let stream = device.build_output_stream(
            stream_config,
            move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let Ok(mut s) = cb_shared.lock() else {
                    out.fill(0.0);
                    return;
                };

                for frame in out.chunks_mut(channels) {
                    // The emulator is stereo. Mono devices take the left
                    // channel; anything wider gets silence in the extra
                    // channels rather than a wrong-sounding upmix.
                    let l = s.queue.pop_front();
                    let r = if channels > 1 {
                        s.queue.pop_front()
                    } else {
                        None
                    };

                    match l {
                        Some(l) => {
                            frame[0] = l;
                            if channels > 1 {
                                frame[1] = r.unwrap_or(l);
                            }
                            for extra in frame.iter_mut().skip(2) {
                                *extra = 0.0;
                            }
                        }
                        None => {
                            s.underruns += 1;
                            frame.fill(0.0);
                        }
                    }
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )?;

        stream.play()?;

        Ok(Audio {
            shared,
            _stream: stream,
        })
    }

    /// Queues samples produced by a frame.
    pub fn submit(&self, samples: &[f32]) {
        if let Ok(mut s) = self.shared.lock() {
            s.queue.extend(samples.iter().copied());
        }
    }

    /// Whether the emulator has run far enough ahead that it should wait.
    ///
    /// This is the pacing mechanism: rather than sleeping against a wall clock
    /// and drifting relative to the audio device, the frame loop simply stops
    /// producing once the queue is full.
    pub fn is_ahead(&self) -> bool {
        self.shared
            .lock()
            .map(|s| s.queue.len() >= HIGH_WATER)
            .unwrap_or(false)
    }

    pub fn underruns(&self) -> u64 {
        self.shared.lock().map(|s| s.underruns).unwrap_or(0)
    }
}
