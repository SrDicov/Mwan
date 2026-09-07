# Mwan — asistente Nix universal para distros musl (`vx` / `vxr`)

Envoltorio imperativo sobre Nix para usar software glibc (Steam, AppImages,
binarios cerrados) en hosts musl sin contaminar el sistema base.
Tesis completa en `tesis.txt`; implementacion fiel a §§1-5.

## Piezas

- `mwan.sh` — bootstrap POSIX sh (cero deps). Detecta distro/init/libc/gpu,
  instala/actualiza Nix + deps, configura `nix.conf` ligero, suscribe canales,
  limpia (`wipe-history + gc + optimise`). Corre en Void/Alpine/Chimera/Adelie/Gentoo/pmOS.
- `vx-fhs.nix` + `vx-fhs-common.nix` — perfil FHS **base** (~400MB medidos,
  sin `steam-run` de GBs) con `glibc+glibc.bin`, libGL (glvnd), Vulkan, fuse2/3,
  dbus, glib. Cadena `[bwrap]->[nixGL]->[bin]`.
- `vx-fhs-appimage.nix` — perfil FHS **extendido** (~1.4GB, lista curada de
  nixpkgs para AppImages: gtk3, nss, cups...). `vxr` lo elige solo ante
  `.AppImage`; solo se descarga al usar el primer AppImage, nunca para el resto.
- `vx-rs/` — binario unico estatico-musl multicall (`vx` gestiona, `vxr` ejecuta).
  `cargo build --release` → `target/release/vx` (~545K, static-pie). Crear `ln -s vx vxr`.
- `tests/doctor.sh` — self-test sin mutar el sistema.
- `docs/MATRIZ-MUSL.md` — que distros soporta y como se instala Nix en cada una.

## Uso rapido (en tu Void musl actual)

```sh
./mwan.sh doctor      # diagnostico (no muta)
./mwan.sh bootstrap   # deja todo listo (nix+deps+canales+gc)
./mwan.sh install hello
./mwan.sh update
./mwan.sh clean

# Rust (ya compilado):
./vx-rs/target/release/vx doctor
./vx-rs/target/release/vx install librewolf
./vx-rs/target/release/vx list
ln -sf vx vx-rs/target/release/vxr
./vx-rs/target/release/vxr --help
./vx-rs/target/release/vxr steam            # GUI del perfil Nix via FHS+nixGLIntel
./vx-rs/target/release/vxr ./App.AppImage   # FUSE o fallback --appimage-extract
./vx-rs/target/release/vxr gh --version     # CLI del perfil Nix via FHS+nixGLIntel
```

Instalacion permanente sugerida:

```sh
mkdir -p ~/.local/bin ~/.config/mwan
cp vx-rs/target/release/vx ~/.local/bin/
ln -sf vx ~/.local/bin/vxr
cp vx-fhs.nix vx-fhs-common.nix vx-fhs-appimage.nix ~/.config/mwan/
cp mwan.sh ~/.local/bin/mwan
```

## Rendimiento medido (Intel N150, Void musl)

- `vxr` primera vez por perfil (evalua nixpkgs): ~8-25s. Siguientes: **~150-400ms**
  gracias a `~/.cache/vx/fhs-path-{base,appimage}` (valida con `nix-store --check-validity`).
- Spawn puro del FHS (`.../bin/vxr-fhs echo`): ~115ms frio.
- `vxr --rebuild` o `./mwan.sh update` invalidan la cache (canales nuevos).
- Binario `vx`: ~545K static-pie musl, cero dependencias.

## Regla de oro (automatica)

Tras cada `vx install/remove/update`: `nix profile wipe-history` +
`nix store gc` (fallback `nix-collect-garbage -d`) + `nix store optimise`.
Los `.desktop` de `~/.nix-profile/share/applications` se reescriben a
`~/.local/share/applications` con `Exec=vxr ...` + marca `# X-Mwan-Managed`.

## Detalles que dan guerra (ya resueltos)

- **Paquetes unfree (steam):** `nix profile` (flakes) ignora tu
  `~/.config/nixpkgs/config.nix` y evalua en modo puro. `vx` refleja tu
  `allowUnfree` con `NIXPKGS_ALLOW_UNFREE=1` e invoca con `--impure`
  (posicion: despues del subcomando). Sin esto, steam falla con
  "unfree license" aunque ya lo tuvieras instalado.
- **`vx update` es por elementos:** un flake local borrado (ej. `~/antigravity`)
  ya no aborta todo; avisa y sigue. Codigo de salida 1 si hubo saltos.
- **Wrappers con sandbox propio:** el `steam` de nixpkgs es un script bwrap;
  `vxr` lo ejecuta en host + nixGL en vez de anidar FHS.
- **AppImages:** perfil extendido automatico + fallback a extraccion con
  `APPDIR` fijado si FUSE falla en el namespace.
- **Seguridad en PCs justos:** candado `/tmp/mwan-nix.lock` serializa
  install/update/gc/search/builds (dos evaluaciones gordas en 3.5GB
  congelan el sistema; visto en la practica).

## Limites v1

- Solo 64-bit (`multiPkgs=[]`), solo Mesa Intel/AMD (NVIDIA avisa y sigue con Intel).
- Hibrido channels+flakes: prefiere `nix profile nixpkgs#pkg`, fallback `nix-env -iA`.
- No compilar en N150 salvo necesidad: siempre substitutos binarios (`max-jobs=2`).
