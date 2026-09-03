//! Where surviving utterances actually go.
//!
//! There is no well maintained cross platform "speak this string" library, so
//! this is a trait with per platform implementations, none of which is on the
//! critical path alone.
//!
//! [`JsonSink`] is the important one. It means Beacon never has to win an
//! argument about which speech engine is best: anyone running Piper, a custom
//! voice pipeline, or an unusual screen reader can subscribe to the stream and
//! do as they like. It is also the insurance policy for the platforms whose
//! native integration is weakest.

use std::io::{self, Write};

use crate::Utterance;

/// Somewhere utterances can be delivered.
pub trait SpeechSink {
    /// Speaks an utterance. `interrupt` means cancel anything in progress.
    fn speak(&mut self, utterance: &Utterance) -> io::Result<()>;

    /// Cancels speech in progress, if the backend can.
    fn cancel(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Human readable name, for diagnostics.
    fn name(&self) -> &'static str;
}

/// Discards everything. For tests and for running headless.
#[derive(Debug, Default)]
pub struct NullSink;

impl SpeechSink for NullSink {
    fn speak(&mut self, _utterance: &Utterance) -> io::Result<()> {
        Ok(())
    }
    fn name(&self) -> &'static str {
        "null"
    }
}

/// Records everything, so tests can assert on what would have been said.
#[derive(Debug, Default)]
pub struct RecordingSink {
    pub spoken: Vec<Utterance>,
    pub cancels: usize,
}

impl SpeechSink for RecordingSink {
    fn speak(&mut self, utterance: &Utterance) -> io::Result<()> {
        self.spoken.push(utterance.clone());
        Ok(())
    }
    fn cancel(&mut self) -> io::Result<()> {
        self.cancels += 1;
        Ok(())
    }
    fn name(&self) -> &'static str {
        "recording"
    }
}

/// Writes line delimited JSON events.
///
/// Deliberately hand rolled rather than pulling in serde: the schema is three
/// fields and must stay stable for external consumers, so writing it out
/// explicitly is a feature.
pub struct JsonSink<W: Write> {
    out: W,
}

impl<W: Write> JsonSink<W> {
    pub fn new(out: W) -> Self {
        JsonSink { out }
    }

    /// Escapes per RFC 8259. Game text is not trusted to be JSON safe.
    fn escape(s: &str, into: &mut String) {
        for c in s.chars() {
            match c {
                '"' => into.push_str("\\\""),
                '\\' => into.push_str("\\\\"),
                '\n' => into.push_str("\\n"),
                '\r' => into.push_str("\\r"),
                '\t' => into.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    into.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => into.push(c),
            }
        }
    }
}

impl<W: Write> SpeechSink for JsonSink<W> {
    fn speak(&mut self, utterance: &Utterance) -> io::Result<()> {
        let mut text = String::with_capacity(utterance.text.len() + 8);
        Self::escape(&utterance.text, &mut text);
        writeln!(
            self.out,
            r#"{{"type":"speak","text":"{}","priority":"{:?}","interrupt":{}}}"#,
            text, utterance.priority, utterance.interrupt
        )?;
        self.out.flush()
    }

    fn cancel(&mut self) -> io::Result<()> {
        writeln!(self.out, r#"{{"type":"cancel"}}"#)?;
        self.out.flush()
    }

    fn name(&self) -> &'static str {
        "json"
    }
}

/// Speaks through speech-dispatcher, the standard Linux speech interface.
///
/// Talks SSIP over the user's socket directly rather than linking libspeechd,
/// which keeps the dependency surface at zero. Orca uses speech-dispatcher
/// too, so the user's configured voice and rate are inherited rather than
/// overridden.
#[cfg(unix)]
pub struct SpeechDispatcherSink {
    stream: std::os::unix::net::UnixStream,
    replies: io::BufReader<std::os::unix::net::UnixStream>,
}

