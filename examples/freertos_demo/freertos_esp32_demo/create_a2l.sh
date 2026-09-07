# Transport layer UDP, given IP address and port 5555 is written to the A2L file
# To install xcpclient: 
#   cd XCPlite 
#   ./build.sh rust_tools cargo_install
#   cd examples/freertos_demo/freertos_esp32_demo

xcpclient --offline --udp --dest-addr 192.168.0.146 --elf .pio/build/lilygo-t-display-s3/firmware.elf --a2l CANape/freertos_demo.a2l --elf-unit-filter xcp_demo --log-level=3
