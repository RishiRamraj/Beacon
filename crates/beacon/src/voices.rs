//! Playing named sounds: one-shots and loops, placed by direction and mixed into the game.
//!
//! The counterpart of [`crate::beacons`] for recorded sounds. A plugin issues
//! [`SoundCmd`]s; this keeps the sounds they started — each a clip, a read position, a
//! playback rate, a volume and a place — and mixes them, a frame's buffer at a time, into the
//! game's audio.
//!
//! # Placement
//!
//! Each voice is rendered as a mono block and handed, with its offset from the player, to
//! [`Spatial`], which places it in stereo, on surround speakers, or for headphones. North is
//! front, always. A loop's volume glides to its new value across one buffer, and a stopped loop
//! fades out over one, so neither clicks; the renderer glides its position the same way.

use std::collections::HashMap;
use std::sync::Arc;

use beacon_plugin::SoundCmd;

use crate::soundbank::{Clip, SoundBank};
use crate::spatial::Spatial;

/// The most one-shots sounding at once. Past this the oldest is dropped: a cue that has been
/// playing longest is the one the player has most likely already heard.
const MAX_ONESHOTS: usize = 24;

struct Voice {
    /// Its name to the renderer, which keeps its gains (and head state) between buffers.
    id: String,
    clip: Arc<Clip>,
    /// Read position in the clip, in samples; fractional so a pitch other than 1 resamples.
    pos: f64,
    rate: f64,
    volume: f32,
    dx: f32,
    dy: f32,
    positional: bool,
}

struct Looping {
    name: String,
    voice: Voice,
    /// Volume to glide to by the end of the next buffer.
    target: f32,
    /// Fading out, to be removed once silent.
    stopping: bool,
}

#[derive(Default)]
pub struct VoiceMixer {
    oneshots: Vec<Voice>,
    loops: HashMap<String, Looping>,
    /// Numbers one-shots for the renderer.
    next_id: u64,
    /// A mono buffer, reused.
    mono: Vec<f32>,
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
                self.next_id += 1;
                self.oneshots.push(Voice {
                    id: format!("v:{}", self.next_id),
                    clip,
                    pos: 0.0,
                    rate: pitch as f64,
                    volume: volume * gain,
                    dx,
                    dy,
                    positional,
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
                let target = volume * gain;
                match self.loops.get_mut(&id) {
                    Some(l) if l.name == name => {
                        l.target = target;
                        l.voice.rate = pitch as f64;
                        l.voice.dx = dx;
                        l.voice.dy = dy;
                        l.stopping = false;
                    }
                    _ => {
                        self.loops.insert(
                            id.clone(),
                            Looping {
                                name,
                                // Starts silent and glides up, so a new loop fades in.
                                voice: Voice {
                                    id: format!("l:{id}"),
                                    clip,
                                    pos: 0.0,
                                    rate: pitch as f64,
                                    volume: 0.0,
                                    dx,
                                    dy,
                                    positional: true,
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
                    l.target = 0.0;
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

    /// Adds every sounding voice into `out` at `master` gain, placed by `spatial` (whose
    /// channel count `out` is interleaved in).
    pub fn mix(&mut self, out: &mut [f32], master: f32, spatial: &mut Spatial) {
        let n = spatial.channels();
        let frames = out.len() / n;
        if frames == 0 {
            return;
        }
        let mut mono = std::mem::take(&mut self.mono);

        self.oneshots.retain_mut(|v| {
            let shift = spatial.rate_for(v.dx, v.dy, v.positional);
            let more = render(v, &mut mono, frames, v.volume * master, v.volume * master, false, shift);
            spatial.add(&v.id, &mono, v.dx, v.dy, v.positional, out);
            more
        });

        self.loops.retain(|_, l| {
            let (from, to) = (l.voice.volume * master, l.target * master);
            let shift = spatial.rate_for(l.voice.dx, l.voice.dy, true);
            render(&mut l.voice, &mut mono, frames, from, to, true, shift);
            spatial.add(&l.voice.id, &mono, l.voice.dx, l.voice.dy, true, out);
            l.voice.volume = l.target;
            !(l.stopping && l.target == 0.0)
        });

        // The renderer forgets sources that have finished.
        let live: std::collections::HashSet<String> = self
            .oneshots
            .iter()
            .map(|v| v.id.clone())
            .chain(self.loops.values().map(|l| l.voice.id.clone()))
            .collect();
        spatial.retain(|k| !(k.starts_with("v:") || k.starts_with("l:")) || live.contains(k));

        self.mono = mono;
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// Renders `frames` of `v` into `mono`, its volume gliding from `from` to `to`, its rate times
/// `shift` (the renderer's pitch cue for where it is), silence past its end. Returns whether the
/// voice has more to play: a one-shot ends at its last sample; a loop wraps.
fn render(
    v: &mut Voice,
    mono: &mut Vec<f32>,
    frames: usize,
    from: f32,
    to: f32,
    looping: bool,
    shift: f32,
) -> bool {
    mono.clear();
    mono.resize(frames, 0.0);
    let len = v.clip.samples.len();
    if len == 0 {
        return false;
    }
    for (i, m) in mono.iter_mut().enumerate() {
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
        let t = i as f32 / frames as f32;
        *m = (a + (b - a) * frac) * (from + (to - from) * t);
        v.pos += v.rate * shift as f64;
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
        m.mix(&mut out, 1.0, &mut Spatial::new(crate::spatial::Mode::Stereo, 2, 48_000));
        let (l, r) = energy(&out);
        assert!(r > l * 5.0, "right {r} vs left {l}");
    }

    #[test]
    fn an_interface_sound_is_centred_wherever_it_was_asked_from() {
        let mut bank = SoundBank::synthesized(48_000);
        let mut m = VoiceMixer::default();
        m.apply(play(100.0, false), &mut bank);
        let mut out = vec![0.0; 2 * 1024];
        m.mix(&mut out, 1.0, &mut Spatial::new(crate::spatial::Mode::Stereo, 2, 48_000));
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
        m.mix(&mut out, 1.0, &mut Spatial::new(crate::spatial::Mode::Stereo, 2, 48_000));
        assert!(m.oneshots.is_empty());
        assert_eq!(m.loops.len(), 1);

        m.apply(SoundCmd::Stop { id: "x".into() }, &mut bank);
        let mut out = vec![0.0; 2 * 800];
        m.mix(&mut out, 1.0, &mut Spatial::new(crate::spatial::Mode::Stereo, 2, 48_000));
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
