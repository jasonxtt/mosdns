// -- [修改] -- 引入新的、可靠的滚动锁定/解锁机制
let savedScrollY = 0;

function lockScroll() {
    savedScrollY = window.scrollY;
    document.body.style.position = 'fixed';
    document.body.style.top = `-${savedScrollY}px`;
    document.body.style.width = '100%';
    // 保持滚动条占位，防止页面宽度变化导致抖动
    document.body.style.overflowY = 'scroll';
}

function unlockScroll() {
    document.body.style.position = '';
    document.body.style.top = '';
    document.body.style.width = '';
    document.body.style.overflowY = '';
    const htmlEl = document.documentElement;
    const prevScrollBehavior = htmlEl.style.scrollBehavior;
    htmlEl.style.scrollBehavior = 'auto';
    window.scrollTo(0, savedScrollY);
    htmlEl.style.scrollBehavior = prevScrollBehavior;
}

// -- [修改] -- 创建一个统一的关闭函数来消除闪烁
function closeAndUnlock(dialogElement) {
    if (dialogElement && dialogElement.open) {
        unlockScroll();
        dialogElement.close();
    }
}


document.addEventListener('DOMContentLoaded', () => {
    const CONSTANTS = { API_BASE_URL: '', LOGS_PER_PAGE: 50, HISTORY_LENGTH: 60, DEFAULT_AUTO_REFRESH_INTERVAL: 15, ANIMATION_DURATION: 1000, MOBILE_BREAKPOINT: 1024, TOAST_DURATION: 3000, SKELETON_ROWS: 10, TOOLTIP_SHOW_DELAY: 200, TOOLTIP_HIDE_DELAY: 250, UPDATE_AUTO_MINUTES_DEFAULT: 1440 };
    let state = { isUpdating: false, isCapturing: false, isMobile: false, isTouchDevice: false, currentLogPage: 1, isLogLoading: false, logPaginationInfo: null, displayedLogs: [], currentLogSearchTerm: '', clientAliases: {}, topDomains: [], topClients: [], slowestQueries: [], domainSetRank: [], shuntColors: {}, logSort: { key: 'query_time', order: 'desc' }, autoRefresh: { enabled: false, intervalId: null, intervalSeconds: CONSTANTS.DEFAULT_AUTO_REFRESH_INTERVAL }, data: { totalQueries: { current: null, previous: null }, avgDuration: { current: null, previous: null } }, history: { totalQueries: [], avgDuration: [], timestamps: [] }, lastUpdateTime: null, adguardRules: [], diversionRules: [], requery: { status: null, config: null, pollId: null }, dataView: { rawEntries: [], filteredEntries: [], viewType: 'domain', currentOffset: 0, currentLimit: 100, currentQuery: '', currentConfig: null, hasMore: true, totalCount: 0 }, coreMode: 'A', cacheStats: {}, listManagerInitialized: false, featureSwitches: {}, systemInfo: {}, update: { status: null, loading: false, auto: { enabled: true, intervalMinutes: CONSTANTS.UPDATE_AUTO_MINUTES_DEFAULT, timerId: null } } };
    const elements = {
        html: document.documentElement, body: document.body, container: document.querySelector('.container'), initialLoader: document.getElementById('initial-loader'),
        colorSwatches: document.querySelectorAll('.color-swatch'),
        themeSwitcher: document.getElementById('theme-switcher-select'),
        layoutSwitcher: document.getElementById('layout-density-select'),
        mainNav: document.querySelector('.main-nav'), navSlider: document.querySelector('.main-nav-slider'),
        tabLinks: document.querySelectorAll('.tab-link'), tabContents: document.querySelectorAll('.tab-content'),
        globalRefreshBtn: document.getElementById('global-refresh-btn'),
        overviewChartModeToggle: document.getElementById('overview-chart-mode-toggle'),
        independentChartPanel: document.getElementById('independent-chart-panel'),
        bigSparklineMerged: document.getElementById('big-sparkline-merged'), lastUpdated: document.getElementById('last-updated'),
        autoRefreshToggle: document.getElementById('auto-refresh-toggle'), autoRefreshIntervalInput: document.getElementById('auto-refresh-interval'), autoRefreshForm: document.getElementById('auto-refresh-form'),
        totalQueries: document.getElementById('total-queries'), avgDuration: document.getElementById('avg-duration'),
        totalQueriesChange: document.getElementById('total-queries-change'), avgDurationChange: document.getElementById('avg-duration-change'),
        sparklineTotal: document.getElementById('sparkline-total'), sparklineAvg: document.getElementById('sparkline-avg'),
        auditStatus: document.getElementById('audit-status'), toggleAuditBtn: document.getElementById('toggle-audit-btn'), clearAuditBtn: document.getElementById('clear-audit-btn'),
        auditCapacity: document.getElementById('audit-capacity'), capacityForm: document.getElementById('capacity-form'), newCapacityInput: document.getElementById('new-capacity'),
        cacheStatsTbody: document.getElementById('cache-stats-tbody'),
        topDomainsBody: document.getElementById('top-domains-body'), topClientsBody: document.getElementById('top-clients-body'), slowestQueriesBody: document.getElementById('slowest-queries-body'),
        shuntResultsBody: document.getElementById('shunt-results-body'),
        // 覆盖配置元素
        overridesModule: document.getElementById('overrides-module'),
        overrideSocks5Input: document.getElementById('override-socks5-log'),
        overrideEcsInput: document.getElementById('override-ecs-log'),
        overridesLoadBtn: document.getElementById('overrides-load-btn-log'),
        overridesSaveBtn: document.getElementById('overrides-save-btn-log'),
        logTable: document.getElementById('log-table'), logTableHead: document.getElementById('log-table-head'), logTableBody: document.getElementById('log-table-body'),
        logQueryTab: document.getElementById('log-query-tab'),
        logSearch: document.getElementById('log-search'), logQueryTableContainer: document.getElementById('log-query-table-container'), logLoader: document.getElementById('log-loader'),
        searchResultsInfo: document.getElementById('search-results-info'),
        toast: document.getElementById('toast'),
        tooltip: document.getElementById('answers-tooltip'),
        aliasModal: document.getElementById('alias-modal'), manageAliasesBtn: document.getElementById('manage-aliases-btn'), manageAliasesBtnMobile: document.getElementById('manage-aliases-btn-mobile'), manualAliasForm: document.getElementById('manual-alias-form'),
        aliasListContainer: document.getElementById('alias-list-container'), importAliasInput: document.getElementById('import-alias-file-input'), saveAllAliasesBtn: document.getElementById('save-all-aliases-btn'),
        systemControlTabIndicator: document.querySelector('a[data-tab="system-control"] .status-indicator'),
        addAdguardRuleBtn: document.getElementById('add-adguard-rule-btn'),
        checkAdguardUpdatesBtn: document.getElementById('check-adguard-updates-btn'),
        adguardRulesTbody: document.getElementById('adguard-rules-tbody'),
        addDiversionRuleBtn: document.getElementById('add-diversion-rule-btn'),
        diversionRulesTbody: document.getElementById('diversion-rules-tbody'),
        ruleModal: document.getElementById('rule-modal'),
        modalTitle: document.getElementById('modal-title'),
        ruleForm: document.getElementById('rule-form'),
        closeRuleModalBtn: document.getElementById('close-rule-modal'),
        cancelRuleModalBtn: document.getElementById('cancel-rule-modal-btn'),
        saveRuleBtn: document.getElementById('save-rule-btn'),
        ruleMode: document.getElementById('rule-mode'),
        ruleTypeWrapper: document.getElementById('rule-type-wrapper'),
        ruleFilesWrapper: document.getElementById('rule-files-wrapper'),
        rulesSubNavLinks: document.querySelectorAll('.sub-nav-link'),
        rulesSubTabContents: document.querySelectorAll('.sub-tab-content'),
        logDetailModal: document.getElementById('log-detail-modal'),
        logDetailModalBody: document.getElementById('log-detail-modal-body'),
        closeLogDetailModalBtn: document.getElementById('close-log-detail-modal'),

        requeryModule: document.getElementById('requery-module'),
        requeryStatusText: document.getElementById('requery-status-text'),
        requeryProgressContainer: document.getElementById('requery-progress-container'),
        requeryProgressBarFill: document.getElementById('requery-progress-bar-fill'),
        requeryProgressBarText: document.getElementById('requery-progress-bar-text'),
        requeryLastRun: document.getElementById('requery-last-run'),
        requeryTriggerBtn: document.getElementById('requery-trigger-btn'),
        requeryCancelBtn: document.getElementById('requery-cancel-btn'),
        requerySchedulerForm: document.getElementById('requery-scheduler-form'),
        requerySchedulerToggle: document.getElementById('requery-scheduler-toggle'),
        requeryIntervalInput: document.getElementById('requery-interval-input'),
        requeryDateRangeInput: document.getElementById('requery-date-range-input'),
        requeryStartDatetimeInput: document.getElementById('requery-start-datetime-input'),
        requeryDomainStatsTbody: document.getElementById('requery-domain-stats-tbody'),
        requeryRefreshStatsBtn: document.getElementById('requery-refresh-stats-btn'),
        updateModule: document.getElementById('update-module'),
        updateCurrentVersion: document.getElementById('update-current-version'),
        updateLatestVersion: document.getElementById('update-latest-version'),
        updateInlineBadge: document.getElementById('update-inline-badge'),
        updateStatusBanner: document.getElementById('update-status-banner'),
        updateStatusText: document.getElementById('update-status-text'),
        updateLastChecked: document.getElementById('update-last-checked'),
        updateTargetInfo: document.getElementById('update-target-info'),
        updateCheckBtn: document.getElementById('update-check-btn'),
        updateApplyBtn: document.getElementById('update-apply-btn'),
        updateForceBtn: document.getElementById('update-force-btn'),
        updateV3Callout: document.getElementById('update-v3-callout'),
        updateV3Btn: document.getElementById('update-v3-btn'),
        updateAutoToggle: document.getElementById('update-auto-toggle'),
        updateIntervalInput: document.getElementById('update-interval-input'),
        updateHintText: document.getElementById('update-hint-text'),

        fakeipDomainCount: document.getElementById('fakeip-domain-count'),
        realipDomainCount: document.getElementById('realip-domain-count'),
        nov4DomainCount: document.getElementById('nov4-domain-count'),
        nov6DomainCount: document.getElementById('nov6-domain-count'),
        totalDomainCount: document.getElementById('total-domain-count'), 
        backupDomainCount: document.getElementById('backup-domain-count'),

        saveShuntRulesBtn: document.getElementById('save-shunt-rules-btn'),
        clearShuntRulesBtn: document.getElementById('clear-shunt-rules-btn'),

        dataViewModal: document.getElementById('data-view-modal'),
        closeDataViewModalBtn: document.getElementById('close-data-view-modal'),
        dataViewModalTitle: document.getElementById('data-view-modal-title'),
        dataViewModalBody: document.getElementById('data-view-modal-body'),
        dataViewSearch: document.getElementById('data-view-search'),
        dataViewModalInfo: document.getElementById('data-view-modal-info'),
        dataViewTableContainer: document.getElementById('data-view-table-container'),

        listMgmtNav: document.querySelector('.list-mgmt-nav'),
        listContentLoader: document.getElementById('list-content-loader'),
        listContentTextArea: document.getElementById('list-content-textarea'),
        listContentInfo: document.getElementById('list-content-info'),
        listSaveBtn: document.getElementById('list-save-btn'),
	listMgmtRealIPHint: document.getElementById('list-mgmt-realip-hint'),
	listMgmtCnFakeipFilterHint: document.getElementById('list-mgmt-cnfakeipfilter-hint'),
        listMgmtClientIpHint: document.getElementById('list-mgmt-client-ip-hint'),
        listMgmtDirectIpHint: document.getElementById('list-mgmt-direct-ip-hint'),
        listMgmtRewriteHint: document.getElementById('list-mgmt-rewrite-hint'),

        featureSwitchesModule: document.getElementById('feature-switches-module'),
        coreModeSwitchGroup: document.getElementById('core-mode-switch-group'),
        secondarySwitchesContainer: document.getElementById('secondary-switches-container'),
        systemInfoContainer: document.getElementById('system-info-container'),
    };
    let toastTimeout;

    const SHUNT_RULE_SAVE_PATHS = ['top_domains/save', 'my_fakeiplist/save', 'my_nodenov4list/save', 'my_nodenov6list/save', 'my_notinlist/save', 'my_nov4list/save', 'my_nov6list/save', 'my_realiplist/save'];
    const SHUNT_RULE_FLUSH_PATHS = ['top_domains/flush', 'my_fakeiplist/flush', 'my_nodenov4list/flush', 'my_nodenov6list/flush', 'my_notinlist/flush', 'my_nov4list/flush', 'my_nov6list/flush', 'my_realiplist/flush'];

    const debounce = (func, wait) => { let timeout; return function executedFunction(...args) { const later = () => { clearTimeout(timeout); func(...args); }; clearTimeout(timeout); timeout = setTimeout(later, wait); }; };

    // 轻量级请求器 + /metrics 简易缓存，减少同一时段的重复请求
    let __metricsInflight = null; let __metricsStamp = 0;
    const api = { fetch: async (url, options = {}) => { try { const response = await fetch(url, { ...options, signal: options.signal }); if (!response.ok) { let errorMsg = `API Error: ${response.status} ${response.statusText}`; try { const errorBody = await response.json(); if (errorBody && errorBody.error) { errorMsg = errorBody.error; } } catch (e) { try { errorMsg = await response.text() || errorMsg; } catch (textErr) { } } if (response.status !== 404) { ui.showToast(errorMsg, 'error'); } throw new Error(errorMsg); } const tc = response.headers.get('X-Total-Count'); const ct = response.headers.get('content-type'); const data = (ct && ct.includes('application/json')) ? await response.json() : await response.text(); return tc !== null ? { body: data, totalCount: parseInt(tc, 10) } : data; } catch (error) { if (error.name !== 'AbortError') { console.error(error); } throw error; } }, getStatus: (signal) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/status`, { signal }), getCapacity: (signal) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/capacity`, { signal }), start: () => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/start`, { method: 'POST' }), stop: () => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/stop`, { method: 'POST' }), clear: () => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/clear`, { method: 'POST' }), setCapacity: (capacity) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/capacity`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ capacity: parseInt(capacity, 10) }) }), getMetrics: (signal) => { const now = Date.now(); if (__metricsInflight && (now - __metricsStamp) < 3000) return __metricsInflight; __metricsInflight = api.fetch('/metrics', { signal }); __metricsStamp = now; return __metricsInflight; }, getCoreMode: (signal) => api.fetch('/plugins/switch3/show', { signal }), clearCache: (cacheTag) => api.fetch(`/plugins/${cacheTag}/flush`), getCacheContents: (cacheTag, signal) => api.fetch(`/plugins/${cacheTag}/show`, { signal }), v2: { getStats: (signal) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/stats`, { signal }), getTopDomains: (signal, limit = 50) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/rank/domain?limit=${limit}`, { signal }), getTopClients: (signal, limit = 50) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/rank/client?limit=${limit}`, { signal }), getSlowest: (signal, limit = 50) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/rank/slowest?limit=${limit}`, { signal }), getDomainSetRank: (signal, limit = 50) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/rank/domain_set?limit=${limit}`, { signal }), getLogs: (signal, params = {}) => { const queryParams = new URLSearchParams({ page: 1, limit: CONSTANTS.LOGS_PER_PAGE, ...params }); for (let [key, value] of queryParams.entries()) { if (!value) { queryParams.delete(key); } } return api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/logs?${queryParams}`, { signal }); } } };

    const requeryApi = {
        getConfig: (signal) => api.fetch(`/plugins/requery`, { signal }),
        getStatus: (signal) => api.fetch(`/plugins/requery/status`, { signal }),
        trigger: () => api.fetch(`/plugins/requery/trigger`, { method: 'POST' }),
        cancel: () => api.fetch(`/plugins/requery/cancel`, { method: 'POST' }),
        updateSchedulerConfig: (config) => api.fetch(`/plugins/requery/scheduler/config`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(config) }),
    };

    const updateApi = {
        getStatus: (signal) => api.fetch(`/api/v1/update/status`, { signal }),
        forceCheck: () => api.fetch(`/api/v1/update/check`, { method: 'POST' }),
        apply: (force = false, preferV3 = false) => api.fetch(`/api/v1/update/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ force, prefer_v3: preferV3 }) })
    };

    const normalizeIP = (ip) => {
        if (typeof ip === 'string' && ip.startsWith('::ffff:')) {
            return ip.substring(7);
        }
        return ip;
    };

    const clientnameApi = {
        get: () => api.fetch(`/plugins/clientname`),
        update: (data) => api.fetch(`/plugins/clientname`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data),
        }),
    };

    const coreApi = {
        getMode: async () => {
            try {
                const response = await api.fetch('/plugins/switch3/show');
                if (typeof response === 'string') {
                    return response.trim();
                }
                return 'A';
            } catch (e) {
                console.error("无法获取核心模式状态:", e);
                return 'A';
            }
        }
    };

    const ui = {
        showToast(message, type = 'success') { if (!elements.toast) return; clearTimeout(toastTimeout); const icon = type === 'success' ? `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z"></path></svg>` : `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-2h2v2zm0-4h-2V7h2v6z"></path></svg>`; elements.toast.innerHTML = `${icon}<span>${message}</span>`; elements.toast.className = `show ${type}`; const hideToast = () => { elements.toast.className = elements.toast.className.replace('show', ''); }; elements.toast.onmouseenter = () => clearTimeout(toastTimeout); elements.toast.onmouseleave = () => toastTimeout = setTimeout(hideToast, CONSTANTS.TOAST_DURATION); toastTimeout = setTimeout(hideToast, CONSTANTS.TOAST_DURATION); },
        setLoading(button, isLoading) { if (!button) return; const textSpan = button.querySelector('span'); button.disabled = isLoading; button.setAttribute('aria-busy', String(isLoading)); if (textSpan) { if (isLoading) { if (!button.dataset.defaultText) { button.dataset.defaultText = textSpan.textContent; } textSpan.textContent = '处理中...'; } else { if (button.dataset.defaultText) { textSpan.textContent = button.dataset.defaultText; } } } },
        updateStatus(isCapturing) { if (!elements.toggleAuditBtn || !elements.auditStatus) return; this.setLoading(elements.toggleAuditBtn, false); const statusIndicator = elements.systemControlTabIndicator; if (statusIndicator) statusIndicator.className = 'status-indicator'; if (typeof isCapturing === 'boolean') { state.isCapturing = isCapturing; elements.auditStatus.textContent = isCapturing ? '运行中' : '已停止'; elements.auditStatus.style.color = isCapturing ? 'var(--color-success)' : 'var(--color-danger)'; const actionText = isCapturing ? '关闭审计' : '开启审计'; elements.toggleAuditBtn.querySelector('span').textContent = actionText; elements.toggleAuditBtn.dataset.defaultText = actionText; elements.toggleAuditBtn.className = `button ${isCapturing ? 'danger' : 'primary'}`; if (statusIndicator) statusIndicator.classList.add(isCapturing ? 'running' : 'stopped'); } else { elements.auditStatus.textContent = '未知'; elements.auditStatus.style.color = 'var(--color-text-secondary)'; elements.toggleAuditBtn.querySelector('span').textContent = '刷新状态'; elements.toggleAuditBtn.dataset.defaultText = '刷新状态'; } },
        updateCapacity(capacity) { if (elements.auditCapacity) elements.auditCapacity.textContent = capacity != null ? `${capacity.toLocaleString()} 条` : '查询失败'; },
        updateOverviewStats() {
            const { totalQueries, avgDuration } = state.data;
            animateValue(elements.totalQueries, totalQueries.previous, totalQueries.current, CONSTANTS.ANIMATION_DURATION);
            animateValue(elements.avgDuration, avgDuration.previous, avgDuration.current, CONSTANTS.ANIMATION_DURATION, 2);
            updateStatChange(elements.totalQueriesChange, totalQueries.previous, totalQueries.current);
            updateStatChange(elements.avgDurationChange, avgDuration.previous, avgDuration.current, true);

            // Standard small charts
            if (elements.sparklineTotal) elements.sparklineTotal.innerHTML = generateSparklineSVG(state.history.totalQueries);
            if (elements.sparklineAvg) elements.sparklineAvg.innerHTML = generateSparklineSVG(state.history.avgDuration, true);

            // Independent mode merged big chart
            const isIndependent = document.querySelector('.stats-grid')?.classList.contains('independent-mode');
            if (isIndependent && elements.bigSparklineMerged) {
                // Adaptive dimensions for mobile readability
                const w = state.isMobile ? 400 : 1000;
                const h = state.isMobile ? 220 : 260;
                elements.bigSparklineMerged.innerHTML = generateDualSparklineSVG(state.history.totalQueries, state.history.avgDuration, state.history.timestamps, w, h);
            }
        },
        renderLogTable(logs, append = false) {
            const tbody = elements.logTableBody;
            if (!tbody) return;
            if (!append) { tbody.innerHTML = ''; state.displayedLogs = []; }
            if (logs.length === 0 && !append) { renderTable(tbody, [], () => { }, 'log-query'); return; }
            const startIndex = state.displayedLogs.length;
            state.displayedLogs.push(...logs);

            // Batch rendering to avoid frame drops when inserting many rows
            const BATCH = 50;
            let idx = 0;
            const renderChunk = () => {
                if (idx >= logs.length) return;
                const frag = document.createDocumentFragment();
                for (let c = 0; c < BATCH && idx < logs.length; c++, idx++) {
                    const log = logs[idx];
                    const row = renderLogItemHTML(log, startIndex + idx);
                    // 仅对前20行做入场动画，减少布局/绘制开销
                    if (startIndex + idx < 20) {
                        row.classList.add('animate-in');
                    }
                    frag.appendChild(row);
                }
                tbody.appendChild(frag);
                if (typeof window !== 'undefined' && 'requestIdleCallback' in window) {
                    requestIdleCallback(renderChunk, { timeout: 300 });
                } else {
                    setTimeout(renderChunk, 0);
                }
            };
            renderChunk();
        },
        updateSearchResultsInfo(pagination) { if (!elements.searchResultsInfo) return; if (state.currentLogSearchTerm?.query && pagination) { elements.searchResultsInfo.innerHTML = `为您找到 <strong>${pagination.total_items.toLocaleString()}</strong> 条相关结果`; } else { elements.searchResultsInfo.innerHTML = ''; } },
        openLogDetailModal(triggerElement) {
            const logIndex = triggerElement.dataset.logIndex ? parseInt(triggerElement.dataset.logIndex, 10) : null;
            const source = triggerElement.dataset.logSource || 'log';
            let data;

            if (source === 'slowest' && logIndex !== null) data = state.slowestQueries[logIndex];
            else if (logIndex !== null) data = state.displayedLogs[logIndex];

            if (!data) return;

            elements.logDetailModalBody.innerHTML = getDetailContentHTML(data);

            // -- [修改] -- 采用新的滚动锁定机制
            lockScroll();
            elements.logDetailModal.showModal();
        },
        openRuleModal(mode, rule = null) {
            const form = elements.ruleForm;
            form.reset();
            elements.ruleMode.value = mode;
            const isDiversion = mode === 'diversion';
            elements.modalTitle.textContent = rule ? `修改${isDiversion ? '分流' : '拦截'}规则` : `添加${isDiversion ? '分流' : '拦截'}规则`;
            elements.ruleTypeWrapper.style.display = isDiversion ? 'block' : 'none';
            elements.ruleFilesWrapper.style.display = isDiversion ? 'block' : 'none';
            const regexpWrapper = document.getElementById('rule-enable-regexp-wrapper');
            if (regexpWrapper) {
                regexpWrapper.style.display = isDiversion ? 'flex' : 'none';
            }
            form.elements['type'].required = isDiversion;
            form.elements['files'].required = isDiversion;

            if (rule) {
                form.elements['id'].value = rule.id ?? rule.name;
                form.elements['name'].value = rule.name;
                form.elements['url'].value = rule.url;
                form.elements['auto_update'].checked = rule.auto_update;
                form.elements['update_interval_hours'].value = rule.update_interval_hours || 24;
                if (form.elements['enable_regexp']) {
                    form.elements['enable_regexp'].checked = rule.enable_regexp || false;
                }
                if (isDiversion) {
                    form.elements['type'].value = rule.type;
                    form.elements['files'].value = rule.files;
                }
            } else {
                form.elements['id'].value = '';
                form.elements['auto_update'].checked = true;
                if (form.elements['enable_regexp']) {
                    form.elements['enable_regexp'].checked = false;
                }
                form.elements['update_interval_hours'].value = 24;
                if (isDiversion) form.elements['type'].value = "";
            }

            // -- [修改] -- 采用新的滚动锁定机制
            lockScroll();
            elements.ruleModal.showModal();
        },
        closeRuleModal() {
            // -- [修改] -- 使用新的统一关闭函数
            closeAndUnlock(elements.ruleModal);
        }
    };

    function updateNavSlider(activeLink) {
        if (!elements.navSlider || !elements.mainNav) return;
        const navRect = elements.mainNav.getBoundingClientRect();
        const linkRect = activeLink.getBoundingClientRect();
        const left = linkRect.left - navRect.left;
        elements.navSlider.style.width = `${linkRect.width}px`;
        elements.navSlider.style.transform = `translateX(${left}px)`;
    }

    function formatDateForInputLocal(isoString) {
        if (!isoString || isoString.startsWith('0001-01-01')) {
            return '';
        }
        try {
            const date = new Date(isoString);
            if (isNaN(date.getTime())) return '';
            const year = date.getFullYear();
            const month = (date.getMonth() + 1).toString().padStart(2, '0');
            const day = date.getDate().toString().padStart(2, '0');
            const hours = date.getHours().toString().padStart(2, '0');
            const minutes = date.getMinutes().toString().padStart(2, '0');
            return `${year}-${month}-${day}T${hours}:${minutes}`;
        } catch (e) {
            console.error("Error formatting date:", e);
            return '';
        }
    }

    const requeryManager = {
        init() {
            const debouncedUpdate = debounce(this.handleUpdateSchedulerConfig.bind(this), 1500);
            elements.requeryTriggerBtn.addEventListener('click', this.handleTrigger.bind(this));
            elements.requeryCancelBtn.addEventListener('click', this.handleCancel.bind(this));
            elements.requerySchedulerToggle.addEventListener('change', this.handleUpdateSchedulerConfig.bind(this));
            elements.requeryIntervalInput.addEventListener('change', debouncedUpdate);
            if (elements.requeryDateRangeInput) {
                elements.requeryDateRangeInput.addEventListener('change', debouncedUpdate);
            }
            elements.requeryStartDatetimeInput.addEventListener('change', debouncedUpdate);
        },

        async updateStatus(signal) {
            this.updateDomainCounts(signal); // 在更新状态时自动刷新统计
            try {
                const [status, config] = await Promise.all([
                    requeryApi.getStatus(signal),
                    requeryApi.getConfig(signal)
                ]);
                state.requery.status = status;
                state.requery.config = config;
                this.render();
            } catch (error) {
                if (error.name !== 'AbortError') {
                    this.render(null, null);
                }
            }
        },

        render() {
            const status = state.requery.status;
            const config = state.requery.config;

            if (!status || !config) {
                elements.requeryStatusText.textContent = '获取状态失败';
                elements.requeryStatusText.style.color = 'var(--color-danger)';
                elements.requeryTriggerBtn.disabled = true;
                return;
            }

            const isRunning = status.task_state === 'running';

            let statusText = '空闲';
            let statusColor = 'var(--color-success)';
            switch (status.task_state) {
                case 'running':
                    statusText = '正在执行...';
                    statusColor = 'var(--color-warning)';
                    this.startPolling();
                    break;
                case 'failed':
                    statusText = '上次执行失败';
                    statusColor = 'var(--color-danger)';
                    this.stopPolling();
                    break;
                case 'cancelled':
                    statusText = '上次任务已取消';
                    statusColor = 'var(--color-text-secondary)';
                    this.stopPolling();
                    break;
                default:
                    this.stopPolling();
                    break;
            }
            elements.requeryStatusText.textContent = statusText;
            elements.requeryStatusText.style.color = statusColor;

            elements.requeryProgressContainer.hidden = !isRunning;
            if (isRunning) {
                const percent = (status.progress.total > 0) ? (status.progress.processed / status.progress.total) * 100 : 0;
                elements.requeryProgressBarFill.style.width = `${percent}%`;
                elements.requeryProgressBarText.textContent = `${Math.floor(percent)}% (${status.progress.processed.toLocaleString()} / ${status.progress.total.toLocaleString()})`;
            }

            if (status.last_run_start_time && !status.last_run_start_time.startsWith('0001-01-01')) {
                let lastRunText = `开始于 ${formatRelativeTime(status.last_run_start_time)}`;
                if (status.last_run_end_time && !status.last_run_end_time.startsWith('0001-01-01')) {
                    const startDate = new Date(status.last_run_start_time);
                    const endDate = new Date(status.last_run_end_time);
                    const durationSeconds = Math.round((endDate - startDate) / 1000);
                    lastRunText = `完成于 ${formatRelativeTime(status.last_run_end_time)} (耗时 ${durationSeconds}秒)`;
                }
                elements.requeryLastRun.textContent = lastRunText;
            } else {
                elements.requeryLastRun.textContent = '从未执行';
            }

            if (config.execution_settings && elements.requeryDateRangeInput) {
                elements.requeryDateRangeInput.value = config.execution_settings.date_range_days || 30;
            }

            elements.requerySchedulerToggle.checked = config.scheduler.enabled;
            elements.requeryIntervalInput.value = config.scheduler.interval_minutes;
            elements.requeryStartDatetimeInput.value = formatDateForInputLocal(config.scheduler.start_datetime);

            const schedulerInputsDisabled = !config.scheduler.enabled;
            elements.requeryIntervalInput.disabled = schedulerInputsDisabled;
            elements.requeryStartDatetimeInput.disabled = schedulerInputsDisabled;

            elements.requeryTriggerBtn.hidden = isRunning;
            elements.requeryCancelBtn.hidden = !isRunning;
            elements.requeryTriggerBtn.disabled = isRunning;
            elements.requeryClearBackupBtn.disabled = isRunning;
        },

        startPolling() {
            if (state.requery.pollId) return;
            state.requery.pollId = setInterval(() => {
                this.updateStatus();
            }, 5000);
        },

        stopPolling() {
            clearInterval(state.requery.pollId);
            state.requery.pollId = null;
        },

        async handleTrigger(e, silent = false) {
            const confirmed = silent ? true : confirm('确定要开始一个全新的刷新任务吗？\n这将完整执行所有步骤，可能需要一些时间。');
            if (confirmed) {
                const btn = e ? e.currentTarget : elements.requeryTriggerBtn;
                ui.setLoading(btn, true);
                try {
                    await requeryApi.trigger();
                    ui.showToast('刷新任务已开始', 'success');
                    await this.updateStatus();
                } catch (error) {
                    // Error toast is already shown by api.fetch
                } finally {
                    ui.setLoading(btn, false);
                }
            }
        },

        async handleCancel(e) {
            if (confirm('确定要取消当前正在执行的任务吗？')) {
                const btn = e.currentTarget;
                ui.setLoading(btn, true);
                try {
                    await requeryApi.cancel();
                    ui.showToast('已发送取消请求', 'success');
                    elements.requeryCancelBtn.hidden = true;
                    elements.requeryTriggerBtn.hidden = false;
                } catch (error) { }
                finally {
                    ui.setLoading(btn, false);
                }
            }
        },

        async handleUpdateSchedulerConfig() {
            const isEnabled = elements.requerySchedulerToggle.checked;
            const interval = parseInt(elements.requeryIntervalInput.value, 10);
            const localTime = elements.requeryStartDatetimeInput.value;
            const dateRangeDays = parseInt(elements.requeryDateRangeInput.value, 10);

            if (isEnabled && (!interval || interval <= 0)) {
                ui.showToast('启用定时任务时，必须设置一个有效的间隔分钟数', 'error');
                return;
            }

            let utcTime = '';
            if (localTime) {
                try {
                    utcTime = new Date(localTime).toISOString();
                } catch (e) {
                    ui.showToast('输入的首次执行时间格式无效', 'error');
                    return;
                }
            }

            if (!dateRangeDays || dateRangeDays < 1) {
                ui.showToast('域名刷新天数必须大于 0', 'error');
                return;
            }

            const newConfig = {
                enabled: isEnabled,
                interval_minutes: interval || 0,
                start_datetime: utcTime,
                date_range_days: dateRangeDays
            };

            try {
                await requeryApi.updateSchedulerConfig(newConfig);
                ui.showToast('定时任务配置已更新', 'success');
                if (state.requery.config) {
                    state.requery.config.scheduler = newConfig;
                    if (!state.requery.config.execution_settings) state.requery.config.execution_settings = {};
                    state.requery.config.execution_settings.date_range_days = dateRangeDays;

                    this.render();
                }
            } catch (error) { }
        },

        async updateDomainCounts(signal) {
            const btn = elements.requeryRefreshStatsBtn;
            if (btn) {
                const svg = btn.querySelector('svg');
                if (svg) svg.style.animation = 'spin 1s linear infinite';
                btn.disabled = true;
            }

            const tbody = elements.requeryDomainStatsTbody;
            if (!tbody) return;

            tbody.innerHTML = `<tr><td colspan="2">正在加载统计数据...</td></tr>`;

            try {
                const [sourceFilesRes, backupCountRes] = await Promise.allSettled([
                    requeryApi.getCounts(signal),
                    requeryApi.getBackupCount(signal)
                ]);

                let html = '';
                let hasSourceData = false;

                if (sourceFilesRes.status === 'fulfilled' && sourceFilesRes.value.status === 'success' && Array.isArray(sourceFilesRes.value.data)) {
                    const sourceData = sourceFilesRes.value.data;
                    if (sourceData.length > 0) {
                        hasSourceData = true;
                        sourceData.forEach(item => {
                            let linkHtml = item.count.toLocaleString();
                            let listType = null;
                            let listTitle = item.alias;

                            if (item.alias.includes('fakeip')) listType = 'fakeip';
                            else if (item.alias.includes('realip')) listType = 'realip';
                            else if (item.alias.includes('nov4')) listType = 'nov4';
                            else if (item.alias.includes('nov6')) listType = 'nov6';
                            else if (item.alias.includes('notin')) listType = 'notin';

                            if (listType) {
                                linkHtml = `<a href="#" class="control-item-link" data-list-type="${listType}" data-list-title="${listTitle}">${item.count.toLocaleString()}</a>`;
                            }

                            html += `
                                <tr>
                                    <td>${item.alias}</td>
                                    <td class="text-right">${linkHtml}</td>
                                </tr>
                            `;
                        });
                    }
                }

                if (!hasSourceData) {
                    html += `<tr><td colspan="2" style="color: var(--color-danger);">获取源文件条目失败</td></tr>`;
                }

                if (backupCountRes.status === 'fulfilled' && backupCountRes.value.status === 'success') {
                    // 为总计行添加一个顶部边框，通过内联样式实现，这是最直接可靠的方式
                    html += `
                        <tr>
                            <td style="border-top: 1px solid var(--color-border); padding-top: 0.8rem; font-weight: 700;">全部域名 (备份总表)</td>
                            <td class="text-right" style="border-top: 1px solid var(--color-border); padding-top: 0.8rem; font-weight: 700; color: var(--color-accent-primary);">${backupCountRes.value.count.toLocaleString()}</td>
                        </tr>
                    `;
                } else {
                    html += `<tr><td colspan="2" style="color: var(--color-danger);">获取备份总数失败</td></tr>`;
                }

                tbody.innerHTML = html;

            } catch (error) {
                if (error.name !== 'AbortError') {
                    tbody.innerHTML = `<tr><td colspan="2" style="padding: 1rem; text-align: center; color: var(--color-danger);">加载统计数据时发生错误</td></tr>`;
                }
            } finally {
                if (btn) {
                    const svg = btn.querySelector('svg');
                    if (svg) svg.style.animation = '';
                    btn.disabled = false;
                }
            }
        }
    };

    const switchManager = {
        profiles: [
            { tag: 'switch3', name: '核心运行模式', tip: '切换后将执行一次“全新任务”刷新分流缓存。兼容模式性能更高，安全模式防泄露和劫持能力更强。', modes: { 'A': { name: '兼容模式', icon: 'fa-globe-americas' }, 'B': { name: '安全模式', icon: 'fa-shield-alt' } } },
            { tag: 'switch1', name: '请求屏蔽', desc: '对无解析结果的请求进行屏蔽', tip: '建议开启，避免无ipv4及ipv6结果的非必要DNS解析。', valueForOn: 'A' },
            { tag: 'switch5', name: '类型屏蔽', desc: '屏蔽 SOA、PTR、HTTPS 等请求', tip: '建议开启，可减少不必要的网络请求，提高效率。', valueForOn: 'A' },
            { tag: 'switch4', name: '过期缓存1', desc: '启用国内缓存、国外缓存 (兼容)、国外缓存 (安全)、国内域名fakeip缓存', tip: '建议开启，可以提升重复查询的响应速度，即使缓存已过期。', valueForOn: 'A' },
            { tag: 'switch13', name: '过期缓存2', desc: '启用全部缓存 (兼容)、全部缓存 (安全)，缓存fakeip，直面客户端', tip: '建议开启，折腾时可临时关闭，排除干扰。', valueForOn: 'A' },
            { tag: 'switch7', name: '广告屏蔽', desc: '启用Adguard在线规则支持', tip: '此开关开启后，“广告拦截”页签中已启用的在线列表才会生效。', valueForOn: 'A' },
            { tag: 'switch9', name: 'CNFakeIP', desc: '国内域名返回fakeip', tip: '请在上游dns中配置cnfake组的mihomo条目', valueForOn: 'B' },
//            { tag: 'switch11', name: '使用阿里私有DOH', desc: '国内上游并发请求至阿里私有DOH。', tip: '打开前在上游DNS设置中添加DOH配置。', valueForOn: 'A' },
//            { tag: 'switch14', name: '使用运营商DNS', desc: '国内上游并发请求至运营商DNS。', tip: '打开前在上游DNS设置中修改运营商DNS配置。', valueForOn: 'A' },
//            { tag: 'switch15', name: '使用其它DNS', desc: '国内上游并发请求至阿里UDP DNS。', tip: '单独开启时也可替换quic://223.5.5.5为其它DNS。', valueForOn: 'A' },
            { tag: 'switch2', name: '指定 Client fakeip', desc: '只允许指定的客户端科学', tip: '按需开启。需要 MosDNS 监听53端口，并正确配置 client_ip 名单。', valueForOn: 'A' },
            { tag: 'switch12', name: '指定 Client realip', desc: '指定客户端不允许科学', tip: '按需开启。需要 MosDNS 监听53端口，并正确配置 client_ip 名单。', valueForOn: 'A' },
            { tag: 'switch6', name: 'IPV6屏蔽', desc: '屏蔽AAAA请求类型', tip: '无IPV6网络环境建议开启', valueForOn: 'A' },
            { tag: 'switch8', name: 'IPV4优先', desc: 'Prefer IPV4（不建议开启）', tip: '当一个域名有IPV4解析记录时，不返回IPV6解析结果。', valueForOn: 'A' },
            { tag: 'switch10', name: 'IPV6优先', desc: 'Prefer IPV6（不建议开启）', tip: '当一个域名有IPV6解析记录时，不返回IPV4解析结果。', valueForOn: 'A' },
        ],

        init() {
            elements.coreModeSwitchGroup.addEventListener('click', e => {
                const btn = e.target.closest('button');
                if (btn && !btn.classList.contains('active')) {
                    this.handleCoreSwitch(btn);
                }
            });

            elements.secondarySwitchesContainer.addEventListener('change', e => {
                const input = e.target.closest('input[type="checkbox"]');
                if (input) {
                    this.handleSecondarySwitch(input);
                }
            });
        },

        async loadStatus(signal) {
            try {
                const fetchPromises = this.profiles.map(p => api.fetch(`/plugins/${p.tag}/show`, { signal }));
                const results = await Promise.allSettled(fetchPromises);

                results.forEach((result, index) => {
                    const profile = this.profiles[index];
                    if (result.status === 'fulfilled') {
                        state.featureSwitches[profile.tag] = result.value.trim();
                    } else {
                        state.featureSwitches[profile.tag] = 'error';
                        console.error(`获取 ${profile.tag} 状态失败:`, result.reason);
                    }
                });
                this.render();
            } catch (error) {
                if (error.name !== 'AbortError') {
                    elements.featureSwitchesModule.innerHTML = '<h3>功能开关</h3><p style="color:var(--color-danger)">加载开关状态失败。</p>';
                }
            }
        },

        render() {
            const coreStatus = state.featureSwitches['switch3'];
            elements.coreModeSwitchGroup.querySelectorAll('button').forEach(btn => {
                btn.classList.toggle('active', btn.dataset.mode === coreStatus);
                btn.disabled = coreStatus === 'error';
            });

            const secondaryProfiles = this.profiles.filter(p => !p.modes);
            let html = '';
            secondaryProfiles.forEach(profile => {
                const status = state.featureSwitches[profile.tag];
                const isChecked = status === profile.valueForOn;
                const isDisabled = status === 'error';
                html += `
                    <div class="control-item">
                        <strong class="switch-label">
                            <span class="title-line">
                                <span>${profile.name}</span>
                                <span class="info-icon" title="${profile.tip}">
                                    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8zm-1-13h2v2h-2V7zm0 4h2v6h-2v-6z"></path></svg>
                                </span>
                            </span>
                            ${profile.desc ? `<span class="switch-desc">${profile.desc}</span>` : ''}
                        </strong>
                        <label class="switch">
                            <input type="checkbox" data-switch-tag="${profile.tag}" ${isChecked ? 'checked' : ''} ${isDisabled ? 'disabled' : ''}>
                            <span class="slider"></span>
                        </label>
                    </div>`;
            });
            elements.secondarySwitchesContainer.innerHTML = html;
            bindInfoIconTooltips();
        },

        async handleCoreSwitch(button) {
            const tag = 'switch3';
            const valueToPost = button.dataset.mode;
            ui.setLoading(button, true);
            button.parentElement.querySelectorAll('button').forEach(b => b.disabled = true);

            try {
                await api.fetch(`/plugins/${tag}/post`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ value: valueToPost }) });
                state.featureSwitches[tag] = valueToPost;
                this.render();
                ui.showToast('核心模式已切换，即将开始刷新分流缓存...', 'success');
                await requeryManager.handleTrigger(null, true);

            } catch (error) {
                ui.showToast('切换核心模式失败!', 'error');
                this.render();
            } finally {
                ui.setLoading(button, false);
                button.parentElement.querySelectorAll('button').forEach(b => b.disabled = false);
                this.render();
            }
        },

        async handleSecondarySwitch(checkbox) {
            const tag = checkbox.dataset.switchTag;
            const profile = this.profiles.find(p => p.tag === tag);
            if (!profile) return;

            checkbox.disabled = true;
            const valueToPost = checkbox.checked ? profile.valueForOn : (profile.valueForOn === 'A' ? 'B' : 'A');

            try {
                await api.fetch(`/plugins/${tag}/post`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ value: valueToPost }) });
                state.featureSwitches[tag] = valueToPost;
                ui.showToast(`“${profile.name}” 已${checkbox.checked ? '启用' : '禁用'}`);
                if (tag === 'switch9') {
                    (async () => {
                        ui.showToast('附加操作：正在清空核心缓存...', 'info');
                        const results = await Promise.allSettled([
                            api.fetch('/plugins/cache_all/flush'),
                            api.fetch('/plugins/cache_all_noleak/flush')
                        ]);

                        const failedCount = results.filter(r => r.status === 'rejected').length;
                        if (failedCount > 0) {
                            ui.showToast(`附加操作：核心缓存清空完成，有 ${failedCount} 个失败。`, 'error');
                        } else {
                            ui.showToast('附加操作：核心缓存已成功清空！', 'success');
                        }
                    })();
                }
            } catch (error) {
                ui.showToast(`切换“${profile.name}”失败`, 'error');
                checkbox.checked = !checkbox.checked;
            } finally {
                checkbox.disabled = false;
            }
        }
    };

    const updateManager = {
        init() {
            if (!elements.updateModule) return;
            const autoCfg = this.loadAutoConfig();
            state.update.auto.enabled = autoCfg.enabled;
            state.update.auto.intervalMinutes = autoCfg.interval;
            elements.updateAutoToggle.checked = autoCfg.enabled;
            elements.updateIntervalInput.value = autoCfg.interval;
            elements.updateAutoToggle.addEventListener('change', () => {
                state.update.auto.enabled = elements.updateAutoToggle.checked;
                this.persistAutoConfig();
                this.applyAutoSchedule(true);
            });
            elements.updateIntervalInput.addEventListener('change', () => {
                const val = parseInt(elements.updateIntervalInput.value, 10);
                if (!Number.isFinite(val) || val < 5) {
                    elements.updateIntervalInput.value = state.update.auto.intervalMinutes;
                    ui.showToast('自动检查间隔至少为 5 分钟', 'error');
                    return;
                }
                state.update.auto.intervalMinutes = Math.min(val, 720);
                this.persistAutoConfig();
                this.applyAutoSchedule(true);
            });
            elements.updateCheckBtn?.addEventListener('click', () => this.forceCheck());
            elements.updateApplyBtn?.addEventListener('click', () => this.applyUpdate());
            this.applyAutoSchedule(false);
            // 延迟到用户进入“系统控制”页或后台定时器触发时再检查更新，避免首屏加载转圈变慢
            elements.updateForceBtn?.addEventListener('click', () => this.applyUpdate(true, elements.updateForceBtn));
            elements.updateV3Btn?.addEventListener('click', () => this.applyUpdate(true, elements.updateV3Btn, true));
        },

        loadAutoConfig() {
            try {
                const raw = localStorage.getItem('mosdns-update-auto');
                if (!raw) throw new Error('empty');
                const parsed = JSON.parse(raw);
                return {
                    enabled: Boolean(parsed.enabled),
                    interval: Number.isFinite(parsed.interval) ? parsed.interval : CONSTANTS.UPDATE_AUTO_MINUTES_DEFAULT,
                };
            } catch (e) {
                return { enabled: true, interval: CONSTANTS.UPDATE_AUTO_MINUTES_DEFAULT };
            }
        },

        persistAutoConfig() {
            const payload = {
                enabled: state.update.auto.enabled,
                interval: state.update.auto.intervalMinutes,
            };
            try {
                localStorage.setItem('mosdns-update-auto', JSON.stringify(payload));
            } catch (e) {
                console.warn('无法保存自动更新配置:', e);
            }
        },

        applyAutoSchedule(resetTimer) {
            if (resetTimer && state.update.auto.timerId) {
                clearInterval(state.update.auto.timerId);
                state.update.auto.timerId = null;
            }
            if (elements.updateIntervalInput) {
                elements.updateIntervalInput.disabled = !state.update.auto.enabled;
            }
            if (!state.update.auto.enabled) {
                this.setHint('自动检查已关闭。您可以随时手动检查更新。');
                return;
            }
            const intervalMs = Math.max(state.update.auto.intervalMinutes, 5) * 60 * 1000;
            this.setHint(`自动检查已启用，每 ${state.update.auto.intervalMinutes} 分钟检查一次。`);
            if (!state.update.auto.timerId) {
                state.update.auto.timerId = setInterval(() => {
                    this.refreshStatus();
                }, intervalMs);
            }
        },

        setHint(text) {
            if (elements.updateHintText) {
                elements.updateHintText.textContent = text;
            }
        },

        // 监听自重启完成：服务可用且版本变化 / 不再 pending_restart 即视为成功
        restartProbeTimerId: null,
        restartProbeActive: false,
        startRestartWatch(prevVersion) {
            if (this.restartProbeActive) return;
            this.restartProbeActive = true;
            const deadline = Date.now() + 90_000; // 最长 90 秒
            const ping = async () => {
                if (Date.now() > deadline) {
                    clearInterval(this.restartProbeTimerId);
                    this.restartProbeActive = false;
                    ui.showToast('重启超时，请手动刷新页面', 'error');
                    return;
                }
                try {
                    const controller = new AbortController();
                    const t = setTimeout(() => controller.abort(), 1500);
                    const res = await fetch('/api/v1/update/status', { cache: 'no-store', signal: controller.signal });
                    clearTimeout(t);
                    if (!res.ok) throw new Error(String(res.status));
                    const st = await res.json();
                    // 成功条件：不再 pending，且版本已变化（若版本相同也可视为已就绪）
                    if (st && !st.pending_restart && st.current_version) {
                        clearInterval(this.restartProbeTimerId);
                        this.restartProbeActive = false;
                        ui.showToast('重启完成', 'success');
                        setTimeout(() => location.reload(), 800);
                    }
                } catch (e) {
                    // 忽略错误，继续轮询
                }
            };
            // 立即触发一次，随后每 1 秒一次
            setTimeout(() => {
                ping(); // 立即执行第一次
                this.restartProbeTimerId = setInterval(ping, 1000); // 随后每秒执行
            }, 1000);
        },

        setUpdateLoading(isLoading, targetBtn) {
            state.update.loading = isLoading;
            if (targetBtn) ui.setLoading(targetBtn, isLoading);
            this.refreshButtons();
        },

        canApply() {
            const status = state.update.status;
            if (!status) return false;
            if (status.pending_restart) return false;
            const cur = status.current_version || '';
            const lat = status.latest_version || '';
            const sameVersion = this.normalizeVer(cur) !== '' && this.normalizeVer(cur) === this.normalizeVer(lat);
            return Boolean(status.update_available && !sameVersion && status.download_url);
        },

        refreshButtons() {
            if (elements.updateApplyBtn) {
                elements.updateApplyBtn.disabled = state.update.loading || !this.canApply();
            }
            if (elements.updateForceBtn) {
                const hasDownload = Boolean(state.update.status?.download_url);
                elements.updateForceBtn.disabled = state.update.loading || !hasDownload;
            }
        },

        // 前端冗余保护：即使后端误报，也以版本号等价判断为准
        normalizeVer(v) {
            if (!v) return '';
            return String(v).trim().toLowerCase().replace(/^v/, '');
        },

        updateStatusUI(status) {
            state.update.status = status;
            if (!elements.updateModule || !status) return;
            const cur = status.current_version || '';
            const lat = status.latest_version || '';
            const sameVersion = this.normalizeVer(cur) !== '' && this.normalizeVer(cur) === this.normalizeVer(lat);
            elements.updateCurrentVersion.textContent = cur || '未知';
            elements.updateLatestVersion.textContent = lat || '--';
            elements.updateTargetInfo.textContent = status.asset_name ? `${status.asset_name} (${status.architecture || '未知'})` : (status.architecture || '未知');
            // 重置内联徽标与横幅
            if (elements.updateInlineBadge) { elements.updateInlineBadge.style.display = 'none'; elements.updateInlineBadge.className = 'badge'; }
            if (elements.updateStatusBanner) elements.updateStatusBanner.style.display = '';
            const effectiveUpdate = status.update_available && !sameVersion;
            elements.updateStatusText.textContent = status.message || (effectiveUpdate ? '发现新版本，可立即更新。' : '当前已是最新版本');
            const lastChecked = status.checked_at ? new Date(status.checked_at) : null;
            elements.updateLastChecked.textContent = lastChecked ? lastChecked.toLocaleString() : '--';
            if (elements.updateApplyBtn) {
                const span = elements.updateApplyBtn.querySelector('span');
                let label = '立即更新';
                if (status.pending_restart) {
                    // 非 Windows：自重启中；Windows：等待手动重启
                    const isWindows = (status.architecture || '').startsWith('windows/');
                    label = isWindows ? '等待重启' : '重启中…';
                } else if (!this.canApply() || sameVersion) label = '已是最新';
                if (span) span.textContent = label;
                elements.updateApplyBtn.dataset.defaultText = label;
            }
            if (elements.updateForceBtn) {
                const span = elements.updateForceBtn.querySelector('span');
                if (span) span.textContent = '强制更新';
                elements.updateForceBtn.dataset.defaultText = '强制更新';
            }
            if (elements.updateCheckBtn) {
                const span = elements.updateCheckBtn.querySelector('span');
                if (span) { span.textContent = '强制检查'; elements.updateCheckBtn.dataset.defaultText = '强制检查'; }
            }
            if (status.pending_restart) {
                const isWindows = (status.architecture || '').startsWith('windows/');
                const msg = isWindows ? '更新已安装，等待手动重启生效。' : '更新已安装，正在自重启…';
                elements.updateStatusText.textContent = msg;
                this.setHint(msg);
            } else if (!effectiveUpdate) {
                // 已是最新：在“最新版本”行右侧显示小徽标，隐藏“立即更新”按钮与冗余横幅
                if (elements.updateInlineBadge) {
                    elements.updateInlineBadge.textContent = '已是最新';
                    elements.updateInlineBadge.classList.add('success');
                    elements.updateInlineBadge.style.display = 'inline-flex';
                }
                if (elements.updateApplyBtn) {
                    elements.updateApplyBtn.style.display = 'none';
                }
                if (elements.updateStatusBanner) {
                    elements.updateStatusBanner.style.display = 'none';
                }
            } else if (status.message) {
                // 截断过长信息，避免溢出
                const trimmed = (status.message || '').toString();
                elements.updateStatusText.textContent = trimmed.length > 120 ? trimmed.slice(0, 117) + '…' : trimmed;
                // 有更新：确保“立即更新”按钮可见
                if (elements.updateApplyBtn) {
                    elements.updateApplyBtn.style.display = '';
                }
            }
            this.refreshButtons();

            // v3 提示：仅在 amd64、CPU 支持 v3 且当前不是 v3 构建时显示
            const arch = (status.architecture || '');
            const showV3 = (arch === 'linux/amd64' || arch === 'windows/amd64') && status.amd64_v3_capable && !status.current_is_v3;
            if (elements.updateV3Callout) {
                elements.updateV3Callout.style.display = showV3 ? 'grid' : 'none';
            }
        },

        async refreshStatus(force = false) {
            if (!elements.updateModule) return;
            try {
                const status = force ? await updateApi.forceCheck() : await updateApi.getStatus();
                this.updateStatusUI(status);
            } catch (error) {
                console.error('检查更新失败:', error);
                ui.showToast('检查更新失败，请稍后重试', 'error');
            }
        },

        async forceCheck() {
            if (state.update.loading) return;
            this.setUpdateLoading(true, elements.updateCheckBtn);
            try {
                const status = await updateApi.forceCheck();
                ui.showToast('已刷新最新版本信息', 'success');
                this.updateStatusUI(status);
            } catch (error) {
                console.error('强制检查更新失败:', error);
                ui.showToast('强制检查失败', 'error');
            } finally {
                this.setUpdateLoading(false, elements.updateCheckBtn);
                this.applyAutoSchedule(true);
            }
        },

        async applyUpdate(force = false, button = elements.updateApplyBtn, preferV3 = false) {
            if (state.update.loading) return;
            if (!force && !this.canApply()) return;
            this.setUpdateLoading(true, button || elements.updateApplyBtn);
            try {
                const prevVersion = state.update.status?.current_version || '';
                const result = await updateApi.apply(force, preferV3);
                if (result.installed) {
                    ui.showToast(result.status?.message || '更新已安装，正在自重启…', 'success');
                } else {
                    ui.showToast(result.status?.message || '更新已处理', 'info');
                }
                if (result.status) this.updateStatusUI(result.status);
                // 非 Windows 且已进入 pending_restart，开始监听重启完成
                const isWindows = (result.status?.architecture || '').startsWith('windows/');
                if (!isWindows && result.status?.pending_restart) {
                    this.startRestartWatch(prevVersion);
                }
            } catch (error) {
                console.error('执行更新失败:', error);
                ui.showToast('更新失败，请检查日志', 'error');
            } finally {
                this.setUpdateLoading(false, button || elements.updateApplyBtn);
                this.applyAutoSchedule(true);
                // 若不存在可更新，确保隐藏“立即更新”按钮的显示残留
                const st = state.update.status;
                if (elements.updateApplyBtn && st && !st.update_available) {
                    elements.updateApplyBtn.style.display = 'none';
                }
            }
        }
    };

    const systemInfoManager = {
        parseMetrics(metricsText) {
            const lines = metricsText.split('\n');
            const metrics = { startTime: 0, cpuTime: 0, residentMemory: 0, heapIdleMemory: 0, threads: 0, openFds: 0, grs: 0, goVersion: "N/A" };
            lines.forEach(line => {
                if (line.startsWith('process_start_time_seconds')) { metrics.startTime = parseFloat(line.split(' ')[1]) || 0; }
                else if (line.startsWith('process_cpu_seconds_total')) { metrics.cpuTime = parseFloat(line.split(' ')[1]) || 0; }
                else if (line.startsWith('process_resident_memory_bytes')) { metrics.residentMemory = parseFloat(line.split(' ')[1]) || 0; }
                else if (line.startsWith('go_memstats_heap_idle_bytes')) { metrics.heapIdleMemory = parseFloat(line.split(' ')[1]) || 0; }
                else if (line.startsWith('go_threads')) { metrics.threads = parseInt(line.split(' ')[1]) || 0; }
                else if (line.startsWith('process_open_fds')) { metrics.openFds = parseInt(line.split(' ')[1]) || 0; }
                else if (line.startsWith('go_goroutines')) { metrics.grs = parseInt(line.split(' ')[1]) || 0; }
                else if (line.startsWith('go_info{version="')) { const match = line.match(/go_info{version="([^"]+)"}/); if (match && match[1]) { metrics.goVersion = match[1]; } }
            });
            return metrics;
        },

        update() {
            const data = state.systemInfo;
            const container = elements.systemInfoContainer;
            if (!container || Object.keys(data).length === 0) {
                container.innerHTML = '<p>暂无系统信息</p>';
                return;
            }

            const items = [
                { label: '启动时间', value: data.startTime ? new Date(data.startTime * 1000).toLocaleString() : 'N/A' },
                { label: 'CPU 时间', value: `${data.cpuTime.toFixed(2)} 秒` },
                { label: '常驻内存 (RSS)', value: `${(data.residentMemory / 1024 / 1024).toFixed(2)} MB` },
                { label: '待用堆内存 (Idle)', value: `${(data.heapIdleMemory / 1024 / 1024).toFixed(2)} MB` },
                { label: 'Go 版本', value: data.goVersion, accent: true },
                { label: '线程数', value: data.threads.toLocaleString() },
                { label: '打开文件描述符', value: data.openFds.toLocaleString() },
                { label: 'go_goroutines', value: data.grs.toLocaleString() },
            ];

            container.innerHTML = items.map(item => `
                <div class="info-item">
                    <span class="info-item-label">${item.label}</span>
                    <span class="info-item-value ${item.accent ? 'accent' : ''}">${item.value}</span>
                </div>
            `).join('');
        },

        async load(signal) {
            try {
                const metricsText = await api.getMetrics(signal);
                state.systemInfo = this.parseMetrics(metricsText);
                this.update();
            } catch (error) {
                if (error.name !== 'AbortError') {
                    console.error("Failed to load system info:", error);
                    elements.systemInfoContainer.innerHTML = '<p style="color:var(--color-danger)">系统信息加载失败</p>';
                }
            }
        }
    };


    const aliasManager = {
        async load() {
            try {
                const aliases = await clientnameApi.get();
                const normalizedAliases = {};
                if (typeof aliases === 'object' && aliases !== null) {
                    for (const ip in aliases) {
                        normalizedAliases[normalizeIP(ip)] = aliases[ip];
                    }
                }
                state.clientAliases = normalizedAliases;
            } catch (error) {
                ui.showToast('加载客户端别名失败', 'error');
                state.clientAliases = {};
            }
        },
        async save() {
            try {
                await clientnameApi.update(state.clientAliases);
            } catch (error) {
                throw error;
            }
        },
        getDisplayName: (ip) => {
            const normalizedIp = normalizeIP(ip);
            return state.clientAliases[normalizedIp] || ip;
        },
        getAliasedClientHTML: (ip) => {
            const normalizedIp = normalizeIP(ip);
            return state.clientAliases[normalizedIp] ? `<span class="client-alias" title="IP: ${ip}">${state.clientAliases[normalizedIp]}</span>` : ip;
        },
        getIpByAlias: (alias) => { const searchTerm = alias.toLowerCase(); for (const ip in state.clientAliases) { if (state.clientAliases[ip].toLowerCase() === searchTerm) { return ip; } } return null; },
        set(ip, name) {
            const normalizedIp = normalizeIP(ip);
            if (name) { state.clientAliases[normalizedIp] = name; } else { delete state.clientAliases[normalizedIp]; }
        },
        async saveAll() {
            const aliasItems = elements.aliasListContainer.querySelectorAll('.alias-item');
            let changed = false;
            aliasItems.forEach(item => {
                const ip = item.dataset.ip;
                const input = item.querySelector('input');
                const newValue = input.value.trim();
                const originalValue = input.dataset.originalValue;
                if (newValue !== originalValue) {
                    this.set(ip, newValue);
                    changed = true;
                }
            });
            if (changed) {
                try {
                    await this.save();
                    ui.showToast('所有别名更改已保存', 'success');
                    await updatePageData(false);
                    await this.renderEditableList();
                } catch (error) {
                    ui.showToast('保存别名失败', 'error');
                }
            } else {
                ui.showToast('没有检测到任何更改');
            }
        },
        async renderEditableList() {
            if (!elements.aliasListContainer) return;
            elements.aliasListContainer.innerHTML = '<p>正在加载客户端列表...</p>';
            try {
                const topClients = await api.v2.getTopClients(null, 200);
                const uniqueIps = [...new Set(topClients.map(client => client.key))].sort();
                if (uniqueIps.length === 0) {
                    elements.aliasListContainer.innerHTML = '<p>日志中暂无客户端 IP 记录。</p>';
                    return;
                }
                elements.aliasListContainer.innerHTML = '';
                uniqueIps.forEach(ip => {
                    const item = document.createElement('div');
                    item.className = 'alias-item';
                    item.dataset.ip = ip;
                    const normalizedIp = normalizeIP(ip);
                    const currentAlias = state.clientAliases[normalizedIp] || '';
                    item.innerHTML = `<span style="font-weight:600;">${ip}</span> <input type="text" placeholder="设置别名..." value="${currentAlias}" data-original-value="${currentAlias}">`;
                    elements.aliasListContainer.appendChild(item);
                });
            } catch (error) {
                elements.aliasListContainer.innerHTML = '<p>加载客户端列表失败。</p>';
            }
        },
        async export() {
            try {
                ui.showToast('正在从服务器获取最新配置...');
                const aliasesToExport = await clientnameApi.get();
                const normalizedAliases = {};
                if (typeof aliasesToExport === 'object' && aliasesToExport !== null) {
                    for (const ip in aliasesToExport) {
                        normalizedAliases[normalizeIP(ip)] = aliasesToExport[ip];
                    }
                } else {
                    throw new Error("从服务器返回的数据格式无效");
                }
                const dataStr = JSON.stringify(normalizedAliases, null, 2);
                const blob = new Blob([dataStr], { type: 'application/json' });
                const url = URL.createObjectURL(blob);
                const a = document.createElement('a');
                a.href = url;
                a.download = `mosdns-aliases-${new Date().toISOString().split('T')[0]}.json`;
                document.body.appendChild(a);
                a.click();
                document.body.removeChild(a);
                URL.revokeObjectURL(url);
                ui.showToast('配置已导出', 'success');
            } catch (error) { }
        },
        import(file) {
            const reader = new FileReader();
            reader.onload = async (e) => {
                try {
                    const newAliases = JSON.parse(e.target.result);
                    if (typeof newAliases !== 'object' || newAliases === null || Array.isArray(newAliases)) throw new Error('无效的JSON对象格式');

                    for (const ip in newAliases) {
                        this.set(ip, newAliases[ip]);
                    }

                    ui.showToast('正在上传配置到服务器...');
                    await this.save();

                    await this.renderEditableList();
                    await updatePageData(false);
                    ui.showToast('配置已成功导入并上传', 'success');
                } catch (error) {
                    ui.showToast(`导入失败: ${error.message}`, 'error');
                }
            };
            reader.readAsText(file);
        },
    };

    const historyManager = {
        load: () => {
            const saved = JSON.parse(localStorage.getItem('mosdnsHistory'));
            if (saved) {
                state.history.totalQueries = saved.totalQueries || [];
                state.history.avgDuration = saved.avgDuration || [];
                state.history.timestamps = saved.timestamps || [];
            }
        },
        add(total, avg) {
            state.history.totalQueries.push(total ?? 0);
            state.history.avgDuration.push(avg ?? 0);
            state.history.timestamps.push(Date.now());
            // Ensure all arrays same length
            const maxLen = CONSTANTS.HISTORY_LENGTH;
            if (state.history.totalQueries.length > maxLen) state.history.totalQueries.shift();
            if (state.history.avgDuration.length > maxLen) state.history.avgDuration.shift();
            if (state.history.timestamps.length > maxLen) state.history.timestamps.shift();
            this.save();
        },
        save: () => {
            localStorage.setItem('mosdnsHistory', JSON.stringify(state.history));
        }
    };

    const adjustLogSearchLayout = () => {
        const logSearch = document.getElementById('log-search');
        const originalContainer = document.getElementById('log-search-container-original');
        const headerActions = document.getElementById('log-header-actions');
        const logQueryTab = elements.logQueryTab;

        if (!logSearch || !originalContainer || !headerActions || !logQueryTab) return;

        const isComfortable = document.documentElement.dataset.layout === 'comfortable';
        const isMidDesktop = window.innerWidth <= 1440 && window.innerWidth > CONSTANTS.MOBILE_BREAKPOINT;

        if (isComfortable && isMidDesktop) {
            if (!headerActions.contains(logSearch)) {
                headerActions.prepend(logSearch);
                logQueryTab.classList.add('search-moved-to-header');
            }
        } else {
            if (!originalContainer.contains(logSearch)) {
                originalContainer.prepend(logSearch);
                logQueryTab.classList.remove('search-moved-to-header');
            }
        }
    };

    const themeManager = {
        init() {
            const savedTheme = localStorage.getItem('mosdns-theme') || 'dark';
            const savedColor = localStorage.getItem('mosdns-color') || 'indigo';
            const savedLayout = localStorage.getItem('mosdns-layout') || 'comfortable';
            const savedChartMode = localStorage.getItem('mosdns-chart-mode') || 'integrated';

            this.setTheme(savedTheme, false);
            this.setColor(savedColor, false);
            this.setLayout(savedLayout, false);
            this.setChartMode(savedChartMode, false);

            elements.themeSwitcher?.addEventListener('change', e => this.setTheme(e.target.value));
            elements.layoutSwitcher?.addEventListener('change', e => this.setLayout(e.target.value));
            elements.overviewChartModeToggle?.addEventListener('change', e => this.setChartMode(e.target.checked ? 'independent' : 'integrated'));
            elements.colorSwatches.forEach(swatch => { swatch.addEventListener('click', () => this.setColor(swatch.dataset.color)); });
        },
        setTheme(theme, save = true) {
            elements.html.setAttribute('data-theme', theme);
            if (elements.themeSwitcher) { elements.themeSwitcher.value = theme; }
            if (save) localStorage.setItem('mosdns-theme', theme);
        },
        setColor(color, save = true) {
            elements.html.setAttribute('data-color-scheme', color);
            document.querySelectorAll('.color-swatch').forEach(s => s.classList.remove('active'));
            document.querySelectorAll(`.color-swatch[data-color="${color}"]`).forEach(s => s.classList.add('active'));
            if (save) localStorage.setItem('mosdns-color', color);
        },
        setLayout(layout, save = true) {
            elements.html.setAttribute('data-layout', layout);
            if (elements.layoutSwitcher) { elements.layoutSwitcher.value = layout; }
            if (save) localStorage.setItem('mosdns-layout', layout);
            adjustLogSearchLayout();
        },
        setChartMode(mode, save = true) {
            const statsGrid = document.querySelector('.stats-grid');
            if (statsGrid) {
                if (mode === 'independent') {
                    statsGrid.classList.add('independent-mode');
                    if (elements.independentChartPanel) elements.independentChartPanel.style.display = 'block';
                } else {
                    statsGrid.classList.remove('independent-mode');
                    if (elements.independentChartPanel) elements.independentChartPanel.style.display = 'none';
                }
            }
            if (elements.overviewChartModeToggle) {
                elements.overviewChartModeToggle.checked = (mode === 'independent');
            }
            if (save) localStorage.setItem('mosdns-chart-mode', mode);

            // Re-render charts to fill the new containers if visible
            if (typeof ui !== 'undefined' && ui.updateOverviewStats) {
                requestAnimationFrame(() => ui.updateOverviewStats());
            }
        }
    };

    const animateValue = (element, start, end, duration, decimals = 0) => { if (!element || start === null || end === null) return; if (start === end) { element.textContent = (decimals > 0 ? parseFloat(end).toFixed(decimals) : Math.floor(end).toLocaleString()); return; } let startTimestamp = null; const step = (timestamp) => { if (!startTimestamp) startTimestamp = timestamp; const progress = Math.min((timestamp - startTimestamp) / duration, 1); const current = start + progress * (end - start); element.textContent = (decimals > 0 ? parseFloat(current).toFixed(decimals) : Math.floor(current).toLocaleString()); if (progress < 1) window.requestAnimationFrame(step); }; window.requestAnimationFrame(step); };
    const updateStatChange = (element, prev, curr, isTime = false) => { if (prev === null || curr === null || prev === 0) { element.style.visibility = 'hidden'; return; } const diff = curr - prev; const change = (diff / prev) * 100; if (Math.abs(change) < 0.1) { element.style.visibility = 'hidden'; return; } const direction = isTime ? (diff < 0 ? 'up' : 'down') : (diff > 0 ? 'up' : 'down'); const icon = direction === 'up' ? '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M12 8L18 14H6L12 8Z"></path></svg>' : '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M12 16L6 10H18L12 16Z"></path></svg>'; element.className = `stat-change ${direction}`; element.innerHTML = `${icon} ${Math.abs(change).toFixed(1)}%`; element.style.visibility = 'visible'; };
    const setupGlowEffect = () => { elements.container?.addEventListener('mousemove', (e) => { const card = e.target.closest('.card:not(dialog)'); if (card) { const rect = card.getBoundingClientRect(); card.style.setProperty('--glow-x', `${e.clientX - rect.left}px`); card.style.setProperty('--glow-y', `${e.clientY - rect.top}px`); } }); };

    // 双波段图表生成器 (独立模式使用 - 增强版)
    // EWMA (Exponential Weighted Moving Average) 平滑算法
    const applyEWMA = (data, alpha = 0.4) => {
        if (!data || data.length < 2) return data;
        const smoothed = [data[0]]; // 第一个值保持不变
        for (let i = 1; i < data.length; i++) {
            smoothed[i] = alpha * data[i] + (1 - alpha) * smoothed[i - 1];
        }
        return smoothed;
    };

    const generateDualSparklineSVG = (data1, data2, timestamps, width = 800, height = 200) => {
        if (!data1 || data1.length < 2 || !data2 || data2.length < 2) return '';

        const isSmall = width < 500;
        // Reduce padding on mobile to maximize chart area
        const pad = isSmall
            ? { top: 20, right: 35, bottom: 25, left: 35 }
            : { top: 25, right: 55, bottom: 30, left: 55 };

        const chartW = width - pad.left - pad.right;
        const chartH = height - pad.top - pad.bottom;


        const getPoints = (data, max) => {
            const range = max === 0 ? 1 : max;
            return data.map((d, i) => {
                const x = pad.left + (i / (data.length - 1)) * chartW;
                const y = pad.top + chartH - (d / range) * chartH;
                return `${x.toFixed(1)},${y.toFixed(1)}`;
            });
        };

        // 应用 EWMA 平滑（查询量用 0.4，响应时间用 0.3 更平滑）
        const smoothed1 = applyEWMA(data1, 0.4);
        const smoothed2 = applyEWMA(data2, 0.3);

        const max1 = Math.max(...smoothed1);
        const max2 = Math.max(...smoothed2);

        // 原始数据路径（虚线显示）
        const pointsRaw1 = getPoints(data1, max1);
        const pointsRaw2 = getPoints(data2, max2);
        const pathRaw1 = `M ${pointsRaw1.join(' L ')}`;
        const pathRaw2 = `M ${pointsRaw2.join(' L ')}`;

        // 平滑数据路径（实线显示）
        const points1 = getPoints(smoothed1, max1);
        const points2 = getPoints(smoothed2, max2);
        const path1 = `M ${points1.join(' L ')}`;
        const path2 = `M ${points2.join(' L ')}`;


        const area1 = `${path1} L ${pad.left + chartW},${pad.top + chartH} L ${pad.left},${pad.top + chartH} Z`;
        const area2 = `${path2} L ${pad.left + chartW},${pad.top + chartH} L ${pad.left},${pad.top + chartH} Z`;

        const tStart = timestamps && timestamps[0] ? new Date(timestamps[0]).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '';
        const tEnd = timestamps && timestamps[timestamps.length - 1] ? new Date(timestamps[timestamps.length - 1]).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '';

        return `<svg viewBox="0 0 ${width} ${height}" preserveAspectRatio="none" style="overflow:visible; font-family: sans-serif; font-size: 10px;">
                <defs>
                    <linearGradient id="grad-primary" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="var(--color-accent-primary)" stop-opacity="0.15"/><stop offset="1" stop-color="var(--color-accent-primary)" stop-opacity="0"/></linearGradient>
                    <linearGradient id="grad-amber" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#f59e0b" stop-opacity="0.15"/><stop offset="1" stop-color="#f59e0b" stop-opacity="0"/></linearGradient>
                </defs>
                <g stroke="var(--color-border)" stroke-width="1" stroke-dasharray="4 4" opacity="0.4">
                    <line x1="${pad.left}" y1="${pad.top}" x2="${width - pad.right}" y2="${pad.top}"/>
                    <line x1="${pad.left}" y1="${pad.top + chartH / 2}" x2="${width - pad.right}" y2="${pad.top + chartH / 2}"/>
                    <line x1="${pad.left}" y1="${pad.top + chartH}" x2="${width - pad.right}" y2="${pad.top + chartH}"/>
                </g>
                <path d="${area1}" fill="url(#grad-primary)" />
                <path d="${area2}" fill="url(#grad-amber)" />
                <!-- 原始数据（虚线，半透明） -->
                <path d="${pathRaw1}" fill="none" stroke="var(--color-accent-primary)" stroke-width="1" stroke-opacity="0.3" stroke-dasharray="3,3" vector-effect="non-scaling-stroke"/>
                <path d="${pathRaw2}" fill="none" stroke="#f59e0b" stroke-width="1" stroke-opacity="0.3" stroke-dasharray="3,3" vector-effect="non-scaling-stroke"/>
                <!-- 平滑数据（实线，粗） -->
                <path d="${path1}" fill="none" stroke="var(--color-accent-primary)" stroke-width="2" stroke-linecap="round" vector-effect="non-scaling-stroke"/>
                <path d="${path2}" fill="none" stroke="#f59e0b" stroke-width="2" stroke-linecap="round" vector-effect="non-scaling-stroke"/>
                <g fill="var(--color-text-secondary)" text-anchor="end" style="font-weight:500; font-size:11px;">
                    <text x="${pad.left - 6}" y="${pad.top + 4}" fill="var(--color-accent-primary)">${Math.round(max1)}</text>
                    <text x="${pad.left - 6}" y="${pad.top + chartH + 4}" fill="var(--color-accent-primary)">0</text>
                </g>
                <g fill="var(--color-text-secondary)" text-anchor="start" style="font-weight:500; font-size:11px;">
                    <text x="${width - pad.right + 6}" y="${pad.top + 4}" fill="#f59e0b">${Math.round(max2)}</text>
                    <text x="${width - pad.right + 6}" y="${pad.top + chartH + 4}" fill="#f59e0b">0</text>
                </g>
                <g fill="var(--color-text-secondary)" style="font-size:11px;">
                    <text x="${pad.left}" y="${height - 5}" text-anchor="start">${tStart}</text>
                    <text x="${width - pad.right}" y="${height - 5}" text-anchor="end">${tEnd}</text>
                </g>
            </svg>`;
    };

    const generateSparklineSVG = (data, isFloat = false, width = 300, height = 60) => {
        if (!data || data.length < 2) return '';

        // 应用 EWMA 平滑
        const smoothed = applyEWMA(data.map(Number), isFloat ? 0.3 : 0.4);

        const maxVal = Math.max(...smoothed);
        const minVal = Math.min(...smoothed);
        const range = maxVal - minVal === 0 ? 1 : maxVal - minVal;

        const points = smoothed.map((d, i) => {
            const x = (i / (smoothed.length - 1)) * width;
            const y = height - ((d - minVal) / range) * height;
            return `${x.toFixed(2)},${y.toFixed(2)}`;
        });

        const pathD = `M ${points.join(' L ')}`;
        const fillPathD = `${pathD} L ${width},${height} L 0,${height} Z`;

        return `<svg viewBox="0 0 ${width} ${height}" preserveAspectRatio="none"><defs><linearGradient id="sparkline-gradient" x1="0%" y1="0%" x2="0%" y2="100%"><stop offset="0%" stop-color="var(--color-accent-primary)" stop-opacity="0.5" /><stop offset="100%" stop-color="var(--color-accent-primary)" stop-opacity="0" /></linearGradient></defs><path d="${fillPathD}" fill="url(#sparkline-gradient)" /><path d="${pathD}" class="sparkline-path" fill="none" /></svg>`;
    };

    const renderTable = (tbody, data, renderRow, tableType) => {
        if (!tbody) return;
        const placeholder = tbody.closest('.card')?.querySelector('.lazy-placeholder');
        if (placeholder) placeholder.style.display = 'none';
        tbody.innerHTML = '';
        if (!data || data.length === 0) {
            let message = '请确保审计功能已开启。';
            let ctaButton = '<button class="button primary tab-link-action" data-tab="system-control">前往系统控制</button>';
            if (tableType === 'log-query' && state.currentLogSearchTerm?.query) {
                message = `没有找到与 "<strong>${state.currentLogSearchTerm.query}</strong>" 匹配的记录。`;
                ctaButton = '';
            } else if (!state.isCapturing && tableType !== 'adguard' && tableType !== 'diversion') {
                message = '审计功能当前已停止。';
            } else if (tableType === 'adguard' || tableType === 'diversion') {
                message = '暂无规则，请点击 "添加规则" 按钮新建一个。';
                ctaButton = '';
            } else if (tableType === 'lazy') {
                message = '没有可显示的数据。';
                ctaButton = '';
            }
            let colspan = tbody.closest('table').querySelectorAll('thead th').length || 2;
            // Fix mobile layout offset for adguard and diversion modules
            if (tableType === 'adguard' || tableType === 'diversion') {
                colspan = 6;
            }
            const emptyRow = document.createElement('tr');
            emptyRow.className = 'empty-state-row';
            emptyRow.innerHTML = `<td colspan="${colspan}"><div class="empty-state-content"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M21.71,3.29C21.32,2.9,20.69,2.9,20.3,3.29L3.29,20.3c-0.39,0.39-0.39,1.02,0,1.41C3.48,21.9,3.74,22,4,22s0.52-0.1,0.71-0.29L21.71,4.7C22.1,4.31,22.1,3.68,21.71,3.29z M12,2C6.48,2,2,6.48,2,12s4.48,10,10,10,10-4.48 10-10S17.52,2,12,2z M12,20c-4.41,0-8-3.59-8-8c0-2.33,1-4.45,2.65-5.92l11.27,11.27C16.45,19,14.33,20,12,20z"></path></svg><strong>暂无数据</strong><p>${message}</p>${ctaButton}</div></td>`;
            tbody.appendChild(emptyRow);
            return;
        }
        const fragment = document.createDocumentFragment();
        data.forEach((item, index) => {
            const row = renderRow(item, index);
            row.classList.add('animate-in');
            row.style.animationDelay = `${index * 20}ms`;
            fragment.appendChild(row);
        });
        tbody.appendChild(fragment);
    };

    function renderSkeletonRows(tbody, rowCount, colCount) {
        tbody.innerHTML = '';
        const fragment = document.createDocumentFragment();
        for (let i = 0; i < rowCount; i++) {
            const tr = document.createElement('tr');
            tr.className = 'skeleton-row';
            let cells = '';
            for (let j = 0; j < colCount; j++) {
                cells += '<td><div class="skeleton"></div></td>';
            }
            tr.innerHTML = cells;
            fragment.appendChild(tr);
        }
        tbody.appendChild(fragment);
    }

    const renderTopDomains = (data) => renderTable(elements.topDomainsBody, data, (item, index) => {
        const tr = document.createElement('tr');
        tr.dataset.rankIndex = index;
        tr.dataset.rankSource = 'domain';

        if (state.isMobile) {
            tr.innerHTML = `
                <td>
                    <div class="mobile-log-row" style="grid-template-areas: 'domain time'; gap: 0.5rem 1rem;">
                        <div class="domain" title="${item.key}">${item.key}</div>
                        <div class="time" style="font-size: 1rem; font-weight: 600;">
                            <a href="#log-query" class="clickable-link" data-filter-value="${item.key}">${item.count.toLocaleString()}</a>
                        </div>
                    </div>
                </td>`;
        } else {
            tr.innerHTML = `
                <td><span class="truncate-text" title="${item.key}">${item.key}</span></td>
                <td class="text-right"><a href="#log-query" class="clickable-link" data-filter-value="${item.key}">${item.count.toLocaleString()}</a></td>`;
        }
        return tr;
    }, 'lazy');

    const renderTopClients = (data) => renderTable(elements.topClientsBody, data, (item, index) => {
        const tr = document.createElement('tr');
        tr.dataset.rankIndex = index;
        tr.dataset.rankSource = 'client';

        if (state.isMobile) {
            tr.innerHTML = `
                 <td>
                    <div class="mobile-log-row" style="grid-template-areas: 'domain time'; gap: 0.5rem 1rem;">
                        <div class="domain">${aliasManager.getAliasedClientHTML(item.key)}</div>
                        <div class="time" style="font-size: 1rem; font-weight: 600;">
                           <a href="#log-query" class="clickable-link" data-exact-search="true" data-filter-value="${item.key}">${item.count.toLocaleString()}</a>
                        </div>
                    </div>
                </td>`;
        } else {
            tr.innerHTML = `
                <td>${aliasManager.getAliasedClientHTML(item.key)}</td>
                <td class="text-right"><a href="#log-query" class="clickable-link" data-exact-search="true" data-filter-value="${item.key}">${item.count.toLocaleString()}</a></td>`;
        }
        return tr;
    }, 'lazy');

    const renderSlowestQueries = (data) => renderTable(elements.slowestQueriesBody, data, renderSlowestQueryItemHTML, 'lazy');

    const chartColors = ['#6d9dff', '#f778ba', '#2dd4bf', '#fb923c', '#a78bfa', '#fde047', '#ff8c8c', '#ef4444', '#f97316', '#f59e0b', '#84cc16', '#10b981', '#06b6d4', '#3b82f6', '#6366f1', '#8b5cf6', '#d946ef', '#f43f5e', '#64748b'];
    const renderDonutChart = (data) => {
        const placeholder = elements.shuntResultsBody.querySelector('.lazy-placeholder');
        if (placeholder) placeholder.style.display = 'none';
        if (!data || data.length === 0) {
            elements.shuntResultsBody.innerHTML = `<div class="empty-state-content" style="padding: 2rem 0;"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M21.71,3.29C21.32,2.9,20.69,2.9,20.3,3.29L3.29,20.3c-0.39,0.39-0.39,1.02,0,1.41C3.48,21.9,3.74,22,4,22s0.52-0.1,0.71-0.29L21.71,4.7C22.1,4.31,22.1,3.68,21.71,3.29z M12,2C6.48,2,2,6.48,2,12s4.48,10,10,10,10-4.48 10-10S17.52,2,12,2z M12,20c-4.41,0-8-3.59-8-8c0-2.33,1-4.45,2.65-5.92l11.27,11.27C16.45,19,14.33,20,12,20z"></path></svg><strong>暂无数据</strong><p>没有检测到分流结果。</p></div>`;
            return;
        }
        const total = data.reduce((sum, item) => sum + item.count, 0);
        const radius = 72; const circumference = 2 * Math.PI * radius; let offset = 0; state.shuntColors = {};
        const paths = data.map((item, index) => {
            const percent = (item.count / total);
            const strokeDashoffset = circumference - (percent * circumference);
            const color = chartColors[index % chartColors.length];
            state.shuntColors[item.key] = color;
            const path = `<circle cx="80" cy="80" r="${radius}" fill="transparent" stroke="${color}" stroke-width="16" stroke-dasharray="${circumference}" stroke-dashoffset="${strokeDashoffset}" transform="rotate(${offset * 360} 80 80)"></circle>`;
            offset += percent;
            return path;
        }).join('');
        const legend = data.map((item, index) => {
            const percent = ((item.count / total) * 100).toFixed(1);
            const color = chartColors[index % chartColors.length];
            return `<li class="donut-legend-item"><span class="legend-color-box" style="background-color: ${color};"></span><span class="legend-label truncate-text" title="${item.key}">${item.key}</span><span class="legend-value">${item.count.toLocaleString()} (${percent}%)</span></li>`;
        }).join('');
        elements.shuntResultsBody.innerHTML = `<div class="donut-chart-wrapper"><div class="donut-chart"><svg viewBox="0 0 160 160">${paths}</svg><div class="donut-chart-center-text"><div class="total">${total.toLocaleString()}</div><div class="label">总计</div></div></div><ul class="donut-legend">${legend}</ul></div>`;
    };

    let updateController;
    async function updatePageData(forceAll = false) {
        if (state.isUpdating) return;
        state.isUpdating = true;
        if (updateController) updateController.abort();
        updateController = new AbortController();
        const { signal } = updateController;
        ui.setLoading(elements.globalRefreshBtn, true);
        const activeTab = document.querySelector('.tab-link.active')?.dataset.tab;
        try {
            // 概览页首屏：尽量少拉数据，避免阻塞渲染
            const shallowOnOverview = (activeTab === 'overview' && !forceAll);
            const basePromises = [
                api.getStatus(signal),
                api.getCapacity(signal),
                api.v2.getStats(signal)
            ];
            if (!shallowOnOverview) basePromises.push(api.v2.getDomainSetRank(signal, 100));

            const results = await Promise.allSettled(basePromises);
            const statusRes = results[0];
            const capacityRes = results[1];
            const statsRes = results[2];
            const domainSetRankRes = results[3]; // 只有在非浅加载时才存在

            ui.updateStatus(statusRes.status === 'fulfilled' ? statusRes.value?.capturing : null);
            ui.updateCapacity(capacityRes.status === 'fulfilled' ? capacityRes.value?.capacity : null);

            if (statsRes.status === 'fulfilled' && statsRes.value) {
                const stats = statsRes.value;
                state.data.totalQueries.previous = state.data.totalQueries.current === null ? stats.total_queries : state.data.totalQueries.current;
                state.data.avgDuration.previous = state.data.avgDuration.current === null ? stats.average_duration_ms : state.data.avgDuration.current;
                state.data.totalQueries.current = stats.total_queries;
                state.data.avgDuration.current = stats.average_duration_ms;
                ui.updateOverviewStats();
                historyManager.add(stats.total_queries, stats.average_duration_ms);
            }

            if (domainSetRankRes && domainSetRankRes.status === 'fulfilled') {
                state.domainSetRank = domainSetRankRes.value || [];
                renderDonutChart(state.domainSetRank);
            }

            // 系统控制页刷新逻辑
            if (activeTab === 'system-control') {
                const sysPromises = [];
                
                // [新增] 只要在系统页，自动刷新时也更新上游DNS数据 (因为它包含动态的监控指标)
                if (typeof upstreamManager !== 'undefined') {
                    sysPromises.push(upstreamManager.loadData());
                }

                // 其他重数据仅在手动刷新(forceAll)时加载
                if (forceAll) {
                    sysPromises.push(state.requery.pollId ? Promise.resolve() : requeryManager.updateStatus(signal));
                    sysPromises.push(updateDomainListStats(signal));
                    sysPromises.push(cacheManager.updateStats(signal));
                    sysPromises.push(switchManager.loadStatus(signal));
                    sysPromises.push(systemInfoManager.load(signal));
                    sysPromises.push(updateManager.refreshStatus(false));
                }
                
                if (sysPromises.length > 0) {
                    await Promise.allSettled(sysPromises);
                }
            }

            if (forceAll) {
                const [topDomainsRes, topClientsRes, slowestRes] = await Promise.allSettled([api.v2.getTopDomains(signal, 100), api.v2.getTopClients(signal, 100), api.v2.getSlowest(signal, 100)]);

                if (topDomainsRes.status === 'fulfilled') { state.topDomains = topDomainsRes.value || []; renderTopDomains(state.topDomains); }
                if (topClientsRes.status === 'fulfilled') { state.topClients = topClientsRes.value || []; renderTopClients(state.topClients); }
                if (slowestRes.status === 'fulfilled') { state.slowestQueries = slowestRes.value || []; renderSlowestQueries(state.slowestQueries); }
            }
            state.lastUpdateTime = new Date();
            updateLastUpdated();
            if (activeTab === 'log-query') await fetchAndRenderLogs(1, false);
            else if (activeTab === 'rules') {
                const activeSubTab = document.querySelector('#rules-tab .sub-nav-link.active').dataset.subTab;
                if (activeSubTab === 'list-mgmt' && !state.listManagerInitialized) {
                    listManager.init();
                } else if (activeSubTab === 'adguard' && state.adguardRules.length === 0) {
                    await adguardManager.load();
                } else if (activeSubTab === 'diversion' && state.diversionRules.length === 0) {
                    await diversionManager.load();
                }
            }
        } catch (error) { if (error.name !== 'AbortError') console.error("Page update failed:", error); }
        finally {
            ui.setLoading(elements.globalRefreshBtn, false);
            state.isUpdating = false;
        }
    }

    let logRequestController;
    async function fetchAndRenderLogs(page = 1, append = false) {
        if (state.isLogLoading && !append) return;
        state.isLogLoading = true;
        if (elements.logLoader) elements.logLoader.style.display = 'block';
        if (!append) renderSkeletonRows(elements.logTableBody, Math.min(20, CONSTANTS.LOGS_PER_PAGE), state.isMobile ? 1 : 5);
        if (!append && logRequestController) logRequestController.abort();
        logRequestController = new AbortController();
        const params = { page, limit: CONSTANTS.LOGS_PER_PAGE, q: state.currentLogSearchTerm.query, exact: state.currentLogSearchTerm.exact };
        try {
            const response = await api.v2.getLogs(logRequestController.signal, params);
            if (!response?.pagination) throw new Error("Invalid response from logs API");
            const { pagination, logs } = response;
            state.logPaginationInfo = pagination;
            state.currentLogPage = pagination.current_page;
            if (!append) ui.updateSearchResultsInfo(pagination);
            ui.renderLogTable(logs || [], append);
        } catch (error) {
            if (error.name !== 'AbortError') { console.error("Failed to fetch logs:", error); ui.showToast('获取日志失败', 'error'); }
        } finally { state.isLogLoading = false; if (elements.logLoader) elements.logLoader.style.display = 'none'; }
    }

    const tableSorter = {
        init() { if (elements.logTableHead) elements.logTableHead.addEventListener('click', this.handleSort.bind(this)); this.updateHeaders(); },
        handleSort(e) { const th = e.target.closest('th[data-sortable]'); if (!th) return; const key = th.dataset.sortKey; if (state.logSort.key === key) { state.logSort.order = state.logSort.order === 'asc' ? 'desc' : 'asc'; } else { state.logSort.key = key; state.logSort.order = 'desc'; } this.sortLogs(); this.updateHeaders(); },
        sortLogs() {
            const { key, order } = state.logSort;
            const tbody = elements.logTableBody;
            const rows = Array.from(tbody.querySelectorAll('tr[data-log-index]'));
            if (rows.length === 0) return;
            rows.sort((a, b) => {
                const logA = state.displayedLogs[parseInt(a.dataset.logIndex, 10)];
                const logB = state.displayedLogs[parseInt(b.dataset.logIndex, 10)];
                if (!logA || !logB) return 0;
                let valA = logA[key], valB = logB[key];
                if (typeof valA === 'string') { valA = valA.toLowerCase(); valB = valB.toLowerCase(); }
                const result = valA < valB ? -1 : (valA > valB ? 1 : 0);
                return order === 'asc' ? result : -result;
            });
            const fragment = document.createDocumentFragment();
            rows.forEach(row => fragment.appendChild(row));
            tbody.appendChild(fragment);
        },
        updateHeaders() { document.querySelectorAll('#log-table-head th[data-sortable]').forEach(th => { th.classList.remove('sorted'); const indicator = th.querySelector('.sort-indicator'); if (indicator) { if (th.dataset.sortKey === state.logSort.key) { th.classList.add('sorted'); indicator.textContent = state.logSort.order === 'asc' ? '▲' : '▼'; } else { indicator.textContent = ' '; } } }); }
    };

    function applyLogFilterAndRender() {
        if (!elements.logSearch) return;
        const rawSearchTerm = elements.logSearch.value.trim();
        let query = rawSearchTerm, exact = false;
        if (rawSearchTerm.startsWith('"') && rawSearchTerm.endsWith('"')) {
            query = rawSearchTerm.slice(1, -1);
            exact = true;
        } else if (!exact) {
            const ipFromAlias = aliasManager.getIpByAlias(query);
            if (ipFromAlias) query = ipFromAlias;
        }
        state.currentLogSearchTerm = { query, exact };
        fetchAndRenderLogs(1, false);
    }

    function loadMoreLogs() { if (state.isLogLoading || !state.logPaginationInfo || state.currentLogPage >= state.logPaginationInfo.total_pages) return; fetchAndRenderLogs(state.currentLogPage + 1, true); }
    function formatDate(isoString) { return isoString ? new Date(isoString).toLocaleString('zh-CN', { hour12: false, year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit' }).replace(/\//g, '-') : 'N/A'; }
    function formatRelativeTime(isoString) { if (!isoString) return 'N/A'; const diffInSeconds = Math.max(0, Math.round((new Date() - new Date(isoString)) / 1000)); if (diffInSeconds < 5) return '刚刚'; if (diffInSeconds < 60) return `${diffInSeconds}秒前`; if (diffInSeconds < 3600) return `${Math.floor(diffInSeconds / 60)}分钟前`; if (diffInSeconds < 86400) return `${Math.floor(diffInSeconds / 3600)}小时前`; if (diffInSeconds < 86400 * 2) return `昨天`; return new Date(isoString).toLocaleDateString('zh-CN', { month: '2-digit', day: '2-digit' }); }

    function updateLastUpdated() {
        if (elements.lastUpdated) {
            if (state.lastUpdateTime) {
                const relativeTime = formatRelativeTime(state.lastUpdateTime.toISOString());
                elements.lastUpdated.textContent = state.isMobile ? relativeTime : `上次更新于 ${relativeTime}`;
            } else {
                elements.lastUpdated.textContent = '';
            }
        }
    }

    function createInteractiveLine(value, copyValue, filterValue, isExact = false, isSmall = false) {
        const copyIcon = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><path d="M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z"></path></svg>`;
        const filterIcon = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><path d="M10 18h4v-2h-4v2zM3 6v2h18V6H3zm3 7h12v-2H6v2z"></path></svg>`;
        const textElement = isSmall ? `<small>${value}</small>` : `<span>${value}</span>`;
        return `
            ${textElement}
            <span class="interactive-btn-container">
                <button class="copy-btn" data-copy-value="${copyValue}" title="复制">${copyIcon}</button>
                <button class="filter-btn" data-filter-value="${filterValue}" data-exact-search="${isExact}" title="过滤此项">${filterIcon}</button>
            </span>`;
    }

    const tooltipManager = (() => {
        const tooltip = elements.tooltip;
        let showTimeout, hideTimeout;
        const _positionAndShow = (targetElement, html) => {
            tooltip.innerHTML = html;
            tooltip.style.visibility = 'hidden';
            tooltip.classList.add('visible');
            requestAnimationFrame(() => {
                const targetRect = targetElement.getBoundingClientRect();
                const tooltipRect = tooltip.getBoundingClientRect();
                let top = targetRect.bottom + 10,
                    left = targetRect.left + (targetRect.width / 2) - (tooltipRect.width / 2);
                if (top + tooltipRect.height > window.innerHeight - 10) top = targetRect.top - tooltipRect.height - 10;
                if (left < 10) left = 10; else if (left + tooltipRect.width > window.innerWidth - 10) left = window.innerWidth - tooltipRect.width - 10;
                tooltip.style.top = `${top}px`;
                tooltip.style.left = `${left}px`;
                tooltip.style.visibility = 'visible';
            });
        };
        const _display = (targetElement) => {
            const logIndex = targetElement.dataset.logIndex ? parseInt(targetElement.dataset.logIndex, 10) : null;
            const rankIndex = targetElement.dataset.rankIndex ? parseInt(targetElement.dataset.rankIndex, 10) : null;
            const source = targetElement.dataset.logSource || targetElement.dataset.rankSource;
            let data;
            if (source === 'slowest' && logIndex !== null) data = state.slowestQueries[logIndex];
            else if (source === 'domain' && rankIndex !== null) data = state.topDomains[rankIndex];
            else if (source === 'client' && rankIndex !== null) data = state.topClients[rankIndex];
            else if (source === 'domain_set' && rankIndex !== null) data = state.domainSetRank[rankIndex];
            else if (logIndex !== null) data = state.displayedLogs[logIndex];
            if (!data) return;
            _positionAndShow(targetElement, getTooltipHTML(data, source));
        };
        const _hide = () => { tooltip.classList.remove('visible'); tooltip.addEventListener('transitionend', () => { if (!tooltip.classList.contains('visible')) tooltip.style.visibility = 'hidden'; }, { once: true }); };
        return {
            handleTriggerEnter(targetElement) { clearTimeout(hideTimeout); showTimeout = setTimeout(() => _display(targetElement), CONSTANTS.TOOLTIP_SHOW_DELAY); },
            handleTriggerLeave() { clearTimeout(showTimeout); hideTimeout = setTimeout(_hide, CONSTANTS.TOOLTIP_HIDE_DELAY); },
            handleTooltipEnter() { clearTimeout(hideTimeout); },
            handleTooltipLeave() { hideTimeout = setTimeout(_hide, CONSTANTS.TOOLTIP_HIDE_DELAY); },
            show(targetElement) { _display(targetElement); },
            hide() { _hide(); },
            showText(targetElement, text) {
                if (!text) return;
                clearTimeout(hideTimeout);
                showTimeout = setTimeout(() => {
                    const safe = String(text).replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/\n/g, '<br>');
                    _positionAndShow(targetElement, `<div style=\"max-width: 40ch; line-height: 1.5;\">${safe}</div>`);
                }, CONSTANTS.TOOLTIP_SHOW_DELAY);
            }
        };
    })();

    // 绑定 info-icon 的提示（功能开关说明）
    function bindInfoIconTooltips() {
        const scope = elements.featureSwitchesModule || document;
        const icons = scope.querySelectorAll('.info-icon');
        icons.forEach(icon => {
            if (icon.dataset.tooltipBound) return;
            icon.dataset.tooltipBound = '1';
            const text = icon.getAttribute('title') || icon.dataset.tip || '';
            icon.setAttribute('aria-label', text);
            icon.addEventListener('mouseenter', () => tooltipManager.showText(icon, text));
            icon.addEventListener('mouseleave', () => tooltipManager.hide());
            icon.addEventListener('focus', () => tooltipManager.showText(icon, text));
            icon.addEventListener('blur', () => tooltipManager.hide());
        });
    }

    // 全局委托绑定，避免动态渲染遗漏（如切换 Tab/刷新模块后失效）
    function mountGlobalInfoIconDelegation() {
        if (document.documentElement.dataset.infoIconDelegationMounted === '1') return;
        document.documentElement.dataset.infoIconDelegationMounted = '1';
        const getIcon = (e) => e.target.closest && e.target.closest('.info-icon');
        document.addEventListener('mouseover', (e) => {
            const icon = getIcon(e);
            if (!icon) return;
            const text = icon.getAttribute('title') || icon.dataset.tip || icon.getAttribute('aria-label') || '';
            tooltipManager.showText(icon, text);
        }, true);
        document.addEventListener('mouseout', (e) => {
            if (getIcon(e)) tooltipManager.hide();
        }, true);
        document.addEventListener('focusin', (e) => {
            const icon = getIcon(e);
            if (!icon) return;
            const text = icon.getAttribute('title') || icon.dataset.tip || icon.getAttribute('aria-label') || '';
            tooltipManager.showText(icon, text);
        });
        document.addEventListener('focusout', (e) => {
            if (getIcon(e)) tooltipManager.hide();
        });
    }

    function getDetailContentHTML(data) {
        if (!data) return '';
        const queryInfo = {}, responseInfo = {}; let answers = [];
        const { ra, aa, tc } = data.response_flags || {}; const flagItems = [ra && 'RA', aa && 'AA', tc && 'TC'].filter(Boolean);
        queryInfo['域名'] = createInteractiveLine(data.query_name, data.query_name, data.query_name, false);
        queryInfo['时间'] = `<span>${formatDate(data.query_time)}</span>`;
        queryInfo['客户端'] = createInteractiveLine(aliasManager.getDisplayName(data.client_ip) + ` (${data.client_ip})`, data.client_ip, data.client_ip, true);
        queryInfo['类型'] = `<span>${data.query_type || 'N/A'}</span>`;
        if (data.query_class) queryInfo['类别'] = `<span>${data.query_class}</span>`;
        if (data.domain_set) queryInfo['分流规则'] = createInteractiveLine(data.domain_set, data.domain_set, data.domain_set, true);
        if (data.trace_id) queryInfo['Trace ID'] = createInteractiveLine(data.trace_id, data.trace_id, data.trace_id, true);

        responseInfo['耗时'] = `<span>${data.duration_ms.toFixed(2)} ms</span>`;
        let statusText = data.response_code || 'N/A';
        if (data.is_blocked) statusText += ' (已拦截)';
        responseInfo['状态'] = `<span>${statusText}</span>`;
        if (flagItems.length) responseInfo['标志'] = `<span>${flagItems.join(', ')}</span>`;
        answers = data.answers || [];
        const buildList = (obj) => Object.entries(obj).map(([key, value]) => `<li><strong>${key}</strong> ${value}</li>`).join('');
        let html = '<h5>查询信息</h5><ul>' + buildList(queryInfo) + '</ul>';
        html += '<h5>响应信息</h5><ul>' + buildList(responseInfo) + '</ul>';
        if (answers.length) html += `<h5>应答记录 (${answers.length})</h5><ul>${answers.map(ans => `<li><strong>${ans.type}</strong> <span>${ans.data}<br><small>(TTL: ${ans.ttl}s)</small></span></li>`).join('')}</ul>`;
        return html;
    }

    const getContrastingTextColor = (hexColor) => { if (!hexColor || hexColor.length < 4) return '#0f172a'; let r = parseInt(hexColor.substr(1, 2), 16), g = parseInt(hexColor.substr(3, 2), 16), b = parseInt(hexColor.substr(5, 2), 16); const yiq = ((r * 299) + (g * 587) + (b * 114)) / 1000; return (yiq >= 128) ? '#0f172a' : '#ffffff'; };
    function getRuleTagHTML(log) { if (!log || !log.domain_set) return ''; const ruleName = log.domain_set; let bgColor = state.shuntColors[ruleName]; if (!bgColor) { let hash = 0; for (let i = 0; i < ruleName.length; i++) hash = ruleName.charCodeAt(i) + ((hash << 5) - hash); bgColor = `hsl(${Math.abs(hash % 360)}, 70%, 45%)`; } const textColor = '#ffffff'; return `<span class="rule-tag" style="background-color: ${bgColor}; color: ${textColor};" title="分流规则: ${ruleName}">${ruleName}</span>`; }
    function getResponseTagHTML(log) { if (!log) return ''; const code = log.response_code || 'UNKNOWN'; let tagClass = 'other'; if (code === 'NOERROR') tagClass = 'noerror'; else if (code === 'NXDOMAIN') tagClass = 'nxdomain'; else if (code === 'SERVFAIL') tagClass = 'servfail'; else if (code === 'REFUSED') tagClass = 'refused'; return `<span class="response-tag ${tagClass}">${code}</span>`; }
    function getResponseSummary(log) { if (!log) return ''; if (log.response_code !== 'NOERROR') return getResponseTagHTML(log); if (log.answers?.length > 0) { const firstIp = log.answers.find(a => a.type === 'A' || a.type === 'AAAA'); const firstCname = log.answers.find(a => a.type === 'CNAME'); let mainText = firstIp?.data ?? firstCname?.data ?? log.answers[0].data; if (mainText.length > 25) mainText = mainText.substring(0, 22) + '...'; if (log.answers.length > 1) mainText += ` (+${log.answers.length - 1})`; return `<span class="truncate-text">${mainText}</span>`; } return '<span>(empty)</span>'; }

    function renderDomainResponseCellHTML(log, source = 'log') {
        // [修改] 移除针对 slowest 的特殊处理，统一显示为 Tag 标签，确保颜色一致性
        const ruleTag = getRuleTagHTML(log);
        return `<div class="domain-response-cell"><span class="domain-name truncate-text" title="${log.query_name}">${log.query_name}</span><div class="response-meta"><span class="response-summary">${getResponseSummary(log)}</span>${ruleTag}</div></div>`;
    }

    function getLogRowClass(log) { if (log.is_blocked) return 'is-blocked'; if (['SERVFAIL', 'NXDOMAIN', 'REFUSED'].includes(log.response_code)) return 'is-fail'; return ''; }

    function renderLogItemHTML(log, globalIndex) {
        const tr = document.createElement('tr'); tr.dataset.logIndex = globalIndex; tr.className = getLogRowClass(log);
        if (state.isMobile) {
            tr.innerHTML = `
                <td>
                    <div class="mobile-log-row">
                        <div class="domain" title="${log.query_name}">${log.query_name}</div>
                        <div class="time">${formatRelativeTime(log.query_time)}</div>
                        <div class="meta">
                            <span class="client">${aliasManager.getDisplayName(log.client_ip)}</span>
                            <span class="duration">${log.duration_ms.toFixed(0)}ms</span>
                            ${getResponseTagHTML(log)}
                            ${getRuleTagHTML(log)}
                        </div>
                    </div>
                </td>`;
        } else {
            tr.innerHTML = `
                <td>${formatRelativeTime(log.query_time)}</td>
                <td>${renderDomainResponseCellHTML(log)}</td>
                <td>${log.query_type}</td>
                <td class="text-center numeric duration-cell">${log.duration_ms.toFixed(2)}</td>
                <td>${aliasManager.getAliasedClientHTML(log.client_ip)}</td>`;
        }
        return tr;
    }

    function renderSlowestQueryItemHTML(log, index) {
        const tr = document.createElement('tr'); tr.dataset.logIndex = index; tr.dataset.logSource = 'slowest'; tr.className = getLogRowClass(log);
        if (state.isMobile) {
            tr.innerHTML = `
                <td>
                    <div class="mobile-log-row">
                        <div class="domain" title="${log.query_name}">${log.query_name}</div>
                        <div class="time">${formatRelativeTime(log.query_time)}</div>
                        <div class="meta">
                            <span class="client">${aliasManager.getDisplayName(log.client_ip)}</span>
                            <span class="duration">${log.duration_ms.toFixed(0)}ms</span>
                            ${getResponseTagHTML(log)}
                            ${getRuleTagHTML(log)}
                        </div>
                    </div>
                </td>`;
        } else {
            tr.innerHTML = `
                <td>${renderDomainResponseCellHTML(log, 'slowest')}</td>
                <td>${aliasManager.getAliasedClientHTML(log.client_ip)}</td>
                <td class="text-right numeric duration-cell">${log.duration_ms.toFixed(2)}</td>`;
        }
        return tr;
    }

    function getTooltipHTML(data, source) {
        if (!data) return '';
        const queryInfo = {}, responseInfo = {}; let answers = [];
        if (['domain', 'client', 'domain_set'].includes(source)) { queryInfo['请求数'] = data.count.toLocaleString(); } else {
            const { ra, aa, tc } = data.response_flags || {}; const flagItems = [ra && 'RA', aa && 'AA', tc && 'TC'].filter(Boolean);
            queryInfo['完整域名'] = createInteractiveLine(data.query_name, data.query_name, data.query_name, false, true);
            queryInfo['精确时间'] = `<small>${formatDate(data.query_time)}</small>`;
            queryInfo['客户端'] = createInteractiveLine(aliasManager.getDisplayName(data.client_ip), data.client_ip, data.client_ip, true, true);
            queryInfo['类型'] = `<small>${data.query_type || 'N/A'}</small>`;
            if (data.query_class) queryInfo['类别'] = `<small>${data.query_class}</small>`;
            if (data.domain_set) queryInfo['规则'] = createInteractiveLine(data.domain_set, data.domain_set, data.domain_set, true, true);
            responseInfo['耗时'] = `<small>${data.duration_ms.toFixed(2)} ms</small>`;
            responseInfo['状态'] = `<small>${data.response_code || 'N/A'}${data.is_blocked ? ' (Blocked)' : ''}</small>`;
            if (flagItems.length) responseInfo['标志'] = `<small>${flagItems.join(', ')}</small>`;
            if (data.trace_id) { queryInfo['Trace ID'] = createInteractiveLine(data.trace_id, data.trace_id, data.trace_id, true, true); }
            answers = data.answers || [];
        }
        const buildList = (obj) => Object.entries(obj).map(([key, value]) => `<li><strong>${key}:</strong> ${value}</li>`).join('');
        let tooltipHTML = ``;
        if (Object.keys(queryInfo).length) tooltipHTML += `<h5>查询信息</h5><ul>${buildList(queryInfo)}</ul>`;
        if (Object.keys(responseInfo).length) tooltipHTML += `<h5 style="margin-top:0.75rem;">响应信息</h5><ul>${buildList(responseInfo)}</ul>`;
        if (answers.length) tooltipHTML += `<h5 style="margin-top:0.75rem;">应答记录 (${answers.length})</h5><ul>${answers.map(ans => `<li>[${ans.type}] ${ans.data} <small>(TTL: ${ans.ttl}s)</small></li>`).join('')}</ul>`;
        return tooltipHTML;
    }

    const autoRefreshManager = {
        start() { this.stop(); if (state.autoRefresh.enabled && state.autoRefresh.intervalSeconds >= 5) { state.autoRefresh.intervalId = setInterval(() => updatePageData(false), state.autoRefresh.intervalSeconds * 1000); } },
        stop() { clearInterval(state.autoRefresh.intervalId); state.autoRefresh.intervalId = null; },
        updateSettings(enabled, seconds) { state.autoRefresh.enabled = enabled; state.autoRefresh.intervalSeconds = Math.max(seconds, 5); localStorage.setItem('mosdnsAutoRefresh', JSON.stringify({ enabled, intervalSeconds: state.autoRefresh.intervalSeconds })); ui.showToast(`自动刷新已${enabled ? `开启, 频率: ${state.autoRefresh.intervalSeconds}秒` : '关闭'}`, 'success'); this.start(); },
        loadSettings() {
            const saved = JSON.parse(localStorage.getItem('mosdnsAutoRefresh'));
            if (saved) {
                state.autoRefresh.enabled = saved.enabled ?? false;
                state.autoRefresh.intervalSeconds = saved.intervalSeconds || CONSTANTS.DEFAULT_AUTO_REFRESH_INTERVAL;
            } else {
                state.autoRefresh.enabled = false;
                state.autoRefresh.intervalSeconds = CONSTANTS.DEFAULT_AUTO_REFRESH_INTERVAL;
            }
            elements.autoRefreshToggle.checked = state.autoRefresh.enabled;
            elements.autoRefreshIntervalInput.value = state.autoRefresh.intervalSeconds;
            elements.autoRefreshIntervalInput.disabled = !state.autoRefresh.enabled;
        }
    };

    function handleNavigation(targetLink) {
        elements.tabLinks.forEach(link => link.classList.remove('active'));
        targetLink.classList.add('active');
        requestAnimationFrame(() => updateNavSlider(targetLink));
        const newHash = targetLink.getAttribute('href');
        if (window.location.hash !== newHash) history.pushState(null, '', newHash);
        const activeTabId = targetLink.dataset.tab;
        elements.tabContents.forEach(el => el.classList.toggle('active', el.id === `${activeTabId}-tab`));
        // 系统控制页采用懒加载；不在切换时主动拉取重数据，由模块可见时触发
        if (activeTabId === 'log-query' && state.displayedLogs.length === 0) {
            applyLogFilterAndRender();
        } else if (activeTabId === 'rules') {
            const activeSubTab = document.querySelector('#rules-tab .sub-nav-link.active').dataset.subTab;
            if (activeSubTab === 'list-mgmt' && !state.listManagerInitialized) {
                listManager.init();
            } else if (activeSubTab === 'adguard' && state.adguardRules.length === 0) {
                renderSkeletonRows(elements.adguardRulesTbody, 5, state.isMobile ? 1 : 6);
                adguardManager.load();
            } else if (activeSubTab === 'diversion' && state.diversionRules.length === 0) {
                renderSkeletonRows(elements.diversionRulesTbody, 5, state.isMobile ? 1 : 7);
                diversionManager.load();
            }
        }
    }

    function handleResize() {
        const wasMobile = state.isMobile;
        state.isMobile = window.innerWidth <= CONSTANTS.MOBILE_BREAKPOINT;

        if (wasMobile !== state.isMobile) {
            const activeTab = document.querySelector('.tab-link.active')?.dataset.tab;
            if (activeTab === 'log-query') {
                ui.renderLogTable(state.displayedLogs);
            } else if (activeTab === 'overview') {
                renderSlowestQueries(state.slowestQueries);
                renderTopDomains(state.topDomains);
                renderTopClients(state.topClients);
            } else if (activeTab === 'rules') {
                adguardManager.render();
                diversionManager.render();
            } else if (activeTab === 'system-control') {
                cacheManager.renderTable();
            }
            updateLastUpdated();
        }

        if (state.isTouchDevice) elements.body.classList.add('touch'); else elements.body.classList.remove('touch');
        requestAnimationFrame(() => { const activeLink = document.querySelector('.tab-link.active'); if (activeLink) updateNavSlider(activeLink); });
        adjustLogSearchLayout();

        // -- [修改] -- 动态调整系统控制页的列宽比例
        // 目标：增加"域名列表统计"宽度，减小"版本与更新"宽度
        const domainModule = document.getElementById('domain-stats-module');
        if (domainModule) {
            const gridContainer = domainModule.parentElement;
            // 仅在 Grid 布局且非移动端堆叠模式下调整 (通常阈值是 1200px 或 1024px)
            if (gridContainer && window.getComputedStyle(gridContainer).display === 'grid' && window.innerWidth > 1200) {
                // 原比例通常是 1fr 1fr 1fr
                // 调整为: 域名统计(1.3倍) - 系统信息(1倍) - 版本更新(0.8倍)
                gridContainer.style.gridTemplateColumns = '1.3fr 1fr 0.8fr';
            } else if (gridContainer) {
                // 屏幕较窄或移动端时，清除内联样式，回归 CSS 默认的响应式布局
                gridContainer.style.gridTemplateColumns = '';
            }
        }
    }

function renderRuleTable(tbody, rules, mode) {
        // [修复] 移除旧的 mobile-rule-card-layout 类，使用新的 mobile-card-view
        tbody.closest('table').classList.toggle('mobile-card-view', state.isMobile);
        
        const sortedRules = [...rules].sort((a, b) => (a.name || '').localeCompare(b.name || ''));
        renderTable(tbody, sortedRules, (rule, index) => {
            // [修复] 如果是移动端，强制使用新的卡片渲染逻辑
            if (state.isMobile) {
                const tr = document.createElement('tr');
                tr.dataset.ruleId = mode === 'adguard' ? rule.id : rule.name;
                if (rule.type) tr.dataset.ruleType = rule.type;

                const lastUpdated = rule.last_updated && !rule.last_updated.startsWith('0001-01-01') ? formatRelativeTime(rule.last_updated) : '从未';
                
                // 构建卡片内容
                tr.innerHTML = `
                    <td>
                        <div class="mobile-system-card">
                            <div class="card-header">
                                <div style="display:flex; flex-direction:column; max-width:70%;">
                                    <span class="card-title">${rule.name}</span>
                                    <small style="color:var(--color-text-secondary); margin-top:4px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;">${rule.url}</small>
                                </div>
                                <label class="switch">
                                    <input type="checkbox" class="rule-enabled-toggle" ${rule.enabled ? 'checked' : ''}>
                                    <span class="slider"></span>
                                </label>
                            </div>
                            <div class="mobile-stats-grid">
                                <div class="mobile-stat-item">
                                    <span class="mobile-stat-label">规则数</span>
                                    <span class="mobile-stat-value">${(rule.rule_count || 0).toLocaleString()}</span>
                                </div>
                                <div class="mobile-stat-item">
                                    <span class="mobile-stat-label">上次更新</span>
                                    <span class="mobile-stat-value">${lastUpdated}</span>
                                </div>
                                ${mode === 'diversion' ? `
                                <div class="mobile-stat-item" style="grid-column: 1 / -1;">
                                    <span class="mobile-stat-label">类型</span>
                                    <span class="mobile-stat-value"><span class="response-tag other">${rule.type}</span></span>
                                </div>` : ''}
                            </div>
                            <div class="mobile-card-actions">
                                ${mode === 'diversion' && rule.url ? `<button class="button secondary small rule-update-btn" style="flex:1;">更新</button>` : ''}
                                <button class="button secondary small rule-edit-btn" style="flex:1;">编辑</button>
                                <button class="button danger small rule-delete-btn" style="flex:1;">删除</button>
                            </div>
                        </div>
                    </td>
                `;
                return tr;
            } else {
                // PC 端保持原样
                const item = renderRuleTableRow(rule, mode);
                item.dataset.ruleId = mode === 'adguard' ? rule.id : rule.name;
                if (rule.type) item.dataset.ruleType = rule.type;
                return item;
            }
        }, mode);
    }

    function renderRuleTableRow(rule, mode) { const tr = document.createElement('tr'); const lastUpdated = rule.last_updated && !rule.last_updated.startsWith('0001-01-01') ? new Date(rule.last_updated).toLocaleString('zh-CN', { hour12: false }).replace(/\//g, '-') : '—'; let html = `<td class="text-center"><label class="switch"><input type="checkbox" class="rule-enabled-toggle" ${rule.enabled ? 'checked' : ''}><span class="slider"></span></label></td><td>${rule.name}</td>`; if (mode === 'diversion') { html += `<td><span class="response-tag other">${rule.type}</span></td>`; } html += `<td><span class="truncate-text" title="${rule.url}">${rule.url}</span></td><td class="text-right">${(rule.rule_count || 0).toLocaleString()}</td><td>${lastUpdated}</td><td class="text-center"><div style="display: inline-flex; gap: 0.5rem; white-space: nowrap;">`; if (mode === 'diversion' && rule.url) { html += `<button class="button secondary rule-update-btn" style="padding: 0.4rem 0.8rem;" title="更新此规则"><span>更新</span></button>`; } html += `<button class="button secondary rule-edit-btn" style="padding: 0.4rem 0.8rem;"><span>编辑</span></button><button class="button danger rule-delete-btn" style="padding: 0.4rem 0.8rem;"><span>删除</span></button></div></td>`; tr.innerHTML = html; return tr; }

    function renderRuleMobileCard(rule, mode) {
        const card = document.createElement('div'); card.className = 'rule-card';
        const lastUpdated = rule.last_updated && !rule.last_updated.startsWith('0001-01-01') ? formatRelativeTime(rule.last_updated) : '从未';
        let metaHtml = `<span class="url" title="${rule.url}">${rule.url}</span>`;
        if (mode === 'diversion') metaHtml += `<span><span class="response-tag other">${rule.type}</span></span>`;
        metaHtml += `<span><strong>规则数:</strong> ${(rule.rule_count || 0).toLocaleString()}</span><span><strong>更新于:</strong> ${lastUpdated}</span>`;
        let actionsHtml = '';
        if (mode === 'diversion' && rule.url) actionsHtml += `<button class="button secondary rule-update-btn"><span>更新</span></button>`;
        actionsHtml += `<button class="button secondary rule-edit-btn"><span>编辑</span></button><button class="button danger rule-delete-btn"><span>删除</span></button>`;
        card.innerHTML = `<div class="rule-card-toggle"><label class="switch"><input type="checkbox" class="rule-enabled-toggle" ${rule.enabled ? 'checked' : ''}><span class="slider"></span></label></div><div class="rule-card-name">${rule.name}</div><div class="rule-card-meta">${metaHtml}</div><div class="rule-card-actions">${actionsHtml}</div>`;
        return card;
    }

    async function handleAdguardUpdateCheck() { ui.setLoading(elements.checkAdguardUpdatesBtn, true); ui.showToast('已开始在后台更新所有启用的拦截规则...'); try { await api.fetch('/plugins/adguard/update', { method: 'POST' }); ui.showToast('更新请求已发送，5秒后自动刷新列表...', 'success'); await new Promise(resolve => setTimeout(resolve, 5000)); await adguardManager.load(); ui.showToast('拦截规则列表已刷新！', 'success'); } catch (e) { } finally { ui.setLoading(elements.checkAdguardUpdatesBtn, false); } }
    async function handleRuleTableClick(event, mode) { const target = event.target.closest('button, input.rule-enabled-toggle'); if (!target) return; const itemElement = target.closest('[data-rule-id]'); if (!itemElement) return; const id = itemElement.dataset.ruleId; const rules = mode === 'adguard' ? state.adguardRules : state.diversionRules; const rule = rules.find(r => (mode === 'adguard' ? r.id : r.name) === id); if (!rule) return; if (target.matches('.rule-edit-btn')) ui.openRuleModal(mode, rule); else if (target.matches('.rule-delete-btn')) { if (confirm(`确定要删除规则 "${rule.name}" 吗？此操作不可恢复。`)) { ui.setLoading(target, true); try { if (mode === 'adguard') await api.fetch(`/plugins/adguard/rules/${id}`, { method: 'DELETE' }); else await api.fetch(`/plugins/${diversionManager.sdSetInstanceMap[rule.type]}/config/${id}`, { method: 'DELETE' }); ui.showToast(`规则 "${rule.name}" 已删除`); await (mode === 'adguard' ? adguardManager.load() : diversionManager.load()); } catch (e) { console.error(`Failed to delete rule ${id}:`, e); } finally { ui.setLoading(target, false); } } } else if (target.matches('.rule-update-btn')) { ui.setLoading(target, true); ui.showToast(`正在后台更新规则 "${rule.name}"...`); try { await api.fetch(`/plugins/${diversionManager.sdSetInstanceMap[rule.type]}/update/${id}`, { method: 'POST' }); ui.showToast('更新请求已发送, 5秒后自动刷新', 'success'); setTimeout(() => diversionManager.load(), 5000); } catch (e) { } finally { ui.setLoading(target, false); } } else if (target.matches('.rule-enabled-toggle')) { const updatedRule = { ...rule, enabled: target.checked }; target.disabled = true; try { if (mode === 'adguard') await api.fetch(`/plugins/adguard/rules/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(updatedRule) }); else await api.fetch(`/plugins/${diversionManager.sdSetInstanceMap[rule.type]}/config/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(updatedRule) }); rule.enabled = target.checked; ui.showToast(`规则 "${rule.name}" 已${target.checked ? '启用' : '禁用'}`); } catch (error) { target.checked = !target.checked; } finally { target.disabled = false; } } }
    async function handleRuleFormSubmit(event) { event.preventDefault(); ui.setLoading(elements.saveRuleBtn, true); const form = elements.ruleForm; const mode = form.elements['mode'].value; const id = form.elements['id'].value; try { if (mode === 'adguard') { const data = { name: form.elements['name'].value, url: form.elements['url'].value, auto_update: form.elements['auto_update'].checked, update_interval_hours: parseInt(form.elements['update_interval_hours'].value, 10) || 24 }; if (id) { const originalRule = state.adguardRules.find(r => r.id === id); await api.fetch(`/plugins/adguard/rules/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...originalRule, ...data }) }); } else { await api.fetch('/plugins/adguard/rules', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...data, enabled: true }) }); } ui.showToast(`广告拦截规则${id ? '更新' : '添加'}成功`); await adguardManager.load(); } else { const data = { name: form.elements['name'].value, url: form.elements['url'].value, type: form.elements['type'].value, files: form.elements['files'].value, auto_update: form.elements['auto_update'].checked, enable_regexp: form.elements['enable_regexp'] ? form.elements['enable_regexp'].checked : false, update_interval_hours: parseInt(form.elements['update_interval_hours'].value, 10) || 24 }; const pluginTag = diversionManager.sdSetInstanceMap[data.type]; if (!pluginTag) throw new Error('无效的分流规则类型'); if (id) { const originalRule = state.diversionRules.find(r => r.name === id); if (data.name !== id) { if (!confirm(`规则名称已从 "${id}" 更改为 "${data.name}"。\n\n这将删除旧规则并创建一个新规则，确定要继续吗？`)) throw new Error('User cancelled name change.'); await api.fetch(`/plugins/${diversionManager.sdSetInstanceMap[originalRule.type]}/config/${id}`, { method: 'DELETE' }); await api.fetch(`/plugins/${pluginTag}/config/${data.name}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...data, enabled: originalRule.enabled }) }); } else { await api.fetch(`/plugins/${pluginTag}/config/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...originalRule, ...data }) }); } } else { await api.fetch(`/plugins/${pluginTag}/config/${data.name}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...data, enabled: true }) }); } ui.showToast(`分流规则${id ? '更新' : '添加'}成功`); await diversionManager.load(); if (!id || (id && data.name !== id)) { ui.showToast('正在后台获取规则详情...'); setTimeout(() => diversionManager.load(), 5000); } } ui.closeRuleModal(); } catch (err) { console.error(`${mode} form submission failed:`, err); } finally { ui.setLoading(elements.saveRuleBtn, false); } }
    const adguardManager = { async load() { try { state.adguardRules = await api.fetch('/plugins/adguard/rules') || []; } catch (error) { state.adguardRules = []; } this.render(); }, render() { renderRuleTable(elements.adguardRulesTbody, state.adguardRules, 'adguard'); }, };
    const diversionManager = { sdSetInstanceMap: { 'geositecn': 'geosite_cn', 'geositenocn': 'geosite_no_cn', 'geoipcn': 'geoip_cn', 'cuscn': 'cuscn', 'cusnocn': 'cusnocn'  }, async load() { try { const promises = Object.values(this.sdSetInstanceMap).map(tag => api.fetch(`/plugins/${tag}/config`)); const results = await Promise.allSettled(promises); state.diversionRules = results.filter(r => r.status === 'fulfilled' && Array.isArray(r.value)).flatMap(r => r.value); } catch (e) { state.diversionRules = []; } this.render(); }, render() { renderRuleTable(elements.diversionRulesTbody, state.diversionRules, 'diversion'); }, };

    // 流式计数工具：避免一次性创建超大字符串数组导致主线程卡顿
    async function countLinesStreaming(url, signal) {
        const res = await fetch(url, { signal });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const reader = res.body?.getReader();
        if (!reader) { // 兼容性回退
            const text = await res.text();
            if (!text) return 0;
            let n = 0; for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) === 10) n++;
            if (text.length > 0 && text.charCodeAt(text.length - 1) !== 10) n++;
            return n;
        }
        const decoder = new TextDecoder();
        let { value, done } = await reader.read();
        let leftover = '';
        let count = 0;
        while (!done) {
            const chunkText = leftover + decoder.decode(value, { stream: true });
            // 统计当前块中的换行符
            for (let i = 0; i < chunkText.length; i++) if (chunkText.charCodeAt(i) === 10) count++;
            // 处理最后一行未以 \n 结尾的情况：保留到下一块
            const lastNl = chunkText.lastIndexOf('\n');
            leftover = lastNl === -1 ? chunkText : chunkText.slice(lastNl + 1);
            ({ value, done } = await reader.read());
        }
        // 最后一块
        const finalText = leftover + decoder.decode();
        if (finalText.length > 0) count++;
        return count;
    }

    async function updateDomainListStats(signal) {
        const listMap = {
            fakeip: { element: elements.fakeipDomainCount, endpoint: '/plugins/my_fakeiplist/show' },
            realip: { element: elements.realipDomainCount, endpoint: '/plugins/my_realiplist/show' },
            nov4: { element: elements.nov4DomainCount, endpoint: '/plugins/my_nov4list/show' },
            nov6: { element: elements.nov6DomainCount, endpoint: '/plugins/my_nov6list/show' },
            total: { element: elements.totalDomainCount, endpoint: '/plugins/top_domains/show' },
        };

        for (const key of Object.keys(listMap)) {
            const { element, endpoint } = listMap[key];
            try {
                // 使用 api.fetch 获取带 Header 的结果，并加上 limit=1 极大地减少网络开销
                const res = await api.fetch(endpoint + '?limit=1', { signal });
                if (res && typeof res === 'object' && res.totalCount !== undefined) {
                    element.textContent = res.totalCount.toLocaleString();
                } else {
                    // 兜底逻辑：如果 Header 获取失败，尝试原来的流式计数（由于后端限制，此时可能仍显示 100）
                    const count = await countLinesStreaming(endpoint, signal);
                    element.textContent = count.toLocaleString();
                }
            } catch (e) {
                if (e.name !== 'AbortError') element.textContent = '获取失败';
            }
        }

        try {
            const status = state.requery.status;
            if (status && typeof status.last_run_domain_count === 'number') {
                elements.backupDomainCount.textContent = `${status.last_run_domain_count.toLocaleString()} 条`;
                elements.backupDomainCount.style.color = 'var(--color-accent-primary)';
            } else {
                elements.backupDomainCount.textContent = '--';
            }
        } catch (e) {
            elements.backupDomainCount.textContent = '--';
        }
    }

    function renderDataViewTable(entries, type = 'domain', append = false) {
        if (!elements.dataViewTableContainer) return;

        // 如果不是追加模式，或者容器内正在加载，则清空
        if (!append || elements.dataViewTableContainer.querySelector('.lazy-placeholder')) {
            elements.dataViewTableContainer.innerHTML = '';
        }

        if (entries.length === 0 && !append) {
            elements.dataViewTableContainer.innerHTML = '<div class="empty-state-content" style="padding: 2rem 0;"><p>没有匹配的条目。</p></div>';
            return;
        }

        // 计算本次需要渲染的新条目
        // 逻辑：如果是追加，只取数组最后 limit 条（即最新获取的那批数据）
        const itemsToRender = append ? entries.slice(-state.dataView.lastBatchSize) : entries;

        if (type === 'cache') {
            let accordionContainer = elements.dataViewTableContainer.querySelector('.accordion-container');
            if (!accordionContainer) {
                accordionContainer = document.createElement('div');
                accordionContainer.className = 'accordion-container';
                elements.dataViewTableContainer.appendChild(accordionContainer);
            }

            itemsToRender.forEach(item => {
                const itemEl = document.createElement('div');
                itemEl.className = 'accordion-item';

                const headerEl = document.createElement('h2');
                headerEl.className = 'accordion-header';
                const buttonEl = document.createElement('button');
                buttonEl.className = 'accordion-button collapsed';
                buttonEl.type = 'button';
                buttonEl.textContent = item.headerTitle;
                headerEl.appendChild(buttonEl);

                const collapseEl = document.createElement('div');
                collapseEl.className = 'accordion-collapse';
                const bodyEl = document.createElement('div');
                bodyEl.className = 'accordion-body';
                collapseEl.appendChild(bodyEl);

                itemEl.append(headerEl, collapseEl);
                accordionContainer.appendChild(itemEl);

                headerEl.addEventListener('click', (ev) => {
                    if (ev.target === buttonEl || buttonEl.contains(ev.target)) return;
                    ev.preventDefault();
                    buttonEl.click();
                });

                buttonEl.addEventListener('click', () => {
                    const isCollapsed = buttonEl.classList.contains('collapsed');
                    if (isCollapsed && bodyEl.innerHTML === '') {
                        const dnsMsgIndex = item.fullText.indexOf('DNS Message:');
                        const metadataText = dnsMsgIndex !== -1 ? item.fullText.substring(0, dnsMsgIndex) : item.fullText;
                        const dnsMessageText = dnsMsgIndex !== -1 ? item.fullText.substring(dnsMsgIndex) : 'DNS Message not found.';

                        const metadataTable = document.createElement('table');
                        metadataTable.className = 'data-table';
                        const tbody = document.createElement('tbody');
                        metadataText.trim().split('\n').forEach(line => {
                            const parts = line.match(/^([^:]+):\s*(.*)$/);
                            if (parts) {
                                const tr = document.createElement('tr');
                                tr.innerHTML = `<td>${parts[1].trim()}</td><td>${parts[2].trim()}</td>`;
                                tbody.appendChild(tr);
                            }
                        });
                        metadataTable.appendChild(tbody);

                        const pre = document.createElement('pre');
                        pre.innerHTML = `<code>${dnsMessageText.trim().replace(/</g, "&lt;").replace(/>/g, "&gt;")}</code>`;

                        bodyEl.appendChild(metadataTable);
                        bodyEl.appendChild(pre);
                    }
                    buttonEl.classList.toggle('collapsed');
                    collapseEl.classList.toggle('show');
                    collapseEl.style.maxHeight = collapseEl.classList.contains('show') ? (bodyEl.scrollHeight + 'px') : '0px';
                });
            });
        } else { 
            // 域名列表渲染
            let tbody = elements.dataViewTableContainer.querySelector('tbody');
            if (!tbody) {
                elements.dataViewTableContainer.innerHTML = `
                    <table class="mobile-card-layout">
                        <thead>
                            <tr>
                                <th style="width: 20%;">次数</th>
                                <th style="width: 30%;">最后日期</th>
                                <th>域名</th>
                            </tr>
                        </thead>
                        <tbody></tbody>
                    </table>`;
                tbody = elements.dataViewTableContainer.querySelector('tbody');
            }

            const html = itemsToRender.map(item => `
                <tr>
                    <td>${item.count}</td>
                    <td>${item.date}</td>
                    <td>${item.domain}</td>
                </tr>
            `).join('');
            tbody.insertAdjacentHTML('beforeend', html);
        }
    }

    async function openDataViewModal(config, isLoadMore = false) {
        const { listType, cacheTag, title } = config;
        if (!isLoadMore) {
            state.dataView.currentOffset = 0;
            state.dataView.rawEntries = [];
            state.dataView.hasMore = true;
            state.dataView.totalCount = 0;
            state.dataView.currentConfig = config;
            elements.dataViewModalTitle.textContent = title;
            elements.dataViewTableContainer.innerHTML = '<div class="lazy-placeholder"><div class="spinner"></div></div>';
            elements.dataViewModalInfo.textContent = '正在加载...';
            if (state.dataView.currentQuery === '' && !elements.dataViewSearch.value) {
                elements.dataViewSearch.value = '';
            }
        }
        if (!isLoadMore) lockScroll();
        elements.dataViewModal.showModal();
        try {
            const q = encodeURIComponent(state.dataView.currentQuery);
            const offset = state.dataView.currentOffset;
            const limit = state.dataView.currentLimit;
            let result;
            let viewType = 'domain';
            if (listType) {
                const endpointMap = { fakeip: '/plugins/my_fakeiplist/show', realip: '/plugins/my_realiplist/show', nov4: '/plugins/my_nov4list/show', nov6: '/plugins/my_nov6list/show', total: '/plugins/top_domains/show' };
                result = await api.fetch(`${endpointMap[listType]}?q=${q}&offset=${offset}&limit=${limit}`);
                viewType = 'domain';
            } else if (cacheTag) {
                result = await api.fetch(`/plugins/${cacheTag}/show?q=${q}&offset=${offset}&limit=${limit}`);
                viewType = 'cache';
            }
            let text = '';
            if (result && typeof result === 'object' && result.totalCount !== undefined) {
                text = result.body;
                state.dataView.totalCount = result.totalCount;
            } else {
                text = result;
            }
            let newEntries = [];
            if (viewType === 'cache') {
                const entries = text.trim() ? text.trim().split('----- Cache Entry -----').filter(entry => entry.trim() !== '') : [];
                newEntries = entries.map((entryText, index) => {
                    const questionMatch = entryText.match(/;; QUESTION SECTION:\s*;\s*([^\s]+)/);
                    const domainSetMatch = entryText.match(/DomainSet:\s*(.+)/);
                    let headerTitle = questionMatch ? questionMatch[1].replace(/\.$/, '') : `Entry #${state.dataView.currentOffset + index + 1}`;
                    if (domainSetMatch) headerTitle += ` [${domainSetMatch[1].trim()}]`;
                    return { headerTitle, fullText: entryText };
                });
            } else {
                const lines = text.trim() ? text.trim().split('\n') : [];
                newEntries = lines.map((line) => {
                    const trimmed = line.trim();
                    const match3 = trimmed.match(/^(\S+)\s+(\S+)\s+(.*)$/);
                    if (match3) return { count: match3[1], date: match3[2], domain: match3[3] };
                    const match2 = trimmed.match(/^(\S+)\s+(.*)$/);
                    if (match2) return { count: match2[1], date: '-', domain: match2[2] };
                    return { count: '-', date: '-', domain: trimmed };
                });
            }
            state.dataView.lastBatchSize = newEntries.length;
            state.dataView.rawEntries = isLoadMore ? [...state.dataView.rawEntries, ...newEntries] : newEntries;
            if (state.dataView.totalCount > 0) {
                state.dataView.hasMore = state.dataView.rawEntries.length < state.dataView.totalCount;
            } else {
                state.dataView.hasMore = newEntries.length >= state.dataView.currentLimit;
            }
            state.dataView.viewType = viewType;
            renderDataViewTable(state.dataView.rawEntries, viewType, isLoadMore);
            state.dataView.currentOffset += newEntries.length;
            updateDataViewFooter();
        } catch (error) {
            console.error("Data view fetch error:", error);
            if (!isLoadMore) elements.dataViewTableContainer.innerHTML = '<div class="empty-state-content"><p style="color:var(--color-danger);">加载失败或请求超时</p></div>';
            elements.dataViewModalInfo.textContent = '请求异常';
        }
    }


    function updateDataViewFooter() {
        const currentCount = state.dataView.rawEntries.length;
        const total = state.dataView.totalCount;
        if (total > 0) {
            elements.dataViewModalInfo.textContent = `当前显示: ${currentCount.toLocaleString()} / ${total.toLocaleString()} 条`;
        } else {
            elements.dataViewModalInfo.textContent = `当前显示: ${currentCount.toLocaleString()} 条`;
        }
        let loadMoreBtn = document.getElementById('dataview-load-more-btn');
        if (state.dataView.hasMore) {
            if (!loadMoreBtn) {
                loadMoreBtn = document.createElement('button');
                loadMoreBtn.id = 'dataview-load-more-btn';
                loadMoreBtn.className = 'button primary small';
                loadMoreBtn.style.cssText = 'margin-left: auto; height: 32px; padding: 0 15px; font-size: 0.85rem;';
                loadMoreBtn.innerHTML = '<span>加载更多</span>';
                loadMoreBtn.onclick = async () => {
                    ui.setLoading(loadMoreBtn, true);
                    await openDataViewModal(state.dataView.currentConfig, true);
                    ui.setLoading(loadMoreBtn, false);
                };
                elements.dataViewModal.querySelector('.modal-footer').appendChild(loadMoreBtn);
            }
            loadMoreBtn.style.display = 'inline-flex';
        } else if (loadMoreBtn) {
            loadMoreBtn.style.display = 'none';
        }
    }

    async function saveAllShuntRules() {
        if (!confirm('确定要保存所有分流规则吗?')) return;
        ui.setLoading(elements.saveShuntRulesBtn, true);
        ui.showToast('正在后台保存所有分流规则...');
        try {
            const requests = SHUNT_RULE_SAVE_PATHS.map(path => api.fetch(`/plugins/${path}`));
            const results = await Promise.allSettled(requests);

            const failed = results.filter(r => r.status === 'rejected');
            if (failed.length > 0) {
                ui.showToast(`部分规则保存失败 (${failed.length}/${results.length})`, 'error');
                console.error("Failed to save some rules:", failed);
            } else {
                ui.showToast('所有分流规则已成功保存', 'success');
            }
        } catch (e) {
            ui.showToast('保存操作时发生未知错误', 'error');
        } finally {
            ui.setLoading(elements.saveShuntRulesBtn, false);
        }
    }

    async function clearAllShuntRules() {
        if (!confirm('【重要操作】确定要清空所有动态生成的分流规则吗？此操作不可撤销。')) return;
        ui.setLoading(elements.clearShuntRulesBtn, true);
        ui.showToast('正在后台清空所有分流规则...');
        try {
            const requests = SHUNT_RULE_FLUSH_PATHS.map(path => api.fetch(`/plugins/${path}`));
            await Promise.allSettled(requests);
            ui.showToast('所有分流规则已清空', 'success');
            await updateDomainListStats();
        } catch (e) {
            ui.showToast('清空操作时发生未知错误', 'error');
        } finally {
            ui.setLoading(elements.clearShuntRulesBtn, false);
        }
    }

