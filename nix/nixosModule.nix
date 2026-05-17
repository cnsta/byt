self:
{
  config,
  pkgs,
  lib,
  ...
}:

let
  inherit (lib) mkIf mkDefault types;
  inherit (lib.options) mkEnableOption mkPackageOption mkOption;

  cfg = config.programs.byt;
in
{
  options.programs.byt = {
    enable = mkEnableOption "byt VPN switcher";

    package = mkPackageOption pkgs "byt" { } // {
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.byt;
    };

    openvpn = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Install NetworkManager's OpenVPN plugin so byt can activate
        imported `.ovpn` configurations. Without it, OpenVPN imports succeed
        but activation fails at runtime with "VPN plugin not found".
      '';
    };

    tailscale = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Enable `services.tailscale` so the Tailscale row in byt works.
        Leave off if you don't use Tailscale — byt will show the row as
        "unavailable" and the rest of the app keeps working.
      '';
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    networking.networkmanager.enable = mkDefault true;

    networking.networkmanager.plugins = mkIf cfg.openvpn [
      pkgs.networkmanager-openvpn
    ];

    services.tailscale.enable = mkIf cfg.tailscale true;
  };
}
