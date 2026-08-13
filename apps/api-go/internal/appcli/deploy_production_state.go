package appcli

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"

	"golang.org/x/sys/unix"
)

const (
	productionServiceName          = "lmm-api.service"
	productionExpectedHost         = "arch-dmit"
	productionDefaultRollback      = 10 * time.Minute
	productionDefaultObservation   = 3 * time.Minute
	productionObservationInterval  = 10 * time.Second
	productionCommandTimeout       = 2 * time.Minute
	productionProbeTimeout         = 8 * time.Second
	productionProbeAttempts        = 45
	productionTransactionFormat    = 1
	productionStatusFormat         = 1
	productionFrontendReleaseKeep  = 3
	productionTransactionMarker    = "deployment.env"
	productionWorkspaceMarker      = ".lmm-deploy-workspace"
	productionManifestFilename     = "deployment.json"
	productionStatusFilename       = "status.json"
	productionProbeTokenFilename   = "probe-token"
	productionConfigRestoreDirname = "config-restore"
)

var (
	productionIDPattern      = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$`)
	productionVersionPattern = regexp.MustCompile(`^[0-9][0-9A-Za-z._+]*$`)
	productionSHA256Pattern  = regexp.MustCompile(`^[0-9a-f]{64}$`)
	productionReasonPattern  = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`)
)

type productionPaths struct {
	WorkRoot         string
	BackupRoot       string
	GlobalLock       string
	TransactionLock  string
	FrontendRoot     string
	SystemdUnitRoot  string
	ConfigDir        string
	DropInDir        string
	NginxRoot        string
	EdgeAssetRoot    string
	InstalledBinary  string
	PackagedFrontend string
	MigrationWorkdir string
	ReleasePackages  string
	PackageCache     string
	RemovedPaths     []string
	Service          string
	ExpectedHost     string
	PublicBaseURL    string
	LocalBaseURL     string
	JournalUnits     []string
}

func defaultProductionPaths() productionPaths {
	return productionPaths{
		WorkRoot:         "/var/lib/lmm-api-go/deploy-work",
		BackupRoot:       "/var/lib/lmm-api-go/deploy-backups",
		GlobalLock:       "/run/lock/lmm-api-go-deploy.lock",
		TransactionLock:  "/var/lib/lmm-api-go/deploy-transaction.lock",
		FrontendRoot:     defaultFrontendRoot,
		SystemdUnitRoot:  "/etc/systemd/system",
		ConfigDir:        "/etc/lmm-api-go",
		DropInDir:        defaultProductionDropInDir,
		NginxRoot:        defaultNginxRoot,
		EdgeAssetRoot:    defaultEdgeAssetRoot,
		InstalledBinary:  "/usr/bin/lmm-api",
		PackagedFrontend: "/usr/share/lmm-api-go/frontend-dist",
		MigrationWorkdir: "/var/lib/lmm-api-go",
		ReleasePackages:  "/var/lib/lmm-api-go/release-packages",
		PackageCache:     "/var/cache/pacman/pkg",
		RemovedPaths: []string{
			"/usr/bin/lmm-api-select",
			"/usr/lib/lmm-api",
			"/usr/lib/systemd/system/lmm-api-go.service",
		},
		Service:       productionServiceName,
		ExpectedHost:  productionExpectedHost,
		PublicBaseURL: "https://api.lmm.best",
		LocalBaseURL:  "http://127.0.0.1:3000",
		JournalUnits:  []string{productionServiceName, "nginx.service"},
	}
}

type productionCommand struct {
	Name      string
	Args      []string
	Env       []string
	Dir       string
	Timeout   time.Duration
	Sensitive bool
}

type productionCommandRunner interface {
	Run(context.Context, productionCommand) ([]byte, error)
}

type osProductionCommandRunner struct{}

