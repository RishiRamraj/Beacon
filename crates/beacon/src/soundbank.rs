//! Named recorded sounds: Toby's, read from the player's own copy, or a synthesized stand-in.
//!
//! The navigation cues are modelled on the Toby Accessibility Mod for Doom, and so are their
//! names — a plugin asks for `"pathfinder/nodebeacon"`, exactly as Toby's SNDINFO calls it.
//! When the player points `sounds.dir` at a Toby install (the git repo, or an unpacked `.pk3`),
//! the bank reads its SNDINFO files and plays Toby's own recordings. Nothing of Toby's ships
//! with Beacon: those files belong to their authors, and some (the hit markers) were taken
//! from commercial games. With no install, or for a name the install lacks, [`crate::synth`]
//! renders a Doom-like stand-in, so every cue still sounds.
//!
//! # Reading SNDINFO the way GZDoom does
//!
//! Toby's SNDINFO and its files disagree in ways GZDoom forgives, so the bank must too:
//!
//! - Case: SNDINFO says `sounds/Markers/dsm1plc.ogg`; the file is `DSM1PLC.ogg`.
//! - Extension: `marker/call2` names `dsmark2.wav`; only `dsmark2.ogg` exists. A path that
//!   misses is retried by its bare name, which is how GZDoom falls back to the lump name.
//! - Bare lump names: `amb/rdoor  rdoor` means `sounds/RDOOR.ogg`.
//! - `$random name { a b c }` picks one member each time; `$volume name v` scales it.
//!   Other directives (`$rolloff`, `$limit`, ...) are read past: Beacon's plugin owns
//!   distance, and the rest are GZDoom channel policy.
//!
//! Files are decoded on first use and kept, downmixed to mono and resampled to the device
//! rate, so playback is a plain read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A decoded sound: mono samples at the device rate.
pub struct Clip {
    pub samples: Vec<f32>,
}

/// What a SNDINFO name stands for.
enum Def {
    /// A file, as SNDINFO wrote it (path or bare lump name), resolved on first use.
    File(String),
    /// One of several names, picked afresh each play.
    Random(Vec<String>),
}

pub struct SoundBank {
    sample_rate: u32,
    /// Lower-cased SNDINFO name to its definition.
    defs: HashMap<String, Def>,
    /// Per-name gain from `$volume`.
    volumes: HashMap<String, f32>,
    /// Every file under the install, by lower-cased path relative to its root.
    by_path: HashMap<String, PathBuf>,
    /// The same files by lower-cased bare name (no directory, no extension): the lump-name
    /// fallback.
    by_stem: HashMap<String, PathBuf>,
    /// Decoded clips, by name, including the misses (`None`) so a missing sound is looked
    /// for once rather than on every play.
    cache: HashMap<String, Option<Arc<Clip>>>,
    /// Picks `$random` members. A tiny xorshift: the choice only has to vary.
    rng: u32,
    /// Where the sounds came from, for telling the player.
    source: Option<PathBuf>,
}

impl SoundBank {
    /// A bank with no install: every sound is synthesized.
    pub fn synthesized(sample_rate: u32) -> Self {
        SoundBank {
            sample_rate,
            defs: HashMap::new(),
            volumes: HashMap::new(),
            by_path: HashMap::new(),
            by_stem: HashMap::new(),
            cache: HashMap::new(),
            rng: 0x9E37_79B9,
            source: None,
        }
    }

