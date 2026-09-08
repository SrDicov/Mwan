# vx-fhs-common.nix — constructor compartido de los perfiles FHS de Mwan.
# Lo usan vx-fhs.nix (base, binarios del perfil Nix) y vx-fhs-appimage.nix
# (extendido, AppImages). Mantener profile/extraBwrapArgs/runScript iguales
# aqui beneficia a ambos perfiles a la vez.
{ pkgs, extraPkgs ? [ ], name ? "vxr-fhs" }:

pkgs.buildFHSEnv {
  inherit name;

  # Base minima comun (~400MB medidos con nixpkgs-unstable x86_64):
  # glibc + cargadores GL/Vulkan + X11 + IPC/audio/fuentes + shell.
  # Los drivers reales los inyecta nixGLIntel desde su propio closure.
  # (El propio buildFHSEnv mete glibcLocales ~230MB sin opcion a excluirlo.)
  targetPkgs = pkgs: (with pkgs; [
    glibc
    glibc.bin # /sbin/ldconfig (lo exige Steam/pressure-vessel)
    gcc-unwrapped.lib # libstdc++.so
    zlib

    libx11
    libxext
    libxcursor
    libxrandr
    libGL # libglvnd; los DRI reales vienen de nixGL
    vulkan-loader
    libdrm

    # fuse v2 + v3: AppImages viejos (tipo 1) exigen libfuse.so.2
    fuse3
    fuse
    alsa-lib
    dbus
    glib
    freetype
    fontconfig
    dejavu_fonts # unicas fuentes garantizadas (ver NOTA de binds arriba)
    # icu DIRECTO (no transitivo): el rootfs solo enlaza en /usr/lib los
    # deps directos, y .NET/Chromium hacen dlopen("libicuuc.so") por nombre.
    # Sin esto: "Couldn't find a valid ICU package" y muerte en ~200ms.
    # (Auditoria 2026-09-08: steam-run si lo enlaza via su multilib.)
    icu
    bash
    coreutils
  ]) ++ extraPkgs;

  # v1 64-bit only: evita el bloat multilib de steam-run
  multiPkgs = pkgs: (with pkgs; [ ]);

  extraBwrapArgs = [
    # /dev/fuse para AppImage (tesis §3.2). try: no aborta en hosts sin fuse.
    "--dev-bind-try /dev/fuse /dev/fuse"
    "--bind-try /tmp/dumps /tmp/dumps"
    # NOTA (auditoria 2026-09-07): se intento --ro-bind de /usr/share/fonts
    # del host, pero bwrap no puede crear el destino (/usr es read-only en el
    # contenedor) y ABORTA el arranque. Las fuentes van en el closure
    # (dejavu_fonts abajo); para NerdFonts del host, copiar a ~/.fonts
    # (HOME si esta bindeado).
    "--unshare-pid"
    "--die-with-parent"
    # NOTA: sin bind de /etc/resolv.conf — el buildFHSEnv moderno ya expone
    # /etc del host via symlinks (nixpkgs#273068); el --ro-bind rompia con
    # "Can't create file at /etc/resolv.conf" porque el destino no existe
    # en el rootfs generado.
    # NOTA: NO --unshare-user por defecto (romperia /dev/dri en varios hosts).
    # Si algun dia aparece el bug PR_SET_KEEPCAPS de bubblewrap, vxr debera
    # detectarlo y reintentar con --unshare-user (pendiente, ver tesis §4.3).
  ];

  profile = ''
    # Sanitizacion musl -> glibc (tesis §4.1)
    unset LD_PRELOAD
    unset GIO_EXTRA_MODULES
    unset LD_LIBRARY_PATH

    export FONTCONFIG_FILE=/etc/fonts/fonts.conf
    export XDG_DATA_DIRS="/usr/share:/usr/local/share:''${XDG_DATA_DIRS:-}"
    export SDL_JOYSTICK_DISABLE_UDEV=1

    # Calienta la cache de fuentes del usuario (auditoria 2026-09-07: en frio,
    # fc-list ve 1 fuente hasta que algo corre fc-cache; incremental = rapido).
    if command -v fc-cache >/dev/null 2>&1; then
      fc-cache >/dev/null 2>&1 || true
    fi

    # Puente de locales glibc (el FHS trae glibcLocales propio de respaldo;
    # si el usuario tiene otro en su perfil se prefiere el suyo, como nixGL).
    if [ -e "$HOME/.nix-profile/lib/locale/locale-archive" ]; then
      export LOCALE_ARCHIVE="$HOME/.nix-profile/lib/locale/locale-archive"
    fi

    # Rutas de drivers estilo NixOS: SOLO si existen (en NixOS las provee el
    # sistema; fuera de NixOS ese dir no existe y exportarlas envenena a
    # libGL/EGL/VA-API/VDPAU. nixGLIntel pone las buenas despues).
    # (Auditoria 2026-09-07: exportarlas a ciegas rompia EGL fuera de NixOS.)
    if [ -d /run/opengl-driver ]; then
      export LIBGL_DRIVERS_PATH="/run/opengl-driver/lib/dri"
      export __EGL_VENDOR_LIBRARY_DIRS="/run/opengl-driver/share/glvnd/egl_vendor.d"
      export LIBVA_DRIVERS_PATH="/run/opengl-driver/lib/dri"
      export VDPAU_DRIVER_PATH="/run/opengl-driver/lib/vdpau"
    fi

    # TZ: Steam/pressure-vessel se confunde con symlinks de /etc/localtime
    if [ -z "''${TZ+x}" ]; then
      new_TZ="$(readlink -f /etc/localtime 2>/dev/null | grep -P -o '(?<=/zoneinfo/).*$' 2>/dev/null || true)"
      if [ -n "$new_TZ" ]; then export TZ="$new_TZ"; fi
    fi
  '';

  # Encadenamiento universal: todo lo que venga despues se ejecuta dentro
  runScript = "bash -c 'exec \"$@\"' bash";
}
