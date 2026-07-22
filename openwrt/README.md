# MosDNS-T for OpenWrt

This branch builds signed packages and publishes them as a GitHub Pages
repository:

- APK for OpenWrt/ImmortalWrt 25.12
- IPK for OpenWrt 24.10

## Supported package architectures

- `x86_64`
- `aarch64_cortex-a53` (MediaTek Filogic)
- `aarch64_generic` (Rockchip ARMv8)

## Install

```sh
install_script=/tmp/mosdns-t-install.sh
rm -f "$install_script"
for url in \
  https://jasonxtt.github.io/mosdns/install.sh \
  https://cdn.jsdelivr.net/gh/jasonxtt/mosdns@openwrt/openwrt/repository/install.sh \
  https://ghproxy.net/https://raw.githubusercontent.com/jasonxtt/mosdns/openwrt/openwrt/repository/install.sh; do
  wget -q -T 15 -O "$install_script" "$url" && break
done
[ -s "$install_script" ] && sh "$install_script"
```

The installer detects `apk` or `opkg`, adds the matching MosDNS-T public
signing key and only installs or upgrades `mosdns-t` and
`luci-app-mosdns-t`. It selects the first reachable repository from GitHub
Pages, the `openwrt-feed` raw mirror, ghproxy and jsDelivr. LuCI repeats this
selection before every package check or upgrade.

When DNSMasq forwarding is disabled, MosDNS-T listens on all addresses at the
configured DNS port. For example, setting the port to `5336` permits both
`dig @127.0.0.1 -p 5336 example.com` and LAN queries to
`dig @router-address -p 5336 example.com`. With forwarding enabled, the same
port is limited to `127.0.0.1` and dnsmasq keeps port 53.

## Release maintenance

1. Merge the matching main release into `openwrt`.
2. Update `PKG_VERSION` and `PKG_RELEASE` in both package Makefiles.
3. Push `openwrt`; GitHub Actions rebuilds and republishes both signed feeds.
4. Tag `openwrt-v<version>-r<release>` to also publish the APK files as a
   GitHub release.

The signing keys are read from repository secrets `OPENWRT_APK_SIGNING_KEY`
and `OPENWRT_IPK_SIGNING_KEY`. Their public halves are committed as
`openwrt/packages/mosdns-t/files/mosdns-t.pem` and
`openwrt/repository/mosdns-t.pub`.
