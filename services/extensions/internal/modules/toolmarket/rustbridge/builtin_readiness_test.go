package rustbridge

import (
	"context"
	"fmt"
	"net/http"
	"testing"

	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

func TestBuiltinAndImageReadinessRequireExactLivePurposeACK(t *testing.T) {
	for _, test := range []struct {
		name string
		body string
		gate bool
		want bool
	}{
		{"registry", fmt.Sprintf(`{"protocol":1,"service":"builtins","ready":true,"tools":29,"catalog_digest":%q}`, market.CompiledBuiltinCatalogDigest()), true, true},
		{"registry digest substitution", `{"protocol":1,"service":"builtins","ready":true,"tools":29,"catalog_digest":"unrelated"}`, true, false},
		{"registry incomplete", fmt.Sprintf(`{"protocol":1,"service":"builtins","ready":true,"tools":28,"catalog_digest":%q}`, market.CompiledBuiltinCatalogDigest()), true, false},
		{"image", `{"protocol":2,"service":"images","ready":true}`, false, true},
		{"image false", `{"protocol":2,"service":"images","ready":false}`, false, false},
		{"image duplicate", `{"protocol":2,"service":"images","ready":false,"ready":true}`, false, false},
		{"image alias", `{"protocol":2,"service":"images","Ready":true}`, false, false},
		{"image extra", `{"protocol":2,"service":"images","ready":true,"caller_ready":true}`, false, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			adapter := drawingTestServer(t, func(w http.ResponseWriter, r *http.Request) {
				if len(r.Header.Values("X-Lmm-User-Credential")) != 0 {
					t.Error("readiness may not use a user Session")
				}
				expected := "/internal/models/images/v1/ready"
				protocol := "2"
				if test.gate {
					expected = "/internal/builtins/v1/ready"
					protocol = "1"
				}
				if r.URL.Path != expected || r.Header.Get("X-Lmm-Protocol") != protocol {
					t.Error("wrong purpose endpoint/protocol")
				}
				w.Header().Set("Content-Type", "application/json")
				w.Write([]byte(test.body))
			})
			var e error
			if test.gate {
				e = adapter.bridge.ReadyBuiltins(context.Background())
			} else {
				e = adapter.Ready(context.Background())
			}
			if (e == nil) != test.want {
				t.Fatalf("ready=%v want=%v", e, test.want)
			}
		})
	}
}
