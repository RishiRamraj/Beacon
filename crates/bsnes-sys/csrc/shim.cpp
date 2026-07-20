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

#include "shim.h"

#include <map>
#include <sstream>
#include <string>
#include <vector>

#include "bsnes.hpp"

namespace {

/* Owned copy of the ROM. bsnes-jg takes the vector by non-const reference and
   retains a view of it, so it has to outlive load(). */
std::vector<uint8_t> g_rom;
std::string g_rom_location;
std::string g_last_error;

void set_error(const char *what) { g_last_error = what ? what : "unknown"; }

/* The Spec structs are POD in both headers, so the C mirrors can be
   reinterpreted rather than copied field by field. Verified below. */
static_assert(sizeof(beacon_audio_spec) == sizeof(Bsnes::Audio::Spec),
              "audio spec layout drift");
static_assert(sizeof(beacon_video_spec) == sizeof(Bsnes::Video::Spec),
              "video spec layout drift");
static_assert(sizeof(beacon_input_spec) == sizeof(Bsnes::Input::Spec),
              "input spec layout drift");

/* Databases bsnes-jg requests by name during cartridge load, chiefly
   boards.bml. Registered by the host from data embedded in the executable. */
std::map<std::string, std::vector<uint8_t>> g_databases;

/* Reporting every file absent is correct for cartridges without coprocessors;
   save RAM is handled by the host later. */
bool open_file(void *, std::string, std::vector<uint8_t> &) { return false; }

bool open_stream(void *, std::string name, std::stringstream &ss) {
  std::map<std::string, std::vector<uint8_t>>::const_iterator it =
      g_databases.find(name);
  if (it == g_databases.end()) {
    return false;
  }
  ss.write(reinterpret_cast<const char *>(it->second.data()),
           static_cast<std::streamsize>(it->second.size()));
  return true;
}

/* Invoked from inside Bsnes::load(). Cartridge::load() clears its game state
   and then calls this, so the ROM has to be handed over here rather than
   beforehand: anything set earlier is wiped before it is read. */
bool rom_load(void *, unsigned id) {
  if (id != Bsnes::GameType::SuperFamicom) {
    return false;
  }
  if (g_rom.size() < 0x8000) {
    set_error("ROM smaller than 32 KiB");
    return false;
  }
  Bsnes::setRomSuperFamicom(g_rom, g_rom_location);
  return true;
}

} // namespace

#define BEACON_TRY try {
#define BEACON_CATCH(ret)                                                      \
  }                                                                            \
  catch (const std::exception &e) {                                            \
    set_error(e.what());                                                       \
    return ret;                                                                \
  }                                                                            \
  catch (...) {                                                                \
    set_error("unknown C++ exception");                                        \
    return ret;                                                                \
  }

/* Void-returning entry points still must not let an exception escape. */
#define BEACON_CATCH_VOID                                                      \
  }                                                                            \
  catch (const std::exception &e) {                                            \
    set_error(e.what());                                                       \
  }                                                                            \
  catch (...) {                                                                \
    set_error("unknown C++ exception");                                        \
  }

