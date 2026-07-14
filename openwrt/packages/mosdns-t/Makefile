include $(TOPDIR)/rules.mk

PKG_NAME:=mosdns-t
PKG_VERSION:=0.7.0
PKG_RELEASE:=4
PKG_BUILD_DIR:=$(BUILD_DIR)/$(PKG_NAME)-$(PKG_VERSION)

CONFIG_COMMIT:=30a5e74d23a5d35ac4adc330382f4522d373755c
CONFIG_ARCHIVE:=mosdns-config-all-$(CONFIG_COMMIT).zip
CONFIG_HASH:=75ade81d74135a2bd02ad850c196d7558a2fc5fb3c476032fda34c16efb1942c

PKG_LICENSE:=GPL-3.0-only
PKG_MAINTAINER:=jasonxtt

include $(INCLUDE_DIR)/package.mk

define Download/mosdns-config
	URL:=https://raw.githubusercontent.com/jasonxtt/file/$(CONFIG_COMMIT)/mosdns/config/
	URL_FILE:=config_all.zip
	FILE:=$(CONFIG_ARCHIVE)
	HASH:=$(CONFIG_HASH)
endef

define Package/mosdns-t
	SECTION:=net
	CATEGORY:=Network
	SUBMENU:=IP Addresses and Names
	TITLE:=MosDNS-T with embedded WebUI
	URL:=https://github.com/jasonxtt/mosdns
	DEPENDS:=+ca-bundle
endef

define Package/mosdns-t/description
	MosDNS-T packaged for OpenWrt with procd, dnsmasq integration and an
	embedded WebUI. Program upgrades are managed through LuCI or apk.
endef

define Package/mosdns-t/conffiles
/etc/config/mosdns-t
/etc/mosdns-t/config_custom.yaml
/etc/mosdns-t/rule/
/etc/mosdns-t/webinfo/
/etc/apk/repositories.d/mosdns-t.list
endef

define Build/Prepare
	rm -rf $(PKG_BUILD_DIR)
	$(INSTALL_DIR) $(PKG_BUILD_DIR)/config
	$(eval $(call Download,mosdns-config))
	unzip -q $(DL_DIR)/$(CONFIG_ARCHIVE) -d $(PKG_BUILD_DIR)/config
	find $(PKG_BUILD_DIR)/config -name .DS_Store -delete
	rm -rf $(PKG_BUILD_DIR)/config/ui
endef

define Build/Compile
endef

define Package/mosdns-t/install
	$(INSTALL_DIR) $(1)/usr/bin
	$(INSTALL_BIN) ./files/mosdns-t $(1)/usr/bin/mosdns-t

	$(INSTALL_DIR) $(1)/etc/mosdns-t
	$(CP) $(PKG_BUILD_DIR)/config/. $(1)/etc/mosdns-t/
	$(SED) 's/listen: ":53"/listen: "127.0.0.1:5335"/' $(1)/etc/mosdns-t/config_custom.yaml
	$(SED) 's|https://1.1.1.1/dns-query|223.5.5.5|; s|https://8.8.8.8/dns-query|223.6.6.6|' $(1)/etc/mosdns-t/sub_config/forward_nocn.yaml
	echo B > $(1)/etc/mosdns-t/rule/switch17.txt
	$(INSTALL_DATA) ./files/config_update_state.json $(1)/etc/mosdns-t/webinfo/config_update_state.json

	$(INSTALL_DIR) $(1)/etc/config
	$(INSTALL_CONF) ./files/mosdns-t.config $(1)/etc/config/mosdns-t

	$(INSTALL_DIR) $(1)/etc/init.d
	$(INSTALL_BIN) ./files/mosdns-t.init $(1)/etc/init.d/mosdns-t

	$(INSTALL_DIR) $(1)/usr/libexec
	$(INSTALL_BIN) ./files/mosdns-t-dnsmasq $(1)/usr/libexec/mosdns-t-dnsmasq

	$(INSTALL_DIR) $(1)/etc/apk/keys
	$(INSTALL_DATA) ./files/mosdns-t.pem $(1)/etc/apk/keys/mosdns-t.pem
	$(INSTALL_DIR) $(1)/etc/apk/repositories.d
	$(INSTALL_CONF) ./files/mosdns-t.list $(1)/etc/apk/repositories.d/mosdns-t.list
endef

define Package/mosdns-t/postinst
#!/bin/sh
[ -n "$${IPKG_INSTROOT}" ] || {
	sed -i "s|@ARCH@|$$(apk --print-arch)|g" /etc/apk/repositories.d/mosdns-t.list
	/etc/init.d/mosdns-t enable
}
exit 0
endef

$(eval $(call BuildPackage,mosdns-t))
