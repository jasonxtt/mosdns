#!/bin/sh
set -eu

if [ "${MOSDNS_CONTAINER_RESOLV_CONF_AUTO:-1}" = "1" ] && [ -w /etc/resolv.conf ]; then
	nameservers="$(awk '$1 == "nameserver" { print $2 }' /etc/resolv.conf)"
	if [ "$nameservers" = "8.8.8.8
8.8.4.4" ] || [ "$nameservers" = "8.8.8.8" ] || [ "$nameservers" = "8.8.4.4" ]; then
		tmp_file="$(mktemp)"
		{
			echo "nameserver 223.5.5.5"
			echo "nameserver 114.114.114.114"
		} > "$tmp_file"
		cat "$tmp_file" > /etc/resolv.conf
		rm -f "$tmp_file"
	fi
fi

exec /usr/bin/mosdns "$@"
