package coremain

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"time"

	"github.com/spf13/cobra"
)

const (
	updateTransactionFormat = 1
	updateHealthTimeout     = 90 * time.Second
)

type updateTransaction struct {
	Format               int       `json:"format"`
	Status               string    `json:"status"`
	TargetVersion        string    `json:"target_version"`
	TargetSignature      string    `json:"target_signature"`
	RequiredConfigSchema int       `json:"required_config_schema"`
	ConfigPackageID      string    `json:"config_package_id"`
	ConfigPackagePath    string    `json:"config_package_path,omitempty"`
	ConfigBaseDir        string    `json:"config_base_dir"`
	ConfigPath           string    `json:"config_path"`
	ExecutablePath       string    `json:"executable_path"`
	CandidatePath        string    `json:"candidate_path"`
	PreviousBinaryPath   string    `json:"previous_binary_path"`
	HealthURL            string    `json:"health_url"`
	OriginalArgs         []string  `json:"original_args"`
	CreatedAt            time.Time `json:"created_at"`
	LastError            string    `json:"last_error,omitempty"`
}

func init() {
	var transactionPath string
	cmd := &cobra.Command{
		Use:    "update-guard",
		Hidden: true,
		Args:   cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runUpdateGuard(transactionPath)
		},
		DisableFlagsInUseLine: true,
		SilenceUsage:          true,
	}
	cmd.Flags().StringVar(&transactionPath, "transaction", "", "update transaction path")
	_ = cmd.MarkFlagRequired("transaction")
	rootCmd.AddCommand(cmd)
}

func stageUpdateTransaction(status UpdateStatus, manifest releaseUpdateManifest, candidatePath, configPackagePath string) (string, error) {
	if runtime.GOOS == "windows" {
		return "", errors.New("事务更新守护暂不支持 Windows")
	}
	baseDir := configBaseDirOrDot(MainConfigBaseDir)
	updateRoot := filepath.Join(baseDir, "update")
	if err := os.MkdirAll(updateRoot, 0o700); err != nil {
		return "", err
	}
	txnDir, err := os.MkdirTemp(updateRoot, "txn-")
	if err != nil {
		return "", err
	}
	cleanup := true
	defer func() {
		if cleanup {
			_ = os.RemoveAll(txnDir)
		}
	}()

	exePath, err := os.Executable()
	if err != nil {
		return "", err
	}
	oldPath := filepath.Join(txnDir, "mosdns.previous")
	if err := copyFile(exePath, oldPath, 0o755); err != nil {
		return "", fmt.Errorf("备份当前二进制失败: %w", err)
	}
	stagedCandidate := filepath.Join(txnDir, "mosdns.candidate")
	if err := copyFile(candidatePath, stagedCandidate, 0o755); err != nil {
		return "", fmt.Errorf("暂存候选二进制失败: %w", err)
	}
	stagedConfig := ""
	if configPackagePath != "" {
		stagedConfig = filepath.Join(txnDir, "config_up.zip")
		if err := copyFile(configPackagePath, stagedConfig, 0o600); err != nil {
			return "", fmt.Errorf("暂存配置包失败: %w", err)
		}
	}
	healthURL := strings.TrimSuffix(resolveLocalRestartEndpoint(), "/api/v1/system/restart") + "/api/v1/system/health"
	if !strings.HasPrefix(healthURL, "http") {
		healthURL = "http://127.0.0.1:9099/api/v1/system/health"
	}
	tx := updateTransaction{
		Format:               updateTransactionFormat,
		Status:               "staged",
		TargetVersion:        status.LatestVersion,
		TargetSignature:      status.AssetSignature,
		RequiredConfigSchema: manifest.RequiredConfigSchema,
		ConfigPackageID:      manifest.ConfigPackageID,
		ConfigPackagePath:    stagedConfig,
		ConfigBaseDir:        baseDir,
		ConfigPath:           currentConfigPath(),
		ExecutablePath:       exePath,
		CandidatePath:        stagedCandidate,
		PreviousBinaryPath:   oldPath,
		HealthURL:            healthURL,
		OriginalArgs:         append([]string(nil), os.Args...),
		CreatedAt:            time.Now(),
	}
	path := filepath.Join(txnDir, "transaction.json")
	if err := writeJSONFileAtomic(path, tx, 0o600); err != nil {
		return "", err
	}
	cleanup = false
	return path, nil
}

func currentConfigPath() string {
	for i, arg := range os.Args {
		if (arg == "-c" || arg == "--config") && i+1 < len(os.Args) {
			return os.Args[i+1]
		}
		if strings.HasPrefix(arg, "--config=") {
			return strings.TrimPrefix(arg, "--config=")
		}
	}
	return filepath.Join(configBaseDirOrDot(MainConfigBaseDir), "config_custom.yaml")
}

