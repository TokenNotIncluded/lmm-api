package appcli

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

func (runtime *productionRuntime) apply(ctx context.Context, workspace productionWorkspace, options productionTransactionOptions) (result productionStatus, returnErr error) {
	if _, err := os.Lstat(workspace.manifestPath); !errors.Is(err, os.ErrNotExist) {
		return productionStatus{}, errors.New("deployment manifest already exists")
	}
	if _, err := os.Lstat(workspace.statusPath); !errors.Is(err, os.ErrNotExist) {
		return productionStatus{}, errors.New("deployment status already exists")
	}
	if err := runtime.validateTransactionLock(workspace); err != nil {
		return productionStatus{}, err
	}
	prepared := productionStatus{Phase: "PREPARING", Version: options.ExpectedVersion}
	if err := runtime.writeStatus(workspace, prepared); err != nil {
		return productionStatus{}, err
	}
	armed := false
	awaitingConfirmation := false
	defer func() {
		if returnErr == nil {
			return
		}
		if awaitingConfirmation {
			return
		}
		if armed {
			if _, rollbackErr := runtime.rollback(ctx, workspace, "activation-failure"); rollbackErr != nil {
				returnErr = errors.Join(returnErr, fmt.Errorf("automatic rollback failed: %w", rollbackErr))
			}
			return
		}
		_ = os.Remove(workspace.probeToken)
		_ = runtime.writeStatus(workspace, productionStatus{
			Phase: "FAILED_PREARM", Version: options.ExpectedVersion, Reason: "activation-preparation-failed",
		})
		_ = runtime.releaseTransactionLock(workspace)
	}()

	for label, staged := range []struct {
		path   string
		digest string
	}{
		{options.Package, options.PackageSHA256},
		{options.RollbackPackage, options.RollbackSHA256},
		{options.ProbeBinary, options.ProbeBinarySHA256},
	} {
		name := []string{"candidate package", "rollback package", "probe binary"}[label]
		if err := runtime.validateStagedFile(workspace, staged.path, staged.digest, name); err != nil {
			return productionStatus{}, err
		}
	}
	archivedEnvironment, err := runtime.validateBackupSet(ctx, workspace, options.BackupDir)
	if err != nil {
		return productionStatus{}, err
	}
	if err := validateBackupAttestation(options.BackupDir, workspace.id); err != nil {
		return productionStatus{}, err
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"is-active", "--quiet", runtime.paths.Service}}); err != nil {
		return productionStatus{}, errors.New("pre-upgrade lmm-api service is not active")
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"is-enabled", "--quiet", runtime.paths.Service}}); err != nil {
		return productionStatus{}, errors.New("pre-upgrade lmm-api service is not enabled")
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "pacman", Args: []string{"-Q", "lmm-api"}}); err == nil {
		return productionStatus{}, errors.New("production still has the split lmm-api package; direct Go deployment only")
	}
	installedPackageOutput, err := runtime.runner.Run(ctx, productionCommand{Name: "pacman", Args: []string{"-Q", "lmm-api-go"}})
	if err != nil {
		return productionStatus{}, fmt.Errorf("query installed Go package: %w", err)
	}
	installedPackage := strings.TrimSpace(string(installedPackageOutput))
	rollbackIdentity, err := runtime.runner.Run(ctx, productionCommand{Name: "pacman", Args: []string{"-Qp", options.RollbackPackage}})
	if err != nil || strings.TrimSpace(string(rollbackIdentity)) != installedPackage {
		return productionStatus{}, errors.New("rollback package does not exactly match installed lmm-api-go")
	}
	candidateIdentity, err := runtime.runner.Run(ctx, productionCommand{Name: "pacman", Args: []string{"-Qp", options.Package}})
	if err != nil || strings.TrimSpace(string(candidateIdentity)) != "lmm-api-go "+options.ExpectedVersion+"-1" {
		return productionStatus{}, errors.New("candidate package identity mismatch")
	}
	probeVersion, err := runtime.runner.Run(ctx, productionCommand{Name: options.ProbeBinary, Args: []string{"version"}})
	if err != nil || strings.TrimSpace(string(probeVersion)) != options.ExpectedVersion {
		return productionStatus{}, errors.New("candidate probe binary version mismatch")
	}
	oldVersion, err := runtime.probeStatus(ctx, options.ProbeBinary, runtime.paths.LocalBaseURL, "")
	if err != nil {
		return productionStatus{}, fmt.Errorf("pre-upgrade local status probe failed: %w", err)
	}
	if _, err := runtime.probeStatus(ctx, options.ProbeBinary, runtime.paths.PublicBaseURL, oldVersion); err != nil {
		return productionStatus{}, fmt.Errorf("pre-upgrade public status probe failed: %w", err)
	}
	comparisonOutput, err := runtime.runner.Run(ctx, productionCommand{Name: "vercmp", Args: []string{oldVersion, options.ExpectedVersion}})
	if err != nil {
		return productionStatus{}, fmt.Errorf("compare release versions: %w", err)
	}
	comparison, err := strconv.Atoi(strings.TrimSpace(string(comparisonOutput)))
	if err != nil || comparison >= 0 {
		return productionStatus{}, fmt.Errorf("candidate is not an upgrade: %s -> %s", oldVersion, options.ExpectedVersion)
	}
	oldFrontendRelease, err := currentFrontendRelease(runtime.paths.FrontendRoot)
	if err != nil {
		return productionStatus{}, fmt.Errorf("read pre-upgrade frontend release: %w", err)
	}
	oldFrontendSHA256, err := sha256File(filepath.Join(runtime.paths.FrontendRoot, "current", "index.html"))
	if err != nil {
		return productionStatus{}, fmt.Errorf("hash pre-upgrade frontend index: %w", err)
	}
	if err := runtime.probeFrontend(ctx, options.ProbeBinary, oldFrontendSHA256); err != nil {
		return productionStatus{}, fmt.Errorf("pre-upgrade public frontend probe failed: %w", err)
	}
	memoryExisted, memoryRestoreSHA256, environmentRestoreSHA256, err := runtime.saveRestoreState(workspace, archivedEnvironment)
	if err != nil {
		return productionStatus{}, err
	}
	nginxEdgeRestoreSHA256 := ""
	if runtime.paths.EdgeAssetRoot != "" {
		nginxEdgeRestoreSHA256, err = runtime.captureEdgePolicyBackup(filepath.Join(workspace.configRestore, "nginx-edge"))
		if err != nil {
			return productionStatus{}, fmt.Errorf("capture nginx edge-policy restore state: %w", err)
		}
	}
	databaseSchema, err := runtime.captureDatabaseAccess(ctx, workspace, archivedEnvironment)
	if err != nil {
		return productionStatus{}, err
	}
	if err := runtime.probeModels(ctx, options.ProbeBinary, workspace.probeToken); err != nil {
		return productionStatus{}, fmt.Errorf("pre-upgrade authenticated business probe failed: %w", err)
	}
	if err := runtime.probeLive(ctx, options.ProbeBinary); err != nil {
		return productionStatus{}, fmt.Errorf("pre-upgrade live probe failed: %w", err)
	}

	deadline := utcSecond(runtime.now().Add(options.RollbackWindow))
	manifest := productionManifest{
		Package: options.Package, PackageSHA256: options.PackageSHA256,
		RollbackPackage: options.RollbackPackage, RollbackSHA256: options.RollbackSHA256,
		ProbeBinary: options.ProbeBinary, ProbeBinarySHA256: options.ProbeBinarySHA256,
		ExpectedVersion: options.ExpectedVersion, OldVersion: oldVersion,
		FrontendIndexSHA256: options.FrontendIndexSHA256,
		OldFrontendRelease:  oldFrontendRelease, OldFrontendIndexSHA256: oldFrontendSHA256,
		BackupDir: options.BackupDir, DatabaseSchema: databaseSchema, DeadlineUTC: deadline,
		MemoryDropInExisted: memoryExisted, MemoryDropInRestoreSHA256: memoryRestoreSHA256,
		EnvironmentRestoreSHA256: environmentRestoreSHA256,
		NginxEdgeRestoreSHA256:   nginxEdgeRestoreSHA256,
		PreserveEdgePolicy:       options.PreserveEdgePolicy,
	}
	if err := runtime.writeManifest(workspace, manifest); err != nil {
		return productionStatus{}, fmt.Errorf("write deployment manifest: %w", err)
	}
	if err := runtime.writeStatus(workspace, productionStatus{
		Phase: "ARMING", Version: options.ExpectedVersion, Previous: oldVersion,
		RollbackTimer: workspace.timerUnit, DeadlineUTC: deadline,
	}); err != nil {
		return productionStatus{}, err
	}
	timerMayBeActive, err := runtime.armRollbackTimer(ctx, workspace, manifest)
	armed = timerMayBeActive
	if err != nil {
		return productionStatus{}, err
	}
	armedStatus := productionStatus{
		Phase: "ARMED", Version: options.ExpectedVersion, Previous: oldVersion,
		RollbackTimer: workspace.timerUnit, DeadlineUTC: deadline,
	}
	if err := runtime.writeStatus(workspace, armedStatus); err != nil {
		return productionStatus{}, err
	}

	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"stop", runtime.paths.Service}}); err != nil {
		return productionStatus{}, fmt.Errorf("stop current Go service: %w", err)
	}
	if err := runtime.writeStatus(workspace, productionStatus{
		Phase: "MIGRATING", Version: options.ExpectedVersion, Previous: oldVersion,
		RollbackTimer: workspace.timerUnit, DeadlineUTC: deadline,
	}); err != nil {
		return productionStatus{}, err
	}
	if err := runtime.runMigration(ctx, manifest, "apply"); err != nil {
		return productionStatus{}, err
	}
	if err := runtime.runMigration(ctx, manifest, "verify"); err != nil {
		return productionStatus{}, err
	}
	if err := runtime.writeStatus(workspace, productionStatus{
		Phase: "DEPLOYING", Version: options.ExpectedVersion, Previous: oldVersion,
		RollbackTimer: workspace.timerUnit, DeadlineUTC: deadline,
	}); err != nil {
		return productionStatus{}, err
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{
		Name: "pacman", Args: []string{"-U", "--noconfirm", options.Package}, Timeout: 5 * time.Minute,
	}); err != nil {
		return productionStatus{}, fmt.Errorf("install candidate package: %w", err)
	}
	if err := runtime.restoreConfiguration(workspace, manifest); err != nil {
		return productionStatus{}, err
	}
	if manifest.NginxEdgeRestoreSHA256 != "" && !manifest.PreserveEdgePolicy {
		if err := runtime.applyEdgePolicyAssets(ctx, runtime.paths.EdgeAssetRoot, filepath.Join(workspace.configRestore, "nginx-edge"), true); err != nil {
			return productionStatus{}, fmt.Errorf("install managed nginx edge policy: %w", err)
		}
	}
	if err := hardenProductionConfiguration(productionHardenOptions{
		EnvFile:   filepath.Join(runtime.paths.ConfigDir, "lmm-api-go.env"),
		DropInDir: runtime.paths.DropInDir,
	}); err != nil {
		return productionStatus{}, err
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"daemon-reload"}}); err != nil {
		return productionStatus{}, fmt.Errorf("reload systemd after package installation: %w", err)
	}
	integrityOutput, err := runtime.runner.Run(ctx, productionCommand{
		Name: "pacman", Args: []string{"-Qkk", "lmm-api-go"}, Env: append(os.Environ(), "LC_ALL=C"),
	})
	if err != nil {
		return productionStatus{}, fmt.Errorf("verify installed package files: %w", err)
	}
	if !strings.Contains(string(integrityOutput), "0 altered files") {
		return productionStatus{}, fmt.Errorf("installed package integrity is not clean: %s", strings.TrimSpace(string(integrityOutput)))
	}
	installedVersion, err := runtime.runner.Run(ctx, productionCommand{Name: runtime.paths.InstalledBinary, Args: []string{"version"}})
	if err != nil || strings.TrimSpace(string(installedVersion)) != options.ExpectedVersion {
		return productionStatus{}, errors.New("installed binary version mismatch")
	}
	for _, removed := range runtime.paths.RemovedPaths {
		if _, err := os.Lstat(removed); err == nil || !errors.Is(err, os.ErrNotExist) {
			return productionStatus{}, fmt.Errorf("removed split-architecture path remains: %s", removed)
		}
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"enable", "--now", runtime.paths.Service}}); err != nil {
		return productionStatus{}, fmt.Errorf("start candidate Go service: %w", err)
	}
	if err := executeFrontendDeploy(frontendDeployOptions{
		Action: "publish", Root: runtime.paths.FrontendRoot, Source: runtime.paths.PackagedFrontend,
		Release: options.ExpectedVersion, Keep: productionFrontendReleaseKeep,
	}); err != nil {
		return productionStatus{}, fmt.Errorf("publish packaged frontend: %w", err)
	}
	restartBaseline, err := runtime.readServiceRestarts(ctx)
	if err != nil {
		return productionStatus{}, err
	}
	manifest.ServiceRestartBaseline = restartBaseline
	manifest.ObservationStartedUTC = utcSecond(runtime.now())
	if err := runtime.writeManifest(workspace, manifest); err != nil {
		return productionStatus{}, err
	}
	if err := runtime.probeRelease(ctx, manifest, options.ExpectedVersion, options.FrontendIndexSHA256); err != nil {
		return productionStatus{}, fmt.Errorf("candidate release probes failed: %w", err)
	}
	awaiting := productionStatus{
		Phase: "AWAITING_CONFIRMATION", Version: options.ExpectedVersion, Previous: oldVersion,
		RollbackTimer: workspace.timerUnit, DeadlineUTC: deadline,
		AutoConfirm: !options.ManualConfirm, ObservationSec: int64(options.ObservationWindow / time.Second),
	}
	if err := runtime.writeStatus(workspace, awaiting); err != nil {
		return productionStatus{}, err
	}
	awaitingConfirmation = true
	if options.ManualConfirm {
		return awaiting, nil
	}
	if err := runtime.observe(ctx, workspace, manifest, options.ObservationWindow); err != nil {
		return productionStatus{}, &productionObservationError{err: fmt.Errorf("observation detected an anomaly; rollback timer remains armed: %w", err)}
	}
	return runtime.confirmLoaded(ctx, workspace, manifest)
}

