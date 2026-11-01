//! Placing sounds around the player: stereo, surround speakers, or headphones.
//!
//! Every positioned sound — the guidance beacons, Toby's recordings, the world and enemy cues
//! — reaches the output through here, as a mono block and an offset from the player. North is
//! always in front (the soundscape does not turn with Link), so the azimuth of an offset is
//! measured clockwise from up the screen: east is 90°, south 180°.
//!
//! - **Stereo** pans left and right with constant power, as Beacon always has. Front and back
//!   cannot be told apart in stereo; that is what the spoken legs of the waypoint guide are for.
//! - **Surround** (5.1, 7.1, quad) places a sound between the two speakers either side of it —
//!   pairwise constant-power panning, which is 2-D VBAP — so a sound to the south comes from
//!   the rear speakers. The game's own music stays on the front pair.
//! - **Headphones** is a structural head model (Brown and Duda, 1998), not measured head
//!   responses: each ear hears the sound a fraction of a millisecond apart (the interaural time
//!   difference), the ear facing away hears it with its highs softened (head shadow), and a
//!   sound behind is duller and a little quieter (a stand-in for the outer ear's front-back
//!   cue). It needs no dataset, so nothing with a licence of its own ships, and it is cheap.
//!   Measured responses would tell front from back better; they can replace this behind the
//!   same interface.
//!
//! Each source keeps its gains (and, on headphones, its filter and delay state) between blocks,
//! so a moving source glides rather than clicking.

use std::collections::HashMap;

/// How positioned sounds are rendered.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Stereo,
    /// Speakers, with each one's azimuth in degrees (`None` for the LFE channel, which takes
    /// no positioned sound).
    Surround(Vec<Option<f32>>),
    Headphones,
}

impl Mode {
    /// The mode for a setting (`auto`, `stereo`, `surround`, `headphones`) on a device with
    /// `channels` outputs. `auto` is surround when the device has a layout Beacon knows, stereo
    /// otherwise; asking for surround on a two-channel device is stereo, since there is nothing
    /// else to give.
    pub fn resolve(setting: &str, channels: usize) -> Mode {
        let surround = layout(channels).map(Mode::Surround);
        match setting.trim().to_ascii_lowercase().as_str() {
            "stereo" => Mode::Stereo,
            "headphones" if channels >= 2 => Mode::Headphones,
            "surround" | "auto" | "" => surround.unwrap_or(Mode::Stereo),
            _ => Mode::Stereo,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Mode::Stereo => "stereo",
            Mode::Surround(_) => "surround",
            Mode::Headphones => "headphones",
        }
    }
}

/// The speaker azimuths of a device with `channels` outputs, in its channel order, or `None`
/// for a count Beacon has no layout for.
///
/// The order is the platform's: Windows and macOS put centre and LFE third and fourth (WAVE
/// FORMAT EXTENSIBLE's mask order), ALSA puts the rear pair there. Every layout begins with
/// front left and front right, which is where the game's stereo goes.
fn layout(channels: usize) -> Option<Vec<Option<f32>>> {
    let fl = Some(-30.0);
    let fr = Some(30.0);
    let c = Some(0.0);
    let lfe = None;
    let (bl, br) = (Some(-110.0), Some(110.0));
    let windows_like = cfg!(any(windows, target_os = "macos"));
    Some(match (channels, windows_like) {
        (4, _) => vec![Some(-45.0), Some(45.0), Some(-135.0), Some(135.0)],
        (6, true) => vec![fl, fr, c, lfe, bl, br],
        (6, false) => vec![fl, fr, bl, br, c, lfe],
        (8, true) => vec![fl, fr, c, lfe, Some(-150.0), Some(150.0), Some(-90.0), Some(90.0)],
        (8, false) => vec![fl, fr, Some(-150.0), Some(150.0), c, lfe, Some(-90.0), Some(90.0)],
        _ => return None,
    })
}

/// The azimuth, in radians clockwise from north, of an offset in screen terms (y down).
pub fn azimuth(dx: f32, dy: f32) -> f32 {
    dx.atan2(-dy)
}

