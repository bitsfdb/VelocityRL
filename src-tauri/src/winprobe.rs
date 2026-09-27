#![allow(dead_code)]

#[cfg(windows)]
struct AutoHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for AutoHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 as isize != -1 {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
struct AlignedTcpBuffer {
    ptr: *mut u8,
    layout: std::alloc::Layout,
}

#[cfg(windows)]
impl AlignedTcpBuffer {
    fn new(size: usize, align: usize) -> Option<Self> {
        let layout = std::alloc::Layout::from_size_align(size, align).ok()?;
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            None
        } else {
            Some(Self { ptr, layout })
        }
    }
}

#[cfg(windows)]
impl Drop for AlignedTcpBuffer {
    fn drop(&mut self) {
        unsafe {
            std::alloc::dealloc(self.ptr, self.layout);
        }
    }
}

#[cfg(windows)]
pub fn process_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    if pid == 0 {
        return None;
    }
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap as isize == -1 {
            return None;
        }
        let _guard = AutoHandle(snap);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                if entry.th32ProcessID == pid {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    found = Some(String::from_utf16_lossy(&entry.szExeFile[..len]));
                    break;
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        found
    }
}

#[cfg(windows)]
pub fn find_process_any(names: &[&str]) -> Option<(u32, String)> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let wanted: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap as isize == -1 {
            return None;
        }
        let _guard = AutoHandle(snap);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut hit = None;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                let lower = name.to_ascii_lowercase();
                if wanted.iter().any(|w| lower == *w) {
                    hit = Some((entry.th32ProcessID, name));
                    break;
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        hit
    }
}

#[cfg(windows)]
pub fn process_path(pid: u32) -> Option<std::path::PathBuf> {
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() || handle as isize == -1 {
            return None;
        }
        let _guard = AutoHandle(handle);
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        if QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size) != 0 && size > 0 {
            let path_str = String::from_utf16_lossy(&buf[..size as usize]);
            Some(std::path::PathBuf::from(path_str))
        } else {
            None
        }
    }
}

