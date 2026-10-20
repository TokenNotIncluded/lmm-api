package assistant

import (
	"bytes"
	"encoding/json"
	"testing"
)

func TestAgentDiskPreservesExactHashedJSONBytes(t *testing.T) {
	args := json.RawMessage("{ \"z\" : 1, \"a\" : {\"x\": 2} }")
	result := json.RawMessage("{ \"response\" : [1, 2], \"ok\" : true }")
	call := AgentToolCall{OperationID: "original", Arguments: args, ArgumentsSHA256: agentDigest(args)}
	turn := &AgentStoredTurn{Tools: map[string]*AgentStoredTool{"original": {Call: call, Result: &AgentToolResult{ResultJSON: result, ResultSHA256: agentDigest(result)}}}, Snapshot: &AgentTurnSnapshot{PendingTools: []AgentToolCall{call}}}
	raw, err := encodeAgentDiskTurn(turn)
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := decodeAgentDiskTurn(raw)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded.Tools["original"].Call.Arguments, args) || !bytes.Equal(decoded.Snapshot.PendingTools[0].Arguments, args) || !bytes.Equal(decoded.Tools["original"].Result.ResultJSON, result) {
		t.Fatal("hashed JSON was normalized")
	}
	if agentDigest(decoded.Tools["original"].Call.Arguments) != call.ArgumentsSHA256 || agentDigest(decoded.Tools["original"].Result.ResultJSON) != turn.Tools["original"].Result.ResultSHA256 {
		t.Fatal("digest changed after disk round trip")
	}
}
