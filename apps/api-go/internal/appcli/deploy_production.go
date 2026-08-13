package appcli

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
)

const (
	defaultProductionEnvFile   = "/etc/lmm-api-go/lmm-api-go.env"
	defaultProductionDropInDir = "/etc/systemd/system/lmm-api.service.d"
	productionMemoryFileName   = "80-production-memory.conf"
	legacyEmergencyMemoryFile  = "99-emergency-memory-safety.conf"
	productionMemoryHigh       = "320M"
	productionMemoryMax        = "384M"
	productionMemorySwapMax    = "256M"
)

type productionHardenOptions struct {
	EnvFile   string
	DropInDir string
}

func runProductionDeploy(args []string, stdout, stderr io.Writer) int {
	if len(args) == 0 {
		writeDeployUsage(stderr)
		return ExitUsage
	}
	switch args[0] {
	case "release":
		return runProductionRelease(args[1:], stdout, stderr)
	case "package":
		return runProductionPackage(args[1:], stdout, stderr)
	case "workspace":
		return runProductionWorkspace(args[1:], stdout, stderr)
	case "backup":
		return runProductionBackup(args[1:], stdout, stderr)
	case "harden":
		options, err := parseProductionHardenOptions(args[1:], stderr)
		if errors.Is(err, flag.ErrHelp) {
			return ExitOK
		}
		if err != nil {
			_, _ = fmt.Fprintf(stderr, "%s deploy production harden: %v\n", ProgramName, err)
			return ExitUsage
		}
		if err := hardenProductionConfiguration(options); err != nil {
			_, _ = fmt.Fprintf(stderr, "%s deploy production harden: %v\n", ProgramName, err)
			return ExitError
		}
		_, _ = fmt.Fprintln(stdout, "configuration=hardened")
		_, _ = fmt.Fprintln(stdout, "systemd_reload_required=true")
		return ExitOK
	case "edge-policy":
		return runProductionEdgePolicy(args[1:], stdout, stderr)
	case "apply", "status", "confirm", "rollback":
		return runProductionTransaction(args[0], args[1:], stdout, stderr)
	case "help", "--help", "-h":
		writeDeployUsage(stdout)
		return ExitOK
	default:
		_, _ = fmt.Fprintf(stderr, "%s deploy production: unknown action %q\n", ProgramName, args[0])
		writeDeployUsage(stderr)
		return ExitUsage
	}
}

func parseProductionHardenOptions(args []string, stderr io.Writer) (productionHardenOptions, error) {
	options := productionHardenOptions{
		EnvFile:   defaultProductionEnvFile,
		DropInDir: defaultProductionDropInDir,
	}
	flags := flag.NewFlagSet("deploy production harden", flag.ContinueOnError)
	flags.SetOutput(stderr)
	flags.StringVar(&options.EnvFile, "env-file", options.EnvFile, "production environment file")
	flags.StringVar(&options.DropInDir, "drop-in-dir", options.DropInDir, "systemd service drop-in directory")
	flags.Usage = func() { writeDeployUsage(stderr) }
	if err := flags.Parse(args); err != nil {
		return productionHardenOptions{}, err
	}
	if flags.NArg() != 0 {
		return productionHardenOptions{}, errors.New("unexpected positional arguments")
	}
	var err error
	options.EnvFile, err = cleanAbsoluteNonRoot(options.EnvFile)
	if err != nil {
		return productionHardenOptions{}, fmt.Errorf("invalid --env-file: %w", err)
	}
	options.DropInDir, err = cleanAbsoluteNonRoot(options.DropInDir)
	if err != nil {
		return productionHardenOptions{}, fmt.Errorf("invalid --drop-in-dir: %w", err)
	}
	return options, nil
}

func hardenProductionConfiguration(options productionHardenOptions) error {
	info, err := os.Lstat(options.EnvFile)
	if err != nil {
		return fmt.Errorf("inspect environment file: %w", err)
	}
	if info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
		return errors.New("environment file must be a real regular file")
	}
	content, err := os.ReadFile(options.EnvFile)
	if err != nil {
		return fmt.Errorf("read environment file: %w", err)
	}
	hardened := hardenProductionEnvironment(content)
	if err := writeAtomicRegularFile(options.EnvFile, hardened, 0o600); err != nil {
		return fmt.Errorf("write hardened environment: %w", err)
	}

	if dropInInfo, err := os.Lstat(options.DropInDir); err == nil {
		if dropInInfo.Mode()&os.ModeSymlink != 0 || !dropInInfo.IsDir() {
			return errors.New("drop-in directory must be a real directory")
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("inspect drop-in directory: %w", err)
	}
	if err := os.MkdirAll(options.DropInDir, 0o755); err != nil {
		return fmt.Errorf("create drop-in directory: %w", err)
	}
	if err := os.Chmod(options.DropInDir, 0o755); err != nil {
		return fmt.Errorf("set drop-in permissions: %w", err)
	}
	legacyMemoryPath := filepath.Join(options.DropInDir, legacyEmergencyMemoryFile)
	if err := retireLegacyEmergencyMemoryDropIn(legacyMemoryPath); err != nil {
		return err
	}
	memoryPath := filepath.Join(options.DropInDir, productionMemoryFileName)
	if memoryInfo, err := os.Lstat(memoryPath); err == nil && memoryInfo.Mode()&os.ModeSymlink != 0 {
		return errors.New("production memory drop-in must not be a symlink")
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("inspect production memory drop-in: %w", err)
	}
	if err := writeAtomicRegularFile(memoryPath, productionMemoryConfig(), 0o644); err != nil {
		return fmt.Errorf("write production memory drop-in: %w", err)
	}
	return nil
}

