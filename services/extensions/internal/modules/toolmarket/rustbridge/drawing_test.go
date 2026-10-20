package rustbridge

import (
	"context"
	"encoding/json"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	market "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
)

func drawingTestServer(t *testing.T, handler http.HandlerFunc) *DrawingAdapter {
	t.Helper()
	dir, e := os.MkdirTemp("", "image-uds-")
	if e != nil {
		t.Fatal(e)
	}
	socket := filepath.Join(dir, "i.sock")
	listener, e := net.Listen("unix", socket)
	if e != nil {
		os.RemoveAll(dir)
		t.Fatal(e)
	}
	server := &http.Server{Handler: handler}
	go server.Serve(listener)
	token := []byte(strings.Repeat("synthetic-image-purpose-", 2))
	client, e := NewDrawing(socket, token, "https://core.example.test", false)
	if e != nil {
		t.Fatal(e)
	}
	t.Cleanup(func() { client.Close(); server.Close(); os.RemoveAll(dir) })
	return client
}
func TestDrawingPrivateWirePreviewQueryAndSettledReceipt(t *testing.T) {
	var posts atomic.Int64
	in := market.BuiltinInvocation{Context: market.BuiltinContext{CallID: "original-call", DispatchID: "original-dispatch"}, ToolName: "drawing.generate", Arguments: json.RawMessage(`{"prompt":"synthetic image"}`)}
	adapter := drawingTestServer(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("X-Lmm-Protocol") != "2" || r.Header.Get("X-Lmm-User-Credential") != "synthetic-personal-session" {
			t.Error("missing session/protocol")
		}
		var body map[string]json.RawMessage
		if json.NewDecoder(r.Body).Decode(&body) != nil {
			t.Error("bad request")
		}
		var bound market.BuiltinContext
		json.Unmarshal(body["context"], &bound)
		if bound.CallID != in.Context.CallID || bound.DispatchID != in.Context.DispatchID {
			t.Error("lost original context")
		}
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/internal/models/images/v1/preview":
			w.Write([]byte(`{"operation_id":"image:original","preview_digest":"digest","input":{"prompt":"synthetic image"},"reservation_credits":21,"expires_at":999,"state":"previewed","confirmation_path":"/core/v1/images/confirm/image:original"}`))
		case "/internal/models/images/v1/query":
			if _, present := body["operation_id"]; present {
				t.Error("ACK-loss query must locate by context without fabricated id")
			}
			w.Write([]byte(`{"operation_id":"image:original","preview_digest":"digest","reservation_credits":21,"state":"confirmed","charged_credits":null,"output":null}`))
		case "/internal/models/images/v1/execute":
			posts.Add(1)
			w.Write([]byte(`{"operation_id":"image:original","preview_digest":"digest","reservation_credits":21,"state":"completed","charged_credits":17,"output":{"images":["synthetic"]}}`))
		default:
			t.Error("unexpected action")
			w.WriteHeader(404)
		}
	})
	preview, e := adapter.InvokeDrawing(context.Background(), "synthetic-personal-session", in, "original-key")
	if e != nil || !preview.AwaitingConfirmation || preview.SettlementStatus != "pending" {
		t.Fatalf("preview: %+v %v", preview, e)
	}
	queried, e := adapter.QueryDrawing(context.Background(), "synthetic-personal-session", in, "")
	if e != nil || !queried.ResumeReady || queried.DownstreamOperationID != "image:original" {
		t.Fatalf("query: %+v %v", queried, e)
	}
	if posts.Load() != 0 {
		t.Fatal("preview or query executed image")
	}
	completed, e := adapter.ResumeDrawing(context.Background(), "synthetic-personal-session", in, "image:original")
	if e != nil || completed.SettlementStatus != "settled" || posts.Load() != 1 {
		t.Fatalf("execute: %+v %v", completed, e)
	}
	var result struct {
		Content []json.RawMessage `json:"content"`
	}
	if json.Unmarshal(completed.Result, &result) != nil || len(result.Content) != 1 {
		t.Fatal("missing MCP result envelope")
	}
}
func TestDrawingDoesNotClearBillingWithoutCoreSettledCharge(t *testing.T) {
	zero := int64(0)
	for _, r := range []imageReceipt{{OperationID: "image:x", PreviewDigest: "x", State: "completed", Output: json.RawMessage(`{}`)}, {OperationID: "image:x", PreviewDigest: "x", State: "completed", ChargedCredits: &zero}, {OperationID: "image:x", PreviewDigest: "x", State: "unknown"}} {
		result, e := imageResult("call", r)
		if e == nil && result.SettlementStatus == "settled" {
			t.Fatal("unsettled image accepted")
		}
	}
}