func (osProductionCommandRunner) Run(parent context.Context, command productionCommand) ([]byte, error) {
	timeout := command.Timeout
	if timeout <= 0 {
		timeout = productionCommandTimeout
	}
	ctx, cancel := context.WithTimeout(parent, timeout)
	defer cancel()
	process := exec.CommandContext(ctx, command.Name, command.Args...)
	if command.Dir != "" {
		process.Dir = command.Dir
	}
	if command.Env != nil {
		process.Env = command.Env
	}
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	process.Stdout = &stdout
	process.Stderr = &stderr
	err := process.Run()
	if err == nil {
		return stdout.Bytes(), nil
	}
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		return nil, fmt.Errorf("command %s timed out", filepath.Base(command.Name))
	}
	if command.Sensitive {
		return nil, fmt.Errorf("command %s failed: %w", filepath.Base(command.Name), err)
	}
	detail := strings.TrimSpace(stderr.String())
	if len(detail) > 1024 {
		detail = detail[:1024] + "..."
	}
	if detail == "" {
		return nil, fmt.Errorf("command %s failed: %w", filepath.Base(command.Name), err)
	}
	return nil, fmt.Errorf("command %s failed: %w: %s", filepath.Base(command.Name), err, detail)
}

type productionRuntime struct {
	paths         productionPaths
	runner        productionCommandRunner
	now           func() time.Time
	sleep         func(time.Duration)
	effectiveUID  func() int
	hostname      func() (string, error)
	probeAttempts int
}

func defaultProductionRuntime() *productionRuntime {
	return &productionRuntime{
		paths:         defaultProductionPaths(),
		runner:        osProductionCommandRunner{},
		now:           time.Now,
		sleep:         time.Sleep,
		effectiveUID:  os.Geteuid,
		hostname:      os.Hostname,
		probeAttempts: productionProbeAttempts,
	}
}

type productionTransactionOptions struct {
	Action              string
	Workspace           string
	Package             string
	PackageSHA256       string
	RollbackPackage     string
	RollbackSHA256      string
	ProbeBinary         string
	ProbeBinarySHA256   string
	ExpectedVersion     string
	FrontendIndexSHA256 string
	BackupDir           string
	RollbackWindow      time.Duration
	ObservationWindow   time.Duration
	ManualConfirm       bool
	PreserveEdgePolicy  bool
	Reason              string
}

type productionManifest struct {
	Format                    int       `json:"format"`
	DeploymentID              string    `json:"deployment_id"`
	Package                   string    `json:"package"`
	PackageSHA256             string    `json:"package_sha256"`
	RollbackPackage           string    `json:"rollback_package"`
	RollbackSHA256            string    `json:"rollback_sha256"`
	ProbeBinary               string    `json:"probe_binary"`
	ProbeBinarySHA256         string    `json:"probe_binary_sha256"`
	ExpectedVersion           string    `json:"expected_version"`
	OldVersion                string    `json:"old_version"`
	FrontendIndexSHA256       string    `json:"frontend_index_sha256"`
	OldFrontendRelease        string    `json:"old_frontend_release"`
	OldFrontendIndexSHA256    string    `json:"old_frontend_index_sha256"`
	BackupDir                 string    `json:"backup_dir"`
	DatabaseSchema            string    `json:"database_schema"`
	DeadlineUTC               time.Time `json:"deadline_utc"`
	ObservationStartedUTC     time.Time `json:"observation_started_utc,omitempty"`
	ServiceRestartBaseline    int64     `json:"service_restart_baseline"`
	MemoryDropInExisted       bool      `json:"memory_dropin_existed"`
	MemoryDropInRestoreSHA256 string    `json:"memory_dropin_restore_sha256,omitempty"`
	EnvironmentRestoreSHA256  string    `json:"environment_restore_sha256"`
	NginxEdgeRestoreSHA256    string    `json:"nginx_edge_restore_sha256,omitempty"`
	PreserveEdgePolicy        bool      `json:"preserve_edge_policy,omitempty"`
}