func scheduleUpdateGuard(transactionPath string, delay time.Duration) error {
	if strings.TrimSpace(transactionPath) == "" {
		return errors.New("更新事务路径为空")
	}
	var tx updateTransaction
	if err := readJSONFile(transactionPath, &tx); err != nil {
		return err
	}
	go func() {
		time.Sleep(delay)
		args := []string{tx.PreviousBinaryPath, "update-guard", "--transaction", transactionPath}
		_ = syscall.Exec(tx.PreviousBinaryPath, args, os.Environ())
		os.Exit(1)
	}()
	return nil
}

func runUpdateGuard(transactionPath string) error {
	var tx updateTransaction
	if err := readJSONFile(transactionPath, &tx); err != nil {
		return err
	}
	if tx.Format != updateTransactionFormat || tx.Status != "staged" {
		return errors.New("更新事务状态无效")
	}
	tx.Status = "installing"
	if err := writeJSONFileAtomic(transactionPath, tx, 0o600); err != nil {
		return execTransactionBinary(tx.ExecutablePath, tx.OriginalArgs)
	}
	if err := installBinary(tx.ExecutablePath, tx.CandidatePath, 0o755); err != nil {
		return rollbackAndExecPrevious(&tx, fmt.Errorf("安装候选二进制失败: %w", err))
	}
	tx.Status = "testing"
	if err := writeJSONFileAtomic(transactionPath, tx, 0o600); err != nil {
		return rollbackAndExecPrevious(&tx, err)
	}

	args := append([]string(nil), tx.OriginalArgs[1:]...)
	cmd := exec.Command(tx.ExecutablePath, args...)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.Stdin = os.Stdin
	cmd.Env = os.Environ()
	cmd.Env = append(cmd.Env, "MOSDNS_UPDATE_GUARD_CANDIDATE="+transactionPath)
	if tx.ConfigPackagePath != "" {
		cmd.Env = append(cmd.Env, "MOSDNS_CONFIG_PACKAGE_URL=file://"+tx.ConfigPackagePath)
	}
	if err := cmd.Start(); err != nil {
		return rollbackAndExecPrevious(&tx, fmt.Errorf("启动候选版本失败: %w", err))
	}

	exitCh := make(chan error, 1)
	go func() { exitCh <- cmd.Wait() }()
	healthErr := waitForCandidateHealth(&tx, exitCh)
	if healthErr != nil {
		if cmd.Process != nil {
			_ = cmd.Process.Signal(syscall.SIGTERM)
			select {
			case <-exitCh:
			case <-time.After(5 * time.Second):
				_ = cmd.Process.Kill()
			}
		}
		return rollbackAndExecPrevious(&tx, healthErr)
	}

	_ = cmd.Process.Signal(syscall.SIGTERM)
	select {
	case <-exitCh:
	case <-time.After(10 * time.Second):
		_ = cmd.Process.Kill()
		<-exitCh
	}
	tx.Status = "verified"
	if err := writeJSONFileAtomic(transactionPath, tx, 0o600); err != nil {
		return rollbackAndExecPrevious(&tx, err)
	}
	if err := saveInstalledUpdateState(tx.ExecutablePath, tx.TargetSignature); err != nil {
		return rollbackAndExecPrevious(&tx, err)
	}
	return execTransactionBinary(tx.ExecutablePath, tx.OriginalArgs)
}

func waitForCandidateHealth(tx *updateTransaction, exitCh <-chan error) error {
	deadline := time.Now().Add(updateHealthTimeout)
	client := &http.Client{Timeout: 2 * time.Second}
	consecutiveSuccesses := 0
	for time.Now().Before(deadline) {
		select {
		case err := <-exitCh:
			if err == nil {
				err = errors.New("候选进程提前退出")
			}
			return err
		default:
		}
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		req, _ := http.NewRequestWithContext(ctx, http.MethodGet, tx.HealthURL, nil)
		resp, err := client.Do(req)
		if err == nil {
			var health struct {
				Ready         bool   `json:"ready"`
				Version       string `json:"version"`
				AppliedSchema int    `json:"applied_config_schema"`
			}
			decodeErr := json.NewDecoder(resp.Body).Decode(&health)
			resp.Body.Close()
			if decodeErr == nil && resp.StatusCode == http.StatusOK && health.Ready && health.Version == tx.TargetVersion && health.AppliedSchema >= tx.RequiredConfigSchema {
				consecutiveSuccesses++
				if consecutiveSuccesses >= 3 {
					cancel()
					return nil
				}
			} else {
				consecutiveSuccesses = 0
			}
		} else {
			consecutiveSuccesses = 0
		}
		cancel()
		time.Sleep(time.Second)
	}
	return errors.New("候选版本在 90 秒内未通过健康检查")
}

