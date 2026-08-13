package appcli

import "testing"

func TestActionableJournalLineFiltersExpectedProxyDisconnects(t *testing.T) {
	tests := []struct {
		name       string
		line       string
		actionable bool
	}{
		{
			name:       "nginx proxy disconnect during restart",
			line:       `nginx: [error] sendfile() failed (32: Broken pipe) while sending request to upstream`,
			actionable: false,
		},
		{
			name:       "unrelated broken pipe",
			line:       `backend: write failed (32: Broken pipe)`,
			actionable: true,
		},
		{
			name:       "real upstream failure",
			line:       `nginx: connect() failed (111: Connection refused) while sending request to upstream`,
			actionable: true,
		},
		{
			name:       "client closes proxied response",
			line:       `nginx: [error] upstream prematurely closed connection while reading upstream, client: 203.0.113.10`,
			actionable: false,
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if got := actionableJournalLine(test.line); got != test.actionable {
				t.Fatalf("actionableJournalLine()=%v, want %v", got, test.actionable)
			}
		})
	}
}
