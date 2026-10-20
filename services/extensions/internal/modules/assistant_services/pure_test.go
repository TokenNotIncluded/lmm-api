// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
package assistant_services

import (
	"encoding/json"
	"strings"
	"testing"
)

func pureResult(t *testing.T, name, raw string) map[string]any {
	t.Helper()
	result, err := ExecutePure(name, json.RawMessage(raw))
	if err != nil {
		t.Fatal(err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(result, &decoded); err != nil {
		t.Fatal(err)
	}
	return decoded
}

func TestPureVisualizationsMainParity(t *testing.T) {
	for _, tc := range []struct{ name, raw, kind string }{
		{"show_chart", `{"title":"Recorded requests","chart_type":"line","labels":["Mon","Tue"],"series":[{"name":"Calls","values":[0,12]}]}`, "chart"},
		{"show_statistics", `{"title":"Summary","items":[{"label":"Requests","value":"12","icon":"chart"}]}`, "statistics"},
		{"show_choices", `{"title":"Choose a period","items":[{"label":"Seven days","value":"Show the last seven days"}]}`, "choices"},
		{"show_flowchart", `{"title":"Process","nodes":[{"id":"a","label":"Start"},{"id":"b","label":"Finish"}],"edges":[{"from":"a","to":"b"}]}`, "flowchart"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r := pureResult(t, tc.name, tc.raw)
			if r["ok"] != true || r["visualization"].(map[string]any)["kind"] != tc.kind {
				t.Fatal(r)
			}
			r = pureResult(t, tc.name, strings.TrimSuffix(tc.raw, "}")+`,"script":"alert(1)"}`)
			if r["ok"] != false || r["status"] != "invalid_visualization" {
				t.Fatal(r)
			}
		})
	}
	for _, raw := range []string{
		`{"title":"Bad","chart_type":"donut","labels":["a"],"series":[{"name":"x","values":[-1]}]}`,
		`{"title":"Bad","chart_type":"bar","labels":["a"],"series":[{"name":"x","values":[1,2]}]}`,
		`{"title":"Bad","chart_type":"line","labels":["a"],"series":[{"name":"x","values":[1e20]}]}`,
	} {
		if r := pureResult(t, "show_chart", raw); r["ok"] != false {
			t.Fatal(r)
		}
	}
	if r := pureResult(t, "show_flowchart", `{"title":"Invalid edge","nodes":[{"id":"a","label":"Start"}],"edges":[{"from":"a","to":"missing"}]}`); r["ok"] != false {
		t.Fatal(r)
	}
}

func TestPureMathRestrictedAndBounded(t *testing.T) {
	for _, tc := range []struct {
		expression string
		want       float64
	}{{"2^3+2**3", 16}, {"percent(25)*amount", 20}, {"clamp(sqrt(81),0,8)", 8}, {"round(pi)", 3}} {
		raw, _ := json.Marshal(map[string]any{"expression": tc.expression, "variables": map[string]any{"amount": 80}})
		r := pureResult(t, "calculate_math", string(raw))
		if r["ok"] != true || r["result"] != tc.want {
			t.Fatal(r)
		}
	}
	for _, raw := range []string{`{}`, `{"expression":"1/0"}`, `{"expression":"sqrt(-1)"}`, `{"expression":"unknown(1)"}`, `{"expression":"sum(1..1000000000)"}`, `{"expression":"len(1..1000000000)"}`, `{"expression":"env.secret"}`, `{"expression":"a","variables":{"a":"3"}}`, `{"expression":"1","variables":{"pi":2}}`, `{"expression":"1","extra":true}`} {
		if r := pureResult(t, "calculate_math", raw); r["ok"] != false {
			t.Fatal(raw, r)
		}
	}
	raw, _ := json.Marshal(map[string]any{"expression": strings.Repeat("1+", 300) + "1"})
	if r := pureResult(t, "calculate_math", string(raw)); r["ok"] != false {
		t.Fatal(r)
	}
}

func TestPureRejectsInvalidEnvelopeAndUnknownTools(t *testing.T) {
	for _, raw := range []string{"null", "[]", "{} {}", strings.Repeat(" ", 16385)} {
		if _, err := ExecutePure("calculate_math", json.RawMessage(raw)); err == nil {
			t.Fatal(raw)
		}
	}
	if _, err := ExecutePure("get_user_overview", json.RawMessage(`{}`)); err == nil {
		t.Fatal("account authority must not be a pure tool")
	}
	defs := PureDefinitions()
	if len(defs) != 5 {
		t.Fatal(defs)
	}
	defs[0].Name = "mutated"
	if PureDefinitions()[0].Name == "mutated" {
		t.Fatal("metadata must not share mutable definitions")
	}
}
