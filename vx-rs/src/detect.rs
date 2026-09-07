//! Deteccion de distro/init/libc/gpu sin dependencias.
//! Replica la logica de mwan.sh para que vx/vxr decidan solos.

use std::fs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub distro: String,
    pub pkgmgr: String,
    pub init: String,
    pub libc: String,
    pub gpu: String,
}

fn read_first(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn have_bin(name: &str) -> bool {
    if name.contains('/') {
        return std::path::Path::new(name).exists();
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            if std::path::Path::new(&format!("{dir}/{name}")).exists() {
                return true;
            }
        }
    }
    false
}

pub fn detect() -> Host {
    // --- distro ---
    let mut distro = String::from("unknown");
    let os = read_first("/etc/os-release").to_lowercase();
    for line in os.lines() {
        if let Some(v) = line.strip_prefix("id=") {
            distro = v.trim_matches('"').trim().to_string();
            break;
        }
    }
    if distro == "unknown" || distro.is_empty() {
        if std::path::Path::new("/etc/void-release").exists() {
            distro = "void".into();
        } else if std::path::Path::new("/etc/alpine-release").exists() {
            distro = "alpine".into();
        } else if have_bin("emerge") {
            distro = "gentoo".into();
        } else if have_bin("apk") {
            distro = "chimera".into();
        }
    }

    // --- pkgmgr ---
    let pkgmgr = if have_bin("xbps-install") {
        "xbps"
    } else if have_bin("apk") {
        "apk"
    } else if have_bin("emerge") {
        "portage"
    } else {
        "unknown"
    }
    .to_string();

    // --- init ---
    let init = if std::path::Path::new("/run/runit").exists()
        || std::path::Path::new("/var/service").exists()
    {
        "runit"
    } else if have_bin("dinitctl") || std::path::Path::new("/etc/dinit.d").exists() {
        "dinit"
    } else if have_bin("openrc-run") || std::path::Path::new("/etc/runlevels").exists() {
        "openrc"
    } else {
        "unknown"
    }
    .to_string();

    // --- libc ---
    let libc = if std::path::Path::new("/lib/ld-musl-x86_64.so.1").exists()
        || std::path::Path::new("/lib/ld-musl-aarch64.so.1").exists()
    {
        "musl"
    } else {
        // fallback: ldd
        let out = std::process::Command::new("ldd")
            .arg("--version")
            .output()
            .map(|o| {
                let mut s = String::from_utf8_lossy(&o.stdout).to_string();
                s.push_str(&String::from_utf8_lossy(&o.stderr));
                s.to_lowercase()
            })
            .unwrap_or_default();
        if out.contains("musl") {
            "musl"
        } else if out.contains("glibc") || out.contains("gnu") {
            "glibc"
        } else {
            "unknown"
        }
    }
    .to_string();

    // --- gpu via sysfs (sin lspci; funciona en Chimera minima) ---
    let mut gpu = String::from("unknown");
    if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
        for e in entries.flatten() {
            let base = e.path();
            let vendor =
                fs::read_to_string(base.join("vendor")).unwrap_or_default();
            let class = fs::read_to_string(base.join("class")).unwrap_or_default();
            if class.trim_start().starts_with("0x03") {
                match vendor.trim() {
                    "0x10de" => {
                        gpu = "nvidia".into();
                        break;
                    }
                    "0x8086" if gpu == "unknown" => gpu = "intel".into(),
                    "0x1002" if gpu == "unknown" => gpu = "amd".into(),
                    _ => {}
                }
            }
        }
    }

    Host {
        distro,
        pkgmgr,
        init,
        libc,
        gpu,
    }
}
