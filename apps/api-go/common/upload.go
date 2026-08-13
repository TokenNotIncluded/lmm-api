package common

import (
	"fmt"
	"mime/multipart"
)

// ReadUpload validates the declared size, enforces the same limit while
// streaming, and always closes the multipart file.
func ReadUpload(header *multipart.FileHeader, limit int64) ([]byte, error) {
	if header == nil {
		return nil, fmt.Errorf("upload is missing")
	}
	if limit < 0 || header.Size > limit {
		return nil, fmt.Errorf("upload %q exceeds %d bytes", header.Filename, limit)
	}
	file, err := header.Open()
	if err != nil {
		return nil, err
	}
	defer file.Close()
	data, err := ReadAllLimit(file, limit)
	if err != nil {
		return nil, fmt.Errorf("read upload %q: %w", header.Filename, err)
	}
	return data, nil
}
