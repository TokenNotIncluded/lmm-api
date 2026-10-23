package personalization

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	services "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant_services"
	_ "github.com/lib/pq"
)

// This authority cannot authenticate anybody. Constructors and installed
// binding projection require no identity/effect calls; this is deliberately
// not evidence of real Core Session, model, business or ledger acceptance.
type installedProjectionRejectAll struct{ calls int }

func (a *installedProjectionRejectAll) Check(context.Context, string, access.Account, access.Permission) (access.Principal, error) {
	a.calls++
	return access.Principal{}, access.ErrUnauthorized
}
func installedProjectionWrite(t *testing.T, path string, value any) {
	t.Helper()
	if !filepath.IsAbs(path) || filepath.Clean(path) != path {
		t.Fatal("absolute artifact path required")
	}
	parent, err := os.Lstat(filepath.Dir(path))
	if err != nil || !parent.IsDir() || parent.Mode().Perm()&0077 != 0 {
		t.Fatal("private artifact parent required")
	}
	raw, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		t.Fatal("cannot serialize public projection")
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		t.Fatal("new artifact filename required")
	}
	defer file.Close()
	if _, err = file.Write(append(raw, '\n')); err != nil {
		t.Fatal("cannot publish projection")
	}
	if err = file.Sync(); err != nil {
		t.Fatal("cannot sync projection")
	}
}

func TestInstalledBasicBindingProjectionPG(t *testing.T) {
	credentialFile := os.Getenv("LMM_ASSISTANT_INSTALLED_PG_RUNTIME_FILE")
	output := os.Getenv("LMM_ASSISTANT_INSTALLED_DESCRIPTORS_FILE")
	if credentialFile == "" && output == "" {
		t.Skip("fresh isolated installed-projection PostgreSQL fixture required")
	}
	if credentialFile == "" || output == "" {
		t.Fatal("both private fixture and artifact paths required")
	}
	info, err := os.Lstat(credentialFile)
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 || info.Size() > 16384 {
		t.Fatal("bounded private credential file required")
	}
	raw, err := os.ReadFile(credentialFile)
	if err != nil {
		t.Fatal("private fixture credential unavailable")
	}
	db, err := sql.Open("postgres", strings.TrimSpace(string(raw)))
	if err != nil {
		t.Fatal("cannot open isolated PostgreSQL fixture")
	}
	defer db.Close()
	db.SetMaxOpenConns(2)
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	var database string
	if err = db.QueryRowContext(ctx, `SELECT current_database()`).Scan(&database); err != nil || !strings.HasPrefix(database, "a09_") {
		t.Fatal("new a09 fixture database required")
	}
	authority := &installedProjectionRejectAll{}
	domain, err := services.New(ctx, db, authority, services.Dependencies{})
	if err != nil {
		t.Fatal("installed service contract or precise runtime privileges rejected")
	}
	repository, err := assistant.NewPostgres(ctx, db, database)
	if err != nil {
		t.Fatal("installed assistant transcript repository rejected")
	}
	history, err := assistant.NewHistory(repository, authority, []byte(strings.Repeat("projection-fixture-cursor-key-", 2)[:32]))
	if err != nil {
		t.Fatal("cannot construct history service")
	}
	personal, err := New(domain, history)
	if err != nil {
		t.Fatal("cannot construct installed personalization adapters")
	}
	bindings := append(personal.Domain().AvailableBindings(), personal.AvailableBindings()...)
	registry, err := assistant.NewAgentRegistry(bindings)
	if err != nil {
		t.Fatal("installed subset registry rejected")
	}
	descriptors := registry.Descriptors()
	expected := map[string]bool{
		"calculate_math": true, "show_chart": true, "show_statistics": true, "show_choices": true, "show_flowchart": true,
		"recall_memory": true, "remember_memory": true, "remember_profile_skill": true, "forget_profile_skill": true,
		"get_site_issues": true, "get_overview_greeting": true, "set_conversation_title": true,
	}
	if len(descriptors) != len(expected) {
		t.Fatalf("installed basic subset count=%d, expected=%d", len(descriptors), len(expected))
	}
	type projectedDescriptor struct {
		assistant.AgentToolDescriptor
		SchemaJSON string `json:"schema_json"`
	}
	projection := make([]projectedDescriptor, 0, len(descriptors))
	names := make([]string, 0, len(descriptors))
	for _, descriptor := range descriptors {
		if !expected[descriptor.Name] {
			t.Fatalf("unexpected or duplicate installed tool: %s", descriptor.Name)
		}
		delete(expected, descriptor.Name)
		names = append(names, descriptor.Name)
		projection = append(projection, projectedDescriptor{AgentToolDescriptor: descriptor, SchemaJSON: string(descriptor.Parameters)})
		sum := sha256.Sum256(descriptor.Parameters)
		if hex.EncodeToString(sum[:]) != descriptor.SchemaSHA256 || descriptor.Domain != "extension" {
			t.Fatal("descriptor schema binding lost")
		}
	}
	if len(expected) != 0 || authority.calls != 0 {
		t.Fatal("projection omitted expected code or attempted authentication/business execution")
	}
	installedProjectionWrite(t, output, projection)
	artifact, err := os.ReadFile(output)
	if err != nil {
		t.Fatal("cannot read back public projection")
	}
	sum := sha256.Sum256(artifact)
	installedProjectionWrite(t, output+".receipt.json", map[string]any{
		"status":           "INSTALLED_BINDING_PROJECTION_ONLY",
		"descriptor_count": len(descriptors), "names": names, "artifact_sha256": hex.EncodeToString(sum[:]),
		"authority": "reject_all_no_calls", "authority_call_count": authority.calls,
		"real_postgres_contract_and_privileges": true, "business_acceptance_passed": false,
		"limits": []string{"Real isolated installed PostgreSQL and precise runtime privileges validated by production constructors.", "Only public installed schema/metadata projected; no Core identity, model, business, browser confirmation or ledger effect executed."},
	})
}
