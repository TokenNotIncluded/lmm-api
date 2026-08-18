/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package setting

import (
	"errors"
	"fmt"
	"net/netip"
	"regexp"
	"strconv"
	"strings"
	"sync"
)

const (
	IPAccessRoutingRulesOptionKey = "IPAccessRoutingRules"
	DefaultIPAccessRoutingRules   = "# China\ndip(geoip:cn) -> reject"

	maxIPAccessRoutingRulesBytes = 16 * 1024
	maxIPAccessRoutingRuleCount  = 256
	maxIPAccessRoutingLineBytes  = 1024
)

type IPAccessRouteAction string

const (
	IPAccessRouteDirect IPAccessRouteAction = "direct"
	IPAccessRouteReject IPAccessRouteAction = "reject"
)

type IPAccessRouteRequest struct {
	ClientIP        string
	CountryCode     string
	L4Protocol      string
	DestinationPort int
}

type ipAccessMatcherKind uint8

const (
	ipAccessMatcherPrefix ipAccessMatcherKind = iota
	ipAccessMatcherCountry
	ipAccessMatcherPrivate
)

type ipAccessMatcher struct {
	kind        ipAccessMatcherKind
	prefix      netip.Prefix
	countryCode string
}

type ipAccessRouteRule struct {
	lineNumber int
	action     IPAccessRouteAction
	dip        []ipAccessMatcher
	protocols  map[string]struct{}
	ports      map[int]struct{}
}

type IPAccessRoutingPolicy struct {
	source string
	rules  []ipAccessRouteRule
}

var (
	ipAccessPredicatePattern = regexp.MustCompile(`^([a-z][a-z0-9_]*)\s*\((.*)\)$`)
	ipAccessRoutingMu        sync.RWMutex
	ipAccessRoutingPolicy    = mustParseIPAccessRoutingPolicy(DefaultIPAccessRoutingRules)
)

func mustParseIPAccessRoutingPolicy(source string) IPAccessRoutingPolicy {
	policy, err := ParseIPAccessRoutingRules(source)
	if err != nil {
		panic(err)
	}
	return policy
}

func GetIPAccessRoutingRules() string {
	ipAccessRoutingMu.RLock()
	defer ipAccessRoutingMu.RUnlock()
	return ipAccessRoutingPolicy.source
}

func UpdateIPAccessRoutingRules(source string) error {
	policy, err := ParseIPAccessRoutingRules(source)
	if err != nil {
		return err
	}
	ipAccessRoutingMu.Lock()
	ipAccessRoutingPolicy = policy
	ipAccessRoutingMu.Unlock()
	return nil
}

func ValidateIPAccessRoutingOption(key, value string) error {
	if key != IPAccessRoutingRulesOptionKey {
		return nil
	}
	_, err := ParseIPAccessRoutingRules(value)
	return err
}

func ParseIPAccessRoutingRules(source string) (IPAccessRoutingPolicy, error) {
	if len(source) > maxIPAccessRoutingRulesBytes {
		return IPAccessRoutingPolicy{}, fmt.Errorf("IP access routing rules cannot exceed %d bytes", maxIPAccessRoutingRulesBytes)
	}

	normalized := strings.ReplaceAll(source, "\r\n", "\n")
	normalized = strings.ReplaceAll(normalized, "\r", "\n")
	normalized = strings.TrimSpace(normalized)
	if normalized == "" {
		return IPAccessRoutingPolicy{}, errors.New("IP access routing rules must contain at least one rule")
	}

	policy := IPAccessRoutingPolicy{source: normalized}
	for index, rawLine := range strings.Split(normalized, "\n") {
		lineNumber := index + 1
		if len(rawLine) > maxIPAccessRoutingLineBytes {
			return IPAccessRoutingPolicy{}, fmt.Errorf("line %d: rule cannot exceed %d bytes", lineNumber, maxIPAccessRoutingLineBytes)
		}
		line := strings.TrimSpace(strings.SplitN(rawLine, "#", 2)[0])
		if line == "" {
			continue
		}
		if len(policy.rules) >= maxIPAccessRoutingRuleCount {
			return IPAccessRoutingPolicy{}, fmt.Errorf("IP access routing rules cannot contain more than %d rules", maxIPAccessRoutingRuleCount)
		}

		rule, err := parseIPAccessRouteRule(line, lineNumber)
		if err != nil {
			return IPAccessRoutingPolicy{}, err
		}
		policy.rules = append(policy.rules, rule)
	}
	if len(policy.rules) == 0 {
		return IPAccessRoutingPolicy{}, errors.New("IP access routing rules must contain at least one rule")
	}
	return policy, nil
}

