#!/bin/sh
# tests/doctor.sh — validacion rapida de Mwan sin mutar el sistema (solo lectura + builds efimeros)
# Uso: sh tests/doctor.sh
set -eu
cd "$(dirname "$0")/.."

fail=0
ok()   { printf '[OK] %s\n' "$*"; }
bad()  { printf '[FALLO] %s\n' "$*"; fail=1; }

printf '=== Mwan self-test ===\n'

# 1. sintaxis POSIX del bootstrap
if sh -n mwan.sh && dash -n mwan.sh 2>/dev/null; then ok "mwan.sh sintaxis POSIX"; else bad "mwan.sh sintaxis"; fi
# sin process substitution (bashismo prohibido en Chimera/dash)
if grep -q '<(' mwan.sh; then bad "mwan.sh contiene <() (no POSIX)"; else ok "mwan.sh sin bashismos <()"; fi
# sin flags GNU-only peligrosos en codigo (ignorando comentarios)
if grep -v '^[[:space:]]*#' mwan.sh | grep -q 'cp --preserve'; then bad "mwan.sh usa cp --preserve (rompe Chimera)"; else ok "mwan.sh usa cp -a portable"; fi

# 2. vx-fhs.nix parsea (base + comun + appimage)
for f in vx-fhs.nix vx-fhs-common.nix vx-fhs-appimage.nix; do
  if nix-instantiate --parse "$f" >/dev/null 2>&1; then ok "$f parsea"; else bad "$f no parsea"; fi
done
# contiene piezas obligatorias de la tesis (viven en el comun de perfiles)
for needle in 'buildFHSEnv' 'glibc.bin' 'fuse3' 'LIBGL_DRIVERS_PATH' 'die-with-parent'; do
  if grep -q "$needle" vx-fhs-common.nix; then ok "vx-fhs-common.nix contiene $needle"; else bad "vx-fhs-common.nix falta $needle"; fi
done
# el perfil appimage trae la lista curada de nixpkgs (gtk3/nss/cups)
for needle in 'gtk3' 'nss' 'cups' 'nspr'; do
  if grep -q "$needle" vx-fhs-appimage.nix; then ok "vx-fhs-appimage.nix contiene $needle"; else bad "vx-fhs-appimage.nix falta $needle"; fi
done

# 3b. salvaguarda anti-OOM: candado mutuo entre ops pesadas (ver triaje 2026-09-07)
if grep -q 'nix_lock' vx-rs/src/vx.rs && grep -q 'nix_lock' vx-rs/src/vxr.rs && grep -q 'mwan_lock' mwan.sh; then ok "candado nix presente (vx/vxr/mwan)"; else bad "falta candado nix"; fi
if [ -d /tmp/mwan-nix.lock ]; then bad "candado /tmp/mwan-nix.lock colgado"; else ok "sin candados colgados"; fi

# 3. Rust compila estatico musl y multicall funciona
if [ -x vx-rs/target/release/vx ]; then ok "binario vx existe"
else
  if (cd vx-rs && cargo build --release >/dev/null 2>&1); then ok "vx compila"; else bad "vx no compila"; fi
fi
if file vx-rs/target/release/vx 2>/dev/null | grep -q -E 'static|statically'; then ok "vx es estatico (musl)"; else bad "vx no es estatico"; fi
if ./vx-rs/target/release/vx version 2>/dev/null | grep -q '0.1.0'; then ok "vx version"; else bad "vx version"; fi
if ./vx-rs/target/release/vx doctor >/dev/null 2>&1; then ok "vx doctor exit 0"; else bad "vx doctor fallo"; fi

# 4. mwan.sh doctor no muta y sale 0 en este host
if ./mwan.sh doctor >/dev/null 2>&1; then ok "mwan.sh doctor exit 0"; else bad "mwan.sh doctor fallo"; fi

# 5. documentacion y matriz presentes
[ -f docs/MATRIZ-MUSL.md ] && ok "docs/MATRIZ-MUSL.md presente" || bad "falta MATRIZ-MUSL.md"
[ -f README.md ] && ok "README.md presente" || bad "falta README.md"

printf '\n%s\n' "$([ "$fail" -eq 0 ] && echo 'SELF-TEST: TODO OK' || echo 'SELF-TEST: HAY FALLOS')"
exit "$fail"