#[cfg(target_os = "linux")]
pub fn process_path(pid: u32) -> Option<std::path::PathBuf> {
    if pid == 0 {
        return None;
    }
    if let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) {
        for part in cmdline.split(|&b| b == 0) {
            let s = String::from_utf8_lossy(part);
            let s_lower = s.to_lowercase();
            if s_lower.ends_with(".exe") || s_lower.contains("rocketleague") {
                if s.starts_with(r"Z:\") || s.starts_with(r"z:\") {
                    let linux_path = s[2..].replace('\\', "/");
                    let p = std::path::PathBuf::from(linux_path);
                    if p.exists() {
                        return Some(p);
                    }
                } else if s.starts_with('/') {
                    let p = std::path::PathBuf::from(s.into_owned());
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    if let Ok(link) = std::fs::read_link(format!("/proc/{pid}/exe")) {
        let name = link.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.contains("wine") && !name.contains("proton") {
            return Some(link);
        }
    }
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn process_path(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

#[cfg(windows)]
pub fn terminate_process(pid: u32) -> bool {
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    if pid == 0 || pid == std::process::id() {
        return false;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() && handle as isize != -1 {
            let _guard = AutoHandle(handle);
            if TerminateProcess(handle, 1) != 0 {
                return true;
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output();
    }
    true
}

#[cfg(target_os = "linux")]
pub fn terminate_process(pid: u32) -> bool {
    if pid == 0 || pid == std::process::id() {
        return false;
    }
    unsafe {
        if libc::kill(pid as i32, libc::SIGTERM) == 0 {
            let _ = libc::kill(pid as i32, libc::SIGKILL);
            return true;
        }
    }
    false
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn terminate_process(_pid: u32) -> bool {
    false
}

#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let _guard = AutoHandle(token);
        let mut elev: TOKEN_ELEVATION = std::mem::zeroed();
        let mut ret: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elev as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        );
        ok != 0 && elev.TokenIsElevated != 0
    }
}

#[cfg(windows)]
pub fn loopback_443_owner() -> Option<u32> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_MODULE, MIB_TCP6TABLE_OWNER_MODULE,
        MIB_TCPROW_OWNER_MODULE, MIB_TCPTABLE_OWNER_MODULE, TCP_TABLE_OWNER_MODULE_LISTENER,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    const MIB_TCP_STATE_LISTEN: u32 = 2;
    const PORT: u32 = 443;

    unsafe fn v4_rows() -> Vec<(u32, u32)> {
        let mut size: u32 = 0;
        let _ = GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_MODULE_LISTENER,
            0,
        );
        let min_header = std::mem::size_of::<MIB_TCPTABLE_OWNER_MODULE>();
        if (size as usize) < min_header {
            return Vec::new();
        }
        let align = std::mem::align_of::<MIB_TCPTABLE_OWNER_MODULE>();
        let Some(buf) = AlignedTcpBuffer::new(size as usize, align) else {
            return Vec::new();
        };
        if GetExtendedTcpTable(
            buf.ptr as *mut core::ffi::c_void,
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_MODULE_LISTENER,
            0,
        ) != 0
        {
            return Vec::new();
        }
        let t = buf.ptr as *const MIB_TCPTABLE_OWNER_MODULE;
        let n = (*t).dwNumEntries as usize;
        if n == 0 {
            return Vec::new();
        }
        let row_size = std::mem::size_of::<MIB_TCPROW_OWNER_MODULE>();
        let header_overhead = std::mem::size_of::<u32>();
        let needed = match n.checked_mul(row_size).and_then(|tot| header_overhead.checked_add(tot)) {
            Some(v) => v,
            None => return Vec::new(),
        };
        if needed > size as usize {
            return Vec::new();
        }
        let rows = std::slice::from_raw_parts((*t).table.as_ptr(), n);
        rows.iter()
            .filter(|r| r.dwState == MIB_TCP_STATE_LISTEN)
            .filter(|r| r.dwLocalAddr == 0 || (r.dwLocalAddr & 0xFF) == 127)
            .map(|r| (u16::from_be(r.dwLocalPort as u16) as u32, r.dwOwningPid))
            .collect()
    }

    unsafe fn v6_rows() -> Vec<(u32, u32, [u8; 16])> {
        let mut size: u32 = 0;
        let _ = GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET6 as u32,
            TCP_TABLE_OWNER_MODULE_LISTENER,
            0,
        );
        let min_header = std::mem::size_of::<MIB_TCP6TABLE_OWNER_MODULE>();
        if (size as usize) < min_header {
            return Vec::new();
        }
        let align = std::mem::align_of::<MIB_TCP6TABLE_OWNER_MODULE>();
        let Some(buf) = AlignedTcpBuffer::new(size as usize, align) else {
            return Vec::new();
        };
        if GetExtendedTcpTable(
            buf.ptr as *mut core::ffi::c_void,
            &mut size,
            0,
            AF_INET6 as u32,
            TCP_TABLE_OWNER_MODULE_LISTENER,
            0,
        ) != 0
        {
            return Vec::new();
        }
        let t = buf.ptr as *const MIB_TCP6TABLE_OWNER_MODULE;
        let n = (*t).dwNumEntries as usize;
        if n == 0 {
            return Vec::new();
        }
        let row_size = std::mem::size_of::<MIB_TCP6ROW_OWNER_MODULE>();
        let header_overhead = std::mem::size_of::<u32>();
        let needed = match n.checked_mul(row_size).and_then(|tot| header_overhead.checked_add(tot)) {
            Some(v) => v,
            None => return Vec::new(),
        };
        if needed > size as usize {
            return Vec::new();
        }
        let rows = std::slice::from_raw_parts((*t).table.as_ptr(), n);
        rows.iter()
            .filter(|r| r.dwState == MIB_TCP_STATE_LISTEN)
            .filter(|r| {
                (r.ucLocalAddr[..15].iter().all(|&b| b == 0) && r.ucLocalAddr[15] == 1)
                    || r.ucLocalAddr.iter().all(|&b| b == 0)
            })
            .map(|r| (u16::from_be(r.dwLocalPort as u16) as u32, r.dwOwningPid, r.ucLocalAddr))
            .collect()
    }

    let mut candidates: Vec<u32> = Vec::new();
    for (port, pid) in unsafe { v4_rows() } {
        if port != PORT {
            continue;
        }
        candidates.push(pid);
    }
    for (port, pid, _addr) in unsafe { v6_rows() } {
        if port != PORT {
            continue;
        }
        candidates.push(pid);
    }
    candidates.into_iter().next()
}

#[cfg(target_os = "linux")]
pub fn process_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_string());

    if let Some(ref c) = comm {
        let c_lower = c.to_lowercase();
        if c_lower.starts_with("wine") || c_lower == "wineserver" || c_lower.starts_with("proton") {
            if let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) {
                for part in cmdline.split(|&b| b == 0) {
                    let s = String::from_utf8_lossy(part);
                    let s_lower = s.to_lowercase();
                    if s_lower.ends_with(".exe") {
                        let filename = s.rsplit(['/', '\\']).next().unwrap_or(&s);
                        return Some(filename.to_string());
                    }
                }
            }
        }
    }
    comm
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn process_name(_pid: u32) -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
pub fn find_process_any(names: &[&str]) -> Option<(u32, String)> {
    let wanted: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    let proc_dir = std::fs::read_dir("/proc").ok()?;
    for entry in proc_dir.flatten() {
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(pid) = file_name.parse::<u32>() else {
            continue;
        };
        if let Some(name) = process_name(pid) {
            let lower = name.to_ascii_lowercase();
            if wanted.iter().any(|w| lower == *w || lower.contains(w) || w.contains(&lower)) {
                return Some((pid, name));
            }
        }
        if let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) {
            let cmdline_str = String::from_utf8_lossy(&cmdline).to_ascii_lowercase();
            for w in &wanted {
                if cmdline_str.contains(w.as_str()) {
                    let name = process_name(pid).unwrap_or_else(|| w.clone());
                    return Some((pid, name));
                }
            }
        }
    }
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn find_process_any(_names: &[&str]) -> Option<(u32, String)> {
    None
}

