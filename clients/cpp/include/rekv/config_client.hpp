#pragma once

#include <cstdint>
#include <functional>
#include <string>
#include <vector>

#include <nlohmann/json.hpp>

namespace rekv {

struct ConfigValue {
    std::string type;
    nlohmann::json value;

    std::int64_t asInt() const;
    double asFloat() const;
    bool asBool() const;
    std::string asString() const;
};

struct ConfigChange {
    std::string path;
    nlohmann::json oldValue;
    nlohmann::json newValue;
    std::uint64_t timestampMs;
};

class ConfigClient {
public:
    explicit ConfigClient(std::string socketPath = "/tmp/rekv.sock");

    ConfigValue get(const std::string& path) const;
    void set(const std::string& path, const ConfigValue& value) const;
    void remove(const std::string& path) const;
    std::vector<std::string> list(const std::string& path) const;
    void watch(const std::string& path,
               const std::function<bool(const ConfigChange&)>& onChange) const;

    static ConfigValue string(std::string value);
    static ConfigValue integer(std::int64_t value);
    static ConfigValue floating(double value);
    static ConfigValue boolean(bool value);
    static ConfigValue object(nlohmann::json value);
    static ConfigValue array(nlohmann::json value);
    static ConfigValue stringList(std::vector<std::string> value);
    static ConfigValue numericList(std::vector<double> value);
    static ConfigValue null();

private:
    nlohmann::json call(const std::string& method,
                        const std::string& path,
                        const nlohmann::json& value = nullptr) const;

    std::string socketPath_;
};

}  // namespace rekv