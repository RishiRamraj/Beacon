//! Spatial-audio beacons: turning positioned tones into stereo sound.
//!
//! A plugin places [`BeaconState`]s — a tone with a direction and a loudness.
//! This mixer renders them as sine tones, panned left/right by direction and
//! scaled by the plugin's volume, and adds them to the game audio. It is the
//! simple, dependency-free first step of the design's spatial audio: constant-
//! power stereo panning, not HRTF. Front and back cannot be told apart in stereo
//! — that is what the spoken cue ("Enemy north") is for — and an HRTF renderer
//! can slot in behind this same interface later.
//!
//! Each beacon keeps a continuous oscillator phase across frames, so a moving
//! source glides rather than clicking. Sound is synthesised here, on the frame
//! path, and mixed into the buffer the audio thread plays — never on the audio
//! thread itself.

use std::collections::HashMap;

use beacon_plugin::BeaconState;

/// The reference tone, an unobtrusive mid pitch. A beacon's `pitch` scales it.
const BASE_FREQ: f32 = 330.0;
/// Per-beacon amplitude before the master volume, leaving headroom for several
/// beacons plus the game audio before the final clamp.
const BEACON_AMPLITUDE: f32 = 0.5;

pub struct BeaconMixer {
    sample_rate: f32,
    /// Oscillator phase per beacon id, in cycles `[0, 1)`, kept between frames.
    phases: HashMap<String, f32>,
}

impl BeaconMixer {
    pub fn new(sample_rate: u32) -> Self {
        BeaconMixer {
            sample_rate: sample_rate as f32,
            phases: HashMap::new(),
        }
    }

    /// Mixes the beacons into `out` (interleaved stereo), additively, then clamps
    /// so the sum with the game audio cannot clip. `master` scales everything.
    pub fn mix(&mut self, beacons: &[BeaconState], out: &mut [f32], master: f32) {
        if beacons.is_empty() {
            self.phases.clear();
            return;
        }

        // Forget oscillators the plugin has cleared.
        self.phases
            .retain(|id, _| beacons.iter().any(|b| &b.id == id));

        let frames = out.len() / 2;
        for b in beacons {
            let dist = (b.dx * b.dx + b.dy * b.dy).sqrt();
            // Pan from the left/right ratio; scale-independent, so the host needs
            // no notion of a game's units.
            let pan = if dist > 0.0 {
                (b.dx / dist).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            // Constant-power panning: equal energy at centre, all to one side at
            // the extremes.
            let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
            let (left_gain, right_gain) = (angle.cos(), angle.sin());
            let amp = master * b.volume.clamp(0.0, 1.0) * BEACON_AMPLITUDE;
            let step = (BASE_FREQ * b.pitch) / self.sample_rate;

            let mut phase = self.phases.get(&b.id).copied().unwrap_or(0.0);
            for i in 0..frames {
                let s = (phase * std::f32::consts::TAU).sin() * amp;
                out[2 * i] += s * left_gain;
                out[2 * i + 1] += s * right_gain;
                phase += step;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
            }
            self.phases.insert(b.id.clone(), phase);
        }

        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beacon(id: &str, dx: f32, dy: f32, volume: f32) -> BeaconState {
        BeaconState {
            id: id.to_string(),
            dx,
            dy,
            pitch: 1.0,
            volume,
        }
    }

    /// Energy (sum of squares) per channel of an interleaved stereo buffer.
    fn energy(out: &[f32]) -> (f32, f32) {
        let mut l = 0.0;
        let mut r = 0.0;
        for f in out.chunks(2) {
            l += f[0] * f[0];
            r += f[1] * f[1];
        }
        (l, r)
    }

    fn render(beacons: &[BeaconState], master: f32) -> Vec<f32> {
        let mut mixer = BeaconMixer::new(48_000);
        let mut out = vec![0.0f32; 2 * 1024];
        mixer.mix(beacons, &mut out, master);
        out
    }

    #[test]
    fn a_source_on_the_right_is_louder_on_the_right() {
        let (l, r) = energy(&render(&[beacon("e", 100.0, 0.0, 1.0)], 1.0));
        assert!(r > l * 5.0, "right {r} should dominate left {l}");
    }

    #[test]
    fn a_source_on_the_left_is_louder_on_the_left() {
        let (l, r) = energy(&render(&[beacon("e", -100.0, 0.0, 1.0)], 1.0));
        assert!(l > r * 5.0, "left {l} should dominate right {r}");
    }

    #[test]
    fn a_source_straight_ahead_is_centred() {
        // dx = 0 (only forward): equal power both sides.
        let (l, r) = energy(&render(&[beacon("e", 0.0, 100.0, 1.0)], 1.0));
        assert!((l - r).abs() < l * 0.05, "expected balance, got {l} vs {r}");
    }

    #[test]
    fn volume_and_master_scale_the_output() {
        let full = energy(&render(&[beacon("e", 100.0, 0.0, 1.0)], 1.0)).1;
        let half_vol = energy(&render(&[beacon("e", 100.0, 0.0, 0.5)], 1.0)).1;
        let half_master = energy(&render(&[beacon("e", 100.0, 0.0, 1.0)], 0.5)).1;
        assert!(half_vol < full * 0.3, "half volume should be much quieter");
        assert!(
            half_master < full * 0.3,
            "half master should be much quieter"
        );

        // Zero volume is silence.
        let (l, r) = energy(&render(&[beacon("e", 100.0, 0.0, 0.0)], 1.0));
        assert_eq!((l, r), (0.0, 0.0));
    }

    #[test]
    fn no_beacons_leaves_the_buffer_untouched() {
        let mut mixer = BeaconMixer::new(48_000);
        let mut out = vec![0.25f32; 8];
        mixer.mix(&[], &mut out, 1.0);
        assert!(out.iter().all(|&s| s == 0.25));
    }

    #[test]
    fn output_never_clips() {
        // Loud game audio already near the rails, plus a beacon.
        let mut mixer = BeaconMixer::new(48_000);
        let mut out = vec![0.95f32; 2 * 512];
        mixer.mix(&[beacon("e", 100.0, 0.0, 1.0)], &mut out, 1.0);
        assert!(out.iter().all(|&s| (-1.0..=1.0).contains(&s)));
    }

    #[test]
    fn phase_is_continuous_across_calls() {
        // Two back-to-back renders should join without a discontinuity at the
        // seam (the sample values step smoothly).
        let mut mixer = BeaconMixer::new(48_000);
        let b = [beacon("e", 0.0, 100.0, 1.0)];
        let mut a1 = vec![0.0f32; 2 * 64];
        let mut a2 = vec![0.0f32; 2 * 64];
        mixer.mix(&b, &mut a1, 1.0);
        mixer.mix(&b, &mut a2, 1.0);
        // The step between the last sample of a1 and the first of a2 is no larger
        // than the largest step within a single buffer.
        let max_internal = a1
            .chunks(2)
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        let seam = (a2[0] - a1[a1.len() - 2]).abs();
        assert!(
            seam <= max_internal + 1e-4,
            "seam {seam} vs max {max_internal}"
        );
    }
}
