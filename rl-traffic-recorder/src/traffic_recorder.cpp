#include "traffic_recorder.h"
#include <chrono>
#include <cstring>

TrafficRecorder& TrafficRecorder::instance() {
    static TrafficRecorder s;
    return s;
}

void TrafficRecorder::start(const std::string& session_dir) {
    _session_dir = session_dir;
    _pkt_count.store(0, std::memory_order_relaxed);
    _recording.store(true, std::memory_order_relaxed);
}

void TrafficRecorder::stop() {
    _recording.store(false, std::memory_order_relaxed);
    if (_pcap) _pcap->close();
    if (_json) _json->close();
    _pcap.reset();
    _json.reset();
}

void TrafficRecorder::on_send(
    const uint8_t* data, size_t len,
    uint32_t dst_ip, uint16_t dst_port)
{
    if (!_recording.load(std::memory_order_relaxed)) return;

    auto ts = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()
    ).count();

    uint32_t src_ip = 0x0A000001;

    if (_pcap && _pcap->is_open()) {
        _pcap->write_packet(data, len, "TX", src_ip, dst_ip,
                           0  , dst_port, 17  );
    }
    if (_json && _json->is_open()) {
        _json->log_packet(data, len, "TX", src_ip, dst_ip,
                         0, dst_port, 17, ts);
    }

    _pkt_count.fetch_add(1, std::memory_order_relaxed);
}

void TrafficRecorder::on_recv(
    const uint8_t* data, size_t len,
    uint32_t src_ip, uint16_t src_port)
{
    if (!_recording.load(std::memory_order_relaxed)) return;

    auto ts = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()
    ).count();

    uint32_t dst_ip = 0x0A000001;

    if (_pcap && _pcap->is_open()) {
        _pcap->write_packet(data, len, "RX", src_ip, dst_ip,
                           src_port, 0  , 17  );
    }
    if (_json && _json->is_open()) {
        _json->log_packet(data, len, "RX", src_ip, dst_ip,
                         src_port, 0, 17, ts);
    }

    _pkt_count.fetch_add(1, std::memory_order_relaxed);
}
