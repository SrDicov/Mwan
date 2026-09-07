#!/bin/sh
# mwan.sh — Asistente universal Nix para distros musl (bootstrap F0-F3)
# POSIX sh puro: corre en Void / Alpine / Chimera / Adelie / Gentoo-musl / postmarketOS
# sin dependencias salvo sh + coreutils basicas. No usa flags GNU-only (cp -a, no --preserve).
# Uso:
#   ./mwan.sh                -> flujo completo: detectar + asegurar + limpiar + dejar listo
#   ./mwan.sh doctor         -> solo diagnostico (exit 0 ok, 1 problemas)
#   ./mwan.sh bootstrap      -> detectar + instalar/actualizar nix y deps
#   ./mwan.sh update         -> actualizar canales + deps del sistema
#   ./mwan.sh clean          -> regla de oro: wipe-history + gc + optimise + cache pkgmgr
#   ./mwan.sh install <pkg>  -> bootstrap si falta + nix profile install (delega a vx si existe)
#   ./mwan.sh run <bin> ...  -> ejecuta via vxr si existe, si no via nixGL directo
set -eu

MWAN_VERSION="0.1.0"
NIXPKGS_CHANNEL_URL="https://nixos.org/channels/nixpkgs-unstable"
NIXGL_CHANNEL_URL="https://github.com/nix-community/nixGL/archive/main.tar.gz"
# Chimera no tiene GNU coreutils: el instalador oficial falla por `cp --preserve=...`
CHIMERA_NIX_INSTALLER_URL="https://raw.githubusercontent.com/elgreams/nix-installer-chimera/main/chinera-nix-install.sh"

log()  { printf '[mwan] %s\n' "$*"; }
warn() { printf '[mwan][WARN] %s\n' "$*" >&2; }
err()  { printf '[mwan][ERROR] %s\n' "$*" >&2; }
die()  { err "$*"; exit 1; }

have() { command -v "$1" >/dev/null 2>&1; }

# --- candado mutuo entre operaciones pesadas de Nix (ver util.rs en vx-rs) ---
# En maquinas justas (3-4GB RAM) dos evaluaciones gordas de nixpkgs en paralelo
# congelan el sistema. mkdir es atomico y portable (Void/Alpine/Chimera/...).
# /tmp es tmpfs: un apagon nunca deja candados podridos. Reentrante por pid.
MWAN_LOCKDIR="/tmp/mwan-nix.lock"
mwan_lock() {
  while ! mkdir "$MWAN_LOCKDIR" 2>/dev/null; do
    _lp=$(cat "$MWAN_LOCKDIR/pid" 2>/dev/null || echo "?")
    if [ "$_lp" = "$$" ]; then return 0; fi # reentrante: ya es nuestro
    if [ "$_lp" != "?" ] && [ -d "/proc/$_lp" ]; then
      log "otra operacion Nix pesada en curso (pid $_lp); esperando para no saturar la RAM..."
      sleep 5
    else
      rm -rf "$MWAN_LOCKDIR" 2>/dev/null || true # dueño muerto: reclamar
    fi
  done
  printf '%s' "$$" > "$MWAN_LOCKDIR/pid" 2>/dev/null || true
}
mwan_unlock() {
  if [ "$(cat "$MWAN_LOCKDIR/pid" 2>/dev/null || echo '?')" = "$$" ]; then
    rm -rf "$MWAN_LOCKDIR" 2>/dev/null || true
  fi
}

# --- privilegios: doas (Chimera) > sudo > su > nada (ya root) ---
PRIV=""
detect_priv() {
  if [ "$(id -u)" -eq 0 ]; then PRIV=""; return 0; fi
  if have doas; then PRIV="doas"
  elif have sudo; then PRIV="sudo"
  elif have su; then PRIV="su -c"
  else PRIV=""
  fi
}
run_priv() {
  # run_priv <cmd...>: ejecuta con privilegios si no somos root
  if [ -z "$PRIV" ]; then "$@"
  elif [ "$PRIV" = "su -c" ]; then su -c "$*"
  else $PRIV "$@"
  fi
}

# --- deteccion ---
DISTRO="unknown"
DISTRO_LIKE=""
PKGMGR="unknown"
INIT="unknown"
LIBC="unknown"
GPU="unknown"

