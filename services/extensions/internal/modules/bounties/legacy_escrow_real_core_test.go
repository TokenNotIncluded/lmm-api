package bounties

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"
)

// Opt in only after seed_go_interop_sessions has populated a fresh owned PG.
// No HTTP stub is installed: the socket must belong to a separately started Core.
func TestLegacyEscrowRealCoreSyntheticFixture(t *testing.T) {
	root := os.Getenv("LMM_ESCROW_OUTPUT")
	if root == "" {
		t.Skip("requires owned synthetic fixture, started Core, and LMM_ESCROW_OUTPUT")
	}
	if !filepath.IsAbs(root) {
		t.Fatal("fixture root must be absolute")
	}
	resolved, err := filepath.EvalSymlinks(root)
	if err != nil || resolved != root {
		t.Fatal("fixture root must not contain symlinks")
	}
	info, err := os.Lstat(root)
	if err != nil || !info.IsDir() || info.Mode().Perm() != 0700 {
		t.Fatal("fixture root must be private 0700")
	}
	read := func(name string) []byte {
		path := filepath.Join(root, name)
		info, err := os.Lstat(path)
		if err != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 {
			t.Fatalf("private regular fixture file required: %s", name)
		}
		raw, err := os.ReadFile(path)
		if err != nil {
			t.Fatalf("cannot read %s", name)
		}
		return raw
	}
	var fixture struct {
		Format      string `json:"format"`
		Migration   string `json:"migration_id"`
		Project     int64  `json:"source_project_id"`
		Claim       int64  `json:"source_challenge_id"`
		Owner       string `json:"owner"`
		Participant string `json:"participant"`
		Stranger    string `json:"stranger"`
		APIKey      string `json:"participant_api_key"`
		Payer       int64  `json:"payer_account_id"`
		Recipient   int64  `json:"recipient_account_id"`
	}
	if json.Unmarshal(read("go-interop.private.json"), &fixture) != nil || fixture.Format != "synthetic-go-escrow-v1" || fixture.Migration != "go-core-interop-v1" || fixture.Project != 9007199254740993 || fixture.Claim != 9007199254740995 || fixture.Owner == "" || fixture.Participant == "" || fixture.Stranger == "" || fixture.APIKey == "" || fixture.Owner == fixture.Participant {
		t.Fatal("invalid synthetic fixture")
	}
	socket := os.Getenv("LMM_ESCROW_CORE_SOCKET")
	token := strings.TrimSpace(string(read("bounties.token")))
	client, err := NewCoreClient(socket, token)
	if err != nil {
		t.Fatal("invalid Core fixture client")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	submit := LegacyEscrowSubmitCommand{MigrationID: fixture.Migration, SourceProjectID: fixture.Project, SourceChallengeID: fixture.Claim, OperationKey: "go-submit", PullRequestURL: "https://github.com/synthetic/rehearsal/pull/1", SubmissionNote: "typed Go to real Core"}
	for _, credential := range []string{fixture.Owner, fixture.Stranger, fixture.APIKey} {
		if _, err := client.SubmitLegacyEscrow(ctx, credential, submit); !errors.Is(err, ErrForbidden) {
			t.Fatal("non-participant Session/API key must be forbidden")
		}
	}
	var submitted [2]LegacyEscrowReceipt
	var failures [2]error
	var wg sync.WaitGroup
	for i := range submitted {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			submitted[i], failures[i] = client.SubmitLegacyEscrow(ctx, fixture.Participant, submit)
		}(i)
	}
	wg.Wait()
	for i := range submitted {
		if failures[i] != nil || submitted[i].Status != "submitted" || submitted[i].JournalID != nil {
			t.Fatal("real participant submit failed")
		}
	}
	if !reflect.DeepEqual(submitted[0], submitted[1]) {
		t.Fatal("duplicate submit receipt changed")
	}
	collision := submit
	collision.PullRequestURL = "https://github.com/synthetic/rehearsal/pull/2"
	if _, err := client.SubmitLegacyEscrow(ctx, fixture.Participant, collision); !errors.Is(err, ErrConflict) {
		t.Fatal("changed command must conflict")
	}
	payout := LegacyEscrowPayoutCommand{MigrationID: fixture.Migration, SourceProjectID: fixture.Project, SourceChallengeID: fixture.Claim, OperationKey: "go-payout", SubmissionOperationKey: &submit.OperationKey}
	for _, credential := range []string{fixture.Participant, fixture.Stranger, fixture.APIKey} {
		if _, err := client.PayoutLegacyEscrow(ctx, credential, payout); !errors.Is(err, ErrForbidden) {
			t.Fatal("non-owner Session/API key must be forbidden")
		}
	}
	paid, err := client.PayoutLegacyEscrow(ctx, fixture.Owner, payout)
	if err != nil || paid.Status != "posted" || paid.JournalID == nil || paid.RemainingUnits != 13 {
		t.Fatal("owner payout failed")
	}
	replay, err := client.PayoutLegacyEscrow(ctx, fixture.Owner, payout)
	if err != nil || !reflect.DeepEqual(paid, replay) {
		t.Fatal("payout replay changed receipt")
	}
	observed, err := client.QueryLegacyEscrowForCommand(ctx, fixture.Owner, payout)
	if err != nil || observed.ActorUserID != 101 || !reflect.DeepEqual(observed.Receipt, paid) {
		t.Fatal("payout exact query failed")
	}
	closed, err := client.CloseLegacyEscrow(ctx, fixture.Owner, LegacyEscrowCloseCommand{MigrationID: fixture.Migration, SourceProjectID: fixture.Project, OperationKey: "go-close"})
	if err != nil || closed.Status != "posted" || closed.RemainingUnits != 0 || closed.JournalID == nil {
		t.Fatal("close did not release remainder")
	}
	t.Log("Go -> separately started Core -> PG commands passed; run verify_go_interop_ledger for independent durable readback")
}
