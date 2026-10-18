package toolmarket

import (
	"encoding/json"
	"sort"
	"strings"
)

// safeToolResult runs before persistence and before returning provider data to a
// caller. Redaction is a confidentiality boundary, not permission to interpret
// tool text, resource links or metadata as trusted instructions.
func safeToolResult(raw json.RawMessage, protected ...string) (json.RawMessage, error) {
	var result map[string]any
	if decodeStrict(raw, &result) != nil || result == nil {
		return nil, ErrUpstream
	}
	if content, ok := result["content"].([]any); !ok || content == nil {
		return nil, ErrUpstream
	}
	if value, ok := result["isError"]; ok {
		if _, valid := value.(bool); !valid {
			return nil, ErrUpstream
		}
	}
	// Longest first prevents a shorter credential from exposing the remainder
	// of a second credential with the same prefix. JSON has already been decoded,
	// so escaped forms of a credential receive exactly the same treatment.
	sort.Slice(protected, func(i, j int) bool { return len(protected[i]) > len(protected[j]) })
	pairs := make([]string, 0, 2*len(protected))
	for _, value := range protected {
		if value != "" {
			pairs = append(pairs, value, "[REDACTED_CREDENTIAL]")
		}
	}
	replace := strings.NewReplacer(pairs...).Replace
	var redact func(any) (any, error)
	redact = func(value any) (any, error) {
		switch v := value.(type) {
		case string:
			return replace(v), nil
		case []any:
			for i := range v {
				clean, err := redact(v[i])
				if err != nil {
					return nil, err
				}
				v[i] = clean
			}
		case map[string]any:
			clean := make(map[string]any, len(v))
			for name, child := range v {
				key := replace(name)
				if _, exists := clean[key]; exists {
					// Do not silently overwrite data when two untrusted member
					// names become equal after redaction.
					return nil, ErrUpstream
				}
				var err error
				if sensitiveResultField(name) {
					clean[key] = "[REDACTED_CREDENTIAL]"
				} else if clean[key], err = redact(child); err != nil {
					return nil, err
				}
			}
			return clean, nil
		}
		return value, nil
	}
	value, err := redact(result)
	if err != nil {
		return nil, err
	}
	clean := value.(map[string]any)
	// The marker comes from this client, never from a supplier's assertion.
	clean["untrusted_content"] = true
	encoded, err := json.Marshal(clean)
	if err != nil || len(encoded) > maxBody {
		return nil, ErrUpstream
	}
	return encoded, nil
}

func sensitiveResultField(name string) bool {
	name = strings.ToLower(strings.NewReplacer("-", "", "_", "", " ", "").Replace(name))
	switch name {
	case "authorization", "proxyauthorization", "cookie", "setcookie", "password", "passwd", "apikey", "xapikey", "accesstoken", "refreshtoken", "clientsecret":
		return true
	default:
		return false
	}
}
