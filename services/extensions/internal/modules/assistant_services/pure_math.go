// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
package assistant_services

import (
	"bytes"
	"encoding/json"
	"errors"
	"github.com/expr-lang/expr"
	"io"
	"math"
	"reflect"
	"regexp"
	"strings"
)

const assistantToolArgumentsMaxBytes = 16 * 1024
const assistantMathExpressionMaxBytes = 512
const assistantMathVariablesMax = 32

var assistantMathExpressionPattern = regexp.MustCompile(`^[0-9A-Za-z_+\-*/%^().,\s]+$`)
var assistantMathVariablePattern = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]{0,31}$`)
var pureMathCallPattern = regexp.MustCompile(`([A-Za-z_][A-Za-z0-9_]*)\s*\(`)

func pureInputString(input map[string]any, key string) string { s, _ := input[key].(string); return s }
func pureInputNumber(input map[string]any, key string) (float64, bool) {
	n, ok := input[key].(float64)
	return n, ok && !math.IsNaN(n) && !math.IsInf(n, 0)
}

// ExecutePure runs only deterministic tools. Validation failures retain the historical tool result envelope.
func ExecutePure(name string, args json.RawMessage) (json.RawMessage, error) {
	if len(args) == 0 || len(args) > assistantToolArgumentsMaxBytes {
		return nil, errors.New("tool arguments exceed bounded JSON object")
	}
	var input map[string]any
	decoder := json.NewDecoder(bytes.NewReader(args))
	if err := decoder.Decode(&input); err != nil || input == nil {
		return nil, errors.New("tool arguments must be a JSON object")
	}
	var extra any
	if decoder.Decode(&extra) != io.EOF {
		return nil, errors.New("tool arguments have trailing content")
	}
	if assistantVisualizationKind(name) != "" {
		visual, err := parseAssistantVisualization(name, input)
		if err != nil {
			return json.Marshal(map[string]any{"ok": false, "status": "invalid_visualization", "error": err.Error()})
		}
		return json.Marshal(map[string]any{"ok": true, "visualization": visual, "message": "Visualization shown in chat. This display does not verify the supplied data or perform any account action."})
	}
	if name == "calculate_math" {
		for key := range input {
			if key != "expression" && key != "variables" {
				return json.Marshal(map[string]any{"ok": false, "error": "unsupported math argument"})
			}
		}
		return json.Marshal(executeAssistantMathTool(input))
	}
	return nil, errors.New("unknown pure tool")
}
func executeAssistantMathTool(input map[string]any) map[string]any {
	expression := strings.TrimSpace(pureInputString(input, "expression"))
	if expression == "" {
		return map[string]any{"ok": false, "error": "a math expression is required"}
	}
	if len(expression) > assistantMathExpressionMaxBytes || !assistantMathExpressionPattern.MatchString(expression) || strings.Contains(expression, "..") {
		return map[string]any{"ok": false, "error": "expression contains unsupported characters or is too long"}
	}

	environment := map[string]any{
		"pi":  math.Pi,
		"e":   math.E,
		"abs": math.Abs, "sqrt": math.Sqrt, "cbrt": math.Cbrt,
		"pow": math.Pow, "exp": math.Exp, "ln": math.Log, "log10": math.Log10,
		"sin": math.Sin, "cos": math.Cos, "tan": math.Tan,
		"asin": math.Asin, "acos": math.Acos, "atan": math.Atan, "atan2": math.Atan2,
		"hypot": math.Hypot, "floor": math.Floor, "ceil": math.Ceil,
		"round": math.Round, "trunc": math.Trunc, "min": math.Min, "max": math.Max,
		"percent": func(value float64) float64 { return value / 100 },
		"clamp": func(value, minimum, maximum float64) float64 {
			return math.Min(math.Max(value, minimum), maximum)
		},
	}
	variables := map[string]float64{}
	for _, call := range pureMathCallPattern.FindAllStringSubmatch(expression, -1) {
		function, exists := environment[call[1]]
		if !exists || reflect.TypeOf(function).Kind() != reflect.Func {
			return map[string]any{"ok": false, "error": "invalid math expression"}
		}
	}
	if rawVariables, exists := input["variables"]; exists {
		variableMap, ok := rawVariables.(map[string]any)
		if !ok || len(variableMap) > assistantMathVariablesMax {
			return map[string]any{"ok": false, "error": "variables must be an object with at most 32 numeric entries"}
		}
		for name := range variableMap {
			if !assistantMathVariablePattern.MatchString(name) {
				return map[string]any{"ok": false, "error": "variable names must be simple ASCII identifiers"}
			}
			if _, reserved := environment[name]; reserved {
				return map[string]any{"ok": false, "error": "variable name conflicts with a math function or constant"}
			}
			value, ok := pureInputNumber(variableMap, name)
			if !ok {
				return map[string]any{"ok": false, "error": "all variables must be finite numbers"}
			}
			variables[name] = value
			environment[name] = value
		}
	}

	// The historical character allowlist alone still admitted expr builtins
	// such as sum(1..1000000000). Expose only the explicit scalar functions.
	program, err := expr.Compile(expression, expr.Env(environment), expr.AsFloat64(), expr.DisableAllBuiltins(), expr.MaxNodes(256))
	if err != nil {
		return map[string]any{"ok": false, "error": "invalid math expression"}
	}
	output, err := expr.Run(program, environment)
	if err != nil {
		return map[string]any{"ok": false, "error": "math expression could not be evaluated"}
	}
	result, ok := output.(float64)
	if !ok || math.IsNaN(result) || math.IsInf(result, 0) {
		return map[string]any{"ok": false, "error": "math result is not a finite number"}
	}
	return map[string]any{
		"ok":         true,
		"expression": expression,
		"variables":  variables,
		"result":     result,
	}
}
