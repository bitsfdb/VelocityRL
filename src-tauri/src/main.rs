#![windows_subsystem = "windows"]

fn main() {
  let args: Vec<String> = std::env::args().collect();

  if args.iter().any(|a| a == "--help" || a == "-h" || a == "help") {
    print_help();
    return;
  }

  let is_recover = args.iter().any(|a| {
    a == "--recover"
      || a == "recover"
      || a == "--restore"
      || a == "restore"
      || a == "--revert"
      || a == "revert"
      || a == "--clean"
      || a == "clean"
      || a == "--cleanup"
      || a == "cleanup"
      || a == "--reset"
      || a == "reset"
      || a == "--uninstall"
      || a == "uninstall"
  });

  #[cfg(target_os = "linux")]
  {
    let is_root = unsafe { libc::geteuid() == 0 };

    if is_recover {
      if !is_root {
        eprintln!("[-] Error: You must run recovery as root (sudo).");
        eprintln!(
          "[-] Please run: sudo {} --recover",
          std::env::args().next().unwrap_or_else(|| "./velocity-rl".into())
        );
        std::process::exit(1);
      }
      run_cli_recover();
      return;
    }

    if !is_root {
      eprintln!("[-] Error: You must run VelocityRL as root (sudo).");
      eprintln!(
        "[-] Please run: sudo {}",
        std::env::args().next().unwrap_or_else(|| "./velocity-rl".into())
      );
      std::process::exit(1);
    }

    if args.iter().any(|a| a == "--setup" || a == "setup") {
      run_cli_setup();
      return;
    }

    // Automatically configure system settings (port 443, root CA, /etc/hosts)
    run_cli_setup();

    // Prepare GUI environment for desktop session
    setup_gui_environment();

    // Drop root privileges back to the desktop user so GTK/WebKit/Glycin/bwrap run safely
    drop_privileges_to_user();
  }

  #[cfg(windows)]
  {
    if is_recover {
      run_cli_recover_windows();
      return;
    }
  }

  app_lib::run();
}

fn print_help() {
  println!("==========================================");
  println!("  VelocityRL v{}", env!("CARGO_PKG_VERSION"));
  println!("==========================================");
  println!();
  println!("Usage: velocity-rl [OPTIONS]");
  println!();
  println!("Options:");
  println!("  --setup       Configure system permissions (port 443, root CA, /etc/hosts)");
  println!("  --recover     Revert all system changes so Rocket League works independently");
  println!("  --help, -h    Show this help message");
  println!();
  println!("Note: VelocityRL requires root privileges on Linux (run with sudo).");
  println!();
}

