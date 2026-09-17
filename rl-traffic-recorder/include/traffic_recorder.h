#pragma once
#include "pcap_writer.h"
#include "json_logger.h"
#include <string>
#include <atomic>
#include <memory>

class TrafficRecorder {
public:
    static TrafficRecorder& instance();

    void start(const std::string& output_dir);
    void stop();

    bool is_recording() const { return _recording.load(std::memory_order_relaxed); }
    uint64_t packets_captured() const { return _pkt_count.load(std::memory_order_relaxed); }

    void on_send(const uint8_t* data, size_t len, uint32_t dst_ip, uint16_t dst_port);
    void on_recv(const uint8_t* data, size_t len, uint32_t src_ip, uint16_t src_port);

    void set_output_dir(const std::string& dir) { _output_dir = dir; }

    const std::string& output_dir() const { return _output_dir; }
    const std::string& session_dir() const { return _session_dir; }
    void set_pcap(std::unique_ptr<PcapWriter> w) { _pcap = std::move(w); }
    void set_json(std::unique_ptr<JsonLogger> l) { _json = std::move(l); }

private:
    TrafficRecorder() = default;

    std::atomic<bool>   _recording{false};
    std::atomic<uint64_t> _pkt_count{0};
    std::string         _output_dir;
    std::string         _session_dir;

    std::unique_ptr<PcapWriter> _pcap;
    std::unique_ptr<JsonLogger> _json;

    uint32_t _local_ip = 0;
};
