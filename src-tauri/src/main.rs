#![windows_subsystem = "windows"]

fn main() {
  #[cfg(target_os = "linux")]
  {
    let is_root = unsafe { libc::geteuid() == 0 };
    if is_root || std::env::args().any(|a| a == "--setup" || a == "setup") {
      run_cli_setup();
      return;
    }

    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
      std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
  }

  app_lib::run();
}

#[cfg(target_os = "linux")]
fn run_cli_setup() {
  println!("==========================================");
  println!("  VelocityRL Linux System Setup");
  println!("==========================================");

  unsafe {
    if libc::geteuid() != 0 {
      eprintln!("[-] Error: Permission setup requires root privileges.");
      eprintln!(
        "[-] Please run with sudo: sudo {} --setup",
        std::env::args().next().unwrap_or_else(|| "./velocity-rl".into())
      );
      std::process::exit(1);
    }
  }

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
      .filter(|line| !line.contains("config.psynet.gg") && !line.contains("api.rlpp.psynet.gg") && !line.contains("ws.rlpp.psynet.gg"))
      .collect();
    let mut new_text = lines.join("\n");
    if !new_text.is_empty() && !new_text.ends_with('\n') {
      new_text.push('\n');
    }
    new_text.push_str("127.0.0.1 config.psynet.gg\n");
    let _ = std::fs::write("/etc/hosts", new_text);
  }

  println!("==========================================");
  println!("  [SUCCESS] All Linux permissions configured!");
  println!("  You can now launch VelocityRL as your normal user.");
  println!("==========================================");
}
