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

/// The device period to ask for, in milliseconds: well under a frame (16.7 ms), so the device
/// drains the queue a little at a time rather than in lumps.
const PERIOD_MS: usize = 10;

/// Shared between the frame loop and the audio callback.
struct Shared {
    /// Interleaved stereo samples awaiting playback.
    queue: std::collections::VecDeque<f32>,
    /// Times the device wanted samples we did not have. Surfaced rather than
    /// hidden: underruns mean the machine cannot keep up, which is exactly the
    /// low-end-hardware question the design left open.
    underruns: u64,
    /// The most samples the device has asked for at once: its real period, whatever was
    /// requested. Reported, because a device taking large chunks is what makes emulation run
    /// in bursts — several frames back to back, and only the last of them shown.
    largest_request: usize,
}

pub struct Audio {
    shared: Arc<Mutex<Shared>>,
    /// The rate the device is actually running at, which the emulator is told to match.
    sample_rate: u32,
    /// The queue length above which the emulator has run far enough ahead to wait. Derived
    /// from the device's rate, so pacing does not depend on it being 48kHz.
    ///
    /// This is the latency of everything heard: the queue sits at this depth in play, so a
    /// sample waits this long behind it. It was once double what its comment claimed — a
    /// 100 ms target with a `* 2` on the end — and every sound came a fifth of a second
    /// after the button that caused it.
    high_water: usize,
    /// The device's channel count, which the mix is interleaved in.
    channels: usize,
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
            s.largest_request = s.largest_request.max(out.len() / channels.max(1));

            // The mix is built in the device's own channel count (see `Audio::channels`), so
            // each frame is taken as it is: stereo for a stereo device, all six for 5.1.
            for frame in out.chunks_mut(channels) {
                let mut short = false;
                for slot in frame.iter_mut() {
                    match s.queue.pop_front() {
                        Some(x) => *slot = T::from_sample(x),
                        None => {
                            short = true;
                            *slot = T::EQUILIBRIUM;
                        }
                    }
                }
                if short {
                    s.underruns += 1;
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
    ///
    /// `buffer` is how much audio to keep queued ahead of the device; see
    /// [`Audio::set_buffer`].
    pub fn new(buffer: Duration) -> Result<Self, Box<dyn std::error::Error>> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;

        let supported = device.default_output_config()?;
        let sample_format = supported.sample_format();
        let buffer_range = supported.buffer_size().clone();
        let mut config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let sample_rate = config.sample_rate;

        // Said out loud every time, because it is the first thing worth knowing when audio
        // fails on a machine nobody can reach: what the device actually asked for.
        eprintln!("audio: {sample_rate} Hz, {channels} channel(s), {sample_format:?}");

        let samples_per_sec = (sample_rate as usize) * channels.max(1);
        let high_water = queue_depth(buffer, samples_per_sec);

        let shared = Arc::new(Mutex::new(Shared {
            queue: std::collections::VecDeque::with_capacity(high_water * 4),
            underruns: 0,
            largest_request: 0,
        }));

        // Every sample type a mix format is likely to be. Converting from the emulator's f32
        // is `FromSample`'s job, so each arm is the same code at a different type.
        if !matches!(
            sample_format,
            SampleFormat::F32
                | SampleFormat::F64
                | SampleFormat::I16
                | SampleFormat::I32
                | SampleFormat::U8
                | SampleFormat::U16
        ) {
            return Err(
                format!("audio device wants an unsupported sample format: {sample_format:?}")
                    .into(),
            );
        }
        let open = |config: cpal::StreamConfig| match sample_format {
            SampleFormat::F32 => build::<f32>(&device, config, channels, &shared),
            SampleFormat::F64 => build::<f64>(&device, config, channels, &shared),
            SampleFormat::I16 => build::<i16>(&device, config, channels, &shared),
            SampleFormat::I32 => build::<i32>(&device, config, channels, &shared),
            SampleFormat::U8 => build::<u8>(&device, config, channels, &shared),
            SampleFormat::U16 => build::<u16>(&device, config, channels, &shared),
            _ => unreachable!("checked above"),
        };

        // Ask for a short period. Left to itself the device picks its own, and on PipeWire's
        // ALSA bridge that was large enough that each request drained several frames' worth of
        // audio at once: emulation then ran in bursts to refill it, the window showed one
        // frame in five, and sound arrived in lumps. A short period means one frame produced,
        // and shown, per frame consumed. It is only a request — a device that refuses it
        // (WASAPI shared mode serves its own period) is opened as it describes itself.
        let wanted = (sample_rate as usize * PERIOD_MS / 1000) as u32;
        let fixed = match buffer_range {
            cpal::SupportedBufferSize::Range { min, max } => Some(wanted.clamp(min, max)),
            cpal::SupportedBufferSize::Unknown => Some(wanted),
        };
        let stream = match fixed {
            Some(frames) => {
                let mut short = config.clone();
                short.buffer_size = cpal::BufferSize::Fixed(frames);
                match open(short) {
                    Ok(stream) => {
                        eprintln!("audio: asked for a {frames}-frame period");
                        stream
                    }
                    Err(e) => {
                        eprintln!("audio: a {frames}-frame period was refused ({e}); using the device's own");
                        config.buffer_size = cpal::BufferSize::Default;
                        open(config)?
                    }
                }
            }
            None => open(config)?,
        };

        stream.play()?;

        Ok(Audio {
            shared,
            sample_rate,
            high_water,
            samples_per_sec,
            channels,
            _stream: stream,
        })
    }

    /// How many channels the device takes: the mix is built in this many (a 5.1 system is six).
    pub fn channels(&self) -> usize {
        self.channels.max(1)
    }

    /// The rate the device settled on. The emulator is created to match it.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Changes how much audio is kept queued ahead of the device, live.
    ///
    /// Shrinking it takes effect as the queue drains below the new mark; nothing queued is
    /// thrown away, because dropping samples is a click.
    pub fn set_buffer(&mut self, buffer: Duration) {
        self.high_water = queue_depth(buffer, self.samples_per_sec);
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

    /// The largest chunk, in frames, the device has asked for: its effective period.
    pub fn largest_request(&self) -> usize {
        self.shared.lock().map(|s| s.largest_request).unwrap_or(0)
    }

    pub fn underruns(&self) -> u64 {
        self.shared.lock().map(|s| s.underruns).unwrap_or(0)
    }
}

/// A duration of audio as a queue length, in interleaved samples.
fn queue_depth(buffer: Duration, samples_per_sec: usize) -> usize {
    (buffer.as_secs_f64() * samples_per_sec as f64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_depth_is_the_buffer_with_no_hidden_multiplier() {
        // 50 ms of 48 kHz stereo is 4800 interleaved samples. Not 9600: the doubling that
        // made every sound a fifth of a second late must not come back.
        assert_eq!(queue_depth(Duration::from_millis(50), 48_000 * 2), 4800);
    }
}
