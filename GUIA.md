# Guía de uso de Mwan (`vx` / `vxr` / `mwan`)

Asistente imperativo sobre Nix para distros **musl** (Void, Alpine, Chimera,
Adélie, Gentoo-musl, postmarketOS). Te deja instalar y abrir software glibc
(Steam, AppImages, binarios cerrados) sin aprender Nix y sin ensuciar el sistema.

> Requisitos: Nix instalado (o deja que `mwan bootstrap` lo ponga),
> `~/.local/bin` en tu `PATH`, GPU Intel/AMD para aceleración (NVIDIA:
> solo software en v1).

---

## 1. Instalación (una vez)

```sh
git clone https://github.com/SrDicov/Mwan.git && cd Mwan
./mwan.sh bootstrap   # detecta tu distro, instala/actualiza Nix y
                      # dependencias, configura nix.conf ligero, suscribe
                      # canales, limpia. Tarda unos minutos la 1ª vez.

mkdir -p ~/.local/bin ~/.config/mwan
cp vx-rs/target/release/vx ~/.local/bin/   # o baja el binario del CI:
ln -sf vx ~/.local/bin/vxr                 #   gh run download -n vx-x86_64-unknown-linux-musl
cp vx-fhs.nix vx-fhs-common.nix vx-fhs-appimage.nix ~/.config/mwan/
cp mwan.sh ~/.local/bin/mwan
```

Comprueba:

```sh
vx doctor        # TODO OK = listo
mwan doctor      # lo mismo desde el bootstrap
```

---

## 2. Uso diario: `vx` (gestionar paquetes)

`vx` habla como `apt`/`xbps`, pero por debajo usa `nix profile` + limpieza
automática (cero huella: tras cada cambio hace `wipe-history + gc + optimise`).

| Comando | Qué hace |
|---|---|
| `vx install steam heroic discord` | Instala 1 o varios (acepta `steam` o `nixpkgs#steam`). Si ya existe, lo actualiza en vez de chocar. Integra lanzadores del menú (`Exec=vxr …`). |
| `vx remove discord` | Desinstala + borra sus lanzadores + limpia. |
| `vx update` | Actualiza elemento por elemento (si un flake local está roto, avisa y sigue; exit 1 si hubo saltos). |
| `vx search firefox` | Busca en nixpkgs (lento en PCs justos: evalúa; paciencia). |
| `vx list` | Lo instalado en tu perfil. |
| `vx gc` | Solo limpieza (regla de oro manual). |
| `vx doctor` | Diagnóstico rápido. |

Notas:

- Las apps GUI aparecen en tu menú solas (Noctalia/Rofi/dmenu leen
  `~/.local/share/applications`). Si un icono queda huérfano, `vx remove` lo borra.
- `steam`, `librewolf` y demás binarios del perfil se lanzan con `vxr`
  (ver §3); el lanzador ya viene reescrito.
- Unfree (steam): `vx` refleja tu `~/.config/nixpkgs/config.nix`
  (`{ allowUnfree = true; }`). Sin ese fichero, steam no se instala (a propósito).

---

## 3. Ejecutar: `vxr` (binarios glibc y AppImages)

`vxr` mete el programa en un FHS glibc mínimo + inyección GPU (nixGL),
sanitizando el entorno musl. Resolución automática:

- `vxr steam` → binario del perfil Nix (lo resuelve a `/nix/store/...`).
- `vxr librewolf`, `vxr gh --version` → igual, con aceleración incluida.
- `vxr ./Juego.AppImage` → perfil extendido AppImage; si FUSE falla en el
  namespace, extrae y ejecuta solo (borra al salir).
- `vxr echo hola` → herramientas del propio FHS.

Flags:

```sh
vxr --rebuild steam        # reconstruir el FHS (tras cambiar canales)
vxr echo --no-gl hola      # sin nixGL (binarios CLI puros)
vxr --help
```

Rendimiento (N150): 1ª vez ~10–25 s (construye), siguientes **~120–400 ms**
gracias a la caché `~/.cache/vx/fhs-path-{base,appimage}`.

---

## 4. Mantenimiento: `mwan`

| Comando | Qué hace |
|---|---|
| `mwan doctor` | Diagnóstico (no toca nada). |
| `mwan bootstrap` | Flujo completo: detectar → asegurar Nix+deps → daemon → `nix.conf` → canales → limpiar. |
| `mwan update` | Sistema + canales + perfil + invalidar cachés FHS + limpiar. |
| `mwan clean` | Solo regla de oro + cachés del gestor nativo. |
| `mwan install <pkg>` | Delega a `vx` si existe; si no, `nix profile` directo. |
| `mwan run <bin> [args]` | Delega a `vxr` si existe. |

Seguridad en PCs justos: `vx`, `vxr` y `mwan` comparten el candado
`/tmp/mwan-nix.lock` — **nunca corren dos operaciones pesadas de Nix a la vez**
(dos evaluaciones gordas en 3–4 GB de RAM congelan el equipo). Si ves
`otra operacion Nix pesada en curso…`, espera: es el candado trabajando.

---

## 5. Problemas típicos

| Síntoma | Causa y arreglo |
|---|---|
| `unfree license` con steam | Falta `{ allowUnfree = true; }` en `~/.config/nixpkgs/config.nix`. |
| App .NET muere al instante (`Couldn't find a valid ICU package`) | Falta `icu` directo en el FHS (el rootfs solo enlaza deps directos para `dlopen` por nombre). Ya incluido desde v0.1.0+; `vxr --rebuild` si tu caché es vieja. |
| `vx update` avisa de un flake y sigue | Directorio de un flake local borrado (ej. `~/antigravity`). Restaura el dir o `vx remove <nombre>`. Exit 1 = hubo saltos, el resto se actualizó. |
| AppImage: `fusermount: Operation not permitted` | Kernel/namespace sin FUSE: `vxr` extrae y ejecuta solo automáticamente. |
| AppImage Electron: `libnspr4.so` no existe | Se usa el perfil base por error (¿renombraste el `.AppImage` sin esa extensión?). Con extensión correcta usa el extendido. |
| `vxr steam` tarda la 1ª vez | Steam descarga su propio runtime (~300 MB) una vez. Normal. |
| Pantalla negra / sin aceleración | Revisa `vxr glxinfo -B` → `direct rendering: Yes`. En NVIDIA v1 no hay driver propietario. |
| `git push` falla tras actualizar `gh` | Reejecuta `gh auth setup-git` o usa el helper a `~/.nix-profile/bin/gh` (rutas absolutas del store caducan con el GC). |
| `ld-linux` huesped en `/usr/lib` (Void musl) | Resto de otro invento; Mwan no lo necesita ni lo toca. No borrar a ciegas si Steam clásico lo usa. |

---

## 6. Desinstalar Mwan (dejar todo como estaba)

```sh
vx gc
rm -f ~/.local/bin/vx ~/.local/bin/vxr ~/.local/bin/mwan
rm -rf ~/.config/mwan ~/.cache/vx
# Tus paquetes Nix y tu sistema quedan intactos. Para quitar Nix entero:
#   sudo rm -rf /nix && xbps-remove nix   # (adapta a tu distro)
```

---

## 7. Chuleta mínima

```sh
vx install librewolf && vxr librewolf          # instalar + abrir
vxr ~/Juego.AppImage                           # AppImage, sin instalar nada
vx update && vx gc                             # mantener
mwan doctor                                    # ¿todo bien?
```
