//! vx — gestor imperativo de cero huella sobre `nix profile`.
//! Hibrido channels/flakes: prefiere `nix profile install nixpkgs#pkg`
//! (funciona con channels y con flakes) y solo usa `nix-env` como fallback legacy.

use crate::detect;
use crate::util;
use std::fs;
use std::path::PathBuf;

fn usage() -> i32 {
    println!("vx 0.1.0 — gestor Nix imperativo para musl");
    println!("uso:");
    println!("  vx install <pkg...>   instala via nixpkgs#pkg + integra .desktop + limpia");
    println!("  vx remove <pkg...>    desinstala + limpia + borra .desktop reescritos");
    println!("  vx update             actualiza todo el perfil + limpia");
    println!("  vx search <texto>     busca en nixpkgs (channels o flakes)");
    println!("  vx list               lista el perfil actual");
    println!("  vx gc                 solo limpieza (wipe-history + gc + optimise)");
    println!("  vx doctor             diagnostico del host + nix");
    println!("notas: instala el binario como `vx` y crea symlink `vxr -> vx` para ejecutar.");
    2
}

/// Regla de oro tesis §2.2: wipe-history + gc + optimise tras cada mutacion.
pub fn aggressive_cleanup() {
    println!("[vx] purga y deduplicacion del store...");
    // Fase A: desvincular generaciones viejas (si el nix es moderno)
    let _ = util::run("nix", &["profile", "wipe-history"]);
    // Fase B: recolector sobre el DAG (nuevo CLI, fallback legacy)
    if util::run("nix", &["store", "gc"]) != 0 {
        let _ = util::run("nix-collect-garbage", &["-d"]);
    }
    // Fase C: hardlinks por contenido identico
    if util::run("nix", &["store", "optimise"]) != 0 {
        let _ = util::run("nix-store", &["--optimise"]);
    }
    if let Ok(du) = std::process::Command::new("du")
        .args(["-sh", "/nix"])
        .output()
    {
        println!("[vx] /nix: {}", String::from_utf8_lossy(&du.stdout).trim());
    }
}

fn nix_profile_args() -> Vec<String> {
    // Nada especial: `nix profile` ya respeta NIX_REMOTE=daemon y el canal nixpkgs.
    vec![]
}