detect_distro() {
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    # Solo se quitan comillas; los ID ya vienen en minusculas (void/alpine/chimera).
    # (Sin `tr [:upper:] [:lower:]`: da error de collation en algunos locales.)
    DISTRO=$(grep -E '^ID=' /etc/os-release | head -n1 | cut -d= -f2 | tr -d '"')
    DISTRO_LIKE=$(grep -E '^ID_LIKE=' /etc/os-release | head -n1 | cut -d= -f2 | tr -d '"' || true)
  fi
  # Fallbacks para minimas sin os-release completo
  if [ -z "$DISTRO" ] || [ "$DISTRO" = "unknown" ]; then
    if [ -f /etc/alpine-release ]; then DISTRO="alpine"
    elif [ -f /etc/void-release ]; then DISTRO="void"
    elif have apk && [ -f /etc/chimera-release ]; then DISTRO="chimera"
    elif have emerge; then DISTRO="gentoo"
    fi
  fi
  # Chimera reporta ID=chimera en os-release moderno; Alpine postmarketOS ID=postmarketos
  case "$DISTRO" in
    void|"") [ -f /etc/void-release ] && DISTRO="void" ;;
  esac
  [ -z "$DISTRO" ] && DISTRO="unknown"
  log "distro detectada: $DISTRO (like: ${DISTRO_LIKE:-none})"
}

detect_pkgmgr() {
  if have xbps-install; then PKGMGR="xbps"
  elif have apk; then PKGMGR="apk"
  elif have emerge; then PKGMGR="portage"
  else PKGMGR="unknown"
  fi
  log "gestor de paquetes: $PKGMGR"
}

detect_init() {
  _p1=""
  if have ps; then _p1=$(ps -p 1 -o comm= 2>/dev/null | tr -d ' ' || true); fi
  if [ -d /run/runit ] || [ -d /var/service ]; then INIT="runit"
  elif have dinitctl || [ -d /etc/dinit.d ]; then INIT="dinit"
  elif have openrc-run || [ -d /etc/runlevels ]; then INIT="openrc"
  elif [ "$_p1" = "systemd" ]; then INIT="systemd"
  else INIT="unknown($_p1)"
  fi
  log "init detectado: $INIT (pid1: ${_p1:-?})"
}

detect_libc() {
  if ldd --version 2>&1 | grep -qi musl; then LIBC="musl"
  elif ldd --version 2>&1 | grep -qi -E 'glibc|gnu'; then LIBC="glibc"
  else
    # fallback: el loader musl existe?
    if [ -e /lib/ld-musl-x86_64.so.1 ] || [ -e /lib/ld-musl-aarch64.so.1 ]; then LIBC="musl"
    else LIBC="unknown"
    fi
  fi
  log "libc detectada: $LIBC"
  if [ "$LIBC" != "musl" ]; then
    warn "mwan esta optimizado para musl; continuamos igual (funciona tambien en glibc)."
  fi
}

