{
  description = "Donut Browser development environment and quick-start commands";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfree = true;
        };
        lib = pkgs.lib;

        nodejs =
          if pkgs ? nodejs_23 then
            pkgs.nodejs_23
          else
            pkgs.nodejs_22;

        rustPackages = with pkgs; [
          cargo
          clippy
          rust-analyzer
          rustc
          rustfmt
        ];

        commonLibs = with pkgs; [
          webkitgtk_4_1
          libsoup_3
          glib
          gtk3
          cairo
          gdk-pixbuf
          pango
          atk
          at-spi2-atk
          at-spi2-core
          dbus
          nss
          nspr
          libdrm
          libgbm
          libxkbcommon
          libx11
          libxcomposite
          libxdamage
          libxext
          libxfixes
          libxrandr
          libxcb
          libxshmfence
          libxtst
          libxi
          xdotool
          libxrender
          libxinerama
          libxcursor
          libxscrnsaver
          fontconfig
          freetype
          fribidi
          harfbuzz
          expat
          libglvnd
          libgpg-error
          e2fsprogs
          gmp
          zlib
          stdenv.cc.cc.lib
        ];

        runtimeLibPath = lib.makeLibraryPath commonLibs;
        nixLd = pkgs.stdenv.cc.bintools.dynamicLinker;
        pkgConfigLibs = [
          pkgs.at-spi2-atk
          pkgs.at-spi2-core
          pkgs.cairo
          pkgs.dbus
          pkgs.gdk-pixbuf
          pkgs.glib
          pkgs.gtk3
          pkgs.libsoup_3
          pkgs.libxkbcommon
          pkgs.openssl
          pkgs.pango
          pkgs.harfbuzz
          pkgs.webkitgtk_4_1
        ];
        pkgConfigPath = lib.makeSearchPath "lib/pkgconfig" (
          pkgConfigLibs ++ map lib.getDev pkgConfigLibs
        );
        releaseVersion = "0.22.6";
        releaseAppImage =
          if system == "x86_64-linux" then
            pkgs.fetchurl {
              url = "https://github.com/zhom/donutbrowser/releases/download/v0.22.6/Donut_0.22.6_amd64.AppImage";
              hash = "sha256-sbYM8YKfQznGDl7kCJFDH2Ak+q//vYuHM6loXHckOAs=";
            }
          else if system == "aarch64-linux" then
            pkgs.fetchurl {
              url = "https://github.com/zhom/donutbrowser/releases/download/v0.22.6/Donut_0.22.6_aarch64.AppImage";
              hash = "sha256-piMZR+ZxOyaxz6lom6aRZDyuU5fsu3kJFbOSsS5YuKI=";
            }
          else
            null;
        releaseUnpacked =
          if releaseAppImage != null then
            pkgs.stdenvNoCC.mkDerivation {
              pname = "donut-release-unpacked";
              version = releaseVersion;
              src = releaseAppImage;
              dontUnpack = true;
              nativeBuildInputs = [ pkgs.xz ];
              installPhase = ''
                runHook preInstall

                cp "$src" ./donut.AppImage
                chmod +x ./donut.AppImage
                ./donut.AppImage --appimage-extract >/dev/null

                mkdir -p "$out"
                cp -a ./squashfs-root "$out/"

                runHook postInstall
              '';
            }
          else
            null;
        releaseWrapped =
          if releaseAppImage != null then
            pkgs.appimageTools.wrapType2 {
              pname = "donut";
              version = releaseVersion;
              src = releaseAppImage;
              extraPkgs = _: commonLibs;
              extraInstallCommands = ''
                for bin in "$out"/bin/*; do
                  if [ -f "$bin" ]; then
                    mv "$bin" "$out/bin/donut-release"
                    break
                  fi
                done
              '';
            }
          else
            null;
        releaseLauncher =
          if releaseUnpacked != null then
            pkgs.writeShellApplication {
              name = "donut-release-start";
              runtimeInputs = with pkgs; [
                coreutils
                xdg-utils
              ];
              text = ''
                set -euo pipefail

                if [ -x "${releaseWrapped}/bin/donut-release" ]; then
                  if "${releaseWrapped}/bin/donut-release" "$@"; then
                    exit 0
                  fi
                  echo "Wrapped AppImage failed, retrying with direct AppRun..." >&2
                fi

                export LD_LIBRARY_PATH="${releaseUnpacked}/squashfs-root/usr/lib:${releaseUnpacked}/squashfs-root/usr/lib64:${runtimeLibPath}:''${LD_LIBRARY_PATH:-}"
                export NIX_LD_LIBRARY_PATH="$LD_LIBRARY_PATH"
                export LIBRARY_PATH="$LD_LIBRARY_PATH"
                export XDG_DATA_DIRS="${releaseUnpacked}/squashfs-root/usr/share:''${XDG_DATA_DIRS:-}"
                exec "${releaseUnpacked}/squashfs-root/AppRun" "$@"
              '';
            }
          else
            pkgs.writeShellApplication {
              name = "donut-release-start";
              text = ''
                echo "Release launcher is supported only on Linux (x86_64/aarch64)."
                exit 1
              '';
            };

        mkApp = name: text:
          let
            app = pkgs.writeShellApplication {
              inherit name;
              runtimeInputs = with pkgs; [
                bash
                coreutils
                findutils
                git
                gnugrep
                gnused
                curl
                gcc
                pkg-config
                openssl
                cargo
                clippy
                rustc
                rustfmt
                nodejs
                pnpm
                cargo-tauri
              ];
              text = ''
                export NODE_ENV=development
                export NIX_LD="${nixLd}"
                export NIX_LD_LIBRARY_PATH="${runtimeLibPath}:''${NIX_LD_LIBRARY_PATH:-}"
                export LD_LIBRARY_PATH="${runtimeLibPath}:''${LD_LIBRARY_PATH:-}"
                export LIBRARY_PATH="${runtimeLibPath}:''${LIBRARY_PATH:-}"
                export PKG_CONFIG_PATH="${pkgConfigPath}:''${PKG_CONFIG_PATH:-}"
                export RUST_SRC_PATH="${pkgs.rustPlatform.rustLibSrc}"
                ${text}
              '';
            };
          in
          {
            type = "app";
            program = "${app}/bin/${name}";
          };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            nodejs
            pnpm
            cargo-tauri
            pkg-config
            openssl
            git
            bashInteractive
            gnumake
            clang
            llvmPackages.bintools
            python3
            curl
            wget
            unzip
            zip
            xz
            biome
            docker
          ] ++ rustPackages ++ commonLibs;

          shellHook = ''
            export NODE_ENV=development
            export NIX_LD="${nixLd}"
            export NIX_LD_LIBRARY_PATH="${runtimeLibPath}:''${NIX_LD_LIBRARY_PATH:-}"
            export LD_LIBRARY_PATH="${runtimeLibPath}:''${LD_LIBRARY_PATH:-}"
            export LIBRARY_PATH="${runtimeLibPath}:''${LIBRARY_PATH:-}"
            export PKG_CONFIG_PATH="${pkgConfigPath}:''${PKG_CONFIG_PATH:-}"
            export RUST_SRC_PATH="${pkgs.rustPlatform.rustLibSrc}"
            export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share:${pkgs.gtk3}/share:''${XDG_DATA_DIRS:-}"

            echo "Donut Browser dev shell ready."
            echo "Quick start:"
            echo "  nix run .#setup"
            echo "  nix run .#tauri-dev"
            echo "  nix run .#full-dev"
            echo "  nix run .#build"
            echo "  nix run .#test"
            echo "  nix run .#release-start"
          '';
        };

        apps.info = mkApp "donut-info" ''
          set -euo pipefail
          echo "Node: $(node --version)"
          echo "pnpm: $(pnpm --version)"
          echo "Rust: $(rustc --version)"
          echo "Cargo: $(cargo --version)"
          echo "Tauri CLI: $(cargo-tauri --version)"
        '';

        apps.deps = mkApp "donut-deps" ''
          set -euo pipefail
          pnpm install
        '';

        apps.dev = mkApp "donut-dev" ''
          set -euo pipefail
          pnpm dev
        '';

        apps."tauri-dev" = mkApp "donut-tauri-dev" ''
          set -euo pipefail
          pnpm tauri dev
        '';

        apps."full-dev" = mkApp "donut-full-dev" ''
          set -euo pipefail
          chmod +x ./scripts/dev.sh
          ./scripts/dev.sh
        '';

        apps.build = mkApp "donut-build" ''
          set -euo pipefail
          pnpm build
          (cd src-tauri && cargo build)
        '';

        apps.start = mkApp "donut-start" ''
          set -euo pipefail
          pnpm start
        '';

        apps.test = mkApp "donut-test" ''
          set -euo pipefail
          pnpm format && pnpm lint && pnpm test
        '';

        apps.setup = mkApp "donut-setup" ''
          set -euo pipefail

          if [ ! -f "package.json" ]; then
            echo "package.json not found. Run this from the donutbrowser repo root."
            exit 1
          fi

          pnpm install
          pnpm copy-proxy-binary

          echo "Setup complete."
          echo "Run the app with:"
          echo "  nix run .#tauri-dev"
          echo "Or run full local stack (sync + minio + tauri):"
          echo "  nix run .#full-dev"
        '';

        apps."release-start" = {
          type = "app";
          program = "${releaseLauncher}/bin/donut-release-start";
        };

        apps.default = self.apps.${system}.setup;
      });
}