    /// Opens a Toby install at `dir`, or falls back to synthesis if it holds no SNDINFO.
    ///
    /// An empty `dir` is the default and means synthesis. A directory that is not a Toby
    /// install is reported and treated the same way, rather than failing to start: the
    /// sounds are an enhancement, and the game must still run without them.
    pub fn open(dir: &str, sample_rate: u32) -> Self {
        let mut bank = SoundBank::synthesized(sample_rate);
        if dir.trim().is_empty() {
            return bank;
        }
        let root = PathBuf::from(dir.trim());
        let mut files = Vec::new();
        walk(&root, &root, &mut files);
        let mut sndinfo = Vec::new();
        for (rel, path) in files {
            let lower = rel.to_ascii_lowercase();
            // SNDINFO lumps sit at the root: SNDINFO, SNDINFO.txt, SNDINFO.doors, ...
            if !lower.contains('/') && (lower == "sndinfo" || lower.starts_with("sndinfo.")) {
                sndinfo.push(path.clone());
            }
            if let Some(stem) = stem_of(&lower) {
                // Prefer a file under sounds/ when two share a bare name.
                let under_sounds = lower.starts_with("sounds/");
                match bank.by_stem.get(&stem) {
                    Some(existing)
                        if !under_sounds
                            || existing
                                .strip_prefix(&root)
                                .map(|p| p.starts_with("sounds"))
                                .unwrap_or(false) => {}
                    _ => {
                        bank.by_stem.insert(stem, path.clone());
                    }
                }
            }
            bank.by_path.insert(lower, path);
        }
        sndinfo.sort();
        for path in &sndinfo {
            match std::fs::read(path) {
                Ok(bytes) => bank.parse_sndinfo(&String::from_utf8_lossy(&bytes)),
                Err(e) => eprintln!("sounds: cannot read {}: {e}", path.display()),
            }
        }
        if bank.defs.is_empty() {
            eprintln!(
                "sounds: no SNDINFO in {}; using synthesized sounds",
                root.display()
            );
        } else {
            eprintln!(
                "sounds: {} names from {} SNDINFO file(s) in {}",
                bank.defs.len(),
                sndinfo.len(),
                root.display()
            );
            bank.source = Some(root);
        }
        bank
    }

    /// The install the sounds come from, or `None` when they are synthesized.
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// Reads one SNDINFO file's definitions into the bank. Later definitions win, as in
    /// GZDoom.
    fn parse_sndinfo(&mut self, text: &str) {
        let lines: Vec<Vec<String>> = text.lines().map(tokenize).collect();
        let mut i = 0;
        while i < lines.len() {
            let toks = &lines[i];
            i += 1;
            let Some(first) = toks.first() else { continue };
            if let Some(directive) = first.strip_prefix('$') {
                match directive.to_ascii_lowercase().as_str() {
                    "random" if toks.len() >= 2 => {
                        // Members follow in braces, which may run over several lines.
                        let name = toks[1].to_ascii_lowercase();
                        let mut members = Vec::new();
                        let mut rest: Vec<String> = toks[2..].to_vec();
                        loop {
                            let mut closed = false;
                            for t in rest.drain(..) {
                                match t.as_str() {
                                    "{" => {}
                                    "}" => closed = true,
                                    _ if !closed => members.push(t.to_ascii_lowercase()),
                                    _ => {}
                                }
                            }
                            if closed || i >= lines.len() {
                                break;
                            }
                            rest = lines[i].clone();
                            i += 1;
                        }
                        if !members.is_empty() {
                            self.defs.insert(name, Def::Random(members));
                        }
                    }
                    "volume" if toks.len() >= 3 => {
                        if let Ok(v) = toks[2].parse::<f32>() {
                            self.volumes
                                .insert(toks[1].to_ascii_lowercase(), v.clamp(0.0, 1.0));
                        }
                    }
                    "alias" if toks.len() >= 3 => {
                        self.defs.insert(
                            toks[1].to_ascii_lowercase(),
                            Def::Random(vec![toks[2].to_ascii_lowercase()]),
                        );
                    }
                    // $rolloff, $limit, $pitchshift, $singular, $ambient, $playersound...:
                    // GZDoom's channel and distance policy, which the plugin owns here.
                    _ => {}
                }
            } else if toks.len() >= 2 {
                self.defs
                    .insert(first.to_ascii_lowercase(), Def::File(toks[1].clone()));
            }
        }
    }

    /// The sound for `name`, and its `$volume`, decoding it on first use. Toby's recording if
    /// the install has it, a synthesized stand-in if not, `None` if neither exists.
    pub fn get(&mut self, name: &str) -> Option<(Arc<Clip>, f32)> {
        let key = name.to_ascii_lowercase();
        // A $random name resolves to a member each time, so it is not cached itself.
        let concrete = self.pick(&key, 0)?;
        let volume = self
            .volumes
            .get(&key)
            .or_else(|| self.volumes.get(&concrete))
            .copied()
            .unwrap_or(1.0);
        if let Some(cached) = self.cache.get(&concrete) {
            return cached.clone().map(|c| (c, volume));
        }
        let clip = self
            .load_file(&concrete)
            .or_else(|| crate::synth::render(&concrete, self.sample_rate))
            .or_else(|| {
                // A member of a $random set, missing from disk, may still synthesize under
                // the set's own name ("toby/hurtfloordetector/rad").
                (concrete != key)
                    .then(|| crate::synth::render(&key, self.sample_rate))
                    .flatten()
            })
            .map(|samples| Arc::new(Clip { samples }));
        if clip.is_none() {
            eprintln!("sounds: no sound for '{name}'");
        }
        self.cache.insert(concrete, clip.clone());
        clip.map(|c| (c, volume))
    }