func productionMemoryConfig() []byte {
	return []byte(fmt.Sprintf(`[Service]
MemoryAccounting=yes
MemoryHigh=%s
MemoryMax=%s
MemorySwapMax=%s
`, productionMemoryHigh, productionMemoryMax, productionMemorySwapMax))
}

func retireLegacyEmergencyMemoryDropIn(path string) error {
	info, err := os.Lstat(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return fmt.Errorf("inspect legacy emergency memory drop-in: %w", err)
	}
	if info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
		return errors.New("legacy emergency memory drop-in must be a real regular file")
	}
	content, err := os.ReadFile(path)
	if err != nil {
		return fmt.Errorf("read legacy emergency memory drop-in: %w", err)
	}
	for _, expected := range []string{"MemoryHigh=256M", "MemoryMax=288M", "MemorySwapMax=64M"} {
		if !strings.Contains(string(content), expected) {
			return fmt.Errorf("refusing to retire unknown legacy memory drop-in: missing %s", expected)
		}
	}
	retired := path + ".disabled"
	if retiredInfo, statErr := os.Lstat(retired); statErr == nil {
		if retiredInfo.Mode()&os.ModeSymlink != 0 || !retiredInfo.Mode().IsRegular() {
			return errors.New("retired legacy memory drop-in must be a real regular file")
		}
		return fmt.Errorf("retired legacy memory drop-in already exists: %s", retired)
	} else if !errors.Is(statErr, os.ErrNotExist) {
		return fmt.Errorf("inspect retired legacy memory drop-in: %w", statErr)
	}
	if err := os.Rename(path, retired); err != nil {
		return fmt.Errorf("retire legacy emergency memory drop-in: %w", err)
	}
	return nil
}

func hardenProductionEnvironment(content []byte) []byte {
	blocked := map[string]struct{}{
		"SESSION_COOKIE_SECURE":      {},
		"SESSION_COOKIE_TRUSTED_URL": {},
		"TRUSTED_PROXIES":            {},
	}
	lines := strings.Split(strings.ReplaceAll(string(content), "\r\n", "\n"), "\n")
	// EnvironmentFile parsing is last-assignment-wins.  Normalize legacy files
	// to that deterministic representation before the transaction parser checks
	// for duplicates; otherwise a harmless historical duplicate blocks every
	// guarded production backup.
	lastAssignment := make(map[string]int)
	for index, line := range lines {
		trimmed := strings.TrimSpace(line)
		key, _, found := strings.Cut(trimmed, "=")
		if !found {
			continue
		}
		key = strings.TrimSpace(key)
		if _, remove := blocked[key]; remove {
			continue
		}
		lastAssignment[key] = index
	}
	kept := make([]string, 0, len(lines)+3)
	for index, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		key, _, found := strings.Cut(trimmed, "=")
		if found {
			key = strings.TrimSpace(key)
			if _, remove := blocked[key]; remove {
				continue
			}
			if lastAssignment[key] != index {
				continue
			}
		}
		kept = append(kept, line)
	}
	kept = append(kept,
		"SESSION_COOKIE_SECURE=true",
		"SESSION_COOKIE_TRUSTED_URL=https://api.lmm.best,https://lmm.best",
		"TRUSTED_PROXIES=127.0.0.1/32,::1/128",
	)
	return []byte(strings.Join(kept, "\n") + "\n")
}

func writeAtomicRegularFile(path string, content []byte, mode fs.FileMode) (returnErr error) {
	parent := filepath.Dir(path)
	temporary, err := os.CreateTemp(parent, "."+filepath.Base(path)+".*.new")
	if err != nil {
		return err
	}
	temporaryPath := temporary.Name()
	defer func() {
		if returnErr != nil {
			_ = os.Remove(temporaryPath)
		}
	}()
	if err := temporary.Chmod(mode); err != nil {
		_ = temporary.Close()
		return err
	}
	if _, err := temporary.Write(content); err != nil {
		_ = temporary.Close()
		return err
	}
	if err := temporary.Sync(); err != nil {
		_ = temporary.Close()
		return err
	}
	if err := temporary.Close(); err != nil {
		return err
	}
	if err := os.Rename(temporaryPath, path); err != nil {
		return err
	}
	return syncDirectory(parent)
}
