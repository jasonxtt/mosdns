#!/bin/sh
set -eu

BASE_URL="${MOSDNS_T_REPO_URL:-https://jasonxtt.github.io/mosdns}"
if [ "$(id -u)" -ne 0 ]; then
	echo "请使用 root 用户运行此脚本。" >&2
	exit 1
fi

install_apk() {
	arch="$(apk --print-arch)"
	repository_url="${BASE_URL}/packages/25.12/${arch}/packages.adb"
	key_file='/etc/apk/keys/mosdns-t.pem'
	repository_file='/etc/apk/repositories.d/mosdns-t.list'

	mkdir -p /etc/apk/keys /etc/apk/repositories.d
	wget -qO "${key_file}.tmp" "${BASE_URL}/keys/mosdns-t.pem"
	mv "${key_file}.tmp" "$key_file"
	printf '%s\n' "$repository_url" > "$repository_file"

	apk update
	apk add --upgrade mosdns-t luci-app-mosdns-t
}

install_opkg() {
	arch="$(opkg print-architecture | awk '$1 == "arch" && $3 + 0 >= priority { arch = $2; priority = $3 + 0 } END { print arch }')"
	[ -n "$arch" ] || {
		echo '无法识别 OpenWrt 软件包架构。' >&2
		exit 1
	}
	repository_url="${BASE_URL}/packages/24.10/${arch}"
	key_file='/etc/opkg/keys/e876ba860888cd76'
	custom_feeds='/etc/opkg/customfeeds.conf'

	mkdir -p /etc/opkg/keys
	wget -qO "${key_file}.tmp" "${BASE_URL}/keys/mosdns-t.pub"
	mv "${key_file}.tmp" "$key_file"
	[ -f "$custom_feeds" ] || : > "$custom_feeds"
	sed -i '/^src\/gz mosdns_t /d' "$custom_feeds"
	printf 'src/gz mosdns_t %s\n' "$repository_url" >> "$custom_feeds"

	opkg update
	opkg install mosdns-t luci-app-mosdns-t
}

if command -v apk >/dev/null 2>&1; then
	install_apk
elif command -v opkg >/dev/null 2>&1; then
	install_opkg
else
	echo '当前系统未找到 apk 或 opkg，无法安装 MosDNS-T。' >&2
	exit 1
fi
/etc/init.d/mosdns-t enable
if /etc/init.d/mosdns-t running; then
	/etc/init.d/mosdns-t restart
else
	/etc/init.d/mosdns-t start
fi

echo "MosDNS-T 安装完成。LuCI 入口：服务 -> MosDNS-T"
