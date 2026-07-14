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
wget -qO- https://jasonxtt.github.io/mosdns/install.sh | sh
```

The installer detects `apk` or `opkg`, adds the matching MosDNS-T public
signing key and only installs or upgrades `mosdns-t` and
`luci-app-mosdns-t`.

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