func (runtime *productionRuntime) observe(ctx context.Context, workspace productionWorkspace, manifest productionManifest, window time.Duration) error {
	deadline := runtime.now().Add(window)
	for {
		if err := runtime.healthCheck(ctx, workspace, manifest); err != nil {
			return err
		}
		remaining := deadline.Sub(runtime.now())
		if remaining <= 0 {
			return nil
		}
		interval := productionObservationInterval
		if remaining < interval {
			interval = remaining
		}
		runtime.sleep(interval)
	}
}

func (runtime *productionRuntime) confirm(ctx context.Context, workspace productionWorkspace) (productionStatus, error) {
	manifest, err := runtime.readManifest(workspace)
	if err != nil {
		return productionStatus{}, err
	}
	return runtime.confirmLoaded(ctx, workspace, manifest)
}

func (runtime *productionRuntime) confirmLoaded(ctx context.Context, workspace productionWorkspace, manifest productionManifest) (productionStatus, error) {
	status, err := runtime.readStatus(workspace)
	if err != nil {
		return productionStatus{}, err
	}
	if status.Phase == "CONFIRMED" {
		if err := runtime.finalizeTransactionFiles(workspace); err != nil {
			return productionStatus{}, err
		}
		return status, nil
	}
	if status.Phase != "AWAITING_CONFIRMATION" && status.Phase != "CONFIRMING" {
		return productionStatus{}, fmt.Errorf("deployment phase %s is not awaiting confirmation", status.Phase)
	}
	if err := runtime.validateTransactionLock(workspace); err != nil {
		return productionStatus{}, err
	}
	if err := runtime.healthCheck(ctx, workspace, manifest); err != nil {
		return productionStatus{}, fmt.Errorf("final production health gate failed: %w", err)
	}
	if err := runtime.preserveConfirmedPackage(manifest); err != nil {
		return productionStatus{}, fmt.Errorf("preserve confirmed rollback package: %w", err)
	}
	if status.Phase == "AWAITING_CONFIRMATION" {
		if err := runtime.writeStatus(workspace, productionStatus{
			Phase: "CONFIRMING", Version: manifest.ExpectedVersion, Previous: manifest.OldVersion,
			RollbackTimer: workspace.timerUnit, DeadlineUTC: manifest.DeadlineUTC,
		}); err != nil {
			return productionStatus{}, err
		}
	}
	if err := runtime.disarmRollbackTimer(ctx, workspace, true); err != nil {
		return productionStatus{}, err
	}
	confirmed := productionStatus{
		Phase: "CONFIRMED", Version: manifest.ExpectedVersion, Previous: manifest.OldVersion,
		Reason: "native-cli-health-gates-passed",
	}
	if err := runtime.writeStatus(workspace, confirmed); err != nil {
		return productionStatus{}, err
	}
	if err := runtime.finalizeTransactionFiles(workspace); err != nil {
		return productionStatus{}, err
	}
	return confirmed, nil
}

