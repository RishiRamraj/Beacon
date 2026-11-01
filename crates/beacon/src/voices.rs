//! Playing named sounds: one-shots and loops, placed by direction and mixed into the game.
//!
//! The counterpart of [`crate::beacons`] for recorded sounds. A plugin issues
//! [`SoundCmd`]s; this keeps the sounds they started — each a clip, a read position, a
//! playback rate and a gain per channel — and mixes them, a frame's buffer at a time, into
//! the interleaved stereo the game produced.
//!
//! # Placement
//!
//! North is front: the listener faces up the screen, always, so the soundscape does not spin
//! each time Link turns, and it matches the map. In stereo only left and right can be heard,
//! so placement is the beacons' constant-power pan from the offset's left/right ratio; front
//! and back await the HRTF renderer, which slots in behind [`pan`].
//!
//! Loop gains glide to their new value across one buffer when the plugin moves a source, and
//! a stopped loop fades out over one, so neither clicks.

use std::collections::HashMap;
use std::sync::Arc;

use beacon_plugin::SoundCmd;

use crate::soundbank::{Clip, SoundBank};

/// The most one-shots sounding at once. Past this the oldest is dropped: a cue that has been
/// playing longest is the one the player has most likely already heard.
const MAX_ONESHOTS: usize = 24;

struct Voice {
    clip: Arc<Clip>,
    /// Read position in the clip, in samples; fractional so a pitch other than 1 resamples.
    pos: f64,
    rate: f64,
    gains: (f32, f32),
}

struct Looping {
    name: String,
    voice: Voice,
    /// Gains to glide to by the end of the next buffer.
    target: (f32, f32),
    /// Fading out, to be removed once silent.
    stopping: bool,
}

#[derive(Default)]
pub struct VoiceMixer {
    oneshots: Vec<Voice>,
    loops: HashMap<String, Looping>,
}

