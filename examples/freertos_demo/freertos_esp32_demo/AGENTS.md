# ESP32 FreeRTOS demo guidance

## Scope

These instructions apply only to `examples/freertos_demo/freertos_esp32_demo`.
Keep ESP32-, Arduino-, FreeRTOS-, and PlatformIO-specific workarounds local to
this example unless a repository-wide change is explicitly requested.

## Context to load

- The XCPlite repository root is `../../..` relative to this directory.
- Read `../../../CLAUDE.md` completely before diagnosing or changing XCPlite
  sources. It documents the architecture, configuration variants, and
  repository-wide invariants.
- Read this directory's `README.md` for board setup, networking, build, upload,
  and offline A2L generation.
- Treat `platformio.ini` and its extra scripts as the authoritative build
  configuration for this example; the root CMake build does not build the ESP32
  firmware.

## ESP32 build invariants

- Build from this directory with `pio run`.
- Use the XCPlite `rtos` configuration. Keep
  `XCPLITE_CONFIGURATION=rtos` and `XCPLIB_CFG_OVERRIDE=\"xcplib_rtos_cfg.h\"`
  in the PlatformIO build flags.
- XCPlite's `xcp_cals`, `xcp_evts`, `xcp_epk`, and `xcp_meta` sections contain
  metadata needed for offline A2L generation. Keep them in flash and retained by
  the linker.
- ESP-IDF requires `.flash.appdesc` and `.flash.rodata` to be adjacent.
  `extra_linker_script.py` therefore collects the runtime descriptor sections
  inside `.flash.rodata` and places the named `xcp_epk` and `xcp_meta` output
  sections immediately afterward. Do not remove or relocate that integration
  without checking the final ELF and firmware image generation.
- The runtime descriptors are inspected during XCPlite initialization, while
  `xcp_epk` and `xcp_meta` are consumed from the ELF by `xcpclient`. Keeping
  these sections in flash avoids permanently consuming internal RAM.

## Verification

After changing build flags, source selection, or linker behavior:

1. Run `pio run`.
2. Confirm `firmware.elf` and `firmware.bin` are generated.
3. For linker changes, confirm the XCPlite start/stop symbols are located in
   `.flash.rodata`, the ELF retains named `xcp_epk` and `xcp_meta` sections when
   present, and no gap exists between `.flash.appdesc` and `.flash.rodata`.

Never commit Wi-Fi credentials from `src/wlan.h` or local build flags.