func (runtime *productionRuntime) rollback(ctx context.Context, workspace productionWorkspace, reason string) (productionStatus, error) {
	manifest, err := runtime.readManifest(workspace)
	if err != nil {
		return productionStatus{}, err
	}
	status, err := runtime.readStatus(workspace)
	if err != nil {
		return productionStatus{}, err
	}
	if status.Phase == "CONFIRMED" || status.Phase == "ROLLED_BACK" {
		if err := runtime.finalizeTransactionFiles(workspace); err != nil {
			return productionStatus{}, err
		}
		return status, nil
	}
	switch status.Phase {
	case "ARMING", "ARMED", "MIGRATING", "DEPLOYING", "AWAITING_CONFIRMATION", "ROLLING_BACK", "ROLLBACK_FAILED":
	default:
		return productionStatus{}, fmt.Errorf("deployment phase %s is not rollback-eligible", status.Phase)
	}
	if !productionReasonPattern.MatchString(reason) {
		return productionStatus{}, errors.New("rollback reason is not audit-safe")
	}
	if err := runtime.validateTransactionLock(workspace); err != nil {
		return productionStatus{}, err
	}
	rolling := productionStatus{
		Phase: "ROLLING_BACK", Version: manifest.ExpectedVersion, Previous: manifest.OldVersion,
		Reason: reason, RollbackTimer: workspace.timerUnit, DeadlineUTC: manifest.DeadlineUTC,
	}
	if err := runtime.writeStatus(workspace, rolling); err != nil {
		return productionStatus{}, err
	}
	fail := func(operationErr error) (productionStatus, error) {
		failed := rolling
		failed.Phase = "ROLLBACK_FAILED"
		failed.Reason = reason + ":" + operationErr.Error()
		_ = runtime.writeStatus(workspace, failed)
		return productionStatus{}, operationErr
	}
	_, _ = runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"stop", runtime.paths.Service}})
	rollbackFrontendIndex := filepath.Join(runtime.paths.FrontendRoot, "releases", manifest.OldFrontendRelease, "index.html")
	rollbackFrontendSHA256, err := sha256File(rollbackFrontendIndex)
	if err != nil || rollbackFrontendSHA256 != manifest.OldFrontendIndexSHA256 {
		return fail(errors.New("previous frontend release changed after deployment was armed"))
	}
	if err := executeFrontendDeploy(frontendDeployOptions{
		Action: "rollback", Root: runtime.paths.FrontendRoot, Release: manifest.OldFrontendRelease,
		Keep: productionFrontendReleaseKeep,
	}); err != nil {
		return fail(fmt.Errorf("restore previous frontend: %w", err))
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{
		Name: "pacman", Args: []string{"-U", "--noconfirm", manifest.RollbackPackage}, Timeout: 5 * time.Minute,
	}); err != nil {
		return fail(fmt.Errorf("install rollback package: %w", err))
	}
	if err := runtime.restoreConfiguration(workspace, manifest); err != nil {
		return fail(err)
	}
	if manifest.NginxEdgeRestoreSHA256 != "" {
		if err := runtime.restoreEdgePolicyBackup(ctx, filepath.Join(workspace.configRestore, "nginx-edge"), manifest.NginxEdgeRestoreSHA256); err != nil {
			return fail(fmt.Errorf("restore nginx edge policy: %w", err))
		}
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"daemon-reload"}}); err != nil {
		return fail(fmt.Errorf("reload systemd for rollback: %w", err))
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"enable", "--now", runtime.paths.Service}}); err != nil {
		return fail(fmt.Errorf("start rolled-back Go service: %w", err))
	}
	if err := runtime.probeRelease(ctx, manifest, manifest.OldVersion, manifest.OldFrontendIndexSHA256); err != nil {
		return fail(fmt.Errorf("rolled-back release probes failed: %w", err))
	}
	// The rollback command is executed by the rollback service itself.  It may
	// disable its timer, but must not stop its own unit before the terminal
	// status and transaction lock are written.
	if err := runtime.disarmRollbackTimer(ctx, workspace, false); err != nil {
		return fail(err)
	}
	rolledBack := productionStatus{
		Phase: "ROLLED_BACK", Version: manifest.OldVersion, Previous: manifest.ExpectedVersion, Reason: reason,
	}
	if err := runtime.writeStatus(workspace, rolledBack); err != nil {
		return fail(err)
	}
	if err := runtime.finalizeTransactionFiles(workspace); err != nil {
		return fail(err)
	}
	return rolledBack, nil
}