    /// Follows `$random`/`$alias` to a name that is a file (or that no SNDINFO defines, which
    /// the synthesizer may still know). Bounded, so a cycle in someone's SNDINFO cannot hang.
    fn pick(&mut self, key: &str, depth: u8) -> Option<String> {
        if depth > 8 {
            return None;
        }
        match self.defs.get(key) {
            Some(Def::Random(members)) => {
                let members = members.clone();
                self.rng ^= self.rng << 13;
                self.rng ^= self.rng >> 17;
                self.rng ^= self.rng << 5;
                let choice = members[(self.rng as usize) % members.len()].clone();
                self.pick(&choice, depth + 1)
            }
            _ => Some(key.to_string()),
        }
    }

    /// Decodes the file SNDINFO names for `key`, if the install has one.
    fn load_file(&self, key: &str) -> Option<Vec<f32>> {
        let Some(Def::File(target)) = self.defs.get(key) else {
            return None;
        };
        let lower = target.replace('\\', "/").to_ascii_lowercase();
        let path = self
            .by_path
            .get(&lower)
            .or_else(|| stem_of(&lower).and_then(|s| self.by_stem.get(&s)))?;
        let bytes = std::fs::read(path).ok()?;
        match decode(&bytes) {
            Some((samples, rate)) => Some(resample(&samples, rate, self.sample_rate)),
            None => {
                eprintln!("sounds: cannot decode {}", path.display());
                None
            }
        }
    }
}

/// Splits a SNDINFO line into tokens: whitespace-separated, double quotes grouping, `//` and
/// `;`-free comments dropped, braces their own tokens.
fn tokenize(line: &str) -> Vec<String> {
    let line = match line.find("//") {
        // A `//` inside quotes would be a path, but no SNDINFO path contains one.
        Some(i) => &line[..i],
        None => line,
    };
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut tok = String::new();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                tok.push(c);
            }
            out.push(tok);
        } else if c == '{' || c == '}' {
            out.push(c.to_string());
            chars.next();
        } else {
            let mut tok = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                    break;
                }
                tok.push(c);
                chars.next();
            }
            out.push(tok);
        }
    }
    out
}

/// A path's bare name: no directories, no extension, lower case.
fn stem_of(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let stem = match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    };
    (!stem.is_empty()).then(|| stem.to_ascii_lowercase())
}

/// Lists every file under `dir`, as (path relative to `root` with `/` separators, full path).
fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        // A git checkout's history is not sounds.
        if name == ".git" {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel, path));
        }
    }
}

/// Decodes Ogg Vorbis, WAV or a Doom DMX lump to mono samples, returning them with their rate.
///
/// Told apart by content, not extension, as GZDoom does: Toby's `DSMARK4.ogg` is a DMX lump.
fn decode(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    if bytes.starts_with(b"OggS") {
        decode_ogg(bytes)
    } else if bytes.starts_with(b"RIFF") {
        decode_wav(bytes)
    } else if bytes.starts_with(&[3, 0]) {
        decode_dmx(bytes)
    } else {
        None
    }
}

/// Reads Doom's native sound lump: format 3, a 16-bit rate, a 32-bit sample count, then
/// unsigned 8-bit samples with 16 bytes of padding at each end.
fn decode_dmx(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    let rate = u16::from_le_bytes(bytes.get(2..4)?.try_into().ok()?) as u32;
    let count = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize;
    let data = bytes.get(8..)?;
    let data = &data[..count.min(data.len())];
    // The padding repeats the first and last samples; dropping it is what DMX did.
    let data = if data.len() > 32 {
        &data[16..data.len() - 16]
    } else {
        data
    };
    if rate == 0 || data.is_empty() {
        return None;
    }
    Some((data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(), rate))
}

fn decode_ogg(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    let mut reader = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(bytes)).ok()?;
    let channels = reader.ident_hdr.audio_channels.max(1) as usize;
    let rate = reader.ident_hdr.audio_sample_rate;
    let mut mono = Vec::new();
    while let Ok(Some(packet)) = reader.read_dec_packet_itl() {
        for frame in packet.chunks(channels) {
            let sum: f32 = frame.iter().map(|&s| s as f32 / 32768.0).sum();
            mono.push(sum / channels as f32);
        }
    }
    Some((mono, rate))
}