#[cfg(unix)]
impl SpeechDispatcherSink {
    /// Connects to the running speech-dispatcher for this user.
    pub fn connect() -> io::Result<Self> {
        let path = Self::socket_path().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no speech-dispatcher socket; is it running?",
            )
        })?;

        let stream = std::os::unix::net::UnixStream::connect(path)?;
        let replies = io::BufReader::new(stream.try_clone()?);
        let mut sink = SpeechDispatcherSink { stream, replies };

        // SSIP requires identifying the client before anything else.
        sink.command("SET self CLIENT_NAME beacon:beacon:main")?;
        sink.expect_ok()?;
        Ok(sink)
    }

    /// Reads one SSIP reply and fails on anything that is not a success code.
    ///
    /// Replies are `NNN-continuation` lines followed by a final `NNN message`.
    /// 2xx is success; anything else is an error worth surfacing, since a
    /// silently ignored command means the player simply hears nothing.
    fn expect_ok(&mut self) -> io::Result<String> {
        use io::BufRead;

        loop {
            let mut line = String::new();
            if self.replies.read_line(&mut line)? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "speech-dispatcher closed the connection",
                ));
            }
            let line = line.trim_end();

            // Continuation lines carry a dash after the code.
            if line.len() >= 4 && line.as_bytes()[3] == b'-' {
                continue;
            }

            let code = line.get(..3).and_then(|c| c.parse::<u16>().ok());
            return match code {
                Some(c) if (200..300).contains(&c) => Ok(line.to_string()),
                _ => Err(io::Error::other(format!("speech-dispatcher: {line}"))),
            };
        }
    }

    fn socket_path() -> Option<std::path::PathBuf> {
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            let p = std::path::Path::new(&dir).join("speech-dispatcher/speechd.sock");
            if p.exists() {
                return Some(p);
            }
        }
        let home = std::env::var("HOME").ok()?;
        let p = std::path::Path::new(&home).join(".cache/speech-dispatcher/speechd.sock");
        p.exists().then_some(p)
    }

    /// Sets speech rate, from -100 (slowest) to 100 (fastest).
    ///
    /// Beacon leaves this alone by default, because a screen reader user has
    /// already tuned their rate and inheriting it is the respectful default.
    /// It is exposed because the default matters enormously to how much can be
    /// conveyed per second, and speech-dispatcher's out of the box rate is
    /// slow for anyone accustomed to a screen reader.
    pub fn set_rate(&mut self, rate: i8) -> io::Result<()> {
        let rate = rate.clamp(-100, 100);
        self.command(&format!("SET self RATE {rate}"))?;
        self.expect_ok()?;
        Ok(())
    }

    /// Sets the speech-dispatcher output module, e.g. "espeak-ng" or a generic
    /// module wrapping Piper.
    pub fn set_module(&mut self, module: &str) -> io::Result<()> {
        self.command(&format!("SET self OUTPUT_MODULE {module}"))?;
        self.expect_ok()?;
        Ok(())
    }

    fn command(&mut self, cmd: &str) -> io::Result<()> {
        write!(self.stream, "{cmd}\r\n")?;
        self.stream.flush()
    }
}

#[cfg(unix)]
impl SpeechSink for SpeechDispatcherSink {
    fn speak(&mut self, utterance: &Utterance) -> io::Result<()> {
        if utterance.interrupt {
            self.cancel()?;
        }

        // SSIP terminates a message with a lone dot, so any line that is
        // already a lone dot has to be escaped or it truncates the message.
        self.command("SPEAK")?;
        self.expect_ok()?; // 230, receiving data

        for line in utterance.text.lines() {
            let line = if line == "." { ".." } else { line };
            write!(self.stream, "{line}\r\n")?;
        }
        self.command(".")?;
        self.expect_ok()?; // 225, message queued
        Ok(())
    }

    fn cancel(&mut self) -> io::Result<()> {
        self.command("CANCEL self")?;
        self.expect_ok()?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "speech-dispatcher"
    }
}

/// Delivers to several sinks at once.
///
/// The JSON stream runs alongside a native sink on every platform, so external
/// tooling always has something to subscribe to.
#[derive(Default)]
pub struct Fanout {
    sinks: Vec<Box<dyn SpeechSink>>,
}

impl Fanout {
    pub fn new() -> Self {
        Fanout { sinks: Vec::new() }
    }

    pub fn push(&mut self, sink: Box<dyn SpeechSink>) {
        self.sinks.push(sink);
    }

    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }
}

