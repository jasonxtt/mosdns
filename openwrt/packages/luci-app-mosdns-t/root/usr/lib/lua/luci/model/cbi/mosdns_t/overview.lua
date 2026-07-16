local fs = require "nixio.fs"
local sys = require "luci.sys"
local http = require "luci.http"
local uci = require "luci.model.uci".cursor()

local function init_action(action)
	return os.execute("/etc/init.d/mosdns-t " .. action .. " >/dev/null 2>&1")
end

local function package_version(name)
	local version = sys.exec("opkg list-installed " .. name .. " 2>/dev/null | awk 'NR == 1 { print $3 }'")
	return version:gsub("%s+$", "")
end

local running = init_action("running") == 0
local enabled = init_action("enabled") == 0
local webui_port = uci:get("mosdns-t", "main", "webui_port") or "9099"
local host = http.getenv("HTTP_HOST") or http.getenv("SERVER_NAME") or "127.0.0.1"
host = host:gsub(":%d+$", "")

local m = Map("mosdns-t", translate("MosDNS-T"), translate("MosDNS-T service controls and OpenWrt integration."))
m.apply_on_parse = true

local s = m:section(NamedSection, "main", "mosdns-t", translate("Status"))
s.anonymous = true

local status = s:option(DummyValue, "_status", translate("Service status"))
status.rawhtml = true
function status.cfgvalue()
	if running then
		return "<span style=\"color:green\">" .. translate("Running") .. "</span>"
	end
	return "<span style=\"color:red\">" .. translate("Stopped") .. "</span>"
end

local autostart = s:option(DummyValue, "_autostart", translate("Autostart"))
function autostart.cfgvalue()
	return enabled and translate("Enabled") or translate("Disabled")
end

local core_version = s:option(DummyValue, "_core_version", translate("Core package"))
function core_version.cfgvalue()
	return package_version("mosdns-t")
end

local luci_version = s:option(DummyValue, "_luci_version", translate("LuCI package"))
function luci_version.cfgvalue()
	return package_version("luci-app-mosdns-t")
end

local controls = m:section(TypedSection, "_controls", translate("Service controls"))
controls.anonymous = true
controls.addremove = false
function controls.cfgsections()
	return { "_controls" }
end

local start = controls:option(Button, "_start", translate("Start"))
start.inputstyle = "apply"
function start.write()
	init_action("start")
	http.redirect(http.getenv("REQUEST_URI"))
end

local stop = controls:option(Button, "_stop", translate("Stop"))
stop.inputstyle = "reset"
function stop.write()
	init_action("stop")
	http.redirect(http.getenv("REQUEST_URI"))
end

local restart = controls:option(Button, "_restart", translate("Restart"))
restart.inputstyle = "reload"
function restart.write()
	init_action("restart")
	http.redirect(http.getenv("REQUEST_URI"))
end

local enable = controls:option(Button, "_enable", translate("Enable autostart"))
enable.inputstyle = "apply"
function enable.write()
	init_action("enable")
	http.redirect(http.getenv("REQUEST_URI"))
end

local disable = controls:option(Button, "_disable", translate("Disable autostart"))
disable.inputstyle = "reset"
function disable.write()
	init_action("disable")
	http.redirect(http.getenv("REQUEST_URI"))
end

local open_webui = controls:option(Button, "_open_webui", translate("Open WebUI"))
open_webui.inputstyle = "find"
function open_webui.write()
	http.redirect("http://" .. host .. ":" .. webui_port .. "/")
end

local settings = m:section(NamedSection, "main", "mosdns-t", translate("Basic settings"))
settings.anonymous = true

local enabled_flag = settings:option(Flag, "enabled", translate("Enable"))
enabled_flag.default = "1"
enabled_flag.rmempty = false

local dnsmasq_forward = settings:option(Flag, "dnsmasq_forward", translate("DNSMasq forward"))
dnsmasq_forward.default = "1"
dnsmasq_forward.rmempty = false

local listen_port = settings:option(Value, "listen_port", translate("DNS listen port"))
listen_port.datatype = "port"
listen_port.placeholder = "5335"

local web_port = settings:option(Value, "webui_port", translate("WebUI port"))
web_port.datatype = "port"
web_port.placeholder = "9099"

local config_file = settings:option(Value, "configfile", translate("Config file"))
config_file.default = "/etc/mosdns-t/config_custom.yaml"
config_file.rmempty = false

local workdir = settings:option(Value, "workdir", translate("Working directory"))
workdir.default = "/etc/mosdns-t"
workdir.rmempty = false

function m.on_after_commit()
	if fs.access("/etc/init.d/mosdns-t") then
		init_action("restart")
	end
end

return m
