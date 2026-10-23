// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"bytes"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"io"
	"mime"
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
)

var toolMarketDrawingUploadSlots = make(chan struct{}, 1)

// The browser request has the normal exact-version/grant invocation document
// plus bounded image files. Nothing is spooled to disk, fetched by URL or added
// to the persisted generic market arguments.
func toolMarketDrawingBody(w http.ResponseWriter, r *http.Request) (json.RawMessage, []toolmarket.DrawingUpload, error) {
	media, params, err := mime.ParseMediaType(r.Header.Get("Content-Type"))
	if err != nil || media != "multipart/form-data" || len(params) != 1 || params["boundary"] == "" {
		return nil, nil, toolmarket.ErrInvalid
	}
	r.Body = http.MaxBytesReader(w, r.Body, toolmarket.DrawingUploadTotalLimit+(512<<10))
	reader, err := r.MultipartReader()
	if err != nil {
		return nil, nil, toolmarket.ErrInvalid
	}
	var request json.RawMessage
	var uploads []toolmarket.DrawingUpload
	var total int64
	for count := 0; ; count++ {
		part, err := reader.NextPart()
		if err == io.EOF {
			break
		}
		if err != nil || count >= toolmarket.DrawingUploadCountLimit+1 {
			return nil, nil, toolmarket.ErrInvalid
		}
		headerBytes := 0
		for key, values := range part.Header {
			headerBytes += len(key)
			for _, value := range values {
				headerBytes += len(value)
			}
		}
		if headerBytes > 32<<10 {
			return nil, nil, toolmarket.ErrInvalid
		}
		name := part.FormName()
		if name == "request" {
			if len(request) != 0 || part.FileName() != "" {
				return nil, nil, toolmarket.ErrInvalid
			}
			raw, e := io.ReadAll(io.LimitReader(part, (64<<10)+1))
			part.Close()
			if e != nil || len(raw) == 0 || len(raw) > 64<<10 {
				return nil, nil, toolmarket.ErrInvalid
			}
			request = raw
			continue
		}
		if name != "images" || part.FileName() == "" || len(uploads) >= toolmarket.DrawingUploadCountLimit {
			return nil, nil, toolmarket.ErrInvalid
		}
		media, params, e := mime.ParseMediaType(part.Header.Get("Content-Type"))
		if e != nil || len(params) != 0 || media != "image/png" && media != "image/jpeg" && media != "image/webp" {
			return nil, nil, toolmarket.ErrInvalid
		}
		raw, e := io.ReadAll(io.LimitReader(part, toolmarket.DrawingUploadFileLimit+1))
		part.Close()
		if e != nil || len(raw) == 0 || len(raw) > toolmarket.DrawingUploadFileLimit || http.DetectContentType(raw) != media {
			return nil, nil, toolmarket.ErrInvalid
		}
		total += int64(len(raw))
		if total > toolmarket.DrawingUploadTotalLimit {
			return nil, nil, toolmarket.ErrInvalid
		}
		hash := sha256.Sum256(raw)
		uploads = append(uploads, toolmarket.DrawingUpload{MediaType: media, SHA256: hex.EncodeToString(hash[:]), ByteLength: int64(len(raw)), DataBase64: base64.StdEncoding.EncodeToString(raw)})
	}
	if len(request) == 0 || len(uploads) == 0 {
		return nil, nil, toolmarket.ErrInvalid
	}
	// Strict invocation decoding still runs in the market module. This local
	// view only verifies that its image metadata exactly matches the sidecar.
	var in struct {
		ToolID    string          `json:"tool_id"`
		VersionID string          `json:"version_id"`
		Arguments json.RawMessage `json:"arguments"`
	}
	if json.Unmarshal(request, &in) != nil {
		return nil, nil, toolmarket.ErrInvalid
	}
	compiled := false
	for _, definition := range toolmarket.CompiledBuiltinDefinitions() {
		if definition.Key != "drawing" || definition.VersionID != in.VersionID {
			continue
		}
		for _, tool := range definition.Tools {
			if tool.Name == "drawing.generate" && tool.ToolID == in.ToolID {
				compiled = true
			}
		}
	}
	if !compiled {
		return nil, nil, toolmarket.ErrInvalid
	}
	var args struct {
		ReferenceImages []struct {
			MediaType  string `json:"media_type"`
			SHA256     string `json:"sha256"`
			ByteLength int64  `json:"byte_length"`
		} `json:"reference_images"`
	}
	if json.Unmarshal(in.Arguments, &args) != nil || len(args.ReferenceImages) != len(uploads) {
		return nil, nil, toolmarket.ErrInvalid
	}
	for i, image := range args.ReferenceImages {
		actual := uploads[i]
		if image.MediaType != actual.MediaType || image.SHA256 != actual.SHA256 || image.ByteLength != actual.ByteLength {
			return nil, nil, toolmarket.ErrInvalid
		}
	}
	if !json.Valid(bytes.TrimSpace(request)) {
		return nil, nil, toolmarket.ErrInvalid
	}
	return request, uploads, nil
}