const cacheManager = {
        config: [
            { key: 'cache_all', name: '全部缓存 (兼容)', tag: 'cache_all' },
            { key: 'cache_cn', name: '国内缓存', tag: 'cache_cn' },
            { key: 'cache_node', name: '节点缓存', tag: 'cache_node' },
            { key: 'cache_google', name: '国外缓存 (兼容)', tag: 'cache_google' },
            { key: 'cache_all_noleak', name: '全部缓存 (安全)', tag: 'cache_all_noleak' },
            { key: 'cache_google_node', name: '国外缓存 (安全)', tag: 'cache_google_node' },
            { key: 'cache_cnmihomo', name: '国内域名fakeip', tag: 'cache_cnmihomo' }
        ],

        parseMetrics(metricsText, cacheTag) {
            const lines = metricsText.split('\n');
            const metrics = { query_total: 0, hit_total: 0, lazy_hit_total: 0, size_current: 0 };
            const query_str = `mosdns_cache_query_total{tag="${cacheTag}"}`;
            const hit_str = `mosdns_cache_hit_total{tag="${cacheTag}"}`;
            const lazy_str = `mosdns_cache_lazy_hit_total{tag="${cacheTag}"}`;
            const size_str = `mosdns_cache_size_current{tag="${cacheTag}"}`;
            lines.forEach(line => {
                if (line.startsWith(query_str)) metrics.query_total = parseFloat(line.split(' ')[1]) || 0;
                else if (line.startsWith(hit_str)) metrics.hit_total = parseFloat(line.split(' ')[1]) || 0;
                else if (line.startsWith(lazy_str)) metrics.lazy_hit_total = parseFloat(line.split(' ')[1]) || 0;
                else if (line.startsWith(size_str)) metrics.size_current = parseFloat(line.split(' ')[1]) || 0;
            });
            return metrics;
        },

        async updateStats(signal) {
            try {
                const metricsRes = await api.getMetrics(signal);
                if (metricsRes) {
                    this.config.forEach(cache => {
                        state.cacheStats[cache.key] = this.parseMetrics(metricsRes, cache.tag);
                    });
                }
                this.renderTable();
            } catch (error) {
                if (error.name !== 'AbortError') {
                    console.error("Failed to update cache stats:", error);
                    this.renderTable(true);
                }
            }
        },

        renderTable(isError = false) {
            const tbody = elements.cacheStatsTbody;
            if (!tbody) return;

            const table = tbody.closest('table');
            if (table) {
                // 关键：根据设备状态切换 CSS 类，触发 log.html 中的高权重样式覆盖
                if (state.isMobile) {
                    table.classList.add('mobile-card-view');
                    table.classList.remove('mobile-card-layout'); 
                } else {
                    table.classList.remove('mobile-card-view');
                    table.classList.remove('mobile-card-layout');
                }
            }

            tbody.innerHTML = '';

            if (isError) {
                const cols = state.isMobile ? 1 : 8;
                tbody.innerHTML = `<tr><td colspan="${cols}" style="text-align:center; color: var(--color-danger);">缓存数据加载失败</td></tr>`;
                return;
            }

            this.config.forEach(cache => {
                const tr = document.createElement('tr');
                const stats = state.cacheStats[cache.key] || { query_total: 0, hit_total: 0, lazy_hit_total: 0, size_current: 0 };
                const hitRate = stats.query_total > 0 ? (stats.hit_total / stats.query_total * 100).toFixed(2) + '%' : '0.00%';
                const lazyRate = stats.query_total > 0 ? (stats.lazy_hit_total / stats.query_total * 100).toFixed(2) + '%' : '0.00%';

                if (state.isMobile) {
                    // 移动端卡片视图
                    tr.innerHTML = `
                        <td>
                            <div class="mobile-system-card">
                                <div class="card-header">
                                    <div class="card-title">${cache.name}</div>
                                    <button class="button danger small clear-cache-btn" data-cache-tag="${cache.tag}" style="padding: 0.3rem 0.6rem;">清空</button>
                                </div>
                                <div class="mobile-stats-grid">
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">条目数</span><span class="mobile-stat-value"><a href="#" class="control-item-link" data-cache-tag="${cache.tag}" data-cache-title="${cache.name}">${stats.size_current.toLocaleString()}</a></span></div>
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">请求总数</span><span class="mobile-stat-value">${stats.query_total.toLocaleString()}</span></div>
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">缓存命中</span><span class="mobile-stat-value">${stats.hit_total.toLocaleString()}</span></div>
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">命中率</span><span class="mobile-stat-value">${hitRate}</span></div>
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">过期命中</span><span class="mobile-stat-value">${stats.lazy_hit_total.toLocaleString()}</span></div>
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">过期命中率</span><span class="mobile-stat-value">${lazyRate}</span></div>
                                </div>
                            </div>
                        </td>
                    `;
                } else {
                    // PC端表格视图
                    tr.innerHTML = `
                        <td>
                            <div class="cache-name-wrapper" style="max-width: 150px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${cache.name}">${cache.name}</div>
                        </td>
                        <td class="text-right">${stats.query_total.toLocaleString()}</td>
                        <td class="text-right">${stats.hit_total.toLocaleString()}</td>
                        <td class="text-right">${stats.lazy_hit_total.toLocaleString()}</td>
                        <td class="text-right">${hitRate}</td>
                        <td class="text-right">${lazyRate}</td>
                        <td class="text-right"><a href="#" class="control-item-link" data-cache-tag="${cache.tag}" data-cache-title="${cache.name}">${stats.size_current.toLocaleString()}</a></td>
                        <td class="text-center"><button class="button danger clear-cache-btn" data-cache-tag="${cache.tag}" style="padding: 0.4rem 0.8rem; font-size: 0.85rem;">清空</button></td>
                    `;
                }
                tbody.appendChild(tr);
            });
        }
    };

    const listManager = {
        MAX_LINES: 10000,
        currentTag: null,
        profiles: [
            { tag: 'whitelist', name: '白名单' },
            { tag: 'blocklist', name: '黑名单' },
            { tag: 'greylist', name: '灰名单' },
            { tag: 'realiplist', name: '!CN fakeip filter' },
            { tag: 'cnfakeipfilter', name: 'CN fakeip filter' },
            { tag: 'ddnslist', name: 'DDNS 域名' },
            { tag: 'client_ip', name: '客户端 IP' },
            { tag: 'direct_ip', name: '直连 IP' },
            { tag: 'rewrite', name: '重定向' }
        ],

        init() {
            if (state.listManagerInitialized) return;
            elements.listMgmtNav.addEventListener('click', e => {
                e.preventDefault();
                const link = e.target.closest('.list-mgmt-link');
                if (link && !link.classList.contains('active')) {
                    this.loadList(link.dataset.listTag);
                }
            });
            elements.listSaveBtn.addEventListener('click', () => this.saveList());
            // 首屏不立即加载巨大列表，交给空闲时机/用户点击触发，避免刷新时卡顿
            if ('requestIdleCallback' in window) requestIdleCallback(() => this.loadList('whitelist'), { timeout: 2000 });
            else setTimeout(() => this.loadList('whitelist'), 1200);
            state.listManagerInitialized = true;
        },

        async loadList(tag) {
            this.currentTag = tag;
            // Abort any previous in-flight request and reset textarea to avoid old content persisting
            try { this._abortController?.abort(); } catch (_) { }
            this._abortController = new AbortController();
            // Clear previous content so switching lists reflects immediately
            elements.listContentTextArea.value = '';
            elements.listContentTextArea.scrollTop = 0;
            elements.listMgmtNav.querySelectorAll('.list-mgmt-link').forEach(l => l.classList.toggle('active', l.dataset.listTag === tag));

            elements.listMgmtClientIpHint.style.display = (tag === 'client_ip') ? 'block' : 'none';
            if (elements.listMgmtDirectIpHint) {
                elements.listMgmtDirectIpHint.style.display = (tag === 'direct_ip') ? 'block' : 'none';
            }
            if (elements.listMgmtRewriteHint) {
                elements.listMgmtRewriteHint.style.display = (tag === 'rewrite') ? 'block' : 'none';
            }
            if (elements.listMgmtRealIPHint) {
                elements.listMgmtRealIPHint.style.display = (tag === 'realiplist') ? 'block' : 'none';
            }
            if (elements.listMgmtCnFakeipFilterHint) {
                elements.listMgmtCnFakeipFilterHint.style.display = (tag === 'cnfakeipfilter') ? 'block' : 'none';
            }

            elements.listContentLoader.style.display = 'flex';
            elements.listContentTextArea.style.display = 'none';
            elements.listContentInfo.textContent = '正在加载...';
            ui.setLoading(elements.listSaveBtn, true);

            try {
                // 流式读取，最多加载 MAX_LINES 行，避免一次性 split 大字符串拖慢主线程
// 这里的 this.MAX_LINES 是 10000
const res = await fetch(`/plugins/${tag}/show?limit=${this.MAX_LINES}`, { 
    signal: this._abortController.signal 
});

                if (!res.ok) throw new Error(`HTTP ${res.status}`);
                const reader = res.body?.getReader();
                let totalLines = 0, shownLines = 0;
                let buffer = '';
                const CHUNK_LIMIT = this.MAX_LINES; // 达到就停止
                let cancelled = false;
                if (reader) {
                    const decoder = new TextDecoder();
                    while (true) {
                        const { value, done } = await reader.read();
                        if (done) break;
                        buffer += decoder.decode(value, { stream: true });
                        let nl;
                        while ((nl = buffer.indexOf('\n')) !== -1) {
                            totalLines++;
                            const line = buffer.slice(0, nl);
                            buffer = buffer.slice(nl + 1);
                            if (shownLines < CHUNK_LIMIT) {
                                elements.listContentTextArea.value += (shownLines ? '\n' : '') + line;
                                shownLines++;
                            }
                            if (shownLines >= CHUNK_LIMIT) {
                                // 够了，取消后续读取
                                cancelled = true;
                                try { reader.cancel(); } catch (_) { }
                                break;
                            }
                        }
                        if (cancelled) break;
                    }
                    // 剩余缓冲
                    if (!cancelled && buffer.length > 0) {
                        totalLines++;
                        if (shownLines < CHUNK_LIMIT) {
                            elements.listContentTextArea.value += (shownLines ? '\n' : '') + buffer;
                            shownLines++;
                        }
                    }
                } else {
                    // 兼容不支持流式的环境
                    const text = await res.text();
                    const parts = text.split('\n');
                    totalLines = parts.length;
                    elements.listContentTextArea.value = parts.slice(0, CHUNK_LIMIT).join('\n');
                    shownLines = Math.min(totalLines, CHUNK_LIMIT);
                }
                if (shownLines >= CHUNK_LIMIT) elements.listContentInfo.textContent = `内容较长，已仅加载前 ${CHUNK_LIMIT} 行。`;
                else elements.listContentInfo.textContent = `共 ${shownLines} 行。`;
            } catch (error) {
                if (error?.name === 'AbortError') {
                    // 用户快速切换导致的中断，不提示错误
                    elements.listContentInfo.textContent = '已取消';
                } else {
                    elements.listContentTextArea.value = `加载列表“${tag}”失败。`;
                    elements.listContentInfo.textContent = '加载失败';
                    ui.showToast(`加载列表“${tag}”失败`, 'error');
                }
            } finally {
                elements.listContentLoader.style.display = 'none';
                elements.listContentTextArea.style.display = 'block';
                ui.setLoading(elements.listSaveBtn, false);
                this._abortController = null;
            }
        },

        async saveList() {
            if (!this.currentTag) return;
            ui.setLoading(elements.listSaveBtn, true);
            try {
                const values = elements.listContentTextArea.value.split('\n').map(s => s.trim()).filter(Boolean);
                await api.fetch(`/plugins/${this.currentTag}/post`, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ values })
                });
                ui.showToast(`列表“${this.currentTag}”已保存`, 'success');
                elements.listContentInfo.textContent = `保存成功！共 ${values.length} 行。`;
            } catch (error) {
                ui.showToast(`保存列表“${this.currentTag}”失败`, 'error');
            } finally {
                ui.setLoading(elements.listSaveBtn, false);
            }
        }
    };

    // Config Manager: MosDNS 远程配置更新及本地备份
    const configManager = {
        init() {
            this.injectCard();
            this.loadSettings();
            this.bindEvents();
        },

        injectCard() {
            const updateModule = document.getElementById('update-module');
            if (!updateModule || !updateModule.parentNode) return;

            // 创建新卡片，使用 control-module 类以保持一致性
            const card = document.createElement('div');
            card.id = 'config-manager-card';
            card.className = 'control-module';
            card.style.gridColumn = '1 / -1';

            // 填充内容，使用与其他 control-module 一致的结构
            card.innerHTML = `
                <h3>
                    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor">
                        <path d="M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96zM14 13v4h-4v-4H7l5-5 5 5h-3z"/>
                    </svg>
                    配置管理
                </h3>
                <p class="module-desc">管理 MosDNS 的本地配置。您可以备份当前配置到本地，或者从远程 URL 下载配置包覆盖当前设置。</p>
                
                <div class="control-item">
                    <label for="cfg-local-dir" class="field-label">MosDNS 本地工作目录</label>
                    <input type="text" id="cfg-local-dir" class="input" placeholder="例如: /etc/mosdns 或 C:\\mosdns">
                </div>

                <div class="control-item">
                    <label for="cfg-remote-url" class="field-label">远程配置下载 URL (ZIP)</label>
                    <input type="text" id="cfg-remote-url" class="input" placeholder="例如: https://github.com/user/repo/archive/master.zip">
                </div>

                <div class="button-group" style="margin-top: 1rem; justify-content: flex-end;">
                    <button class="button secondary" id="cfg-backup-btn">
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z"/></svg>
                        <span>备份配置</span>
                    </button>
                    <button class="button primary" id="cfg-update-btn">
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M12 4V1L8 5l4 4V6c3.31 0 6 2.69 6 6 0 1.01-.25 1.97-.7 2.8l1.46 1.46C19.54 15.03 20 13.57 20 12c0-4.42-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6 0-1.01.25-1.97.7-2.8L5.24 7.74C4.46 8.97 4 10.43 4 12c0 4.42 3.58 8 8 8v3l4-4-4-4v3z"/></svg>
                        <span>应用远程配置</span>
                    </button>
                </div>
            `;

            // 插入 DOM (插入到 updateModule 之后)
            updateModule.parentNode.insertBefore(card, updateModule.nextSibling);
        },

        loadSettings() {
            const savedDir = localStorage.getItem('mosdns-config-dir');
            const savedUrl = localStorage.getItem('mosdns-config-url');
            const dirInput = document.getElementById('cfg-local-dir');
            const urlInput = document.getElementById('cfg-remote-url');

            if (dirInput && savedDir) dirInput.value = savedDir;
            if (urlInput && savedUrl) urlInput.value = savedUrl;
        },

        saveSettings() {
            const dirInput = document.getElementById('cfg-local-dir');
            const urlInput = document.getElementById('cfg-remote-url');
            if (dirInput) localStorage.setItem('mosdns-config-dir', dirInput.value.trim());
            if (urlInput) localStorage.setItem('mosdns-config-url', urlInput.value.trim());
        },

        bindEvents() {
            const backupBtn = document.getElementById('cfg-backup-btn');
            const updateBtn = document.getElementById('cfg-update-btn');
            const dirInput = document.getElementById('cfg-local-dir');
            const urlInput = document.getElementById('cfg-remote-url');

            // 自动保存输入
            dirInput?.addEventListener('change', () => this.saveSettings());
            urlInput?.addEventListener('change', () => this.saveSettings());

            backupBtn?.addEventListener('click', () => this.handleBackup(backupBtn));
            updateBtn?.addEventListener('click', () => this.handleUpdate(updateBtn));
        },

        async handleBackup(btn) {
            const dir = document.getElementById('cfg-local-dir').value.trim();
            if (!dir) {
                ui.showToast('请先输入 MosDNS 本地工作目录', 'error');
                return;
            }
            this.saveSettings();
            ui.setLoading(btn, true);

            try {
                const response = await fetch('/api/v1/config/export', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ dir })
                });

                if (!response.ok) {
                    const text = await response.text();
                    throw new Error(text || `HTTP ${response.status}`);
                }

                const blob = await response.blob();
                const url = window.URL.createObjectURL(blob);
                const a = document.createElement('a');
                a.style.display = 'none';
                a.href = url;
                // 尝试从 Content-Disposition 获取文件名
                const disposition = response.headers.get('Content-Disposition');
                let filename = 'mosdns_backup.zip';
                if (disposition && disposition.indexOf('attachment') !== -1) {
                    const filenameRegex = /filename[^;=\n]*=((['"]).*?\2|[^;\n]*)/;
                    const matches = filenameRegex.exec(disposition);
                    if (matches != null && matches[1]) {
                        filename = matches[1].replace(/['"]/g, '');
                    }
                }
                a.download = filename;
                document.body.appendChild(a);
                a.click();
                window.URL.revokeObjectURL(url);
                ui.showToast('备份文件下载开始', 'success');
            } catch (error) {
                console.error('Backup failed:', error);
                ui.showToast(`备份失败: ${error.message}`, 'error');
            } finally {
                ui.setLoading(btn, false);
            }
        },

        async handleUpdate(btn) {
            const dir = document.getElementById('cfg-local-dir').value.trim();
            const url = document.getElementById('cfg-remote-url').value.trim();

            if (!dir || !url) {
                ui.showToast('请完整填写本地目录和远程 URL', 'error');
                return;
            }

            if (!confirm('确定要从远程 URL 更新配置吗？\n\n1. 当前配置将备份到 backup 子目录。\n2. 新配置将覆盖现有文件。\n3. MosDNS 将自动重启。\n\n此操作存在风险，请确保 URL 可信。')) {
                return;
            }

            this.saveSettings();
            ui.setLoading(btn, true);

            try {
                const res = await api.fetch('/api/v1/config/update_from_url', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ url, dir })
                });

                ui.showToast(res.message || '更新成功，4秒后重启...', 'success');

                // 等待重启
                setTimeout(() => {
                    location.reload();
                }, 4000);
            } catch (error) {
                console.error('Update failed:', error);
                ui.showToast(`更新失败: ${error.message}`, 'error');
                ui.setLoading(btn, false);
            }
        }
    };

    // -- [修改] -- 终极修复版：修复头部图标及状态显示
    const overridesManager = {
        state: { replacements: [] },

        getElements() {
            const get = (id) => document.getElementById(id) || document.getElementById(id.replace('-log', ''));
            return {
                module: get('overrides-module'), // 旧卡片 DOM
                socks5: get('override-socks5-log'),
                ecs: get('override-ecs-log'),
                oldSaveBtn: get('overrides-save-btn-log'),
                oldLoadBtn: get('overrides-load-btn-log')
            };
        },

        // --- 主入口 ---
        async load(silent = false) {
            const els = this.getElements();
            if (!els.socks5) return;

            // 1. 隐藏旧按钮
            if (els.oldSaveBtn) els.oldSaveBtn.style.display = 'none';
            if (els.oldLoadBtn) els.oldLoadBtn.style.display = 'none';

            // 2. 注入新板块
            this.injectNewCard();

            try {
                const data = await api.fetch('/api/v1/overrides');

                if (els.socks5) els.socks5.value = (data && data.socks5) || '';
                if (els.ecs) els.ecs.value = (data && data.ecs) || '';

                this.state.replacements = (data && data.replacements) ? data.replacements : [];
                this.renderReplacementsTable();

                if (!silent) ui.showToast('已读取当前覆盖配置');
            } catch (e) {
                if (!silent) ui.showToast('读取覆盖配置失败', 'error');
            }
        },

        // --- 核心逻辑：创建并正确布局新卡片 ---
        injectNewCard() {
            if (document.getElementById('replacements-card')) return;

            const els = this.getElements();
            if (!els.module || !els.module.parentNode) return;

            // 创建新卡片，使用 control-module 类以保持一致性
            const newCard = document.createElement('div');
            newCard.id = 'replacements-card';
            newCard.className = 'control-module';
            newCard.style.gridColumn = '1 / -1';

            // 填充内容
            newCard.innerHTML = `
                <div class="module-header">
                    <h3>
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor">
                            <path d="M7.5 5.6L5 7 6.4 4.5 5 2 7.5 3.4 10 2 8.6 4.5 10 7 7.5 5.6zm12 9.8L22 17l-2.5 1.4L18.1 22l-1.4-2.5L14.2 18l2.5-1.4L18.1 14l1.4 2.5zM11 10c0-3.31 2.69-6 6-6s6 2.69 6 6-2.69 6-6 6-6-2.69-6-6zm-8 8c0-3.31 2.69-6 6-6s6 2.69 6 6-2.69 6-6 6-6-2.69-6-6z"/>
                        </svg>
                        其它设置
                    </h3>
                    <button class="button secondary small" id="rep-add-btn">
                        <span>+ 添加规则</span>
                    </button>
                </div>

                <p class="module-desc">配置说明：https://github.com/yyysuo/mosdns</p>
                
                <div class="scrollable-table-container">
                    <table class="data-table" style="min-width: 650px;">
                        <thead>
                            <tr>
                                <th style="width: 15%;">状态</th>
                                <th style="width: 25%;">原值 (查找)</th>
                                <th style="width: 25%;">新值 (替换)</th>
                                <th>备注</th>
                                <th style="width: 60px; text-align: center;">操作</th>
                            </tr>
                        </thead>
                        <tbody id="rep-tbody"></tbody>
                    </table>
                </div>

                <div class="button-group" style="margin-top: 1rem; justify-content: flex-end;">
                    <span style="color: var(--color-text-secondary); font-size: 0.85em; margin-right: auto;">保存应用SOCKS5/ECS IP/上游DNS设置</span>
                    <button class="button primary" id="rep-save-btn">
                        <span>保存并重启</span>
                    </button>
                </div>
            `;

            // 插入 DOM
            els.module.parentNode.insertBefore(newCard, els.module.nextSibling);


            // 6. 绑定事件
            newCard.querySelector('#rep-add-btn').addEventListener('click', () => {
                this.state.replacements.push({ original: '', new: '', comment: '' }); // 新增行没有 result，会显示“未保存”
                this.renderReplacementsTable();
            });

            newCard.querySelector('#rep-save-btn').addEventListener('click', () => this.save());

            const tbody = newCard.querySelector('#rep-tbody');
            tbody.addEventListener('click', (e) => {
                const btn = e.target.closest('.rep-del-btn');
                if (btn) {
                    const idx = parseInt(btn.dataset.index);
                    this.state.replacements.splice(idx, 1);
                    this.renderReplacementsTable();
                }
            });

            tbody.addEventListener('input', (e) => {
                if (e.target.matches('input')) {
                    const idx = parseInt(e.target.dataset.index);
                    const field = e.target.dataset.field;
                    this.state.replacements[idx][field] = e.target.value;
                }
            });
        },

renderReplacementsTable() {
            const tbody = document.getElementById('rep-tbody');
            if (!tbody) return;

            const table = tbody.closest('table');
            if (table) {
                if (state.isMobile) {
                    table.classList.add('mobile-card-view');
                } else {
                    table.classList.remove('mobile-card-view');
                }
            }

            if (this.state.replacements.length === 0) {
                tbody.innerHTML = '<tr><td colspan="5" class="text-center" style="padding: 2rem; color: var(--color-text-secondary); font-style: italic;">暂无替换规则</td></tr>';
                return;
            }

            tbody.innerHTML = this.state.replacements.map((rule, index) => {
                let statusHtml = '<span class="response-tag other" style="font-size: 0.8em; opacity: 0.7;">未保存</span>';
                if (rule.result) {
                    if (rule.result.startsWith('Success')) {
                        statusHtml = `<span class="response-tag noerror" style="font-size: 0.8em;">${rule.result}</span>`;
                    } else if (rule.result.includes('Not Found')) {
                        statusHtml = `<span class="response-tag nxdomain" style="font-size: 0.8em;">${rule.result}</span>`;
                    } else {
                        statusHtml = `<span class="response-tag other" style="font-size: 0.8em;">${rule.result}</span>`;
                    }
                }

                if (state.isMobile) {
                    // 移动端卡片视图
                    return `
                        <tr>
                            <td>
                                <div class="mobile-system-card">
                                    <div class="card-header">
                                        <div style="font-size:0.9rem; font-weight:600;">规则 #${index + 1}</div>
                                        ${statusHtml}
                                    </div>
                                    <div class="mobile-override-row">
                                        <div>
                                            <label style="font-size:0.75rem; color:var(--color-text-secondary);">原值 (查找)</label>
                                            <input type="text" class="input" value="${rule.original || ''}" data-index="${index}" data-field="original" placeholder="例如: 1.1.1.1">
                                        </div>
                                        <div>
                                            <label style="font-size:0.75rem; color:var(--color-text-secondary);">新值 (替换)</label>
                                            <input type="text" class="input" value="${rule.new || ''}" data-index="${index}" data-field="new" placeholder="例如: 127.0.0.1">
                                        </div>
                                        <div>
                                            <label style="font-size:0.75rem; color:var(--color-text-secondary);">备注</label>
                                            <input type="text" class="input" value="${rule.comment || ''}" data-index="${index}" data-field="comment" placeholder="可选">
                                        </div>
                                    </div>
                                    <div style="text-align:right; margin-top:0.8rem;">
                                        <button class="button danger small rep-del-btn" data-index="${index}" style="width:100%;">删除规则</button>
                                    </div>
                                </div>
                            </td>
                        </tr>
                    `;
                } else {
                    // PC端表格视图
                    return `
                        <tr style="border-bottom: 1px solid var(--color-border);">
                            <td style="padding: 8px;">${statusHtml}</td>
                            <td style="padding: 8px;">
                                <input type="text" class="input" style="width: 100%;" value="${rule.original || ''}" data-index="${index}" data-field="original" placeholder="例如: 1.1.1.1">
                            </td>
                            <td style="padding: 8px;">
                                <input type="text" class="input" style="width: 100%;" value="${rule.new || ''}" data-index="${index}" data-field="new" placeholder="例如: 127.0.0.1">
                            </td>
                            <td style="padding: 8px;">
                                <input type="text" class="input" style="width: 100%;" value="${rule.comment || ''}" data-index="${index}" data-field="comment" placeholder="备注 (可选)">
                            </td>
                            <td style="padding: 8px; text-align: center;">
                                <button class="button danger small rep-del-btn" data-index="${index}" title="删除此行" style="padding: 6px 10px;">
                                    <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
                                </button>
                            </td>
                        </tr>
                    `;
                }
            }).join('');
        },

        async save() {
            const els = this.getElements();
            if (!els.socks5 || !els.ecs) return;

            const btn = document.getElementById('rep-save-btn');
            ui.setLoading(btn, true);

            const socks5 = els.socks5.value.trim();
            const ecs = els.ecs.value.trim();
            // 过滤掉空行
            const validReplacements = this.state.replacements
                .map(r => ({
                    original: r.original.trim(),
                    new: r.new.trim(),
                    comment: r.comment ? r.comment.trim() : ''
                }))
                .filter(r => r.original);

            const payload = {
                socks5: socks5,
                ecs: ecs,
                replacements: validReplacements
            };

            try {
                // 发送 POST 请求
                await api.fetch('/api/v1/overrides', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(payload) });
                ui.showToast('配置已保存，4秒后重启服务…', 'success');
                try {
                    await api.fetch('/api/v1/system/restart', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ delay_ms: 300 }) });
                    // 重启后刷新页面
                    setTimeout(() => { location.reload(); }, 4000);
                } catch (err) {
                    ui.showToast('自动重启请求失败，请尝试手动重启', 'error');
                    ui.setLoading(btn, false);
                }
            } catch (e) {
                ui.showToast('保存配置失败', 'error');
                console.error("Save Error:", e);
                ui.setLoading(btn, false);
            }
        }
    };

