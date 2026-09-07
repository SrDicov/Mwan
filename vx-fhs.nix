# vx-fhs.nix — Perfil BASE (~400MB): binarios del perfil Nix (steam, librewolf...).
# Steam trae su propio runtime (pressure-vessel) y nixGL inyecta los drivers,
# asi que la base se mantiene minima. Para AppImages ver vx-fhs-appimage.nix.
# Uso: nix build -f vx-fhs.nix --no-link --print-out-paths
{ pkgs ? import <nixpkgs> { } }:

import ./vx-fhs-common.nix { inherit pkgs; }