func (runtime *productionRuntime) armRollbackTimer(ctx context.Context, workspace productionWorkspace, manifest productionManifest) (bool, error) {
	if err := ensureRealDirectory(runtime.paths.SystemdUnitRoot, 0o755); err != nil {
		return false, fmt.Errorf("prepare systemd unit directory: %w", err)
	}
	for _, path := range []string{workspace.timerPath, workspace.rollbackPath} {
		if _, err := os.Lstat(path); !errors.Is(err, os.ErrNotExist) {
			return false, errors.New("release-scoped rollback unit already exists or is unsafe")
		}
	}
	rollbackContent := fmt.Sprintf(`[Unit]
Description=LMM API Go release-scoped automatic rollback (%s)

[Service]
Type=oneshot
ExecStart=%s deploy production rollback --workspace %s --reason watchdog-deadline
	TimeoutStartSec=10min
Restart=on-failure
RestartSec=10s
`, workspace.id, manifest.ProbeBinary, workspace.root)
	timerContent := fmt.Sprintf(`[Unit]
Description=LMM API Go rollback deadline (%s)

[Timer]
OnCalendar=@%d
AccuracySec=1s
Persistent=true
Unit=%s

[Install]
WantedBy=timers.target
`, workspace.id, manifest.DeadlineUTC.Unix(), workspace.rollbackUnit)
	if err := writeAtomicRegularFile(workspace.rollbackPath, []byte(rollbackContent), 0o644); err != nil {
		return false, fmt.Errorf("write rollback service: %w", err)
	}
	if err := writeAtomicRegularFile(workspace.timerPath, []byte(timerContent), 0o644); err != nil {
		_ = os.Remove(workspace.rollbackPath)
		return false, fmt.Errorf("write rollback timer: %w", err)
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"daemon-reload"}}); err != nil {
		_ = os.Remove(workspace.timerPath)
		_ = os.Remove(workspace.rollbackPath)
		_, _ = runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"daemon-reload"}})
		return false, fmt.Errorf("reload rollback units: %w", err)
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"enable", "--now", workspace.timerUnit}}); err != nil {
		// systemctl may have started the timer before reporting an error. Treat
		// this as armed so the caller performs the release-scoped rollback path
		// and retains the transaction lock if disarming cannot be proven.
		return true, fmt.Errorf("arm rollback timer: %w", err)
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"is-active", "--quiet", workspace.timerUnit}}); err != nil {
		return true, errors.New("rollback timer did not become active")
	}
	return true, nil
}

