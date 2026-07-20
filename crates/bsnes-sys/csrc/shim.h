/*
 * Beacon - an accessible SNES emulator
 * Copyright (C) 2026 Beacon contributors
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */

/*
 * A flat C ABI over bsnes-jg's C++ `Bsnes::` namespace.
 *
 * Everything crossing this boundary is POD. bsnes-jg's own API takes
 * std::string / std::vector / std::stringstream by reference, none of which
 * can travel over FFI, so this layer owns the conversion.
 *
 * Every function is noexcept: exceptions are caught here and reported as
 * return codes, because unwinding through `extern "C"` into Rust is undefined
 * behaviour. See docs/decisions/0003-rust-host-and-cpp-shim.md.
 */

#ifndef BEACON_BSNES_SHIM_H
#define BEACON_BSNES_SHIM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Memory region ids. Mirrors Bsnes::Memory. Only the regions bsnes-jg
   actually exposes are listed; ARAM, OAM and CGRAM are not public. */
#define BEACON_MEM_CART_RAM 0
#define BEACON_MEM_RTC 1
#define BEACON_MEM_SGB_CART_RAM 2
#define BEACON_MEM_BSX_DOWNLOAD_RAM 3
#define BEACON_MEM_SUFAMI_A_RAM 4
#define BEACON_MEM_SUFAMI_B_RAM 5
#define BEACON_MEM_MAIN_RAM 6
#define BEACON_MEM_VIDEO_RAM 7

/* Return codes. */
#define BEACON_OK 0
#define BEACON_ERR_FAILED -1
#define BEACON_ERR_EXCEPTION -2

/* POD mirrors of the Bsnes::*::Spec structs. Field order and types must match
   src/bsnes.hpp exactly; build.rs asserts the sizes agree. */
typedef struct {
    double freq;
    unsigned spf;
    unsigned rsqual;
    float *buf;
    void *ptr;
    void (*cb)(const void *, size_t);
} beacon_audio_spec;

typedef struct {
    uint32_t *buf;
    void *ptr;
    void (*cb)(const void *, unsigned, unsigned, unsigned);
} beacon_video_spec;

typedef struct {
    unsigned port;
    unsigned device;
    void *ptr;
    int (*cb)(const void *, unsigned, unsigned);
} beacon_input_spec;

/* Lifecycle. `loc` is the ROM path, used by bsnes-jg for companion-file
   lookup; it is copied, so the caller keeps ownership. */
int beacon_bsnes_set_rom_sfc(const uint8_t *data, size_t len, const char *loc);
int beacon_bsnes_load(void);
int beacon_bsnes_loaded(void);
void beacon_bsnes_power(void);
void beacon_bsnes_reset(void);
void beacon_bsnes_unload(void);

/* Advances the emulator by exactly one video frame. */
void beacon_bsnes_run(void);

/* Returns a borrowed pointer into emulator-owned memory, or NULL. The pointer
   stays valid until unload(); the contents change every frame. */
uint8_t *beacon_bsnes_memory(unsigned type, size_t *out_len);

/* Savestates. serialize() writes serialize_size() bytes into `data`. */
unsigned beacon_bsnes_serialize_size(void);
int beacon_bsnes_serialize(uint8_t *data, unsigned len);
int beacon_bsnes_unserialize(const uint8_t *data, unsigned len);

/* Configuration. */
void beacon_bsnes_set_audio_spec(beacon_audio_spec spec);
void beacon_bsnes_set_video_spec(beacon_video_spec spec);
void beacon_bsnes_set_input_spec(beacon_input_spec spec);
void beacon_bsnes_set_region(unsigned region);
unsigned beacon_bsnes_get_region(void);

/* Registers a database file (boards.bml and friends) that bsnes-jg asks for by
   name while loading a cartridge. The bytes are copied. Databases must be
   registered before load(); Beacon embeds them in the executable so there are
   no loose data files to install. */
int beacon_bsnes_add_database(const char *name, const uint8_t *data, size_t len);

/* Installs the companion-file callbacks bsnes-jg requires. Stream requests are
   served from the registered databases; coprocessor BIOS images and save RAM
   are reported absent, which is correct for cartridges without special chips. */
void beacon_bsnes_install_callbacks(void);

/* Last error message, or NULL. Valid until the next failing call. */
const char *beacon_bsnes_last_error(void);

#ifdef __cplusplus
}
#endif

#endif /* BEACON_BSNES_SHIM_H */
