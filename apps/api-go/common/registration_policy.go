package common

import (
	"errors"
	"sort"
	"strings"
)

// RegistrationDisabledMethodsOptionKey stores OAuth methods that may still be
// used to sign in, but must not create new accounts. Values are comma- or
// newline-separated stable provider IDs (for example, "github" or
// "custom:company-sso").
const RegistrationDisabledMethodsOptionKey = "RegistrationDisabledMethods"

const maxRegistrationDisabledMethods = 64

var builtInRegistrationMethods = map[string]struct{}{
	"github": {}, "discord": {}, "oidc": {}, "telegram": {}, "linuxdo": {}, "wechat": {},
}

func normalizeRegistrationMethod(value string) string {
	return strings.ToLower(strings.TrimSpace(value))
}

func validCustomRegistrationMethod(value string) bool {
	const prefix = "custom:"
	if !strings.HasPrefix(value, prefix) {
		return false
	}
	slug := strings.TrimPrefix(value, prefix)
	if slug == "" || len(slug) > 64 {
		return false
	}
	for _, char := range slug {
		if !((char >= 'a' && char <= 'z') || (char >= '0' && char <= '9') || char == '-') {
			return false
		}
	}
	return true
}

// ParseRegistrationDisabledMethods validates and canonicalizes the persisted
// list. Unknown built-in IDs are rejected so a typo cannot silently hide a
// different method after a provider is added later.
func ParseRegistrationDisabledMethods(raw string) ([]string, error) {
	parts := strings.FieldsFunc(raw, func(char rune) bool {
		return char == ',' || char == '\n' || char == '\r'
	})
	seen := make(map[string]struct{}, len(parts))
	methods := make([]string, 0, len(parts))
	for _, part := range parts {
		method := normalizeRegistrationMethod(part)
		if method == "" {
			continue
		}
		if _, ok := builtInRegistrationMethods[method]; !ok && !validCustomRegistrationMethod(method) {
			return nil, errors.New("unknown registration method: " + method)
		}
		if _, ok := seen[method]; ok {
			continue
		}
		seen[method] = struct{}{}
		methods = append(methods, method)
		if len(methods) > maxRegistrationDisabledMethods {
			return nil, errors.New("too many disabled registration methods")
		}
	}
	sort.Strings(methods)
	return methods, nil
}

func GetRegistrationDisabledMethods() []string {
	OptionMapRWMutex.RLock()
	raw := ""
	if OptionMap != nil {
		raw = OptionMap[RegistrationDisabledMethodsOptionKey]
	}
	OptionMapRWMutex.RUnlock()
	methods, err := ParseRegistrationDisabledMethods(raw)
	if err != nil {
		return nil
	}
	return methods
}

func IsRegistrationMethodDisabled(method string) bool {
	method = normalizeRegistrationMethod(method)
	if method == "" {
		return false
	}
	for _, disabled := range GetRegistrationDisabledMethods() {
		if method == disabled {
			return true
		}
	}
	return false
}
