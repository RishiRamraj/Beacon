//! Audio output, and the clock the whole emulator runs on.
//!
//! Audio is what paces emulation. A dropped video frame is a visual hiccup; a
//! starved audio buffer is a click, and for a player navigating by sound a
//! click is indistinguishable from a cue. So the frame loop runs as fast as the
//! audio queue drains and no faster.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

/// How much audio to keep queued ahead of the output device, as a fraction of a second.
///
/// Long enough to absorb a slow frame, short enough that input does not feel detached from
/// sound. Derived from the rate the device actually runs at rather than assumed, or the whole
/// pacing loop would be wrong by the ratio between them.
const TARGET_QUEUED_SECONDS: f32 = 0.1;

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
    /// The rate the device is actually running at, which the emulator is told to match.
    sample_rate: u32,
    /// The queue length above which the emulator has run far enough ahead to wait. Derived
    /// from the device's rate, so pacing does not depend on it being 48kHz.
    high_water: usize,
    // Held to keep the device alive; dropping this stops playback.
    _stream: cpal::Stream,
}

impl Audio {
    /// Opens the default output device, preferring `preferred` where the device allows it.
    ///
    /// The rate is the DEVICE's to choose, not Beacon's. Asking for an arbitrary one and
    /// hoping worked on Linux, where ALSA and PulseAudio resample without saying so, and
    /// failed outright on Windows: WASAPI in shared mode serves only the device's own mix
    /// format and rejects anything else — "Stream configuration is not supported in shared
    /// mode", which killed Beacon on startup before a window ever appeared.
    ///
    /// So the supported configurations are asked for, `preferred` is used only if one of them
    /// covers it, and otherwise the device's default stands. Whatever comes out of that is
    /// reported by [`Audio::sample_rate`] and handed to the emulator, which resamples to meet
    /// it. cpal does not negotiate on a caller's behalf, whatever the old comment here hoped.
    pub fn new(preferred: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;

        let config = device.default_output_config()?;
        let channels = config.channels() as usize;

        let supported = device
            .supported_output_configs()
            .map(|mut configs| {
                configs.any(|range| {
                    range.sample_format() == SampleFormat::F32
                        && range.channels() == config.channels()
                        && range.min_sample_rate() <= preferred
                        && preferred <= range.max_sample_rate()
                })
            })
            .unwrap_or(false);

        let sample_rate = if supported {
            preferred
        } else {
            config.sample_rate()
        };
        if sample_rate != preferred {
            eprintln!(
                "audio: device runs at {sample_rate} Hz, not {preferred}; emulating to match"
            );
        }

        let stream_config = cpal::StreamConfig {
            channels: config.channels(),
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let target = (sample_rate as f32 * TARGET_QUEUED_SECONDS) as usize * channels.max(1);

        let shared = Arc::new(Mutex::new(Shared {
            queue: std::collections::VecDeque::with_capacity(target * 4),
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
            sample_rate,
            high_water: target * 2,
            _stream: stream,
        })
    }

    /// The rate the device settled on. The emulator is created to match it.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
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
            .map(|s| s.queue.len() >= self.high_water)
            .unwrap_or(false)
    }

    pub fn underruns(&self) -> u64 {
        self.shared.lock().map(|s| s.underruns).unwrap_or(0)
    }
}