// [插入点] 修复后的 Upstream 管理器
// [新增] 上游DNS管理器
    const upstreamManager = {
        state: {
            tags: [],      // 后端发现的 aliapi tags
            config: {},    // Map<pluginTag, Array<UpstreamConfig>>
            metrics: {}    // 解析后的 metrics 数据
        },

        init() {
            if (!document.getElementById('upstream-dns-module')) return;
            
            this.bindEvents();
            // 如果已经在系统控制标签页，立即加载
            const tab = document.getElementById('system-control-tab');
            if (tab && tab.classList.contains('active')) {
                this.loadData();
            }
        },

        bindEvents() {
            document.getElementById('add-upstream-btn')?.addEventListener('click', () => this.openModal());
            document.getElementById('upstream-restart-btn')?.addEventListener('click', () => this.restartService());
            
            // 列表操作委托
            document.getElementById('upstream-dns-tbody')?.addEventListener('click', (e) => {
                const btn = e.target.closest('button');
                if (!btn) return;
                const row = btn.closest('tr');
                const group = row.dataset.group;
                const index = parseInt(row.dataset.index, 10);

                if (btn.classList.contains('edit-btn')) {
                    this.openModal(group, index);
                } else if (btn.classList.contains('delete-btn')) {
                    this.deleteUpstream(group, index);
                }
            });

            // 启用开关委托
            document.getElementById('upstream-dns-tbody')?.addEventListener('change', (e) => {
                if (e.target.matches('.upstream-enable-toggle')) {
                    const row = e.target.closest('tr');
                    this.toggleEnable(row.dataset.group, parseInt(row.dataset.index, 10), e.target.checked);
                }
            });

            const modal = document.getElementById('upstream-modal');
            const form = document.getElementById('upstream-form');
            const protocolSelect = document.getElementById('upstream-protocol');

            document.getElementById('close-upstream-modal')?.addEventListener('click', () => closeAndUnlock(modal));
            document.getElementById('cancel-upstream-modal')?.addEventListener('click', () => closeAndUnlock(modal));
            
            protocolSelect?.addEventListener('change', () => this.updateFormFields(protocolSelect.value));
            
            form?.addEventListener('submit', (e) => {
                e.preventDefault();
                this.handleSave(new FormData(form));
            });
        },

        async loadData() {
            try {
                const [tagsRes, configRes, metricsRaw] = await Promise.all([
                    api.fetch('/api/v1/upstream/tags'),
                    api.fetch('/api/v1/upstream/config'),
                    api.getMetrics()
                ]);

                // 确保 tagsRes 是数组
                this.state.tags = Array.isArray(tagsRes) ? tagsRes : [];
                this.state.config = configRes || {};
                this.parseMetrics(metricsRaw);
                this.renderTable();
            } catch (e) {
                console.error("Upstream data load failed", e);
                ui.showToast('加载上游配置失败', 'error');
            }
        },

        parseMetrics(rawText) {
            const parse = (prefix) => {
                const regex = new RegExp(`${prefix}\\{[^}]*metrics_tag="([^"]+)"[^}]*tag="([^"]+)"[^}]*\\} ([0-9.eE+-]+)`, 'g');
                const map = {};
                let match;
                while ((match = regex.exec(rawText)) !== null) {
                    const group = match[1];
                    const name = match[2];
                    const val = parseFloat(match[3]);
                    const key = `${group}|${name}`;
                    map[key] = val;
                }
                return map;
            };

            this.state.metrics = { 
                latSum: parse('mosdns_aliapi_response_latency_millisecond_sum'),
                latCount: parse('mosdns_aliapi_response_latency_millisecond_count'),
                queryTotal: parse('mosdns_aliapi_query_total'),
                errorTotal: parse('mosdns_aliapi_error_total'),
                winnerTotal: parse('mosdns_aliapi_upstream_winner_total')
            };
        },

renderTable() {
            const tbody = document.getElementById('upstream-dns-tbody');
            if (!tbody) return;
            
            const table = tbody.closest('table');
            if (table) {
                if (state.isMobile) {
                    table.classList.add('mobile-card-view');
                } else {
                    table.classList.remove('mobile-card-view');
                }
            }

            tbody.innerHTML = '';

            const allUpstreams = [];
            for (const [group, upstreams] of Object.entries(this.state.config)) {
                if (!upstreams || !Array.isArray(upstreams)) continue;
                upstreams.forEach((u, idx) => {
                    allUpstreams.push({ u: u, group: group, originalIndex: idx });
                });
            }

            if (allUpstreams.length === 0) {
                tbody.innerHTML = '<tr><td colspan="9" class="text-center" style="padding:2rem;">暂无上游配置，请点击添加。</td></tr>';
                return;
            }

            allUpstreams.sort((a, b) => {
                const enabledDiff = (b.u.enabled ? 1 : 0) - (a.u.enabled ? 1 : 0);
                if (enabledDiff !== 0) return enabledDiff;
                const groupDiff = a.group.localeCompare(b.group);
                if (groupDiff !== 0) return groupDiff;
                return (a.u.tag || '').localeCompare(b.u.tag || '');
            });

            allUpstreams.forEach((item) => {
                const u = item.u;
                const group = item.group;
                const index = item.originalIndex;
                const key = `${group}|${u.tag}`;
                const stats = u.enabled ? this.getStats(key) : { avgLat: '-', query: '-', error: '-', rate: '-', winner: '-', winRate: '-' };
                
                const tr = document.createElement('tr');
                tr.dataset.group = group; 
                tr.dataset.index = index; 
                
                if (!u.enabled) tr.style.opacity = '0.6';

                if (state.isMobile) {
                    // [修复] 移动端卡片视图：补充采纳数和采纳率
                    tr.innerHTML = `
                        <td>
                            <div class="mobile-system-card">
                                <div class="card-header">
                                    <div style="display:flex; flex-direction:column; max-width: 70%;">
                                        <span class="card-title">${u.tag}</span>
                                        <small style="color:var(--color-text-secondary); margin-top:4px;">${group} · ${u.protocol}</small>
                                    </div>
                                    <label class="switch">
                                        <input type="checkbox" class="upstream-enable-toggle" ${u.enabled ? 'checked' : ''}>
                                        <span class="slider"></span>
                                    </label>
                                </div>
                                <div class="mobile-stats-grid">
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">平均响应</span><span class="mobile-stat-value">${stats.avgLat}</span></div>
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">请求数</span><span class="mobile-stat-value">${stats.query}</span></div>
                                    
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">采纳数</span><span class="mobile-stat-value">${stats.winner}</span></div>
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">采纳率</span><span class="mobile-stat-value">${stats.winRate}</span></div>
                                    
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">错误数</span><span class="mobile-stat-value" style="${stats.error > 0 ? 'color:var(--color-danger)' : ''}">${stats.error}</span></div>
                                    <div class="mobile-stat-item"><span class="mobile-stat-label">出错率</span><span class="mobile-stat-value">${stats.rate}</span></div>
                                </div>
                                <div class="mobile-card-actions">
                                    <button class="button secondary small edit-btn" style="flex:1;">编辑</button>
                                    <button class="button danger small delete-btn" style="flex:1;">删除</button>
                                </div>
                            </div>
                        </td>
                    `;
                } else {
                    // PC端表格视图
                    tr.innerHTML = `
                        <td class="text-center">
                            <label class="switch">
                                <input type="checkbox" class="upstream-enable-toggle" ${u.enabled ? 'checked' : ''}>
                                <span class="slider"></span>
                            </label>
                        </td>
                        <td>${group}</td>
                        <td>${u.tag}</td>
                        <td>${u.protocol}</td>
                        <td class="text-center">${stats.avgLat}</td>
                        <td class="text-center">${stats.query}</td>
                        <td class="text-center">${stats.winner}</td>
                        <td class="text-center">${stats.winRate}</td>
                        <td class="text-center">${stats.error}</td>
                        <td class="text-center">${stats.rate}</td>
                        <td class="text-center">
                             <div style="display: inline-flex; gap: 0.5rem;">
                                <button class="button secondary small edit-btn" style="padding: 0.3rem 0.6rem;">编辑</button>
                                <button class="button danger small delete-btn" style="padding: 0.3rem 0.6rem;">删除</button>
                            </div>
                        </td>
                    `;
                }
                tbody.appendChild(tr);
            });
        },

        getStats(key) {
            const m = this.state.metrics;
            const q = m.queryTotal[key] || 0;
            const e = m.errorTotal[key] || 0;
            const w = m.winnerTotal[key] || 0;
            const lSum = m.latSum[key] || 0;
            const lCount = m.latCount[key] || 0;
            const avg = lCount > 0 ? (lSum / lCount).toFixed(2) + ' ms' : '0 ms';
            const rate = q > 0 ? ((e / q) * 100).toFixed(2) + '%' : '0.00%';
            const winRate = q > 0 ? ((w / q) * 100).toFixed(2) + '%' : '0.00%';
            return { avgLat: avg, query: q, error: e, rate: rate, winner: w, winRate: winRate };
        },

        openModal(group = null, index = null) {
            const modal = document.getElementById('upstream-modal');
            const form = document.getElementById('upstream-form');
            const groupSelect = document.getElementById('upstream-group');
            const protocolSelect = document.getElementById('upstream-protocol');
            
            form.reset();
            
            // 填充所属组下拉框
            groupSelect.innerHTML = '';
            const allGroups = new Set();
            
            // 1. 添加从后端读取到的 aliapi 插件的 Tag
            if (Array.isArray(this.state.tags)) {
                this.state.tags.forEach(t => {
                    // 过滤非字符串 (防止 JS for-in 遍历数字索引等问题)
                    if (t && typeof t === 'string' && isNaN(t)) {
                        allGroups.add(t);
                    }
                });
            }
            
            // 2. 添加配置文件中已存在的组名（即便当前未扫描到，也应允许编辑）
            if (this.state.config && typeof this.state.config === 'object') {
                Object.keys(this.state.config).forEach(k => {
                    if (k && isNaN(k)) {
                        allGroups.add(k);
                    }
                });
            }
            // 如果没有找到组，下拉框保持为空，用户点击保存时会提示"请选择所属组"
            allGroups.forEach(g => {
                const opt = document.createElement('option');
                opt.value = g; opt.textContent = g;
                groupSelect.appendChild(opt);
            });

            if (group && index !== null) {
                // 编辑模式
                document.getElementById('upstream-modal-title').textContent = '编辑上游DNS';
                const u = this.state.config[group][index];
                
                groupSelect.value = group;
                groupSelect.disabled = true; // 编辑时不允许改组
                document.getElementById('upstream-original-tag').value = index; 
                
                form.elements['tag'].value = u.tag || '';
                protocolSelect.value = u.protocol || 'udp';
                
                form.elements['addr'].value = u.addr || '';
                form.elements['dial_addr'].value = u.dial_addr || '';
                form.elements['socks5'].value = u.socks5 || '';
                form.elements['bootstrap'].value = u.bootstrap || '';
                form.elements['bootstrap_version'].value = u.bootstrap_version || 0;
                
                if(form.elements['enable_pipeline']) form.elements['enable_pipeline'].checked = !!u.enable_pipeline;
                if(form.elements['enable_http3']) form.elements['enable_http3'].checked = !!u.enable_http3;
                if(form.elements['insecure_skip_verify']) form.elements['insecure_skip_verify'].checked = !!u.insecure_skip_verify;
                form.elements['idle_timeout'].value = u.idle_timeout || '';
                form.elements['upstream_query_timeout'].value = u.upstream_query_timeout || '';
                form.elements['bind_to_device'].value = u.bind_to_device || '';
                form.elements['so_mark'].value = u.so_mark || '';

                form.elements['account_id'].value = u.account_id || '';
                form.elements['access_key_id'].value = u.access_key_id || '';
                form.elements['access_key_secret'].value = u.access_key_secret || '';
                form.elements['server_addr'].value = u.server_addr || '223.5.5.5';
                form.elements['ecs_client_ip'].value = u.ecs_client_ip || '';
                form.elements['ecs_client_mask'].value = u.ecs_client_mask || '';
                
            } else {
                // 添加模式
                document.getElementById('upstream-modal-title').textContent = '添加上游DNS';
                groupSelect.disabled = false;
                if(groupSelect.options.length > 0) groupSelect.selectedIndex = 0;
                document.getElementById('upstream-original-tag').value = -1;
                protocolSelect.value = 'aliapi'; 
            }

            this.updateFormFields(protocolSelect.value);
            lockScroll();
            modal.showModal();
        },

        updateFormFields(protocol) {
            const groupDns = document.getElementById('group-dns');
            const groupAliapi = document.getElementById('group-aliapi');
            const show = (sel) => groupDns.querySelectorAll(sel).forEach(el => el.style.display = 'block');
            const hide = (sel) => groupDns.querySelectorAll(sel).forEach(el => el.style.display = 'none');

            if (protocol === 'aliapi') {
                groupAliapi.style.display = 'block';
                groupDns.style.display = 'none';
            } else {
                groupAliapi.style.display = 'none';
                groupDns.style.display = 'block';

                hide('.field-socks5, .field-pipeline, .field-http3, .field-tls-verify');

                if (['tcp', 'dot', 'tls'].includes(protocol)) show('.field-pipeline');
                if (['https', 'doh', 'quic', 'doq'].includes(protocol)) show('.field-http3'); 
                if (['dot', 'tls', 'tcp', 'doh', 'https', 'quic', 'doq'].includes(protocol)) {
                     show('.field-tls-verify');
                     show('.field-socks5');
                }
            }
        },

async handleSave(formData) {
            const btn = document.getElementById('save-upstream-btn');
            ui.setLoading(btn, true);

            const groupSelect = document.getElementById('upstream-group');
            const pluginTag = groupSelect.value;
            if (!pluginTag) {
                ui.showToast('请选择所属组', 'error');
                ui.setLoading(btn, false);
                return;
            }

            const idx = parseInt(document.getElementById('upstream-original-tag').value, 10);
            
            // [新增] 先获取协议，用于清洗数据
            const protocol = formData.get('protocol'); 

            // 构建对象
            const newUpstream = {
                tag: formData.get('tag'),
                protocol: protocol,
                
                // [修改] 数据清洗逻辑：互斥字段置空
                
                // 1. DNS 通用字段 (非 AliAPI 时有效)
                addr: (protocol !== 'aliapi') ? formData.get('addr') : '',
                dial_addr: (protocol !== 'aliapi') ? formData.get('dial_addr') : '',
                idle_timeout: (protocol !== 'aliapi') ? (parseInt(formData.get('idle_timeout')) || 0) : 0,
                upstream_query_timeout: (protocol !== 'aliapi') ? (parseInt(formData.get('upstream_query_timeout')) || 0) : 0,
                bind_to_device: (protocol !== 'aliapi') ? formData.get('bind_to_device') : '',
                so_mark: (protocol !== 'aliapi') ? (parseInt(formData.get('so_mark')) || 0) : 0,
                
                enable_pipeline: (protocol !== 'aliapi') ? (formData.get('enable_pipeline') === 'on') : false,
                enable_http3: (protocol !== 'aliapi') ? (formData.get('enable_http3') === 'on') : false,
                insecure_skip_verify: (protocol !== 'aliapi') ? (formData.get('insecure_skip_verify') === 'on') : false,
                socks5: (protocol !== 'aliapi') ? formData.get('socks5') : '',
                bootstrap: (protocol !== 'aliapi') ? formData.get('bootstrap') : '',
                bootstrap_version: (protocol !== 'aliapi') ? (parseInt(formData.get('bootstrap_version')) || 0) : 0,
                
                // 2. AliAPI 专用字段 (仅 AliAPI 时有效)
                account_id: (protocol === 'aliapi') ? formData.get('account_id') : '',
                access_key_id: (protocol === 'aliapi') ? formData.get('access_key_id') : '',
                access_key_secret: (protocol === 'aliapi') ? formData.get('access_key_secret') : '',
                server_addr: (protocol === 'aliapi') ? formData.get('server_addr') : '',
                ecs_client_ip: (protocol === 'aliapi') ? formData.get('ecs_client_ip') : '',
                ecs_client_mask: (protocol === 'aliapi') ? (parseInt(formData.get('ecs_client_mask')) || 0) : 0,
            };

            // 安全初始化
            if (!this.state.config || typeof this.state.config !== 'object' || Array.isArray(this.state.config)) {
                this.state.config = {};
            }
            if (!Array.isArray(this.state.config[pluginTag])) {
                this.state.config[pluginTag] = [];
            }

            let list = this.state.config[pluginTag];

            if (idx >= 0 && list[idx]) {
                // 编辑：保留原 Enabled 状态
                newUpstream.enabled = list[idx].enabled;
                list[idx] = newUpstream;
            } else {
                // 新增：默认启用
                newUpstream.enabled = true;
                list.push(newUpstream);
            }

            try {
                // 保存到后端
                await api.fetch('/api/v1/upstream/config', {
                    method: 'POST',
                    headers: {'Content-Type': 'application/json'},
                    body: JSON.stringify({ 
                        plugin_tag: pluginTag, 
                        upstreams: list 
                    })
                });
                ui.showToast('保存成功');
                closeAndUnlock(document.getElementById('upstream-modal'));
                await this.loadData();
            } catch (e) {
                console.error("Save upstream error:", e);
                ui.showToast('保存失败: ' + e.message, 'error');
            } finally {
                ui.setLoading(btn, false);
            }
        },
        
        async toggleEnable(group, index, enabled) {
            const list = this.state.config[group];
            if (!list || !list[index]) return;
            // 乐观更新
            list[index].enabled = enabled;
            try {
                await api.fetch('/api/v1/upstream/config', {
                    method: 'POST',
                    headers: {'Content-Type': 'application/json'},
                    body: JSON.stringify({ plugin_tag: group, upstreams: list })
                });
                this.renderTable();
            } catch (e) {
                ui.showToast('状态更新失败', 'error');
                // 回滚
                list[index].enabled = !enabled;
                this.renderTable();
            }
        },

        async deleteUpstream(group, index) {
            if (!confirm('确定要删除此上游吗？')) return;
            const list = this.state.config[group];
            list.splice(index, 1);
            try {
                await api.fetch('/api/v1/upstream/config', {
                    method: 'POST',
                    headers: {'Content-Type': 'application/json'},
                    body: JSON.stringify({ plugin_tag: group, upstreams: list })
                });
                ui.showToast('已删除');
                this.renderTable();
            } catch (e) {
                ui.showToast('删除失败', 'error');
            }
        },

        async restartService() {
            if(!confirm('重启 MosDNS 以应用更改？')) return;
            try {
                await api.fetch('/api/v1/system/restart', { method: 'POST', body: JSON.stringify({ delay_ms: 500 }) });
                ui.showToast('正在重启...', 'success');
                setTimeout(() => location.reload(), 4000);
            } catch(e) {
                ui.showToast('重启请求失败', 'error');
            }
        }
    };
    // [插入点结束]

    function setupEventListeners() {
        // -- [修改] -- 统一处理所有弹窗的关闭行为（遮罩层点击和ESC键）
        document.querySelectorAll('dialog').forEach(dialog => {
            // 1. 点击遮罩层时关闭
            dialog.addEventListener('click', (event) => {
                // [修改点] 增加判断：如果当前弹窗是 upstream-modal，则不响应遮罩层点击
                if (event.target === dialog && dialog.id !== 'upstream-modal') {
                    closeAndUnlock(dialog);
                }
            });
            // 2. 按 ESC 键时关闭
            dialog.addEventListener('cancel', (event) => {
                event.preventDefault(); // 阻止浏览器默认的关闭行为（因为我们要在 closeAndUnlock 里处理滚动条解锁）
                
                // 如果你也希望 ESC 键在 upstream-modal 时失效，可以把下面的判断加上
                // 但通常 ESC 关闭是符合用户习惯的，建议保留
                closeAndUnlock(dialog); 
            });
        });

        elements.tabLinks.forEach(link => link.addEventListener('click', (e) => { e.preventDefault(); handleNavigation(link); }));
        // 覆盖配置：按钮事件
        if (elements.overridesLoadBtn) elements.overridesLoadBtn.addEventListener('click', () => overridesManager.load(false));
        if (elements.overridesSaveBtn) elements.overridesSaveBtn.addEventListener('click', () => overridesManager.save());
        window.addEventListener('popstate', () => { const hash = window.location.hash || '#overview'; const targetLink = document.querySelector(`.tab-link[href="${hash}"]`); handleNavigation(targetLink || elements.tabLinks[0]); });
        window.addEventListener('resize', debounce(handleResize, 150));
        elements.globalRefreshBtn?.addEventListener('click', () => updatePageData(true));
        setInterval(updateLastUpdated, 5000);
        [elements.autoRefreshToggle, elements.autoRefreshIntervalInput].forEach(el => el && el.addEventListener('change', () => { const enabled = elements.autoRefreshToggle.checked; elements.autoRefreshIntervalInput.disabled = !enabled; autoRefreshManager.updateSettings(enabled, parseInt(elements.autoRefreshIntervalInput.value, 10) || 15); }));
        document.addEventListener('visibilitychange', () => document.hidden ? autoRefreshManager.stop() : autoRefreshManager.start());

        elements.toggleAuditBtn?.addEventListener('click', async (e) => {
            const btn = e.currentTarget;
            ui.setLoading(btn, true);
            try {
                await (state.isCapturing ? api.stop() : api.start());
                await updatePageData(true);
            } catch (error) {
                console.error("操作失败:", error);
                ui.setLoading(btn, false);
            }
        });

        elements.clearAuditBtn?.addEventListener('click', async (e) => {
            if (confirm('确定要清空所有内存审计日志吗？此操作不可恢复。')) {
                const btn = e.currentTarget;
                ui.setLoading(btn, true);
                try {
                    await api.clear();
                    ui.showToast('日志已清空', 'success');
                    if (elements.logSearch) elements.logSearch.value = '';
                    await updatePageData(true);
                } catch (error) {
                    ui.showToast('清空日志失败', 'error');
                } finally {
                    ui.setLoading(btn, false);
                }
            }
        });

        elements.capacityForm?.addEventListener('submit', async (e) => {
            e.preventDefault();
            const newCapacity = parseInt(elements.newCapacityInput.value, 10);
            if (!newCapacity || newCapacity <= 0 || newCapacity > 400000) {
                ui.showToast('请输入1到400000之间的有效容量', 'error');
                return;
            }
            if (confirm(`确定要将容量设置为 ${newCapacity.toLocaleString()} 吗？\n\n注意：这将清空当前所有日志。`)) {
                const btn = e.currentTarget.querySelector('button');
                ui.setLoading(btn, true);
                try {
                    await api.setCapacity(newCapacity);
                    ui.showToast(`容量已成功设置为 ${newCapacity.toLocaleString()}`, 'success');
                    elements.newCapacityInput.value = '';
                    if (elements.logSearch) elements.logSearch.value = '';
                    await updatePageData(true);
                } catch (error) {
                    console.error("Set capacity failed:", error);
                } finally {
                    ui.setLoading(btn, false);
                }
            }
        });

        elements.logSearch?.addEventListener('input', debounce(applyLogFilterAndRender, 300));
        elements.logQueryTableContainer?.addEventListener('scroll', () => { const { scrollTop, scrollHeight, clientHeight } = elements.logQueryTableContainer; if (clientHeight + scrollTop >= scrollHeight - 200) loadMoreLogs(); }, { passive: true });

const handleInteractiveClick = (e) => {
            const interactiveButton = e.target.closest('.copy-btn, .filter-btn');
            const clickableLink = e.target.closest('.clickable-link, .tab-link-action');
            // [修改] 增加选择器精确度，确保能稳定获取到行元素
            const logRow = e.target.closest('tr[data-log-index], tr[data-rank-index]');

            if (interactiveButton) {
                e.stopPropagation();
                if (interactiveButton.matches('.copy-btn')) {
                    const textToCopy = interactiveButton.dataset.copyValue;
                    if (navigator.clipboard && window.isSecureContext) {
                        navigator.clipboard.writeText(textToCopy).then(() => {
                            ui.showToast('已复制到剪贴板');
                        }).catch(() => {
                            ui.showToast('复制失败', 'error');
                        });
                    } else {
                        const textArea = document.createElement("textarea");
                        textArea.value = textToCopy;
                        textArea.style.position = "absolute";
                        textArea.style.left = "-9999px";
                        const parentElement = elements.logDetailModal.open ? elements.logDetailModal : document.body;
                        parentElement.appendChild(textArea);
                        textArea.select();
                        try {
                            document.execCommand('copy');
                            ui.showToast('已复制到剪贴板');
                        } catch (err) {
                            ui.showToast('复制失败', 'error');
                        } finally {
                            parentElement.removeChild(textArea);
                        }
                    }
                } else if (interactiveButton.matches('.filter-btn')) {
                    const value = interactiveButton.dataset.filterValue;
                    const isExact = interactiveButton.dataset.exactSearch === 'true';
                    elements.logSearch.value = isExact ? `"${value}"` : value;
                    const logQueryLink = document.querySelector('.tab-link[href="#log-query"]');
                    if (logQueryLink && !logQueryLink.classList.contains('active')) {
                        handleNavigation(logQueryLink);
                    } else {
                        applyLogFilterAndRender();
                    }
                    tooltipManager.hide();
                    if (elements.logDetailModal.open) elements.logDetailModal.close();
                }
            } else if (clickableLink) {
                e.preventDefault();
                if (clickableLink.matches('.clickable-link')) {
                    const value = clickableLink.dataset.filterValue;
                    const isExact = clickableLink.dataset.exactSearch === 'true';
                    elements.logSearch.value = isExact ? `"${value}"` : value;
                    const logQueryLink = document.querySelector('.tab-link[href="#log-query"]');
                    if (logQueryLink) {
                        if (!logQueryLink.classList.contains('active')) {
                            handleNavigation(logQueryLink);
                        }
                        applyLogFilterAndRender();
                    }
                } else if (clickableLink.matches('.tab-link-action')) {
                    const link = document.querySelector(`.tab-link[data-tab="${clickableLink.dataset.tab}"]`);
                    if (link) handleNavigation(link);
                }
            } else if (logRow) {
                // [修改] 增加错误捕获，防止数据缺失导致白屏
                try {
                    ui.openLogDetailModal(logRow);
                } catch (err) {
                    console.error("Open modal failed:", err);
                    ui.showToast("无法加载详情", "error");
                }
            }
        };

        elements.body.addEventListener('click', handleInteractiveClick);
        elements.logDetailModal.addEventListener('click', handleInteractiveClick);

        elements.body.addEventListener('mouseover', e => { if (state.isTouchDevice) return; const trigger = e.target.closest('[data-log-index], [data-rank-index], [data-rule-id]'); if (trigger) tooltipManager.handleTriggerEnter(trigger); });
        elements.body.addEventListener('mouseout', e => { if (state.isTouchDevice) return; const trigger = e.target.closest('[data-log-index], [data-rank-index], [data-rule-id]'); if (trigger) tooltipManager.handleTriggerLeave(); });
        elements.tooltip.addEventListener('mouseenter', () => tooltipManager.handleTooltipEnter());
        elements.tooltip.addEventListener('mouseleave', () => tooltipManager.handleTooltipLeave());

        // -- [修改] -- 所有关闭按钮都使用新的统一函数
        elements.closeLogDetailModalBtn?.addEventListener('click', () => closeAndUnlock(elements.logDetailModal));

        if (elements.aliasModal) {
            [elements.manageAliasesBtn, elements.manageAliasesBtnMobile].forEach(btn => btn?.addEventListener('click', async () => {
                await aliasManager.renderEditableList();
                lockScroll();
                elements.aliasModal.showModal();
            }));
            document.getElementById('close-alias-modal')?.addEventListener('click', () => closeAndUnlock(elements.aliasModal));

            elements.saveAllAliasesBtn?.addEventListener('click', async () => {
                const btn = elements.saveAllAliasesBtn;
                ui.setLoading(btn, true);
                try {
                    await aliasManager.saveAll();
                } finally {
                    ui.setLoading(btn, false);
                }
            });

            document.getElementById('export-aliases-btn')?.addEventListener('click', async (e) => {
                const btn = e.currentTarget;
                ui.setLoading(btn, true);
                try {
                    await aliasManager.export();
                } finally {
                    ui.setLoading(btn, false);
                }
            });

            document.getElementById('import-aliases-btn')?.addEventListener('click', () => elements.importAliasInput?.click());
            elements.importAliasInput?.addEventListener('change', (e) => { if (e.target.files?.length > 0) { aliasManager.import(e.target.files[0]); e.target.value = ''; } });

            elements.manualAliasForm.addEventListener('submit', async (e) => {
                e.preventDefault();
                const btn = e.currentTarget.querySelector('button');
                ui.setLoading(btn, true);
                const ip = document.getElementById('manual-alias-ip').value.trim();
                const name = document.getElementById('manual-alias-name').value.trim();
                try {
                    if (ip && name) {
                        aliasManager.set(ip, name);
                        await aliasManager.save();
                        ui.showToast(`已添加别名: ${name} -> ${ip}`, 'success');
                        e.target.reset();
                        await aliasManager.renderEditableList();
                        await updatePageData(false);
                    } else {
                        ui.showToast('IP地址和别名均不能为空', 'error');
                    }
                } catch (err) { }
                finally {
                    ui.setLoading(btn, false);
                }
            });
        }
        elements.ruleForm.addEventListener('submit', handleRuleFormSubmit);
        elements.closeRuleModalBtn.addEventListener('click', () => closeAndUnlock(elements.ruleModal));
        elements.cancelRuleModalBtn.addEventListener('click', () => closeAndUnlock(elements.ruleModal));
        elements.addAdguardRuleBtn.addEventListener('click', () => ui.openRuleModal('adguard'));
        elements.checkAdguardUpdatesBtn.addEventListener('click', handleAdguardUpdateCheck);
        elements.adguardRulesTbody.addEventListener('click', (e) => handleRuleTableClick(e, 'adguard'));
        elements.addDiversionRuleBtn.addEventListener('click', () => ui.openRuleModal('diversion'));
        elements.diversionRulesTbody.addEventListener('click', (e) => handleRuleTableClick(e, 'diversion'));

        elements.rulesSubNavLinks.forEach(link => {
            link.addEventListener('click', () => {
                elements.rulesSubNavLinks.forEach(l => l.classList.remove('active'));
                link.classList.add('active');
                const tabId = link.dataset.subTab;
                elements.rulesSubTabContents.forEach(content => {
                    content.classList.toggle('active', content.id === `${tabId}-sub-tab`);
                });

                if (tabId === 'list-mgmt' && !state.listManagerInitialized) {
                    listManager.init();
                } else if (tabId === 'adguard' && state.adguardRules.length === 0) {
                    renderSkeletonRows(elements.adguardRulesTbody, 5, state.isMobile ? 1 : 6);
                    adguardManager.load();
                } else if (tabId === 'diversion' && state.diversionRules.length === 0) {
                    renderSkeletonRows(elements.diversionRulesTbody, 5, state.isMobile ? 1 : 7);
                    diversionManager.load();
                }
            });
        });


        document.body.addEventListener('click', (e) => {
            const domainListLink = e.target.closest('a.control-item-link[data-list-type]');
            const cacheListLink = e.target.closest('a.control-item-link[data-cache-tag]');
            const clearCacheBtn = e.target.closest('.clear-cache-btn[data-cache-tag]');

            if (domainListLink) {
                e.preventDefault();
                openDataViewModal({
                    listType: domainListLink.dataset.listType,
                    title: domainListLink.dataset.listTitle
                });
            } else if (cacheListLink) {
                e.preventDefault();
                openDataViewModal({
                    cacheTag: cacheListLink.dataset.cacheTag,
                    title: cacheListLink.dataset.cacheTitle
                });
            } else if (clearCacheBtn) {
                e.preventDefault();
                const cacheTag = clearCacheBtn.dataset.cacheTag;
                if (confirm(`确定要清空缓存 "${cacheTag}" 吗？`)) {
                    ui.setLoading(clearCacheBtn, true);
                    api.clearCache(cacheTag)
                        .then(() => {
                            ui.showToast(`缓存 "${cacheTag}" 已清空`, 'success');
                            return cacheManager.updateStats();
                        })
                        .catch(err => {
                            ui.showToast(`清空缓存 "${cacheTag}" 失败`, 'error');
                        })
                        .finally(() => {
                            // The button is part of a re-rendered table, so no need to setLoading(false)
                        });
                }
            }
        });

        // 1. 数据查看弹窗关闭监听
        if (elements.closeDataViewModalBtn) {
            elements.closeDataViewModalBtn.addEventListener('click', function() {
                closeAndUnlock(elements.dataViewModal);
            });
        }

        // 2. 数据查看搜索输入监听 (统一走后端分页和搜索)
        if (elements.dataViewSearch) {
            elements.dataViewSearch.addEventListener('input', debounce(function() {
                var searchTerm = elements.dataViewSearch.value.trim();
                // 更新全局查询词状态
                state.dataView.currentQuery = searchTerm;
                // 无论是缓存还是域名列表，现在都统一触发后端分页查询接口
                openDataViewModal(state.dataView.currentConfig, false);
            }, 400));
        }

        // 3. 分流规则重要操作按钮监听
        if (elements.saveShuntRulesBtn) {
            elements.saveShuntRulesBtn.addEventListener('click', saveAllShuntRules);
        }
        if (elements.clearShuntRulesBtn) {
            elements.clearShuntRulesBtn.addEventListener('click', clearAllShuntRules);
        }
    }

    function setupLazyLoading() {
        const lazyLoadObserver = new IntersectionObserver((entries, observer) => {
            entries.forEach(entry => {
                if (entry.isIntersecting) {
                    const card = entry.target;
                    const cardId = card.id;
                    switch (cardId) {
                        case 'top-domains-card': api.v2.getTopDomains(null, 100).then(data => { state.topDomains = data || []; renderTopDomains(state.topDomains); }).catch(console.error); break;
                        case 'top-clients-card': api.v2.getTopClients(null, 100).then(data => { state.topClients = data || []; renderTopClients(state.topClients); }).catch(console.error); break;
                        case 'slowest-queries-card': api.v2.getSlowest(null, 100).then(data => { state.slowestQueries = data || []; renderSlowestQueries(state.slowestQueries); }).catch(console.error); break;
                        case 'shunt-results-card': api.v2.getDomainSetRank(null, 100).then(data => { state.domainSetRank = data || []; renderDonutChart(state.domainSetRank); }).catch(console.error); break;
                    }
                    observer.unobserve(card);
                }
            });
        }, { rootMargin: "50px" });
        document.querySelectorAll('.lazy-load-card').forEach(card => lazyLoadObserver.observe(card));
    }

    // 系统控制页模块懒加载：模块进入可视区时才请求
    function setupSystemControlLazyLoading() {
        const root = document.getElementById('system-control-tab');
        if (!root) return;
        const seen = new Set();
        const map = new Map();

        // 简易并发队列，避免系统控制页一次性触发过多请求
        const SYS_MAX = 2; // 同时最多执行2个模块任务
        const queue = [];
        let running = 0;
        const pump = () => {
            while (running < SYS_MAX && queue.length > 0) {
                const job = queue.shift();
                running++;
                Promise.resolve()
                    .then(job)
                    .catch(() => { })
                    .finally(() => { running--; pump(); });
            }
        };
        const enqueue = (fn) => { queue.push(fn); pump(); };
        const io = new IntersectionObserver((entries) => {
            entries.forEach(entry => {
                if (entry.isIntersecting && !seen.has(entry.target)) {
                    seen.add(entry.target);
                    const fn = map.get(entry.target);
                    if (typeof fn === 'function') enqueue(() => {
                        if (typeof window !== 'undefined' && 'requestIdleCallback' in window) {
                            return new Promise((resolve) => window.requestIdleCallback(() => { fn(); resolve(); }, { timeout: 1500 }));
                        }
                        return new Promise((resolve) => setTimeout(() => { fn(); resolve(); }, 300));
                    });
                }
            });
        }, { root, rootMargin: '50px' });

        const watch = (selector, fn) => { const el = document.querySelector(selector); if (!el) return; map.set(el, fn); io.observe(el); };
        watch('#system-info-module', () => systemInfoManager.load());
        watch('#update-module', () => updateManager.refreshStatus(false));
        watch('#feature-switches-module', () => switchManager.loadStatus());
        watch('#domain-stats-module', () => updateDomainListStats());
        watch('#requery-module', () => requeryManager.updateStatus());
        watch('#overrides-module', () => overridesManager.load(true));
        watch('#cache-stats-table', () => cacheManager.updateStats());
        watch('#upstream-dns-module', () => upstreamManager.loadData());
    }

    async function init() {
        state.isTouchDevice = ('ontouchstart' in window) || (navigator.maxTouchPoints > 0);
        themeManager.init();
        // 根据进入页签决定是否首屏加载别名（仅日志/概览需要）。避免 system-control 首屏的额外请求。
        const firstHash = window.location.hash || '#overview';
        const firstTab = (document.querySelector(`.tab-link[href="${firstHash}"]`)?.dataset.tab) || firstHash.replace('#', '');
        const loadAliasesAsync = () => aliasManager.load().then(() => {
            // 别名加载后，如当前在 log-query，轻量重渲染以显示别名
            const activeTab = document.querySelector('.tab-link.active')?.dataset.tab;
            if (activeTab === 'log-query' && state.displayedLogs.length) {
                ui.renderLogTable(state.displayedLogs, false);
            }
        });
        if (firstTab === 'overview' || firstTab === 'log-query') {
            // 不阻塞首屏：并行加载别名
            loadAliasesAsync();
        } else {
            // 延后到空闲时加载，供后续切换使用
            if ('requestIdleCallback' in window) requestIdleCallback(loadAliasesAsync);
            else setTimeout(loadAliasesAsync, 1500);
        }
        historyManager.load();
        autoRefreshManager.loadSettings();
        tableSorter.init();
        switchManager.init();
        // 绑定 info-icon 提示（例如日志容量的说明图标）
        bindInfoIconTooltips();
        updateManager.init();

        // -- [修改] -- 初始化配置管理器
        configManager.init();

        mountGlobalInfoIconDelegation();
        setupEventListeners();
        setupGlowEffect();
        setupLazyLoading();
        setupSystemControlLazyLoading();

        // -- [新增] -- 移动端预加载优化：提前加载常用模块数据
        if (window.innerWidth <= CONSTANTS.MOBILE_BREAKPOINT) {
            // 延迟500ms后开始预加载，避免阻塞首屏
            setTimeout(() => {
                // 预加载系统控制页的关键模块
                Promise.allSettled([
                    switchManager.loadStatus(),
                    overridesManager.load(true),
                    updateManager.refreshStatus(false)
                ]).catch(() => { });
            }, 500);
        }

        handleResize();
        const initialHash = firstHash;
        const initialLink = document.querySelector(`.tab-link[href="${initialHash}"]`);
        if (initialLink) handleNavigation(initialLink);
        // 首屏统一轻量刷新，所有重数据由懒加载或"刷新"按钮触发
        await updatePageData(false);
        if (document.fonts?.ready) await document.fonts.ready;
        requestAnimationFrame(() => { const activeLink = document.querySelector('.tab-link.active'); if (activeLink) updateNavSlider(activeLink); });
        elements.initialLoader.style.opacity = '0';
        elements.initialLoader.addEventListener('transitionend', () => elements.initialLoader.remove());
        if (!document.hidden) autoRefreshManager.start();
        requeryManager.init();
        upstreamManager.init();
    }

    init();
});

