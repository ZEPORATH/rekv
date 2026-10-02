#include <iostream>

#include "rekv/config_client.hpp"

int main(int argc, char** argv) {
    try {
        const std::string socketPath = argc > 1 ? argv[1] : "/tmp/rekv.sock";
        rekv::ConfigClient client(socketPath);
        client.watch("/platform_manager/io_devices[id = ECU0]/baud_rate", [](const auto& change) {
            std::cout << change.path << " = " << change.newValue << std::endl;
            return true;
        });
    } catch (const std::exception& error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}