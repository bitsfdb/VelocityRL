#include <windows.h>
#include <shellapi.h>
#include <stdio.h>
#include <string.h>

extern "C" {

static wchar_t g_last_error[1024] = L"";

static void set_last_error(const wchar_t* message) {
    wcsncpy_s(g_last_error, _countof(g_last_error), message, _TRUNCATE);
}

__declspec(dllexport) int __stdcall TagameProbe_GetLastErrorW(wchar_t* buffer, int buffer_len) {
    if (!buffer || buffer_len <= 0) {
        return (int)wcslen(g_last_error);
    }
    wcsncpy_s(buffer, (size_t)buffer_len, g_last_error, _TRUNCATE);
    return 0;
}

static int get_module_dir(wchar_t* out, size_t out_len) {
    HMODULE self = NULL;
    if (!GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            (LPCWSTR)&get_module_dir,
            &self)) {
        set_last_error(L"GetModuleHandleExW failed");
        return 0;
    }
    if (!GetModuleFileNameW(self, out, (DWORD)out_len)) {
        set_last_error(L"GetModuleFileNameW failed");
        return 0;
    }
    wchar_t* slash = wcsrchr(out, L'\\');
    if (slash) {
        *slash = L'\0';
    }
    return 1;
}

static int build_paths(wchar_t* script_path, size_t script_len, wchar_t* repo_python_dir, size_t repo_len) {
    wchar_t module_dir[MAX_PATH];
    if (!get_module_dir(module_dir, _countof(module_dir))) {
        return 0;
    }

    // Expected layout: <repo>/tools/nametag_probe/nametag_probe.dll
    if (swprintf_s(script_path, script_len, L"%s\\..\\..\\python\\tagame_nametag_probe.py", module_dir) < 0) {
        set_last_error(L"Failed to build script path");
        return 0;
    }
    if (swprintf_s(repo_python_dir, repo_len, L"%s\\..\\..\\python", module_dir) < 0) {
        set_last_error(L"Failed to build python dir path");
        return 0;
    }
    return 1;
}

static int run_python_command(const wchar_t* command_line) {
    STARTUPINFOW si;
    PROCESS_INFORMATION pi;
    DWORD exit_code = 1;

    ZeroMemory(&si, sizeof(si));
    si.cb = sizeof(si);
    ZeroMemory(&pi, sizeof(pi));

    wchar_t mutable_cmd[32768];
    wcsncpy_s(mutable_cmd, _countof(mutable_cmd), command_line, _TRUNCATE);

    if (!CreateProcessW(NULL, mutable_cmd, NULL, NULL, FALSE, CREATE_NO_WINDOW, NULL, NULL, &si, &pi)) {
        swprintf_s(g_last_error, _countof(g_last_error), L"CreateProcessW failed (%lu)", GetLastError());
        return 1;
    }

    WaitForSingleObject(pi.hProcess, INFINITE);
    GetExitCodeProcess(pi.hProcess, &exit_code);
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return (int)exit_code;
}

static int run_probe(const wchar_t* subcommand, const wchar_t* upk_path, const wchar_t* out_json, const wchar_t* extra_args) {
    wchar_t script_path[MAX_PATH];
    wchar_t repo_python_dir[MAX_PATH];
    if (!build_paths(script_path, _countof(script_path), repo_python_dir, _countof(repo_python_dir))) {
        return 1;
    }

    if (GetFileAttributesW(script_path) == INVALID_FILE_ATTRIBUTES) {
        swprintf_s(g_last_error, _countof(g_last_error), L"Probe script not found: %s", script_path);
        return 1;
    }

    wchar_t command_line[32768];
    if (extra_args && extra_args[0]) {
        swprintf_s(
            command_line,
            _countof(command_line),
            L"py -3 \"%s\" %s \"%s\" %s -o \"%s\"",
            script_path,
            subcommand,
            upk_path,
            extra_args,
            out_json);
    } else {
        swprintf_s(
            command_line,
            _countof(command_line),
            L"py -3 \"%s\" %s \"%s\" -o \"%s\"",
            script_path,
            subcommand,
            upk_path,
            out_json);
    }

    int code = run_python_command(command_line);
    if (code != 0) {
        swprintf_s(
            g_last_error,
            _countof(g_last_error),
            L"Probe command failed (exit %d). Command: %s",
            code,
            command_line);
    } else {
        g_last_error[0] = L'\0';
    }
    return code;
}

__declspec(dllexport) int __stdcall TagameProbe_DumpW(const wchar_t* upk_path, const wchar_t* out_json) {
    if (!upk_path || !out_json) {
        set_last_error(L"TagameProbe_DumpW: null argument");
        return 1;
    }
    return run_probe(L"dump", upk_path, out_json, NULL);
}

__declspec(dllexport) int __stdcall TagameProbe_SnapshotW(const wchar_t* upk_path, const wchar_t* out_json) {
    if (!upk_path || !out_json) {
        set_last_error(L"TagameProbe_SnapshotW: null argument");
        return 1;
    }
    return run_probe(L"snapshot", upk_path, out_json, NULL);
}

__declspec(dllexport) int __stdcall TagameProbe_DiffSnapshotW(
    const wchar_t* after_upk,
    const wchar_t* snapshot_json,
    const wchar_t* out_json) {
    if (!after_upk || !snapshot_json || !out_json) {
        set_last_error(L"TagameProbe_DiffSnapshotW: null argument");
        return 1;
    }

    wchar_t extra_args[1024];
    swprintf_s(extra_args, _countof(extra_args), L"--snapshot \"%s\"", snapshot_json);
    return run_probe(L"diff", after_upk, out_json, extra_args);
}

__declspec(dllexport) int __stdcall TagameProbe_DiffW(
    const wchar_t* before_upk,
    const wchar_t* after_upk,
    const wchar_t* out_json) {
    if (!before_upk || !after_upk || !out_json) {
        set_last_error(L"TagameProbe_DiffW: null argument");
        return 1;
    }

    wchar_t script_path[MAX_PATH];
    wchar_t repo_python_dir[MAX_PATH];
    if (!build_paths(script_path, _countof(script_path), repo_python_dir, _countof(repo_python_dir))) {
        return 1;
    }

    wchar_t command_line[32768];
    swprintf_s(
        command_line,
        _countof(command_line),
        L"py -3 \"%s\" diff \"%s\" \"%s\" -o \"%s\"",
        script_path,
        after_upk,
        before_upk,
        out_json);

    int code = run_python_command(command_line);
    if (code != 0) {
        swprintf_s(
            g_last_error,
            _countof(g_last_error),
            L"Probe diff failed (exit %d). Command: %s",
            code,
            command_line);
    } else {
        g_last_error[0] = L'\0';
    }
    return code;
}

BOOL WINAPI DllMain(HINSTANCE hinstDLL, DWORD fdwReason, LPVOID lpvReserved) {
    (void)hinstDLL;
    (void)fdwReason;
    (void)lpvReserved;
    return TRUE;
}

}  // extern "C"
