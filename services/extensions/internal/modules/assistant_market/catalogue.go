// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Catalogue projections follow fixed main 3948a0a assistant_catalog_tools.go.
package assistant_market

import (
	"bytes"
	"context"
	_ "embed"
	"encoding/json"
	"strconv"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/redaction"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
)

//go:embed read_definitions.json
var readDefinitionsJSON []byte

type catalogueExecutor struct {
	parent     *Executor
	definition definition
}

// Bindings advertises only real implemented adapters. Connected-tool discovery
// requires a separate authoritative assistant allowlist and is not advertised.
func (e *Executor) Bindings() ([]assistant.AgentToolBinding, error) {
	if e == nil || access.Missing(e.market) {
		return nil, access.ErrInvalid
	}
	var definitions []definition
	if json.Unmarshal(readDefinitionsJSON, &definitions) != nil || len(definitions) != 2 {
		return nil, access.ErrUnavailable
	}
	out := []assistant.AgentToolBinding{e.Binding()}
	for _, d := range definitions {
		if (d.Name != "get_tool_market_services" && d.Name != "get_tool_market_service") || d.Version != assistant.FixedMainToolVersion || d.Effect != "read_only" || digest([]byte(d.SchemaJSON)) != d.SchemaSHA256 {
			return nil, access.ErrUnavailable
		}
		out = append(out, assistant.AgentToolBinding{Name: d.Name, Description: d.Description, Version: d.Version, Effect: d.Effect, Access: d.Access, PriceVersion: assistant.BuiltinToolPriceVersion, PriceSHA256: assistant.FreeAgentPriceSHA256(), Parameters: json.RawMessage(d.SchemaJSON), Executor: &catalogueExecutor{parent: e, definition: d}})
	}
	return out, nil
}
func (c *catalogueExecutor) Execute(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	return c.read(ctx, in)
}
func (c *catalogueExecutor) Inspect(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	return c.read(ctx, in)
}
func (c *catalogueExecutor) read(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	if c == nil || c.parent == nil || access.Missing(c.parent.market) || in.Credential == "" || in.Actor.UserID <= 0 || in.Actor.Account != (access.Account{Kind: "personal", ID: in.Actor.UserID}) || len(in.Call.OperationID) == 0 || len(in.Call.OperationID) > 128 || digest(in.Call.Arguments) != in.Call.ArgumentsSHA256 {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	d := c.definition
	s := in.Call.Scope
	if s.Name != d.Name || s.Version != d.Version || s.Domain != "extension" || s.SchemaSHA256 != d.SchemaSHA256 || s.PriceVersion != assistant.BuiltinToolPriceVersion || s.PriceSHA256 != assistant.FreeAgentPriceSHA256() {
		return assistant.AgentToolResult{}, access.ErrConflict
	}
	values, err := toolmarket.ValidateLegacyArguments(json.RawMessage(d.SchemaJSON), in.Call.Arguments)
	if err != nil {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	account := strconv.FormatInt(in.Actor.Account.ID, 10)
	user, err := c.parent.market.WebAuthorize(ctx, in.Credential, account, "toolmarket.read")
	if err != nil {
		return assistant.AgentToolResult{}, translate(err)
	}
	if user != in.Actor.UserID {
		return assistant.AgentToolResult{}, access.ErrForbidden
	}
	var value any
	if d.Name == "get_tool_market_service" {
		id := values["service_id"].(string)
		detail, err := c.parent.market.PublicWebDetail(ctx, in.Credential, account, id)
		if err != nil {
			return assistant.AgentToolResult{}, translate(err)
		}
		if detail.Service.ID != id || detail.Version.ServiceID != id || detail.Version.ID != detail.Service.LiveVersionID {
			return assistant.AgentToolResult{}, access.ErrConflict
		}
		view, err := c.serviceView(ctx, in, account, detail, true)
		if err != nil {
			return assistant.AgentToolResult{}, err
		}
		value = map[string]any{"ok": true, "service": view}
	} else {
		offset, limit := 0, 20
		query, _ := values["query"].(string)
		if n, ok := values["offset"].(json.Number); ok {
			v, _ := n.Int64()
			offset = int(v)
		}
		if n, ok := values["limit"].(json.Number); ok {
			v, _ := n.Int64()
			limit = int(v)
		}
		rows, err := c.parent.market.PublicWebCatalogue(ctx, in.Credential, account, toolmarket.WebQuery{Text: query, Offset: offset, Limit: limit})
		if err != nil {
			return assistant.AgentToolResult{}, translate(err)
		}
		items := []map[string]any{}
		for _, row := range rows {
			detail, err := c.parent.market.PublicWebDetail(ctx, in.Credential, account, row.ID)
			if err != nil {
				return assistant.AgentToolResult{}, translate(err)
			}
			if detail.Service.ID != row.ID || detail.Version.ServiceID != row.ID || detail.Version.ID != row.VersionID || detail.Service.LiveVersionID != row.VersionID {
				return assistant.AgentToolResult{}, access.ErrConflict
			}
			view, err := c.serviceView(ctx, in, account, detail, false)
			if err != nil {
				return assistant.AgentToolResult{}, err
			}
			items = append(items, view)
		}
		value = map[string]any{"ok": true, "entity_kind": "tool_market_service", "items": items, "offset": offset, "limit": limit, "has_more": len(rows) == limit}
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	clean, err := redaction.JSON(raw, in.Credential)
	if err != nil || len(clean) > 60<<10 {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	// Redaction can change an untrusted schema's text or property names. Do
	// not publish an altered schema next to the original digest as exact metadata.
	var safe any
	decoder := json.NewDecoder(bytes.NewReader(clean))
	decoder.UseNumber()
	if decoder.Decode(&safe) != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	pruneChangedSchemas(safe)
	clean, err = json.Marshal(safe)
	if err != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	return assistant.AgentToolResult{OperationID: in.Call.OperationID, State: assistant.AgentToolSucceeded, ResultJSON: clean, ResultSHA256: digest(clean), DomainReceiptID: in.Call.OperationID}, nil
}
func clipped(value string, max int) (string, bool) {
	r := []rune(value)
	if len(r) > max {
		return string(r[:max]), true
	}
	return value, false
}
func serviceView(d toolmarket.WebDetail, full bool) (map[string]any, error) {
	maxDescription, maxTools := 1200, 10
	if full {
		maxDescription, maxTools = 16000, 50
	}
	description, truncated := clipped(d.Version.Description, maxDescription)
	tools := []map[string]any{}
	toolsTruncated := false
	schemaBytes := 0
	for i, t := range d.Tools {
		if i >= maxTools {
			toolsTruncated = true
			break
		}
		if t.VersionID != d.Version.ID {
			return nil, access.ErrConflict
		}
		description, truncated := clipped(t.Description, 2000)
		permissions := []string{}
		if t.Permissions != "" && json.Unmarshal([]byte(t.Permissions), &permissions) != nil {
			return nil, access.ErrUnavailable
		}
		priceDigest, err := toolmarket.LegacyQuotePricingDigest(t.LegacyPricing())
		if err != nil {
			return nil, access.ErrUnavailable
		}
		row := map[string]any{"price_sha256": priceDigest, "id": t.ToolID, "name": t.Name, "description": description, "description_truncated": truncated, "permissions": permissions, "price_quota": t.PriceQuota, "billing_mode": t.BillingMode, "input_token_price_quota": t.InputTokenPriceQuota, "max_input_tokens": t.MaxInputTokens, "billing_rules": t.BillingRules, "schema_omitted": true}
		if full && len(t.InputSchema) <= 16000 && schemaBytes+len(t.InputSchema) <= 24<<10 && json.Valid([]byte(t.InputSchema)) {
			row["input_schema"] = json.RawMessage(t.InputSchema)
			row["input_schema_json"] = t.InputSchema
			row["input_schema_sha256"] = digest([]byte(t.InputSchema))
			row["schema_omitted"] = false
			schemaBytes += len(t.InputSchema)
		}
		tools = append(tools, row)
	}
	return map[string]any{"entity_kind": "tool_market_service", "id": d.Service.ID, "version_id": d.Version.ID, "name": d.Version.Name, "description": description, "description_truncated": truncated, "execution_type": d.Version.ExecutionType, "visibility": d.Version.Visibility, "validated": d.Validated, "tools": tools, "tool_count": len(d.Tools), "tools_truncated": toolsTruncated, "href": "/tool-market?service_id=" + d.Service.ID, "price_unit": "CREDIT", "credits_per_usd": int64(500000)}, nil
}

func pruneChangedSchemas(value any) {
	switch v := value.(type) {
	case map[string]any:
		if schema, ok := v["input_schema_json"].(string); ok {
			expected, _ := v["input_schema_sha256"].(string)
			var original any
			decoder := json.NewDecoder(bytes.NewBufferString(schema))
			decoder.UseNumber()
			originalErr := decoder.Decode(&original)
			originalJSON, _ := json.Marshal(original)
			publishedJSON, _ := json.Marshal(v["input_schema"])
			if digest([]byte(schema)) != expected || originalErr != nil || !bytes.Equal(originalJSON, publishedJSON) {
				delete(v, "input_schema")
				delete(v, "input_schema_json")
				delete(v, "input_schema_sha256")
				v["schema_omitted"] = true
			}
		}
		for _, child := range v {
			pruneChangedSchemas(child)
		}
	case []any:
		for _, child := range v {
			pruneChangedSchemas(child)
		}
	}
}

// Exact execution-resource digests describe the published quote. The separate
// input_schema_sha256 describes only the literal schema bytes, never authority.
func (c *catalogueExecutor) serviceView(ctx context.Context, in assistant.AgentInvocation, account string, d toolmarket.WebDetail, full bool) (map[string]any, error) {
	view, err := serviceView(d, full)
	if err != nil {
		return nil, err
	}
	rows := view["tools"].([]map[string]any)
	for _, row := range rows {
		id := row["id"].(string)
		quote, err := c.parent.market.LegacyReadToolQuote(ctx, in.Credential, account, d.Service.ID, d.Version.ID, id)
		if err != nil || quote.ID == "" || quote.ServiceID != d.Service.ID || quote.VersionID != d.Version.ID || quote.ToolID != id || !validDigest(quote.RemoteDigest) || !validDigest(quote.PricingDigest) {
			row["execution_binding_unavailable"] = true
			continue
		}
		row["resource_schema_sha256"] = quote.RemoteDigest
		row["resource_price_sha256"] = quote.PricingDigest
	}
	return view, nil
}
