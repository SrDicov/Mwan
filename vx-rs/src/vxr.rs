//! vxr — ejecutor universal glibc dentro del FHS + nixGL (Mesa v1).
//! Cadena: [bwrap FHS vx-fhs.nix] -> [nixGLIntel] -> [binario]
//! + fallback AppImage por extraccion si FUSE esta bloqueado en el namespace.
//! + sanitizacion musl (tesis §4.1) y deteccion NVIDIA (aviso, sigue con Mesa).

use crate::detect;
use crate::util;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Localiza el .nix del perfil pedido ("vx-fhs.nix" base o
/// "vx-fhs-appimage.nix" extendido).
/// Orden: ./ -> ancestros (repo/dev) -> ~/.config/mwan/ -> /etc/mwan/ ->
/// /usr/share/mwan/ -> junto al binario. Si se pide appimage y no existe,
/// se degrada al base (mejor intentarlo que rendirse).
fn find_fhs_file(filename: &str) -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = vec![PathBuf::from(filename)];
    // Subir hasta 2 niveles por si se invoca desde vx-rs/ o tests/ del repo
    if let Ok(cwd) = std::env::current_dir() {
        let mut anc = cwd.as_path();
        for _ in 0..2 {
            if let Some(parent) = anc.parent() {
                cands.push(parent.join(filename));
                anc = parent;
            } else {
                break;
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        cands.push(PathBuf::from(format!("{home}/.config/mwan/{filename}")));
    }
    cands.push(PathBuf::from(format!("/etc/mwan/{filename}")));
    cands.push(PathBuf::from(format!("/usr/share/mwan/{filename}")));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join(filename));
            cands.push(dir.join(format!("../{filename}")));
        }
    }
    for c in cands {
        if c.is_file() {
            return Some(c);
        }
    }
    // Degradacion: perfil appimage ausente -> base (pide bootstrap si ni ese).
    if filename != "vx-fhs.nix" {
        return find_fhs_file("vx-fhs.nix");
    }
    None
}

/// "appimage" si el fichero es el perfil extendido, "base" en otro caso.
fn profile_of(fhs: &Path) -> &'static str {
    if fhs
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .contains("appimage")
    {
        "appimage"
    } else {
        "base"
    }
}

