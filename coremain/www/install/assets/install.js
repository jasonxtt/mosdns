// MosDNS-Lite 安装向导 - 前端交互逻辑

let currentStep = 1;
const totalSteps = 5;

let installData = {
    workDir: '/cus/mosdns',
    listenPort: 53,
    adminPort: 9099,
    upstreamDNS: '223.5.5.5',
    enableCache: true,
    enableAdBlock: true,
    enableShunt: true
};

// 初始化
document.addEventListener('DOMContentLoaded', function() {
    // 绑定导航按钮
    document.getElementById('btn-next').addEventListener('click', handleNext);
    document.getElementById('btn-prev').addEventListener('click', handlePrev);

    // 绑定上游 DNS 预设按钮
    document.querySelectorAll('.preset-btn').forEach(btn => {
        btn.addEventListener('click', handlePresetClick);
    });

    // 绑定复选框
    document.querySelectorAll('.checkbox-item').forEach(item => {
        item.addEventListener('click', handleCheckboxClick);
    });

    // 加载系统检查
    checkSystemStatus();
});

// 系统状态检查
async function checkSystemStatus() {
    try {
        const response = await fetch('/api/v1/install/status');
        const status = await response.json();

        // 检查 root 权限
        updateCheckStatus('check-root', status.hasRootPermission);

        // 检查端口 53
        updateCheckStatus('check-port-53', !status.portConflicts.listenPort);

        // 检查端口 9099
        updateCheckStatus('check-port-9099', !status.portConflicts.adminPort);

        // 如果已安装，提示用户
        if (status.installed) {
            alert('检测到 MosDNS 已安装，继续安装将覆盖现有配置。');
        }
    } catch (error) {
        console.error('系统检查失败:', error);
        updateCheckStatus('check-root', false, '检查失败');
    }
}

function updateCheckStatus(checkId, success, customText) {
    const el = document.getElementById(checkId);
    const icon = el.querySelector('.icon');
    const text = el.querySelector('span:last-child');

    if (success) {
        el.className = 'status-check success';
        icon.textContent = '✓';
    } else {
        el.className = 'status-check error';
        icon.textContent = '✗';
    }

    if (customText) {
        text.textContent = customText;
    }
}

// 下一步
function handleNext() {
    // 收集当前步骤数据
    collectStepData(currentStep);

    if (currentStep === 1) {
        // 步骤 1 检查是否通过
        const allChecks = document.querySelectorAll('.status-check.success');
        if (allChecks.length < 3) {
            alert('系统检查未通过，无法继续安装。');
            return;
        }
    }

    if (currentStep < totalSteps) {
        currentStep++;
        updateWizardUI();

        if (currentStep === 5) {
            // 开始安装
            startInstall();
        }
    }
}

// 上一步
function handlePrev() {
    if (currentStep > 1) {
        currentStep--;
        updateWizardUI();
    }
}

// 更新向导 UI
function updateWizardUI() {
    // 更新进度条
    document.querySelectorAll('.progress-step').forEach((step, index) => {
        const stepNum = index + 1;
        step.classList.remove('active', 'completed');
        if (stepNum === currentStep) {
            step.classList.add('active');
        } else if (stepNum < currentStep) {
            step.classList.add('completed');
        }
    });

    // 更新步骤内容
    document.querySelectorAll('.step-content').forEach(content => {
        content.classList.remove('active');
    });
    const activeContent = document.querySelector(`.step-content[data-step="${currentStep}"]`);
    if (activeContent) {
        activeContent.classList.add('active');
    }

    // 更新按钮状态
    document.getElementById('btn-prev').disabled = (currentStep === 1);
    
    const nextBtn = document.getElementById('btn-next');
    if (currentStep === totalSteps) {
        nextBtn.style.display = 'none';
    } else {
        nextBtn.style.display = 'block';
        nextBtn.disabled = false;
    }

    // 隐藏页脚（安装中和完成时）
    const footer = document.getElementById('wizard-footer');
    if (currentStep === 5 || document.getElementById('step-complete')) {
        footer.classList.add('hidden');
    } else {
        footer.classList.remove('hidden');
    }
}

// 收集步骤数据
function collectStepData(step) {
    switch(step) {
        case 2:
            installData.workDir = document.getElementById('workDir').value || '/cus/mosdns';
            break;
        case 3:
            installData.listenPort = parseInt(document.getElementById('listenPort').value) || 53;
            installData.adminPort = parseInt(document.getElementById('adminPort').value) || 9099;
            installData.upstreamDNS = document.getElementById('upstreamDNS').value || '223.5.5.5';
            break;
        case 4:
            installData.enableCache = document.getElementById('enableCache').checked;
            installData.enableAdBlock = document.getElementById('enableAdBlock').checked;
            installData.enableShunt = document.getElementById('enableShunt').checked;
            break;
    }
}

// 处理预设按钮点击
function handlePresetClick(e) {
    const btn = e.target;
    const dns = btn.dataset.dns;

    document.querySelectorAll('.preset-btn').forEach(b => b.classList.remove('selected'));
    btn.classList.add('selected');

    const input = document.getElementById('upstreamDNS');
    if (dns === 'custom') {
        input.value = '';
        input.focus();
    } else {
        input.value = dns;
    }
}

// 处理复选框点击
function handleCheckboxClick(e) {
    if (e.target.tagName !== 'INPUT') {
        const checkbox = e.querySelector('input[type="checkbox"]');
        if (checkbox) {
            checkbox.checked = !checkbox.checked;
        }
    }
    e.classList.toggle('selected');
}

// 开始安装
async function startInstall() {
    updateWizardUI();

    try {
        const response = await fetch('/api/v1/install/apply', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(installData)
        });

        const progress = await response.json();

        if (progress.success) {
            // 更新安装进度
            progress.steps.forEach(step => {
                updateInstallStep(step.name, step.status, step.message);
            });

            // 显示完成页面
            setTimeout(() => {
                showCompletePage(progress.webuiUrl);
            }, 2000);
        } else {
            alert('安装失败：' + progress.message);
        }
    } catch (error) {
        console.error('安装失败:', error);
        alert('安装过程中发生错误，请重试。');
    }
}

// 更新安装步骤状态
function updateInstallStep(stepName, status, message) {
    const el = document.getElementById('step-' + stepName);
    if (!el) return;

    const icon = el.querySelector('.icon');

    if (status === 'running') {
        el.className = 'progress-item running';
        icon.innerHTML = '<span class="spinner"></span>';
    } else if (status === 'success') {
        el.className = 'progress-item success';
        icon.textContent = '✓';
    } else if (status === 'failed') {
        el.className = 'progress-item error';
        icon.textContent = '✗';
    }

    el.querySelector('span:last-child').textContent = message;
}

// 显示完成页面
function showCompletePage(webuiUrl) {
    // 隐藏所有步骤内容
    document.querySelectorAll('.step-content').forEach(content => {
        content.classList.remove('active');
    });

    // 显示完成页面
    const completePage = document.getElementById('step-complete');
    if (completePage) {
        completePage.classList.add('active');
    }

    // 设置 WebUI 链接
    const link = document.getElementById('webui-link');
    if (link && webuiUrl) {
        link.href = webuiUrl;
    }

    // 更新标题
    document.querySelector('.wizard-header h1').textContent = '🎉 安装完成';
    document.querySelector('.wizard-header p').textContent = '感谢使用 MosDNS-Lite';

    // 隐藏进度条
    document.getElementById('wizard-progress').classList.add('hidden');
}
