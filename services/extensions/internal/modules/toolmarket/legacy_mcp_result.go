// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"encoding/json"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func legacyMCPOutput(value any, err error) (*mcp.CallToolResult, error) {
	if err != nil {
		return &mcp.CallToolResult{IsError: true, Content: []mcp.Content{&mcp.TextContent{Text: err.Error()}}}, nil
	}
	raw, e := json.Marshal(value)
	if e != nil {
		return nil, e
	}
	return &mcp.CallToolResult{StructuredContent: json.RawMessage(raw), Content: []mcp.Content{&mcp.TextContent{Text: string(raw)}}}, nil
}

type legacyMCPContentMeta struct {
	Meta     mcp.Meta `json:"_meta"`
	Resource *struct {
		Meta mcp.Meta `json:"_meta"`
	} `json:"resource"`
}

func legacyMCPRestoreMeta(contents []mcp.Content, raw []legacyMCPContentMeta) {
	for i, item := range contents {
		if i >= len(raw) {
			return
		}
		meta := raw[i].Meta
		switch v := item.(type) {
		case *mcp.TextContent:
			v.Meta = meta
		case *mcp.ImageContent:
			v.Meta = meta
		case *mcp.AudioContent:
			v.Meta = meta
		case *mcp.ResourceLink:
			v.Meta = meta
		case *mcp.EmbeddedResource:
			v.Meta = meta
			if v.Resource != nil && raw[i].Resource != nil {
				v.Resource.Meta = raw[i].Resource.Meta
			}
		}
	}
}
func legacyMCPExecutionOutput(response LegacyWebExecutionResponse) (*mcp.CallToolResult, error) {
	result := &mcp.CallToolResult{Content: []mcp.Content{}}
	if len(response.Result) > 0 {
		if json.Unmarshal(response.Result, result) != nil {
			return legacyMCPOutput(nil, ErrInvalid)
		}
		var native struct {
			StructuredContent json.RawMessage        `json:"structuredContent"`
			Meta              mcp.Meta               `json:"_meta"`
			Content           []legacyMCPContentMeta `json:"content"`
		}
		decoder := json.NewDecoder(bytes.NewReader(response.Result))
		decoder.UseNumber()
		if decoder.Decode(&native) != nil {
			return legacyMCPOutput(nil, ErrInvalid)
		}
		if len(native.StructuredContent) > 0 {
			result.StructuredContent = native.StructuredContent
		}
		result.Meta = native.Meta
		legacyMCPRestoreMeta(result.Content, native.Content)
	} else {
		result.IsError = true
		result.Content = []mcp.Content{&mcp.TextContent{Text: "The original call has no final result available. Query metamcp action=call_status instead of starting another request."}}
	}
	if result.Meta == nil {
		result.Meta = mcp.Meta{}
	}
	result.Meta["lmm/market"] = map[string]any{"call": response.Call, "result_expired": response.ResultExpired, "result_expires_at": response.ResultExpiresAt, "error_code": response.ErrorCode}
	if response.Call["execution_status"] == "failed" || response.Call["execution_status"] == "cancelled" {
		result.IsError = true
	}
	return result, nil
}
