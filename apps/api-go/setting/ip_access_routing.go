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
	"net"
	"net/netip"
	neturl "net/url"
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
	// ClientIP is the trusted edge-observed connecting address. The HTTP
	// ingress policy intentionally maps both dip()/ip() and sip() to this
	// value; destination-IP headers are never accepted from clients.
	ClientIP        string
	CountryCode     string
	Domain          string
	L4Protocol      string
	DestinationPort int
	SourcePort      int
	SourceMAC       string
	ProcessName     string
	DSCP            int
	DSCPSet         bool
}

type ipAccessMatcherKind uint8

const (
	ipAccessMatcherPrefix ipAccessMatcherKind = iota
	ipAccessMatcherCountry
	ipAccessMatcherPrivate
	ipAccessMatcherExternal
)

type ipAccessMatcher struct {
	kind        ipAccessMatcherKind
	prefix      netip.Prefix
	countryCode string
	external    string
}

type ipAccessIPDirection uint8

const (
	ipAccessIPDestination ipAccessIPDirection = iota
	ipAccessIPSource
)

type ipAccessPortRange struct {
	min int
	max int
}

type ipAccessDomainMatcherKind uint8

const (
	ipAccessDomainSuffix ipAccessDomainMatcherKind = iota
	ipAccessDomainFull
	ipAccessDomainKeyword
	ipAccessDomainRegex
	ipAccessDomainGeoSite
	ipAccessDomainExternal
)

type ipAccessDomainMatcher struct {
	kind     ipAccessDomainMatcherKind
	value    string
	compiled *regexp.Regexp
}

type ipAccessConditionKind uint8

const (
	ipAccessConditionIP ipAccessConditionKind = iota
	ipAccessConditionDomain
	ipAccessConditionL4Protocol
	ipAccessConditionPort
	ipAccessConditionIPVersion
	ipAccessConditionMAC
	ipAccessConditionProcess
	ipAccessConditionDSCP
)

type ipAccessRouteCondition struct {
	kind        ipAccessConditionKind
	negated     bool
	ipDirection ipAccessIPDirection
	ipMatchers  []ipAccessMatcher
	domains     []ipAccessDomainMatcher
	values      map[string]struct{}
	ports       []ipAccessPortRange
	ipVersions  map[int]struct{}
	macs        map[string]struct{}
	processes   map[string]struct{}
	dscps       map[int]struct{}
}

type ipAccessRouteRule struct {
	lineNumber int
	action     IPAccessRouteAction
	conditions []ipAccessRouteCondition
}

type IPAccessRoutingPolicy struct {
	source   string
	rules    []ipAccessRouteRule
	fallback IPAccessRouteAction
}