type productionStatus struct {
	Format         int       `json:"format"`
	DeploymentID   string    `json:"deployment_id"`
	Phase          string    `json:"phase"`
	Version        string    `json:"version,omitempty"`
	Previous       string    `json:"previous_version,omitempty"`
	Reason         string    `json:"reason,omitempty"`
	RollbackTimer  string    `json:"rollback_timer,omitempty"`
	DeadlineUTC    time.Time `json:"deadline_utc,omitempty"`
	UpdatedUTC     time.Time `json:"updated_utc"`
	AutoConfirm    bool      `json:"auto_confirm,omitempty"`
	ObservationSec int64     `json:"observation_seconds,omitempty"`
}

type productionWorkspace struct {
	root          string
	id            string
	stateDir      string
	stagingDir    string
	manifestPath  string
	statusPath    string
	probeToken    string
	timerUnit     string
	rollbackUnit  string
	timerPath     string
	rollbackPath  string
	configRestore string
}

type productionObservationError struct{ err error }

func (err *productionObservationError) Error() string { return err.err.Error() }
func (err *productionObservationError) Unwrap() error { return err.err }

func runProductionTransaction(action string, args []string, stdout, stderr io.Writer) int {
	options, err := parseProductionTransactionOptions(action, args, stderr)
	if errors.Is(err, flag.ErrHelp) {
		return ExitOK
	}
	if err != nil {
		_, _ = fmt.Fprintf(stderr, "%s deploy production %s: %v\n", ProgramName, action, err)
		return ExitUsage
	}
	runtime := defaultProductionRuntime()
	status, err := runtime.executeTransaction(context.Background(), options)
	if err != nil {
		_, _ = fmt.Fprintf(stderr, "%s deploy production %s: %v\n", ProgramName, action, err)
		return ExitError
	}
	encoded, err := json.MarshalIndent(status, "", "  ")
	if err != nil {
		_, _ = fmt.Fprintf(stderr, "%s deploy production %s: encode status: %v\n", ProgramName, action, err)
		return ExitError
	}
	_, _ = stdout.Write(append(encoded, '\n'))
	return ExitOK
}

