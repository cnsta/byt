self:
{
  config,
  pkgs,
  lib,
  ...
}:
let
  inherit (lib.options) mkEnableOption mkPackageOption;
  inherit (lib) mkIf mkMerge;
  cfg = config.hardware.lightcrazy;
in
{
  options.hardware.lightcrazy = {
    enable = mkEnableOption "LightCrazy — installs the package and udev rules for the Pulsar X2 CrazyLight";

    package = mkPackageOption pkgs "lightcrazy" { } // {
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.lightcrazy;
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
  };
}
