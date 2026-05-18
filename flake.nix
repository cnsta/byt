{
  description = "byt: simple VPN switcher for Linux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?ref=nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forEachSystem = nixpkgs.lib.genAttrs systems;

      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

      rustToolchainFor =
        pkgs:
        pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
        };

      runtimeLibsFor =
        pkgs: with pkgs; [
          libxkbcommon
          wayland
          vulkan-loader
          libGL
          fontconfig
          freetype
          libX11
          libXcursor
          libXi
          libXrandr
        ];
    in
    {
      devShells = forEachSystem (
        system:
        let
          pkgs = pkgsFor system;
          rustToolchain = rustToolchainFor pkgs;
          runtimeLibs = runtimeLibsFor pkgs;
        in
        {
          default = pkgs.mkShell {
            packages = [
              rustToolchain
              pkgs.pkg-config
            ];
            buildInputs = runtimeLibs;

            env = {
              RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
              LD_LIBRARY_PATH = nixpkgs.lib.makeLibraryPath runtimeLibs;
            };

            shellHook = ''
              echo "🦀 byt dev shell — $(rustc --version)"
            '';
          };
        }
      );

      packages = forEachSystem (
        system:
        let
          pkgs = pkgsFor system;
          rustToolchain = rustToolchainFor pkgs;
        in
        {
          byt = pkgs.callPackage ./nix/package.nix {
            rev = self.rev or "dirty";
            rustPlatform = pkgs.makeRustPlatform {
              cargo = rustToolchain;
              rustc = rustToolchain;
            };
          };
          default = self.packages.${system}.byt;
        }
      );

      apps = forEachSystem (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.byt}/bin/byt";
        };
      });

      nixosModules = {
        byt = import ./nix/nixosModule.nix self;
        default = self.nixosModules.byt;
      };

      formatter = forEachSystem (system: (pkgsFor system).nixfmt-rfc-style);
    };
}