func (runtime *productionRuntime) disarmRollbackTimer(ctx context.Context, workspace productionWorkspace, stopRollbackService bool) error {
	timerExists := false
	rollbackExists := false
	for path, exists := range map[string]*bool{workspace.timerPath: &timerExists, workspace.rollbackPath: &rollbackExists} {
		info, err := os.Lstat(path)
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil || info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
			return fmt.Errorf("rollback unit path is unsafe: %s", path)
		}
		*exists = true
	}
	if timerExists {
		if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"disable", "--now", workspace.timerUnit}}); err != nil {
			return fmt.Errorf("disable rollback timer: %w", err)
		}
	}
	if rollbackExists && stopRollbackService {
		_, _ = runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"stop", workspace.rollbackUnit}})
	}
	_, _ = runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"reset-failed", workspace.timerUnit, workspace.rollbackUnit}})
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"is-active", "--quiet", workspace.timerUnit}}); err == nil {
		return errors.New("rollback timer remains active after disable")
	}
	if stopRollbackService {
		if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"is-active", "--quiet", workspace.rollbackUnit}}); err == nil {
			return errors.New("rollback service remains active after stop")
		}
	}
	for _, path := range []string{workspace.timerPath, workspace.rollbackPath} {
		info, err := os.Lstat(path)
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil || info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
			return fmt.Errorf("rollback unit path is unsafe: %s", path)
		}
		if err := os.Remove(path); err != nil {
			return fmt.Errorf("remove rollback unit %s: %w", filepath.Base(path), err)
		}
	}
	if _, err := runtime.runner.Run(ctx, productionCommand{Name: "systemctl", Args: []string{"daemon-reload"}}); err != nil {
		return fmt.Errorf("reload systemd after removing rollback units: %w", err)
	}
	return nil
}

func (runtime *productionRuntime) finalizeTransactionFiles(workspace productionWorkspace) error {
	if err := os.Remove(workspace.probeToken); err != nil && !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("remove production probe token: %w", err)
	}
	return runtime.releaseTransactionLock(workspace)
}

func (runtime *productionRuntime) preserveConfirmedPackage(manifest productionManifest) error {
	if err := ensureRealDirectory(runtime.paths.ReleasePackages, 0o700); err != nil {
		return err
	}
	if !strings.Contains(filepath.Base(manifest.Package), ".pkg.tar.") {
		return errors.New("candidate package filename is not an Arch package")
	}
	destination := filepath.Join(runtime.paths.ReleasePackages, filepath.Base(manifest.Package))
	if info, err := os.Lstat(destination); err == nil {
		if info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
			return errors.New("preserved release package path is unsafe")
		}
		digest, err := sha256File(destination)
		if err != nil || digest != manifest.PackageSHA256 {
			return errors.New("preserved release package conflicts with the confirmed candidate")
		}
		return nil
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	if err := copyRegularFile(manifest.Package, destination, 0o600, true); err != nil {
		return err
	}
	return syncDirectory(runtime.paths.ReleasePackages)
}