func rollbackAndExecPrevious(tx *updateTransaction, cause error) error {
	if err := restoreConfigAfterFailedCandidate(tx); err != nil {
		cause = fmt.Errorf("%v；恢复旧配置失败: %w", cause, err)
	}
	if err := installBinary(tx.ExecutablePath, tx.PreviousBinaryPath, 0o755); err != nil {
		return fmt.Errorf("%v；恢复旧二进制失败: %w", cause, err)
	}
	tx.Status = "rolled_back"
	tx.LastError = cause.Error()
	_ = writeJSONFileAtomic(filepath.Join(filepath.Dir(tx.PreviousBinaryPath), "transaction.json"), tx, 0o600)
	return execTransactionBinary(tx.ExecutablePath, tx.OriginalArgs)
}

func restoreConfigAfterFailedCandidate(tx *updateTransaction) error {
	if tx.ConfigPackagePath == "" {
		return nil
	}
	state := loadConfigUpdateState(tx.ConfigBaseDir)
	if state.PackageID != tx.ConfigPackageID || state.BackupDir == "" {
		return nil
	}
	if err := restoreConfigBackup(tx.ConfigBaseDir, state.BackupDir); err != nil {
		return err
	}
	previous := loadBackupPreviousState(state.BackupDir)
	return writeConfigUpdateState(configUpdateStatePath(tx.ConfigBaseDir), previous)
}

func execTransactionBinary(path string, originalArgs []string) error {
	args := append([]string{path}, originalArgs[1:]...)
	return syscall.Exec(path, args, os.Environ())
}

func readJSONFile(path string, target any) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	return json.Unmarshal(data, target)
}

func saveInstalledUpdateState(exePath, signature string) error {
	if signature == "" {
		return nil
	}
	data, err := json.Marshal(updateState{AssetSignature: signature, UpdatedAt: time.Now()})
	if err != nil {
		return err
	}
	return writeBytesAtomic(filepath.Join(filepath.Dir(exePath), stateFileName), data, 0o600)
}

func recoverAbandonedUpdateTransaction(baseDir string) error {
	root := filepath.Join(configBaseDirOrDot(baseDir), "update")
	entries, err := os.ReadDir(root)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	activeCandidate := os.Getenv("MOSDNS_UPDATE_GUARD_CANDIDATE")
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		path := filepath.Join(root, entry.Name(), "transaction.json")
		var tx updateTransaction
		if readJSONFile(path, &tx) != nil || (tx.Status != "installing" && tx.Status != "testing") || path == activeCandidate {
			continue
		}
		_ = restoreConfigAfterFailedCandidate(&tx)
		if err := installBinary(tx.ExecutablePath, tx.PreviousBinaryPath, 0o755); err != nil {
			return fmt.Errorf("恢复中断更新的旧二进制失败: %w", err)
		}
		tx.Status = "rolled_back"
		tx.LastError = "检测到未完成的二进制更新，已在启动前自动回滚"
		_ = writeJSONFileAtomic(path, tx, 0o600)
		return execTransactionBinary(tx.ExecutablePath, tx.OriginalArgs)
	}
	return nil
}

func latestUpdateRollback(baseDir string) (string, bool) {
	root := filepath.Join(configBaseDirOrDot(baseDir), "update")
	entries, err := os.ReadDir(root)
	if err != nil {
		return "", false
	}
	var latest updateTransaction
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		var tx updateTransaction
		if readJSONFile(filepath.Join(root, entry.Name(), "transaction.json"), &tx) != nil || tx.Status != "rolled_back" {
			continue
		}
		if latest.CreatedAt.IsZero() || tx.CreatedAt.After(latest.CreatedAt) {
			latest = tx
		}
	}
	if latest.CreatedAt.IsZero() {
		return "", false
	}
	message := latest.LastError
	if message == "" {
		message = "候选版本未通过健康检查"
	}
	return message, true
}

func cleanupVerifiedUpdateTransactions(baseDir string) {
	root := filepath.Join(configBaseDirOrDot(baseDir), "update")
	entries, err := os.ReadDir(root)
	if err != nil {
		return
	}
	hasVerified := false
	statuses := make(map[string]string, len(entries))
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		path := filepath.Join(root, entry.Name(), "transaction.json")
		var tx updateTransaction
		if readJSONFile(path, &tx) == nil {
			statuses[path] = tx.Status
			hasVerified = hasVerified || tx.Status == "verified"
		}
	}
	if !hasVerified {
		return
	}
	for path, status := range statuses {
		if status == "verified" || status == "rolled_back" {
			_ = os.RemoveAll(filepath.Dir(path))
		}
	}
}
