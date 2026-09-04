//! Audio output, and the clock the whole emulator runs on.
//!
//! Audio is what paces emulation. A dropped video frame is a visual hiccup; a
//! starved audio buffer is a click, and for a player navigating by sound a
//! click is indistinguishable from a cue. So the frame loop runs as fast as the
//! audio queue drains and no faster.

use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    /// Samples consumed per second: the device's rate times its channel count. What turns a
    /// queue depth into an amount of TIME, which is what the event loop needs to know in
    /// order to sleep rather than spin.
    samples_per_sec: usize,
    // Held to keep the device alive; dropping this stops playback.
    _stream: cpal::Stream,
}

/// Builds an output stream of one sample type, feeding it from the shared queue.
///
/// Generic over the type because the device names it and Beacon does not get to choose;
/// `FromSample` converts the emulator's f32 into whatever that is.
fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    shared: &Arc<Mutex<Shared>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let shared = Arc::clone(shared);
    device.build_output_stream(
        config,
        move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
            let Ok(mut s) = shared.lock() else {
                out.fill(T::EQUILIBRIUM);
                return;
            };

            for frame in out.chunks_mut(channels) {
                // The emulator is stereo. Mono devices take the left channel; anything wider
                // gets silence in the extra channels rather than a wrong-sounding upmix.
                let l = s.queue.pop_front();
                let r = if channels > 1 {
                    s.queue.pop_front()
                } else {
                    None
                };

                match l {
                    Some(l) => {
                        frame[0] = T::from_sample(l);
                        if channels > 1 {
                            frame[1] = T::from_sample(r.unwrap_or(l));
                        }
                        for extra in frame.iter_mut().skip(2) {
                            *extra = T::EQUILIBRIUM;
                        }
                    }
                    None => {
                        s.underruns += 1;
                        frame.fill(T::EQUILIBRIUM);
                    }
                }
            }
        },
        |err| eprintln!("audio stream error: {err}"),
        None,
    )
}

impl Audio {
    /// Opens the default output device, exactly as the device describes itself.
    ///
    /// Nothing here is negotiated, and that is the point. Beacon used to build a stream
    /// configuration of its own and hand it over, which worked on Linux because ALSA and
    /// PulseAudio quietly convert whatever they are given. WASAPI in shared mode does not: it
    /// serves the device's own mix format and refuses anything else, so Beacon died on Windows
    /// with "Stream configuration is not supported in shared mode" before a window appeared.
    ///
    /// Fixing only the sample RATE was not enough — that was the difference I could see, and I
    /// assumed it was the whole of it. The mix format is a rate AND a channel count AND a
    /// sample type, and any one of them being wrong is refused the same way. So the device's
    /// own configuration is used whole, and the sample type it names decides which stream is
    /// built. That cannot mismatch, because none of it was chosen here.
    ///
    /// The rate that falls out is reported by [`Audio::sample_rate`] and handed to the
    /// emulator, which resamples to meet it — libsamplerate is vendored into bsnes-jg, so
    /// that is its job rather than something Beacon does again on the way out.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;

        let supported = device.default_output_config()?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let sample_rate = config.sample_rate;

        // Said out loud every time, because it is the first thing worth knowing when audio
        // fails on a machine nobody can reach: what the device actually asked for.
        eprintln!("audio: {sample_rate} Hz, {channels} channel(s), {sample_format:?}");

        let target = (sample_rate as f32 * TARGET_QUEUED_SECONDS) as usize * channels.max(1);

        let shared = Arc::new(Mutex::new(Shared {
            queue: std::collections::VecDeque::with_capacity(target * 4),
            underruns: 0,
        }));

        // Every sample type a mix format is likely to be. Converting from the emulator's f32
        // is `FromSample`'s job, so each arm is the same code at a different type.
        let stream = match sample_format {
            SampleFormat::F32 => build::<f32>(&device, config, channels, &shared),
            SampleFormat::F64 => build::<f64>(&device, config, channels, &shared),
            SampleFormat::I16 => build::<i16>(&device, config, channels, &shared),
            SampleFormat::I32 => build::<i32>(&device, config, channels, &shared),
            SampleFormat::U8 => build::<u8>(&device, config, channels, &shared),
            SampleFormat::U16 => build::<u16>(&device, config, channels, &shared),
            other => {
                return Err(
                    format!("audio device wants an unsupported sample format: {other:?}").into(),
                )
            }
        }?;

        stream.play()?;

        Ok(Audio {
            shared,
            sample_rate,
            high_water: target * 2,
            samples_per_sec: (sample_rate as usize) * channels.max(1),
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

    /// How long the emulator may do nothing at all before the audio queue needs refilling.
    ///
    /// The pacing rule above says WHETHER to produce a frame; this says how long until the
    /// answer could change, which is what lets the event loop wait instead of asking again
    /// as fast as the CPU allows. Zero when the queue is at or below the mark: produce now.
    pub fn headroom(&self) -> Duration {
        let queued = self.shared.lock().map(|s| s.queue.len()).unwrap_or(0);
        let spare = queued.saturating_sub(self.high_water);
        if spare == 0 || self.samples_per_sec == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(spare as f64 / self.samples_per_sec as f64)
    }

    pub fn underruns(&self) -> u64 {
        self.shared.lock().map(|s| s.underruns).unwrap_or(0)
    }
}