#[cfg(target_os = "linux")]
fn setup_gui_environment() {
  if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
  }

  // WebKitGTK sandbox (bubblewrap) cannot create unprivileged user namespaces when running as root
  if std::env::var_os("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS").is_none() {
    std::env::set_var("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS", "1");
  }

  let sudo_user_opt = std::env::var("SUDO_USER").ok().filter(|u| !u.trim().is_empty() && u.trim() != "root");
  let sudo_uid_opt = std::env::var("SUDO_UID").ok().filter(|id| !id.trim().is_empty() && id.trim() != "0");

  let uid = sudo_uid_opt.unwrap_or_else(|| {
    if let Ok(entries) = std::fs::read_dir("/run/user") {
      for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s != "0" && s.chars().all(|c| c.is_ascii_digit()) {
          return s.to_string();
        }
      }
    }
    "1000".to_string()
  });

  let user_runtime = std::path::PathBuf::from(format!("/run/user/{uid}"));
  if user_runtime.exists() {
    let cur_rt = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    if cur_rt.is_empty() || cur_rt == "/run/user/0" {
      std::env::set_var("XDG_RUNTIME_DIR", &user_runtime);
    }

    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
      if let Ok(entries) = std::fs::read_dir(&user_runtime) {
        for entry in entries.flatten() {
          let name = entry.file_name();
          let s = name.to_string_lossy();
          if s.starts_with("wayland-") && !s.ends_with(".lock") {
            std::env::set_var("WAYLAND_DISPLAY", &*s);
            break;
          }
        }
      }
    }

    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
      let bus_path = user_runtime.join("bus");
      if bus_path.exists() {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", bus_path.display()));
      }
    }
  }

  // Scan user processes in /proc to pick up session environment variables if still missing
  if std::env::var_os("WAYLAND_DISPLAY").is_none() || std::env::var_os("DISPLAY").is_none() {
    if let Ok(entries) = std::fs::read_dir("/proc") {
      for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if !s.chars().all(|c| c.is_ascii_digit()) {
          continue;
        }
        let env_file = entry.path().join("environ");
        if let Ok(raw) = std::fs::read(&env_file) {
          let mut found_wayland = None;
          let mut found_display = None;
          for chunk in raw.split(|&b| b == 0) {
            if let Ok(line) = std::str::from_utf8(chunk) {
              if let Some(val) = line.strip_prefix("WAYLAND_DISPLAY=") {
                if !val.is_empty() {
                  found_wayland = Some(val.to_string());
                }
              } else if let Some(val) = line.strip_prefix("DISPLAY=") {
                if !val.is_empty() {
                  found_display = Some(val.to_string());
                }
              }
            }
          }
          if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            if let Some(w) = found_wayland {
              std::env::set_var("WAYLAND_DISPLAY", w);
            }
          }
          if std::env::var_os("DISPLAY").is_none() {
            if let Some(d) = found_display {
              std::env::set_var("DISPLAY", d);
            }
          }
          if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_some() {
            break;
          }
        }
      }
    }
  }

  // Restore user home & directories
  if let Some(ref u) = sudo_user_opt {
    let user_home = std::path::PathBuf::from("/home").join(u.trim());
    if user_home.exists() {
      std::env::set_var("HOME", &user_home);

      if std::env::var_os("XAUTHORITY").is_none() {
        let user_xauth = user_home.join(".Xauthority");
        if user_xauth.exists() {
          std::env::set_var("XAUTHORITY", &user_xauth);
        } else {
          let rt_xauth = user_runtime.join("Xauthority");
          if rt_xauth.exists() {
            std::env::set_var("XAUTHORITY", &rt_xauth);
          }
        }
      }

      if std::env::var_os("XDG_CONFIG_HOME").is_none() {
        std::env::set_var("XDG_CONFIG_HOME", user_home.join(".config"));
      }

      if std::env::var_os("XDG_DATA_HOME").is_none() {
        std::env::set_var("XDG_DATA_HOME", user_home.join(".local/share"));
      }

      // If X11 / Xwayland is used, authorize root via xhost
      let _ = std::process::Command::new("su")
        .args([u.trim(), "-c", "xhost +si:localuser:root 2>/dev/null"])
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status();
    }
  }

  // Fallback for DISPLAY if still unset
  if std::env::var_os("DISPLAY").is_none() {
    if std::path::Path::new("/tmp/.X11-unix/X0").exists() {
      std::env::set_var("DISPLAY", ":0");
    } else if std::path::Path::new("/tmp/.X11-unix/X1").exists() {
      std::env::set_var("DISPLAY", ":1");
    }
  }

  // Instruct GDK to prioritize Wayland when available
  if std::env::var_os("GDK_BACKEND").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_some() {
    std::env::set_var("GDK_BACKEND", "wayland,x11");
  }

  let display_str = std::env::var("WAYLAND_DISPLAY")
    .map(|w| format!("Wayland ({w})"))
    .or_else(|_| std::env::var("DISPLAY").map(|d| format!("X11 ({d})")))
    .unwrap_or_else(|_| "none".into());
  println!("[*] Display server target: {display_str}");
}

