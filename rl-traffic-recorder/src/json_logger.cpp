#include "json_logger.h"
#include <chrono>
#include <sstream>
#include <iomanip>

JsonLogger::~JsonLogger() {
    close();
}

bool JsonLogger::open(const std::string& path) {
    std::lock_guard lock(_mtx);
    _file.open(path, std::ios::app | std::ios::binary);
    _count = 0;
    return _file.is_open();
}

void JsonLogger::close() {
    std::lock_guard lock(_mtx);
    if (_file.is_open()) _file.close();
    _count = 0;
}

void JsonLogger::log_packet(
    const uint8_t* data, size_t len,
    const char* direction,
    uint32_t src_ip, uint32_t dst_ip,
    uint16_t src_port, uint16_t dst_port,
    uint8_t protocol,
    int64_t ts_ms)
{
    std::lock_guard lock(_mtx);
    if (!_file.is_open()) return;

    std::string hex = hex_dump(data, len, 256);

    _file
        << "{\"ts\":" << ts_ms
        << ",\"dir\":\"" << direction << "\""
        << ",\"src\":\"" << ip_to_string(src_ip) << ":" << src_port << "\""
        << ",\"dst\":\"" << ip_to_string(dst_ip) << ":" << dst_port << "\""
        << ",\"proto\":\"" << protocol_name(protocol) << "\""
        << ",\"len\":" << len
        << ",\"hex\":\"" << hex << "\""
        << "}\n";

    _count++;
}

std::string JsonLogger::ip_to_string(uint32_t ip) {
    char buf[16];
    snprintf(buf, sizeof(buf), "%u.%u.%u.%u",
             ip & 0xFF, (ip >> 8) & 0xFF,
             (ip >> 16) & 0xFF, (ip >> 24) & 0xFF);
    return buf;
}

std::string JsonLogger::protocol_name(uint8_t p) {
    switch (p) {
        case 6:  return "TCP";
        case 17: return "UDP";
        case 1:  return "ICMP";
        default: return "PROTO_" + std::to_string(p);
    }
}

std::string JsonLogger::hex_dump(const uint8_t* data, size_t len, size_t max_bytes) {
    size_t n = (len < max_bytes) ? len : max_bytes;
    std::ostringstream ss;
    ss << std::hex << std::setfill('0');
    for (size_t i = 0; i < n; i++) {
        ss << std::setw(2) << static_cast<unsigned>(data[i]);
    }
    if (len > max_bytes) ss << "...(" << (len - max_bytes) << " more)";
    return ss.str();
}