var (
	ipAccessPredicatePattern = regexp.MustCompile(`^(!)?\s*([a-z][a-z0-9_]*)\s*\((.*)\)$`)
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

	policy := IPAccessRoutingPolicy{source: normalized, fallback: IPAccessRouteDirect}
	fallbackSeen := false
	for index, rawLine := range strings.Split(normalized, "\n") {
		lineNumber := index + 1
		if len(rawLine) > maxIPAccessRoutingLineBytes {
			return IPAccessRoutingPolicy{}, fmt.Errorf("line %d: rule cannot exceed %d bytes", lineNumber, maxIPAccessRoutingLineBytes)
		}
		line := strings.TrimSpace(stripIPAccessComment(rawLine))
		if line == "" {
			continue
		}
		if line == "routing {" || line == "}" {
			continue
		}
		if strings.HasPrefix(strings.ToLower(line), "fallback:") {
			if fallbackSeen {
				return IPAccessRoutingPolicy{}, fmt.Errorf("line %d: duplicate fallback", lineNumber)
			}
			action, err := parseIPAccessAction(strings.TrimSpace(line[len("fallback:"):]), lineNumber)
			if err != nil {
				return IPAccessRoutingPolicy{}, err
			}
			policy.fallback = action
			fallbackSeen = true
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
	if len(policy.rules) == 0 && !fallbackSeen {
		return IPAccessRoutingPolicy{}, errors.New("IP access routing rules must contain at least one rule")
	}
	return policy, nil
}

func parseIPAccessRouteRule(line string, lineNumber int) (ipAccessRouteRule, error) {
	parts, err := splitIPAccessTopLevel(line, "->")
	if err != nil {
		return ipAccessRouteRule{}, fmt.Errorf("line %d: %w", lineNumber, err)
	}
	if len(parts) != 2 {
		return ipAccessRouteRule{}, fmt.Errorf("line %d: expected conditions -> direct or reject", lineNumber)
	}
	conditions := strings.TrimSpace(parts[0])
	if conditions == "" {
		return ipAccessRouteRule{}, fmt.Errorf("line %d: at least one condition is required", lineNumber)
	}
	action, err := parseIPAccessAction(parts[1], lineNumber)
	if err != nil {
		return ipAccessRouteRule{}, err
	}

	rule := ipAccessRouteRule{lineNumber: lineNumber, action: action, conditions: make([]ipAccessRouteCondition, 0)}
	seenPredicates := make(map[string]struct{})
	conditionParts, err := splitIPAccessTopLevel(conditions, "&&")
	if err != nil {
		return ipAccessRouteRule{}, fmt.Errorf("line %d: %w", lineNumber, err)
	}
	for _, rawCondition := range conditionParts {
		condition := strings.TrimSpace(rawCondition)
		match := ipAccessPredicatePattern.FindStringSubmatch(condition)
		if match == nil {
			return ipAccessRouteRule{}, fmt.Errorf("line %d: invalid condition %q", lineNumber, condition)
		}
		name := strings.ToLower(match[2])
		if _, exists := seenPredicates[name]; exists {
			return ipAccessRouteRule{}, fmt.Errorf("line %d: duplicate %s() condition", lineNumber, name)
		}
		seenPredicates[name] = struct{}{}
		arguments, err := parseIPAccessArguments(match[3], lineNumber, name)
		if err != nil {
			return ipAccessRouteRule{}, err
		}

		parsed := ipAccessRouteCondition{negated: match[1] == "!"}
		switch name {
		case "dip", "ip":
			parsed.kind = ipAccessConditionIP
			parsed.ipDirection = ipAccessIPDestination
			parsed.ipMatchers, err = parseIPAccessDIPMatchers(arguments, lineNumber)
		case "sip":
			parsed.kind = ipAccessConditionIP
			parsed.ipDirection = ipAccessIPSource
			parsed.ipMatchers, err = parseIPAccessDIPMatchers(arguments, lineNumber)
		case "domain", "qname":
			if name == "qname" {
				err = fmt.Errorf("line %d: qname() is a DNS-only Daed matcher and is unavailable for inbound HTTP routing", lineNumber)
				break
			}
			parsed.kind = ipAccessConditionDomain
			parsed.domains, err = parseIPAccessDomainMatchers(arguments, lineNumber)
		case "l4proto":
			parsed.kind = ipAccessConditionL4Protocol
			parsed.values, err = parseIPAccessProtocols(arguments, lineNumber)
		case "dport", "sport":
			parsed.kind = ipAccessConditionPort
			if name == "sport" {
				parsed.ipDirection = ipAccessIPSource
			}
			parsed.ports, err = parseIPAccessPorts(arguments, lineNumber, name)
		case "ipversion":
			parsed.kind = ipAccessConditionIPVersion
			parsed.ipVersions, err = parseIPAccessVersions(arguments, lineNumber)
		case "mac":
			err = fmt.Errorf("line %d: mac() requires packet metadata unavailable to inbound HTTP routing", lineNumber)
		case "pname":
			err = fmt.Errorf("line %d: pname() requires local-process metadata unavailable to inbound HTTP routing", lineNumber)
		case "dscp":
			err = fmt.Errorf("line %d: dscp() requires packet metadata unavailable to inbound HTTP routing", lineNumber)
		default:
			err = fmt.Errorf("line %d: unsupported condition %s()", lineNumber, name)
		}
		if err != nil {
			return ipAccessRouteRule{}, err
		}
		rule.conditions = append(rule.conditions, parsed)
	}
	return rule, nil
}

func parseIPAccessAction(raw string, lineNumber int) (IPAccessRouteAction, error) {
	action := strings.ToLower(strings.TrimSpace(raw))
	switch action {
	case "direct", "must_direct", "must_rules", "direct(must)":
		return IPAccessRouteDirect, nil
	case "reject", "block":
		return IPAccessRouteReject, nil
	default:
		return "", fmt.Errorf("line %d: action must be direct or reject (Daed aliases: must_direct, must_rules, block)", lineNumber)
	}
}

func stripIPAccessComment(raw string) string {
	var quote byte
	escaped := false
	for index := 0; index < len(raw); index++ {
		char := raw[index]
		if quote != 0 {
			if escaped {
				escaped = false
				continue
			}
			if char == '\\' {
				escaped = true
				continue
			}
			if char == quote {
				quote = 0
			}
			continue
		}
		if char == '\'' || char == '"' {
			quote = char
			continue
		}
		if char == '#' {
			return raw[:index]
		}
	}
	return raw
}

func splitIPAccessTopLevel(raw, separator string) ([]string, error) {
	parts := make([]string, 0, 2)
	start := 0
	depth := 0
	var quote byte
	escaped := false
	for index := 0; index < len(raw); index++ {
		char := raw[index]
		if quote != 0 {
			if escaped {
				escaped = false
				continue
			}
			if char == '\\' {
				escaped = true
				continue
			}
			if char == quote {
				quote = 0
			}
			continue
		}
		switch char {
		case '\'', '"':
			quote = char
		case '(':
			depth++
		case ')':
			if depth == 0 {
				return nil, errors.New("unexpected closing parenthesis")
			}
			depth--
		}
		if depth == 0 && strings.HasPrefix(raw[index:], separator) {
			parts = append(parts, raw[start:index])
			index += len(separator) - 1
			start = index + 1
		}
	}
	if quote != 0 {
		return nil, errors.New("unterminated quoted value")
	}
	if depth != 0 {
		return nil, errors.New("unbalanced parentheses")
	}
	parts = append(parts, raw[start:])
	return parts, nil
}

func parseIPAccessArguments(raw string, lineNumber int, predicate string) ([]string, error) {
	parts, err := splitIPAccessTopLevel(raw, ",")
	if err != nil {
		return nil, fmt.Errorf("line %d: %s() %w", lineNumber, predicate, err)
	}
	arguments := make([]string, 0, len(parts))
	for _, part := range parts {
		argument := trimIPAccessQuotes(strings.TrimSpace(part))
		if argument == "" {
			return nil, fmt.Errorf("line %d: %s() contains an empty value", lineNumber, predicate)
		}
		arguments = append(arguments, argument)
	}
	return arguments, nil
}

func trimIPAccessQuotes(value string) string {
	if len(value) >= 2 {
		first, last := value[0], value[len(value)-1]
		if (first == '\'' || first == '"') && first == last {
			return value[1 : len(value)-1]
		}
	}
	return value
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
		} else if strings.HasPrefix(lower, "ext:") {
			return nil, fmt.Errorf("line %d: ext: IP matchers require a Daed DAT source unavailable to inbound HTTP routing", lineNumber)
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

func parseIPAccessDomainMatchers(arguments []string, lineNumber int) ([]ipAccessDomainMatcher, error) {
	matchers := make([]ipAccessDomainMatcher, 0, len(arguments))
	for _, argument := range arguments {
		kind := ipAccessDomainSuffix
		value := strings.TrimSpace(argument)
		if prefix, candidate, found := strings.Cut(value, ":"); found {
			value = trimIPAccessQuotes(strings.TrimSpace(candidate))
			switch strings.ToLower(strings.TrimSpace(prefix)) {
			case "suffix":
				kind = ipAccessDomainSuffix
			case "full":
				kind = ipAccessDomainFull
			case "keyword":
				kind = ipAccessDomainKeyword
			case "regex":
				kind = ipAccessDomainRegex
			case "geosite":
				return nil, fmt.Errorf("line %d: geosite domain matchers require a Daed geosite source unavailable to inbound HTTP routing", lineNumber)
			case "ext":
				return nil, fmt.Errorf("line %d: ext domain matchers require a Daed DAT source unavailable to inbound HTTP routing", lineNumber)
			default:
				return nil, fmt.Errorf("line %d: unsupported domain matcher %q", lineNumber, prefix)
			}
		}
		if value == "" {
			return nil, fmt.Errorf("line %d: domain() contains an empty value", lineNumber)
		}
		matcher := ipAccessDomainMatcher{kind: kind, value: strings.ToLower(value)}
		if kind == ipAccessDomainRegex {
			compiled, err := regexp.Compile(value)
			if err != nil {
				return nil, fmt.Errorf("line %d: invalid domain regex: %w", lineNumber, err)
			}
			matcher.compiled = compiled
		}
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
		if !regexp.MustCompile(`^[a-z][a-z0-9_-]*$`).MatchString(protocol) {
			return nil, fmt.Errorf("line %d: invalid layer-4 protocol %q", lineNumber, argument)
		}
		protocols[protocol] = struct{}{}
	}
	return protocols, nil
}

func parseIPAccessPorts(arguments []string, lineNumber int, predicate string) ([]ipAccessPortRange, error) {
	ports := make([]ipAccessPortRange, 0, len(arguments))
	for _, argument := range arguments {
		bounds := strings.Split(strings.TrimSpace(argument), "-")
		if len(bounds) > 2 || len(bounds) == 0 {
			return nil, fmt.Errorf("line %d: %s ports must be integers or ranges between 1 and 65535", lineNumber, predicate)
		}
		min, err := strconv.Atoi(strings.TrimSpace(bounds[0]))
		if err != nil || min < 1 || min > 65535 {
			return nil, fmt.Errorf("line %d: %s ports must be integers or ranges between 1 and 65535", lineNumber, predicate)
		}
		max := min
		if len(bounds) == 2 {
			max, err = strconv.Atoi(strings.TrimSpace(bounds[1]))
			if err != nil || max < min || max > 65535 {
				return nil, fmt.Errorf("line %d: %s ports must be integers or ranges between 1 and 65535", lineNumber, predicate)
			}
		}
		ports = append(ports, ipAccessPortRange{min: min, max: max})
	}
	return ports, nil
}

func parseIPAccessVersions(arguments []string, lineNumber int) (map[int]struct{}, error) {
	versions := make(map[int]struct{}, len(arguments))
	for _, argument := range arguments {
		version, err := strconv.Atoi(argument)
		if err != nil || (version != 4 && version != 6) {
			return nil, fmt.Errorf("line %d: ipversion must be 4 or 6", lineNumber)
		}
		versions[version] = struct{}{}
	}
	return versions, nil
}

func parseIPAccessMACs(arguments []string, lineNumber int) (map[string]struct{}, error) {
	macs := make(map[string]struct{}, len(arguments))
	for _, argument := range arguments {
		mac, err := net.ParseMAC(argument)
		if err != nil || len(mac) != 6 {
			return nil, fmt.Errorf("line %d: mac() expects a six-byte MAC address", lineNumber)
		}
		macs[strings.ToLower(mac.String())] = struct{}{}
	}
	return macs, nil
}

func parseIPAccessDSCPs(arguments []string, lineNumber int) (map[int]struct{}, error) {
	dscps := make(map[int]struct{}, len(arguments))
	for _, argument := range arguments {
		value, err := strconv.ParseInt(argument, 0, 8)
		if err != nil || value < 0 || value > 63 {
			return nil, fmt.Errorf("line %d: dscp must be an integer between 0 and 63", lineNumber)
		}
		dscps[int(value)] = struct{}{}
	}
	return dscps, nil
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
	request.ClientIP = address.String()
	request.CountryCode = countryCode
	request.L4Protocol = protocol

	ipAccessRoutingMu.RLock()
	policy := ipAccessRoutingPolicy
	ipAccessRoutingMu.RUnlock()
	for _, rule := range policy.rules {
		matched, unknown := rule.matches(request, address)
		if unknown != "" {
			return "", rule.lineNumber, fmt.Errorf("line %d cannot be evaluated: %s", rule.lineNumber, unknown)
		}
		if matched {
			return rule.action, rule.lineNumber, nil
		}
	}
	return policy.fallback, 0, nil
}

func (rule ipAccessRouteRule) matches(request IPAccessRouteRequest, clientAddress netip.Addr) (bool, string) {
	for _, condition := range rule.conditions {
		matched, known, err := condition.matches(request, clientAddress)
		if err != nil {
			return false, err.Error()
		}
		// Some Daed predicates describe local packet metadata (for example
		// pname/mac/dscp) that an HTTP edge cannot observe. Never turn an
		// unavailable condition into a match or silently fall through to a
		// permissive fallback; the policy endpoint must fail closed instead.
		if !known {
			return false, "required matcher metadata is unavailable"
		}
		if condition.negated {
			matched = !matched
		}
		if !matched {
			return false, ""
		}
	}
	return true, ""
}

func (condition ipAccessRouteCondition) matches(request IPAccessRouteRequest, clientAddress netip.Addr) (bool, bool, error) {
	switch condition.kind {
	case ipAccessConditionIP:
		address := clientAddress
		matched := false
		countryUnknown := false
		for _, matcher := range condition.ipMatchers {
			switch matcher.kind {
			case ipAccessMatcherPrefix:
				matched = matched || matcher.prefix.Contains(address)
			case ipAccessMatcherCountry:
				if request.CountryCode == "" {
					countryUnknown = true
				} else {
					matched = matched || matcher.countryCode == request.CountryCode
				}
			case ipAccessMatcherPrivate:
				matched = matched || isIPAccessPrivate(address)
			case ipAccessMatcherExternal:
				// Custom DAT files are valid Daed syntax but are not loaded by
				// the HTTP edge. Keep the condition inert until such a source
				// is explicitly provisioned.
			}
		}
		if matched {
			return true, true, nil
		}
		if countryUnknown {
			return false, true, errors.New("edge country is unavailable")
		}
		for _, matcher := range condition.ipMatchers {
			if matcher.kind == ipAccessMatcherExternal {
				return false, false, nil
			}
		}
		return false, true, nil
	case ipAccessConditionDomain:
		host := normalizeIPAccessDomain(request.Domain)
		if host == "" {
			return false, false, nil
		}
		unknown := false
		for _, matcher := range condition.domains {
			var matches bool
			switch matcher.kind {
			case ipAccessDomainSuffix:
				matches = host == matcher.value || strings.HasSuffix(host, "."+matcher.value)
			case ipAccessDomainFull:
				matches = host == matcher.value
			case ipAccessDomainKeyword:
				matches = strings.Contains(host, matcher.value)
			case ipAccessDomainRegex:
				matches = matcher.compiled.MatchString(host)
			case ipAccessDomainGeoSite, ipAccessDomainExternal:
				unknown = true
			}
			if matches {
				return true, true, nil
			}
		}
		if unknown {
			return false, false, nil
		}
		return false, true, nil
	case ipAccessConditionL4Protocol:
		if request.L4Protocol == "" {
			return false, true, errors.New("layer-4 protocol is unavailable")
		}
		_, matched := condition.values[request.L4Protocol]
		return matched, true, nil
	case ipAccessConditionPort:
		port := request.DestinationPort
		if conditionPortIsSource(condition) {
			port = request.SourcePort
		}
		if port < 1 || port > 65535 {
			if conditionPortIsSource(condition) {
				return false, true, errors.New("source port is unavailable")
			}
			return false, true, errors.New("destination port is unavailable")
		}
		for _, bounds := range condition.ports {
			if port >= bounds.min && port <= bounds.max {
				return true, true, nil
			}
		}
		return false, true, nil
	case ipAccessConditionIPVersion:
		version := 6
		if clientAddress.Is4() {
			version = 4
		}
		_, matched := condition.ipVersions[version]
		return matched, true, nil
	case ipAccessConditionMAC:
		mac := strings.TrimSpace(strings.ToLower(request.SourceMAC))
		if mac == "" {
			return false, false, nil
		}
		parsed, err := net.ParseMAC(mac)
		if err != nil || len(parsed) != 6 {
			return false, false, nil
		}
		_, matched := condition.macs[strings.ToLower(parsed.String())]
		return matched, true, nil
	case ipAccessConditionProcess:
		process := strings.ToLower(strings.TrimSpace(request.ProcessName))
		if process == "" {
			return false, false, nil
		}
		_, matched := condition.processes[process]
		return matched, true, nil
	case ipAccessConditionDSCP:
		if !request.DSCPSet && request.DSCP == 0 {
			return false, false, nil
		}
		_, matched := condition.dscps[request.DSCP]
		return matched, true, nil
	default:
		return false, false, nil
	}
}

func conditionPortIsSource(condition ipAccessRouteCondition) bool {
	// The parser stores source/destination port conditions in the same shape;
	// the direction is encoded by the first value's sentinel in this compact
	// policy representation. A source condition has no destination port list.
	return condition.ipDirection == ipAccessIPSource
}

func normalizeIPAccessDomain(raw string) string {
	host := strings.ToLower(strings.TrimSpace(raw))
	if host == "" {
		return ""
	}
	if strings.Contains(host, "://") {
		if parsed, err := neturl.Parse(host); err == nil {
			host = parsed.Hostname()
		}
	}
	if host, _, err := net.SplitHostPort(host); err == nil {
		return strings.TrimSuffix(host, ".")
	}
	return strings.TrimSuffix(host, ".")
}

func isIPAccessPrivate(address netip.Addr) bool {
	return address.IsPrivate() || address.IsLoopback() || address.IsLinkLocalUnicast() ||
		address.IsLinkLocalMulticast() || address.IsUnspecified()
}
