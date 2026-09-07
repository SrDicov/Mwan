# Matriz musl — soporte Mwan v1 (Tier-1) + Tier-2 experimental

Investigado sep-2026. Criterio Tier-1: distro de proposito general/desktop con
Nix viable y comunidad activa. El resto es Tier-2 (se acepta `doctor` pero sin
garantia de `bootstrap` automatico).

| Distro | libc | Gestor | Init | Nix como se instala | Estado Mwan v1 |
|---|---|---|---|---|---|
| **Void musl** (tu host) | musl 1.2.6 | `xbps` | `runit` | `xbps-install -Sy nix` + `ln -s /etc/sv/nix-daemon /var/service/` | Soportado total. Sin `multilib` ni `i686-musl`. |
| **Alpine ≥3.0** | musl | `apk` + OpenRC | `openrc` | `apk add nix` (community 2.31.x) o installer `--no-daemon`. OJO: `nix-static` segfault como root → usar usuario normal | Soportado. Single-user si no hay daemon |
| **Chimera** | musl+mimalloc, LLVM, FreeBSD userland, sin GNU coreutils | `apk-v3` + `cports` | `dinit` | Fork `nix-installer-chimera` (`cp -a`, no `cp --preserve`). `dinitctl enable nix-daemon` | Soportado via fork. Flatpak es su via oficial glibc |
| **Adelie** | musl | `apk` | `openrc` | `apk add nix` o installer portatil (igual que Alpine) | Soportado (misma receta Alpine) |
| **Gentoo musl** | musl (stages `amd64-musl`, `i686-musl`, arm, ppc64…) | `portage` | `openrc`/`systemd` | `emerge app-admin/nix`. OJO `package.mask` grande en musl, compilacion lenta | Soportado con aviso de compilacion |
| **postmarketOS** | musl (base Alpine) | `apk` | `openrc`/`s6` | Igual que Alpine, normalmente single-user por RAM | Soportado modo ligero |

## Tier-2 experimental (sin garantia)

`Dragora 3`, `Morpheus`, `Sabotage`, forks `KISS` — sin mantenedor Nix activo.
`OpenWrt`, `Talos` — embebido/k8s, fuera de alcance (no desktop, sin FHS GUI).

## Recetas por init (lo que automatiza `mwan.sh`)

- `runit`: `ln -s /etc/sv/nix-daemon /var/service/` (+ `sv status nix-daemon`)
- `openrc`: `rc-update add nix-daemon default && rc-service nix-daemon start`
- `dinit`: `dinitctl enable nix-daemon && dinitctl start nix-daemon`
- desconocido: aviso + single-user (`--no-daemon`)

## Notas Nix-en-musl (bugs reales)

- `nix-channel --update` crasheo historicamente en Void-musl (void-packages#37382,
  ligado a postgres/template). Si falla: re-loguear (`source /etc/profile`) y
  reintentar, o usar `nix profile` con flakes.
- Nix oficial no garantiza build *sobre* musl (NixOS/nix#11931: libgit2/musl).
  No importa para Mwan: el Nix que usamos es binario glibc con su propio loader
  en `/nix/store/...-glibc/.../ld-linux-x86-64.so.2`, y los paquetes que instala
  traen su glibc. El host sigue musl puro.
- v1 = Mesa Intel/AMD (`nixGLIntel`). NVIDIA propietario excluido a proposito:
  exige glibc en userspace y es inestable sobre musl (doc gaming §2).
