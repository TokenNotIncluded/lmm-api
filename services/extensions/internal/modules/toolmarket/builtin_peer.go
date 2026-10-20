// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"net/netip"
)

type builtinPeerContextKey struct{}

// WithBuiltinPeerIP accepts the actual connection's peer from a trusted host.
// Request arguments and forwarding headers must never populate it.
func WithBuiltinPeerIP(ctx context.Context, peer string) context.Context {
	ip, err := netip.ParseAddr(peer)
	if err != nil || ip.Zone() != "" || ip.IsUnspecified() || ip.IsMulticast() {
		return ctx
	}
	return context.WithValue(ctx, builtinPeerContextKey{}, ip.Unmap().String())
}
func builtinPeerIP(ctx context.Context) string {
	peer, _ := ctx.Value(builtinPeerContextKey{}).(string)
	return peer
}
