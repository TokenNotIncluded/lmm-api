// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"net/textproto"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
)

type drawingUploadTransport struct {
	marketTransportFake
	uploads []toolmarket.DrawingUpload
}

func (f *drawingUploadTransport) LegacyWeb(ctx context.Context, token, account string, in toolmarket.WebRequest) (any, error) {
	f.uploads = toolmarket.DrawingUploads(ctx)
	return f.marketTransportFake.LegacyWeb(ctx, token, account, in)
}
func drawingMultipartFixture(t *testing.T, badHash bool, media string, count int) *http.Request {
	t.Helper()
	image, err := base64.StdEncoding.DecodeString("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jfs8AAAAASUVORK5CYII=")
	if err != nil {
		t.Fatal(err)
	}
	hash := sha256.Sum256(image)
	sum := hex.EncodeToString(hash[:])
	if badHash {
		sum = strings.Repeat("0", 64)
	}
	var toolID, versionID string
	for _, d := range toolmarket.CompiledBuiltinDefinitions() {
		if d.Key == "drawing" {
			for _, tool := range d.Tools {
				if tool.Name == "drawing.generate" {
					toolID, versionID = tool.ToolID, d.VersionID
				}
			}
		}
	}
	var refs []map[string]any
	for i := 0; i < count; i++ {
		refs = append(refs, map[string]any{"media_type": media, "sha256": sum, "byte_length": len(image)})
	}
	request, _ := json.Marshal(map[string]any{"tool_id": toolID, "version_id": versionID, "grant_id": "grant", "request_id": "original-request", "arguments": map[string]any{"reference_images": refs}})
	var body bytes.Buffer
	w := multipart.NewWriter(&body)
	field, _ := w.CreateFormField("request")
	field.Write(request)
	for i := 0; i < count; i++ {
		header := textproto.MIMEHeader{"Content-Disposition": {`form-data; name="images"; filename="reference.png"`}, "Content-Type": {media}}
		part, e := w.CreatePart(header)
		if e != nil {
			t.Fatal(e)
		}
		part.Write(image)
	}
	w.Close()
	r := httptest.NewRequest("POST", "/api/tool-market/drawing/invoke", &body)
	r.Header.Set("Content-Type", w.FormDataContentType())
	r.Header.Set("Authorization", "Bearer "+webToken)
	return r
}
func TestMarketDrawingMultipartBindsTransientBytesAndPreservesExactGrant(t *testing.T) {
	core, service := &authCore{}, &drawingUploadTransport{}
	rec := httptest.NewRecorder()
	marketMux(t, core, service).ServeHTTP(rec, drawingMultipartFixture(t, false, "image/png", 1))
	if rec.Code != 200 || service.businessCalls != 1 || service.permission != "toolmarket.execute" || service.account != "17" || len(service.uploads) != 1 || service.request.Action != "invoke" {
		t.Fatal(rec.Code, service)
	}
	if strings.Contains(string(service.request.Body), "data_base64") || service.uploads[0].DataBase64 == "" {
		t.Fatal("image bytes entered generic arguments or were lost")
	}
	for _, tc := range []struct {
		hash  bool
		media string
		count int
	}{{true, "image/png", 1}, {false, "image/jpeg", 1}, {false, "image/png", 9}, {false, "image/png", 0}} {
		f := &drawingUploadTransport{}
		rec := httptest.NewRecorder()
		marketMux(t, &authCore{}, f).ServeHTTP(rec, drawingMultipartFixture(t, tc.hash, tc.media, tc.count))
		if rec.Code != 422 || f.businessCalls != 0 {
			t.Fatal("unbound image reached market execution", rec.Code, tc)
		}
	}
}

func TestMarketDrawingMultipartAuthorizationPrecedesReading(t *testing.T) {
	service := &drawingUploadTransport{}
	service.authErr = toolmarket.ErrDenied
	r := drawingMultipartFixture(t, false, "image/png", 1)
	body := &marketUnreadBody{}
	r.Body = body
	rec := httptest.NewRecorder()
	marketMux(t, &authCore{}, service).ServeHTTP(rec, r)
	if rec.Code != 403 || body.reads != 0 || service.businessCalls != 0 {
		t.Fatal("unauthorized image body was read", rec.Code, body.reads)
	}
}
