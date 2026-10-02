#include "rekv/config_client.hpp"

#include <atomic>
#include <cerrno>
#include <cstring>
#include <stdexcept>
#include <utility>

#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

namespace rekv {
namespace {

constexpr std::size_t kMaxMessageBytes = 64 * 1024;
std::atomic<std::uint64_t> nextRequestId{1};

void writeAll(int descriptor, const char* data, std::size_t size) {
    std::size_t offset = 0;
    while (offset < size) {
        const auto written = ::send(descriptor, data + offset, size - offset, MSG_NOSIGNAL);
        if (written < 0 && errno == EINTR) continue;
        if (written <= 0) throw std::runtime_error(std::strerror(errno));
        offset += static_cast<std::size_t>(written);
    }
}

std::string readLine(int descriptor, std::string& buffered) {
    char buffer[1024];
    while (buffered.size() <= kMaxMessageBytes) {
        const auto newline = buffered.find('\n');
        if (newline != std::string::npos) {
            auto line = buffered.substr(0, newline);
            buffered.erase(0, newline + 1);
            return line;
        }
        const auto received = ::recv(descriptor, buffer, sizeof(buffer), 0);
        if (received < 0 && errno == EINTR) continue;
        if (received <= 0) throw std::runtime_error("socket closed before response");
        buffered.append(buffer, static_cast<std::size_t>(received));
        if (buffered.size() > kMaxMessageBytes) {
            const auto newline = buffered.find('\n');
            if (newline == std::string::npos || newline > kMaxMessageBytes) {
                throw std::runtime_error("response exceeds maximum size");
            }
        }
    }
    throw std::runtime_error("response exceeds maximum size");
}

int connectSocket(const std::string& socketPath) {
    const int descriptor = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (descriptor < 0) throw std::runtime_error(std::strerror(errno));
    sockaddr_un address{};
    address.sun_family = AF_UNIX;
    if (socketPath.size() >= sizeof(address.sun_path)) {
        ::close(descriptor);
        throw std::runtime_error("Unix socket path is too long");
    }
    std::memcpy(address.sun_path, socketPath.c_str(), socketPath.size() + 1);
    if (::connect(descriptor, reinterpret_cast<sockaddr*>(&address), sizeof(address)) < 0) {
        const auto message = std::strerror(errno);
        ::close(descriptor);
        throw std::runtime_error(message);
    }
    return descriptor;
}

}  // namespace

std::int64_t ConfigValue::asInt() const {
    if (type != "integer") throw std::runtime_error("configuration value is not an integer");
    return value.get<std::int64_t>();
}

double ConfigValue::asFloat() const {
    if (type != "float") throw std::runtime_error("configuration value is not a float");
    return value.get<double>();
}

bool ConfigValue::asBool() const {
    if (type != "boolean") throw std::runtime_error("configuration value is not a boolean");
    return value.get<bool>();
}

std::string ConfigValue::asString() const {
    if (type != "string") throw std::runtime_error("configuration value is not a string");
    return value.get<std::string>();
}

ConfigClient::ConfigClient(std::string socketPath) : socketPath_(std::move(socketPath)) {}

ConfigValue ConfigClient::get(const std::string& path) const {
    const auto response = call("get", path);
    return {response.at("type").get<std::string>(), response.at("value")};
}

void ConfigClient::set(const std::string& path, const ConfigValue& value) const {
    call("set", path, nlohmann::json{{"type", value.type}, {"value", value.value}});
}

void ConfigClient::remove(const std::string& path) const {
    call("delete", path);
}

std::vector<std::string> ConfigClient::list(const std::string& path) const {
    return call("list", path).get<std::vector<std::string>>();
}

void ConfigClient::watch(
    const std::string& path,
    const std::function<bool(const ConfigChange&)>& onChange) const {
    const int descriptor = connectSocket(socketPath_);
    try {
        const auto request = nlohmann::json{
            {"id", nextRequestId.fetch_add(1)}, {"method", "watch"}, {"path", path}};
        auto bytes = request.dump();
        bytes.push_back('\n');
        writeAll(descriptor, bytes.data(), bytes.size());

        std::string buffered;
        while (true) {
            const auto response = nlohmann::json::parse(readLine(descriptor, buffered));
            if (!response.value("ok", false)) {
                const auto message = response.contains("error")
                    ? response["error"].value("message", "watch failed")
                    : "watch failed";
                throw std::runtime_error(message);
            }
            if (response.value("event", std::string()) != "change") continue;

            const ConfigChange change{
                response.at("path").get<std::string>(),
                response.value("old_value", nlohmann::json()),
                response.value("new_value", nlohmann::json()),
                response.value("timestamp_ms", std::uint64_t{0}),
            };
            if (!onChange(change)) {
                const auto unwatch = nlohmann::json{
                    {"id", nextRequestId.fetch_add(1)}, {"method", "unwatch"}}.dump() + "\n";
                writeAll(descriptor, unwatch.data(), unwatch.size());
                break;
            }
        }
        ::close(descriptor);
    } catch (...) {
        ::close(descriptor);
        throw;
    }
}

nlohmann::json ConfigClient::call(const std::string& method,
                                 const std::string& path,
                                 const nlohmann::json& value) const {
    nlohmann::json request{{"id", nextRequestId.fetch_add(1)}, {"method", method}, {"path", path}};
    if (method == "set") request["value"] = value;
    auto bytes = request.dump();
    bytes.push_back('\n');
    if (bytes.size() > kMaxMessageBytes) throw std::runtime_error("request exceeds maximum size");

    const int descriptor = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (descriptor < 0) throw std::runtime_error(std::strerror(errno));

    sockaddr_un address{};
    address.sun_family = AF_UNIX;
    if (socketPath_.size() >= sizeof(address.sun_path)) {
        ::close(descriptor);
        throw std::runtime_error("Unix socket path is too long");
    }
    std::memcpy(address.sun_path, socketPath_.c_str(), socketPath_.size() + 1);

    if (::connect(descriptor, reinterpret_cast<sockaddr*>(&address), sizeof(address)) < 0) {
        const auto message = std::strerror(errno);
        ::close(descriptor);
        throw std::runtime_error(message);
    }

    try {
        writeAll(descriptor, bytes.data(), bytes.size());
        std::string buffered;
        const auto response = nlohmann::json::parse(readLine(descriptor, buffered));
        if (!response.at("ok").get<bool>()) {
            const auto& error = response.at("error");
            throw std::runtime_error(error.at("code").get<std::string>() + ": " +
                                     error.at("message").get<std::string>());
        }
        auto result = response.value("result", nlohmann::json());
        ::close(descriptor);
        return result;
    } catch (...) {
        ::close(descriptor);
        throw;
    }
}

ConfigValue ConfigClient::string(std::string value) { return {"string", std::move(value)}; }
ConfigValue ConfigClient::integer(std::int64_t value) { return {"integer", value}; }
ConfigValue ConfigClient::floating(double value) { return {"float", value}; }
ConfigValue ConfigClient::boolean(bool value) { return {"boolean", value}; }
ConfigValue ConfigClient::object(nlohmann::json value) { return {"object", std::move(value)}; }
ConfigValue ConfigClient::array(nlohmann::json value) { return {"array", std::move(value)}; }
ConfigValue ConfigClient::stringList(std::vector<std::string> value) { return {"str_list", std::move(value)}; }
ConfigValue ConfigClient::numericList(std::vector<double> value) { return {"numeric_list", std::move(value)}; }
ConfigValue ConfigClient::null() { return {"null", nullptr}; }

}  // namespace rekv