#include <iostream>

#include "rekv/config_client.hpp"

int main(int argc, char** argv) {
    try {
        const std::string socketPath = argc > 1 ? argv[1] : "/tmp/rekv.sock";
        rekv::ConfigClient client(socketPath);
        const auto value = client.get("/platform_manager/io_devices[id = ECU0]/baud_rate");
        if (value.type == "float") {
            std::cout << value.asFloat() << '\n';
        } else {
            std::cout << value.asInt() << '\n';
        }
    } catch (const std::exception& error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}