fn fuse_usable_in_namespace() -> bool {
    // /dev/fuse debe existir y ser abrible; ademas bwrap debe permitir el bind.
    // Chequeo barato: existe el nodo + fusermount3 responde.
    if !Path::new("/dev/fuse").exists() {
        return false;
    }
    Command::new("fusermount3")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_appimage(target: &str) -> bool {
    target.ends_with(".AppImage") || target.ends_with(".appimage")
}

/// Fallback tesis §3.2: extrae el AppImage a cache y ejecuta su AppRun DENTRO
/// del FHS, barriendo al salir.
/// La extraccion corre DENTRO del FHS a proposito: el runtime AppImage es un
/// binario glibc con su propio unsquashfs embebido. Fuera del FHS solo
/// funcionaria en hosts con una compat-glibc accidental (este Void tiene un
/// ld-linux huesped en /usr/lib, pero un Alpine/Chimera puro no), asi que la
/// via portable es extraer dentro del namespace glibc. El destino esta bajo
/// $HOME/.cache (bindeado dentro) para que ambas fases lo vean.
fn extract_and_run_fhs(fhs: &Path, target: &str, rest: &[String], force_rebuild: bool) -> i32 {
    let entry = match build_fhs_entry(fhs, force_rebuild) {
        Some(e) => e,
        None => {
            eprintln!("[vxr] fallo construyendo el FHS. Prueba `./mwan.sh bootstrap`.");
            return 1;
        }
    };
    let cache_base = std::env::var("XDG_CACHE_HOME")
        .map(|h| format!("{h}/vx-cache"))
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| format!("{h}/.cache/vx-cache"))
                .unwrap_or_else(|_| "/tmp/vx-cache".into())
        });
    let stem = Path::new(target)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("app");
    let dest = PathBuf::from(format!("{cache_base}/{stem}.extracted"));
    let _ = fs::create_dir_all(&cache_base);
    let _ = fs::remove_dir_all(&dest);
    if fs::create_dir_all(&dest).is_err() {
        eprintln!("[vxr] no se pudo crear cache {dest:?}");
        return 1;
    }
    println!("[vxr] FUSE bloqueado: extrayendo {stem} a {dest:?} ...");
    // Ruta absoluta: dentro del namespace el CWD de extraccion sera `dest`.
    let target_abs = if Path::new(target).is_absolute() {
        target.to_string()
    } else {
        std::env::current_dir()
            .map(|c| c.join(target).to_string_lossy().into_owned())
            .unwrap_or_else(|_| target.to_string())
    };
    let st = Command::new(&entry)
        .arg(&target_abs)
        .arg("--appimage-extract")
        .current_dir(&dest)
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .status();
    match st {
        Ok(s) if s.success() => {}
        _ => {
            eprintln!("[vxr] fallo --appimage-extract. ¿binario corrupto o sin +x?");
            return 1;
        }
    }
    // Localiza AppRun (squashfs-root/AppRun)
    let apprun = dest.join("squashfs-root/AppRun");
    if !apprun.exists() {
        eprintln!("[vxr] extraccion sin squashfs-root/AppRun");
        return 1;
    }
    let _ = fs::set_permissions(&apprun, fs::Permissions::from_mode(0o755));
    println!("[vxr] lanzando AppRun extraido dentro del FHS...");
    // APPDIR obligatorio: el AppRun deriva rutas de $APPDIR, y su autodeteccion
    // busca "$path/$1" hacia arriba (falla con flags como --help porque ningun
    // dir contiene un fichero llamado asi). El runtime FUSE lo pondria solo;
    // aqui lo fijamos nosotros al squashfs-root extraido.
    // (Proceso de un solo uso: set_var no contamina nada mas.)
    if let Some(root) = dest.join("squashfs-root").to_str() {
        std::env::set_var("APPDIR", root);
    }
    let apprun_s = apprun.to_str().unwrap_or("").to_string();
    let code = run_with_entry(&entry, &build_inner(&apprun_s, rest, true));
    let _ = fs::remove_dir_all(&dest);
    code
}

/// Resuelve `nixGLIntel` (fallback `nixGL`) a ruta absoluta de /nix/store (canonica).
/// Dentro del FHS solo es visible /nix/store (+ binds), NO ~/.nix-profile,
/// asi que hay que pasar la ruta canonica, no el nombre del wrapper.
fn resolve_nixgl() -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    for name in ["nixGLIntel", "nixGL"] {
        for dir in path.split(':') {
            if dir.is_empty() {
                continue;
            }
            let cand = PathBuf::from(format!("{dir}/{name}"));
            if cand.exists() {
                // canonicalize resuelve ~/.nix-profile/bin/nixGLIntel -> /nix/store/.../bin/nixGLIntel
                if let Ok(abs) = fs::canonicalize(&cand) {
                    // Solo vale si cae dentro de /nix/store (visible en el FHS).
                    // Un nixGL de /usr/bin del host NO serviria dentro.
                    if abs.starts_with("/nix/store") {
                        return abs.to_str().map(|s| s.to_string());
                    }
                } else if cand.starts_with("/nix/store") {
                    return cand.to_str().map(|s| s.to_string());
                }
            }
        }
    }
    None
}

/// Resuelve el binario objetivo a ruta absoluta canonica cuando es un nombre
/// simple (ej. `steam` -> `/nix/store/.../bin/steam`).
/// Motivo: dentro del FHS el PATH NO incluye ~/.nix-profile/bin, asi que los
/// nombres del perfil deben resolverse en el HOST antes de entrar.
/// - Con `/` (ruta): se pasa tal cual (AppImages, $HOME y /tmp estan bindeados).
/// - Nombre simple: se busca en el PATH del host y se canonicaliza
///   (los symlinks de nix-profile apuntan a /nix/store, visible dentro).
///   Si no aparece en el host, se deja tal cual para que lo resuelva el FHS
///   (coreutils: echo, bash, python3...).
fn resolve_target(target: &str) -> String {
    if target.contains('/') {
        return target.to_string();
    }
    // Solo se resuelven binarios de Nix: un /usr/bin/echo del host (musl) NO
    // funcionaria dentro del FHS (su loader musl no es visible ahi), mientras
    // que el `echo` del propio FHS si. En cambio `steam` del perfil DEBE
    // resolverse aqui porque el FHS no incluye ~/.nix-profile/bin en su PATH.
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let cand = PathBuf::from(format!("{dir}/{target}"));
        if !cand.exists() {
            continue;
        }
        let abs = fs::canonicalize(&cand)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| cand.to_string_lossy().into_owned());
        if abs.starts_with("/nix/") {
            return abs;
        }
        // Binario del host (ej. /usr/bin/echo): se deja el nombre simple para
        // que lo resuelva el FHS con sus propias herramientas glibc.
    }
    target.to_string()
}

