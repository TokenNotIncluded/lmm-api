package assistant

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"os"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	_ "github.com/lib/pq"
)

// This is a real storage component test on a newly allocated runner database.
// Its synthetic receipts are not Core, provider, authorization or funds proof.
func TestAgentPostgresJournalExactBytesAndImmutableRecovery(t *testing.T) {
	path := os.Getenv("LMM_ASSISTANT_AGENT_PG_RUNTIME_FILE")
	if path == "" {
		t.Skip("fresh private runner runtime URL file required")
	}
	info, err := os.Lstat(path)
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm()&0077 != 0 {
		t.Fatal("private regular runtime URL file required")
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal("runtime URL unavailable")
	}
	db, err := sql.Open("postgres", strings.TrimSpace(string(raw)))
	if err != nil {
		t.Fatal("runtime connection failed")
	}
	defer db.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	var database string
	if db.QueryRowContext(ctx, `SELECT current_database()`).Scan(&database) != nil || !strings.HasPrefix(database, "a09_") {
		t.Fatal("fresh assistant runner database required")
	}
	journal, err := NewAgentPostgresJournal(ctx, db, database)
	if err != nil {
		t.Fatal(err)
	}
	owner := time.Now().UnixNano()%1_000_000_000 + 1_000_000
	var conversation int64
	if err = db.QueryRowContext(ctx, `INSERT INTO lmm_assistant.conversations(user_id,title,last_message_preview,created_at,updated_at) VALUES($1,'Storage component','Pending',1,1) RETURNING id`, owner).Scan(&conversation); err != nil {
		t.Fatal("conversation insert failed")
	}
	const turnID = "storage-roundtrip"
	coreID := "lmm_cv_" + agentDigest([]byte("storage-component:"+strconv.FormatInt(conversation, 10)))
	args := json.RawMessage("{ \"expression\" : \"1 + 2\" }")
	result := json.RawMessage("{ \"value\" : 3, \"ok\" : true }")
	call := AgentToolCall{OperationID: "storage-tool", CoreConversationID: coreID, TurnID: turnID, ModelOperationID: "storage-model", ProviderCallID: "storage-provider", Scope: AgentToolScope{Name: "calculate_math", Version: FixedMainToolVersion, SchemaSHA256: agentDigest([]byte(`{}`)), PriceVersion: BuiltinToolPriceVersion, PriceSHA256: FreeAgentPriceSHA256(), Domain: "extension"}, Arguments: args, ArgumentsSHA256: agentDigest(args)}
	err = journal.WithinAgent(ctx, owner, conversation, turnID, func(local *AgentLocalState) error {
		local.Mapping = &DelegationConversationMapping{CreationKey: strings.Repeat("d", 32), CoreConversationID: coreID, Phase: "registered"}
		local.Turn = &AgentStoredTurn{ID: turnID, Input: "storage component", InputSHA256: agentDigest([]byte("storage component")), Phase: "started", Snapshot: &AgentTurnSnapshot{Key: AgentTurnKey{coreID, turnID}, OwnerUserID: owner, CoreOwnerAccountID: 1, Account: access.Account{Kind: "personal", ID: owner}, OriginalTextSHA256: agentDigest([]byte("storage component")), Revision: 1, MaxRounds: 2, Phase: AgentTurnAwaitingTools, BudgetCredits: 1, PendingTools: []AgentToolCall{call}}, Tools: map[string]*AgentStoredTool{call.OperationID: {Call: call, ClaimKey: strings.Repeat("e", 32), Phase: "result_stored", Result: &AgentToolResult{OperationID: call.OperationID, State: AgentToolSucceeded, ResultJSON: result, ResultSHA256: agentDigest(result), DomainReceiptID: "storage-component-only"}}}}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	// A separate pool/journal models a restarted Go process, not an object cache.
	second, err := sql.Open("postgres", strings.TrimSpace(string(raw)))
	if err != nil {
		t.Fatal("second runtime connection failed")
	}
	defer second.Close()
	restarted, err := NewAgentPostgresJournal(ctx, second, database)
	if err != nil {
		t.Fatal(err)
	}
	err = restarted.WithinAgent(ctx, owner, conversation, turnID, func(local *AgentLocalState) error {
		tool := local.Turn.Tools[call.OperationID]
		if !bytes.Equal(tool.Call.Arguments, args) || !bytes.Equal(tool.Result.ResultJSON, result) || !bytes.Equal(local.Turn.Snapshot.PendingTools[0].Arguments, args) {
			t.Fatal("restart normalized hashed JSON")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if err = restarted.WithinAgent(ctx, owner, conversation, turnID, func(local *AgentLocalState) error { local.Turn.Input = "replacement"; return nil }); !errors.Is(err, access.ErrConflict) {
		t.Fatalf("immutable input accepted: %v", err)
	}
	if err = restarted.WithinAgent(ctx, owner+1, conversation, turnID, func(*AgentLocalState) error { return nil }); !errors.Is(err, access.ErrNotFound) {
		t.Fatalf("other owner accepted: %v", err)
	}
}
