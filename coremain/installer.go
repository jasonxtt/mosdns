package coremain

import (
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/go-chi/chi/v5"
)

// InstallStatus 安装状态
type InstallStatus struct {
	Installed        bool          `json:"installed"`
	WorkDir          string        `json:"workDir"`
	BinaryPath       string        `json:"binaryPath"`
	PortConflicts    PortConflicts `json:"portConflicts"`
	HasRootPermission bool        `json:"hasRootPermission"`
}

type PortConflicts struct {
	ListenPort bool `json:"listenPort"`
	AdminPort  bool `json:"adminPort"`
}

// InstallRequest 安装请求
type InstallRequest struct {
	WorkDir       string `json:"workDir"`
	ListenPort    int    `json:"listenPort"`
	AdminPort     int    `json:"adminPort"`
	UpstreamDNS   string `json:"upstreamDNS"`
	EnableCache   bool   `json:"enableCache"`
	EnableAdBlock bool   `json:"enableAdBlock"`
	EnableShunt   bool   `json:"enableShunt"`
}

// InstallStep 安装步骤
type InstallStep struct {
	Name    string `json:"name"`
	Status  string `json:"status"` // success, failed, running
	Message string `json:"message"`
}

// InstallProgress 安装进度
type InstallProgress struct {
	Success bool         `json:"success"`
	Steps   []InstallStep `json:"steps"`
	WebUIURL string      `json:"webuiUrl"`
	Message  string      `json:"message"`
}

// StartInstallWizard 启动安装向导
func StartInstallWizard(port int) {
	r := chi.NewRouter()

	// 静态文件
	r.Handle("/assets/*", http.FileServer(http.FS(installFS)))

	// API 端点
	r.Get("/api/v1/install/status", handleInstallStatus)
	r.Post("/api/v1/install/submit", handleInstallSubmit)
	r.Post("/api/v1/install/apply", handleInstallApply)

	// 主页面
	r.Get("/", handleInstallIndex)

	addr := fmt.Sprintf("0.0.0.0:%d", port)
	fmt.Printf("\n")
	fmt.Printf("╔════════════════════════════════════════════════════════╗\n")
	fmt.Printf("║     MosDNS-Lite 安装向导                                ║\n")
	fmt.Printf("╠════════════════════════════════════════════════════════╣\n")
	fmt.Printf("║  请访问：http://<服务器 IP>:%d                      ║\n", port)
	fmt.Printf("║                                                         ║\n")
	fmt.Printf("║  按 Ctrl+C 退出安装向导                                 ║\n")
	fmt.Printf("╚════════════════════════════════════════════════════════╝\n")
	fmt.Printf("\n")

	if err := http.ListenAndServe(addr, r); err != nil {
		fmt.Printf("安装向导启动失败：%v\n", err)
		os.Exit(1)
	}
}

func handleInstallIndex(w http.ResponseWriter, r *http.Request) {
	data, err := installFS.ReadFile("www/install/index.html")
	if err != nil {
		http.Error(w, "Failed to load page", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Write(data)
}

func handleInstallStatus(w http.ResponseWriter, r *http.Request) {
	status := InstallStatus{
		WorkDir:    "/cus/mosdns",
		BinaryPath: "/usr/local/bin/mosdns-lite",
	}

	// 检查是否已安装
	if _, err := os.Stat("/etc/systemd/system/mosdns.service"); err == nil {
		status.Installed = true
	}

	// 检查 root 权限
	status.HasRootPermission = (os.Geteuid() == 0)

	// 检查端口占用
	status.PortConflicts.ListenPort = isPortInUse(53)
	status.PortConflicts.AdminPort = isPortInUse(9099)

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(status)
}

func handleInstallSubmit(w http.ResponseWriter, r *http.Request) {
	var req InstallRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "Invalid request", http.StatusBadRequest)
		return
	}

	// 基本验证
	if req.WorkDir == "" {
		req.WorkDir = "/cus/mosdns"
	}
	if req.ListenPort == 0 {
		req.ListenPort = 53
	}
	if req.AdminPort == 0 {
		req.AdminPort = 9099
	}
	if req.UpstreamDNS == "" {
		req.UpstreamDNS = "223.5.5.5"
	}

	// 检查端口占用
	if isPortInUse(req.ListenPort) {
		http.Error(w, fmt.Sprintf("端口 %d 被占用", req.ListenPort), http.StatusBadRequest)
		return
	}
	if isPortInUse(req.AdminPort) {
		http.Error(w, fmt.Sprintf("端口 %d 被占用", req.AdminPort), http.StatusBadRequest)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]bool{"success": true})
}

