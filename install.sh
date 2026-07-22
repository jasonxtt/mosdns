#!/bin/sh
set -eu

DEFAULT_REPO_URLS='https://jasonxtt.github.io/mosdns
https://raw.githubusercontent.com/jasonxtt/mosdns/openwrt-feed
https://ghproxy.net/https://raw.githubusercontent.com/jasonxtt/mosdns/openwrt-feed
https://cdn.jsdelivr.net/gh/jasonxtt/mosdns@openwrt-feed'
REPO_URLS="${MOSDNS_T_REPO_URLS:-${MOSDNS_T_REPO_URL:-$DEFAULT_REPO_URLS}}"
SELECTED_REPO_URL=''

select_repository() {
	index_path="$1"
	key_path="$2"
	key_file="$3"
	index_tmp="/tmp/mosdns-t-repository-index.$$"
	key_tmp="${key_file}.tmp"

	for base_url in $REPO_URLS; do
		rm -f "$index_tmp" "$key_tmp"
		if wget -q -T 15 -O "$index_tmp" "${base_url}/${index_path}" &&
			[ -s "$index_tmp" ] &&
			wget -q -T 15 -O "$key_tmp" "${base_url}/${key_path}" &&
			[ -s "$key_tmp" ]; then
			mv "$key_tmp" "$key_file"
			rm -f "$index_tmp"
			SELECTED_REPO_URL="$base_url"
			return 0
		fi
	done

	rm -f "$index_tmp" "$key_tmp"
	echo '所有 MosDNS-T 软件源均无法访问。' >&2
	return 1
}
if [ "$(id -u)" -ne 0 ]; then
	echo "请使用 root 用户运行此脚本。" >&2
	exit 1
fi

install_apk() {
	arch="$(apk --print-arch)"
	key_file='/etc/apk/keys/mosdns-t.pem'
	repository_file='/etc/apk/repositories.d/mosdns-t.list'

	mkdir -p /etc/apk/keys /etc/apk/repositories.d
	select_repository "packages/25.12/${arch}/packages.adb" 'keys/mosdns-t.pem' "$key_file"
	repository_url="${SELECTED_REPO_URL}/packages/25.12/${arch}/packages.adb"
	printf '%s\n' "$repository_url" > "$repository_file"

	apk update || echo '系统软件源更新失败，继续尝试安装 MosDNS-T。' >&2
	apk add --upgrade mosdns-t luci-app-mosdns-t
}

install_opkg() {
	arch="$(opkg print-architecture | awk '$1 == "arch" && $3 + 0 >= priority { arch = $2; priority = $3 + 0 } END { print arch }')"
	[ -n "$arch" ] || {
		echo '无法识别 OpenWrt 软件包架构。' >&2
		exit 1
	}
	key_file='/etc/opkg/keys/e876ba860888cd76'
	custom_feeds='/etc/opkg/customfeeds.conf'

	mkdir -p /etc/opkg/keys
	select_repository "packages/24.10/${arch}/Packages.gz" 'keys/mosdns-t.pub' "$key_file"
	repository_url="${SELECTED_REPO_URL}/packages/24.10/${arch}"
	[ -f "$custom_feeds" ] || : > "$custom_feeds"
	sed -i '/^src\/gz mosdns_t /d' "$custom_feeds"
	printf 'src/gz mosdns_t %s\n' "$repository_url" >> "$custom_feeds"

	opkg_update_log="/tmp/mosdns-t-opkg-update.$$"
	rm -f /var/opkg-lists/mosdns_t "$opkg_update_log"
	if ! opkg update > "$opkg_update_log" 2>&1; then
		cat "$opkg_update_log"
		if [ -s /var/opkg-lists/mosdns_t ]; then
			echo '系统软件源更新失败，但 MosDNS-T 软件源已更新，继续安装。' >&2
		else
			rm -f "$opkg_update_log"
			echo 'MosDNS-T 软件源更新失败。' >&2
			exit 1
		fi
	else
		cat "$opkg_update_log"
	fi
	rm -f "$opkg_update_log"
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
