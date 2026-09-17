
#include <winsock2.h>
#include <ws2tcpip.h>
#include <in6addr.h>

#include "traffic_recorder.h"
#include <MinHook.h>
#include <cstring>
#include <unordered_map>
#include <chrono>

using fn_send   = int(SOCKET, const char*, int, int);
using fn_recv   = int(SOCKET, char*, int, int);
using fn_sendto = int(SOCKET, const char*, int, int, const struct sockaddr*, int);
using fn_recvfrom = int(SOCKET, char*, int, int, struct sockaddr*, int*);

static fn_send*    o_send    = nullptr;
static fn_recv*    o_recv    = nullptr;
static fn_sendto*  o_sendto  = nullptr;
static fn_recvfrom* o_recvfrom = nullptr;

struct SocketInfo {
    uint32_t remote_ip;
    uint16_t remote_port;
    uint16_t local_port;
};
static std::unordered_map<SOCKET, SocketInfo> g_sock_map;
static std::mutex g_sock_mtx;

static void track_socket(SOCKET s, const struct sockaddr* addr) {
    if (!addr) return;
    std::lock_guard lock(g_sock_mtx);
    SocketInfo info{};
    if (addr->sa_family == AF_INET) {
        auto* sin = reinterpret_cast<const struct sockaddr_in*>(addr);
        info.remote_ip   = ntohl(sin->sin_addr.s_addr);
        info.remote_port = ntohs(sin->sin_port);
    }
    g_sock_map[s] = info;
}

static SocketInfo get_socket_info(SOCKET s) {
    std::lock_guard lock(g_sock_mtx);
    auto it = g_sock_map.find(s);
    if (it != g_sock_map.end()) return it->second;
    return {};
}

static int64_t now_ms() {
    using namespace std::chrono;
    return duration_cast<milliseconds>(
        system_clock::now().time_since_epoch()
    ).count();
}

static void detect_local_ip(const struct sockaddr* addr) {
    if (!addr || addr->sa_family != AF_INET) return;
    auto& rec = TrafficRecorder::instance();

}

static int hk_send(SOCKET s, const char* buf, int len, int flags) {
    int result = o_send(s, buf, len, flags);
    if (result > 0 && TrafficRecorder::instance().is_recording()) {
        auto info = get_socket_info(s);
        TrafficRecorder::instance().on_send(
            reinterpret_cast<const uint8_t*>(buf),
            static_cast<size_t>(result),
            info.remote_ip,
            info.remote_port
        );
    }
    return result;
}

static int hk_recv(SOCKET s, char* buf, int len, int flags) {
    int result = o_recv(s, buf, len, flags);
    if (result > 0 && TrafficRecorder::instance().is_recording()) {
        auto info = get_socket_info(s);
        TrafficRecorder::instance().on_recv(
            reinterpret_cast<const uint8_t*>(buf),
            static_cast<size_t>(result),
            info.remote_ip,
            info.remote_port
        );
    }
    return result;
}

static int hk_sendto(SOCKET s, const char* buf, int len, int flags,
                      const struct sockaddr* addr, int addrlen) {

    if (addr) track_socket(s, addr);

    int result = o_sendto(s, buf, len, flags, addr, addrlen);
    if (result > 0 && TrafficRecorder::instance().is_recording()) {
        uint32_t dst_ip = 0;
        uint16_t dst_port = 0;
        if (addr && addr->sa_family == AF_INET) {
            auto* sin = reinterpret_cast<const struct sockaddr_in*>(addr);
            dst_ip   = ntohl(sin->sin_addr.s_addr);
            dst_port = ntohs(sin->sin_port);
        }
        TrafficRecorder::instance().on_send(
            reinterpret_cast<const uint8_t*>(buf),
            static_cast<size_t>(result),
            dst_ip,
            dst_port
        );
    }
    return result;
}

static int hk_recvfrom(SOCKET s, char* buf, int len, int flags,
                        struct sockaddr* addr, int* addrlen) {

    struct sockaddr_in src_addr{};
    int src_len = sizeof(src_addr);

    int result = o_recvfrom(s, buf, len, flags,
                            addr ? addr : reinterpret_cast<struct sockaddr*>(&src_addr),
                            addr ? addrlen : &src_len);

    if (result > 0 && TrafficRecorder::instance().is_recording()) {
        uint32_t src_ip = 0;
        uint16_t src_port = 0;
        auto* effective = addr ? addr : reinterpret_cast<struct sockaddr*>(&src_addr);
        if (effective->sa_family == AF_INET) {
            auto* sin = reinterpret_cast<struct sockaddr_in*>(effective);
            src_ip   = ntohl(sin->sin_addr.s_addr);
            src_port = ntohs(sin->sin_port);
        }
        TrafficRecorder::instance().on_recv(
            reinterpret_cast<const uint8_t*>(buf),
            static_cast<size_t>(result),
            src_ip,
            src_port
        );
    }
    return result;
}

bool install_winsock_hooks() {
    MH_STATUS s1 = MH_Initialize();
    if (s1 != MH_OK && s1 != MH_ERROR_ALREADY_INITIALIZED) return false;

    HMODULE ws2 = GetModuleHandleA("ws2_32.dll");
    if (!ws2) ws2 = LoadLibraryA("ws2_32.dll");
    if (!ws2) return false;

    auto* p_send    = reinterpret_cast<void*>(GetProcAddress(ws2, "send"));
    auto* p_recv    = reinterpret_cast<void*>(GetProcAddress(ws2, "recv"));
    auto* p_sendto  = reinterpret_cast<void*>(GetProcAddress(ws2, "sendto"));
    auto* p_recvfrom = reinterpret_cast<void*>(GetProcAddress(ws2, "recvfrom"));

    if (!p_send || !p_recv || !p_sendto || !p_recvfrom) return false;

    MH_CreateHook(p_send,    reinterpret_cast<void*>(&hk_send),    reinterpret_cast<void**>(&o_send));
    MH_CreateHook(p_recv,    reinterpret_cast<void*>(&hk_recv),    reinterpret_cast<void**>(&o_recv));
    MH_CreateHook(p_sendto,  reinterpret_cast<void*>(&hk_sendto),  reinterpret_cast<void**>(&o_sendto));
    MH_CreateHook(p_recvfrom,reinterpret_cast<void*>(&hk_recvfrom),reinterpret_cast<void**>(&o_recvfrom));

    MH_EnableHook(MH_ALL_HOOKS);
    return true;
}

void remove_winsock_hooks() {
    MH_DisableHook(MH_ALL_HOOKS);
    MH_Uninitialize();
}
