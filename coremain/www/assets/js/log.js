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
    const CONSTANTS = { API_BASE_URL: '', LOGS_PER_PAGE: 50, HISTORY_LENGTH: 30, DEFAULT_AUTO_REFRESH_INTERVAL: 15, ANIMATION_DURATION: 1000, MOBILE_BREAKPOINT: 768, TOAST_DURATION: 3000, SKELETON_ROWS: 10, TOOLTIP_SHOW_DELAY: 200, TOOLTIP_HIDE_DELAY: 250 };
    let state = { isUpdating: false, isCapturing: false, isMobile: false, isTouchDevice: false, currentLogPage: 1, isLogLoading: false, logPaginationInfo: null, displayedLogs: [], currentLogSearchTerm: '', clientAliases: {}, topDomains: [], topClients: [], slowestQueries: [], domainSetRank: [], shuntColors: {}, logSort: { key: 'query_time', order: 'desc' }, autoRefresh: { enabled: false, intervalId: null, intervalSeconds: CONSTANTS.DEFAULT_AUTO_REFRESH_INTERVAL }, data: { totalQueries: { current: null, previous: null }, avgDuration: { current: null, previous: null } }, history: { totalQueries: [], avgDuration: [] }, lastUpdateTime: null, adguardRules: [], diversionRules: [], requery: { status: null, config: null, pollId: null }, dataView: { rawEntries: [], filteredEntries: [] }, coreMode: 'A', cacheStats: {}, listManagerInitialized: false, featureSwitches: {}, systemInfo: {} };
    const elements = { 
        html: document.documentElement, body: document.body, container: document.querySelector('.container'), initialLoader: document.getElementById('initial-loader'), 
        colorSwatches: document.querySelectorAll('.color-swatch'), 
        themeSwitcher: document.getElementById('theme-switcher-select'),
        layoutSwitcher: document.getElementById('layout-density-select'),
        mainNav: document.querySelector('.main-nav'), navSlider: document.querySelector('.main-nav-slider'), 
        tabLinks: document.querySelectorAll('.tab-link'), tabContents: document.querySelectorAll('.tab-content'), 
        globalRefreshBtn: document.getElementById('global-refresh-btn'), lastUpdated: document.getElementById('last-updated'), 
        autoRefreshToggle: document.getElementById('auto-refresh-toggle'), autoRefreshIntervalInput: document.getElementById('auto-refresh-interval'), autoRefreshForm: document.getElementById('auto-refresh-form'), 
        totalQueries: document.getElementById('total-queries'), avgDuration: document.getElementById('avg-duration'), 
        totalQueriesChange: document.getElementById('total-queries-change'), avgDurationChange: document.getElementById('avg-duration-change'),
        sparklineTotal: document.getElementById('sparkline-total'), sparklineAvg: document.getElementById('sparkline-avg'), 
        auditStatus: document.getElementById('audit-status'), toggleAuditBtn: document.getElementById('toggle-audit-btn'), clearAuditBtn: document.getElementById('clear-audit-btn'), 
        auditCapacity: document.getElementById('audit-capacity'), capacityForm: document.getElementById('capacity-form'), newCapacityInput: document.getElementById('new-capacity'), 
        cacheStatsTbody: document.getElementById('cache-stats-tbody'),
        topDomainsBody: document.getElementById('top-domains-body'), topClientsBody: document.getElementById('top-clients-body'), slowestQueriesBody: document.getElementById('slowest-queries-body'), 
        shuntResultsBody: document.getElementById('shunt-results-body'),
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
        requeryStartDatetimeInput: document.getElementById('requery-start-datetime-input'),
        requeryClearBackupBtn: document.getElementById('requery-clear-backup-btn'),
        requeryDomainStatsTbody: document.getElementById('requery-domain-stats-tbody'),
        requeryRefreshStatsBtn: document.getElementById('requery-refresh-stats-btn'),

        fakeipDomainCount: document.getElementById('fakeip-domain-count'),
        realipDomainCount: document.getElementById('realip-domain-count'),
        nov4DomainCount: document.getElementById('nov4-domain-count'),
        nov6DomainCount: document.getElementById('nov6-domain-count'),
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
        listMgmtClientIpHint: document.getElementById('list-mgmt-client-ip-hint'),
        listMgmtRewriteHint: document.getElementById('list-mgmt-rewrite-hint'),

        featureSwitchesModule: document.getElementById('feature-switches-module'),
        coreModeSwitchGroup: document.getElementById('core-mode-switch-group'),
        secondarySwitchesContainer: document.getElementById('secondary-switches-container'),
        systemInfoContainer: document.getElementById('system-info-container'),
    };
    let toastTimeout;
    
    const SHUNT_RULE_SAVE_PATHS = ['top_domains/save','my_fakeiplist/save', 'my_nodenov4list/save', 'my_nodenov6list/save', 'my_notinlist/save', 'my_nov4list/save', 'my_nov6list/save', 'my_realiplist/save'];
    const SHUNT_RULE_FLUSH_PATHS = ['top_domains/flush', 'my_fakeiplist/flush', 'my_nodenov4list/flush', 'my_nodenov6list/flush', 'my_notinlist/flush', 'my_nov4list/flush', 'my_nov6list/flush', 'my_realiplist/flush'];

    const debounce = (func, wait) => { let timeout; return function executedFunction(...args) { const later = () => { clearTimeout(timeout); func(...args); }; clearTimeout(timeout); timeout = setTimeout(later, wait); }; };

    const api = { fetch: async (url, options = {}) => { try { const response = await fetch(url, { ...options, signal: options.signal }); if (!response.ok) { let errorMsg = `API Error: ${response.status} ${response.statusText}`; try { const errorBody = await response.json(); if (errorBody && errorBody.error) { errorMsg = errorBody.error; } } catch (e) { try { errorMsg = await response.text() || errorMsg; } catch (textErr) {} } if (response.status !== 404) { ui.showToast(errorMsg, 'error'); } throw new Error(errorMsg); } const contentType = response.headers.get('content-type'); if (contentType && contentType.includes('application/json')) return response.json(); return response.text(); } catch (error) { if (error.name !== 'AbortError') { console.error(error); } throw error; } }, getStatus: (signal) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/status`, { signal }), getCapacity: (signal) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/capacity`, { signal }), start: () => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/start`, { method: 'POST' }), stop: () => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/stop`, { method: 'POST' }), clear: () => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/clear`, { method: 'POST' }), setCapacity: (capacity) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v1/audit/capacity`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ capacity: parseInt(capacity, 10) }) }), getMetrics: (signal) => api.fetch('/metrics', { signal }), getCoreMode: (signal) => api.fetch('/plugins/switch3/show', { signal }), clearCache: (cacheTag) => api.fetch(`/plugins/${cacheTag}/flush`), getCacheContents: (cacheTag, signal) => api.fetch(`/plugins/${cacheTag}/show`, { signal }), v2: { getStats: (signal) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/stats`, { signal }), getTopDomains: (signal, limit = 50) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/rank/domain?limit=${limit}`, { signal }), getTopClients: (signal, limit = 50) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/rank/client?limit=${limit}`, { signal }), getSlowest: (signal, limit = 50) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/rank/slowest?limit=${limit}`, { signal }), getDomainSetRank: (signal, limit = 50) => api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/rank/domain_set?limit=${limit}`, { signal }), getLogs: (signal, params = {}) => { const queryParams = new URLSearchParams({ page: 1, limit: CONSTANTS.LOGS_PER_PAGE, ...params }); for(let [key, value] of queryParams.entries()){ if(!value) { queryParams.delete(key); } } return api.fetch(`${CONSTANTS.API_BASE_URL}/api/v2/audit/logs?${queryParams}`, { signal }); } } };
    
    const requeryApi = {
        getConfig: (signal) => api.fetch(`/plugins/requery`, { signal }), 
        getStatus: (signal) => api.fetch(`/plugins/requery/status`, { signal }),
        trigger: () => api.fetch(`/plugins/requery/trigger`, { method: 'POST' }),
        cancel: () => api.fetch(`/plugins/requery/cancel`, { method: 'POST' }),
        updateSchedulerConfig: (config) => api.fetch(`/plugins/requery/scheduler/config`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(config) }),
        clearBackup: () => api.fetch(`/plugins/requery/clear_backup`, { method: 'POST' }),
        getBackupCount: (signal) => api.fetch(`/plugins/requery/stats/backup_file_count`, { signal }),
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
        updateStatus(isCapturing) { if (!elements.toggleAuditBtn || !elements.auditStatus) return; this.setLoading(elements.toggleAuditBtn, false); const statusIndicator = elements.systemControlTabIndicator; if(statusIndicator) statusIndicator.className = 'status-indicator'; if (typeof isCapturing === 'boolean') { state.isCapturing = isCapturing; elements.auditStatus.textContent = isCapturing ? '运行中' : '已停止'; elements.auditStatus.style.color = isCapturing ? 'var(--color-success)' : 'var(--color-danger)'; const actionText = isCapturing ? '关闭审计' : '开启审计'; elements.toggleAuditBtn.querySelector('span').textContent = actionText; elements.toggleAuditBtn.dataset.defaultText = actionText; elements.toggleAuditBtn.className = `button ${isCapturing ? 'danger' : 'primary'}`; if(statusIndicator) statusIndicator.classList.add(isCapturing ? 'running' : 'stopped'); } else { elements.auditStatus.textContent = '未知'; elements.auditStatus.style.color = 'var(--color-text-secondary)'; elements.toggleAuditBtn.querySelector('span').textContent = '刷新状态'; elements.toggleAuditBtn.dataset.defaultText = '刷新状态'; } },
        updateCapacity(capacity) { if (elements.auditCapacity) elements.auditCapacity.textContent = capacity != null ? `${capacity.toLocaleString()} 条` : '查询失败'; },
        updateOverviewStats() {
            const { totalQueries, avgDuration } = state.data;
            animateValue(elements.totalQueries, totalQueries.previous, totalQueries.current, CONSTANTS.ANIMATION_DURATION);
            animateValue(elements.avgDuration, avgDuration.previous, avgDuration.current, CONSTANTS.ANIMATION_DURATION, 2);
            updateStatChange(elements.totalQueriesChange, totalQueries.previous, totalQueries.current);
            updateStatChange(elements.avgDurationChange, avgDuration.previous, avgDuration.current, true);
            if (elements.sparklineTotal) elements.sparklineTotal.innerHTML = generateSparklineSVG(state.history.totalQueries); 
            if (elements.sparklineAvg) elements.sparklineAvg.innerHTML = generateSparklineSVG(state.history.avgDuration, true);
        },
        renderLogTable(logs, append = false) {
            const tbody = elements.logTableBody;
            if (!tbody) return;
            if (!append) { tbody.innerHTML = ''; state.displayedLogs = []; }
            if (logs.length === 0 && !append) { renderTable(tbody, [], () => {}, 'log-query'); return; }
            const startIndex = state.displayedLogs.length;
            state.displayedLogs.push(...logs);
            const fragment = document.createDocumentFragment();
            logs.forEach((log, i) => { 
                const row = renderLogItemHTML(log, startIndex + i);
                requestAnimationFrame(() => {
                    row.classList.add('animate-in');
                    row.style.animationDelay = `${i * 20}ms`; 
                });
                fragment.appendChild(row); 
            });
            tbody.appendChild(fragment);
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
            form.elements['type'].required = isDiversion;
            form.elements['files'].required = isDiversion;

            if (rule) {
                form.elements['id'].value = rule.id ?? rule.name;
                form.elements['name'].value = rule.name;
                form.elements['url'].value = rule.url;
                form.elements['auto_update'].checked = rule.auto_update;
                form.elements['update_interval_hours'].value = rule.update_interval_hours || 24;
                if (isDiversion) {
                    form.elements['type'].value = rule.type;
                    form.elements['files'].value = rule.files;
                }
            } else {
                form.elements['id'].value = '';
                form.elements['auto_update'].checked = true;
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
            elements.requeryStartDatetimeInput.addEventListener('change', debouncedUpdate);
            elements.requeryClearBackupBtn.addEventListener('click', this.handleClearBackup.bind(this));
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
                } catch (error) {} 
                finally {
                    ui.setLoading(btn, false);
                }
            }
        },
        
        async handleUpdateSchedulerConfig() {
             const isEnabled = elements.requerySchedulerToggle.checked;
             const interval = parseInt(elements.requeryIntervalInput.value, 10);
             const localTime = elements.requeryStartDatetimeInput.value;

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

             const newConfig = {
                 enabled: isEnabled,
                 interval_minutes: interval || 0,
                 start_datetime: utcTime,
             };

             try {
                 await requeryApi.updateSchedulerConfig(newConfig);
                 ui.showToast('定时任务配置已更新', 'success');
                 if (state.requery.config) {
                     state.requery.config.scheduler = newConfig;
                     this.render();
                 }
             } catch (error) {}
        },

        async handleClearBackup(e) {
            if (confirm('【重要操作】确定要清空全量域名备份文件吗？\n这将删除所有累积的历史域名，下次任务将只处理源文件中的域名。')) {
                 const btn = e.currentTarget;
                 ui.setLoading(btn, true);
                 try {
                     await requeryApi.clearBackup();
                     ui.showToast('全量备份文件已清空', 'success');
                 } catch (error) {} 
                 finally {
                     ui.setLoading(btn, false);
                 }
            }
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
                            html += `
                                <tr>
                                    <td>${item.alias}</td>
                                    <td class="text-right">${item.count.toLocaleString()}</td>
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
            { tag: 'switch3', name: '核心运行模式', tip: '切换后将执行一次“全新任务”刷新分流缓存。兼容模式性能更高，安全模式防泄露和劫持能力更强。', modes: { 'A': { name: '兼容模式', icon: 'fa-globe-americas' }, 'B': { name: '安全模式', icon: 'fa-shield-alt' }}},
            { tag: 'switch1', name: '请求屏蔽', desc: '对无解析结果的请求进行屏蔽', tip: '建议开启，避免无ipv4及ipv6结果的非必要DNS解析。', valueForOn: 'A' },
            { tag: 'switch5', name: '类型屏蔽', desc: '屏蔽 SOA、PTR、HTTPS 等请求', tip: '建议开启，可减少不必要的网络请求，提高效率。', valueForOn: 'A' },
            { tag: 'switch6', name: 'IPV6屏蔽', desc: '屏蔽AAAA请求类型', tip: '无IPV6网络环境建议开启', valueForOn: 'A' },
            { tag: 'switch2', name: '指定 Client', desc: '对特定客户端 IP 应用分流策略', tip: '按需开启。需要 MosDNS 监听53端口，并正确配置 client_ip 名单。', valueForOn: 'A' },
            { tag: 'switch4', name: '过期缓存', desc: '启用 Lazy Cache（乐观缓存）', tip: '建议开启，可以提升重复查询的响应速度，即使缓存已过期。', valueForOn: 'A' },
            { tag: 'switch7', name: '广告屏蔽', desc: '启用Adguard在线规则支持', tip: '此开关开启后，“广告拦截”页签中已启用的在线列表才会生效。', valueForOn: 'A' },
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
            } catch (error) {
                ui.showToast(`切换“${profile.name}”失败`, 'error');
                checkbox.checked = !checkbox.checked;
            } finally {
                checkbox.disabled = false;
            }
        }
    };
    
    const systemInfoManager = {
        parseMetrics(metricsText) {
            const lines = metricsText.split('\n');
            const metrics = { startTime: 0, cpuTime: 0, residentMemory: 0, heapIdleMemory: 0, threads: 0, openFds: 0, goVersion: "N/A" };
            lines.forEach(line => {
                if (line.startsWith('process_start_time_seconds')) { metrics.startTime = parseFloat(line.split(' ')[1]) || 0; } 
                else if (line.startsWith('process_cpu_seconds_total')) { metrics.cpuTime = parseFloat(line.split(' ')[1]) || 0; } 
                else if (line.startsWith('process_resident_memory_bytes')) { metrics.residentMemory = parseFloat(line.split(' ')[1]) || 0; } 
                else if (line.startsWith('go_memstats_heap_idle_bytes')) { metrics.heapIdleMemory = parseFloat(line.split(' ')[1]) || 0; } 
                else if (line.startsWith('go_threads')) { metrics.threads = parseInt(line.split(' ')[1]) || 0; } 
                else if (line.startsWith('process_open_fds')) { metrics.openFds = parseInt(line.split(' ')[1]) || 0; } 
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
            } catch (error) {}
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

    const historyManager = { load: () => { const saved = JSON.parse(localStorage.getItem('mosdnsHistory')); if (saved) { state.history.totalQueries = saved.totalQueries || []; state.history.avgDuration = saved.avgDuration || []; } }, add(total, avg) { state.history.totalQueries.push(total ?? 0); state.history.avgDuration.push(avg ?? 0); if (state.history.totalQueries.length > CONSTANTS.HISTORY_LENGTH) state.history.totalQueries.shift(); this.save(); }, save: () => { localStorage.setItem('mosdnsHistory', JSON.stringify(state.history)); } };
    
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
            this.setTheme(savedTheme, false); 
            this.setColor(savedColor, false); 
            this.setLayout(savedLayout, false); 
            elements.themeSwitcher?.addEventListener('change', e => this.setTheme(e.target.value)); 
            elements.layoutSwitcher?.addEventListener('change', e => this.setLayout(e.target.value)); 
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
            if(save) localStorage.setItem('mosdns-color', color); 
        }, 
        setLayout(layout, save = true) { 
            elements.html.setAttribute('data-layout', layout); 
            if (elements.layoutSwitcher) { elements.layoutSwitcher.value = layout; } 
            if(save) localStorage.setItem('mosdns-layout', layout); 
            adjustLogSearchLayout();
        } 
    };

    const animateValue = (element, start, end, duration, decimals = 0) => { if (!element || start === null || end === null) return; if (start === end) { element.textContent = (decimals > 0 ? parseFloat(end).toFixed(decimals) : Math.floor(end).toLocaleString()); return; } let startTimestamp = null; const step = (timestamp) => { if (!startTimestamp) startTimestamp = timestamp; const progress = Math.min((timestamp - startTimestamp) / duration, 1); const current = start + progress * (end - start); element.textContent = (decimals > 0 ? parseFloat(current).toFixed(decimals) : Math.floor(current).toLocaleString()); if (progress < 1) window.requestAnimationFrame(step); }; window.requestAnimationFrame(step); };
    const updateStatChange = (element, prev, curr, isTime = false) => { if (prev === null || curr === null || prev === 0) { element.style.visibility = 'hidden'; return; } const diff = curr - prev; const change = (diff / prev) * 100; if (Math.abs(change) < 0.1) { element.style.visibility = 'hidden'; return; } const direction = isTime ? (diff < 0 ? 'up' : 'down') : (diff > 0 ? 'up' : 'down'); const icon = direction === 'up' ? '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M12 8L18 14H6L12 8Z"></path></svg>' : '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M12 16L6 10H18L12 16Z"></path></svg>'; element.className = `stat-change ${direction}`; element.innerHTML = `${icon} ${Math.abs(change).toFixed(1)}%`; element.style.visibility = 'visible'; };
    const setupGlowEffect = () => { elements.container?.addEventListener('mousemove', (e) => { const card = e.target.closest('.card:not(dialog)'); if (card) { const rect = card.getBoundingClientRect(); card.style.setProperty('--glow-x', `${e.clientX - rect.left}px`); card.style.setProperty('--glow-y', `${e.clientY - rect.top}px`); } }); };
    const generateSparklineSVG = (data, isFloat = false, width = 300, height = 60) => { if (!data || data.length < 2) return ''; const numericData = data.map(Number); const maxVal = Math.max(...numericData); const minVal = Math.min(...numericData); const range = maxVal - minVal === 0 ? 1 : maxVal - minVal; const points = numericData.map((d, i) => { const x = (i / (data.length - 1)) * width; const y = height - ((d - minVal) / range) * height; return `${x.toFixed(2)},${y.toFixed(2)}`; }); const pathD = `M ${points.join(' L ')}`; const fillPathD = `${pathD} L ${width},${height} L 0,${height} Z`; return `<svg viewBox="0 0 ${width} ${height}" preserveAspectRatio="none"><defs><linearGradient id="sparkline-gradient" x1="0%" y1="0%" x2="0%" y2="100%"><stop offset="0%" stop-color="var(--color-accent-primary)" stop-opacity="0.5" /><stop offset="100%" stop-color="var(--color-accent-primary)" stop-opacity="0" /></linearGradient></defs><path d="${fillPathD}" fill="url(#sparkline-gradient)" /><path d="${pathD}" class="sparkline-path" fill="none" /></svg>`; };
    
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
            const colspan = state.isMobile ? 1 : (tbody.previousElementSibling?.rows[0]?.cells.length || 2); 
            const emptyRow = document.createElement('tr'); 
            emptyRow.className = 'empty-state-row'; 
            emptyRow.innerHTML = `<td colspan="${colspan}"><div class="empty-state-content"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M21.71,3.29C21.32,2.9,20.69,2.9,20.3,3.29L3.29,20.3c-0.39,0.39-0.39,1.02,0,1.41C3.48,21.9,3.74,22,4,22s0.52-0.1,0.71-0.29L21.71,4.7C22.1,4.31,22.1,3.68,21.71,3.29z M12,2C6.48,2,2,6.48,2,12s4.48,10,10,10,10-4.48,10-10S17.52,2,12,2z M12,20c-4.41,0-8-3.59-8-8c0-2.33,1-4.45,2.65-5.92l11.27,11.27C16.45,19,14.33,20,12,20z"></path></svg><strong>暂无数据</strong><p>${message}</p>${ctaButton}</div></td>`; 
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
    
    const chartColors = ['#6d9dff', '#f778ba', '#2dd4bf', '#fb923c', '#a78bfa', '#fde047', '#ff8c8c'];
    const renderDonutChart = (data) => {
        const placeholder = elements.shuntResultsBody.querySelector('.lazy-placeholder');
        if (placeholder) placeholder.style.display = 'none';
        if (!data || data.length === 0) {
            elements.shuntResultsBody.innerHTML = `<div class="empty-state-content" style="padding: 2rem 0;"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M21.71,3.29C21.32,2.9,20.69,2.9,20.3,3.29L3.29,20.3c-0.39,0.39-0.39,1.02,0,1.41C3.48,21.9,3.74,22,4,22s0.52-0.1,0.71-0.29L21.71,4.7C22.1,4.31,22.1,3.68,21.71,3.29z M12,2C6.48,2,2,6.48,2,12s4.48,10,10,10,10-4.48,10-10S17.52,2,12,2z M12,20c-4.41,0-8-3.59-8-8c0-2.33,1-4.45,2.65-5.92l11.27,11.27C16.45,19,14.33,20,12,20z"></path></svg><strong>暂无数据</strong><p>没有检测到分流结果。</p></div>`;
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
            const [statusRes, capacityRes, statsRes, domainSetRankRes] = await Promise.allSettled([api.getStatus(signal), api.getCapacity(signal), api.v2.getStats(signal), api.v2.getDomainSetRank(signal, 100)]);

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

            if (domainSetRankRes.status === 'fulfilled') { 
                state.domainSetRank = domainSetRankRes.value || []; 
                renderDonutChart(state.domainSetRank); 
            }
            
            if (activeTab === 'system-control' || forceAll) {
                await Promise.allSettled([
                    state.requery.pollId ? Promise.resolve() : requeryManager.updateStatus(signal),
                    updateDomainListStats(signal),
                    cacheManager.updateStats(signal),
                    switchManager.loadStatus(signal),
                    systemInfoManager.load(signal),
                ]);
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
                if(activeSubTab === 'list-mgmt' && !state.listManagerInitialized) {
                    listManager.init();
                } else {
                    await Promise.all([adguardManager.load(), diversionManager.load()]);
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
        if (!append) renderSkeletonRows(elements.logTableBody, CONSTANTS.LOGS_PER_PAGE, state.isMobile ? 1 : 5);
        if (!append && logRequestController) logRequestController.abort();
        logRequestController = new AbortController();
        const params = { page, limit: CONSTANTS.LOGS_PER_PAGE, q: state.currentLogSearchTerm.query, exact: state.currentLogSearchTerm.exact };
        try {
            const response = await api.v2.getLogs(logRequestController.signal, params);
            if (!response?.pagination) throw new Error("Invalid response from logs API");
            const { pagination, logs } = response;
            state.logPaginationInfo = pagination;
            state.currentLogPage = pagination.current_page;
            if(!append) ui.updateSearchResultsInfo(pagination);
            ui.renderLogTable(logs || [], append);
        } catch (error) { if (error.name !== 'AbortError') { console.error("Failed to fetch logs:", error); ui.showToast('获取日志失败', 'error'); }
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
        updateHeaders() { document.querySelectorAll('#log-table-head th[data-sortable]').forEach(th => { th.classList.remove('sorted'); const indicator = th.querySelector('.sort-indicator'); if(indicator) { if (th.dataset.sortKey === state.logSort.key) { th.classList.add('sorted'); indicator.textContent = state.logSort.order === 'asc' ? '▲' : '▼'; } else { indicator.textContent = ' '; } } }); }
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
        if(data.is_blocked) statusText += ' (已拦截)';
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
    function getRuleTagHTML(log) { if (!log || !log.domain_set) return ''; const ruleName = log.domain_set; const bgColor = state.shuntColors[ruleName]; if (!bgColor) return ''; const textColor = getContrastingTextColor(bgColor); return `<span class="rule-tag" style="background-color: ${bgColor}; color: ${textColor};" title="分流规则: ${ruleName}">${ruleName}</span>`; }
    function getResponseTagHTML(log) { if (!log) return ''; const code = log.response_code || 'UNKNOWN'; let tagClass = 'other'; if (code === 'NOERROR') tagClass = 'noerror'; else if (code === 'NXDOMAIN') tagClass = 'nxdomain'; else if (code === 'SERVFAIL') tagClass = 'servfail'; else if (code === 'REFUSED') tagClass = 'refused'; return `<span class="response-tag ${tagClass}">${code}</span>`; }
    function getResponseSummary(log) { if (!log) return ''; if (log.response_code !== 'NOERROR') return getResponseTagHTML(log); if (log.answers?.length > 0) { const firstIp = log.answers.find(a => a.type === 'A' || a.type === 'AAAA'); const firstCname = log.answers.find(a => a.type === 'CNAME'); let mainText = firstIp?.data ?? firstCname?.data ?? log.answers[0].data; if (mainText.length > 25) mainText = mainText.substring(0, 22) + '...'; if (log.answers.length > 1) mainText += ` (+${log.answers.length - 1})`; return `<span class="truncate-text">${mainText}</span>`; } return '<span>(empty)</span>'; }
    
    function renderDomainResponseCellHTML(log, source = 'log') {
        const ruleColor = log.domain_set ? state.shuntColors[log.domain_set] : null;
        const domainStyle = (source === 'slowest' && ruleColor) ? `style="color: ${ruleColor}; font-weight: 700;"` : '';
        const domainTitle = (source === 'slowest' && ruleColor) ? `${log.query_name} (规则: ${log.domain_set})` : log.query_name;
        const ruleTag = (source === 'slowest' && ruleColor) ? '' : getRuleTagHTML(log);
        return `<div class="domain-response-cell"><span class="domain-name truncate-text" ${domainStyle} title="${domainTitle}">${log.query_name}</span><div class="response-meta"><span class="response-summary">${getResponseSummary(log)}</span>${ruleTag}</div></div>`;
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
                <td class="text-right">${log.duration_ms.toFixed(2)}</td>
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
                <td class="text-right">${log.duration_ms.toFixed(2)}</td>`;
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
        if (activeTabId === 'log-query' && state.displayedLogs.length === 0) {
            applyLogFilterAndRender();
        } else if (activeTabId === 'rules') {
            const activeSubTab = document.querySelector('#rules-tab .sub-nav-link.active').dataset.subTab;
            if (activeSubTab === 'list-mgmt' && !state.listManagerInitialized) {
                listManager.init();
            } else if ((activeSubTab === 'adguard' && state.adguardRules.length === 0) || (activeSubTab === 'diversion' && state.diversionRules.length === 0)) {
                renderSkeletonRows(elements.adguardRulesTbody, 5, state.isMobile ? 1 : 6);
                renderSkeletonRows(elements.diversionRulesTbody, 5, state.isMobile ? 1 : 7);
                Promise.all([adguardManager.load(), diversionManager.load()]);
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
    }
    
    function renderRuleTable(tbody, rules, mode) {
        tbody.closest('table').classList.toggle('mobile-rule-card-layout', state.isMobile); 
        const sortedRules = [...rules].sort((a, b) => (a.name || '').localeCompare(b.name || '')); 
        renderTable(tbody, sortedRules, (rule, index) => { 
            const item = state.isMobile ? renderRuleMobileCard(rule, mode) : renderRuleTableRow(rule, mode); 
            item.dataset.ruleId = mode === 'adguard' ? rule.id : rule.name; 
            if(rule.type) item.dataset.ruleType = rule.type; 
            return item; 
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

    async function handleAdguardUpdateCheck() { ui.setLoading(elements.checkAdguardUpdatesBtn, true); ui.showToast('已开始在后台更新所有启用的拦截规则...'); try { await api.fetch('/plugins/adguard/update', { method: 'POST' }); ui.showToast('更新请求已发送，5秒后自动刷新列表...', 'success'); await new Promise(resolve => setTimeout(resolve, 5000)); await adguardManager.load(); ui.showToast('拦截规则列表已刷新！', 'success'); } catch (e) {} finally { ui.setLoading(elements.checkAdguardUpdatesBtn, false); } }
    async function handleRuleTableClick(event, mode) { const target = event.target.closest('button, input.rule-enabled-toggle'); if (!target) return; const itemElement = target.closest('[data-rule-id]'); if (!itemElement) return; const id = itemElement.dataset.ruleId; const rules = mode === 'adguard' ? state.adguardRules : state.diversionRules; const rule = rules.find(r => (mode === 'adguard' ? r.id : r.name) === id); if (!rule) return; if (target.matches('.rule-edit-btn')) ui.openRuleModal(mode, rule); else if (target.matches('.rule-delete-btn')) { if (confirm(`确定要删除规则 "${rule.name}" 吗？此操作不可恢复。`)) { ui.setLoading(target, true); try { if (mode === 'adguard') await api.fetch(`/plugins/adguard/rules/${id}`, { method: 'DELETE' }); else await api.fetch(`/plugins/${diversionManager.sdSetInstanceMap[rule.type]}/config/${id}`, { method: 'DELETE' }); ui.showToast(`规则 "${rule.name}" 已删除`); await (mode === 'adguard' ? adguardManager.load() : diversionManager.load()); } catch(e) { console.error(`Failed to delete rule ${id}:`, e); } finally { ui.setLoading(target, false); } } } else if (target.matches('.rule-update-btn')) { ui.setLoading(target, true); ui.showToast(`正在后台更新规则 "${rule.name}"...`); try { await api.fetch(`/plugins/${diversionManager.sdSetInstanceMap[rule.type]}/update/${id}`, { method: 'POST' }); ui.showToast('更新请求已发送, 5秒后自动刷新', 'success'); setTimeout(() => diversionManager.load(), 5000); } catch(e) {} finally { ui.setLoading(target, false); } } else if (target.matches('.rule-enabled-toggle')) { const updatedRule = { ...rule, enabled: target.checked }; target.disabled = true; try { if (mode === 'adguard') await api.fetch(`/plugins/adguard/rules/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(updatedRule) }); else await api.fetch(`/plugins/${diversionManager.sdSetInstanceMap[rule.type]}/config/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(updatedRule) }); rule.enabled = target.checked; ui.showToast(`规则 "${rule.name}" 已${target.checked ? '启用' : '禁用'}`); } catch (error) { target.checked = !target.checked; } finally { target.disabled = false; } } }
    async function handleRuleFormSubmit(event) { event.preventDefault(); ui.setLoading(elements.saveRuleBtn, true); const form = elements.ruleForm; const mode = form.elements['mode'].value; const id = form.elements['id'].value; try { if (mode === 'adguard') { const data = { name: form.elements['name'].value, url: form.elements['url'].value, auto_update: form.elements['auto_update'].checked, update_interval_hours: parseInt(form.elements['update_interval_hours'].value, 10) || 24 }; if (id) { const originalRule = state.adguardRules.find(r => r.id === id); await api.fetch(`/plugins/adguard/rules/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...originalRule, ...data }) }); } else { await api.fetch('/plugins/adguard/rules', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...data, enabled: true }) }); } ui.showToast(`广告拦截规则${id ? '更新' : '添加'}成功`); await adguardManager.load(); } else { const data = { name: form.elements['name'].value, url: form.elements['url'].value, type: form.elements['type'].value, files: form.elements['files'].value, auto_update: form.elements['auto_update'].checked, update_interval_hours: parseInt(form.elements['update_interval_hours'].value, 10) || 24 }; const pluginTag = diversionManager.sdSetInstanceMap[data.type]; if (!pluginTag) throw new Error('无效的分流规则类型'); if (id) { const originalRule = state.diversionRules.find(r => r.name === id); if (data.name !== id) { if (!confirm(`规则名称已从 "${id}" 更改为 "${data.name}"。\n\n这将删除旧规则并创建一个新规则，确定要继续吗？`)) throw new Error('User cancelled name change.'); await api.fetch(`/plugins/${diversionManager.sdSetInstanceMap[originalRule.type]}/config/${id}`, { method: 'DELETE' }); await api.fetch(`/plugins/${pluginTag}/config/${data.name}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...data, enabled: originalRule.enabled }) }); } else { await api.fetch(`/plugins/${pluginTag}/config/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...originalRule, ...data }) }); } } else { await api.fetch(`/plugins/${pluginTag}/config/${data.name}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...data, enabled: true }) }); } ui.showToast(`分流规则${id ? '更新' : '添加'}成功`); await diversionManager.load(); if (!id || (id && data.name !== id)) { ui.showToast('正在后台获取规则详情...'); setTimeout(() => diversionManager.load(), 5000); } } ui.closeRuleModal(); } catch (err) { console.error(`${mode} form submission failed:`, err); } finally { ui.setLoading(elements.saveRuleBtn, false); } }
    const adguardManager = { async load() { try { state.adguardRules = await api.fetch('/plugins/adguard/rules') || []; } catch (error) { state.adguardRules = []; } this.render(); }, render() { renderRuleTable(elements.adguardRulesTbody, state.adguardRules, 'adguard'); }, };
    const diversionManager = { sdSetInstanceMap: { 'geositecn': 'geosite_cn', 'geositenocn': 'geosite_no_cn', 'geoipcn': 'geoip_cn' }, async load() { try { const promises = Object.values(this.sdSetInstanceMap).map(tag => api.fetch(`/plugins/${tag}/config`)); const results = await Promise.allSettled(promises); state.diversionRules = results.filter(r => r.status === 'fulfilled' && Array.isArray(r.value)).flatMap(r => r.value); } catch(e) { state.diversionRules = []; } this.render(); }, render() { renderRuleTable(elements.diversionRulesTbody, state.diversionRules, 'diversion'); }, };
    
    async function updateDomainListStats(signal) {
        const listMap = {
            fakeip: { element: elements.fakeipDomainCount, endpoint: '/plugins/my_fakeiplist/show' },
            realip: { element: elements.realipDomainCount, endpoint: '/plugins/my_realiplist/show' },
            nov4: { element: elements.nov4DomainCount, endpoint: '/plugins/my_nov4list/show' },
            nov6: { element: elements.nov6DomainCount, endpoint: '/plugins/my_nov6list/show' },
        };

        const promises = Object.values(listMap).map(item => api.fetch(item.endpoint, { signal }));
        const backupPromise = requeryApi.getBackupCount(signal);
        
        const results = await Promise.allSettled(promises);

        results.forEach((result, index) => {
            const key = Object.keys(listMap)[index];
            const { element } = listMap[key];
            if (result.status === 'fulfilled' && typeof result.value === 'string') {
                const count = result.value.trim() === '' ? 0 : result.value.trim().split('\n').length;
                element.textContent = count.toLocaleString();
            } else {
                element.textContent = '获取失败';
            }
        });

        try {
            const backupRes = await backupPromise;
            if (backupRes && backupRes.status === 'success') {
                elements.backupDomainCount.textContent = `${backupRes.count.toLocaleString()} 条`;
            } else {
                elements.backupDomainCount.textContent = '获取失败';
            }
        } catch(e) {
            elements.backupDomainCount.textContent = '获取失败';
        }
    }

    function renderDataViewTable(entries, type = 'domain') {
        if (!elements.dataViewTableContainer) return;

        elements.dataViewTableContainer.innerHTML = '';
        
        if (entries.length === 0) {
            elements.dataViewTableContainer.innerHTML = '<div class="empty-state-content" style="padding: 2rem 0;"><p>此列表为空或没有匹配的条目。</p></div>';
        } else if (type === 'cache') {
            const accordionContainer = document.createElement('div');
            entries.forEach(item => {
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
                    
                    if (collapseEl.classList.contains('show')) {
                        collapseEl.style.maxHeight = bodyEl.scrollHeight + 'px';
                    } else {
                        collapseEl.style.maxHeight = null;
                    }
                });
            });
            elements.dataViewTableContainer.appendChild(accordionContainer);
        } else { // domain list
             elements.dataViewTableContainer.innerHTML = `
                <table class="mobile-card-layout">
                    <thead>
                        <tr>
                            <th style="width: 25%;">序号 / ID</th>
                            <th>域名 / 值</th>
                        </tr>
                    </thead>
                    <tbody>
                        ${entries.map(item => `<tr><td>${item.id}</td><td>${item.value}</td></tr>`).join('')}
                    </tbody>
                </table>`;
        }
        
        elements.dataViewModalInfo.textContent = `总计: ${state.dataView.rawEntries.length} | 显示: ${entries.length}`;
    }

    async function openDataViewModal(config) {
        const { listType, cacheTag, title } = config;
        elements.dataViewModalTitle.textContent = title;
        elements.dataViewTableContainer.innerHTML = '<div class="lazy-placeholder"><div class="spinner"></div></div>';
        elements.dataViewModalInfo.textContent = '正在加载...';
        elements.dataViewSearch.value = '';

        lockScroll();
        elements.dataViewModal.showModal();

        try {
            let text = '';
            let viewType = 'domain';

            if (listType) {
                const endpointMap = {
                    fakeip: '/plugins/my_fakeiplist/show',
                    realip: '/plugins/my_realiplist/show',
                    nov4: '/plugins/my_nov4list/show',
                    nov6: '/plugins/my_nov6list/show'
                };
                const endpoint = endpointMap[listType];
                if (!endpoint) throw new Error('Unknown list type');
                text = await api.fetch(endpoint);
            } else if (cacheTag) {
                text = await api.getCacheContents(cacheTag);
                viewType = 'cache';
            }

            if (viewType === 'cache') {
                const entries = text.trim() ? text.trim().split('----- Cache Entry -----').filter(entry => entry.trim() !== '') : [];
                state.dataView.rawEntries = entries.map((entryText, index) => {
                    const questionMatch = entryText.match(/;; QUESTION SECTION:\s*;\s*([^\s]+)/);
                    const domainSetMatch = entryText.match(/DomainSet:\s*(.+)/);
                    let headerTitle = questionMatch ? questionMatch[1].replace(/\.$/, '') : `Cache Entry #${index + 1}`;
                    if (domainSetMatch) headerTitle += ` [${domainSetMatch[1].trim()}]`;
                    return { headerTitle, fullText: entryText, index };
                });
            } else {
                const lines = text.trim() ? text.trim().split('\n') : [];
                state.dataView.rawEntries = lines.map((line, index) => {
                    const parts = line.trim().match(/^(\S+)\s+(.*)$/);
                    if (parts) return { id: parts[1], value: parts[2] };
                    return { id: index + 1, value: line };
                });
            }
            
            state.dataView.viewType = viewType;
            state.dataView.filteredEntries = state.dataView.rawEntries;
            renderDataViewTable(state.dataView.filteredEntries, viewType);

        } catch (error) {
            elements.dataViewTableContainer.innerHTML = `<div class="empty-state-content" style="padding: 2rem 0;"><p style="color: var(--color-danger);">加载列表失败</p><small>${error.message}</small></div>`;
            elements.dataViewModalInfo.textContent = `加载失败`;
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
            { key: 'cache_google_node', name: '国外缓存 (安全)', tag: 'cache_google_node' }
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
                    this.renderTable(true); // Render with error state
                }
            }
        },

        renderTable(isError = false) {
            const tbody = elements.cacheStatsTbody;
            if (!tbody) return;
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
                    tr.innerHTML = `
                        <td>
                            <div class="cache-card-mobile">
                                <div class="cache-card-header">
                                    <strong class="cache-name">${cache.name}</strong>
                                    <button class="button danger clear-cache-btn" data-cache-tag="${cache.tag}" style="padding: 0.4rem 0.8rem;"><span>清空</span></button>
                                </div>
                                <div class="cache-card-body">
                                    <div><span>请求总数</span><strong>${stats.query_total.toLocaleString()}</strong></div>
                                    <div><span>缓存命中</span><strong>${stats.hit_total.toLocaleString()}</strong></div>
                                    <div><span>过期命中</span><strong>${stats.lazy_hit_total.toLocaleString()}</strong></div>
                                    <div><span>命中率</span><strong>${hitRate}</strong></div>
                                    <div><span>过期命中率</span><strong>${lazyRate}</strong></div>
                                    <div><span>条目数</span><a href="#" class="control-item-link" data-cache-tag="${cache.tag}" data-cache-title="${cache.name}"><strong style="color:var(--color-accent-primary);">${stats.size_current.toLocaleString()}</strong></a></div>
                                </div>
                            </div>
                        </td>`;
                } else {
                    tr.innerHTML = `
                        <td>${cache.name}</td>
                        <td class="text-right">${stats.query_total.toLocaleString()}</td>
                        <td class="text-right">${stats.hit_total.toLocaleString()}</td>
                        <td class="text-right">${stats.lazy_hit_total.toLocaleString()}</td>
                        <td class="text-right">${hitRate}</td>
                        <td class="text-right">${lazyRate}</td>
                        <td class="text-right"><a href="#" class="control-item-link" data-cache-tag="${cache.tag}" data-cache-title="${cache.name}">${stats.size_current.toLocaleString()}</a></td>
                        <td class="text-center"><button class="button danger clear-cache-btn" data-cache-tag="${cache.tag}" style="padding: 0.4rem 0.8rem;"><span>清空</span></button></td>
                    `;
                }
                tbody.appendChild(tr);
            });
        }
    };
    
    const listManager = {
        MAX_LINES: 500,
        currentTag: null,
        profiles: [
            { tag: 'whitelist', name: '白名单' },
            { tag: 'blocklist', name: '黑名单' },
            { tag: 'greylist', name: '灰名单' },
            { tag: 'ddnslist', name: 'DDNS 域名' },
            { tag: 'client_ip', name: '客户端 IP' },
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
            this.loadList('whitelist'); // Load first list by default
            state.listManagerInitialized = true;
        },
        
        async loadList(tag) {
            this.currentTag = tag;
            elements.listMgmtNav.querySelectorAll('.list-mgmt-link').forEach(l => l.classList.toggle('active', l.dataset.listTag === tag));
            
            elements.listMgmtClientIpHint.style.display = (tag === 'client_ip') ? 'block' : 'none';
            if (elements.listMgmtRewriteHint) {
                elements.listMgmtRewriteHint.style.display = (tag === 'rewrite') ? 'block' : 'none';
            }

            elements.listContentLoader.style.display = 'flex';
            elements.listContentTextArea.style.display = 'none';
            elements.listContentInfo.textContent = '正在加载...';
            ui.setLoading(elements.listSaveBtn, true);

            try {
                const text = await api.fetch(`/plugins/${tag}/show`);
                const lines = text.trim() === '' ? [] : text.trim().split('\n');
                
                if (lines.length > this.MAX_LINES) {
                    elements.listContentTextArea.value = lines.slice(0, this.MAX_LINES).join('\n');
                    elements.listContentInfo.textContent = `内容过长，仅显示前 ${this.MAX_LINES} 行（共 ${lines.length} 行）。`;
                } else {
                    elements.listContentTextArea.value = text.trim();
                    elements.listContentInfo.textContent = `共 ${lines.length} 行。`;
                }

            } catch (error) {
                elements.listContentTextArea.value = `加载列表“${tag}”失败。`;
                elements.listContentInfo.textContent = '加载失败';
                ui.showToast(`加载列表“${tag}”失败`, 'error');
            } finally {
                elements.listContentLoader.style.display = 'none';
                elements.listContentTextArea.style.display = 'block';
                ui.setLoading(elements.listSaveBtn, false);
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


    function setupEventListeners() {
        // -- [修改] -- 统一处理所有弹窗的关闭行为（遮罩层点击和ESC键）
        document.querySelectorAll('dialog').forEach(dialog => {
            // 点击遮罩层时关闭
            dialog.addEventListener('click', (event) => {
                if (event.target === dialog) {
                    closeAndUnlock(dialog);
                }
            });
            // 按 ESC 键时关闭
            dialog.addEventListener('cancel', (event) => {
                event.preventDefault(); // 阻止默认的关闭行为
                closeAndUnlock(dialog);
            });
        });

        elements.tabLinks.forEach(link => link.addEventListener('click', (e) => { e.preventDefault(); handleNavigation(link); }));
        window.addEventListener('popstate', () => { const hash = window.location.hash || '#overview'; const targetLink = document.querySelector(`.tab-link[href="${hash}"]`); handleNavigation(targetLink || elements.tabLinks[0]); });
        window.addEventListener('resize', debounce(handleResize, 150));
        elements.globalRefreshBtn?.addEventListener('click', () => updatePageData(true));
        setInterval(updateLastUpdated, 5000);
        elements.autoRefreshForm.addEventListener('change', (e) => { if(['checkbox', 'number'].includes(e.target.type)) { const enabled = elements.autoRefreshToggle.checked; const interval = parseInt(elements.autoRefreshIntervalInput.value, 10); elements.autoRefreshIntervalInput.disabled = !enabled; autoRefreshManager.updateSettings(enabled, interval); } });
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
            const logRow = e.target.closest('[data-log-index], [data-rank-index]');

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
                    if(link) handleNavigation(link);
                }
            } else if (logRow) {
                ui.openLogDetailModal(logRow);
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
                } catch (err) {} 
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

        elements.closeDataViewModalBtn?.addEventListener('click', () => closeAndUnlock(elements.dataViewModal));
        elements.dataViewSearch?.addEventListener('input', debounce(() => {
            const searchTerm = elements.dataViewSearch.value.toLowerCase();
            if (state.dataView.viewType === 'cache') {
                state.dataView.filteredEntries = state.dataView.rawEntries.filter(item => 
                    item.headerTitle.toLowerCase().includes(searchTerm) ||
                    item.fullText.toLowerCase().includes(searchTerm)
                );
            } else {
                state.dataView.filteredEntries = state.dataView.rawEntries.filter(item => 
                    item.value.toLowerCase().includes(searchTerm)
                );
            }
            renderDataViewTable(state.dataView.filteredEntries, state.dataView.viewType);
        }, 250));

        elements.saveShuntRulesBtn?.addEventListener('click', saveAllShuntRules);
        elements.clearShuntRulesBtn?.addEventListener('click', clearAllShuntRules);
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

    async function init() {
        state.isTouchDevice = ('ontouchstart' in window) || (navigator.maxTouchPoints > 0);
        themeManager.init();
        await aliasManager.load(); 
        historyManager.load();
        autoRefreshManager.loadSettings();
        tableSorter.init();
        switchManager.init();
        mountGlobalInfoIconDelegation();
        setupEventListeners();
        setupGlowEffect();
        setupLazyLoading();
        handleResize();
        const initialHash = window.location.hash || '#overview';
        const initialLink = document.querySelector(`.tab-link[href="${initialHash}"]`);
        if (initialLink) handleNavigation(initialLink);
        await updatePageData(true);
        if (document.fonts?.ready) await document.fonts.ready;
        requestAnimationFrame(() => { const activeLink = document.querySelector('.tab-link.active'); if(activeLink) updateNavSlider(activeLink); });
        elements.initialLoader.style.opacity = '0';
        elements.initialLoader.addEventListener('transitionend', () => elements.initialLoader.remove());
        if (!document.hidden) autoRefreshManager.start();
        requeryManager.init();
    }

    init();
});
