'use strict';
'require form';
'require fs';
'require poll';
'require rpc';
'require uci';
'require ui';
'require view';

const callServiceList = rpc.declare({
	object: 'service',
	method: 'list',
	params: [ 'name' ],
	expect: { '': {} }
});

function getStatus() {
	return L.resolveDefault(callServiceList('mosdns-t'), {}).then(res => {
		try {
			return !!res['mosdns-t'].instances.main.running;
		} catch (e) {
			return false;
		}
	});
}

function runAction(action) {
	return fs.exec('/etc/init.d/mosdns-t', [ action ]).then(() => {
		ui.addNotification(null, E('p', {}, _('操作已执行。')), 'info');
		window.setTimeout(() => window.location.reload(), 800);
	});
}

function getAutostart() {
	return L.resolveDefault(fs.exec('/etc/init.d/mosdns-t', [ 'enabled' ]), { code: 1 })
		.then(res => res.code === 0);
}

function parsePackageStatus(result) {
	const status = {};
	String(result && result.stdout || '').trim().split('\n').forEach(line => {
		const separator = line.indexOf('=');
		if (separator > 0)
			status[line.substring(0, separator)] = line.substring(separator + 1);
	});
	return status;
}

function getPackageStatus() {
	return L.resolveDefault(fs.exec('/usr/libexec/mosdns-t-updater', [ 'status' ]), {})
		.then(parsePackageStatus);
}

function runUpdater(action) {
	return fs.exec('/usr/libexec/mosdns-t-updater', [ action ]).then(result => {
		if (result.code !== 0)
			throw new Error(result.stderr || _('软件包操作失败'));
		ui.addNotification(null, E('p', {}, action === 'upgrade'
			? _('MosDNS-T 已升级并重启。')
			: _('软件源已刷新。')), 'info');
		window.setTimeout(() => window.location.reload(), 800);
	});
}

function writePort(sectionId, option, action, value) {
	return fs.exec('/usr/libexec/mosdns-t-settings', [ action, value ]).then(result => {
		if (result.code !== 0)
			throw new Error(result.stderr || _('端口设置失败'));
		uci.set('mosdns-t', sectionId, option, value);
	});
}

return view.extend({
	load() {
		return Promise.all([
			uci.load('mosdns-t'),
			getStatus(),
			L.resolveDefault(fs.exec('/usr/bin/mosdns-t', [ 'version' ]), {}),
			getAutostart(),
			getPackageStatus()
		]);
	},

	render(data) {
		const running = data[1];
		const version = data[2] && data[2].stdout ? data[2].stdout.trim() : '';
		const autostart = data[3];
		const packageStatus = data[4] || {};
		const updateAvailable = packageStatus.update_available === '1';
		const updateChecked = packageStatus.update_checked === '1';
		const webuiPort = packageStatus.webui_port || '9099';
		const dnsPort = packageStatus.dns_port || '5335';
		let m, s, o;
		const upgradeButton = E('button', {
			class: 'btn cbi-button cbi-button-positive',
			click: ui.createHandlerFn(this, () => runUpdater('upgrade'))
		}, _('升级 MosDNS-T'));
		upgradeButton.disabled = !updateAvailable;

		m = new form.Map('mosdns-t', _('MosDNS-T'),
			_('接管 dnsmasq 时，dnsmasq 保留 53 端口并把请求转发到 MosDNS-T；未接管时，MosDNS-T 直接在所设端口提供 DNS 服务。'));

		s = m.section(form.TypedSection);
		s.anonymous = true;
		s.render = () => E('div', { class: 'cbi-section' }, [
			E('p', { id: 'mosdns-t-status' }, [
				E('strong', {}, _('运行状态：')),
				E('span', { style: 'color:%s'.format(running ? 'green' : 'red') },
					running ? _('运行中') : _('已停止'))
			]),
			E('p', {}, [
				E('strong', {}, _('版本：')),
				E('span', {}, version || _('未知'))
			]),
			E('div', { class: 'right' }, [
				E('button', {
					class: 'btn cbi-button cbi-button-apply',
					click: ui.createHandlerFn(this, () => runAction('start'))
				}, _('启动')),
				' ',
				E('button', {
					class: 'btn cbi-button cbi-button-action',
					click: ui.createHandlerFn(this, () => runAction('restart'))
				}, _('重启')),
				' ',
				E('button', {
					class: 'btn cbi-button cbi-button-negative',
					click: ui.createHandlerFn(this, () => runAction('stop'))
				}, _('停止')),
				' ',
				E('a', {
					class: 'btn cbi-button cbi-button-positive',
					href: 'http://' + window.location.hostname + ':' + webuiPort + '/',
					target: '_blank',
					rel: 'noreferrer'
				}, _('打开 MosDNS WebUI'))
			])
		]);

		s = m.section(form.TypedSection);
		s.anonymous = true;
		s.render = () => E('div', { class: 'cbi-section' }, [
			E('p', {}, '%s %s'.format(_('已安装：'), packageStatus.core_version || _('未知'))),
			E('p', {}, updateAvailable
				? _('软件源中有新版本可用。')
				: updateChecked
					? _('当前已是最新版本。')
					: _('点击“检查更新”刷新 MosDNS-T 软件源。')),
			E('div', { class: 'right' }, [
				E('button', {
					class: 'btn cbi-button cbi-button-action',
					click: ui.createHandlerFn(this, () => runUpdater('check'))
				}, _('检查更新')),
				' ',
				upgradeButton
			])
		]);

		s = m.section(form.NamedSection, 'main', 'mosdns-t', _('基础设置'));
		s.anonymous = true;

		o = s.option(form.Flag, '_autostart', _('开机自启动'));
		o.cfgvalue = () => autostart ? '1' : '0';
		o.write = (sectionId, value) => fs.exec('/etc/init.d/mosdns-t',
			[ value === '1' ? 'enable' : 'disable' ]);
		o.remove = () => fs.exec('/etc/init.d/mosdns-t', [ 'disable' ]);
		o.rmempty = false;
		o.description = _('控制路由器启动时是否自动启动 MosDNS-T。');

		o = s.option(form.Flag, 'dnsmasq_forward', _('接管 dnsmasq 上游'));
		o.default = o.enabled;
		o.rmempty = false;
		o.description = _('启用后，dnsmasq 继续监听 53，并把上游请求发往当前 MosDNS-T DNS 端口。');

		o = s.option(form.Value, 'listen_port', _('MosDNS DNS 端口'));
		o.cfgvalue = () => dnsPort;
		o.datatype = 'port';
		o.rmempty = false;
		o.write = (sectionId, value) => writePort(sectionId, 'listen_port', 'set-dns-port', value);
		o.description = _('接管 dnsmasq 时仅监听 127.0.0.1；未接管时监听全部地址，可由局域网客户端直接访问。保存并应用后会更新 UDP/TCP 监听地址和 dnsmasq 转发目标。');

		o = s.option(form.Value, 'webui_port', _('MosDNS WebUI 端口'));
		o.cfgvalue = () => webuiPort;
		o.datatype = 'port';
		o.rmempty = false;
		o.write = (sectionId, value) => writePort(sectionId, 'webui_port', 'set-webui-port', value);
		o.description = _('与 MosDNS WebUI 使用同一个端口设置；从任一界面修改后，另一边都会显示相同端口。');

		poll.add(() => getStatus().then(isRunning => {
			const el = document.getElementById('mosdns-t-status');
			if (el)
				el.lastElementChild.textContent = isRunning ? _('运行中') : _('已停止');
		}));

		return m.render();
	}
});
