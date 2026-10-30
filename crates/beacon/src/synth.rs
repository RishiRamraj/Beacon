//! Doom-like stand-ins for Toby's sounds, for a player without a Toby install.
//!
//! Each cue Beacon uses gets a short synthesized sound in the spirit of its Toby original —
//! square-wave blips and buzzes like Doom's own interface sounds, noise ticks for the
//! hazard detectors — so navigation works out of the box and sounds the same family of
//! sounds as the real thing. They are rendered once, on first use, at the device rate.
//!
//! The recipes are deliberately distinct from one another in rhythm and register, because
//! that is what makes cues learnable: the path beacon, a marker and an exit must never be
//! mistaken for each other even played at once.

/// Renders the stand-in for a Toby sound name, or `None` if Beacon has no stand-in for it.
pub fn render(name: &str, rate: u32) -> Option<Vec<f32>> {
    let s = Synth { rate: rate as f32 };
    let out = match name {
        // --- Pathfinder: the carrot and the route's events.
        "pathfinder/nodebeacon" => s.seq(&[
            (s.square(660.0, 0.045, 0.6), 0.03),
            (s.square(660.0, 0.045, 0.6), 0.0),
        ]),
        "pathfinder/nodebeaconfast" => s.seq(&[
            (s.square(990.0, 0.03, 0.6), 0.02),
            (s.square(990.0, 0.03, 0.6), 0.02),
            (s.square(990.0, 0.03, 0.6), 0.0),
        ]),
        "pathfinder/nodereached" => s.chirp(500.0, 1000.0, 0.08, 0.6),
        "pathfinder/destreached" => s.seq(&[
            (s.square(523.0, 0.09, 0.5), 0.0),
            (s.square(659.0, 0.09, 0.5), 0.0),
            (s.square(784.0, 0.16, 0.5), 0.0),
        ]),
        "pathfinder/pathfound" => s.seq(&[
            (s.square(440.0, 0.07, 0.5), 0.0),
            (s.square(660.0, 0.1, 0.5), 0.0),
        ]),
        "pathfinder/nopath" => s.seq(&[
            (s.square(330.0, 0.1, 0.5), 0.02),
            (s.square(220.0, 0.18, 0.5), 0.0),
        ]),
        "pathfinder/markermenu" => s.chirp(700.0, 900.0, 0.05, 0.4),

        // --- Hazards and walls.
        "dropoff/beacon" => s.click(0.012, 0.8, 4000.0),
        "misc/wallhit" => mix(&s.noise(0.06, 0.7, 400.0), &s.sine_decay(90.0, 0.08, 0.8)),
        "toby/hurtfloordetector/rad" => s.click(0.006, 0.7, 6000.0),
        "toby/proximitydetector/wind" => s.wind(1.0, 0.5),

        // --- Markers: one bell each, rising with the number, so two are told apart.
        n if n.starts_with("marker/beacon") => {
            let k: u32 = n["marker/beacon".len()..]
                .parse()
                .ok()
                .filter(|k| (1..=10).contains(k))?;
            s.bell(392.0 * 1.12f32.powi(k as i32 - 1), 0.22, 0.6)
        }
        "marker/markers/undofail" => s.seq(&[
            (s.square(196.0, 0.06, 0.5), 0.03),
            (s.square(196.0, 0.06, 0.5), 0.0),
        ]),
        "marker/automarkerplacednonarration" | "marker/automarkerplaced" => {
            s.bell(1046.0, 0.15, 0.4)
        }

        // --- Exits, doors and switches. Loops are whole cycles so they repeat seamlessly.
        "amb/exit1" => s.hum(110.0, 8.0, 0.5, 0.5),
        "amb/secretexit" => s.hum(147.0, 6.0, 0.5, 0.5),
        "amb/exit2" => s.hum(880.0, 12.0, 0.5, 0.3),
        "amb/door2" => s.chirp(220.0, 120.0, 0.15, 0.6),
        "amb/switch" | "amb/switch2" => s.seq(&[
            (s.click(0.01, 0.8, 2500.0), 0.06),
            (s.click(0.01, 0.8, 2500.0), 0.0),
        ]),
        "amb/rdoor" => s.hum(165.0, 3.0, 0.6, 0.45),
        "amb/bdoor" => s.hum(196.0, 2.0, 0.6, 0.45),
        "door/deniedred" | "door/deniedall" => s.square(110.0, 0.25, 0.5),
        "door/redgranted" => s.chirp(330.0, 660.0, 0.12, 0.5),

        // --- Pickups, by Toby's universal categories.
        "toby/universalbeacons/key" => s.seq(&[
            (s.bell(1318.0, 0.08, 0.5), 0.0),
            (s.bell(1760.0, 0.12, 0.5), 0.0),
        ]),
        "toby/universalbeacons/health" => s.seq(&[
            (s.bell(784.0, 0.08, 0.5), 0.0),
            (s.bell(988.0, 0.12, 0.5), 0.0),
        ]),
        "toby/universalbeacons/ammo" => s.seq(&[
            (s.click(0.01, 0.7, 3000.0), 0.04),
            (s.bell(1175.0, 0.08, 0.4), 0.0),
        ]),
        "toby/universalbeacons/armor" => s.bell(587.0, 0.2, 0.5),
        "toby/universalbeacons/weapon" => s.seq(&[
            (s.square(392.0, 0.06, 0.4), 0.0),
            (s.square(523.0, 0.1, 0.4), 0.0),
        ]),
        "misc/key" => s.bell(1568.0, 0.1, 0.4),

        // --- Targeting.
        "weapons/lock" => s.square(1200.0, 0.025, 0.4),

        _ => return None,
    };
    Some(out)
}