/// Cache del FHS construido: evita re-evaluar nixpkgs (~8s en N150) en cada
/// invocacion. Clave = (tamano, mtime) de vx-fhs.nix; el path del store se
/// valida con `nix-store --check-validity`. Se invalida al cambiar el .nix,
/// con `vxr --rebuild` o con `mwan.sh update` (borra ~/.cache/vx/fhs-path).
fn cache_path(fhs: &Path) -> Option<PathBuf> {
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(format!("{h}/.cache")))
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
        });
    // Una cache por perfil: base y appimage son closures distintos.
    Some(
        base.join("vx")
            .join(format!("fhs-path-{}", profile_of(fhs))),
    )
}

fn cache_lookup(fhs: &Path, force_rebuild: bool) -> Option<String> {
    if force_rebuild {
        return None;
    }
    let meta = fs::metadata(fhs).ok()?;
    let len = meta.len();
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let cache = cache_path(fhs)?;
    let content = fs::read_to_string(&cache).ok()?;
    let mut parts = content.split_whitespace();
    if parts.next()? != len.to_string() || parts.next()? != mtime.to_string() {
        return None; // el .nix cambio
    }
    let store = parts.next()?.to_string();
    let entry = format!("{store}/bin/vxr-fhs");
    if !Path::new(&entry).exists() {
        return None;
    }
    // ¿sigue valido en el store? (por si hubo `nix-collect-garbage`)
    let valid = Command::new("nix-store")
        .args(["--check-validity", "--print-invalid", &store])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);
    if valid {
        Some(entry)
    } else {
        None
    }
}

