module("luci.controller.mosdns_t", package.seeall)

function index()
	local fs = require "nixio.fs"
	local dispatcher = fs.readfile("/usr/lib/lua/luci/dispatcher.lua") or ""

	if dispatcher:find("menu.d", 1, true) then
		return
	end

	local page = entry({"admin", "services", "mosdns-t"}, call("redirect_webui"), _("MosDNS-T"), 30)
	page.dependent = true
	page.acl_depends = { "luci-app-mosdns-t" }
end

function redirect_webui()
	local http = require "luci.http"
	local uci = require "luci.model.uci".cursor()
	local port = uci:get("mosdns-t", "main", "webui_port") or "9099"
	local host = http.getenv("HTTP_HOST") or http.getenv("SERVER_NAME") or "127.0.0.1"

	host = host:gsub(":%d+$", "")
	http.redirect("http://" .. host .. ":" .. port .. "/")
end