func parseProductionTransactionOptions(action string, args []string, stderr io.Writer) (productionTransactionOptions, error) {
	options := productionTransactionOptions{
		Action:            action,
		RollbackWindow:    productionDefaultRollback,
		ObservationWindow: productionDefaultObservation,
		Reason:            "operator-request",
	}
	flags := flag.NewFlagSet("deploy production "+action, flag.ContinueOnError)
	flags.SetOutput(stderr)
	flags.StringVar(&options.Workspace, "workspace", "", "marker-owned target deployment workspace")
	switch action {
	case "apply":
		flags.StringVar(&options.Package, "package", "", "candidate lmm-api-go package")
		flags.StringVar(&options.PackageSHA256, "package-sha256", "", "candidate package SHA-256")
		flags.StringVar(&options.RollbackPackage, "rollback-package", "", "captured installed lmm-api-go package")
		flags.StringVar(&options.RollbackSHA256, "rollback-sha256", "", "rollback package SHA-256")
		flags.StringVar(&options.ProbeBinary, "probe-binary", "", "candidate lmm-api-go binary used for migration and probes")
		flags.StringVar(&options.ProbeBinarySHA256, "probe-binary-sha256", "", "probe binary SHA-256")
		flags.StringVar(&options.ExpectedVersion, "expected-version", "", "candidate release version")
		flags.StringVar(&options.FrontendIndexSHA256, "frontend-index-sha256", "", "candidate index.html SHA-256")
		flags.StringVar(&options.BackupDir, "backup-dir", "", "verified target backup directory")
		rollbackSeconds := int(options.RollbackWindow / time.Second)
		observationSeconds := int(options.ObservationWindow / time.Second)
		flags.IntVar(&rollbackSeconds, "rollback-seconds", rollbackSeconds, "automatic rollback window (600-1800)")
		flags.IntVar(&observationSeconds, "observation-seconds", observationSeconds, "stability observation window (120-360)")
		flags.BoolVar(&options.ManualConfirm, "manual-confirm", false, "leave a healthy release awaiting an explicit confirm command")
		flags.BoolVar(&options.PreserveEdgePolicy, "preserve-edge-policy", false, "preserve the active nginx edge policy instead of installing package defaults")
		if err := flags.Parse(args); err != nil {
			return productionTransactionOptions{}, err
		}
		options.RollbackWindow = time.Duration(rollbackSeconds) * time.Second
		options.ObservationWindow = time.Duration(observationSeconds) * time.Second
	case "rollback":
		flags.StringVar(&options.Reason, "reason", options.Reason, "audit-safe rollback reason")
		if err := flags.Parse(args); err != nil {
			return productionTransactionOptions{}, err
		}
	case "status", "confirm":
		if err := flags.Parse(args); err != nil {
			return productionTransactionOptions{}, err
		}
	default:
		return productionTransactionOptions{}, fmt.Errorf("unsupported action %q", action)
	}
	flags.Usage = func() { writeDeployUsage(stderr) }
	if flags.NArg() != 0 {
		return productionTransactionOptions{}, errors.New("unexpected positional arguments")
	}
	if options.Workspace == "" {
		return productionTransactionOptions{}, errors.New("--workspace is required")
	}
	workspace, err := cleanAbsoluteNonRoot(options.Workspace)
	if err != nil {
		return productionTransactionOptions{}, fmt.Errorf("invalid --workspace: %w", err)
	}
	options.Workspace = workspace
	if action == "apply" {
		for label, value := range map[string]string{
			"--package": options.Package, "--package-sha256": options.PackageSHA256,
			"--rollback-package": options.RollbackPackage, "--rollback-sha256": options.RollbackSHA256,
			"--probe-binary": options.ProbeBinary, "--probe-binary-sha256": options.ProbeBinarySHA256,
			"--expected-version": options.ExpectedVersion, "--frontend-index-sha256": options.FrontendIndexSHA256,
			"--backup-dir": options.BackupDir,
		} {
			if value == "" {
				return productionTransactionOptions{}, fmt.Errorf("%s is required", label)
			}
		}
		if !productionSHA256Pattern.MatchString(options.PackageSHA256) ||
			!productionSHA256Pattern.MatchString(options.RollbackSHA256) ||
			!productionSHA256Pattern.MatchString(options.ProbeBinarySHA256) ||
			!productionSHA256Pattern.MatchString(options.FrontendIndexSHA256) {
			return productionTransactionOptions{}, errors.New("all SHA-256 values must be 64 lowercase hexadecimal characters")
		}
		if !productionVersionPattern.MatchString(options.ExpectedVersion) {
			return productionTransactionOptions{}, errors.New("invalid --expected-version")
		}
		if options.RollbackWindow < 10*time.Minute || options.RollbackWindow > 30*time.Minute {
			return productionTransactionOptions{}, errors.New("--rollback-seconds must be between 600 and 1800")
		}
		if options.ObservationWindow < 2*time.Minute || options.ObservationWindow > 6*time.Minute {
			return productionTransactionOptions{}, errors.New("--observation-seconds must be between 120 and 360")
		}
		for label, value := range map[string]*string{
			"--package": &options.Package, "--rollback-package": &options.RollbackPackage,
			"--probe-binary": &options.ProbeBinary, "--backup-dir": &options.BackupDir,
		} {
			clean, err := cleanAbsoluteNonRoot(*value)
			if err != nil {
				return productionTransactionOptions{}, fmt.Errorf("invalid %s: %w", label, err)
			}
			*value = clean
		}
	}
	if action == "rollback" && !productionReasonPattern.MatchString(options.Reason) {
		return productionTransactionOptions{}, errors.New("--reason must contain only audit-safe letters, digits, dot, underscore, colon, or dash")
	}
	return options, nil
}