fn cache_store(fhs: &Path, store: &str) {
    let meta = match fs::metadata(fhs) {
        Ok(m) => m,
        Err(_) => return,
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Some(cache) = cache_path(fhs) {
        let _ = fs::create_dir_all(cache.parent().unwrap());
        let _ = fs::write(&cache, format!("{} {} {store}\n", meta.len(), mtime));
    }
}

/// Construye el FHS (sin link GC-root) y devuelve `<store>/bin/vxr-fhs`.
/// Usa `nix build -f` (moderno, channels+flakes) con fallback a `nix-build` legacy
/// (nix 2.3 de Alpine/Chimera sin `nix-command`).
fn build_fhs_entry(fhs: &Path, force_rebuild: bool) -> Option<String> {
    if let Some(entry) = cache_lookup(fhs, force_rebuild) {
        return Some(entry);
    }
    // El build evalua nixpkgs (pico serio de RAM): serializar con el resto
    // de operaciones Nix. El candado se suelta al salir (antes de ejecutar
    // la app, para no bloquear `vx install` durante sesiones largas).
    let _lock = util::nix_lock();
    // Re-chequear tras esperar: otro proceso puede haberlo construido.
    if let Some(entry) = cache_lookup(fhs, force_rebuild) {
        return Some(entry);
    }
    let fhs_str = fhs.to_str().unwrap_or("vx-fhs.nix");
    println!("[vxr] construyendo FHS (solo la primera vez o si cambio vx-fhs.nix)...");
    // Via moderna: mismo comando en channels y flakes
    if let Some(out) = util::capture(
        "nix",
        &["build", "-f", fhs_str, "--no-link", "--print-out-paths"],
    ) {
        let store = out.lines().last().unwrap_or("").trim().to_string();
        if !store.is_empty() {
            let entry = format!("{store}/bin/vxr-fhs");
            if Path::new(&entry).exists() {
                cache_store(fhs, &store);
                return Some(entry);
            }
        }
    }
    // Fallback legacy
    if let Some(out) = util::capture("nix-build", &[fhs_str, "--no-out-link"]) {
        let store = out.lines().last().unwrap_or("").trim().to_string();
        if !store.is_empty() {
            let entry = format!("{store}/bin/vxr-fhs");
            if Path::new(&entry).exists() {
                cache_store(fhs, &store);
                return Some(entry);
            }
        }
    }
    None
}

/// Ejecuta <target> <rest> dentro del FHS. Si with_nixgl, envuelve con nixGLIntel
/// (ruta canonica de /nix/store para que sea visible dentro del namespace).
/// `is_appimage`: la salida se propaga en vivo pero tambien se inspecciona;
/// si el runtime AppImage muere por FUSE (falta lib, montaje denegado en el
/// namespace, kernel endurecido...), se reintenta automaticamente por
/// extraccion (tesis §3.2), en vez de dejar al usuario con el error crudo.
fn run_inside_fhs(
    fhs: &Path,
    target: &str,
    rest: &[String],
    with_nixgl: bool,
    force_rebuild: bool,
) -> i32 {
    let resolved = resolve_target(target);
    let inner = build_inner_resolved(&resolved, rest, with_nixgl);

    // Scripts con bwrap propio (steam de nixpkgs...): en host + nixGL,
    // sin FHS anidado y sin pagar la construccion del FHS.
    if is_script_file(&resolved) {
        println!("[vxr] wrapper con sandbox propio -> host + nixGL (sin FHS anidado)");
        println!("[vxr] HOST -> {}", inner.join(" "));
        return util::run_dyn(&inner[0], &inner[1..].to_vec());
    }

    let entry = match build_fhs_entry(fhs, force_rebuild) {
        Some(e) => e,
        None => {
            eprintln!("[vxr] fallo construyendo el FHS (nix build -f). ¿canales rotos? Prueba `./mwan.sh bootstrap`.");
            return 1;
        }
    };

    println!("[vxr] FHS -> {}", inner.join(" "));
    if is_appimage(target) {
        return run_appimage_piped(&entry, &inner, fhs, target, rest, force_rebuild);
    }
    run_with_entry(&entry, &inner)
}

/// ¿es un script de texto (shebang) en vez de un ELF?
/// Los wrappers como el `steam` de nixpkgs son scripts bash que levantan SU
/// PROPIO bwrap/pressure-vessel: meterlos dentro de nuestro FHS anida
/// namespaces (lento/fragil). Se ejecutan en el host (+ nixGL), como hace
/// `nixGL steam` en el flujo clasico. Los binarios del FHS (echo, bash...)
/// y los AppRun extraidos siguen yendo dentro.
fn is_script_file(path: &str) -> bool {
    if !path.contains('/') {
        return false; // nombre simple: lo resolvera el FHS
    }
    let bytes = fs::read(path).unwrap_or_default();
    bytes.len() >= 2 && bytes[0] == b'#' && bytes[1] == b'!'
}

/// ICD de Vulkan derivado del script nixGLIntel (lee su MESA_64) + GPU host.
/// nixGL (nixpkgs) no exporta VK_ICD_FILENAMES; sin ICD, Vulkan y ANGLE
/// (Chromium/Electron, muchos juegos) fallan en silencio dentro del FHS.
/// Devuelve None si no se puede determinar con seguridad (se sigue sin ICD,
/// que es el comportamiento anterior).
fn vulkan_icd() -> Option<String> {
    let nixgl = resolve_nixgl()?;
    let content = fs::read_to_string(&nixgl).ok()?;
    let mesa = content.lines().find_map(|l| {
        let t = l.trim().strip_prefix("export ").unwrap_or(l.trim());
        t.strip_prefix("MESA_64=")
            .map(|v| v.trim().trim_matches('"').to_string())
    })?;
    if !mesa.starts_with("/nix/store/") {
        return None;
    }
    let file = match detect::detect().gpu.as_str() {
        "intel" => "intel_icd.x86_64.json",
        "amd" => "radeon_icd.x86_64.json",
        _ => return None, // nvidia/unknown: no adivinar (v1 solo Mesa)
    };
    let icd = format!("{mesa}/share/vulkan/icd.d/{file}");
    if Path::new(&icd).is_file() {
        Some(icd)
    } else {
        None
    }
}

/// Aplica variables que solo tienen sentido con aceleración (nixGL delante).
fn apply_gl_env(cmd: &mut Command, inner: &[String]) {
    let uses_nixgl = inner.first().map(|s| s.contains("nixGL")).unwrap_or(false);
    if uses_nixgl {
        if let Some(icd) = vulkan_icd() {
            cmd.env("VK_ICD_FILENAMES", &icd);
            cmd.env("VK_DRIVER_FILES", &icd);
        }
    }
}

/// Arma el argv interno del contenedor.
fn build_inner(target: &str, rest: &[String], with_nixgl: bool) -> Vec<String> {
    build_inner_resolved(&resolve_target(target), rest, with_nixgl)
}

/// Variante con el objetivo ya resuelto (evita doble busqueda en PATH).
fn build_inner_resolved(resolved: &str, rest: &[String], with_nixgl: bool) -> Vec<String> {
    let mut inner: Vec<String> = vec![];
    if with_nixgl {
        match resolve_nixgl() {
            Some(nixgl) => inner.push(nixgl),
            None => eprintln!(
                "[vxr][AVISO] nixGLIntel no encontrado en PATH; se ejecuta sin aceleracion GPU."
            ),
        }
    }
    inner.push(resolved.to_string());
    inner.extend_from_slice(rest);
    inner
}

/// Ejecuta un argv ya resuelto dentro del FHS con stdio heredado.
fn run_with_entry(entry: &str, inner: &[String]) -> i32 {
    let mut cmd = Command::new(entry);
    cmd.args(inner);
    cmd.env_remove("LD_LIBRARY_PATH");
    cmd.env_remove("LD_PRELOAD");
    apply_gl_env(&mut cmd, inner);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    match cmd.status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("[vxr] fallo lanzando FHS: {e}");
            127
        }
    }
}