// ===============================================
// 系统控制子菜单模块 (独立初始化)
// ===============================================
document.addEventListener('DOMContentLoaded', function () {
    // 延迟初始化，确保主代码已执行
    initSystemSubNav();

    function initSystemSubNav() {
        const systemTab = document.getElementById('system-control-tab');
        if (!systemTab) return;

        const grid = systemTab.querySelector('.control-panel-grid');
        if (!grid || systemTab.querySelector('.system-sub-nav')) return; // 已存在则跳过

        // 创建子导航
        const subNav = document.createElement('nav');
        subNav.className = 'system-sub-nav';
        subNav.setAttribute('role', 'tablist');
        subNav.innerHTML = `
            <button class="system-sub-nav-btn active" data-category="all" role="tab">
                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M3 13h8V3H3v10zm0 8h8v-6H3v6zm10 0h8V11h-8v10zm0-18v6h8V3h-8z"/></svg>
                <span>全部</span>
            </button>
            <button class="system-sub-nav-btn" data-category="basic" role="tab">
                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M12.9 6.858l4.242 4.243L7.242 21H3v-4.243l9.9-9.9zm1.414-1.414l2.121-2.122a1 1 0 0 1 1.414 0l2.829 2.829a1 1 0 0 1 0 1.414l-2.122 2.121-4.242-4.242z"/></svg>
                <span>基础设置</span>
            </button>
            <button class="system-sub-nav-btn" data-category="data" role="tab">
                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M4 19h16v-7h2v8a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1v-8h2v7zM20 3H4v7h2V5h12v5h2V4a1 1 0 0 0-1-1z"/></svg>
                <span>数据管理</span>
            </button>
            <button class="system-sub-nav-btn" data-category="system" role="tab">
                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M12 1l9.5 5.5v11L12 23l-9.5-5.5v-11L12 1zm0 2.31L4.5 7.65v8.7l7.5 4.34 7.5-4.34V7.65L12 3.31z"/></svg>
                <span>系统信息</span>
            </button>
            <button class="system-sub-nav-btn" data-category="advanced" role="tab">
                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z"/></svg>
                <span>高级设置</span>
            </button>
        `;
        systemTab.insertBefore(subNav, grid);

        // 给模块添加分类 (按模块顺序)
        const modules = grid.querySelectorAll('.control-module');
        const moduleArray = Array.from(modules);

        moduleArray.forEach(function (mod, index) {
            // 已有分类则跳过
            if (mod.dataset.category) return;

            const id = mod.id;

            // 按 ID 分配类别
            if (id === 'auto-refresh-module' || id === 'appearance-module') {
                mod.dataset.category = 'basic';
            } else if (id === 'domain-stats-module' || id === 'requery-module') {
                mod.dataset.category = 'data';
            } else if (id === 'system-info-module' || id === 'update-module') {
                mod.dataset.category = 'system';
            } else if (id === 'feature-switches-module' || id === 'overrides-module') {
                mod.dataset.category = 'advanced';
            } else if (mod.querySelector('#cache-stats-table')) {
                mod.dataset.category = 'data';
            } else if (index < 4 && mod.classList.contains('control-module--mini')) {
                // 前4个 mini 模块（审计、日志容量、自动刷新、外观）属于基础设置
                mod.dataset.category = 'basic';
            } else {
                // 未分类的模块默认放入高级设置
                mod.dataset.category = 'advanced';
            }
        });

        // 分类函数 - 给新模块分配类别
        function categorizeModules() {
            grid.querySelectorAll('.control-module').forEach(function (mod, index) {
                if (mod.dataset.category) return; // 已分类跳过

                const id = mod.id;
                if (id === 'auto-refresh-module' || id === 'appearance-module') {
                    mod.dataset.category = 'basic';
                } else if (id === 'domain-stats-module' || id === 'requery-module') {
                    mod.dataset.category = 'data';
                } else if (id === 'system-info-module' || id === 'update-module') {
                    mod.dataset.category = 'system';
                } else if (id === 'feature-switches-module' || id === 'overrides-module' || id === 'replacements-card' || id === 'socks-ecs-module') {
                    mod.dataset.category = 'advanced';
                } else if (mod.querySelector('#cache-stats-table')) {
                    mod.dataset.category = 'data';
                } else if (mod.classList.contains('control-module--mini')) {
                    mod.dataset.category = 'basic';
                } else {
                    mod.dataset.category = 'advanced';
                }
            });
        }

        // 子导航点击事件
        subNav.addEventListener('click', function (e) {
            var btn = e.target.closest('.system-sub-nav-btn');
            if (!btn) return;

            var category = btn.dataset.category;

            // 更新按钮激活状态
            subNav.querySelectorAll('.system-sub-nav-btn').forEach(function (b) {
                b.classList.remove('active');
            });
            btn.classList.add('active');

            // 重新分类（处理动态加载的模块）
            categorizeModules();

            // 获取最新模块列表并过滤
            var allModules = grid.querySelectorAll('.control-module');
            allModules.forEach(function (mod) {
                var modCat = mod.dataset.category;
                if (category === 'all' || modCat === category) {
                    mod.style.display = '';
                } else {
                    mod.style.display = 'none';
                }
            });

            // 处理配置管理卡片 (config-manager-card)
            var configCard = document.getElementById('config-manager-card');
            if (configCard) {
                // 配置管理属于"系统信息"分类
                if (category === 'all' || category === 'system') {
                    configCard.style.display = '';
                } else {
                    configCard.style.display = 'none';
                }
            }
        });

        // 监听动态添加的模块
        var currentCategory = 'basic'; // 默认显示基础设置
        // 初始化时立即触发一次过滤
        var basicBtn = subNav.querySelector('.system-sub-nav-btn[data-category="basic"]');
        if (basicBtn) basicBtn.click();

        var observer = new MutationObserver(function (mutations) {
            mutations.forEach(function (mutation) {
                mutation.addedNodes.forEach(function (node) {
                    if (node.nodeType === 1 && node.classList && node.classList.contains('control-module')) {
                        // 给新模块分类
                        if (!node.dataset.category) {
                            node.dataset.category = 'advanced'; // 默认高级设置
                        }
                        // 根据当前选中的分类决定是否显示
                        var activeBtn = subNav.querySelector('.system-sub-nav-btn.active');
                        if (activeBtn) {
                            var cat = activeBtn.dataset.category;
                            if (cat !== 'all' && node.dataset.category !== cat) {
                                node.style.display = 'none';
                            }
                        }
                    }
                });
            });
        });
        observer.observe(grid, { childList: true, subtree: false });

        console.log('System sub-navigation initialized');

        // 绑定分流规则帮助按钮
        const helpBtn = document.getElementById('diversion-help-btn');
        if (helpBtn) {
            helpBtn.addEventListener('click', function () {
                const modalHtml = `
                    <dialog id="help-modal" class="card" style="padding: 0; max-width: 500px; width: 90%; border: none; box-shadow: var(--shadow-lg); border-radius: var(--border-radius-lg);">
                        <header class="card-header" style="display: flex; justify-content: space-between; align-items: center; padding: 1rem 1.5rem; border-bottom: 1px solid var(--color-border);">
                            <h3 style="margin: 0;">分流规则说明</h3>
                            <button class="button icon-only" onclick="this.closest('dialog').close(); this.closest('dialog').remove();" style="background: transparent; border: none; cursor: pointer;">
                                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" width="24" height="24"><path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"></path></svg>
                            </button>
                        </header>
                        <div class="card-body" style="padding: 1.5rem;">
                            <div style="line-height: 1.6; font-size: 0.95rem;">
                                <p style="margin-bottom: 0.5rem;"><strong>geositecn:</strong> 中国大陆域名列表，用于直连。</p>
                                <p style="margin-bottom: 0.5rem;"><strong>geositenocn:</strong> 非中国大陆域名列表，用于代理。</p>
                                <p style="margin-bottom: 0.5rem;"><strong>geoipcn:</strong> 中国大陆 IP 列表。</p>
                                <p style="margin-bottom: 0.5rem;"><strong>cuscn:</strong> 自定义中国大陆域名。</p>
                                <p style="margin-bottom: 0.5rem;"><strong>cusnocn:</strong> 自定义非中国大陆域名。</p>
                            </div>
                        </div>
                        <footer class="modal-footer" style="padding: 1rem 1.5rem; border-top: 1px solid var(--color-border); text-align: right;">
                            <button class="button primary" onclick="this.closest('dialog').close(); this.closest('dialog').remove();">关闭</button>
                        </footer>
                    </dialog>
                `;
                document.body.insertAdjacentHTML('beforeend', modalHtml);
                const modal = document.getElementById('help-modal');
                modal.showModal();
                modal.addEventListener('click', (e) => {
                    if (e.target === modal) {
                        modal.close();
                        modal.remove();
                    }
                });
            });
        }
    }
});