#[cfg(target_os = "linux")]
pub fn is_elevated() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn is_elevated() -> bool {
    false
}

#[cfg(windows)]
pub fn flush_dns_cache() -> bool {
    #[link(name = "dnsapi")]
    extern "system" {
        fn DnsFlushResolverCache() -> windows_sys::Win32::Foundation::BOOL;
    }
    unsafe { DnsFlushResolverCache() != 0 }
}

#[cfg(target_os = "linux")]
pub fn loopback_443_owner() -> Option<u32> {
    let mut inodes: Vec<u64> = Vec::new();
    for file in &["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(content) = std::fs::read_to_string(file) {
            for line in content.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 10 {
                    continue;
                }
                let local_addr = parts[1];
                let state = parts[3];
                if state == "0A" && local_addr.ends_with(":01BB") {
                    if let Ok(inode) = parts[9].parse::<u64>() {
                        inodes.push(inode);
                    }
                }
            }
        }
    }
    if inodes.is_empty() {
        return None;
    }

    let proc_dir = std::fs::read_dir("/proc").ok()?;
    for entry in proc_dir.flatten() {
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(pid) = file_name.parse::<u32>() else {
            continue;
        };
        let fd_dir = match std::fs::read_dir(format!("/proc/{pid}/fd")) {
            Ok(d) => d,
            Err(_) => continue,
        };
        for fd in fd_dir.flatten() {
            if let Ok(target) = std::fs::read_link(fd.path()) {
                let s = target.to_string_lossy();
                for &inode in &inodes {
                    let pat = format!("socket:[{inode}]");
                    if s == pat {
                        return Some(pid);
                    }
                }
            }
        }
    }
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn loopback_443_owner() -> Option<u32> {
    None
}

#[cfg(target_os = "linux")]
pub fn flush_dns_cache() -> bool {
    let _ = std::process::Command::new("resolvectl")
        .arg("flush-caches")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let _ = std::process::Command::new("systemd-resolve")
        .arg("--flush-caches")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let _ = std::process::Command::new("nscd")
        .args(["-i", "hosts"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    true
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn flush_dns_cache() -> bool {
    true
}

#[cfg(windows)]
pub fn to_wide_null(p: &std::path::Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
pub fn replace_file_atomic(from: &std::path::Path, to: &std::path::Path) -> Result<(), std::io::Error> {
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let src = to_wide_null(from);
    let dst = to_wide_null(to);
    let ok = unsafe {
        MoveFileExW(
            src.as_ptr(),
            dst.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn replace_file_atomic(from: &std::path::Path, to: &std::path::Path) -> Result<(), std::io::Error> {
    std::fs::rename(from, to)
}

#[cfg(windows)]
pub fn refresh_wininet_settings() {
    #[link(name = "wininet")]
    extern "system" {
        fn InternetSetOptionW(
            h_internet: *mut std::ffi::c_void,
            dw_option: u32,
            lp_buffer: *mut std::ffi::c_void,
            dw_buffer_length: u32,
        ) -> i32;
    }
    const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;
    const INTERNET_OPTION_REFRESH: u32 = 37;
    unsafe {
        InternetSetOptionW(std::ptr::null_mut(), INTERNET_OPTION_SETTINGS_CHANGED, std::ptr::null_mut(), 0);
        InternetSetOptionW(std::ptr::null_mut(), INTERNET_OPTION_REFRESH, std::ptr::null_mut(), 0);
    }
}

#[cfg(not(windows))]
pub fn refresh_wininet_settings() {}
