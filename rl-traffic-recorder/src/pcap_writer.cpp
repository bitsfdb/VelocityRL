#include "pcap_writer.h"
#include <cstring>
#include <chrono>

#ifndef ntohs
#include <winsock2.h>
#endif

PcapWriter::~PcapWriter() {
    close();
}

bool PcapWriter::open(const std::string& path) {
    std::lock_guard lock(_mtx);
    _file.open(path, std::ios::binary | std::ios::trunc);
    if (!_file.is_open()) return false;

    PcapGlobalHeader hdr{};
    _file.write(reinterpret_cast<const char*>(&hdr), sizeof(hdr));
    _bytes_written = sizeof(hdr);
    _pkt_count = 0;
    return _file.good();
}

void PcapWriter::close() {
    std::lock_guard lock(_mtx);
    if (_file.is_open()) _file.close();
    _pkt_count = 0;
    _bytes_written = 0;
}

void PcapWriter::write_packet(
    const uint8_t* data, size_t len,
    const char*  ,
    uint32_t src_ip, uint32_t dst_ip,
    uint16_t src_port, uint16_t dst_port,
    uint8_t protocol)
{
    write_framed(data, len, src_ip, dst_ip, src_port, dst_port, protocol);
}

void PcapWriter::write_framed(
    const uint8_t* payload, size_t payload_len,
    uint32_t src_ip, uint32_t dst_ip,
    uint16_t src_port, uint16_t dst_port,
    uint8_t protocol)
{
    std::lock_guard lock(_mtx);
    if (!_file.is_open()) return;

    constexpr uint32_t FAKE_CLIENT_IP = 0x0A000001;
    constexpr uint32_t FAKE_SERVER_IP = 0x0A000002;

    size_t transport_hdr_size = (protocol == 17) ? 8 : 20;
    size_t ip_total = 20 + transport_hdr_size + payload_len;
    size_t frame_size = 14 + ip_total;

    std::vector<uint8_t> frame(frame_size);

    EthernetHeader* eth = reinterpret_cast<EthernetHeader*>(frame.data());
    memset(eth->dst, 0x00, 6);
    memset(eth->src, 0xAA, 6);
    eth->ether_type = htons(0x0800);

    IPv4Header* ip = reinterpret_cast<IPv4Header*>(frame.data() + 14);
    ip->ver_ihl      = 0x45;
    ip->tos           = 0;
    ip->total_length  = htons(static_cast<uint16_t>(ip_total));
    ip->id            = htons(0x1234);
    ip->flags_fragment = htons(0x4000);
    ip->ttl           = 64;
    ip->protocol      = protocol;
    ip->checksum      = 0;
    ip->src_ip        = FAKE_CLIENT_IP;
    ip->dst_ip        = FAKE_SERVER_IP;

    uint32_t sum = 0;
    for (size_t i = 0; i < 20; i += 2)
        sum += (frame[14 + i] << 8) | frame[14 + i + 1];
    while (sum >> 16) sum = (sum & 0xFFFF) + (sum >> 16);
    ip->checksum = htons(static_cast<uint16_t>(~sum));

    size_t off = 34;
    if (protocol == 17) {

        UdpHeader* udp = reinterpret_cast<UdpHeader*>(frame.data() + off);
        udp->src_port = htons(src_port);
        udp->dst_port = htons(dst_port);
        udp->length   = htons(static_cast<uint16_t>(8 + payload_len));
        udp->checksum = 0;
    } else {

        TcpHeader* tcp = reinterpret_cast<TcpHeader*>(frame.data() + off);
        tcp->src_port  = htons(src_port);
        tcp->dst_port  = htons(dst_port);
        tcp->seq       = 0;
        tcp->ack       = 0;
        tcp->data_offset_flags = 0x50;
        tcp->flags     = 0x02;
        tcp->window    = htons(65535);
        tcp->checksum  = 0;
        tcp->urgent    = 0;
    }
    off += transport_hdr_size;

    if (payload_len > 0 && off + payload_len <= frame_size)
        memcpy(frame.data() + off, payload, payload_len);

    auto now = std::chrono::system_clock::now();
    auto dur = now.time_since_epoch();
    auto sec = std::chrono::duration_cast<std::chrono::seconds>(dur);
    auto usec = std::chrono::duration_cast<std::chrono::microseconds>(dur - sec);

    PcapPacketHeader pkt{};
    pkt.ts_sec   = static_cast<uint32_t>(sec.count());
    pkt.ts_usec  = static_cast<uint32_t>(usec.count());
    pkt.incl_len = static_cast<uint32_t>(frame_size);
    pkt.orig_len = static_cast<uint32_t>(frame_size);

    _file.write(reinterpret_cast<const char*>(&pkt), sizeof(pkt));
    _file.write(reinterpret_cast<const char*>(frame.data()), frame_size);

    _bytes_written += sizeof(pkt) + frame_size;
    _pkt_count++;
}