/// Sums two sounds of possibly different lengths.
fn mix(a: &[f32], b: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0; a.len().max(b.len())];
    for (i, o) in out.iter_mut().enumerate() {
        *o = a.get(i).copied().unwrap_or(0.0) + b.get(i).copied().unwrap_or(0.0);
    }
    out
}

struct Synth {
    rate: f32,
}

impl Synth {
    fn len(&self, secs: f32) -> usize {
        (secs * self.rate) as usize
    }

    /// A short attack and release, so a tone starts and ends without a click.
    fn envelope(&self, i: usize, n: usize) -> f32 {
        let edge = self.len(0.004).max(1);
        let a = (i as f32 / edge as f32).min(1.0);
        let r = ((n - i) as f32 / edge as f32).min(1.0);
        a.min(r)
    }

    /// A softened square wave: Doom's interface sounds are buzzy, not pure.
    fn square(&self, freq: f32, secs: f32, amp: f32) -> Vec<f32> {
        let n = self.len(secs);
        (0..n)
            .map(|i| {
                let t = i as f32 / self.rate;
                let s = (t * freq * std::f32::consts::TAU).sin();
                // tanh of a driven sine: square-ish, without the harshest harmonics.
                (s * 3.0).tanh() * amp * 0.6 * self.envelope(i, n)
            })
            .collect()
    }

    /// A glide from one pitch to another.
    fn chirp(&self, from: f32, to: f32, secs: f32, amp: f32) -> Vec<f32> {
        let n = self.len(secs);
        let mut phase = 0.0f32;
        (0..n)
            .map(|i| {
                let f = from + (to - from) * i as f32 / n as f32;
                phase += f / self.rate;
                let s = (phase * std::f32::consts::TAU).sin();
                (s * 2.0).tanh() * amp * 0.6 * self.envelope(i, n)
            })
            .collect()
    }

    /// A struck bell: a fundamental and an inharmonic partial, decaying.
    fn bell(&self, freq: f32, secs: f32, amp: f32) -> Vec<f32> {
        let n = self.len(secs);
        (0..n)
            .map(|i| {
                let t = i as f32 / self.rate;
                let decay = (-t * 5.0 / secs).exp();
                let s = (t * freq * std::f32::consts::TAU).sin()
                    + 0.4 * (t * freq * 2.76 * std::f32::consts::TAU).sin();
                s * 0.7 * amp * decay * self.envelope(i, n)
            })
            .collect()
    }

    /// A low decaying sine, the body of a thud.
    fn sine_decay(&self, freq: f32, secs: f32, amp: f32) -> Vec<f32> {
        let n = self.len(secs);
        (0..n)
            .map(|i| {
                let t = i as f32 / self.rate;
                (t * freq * std::f32::consts::TAU).sin()
                    * amp
                    * (-t * 4.0 / secs).exp()
                    * self.envelope(i, n)
            })
            .collect()
    }

    /// White noise through a one-pole low-pass at `cutoff`, decaying.
    fn noise(&self, secs: f32, amp: f32, cutoff: f32) -> Vec<f32> {
        let n = self.len(secs);
        let mut rng = Lcg(0x1234_5678);
        let k = (cutoff / self.rate * std::f32::consts::TAU).min(1.0);
        let mut y = 0.0f32;
        (0..n)
            .map(|i| {
                y += k * (rng.next() - y);
                let t = i as f32 / self.rate;
                y * amp * 2.0 * (-t * 5.0 / secs).exp() * self.envelope(i, n)
            })
            .collect()
    }

