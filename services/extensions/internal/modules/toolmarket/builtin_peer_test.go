// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"testing"
)

func TestBuiltinPeerIsTrustedContextAndCanonicalIP(t *testing.T) {
	if got := builtinPeerIP(WithBuiltinPeerIP(context.Background(), "::ffff:192.0.2.4")); got != "192.0.2.4" {
		t.Fatal("actual peer not normalized", got)
	}
	for _, invalid := range []string{"", "host.example", "192.0.2.4:3000", "fe80::1%eth0", "0.0.0.0", "ff02::1"} {
		if builtinPeerIP(WithBuiltinPeerIP(context.Background(), invalid)) != "" {
			t.Fatal("invalid peer accepted", invalid)
		}
	}
}
