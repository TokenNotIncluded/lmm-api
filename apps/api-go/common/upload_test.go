package common

import (
	"bytes"
	"mime/multipart"
	"net/textproto"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestReadUploadEnforcesStreamingLimit(t *testing.T) {
	var body bytes.Buffer
	writer := multipart.NewWriter(&body)
	header := textproto.MIMEHeader{}
	header.Set("Content-Disposition", `form-data; name="file"; filename="test.bin"`)
	part, err := writer.CreatePart(header)
	require.NoError(t, err)
	_, err = part.Write([]byte("12345"))
	require.NoError(t, err)
	require.NoError(t, writer.Close())

	reader := multipart.NewReader(bytes.NewReader(body.Bytes()), writer.Boundary())
	form, err := reader.ReadForm(32)
	require.NoError(t, err)
	upload := form.File["file"][0]
	_, err = ReadUpload(upload, 4)
	assert.Error(t, err)
	data, err := ReadUpload(upload, 5)
	require.NoError(t, err)
	assert.Equal(t, "12345", string(data))
}