detect_gpu() {
  GPU="unknown"
  # Via sysfs (sin lspci, funciona en Chimera minima)
  if [ -d /sys/bus/pci/devices ]; then
    for d in /sys/bus/pci/devices/*; do
      if [ -r "$d/vendor" ] && [ -r "$d/class" ]; then
        _v=$(cat "$d/vendor" 2>/dev/null || true)
        _c=$(cat "$d/class" 2>/dev/null || true)
        case "$_c" in
          0x03*) # display controller
            case "$_v" in
              0x10de) GPU="nvidia" ;;
              0x8086) [ "$GPU" = "unknown" ] && GPU="intel" ;;
              0x1002) [ "$GPU" = "unknown" ] && GPU="amd" ;;
            esac
            ;;
        esac
      fi
    done
  fi
  if [ "$GPU" = "unknown" ] && have lspci; then
    if lspci 2>/dev/null | grep -qi nvidia; then GPU="nvidia"
    elif lspci 2>/dev/null | grep -qi -E 'amd|radeon'; then GPU="amd"
    elif lspci 2>/dev/null | grep -qi intel; then GPU="intel"
    fi
  fi
  log "gpu detectada: $GPU (v1 solo Mesa Intel/AMD; NVIDIA avisa y sigue con Intel)"
}

detect_all() {
  detect_distro; detect_pkgmgr; detect_init; detect_libc; detect_gpu
}

# --- instalacion / actualizacion por distro ---
xbps_ensure() {
  # $1... = paquetes xbps
  log "xbps: sincronizando indices..."
  run_priv xbps-install -S || warn "no se pudo sincronizar xbps (seguimos)"
  log "xbps: instalando/actualizando: $*"
  # -Sy ya hecho con -S; -y = sin confirmacion, -u = upgrade si ya instalado
  run_priv xbps-install -y "$@" || die "fallo xbps-install $*"
}

apk_ensure() {
  log "apk: actualizando indices..."
  run_priv apk update || warn "no se pudo actualizar apk (seguimos)"
  log "apk: instalando/actualizando: $*"
  run_priv apk add --no-cache "$@" || die "fallo apk add $*"
}

portage_ensure() {
  warn "portage: la instalacion puede compilar (lento en musl). Se intenta binario si existe."
  run_priv emerge --ask=n --update --newuse "$@" || die "fallo emerge $*"
}

ensure_host_deps() {
  case "$PKGMGR" in
    xbps)
      xbps_ensure nix bubblewrap fuse3 mesa mesa-dri mesa-vulkan-intel vulkan-loader dbus curl xz git elogind 2>/dev/null \
        || xbps_ensure nix bubblewrap fuse3 mesa vulkan-loader dbus curl xz git
      ;;
    apk)
      # Nombres difieren entre Alpine/Chimera/Adelie/pmOS; se intenta el set comun y se degrada.
      apk_ensure nix bubblewrap fuse3 mesa-vulkan-intel vulkan-loader dbus curl xz git 2>/dev/null \
        || apk_ensure nix bubblewrap fuse3 mesa vulkan-loader dbus curl xz git 2>/dev/null \
        || apk_ensure nix bubblewrap fuse curl xz 2>/dev/null \
        || warn "apk parcial: instala a mano 'nix bubblewrap fuse3 mesa dbus curl xz git'"
      ;;
    portage)
      portage_ensure app-admin/nix sys-apps/bubblewrap sys-fs/fuse:3 media-libs/mesa media-libs/vulkan-loader sys-apps/dbus net-misc/curl app-arch/xz dev-vcs/git
      ;;
    *)
      warn "gestor desconocido: no puedo instalar deps del host. Instala a mano: nix, bubblewrap, fuse3, mesa, vulkan-loader, dbus, curl, xz, git."
      ;;
  esac
}

ensure_nix_itself() {
  if have nix && have nix-daemon; then
    log "nix ya presente: $(nix --version 2>/dev/null || echo '?')"
    return 0
  fi
  warn "nix no encontrado: instalando segun distro ($DISTRO/$PKGMGR)..."
  case "$DISTRO" in
    void) xbps_ensure nix ;;
    alpine|adelie|postmarketos|postmarketOS)
      apk_ensure nix 2>/dev/null || {
        warn "apk sin paquete nix: usando instalador oficial single-user"
        have curl || die "se necesita curl para el instalador nix"
        curl -fsSL https://nixos.org/nix/install | sh -s -- --no-daemon || die "fallo instalador nix"
      }
      ;;
    chimera)
      # El instalador oficial usa flags GNU; usar el fork portable
      have curl || die "se necesita curl para el instalador nix-chimera"
      log "chimera: usando nix-installer-chimera (portable, sin GNU coreutils)..."
      curl -fsSL "$CHIMERA_NIX_INSTALLER_URL" | sh || die "fallo instalador nix-chimera"
      ;;
    gentoo) portage_ensure app-admin/nix ;;
    *)
      case "$PKGMGR" in
        xbps) xbps_ensure nix ;;
        apk) apk_ensure nix || die "instala nix manualmente" ;;
        portage) portage_ensure app-admin/nix ;;
        *) die "sin nix y sin gestor conocido. Instala nix manualmente: https://nixos.org/download" ;;
      esac
      ;;
  esac
  have nix || die "nix sigue sin estar en PATH tras instalar. Re-loguea (source /etc/profile) y reintenta."
  log "nix instalado: $(nix --version 2>/dev/null || echo '?')"
}

ensure_nix_daemon() {
  case "$INIT" in
    runit)
      if [ -e /etc/sv/nix-daemon ] && [ ! -e /var/service/nix-daemon ]; then
        log "runit: habilitando nix-daemon..."
        run_priv ln -s /etc/sv/nix-daemon /var/service/ || warn "no se pudo habilitar nix-daemon"
        sleep 2
      fi
      ;;
    openrc)
      if have rc-update && have rc-service; then
        log "openrc: habilitando nix-daemon..."
        run_priv rc-update add nix-daemon default 2>/dev/null || true
        run_priv rc-service nix-daemon start 2>/dev/null || true
      fi
      ;;
    dinit)
      if have dinitctl; then
        log "dinit: habilitando nix-daemon..."
        run_priv dinitctl enable nix-daemon 2>/dev/null || warn "revisa 'dinitctl enable nix-daemon' a mano"
        run_priv dinitctl start nix-daemon 2>/dev/null || true
      fi
      ;;
    *) warn "init $INIT sin receta automatica: si usas multi-usuario arranca nix-daemon a mano; si no, nix funciona en single-user." ;;
  esac
  # Aviso no fatal si el daemon no responde (single-user sigue valiendo)
  if have nix; then
    if nix store ping >/dev/null 2>&1; then log "nix store responde OK"
    else warn "nix store no responde (daemon parado?). Single-user sigue OK; multi-user requiere nix-daemon."
    fi
  fi
}

ensure_profile_env() {
  # /etc/profile y nix.sh no son `set -u` limpios: se importan con -u/-e
  # desactivados y luego se restauran las opciones originales.
  _old_opts="$-"
  set +e +u
  # Nix en Void instala /etc/profile.d/nix.sh; en instalaciones manuales puede faltar en la sesion actual
  for f in /etc/profile.d/nix.sh /etc/profile; do
    if [ -r "$f" ]; then
      # shellcheck disable=SC1090
      . "$f" 2>/dev/null || true
    fi
  done
  case "$_old_opts" in *e*) set -e ;; esac
  case "$_old_opts" in *u*) set -u ;; esac
  have nix || export PATH="$HOME/.nix-profile/bin:/nix/var/nix/profiles/default/bin:$PATH"
}

ensure_channels() {
  have nix-channel || { warn "sin nix-channel (instalacion solo-flakes?). Saltando canales."; return 0; }
  _list=$(nix-channel --list 2>/dev/null || true)
  case "$_list" in
    *nixpkgs*) log "canal nixpkgs ya suscrito" ;;
    *) log "suscribiendo canal nixpkgs-unstable..."; nix-channel --add "$NIXPKGS_CHANNEL_URL" nixpkgs || warn "fallo al suscribir nixpkgs" ;;
  esac
  case "$_list" in
    *nixgl*) log "canal nixgl ya suscrito" ;;
    *) log "suscribiendo canal nixgl..."; nix-channel --add "$NIXGL_CHANNEL_URL" nixgl || warn "fallo al suscribir nixgl (no fatal en v1 Mesa)" ;;
  esac
  log "actualizando canales (puede tardar)..."
  nix-channel --update || warn "nix-channel --update fallo (bug conocido en musl antiguo). Reintenta tras re-loguear o usa flakes."
}

ensure_nix_conf() {
  _conf="/etc/nix/nix.conf"
  if [ ! -w "$_conf" ] && [ "$(id -u)" -ne 0 ]; then
    warn "sin permiso para escribir $_conf; se configura ~/.config/nix/nix.conf de usuario"
    _conf="$HOME/.config/nix/nix.conf"
    mkdir -p "$(dirname "$_conf")"
    [ -f "$_conf" ] || : > "$_conf"
  fi
  [ -f "$_conf" ] || { run_priv mkdir -p "$(dirname "$_conf")" 2>/dev/null || mkdir -p "$(dirname "$_conf")"; : > "$_conf" 2>/dev/null || run_priv sh -c ": > $_conf"; }
  _need=""
  grep -q '^[[:space:]]*experimental-features' "$_conf" 2>/dev/null || _need="$_need experimental-features=nix-command flakes"
  grep -q '^[[:space:]]*auto-optimise-store' "$_conf" 2>/dev/null || _need="$_need auto-optimise-store=true"
  grep -q '^[[:space:]]*substituters' "$_conf" 2>/dev/null || _need="$_need substituters=https://cache.nixos.org https://nix-community.cachix.org"
  grep -q '^[[:space:]]*max-jobs' "$_conf" 2>/dev/null || _need="$_need max-jobs=2"
  if [ -n "$_need" ]; then
    log "optimizando $_conf para musl ligero..."
    # cp -a (portable, Chimera-safe). No usar --preserve.
    cp -a "$_conf" "$_conf.bak-mwan" 2>/dev/null || cp "$_conf" "$_conf.bak-mwan" 2>/dev/null || true
    for line in "experimental-features = nix-command flakes" "auto-optimise-store = true" "substituters = https://cache.nixos.org https://nix-community.cachix.org" "trusted-public-keys = cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY= nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs=" "keep-outputs = false" "keep-derivations = false" "max-jobs = 2"; do
      _k=$(printf '%s' "$line" | cut -d= -f1 | tr -d ' ')
      grep -q "^$_k" "$_conf" 2>/dev/null || printf '%s\n' "$line" >> "$_conf" 2>/dev/null || run_priv sh -c "printf '%s\n' '$line' >> $_conf"
    done
    log "respaldo en $_conf.bak-mwan"
  else
    log "nix.conf ya optimizado"
  fi
}

do_clean() {
  mwan_lock
  log "limpieza regla de oro (wipe-history + gc + optimise)..."
  if have nix; then
    nix profile wipe-history 2>/dev/null || nix-collect-garbage -d 2>/dev/null || warn "wipe-history no disponible (perfil viejo?)"
    nix-collect-garbage -d 2>/dev/null || nix store gc 2>/dev/null || warn "gc fallo (daemon parado?)"
    nix-store --optimise 2>/dev/null || nix store optimise 2>/dev/null || warn "optimise no disponible"
  fi
  case "$PKGMGR" in
    xbps) run_priv xbps-remove -yOo 2>/dev/null || true ;; # -y: sin pregunta (solo huerfanos)
    apk) run_priv apk cache clean 2>/dev/null || true ;;
    *) : ;;
  esac
  if have nix; then
    _sz=$(du -sh /nix 2>/dev/null | cut -f1 || echo "?")
    log "/nix actual: $_sz"
  fi
  mwan_unlock
}

do_update() {
  mwan_lock
  case "$PKGMGR" in
    xbps) run_priv xbps-install -Su 2>/dev/null || warn "fallo xbps -Su" ;;
    apk) run_priv apk upgrade 2>/dev/null || warn "fallo apk upgrade" ;;
    portage) warn "portage upgrade completo omitido (costoso). Usa emerge -uDN @world a mano." ;;
    *) warn "update del sistema omitido (gestor desconocido)" ;;
  esac
  if have nix-channel; then nix-channel --update || warn "fallo channel update"; fi
  if have nix; then nix profile upgrade '.*' 2>/dev/null || warn "nada que actualizar en nix profile (o usa flakes)"; fi
  # Los canales cambiaron -> los FHS cacheados de vxr quedan obsoletos; se invalidan
  # (el proximo `vxr` reconstruye cada perfil una vez y vuelve a cachear).
  rm -f "$HOME/.cache/vx"/fhs-path* "${XDG_CACHE_HOME:-$HOME/.cache}"/vx/fhs-path* 2>/dev/null || true
  do_clean
  mwan_unlock
}

cmd_doctor() {
  _fail=0
  detect_all
  ensure_profile_env
  printf '\n=== mwan doctor v%s ===\n' "$MWAN_VERSION"
  printf 'distro=%s pkgmgr=%s init=%s libc=%s gpu=%s\n' "$DISTRO" "$PKGMGR" "$INIT" "$LIBC" "$GPU"
  for t in nix nix-channel nix-store bwrap fusermount3 curl xz git; do
    if have "$t"; then printf '  [OK] %s: %s\n' "$t" "$(command -v "$t")"
    else printf '  [FALTA] %s\n' "$t"; _fail=1
    fi
  done
  if [ -c /dev/fuse ] || [ -c /dev/fuse3 ]; then printf '  [OK] /dev/fuse presente\n'
  else printf '  [FALTA] /dev/fuse (AppImage necesitara fallback extract)\n'; _fail=1
  fi
  if [ -e /dev/dri/card0 ] || [ -d /dev/dri ]; then printf '  [OK] /dev/dri presente (Mesa %s)\n' "$GPU"
  else printf '  [AVISO] sin /dev/dri (render software)\n'
  fi
  if have nix && nix store ping >/dev/null 2>&1; then printf '  [OK] nix store responde\n'
  else printf '  [AVISO] nix store no responde (daemon parado o single-user)\n'
  fi
  if [ "$GPU" = "nvidia" ]; then printf '  [AVISO] NVIDIA detectada: v1 solo Mesa Intel/AMD. Se sigue con Intel.\n'; fi
  if [ "$_fail" -eq 0 ]; then printf 'doctor: TODO OK\n'; else printf 'doctor: FALTAN piezas -> ejecuta ./mwan.sh bootstrap\n'; fi
  return "$_fail"
}

cmd_bootstrap() {
  mwan_lock
  detect_all
  detect_priv
  ensure_nix_itself
  ensure_profile_env
  ensure_host_deps
  ensure_nix_daemon
  ensure_nix_conf
  ensure_channels
  do_clean
  mwan_unlock
  printf '\n[mwan] sistema listo. Prueba: ./mwan.sh doctor && ./mwan.sh install hello\n'
}

cmd_install() {
  # ./mwan.sh install <pkg> [más...]: delega a vx si existe, si no nix profile
  [ $# -ge 1 ] || die "uso: $0 install <paquete> [...]"
  detect_priv
  ensure_profile_env
  if ! have nix; then cmd_bootstrap; fi
  # Se prefiere el vx instalado en PATH; fallback al build de desarrollo local.
  if have vx; then
    log "delegando a vx install $*"
    exec vx install "$@"
  elif [ -x ./target/release/vx ]; then
    log "delegando a ./target/release/vx install $*"
    exec ./target/release/vx install "$@"
  fi
  # Fallback sin vx: nix profile directo, bajo candado (vx se bloquea solo).
  mwan_lock
  for p in "$@"; do
    log "nix profile install nixpkgs#$p ..."
    # El listado normal trae colores ANSI; se usa --json para el chequeo.
    if nix profile list --json 2>/dev/null | grep -F -q "\"$p\":"; then
      log "$p ya instalado; actualizando..."
      nix profile upgrade "$p" 2>/dev/null || nix profile upgrade '.*' 2>/dev/null || true
    else
      nix profile install "nixpkgs#$p" || { mwan_unlock; die "fallo instalando $p"; }
    fi
  done
  do_clean
  mwan_unlock
  log "instalado: $*. Para GUI recuerda que vx reescribe .desktop a vxr (si usas vx)."
}

cmd_run() {
  [ $# -ge 1 ] || die "uso: $0 run <binario> [args...]"
  ensure_profile_env
  if have vxr; then
    exec vxr "$@"
  elif [ -x ./target/release/vxr ]; then
    exec ./target/release/vxr "$@"
  fi
  # Fallback sin vxr: nixGL directo si es GUI, si no ejecucion directa sanitizada
  env -u LD_LIBRARY_PATH -u LD_PRELOAD "$@" 2>/dev/null || exec "$@"
}

main() {
  cmd="${1:-full}"
  case "$cmd" in
    doctor) shift; cmd_doctor "$@" ;;
    bootstrap) shift; cmd_bootstrap "$@" ;;
    update|upgrade) shift; detect_all; detect_priv; ensure_profile_env; do_update "$@" ;;
    clean|gc) shift; detect_all; detect_priv; ensure_profile_env; do_clean "$@" ;;
    install|add|i) shift; cmd_install "$@" ;;
    run|r|exec) shift; cmd_run "$@" ;;
    full|"") cmd_bootstrap ;;
    version|--version|-V) printf 'mwan %s\n' "$MWAN_VERSION" ;;
    help|--help|-h)
      printf 'mwan %s — asistente Nix universal-musl\n' "$MWAN_VERSION"
      printf 'uso: ./mwan.sh [doctor|bootstrap|update|clean|install <pkg>|run <bin>|help]\n'
      ;;
    *) die "comando desconocido: $cmd (prueba ./mwan.sh help)" ;;
  esac
}

main "$@"
