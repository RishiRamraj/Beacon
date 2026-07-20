//! Raw FFI declarations for the bsnes-jg C ABI shim.
//!
//! This crate is the unsafe boundary and nothing more. It deliberately exposes
//! no safe abstraction; see the `beacon-emu` crate for that.
//!
//! The declarations here must stay in step with `csrc/shim.h` by hand. There is
//! no bindgen step: the surface is a dozen POD functions, and a hand-written
//! block is easier to audit than a generated one.

use std::ffi::{c_char, c_int, c_uint, c_void};

/// Memory region ids, mirroring `Bsnes::Memory`.
///
/// bsnes-jg exposes only these. ARAM, OAM and CGRAM exist internally but are
/// not public; reaching them would require patching the emulator.
pub mod memory {
    pub const CART_RAM: u32 = 0;
    pub const RTC: u32 = 1;
    pub const SGB_CART_RAM: u32 = 2;
    pub const BSX_DOWNLOAD_RAM: u32 = 3;
    pub const SUFAMI_A_RAM: u32 = 4;
    pub const SUFAMI_B_RAM: u32 = 5;
    /// 128 KiB of SNES work RAM. This is what game instrumentation reads.
    pub const MAIN_RAM: u32 = 6;
    pub const VIDEO_RAM: u32 = 7;
}

pub mod region {
    pub const NTSC: u32 = 0;
    pub const PAL: u32 = 1;
}

pub const OK: c_int = 0;
pub const ERR_FAILED: c_int = -1;
pub const ERR_EXCEPTION: c_int = -2;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioSpec {
    pub freq: f64,
    pub spf: c_uint,
    pub rsqual: c_uint,
    pub buf: *mut f32,
    pub ptr: *mut c_void,
    pub cb: Option<unsafe extern "C" fn(*const c_void, usize)>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VideoSpec {
    pub buf: *mut u32,
    pub ptr: *mut c_void,
    pub cb: Option<unsafe extern "C" fn(*const c_void, c_uint, c_uint, c_uint)>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputSpec {
    pub port: c_uint,
    pub device: c_uint,
    pub ptr: *mut c_void,
    pub cb: Option<unsafe extern "C" fn(*const c_void, c_uint, c_uint) -> c_int>,
}

extern "C" {
    pub fn beacon_bsnes_set_rom_sfc(data: *const u8, len: usize, loc: *const c_char) -> c_int;
    pub fn beacon_bsnes_load() -> c_int;
    pub fn beacon_bsnes_loaded() -> c_int;
    pub fn beacon_bsnes_power();
    pub fn beacon_bsnes_reset();
    pub fn beacon_bsnes_unload();

    /// Advances the emulator by exactly one video frame.
    pub fn beacon_bsnes_run();

    /// Borrowed pointer into emulator-owned memory, or null. Valid until
    /// `beacon_bsnes_unload`; contents change every frame.
    pub fn beacon_bsnes_memory(ty: c_uint, out_len: *mut usize) -> *mut u8;

    pub fn beacon_bsnes_serialize_size() -> c_uint;
    pub fn beacon_bsnes_serialize(data: *mut u8, len: c_uint) -> c_int;
    pub fn beacon_bsnes_unserialize(data: *const u8, len: c_uint) -> c_int;

    pub fn beacon_bsnes_set_audio_spec(spec: AudioSpec);
    pub fn beacon_bsnes_set_video_spec(spec: VideoSpec);
    pub fn beacon_bsnes_set_input_spec(spec: InputSpec);
    pub fn beacon_bsnes_set_region(region: c_uint);
    pub fn beacon_bsnes_get_region() -> c_uint;

    /// Registers a database bsnes-jg requests by name during cartridge load.
    /// Must be called before `beacon_bsnes_load`.
    pub fn beacon_bsnes_add_database(name: *const c_char, data: *const u8, len: usize) -> c_int;

    pub fn beacon_bsnes_install_callbacks();

    /// Last error message, or null. Valid until the next failing call.
    pub fn beacon_bsnes_last_error() -> *const c_char;
}
