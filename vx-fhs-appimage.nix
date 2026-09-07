# vx-fhs-appimage.nix — Perfil EXTENDIDO para AppImages (tesis §1.1: "perfiles
# de clausura adicionales" segun heuristica; aqui la heuristica es la extension
# .AppImage, que vxr detecta solo).
# Los AppImage (sobre todo Electron) esperan las system-libs de un Ubuntu base
# (nss, nspr, gtk3, cups...) que NO estan ni en musl ni en el perfil base.
# La lista extiende la base con el conjunto curado de nixpkgs para AppImages
# (pkgs/build-support/appimage/default.nix: defaultFhsEnvArgs, "taken from the
# Steam chroot" + excludelist de pkg2appimage). Medido 2026-09: closure ~1.4GB.
# Es el coste honesto del soporte universal AppImage (un gtk3 entero); sigue
# por debajo de steam-run y solo se construye/descarga la primera vez que se
# usa un AppImage, nunca para el resto.
# Uso: nix build -f vx-fhs-appimage.nix --no-link --print-out-paths
{ pkgs ? import <nixpkgs> { } }:

import ./vx-fhs-common.nix {
  inherit pkgs;
  extraPkgs = with pkgs; [
    # Herramientas que los AppImage suelen invocar
    bashInteractive
    zenity
    which
    desktop-file-utils # update-desktop-database (lo invocan los propios AppImage)
    xdg-utils
    xdg-user-dirs # apps de escritorio flutter
    iana-etc
    perl

    # Esquemas/iconos para que GTK no avise
    gsettings-desktop-schemas
    hicolor-icon-theme
    krb5
    libsecret # bitwarden y cia (solo x86_64)

    # Librerias del excludelist de pkg2appimage (se esperan en el host)
    libxcomposite
    libxtst
    libxfixes
    libGLU
    libSM
    libICE
    libXt
    libxmu
    libXinerama
    libXdamage
    libXrender
    libXScrnSaver
    libXxf86vm
    libXi
    libXft
    libxcb
    xkeyboard-config
    libpciaccess
    libuuid
    bzip2
    expat
    curlWithGnuTls
    openssl
    libidn
    dbus-glib
    atk
    at-spi2-atk
    libudev0-shim
    udev
    libusb1
    libcap
    cups
    cairo
    pango
    gdk-pixbuf
    librsvg
    pixman
    libgcrypt
    nspr
    nss
    gtk3
    libcanberra
    libvpx
    libxkbcommon
    libgbm
    wayland
    libvdpau
    libsamplerate
    libmikmod
    libthai
    libtheora
    libtiff
    libjpeg
    libpng12
    flac
    speex
    libogg
    libvorbis
    libpulseaudio
    SDL2
    SDL2_image
    SDL2_ttf
    SDL2_mixer
    libglut
    glew_1_10
    onetbb
  ];
}
