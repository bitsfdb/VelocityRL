#include "pch.h"
#include "VelocityReplaySync.h"
#include "bakkesmod/wrappers/includes.h"
#include "bakkesmod/wrappers/GameEvent/ServerWrapper.h"
#include "bakkesmod/wrappers/GameObject/PriWrapper.h"
#include "bakkesmod/wrappers/GameObject/PlayerReplicationInfoWrapper.h"
#include "bakkesmod/wrappers/ReplayServerWrapper.h"
#include "bakkesmod/wrappers/GameEvent/ReplayWrapper.h"
#include <fstream>
#include <sstream>
#include <filesystem>
#include <WinSock2.h>
#include <WS2tcpip.h>
#include <ShlObj.h>

#pragma comment(lib, "ws2_32.lib")

BAKKESMOD_PLUGIN(VelocityReplaySync, "VelocityRL Replay Sync", plugin_version, PLUGINTYPE_FREEPLAY)

static const char* kDefaultEndpoint = "http://127.0.0.1:27505/replays/archive";

void VelocityReplaySync::onLoad()
{
    gameWrapper->Execute([this](GameWrapper* gw) {
        cvarManager->log("=== VelocityRL Replay Sync loaded ===");
        cvarManager->log("  Opt in from VelocityRL: Miscellaneous -> Secure replay archive");
    });

    gameWrapper->HookEventPost("Function TAGame.GameEvent_Soccar_TA.OnMatchEnded",
        [this](std::string) { OnMatchEnded({}); });
    gameWrapper->HookEventPost("Function TAGame.GameEvent_Soccar_TA.EventMatchEnded",
        [this](std::string) { OnMatchEnded({}); });

    cvarManager->registerCvar("velocity_replay_debug", "0", "Verbose replay sync logging", true);
}

void VelocityReplaySync::onUnload()
{
    if (worker_.joinable()) worker_.join();
}

std::string VelocityReplaySync::VelocityConfigPath()
{
    char appdata[MAX_PATH];
    if (FAILED(SHGetFolderPathA(NULL, CSIDL_APPDATA, NULL, 0, appdata)))
        return "";
    return std::string(appdata) + "\\com.velocityrl.app\\config.json";
}

bool VelocityReplaySync::IsOptedIn()
{
    return ReadOptInFromConfig() == "true";
}

std::string VelocityReplaySync::ReadOptInFromConfig()
{
    std::ifstream f(VelocityConfigPath());
    if (!f) return "false";
    std::stringstream ss; ss << f.rdbuf();
    std::string s = ss.str();

    const std::string key = "\"replay_opt_in\"";
    auto pos = s.find(key);
    if (pos == std::string::npos) return "false";
    pos = s.find(':', pos + key.size());
    if (pos == std::string::npos) return "false";
    ++pos;
    while (pos < s.size() && (s[pos] == ' ' || s[pos] == '\t')) ++pos;
    if (s.compare(pos, 4, "true") == 0) return "true";
    return "false";
}

std::string VelocityReplaySync::ArchiveEndpoint()
{

    return kDefaultEndpoint;
}

void VelocityReplaySync::OnMatchEnded(std::vector<std::string> params)
{
    if (!IsOptedIn()) return;
    if (uploadInFlight_.load()) return;

    auto server = gameWrapper->GetGameEventAsServer();
    if (server.IsNull()) return;

    std::string playerName = "unknown";
    std::string playerId = "unknown";
    auto pc = server.GetLocalPrimaryPlayer();
    if (!pc.IsNull())
    {
        auto pri = pc.GetPRI();
        if (!pri.IsNull())
        {
            playerName = pri.GetPlayerName().ToString();
            playerId = pri.GetUniqueIdWrapper().GetIdString();
        }
    }

    std::filesystem::path replaysDir;
    {
        char docs[MAX_PATH];
        if (SUCCEEDED(SHGetFolderPathA(NULL, CSIDL_PERSONAL, NULL, 0, docs)))
            replaysDir = std::filesystem::path(docs) / "My Games" / "Rocket League" / "TAGame" / "Demos";
        else
            replaysDir = gameWrapper->GetDataFolder();
    }

    std::filesystem::path replayPath;
    std::error_code ec;
    std::filesystem::file_time_type newest{};
    for (auto& entry : std::filesystem::directory_iterator(replaysDir, ec))
    {
        if (!entry.is_regular_file()) continue;
        if (entry.path().extension().string() != ".replay") continue;
        auto t = entry.last_write_time(ec);
        if (!ec && t > newest) { newest = t; replayPath = entry.path(); }
    }
    if (replayPath.empty())
    {
        cvarManager->log("[replay-sync] no .replay file found in " + replaysDir.string());
        return;
    }

    uploadInFlight_.store(true);
    if (worker_.joinable()) worker_.join();
    std::filesystem::path pathCopy = replayPath;
    std::string nameCopy = playerName;
    std::string idCopy = playerId;
    worker_ = std::thread([this, pathCopy, nameCopy, idCopy]() {
        if (cvarManager->getCvar("velocity_replay_debug").getBoolValue())
            cvarManager->log("[replay-sync] archiving " + pathCopy.string());
        if (UploadReplay(pathCopy.string(), nameCopy, idCopy))
            cvarManager->log("[replay-sync] archived securely.");
        else
            cvarManager->log("[replay-sync] archive failed (will retry next match).");
        uploadInFlight_.store(false);
    });
}