func (runtime *productionRuntime) executeTransaction(ctx context.Context, options productionTransactionOptions) (productionStatus, error) {
	workspace, err := runtime.openWorkspace(options.Workspace)
	if err != nil {
		return productionStatus{}, err
	}
	if options.Action != "status" {
		if runtime.effectiveUID() != 0 {
			return productionStatus{}, errors.New("must run as root")
		}
		hostname, err := runtime.hostname()
		if err != nil {
			return productionStatus{}, fmt.Errorf("read production host identity: %w", err)
		}
		if hostname != runtime.paths.ExpectedHost {
			return productionStatus{}, fmt.Errorf("production host identity mismatch: got %q", hostname)
		}
	}
	lock, err := runtime.acquireGlobalLock(ctx)
	if err != nil {
		return productionStatus{}, err
	}
	defer func() {
		_ = unix.Flock(int(lock.Fd()), unix.LOCK_UN)
		_ = lock.Close()
	}()

	switch options.Action {
	case "apply":
		return runtime.apply(ctx, workspace, options)
	case "status":
		return runtime.readStatus(workspace)
	case "confirm":
		return runtime.confirm(ctx, workspace)
	case "rollback":
		return runtime.rollback(ctx, workspace, options.Reason)
	default:
		return productionStatus{}, fmt.Errorf("unsupported action %q", options.Action)
	}
}

func (runtime *productionRuntime) openWorkspace(root string) (productionWorkspace, error) {
	if filepath.Dir(root) != filepath.Clean(runtime.paths.WorkRoot) {
		return productionWorkspace{}, errors.New("workspace must be one direct child of the production work root")
	}
	id := filepath.Base(root)
	if !productionIDPattern.MatchString(id) {
		return productionWorkspace{}, errors.New("invalid deployment ID")
	}
	info, err := os.Lstat(root)
	if err != nil || info.Mode()&os.ModeSymlink != 0 || !info.IsDir() {
		return productionWorkspace{}, errors.New("workspace must be a real directory")
	}
	if err := requireRealDirectory(runtime.paths.WorkRoot); err != nil {
		return productionWorkspace{}, errors.New("production work root must be a real directory")
	}
	// Arch systemd's DynamicUser layout exposes /var/lib/lmm-api-go as a
	// managed symlink to /var/lib/private/lmm-api-go.  The workspace itself
	// must still be a real directory, but rejecting that trusted parent alias
	// makes the CLI unable to abort or inspect its own production transactions.
	canonicalWorkRoot, err := filepath.EvalSymlinks(runtime.paths.WorkRoot)
	if err != nil {
		return productionWorkspace{}, fmt.Errorf("resolve production work root: %w", err)
	}
	canonical, err := filepath.EvalSymlinks(root)
	if err != nil || filepath.Dir(filepath.Clean(canonical)) != filepath.Clean(canonicalWorkRoot) {
		return productionWorkspace{}, errors.New("workspace must be a direct child of the canonical production work root")
	}
	marker := filepath.Join(root, productionWorkspaceMarker)
	markerContent, err := readPrivateRegularFile(marker, 16<<10)
	if err != nil {
		return productionWorkspace{}, fmt.Errorf("read workspace marker: %w", err)
	}
	markerValues, err := parseSimpleManifest(markerContent)
	if err != nil || markerValues["deployment_id"] != id {
		return productionWorkspace{}, errors.New("workspace marker does not own this deployment ID")
	}
	stateDir := filepath.Join(root, "state")
	if err := ensureRealDirectory(stateDir, 0o700); err != nil {
		return productionWorkspace{}, fmt.Errorf("prepare deployment state: %w", err)
	}
	stagingDir := filepath.Join(root, "staging")
	if err := requireRealDirectory(stagingDir); err != nil {
		return productionWorkspace{}, fmt.Errorf("validate deployment staging: %w", err)
	}
	return productionWorkspace{
		root:          root,
		id:            id,
		stateDir:      stateDir,
		stagingDir:    stagingDir,
		manifestPath:  filepath.Join(stateDir, productionManifestFilename),
		statusPath:    filepath.Join(stateDir, productionStatusFilename),
		probeToken:    filepath.Join(stateDir, productionProbeTokenFilename),
		timerUnit:     "lmm-api-go-rollback-" + id + ".timer",
		rollbackUnit:  "lmm-api-go-rollback-" + id + ".service",
		timerPath:     filepath.Join(runtime.paths.SystemdUnitRoot, "lmm-api-go-rollback-"+id+".timer"),
		rollbackPath:  filepath.Join(runtime.paths.SystemdUnitRoot, "lmm-api-go-rollback-"+id+".service"),
		configRestore: filepath.Join(stateDir, productionConfigRestoreDirname),
	}, nil
}

