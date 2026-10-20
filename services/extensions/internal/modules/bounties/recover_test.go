package bounties

import (
	"context"
	"testing"
)

type recoveryAuthority struct {
	*authority
	commands map[string]Command
	receipts map[string]Receipt
	queries  int
	previews int
}

func (a *recoveryAuthority) Preview(ctx context.Context, session string, c Command) (Receipt, error) {
	a.previews++
	return a.authority.Preview(ctx, session, c)
}
func (a *recoveryAuthority) Execute(ctx context.Context, session string, c Command) (Receipt, error) {
	r, e := a.authority.Execute(ctx, session, c)
	if e != nil {
		return r, e
	}
	r.ProjectRevision = int64(a.calls)
	r.PublishedAt = 100
	r.EscrowCredit = 1000 - int64(a.calls)*10
	if a.commands == nil {
		a.commands = map[string]Command{}
		a.receipts = map[string]Receipt{}
	}
	c.Confirmation = ""
	a.commands[c.OperationID] = c
	a.receipts[c.OperationID] = r
	return r, nil
}
func (a *recoveryAuthority) QueryForCommand(_ context.Context, _ string, c Command) (Receipt, error) {
	a.queries++
	saved, ok := a.commands[c.OperationID]
	if !ok {
		return Receipt{}, ErrNotFound
	}
	c.Confirmation = ""
	if saved != c {
		return Receipt{}, ErrConflict
	}
	return a.receipts[c.OperationID], nil
}
func TestRecoveryUsesExactSavedCommandAndNeverRepeatsAction(t *testing.T) {
	s, base := setup(t)
	a := &recoveryAuthority{authority: base}
	s.Funds = a
	invoke(t, s, "owner-token", "create_draft", "draft")
	in := Invocation{Command: Command{Action: "publish", OperationID: "publish", ProjectID: "p", Confirmation: "browser-confirmed"}}
	a.Execute(context.Background(), "owner-token", Command{Action: "publish", OperationID: "publish", ProjectID: "p", Confirmation: "browser-confirmed", RewardCredit: 100, RewardSlots: 2, RepositoryURL: "https://github.com/owner/repo"})
	out, e := s.RecoverOperation(context.Background(), "owner-token", in)
	if e != nil || out.Project.Status != "published" || a.calls != 1 || a.queries != 1 || a.previews != 0 {
		t.Fatalf("%+v %v", out, e)
	}
	out, e = s.RecoverOperation(context.Background(), "owner-token", in)
	if e != nil || out.Project.Status != "published" || a.calls != 1 || a.queries != 1 {
		t.Fatalf("%+v %v", out, e)
	}
	in.Command.Credit = 99
	if _, e = s.RecoverOperation(context.Background(), "owner-token", in); e != ErrConflict {
		t.Fatal(e)
	}
}
func TestPendingRecoveryDoesNotProjectOrInventOperation(t *testing.T) {
	for _, state := range []string{"awaiting_confirmation", "confirmed", "confirmation_expired"} {
		t.Run(state, func(t *testing.T) {
			s, base := setup(t)
			a := &recoveryAuthority{authority: base, commands: map[string]Command{}, receipts: map[string]Receipt{}}
			s.Funds = a
			invoke(t, s, "owner-token", "create_draft", "draft")
			cmd := Command{Action: "publish", OperationID: "pending", ProjectID: "p", RewardCredit: 100, RewardSlots: 2, RepositoryURL: "https://github.com/owner/repo"}
			a.commands[cmd.OperationID] = cmd
			a.receipts[cmd.OperationID] = Receipt{OperationID: "pending", ActorUserID: "owner", ProjectID: "p", State: state}
			out, e := s.RecoverOperation(context.Background(), "owner-token", Invocation{Command: cmd})
			if e != nil || out.Receipt.State != state {
				t.Fatal(out, e)
			}
			out, e = invoke(t, s, "owner-token", "get", "")
			if e != nil || out.Project.Status != "draft" {
				t.Fatal(out, e)
			}
			s.Store.Within(context.Background(), func(tx Tx) error {
				var op operation
				if e := tx.Get("operations", "pending", &op); e != ErrNotFound {
					t.Fatalf("pending persisted as complete: %v", e)
				}
				return nil
			})
			if a.calls != 0 || a.previews != 0 {
				t.Fatal("recovery issued action")
			}
		})
	}
}

func TestOldDisputeRecoveryPreservesNewerEntityAndRecordsOperation(t *testing.T) {
	s, base := setup(t)
	a := &recoveryAuthority{authority: base, commands: map[string]Command{}, receipts: map[string]Receipt{}}
	s.Funds = a
	ctx := context.Background()
	cmd := Command{Action: "open_dispute", OperationID: "old-open", ProjectID: "p", ChallengeID: "ch", DisputeID: "d", Reason: "other", Statement: "a sufficiently detailed statement"}
	a.commands[cmd.OperationID] = cmd
	a.receipts[cmd.OperationID] = Receipt{OperationID: cmd.OperationID, ProjectID: "p", ChallengeID: "ch", DisputeID: "d", ActorUserID: "worker", State: "open", Status: "published", ProjectRevision: 2, EscrowCredit: 100, PublishedAt: 100}
	s.Store.Within(ctx, func(tx Tx) error {
		tx.Put("projects", "p", Project{ID: "p", OwnerUserID: "owner", Status: "completed", CoreRevision: 4, EscrowCredit: 0, PublishedAt: 100})
		tx.Put("challenges", "ch", Challenge{ID: "ch", ProjectID: "p", ParticipantUserID: "worker", Status: "approved", CoreRevision: 4})
		return tx.Put("disputes", "d", Dispute{ID: "d", ProjectID: "p", ChallengeID: "ch", OpenedByUserID: "worker", Status: "resolved_paid", CoreRevision: 4})
	})
	out, e := s.RecoverOperation(ctx, "worker-token", Invocation{Command: cmd})
	if e != nil || out.Dispute.Status != "resolved_paid" || out.Dispute.CoreRevision != 4 {
		t.Fatal(out, e)
	}
	s.Store.Within(ctx, func(tx Tx) error {
		var p Project
		tx.Get("projects", "p", &p)
		if p.CoreRevision != 4 || p.Status != "completed" || p.EscrowCredit != 0 {
			t.Fatal("older dispute reverted project", p)
		}
		var op operation
		if e := tx.Get("operations", cmd.OperationID, &op); e != nil {
			t.Fatal(e)
		}
		return nil
	})
	if a.calls != 0 || a.previews != 0 || a.queries != 1 {
		t.Fatal("recovery issued action")
	}
}