/// Reads a PCM or float WAV, forgivingly.
///
/// Hand-rolled rather than a library because Toby's files are not all well-formed: its
/// `key.wav` claims a byte rate of half what its own format implies, which a strict reader
/// refuses and GZDoom ignores. Only the fields that decide the samples are trusted — format,
/// channels, rate, bits — and chunks are walked by their sizes, so an odd `LIST` or `fact`
/// chunk is stepped over.
fn decode_wav(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    let u16_at = |o: usize| Some(u16::from_le_bytes(bytes.get(o..o + 2)?.try_into().ok()?));
    let u32_at = |o: usize| Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?));
    if bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let (mut format, mut channels, mut rate, mut bits) = (0u16, 0usize, 0u32, 0u16);
    let mut data: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(at + 4)? as usize;
        let body = at + 8;
        match id {
            b"fmt " => {
                format = u16_at(body)?;
                channels = u16_at(body + 2)?.max(1) as usize;
                rate = u32_at(body + 4)?;
                bits = u16_at(body + 14)?;
                // WAVE_FORMAT_EXTENSIBLE: the real format is the subformat's first word.
                if format == 0xFFFE {
                    format = u16_at(body + 24)?;
                }
            }
            b"data" => {
                // A truncated file's data chunk claims more than is there: take what is.
                data = Some(&bytes[body..(body + size).min(bytes.len())]);
            }
            _ => {}
        }
        // Chunks are word-aligned.
        at = body + size + (size & 1);
    }
    let data = data?;
    let interleaved: Vec<f32> = match (format, bits) {
        (1, 8) => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
        (1, 16) => data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect(),
        (1, 24) => data
            .chunks_exact(3)
            .map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        (1, 32) => data
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0)
            .collect(),
        (3, 32) => data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        _ => return None,
    };
    let mono = interleaved
        .chunks(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect();
    Some((mono, rate))
}