#[cfg(target_os = "linux")]
fn run_cli_recover() {
  println!("==========================================");
  println!("  VelocityRL System Recovery");
  println!("==========================================");

  println!("[*] Restoring /etc/hosts...");
  if let Ok(content) = std::fs::read_to_string("/etc/hosts") {
    let lines: Vec<&str> = content
      .lines()
      .filter(|line| {
        !line.contains("config.psynet.gg")
          && !line.contains("api.rlpp.psynet.gg")
          && !line.contains("ws.rlpp.psynet.gg")
          && !line.contains("api.epicgames.dev")
      })
      .collect();
    let mut new_text = lines.join("\n");
    if !new_text.is_empty() {
      new_text.push('\n');
    }
    let _ = std::fs::write("/etc/hosts", new_text);
    println!("    [✓] Removed VelocityRL domain redirects from /etc/hosts");
  }

  println!("[*] Flushing DNS cache...");
  let _ = std::process::Command::new("systemd-resolve").arg("--flush-caches").status();
  let _ = std::process::Command::new("resolvectl").arg("flush-caches").status();
  let _ = std::process::Command::new("nscd").args(["-i", "hosts"]).status();

  println!("[*] Disabling Wine/Proton proxies...");
  app_lib::psynet::set_system_proxy_enabled(false);
  println!("    [✓] Disabled system proxy in Wine/Proton user.reg");

  println!("[*] Removing VelocityRL Root CA from system trust store...");
  let mut ca_removed = false;
  let arch_ca = std::path::Path::new("/etc/ca-certificates/trust-source/anchors/velocityrl_ca.crt");
  if arch_ca.exists() {
    let _ = std::fs::remove_file(arch_ca);
    let _ = std::process::Command::new("update-ca-trust").status();
    ca_removed = true;
  }
  let deb_ca = std::path::Path::new("/usr/local/share/ca-certificates/velocityrl_ca.crt");
  if deb_ca.exists() {
    let _ = std::fs::remove_file(deb_ca);
    let _ = std::process::Command::new("update-ca-certificates").args(["--fresh"]).status();
    ca_removed = true;
  }
  let rhel_ca = std::path::Path::new("/etc/pki/ca-trust/source/anchors/velocityrl_ca.crt");
  if rhel_ca.exists() {
    let _ = std::fs::remove_file(rhel_ca);
    let _ = std::process::Command::new("update-ca-trust").status();
    ca_removed = true;
  }
  let tmp_ca = std::env::temp_dir().join("velocityrl_ca_remove.crt");
  let ca_bytes = include_bytes!("../resources/certs/velocityrl_ca.crt");
  if std::fs::write(&tmp_ca, ca_bytes).is_ok() {
    let _ = std::process::Command::new("trust")
      .args(["anchor", "--remove", &tmp_ca.to_string_lossy()])
      .stderr(std::process::Stdio::null())
      .status();
    let _ = std::fs::remove_file(&tmp_ca);
  }
  if ca_removed {
    println!("    [✓] Removed root certificate and updated trust store");
  } else {
    println!("    [✓] No certificate left in system trust anchors");
  }

  println!("[*] Removing custom sysctl configuration...");
  let sysctl_file = std::path::Path::new("/etc/sysctl.d/50-velocityrl.conf");
  if sysctl_file.exists() {
    let _ = std::fs::remove_file(sysctl_file);
    println!("    [✓] Removed /etc/sysctl.d/50-velocityrl.conf");
  }

  println!("[*] Cleaning up temporary proxy files...");
  let _ = std::fs::remove_dir_all("/tmp/VelocityRL_proxy");
  println!("    [✓] Cleared /tmp/VelocityRL_proxy");

  println!("==========================================");
  println!("  [SUCCESS] All settings restored to normal!");
  println!("  Rocket League will now run independently.");
  println!("==========================================");
}

#[cfg(windows)]
fn run_cli_recover_windows() {
  println!("==========================================");
  println!("  VelocityRL System Recovery");
  println!("==========================================");
  println!("[*] Reverting hosts file...");
  let _ = app_lib::psynet::revert_config_hosts();
  println!("[*] Disabling system proxy...");
  app_lib::psynet::set_system_proxy_enabled(false);
  println!("[*] Flushing DNS cache...");
  let _ = std::process::Command::new("ipconfig").arg("/flushdns").status();
  println!("==========================================");
  println!("  [SUCCESS] System restored to normal!");
  println!("==========================================");
}

