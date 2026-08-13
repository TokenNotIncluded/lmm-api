package appcli

import (
	"bytes"
	"path/filepath"
	"strings"
	"testing"
)

func validProductionReleaseArguments(root string) []string {
	return []string{
		"--repo", filepath.Join(root, "repo"),
		"--workspace", filepath.Join(root, "workspace"),
		"--confirm", "api.lmm.best",
	}
}

func TestParseProductionReleaseRequiresExplicitProductionConfirmation(t *testing.T) {
	arguments := validProductionReleaseArguments(t.TempDir())
	arguments[len(arguments)-1] = "wrong-host"
	_, err := parseProductionReleaseOptions(arguments, &bytes.Buffer{})
	if err == nil || !strings.Contains(err.Error(), "--confirm must equal api.lmm.best") {
		t.Fatalf("confirmation error=%v", err)
	}
}

func TestParseProductionReleaseConstrainsRollbackAndObservationWindows(t *testing.T) {
	tests := []struct {
		name  string
		flags []string
		want  string
	}{
		{name: "short observation", flags: []string{"--observation-seconds", "119"}, want: "between 120 and 360"},
		{name: "long observation", flags: []string{"--observation-seconds", "361"}, want: "between 120 and 360"},
		{name: "short rollback", flags: []string{"--rollback-seconds", "599"}, want: "between 600 and 1800"},
		{name: "long rollback", flags: []string{"--rollback-seconds", "1801"}, want: "between 600 and 1800"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			arguments := append(validProductionReleaseArguments(t.TempDir()), test.flags...)
			_, err := parseProductionReleaseOptions(arguments, &bytes.Buffer{})
			if err == nil || !strings.Contains(err.Error(), test.want) {
				t.Fatalf("window error=%v", err)
			}
		})
	}
}

func TestParseProductionReleaseAcceptsSafeAbsoluteInputs(t *testing.T) {
	root := t.TempDir()
	arguments := append(validProductionReleaseArguments(root),
		"--rollback-package", filepath.Join(root, "rollback.pkg.tar.zst"),
		"--observation-seconds", "240",
		"--rollback-seconds", "900",
		"--manual-confirm",
		"--preserve-edge-policy",
	)
	options, err := parseProductionReleaseOptions(arguments, &bytes.Buffer{})
	if err != nil {
		t.Fatal(err)
	}
	if options.ObservationSeconds != 240 || options.RollbackSeconds != 900 || options.Confirm != "api.lmm.best" ||
		!options.ManualConfirm || !options.PreserveEdgePolicy {
		t.Fatalf("options=%#v", options)
	}
}

func TestParseProductionReleaseRequiresAgeFilesOnlyWithBackups(t *testing.T) {
	arguments := append(validProductionReleaseArguments(t.TempDir()), "--with-backups")
	_, err := parseProductionReleaseOptions(arguments, &bytes.Buffer{})
	if err == nil || !strings.Contains(err.Error(), "required with --with-backups") {
		t.Fatalf("backup input error=%v", err)
	}
}
