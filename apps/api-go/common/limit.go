package common

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/http"

	"github.com/QuantumNous/new-api/constant"
)

var ErrLimitExceeded = errors.New("byte limit exceeded")

// LimitBuffer is an io.Writer whose retained memory never exceeds Limit.
// Once full it rejects the write without growing its backing slice.
type LimitBuffer struct {
	buffer bytes.Buffer
	limit  int
}

func NewLimitBuffer(limit int) *LimitBuffer {
	if limit < 0 {
		limit = 0
	}
	return &LimitBuffer{limit: limit}
}

func (b *LimitBuffer) Write(data []byte) (int, error) {
	if len(data) > b.limit-b.buffer.Len() {
		return 0, ErrLimitExceeded
	}
	return b.buffer.Write(data)
}

func (b *LimitBuffer) WriteString(data string) (int, error) {
	if len(data) > b.limit-b.buffer.Len() {
		return 0, ErrLimitExceeded
	}
	return b.buffer.WriteString(data)
}

func (b *LimitBuffer) Bytes() []byte { return b.buffer.Bytes() }
func (b *LimitBuffer) Len() int      { return b.buffer.Len() }
func (b *LimitBuffer) Reset()        { b.buffer.Reset() }

// ReadAllLimit reads at most limit bytes and reports overflow distinctly.
func ReadAllLimit(reader io.Reader, limit int64) ([]byte, error) {
	if limit < 0 {
		limit = 0
	}
	data, err := io.ReadAll(io.LimitReader(reader, limit+1))
	if err != nil {
		return nil, err
	}
	if int64(len(data)) > limit {
		return nil, ErrLimitExceeded
	}
	return data, nil
}

func ResponseBodyLimit() int64 {
	megabytes := constant.MaxResponseBodyMB
	if megabytes <= 0 {
		megabytes = 32
	}
	return int64(megabytes) << 20
}

func ReadResponseBody(response *http.Response) ([]byte, error) {
	if response == nil || response.Body == nil {
		return nil, io.ErrUnexpectedEOF
	}
	return ReadAllLimit(response.Body, ResponseBodyLimit())
}

// SHA256RequestBody hashes a replayable request body without copying it when
// GetBody is available. The fallback retains at most limit bytes so signing a
// request can never turn an unbounded body into an unbounded duplicate.
func SHA256RequestBody(request *http.Request, limit int64) (string, error) {
	if request == nil {
		return "", io.ErrUnexpectedEOF
	}
	if limit < 0 {
		limit = 0
	}
	if request.Body == nil {
		sum := sha256.Sum256(nil)
		return hex.EncodeToString(sum[:]), nil
	}
	if request.GetBody != nil {
		body, err := request.GetBody()
		if err != nil {
			return "", err
		}
		defer body.Close()
		return hashReader(body, limit)
	}

	data, err := ReadAllLimit(request.Body, limit)
	if err != nil {
		return "", err
	}
	_ = request.Body.Close()
	request.Body = io.NopCloser(bytes.NewReader(data))
	request.GetBody = func() (io.ReadCloser, error) {
		return io.NopCloser(bytes.NewReader(data)), nil
	}
	return hashReader(bytes.NewReader(data), limit)
}

func hashReader(reader io.Reader, limit int64) (string, error) {
	hash := sha256.New()
	written, err := io.Copy(hash, io.LimitReader(reader, limit+1))
	if err != nil {
		return "", err
	}
	if written > limit {
		return "", ErrLimitExceeded
	}
	return hex.EncodeToString(hash.Sum(nil)), nil
}

// MarshalLimit JSON-encodes without ever retaining more than limit bytes.
func MarshalLimit(value any, limit int) ([]byte, error) {
	buffer := NewLimitBuffer(limit)
	encoder := json.NewEncoder(buffer)
	if err := encoder.Encode(value); err != nil {
		return nil, err
	}
	data := buffer.Bytes()
	if len(data) > 0 && data[len(data)-1] == '\n' {
		data = data[:len(data)-1]
	}
	return data, nil
}
