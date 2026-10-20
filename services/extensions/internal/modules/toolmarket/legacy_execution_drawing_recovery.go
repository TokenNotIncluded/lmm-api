// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"strconv"
)

// This exact retired definition is never exposed by discovery, installation or
// initial dispatch. It only describes existing calls from the previous build.
//
//go:embed legacy_execution_drawing_v1.json
var legacyDrawingV1JSON []byte

func drawingV1RecoveryDefinition(version, toolID string) (CompiledBuiltinDefinition, CompiledBuiltinTool, error) {
	sum := sha256.Sum256(legacyDrawingV1JSON)
	if hex.EncodeToString(sum[:]) != "59eee8524800a8cf3218a716830ece16fb7084339c523f1e13195d2bba9cfbce" {
		return CompiledBuiltinDefinition{}, CompiledBuiltinTool{}, ErrDenied
	}
	var definition CompiledBuiltinDefinition
	if json.Unmarshal(legacyDrawingV1JSON, &definition) != nil || version != "37ff72e9-6f8e-5e34-a8e1-988253f20d27" || definition.VersionID != version || definition.Digest != "62c1757061b0aa45e71d509e61bacd4aa8dfec69492553ce3718c2369f8ecd32" {
		return definition, CompiledBuiltinTool{}, ErrDenied
	}
	for _, tool := range definition.Tools {
		if tool.Name == "drawing.generate" && tool.ToolID == toolID {
			return definition, tool, nil
		}
	}
	return definition, CompiledBuiltinTool{}, ErrDenied
}

type drawingRecoveryPermitKey struct{}
type drawingRecoveryPermit struct {
	Owner           Principal
	Context         BuiltinContext
	ArgumentsDigest string
}

func (m *Module) builtinRecoveryContext(ctx context.Context, r LegacyExecution) (context.Context, CompiledBuiltinDefinition, CompiledBuiltinTool, error) {
	if r.Builtin == nil {
		return ctx, CompiledBuiltinDefinition{}, CompiledBuiltinTool{}, ErrDenied
	}
	definition, tool, err := compiledBuiltin(r.Builtin.ServiceKey, r.Builtin.ToolName)
	if err == nil && definition.VersionID == r.Input.VersionID && tool.ToolID == r.Input.ToolID && tool.InputSchemaDigest == r.Builtin.InputSchemaDigest {
		return ctx, definition, tool, nil
	}
	definition, tool, err = drawingV1RecoveryDefinition(r.Input.VersionID, r.Input.ToolID)
	if err != nil || r.Builtin.ServiceKey != "drawing" || r.Builtin.ToolName != tool.Name || r.Builtin.InputSchemaDigest != tool.InputSchemaDigest || r.MeteringBinding.ServiceID != definition.ServiceID || r.MeteringBinding.RemoteDigest != tool.InputSchemaDigest || r.Pricing.PriceQuota != 0 || r.Pricing.BillingMode != "" || r.ProviderPricing != nil || r.Core.ID == "" || r.Core.DispatchID == "" || !legacyCallMatches(r.Core, r) {
		return ctx, definition, tool, ErrDenied
	}
	user, err := strconv.ParseInt(r.Owner.User, 10, 64)
	var args any
	if err != nil || user <= 0 || !r.Owner.valid() || r.ID != digest([]any{user, r.Input.ClientID, r.Input.RequestKey}) || r.RequestHash != digest(r.Input) || decodeStrict(r.Input.Arguments, &args) != nil || r.InputDigest != digest(args) || validateArgs(json.RawMessage(tool.InputSchema), r.Input.Arguments) != nil {
		return ctx, definition, tool, ErrDenied
	}
	err = m.store.view(func(d database) error {
		stored, ok := d.LegacyExecutions[r.ID]
		if !ok || stored.Owner != r.Owner || stored.RequestHash != r.RequestHash || stored.InputDigest != r.InputDigest || digest(stored.Input) != digest(r.Input) || stored.Core.ID != r.Core.ID || stored.Core.DispatchID != r.Core.DispatchID || stored.QuoteID != r.QuoteID || stored.Builtin == nil || stored.Builtin.ServiceKey != r.Builtin.ServiceKey || stored.Builtin.ToolName != r.Builtin.ToolName || stored.Builtin.InputSchemaDigest != r.Builtin.InputSchemaDigest || d.Web == nil {
			return ErrDenied
		}
		snapshot, ok := d.Web.Versions[definition.VersionID]
		if !ok || snapshot.Digest != definition.Digest || snapshot.Version.ID != definition.VersionID || snapshot.Version.ExecutionType != "builtin" || snapshot.Version.Digest != definition.Digest || snapshot.ValidationDigest != definition.Digest || snapshot.Version.ServiceID != definition.ServiceID || snapshot.RemoteDigests[tool.ToolID] != tool.InputSchemaDigest {
			return ErrDenied
		}
		for _, original := range snapshot.Tools {
			if original.ToolID == tool.ToolID && original.Name == tool.Name && original.VersionID == definition.VersionID && sameWebSchema(original.InputSchema, tool.InputSchema) && sameWebSchema(original.OutputSchema, tool.OutputSchema) && original.Permissions == tool.Permissions && original.PriceQuota == 0 && original.BillingMode == "" && original.ProviderPricing == nil {
				return nil
			}
		}
		return ErrDenied
	})
	if err != nil {
		return ctx, definition, tool, err
	}
	invocation := legacyBuiltinInvocation(r, "")
	permit := drawingRecoveryPermit{Owner: r.Owner, Context: invocation.Context, ArgumentsDigest: digest(args)}
	return context.WithValue(ctx, drawingRecoveryPermitKey{}, permit), definition, tool, nil
}

// Only the module's existing-call validation above can create this permit.
// A caller-supplied old version, arguments or context cannot select this path.
func drawingRecoveryInvocation(ctx context.Context, in BuiltinInvocation) (CompiledBuiltinDefinition, CompiledBuiltinTool, error) {
	permit, ok := ctx.Value(drawingRecoveryPermitKey{}).(drawingRecoveryPermit)
	var args any
	if !ok || permit.Owner != in.Owner || permit.Context != in.Context || in.ServiceKey != "drawing" || in.ToolName != "drawing.generate" || decodeStrict(in.Arguments, &args) != nil || digest(args) != permit.ArgumentsDigest {
		return CompiledBuiltinDefinition{}, CompiledBuiltinTool{}, ErrDenied
	}
	definition, tool, err := drawingV1RecoveryDefinition(in.Context.VersionID, in.Context.ToolID)
	if err != nil || validateArgs(json.RawMessage(tool.InputSchema), in.Arguments) != nil {
		return definition, tool, ErrDenied
	}
	return definition, tool, nil
}
