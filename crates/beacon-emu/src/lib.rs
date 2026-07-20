//! A safe wrapper over bsnes-jg.
//!
//! bsnes-jg's API is a set of free functions over global emulator state, so at
//! most one [`Emulator`] can exist at a time. That is enforced here rather than
//! documented and hoped for.
//!
//! The central operation is [`Emulator::run_frame`], which advances the
//! emulator by exactly one video frame and hands back a borrow of work RAM.
//! Instrumentation runs between frames against real memory, which is the whole
//! point of embedding the emulator rather than polling it over a socket.

use std::ffi::{CStr, CString};
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use bsnes_sys as sys;

/// SNES work RAM size. bsnes-jg reports this at runtime; the constant exists to
/// catch a mismatch rather than to be trusted blindly.
pub const MAIN_RAM_LEN: usize = 128 * 1024;

static INSTANCE_HELD: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub enum Error {
    /// An [`Emulator`] already exists. bsnes-jg keeps global state, so a second
    /// instance would silently share it.
    AlreadyInstantiated,
    Io(std::io::Error),
    /// The ROM path was not representable as a C string.
    InvalidPath,
    /// bsnes-jg rejected the ROM, typically because it is not a valid SFC image.
    LoadFailed(Option<String>),
    /// A C++ exception crossed the shim and was converted to an error.
    Emulator(String),
    /// A memory region the emulator does not expose, or is not mapped yet.
    NoSuchMemoryRegion(u32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::AlreadyInstantiated => {
                write!(f, "an Emulator already exists; bsnes-jg is not reentrant")
            }
            Error::Io(e) => write!(f, "reading ROM: {e}"),
            Error::InvalidPath => write!(f, "ROM path contains an interior nul byte"),
            Error::LoadFailed(Some(m)) => write!(f, "bsnes-jg failed to load the ROM: {m}"),
            Error::LoadFailed(None) => write!(f, "bsnes-jg failed to load the ROM"),
            Error::Emulator(m) => write!(f, "emulator error: {m}"),
            Error::NoSuchMemoryRegion(t) => write!(f, "memory region {t} is not available"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

type Result<T> = std::result::Result<T, Error>;

/// Reads the shim's last error message, if it set one.
fn last_error() -> Option<String> {
    // SAFETY: the shim returns either null or a pointer to a NUL-terminated
    // string it owns, valid until the next failing call. We copy it immediately.
    unsafe {
        let p = sys::beacon_bsnes_last_error();
        if p.is_null() {
            None
        } else {
            Some(CStr::from_ptr(p).to_string_lossy().into_owned())
        }
    }
}

/// Cartridge board database. bsnes-jg cannot map a cartridge without it, so
/// loading any ROM fails if it is missing.
const BOARDS_BML: &[u8] = include_bytes!("../../../vendor/bsnes-jg/Database/boards.bml");

/// Per-title overrides for images whose headers are ambiguous or wrong.
const SUPER_FAMICOM_BML: &[u8] =
    include_bytes!("../../../vendor/bsnes-jg/Database/SuperFamicom.bml");

/// Hands bsnes-jg the databases it asks for by name.
///
/// These are embedded in the executable rather than installed alongside it:
/// a user should be able to download one file and point it at a ROM.
///
/// # Safety
/// Must be called before `beacon_bsnes_load`.
unsafe fn register_databases() -> Result<()> {
    for (name, bytes) in [
        ("boards.bml", BOARDS_BML),
        ("SuperFamicom.bml", SUPER_FAMICOM_BML),
    ] {
        let cname = CString::new(name).map_err(|_| Error::InvalidPath)?;
        let rc = sys::beacon_bsnes_add_database(cname.as_ptr(), bytes.as_ptr(), bytes.len());
        if rc != sys::OK {
            return Err(Error::Emulator(
                last_error().unwrap_or_else(|| format!("could not register {name}")),
            ));
        }
    }
    Ok(())
}

/// Widest frame the SNES can output (hires modes double the 256px width).
pub const VIDEO_MAX_WIDTH: usize = 512;
/// Tallest frame the SNES can output (interlaced modes double 248 lines).
pub const VIDEO_MAX_HEIGHT: usize = 496;
/// Audio samples per frame, sized for PAL's lower frame rate so NTSC also fits.
const AUDIO_MAX_SPF: usize = (48_000 / 50) * 2;
const AUDIO_SAMPLE_RATE: f64 = 48_000.0;

/// Dimensions of the most recently emitted video frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameInfo {
    pub width: u32,
    pub height: u32,
    /// Row stride in bytes, as reported by the emulator.
    pub pitch: u32,
}

/// Buffers the emulator writes into, plus what its callbacks report back.
///
/// Boxed and never moved: bsnes-jg holds raw pointers to all of this for the
/// lifetime of the emulator, so the addresses have to stay put even as the
/// owning [`Emulator`] is moved by value.
struct Buffers {
    video: Vec<u32>,
    audio: Vec<f32>,
    frame_info: FrameInfo,
    audio_samples: u64,
    /// Held buttons per controller port, polled by the emulator each latch.
    buttons: [u16; 2],
}

/// # Safety
/// `ptr` is the `Buffers` pointer handed to `setVideoSpec`, alive for the
/// emulator's lifetime, and bsnes-jg calls this only from `run()`.
unsafe extern "C" fn on_video(ptr: *const std::ffi::c_void, w: u32, h: u32, pitch: u32) {
    if let Some(b) = (ptr as *mut Buffers).as_mut() {
        b.frame_info = FrameInfo {
            width: w,
            height: h,
            pitch,
        };
    }
}

/// # Safety
/// As [`on_video`], for the audio spec.
unsafe extern "C" fn on_audio(ptr: *const std::ffi::c_void, samples: usize) {
    if let Some(b) = (ptr as *mut Buffers).as_mut() {
        b.audio_samples += samples as u64;
    }
}

/// SNES controller buttons, as the bit positions bsnes-jg expects.
///
/// The emulator polls once per latch and reads the whole 16-bit word, so a
/// button mask is the natural unit rather than individual queries.
pub mod button {
    pub const B: u16 = 1 << 15;
    pub const Y: u16 = 1 << 14;
    pub const SELECT: u16 = 1 << 13;
    pub const START: u16 = 1 << 12;
    pub const UP: u16 = 1 << 11;
    pub const DOWN: u16 = 1 << 10;
    pub const LEFT: u16 = 1 << 9;
    pub const RIGHT: u16 = 1 << 8;
    pub const A: u16 = 1 << 7;
    pub const X: u16 = 1 << 6;
    pub const L: u16 = 1 << 5;
    pub const R: u16 = 1 << 4;
}

/// # Safety
/// As [`on_video`], for the input spec. Called once per controller latch.
unsafe extern "C" fn on_input(ptr: *const std::ffi::c_void, port: u32, _id: u32) -> i32 {
    match (ptr as *const Buffers).as_ref() {
        Some(b) => *b.buttons.get(port as usize).unwrap_or(&0) as i32,
        None => 0,
    }
}

/// A loaded SNES emulator instance.
pub struct Emulator {
    frame: u64,
    /// Boxed so its address is stable across moves of `Emulator`; bsnes-jg
    /// retains raw pointers into it.
    buffers: Box<Buffers>,
}

impl Emulator {
    /// Loads a ROM from disk and powers the system on.
    ///
    /// A 512-byte copier header is detected and stripped, since bsnes-jg
    /// expects a headerless image.
    pub fn load(rom_path: &Path) -> Result<Self> {
        // Claim the global instance before touching any emulator state.
        if INSTANCE_HELD.swap(true, Ordering::AcqRel) {
            return Err(Error::AlreadyInstantiated);
        }

        // From here on, any early return must release the claim.
        match Self::load_inner(rom_path) {
            Ok(emu) => Ok(emu),
            Err(e) => {
                INSTANCE_HELD.store(false, Ordering::Release);
                Err(e)
            }
        }
    }

    fn load_inner(rom_path: &Path) -> Result<Self> {
        let bytes = std::fs::read(rom_path)?;
        let rom = strip_copier_header(&bytes);

        let loc =
            CString::new(rom_path.to_string_lossy().as_bytes()).map_err(|_| Error::InvalidPath)?;

        let mut buffers = Box::new(Buffers {
            video: vec![0u32; VIDEO_MAX_WIDTH * VIDEO_MAX_HEIGHT],
            audio: vec![0f32; AUDIO_MAX_SPF],
            frame_info: FrameInfo::default(),
            audio_samples: 0,
            buttons: [0; 2],
        });
        let ctx = (&mut *buffers) as *mut Buffers as *mut std::ffi::c_void;

        // SAFETY: `rom` is a valid slice for the duration of the call, and the
        // shim copies it into storage it owns. `loc` outlives the call. The
        // spec pointers all target `buffers`, which is boxed and outlives the
        // emulator because Drop unloads before it is freed.
        unsafe {
            register_databases()?;
            sys::beacon_bsnes_install_callbacks();

            let rc = sys::beacon_bsnes_set_rom_sfc(rom.as_ptr(), rom.len(), loc.as_ptr());
            if rc != sys::OK {
                return Err(Error::LoadFailed(last_error()));
            }

            // Specs must be set before run(): bsnes-jg calls these every frame
            // without checking, so a null callback is a segfault.
            sys::beacon_bsnes_set_video_spec(sys::VideoSpec {
                buf: buffers.video.as_mut_ptr(),
                ptr: ctx,
                cb: Some(on_video),
            });
            sys::beacon_bsnes_set_audio_spec(sys::AudioSpec {
                freq: AUDIO_SAMPLE_RATE,
                spf: AUDIO_MAX_SPF as u32,
                rsqual: 0,
                buf: buffers.audio.as_mut_ptr(),
                ptr: ctx,
                cb: Some(on_audio),
            });
            if sys::beacon_bsnes_load() != sys::OK {
                return Err(Error::LoadFailed(last_error()));
            }

            sys::beacon_bsnes_power();

            // Controllers are connected after power-on: powering the system
            // reinitialises the controller ports, so anything attached earlier
            // is discarded and the CPU polls a dangling device.
            for port in 0..2 {
                sys::beacon_bsnes_set_input_spec(sys::InputSpec {
                    port,
                    device: 1, // Gamepad
                    ptr: ctx,
                    cb: Some(on_input),
                });
            }
        }

        Ok(Emulator { frame: 0, buffers })
    }

    /// Dimensions of the most recent video frame.
    pub fn frame_info(&self) -> FrameInfo {
        self.buffers.frame_info
    }

    /// The most recent video frame, as raw pixels.
    pub fn framebuffer(&self) -> &[u32] {
        &self.buffers.video
    }

    /// Audio samples emitted since power-on.
    pub fn audio_samples(&self) -> u64 {
        self.buffers.audio_samples
    }

    /// Sets the buttons held on a controller port, as a mask of [`button`]
    /// constants. Takes effect from the next frame.
    pub fn set_buttons(&mut self, port: usize, mask: u16) {
        if let Some(slot) = self.buffers.buttons.get_mut(port) {
            *slot = mask;
        }
    }

    /// Advances the emulator by exactly one video frame, then returns the
    /// frame number just completed.
    ///
    /// This is the hook point for instrumentation: after it returns, work RAM
    /// holds a consistent post-frame snapshot.
    pub fn run_frame(&mut self) -> u64 {
        // SAFETY: the instance guard guarantees a loaded emulator.
        unsafe { sys::beacon_bsnes_run() };
        self.frame += 1;
        self.frame
    }

    /// Frames executed since power-on.
    pub fn frame_count(&self) -> u64 {
        self.frame
    }

    /// Borrows SNES work RAM. The contents change on every [`run_frame`].
    ///
    /// [`run_frame`]: Emulator::run_frame
    pub fn main_ram(&self) -> Result<&[u8]> {
        self.memory(sys::memory::MAIN_RAM)
    }

    /// Borrows a memory region by id. See [`bsnes_sys::memory`].
    pub fn memory(&self, ty: u32) -> Result<&[u8]> {
        let mut len: usize = 0;
        // SAFETY: the returned pointer is emulator-owned and valid until
        // unload, which cannot happen while `&self` is borrowed.
        let ptr = unsafe { sys::beacon_bsnes_memory(ty, &mut len) };
        if ptr.is_null() || len == 0 {
            return Err(Error::NoSuchMemoryRegion(ty));
        }
        Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
    }

    /// Captures a savestate.
    ///
    /// Combined with an input log this makes a session reproducible, which is
    /// what allows golden-file regression tests over the event stream.
    pub fn save_state(&self) -> Result<Vec<u8>> {
        // SAFETY: size query has no preconditions beyond a loaded emulator.
        let size = unsafe { sys::beacon_bsnes_serialize_size() };
        if size == 0 {
            return Err(Error::Emulator("serialize size is zero".into()));
        }

        let mut buf = vec![0u8; size as usize];
        // SAFETY: buf is exactly `size` bytes, which is what the shim checks.
        let rc = unsafe { sys::beacon_bsnes_serialize(buf.as_mut_ptr(), size) };
        if rc != sys::OK {
            return Err(Error::Emulator(
                last_error().unwrap_or_else(|| "serialize failed".into()),
            ));
        }
        Ok(buf)
    }

    /// Restores a savestate captured by [`save_state`].
    ///
    /// [`save_state`]: Emulator::save_state
    pub fn load_state(&mut self, data: &[u8]) -> Result<()> {
        // SAFETY: data is a valid slice for the duration of the call.
        let rc = unsafe { sys::beacon_bsnes_unserialize(data.as_ptr(), data.len() as u32) };
        if rc != sys::OK {
            return Err(Error::Emulator(
                last_error().unwrap_or_else(|| "unserialize failed".into()),
            ));
        }
        Ok(())
    }

    /// Resets the console, as the physical reset button would.
    pub fn reset(&mut self) {
        // SAFETY: the instance guard guarantees a loaded emulator.
        unsafe { sys::beacon_bsnes_reset() };
        self.frame = 0;
    }

    /// The region bsnes-jg detected from the ROM.
    pub fn region(&self) -> Region {
        // SAFETY: no preconditions beyond a loaded emulator.
        match unsafe { sys::beacon_bsnes_get_region() } {
            sys::region::PAL => Region::Pal,
            _ => Region::Ntsc,
        }
    }
}

impl Drop for Emulator {
    fn drop(&mut self) {
        // SAFETY: called once, on the sole live instance.
        unsafe { sys::beacon_bsnes_unload() };
        INSTANCE_HELD.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Ntsc,
    Pal,
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Region::Ntsc => write!(f, "NTSC"),
            Region::Pal => write!(f, "PAL"),
        }
    }
}

/// Strips a 512-byte copier header if present.
///
/// SNES ROM images are a whole number of 32 KiB banks, so a 512-byte remainder
/// is the header that old copiers prepended. bsnes-jg wants it gone.
pub fn strip_copier_header(rom: &[u8]) -> &[u8] {
    if rom.len() % 32768 == 512 {
        &rom[512..]
    } else {
        rom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_copier_header() {
        let headered = vec![0u8; 512 + 32768];
        assert_eq!(strip_copier_header(&headered).len(), 32768);
    }

    #[test]
    fn leaves_headerless_roms_alone() {
        let bare = vec![0u8; 1024 * 1024];
        assert_eq!(strip_copier_header(&bare).len(), 1024 * 1024);
    }

    #[test]
    fn rejects_a_nonexistent_rom_and_releases_the_guard() {
        let missing = Path::new("/nonexistent/beacon-test.sfc");
        assert!(matches!(Emulator::load(missing), Err(Error::Io(_))));
        // The guard must be released, or every later load fails.
        assert!(matches!(Emulator::load(missing), Err(Error::Io(_))));
    }
}