impl SpeechSink for Fanout {
    /// A failing sink does not stop the others: losing the screen reader
    /// should not also cost the player the JSON stream.
    fn speak(&mut self, utterance: &Utterance) -> io::Result<()> {
        let mut first_err = None;
        for sink in &mut self.sinks {
            if let Err(e) = sink.speak(utterance) {
                first_err.get_or_insert(e);
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn cancel(&mut self) -> io::Result<()> {
        let mut first_err = None;
        for sink in &mut self.sinks {
            if let Err(e) = sink.cancel() {
                first_err.get_or_insert(e);
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn name(&self) -> &'static str {
        "fanout"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Priority;

    fn utter(text: &str, interrupt: bool) -> Utterance {
        Utterance {
            text: text.into(),
            priority: Priority::Navigation,
            interrupt,
        }
    }

    #[test]
    fn json_is_one_object_per_line() {
        let mut buf = Vec::new();
        {
            let mut sink = JsonSink::new(&mut buf);
            sink.speak(&utter("entering kakariko", false)).unwrap();
            sink.speak(&utter("low health", true)).unwrap();
        }
        let out = String::from_utf8(buf).unwrap();
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains(r#""text":"entering kakariko""#));
        assert!(lines[1].contains(r#""interrupt":true"#));
    }

    #[test]
    fn json_escapes_quotes_and_control_characters() {
        let mut buf = Vec::new();
        {
            let mut sink = JsonSink::new(&mut buf);
            sink.speak(&utter("he said \"hi\"\nthen left\t", false))
                .unwrap();
        }
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains(r#"\"hi\""#));
        assert!(out.contains(r"\n"));
        assert!(out.contains(r"\t"));
        // Still exactly one line: the newline was escaped, not emitted.
        assert_eq!(out.lines().count(), 1);
    }

    #[test]
    fn fanout_delivers_to_every_sink() {
        let mut a = RecordingSink::default();
        a.speak(&utter("one", false)).unwrap();
        assert_eq!(a.spoken.len(), 1);

        let mut fan = Fanout::new();
        fan.push(Box::new(RecordingSink::default()));
        fan.push(Box::new(NullSink));
        assert!(fan.speak(&utter("two", false)).is_ok());
    }

    #[test]
    fn fanout_survives_a_failing_sink() {
        struct Failing;
        impl SpeechSink for Failing {
            fn speak(&mut self, _: &Utterance) -> io::Result<()> {
                Err(io::Error::other("nope"))
            }
            fn name(&self) -> &'static str {
                "failing"
            }
        }

        let mut fan = Fanout::new();
        fan.push(Box::new(Failing));
        fan.push(Box::new(NullSink));
        // Reports the error but still attempted the healthy sink.
        assert!(fan.speak(&utter("x", false)).is_err());
    }
}

/// Speaks through the screen reader the player is already running, on Windows.
///
/// The same principle as the speech-dispatcher sink, which exists so that Orca's user gets
/// their own configured voice and rate rather than Beacon's idea of one. Windows has no
/// speech-dispatcher; the equivalent is the screen reader itself, which is where a blind
/// Windows user's voice, rate and punctuation settings live.
///
/// Two libraries are tried, in order:
///
/// - **Tolk**, a shim that speaks through NVDA, JAWS or Window-Eyes if one is running and
///   falls back to SAPI if none is. Preferred because that fallback is the only way a player
///   with no screen reader hears anything.
/// - **NVDA's own controller client**, for a player who has NVDA but not Tolk.
///
/// Both are DLLs that ship beside the executable rather than link-time dependencies, so
/// they are loaded by name at runtime and their absence is not an error — it just means this
/// sink is unavailable and Beacon falls back to the JSON stream. That is also why this
/// declares the three kernel32 entry points it needs by hand instead of taking a dependency:
/// three `extern "system"` lines against a stable ABI is less to justify than a crate.
///
/// UNTESTED. Written on Linux, where it cannot run, and type-checked by cross-compiling.
/// Nothing here has spoken a word.
#[cfg(windows)]
pub struct ScreenReaderSink {
    /// Kept so the library stays loaded for the process's life; never called again.
    _library: Handle,
    speak: SpeakFn,
    silence: SilenceFn,
    name: &'static str,
}

#[cfg(windows)]
type Handle = *mut core::ffi::c_void;

/// `Tolk_Output(text, interrupt) -> bool`, or `nvdaController_speakText(text) -> u32`.
///
/// The two differ in arity, so each is wrapped in a closure-free enum-free way: the loader
/// stores a small adapter rather than the raw pointer.
#[cfg(windows)]
enum SpeakFn {
    /// Tolk: takes the interrupt flag itself.
    Tolk(unsafe extern "system" fn(*const u16, bool) -> bool),
    /// NVDA: cancelling is a separate call.
    Nvda(unsafe extern "system" fn(*const u16) -> u32),
}

#[cfg(windows)]
enum SilenceFn {
    Tolk(unsafe extern "system" fn()),
    Nvda(unsafe extern "system" fn() -> u32),
}

#[cfg(windows)]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *const core::ffi::c_void;
}

/// A NUL-terminated wide string, as every Windows text API wants.
#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The first of `names` a library exports, or null if it exports none of them.
///
/// Both spellings have to be tried. Tolk's header and its documentation say `Tolk_Load` and
/// `Tolk_Output`; the release build that people actually download exports them lowercase,
/// `Tolk_load` and `Tolk_output`. `GetProcAddress` is case sensitive, so looking up only the
/// documented name found nothing in a `Tolk.dll` sitting right beside the executable — and
/// Beacon then reported that there was no Tolk.dll at all.
///
/// # Safety
///
/// `module` must be a live library handle.
#[cfg(windows)]
unsafe fn proc_any(module: Handle, names: &[&core::ffi::CStr]) -> *const core::ffi::c_void {
    for name in names {
        let found = GetProcAddress(module, name.as_ptr() as *const u8);
        if !found.is_null() {
            return found;
        }
    }
    core::ptr::null()
}

#[cfg(windows)]
impl ScreenReaderSink {
    /// Loads whichever library is present, or fails if neither is.
    pub fn connect() -> io::Result<Self> {
        unsafe {
            // Tolk first: it covers the screen readers AND falls back to SAPI, so it is the
            // only option that speaks for a player without a screen reader at all.
            let tolk = LoadLibraryW(wide("Tolk.dll").as_ptr());
            let mut tolk_loaded_but_unusable = false;
            if !tolk.is_null() {
                let load = proc_any(tolk, &[c"Tolk_Load", c"Tolk_load"]);
                let out = proc_any(tolk, &[c"Tolk_Output", c"Tolk_output"]);
                let silence = proc_any(tolk, &[c"Tolk_Silence", c"Tolk_silence"]);
                tolk_loaded_but_unusable = load.is_null() || out.is_null() || silence.is_null();
                if !tolk_loaded_but_unusable {
                    // SAPI is not one of Tolk's drivers unless it is asked for, and asking
                    // before loading is what makes Beacon speak for a player who has no
                    // screen reader running at all. Optional: an older Tolk without it still
                    // works, with the readers only.
                    let try_sapi = proc_any(tolk, &[c"Tolk_TrySAPI", c"Tolk_trySAPI"]);
                    if !try_sapi.is_null() {
                        let try_sapi = core::mem::transmute::<
                            *const core::ffi::c_void,
                            unsafe extern "system" fn(bool) -> bool,
                        >(try_sapi);
                        try_sapi(true);
                    }
                    // Tolk_Load picks the screen reader; without it Tolk_Output says nothing.
                    let load: unsafe extern "system" fn() =
                        core::mem::transmute::<_, unsafe extern "system" fn()>(load);
                    load();
                    return Ok(ScreenReaderSink {
                        _library: tolk,
                        speak: SpeakFn::Tolk(core::mem::transmute::<
                            *const core::ffi::c_void,
                            unsafe extern "system" fn(*const u16, bool) -> bool,
                        >(out)),
                        silence: SilenceFn::Tolk(core::mem::transmute::<
                            *const core::ffi::c_void,
                            unsafe extern "system" fn(),
                        >(silence)),
                        name: "Tolk",
                    });
                }
            }

            let nvda = LoadLibraryW(wide("nvdaControllerClient64.dll").as_ptr());
            if !nvda.is_null() {
                let speak = proc_any(nvda, &[c"nvdaController_speakText"]);
                let cancel = proc_any(nvda, &[c"nvdaController_cancelSpeech"]);
                if !speak.is_null() && !cancel.is_null() {
                    return Ok(ScreenReaderSink {
                        _library: nvda,
                        speak: SpeakFn::Nvda(core::mem::transmute::<
                            *const core::ffi::c_void,
                            unsafe extern "system" fn(*const u16) -> u32,
                        >(speak)),
                        silence: SilenceFn::Nvda(core::mem::transmute::<
                            *const core::ffi::c_void,
                            unsafe extern "system" fn() -> u32,
                        >(cancel)),
                        name: "NVDA",
                    });
                }
            }

            // Named apart, because "there is no DLL" and "the DLL is there and wrong" send
            // someone looking in completely different places.
            if tolk_loaded_but_unusable {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Tolk.dll loaded but exports no speech functions this knows: not Tolk, \
                     or a build with different names",
                ));
            }
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no Tolk.dll or nvdaControllerClient64.dll beside the executable",
        ))
    }
}

#[cfg(windows)]
impl SpeechSink for ScreenReaderSink {
    fn speak(&mut self, utterance: &Utterance) -> io::Result<()> {
        let text = wide(&utterance.text);
        unsafe {
            match self.speak {
                SpeakFn::Tolk(f) => {
                    f(text.as_ptr(), utterance.interrupt);
                }
                SpeakFn::Nvda(f) => {
                    // NVDA has no interrupt flag, so cancelling first is what makes an
                    // interrupting utterance interrupt.
                    if utterance.interrupt {
                        self.cancel()?;
                    }
                    f(text.as_ptr());
                }
            }
        }
        Ok(())
    }

    fn cancel(&mut self) -> io::Result<()> {
        unsafe {
            match self.silence {
                SilenceFn::Tolk(f) => f(),
                SilenceFn::Nvda(f) => {
                    f();
                }
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        self.name
    }
}
