// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import "context"

const DrawingUploadTotalLimit = 16 << 20
const DrawingUploadFileLimit = 10 << 20
const DrawingUploadCountLimit = 8

// DrawingUpload is a transient sidecar for the Core preview, never market
// arguments or spending authority. Core independently verifies all bindings.
type DrawingUpload struct {
	MediaType  string `json:"media_type"`
	SHA256     string `json:"sha256"`
	ByteLength int64  `json:"byte_length"`
	DataBase64 string `json:"data_base64"`
}
type drawingUploadsKey struct{}

func WithDrawingUploads(ctx context.Context, uploads []DrawingUpload) context.Context {
	return context.WithValue(ctx, drawingUploadsKey{}, append([]DrawingUpload(nil), uploads...))
}
func DrawingUploads(ctx context.Context) []DrawingUpload {
	uploads, _ := ctx.Value(drawingUploadsKey{}).([]DrawingUpload)
	return append([]DrawingUpload(nil), uploads...)
}
