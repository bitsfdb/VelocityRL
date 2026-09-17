#pragma once

#include "bakkesmod/plugin/bakkesmodplugin.h"
#include <string>
#include <atomic>
#include <thread>

constexpr auto plugin_version = "1.0";

class VelocityReplaySync : public BakkesMod::Plugin::BakkesModPlugin
{
public:
    void onLoad() override;
    void onUnload() override;

private:
    void OnMatchEnded(std::vector<std::string> params);

    bool IsOptedIn();
    std::string VelocityConfigPath();
    std::string ReadOptInFromConfig();

    std::string ArchiveEndpoint();

    bool UploadReplay(const std::string& replayPath, const std::string& playerName, const std::string& playerId);

    std::atomic<bool> uploadInFlight_{ false };
    std::thread worker_;
};