func parseIPAccessRouteRule(line string, lineNumber int) (ipAccessRouteRule, error) {
	parts := strings.Split(line, "->")
	if len(parts) != 2 {
		return ipAccessRouteRule{}, fmt.Errorf("line %d: expected conditions -> direct or reject", lineNumber)
	}
	conditions := strings.TrimSpace(parts[0])
	action := IPAccessRouteAction(strings.TrimSpace(parts[1]))
	if conditions == "" {
		return ipAccessRouteRule{}, fmt.Errorf("line %d: at least one condition is required", lineNumber)
	}
	if action != IPAccessRouteDirect && action != IPAccessRouteReject {
		return ipAccessRouteRule{}, fmt.Errorf("line %d: action must be direct or reject", lineNumber)
	}

	rule := ipAccessRouteRule{lineNumber: lineNumber, action: action}
	seenPredicates := make(map[string]struct{})
	for _, rawCondition := range strings.Split(conditions, "&&") {
		condition := strings.TrimSpace(rawCondition)
		match := ipAccessPredicatePattern.FindStringSubmatch(condition)
		if match == nil {
			return ipAccessRouteRule{}, fmt.Errorf("line %d: invalid condition %q", lineNumber, condition)
		}
		name := match[1]
		if _, exists := seenPredicates[name]; exists {
			return ipAccessRouteRule{}, fmt.Errorf("line %d: duplicate %s() condition", lineNumber, name)
		}
		seenPredicates[name] = struct{}{}
		arguments, err := parseIPAccessArguments(match[2], lineNumber, name)
		if err != nil {
			return ipAccessRouteRule{}, err
		}

		switch name {
		case "dip":
			rule.dip, err = parseIPAccessDIPMatchers(arguments, lineNumber)
		case "l4proto":
			rule.protocols, err = parseIPAccessProtocols(arguments, lineNumber)
		case "dport":
			rule.ports, err = parseIPAccessPorts(arguments, lineNumber)
		case "domain", "pname":
			err = fmt.Errorf("line %d: %s() is not available for inbound HTTP routing; use dip()", lineNumber, name)
		default:
			err = fmt.Errorf("line %d: unsupported condition %s()", lineNumber, name)
		}
		if err != nil {
			return ipAccessRouteRule{}, err
		}
	}
	if len(rule.dip) == 0 {
		return ipAccessRouteRule{}, fmt.Errorf("line %d: every inbound routing rule must include dip()", lineNumber)
	}
	return rule, nil
}

func parseIPAccessArguments(raw string, lineNumber int, predicate string) ([]string, error) {
	parts := strings.Split(raw, ",")
	arguments := make([]string, 0, len(parts))
	for _, part := range parts {
		argument := strings.TrimSpace(part)
		if argument == "" {
			return nil, fmt.Errorf("line %d: %s() contains an empty value", lineNumber, predicate)
		}
		arguments = append(arguments, argument)
	}
	return arguments, nil
}

func parseIPAccessDIPMatchers(arguments []string, lineNumber int) ([]ipAccessMatcher, error) {
	matchers := make([]ipAccessMatcher, 0, len(arguments))
	seen := make(map[string]struct{}, len(arguments))
	for _, argument := range arguments {
		lower := strings.ToLower(argument)
		matcher := ipAccessMatcher{}
		canonical := ""
		if strings.HasPrefix(lower, "geoip:") {
			value := strings.TrimPrefix(lower, "geoip:")
			switch {
			case value == "private":
				matcher.kind = ipAccessMatcherPrivate
				canonical = "geoip:private"
			case len(value) == 2 && value[0] >= 'a' && value[0] <= 'z' && value[1] >= 'a' && value[1] <= 'z':
				matcher.kind = ipAccessMatcherCountry
				matcher.countryCode = strings.ToUpper(value)
				canonical = "geoip:" + value
			default:
				return nil, fmt.Errorf("line %d: invalid geoip value %q; use geoip:xx or geoip:private", lineNumber, argument)
			}
		} else {
			prefix, err := parseIPAccessPrefix(argument)
			if err != nil {
				return nil, fmt.Errorf("line %d: invalid dip value %q: %w", lineNumber, argument, err)
			}
			matcher.kind = ipAccessMatcherPrefix
			matcher.prefix = prefix
			canonical = prefix.String()
		}
		if _, exists := seen[canonical]; exists {
			continue
		}
		seen[canonical] = struct{}{}
		matchers = append(matchers, matcher)
	}
	return matchers, nil
}