func handleInstallApply(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")

	var req InstallRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		json.NewEncoder(w).Encode(InstallProgress{
			Success: false,
			Message: "无效的请求参数",
		})
		return
	}

	progress := InstallProgress{
		Success: true,
		Steps:   []InstallStep{},
	}

	// 步骤 1：检查 root 权限
	progress.Steps = append(progress.Steps, InstallStep{
		Name:    "check_permission",
		Status:  "running",
		Message: "检查权限...",
	})
	json.NewEncoder(w).Encode(progress)

	if os.Geteuid() != 0 {
		progress.Success = false
		progress.Steps[len(progress.Steps)-1] = InstallStep{
			Name:    "check_permission",
			Status:  "failed",
			Message: "需要 root 权限，请使用 sudo 运行",
		}
		json.NewEncoder(w).Encode(progress)
		return
	}
	progress.Steps[len(progress.Steps)-1] = InstallStep{
		Name:    "check_permission",
		Status:  "success",
		Message: "root 权限检查通过",
	}

	// 步骤 2：创建目录
	progress.Steps = append(progress.Steps, InstallStep{
		Name:    "create_dir",
		Status:  "running",
		Message: "创建目录...",
	})
	json.NewEncoder(w).Encode(progress)

	if err := os.MkdirAll(req.WorkDir, 0755); err != nil {
		progress.Success = false
		progress.Steps[len(progress.Steps)-1] = InstallStep{
			Name:    "create_dir",
			Status:  "failed",
			Message: fmt.Sprintf("创建目录失败：%v", err),
		}
		json.NewEncoder(w).Encode(progress)
		return
	}
	os.MkdirAll(filepath.Join(req.WorkDir, "rules"), 0755)
	os.MkdirAll(filepath.Join(req.WorkDir, "lists"), 0755)
	os.MkdirAll(filepath.Join(req.WorkDir, "backup"), 0755)
	progress.Steps[len(progress.Steps)-1] = InstallStep{
		Name:    "create_dir",
		Status:  "success",
		Message: fmt.Sprintf("目录已创建：%s", req.WorkDir),
	}

	// 步骤 3：生成配置文件
	progress.Steps = append(progress.Steps, InstallStep{
		Name:    "generate_config",
		Status:  "running",
		Message: "生成配置文件...",
	})
	json.NewEncoder(w).Encode(progress)

	configContent, err := generateConfigTemplate(req)
	if err != nil {
		progress.Success = false
		progress.Steps[len(progress.Steps)-1] = InstallStep{
			Name:    "generate_config",
			Status:  "failed",
			Message: fmt.Sprintf("生成配置失败：%v", err),
		}
		json.NewEncoder(w).Encode(progress)
		return
	}

	configPath := filepath.Join(req.WorkDir, "config_custom.yaml")
	if err := os.WriteFile(configPath, configContent, 0644); err != nil {
		progress.Success = false
		progress.Steps[len(progress.Steps)-1] = InstallStep{
			Name:    "generate_config",
			Status:  "failed",
			Message: fmt.Sprintf("写入配置失败：%v", err),
		}
		json.NewEncoder(w).Encode(progress)
		return
	}
	progress.Steps[len(progress.Steps)-1] = InstallStep{
		Name:    "generate_config",
		Status:  "success",
		Message: "配置文件已生成",
	}

	// 步骤 4：复制规则文件
	progress.Steps = append(progress.Steps, InstallStep{
		Name:    "copy_rules",
		Status:  "running",
		Message: "复制规则文件...",
	})
	json.NewEncoder(w).Encode(progress)

	if err := copyEmbeddedFiles(); err != nil {
		progress.Success = false
		progress.Steps[len(progress.Steps)-1] = InstallStep{
			Name:    "copy_rules",
			Status:  "failed",
			Message: fmt.Sprintf("复制规则失败：%v", err),
		}
		json.NewEncoder(w).Encode(progress)
		return
	}
	progress.Steps[len(progress.Steps)-1] = InstallStep{
		Name:    "copy_rules",
		Status:  "success",
		Message: "规则文件已复制",
	}

	// 步骤 5：复制二进制
	progress.Steps = append(progress.Steps, InstallStep{
		Name:    "copy_binary",
		Status:  "running",
		Message: "复制二进制文件...",
	})
	json.NewEncoder(w).Encode(progress)

	exePath, err := os.Executable()
	if err != nil {
		progress.Success = false
		progress.Steps[len(progress.Steps)-1] = InstallStep{
			Name:    "copy_binary",
			Status:  "failed",
			Message: fmt.Sprintf("获取二进制路径失败：%v", err),
		}
		json.NewEncoder(w).Encode(progress)
		return
	}

	targetPath := "/usr/local/bin/mosdns-lite"
	if err := copyFile(exePath, targetPath, 0755); err != nil {
		progress.Success = false
		progress.Steps[len(progress.Steps)-1] = InstallStep{
			Name:    "copy_binary",
			Status:  "failed",
			Message: fmt.Sprintf("复制二进制失败：%v", err),
		}
		json.NewEncoder(w).Encode(progress)
		return
	}
	progress.Steps[len(progress.Steps)-1] = InstallStep{
		Name:    "copy_binary",
		Status:  "success",
		Message: fmt.Sprintf("二进制已复制到：%s", targetPath),
	}

	// 步骤 6：生成 systemd 服务
	progress.Steps = append(progress.Steps, InstallStep{
		Name:    "install_systemd",
		Status:  "running",
		Message: "注册 systemd 服务...",
	})
	json.NewEncoder(w).Encode(progress)

	systemdContent := generateSystemdService(req.WorkDir)
	systemdPath := "/etc/systemd/system/mosdns.service"
	if err := os.WriteFile(systemdPath, systemdContent, 0644); err != nil {
		progress.Success = false
		progress.Steps[len(progress.Steps)-1] = InstallStep{
			Name:    "install_systemd",
			Status:  "failed",
			Message: fmt.Sprintf("生成 systemd 配置失败：%v", err),
		}
		json.NewEncoder(w).Encode(progress)
		return
	}
	progress.Steps[len(progress.Steps)-1] = InstallStep{
		Name:    "install_systemd",
		Status:  "success",
		Message: "systemd 服务已注册",
	}

	// 步骤 7：启动服务
	progress.Steps = append(progress.Steps, InstallStep{
		Name:    "start_service",
		Status:  "running",
		Message: "启动服务...",
	})
	json.NewEncoder(w).Encode(progress)

	// 重载 systemd
	exec.Command("systemctl", "daemon-reload").Run()
	time.Sleep(500 * time.Millisecond)

	// 启用服务
	exec.Command("systemctl", "enable", "mosdns").Run()
	time.Sleep(500 * time.Millisecond)

	// 启动服务
	if err := exec.Command("systemctl", "start", "mosdns").Run(); err != nil {
		progress.Success = false
		progress.Steps[len(progress.Steps)-1] = InstallStep{
			Name:    "start_service",
			Status:  "failed",
			Message: fmt.Sprintf("启动服务失败：%v", err),
		}
		json.NewEncoder(w).Encode(progress)
		return
	}
	progress.Steps[len(progress.Steps)-1] = InstallStep{
		Name:    "start_service",
		Status:  "success",
		Message: "服务已启动",
	}

	// 获取服务器 IP
	serverIP := getServerIP()
	progress.WebUIURL = fmt.Sprintf("http://%s:%d/log", serverIP, req.AdminPort)
	progress.Message = "安装完成！"

	json.NewEncoder(w).Encode(progress)

	// 延迟退出安装向导
	go func() {
		time.Sleep(3 * time.Second)
		fmt.Println("\n安装完成，安装向导即将退出...")
		os.Exit(0)
	}()
}

