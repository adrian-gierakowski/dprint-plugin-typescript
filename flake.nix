{
  description = "A dprint plugin for formatting TypeScript and JavaScript";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?rev=c6a788f552b7b7af703b1a29802a7233c0067908";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ rust-overlay.overlays.default ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rustVersion = pkgs.rust-bin.stable."1.88.0".default;
        # Define the rust toolchain once with all necessary extensions and targets.
        rustToolchain = rustVersion.override {
          extensions = [ "rust-src" ];
          targets = [ "wasm32-unknown-unknown" ];
        };
      in
      {
        # This now builds the .wasm file instead of a native .so file.
        packages.default = pkgs.rustPlatform.buildRustPackage rec {
          pname = "dprint-plugin-typescript";
          version = "0.95.8";

          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          # We don't need native dependencies like openssl for a wasm build.
          buildInputs = [ ];
          nativeBuildInputs = with pkgs; [
            # Ensure the build environment uses the same toolchain as the dev shell.
            rustToolchain
            pkg-config
          ];

          # --- FIXED BUILD PROCESS ---
          # Instead of relying on environment variables that can be ignored,
          # we now explicitly define the build command.
          buildPhase = ''
            runHook preBuild

            echo "--- Running explicit WASM build command ---"
            cargo build --release \
              --target wasm32-unknown-unknown \
              --features "wasm"
            echo "-------------------------------------------"

            runHook postBuild
          '';

          # The installPhase remains the same, as it should now find the
          # correctly built .wasm file.
          installPhase = ''
            runHook preInstall

            # For debugging, list the contents of the release directory.
            echo "--- Contents of target/wasm32-unknown-unknown/release/ ---"
            ls -la target/wasm32-unknown-unknown/release/
            echo "---------------------------------------------------------"

            # Find the generated .wasm file instead of hardcoding the name.
            wasm_file=$(find target/wasm32-unknown-unknown/release -maxdepth 1 -type f -name "*.wasm")

            if [ -z "$wasm_file" ]; then
              echo "ERROR: Build succeeded, but no .wasm file was found." >&2
              exit 1
            fi

            echo "Found Wasm file: $wasm_file"

            # Install the found wasm file to a consistent output name.
            install -Dm644 "$wasm_file" \
              "$out/dprint-plugin-typescript.wasm"

            runHook postInstall
          '';
        };
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            (rustVersion.override {
              extensions = [ "rust-src" ];
              targets = [ "wasm32-unknown-unknown" ];
            })
            cargo
            clippy
            rustfmt
            openssl
            pkg-config
          ];
        };
      }
    );
}
