# C++ client

Requires a C++17 compiler and the header-only nlohmann JSON library. On Debian/Raspberry Pi OS, install `nlohmann-json3-dev`.

Build from the repository root with `make -C clients/cpp`. `watch_baud_rate` subscribes to `/platform_manager/io_devices[id = ECU0]/baud_rate` and prints each update. `read_value` gets the same value once. Both accept an alternate socket path as their first argument.