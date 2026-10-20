package bounties

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestRecordsArchiveContentAndAnonymousPrivacy(t *testing.T) {
	s, _ := setup(t)
	ctx := context.Background()
	invoke(t, s, "owner-token", "create_draft", "create")
	if _, e := s.Detail(ctx, "", "p"); e != ErrNotFound {
		t.Fatal("anonymous draft exposed", e)
	}
	invoke(t, s, "owner-token", "publish", "publish")
	content := draft()
	content.Title = "Updated title"
	content.RewardCredit = 999
	content.RepositoryURL = "https://github.com/attacker/repo"
	out, e := s.Records(ctx, "owner-token", RecordRequest{Action: "content", ProjectID: "p", Draft: content})
	if e != nil || out.Project.RewardCredit != 100 || out.Project.RepositoryURL != "https://github.com/owner/repo" || out.Project.Title != "Updated title" {
		t.Fatal(out, e)
	}
	if _, e = s.Records(ctx, "worker-token", RecordRequest{Action: "content", ProjectID: "p", Draft: content}); e != ErrForbidden {
		t.Fatal(e)
	}
	if _, e = s.Records(ctx, "owner-token", RecordRequest{Action: "archive", ProjectID: "p"}); e != ErrConflict {
		t.Fatal(e)
	}
	s.Store.Within(ctx, func(tx Tx) error {
		var p Project
		tx.Get("projects", "p", &p)
		p.Status = "closed"
		return tx.Put("projects", "p", p)
	})
	if _, e = s.Records(ctx, "owner-token", RecordRequest{Action: "archive", ProjectID: "p"}); e != nil {
		t.Fatal(e)
	}
	out, e = s.Records(ctx, "owner-token", RecordRequest{Action: "mine"})
	if e != nil || len(out.Items) != 0 {
		t.Fatal(out, e)
	}
	out, e = s.Records(ctx, "owner-token", RecordRequest{Action: "mine", Archived: true})
	if e != nil || len(out.Items) != 1 {
		t.Fatal(out, e)
	}
	if _, e = s.Detail(ctx, "", "p"); e != ErrNotFound {
		t.Fatal("archived project exposed", e)
	}
	if _, e = s.Records(ctx, "owner-token", RecordRequest{Action: "unarchive", ProjectID: "p"}); e != nil {
		t.Fatal(e)
	}
}
func TestReceiptNotificationsOwnershipReadThankAndLedger(t *testing.T) {
	s, _ := setup(t)
	ctx := context.Background()
	p := Project{ID: "p", OwnerUserID: "owner", Draft: Draft{Title: "Native bounty"}}
	ch := Challenge{ID: "ch", ProjectID: "p", ParticipantUserID: "worker"}
	cmd := Command{Action: "tip", OperationID: "tip-op", ProjectID: "p", ChallengeID: "ch", ReviewNote: "thanks"}
	receipt := Receipt{OperationID: "tip-op", Credit: 7, JournalIDs: []int64{42}}
	if e := s.Store.Within(ctx, func(tx Tx) error {
		tx.Put("projects", "p", p)
		tx.Put("challenges", "ch", ch)
		return s.projectNotification(tx, Actor{UserID: "owner"}, cmd, receipt, p, ch, 100)
	}); e != nil {
		t.Fatal(e)
	}
	s.Store.Within(ctx, func(tx Tx) error { return s.projectNotification(tx, Actor{UserID: "owner"}, cmd, receipt, p, ch, 101) })
	out, e := s.Records(ctx, "worker-token", RecordRequest{Action: "tips_received"})
	if e != nil || len(out.Items) != 1 {
		t.Fatal(out, e)
	}
	if _, e = s.Records(ctx, "owner-token", RecordRequest{Action: "thank_tip", NotificationID: "tip-op"}); e != ErrForbidden {
		t.Fatal(e)
	}
	out, e = s.Records(ctx, "worker-token", RecordRequest{Action: "thank_tip", NotificationID: "tip-op"})
	if e != nil || out.Notification.ThankedAt == 0 || out.Notification.RecipientReadAt == 0 || out.Notification.Quota != 7 {
		t.Fatal(out, e)
	}
	var ledger []json.RawMessage
	s.Store.Within(ctx, func(tx Tx) error { ledger, _ = tx.(ViewTx).All("ledger", Query{ProjectID: "p"}); return nil })
	if len(ledger) != 1 || !strings.Contains(string(ledger[0]), "42") {
		t.Fatal("receipt ledger duplicated/lost", ledger)
	}
	s.Store.Within(ctx, func(tx Tx) error {
		return tx.Put("notifications", "reward", Notification{ID: "reward", RecipientUserID: "worker", Kind: "reward_transfer", Quota: 99})
	})
	if _, e = s.Records(ctx, "worker-token", RecordRequest{Action: "tips_read"}); e != nil {
		t.Fatal(e)
	}
	s.Store.Within(ctx, func(tx Tx) error {
		var n Notification
		tx.Get("notifications", "reward", &n)
		if n.RecipientReadAt != 0 {
			t.Fatal("tips read marked other reward")
		}
		return nil
	})
	if _, e = s.Records(ctx, "worker-token", RecordRequest{Action: "notifications_read"}); e != nil {
		t.Fatal(e)
	}
}
