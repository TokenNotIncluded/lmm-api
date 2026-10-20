package assistant_services

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestIssueDetailEscapingStaysWithinToolBudget(t *testing.T) {
	events := []IssueEvent{}
	for i := 0; i < 30; i++ {
		event := boundedIssueEvent(IssueEvent{ID: int64(i + 1), IssueID: 1, Visibility: "admin", Status: "in_progress", Note: strings.Repeat("<", 1500)})
		if !event.NoteTruncated || len(event.Note) != 150 {
			t.Fatal("missing explicit note truncation")
		}
		events = append(events, event)
	}
	value := map[string]any{"ok": true, "issue": Issue{ID: 1, ReporterID: 7, Kind: "security", Title: strings.Repeat("<", 128), Body: strings.Repeat("<", 4000), Visibility: "admin", Status: "in_progress", Revision: 1}, "events": events}
	raw, err := json.Marshal(value)
	if err != nil || len(raw) > 64<<10 {
		t.Fatalf("detail bytes=%d err=%v", len(raw), err)
	}
}
