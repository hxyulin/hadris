{
  description = "Hadris filesystem conformance development shell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];
      forAllSystems = function:
        nixpkgs.lib.genAttrs systems (system: function (import nixpkgs { inherit system; }));
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cdrtools
            pkgs.dosfstools
            pkgs.libarchive
            pkgs.mtools
            pkgs.python3
            pkgs.prek
            pkgs.qemu
            pkgs.xorriso
            pkgs._7zz
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
            pkgs.util-linux
            pkgs.exfatprogs
            pkgs.udftools
            pkgs.ntfs3g
          ];
          LC_ALL = "C.UTF-8";
        };
      });
    };
}
