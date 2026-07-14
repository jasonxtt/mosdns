#!/bin/sh
set -eu

BASE_URL="${MOSDNS_T_REPO_URL:-https://jasonxtt.github.io/mosdns}"
OPENWRT_SERIES="25.12"
KEY_FILE="/etc/apk/keys/mosdns-t.pem"
REPOSITORY_FILE="/etc/apk/repositories.d/mosdns-t.list"

if [ "$(id -u)" -ne 0 ]; then
	echo "请使用 root 用户运行此脚本。" >&2
	exit 1
fi

if ! command -v apk >/dev/null 2>&1; then
	echo "当前系统不是使用 apk 的 OpenWrt/ImmortalWrt 25.12。" >&2
	exit 1
fi

ARCH="$(apk --print-arch)"
REPOSITORY_URL="${BASE_URL}/packages/${OPENWRT_SERIES}/${ARCH}/packages.adb"

mkdir -p /etc/apk/keys /etc/apk/repositories.d
wget -qO "${KEY_FILE}.tmp" "${BASE_URL}/keys/mosdns-t.pem"
mv "${KEY_FILE}.tmp" "$KEY_FILE"
printf '%s\n' "$REPOSITORY_URL" > "$REPOSITORY_FILE"

apk update
apk add --upgrade mosdns-t luci-app-mosdns-t
/etc/init.d/mosdns-t enable
if /etc/init.d/mosdns-t running; then
	/etc/init.d/mosdns-t restart
else
	/etc/init.d/mosdns-t start
fi

echo "MosDNS-T 安装完成。LuCI 入口：服务 -> MosDNS-T"