func (runtime *productionRuntime) acquireGlobalLock(ctx context.Context) (*os.File, error) {
	parent := filepath.Dir(runtime.paths.GlobalLock)
	if err := ensureRealDirectory(parent, 0o755); err != nil {
		return nil, fmt.Errorf("prepare deployment lock: %w", err)
	}
	lock, err := os.OpenFile(runtime.paths.GlobalLock, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return nil, fmt.Errorf("open deployment lock: %w", err)
	}
	if err := lock.Chmod(0o600); err != nil {
		_ = lock.Close()
		return nil, fmt.Errorf("protect deployment lock: %w", err)
	}
	deadline := runtime.now().Add(2 * time.Minute)
	for {
		if err := unix.Flock(int(lock.Fd()), unix.LOCK_EX|unix.LOCK_NB); err == nil {
			return lock, nil
		} else if !errors.Is(err, unix.EWOULDBLOCK) && !errors.Is(err, unix.EAGAIN) {
			_ = lock.Close()
			return nil, fmt.Errorf("lock production deployment: %w", err)
		}
		if runtime.now().After(deadline) {
			_ = lock.Close()
			return nil, errors.New("another production deployment holds the global lock")
		}
		select {
		case <-ctx.Done():
			_ = lock.Close()
			return nil, ctx.Err()
		default:
			runtime.sleep(250 * time.Millisecond)
		}
	}
}

func (runtime *productionRuntime) writeStatus(workspace productionWorkspace, status productionStatus) error {
	status.Format = productionStatusFormat
	status.DeploymentID = workspace.id
	status.UpdatedUTC = runtime.now().UTC().Truncate(time.Second)
	encoded, err := json.MarshalIndent(status, "", "  ")
	if err != nil {
		return err
	}
	return writeAtomicRegularFile(workspace.statusPath, append(encoded, '\n'), 0o600)
}

func (runtime *productionRuntime) readStatus(workspace productionWorkspace) (productionStatus, error) {
	content, err := readPrivateRegularFile(workspace.statusPath, 64<<10)
	if err != nil {
		return productionStatus{}, fmt.Errorf("read deployment status: %w", err)
	}
	var status productionStatus
	if err := json.Unmarshal(content, &status); err != nil {
		return productionStatus{}, fmt.Errorf("decode deployment status: %w", err)
	}
	if status.Format != productionStatusFormat || status.DeploymentID != workspace.id || status.Phase == "" {
		return productionStatus{}, errors.New("deployment status identity is invalid")
	}
	return status, nil
}

func (runtime *productionRuntime) writeManifest(workspace productionWorkspace, manifest productionManifest) error {
	manifest.Format = productionTransactionFormat
	manifest.DeploymentID = workspace.id
	encoded, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return err
	}
	return writeAtomicRegularFile(workspace.manifestPath, append(encoded, '\n'), 0o600)
}

func (runtime *productionRuntime) readManifest(workspace productionWorkspace) (productionManifest, error) {
	content, err := readPrivateRegularFile(workspace.manifestPath, 256<<10)
	if err != nil {
		return productionManifest{}, fmt.Errorf("read deployment manifest: %w", err)
	}
	var manifest productionManifest
	if err := json.Unmarshal(content, &manifest); err != nil {
		return productionManifest{}, fmt.Errorf("decode deployment manifest: %w", err)
	}
	if manifest.Format != productionTransactionFormat || manifest.DeploymentID != workspace.id {
		return productionManifest{}, errors.New("deployment manifest identity is invalid")
	}
	if err := runtime.validateManifest(workspace, manifest); err != nil {
		return productionManifest{}, err
	}
	return manifest, nil
}

