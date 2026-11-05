{
  description = "Hadron legacy kernel development shell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          # rustup reads rust-toolchain.toml; gluon needs OpenSSL through git2.
          packages = with pkgs; [
            rustup
            just
            pkg-config
            openssl
            qemu
          ];
        };
      });
    };
}
