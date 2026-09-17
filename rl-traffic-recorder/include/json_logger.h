#pragma once
#include <cstdint>
#include <string>
#include <mutex>
#include <fstream>

class JsonLogger {
public:
    JsonLogger() = default;
    ~JsonLogger();

    bool open(const std::string& path);
    void close();

    void log_packet(
        const uint8_t* data, size_t len,
        const char* direction,
        uint32_t src_ip, uint32_t dst_ip,
        uint16_t src_port, uint16_t dst_port,
        uint8_t  protocol,
        int64_t  ts_ms
    );

    bool is_open() const { return _file.is_open(); }
    uint64_t entry_count() const { return _count; }

private:
    std::ofstream _file;
    std::mutex    _mtx;
    uint64_t      _count = 0;

    static std::string ip_to_string(uint32_t ip);
    static std::string protocol_name(uint8_t p);
    static std::string hex_dump(const uint8_t* data, size_t len, size_t max_bytes);
};
