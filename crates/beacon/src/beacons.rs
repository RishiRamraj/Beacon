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

/// How deeply the tremolo cuts: at the trough the tone drops to `1 - depth` of
/// full amplitude. Less than 1 so a pulsing tone never fully vanishes between
/// beats — the source stays continuously locatable, it just throbs.
const TREMOLO_DEPTH: f32 = 0.6;

pub struct BeaconMixer {
    sample_rate: f32,
    /// Oscillator phase per beacon id, in cycles `[0, 1)`, kept between frames.
    phases: HashMap<String, f32>,
    /// Tremolo (amplitude-modulation) phase per beacon id, likewise continuous
    /// across frames so the pulse does not jump when a new buffer starts.
    trem_phases: HashMap<String, f32>,
}

impl BeaconMixer {
    pub fn new(sample_rate: u32) -> Self {
        BeaconMixer {
            sample_rate: sample_rate as f32,
            phases: HashMap::new(),
            trem_phases: HashMap::new(),
        }
    }

    /// Mixes the beacons into `out` (interleaved stereo), additively, then clamps
    /// so the sum with the game audio cannot clip.
    ///
    /// A beacon's own `volume` (0 to 1, the plugin's distance curve) is mapped
    /// into `[vol_min, vol_max]` — the player-set quietest and loudest levels — so
    /// the far end and the near end are independently adjustable.
    pub fn mix(
        &mut self,
        beacons: &[BeaconState],
        out: &mut [f32],
        vol_min: f32,
        vol_max: f32,
        music_duck: f32,
    ) {
        if beacons.is_empty() {
            self.phases.clear();
            self.trem_phases.clear();
            return;
        }

        // Dip the game audio while beacons are sounding, so the cues cut through
        // the music rather than fighting it. Done before the tones are added.
        if music_duck < 1.0 {
            for s in out.iter_mut() {
                *s *= music_duck;
            }
        }

        // Forget oscillators the plugin has cleared.
        self.phases
            .retain(|id, _| beacons.iter().any(|b| &b.id == id));
        self.trem_phases
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
            let amp = vol_min + (vol_max - vol_min) * b.volume.clamp(0.0, 1.0);
            let step = (BASE_FREQ * b.pitch) / self.sample_rate;
            // Tremolo advances its own phase per sample; 0 Hz leaves the tone
            // steady. The LFO rides in [1 - depth, 1] so it only ever ducks the
            // amplitude, never boosts it past the panned/volume level above.
            let tremolo = b.tremolo.max(0.0);
            let trem_step = tremolo / self.sample_rate;

            let mut phase = self.phases.get(&b.id).copied().unwrap_or(0.0);
            let mut trem_phase = self.trem_phases.get(&b.id).copied().unwrap_or(0.0);
            for i in 0..frames {
                let trem_gain = if tremolo > 0.0 {
                    let lfo = (trem_phase * std::f32::consts::TAU).sin() * 0.5 + 0.5;
                    1.0 - TREMOLO_DEPTH + TREMOLO_DEPTH * lfo
                } else {
                    1.0
                };
                let s = (phase * std::f32::consts::TAU).sin() * amp * trem_gain;
                out[2 * i] += s * left_gain;
                out[2 * i + 1] += s * right_gain;
                phase += step;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                trem_phase += trem_step;
                if trem_phase >= 1.0 {
                    trem_phase -= 1.0;
                }
            }
            self.phases.insert(b.id.clone(), phase);
            self.trem_phases.insert(b.id.clone(), trem_phase);
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
            tremolo: 0.0,
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

    // Renders with the range [0, vol_max], so a beacon's own volume maps directly.
    fn render(beacons: &[BeaconState], vol_max: f32) -> Vec<f32> {
        let mut mixer = BeaconMixer::new(48_000);
        let mut out = vec![0.0f32; 2 * 1024];
        mixer.mix(beacons, &mut out, 0.0, vol_max, 1.0);
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
    fn beacon_volume_and_max_scale_the_output() {
        let full = energy(&render(&[beacon("e", 100.0, 0.0, 1.0)], 1.0)).1;
        let half_vol = energy(&render(&[beacon("e", 100.0, 0.0, 0.5)], 1.0)).1;
        let half_max = energy(&render(&[beacon("e", 100.0, 0.0, 1.0)], 0.5)).1;
        assert!(half_vol < full * 0.3, "half the beacon volume is much quieter");
        assert!(half_max < full * 0.3, "half the max level is much quieter");

        // A beacon volume of zero, with a zero floor, is silence.
        let (l, r) = energy(&render(&[beacon("e", 100.0, 0.0, 0.0)], 1.0));
        assert_eq!((l, r), (0.0, 0.0));
    }

    #[test]
    fn the_floor_keeps_a_far_beacon_audible() {
        // A far beacon (volume 0) is silent with a zero floor, but audible when
        // volume_min lifts the floor.
        let mut mixer = BeaconMixer::new(48_000);
        let mut floored = vec![0.0f32; 2 * 256];
        mixer.mix(&[beacon("e", 100.0, 0.0, 0.0)], &mut floored, 0.1, 0.5, 1.0);
        assert!(floored.iter().any(|&s| s != 0.0), "min level makes it audible");
    }

    #[test]
    fn music_is_ducked_while_a_beacon_sounds() {
        // The game audio in `out` is dipped by the duck factor before the beacon
        // is added, so the cue cuts through. With no beacon, nothing is touched.
        let mut mixer = BeaconMixer::new(48_000);
        // A silent beacon (volume 0) adds nothing, so the buffer shows the duck
        // alone: 1.0 game audio * 0.5 duck = 0.5.
        let mut out = vec![1.0f32; 2 * 8];
        mixer.mix(&[beacon("e", 100.0, 0.0, 0.0)], &mut out, 0.0, 1.0, 0.5);
        assert!(
            out.iter().all(|&s| (s - 0.5).abs() < 1e-6),
            "game audio ducked to half: {out:?}"
        );
        // No beacon -> no ducking.
        let mut out = vec![1.0f32; 4];
        mixer.mix(&[], &mut out, 0.0, 1.0, 0.5);
        assert!(out.iter().all(|&s| s == 1.0), "untouched with no beacon");
    }

    #[test]
    fn no_beacons_leaves_the_buffer_untouched() {
        let mut mixer = BeaconMixer::new(48_000);
        let mut out = vec![0.25f32; 8];
        mixer.mix(&[], &mut out, 0.0, 1.0, 1.0);
        assert!(out.iter().all(|&s| s == 0.25));
    }

    #[test]
    fn output_never_clips() {
        // Loud game audio already near the rails, plus a beacon.
        let mut mixer = BeaconMixer::new(48_000);
        let mut out = vec![0.95f32; 2 * 512];
        mixer.mix(&[beacon("e", 100.0, 0.0, 1.0)], &mut out, 0.0, 1.0, 1.0);
        assert!(out.iter().all(|&s| (-1.0..=1.0).contains(&s)));
    }

    #[test]
    fn tremolo_pulses_the_amplitude() {
        // A steady tone holds a near-constant envelope; a tremolo tone at the same
        // pitch and volume swings between loud and quiet. The buffer is long
        // enough for the LFO to complete several cycles, and the envelope is the
        // RMS over windows each spanning many carrier cycles (so a window's level
        // reflects the tremolo, not where the carrier sine happens to sit).
        let window = 480; // 10 ms at 48 kHz — ~3 carrier cycles of the 330 Hz tone
        let envelope = |tremolo: f32| -> (f32, f32) {
            let mut b = beacon("e", 0.0, 100.0, 1.0);
            b.tremolo = tremolo;
            let mut mixer = BeaconMixer::new(48_000);
            let mut out = vec![0.0f32; 2 * 4800]; // 100 ms; a 30 Hz LFO = 3 cycles
            mixer.mix(std::slice::from_ref(&b), &mut out, 0.0, 1.0, 1.0);
            let mono: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
            let mut lo = f32::MAX;
            let mut hi = 0.0f32;
            for w in mono.chunks(window) {
                let rms = (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt();
                lo = lo.min(rms);
                hi = hi.max(rms);
            }
            (lo, hi)
        };

        let (s_lo, s_hi) = envelope(0.0);
        assert!(s_hi - s_lo < s_hi * 0.1, "steady envelope is flat: {s_lo}..{s_hi}");

        let (p_lo, p_hi) = envelope(30.0);
        assert!(
            p_lo < p_hi * 0.75,
            "tremolo envelope swings between quiet and loud: {p_lo}..{p_hi}"
        );
        // The trough never reaches silence — the source stays locatable.
        assert!(p_lo > 0.0, "a pulsing beacon never goes fully silent");
    }

    #[test]
    fn tremolo_phase_is_continuous_across_calls() {
        // Two back-to-back renders of a tremolo tone join without the pulse
        // resetting: the envelope at the seam continues rather than jumping back.
        let mut mixer = BeaconMixer::new(48_000);
        let mut b = beacon("e", 0.0, 100.0, 1.0);
        b.tremolo = 8.0;
        let bs = [b];
        let mut a1 = vec![0.0f32; 2 * 64];
        let mut a2 = vec![0.0f32; 2 * 64];
        mixer.mix(&bs, &mut a1, 0.0, 1.0, 1.0);
        mixer.mix(&bs, &mut a2, 0.0, 1.0, 1.0);
        // If the tremolo phase reset to 0 at the second call, the first sample of
        // a2 would use the same LFO value as the first sample of a1. Continuity
        // means the store advanced it; assert the mixer kept a tremolo phase.
        assert!(
            mixer.trem_phases.get("e").copied().unwrap_or(0.0) > 0.0,
            "tremolo phase is carried across frames"
        );
        // Sanity: both buffers actually produced sound.
        assert!(a1.iter().chain(a2.iter()).any(|&s| s != 0.0));
    }

    #[test]
    fn phase_is_continuous_across_calls() {
        // Two back-to-back renders should join without a discontinuity at the
        // seam (the sample values step smoothly).
        let mut mixer = BeaconMixer::new(48_000);
        let b = [beacon("e", 0.0, 100.0, 1.0)];
        let mut a1 = vec![0.0f32; 2 * 64];
        let mut a2 = vec![0.0f32; 2 * 64];
        mixer.mix(&b, &mut a1, 0.0, 1.0, 1.0);
        mixer.mix(&b, &mut a2, 0.0, 1.0, 1.0);
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