    /// A very short bright tick: the Geiger click and the drop-off warning.
    fn click(&self, secs: f32, amp: f32, cutoff: f32) -> Vec<f32> {
        let mut rng = Lcg(0x0bad_cafe);
        let n = self.len(secs).max(8);
        let k = (cutoff / self.rate * std::f32::consts::TAU).min(1.0);
        let mut y = 0.0f32;
        let mut prev = 0.0f32;
        (0..n)
            .map(|i| {
                y += k * (rng.next() - y);
                // High-pass the low-passed noise: a band, bright but not hissy.
                let hp = y - prev;
                prev = y;
                hp * (1.0 - i as f32 / n as f32)
            })
            .collect::<Vec<f32>>()
            .normalized(amp)
    }

    /// A droning loop with a tremolo, made of whole cycles of both so it repeats seamlessly.
    fn hum(&self, freq: f32, trem: f32, secs: f32, amp: f32) -> Vec<f32> {
        // Round the length to whole tremolo cycles, then the pitch to whole cycles in it.
        let cycles_t = (secs * trem).round().max(1.0);
        let secs = cycles_t / trem;
        let freq = (freq * secs).round() / secs;
        let n = self.len(secs);
        (0..n)
            .map(|i| {
                let t = i as f32 / self.rate;
                let s = (t * freq * std::f32::consts::TAU).sin();
                let lfo = 0.65 + 0.35 * (t * trem * std::f32::consts::TAU).sin();
                (s * 2.5).tanh() * amp * 0.5 * lfo
            })
            .collect()
    }

    /// Soft filtered noise, looped: the proximity detector's "wind". Filtered circularly so
    /// the loop point does not click.
    fn wind(&self, secs: f32, amp: f32) -> Vec<f32> {
        let n = self.len(secs);
        let mut rng = Lcg(0x5eed_0001);
        let raw: Vec<f32> = (0..n).map(|_| rng.next()).collect();
        let k = (600.0 / self.rate * std::f32::consts::TAU).min(1.0);
        let mut y = 0.0f32;
        // Two passes round the ring settle the filter, so the end flows into the start.
        let mut out = vec![0.0f32; n];
        for pass in 0..2 {
            for i in 0..n {
                y += k * (raw[i] - y);
                if pass == 1 {
                    out[i] = y * amp * 2.5;
                }
            }
        }
        out
    }

    /// Joins sounds end to end, each followed by its gap in seconds.
    fn seq(&self, parts: &[(Vec<f32>, f32)]) -> Vec<f32> {
        let mut out = Vec::new();
        for (sound, gap) in parts {
            out.extend_from_slice(sound);
            out.extend(std::iter::repeat_n(0.0, self.len(*gap)));
        }
        out
    }
}

/// Scaling a sound to a given peak, for recipes whose raw level is hard to predict.
trait Normalized {
    fn normalized(self, peak: f32) -> Vec<f32>;
}

impl Normalized for Vec<f32> {
    fn normalized(mut self, peak: f32) -> Vec<f32> {
        let max = self.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        if max > 0.0 {
            let k = peak / max;
            self.iter_mut().for_each(|s| *s *= k);
        }
        self
    }
}

/// A small deterministic noise source in [-1, 1].
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUES: &[&str] = &[
        "pathfinder/nodebeacon",
        "pathfinder/nodebeaconfast",
        "pathfinder/nodereached",
        "pathfinder/destreached",
        "pathfinder/pathfound",
        "pathfinder/nopath",
        "dropoff/beacon",
        "misc/wallhit",
        "toby/hurtfloordetector/rad",
        "toby/proximitydetector/wind",
        "marker/beacon1",
        "marker/beacon10",
        "amb/exit1",
        "amb/door2",
        "amb/switch",
        "amb/rdoor",
        "door/deniedred",
        "toby/universalbeacons/key",
        "weapons/lock",
    ];

    #[test]
    fn every_cue_renders_audible_and_unclipped() {
        for name in CUES {
            let s = render(name, 48_000).unwrap_or_else(|| panic!("{name} has no stand-in"));
            assert!(!s.is_empty(), "{name} is empty");
            let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.05, "{name} is near-silent: {peak}");
            assert!(peak <= 1.0, "{name} clips: {peak}");
        }
    }

    #[test]
    fn markers_differ_and_out_of_range_numbers_do_not_exist() {
        assert_ne!(
            render("marker/beacon1", 48_000),
            render("marker/beacon2", 48_000)
        );
        assert!(render("marker/beacon11", 48_000).is_none());
        assert!(render("not/a/cue", 48_000).is_none());
    }
}