// 辅助函数

func isPortInUse(port int) bool {
	addr := fmt.Sprintf(":%d", port)
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		return true
	}
	ln.Close()
	return false
}

func generateConfigTemplate(req InstallRequest) ([]byte, error) {
	// 第一阶段使用最简配置模板
	// 后续再从 embedded 读取完整模板
	tmpl := fmt.Sprintf(`log:
  level: info

plugins:
  - tag: udp_server
    type: udp_server
    args:
      entry: sequence
      listen: "0.0.0.0:%d"

  - tag: tcp_server
    type: tcp_server
    args:
      entry: sequence
      listen: "0.0.0.0:%d"

  - tag: upstream_cn
    type: forward
    args:
      upstreams:
        - addr: %s
          bootstrap: 223.5.5.5
          dial_addr: 223.5.5.5

  - tag: cache
    type: cache
    args:
      size: 8000
      ttl: 86400

  - tag: sequence
    type: sequence
    args:
      exec:
        - cache
        - upstream_cn
`, req.ListenPort, req.AdminPort, req.UpstreamDNS)

	return []byte(tmpl), nil
}

func copyEmbeddedFiles() error {
	// 复制规则文件
	rules := []string{"adguard.yaml", "shunt.yaml"}
	for _, rule := range rules {
		data, err := rulesTemplate.ReadFile("www/install/templates/rules/" + rule)
		if err != nil {
			continue // 文件不存在则跳过
		}
		os.WriteFile("/cus/mosdns/rules/"+rule, data, 0644)
	}

	// 复制名单文件
	lists := []string{"whitelist.txt", "blacklist.txt", "chn_domain.txt"}
	for _, list := range lists {
		data, err := listsTemplate.ReadFile("www/install/templates/lists/" + list)
		if err != nil {
			continue
		}
		os.WriteFile("/cus/mosdns/lists/"+list, data, 0644)
	}

	return nil
}



func generateSystemdService(workDir string) []byte {
	return []byte(fmt.Sprintf(`[Unit]
Description=MosDNS-Lite DNS Server
ConditionFileIsExecutable=/usr/local/bin/mosdns-lite
After=network.target

[Service]
StartLimitInterval=5
StartLimitBurst=10
ExecStart=/usr/local/bin/mosdns-lite start --as-service -d %s -c %s/config_custom.yaml
Restart=always
RestartSec=120
EnvironmentFile=-/etc/sysconfig/mosdns

[Install]
WantedBy=multi-user.target
`, workDir, workDir))
}

func getServerIP() string {
	// 获取服务器 IP 地址
	addrs, err := net.InterfaceAddrs()
	if err != nil {
		return "127.0.0.1"
	}

	for _, addr := range addrs {
		if ipnet, ok := addr.(*net.IPNet); ok && !ipnet.IP.IsLoopback() {
			if ipnet.IP.To4() != nil {
				return ipnet.IP.String()
			}
		}
	}

	return "127.0.0.1"
}