/// Pairwise constant-power gains for a sound at `az` (radians) over `speakers`.
fn surround_gains(speakers: &[Option<f32>], az: f32) -> Vec<f32> {
    let mut gains = vec![0.0; speakers.len()];
    let deg = az.to_degrees();
    // The positioned speakers, by azimuth, as a ring.
    let mut ring: Vec<(usize, f32)> = speakers
        .iter()
        .enumerate()
        .filter_map(|(i, a)| a.map(|a| (i, a)))
        .collect();
    ring.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    if ring.is_empty() {
        return gains;
    }
    // The angle, brought into the turn that starts at the first speaker, so the gap across the
    // back (last speaker round to the first) is the last segment.
    let start = ring[0].1;
    let mut d = deg;
    while d < start {
        d += 360.0;
    }
    while d >= start + 360.0 {
        d -= 360.0;
    }
    for k in 0..ring.len() {
        let (i0, a0) = ring[k];
        let (i1, mut a1) = ring[(k + 1) % ring.len()];
        if k + 1 == ring.len() {
            a1 += 360.0;
        }
        if d >= a0 && d <= a1 {
            let t = if a1 > a0 { (d - a0) / (a1 - a0) } else { 0.0 };
            let t = t * std::f32::consts::FRAC_PI_2;
            gains[i0] += t.cos();
            gains[i1] += t.sin();
            return gains;
        }
    }
    gains
}

/// One first-order filter section, `y = (b0 x + b1 x1 - a1 y1)`.
#[derive(Default, Clone)]
struct Section {
    b0: f32,
    b1: f32,
    a1: f32,
    x1: f32,
    y1: f32,
}

impl Section {
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 - self.a1 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }

    /// The head-shadow shelf of Brown and Duda, `(1 + α s / 2ω0) / (1 + s / 2ω0)`, by the
    /// bilinear transform: flat below the head's corner frequency, `α` times above it.
    fn set_shadow(&mut self, alpha: f32, rate: f32) {
        let w0 = SPEED_OF_SOUND / HEAD_RADIUS;
        let k = 2.0 * rate / (2.0 * w0);
        let a0 = 1.0 + k;
        self.b0 = (1.0 + alpha * k) / a0;
        self.b1 = (1.0 - alpha * k) / a0;
        self.a1 = (1.0 - k) / a0;
    }

    /// A one-pole low-pass at `hz`.
    fn set_lowpass(&mut self, hz: f32, rate: f32) {
        let c = (-std::f32::consts::TAU * hz / rate).exp();
        self.b0 = 1.0 - c;
        self.b1 = 0.0;
        self.a1 = -c;
    }
}

const HEAD_RADIUS: f32 = 0.0875; // metres
const SPEED_OF_SOUND: f32 = 343.0; // metres per second
const DELAY_LEN: usize = 64; // samples of history: more than the largest ear delay at 96 kHz

/// The state of one source on headphones: a delay line, and each ear's filters.
#[derive(Clone)]
struct Head {
    history: Vec<f32>,
    at: usize,
    shadow: [Section; 2],
    rear: [Section; 2],
    delay: [f32; 2],
    gain: [f32; 2],
}

impl Head {
    fn new() -> Head {
        Head {
            history: vec![0.0; DELAY_LEN],
            at: 0,
            shadow: Default::default(),
            rear: Default::default(),
            delay: [0.0; 2],
            gain: [0.0; 2],
        }
    }

    /// Sets each ear's delay, shadow and rear filtering for a sound at `az`.
    fn aim(&mut self, az: f32, rate: f32) {
        let behind = (-az.cos()).max(0.0); // 0 in front, 1 dead behind
        for (ear, side) in [(0usize, -1.0f32), (1, 1.0)] {
            // The angle between the sound and this ear, which points straight out sideways.
            let ear_az = side * std::f32::consts::FRAC_PI_2;
            let mut theta = (az - ear_az).abs();
            if theta > std::f32::consts::PI {
                theta = std::f32::consts::TAU - theta;
            }
            // Woodworth's delay: the path round the head, nothing for the near ear.
            let t = if theta < std::f32::consts::FRAC_PI_2 {
                -theta.cos()
            } else {
                theta - std::f32::consts::FRAC_PI_2
            };
            self.delay[ear] = (HEAD_RADIUS / SPEED_OF_SOUND) * (1.0 + t) * rate;
            // Brown and Duda's shadow: α from 2 facing the sound to 0.1 at 150° from it.
            let (alpha_min, theta_min) = (0.1f32, 150f32.to_radians());
            let alpha = (1.0 + alpha_min / 2.0)
                + (1.0 - alpha_min / 2.0) * (theta / theta_min * std::f32::consts::PI).cos();
            self.shadow[ear].set_shadow(alpha.max(alpha_min), rate);
            // Behind: duller, as the outer ear makes it.
            self.rear[ear].set_lowpass(16_000.0 - 12_000.0 * behind, rate);
            self.gain[ear] = 1.0 - 0.25 * behind;
        }
    }