/// Ejecuta un AppImage con stdout/stderr en vivo (tee) pero inspeccionando la
/// salida: si el runtime muere por FUSE, se reintenta por extraccion.
fn run_appimage_piped(
    entry: &str,
    inner: &[String],
    fhs: &Path,
    target: &str,
    rest: &[String],
    force_rebuild: bool,
) -> i32 {
    use std::io::{Read, Write};
    let mut cmd = Command::new(entry);
    cmd.args(inner);
    cmd.env_remove("LD_LIBRARY_PATH");
    cmd.env_remove("LD_PRELOAD");
    apply_gl_env(&mut cmd, inner);
    let mut child = match cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[vxr] fallo lanzando FHS: {e}");
            return 127;
        }
    };
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    // Hilos que reenvian en vivo y guardan los primeros 64KB para el diagnostico.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let tx1 = tx.clone();
    let h1 = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut saved = Vec::new();
        let mut lock = std::io::stdout().lock();
        if let Some(ref mut s) = out {
            loop {
                match s.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = lock.write_all(&buf[..n]);
                        if saved.len() < 65536 {
                            saved.extend_from_slice(&buf[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = lock.flush();
        }
        let _ = tx1.send(saved);
    });
    let h2 = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut saved = Vec::new();
        let mut lock = std::io::stderr().lock();
        if let Some(ref mut s) = err {
            loop {
                match s.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = lock.write_all(&buf[..n]);
                        if saved.len() < 65536 {
                            saved.extend_from_slice(&buf[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = lock.flush();
        }
        let _ = tx.send(saved);
    });
    let status = child.wait();
    let _ = h1.join();
    let _ = h2.join();
    // Ambos hilos terminaron y soltaron sus emisores: el for termina solo.
    let mut combined = String::new();
    for chunk in rx {
        combined.push_str(&String::from_utf8_lossy(&chunk));
    }
    let code = status.map(|s| s.code().unwrap_or(1)).unwrap_or(127);
    if code == 0 {
        return 0;
    }
    let low = combined.to_lowercase();
    let fuse_fail = [
        "fuse",
        "cannot mount",
        "mount failed",
        "appimage-extract",
        "no such file or directory",
    ]
    .iter()
    .any(|k| low.contains(k));
    if fuse_fail {
        eprintln!("[vxr] el runtime AppImage fallo por FUSE/montaje (codigo {code}); reintentando por extraccion...");
        return extract_and_run_fhs(fhs, target, rest, force_rebuild);
    }
    code
}

fn usage() -> i32 {
    println!("vxr — ejecutor glibc/FHS + nixGL (Mesa Intel/AMD v1)");
    println!("uso: vxr [--rebuild] <binario|/ruta/app.AppImage> [args...]");
    println!("ej:  vxr steam | vxr ./Joplin.AppImage | vxr gh --version");
    println!("     vxr --rebuild echo hola   (reconstruye el FHS tras cambiar canales)");
    println!("     vxr echo --no-gl hola     (sin nixGL, para binarios CLI puros)");
    2
}

pub fn main(args: &[String]) -> i32 {
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        return usage();
    }
    let host = detect::detect();
    if host.gpu == "nvidia" {
        eprintln!("[vxr][AVISO] GPU NVIDIA detectada: v1 solo da soporte Mesa Intel/AMD.");
        eprintln!("[vxr][AVISO] Se continua con nixGLIntel; el driver propietario no se inyecta.");
    }

    // Flag propia de vxr: solo `--rebuild' como PRIMER argumento (para no robar
    // ese literal si la app objetivo lo usa como suyo, ej. `vxr echo --rebuild`).
    let mut args: Vec<String> = args.to_vec();
    let mut force_rebuild = false;
    if args.first().map(|s| s.as_str()) == Some("--rebuild") {
        force_rebuild = true;
        args.remove(0);
    }
    if args.is_empty() {
        return usage();
    }

    let target = args[0].clone();
    let rest = args[1..].to_vec();

    // Heuristica de perfil (tesis §1.1): los AppImage usan SIEMPRE el perfil
    // extendido (necesitan las system-libs aunque el montaje FUSE funcione);
    // el resto usa el base minimo.
    let want_appimage_profile = is_appimage(&target);
    let fhs_filename = if want_appimage_profile {
        "vx-fhs-appimage.nix"
    } else {
        "vx-fhs.nix"
    };

    let target_path_exists = target.contains('/') && Path::new(&target).exists();

    // Caso AppImage con FUSE roto -> extraccion (tesis §3.2)
    if want_appimage_profile && !fuse_usable_in_namespace() {
        if let Some(fhs) = find_fhs_file(fhs_filename) {
            return extract_and_run_fhs(&fhs, &target, &rest, force_rebuild);
        }
        eprintln!("[vxr] sin FUSE y sin perfil FHS localizable. Ejecuta `./mwan.sh bootstrap`.");
        return 1;
    }

    // Si el target es una ruta local sin +x, avisa (error tipico musl: ENOENT por loader)
    if target_path_exists {
        if let Ok(meta) = fs::metadata(&target) {
            if meta.permissions().mode() & 0o111 == 0 {
                eprintln!("[vxr] aviso: {target} no tiene permiso +x. Haz chmod +x primero.");
            }
        }
    }

    match find_fhs_file(fhs_filename) {
        Some(fhs) => {
            // Heuristica GUI vs CLI: por defecto SIEMPRE nixGL (inofensivo en CLI,
            // necesario en GUI). `--no-gl` lo desactiva para binarios puros.
            let (no_gl, rest2) = if rest.first().map(|s| s.as_str()) == Some("--no-gl") {
                (true, rest[1..].to_vec())
            } else {
                (false, rest)
            };
            run_inside_fhs(&fhs, &target, &rest2, !no_gl, force_rebuild)
        }
        None => {
            eprintln!(
                "[vxr] no se encontro {fhs_filename} (buscado en ./, ~/.config/mwan/, /etc/mwan/)."
            );
            eprintln!("[vxr] fallback: ejecucion directa sanitizada (puede fallar en musl si el binario es glibc)...");
            // Sin FHS no hay nixGL que desactivar: se retira --no-gl si venia primero.
            let rest: Vec<String> = if rest.first().map(|s| s.as_str()) == Some("--no-gl") {
                rest[1..].to_vec()
            } else {
                rest
            };
            let mut cmd = Command::new(&target);
            cmd.args(&rest);
            cmd.env_remove("LD_LIBRARY_PATH");
            cmd.env_remove("LD_PRELOAD");
            cmd.stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            match cmd.status() {
                Ok(s) => s.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("[vxr] fallo ejecucion directa: {e}");
                    127
                }
            }
        }
    }
}