/// Nombres de los elementos del perfil (via --json: el listado normal trae
/// colores ANSI). Parser minimo sin dependencias: las claves que preceden a
/// `":{"active":` son los nombres.
fn profile_element_names() -> Vec<String> {
    let out = match std::process::Command::new("nix")
        .args(["profile", "list", "--json"])
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };
    let s = String::from_utf8_lossy(&out.stdout);
    let mut names = Vec::new();
    for chunk in s.split("\":{\"active\":") {
        if let Some(pos) = chunk.rfind('"') {
            let name = &chunk[pos + 1..];
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || "._+-".contains(c))
            {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// ¿esta `pkg` ya como elemento del perfil? (via --json: el listado normal
/// trae escapes ANSI de color que rompen los grep anclados).
fn profile_has(pkg: &str) -> bool {
    // Solo tiene sentido para nombres simples; URLs/atributos con # / : se saltan.
    if pkg.contains('/') || pkg.contains(':') {
        return false;
    }
    let out = match std::process::Command::new("nix")
        .args(["profile", "list", "--json"])
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !out.status.success() {
        return false; // nix sin `profile` (legacy): se intenta instalar igual
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.contains(&format!("\"{pkg}\":"))
}

fn install_one(pkg: &str) -> i32 {
    // Acepta `steam` o `nixpkgs#steam` o `flake:nixpkgs#steam`
    let spec = if pkg.contains('#') {
        pkg.to_string()
    } else {
        format!("nixpkgs#{pkg}")
    };
    // Idempotencia: si ya esta, actualizar en vez de chocar por prioridad.
    let short = spec.rsplit('#').next().unwrap_or(&spec);
    let short = short.rsplit('.').next().unwrap_or(short);
    if profile_has(short) {
        println!("[vx] {short} ya esta en el perfil; actualizando...");
        let code = util::run("nix", &["profile", "upgrade", short]);
        rewrite_desktop_entries();
        aggressive_cleanup();
        return code;
    }
    println!("[vx] instalando {spec} ...");
    let mut args = nix_profile_args();
    args.push("profile".into());
    args.push("install".into());
    args.push(spec.clone());
    let code = util::run_dyn("nix", &args);
    if code != 0 {
        // Fallback legacy para nix viejos sin `profile` (Alpine/Chimera con nix 2.3)
        eprintln!("[vx] `nix profile` fallo; probando `nix-env -iA` legacy...");
        let attr = spec.rsplit('#').next().unwrap_or(&spec).to_string();
        let legacy = util::run("nix-env", &["-iA", &format!("nixpkgs.{attr}")]);
        if legacy != 0 {
            eprintln!("[vx] fallo instalando {pkg}");
            return legacy;
        }
    }
    rewrite_desktop_entries();
    aggressive_cleanup();
    println!("[vx] {pkg} listo. Lanza con `vxr <bin>` o desde el menu.");
    0
}

fn remove_one(pkg: &str) -> i32 {
    println!("[vx] eliminando {pkg} ...");
    // `nix profile remove` acepta el nombre del elemento del perfil, no el attr.
    // Se intenta tal cual y luego por regex `.*pkg.*`.
    let mut code = util::run("nix", &["profile", "remove", pkg]);
    if code != 0 {
        code = util::run("nix", &["profile", "remove", &format!(".*{pkg}.*")]);
    }
    if code != 0 {
        // fallback legacy
        code = util::run("nix-env", &["-e", pkg]);
    }
    remove_rewritten_desktops(pkg);
    aggressive_cleanup();
    code
}

/// Tesis §2.3: reescribe Exec=cmd -> Exec=vxr cmd en ~/.local/share/applications
/// Lee de ~/.nix-profile/share/applications (y /nix/var/nix/profiles/default/share).
fn rewrite_desktop_entries() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let nix_apps = [
        PathBuf::from(format!("{home}/.nix-profile/share/applications")),
        PathBuf::from("/nix/var/nix/profiles/default/share/applications"),
    ];
    let out_dir = PathBuf::from(format!("{home}/.local/share/applications"));
    let _ = fs::create_dir_all(&out_dir);

    // Vars que la app necesita para encontrar sus assets dentro de nix (doc gaming §10.1)
    let mut found = 0;
    for src_dir in &nix_apps {
        let entries = fs::read_dir(src_dir);
        if entries.is_err() {
            continue;
        }
        for e in entries.unwrap().flatten() {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                continue;
            }
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let fname = match path.file_name().and_then(|s| s.to_str()) {
                Some(f) => f.to_string(),
                None => continue,
            };
            let mut rewritten = String::with_capacity(content.len() + 256);
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("Exec=") {
                    let trimmed = rest.trim_start();
                    if trimmed.starts_with("vxr ") || trimmed.starts_with("nixGL ") {
                        rewritten.push_str(line);
                    } else {
                        // Prefija vxr (vxr ya encadena nixGL dentro del FHS)
                        rewritten.push_str(&format!("Exec=vxr {rest}"));
                    }
                } else {
                    rewritten.push_str(line);
                }
                rewritten.push('\n');
            }
            // Marca de que lo gestiona vx (para borrar en remove sin tocar los del usuario)
            if !rewritten.contains("# X-Mwan-Managed") {
                rewritten.push_str("# X-Mwan-Managed=true (generado por vx)\n");
            }
            let dest = out_dir.join(&fname);
            // Solo escribe si cambia (evita tocar mtime sin necesidad)
            let cur = fs::read_to_string(&dest).unwrap_or_default();
            if cur != rewritten {
                if fs::write(&dest, &rewritten).is_ok() {
                    found += 1;
                }
            }
        }
    }
    if found > 0 {
        println!(
            "[vx] integrados {found} lanzadores en ~/.local/share/applications (Exec=vxr ...)"
        );
        let _ = util::run(
            "update-desktop-database",
            &[&format!("{}/.local/share/applications", home)],
        );
    }
}

fn remove_rewritten_desktops(pkg: &str) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let out_dir = PathBuf::from(format!("{home}/.local/share/applications"));
    let entries = match fs::read_dir(&out_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
            continue;
        }
        let content = fs::read_to_string(&path).unwrap_or_default();
        if content.contains("# X-Mwan-Managed=true")
            && content.to_lowercase().contains(&pkg.to_lowercase())
        {
            let _ = fs::remove_file(&path);
            println!("[vx] lanzador huerfano eliminado: {}", path.display());
        }
    }
}

fn cmd_search(term: &str) -> i32 {
    if util::nix_supports_flakes() {
        util::run("nix", &["search", "nixpkgs", term])
    } else {
        // legacy channels
        util::run("nix-env", &["-qaP", &format!(".*{term}.*")])
    }
}