    /// Runs one sample through, returning the two ears.
    fn run(&mut self, x: f32) -> [f32; 2] {
        self.history[self.at] = x;
        let mut out = [0.0; 2];
        for (ear, o) in out.iter_mut().enumerate() {
            let d = self.delay[ear].clamp(0.0, (DELAY_LEN - 2) as f32);
            let back = d.floor() as usize;
            let frac = d - back as f32;
            let i0 = (self.at + DELAY_LEN - back) % DELAY_LEN;
            let i1 = (i0 + DELAY_LEN - 1) % DELAY_LEN;
            let s = self.history[i0] * (1.0 - frac) + self.history[i1] * frac;
            let s = self.shadow[ear].run(s);
            *o = self.rear[ear].run(s) * self.gain[ear];
        }
        self.at = (self.at + 1) % DELAY_LEN;
        out
    }
}

/// The renderer: places mono sources into an interleaved buffer of `channels` channels.
pub struct Spatial {
    mode: Mode,
    /// How far below its own pitch a sound directly behind plays, as a rate: 2^(-semitones/12).
    behind_rate: f32,
    channels: usize,
    rate: f32,
    /// Per source: the gains last used, so the next block glides from them.
    gains: HashMap<String, Vec<f32>>,
    heads: HashMap<String, Head>,
}

