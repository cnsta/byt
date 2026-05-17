{
  lib,
  udev,
  pkg-config,
  rustPlatform,
  dbus,
  makeWrapper,
  rev ? "dirty",
}:
let
  cargoToml = lib.importTOML ../Cargo.toml;
  runtimeDeps = [
    udev
    dbus
  ];
in
rustPlatform.buildRustPackage {
  pname = "byt";
  version = "${cargoToml.package.version}-${rev}";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../src
      ../assets
      ../build.rs
      ../Cargo.lock
      ../Cargo.toml
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;
  strictDeps = true;

  nativeBuildInputs = [
    pkg-config
    rustPlatform.bindgenHook
    makeWrapper
  ];

  buildInputs = runtimeDeps;

  postInstall = ''
    for bin in $out/bin/*; do
      wrapProgram $bin \
        --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeDeps}"
    done
  '';

  meta = {
    description = "byt: basic vpn switcher";
    longDescription = ''
      Somethingsomethingvpnswitcher
    '';
    homepage = "https://github.com/cnsta/byt";
    license = lib.licenses.mit;
    mainProgram = "byt";
    platforms = lib.platforms.linux;
  };
}
