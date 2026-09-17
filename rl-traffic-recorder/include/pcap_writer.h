#pragma once
#include <cstdint>
#include <string>
#include <vector>
#include <mutex>
#include <fstream>

#pragma pack(push, 1)
struct PcapGlobalHeader {
    uint32_t magic_number  = 0xA1B2C3D4;
    uint16_t version_major = 2;
    uint16_t version_minor = 4;
    int32_t  thiszone      = 0;
    uint32_t sigfigs       = 0;
    uint32_t snaplen       = 65535;
    uint32_t network       = 1;
};

struct PcapPacketHeader {
    uint32_t ts_sec;
    uint32_t ts_usec;
    uint32_t incl_len;
    uint32_t orig_len;
};
#pragma pack(pop)

struct EthernetHeader {
    uint8_t  dst[6];
    uint8_t  src[6];
    uint16_t ether_type;
};

struct IPv4Header {
    uint8_t  ver_ihl;
    uint8_t  tos;
    uint16_t total_length;
    uint16_t id;
    uint16_t flags_fragment;
    uint8_t  ttl;
    uint8_t  protocol;
    uint16_t checksum;
    uint32_t src_ip;
    uint32_t dst_ip;
};

struct TcpHeader {
    uint16_t src_port;
    uint16_t dst_port;
    uint32_t seq;
    uint32_t ack;
    uint8_t  data_offset_flags;
    uint8_t  flags;
    uint16_t window;
    uint16_t checksum;
    uint16_t urgent;
};

struct UdpHeader {
    uint16_t src_port;
    uint16_t dst_port;
    uint16_t length;
    uint16_t checksum;
};

class PcapWriter {
public:
    PcapWriter() = default;
    ~PcapWriter();

    bool open(const std::string& path);

    void close();

    void write_packet(
        const uint8_t* data, size_t len,
        const char* direction,
        uint32_t src_ip, uint32_t dst_ip,
        uint16_t src_port, uint16_t dst_port,
        uint8_t  protocol
    );

    bool is_open() const { return _file.is_open(); }
    uint64_t packet_count() const { return _pkt_count; }

private:
    void write_framed(const uint8_t* payload, size_t payload_len,
                      uint32_t src_ip, uint32_t dst_ip,
                      uint16_t src_port, uint16_t dst_port,
                      uint8_t protocol);

    std::ofstream _file;
    std::mutex    _mtx;
    uint64_t      _pkt_count = 0;
    size_t        _bytes_written = 0;
};
