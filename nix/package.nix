{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,

  libxkbcommon,
  wayland,
  vulkan-loader,
  libGL,
  fontconfig,
  freetype,
  libX11,
  libXcursor,
  libXi,
  libXrandr,

  rev ? "dirty",
}:

let
  cargoToml = lib.importTOML ../Cargo.toml;

  runtimeLibs = [
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

rustPlatform.buildRustPackage {
  pname = "byt";
  version = "${cargoToml.package.version}-${rev}";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../src
      ../Cargo.lock
      ../Cargo.toml
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  strictDeps = true;

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs = runtimeLibs;

  postFixup = ''
    wrapProgram $out/bin/byt \
      --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibs}"
  '';

  meta = {
    description = "Simple VPN switcher for Linux (NetworkManager + Tailscale)";
    longDescription = ''
      byt is a small Linux app for switching between NetworkManager-managed
      VPN connections (WireGuard and OpenVPN) and Tailscale. Provides a
      keyboard-driven iced GUI and a CLI (`byt status`, `byt import`) for
      scripting.
    '';
    homepage = "https://github.com/cnsta/byt";
    license = lib.licenses.mit;
    mainProgram = "byt";
    platforms = lib.platforms.linux;
  };
}
