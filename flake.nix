{
  description = "A dprint plugin for formatting TypeScript and JavaScript";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ rust-overlay.overlays.default ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rustVersion = pkgs.rust-bin.stable."1.79.0".default;
      in
      {
        packages.dprint-plugin-typescript = pkgs.rustPlatform.buildRustPackage {
          pname = "dprint-plugin-typescript";
          version = "0.95.8";

          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          buildInputs = with pkgs; [
            openssl
          ];
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            (rustVersion.override {
              extensions = [ "rust-src" ];
            })
            cargo
            clippy
            rustfmt
            openssl
            pkg-config
          ];
        };
      });
}
