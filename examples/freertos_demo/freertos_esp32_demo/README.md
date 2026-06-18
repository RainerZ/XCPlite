# ESP32 FreeRTOS Demo

This example runs XCPlite on an ESP32 board using the Arduino framework and the ESP32 FreeRTOS/lwIP runtime.


## Preconditions

You need:

- vscode with PlatformIO installed, or PlatformIO Core available as `pio`.
- An ESP32-S3 board compatible with `lilygo-t-display-s3`, or an adapted `platformio.ini`.
- A 2.4 GHz WLAN. ESP32 does not connect to 5 GHz-only networks.
- The ESP32 and the PC running the XCP client must be on the same network.
- `xcpclient` for testing and offline A2L generation (see getting xcpclient below).
- A CANape full licence or demo version


![Demo Board](ESP32.png)

## Demo Details

The demo currently:

- Connects the ESP32 to a 2.4 GHz WLAN.
- Scans for the configured SSID and selects the strongest matching BSSID.
- Prints Wi-Fi RSSI, channel, encryption mode, BSSID, disconnect reason, and assigned IP address.
- Starts the XCPlite server after Wi-Fi is connected.
- Binds the XCP UDP server to `0.0.0.0:5555`.
- Displays some status on the T-Display-S3 LCD using LovyanGFX.
- Creates the 2 demo tasks

### Scope Pins

Both demo tasks are pinned to the same ESP32 core so the scheduler interaction is visible in XCP measurements and on a scope:

- `fastTask`: GPIO2 / IO2, default period 1 ms
- `slowTask`: GPIO1 / IO1, default period 10 ms

Connect both probe grounds to board GND. The pins are driven high while the task is running.




## Quick Path

1. Configure Wi-Fi credentials in `src/wlan.h` or via PlatformIO build flags.
2. Build and upload the firmware:
   ```bash
   pio run --target upload
   ```
3. Open the serial monitor and note the ESP32 IP address:
   ```bash
   pio device monitor
   ```
4. Generate the A2L file from the firmware ELF:
   ```bash
   xcpclient --offline --elf .pio/build/lilygo-t-display-s3/firmware.elf --a2l esp32_freertos_demo.a2l --elf-unit-filter main_cpp
   ```
5. Connect with CANape using `CANape_Project`, or run a basic xcpclient measurement test:
   ```bash
   xcpclient --udp --dest-addr <esp32-ip-address> --a2l esp32_freertos_demo.a2l --mea global_counter
   ```


## Wi-Fi Credentials

The sketch includes `wlan.h` when `WIFI_SSID` or `WIFI_PASSWORD` are not provided by build flags:

```cpp
#if !defined(WIFI_SSID) || !defined(WIFI_PASSWORD)
#include "wlan.h"
#endif
```

Example:
```cpp
#pragma once

#define WIFI_SSID "your-ssid"
#define WIFI_PASSWORD "your-password"
```

`wlan.h` is ignored by this example's `.gitignore`.

Alternatively, pass credentials with PlatformIO build flags:

```ini
build_flags =
    -DWIFI_SSID=\"your-ssid\"
    -DWIFI_PASSWORD=\"your-password\"
```


## Configure, Build and Run

### Configure

The XCPlite repository `inc/` and `src/` folders need to be in the include path.


XCP server connection options are set in main.cpp
- XCP on Ethernet over UDP
- XCP server port: `5555`

TCP is currently not supported.  





### Build

XCPlite source files are built directly from the XCPlite repository `src/` folder.  
Building a library could be added later.  

If `pio` is in your shell path:

```bash
pio run
```

### Upload

The current serial port is configured in `platformio.ini`:

Example:
```ini
upload_port = /dev/cu.usbmodem101
monitor_port = /dev/cu.usbmodem101
```

Adjust it if your board enumerates differently:
```bash
pio device list
```

Upload:
```bash
pio run --target upload
```

If upload has trouble entering the bootloader, hold BOOT while upload starts and release it when PlatformIO prints `Connecting...`.


### Serial Monitor

```bash
pio device monitor
```

You should see log messages from the XCP server.  
The log level may be set with XCP_LOG_LEVEL.


### Network Test

First confirm that the board confirms receiving an IP address in the serial log.

Then try:
```bash
ping <esp32-ip-address>
```

The XCP server listens on UDP port `5555`.


### XCP test

Execute a basic XCP connection test:

```bash
xcpclient --udp --dest-addr 192.168.0.146
```

The upload A2L file error message can be ignored, as the FreeRTOS implementation does not support on-target A2L generation and A2L upload.

Execute a measurement

```bash

# with ELF file

xcpclient --udp --dest-addr 192.168.0.146 --elf .pio/build/lilygo-t-display-s3/firmware.elf --elf-unit-filter xcp_demo --mea global_counter

# or with an A2L file

xcpclient --udp --dest-addr 192.168.0.146 --a2l CANape/esp32_freertos_demo.a2l  --mea global_counter

```


## Offline A2L generation

Locate the ELF file and generate the A2L file with xcpclient (see chapter xcpclient below).

Recommended command from this example directory:

```bash
xcpclient --offline --udp --dest-addr 192.168.0.146 --elf .pio/build/lilygo-t-display-s3/firmware.elf --a2l CANape/esp32_freertos_demo.a2l --elf-unit-filter xcp_demo
```

`--elf-unit-filter xcp_demo` keeps the generated A2L focused on this demo application instead of adding all symbols from all linked code. Use `--offline --udp --dest-addr x.x.x.x" to write the target IP address into the A2l file, otherwise it will default to localhost and you need to change it manually in CANape.  


## Adapting to other ESP32 Hardware in PlatformIO

Change the PlatformIO board in `platformio.ini`:

```ini
board = esp32dev
```

or choose the exact board ID from PlatformIO.

For a board without the LilyGo display:

- Remove `lovyan03/LovyanGFX` from `lib_deps`.
- Leave `OPTION_DISPLAY` undefined.





## XCPlite Source Selection

The PlatformIO build uses `extra_script.py` to compile only the XCPlite source files needed for 32 bit embedded targets:

```text
src/xcpappl.c
src/xcplite.c
src/xcpethserver.c
src/xcpethtl.c
src/queue32.c
src/cal.c
src/platform.c
```

The source files remain in the XCPlite repository `src/` folder. They are not copied into this example.





