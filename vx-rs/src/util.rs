//! Utilidades de ejecucion de subprocesos.

use std::process::{Command, Stdio};

/// Ejecuta y hereda stdio. Devuelve codigo de salida (0 = ok).
pub fn run(prog: &str, args: &[&str]) -> i32 {
    match Command::new(prog)
        .args(args)
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!("[vx] no se pudo ejecutar {prog}: {e}");
            127
        }
    }
}

/// Ejecuta con args dinamicos (Vec<String>).
pub fn run_dyn(prog: &str, args: &[String]) -> i32 {
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(prog, &refs)
}

/// Captura stdout (para detecciones). Devuelve None si falla.
pub fn capture(prog: &str, args: &[&str]) -> Option<String> {
    Command::new(prog)
        .args(args)
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
}

/// ¿nix soporta `nix profile` + flakes? (nix >= 2.4 con experimental-features)
pub fn nix_supports_flakes() -> bool {
    let conf = capture("nix", &["show-config", "experimental-features"]).unwrap_or_default();
    conf.contains("flakes")
        || std::fs::read_to_string("/etc/nix/nix.conf")
            .unwrap_or_default()
            .contains("flakes")
}

// ---------------------------------------------------------------------------
// Bloqueo mutuo entre operaciones pesadas de Nix.
// Causa: en maquinas justas (N150 3.5GB) dos evaluaciones gordas de nixpkgs
// en paralelo (ej. `vx search` + `mwan bootstrap`) agotan la RAM y congelan
// el sistema al punto de exigir apagado forzoso. Este candado serializa:
//   vx install/remove/update/gc/search, vxr (fase de build del FHS),
//   mwan bootstrap/update/clean.
// Mecanismo: `mkdir` atomico (portable, sin dependencias). El dir vive en
// /tmp (tmpfs: se limpia solo al arrancar, asi que un apagon nunca deja
// candados podridos). Si el dueño murio (kill -9), se detecta por /proc y
// se reclama. Reentrante para el mismo pid (sin autobloqueo).
// ---------------------------------------------------------------------------

const NIX_LOCK_DIR: &str = "/tmp/mwan-nix.lock";

/// Guardia RAII: al salir de ambito libera el candado (si es nuestro).
pub struct NixGuard {
    owned: bool,
}

impl Drop for NixGuard {
    fn drop(&mut self) {
        if !self.owned {
            return;
        }
        // Solo borra si el pid sigue siendo el nuestro (no robar candados ajenos).
        let pid_file = std::path::Path::new(NIX_LOCK_DIR).join("pid");
        let mine = std::fs::read_to_string(&pid_file)
            .unwrap_or_default()
            .trim()
            .to_string();
        if mine == std::process::id().to_string() {
            let _ = std::fs::remove_file(&pid_file);
            let _ = std::fs::remove_dir(NIX_LOCK_DIR);
        }
    }
}

fn lock_owner_alive() -> Option<u32> {
    let pid_str = std::fs::read_to_string(std::path::Path::new(NIX_LOCK_DIR).join("pid"))
        .ok()?
        .trim()
        .to_string();
    let pid: u32 = pid_str.parse().ok()?;
    if pid == std::process::id() {
        return Some(pid); // reentrante: somos nosotros
    }
    // ¿sigue vivo? (Linux: /proc/<pid> existe)
    if std::path::Path::new(&format!("/proc/{pid}")).exists() {
        Some(pid)
    } else {
        // Dueño muerto (kill -9): reclamar.
        let _ = std::fs::remove_dir_all(NIX_LOCK_DIR);
        None
    }
}

/// Adquiere el candado global de Nix (bloquea con avisos hasta lograrlo).
pub fn nix_lock() -> NixGuard {
    let me = std::process::id();
    let mut waited = false;
    loop {
        match std::fs::create_dir(NIX_LOCK_DIR) {
            Ok(()) => {
                let _ = std::fs::write(
                    std::path::Path::new(NIX_LOCK_DIR).join("pid"),
                    me.to_string(),
                );
                if waited {
                    eprintln!("[vx] candado nix adquirido, continuando...");
                }
                return NixGuard { owned: true };
            }
            Err(_) => {
                match lock_owner_alive() {
                    // Reentrante o candado reclamado: reintentar de inmediato.
                    None => continue,
                    Some(pid) if pid == me => {
                        return NixGuard { owned: false };
                    }
                    Some(pid) => {
                        if !waited {
                            eprintln!("[vx] otra operacion Nix pesada en curso (pid {pid}); esperando para no saturar la RAM...");
                            waited = true;
                        }
                        std::thread::sleep(std::time::Duration::from_secs(5));
                    }
                }
            }
        }
    }
}