extern "C" {

int beacon_bsnes_set_rom_sfc(const uint8_t *data, size_t len, const char *loc) {
  BEACON_TRY
  if (!data || len == 0) {
    set_error("empty ROM buffer");
    return BEACON_ERR_FAILED;
  }
  /* Stored only. The emulator is handed the ROM from rom_load(), which
     Bsnes::load() calls after it has cleared its cartridge state. */
  g_rom.assign(data, data + len);
  g_rom_location = loc ? loc : "";
  return BEACON_OK;
  BEACON_CATCH(BEACON_ERR_EXCEPTION)
}

int beacon_bsnes_load(void) {
  BEACON_TRY
  return Bsnes::load() ? BEACON_OK : BEACON_ERR_FAILED;
  BEACON_CATCH(BEACON_ERR_EXCEPTION)
}

int beacon_bsnes_loaded(void) {
  BEACON_TRY
  return Bsnes::loaded() ? 1 : 0;
  BEACON_CATCH(0)
}

void beacon_bsnes_power(void) {
  BEACON_TRY
  Bsnes::power();
  BEACON_CATCH_VOID
}

void beacon_bsnes_reset(void) {
  BEACON_TRY
  Bsnes::reset();
  BEACON_CATCH_VOID
}

void beacon_bsnes_unload(void) {
  BEACON_TRY
  Bsnes::unload();
  g_rom.clear();
  g_rom.shrink_to_fit();
  BEACON_CATCH_VOID
}

void beacon_bsnes_run(void) {
  BEACON_TRY
  Bsnes::run();
  BEACON_CATCH_VOID
}

uint8_t *beacon_bsnes_memory(unsigned type, size_t *out_len) {
  BEACON_TRY
  std::pair<void *, unsigned> mem = Bsnes::getMemoryRaw(type);
  if (out_len) {
    *out_len = mem.second;
  }
  return static_cast<uint8_t *>(mem.first);
  BEACON_CATCH(nullptr)
}

unsigned beacon_bsnes_serialize_size(void) {
  BEACON_TRY
  return Bsnes::serializeSize();
  BEACON_CATCH(0)
}

int beacon_bsnes_serialize(uint8_t *data, unsigned len) {
  BEACON_TRY
  if (!data || len < Bsnes::serializeSize()) {
    set_error("serialize buffer too small");
    return BEACON_ERR_FAILED;
  }
  return Bsnes::serialize(data) ? BEACON_OK : BEACON_ERR_FAILED;
  BEACON_CATCH(BEACON_ERR_EXCEPTION)
}

int beacon_bsnes_unserialize(const uint8_t *data, unsigned len) {
  BEACON_TRY
  if (!data) {
    set_error("null savestate buffer");
    return BEACON_ERR_FAILED;
  }
  return Bsnes::unserialize(data, len) ? BEACON_OK : BEACON_ERR_FAILED;
  BEACON_CATCH(BEACON_ERR_EXCEPTION)
}

void beacon_bsnes_set_audio_spec(beacon_audio_spec spec) {
  BEACON_TRY
  Bsnes::Audio::Spec native;
  native.freq = spec.freq;
  native.spf = spec.spf;
  native.rsqual = spec.rsqual;
  native.buf = spec.buf;
  native.ptr = spec.ptr;
  native.cb = spec.cb;
  Bsnes::setAudioSpec(native);
  BEACON_CATCH_VOID
}

void beacon_bsnes_set_video_spec(beacon_video_spec spec) {
  BEACON_TRY
  Bsnes::Video::Spec native;
  native.buf = spec.buf;
  native.ptr = spec.ptr;
  native.cb = spec.cb;
  Bsnes::setVideoSpec(native);
  BEACON_CATCH_VOID
}

void beacon_bsnes_set_input_spec(beacon_input_spec spec) {
  BEACON_TRY
  Bsnes::Input::Spec native;
  native.port = spec.port;
  native.device = spec.device;
  native.ptr = spec.ptr;
  native.cb = spec.cb;
  Bsnes::setInputSpec(native);
  BEACON_CATCH_VOID
}

void beacon_bsnes_set_region(unsigned region) {
  BEACON_TRY
  Bsnes::setRegion(region);
  BEACON_CATCH_VOID
}

unsigned beacon_bsnes_get_region(void) {
  BEACON_TRY
  return Bsnes::getRegion();
  BEACON_CATCH(0)
}

int beacon_bsnes_add_database(const char *name, const uint8_t *data,
                              size_t len) {
  BEACON_TRY
  if (!name || !data || len == 0) {
    set_error("empty database registration");
    return BEACON_ERR_FAILED;
  }
  g_databases[std::string(name)].assign(data, data + len);
  return BEACON_OK;
  BEACON_CATCH(BEACON_ERR_EXCEPTION)
}

void beacon_bsnes_install_callbacks(void) {
  BEACON_TRY
  Bsnes::setOpenFileCallback(nullptr, open_file);
  Bsnes::setOpenStreamCallback(nullptr, open_stream);
  Bsnes::setRomLoadCallback(nullptr, rom_load);
  BEACON_CATCH_VOID
}

const char *beacon_bsnes_last_error(void) {
  return g_last_error.empty() ? nullptr : g_last_error.c_str();
}

} // extern "C"
