{
  description = "Myth II: Soulblighter source port - development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, advisory-db }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
            "clippy"
            "rustfmt"
          ];
          targets = [
            "wasm32-unknown-unknown"
          ];
        };

        runtimeLibs = with pkgs; [
          vulkan-loader
          vulkan-headers

          wayland
          libxkbcommon
          libx11
          libxcursor
          libxrandr
          libxi

          alsa-lib

          udev
        ];

      in {
        devShells.default = pkgs.mkShell {
          name = "myth2-rust-dev";

          nativeBuildInputs = with pkgs; [
            rustToolchain
            pkg-config
            cmake
            nasm
            wasm-bindgen-cli_0_2_129
            binaryen
            cargo-deny
            vulkan-tools
            zig
            cargo-zigbuild
          ];

          buildInputs = runtimeLibs;

          LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath runtimeLibs}:/usr/lib:/usr/lib64";

          MYTH2_MESA_ICD_DIR = "${pkgs.mesa}/share/vulkan/icd.d";

          shellHook = ''
            export MYTH2_DATA_DIR="''${MYTH2_DATA_DIR:-$HOME/Myth II}"
            if [ -z "''${VK_ICD_FILENAMES:-}" ] && [ -z "''${VK_DRIVER_FILES:-}" ] \
               && [ -z "''${VK_ADD_DRIVER_FILES:-}" ]; then
              VK_ADD_DRIVER_FILES=""
              for icd in "$MYTH2_MESA_ICD_DIR"/*.json; do
                case "$(basename "$icd")" in dzn_icd.*) continue ;; esac
                VK_ADD_DRIVER_FILES="''${VK_ADD_DRIVER_FILES:+$VK_ADD_DRIVER_FILES:}$icd"
              done
              export VK_ADD_DRIVER_FILES
              unset icd
            fi
            echo "Myth II source port dev shell: $(rustc --version); game data: $MYTH2_DATA_DIR"
          '';
        };
      }
    );
}
