<p align="center">
  <img src="assets/byt.svg" width="180" alt="byt">
</p>

<p align="center">
  Keyboard-driven VPN switcher for Linux.
</p>

---

`byt` swaps between NetworkManager-managed VPNs (WireGuard, OpenVPN) and
Tailscale from a small Iced GUI or the command line.

> [!WARNING]
> Very much expect bugs. I rarely use OpenVPN for example, so it's minimally
> tested. Feel free to contribute if you find anything wrong.

## Features

- Iced GUI with keyboard navigation in mind
- WireGuard and OpenVPN via NetworkManager (D-Bus, polkit-authenticated)
- Tailscale via systemd
- Mutual exclusion: activating one VPN deactivates the others
- Import provider configs: `byt import path/to/provider.conf`
- Live state via NetworkManager D-Bus signals, no polling
- Single-instance lock

## Requirements

- Linux with systemd
- NetworkManager
- For OpenVPN: the `NetworkManager-openvpn` plugin
- For Tailscale (optional): `tailscale` and `tailscaled`
- A running polkit authentication agent (most desktops include one)

## Usage

```sh
byt                  # open the GUI
byt status           # print current VPN state
byt import foo.conf
byt --help
```

### Keys

| Key                | Action         |
| ------------------ | -------------- |
| `↑` `↓` or `k` `j` | move selection |
| `x`                | disconnect     |
| `i`                | import config  |
| `d`                | delete config  |
| `r`                | refresh        |
| `q` or `Esc`       | quit           |

## Install

### Nix

```sh
nix run github:cnsta/byt
```

NixOS module:

```nix
# flake inputs
byt.url = "github:cnsta/byt";

# system config
imports = [ inputs.byt.nixosModules.default ];
programs.byt.enable = true;
```

### From source

```sh
cargo build --release
```

Sample configs in `doc/examples/` if you want to try the import flow without
real provider files.