/// Left and right gains for a source at (`dx`, `dy`), at `volume`.
///
/// Constant-power: equal energy at centre, all to one side at the extremes. An interface
/// sound (`positional` false), or one exactly at the listener, is centred.
pub fn pan(dx: f32, dy: f32, volume: f32, positional: bool) -> (f32, f32) {
    let dist = (dx * dx + dy * dy).sqrt();
    let p = if positional && dist > 0.0 {
        (dx / dist).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let angle = (p + 1.0) * std::f32::consts::FRAC_PI_4;
    (angle.cos() * volume, angle.sin() * volume)
}

impl VoiceMixer {
    /// Carries out one plugin command, fetching its sound from `bank`.
    pub fn apply(&mut self, cmd: SoundCmd, bank: &mut SoundBank) {
        match cmd {
            SoundCmd::Play {
                name,
                dx,
                dy,
                volume,
                pitch,
                positional,
            } => {
                let Some((clip, gain)) = bank.get(&name) else {
                    return;
                };
                if self.oneshots.len() >= MAX_ONESHOTS {
                    self.oneshots.remove(0);
                }
                self.oneshots.push(Voice {
                    clip,
                    pos: 0.0,
                    rate: pitch as f64,
                    gains: pan(dx, dy, volume * gain, positional),
                });
            }
            SoundCmd::Loop {
                id,
                name,
                dx,
                dy,
                volume,
                pitch,
            } => {
                let Some((clip, gain)) = bank.get(&name) else {
                    return;
                };
                let target = pan(dx, dy, volume * gain, true);
                match self.loops.get_mut(&id) {
                    Some(l) if l.name == name => {
                        l.target = target;
                        l.voice.rate = pitch as f64;
                        l.stopping = false;
                    }
                    _ => {
                        self.loops.insert(
                            id,
                            Looping {
                                name,
                                // Starts silent and glides up, so a new loop fades in.
                                voice: Voice {
                                    clip,
                                    pos: 0.0,
                                    rate: pitch as f64,
                                    gains: (0.0, 0.0),
                                },
                                target,
                                stopping: false,
                            },
                        );
                    }
                }
            }
            SoundCmd::Stop { id } => {
                if let Some(l) = self.loops.get_mut(&id) {
                    l.stopping = true;
                    l.target = (0.0, 0.0);
                }
            }
        }
    }

    /// Whether anything is sounding, so the host knows to duck the music under it.
    pub fn active(&self) -> bool {
        !self.oneshots.is_empty() || !self.loops.is_empty()
    }

    /// Drops every sound at once: a plugin reload, a ROM swap, mute.
    pub fn clear(&mut self) {
        self.oneshots.clear();
        self.loops.clear();
    }

    /// Adds every sounding voice into `out` (interleaved stereo) at `master` gain.
    pub fn mix(&mut self, out: &mut [f32], master: f32) {
        let frames = out.len() / 2;
        if frames == 0 {
            return;
        }

        self.oneshots.retain_mut(|v| {
            let g = (v.gains.0 * master, v.gains.1 * master);
            render(v, out, frames, g, g, false)
        });

        self.loops.retain(|_, l| {
            let from = (l.voice.gains.0 * master, l.voice.gains.1 * master);
            let to = (l.target.0 * master, l.target.1 * master);
            render(&mut l.voice, out, frames, from, to, true);
            l.voice.gains = l.target;
            !(l.stopping && l.target == (0.0, 0.0))
        });

        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// Adds `frames` of `v` into `out`, gliding the gains from `from` to `to`. Returns whether the
/// voice has more to play: a one-shot ends at its last sample; a loop wraps.
fn render(
    v: &mut Voice,
    out: &mut [f32],
    frames: usize,
    from: (f32, f32),
    to: (f32, f32),
    looping: bool,
) -> bool {
    let len = v.clip.samples.len();
    if len == 0 {
        return false;
    }
    for i in 0..frames {
        if v.pos >= len as f64 {
            if looping {
                v.pos %= len as f64;
            } else {
                return false;
            }
        }
        let j = v.pos as usize;
        let frac = (v.pos - j as f64) as f32;
        let a = v.clip.samples[j];
        let b = if j + 1 < len {
            v.clip.samples[j + 1]
        } else if looping {
            v.clip.samples[0]
        } else {
            a
        };
        let s = a + (b - a) * frac;
        let t = i as f32 / frames as f32;
        out[2 * i] += s * (from.0 + (to.0 - from.0) * t);
        out[2 * i + 1] += s * (from.1 + (to.1 - from.1) * t);
        v.pos += v.rate;
    }
    looping || v.pos < len as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(out: &[f32]) -> (f32, f32) {
        out.chunks(2)
            .fold((0.0, 0.0), |(l, r), f| (l + f[0] * f[0], r + f[1] * f[1]))
    }

    fn play(dx: f32, positional: bool) -> SoundCmd {
        SoundCmd::Play {
            name: "pathfinder/nodebeacon".into(),
            dx,
            dy: 0.0,
            volume: 1.0,
            pitch: 1.0,
            positional,
        }
    }

    #[test]
    fn a_sound_to_the_east_is_heard_on_the_right() {
        let mut bank = SoundBank::synthesized(48_000);
        let mut m = VoiceMixer::default();
        m.apply(play(100.0, true), &mut bank);
        let mut out = vec![0.0; 2 * 1024];
        m.mix(&mut out, 1.0);
        let (l, r) = energy(&out);
        assert!(r > l * 5.0, "right {r} vs left {l}");
    }

    #[test]
    fn an_interface_sound_is_centred_wherever_it_was_asked_from() {
        let mut bank = SoundBank::synthesized(48_000);
        let mut m = VoiceMixer::default();
        m.apply(play(100.0, false), &mut bank);
        let mut out = vec![0.0; 2 * 1024];
        m.mix(&mut out, 1.0);
        let (l, r) = energy(&out);
        assert!((l - r).abs() < l * 0.01, "{l} vs {r}");
    }

    #[test]
    fn a_one_shot_ends_and_a_loop_keeps_going_until_stopped() {
        let mut bank = SoundBank::synthesized(48_000);
        let mut m = VoiceMixer::default();
        m.apply(play(0.0, true), &mut bank);
        m.apply(
            SoundCmd::Loop {
                id: "x".into(),
                name: "amb/exit1".into(),
                dx: 0.0,
                dy: 0.0,
                volume: 1.0,
                pitch: 1.0,
            },
            &mut bank,
        );
        // A second of audio: the 0.12 s one-shot is long finished, the loop is not.
        let mut out = vec![0.0; 2 * 48_000];
        m.mix(&mut out, 1.0);
        assert!(m.oneshots.is_empty());
        assert_eq!(m.loops.len(), 1);

        m.apply(SoundCmd::Stop { id: "x".into() }, &mut bank);
        let mut out = vec![0.0; 2 * 800];
        m.mix(&mut out, 1.0);
        assert!(
            !m.active(),
            "a stopped loop fades out over one buffer and is gone"
        );
    }

    #[test]
    fn an_unknown_sound_is_silently_ignored() {
        let mut bank = SoundBank::synthesized(48_000);
        let mut m = VoiceMixer::default();
        m.apply(
            SoundCmd::Play {
                name: "no/such".into(),
                dx: 0.0,
                dy: 0.0,
                volume: 1.0,
                pitch: 1.0,
                positional: true,
            },
            &mut bank,
        );
        assert!(!m.active());
    }
}