fn cmd_doctor() -> i32 {
    let h = detect::detect();
    println!("vx doctor — host: {h:?}");
    let mut fail = 0;
    for t in [
        "nix",
        "nix-store",
        "bwrap",
        "fusermount3",
        "curl",
        "xz",
        "git",
    ] {
        let ok = std::process::Command::new("which")
            .arg(t)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            || crate::util::capture("command", &["-v", t]).is_some()
            || std::path::Path::new(&format!("/usr/bin/{t}")).exists()
            || std::path::Path::new(&format!("/run/current-system/sw/bin/{t}")).exists();
        // chequeo portable: intenta ejecutar --version/which
        let present = std::process::Command::new(t)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            || ok
            || {
                let p = std::env::var("PATH").unwrap_or_default();
                p.split(':')
                    .any(|d| std::path::Path::new(&format!("{d}/{t}")).exists())
            };
        if present {
            println!("  [OK] {t}");
        } else {
            println!("  [FALTA] {t}");
            fail = 1;
        }
    }
    if std::path::Path::new("/dev/fuse").exists() {
        println!("  [OK] /dev/fuse");
    } else {
        println!("  [FALTA] /dev/fuse (AppImage usara extraccion)");
    }
    if h.gpu == "nvidia" {
        println!("  [AVISO] NVIDIA detectada: v1 usa Mesa Intel/AMD. Sigue igual pero sin aceleracion propietaria.");
    }
    if h.libc != "musl" {
        println!(
            "  [AVISO] libc={} (mwan optimizado para musl, funciona igual)",
            h.libc
        );
    }
    if fail == 0 {
        println!("doctor: TODO OK");
    } else {
        println!("doctor: faltan piezas -> ejecuta ./mwan.sh bootstrap");
    }
    fail
}

pub fn main(args: &[String]) -> i32 {
    if args.is_empty() {
        return usage();
    }
    match args[0].as_str() {
        "install" | "add" | "i" => {
            if args.len() < 2 {
                eprintln!("uso: vx install <pkg...>");
                return 2;
            }
            let _lock = util::nix_lock();
            let mut code = 0;
            for p in &args[1..] {
                let c = install_one(p);
                if c != 0 {
                    code = c;
                }
            }
            code
        }
        "remove" | "rm" | "uninstall" => {
            if args.len() < 2 {
                eprintln!("uso: vx remove <pkg...>");
                return 2;
            }
            let _lock = util::nix_lock();
            let mut code = 0;
            for p in &args[1..] {
                let c = remove_one(p);
                if c != 0 {
                    code = c;
                }
            }
            code
        }
        "update" | "upgrade" => {
            let _lock = util::nix_lock();
            println!("[vx] actualizando perfil...");
            // Por elementos y no `--all`: un solo elemento roto (ej. un flake
            // local cuyo directorio se borro) abortaria todo el upgrade.
            // Los de channels avisan "can't upgrade" y siguen (codigo 0).
            let names = profile_element_names();
            if names.is_empty() {
                eprintln!("[vx] perfil vacio o nix legacy sin `profile` (prueba `nix-channel --update`)");
            }
            let mut code = 0;
            for n in &names {
                println!("[vx] actualizando {n}...");
                if util::run("nix", &["profile", "upgrade", n]) != 0 {
                    eprintln!("[vx][AVISO] no se pudo actualizar {n} (¿flake local borrado?); se sigue con el resto.");
                    code = 1;
                }
            }
            rewrite_desktop_entries();
            aggressive_cleanup();
            code
        }
        "search" | "s" => {
            if args.len() < 2 {
                eprintln!("uso: vx search <texto>");
                return 2;
            }
            // search tambien evalua nixpkgs (GBs de RAM): candado para no
            // coincidir con install/update/gc. En N150 tarda minutos: normal.
            let _lock = util::nix_lock();
            cmd_search(&args[1..].join(" "))
        }
        "list" | "ls" => util::run("nix", &["profile", "list"]),
        "gc" | "clean" => {
            let _lock = util::nix_lock();
            aggressive_cleanup();
            0
        }
        "doctor" => cmd_doctor(),
        "help" | "--help" | "-h" => usage(),
        "version" | "--version" | "-V" => {
            println!("vx 0.1.0");
            0
        }
        other => {
            eprintln!("[vx] comando desconocido: {other}");
            usage()
        }
    }
}