#[cfg(target_os = "linux")]
fn run_cli_setup() {
  println!("==========================================");
  println!("  VelocityRL Linux System Setup");
  println!("==========================================");

  println!("[*] Configuring port 443 for regular user processes...");
  let _ = std::process::Command::new("sysctl")
    .args(["-w", "net.ipv4.ip_unprivileged_port_start=80"])
    .status();
  let _ = std::fs::write("/etc/sysctl.d/50-velocityrl.conf", "net.ipv4.ip_unprivileged_port_start = 80\n");

  println!("[*] Installing VelocityRL Root CA to system trust store...");
  let pid = std::process::id();
  let tmp_ca = std::env::temp_dir().join(format!("velocityrl_ca_{pid}.crt"));
  let ca_bytes = include_bytes!("../resources/certs/velocityrl_ca.crt");
  let _ = std::fs::write(&tmp_ca, ca_bytes);

  if std::path::Path::new("/etc/ca-certificates/trust-source/anchors").exists() {
    let _ = std::fs::copy(&tmp_ca, "/etc/ca-certificates/trust-source/anchors/velocityrl_ca.crt");
    let _ = std::process::Command::new("update-ca-trust").status();
  } else if std::path::Path::new("/usr/local/share/ca-certificates").exists() {
    let _ = std::fs::copy(&tmp_ca, "/usr/local/share/ca-certificates/velocityrl_ca.crt");
    let _ = std::process::Command::new("update-ca-certificates").status();
  } else if std::path::Path::new("/etc/pki/ca-trust/source/anchors").exists() {
    let _ = std::fs::copy(&tmp_ca, "/etc/pki/ca-trust/source/anchors/velocityrl_ca.crt");
    let _ = std::process::Command::new("update-ca-trust").status();
  } else {
    let _ = std::process::Command::new("trust").args(["anchor", &tmp_ca.to_string_lossy()]).status();
  }
  let _ = std::fs::remove_file(&tmp_ca);

  println!("[*] Configuring /etc/hosts loopback redirect...");
  if let Ok(content) = std::fs::read_to_string("/etc/hosts") {
    let lines: Vec<&str> = content
      .lines()
      .filter(|line| {
        !line.contains("config.psynet.gg")
          && !line.contains("api.rlpp.psynet.gg")
          && !line.contains("ws.rlpp.psynet.gg")
          && !line.contains("api.epicgames.dev")
      })
      .collect();
    let mut new_text = lines.join("\n");
    if !new_text.is_empty() && !new_text.ends_with('\n') {
      new_text.push('\n');
    }
    new_text.push_str("127.0.0.1 config.psynet.gg\n");
    new_text.push_str("127.0.0.1 api.epicgames.dev\n");
    let _ = std::fs::write("/etc/hosts", new_text);
  }

  println!("==========================================");
  println!("  [SUCCESS] All Linux permissions configured!");
  println!("==========================================");
}


#[cfg(target_os = "linux")]
fn drop_privileges_to_user() {
  if unsafe { libc::geteuid() != 0 } {
    return;
  }

  let sudo_user_opt = std::env::var("SUDO_USER").ok().filter(|u| !u.trim().is_empty() && u.trim() != "root");
  let sudo_uid_opt = std::env::var("SUDO_UID").ok().and_then(|id| id.trim().parse::<libc::uid_t>().ok()).filter(|&id| id != 0);
  let sudo_gid_opt = std::env::var("SUDO_GID").ok().and_then(|id| id.trim().parse::<libc::gid_t>().ok()).filter(|&id| id != 0);

  let (u, uid, gid) = if let (Some(u), Some(uid)) = (sudo_user_opt.as_ref(), sudo_uid_opt) {
    let gid = sudo_gid_opt.unwrap_or(uid as libc::gid_t);
    (u.clone(), uid, gid)
  } else if let Some(ref u) = sudo_user_opt {
    if let Ok(c_user) = std::ffi::CString::new(u.as_str()) {
      let pwd = unsafe { libc::getpwnam(c_user.as_ptr()) };
      if !pwd.is_null() {
        let uid = unsafe { (*pwd).pw_uid };
        let gid = unsafe { (*pwd).pw_gid };
        (u.clone(), uid, gid)
      } else {
        return;
      }
    } else {
      return;
    }
  } else {
    // If running under pure root shell with no SUDO_USER, check for active desktop user in /run/user
    if let Ok(entries) = std::fs::read_dir("/run/user") {
      let mut found = None;
      for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s != "0" && s.chars().all(|c| c.is_ascii_digit()) {
          if let Ok(uid) = s.parse::<libc::uid_t>() {
            let pwd = unsafe { libc::getpwuid(uid) };
            if !pwd.is_null() {
              let user_name = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_name) }.to_string_lossy().to_string();
              let gid = unsafe { (*pwd).pw_gid };
              found = Some((user_name, uid, gid));
              break;
            }
          }
        }
      }
      if let Some(target) = found {
        target
      } else {
        return;
      }
    } else {
      return;
    }
  };

  if uid == 0 {
    return;
  }

  println!("[*] Dropping GUI privileges to user '{u}' (UID: {uid}, GID: {gid})...");
  if let Ok(c_user) = std::ffi::CString::new(u.as_str()) {
    unsafe {
      libc::initgroups(c_user.as_ptr(), gid);
    }
  }
  unsafe {
    libc::setgid(gid);
    libc::setuid(uid);
  }
}
