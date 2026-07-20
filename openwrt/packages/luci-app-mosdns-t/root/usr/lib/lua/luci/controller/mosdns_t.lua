module("luci.controller.mosdns_t", package.seeall)

function index()
	local fs = require "nixio.fs"
	local dispatcher = fs.readfile("/usr/lib/lua/luci/dispatcher.lua") or ""

	if fs.access("/usr/share/ucode/luci/dispatcher.uc") then
		return
	end

	if dispatcher:find("menu.d", 1, true) then
		return
	end

	local page = entry({"admin", "services", "mosdns-t"}, cbi("mosdns_t/overview"), _("MosDNS-T"), 30)
	page.dependent = true
	page.acl_depends = { "luci-app-mosdns-t" }
end
