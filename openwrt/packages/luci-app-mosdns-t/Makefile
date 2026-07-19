include $(TOPDIR)/rules.mk

PKG_NAME:=luci-app-mosdns-t
PKG_VERSION:=0.7.1
PKG_RELEASE:=1
PKGARCH:=all

PKG_LICENSE:=GPL-3.0-only
PKG_MAINTAINER:=jasonxtt

include $(INCLUDE_DIR)/package.mk

define Package/luci-app-mosdns-t
	SECTION:=luci
	CATEGORY:=LuCI
	SUBMENU:=3. Applications
	TITLE:=LuCI support for MosDNS-T
	DEPENDS:=+luci-base +mosdns-t
endef

define Package/luci-app-mosdns-t/description
	LuCI service controls, targeted apk upgrades and WebUI launcher for MosDNS-T.
endef

define Build/Compile
endef

define Package/luci-app-mosdns-t/install
	$(INSTALL_DIR) $(1)/www/luci-static/resources/view/mosdns-t
	$(INSTALL_DATA) ./htdocs/luci-static/resources/view/mosdns-t/overview.js \
		$(1)/www/luci-static/resources/view/mosdns-t/overview.js
	$(INSTALL_DIR) $(1)/usr/lib/lua/luci/controller
	$(INSTALL_DATA) ./root/usr/lib/lua/luci/controller/mosdns_t.lua \
		$(1)/usr/lib/lua/luci/controller/mosdns_t.lua
	$(INSTALL_DIR) $(1)/usr/lib/lua/luci/model/cbi/mosdns_t
	$(INSTALL_DATA) ./root/usr/lib/lua/luci/model/cbi/mosdns_t/overview.lua \
		$(1)/usr/lib/lua/luci/model/cbi/mosdns_t/overview.lua
	$(INSTALL_DIR) $(1)/usr/share/luci/menu.d
	$(INSTALL_DATA) ./root/usr/share/luci/menu.d/luci-app-mosdns-t.json \
		$(1)/usr/share/luci/menu.d/luci-app-mosdns-t.json
	$(INSTALL_DIR) $(1)/usr/share/rpcd/acl.d
	$(INSTALL_DATA) ./root/usr/share/rpcd/acl.d/luci-app-mosdns-t.json \
		$(1)/usr/share/rpcd/acl.d/luci-app-mosdns-t.json
	$(INSTALL_DIR) $(1)/usr/share/ucitrack
	$(INSTALL_DATA) ./root/usr/share/ucitrack/luci-app-mosdns-t.json \
		$(1)/usr/share/ucitrack/luci-app-mosdns-t.json
	$(INSTALL_DIR) $(1)/usr/libexec
	$(INSTALL_BIN) ./root/usr/libexec/mosdns-t-updater $(1)/usr/libexec/mosdns-t-updater
	$(INSTALL_BIN) ./root/usr/libexec/mosdns-t-settings $(1)/usr/libexec/mosdns-t-settings
endef

$(eval $(call BuildPackage,luci-app-mosdns-t))
