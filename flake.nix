{
  description = "byt: basic vpn switcher";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs?ref=nixos-unstable";

  outputs =
    {
      self,
      nixpkgs,
      ...
    }:
    let
      systems = nixpkgs.lib.systems.flakeExposed;
      forEachSystem = nixpkgs.lib.attrsets.genAttrs systems;
      pkgsFor = nixpkgs.legacyPackages;
    in
    {
      devShells = forEachSystem (system: {
        default =
          let
            pkgs = pkgsFor.${system};
          in
          pkgs.mkShell {
            env.RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

            nativeBuildInputs = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer-unwrapped
              pkg-config
              python3
            ];

            buildInputs = with pkgs; [
              udev
              libusb1
              dbus
              glib
              gobject-introspection
            ];

            shellHook = ''
              echo "🦀 byt dev shell"
            '';
          };
      });

      packages = forEachSystem (system: {
        byt = pkgsFor.${system}.callPackage ./nix/package.nix {
          rev = self.rev or "dirty";
        };
        default = self.packages.${system}.byt;
      });

      nixosModules = {
        byt = import ./nix/nixosModule.nix self;
        default = self.nixosModules.byt;
      };
    };
}
