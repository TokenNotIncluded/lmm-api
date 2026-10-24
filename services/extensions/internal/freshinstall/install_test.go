package freshinstall

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCompiledPlansAreClosedAndTransactionSafe(t *testing.T) {
	for _, component := range []string{"assistant", "toolmarket", "toolmarket-builtins", "payments"} {
		p, e := compiled(component)
		if e != nil || p.schema == "" || len(p.grants) == 0 {
			t.Fatalf("plan %s: %v", component, e)
		}
		if strings.Contains(p.schema, "\nBEGIN;") || strings.Contains(p.schema, "\nCOMMIT;") {
			t.Fatalf("nested transaction in %s", component)
		}
	}
	for _, component := range []string{"", "core", "wallet", "../schema.sql", "toolmarket,assistant"} {
		if _, e := compiled(component); e == nil {
			t.Fatalf("accepted %q", component)
		}
	}
	p, _ := compiled("assistant")
	for _, table := range []string{"delegation_contract", "agent_contract", "services_contract"} {
		if !strings.Contains(p.schema, table) {
			t.Fatal(table)
		}
	}
}
func TestOwnerURLCannotOverrideCheckedDatabaseOrRole(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "owner-url")
	p, _ := compiled("toolmarket")
	c := Config{Component: "toolmarket", OwnerURLFile: file, ExpectedDatabase: "lmm_toolmarket_new", OwnerRole: "module_owner", RuntimeRole: "module_runtime"}
	for _, test := range []struct {
		raw string
		ok  bool
	}{
		{"postgres://installer:private@127.0.0.1:55555/lmm_toolmarket_new?sslmode=verify-full&sslrootcert=%2Fetc%2Fssl%2Fcerts%2Fca-certificates.crt", true},
		{"postgres://installer:private@127.0.0.1:55555/lmm_toolmarket_new?sslmode=disable", true},
		{"postgresql://installer:private@/lmm_toolmarket_new?host=%2Frun%2Flmm-postgresql&port=5433&sslmode=disable", true},
		{"postgresql://installer:private@127.0.0.1/lmm_toolmarket_new?host=%2Frun%2Flmm-postgresql&sslmode=require", false},
		{"postgres://installer:private@192.0.2.10:55555/lmm_toolmarket_new?sslmode=disable", false},
		{"postgres://installer:private@127.0.0.1:55555/lmm_toolmarket_new?sslmode=verify-full", false},
		{"postgresql://installer:private@/lmm_core_new?host=%2Frun%2Flmm-postgresql&sslmode=disable", false},
		{"postgresql://installer@/lmm_toolmarket_new?host=%2Frun%2Flmm-postgresql&sslmode=disable", false},
		{"postgres://installer:private@127.0.0.1:55555/lmm_toolmarket_new?options=-crole%3Dsuperuser", false},
		{"postgres://installer:private@127.0.0.1:55555/lmm_core_new", false},
		{"postgres://installer:private@127.0.0.1:55555/lmm_toolmarket_new?sslmode=disable&sslmode=require", false},
	} {
		if os.WriteFile(file, []byte(test.raw), 0600) != nil {
			t.Fatal("file")
		}
		got, err := ownerURL(c, p)
		if (err == nil) != test.ok {
			t.Fatal("unexpected owner URL acceptance")
		}
		if test.ok && got != test.raw {
			t.Fatal("owner URL changed")
		}
	}

	c.OwnerRole = c.RuntimeRole
	if _, e := ownerURL(c, p); e == nil {
		t.Fatal("runtime as owner")
	}
}
func TestCLIRejectsArbitrarySQLAndRuntimeConfiguration(t *testing.T) {
	for _, args := range [][]string{{"--sql-file", "/tmp/schema.sql"}, {"--config", "runtime.toml"}, {"--component", "core"}, {"--owner-url-file", "secret", "extra"}} {
		var out bytes.Buffer
		if RunCLI(context.Background(), args, &out) == nil || out.Len() != 0 {
			t.Fatal("unexpected CLI success/output")
		}
	}
}
