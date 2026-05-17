# Example provider configs

Reference files showing the two formats `byt` can import. They use documentation
IP ranges (RFC 5737, RFC 3849) and placeholder keys/certs — they are **not
functional** as-is. Drop in your provider's real values to test, or use them
just to confirm the parser/preview output:

```sh
byt import doc/examples/wireguard-single.conf --name example-wg
```

| File                      | Kind      | Auth          | Notes                                                         |
| ------------------------- | --------- | ------------- | ------------------------------------------------------------- |
| `wireguard-single.conf`   | WireGuard | key pair      | typical single-hop config                                     |
| `wireguard-multihop.conf` | WireGuard | key pair      | identical format; routing differs server-side                 |
| `openvpn-userpass.ovpn`   | OpenVPN   | user/password | inline CA + tls-auth; `.ovpn` and `.conf` are the same format |
| `openvpn-cert.conf`       | OpenVPN   | client cert   | inline CA + cert + key                                        |

## After importing an OpenVPN user/password config

NetworkManager doesn't pick up `auth-user-pass` credentials from the file (the
file just declares that the protocol requires them). Set them after import with:

```sh
nmcli connection modify <name> vpn.user-name '<username>'
nmcli connection modify <name> +vpn.data password-flags=0
nmcli connection modify <name> vpn.secrets password='<password>'
```

`byt import` prints this reminder when it detects `auth-user-pass`.
