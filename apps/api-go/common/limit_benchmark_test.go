package common

import (
	"bytes"
	"net/http"
	"testing"
)

func BenchmarkReadAllLimit64K(b *testing.B) {
	payload := bytes.Repeat([]byte("x"), 64<<10)
	b.ReportAllocs()
	b.SetBytes(int64(len(payload)))
	for b.Loop() {
		if _, err := ReadAllLimit(bytes.NewReader(payload), int64(len(payload))); err != nil {
			b.Fatal(err)
		}
	}
}

func BenchmarkSHA256ReplayBody64K(b *testing.B) {
	payload := bytes.Repeat([]byte("x"), 64<<10)
	b.ReportAllocs()
	b.SetBytes(int64(len(payload)))
	for b.Loop() {
		request, err := http.NewRequest(http.MethodPost, "https://example.com", bytes.NewReader(payload))
		if err != nil {
			b.Fatal(err)
		}
		if _, err := SHA256RequestBody(request, int64(len(payload))); err != nil {
			b.Fatal(err)
		}
	}
}