func (runtime *productionRuntime) validateManifest(workspace productionWorkspace, manifest productionManifest) error {
	if !productionVersionPattern.MatchString(manifest.ExpectedVersion) || !productionVersionPattern.MatchString(manifest.OldVersion) {
		return errors.New("deployment manifest contains an invalid version")
	}
	for _, digest := range []string{
		manifest.PackageSHA256, manifest.RollbackSHA256, manifest.ProbeBinarySHA256,
		manifest.FrontendIndexSHA256, manifest.OldFrontendIndexSHA256, manifest.EnvironmentRestoreSHA256,
	} {
		if !productionSHA256Pattern.MatchString(digest) {
			return errors.New("deployment manifest contains an invalid SHA-256")
		}
	}
	for label, path := range map[string]string{
		"package": manifest.Package, "rollback package": manifest.RollbackPackage, "probe binary": manifest.ProbeBinary,
	} {
		if !pathWithinRoot(workspace.stagingDir, path) {
			return fmt.Errorf("deployment manifest %s escapes staging", label)
		}
	}
	if manifest.BackupDir != filepath.Join(runtime.paths.BackupRoot, workspace.id) {
		return errors.New("deployment manifest backup path is not release-scoped")
	}
	if !releaseIDPattern.MatchString(manifest.OldFrontendRelease) || !isDatabaseSchema(manifest.DatabaseSchema) {
		return errors.New("deployment manifest contains unsafe release or schema data")
	}
	for label, staged := range []struct {
		path   string
		digest string
	}{
		{manifest.Package, manifest.PackageSHA256},
		{manifest.RollbackPackage, manifest.RollbackSHA256},
		{manifest.ProbeBinary, manifest.ProbeBinarySHA256},
	} {
		name := []string{"candidate package", "rollback package", "probe binary"}[label]
		if err := runtime.validateStagedFile(workspace, staged.path, staged.digest, name); err != nil {
			return fmt.Errorf("deployment manifest %s failed validation: %w", name, err)
		}
	}
	return nil
}

func readPrivateRegularFile(path string, maximum int64) ([]byte, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return nil, err
	}
	if info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
		return nil, errors.New("path is not a real regular file")
	}
	if info.Size() > maximum {
		return nil, errors.New("file exceeds the safe size limit")
	}
	return os.ReadFile(path)
}

func requireRealDirectory(path string) error {
	info, err := os.Lstat(path)
	if err != nil {
		return err
	}
	if info.Mode()&os.ModeSymlink != 0 || !info.IsDir() {
		return errors.New("path is not a real directory")
	}
	return nil
}

func ensureRealDirectory(path string, mode os.FileMode) error {
	if info, err := os.Lstat(path); err == nil {
		if info.Mode()&os.ModeSymlink != 0 || !info.IsDir() {
			return errors.New("path is not a real directory")
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	} else if err := os.MkdirAll(path, mode); err != nil {
		return err
	}
	return os.Chmod(path, mode)
}

func parseSimpleManifest(content []byte) (map[string]string, error) {
	values := make(map[string]string)
	for _, rawLine := range strings.Split(strings.ReplaceAll(string(content), "\r\n", "\n"), "\n") {
		line := strings.TrimSpace(rawLine)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		key, value, found := strings.Cut(line, "=")
		if !found || strings.TrimSpace(key) == "" {
			return nil, errors.New("invalid marker assignment")
		}
		key = strings.TrimSpace(key)
		if _, exists := values[key]; exists {
			return nil, fmt.Errorf("duplicate marker key %s", key)
		}
		values[key] = strings.TrimSpace(value)
	}
	return values, nil
}

func sha256File(path string) (string, error) {
	file, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer file.Close()
	hash := sha256.New()
	if _, err := io.Copy(hash, file); err != nil {
		return "", err
	}
	return hex.EncodeToString(hash.Sum(nil)), nil
}

func parseSingleInt(output []byte, label string) (int64, error) {
	value := strings.TrimSpace(string(output))
	parsed, err := strconv.ParseInt(value, 10, 64)
	if err != nil || parsed < 0 {
		return 0, fmt.Errorf("invalid %s value %q", label, value)
	}
	return parsed, nil
}