/// Linear-interpolation resampling. Plenty for short cues; done once, at load.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let len = ((input.len() as f64) / ratio).floor() as usize;
    (0..len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let j = pos as usize;
            let frac = (pos - j as f64) as f32;
            let a = input[j];
            let b = *input.get(j + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_handles_quotes_comments_and_braces() {
        assert_eq!(
            tokenize(r#"amb/door2   "sounds/Door Switches/X.wav"  // a door"#),
            vec!["amb/door2", "sounds/Door Switches/X.wav"]
        );
        assert_eq!(
            tokenize("$random rad { r1 r2}"),
            vec!["$random", "rad", "{", "r1", "r2", "}"]
        );
    }

    #[test]
    fn sndinfo_definitions_volumes_and_multiline_random_sets() {
        let mut bank = SoundBank::synthesized(48_000);
        bank.parse_sndinfo(
            "pathfinder/nodebeacon \"sounds/PathFinder/PathBeacon.wav\"\n\
             $volume pathfinder/nodebeacon 0.75\n\
             $random toby/rad\n{ r1 r2\n r3 }\n\
             $rolloff toby/rad linear 64 256\n\
             amb/rdoor rdoor\n",
        );
        assert!(matches!(
            bank.defs.get("pathfinder/nodebeacon"),
            Some(Def::File(p)) if p == "sounds/PathFinder/PathBeacon.wav"
        ));
        assert_eq!(bank.volumes.get("pathfinder/nodebeacon"), Some(&0.75));
        assert!(matches!(bank.defs.get("toby/rad"), Some(Def::Random(m)) if m.len() == 3));
        assert!(matches!(bank.defs.get("amb/rdoor"), Some(Def::File(p)) if p == "rdoor"));
        // The $rolloff line was read past, not taken for a definition.
        assert!(!bank.defs.contains_key("$rolloff"));
    }

    #[test]
    fn a_random_set_picks_one_of_its_members() {
        let mut bank = SoundBank::synthesized(48_000);
        bank.parse_sndinfo("$random r { a b }\n");
        for _ in 0..10 {
            let picked = bank.pick("r", 0).unwrap();
            assert!(picked == "a" || picked == "b", "{picked}");
        }
    }

    #[test]
    fn stems_drop_directories_and_extensions() {
        assert_eq!(
            stem_of("sounds/markers/dsm1plc.ogg").as_deref(),
            Some("dsm1plc")
        );
        assert_eq!(stem_of("rdoor").as_deref(), Some("rdoor"));
    }

    #[test]
    fn resampling_scales_length_by_the_rate_ratio() {
        let input = vec![0.0f32; 11_025];
        assert_eq!(resample(&input, 11_025, 44_100).len(), 44_100);
        assert_eq!(resample(&input, 44_100, 44_100).len(), 11_025);
    }

    #[test]
    fn a_wav_decodes_to_mono() {
        // A tiny stereo 16-bit WAV, written with hound, comes back as mono samples.
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let spec = hound::WavSpec {
                channels: 2,
                sample_rate: 22_050,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
            for _ in 0..100 {
                w.write_sample(16384i16).unwrap();
                w.write_sample(0i16).unwrap();
            }
            w.finalize().unwrap();
        }
        let (mono, rate) = decode(buf.get_ref()).unwrap();
        assert_eq!(rate, 22_050);
        assert_eq!(mono.len(), 100);
        assert!(
            (mono[0] - 0.25).abs() < 1e-3,
            "averaged channels: {}",
            mono[0]
        );
    }

    /// The cues the navigation uses, which a Toby install must resolve to its own files.
    const CORE: &[&str] = &[
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
        "marker/beacon2",
        "marker/beacon3",
        "marker/beacon4",
        "marker/beacon5",
        "marker/beacon6",
        "marker/beacon7",
        "marker/beacon8",
        "marker/beacon9",
        "marker/beacon10",
        "marker/markers/undofail",
        "amb/exit1",
        "amb/exit2",
        "amb/door2",
        "amb/switch",
        "amb/rdoor",
        "amb/bdoor",
        "door/deniedred",
        "door/redgranted",
        "toby/universalbeacons/key",
        "toby/universalbeacons/health",
        "toby/universalbeacons/ammo",
        "toby/universalbeacons/armor",
        "toby/universalbeacons/weapon",
        "weapons/lock",
    ];

    /// Run against a real install with `BEACON_TOBY_DIR=/path/to/toby cargo test`. Skipped
    /// otherwise: no copy of Toby is assumed, let alone shipped.
    #[test]
    fn a_toby_install_resolves_every_core_cue_to_its_own_file() {
        let Ok(dir) = std::env::var("BEACON_TOBY_DIR") else {
            return;
        };
        let bank = SoundBank::open(&dir, 48_000);
        assert!(bank.source().is_some(), "no SNDINFO under {dir}");
        let mut missing = Vec::new();
        for name in CORE {
            let mut b = SoundBank::open(&dir, 48_000);
            let concrete = b.pick(name, 0).unwrap();
            match b.load_file(&concrete) {
                Some(s) if !s.is_empty() => {}
                _ => missing.push(*name),
            }
        }
        assert!(missing.is_empty(), "not found in the install: {missing:?}");
    }

    #[test]
    fn a_dmx_lump_decodes_without_its_padding() {
        let mut lump = vec![3, 0];
        lump.extend_from_slice(&11_025u16.to_le_bytes());
        lump.extend_from_slice(&34u32.to_le_bytes());
        lump.extend(std::iter::repeat_n(128u8, 16));
        lump.extend_from_slice(&[192, 64]);
        lump.extend(std::iter::repeat_n(128u8, 16));
        let (mono, rate) = decode(&lump).unwrap();
        assert_eq!(rate, 11_025);
        assert_eq!(mono, vec![0.5, -0.5]);
    }

    #[test]
    fn a_wav_with_a_wrong_byte_rate_still_decodes() {
        // Toby's key.wav: 16-bit mono 16 kHz, but a byte rate of 16000 where 32000 is right.
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF\x2c\x00\x00\x00WAVEfmt \x10\x00\x00\x00");
        wav.extend_from_slice(&[1, 0, 1, 0]); // PCM, mono
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes()); // the wrong byte rate
        wav.extend_from_slice(&[2, 0, 16, 0]);
        wav.extend_from_slice(b"data\x04\x00\x00\x00");
        wav.extend_from_slice(&[0x00, 0x40, 0x00, 0xC0]);
        let (mono, rate) = decode(&wav).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(mono, vec![0.5, -0.5]);
    }

    #[test]
    fn with_no_install_known_cues_still_sound() {
        let mut bank = SoundBank::synthesized(48_000);
        assert!(bank.get("pathfinder/nodebeacon").is_some());
        assert!(bank.get("no/such/sound").is_none());
    }
}

