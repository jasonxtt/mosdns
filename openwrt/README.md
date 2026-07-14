# MosDNS-T for OpenWrt

This branch builds signed APK packages for OpenWrt/ImmortalWrt 25.12 and
publishes them as a GitHub Pages repository.

## Supported package architectures

- `x86_64`
- `aarch64_cortex-a53` (MediaTek Filogic)
- `aarch64_generic` (Rockchip ARMv8)

## Install

```sh
wget -qO- https://jasonxtt.github.io/mosdns/install.sh | sh
```

The installer adds the MosDNS-T public signing key and only installs or
upgrades `mosdns-t` and `luci-app-mosdns-t`.

## Release maintenance

1. Merge the matching main release into `openwrt`.
2. Update `PKG_VERSION` and `PKG_RELEASE` in both package Makefiles.
3. Push `openwrt`; GitHub Actions rebuilds and republishes the signed feed.
4. Tag `openwrt-v<version>-r<release>` to also publish the APK files as a
   GitHub release.

The signing key is read from the repository secret
`OPENWRT_APK_SIGNING_KEY`. Its public half is committed as
`openwrt/packages/mosdns-t/files/mosdns-t.pem`.