impl Spatial {
    pub fn new(mode: Mode, channels: usize, rate: u32) -> Spatial {
        Spatial {
            mode,
            behind_rate: 2f32.powf(-4.0 / 12.0),
            channels: channels.max(1),
            rate: rate as f32,
            gains: HashMap::new(),
            heads: HashMap::new(),
        }
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// How many semitones lower a sound directly behind plays (0 for none).
    pub fn set_behind_semitones(&mut self, semitones: f32) {
        self.behind_rate = 2f32.powf(-semitones.clamp(0.0, 12.0) / 12.0);
    }

    /// The playback-rate multiplier for a source at (`dx`, `dy`): lower behind, in stereo and on
    /// headphones.
    ///
    /// Two ears can place a sound left or right, but in front or behind is the hard call — in
    /// stereo there is nothing to tell them apart at all, and the head model's cues are subtle.
    /// So a sound behind is also pitched lower, a learnable earcon: `audio.behind_semitones`
    /// directly behind, half that at the sides, nothing in front. Speakers behind the player
    /// need no such help, so surround leaves the pitch alone.
    pub fn rate_for(&self, dx: f32, dy: f32, positional: bool) -> f32 {
        if matches!(self.mode, Mode::Surround(_)) || !positional || (dx == 0.0 && dy == 0.0) {
            return 1.0;
        }
        let behind = (1.0 - azimuth(dx, dy).cos()) / 2.0; // 0 in front, 0.5 at the sides, 1 behind
        // In semitones, so the sides are half the interval, not half the rate.
        self.behind_rate.powf(behind)
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Forgets every source not in `live`, so state does not outlive its sound.
    pub fn retain(&mut self, live: impl Fn(&str) -> bool) {
        self.gains.retain(|k, _| live(k));
        self.heads.retain(|k, _| live(k));
    }

    /// The channel gains for a source at (`dx`, `dy`), or centred for an interface sound.
    fn gains_for(&self, dx: f32, dy: f32, positional: bool) -> Vec<f32> {
        let n = self.channels;
        let mut g = vec![0.0; n];
        let placed = positional && (dx != 0.0 || dy != 0.0);
        if n == 1 {
            g[0] = 1.0;
            return g;
        }
        let stereo = |g: &mut Vec<f32>| {
            let p = if placed { azimuth(dx, dy).sin() } else { 0.0 };
            let a = (p + 1.0) * std::f32::consts::FRAC_PI_4;
            g[0] = a.cos();
            g[1] = a.sin();
        };
        match &self.mode {
            Mode::Surround(speakers) if placed => {
                g = surround_gains(speakers, azimuth(dx, dy));
                g.resize(n, 0.0);
            }
            // Headphones apply their own model in `add`; these are its pan into two ears.
            _ => stereo(&mut g),
        }
        g
    }

    /// Adds `mono` from source `id` at (`dx`, `dy`) into `out` (interleaved, `channels` wide).
    pub fn add(&mut self, id: &str, mono: &[f32], dx: f32, dy: f32, positional: bool, out: &mut [f32]) {
        let n = self.channels;
        let frames = mono.len().min(out.len() / n);
        if frames == 0 {
            return;
        }
        let placed = positional && (dx != 0.0 || dy != 0.0);
        if self.mode == Mode::Headphones && placed && n >= 2 {
            let rate = self.rate;
            let head = self.heads.entry(id.to_string()).or_insert_with(Head::new);
            head.aim(azimuth(dx, dy), rate);
            for (i, &x) in mono.iter().take(frames).enumerate() {
                let [l, r] = head.run(x);
                out[i * n] += l;
                out[i * n + 1] += r;
            }
            return;
        }
        let to = self.gains_for(dx, dy, positional);
        let from = self.gains.get(id).cloned().unwrap_or_else(|| to.clone());
        for (i, &x) in mono.iter().take(frames).enumerate() {
            let t = i as f32 / frames as f32;
            for c in 0..n {
                let g = from[c] + (to[c] - from[c]) * t;
                if g != 0.0 {
                    out[i * n + c] += x * g;
                }
            }
        }
        self.gains.insert(id.to_string(), to);
    }

    /// Spreads the game's stereo, `stereo`, into an `out` of this renderer's channels: front
    /// left and right, or the one channel of a mono device.
    pub fn place_game(&self, stereo: &[f32], out: &mut Vec<f32>) {
        let n = self.channels;
        let frames = stereo.len() / 2;
        out.clear();
        out.resize(frames * n, 0.0);
        for i in 0..frames {
            let (l, r) = (stereo[2 * i], stereo[2 * i + 1]);
            if n == 1 {
                out[i] = 0.5 * (l + r);
            } else {
                out[i * n] = l;
                out[i * n + 1] = r;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * 0.05).sin()).collect()
    }

    fn energy(out: &[f32], channels: usize, c: usize) -> f32 {
        out.chunks(channels).map(|f| f[c] * f[c]).sum()
    }

    #[test]
    fn azimuth_is_clockwise_from_north() {
        assert!(azimuth(0.0, -1.0).abs() < 1e-6, "north is 0");
        assert!((azimuth(1.0, 0.0) - std::f32::consts::FRAC_PI_2).abs() < 1e-6, "east is 90°");
        assert!((azimuth(0.0, 1.0).abs() - std::f32::consts::PI).abs() < 1e-6, "south is 180°");
    }

    #[test]
    fn auto_is_surround_on_a_known_layout_and_stereo_otherwise() {
        assert!(matches!(Mode::resolve("auto", 6), Mode::Surround(_)));
        assert_eq!(Mode::resolve("auto", 2), Mode::Stereo);
        assert_eq!(Mode::resolve("surround", 2), Mode::Stereo, "nothing else to give");
        assert_eq!(Mode::resolve("headphones", 2), Mode::Headphones);
        assert_eq!(Mode::resolve("stereo", 6), Mode::Stereo);
    }

    #[test]
    fn in_surround_south_comes_from_the_rear_and_north_from_the_front() {
        let speakers = layout(6).unwrap();
        let (rear, front): (Vec<usize>, Vec<usize>) = {
            let rear: Vec<usize> = speakers
                .iter()
                .enumerate()
                .filter(|(_, a)| a.is_some_and(|a| a.abs() > 90.0))
                .map(|(i, _)| i)
                .collect();
            (rear, vec![0, 1])
        };
        let mut s = Spatial::new(Mode::Surround(speakers.clone()), 6, 48_000);
        let mut south = vec![0.0; 6 * 512];
        s.add("x", &tone(512), 0.0, 100.0, true, &mut south);
        let r: f32 = rear.iter().map(|&c| energy(&south, 6, c)).sum();
        let f: f32 = front.iter().map(|&c| energy(&south, 6, c)).sum();
        assert!(r > 10.0 * f.max(1e-9), "south is in the rear speakers: rear {r} front {f}");

        let mut s = Spatial::new(Mode::Surround(speakers), 6, 48_000);
        let mut north = vec![0.0; 6 * 512];
        s.add("y", &tone(512), 0.0, -100.0, true, &mut north);
        let r: f32 = rear.iter().map(|&c| energy(&north, 6, c)).sum();
        assert!(r < 1e-6, "north is nowhere near the rear: {r}");
    }

    #[test]
    fn surround_gains_keep_constant_power_all_the_way_round() {
        let speakers = layout(6).unwrap();
        for deg in (0..360).step_by(15) {
            let g = surround_gains(&speakers, (deg as f32).to_radians());
            let p: f32 = g.iter().map(|x| x * x).sum();
            assert!((p - 1.0).abs() < 1e-3, "{deg}°: power {p}");
        }
    }

    #[test]
    fn on_headphones_the_near_ear_is_first_and_loudest_and_behind_is_duller() {
        // Noise-like input: the head shadow and the rear filter both act on the high end.
        let noise: Vec<f32> = (0..4096u32)
            .map(|i| ((i.wrapping_mul(2_654_435_761) >> 8) as f32 / (1u32 << 24) as f32) - 0.5)
            .collect();
        let mut s = Spatial::new(Mode::Headphones, 2, 48_000);
        let mut east = vec![0.0; 2 * 4096];
        s.add("e", &noise, 100.0, 0.0, true, &mut east);
        let (l, r) = (energy(&east, 2, 0), energy(&east, 2, 1));
        assert!(r > 1.5 * l, "east is in the right ear: right {r} left {l}");
        let mut front = vec![0.0; 2 * 4096];
        Spatial::new(Mode::Headphones, 2, 48_000).add("f", &noise, 0.0, -100.0, true, &mut front);
        let mut back = vec![0.0; 2 * 4096];
        Spatial::new(Mode::Headphones, 2, 48_000).add("b", &noise, 0.0, 100.0, true, &mut back);
        let ef = energy(&front, 2, 0) + energy(&front, 2, 1);
        let eb = energy(&back, 2, 0) + energy(&back, 2, 1);
        assert!(eb < 0.8 * ef, "behind is duller and quieter: back {eb} front {ef}");
    }

    #[test]
    fn behind_is_a_major_third_lower_on_headphones_and_in_stereo_but_not_on_speakers() {
        let h = Spatial::new(Mode::Headphones, 2, 48_000);
        assert_eq!(h.rate_for(0.0, -10.0, true), 1.0, "in front: as recorded");
        assert!((h.rate_for(0.0, 10.0, true) - 0.794).abs() < 0.001, "behind: four semitones down");
        let mut one = Spatial::new(Mode::Headphones, 2, 48_000);
        one.set_behind_semitones(1.0);
        assert!((one.rate_for(0.0, 10.0, true) - 0.944).abs() < 0.001, "adjustable: one semitone");
        let side = h.rate_for(10.0, 0.0, true);
        assert!((side - 0.891).abs() < 0.001, "the sides are two semitones down: {side}");
        assert_eq!(h.rate_for(0.0, 10.0, false), 1.0, "an interface sound is never shifted");
        let st = Spatial::new(Mode::Stereo, 2, 48_000);
        assert!(st.rate_for(0.0, 10.0, true) < 1.0, "stereo needs it most");
        let sur = Spatial::new(Mode::Surround(layout(6).unwrap()), 6, 48_000);
        assert_eq!(sur.rate_for(0.0, 10.0, true), 1.0, "speakers behind need no help");
    }

    #[test]
    fn the_games_stereo_goes_to_the_front_pair() {
        let s = Spatial::new(Mode::Surround(layout(6).unwrap()), 6, 48_000);
        let mut out = Vec::new();
        s.place_game(&[0.5, -0.5, 0.25, -0.25], &mut out);
        assert_eq!(out, vec![0.5, -0.5, 0.0, 0.0, 0.0, 0.0, 0.25, -0.25, 0.0, 0.0, 0.0, 0.0]);
    }
}
