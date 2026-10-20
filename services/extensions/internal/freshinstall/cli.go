package freshinstall

import (
	"context"
	"encoding/json"
	"flag"
	"io"
)

// RunCLI handles only the explicit offline owner operation. Normal startup does
// not parse these flags or read an installation credential.
func RunCLI(ctx context.Context, args []string, out io.Writer) error {
	var c Config
	f := flag.NewFlagSet("install-db", flag.ContinueOnError)
	f.SetOutput(io.Discard)
	f.StringVar(&c.OwnerURLFile, "owner-url-file", "", "private installation login URL file")
	f.StringVar(&c.ExpectedDatabase, "database", "", "exact fresh dedicated database")
	f.StringVar(&c.OwnerRole, "owner-role", "", "actual database owner; SET ROLE supported")
	f.StringVar(&c.RuntimeRole, "runtime-role", "", "preprovisioned unprivileged runtime login")
	f.StringVar(&c.Component, "component", "", "assistant, toolmarket, toolmarket-builtins or payments")
	f.StringVar(&c.EncryptionKeyFile, "encryption-key-file", "", "private raw 32-byte market snapshot key")
	if f.Parse(args) != nil || f.NArg() != 0 {
		return ErrRejected
	}
	receipt, e := Install(ctx, c)
	if e != nil {
		return e
	}
	return json.NewEncoder(out).Encode(receipt)
}
