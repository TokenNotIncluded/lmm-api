package postgresurl

import (
	"net/url"
	"strings"
	"testing"

	"github.com/lib/pq"
)

func TestUnixURLClosedSocketProfile(t *testing.T) {
	base := "postgresql://runtime:synthetic@/lmm_toolmarket?host=%2Frun%2Flmm-postgresql&port=5433&sslmode=disable"
	for _, raw := range []string{base, strings.Replace(base, "&port=5433", "", 1)} {
		u, err := url.Parse(raw)
		if err != nil {
			t.Fatal("parse fixture")
		}
		unix, err := Unix(u)
		if !unix || err != nil {
			t.Fatal("valid explicit socket rejected", err)
		}
		parsed, err := pq.ParseURL(raw)
		if err != nil || !strings.Contains(parsed, "host='/run/lmm-postgresql'") || !strings.Contains(parsed, "sslmode='disable'") {
			t.Fatal("pinned lib/pq socket parsing differs")
		}
		if strings.Contains(raw, "port=") && !strings.Contains(parsed, "port='5433'") {
			t.Fatal("pinned lib/pq lost socket port")
		}
	}
	for _, raw := range []string{
		strings.Replace(base, "runtime:synthetic@", "runtime@", 1),
		strings.Replace(base, "synthetic@", "@", 1),
		strings.Replace(base, "runtime:", "runtime%00:", 1),
		strings.Replace(base, "synthetic@", "synthetic%00@", 1),
		strings.Replace(base, "@/", "@:5433/", 1),
		strings.Replace(base, "%2Frun%2Flmm-postgresql", "relative", 1),
		strings.Replace(base, "%2Frun%2Flmm-postgresql", "%2Frun%2F..%2Fsocket", 1),
		strings.Replace(base, "%2Frun%2Flmm-postgresql", "%2Frun%2F.%2Fsocket", 1),
		strings.Replace(base, "%2Frun%2Flmm-postgresql", "%2Frun%2Fsocket%00", 1),
		strings.Replace(base, "%2Frun%2Flmm-postgresql", "%2Frun%2Fsocket%0A", 1),
		strings.Replace(base, "%2Frun%2Flmm-postgresql", "%2Frun%2Fsocket,%2Fother", 1),
		strings.Replace(base, "port=5433", "port=80", 1),
		strings.Replace(base, "port=5433", "port=65536", 1),
		strings.Replace(base, "port=5433", "port=05433", 1),
		strings.Replace(base, "port=5433", "port=", 1),
		strings.Replace(base, "sslmode=disable", "sslmode=require", 1),
		strings.Replace(base, "&sslmode=disable", "", 1),
		base + "&host=%2Fother", base + "&port=5434", base + "&sslmode=disable",
		base + "&hostaddr=127.0.0.1", base + "&service=other", base + "&options=-crole%3Dother", base + "&unknown=1", base + "&bad=%zz",
	} {
		u, err := url.Parse(raw)
		if err != nil {
			continue
		}
		if _, err := Unix(u); err == nil {
			t.Fatal("invalid socket profile accepted")
		}
	}
}

func TestUnixURLRejectsConnectionEnvironmentWithoutLeakingValues(t *testing.T) {
	u, _ := url.Parse("postgres://runtime:synthetic@/lmm_toolmarket?host=%2Frun%2Flmm-postgresql&sslmode=disable")
	for _, name := range []string{"PGHOST", "PGPORT", "PGDATABASE", "PGUSER", "PGPASSWORD", "PGOPTIONS", "PGHOSTADDR", "PGSERVICE", "PGPASSFILE"} {
		t.Run(name, func(t *testing.T) {
			t.Setenv(name, "private-environment-sentinel")
			if _, err := Unix(u); err == nil || strings.Contains(err.Error(), "sentinel") || strings.Contains(err.Error(), "synthetic") {
				t.Fatal("environment accepted or value leaked")
			}
		})
	}
	t.Run("empty unsupported variable", func(t *testing.T) {
		t.Setenv("PGSERVICE", "")
		if _, err := Unix(u); err == nil {
			t.Fatal("lib/pq panic path allowed")
		}
	})
	t.Run("PGDATA is unrelated", func(t *testing.T) {
		t.Setenv("PGDATA", "/synthetic/data")
		if _, err := Unix(u); err != nil {
			t.Fatal("unrelated server environment rejected")
		}
	})
	tcp, _ := url.Parse("postgres://runtime:synthetic@127.0.0.1/lmm_toolmarket?sslmode=verify-full")
	t.Setenv("PGHOST", "/unrelated")
	if unix, err := Unix(tcp); unix || err != nil {
		t.Fatal("TCP policy changed")
	}
}
