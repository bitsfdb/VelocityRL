#include "bakkesmod/plugin/bakkesmodplugin.h"
#include "traffic_recorder.h"
#include <shlobj.h>
#include <memory>
#include <filesystem>
#include <chrono>
#include <sstream>
#include <iomanip>

bool install_winsock_hooks();
void remove_winsock_hooks();

namespace fs = std::filesystem;

static std::string make_session_dir(const std::string& base) {
    auto now = std::chrono::system_clock::now();
    auto tt  = std::chrono::system_clock::to_time_t(now);
    struct tm t{};
    localtime_s(&t, &tt);

    char buf[64];
    strftime(buf, sizeof(buf), "capture_%Y%m%d_%H%M%S", &t);

    std::string dir = base + "\\" + buf;
    fs::create_directories(dir);
    return dir;
}

class TrafficRecorderPlugin : public BakkesMod::Plugin::BakkesModPlugin {
public:
    void onLoad() override;
    void onUnload() override;

private:
    bool _hooks_installed = false;
};

void TrafficRecorderPlugin::onLoad() {
    auto& rec = TrafficRecorder::instance();

    char docs[MAX_PATH]{};
    SHGetFolderPathA(nullptr, CSIDL_PERSONAL, nullptr, 0, docs);
    std::string output_base = std::string(docs) + "\\rl-captures";
    fs::create_directories(output_base);
    rec.set_output_dir(output_base);

    if (!_hooks_installed) {
        _hooks_installed = install_winsock_hooks();
        if (_hooks_installed) {
            cvarManager->log("[traffic] Winsock hooks installed — all RL network traffic will be captured");
        } else {
            cvarManager->log("[traffic] WARNING: failed to install Winsock hooks");
        }
    }

    cvarManager->registerCvar("tr_start", "", "Start network capture").addOnValueChanged(
        [this, &rec](std::string, CVarWrapper) {
            if (rec.is_recording()) {
                cvarManager->log("[traffic] already recording");
                return;
            }
            std::string session = make_session_dir(rec.output_dir().empty()
                ? "C:\\Users\\Public\\rl-captures"
                : rec.output_dir());

            std::string pcap_path = session + "\\capture.pcap";
            auto pcap = std::make_unique<PcapWriter>();
            if (!pcap->open(pcap_path)) {
                cvarManager->log("[traffic] ERROR: cannot create " + pcap_path);
                return;
            }

            std::string json_path = session + "\\capture.jsonl";
            auto json = std::make_unique<JsonLogger>();
            if (!json->open(json_path)) {
                cvarManager->log("[traffic] ERROR: cannot create " + json_path);
                return;
            }

            rec.set_pcap(std::move(pcap));
            rec.set_json(std::move(json));
            rec.start(session);

            cvarManager->log("[traffic] ▶ recording to " + session);
            cvarManager->log("[traffic]   PCAP: " + pcap_path);
            cvarManager->log("[traffic]   JSON: " + json_path);
            cvarManager->log("[traffic]   Open the .pcap in Wireshark to analyze");
        }
    );

    cvarManager->registerCvar("tr_stop", "", "Stop network capture").addOnValueChanged(
        [this, &rec](std::string, CVarWrapper) {
            if (!rec.is_recording()) {
                cvarManager->log("[traffic] not recording");
                return;
            }
            uint64_t pkts = rec.packets_captured();
            std::string dir = rec.session_dir();
            rec.stop();
            cvarManager->log("[traffic] ■ stopped — " + std::to_string(pkts) + " packets captured");
            cvarManager->log("[traffic]   session: " + dir);
        }
    );

    cvarManager->registerCvar("tr_status", "", "Show capture status").addOnValueChanged(
        [this, &rec](std::string, CVarWrapper) {
            if (rec.is_recording()) {
                cvarManager->log("[traffic] ● recording — " +
                    std::to_string(rec.packets_captured()) + " packets so far");
                cvarManager->log("[traffic]   session: " + rec.session_dir());
            } else {
                cvarManager->log("[traffic] ○ idle — use tr_start to begin");
            }
            cvarManager->log("[traffic]   hooks: " +
                std::string(_hooks_installed ? "active" : "FAILED"));
        }
    );

    cvarManager->registerCvar("tr_outdir", output_base, "Set capture output directory").addOnValueChanged(
        [this, &rec](std::string val, CVarWrapper) {
            if (val.empty()) return;
            fs::create_directories(val);
            rec.set_output_dir(val);
            cvarManager->log("[traffic] output dir: " + val);
        }
    );

    cvarManager->log("[traffic] loaded — commands: tr_start, tr_stop, tr_status, tr_outdir");
}

void TrafficRecorderPlugin::onUnload() {
    auto& rec = TrafficRecorder::instance();
    if (rec.is_recording()) {
        rec.stop();
        cvarManager->log("[traffic] auto-stopped recording on unload");
    }
    if (_hooks_installed) {
        remove_winsock_hooks();
        _hooks_installed = false;
    }
}

BAKKESMOD_PLUGIN(TrafficRecorderPlugin, "RL Traffic Recorder", "1.0.0", 0)