bool VelocityReplaySync::UploadReplay(const std::string& replayPath, const std::string& playerName, const std::string& playerId)
{
    std::ifstream f(replayPath, std::ios::binary);
    if (!f) return false;
    std::stringstream ss; ss << f.rdbuf();
    std::string body = ss.str();
    if (body.empty()) return false;

    std::string endpoint = ArchiveEndpoint();
    const std::string scheme = "http://";
    if (endpoint.rfind(scheme, 0) != 0) return false;
    endpoint = endpoint.substr(scheme.size());
    auto slash = endpoint.find('/');
    std::string hostport = endpoint.substr(0, slash);
    std::string path = slash == std::string::npos ? "/" : endpoint.substr(slash);
    std::string host = hostport;
    std::string port = "80";
    auto colon = hostport.rfind(':');
    if (colon != std::string::npos) { host = hostport.substr(0, colon); port = hostport.substr(colon + 1); }

    bool ok = false;
    WSADATA wsa{};
    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0) return false;

    do
    {
        addrinfo hints{}; hints.ai_family = AF_INET; hints.ai_socktype = SOCK_STREAM;
        addrinfo* res = nullptr;
        if (getaddrinfo(host.c_str(), port.c_str(), &hints, &res) != 0 || !res) break;

        SOCKET sock = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
        if (sock == INVALID_SOCKET) { freeaddrinfo(res); break; }
        if (connect(sock, res->ai_addr, (int)res->ai_addrlen) != 0) { closesocket(sock); freeaddrinfo(res); break; }
        freeaddrinfo(res);

        std::string safeName = playerName;
        for (auto& c : safeName) if (c == '\r' || c == '\n') c = ' ';
        std::string safeId = playerId;
        for (auto& c : safeId) if (c == '\r' || c == '\n') c = ' ';

        std::string replayFile = replayPath;
        auto lastSlash = replayFile.find_last_of("\\/");
        if (lastSlash != std::string::npos) replayFile = replayFile.substr(lastSlash + 1);
        for (auto& c : replayFile) if (c == '\r' || c == '\n') c = ' ';

        std::ostringstream req;
        req << "POST " << path << " HTTP/1.1\r\n"
            << "Host: " << hostport << "\r\n"
            << "Content-Type: application/octet-stream\r\n"
            << "X-Player-Name: " << safeName << "\r\n"
            << "X-Player-Id: " << safeId << "\r\n"
            << "X-Replay-Name: " << replayFile << "\r\n"
            << "Content-Length: " << body.size() << "\r\n"
            << "Connection: close\r\n\r\n"
            << body;
        std::string reqStr = req.str();
        int sent = 0;
        while (sent < (int)reqStr.size())
        {
            int n = ::send(sock, reqStr.data() + sent, (int)reqStr.size() - sent, 0);
            if (n <= 0) break;
            sent += n;
        }
        ok = sent == (int)reqStr.size();

        char resp[1024]{};
        ::recv(sock, resp, sizeof(resp) - 1, 0);
        ok = ok && strstr(resp, " 2") != nullptr;
        closesocket(sock);
    } while (false);
    WSACleanup();
    return ok;
}