func parseIPAccessPrefix(raw string) (netip.Prefix, error) {
	if address, err := netip.ParseAddr(raw); err == nil {
		address = address.Unmap()
		return netip.PrefixFrom(address, address.BitLen()), nil
	}
	prefix, err := netip.ParsePrefix(raw)
	if err != nil {
		return netip.Prefix{}, errors.New("expected an IPv4 address, IPv6 address, or CIDR")
	}
	address := prefix.Addr()
	if address.Is4In6() {
		if prefix.Bits() < 96 {
			return netip.Prefix{}, errors.New("IPv4-mapped IPv6 CIDR must have at least 96 prefix bits")
		}
		prefix = netip.PrefixFrom(address.Unmap(), prefix.Bits()-96)
	}
	return prefix.Masked(), nil
}

func parseIPAccessProtocols(arguments []string, lineNumber int) (map[string]struct{}, error) {
	protocols := make(map[string]struct{}, len(arguments))
	for _, argument := range arguments {
		protocol := strings.ToLower(argument)
		if protocol != "tcp" {
			return nil, fmt.Errorf("line %d: inbound HTTP routing supports only l4proto(tcp)", lineNumber)
		}
		protocols[protocol] = struct{}{}
	}
	return protocols, nil
}

func parseIPAccessPorts(arguments []string, lineNumber int) (map[int]struct{}, error) {
	ports := make(map[int]struct{}, len(arguments))
	for _, argument := range arguments {
		port, err := strconv.Atoi(argument)
		if err != nil || port < 1 || port > 65535 {
			return nil, fmt.Errorf("line %d: destination ports must be integers between 1 and 65535", lineNumber)
		}
		ports[port] = struct{}{}
	}
	return ports, nil
}

func EvaluateIPAccessRoute(request IPAccessRouteRequest) (IPAccessRouteAction, int, error) {
	address, err := netip.ParseAddr(strings.TrimSpace(request.ClientIP))
	if err != nil {
		return "", 0, errors.New("client IP is unavailable or invalid")
	}
	address = address.Unmap()
	countryCode := strings.ToUpper(strings.TrimSpace(request.CountryCode))
	if countryCode != "" && (len(countryCode) != 2 || countryCode[0] < 'A' || countryCode[0] > 'Z' || countryCode[1] < 'A' || countryCode[1] > 'Z') {
		return "", 0, errors.New("edge country code is invalid")
	}
	protocol := strings.ToLower(strings.TrimSpace(request.L4Protocol))

	ipAccessRoutingMu.RLock()
	policy := ipAccessRoutingPolicy
	ipAccessRoutingMu.RUnlock()
	for _, rule := range policy.rules {
		matched, unknown := rule.matches(address, countryCode, protocol, request.DestinationPort)
		if unknown != "" {
			return "", rule.lineNumber, fmt.Errorf("line %d cannot be evaluated: %s", rule.lineNumber, unknown)
		}
		if matched {
			return rule.action, rule.lineNumber, nil
		}
	}
	return IPAccessRouteDirect, 0, nil
}

func (rule ipAccessRouteRule) matches(address netip.Addr, countryCode, protocol string, port int) (bool, string) {
	dipMatched := false
	countryUnknown := false
	for _, matcher := range rule.dip {
		switch matcher.kind {
		case ipAccessMatcherPrefix:
			if matcher.prefix.Contains(address) {
				dipMatched = true
			}
		case ipAccessMatcherCountry:
			if countryCode == "" {
				countryUnknown = true
			} else if matcher.countryCode == countryCode {
				dipMatched = true
			}
		case ipAccessMatcherPrivate:
			if isIPAccessPrivate(address) {
				dipMatched = true
			}
		}
		if dipMatched {
			break
		}
	}
	if !dipMatched {
		if countryUnknown {
			return false, "edge country is unavailable"
		}
		return false, ""
	}
	if len(rule.protocols) > 0 {
		if protocol == "" {
			return false, "layer-4 protocol is unavailable"
		}
		if _, matches := rule.protocols[protocol]; !matches {
			return false, ""
		}
	}
	if len(rule.ports) > 0 {
		if port < 1 || port > 65535 {
			return false, "destination port is unavailable"
		}
		if _, matches := rule.ports[port]; !matches {
			return false, ""
		}
	}
	return true, ""
}

func isIPAccessPrivate(address netip.Addr) bool {
	return address.IsPrivate() || address.IsLoopback() || address.IsLinkLocalUnicast() ||
		address.IsLinkLocalMulticast() || address.IsUnspecified()
}
