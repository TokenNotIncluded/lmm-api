package appcli

import (
	"archive/tar"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

type fakeProductionRunner struct {
	t                        *testing.T
	candidatePackage         string
	rollbackPackage          string
	probeBinary              string
	installedBinary          string
	frontendRoot             string
	candidateFrontendIndex   string
	oldVersion               string
	newVersion               string
	installedVersion         string
	packageName              string
	resolveProvides          bool
	serviceActive            bool
	timerActive              bool
	migrationFailure         bool
	rollbackMigrationFailure bool
	failTimerEnable          bool
	commands                 []productionCommand
}

func (runner *fakeProductionRunner) Run(_ context.Context, command productionCommand) ([]byte, error) {
	runner.commands = append(runner.commands, command)
	base := filepath.Base(command.Name)
	if command.Name == runner.probeBinary || command.Name == runner.installedBinary {
		if len(command.Args) == 0 {
			return nil, errors.New("missing native CLI command")
		}
		switch command.Args[0] {
		case "version":
			if command.Name == runner.probeBinary {
				return []byte(runner.newVersion + "\n"), nil
			}
			return []byte(runner.installedVersion + "\n"), nil
		case "migrate":
			if runner.migrationFailure && len(command.Args) > 1 && command.Args[1] == "--apply" {
				return nil, errors.New("injected migration failure")
			}
			if runner.rollbackMigrationFailure && command.Name == runner.installedBinary {
				return nil, errors.New("injected rollback compatibility failure")
			}
			return nil, nil
		case "request":
			return runner.nativeRequest(command.Args)
		}
	}
	switch base {
	case "bsdtar":
		if len(command.Args) != 3 || command.Args[0] != "-xOf" || command.Args[1] != runner.candidatePackage || command.Args[2] != productionPackagedFrontendIndex {
			return nil, fmt.Errorf("unexpected bsdtar arguments: %v", command.Args)
		}
		return os.ReadFile(runner.candidateFrontendIndex)
	case "pg_restore":
		return []byte("archive ok\n"), nil
	case "pg_dump":
		for _, argument := range command.Args {
			if strings.HasPrefix(argument, "--file=") {
				if err := os.WriteFile(strings.TrimPrefix(argument, "--file="), []byte("postgresql-custom-backup"), 0o600); err != nil {
					runner.t.Fatal(err)
				}
				return nil, nil
			}
		}
		return nil, errors.New("pg_dump output is missing")
	case "psql":
		joined := strings.Join(command.Args, " ")
		if strings.Contains(joined, "current_schema") {
			return []byte("public\n"), nil
		}
		return []byte("abcdefghijklmnopqrstuvwxyz123456\n"), nil
	case "vercmp":
		return []byte("-1\n"), nil
	case "pacman":
		return runner.pacman(command.Args)
	case "systemctl":
		return runner.systemctl(command.Args)
	case "journalctl":
		return nil, nil
	case "age":
		output := ""
		decrypt := false
		for index, argument := range command.Args {
			if argument == "--decrypt" {
				decrypt = true
			}
			if argument == "--output" && index+1 < len(command.Args) {
				output = command.Args[index+1]
			}
		}
		if output == "" || len(command.Args) == 0 {
			return nil, errors.New("age output is missing")
		}
		content, err := os.ReadFile(command.Args[len(command.Args)-1])
		if err != nil {
			return nil, err
		}
		if decrypt {
			content = bytes.TrimPrefix(content, []byte("AGE-ENCRYPTED\n"))
		} else {
			content = append([]byte("AGE-ENCRYPTED\n"), content...)
		}
		if err := os.WriteFile(output, content, 0o600); err != nil {
			return nil, err
		}
		return nil, nil
	default:
		return nil, fmt.Errorf("unexpected command: %s %s", command.Name, strings.Join(command.Args, " "))
	}
}

func (runner *fakeProductionRunner) nativeRequest(args []string) ([]byte, error) {
	value := func(flag string) string {
		for index := range args {
			if args[index] == flag && index+1 < len(args) {
				return args[index+1]
			}
		}
		return ""
	}
	path := value("--path")
	switch path {
	case "/api/status":
		return []byte(fmt.Sprintf(`{"success":true,"ready":true,"data":{"version":%q}}`, runner.installedVersion)), nil
	case "/api/livez":
		if !runner.serviceActive {
			return nil, errors.New("service stopped")
		}
		return []byte(`{"success":true,"live":true}`), nil
	case "/v1/models":
		return []byte(`{"data":[]}`), nil
	case "/":
		return os.ReadFile(filepath.Join(runner.frontendRoot, "current", "index.html"))
	default:
		return nil, fmt.Errorf("unexpected request path %s", path)
	}
}

func (runner *fakeProductionRunner) pacman(args []string) ([]byte, error) {
	if len(args) < 2 {
		return nil, errors.New("invalid pacman arguments")
	}
	switch args[0] {
	case "-Q":
		if args[1] == "lmm-api" {
			return nil, errors.New("package not found")
		}
		if args[1] != runner.packageName && !(runner.resolveProvides && runner.packageName == productionAURPackageName && args[1] == productionSourcePackageName) {
			return nil, errors.New("package not found")
		}
		return []byte(runner.packageName + " " + runner.installedVersion + "-1\n"), nil
	case "-Qp":
		switch args[1] {
		case runner.candidatePackage:
			return []byte(runner.packageName + " " + runner.newVersion + "-1\n"), nil
		case runner.rollbackPackage:
			return []byte(runner.packageName + " " + runner.oldVersion + "-1\n"), nil
		}
		if filepath.Base(args[1]) == filepath.Base(runner.candidatePackage) {
			return []byte(runner.packageName + " " + runner.newVersion + "-1\n"), nil
		}
	case "-Qkk":
		return []byte(runner.packageName + ": 42 total files, 0 altered files\n"), nil
	case "-Qi":
		return []byte("Name : " + runner.packageName + "\nVersion : " + runner.installedVersion + "-1\n"), nil
	case "-U":
		path := args[len(args)-1]
		if path == runner.candidatePackage {
			runner.installedVersion = runner.newVersion
		} else if path == runner.rollbackPackage {
			runner.installedVersion = runner.oldVersion
		} else {
			return nil, errors.New("unknown package")
		}
		return nil, nil
	}
	return nil, fmt.Errorf("unexpected pacman arguments: %v", args)
}

func (runner *fakeProductionRunner) systemctl(args []string) ([]byte, error) {
	if len(args) == 0 {
		return nil, errors.New("missing systemctl action")
	}
	switch args[0] {
	case "is-active":
		unit := args[len(args)-1]
		if strings.HasSuffix(unit, ".timer") {
			if runner.timerActive {
				return []byte("active\n"), nil
			}
			return nil, errors.New("inactive")
		}
		if strings.Contains(unit, "rollback-") {
			return nil, errors.New("inactive")
		}
		if runner.serviceActive {
			return []byte("active\n"), nil
		}
		return nil, errors.New("inactive")
	case "is-enabled":
		return []byte("enabled\n"), nil
	case "stop":
		unit := args[len(args)-1]
		if strings.Contains(unit, "rollback-") {
			return nil, nil
		}
		runner.serviceActive = false
		return nil, nil
	case "enable":
		unit := args[len(args)-1]
		if strings.HasSuffix(unit, ".timer") {
			runner.timerActive = true
			if runner.failTimerEnable {
				return nil, errors.New("injected ambiguous timer enable failure")
			}
		} else {
			runner.serviceActive = true
		}
		return nil, nil
	case "disable":
		runner.timerActive = false
		return nil, nil
	case "daemon-reload", "reset-failed":
		return nil, nil
	case "show":
		property := ""
		for _, argument := range args {
			if strings.HasPrefix(argument, "--property=") {
				property = strings.TrimPrefix(argument, "--property=")
			}
		}
		switch property {
		case "NRestarts":
			return []byte("0\n"), nil
		case "MemoryCurrent":
			return []byte("67108864\n"), nil
		case "MemoryHigh":
			return []byte("335544320\n"), nil
		case "MemoryMax":
			return []byte("402653184\n"), nil
		}
		return []byte("LoadState=loaded\nActiveState=active\nSubState=running\nUnitFileState=enabled\n"), nil
	}
	return nil, fmt.Errorf("unexpected systemctl arguments: %v", args)
}

type productionFixture struct {
	runtime         *productionRuntime
	runner          *fakeProductionRunner
	workspace       productionWorkspace
	options         productionTransactionOptions
	environment     []byte
	oldMemoryDropIn []byte
}

func newProductionFixture(t *testing.T) productionFixture {
	t.Helper()
	root := t.TempDir()
	paths := productionPaths{
		WorkRoot:         filepath.Join(root, "work"),
		BackupRoot:       filepath.Join(root, "backups"),
		GlobalLock:       filepath.Join(root, "run", "deploy.lock"),
		TransactionLock:  filepath.Join(root, "transaction.lock"),
		FrontendRoot:     filepath.Join(root, "frontend"),
		SystemdUnitRoot:  filepath.Join(root, "systemd"),
		ConfigDir:        filepath.Join(root, "etc", "lmm-api-go"),
		DropInDir:        filepath.Join(root, "systemd", "lmm-api.service.d"),
		InstalledBinary:  filepath.Join(root, "usr", "bin", "lmm-api"),
		PackagedFrontend: filepath.Join(root, "usr", "share", "lmm-api-go", "frontend-dist"),
		ReleasePackages:  filepath.Join(root, "var", "lib", "lmm-api-go", "release-packages"),
		PackageCache:     filepath.Join(root, "var", "cache", "pacman", "pkg"),
		RemovedPaths:     []string{filepath.Join(root, "usr", "bin", "lmm-api-go")},
		Service:          productionServiceName,
		ExpectedHost:     productionExpectedHost,
		PublicBaseURL:    "https://api.lmm.best",
		LocalBaseURL:     "http://127.0.0.1:3000",
		JournalUnits:     []string{productionServiceName, "nginx.service"},
	}
	for _, directory := range []string{
		paths.WorkRoot, paths.BackupRoot, filepath.Dir(paths.GlobalLock), paths.SystemdUnitRoot,
		paths.ConfigDir, paths.DropInDir, filepath.Dir(paths.InstalledBinary),
	} {
		if err := os.MkdirAll(directory, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	id := "go-test-20260810T000000Z"
	workspaceRoot := filepath.Join(paths.WorkRoot, id)
	staging := filepath.Join(workspaceRoot, "staging")
	if err := os.MkdirAll(staging, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(workspaceRoot, productionWorkspaceMarker), []byte("format=1\ndeployment_id="+id+"\nrole=target\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(paths.TransactionLock, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(paths.TransactionLock, productionTransactionMarker), []byte("format=1\ndeployment_id="+id+"\nstatus=ACTIVE\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	oldVersion := "0.1.0.r282.g546910cef"
	newVersion := "0.1.1.r300.gabcdef123"
	candidate := filepath.Join(staging, "lmm-api-go-"+newVersion+"-1-x86_64.pkg.tar.zst")
	rollbackPackage := filepath.Join(staging, "lmm-api-go-"+oldVersion+"-1-x86_64.pkg.tar.zst")
	probe := filepath.Join(staging, "lmm-api-go")
	for path, body := range map[string]string{candidate: "candidate", rollbackPackage: "rollback", probe: "probe"} {
		if err := os.WriteFile(path, []byte(body), 0o700); err != nil {
			t.Fatal(err)
		}
	}
	environment := []byte("SQL_DSN=postgres://user:password@127.0.0.1/lmm\nSESSION_COOKIE_SECURE=false\n")
	if err := os.WriteFile(filepath.Join(paths.ConfigDir, "lmm-api-go.env"), environment, 0o600); err != nil {
		t.Fatal(err)
	}
	oldMemory := []byte("[Service]\nMemoryHigh=224M\n")
	if err := os.WriteFile(filepath.Join(paths.DropInDir, productionMemoryFileName), oldMemory, 0o644); err != nil {
		t.Fatal(err)
	}
	oldFrontend := writeFrontendFixture(t, root, "old.111.js", "old")
	if err := executeFrontendDeploy(frontendDeployOptions{Action: "publish", Root: paths.FrontendRoot, Source: oldFrontend, Release: oldVersion, Keep: 3}); err != nil {
		t.Fatal(err)
	}
	newFrontend := writeFrontendFixture(t, root, "new.222.js", "new")
	if err := os.MkdirAll(paths.PackagedFrontend, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := copyFrontendTree(newFrontend, paths.PackagedFrontend); err != nil {
		t.Fatal(err)
	}
	backupDir := filepath.Join(paths.BackupRoot, id)
	if err := writeTestBackupSet(backupDir, environment); err != nil {
		t.Fatal(err)
	}
	runner := &fakeProductionRunner{
		t: t, candidatePackage: candidate, rollbackPackage: rollbackPackage,
		probeBinary: probe, installedBinary: paths.InstalledBinary, frontendRoot: paths.FrontendRoot,
		candidateFrontendIndex: filepath.Join(paths.PackagedFrontend, "index.html"),
		oldVersion:             oldVersion, newVersion: newVersion, installedVersion: oldVersion,
		packageName: productionSourcePackageName, serviceActive: true,
	}
	clock := time.Date(2026, 8, 10, 1, 0, 0, 0, time.UTC)
	runtime := &productionRuntime{
		paths: paths, runner: runner, now: func() time.Time { return clock }, sleep: func(time.Duration) {},
		effectiveUID: func() int { return 0 }, hostname: func() (string, error) { return productionExpectedHost, nil },
		probeAttempts: 1,
	}
	workspace, err := runtime.openWorkspace(workspaceRoot)
	if err != nil {
		t.Fatal(err)
	}
	options := productionTransactionOptions{
		Action: "apply", Workspace: workspaceRoot,
		Package: candidate, PackageSHA256: mustHashFile(t, candidate),
		RollbackPackage: rollbackPackage, RollbackSHA256: mustHashFile(t, rollbackPackage),
		ProbeBinary: probe, ProbeBinarySHA256: mustHashFile(t, probe), ExpectedVersion: newVersion,
		FrontendIndexSHA256: mustHashFile(t, filepath.Join(paths.PackagedFrontend, "index.html")),
		BackupDir:           backupDir, RollbackWindow: 10 * time.Minute, ObservationWindow: 2 * time.Minute,
		ManualConfirm: true, ActivateFrontend: true,
	}
	return productionFixture{runtime: runtime, runner: runner, workspace: workspace, options: options, environment: environment, oldMemoryDropIn: oldMemory}
}

func writeTestBackupSet(root string, environment []byte) error {
	if err := os.MkdirAll(root, 0o700); err != nil {
		return err
	}
	for _, name := range []string{"application.archive", "frontend.archive", "database.archive", "rollback.package"} {
		if err := os.WriteFile(filepath.Join(root, name), []byte(name), 0o600); err != nil {
			return err
		}
	}
	var archive bytes.Buffer
	writer := tar.NewWriter(&archive)
	if err := writer.WriteHeader(&tar.Header{Name: "lmm-api-go/", Typeflag: tar.TypeDir, Mode: 0o700}); err != nil {
		return err
	}
	if err := writer.WriteHeader(&tar.Header{Name: "lmm-api-go/lmm-api-go.env", Typeflag: tar.TypeReg, Mode: 0o600, Size: int64(len(environment))}); err != nil {
		return err
	}
	if _, err := writer.Write(environment); err != nil {
		return err
	}
	if err := writer.Close(); err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(root, "configuration.archive"), archive.Bytes(), 0o600); err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(root, "manifest.env"), []byte("format=1\n"), 0o600); err != nil {
		return err
	}
	attestation, err := json.Marshal(productionBackupAttestation{
		Format: 1, DeploymentID: filepath.Base(root), ControllerDigest: strings.Repeat("c", 64),
		OffhostDigest: strings.Repeat("d", 64), VerifiedUTC: time.Date(2026, 8, 10, 0, 0, 0, 0, time.UTC),
	})
	if err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(root, productionBackupAttestationFilename), append(attestation, '\n'), 0o600); err != nil {
		return err
	}
	var sums strings.Builder
	for _, name := range []string{"application.archive", "frontend.archive", "configuration.archive", "database.archive", "rollback.package"} {
		digest, err := sha256File(filepath.Join(root, name))
		if err != nil {
			return err
		}
		_, _ = fmt.Fprintf(&sums, "%s  %s\n", digest, name)
	}
	return os.WriteFile(filepath.Join(root, "SHA256SUMS"), []byte(sums.String()), 0o600)
}

func mustHashFile(t *testing.T, path string) string {
	t.Helper()
	digest, err := sha256File(path)
	if err != nil {
		t.Fatal(err)
	}
	return digest
}

func TestProductionPackageIdentitySupportsSourceAndAURPackages(t *testing.T) {
	for _, name := range []string{productionSourcePackageName, productionAURPackageName} {
		for _, pkgrel := range []string{"1", "2", "1.1"} {
			t.Run(name+"-"+pkgrel, func(t *testing.T) {
				wantVersion := "0.1.5-" + pkgrel
				gotName, version, identity, err := parseProductionPackageIdentity([]byte(name + " " + wantVersion + "\n"))
				if err != nil || gotName != name || version != wantVersion || identity != name+" "+wantVersion {
					t.Fatalf("identity=(%q, %q, %q) err=%v", gotName, version, identity, err)
				}
				if !productionPackageMatches(version, "0.1.5") {
					t.Fatalf("package version %q did not match release", version)
				}
			})
		}
	}
}

func TestInstalledGoPackageDeduplicatesPacmanProvidesAlias(t *testing.T) {
	fixture := newProductionFixture(t)
	fixture.runner.packageName = productionAURPackageName
	fixture.runner.resolveProvides = true

	name, identity, err := fixture.runtime.installedGoPackage(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	want := productionAURPackageName + " " + fixture.runner.installedVersion + "-1"
	if name != productionAURPackageName || identity != want {
		t.Fatalf("installedGoPackage()=(%q, %q), want (%q, %q)", name, identity, productionAURPackageName, want)
	}
}

func TestPackageIntegritySummaryIsExact(t *testing.T) {
	name := productionAURPackageName
	for _, test := range []struct {
		name   string
		output string
		clean  bool
	}{
		{name: "clean", output: name + ": 42 total files, 0 altered files\n", clean: true},
		{name: "modified backup is package-valid", output: "backup file: " + name + ": /etc/lmm-api-go/lmm-api-go.env (SHA256 checksum mismatch)\n" + name + ": 42 total files, 0 altered files\n", clean: true},
		{name: "ten altered", output: name + ": 42 total files, 10 altered files\n"},
		{name: "wrong package", output: productionSourcePackageName + ": 42 total files, 0 altered files\n"},
		{name: "ambiguous summaries", output: name + ": 42 total files, 0 altered files\n" + name + ": 42 total files, 0 altered files\n"},
		{name: "backup after summary", output: name + ": 42 total files, 0 altered files\nbackup file: " + name + ": /etc/lmm-api-go/lmm-api-go.env (SHA256 checksum mismatch)\n"},
		{name: "other package backup", output: "backup file: " + productionSourcePackageName + ": /etc/lmm-api-go/lmm-api-go.env (SHA256 checksum mismatch)\n" + name + ": 42 total files, 0 altered files\n"},
		{name: "missing total", output: name + ": total files, 0 altered files\n"},
	} {
		t.Run(test.name, func(t *testing.T) {
			if got := packageIntegrityClean([]byte(test.output), name); got != test.clean {
				t.Fatalf("packageIntegrityClean()=%v, want %v for %q", got, test.clean, test.output)
			}
		})
	}
}

func TestNativeBackendUpgradeLeavesIndependentFrontendUntouched(t *testing.T) {
	fixture := newProductionFixture(t)
	fixture.runner.packageName = productionAURPackageName
	fixture.options.ActivateFrontend = false
	fixture.options.FrontendIndexSHA256 = ""

	status, err := fixture.runtime.apply(context.Background(), fixture.workspace, fixture.options)
	if err != nil {
		t.Fatal(err)
	}
	if status.Phase != "AWAITING_CONFIRMATION" {
		t.Fatalf("status=%#v", status)
	}
	if current, err := currentFrontendRelease(fixture.runtime.paths.FrontendRoot); err != nil || current != fixture.runner.oldVersion {
		t.Fatalf("frontend changed during backend-only upgrade: current=%q err=%v", current, err)
	}
	manifest, err := fixture.runtime.readManifest(fixture.workspace)
	if err != nil {
		t.Fatal(err)
	}
	if manifest.PackageName != productionAURPackageName ||
		manifest.PackageIdentity != productionAURPackageName+" "+fixture.runner.newVersion+"-1" ||
		manifest.ActivateFrontend {
		t.Fatalf("manifest=%#v", manifest)
	}
	if manifest.FrontendIndexSHA256 != manifest.OldFrontendIndexSHA256 {
		t.Fatalf("backend-only transaction changed frontend identity: %#v", manifest)
	}
}

func TestNativeMigrationsUseReleaseScopedDisposableDirectories(t *testing.T) {
	fixture := newProductionFixture(t)
	if _, err := fixture.runtime.apply(context.Background(), fixture.workspace, fixture.options); err != nil {
		t.Fatal(err)
	}

	wantRoot := filepath.Join(fixture.workspace.root, "tmp", "migrations")
	want := map[string]struct {
		binary string
		mode   string
	}{
		"candidate-apply":  {binary: fixture.runner.probeBinary, mode: "apply"},
		"candidate-verify": {binary: fixture.runner.probeBinary, mode: "verify"},
		"rollback-verify":  {binary: fixture.runner.installedBinary, mode: "verify"},
	}
	seen := map[string]bool{}
	rollbackVerifyIndex := -1
	installIndex := -1
	for index, command := range fixture.runner.commands {
		if command.Name == "pacman" && len(command.Args) > 0 && command.Args[0] == "-U" && installIndex == -1 {
			installIndex = index
		}
		if len(command.Args) != 2 || command.Args[0] != "migrate" {
			continue
		}
		name := filepath.Base(command.Dir)
		expected, ok := want[name]
		if !ok || command.Name != expected.binary || command.Args[1] != "--"+expected.mode {
			t.Fatalf("unexpected migration %s: %#v", name, command)
		}
		wantDir := filepath.Join(wantRoot, name)
		if command.Dir != wantDir {
			t.Fatalf("migration %s workdir=%q, want %q", name, command.Dir, wantDir)
		}
		if info, err := os.Stat(command.Dir); err != nil || !info.IsDir() || info.Mode().Perm() != 0o700 {
			t.Fatalf("migration %s directory info=%v err=%v", name, info, err)
		}
		seen[name] = true
		if name == "rollback-verify" {
			rollbackVerifyIndex = index
		}
	}
	if len(seen) != len(want) {
		t.Fatalf("migration commands=%v", seen)
	}
	if rollbackVerifyIndex < 0 || installIndex < 0 || rollbackVerifyIndex > installIndex {
		t.Fatalf("rollback verify index=%d package install index=%d", rollbackVerifyIndex, installIndex)
	}
}

func TestNativeProductionApplyAndConfirmOwnReleaseState(t *testing.T) {
	fixture := newProductionFixture(t)
	status, err := fixture.runtime.apply(context.Background(), fixture.workspace, fixture.options)
	if err != nil {
		t.Fatal(err)
	}
	if status.Phase != "AWAITING_CONFIRMATION" || !fixture.runner.timerActive || fixture.runner.installedVersion != fixture.runner.newVersion {
		t.Fatalf("apply status=%#v timer=%t installed=%s", status, fixture.runner.timerActive, fixture.runner.installedVersion)
	}
	if current, err := currentFrontendRelease(fixture.runtime.paths.FrontendRoot); err != nil || current != fixture.runner.newVersion {
		t.Fatalf("frontend current=%q err=%v", current, err)
	}
	memory, err := os.ReadFile(filepath.Join(fixture.runtime.paths.DropInDir, productionMemoryFileName))
	if err != nil || !strings.Contains(string(memory), "MemoryHigh="+productionMemoryHigh) ||
		!strings.Contains(string(memory), "Environment=GOMEMLIMIT="+productionGoMemoryLimit) {
		t.Fatalf("hardened memory=%q err=%v", memory, err)
	}
	confirmed, err := fixture.runtime.confirm(context.Background(), fixture.workspace)
	if err != nil {
		t.Fatal(err)
	}
	if confirmed.Phase != "CONFIRMED" || fixture.runner.timerActive {
		t.Fatalf("confirm status=%#v timer=%t", confirmed, fixture.runner.timerActive)
	}
	for _, path := range []string{fixture.workspace.timerPath, fixture.workspace.rollbackPath, fixture.workspace.probeToken, fixture.runtime.paths.TransactionLock} {
		if _, err := os.Lstat(path); !errors.Is(err, os.ErrNotExist) {
			t.Errorf("confirmed deployment retained %s: %v", path, err)
		}
	}
	confirmedAgain, err := fixture.runtime.confirm(context.Background(), fixture.workspace)
	if err != nil || confirmedAgain.Phase != "CONFIRMED" {
		t.Fatalf("idempotent confirm status=%#v err=%v", confirmedAgain, err)
	}
	preserved := filepath.Join(fixture.runtime.paths.ReleasePackages, filepath.Base(fixture.options.Package))
	if digest, err := sha256File(preserved); err != nil || digest != fixture.options.PackageSHA256 {
		t.Fatalf("preserved package digest=%q err=%v", digest, err)
	}
	currentPackage, err := fixture.runtime.currentPackage(context.Background())
	if err != nil || currentPackage.Package != preserved || currentPackage.PackageSHA256 != fixture.options.PackageSHA256 {
		t.Fatalf("current package=%#v err=%v", currentPackage, err)
	}
}

func TestNativeProductionFailureAfterArmingRestoresDirectGoRelease(t *testing.T) {
	fixture := newProductionFixture(t)
	fixture.runner.migrationFailure = true
	_, err := fixture.runtime.apply(context.Background(), fixture.workspace, fixture.options)
	if err == nil || !strings.Contains(err.Error(), "migration candidate-apply") {
		t.Fatalf("apply error=%v", err)
	}
	status, statusErr := fixture.runtime.readStatus(fixture.workspace)
	if statusErr != nil {
		t.Fatal(statusErr)
	}
	if status.Phase != "ROLLED_BACK" || fixture.runner.timerActive || fixture.runner.installedVersion != fixture.runner.oldVersion {
		t.Fatalf("rollback status=%#v timer=%t installed=%s", status, fixture.runner.timerActive, fixture.runner.installedVersion)
	}
	if current, err := currentFrontendRelease(fixture.runtime.paths.FrontendRoot); err != nil || current != fixture.runner.oldVersion {
		t.Fatalf("frontend current=%q err=%v", current, err)
	}
	environment, err := os.ReadFile(filepath.Join(fixture.runtime.paths.ConfigDir, "lmm-api-go.env"))
	if err != nil || !bytes.Equal(environment, fixture.environment) {
		t.Fatalf("restored environment=%q err=%v", environment, err)
	}
	memory, err := os.ReadFile(filepath.Join(fixture.runtime.paths.DropInDir, productionMemoryFileName))
	if err != nil || !bytes.Equal(memory, fixture.oldMemoryDropIn) {
		t.Fatalf("restored memory=%q err=%v", memory, err)
	}
	if _, err := os.Lstat(fixture.runtime.paths.TransactionLock); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("transaction lock was not released: %v", err)
	}
}

func TestNativeProductionBlocksSchemaThatPreviousReleaseCannotRead(t *testing.T) {
	fixture := newProductionFixture(t)
	fixture.runner.rollbackMigrationFailure = true
	_, err := fixture.runtime.apply(context.Background(), fixture.workspace, fixture.options)
	if err == nil || !strings.Contains(err.Error(), "migration rollback-verify") {
		t.Fatalf("apply error=%v", err)
	}
	status, statusErr := fixture.runtime.readStatus(fixture.workspace)
	if statusErr != nil {
		t.Fatal(statusErr)
	}
	if status.Phase != "ROLLED_BACK" || fixture.runner.installedVersion != fixture.runner.oldVersion {
		t.Fatalf("rollback status=%#v installed=%s", status, fixture.runner.installedVersion)
	}
	for _, command := range fixture.runner.commands {
		if command.Name == "pacman" && len(command.Args) > 0 && command.Args[0] == "-U" &&
			command.Args[len(command.Args)-1] == fixture.options.Package {
			t.Fatalf("candidate package was installed after rollback compatibility failed: %v", command.Args)
		}
	}
}

func TestNativeProductionRollbackDoesNotStopItsOwnService(t *testing.T) {
	fixture := newProductionFixture(t)
	if _, err := fixture.runtime.apply(context.Background(), fixture.workspace, fixture.options); err != nil {
		t.Fatal(err)
	}
	if _, err := fixture.runtime.rollback(context.Background(), fixture.workspace, "watchdog-deadline"); err != nil {
		t.Fatal(err)
	}
	status, err := fixture.runtime.readStatus(fixture.workspace)
	if err != nil {
		t.Fatal(err)
	}
	if status.Phase != "ROLLED_BACK" {
		t.Fatalf("rollback status=%#v", status)
	}
	for _, command := range fixture.runner.commands {
		if len(command.Args) >= 2 && command.Name == "systemctl" && command.Args[0] == "stop" && strings.Contains(command.Args[len(command.Args)-1], "rollback-") {
			t.Fatalf("rollback attempted to stop its own service: %v", command.Args)
		}
	}
}

func TestNativeProductionAmbiguousTimerArmFailureRollsBackAndReleasesLock(t *testing.T) {
	fixture := newProductionFixture(t)
	fixture.runner.failTimerEnable = true
	_, err := fixture.runtime.apply(context.Background(), fixture.workspace, fixture.options)
	if err == nil || !strings.Contains(err.Error(), "arm rollback timer") {
		t.Fatalf("apply error=%v", err)
	}
	status, statusErr := fixture.runtime.readStatus(fixture.workspace)
	if statusErr != nil {
		t.Fatal(statusErr)
	}
	if status.Phase != "ROLLED_BACK" || fixture.runner.timerActive {
		t.Fatalf("status=%#v timer=%t", status, fixture.runner.timerActive)
	}
	if fixture.runner.installedVersion != fixture.runner.oldVersion {
		t.Fatalf("installed=%s want=%s", fixture.runner.installedVersion, fixture.runner.oldVersion)
	}
	if _, err := os.Lstat(fixture.runtime.paths.TransactionLock); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("transaction lock was not released: %v", err)
	}
}
