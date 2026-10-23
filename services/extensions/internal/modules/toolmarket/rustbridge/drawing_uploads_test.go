// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"testing"

	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

func TestDrawingUploadsOnlyExpandInitialPreviewWire(t *testing.T) {
	preview, query, execute := 0, 0, 0
	upload := market.DrawingUpload{MediaType: "image/png", SHA256: strings.Repeat("a", 64), ByteLength: 70000, DataBase64: strings.Repeat("A", 100000)}
	in := market.BuiltinInvocation{ToolName: "drawing.generate", Context: market.BuiltinContext{CallID: "call", DispatchID: "dispatch"}, Arguments: json.RawMessage(`{"prompt":"edit","reference_images":[]}`)}
	adapter := drawingTestServer(t, func(w http.ResponseWriter, r *http.Request) {
		var body map[string]json.RawMessage
		if json.NewDecoder(r.Body).Decode(&body) != nil {
			t.Error("invalid private wire")
		}
		if r.Header.Get("X-Lmm-User-Credential") != "original-session" {
			t.Error("original session lost")
		}
		switch r.URL.Path {
		case "/internal/models/images/v1/preview":
			preview++
			var uploads []market.DrawingUpload
			if json.Unmarshal(body["uploads"], &uploads) != nil || len(uploads) != 1 || uploads[0] != upload {
				t.Error("transient sidecar changed")
			}
			w.Write([]byte(`{"operation_id":"image:original","preview_digest":"digest","input":{},"reservation_credits":12,"expires_at":999,"state":"previewed","confirmation_path":"/api/user/auth/images/confirm/image:original"}`))
		case "/internal/models/images/v1/query":
			query++
			if body["uploads"] != nil {
				t.Error("query carried replacement image")
			}
			w.Write([]byte(`{"operation_id":"image:original","preview_digest":"digest","reservation_credits":12,"state":"confirmed","charged_credits":null,"output":null,"expires_at":999}`))
		case "/internal/models/images/v1/execute":
			execute++
			if body["uploads"] != nil {
				t.Error("resume carried replacement image")
			}
			w.Write([]byte(`{"operation_id":"image:original","preview_digest":"digest","reservation_credits":12,"state":"completed","charged_credits":10,"output":{},"expires_at":999}`))
		default:
			t.Error("unexpected private request")
			w.WriteHeader(404)
		}
	})
	ctx := market.WithDrawingUploads(context.Background(), []market.DrawingUpload{upload})
	if _, err := adapter.InvokeDrawing(ctx, "original-session", in, ""); err != nil {
		t.Fatal(err)
	}
	observed, err := adapter.QueryDrawing(ctx, "original-session", in, "image:original")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(observed.Result), `"expires_at":999`) || !strings.Contains(string(observed.Result), `https://core.example.test/api/user/auth/images/confirm/image:original`) {
		t.Fatal("query lost original confirmation expiry/URL")
	}
	if _, err = adapter.ResumeDrawing(ctx, "original-session", in, "image:original"); err != nil {
		t.Fatal(err)
	}
	if preview != 1 || query != 1 || execute != 1 {
		t.Fatal("preview was retried", preview, query)
	}
	var out any
	if err := adapter.requestImage(ctx, "original-session", "query", map[string]string{"unexpected": strings.Repeat("x", 90<<10)}, &out); err != market.ErrInvalid || query != 1 {
		t.Fatal("generic command bound widened", err, query)
	}
}
