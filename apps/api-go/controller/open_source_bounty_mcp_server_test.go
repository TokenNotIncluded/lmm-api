package controller

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"slices"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/glebarez/sqlite"
	"github.com/modelcontextprotocol/go-sdk/mcp"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

type openSourceBountyBearerTransport struct {
	token string
}

func (transport openSourceBountyBearerTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	clone := request.Clone(request.Context())
	clone.Header = request.Header.Clone()
	clone.Header.Set("Authorization", "Bearer "+transport.token)
	return http.DefaultTransport.RoundTrip(clone)
}

func setupOpenSourceBountyMCPControllerTest(t *testing.T) (*gorm.DB, model.User, string) {
	t.Helper()
	previousDB, previousLogDB := model.DB, model.LOG_DB
	previousRedisEnabled := common.RedisEnabled
	previousMainDatabaseType, previousLogDatabaseType := common.MainDatabaseType(), common.LogDatabaseType()
	common.RedisEnabled = false
	common.SetDatabaseTypes(common.DatabaseTypeSQLite, common.DatabaseTypeSQLite)
	common.OptionMapRWMutex.Lock()
	if common.OptionMap == nil {
		common.OptionMap = make(map[string]string)
	}
	previousFeeRate, hadPreviousFeeRate := common.OptionMap[model.OpenSourceBountyFeeRateOptionKey]
	common.OptionMap[model.OpenSourceBountyFeeRateOptionKey] = "2.5"
	common.OptionMapRWMutex.Unlock()

	dsn := fmt.Sprintf("file:%s?mode=memory&cache=shared", strings.ReplaceAll(t.Name(), "/", "_"))
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{})
	require.NoError(t, err)
	model.DB, model.LOG_DB = db, db
	require.NoError(t, db.AutoMigrate(
		&model.User{}, &model.Log{}, &model.OpenSourceBountyProject{}, &model.OpenSourceBountyChallenge{},
		&model.OpenSourceBountyLedger{}, &model.OpenSourceBountyMCPToken{}, &model.OpenSourceBountyMCPConfirmation{},
		&model.OpenSourceBountyMCPOperation{},
		&model.OpenSourceBountyRESTOperation{},
		&model.OpenSourceBountyDispute{},
	))
	root := model.User{Username: "fee-recipient-root", Password: "password", AffCode: "fee-recipient-root", Quota: 0, Role: common.RoleRootUser, Status: common.UserStatusEnabled}
	require.NoError(t, db.Create(&root).Error)
	levelOne := model.TrustLevelMinUser + 1
	user := model.User{Username: "mcp-owner", Password: "password", AffCode: "mcp-owner", Quota: 10_000, Role: common.RoleCommonUser, Status: common.UserStatusEnabled, TrustLevelOverride: &levelOne}
	require.NoError(t, db.Create(&user).Error)
	token, _, err := model.RotateOpenSourceBountyMCPToken(user.Id)
	require.NoError(t, err)

	t.Cleanup(func() {
		model.DB, model.LOG_DB = previousDB, previousLogDB
		common.RedisEnabled = previousRedisEnabled
		common.SetDatabaseTypes(previousMainDatabaseType, previousLogDatabaseType)
		common.OptionMapRWMutex.Lock()
		if hadPreviousFeeRate {
			common.OptionMap[model.OpenSourceBountyFeeRateOptionKey] = previousFeeRate
		} else {
			delete(common.OptionMap, model.OpenSourceBountyFeeRateOptionKey)
		}
		common.OptionMapRWMutex.Unlock()
		sqlDB, err := db.DB()
		if err == nil {
			_ = sqlDB.Close()
		}
	})
	return db, user, token
}

func TestOpenSourceBountyMCPAuthenticationToolsAndPublishConfirmation(t *testing.T) {
	db, user, token := setupOpenSourceBountyMCPControllerTest(t)
	var root model.User
	require.NoError(t, db.Where("username = ?", "fee-recipient-root").First(&root).Error)
	server := httptest.NewServer(NewOpenSourceBountyMCPHandler())
	t.Cleanup(server.Close)

	for _, test := range []struct {
		name  string
		token string
	}{
		{name: "missing bearer"},
		{name: "invalid bearer", token: "lmm_mcp_invalid"},
	} {
		t.Run(test.name, func(t *testing.T) {
			request, err := http.NewRequest(http.MethodPost, server.URL, strings.NewReader(`{}`))
			require.NoError(t, err)
			request.Header.Set("Content-Type", "application/json")
			if test.token != "" {
				request.Header.Set("Authorization", "Bearer "+test.token)
			}
			response, err := http.DefaultClient.Do(request)
			require.NoError(t, err)
			defer response.Body.Close()
			assert.Equal(t, http.StatusUnauthorized, response.StatusCode)
		})
	}

	t.Run("valid bearer behind loopback reverse proxy", func(t *testing.T) {
		request, err := http.NewRequest(http.MethodPost, server.URL, strings.NewReader(`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"reverse-proxy-test","version":"1.0.0"}}}`))
		require.NoError(t, err)
		request.Host = "api.lmm.best"
		request.Header.Set("Authorization", "Bearer "+token)
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("Accept", "application/json, text/event-stream")

		response, err := http.DefaultClient.Do(request)
		require.NoError(t, err)
		defer response.Body.Close()
		assert.Equal(t, http.StatusOK, response.StatusCode)
	})

	ctx := context.Background()
	client := mcp.NewClient(&mcp.Implementation{Name: "open-source-bounty-test", Version: "1.0.0"}, &mcp.ClientOptions{
		MultiRoundTrip: &mcp.MultiRoundTripOptions{Disabled: true},
	})
	session, err := client.Connect(ctx, &mcp.StreamableClientTransport{
		Endpoint:             server.URL,
		HTTPClient:           &http.Client{Transport: openSourceBountyBearerTransport{token: token}},
		DisableStandaloneSSE: true,
	}, nil)
	require.NoError(t, err)
	t.Cleanup(func() { _ = session.Close() })

	tools, err := session.ListTools(ctx, nil)
	require.NoError(t, err)
	names := make([]string, 0, len(tools.Tools))
	for _, tool := range tools.Tools {
		names = append(names, tool.Name)
	}
	assert.True(t, slices.IsSorted(names), "tool discovery order must be deterministic")
	assert.Contains(t, names, "open_source_bounties.publish")
	assert.Contains(t, names, "open_source_bounties.cancel")
	assert.Contains(t, names, "open_source_bounties.tip")
	assert.Contains(t, names, "open_source_bounties.rate_owner")
	toolSchema, err := json.Marshal(tools.Tools)
	require.NoError(t, err)
	assert.NotContains(t, string(toolSchema), "promotion_quota")
	assert.NotContains(t, string(toolSchema), "encrypted_review_message")
	var submitTool *mcp.Tool
	for _, tool := range tools.Tools {
		if tool.Name == "open_source_bounties.submit" {
			submitTool = tool
			break
		}
	}
	require.NotNil(t, submitTool)
	submitSchema, ok := submitTool.InputSchema.(map[string]any)
	require.True(t, ok)
	requiredInputs, ok := submitSchema["required"].([]any)
	require.True(t, ok)
	assert.Contains(t, requiredInputs, "project_id")
	assert.NotContains(t, requiredInputs, "issue_url")
	assert.NotContains(t, requiredInputs, "pull_request_url")
	assert.Contains(t, submitTool.Description, "at least one")

	listResult, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "open_source_bounties.list", Arguments: map[string]any{"page": 1, "page_size": 20}})
	require.NoError(t, err)
	require.False(t, listResult.NeedsInput())
	assert.Equal(t, float64(0), listResult.StructuredContent.(map[string]any)["data"].(map[string]any)["total"])

	project, err := model.CreateOpenSourceBountyDraft(user.Id, model.OpenSourceBountyDraftInput{
		RepositoryUrl: "https://github.com/example/mcp-fee", Title: "Fix reproducible MCP defects",
		Description: "Find a reproducible defect and provide a focused fix with verification.",
		Rules:       "The Issue must include reproduction, expected behavior, actual behavior, impact, and the linked pull request must include verification.",
		RewardQuota: 333, RewardSlots: 3,
	})
	require.NoError(t, err)

	first, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "open_source_bounties.publish", Arguments: map[string]any{"project_id": project.Id}})
	require.NoError(t, err)
	require.True(t, first.NeedsInput())
	require.NotEmpty(t, first.RequestState)
	confirmation, ok := first.InputRequests["confirmation"].(*mcp.ElicitParams)
	require.True(t, ok)
	assert.Contains(t, confirmation.Message, "credits the public 2.50% platform fee of 27 to super administrator \"fee-recipient-root\"")
	assert.Contains(t, confirmation.Message, "Your net balance decrease is 999")
	var before model.User
	require.NoError(t, db.First(&before, user.Id).Error)
	assert.Equal(t, 10_000, before.Quota, "input-required confirmation must not debit balance")

	common.OptionMapRWMutex.Lock()
	common.OptionMap[model.OpenSourceBountyFeeRateOptionKey] = "3.0"
	common.OptionMapRWMutex.Unlock()
	stale, err := session.CallTool(ctx, &mcp.CallToolParams{
		Name: "open_source_bounties.publish", Arguments: map[string]any{"project_id": project.Id},
		InputResponses: mcp.InputResponseMap{"confirmation": &mcp.ElicitResult{Action: "accept", Content: map[string]any{"confirmed": true}}},
		RequestState:   first.RequestState,
	})
	assert.True(t, err != nil || stale.IsError, "a fee change must invalidate the exact publication confirmation")
	require.NoError(t, db.First(&before, user.Id).Error)
	assert.Equal(t, 10_000, before.Quota)
	common.OptionMapRWMutex.Lock()
	common.OptionMap[model.OpenSourceBountyFeeRateOptionKey] = "2.5"
	common.OptionMapRWMutex.Unlock()
	staleReplay, err := session.CallTool(ctx, &mcp.CallToolParams{
		Name: "open_source_bounties.publish", Arguments: map[string]any{"project_id": project.Id},
		InputResponses: mcp.InputResponseMap{"confirmation": &mcp.ElicitResult{Action: "accept", Content: map[string]any{"confirmed": true}}},
		RequestState:   first.RequestState,
	})
	assert.True(t, err != nil || staleReplay.IsError, "a stale confirmation stays invalid after the fee is restored")

	fresh, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "open_source_bounties.publish", Arguments: map[string]any{"project_id": project.Id}})
	require.NoError(t, err)
	require.True(t, fresh.NeedsInput())
	second, err := session.CallTool(ctx, &mcp.CallToolParams{
		Name: "open_source_bounties.publish", Arguments: map[string]any{"project_id": project.Id},
		InputResponses: mcp.InputResponseMap{"confirmation": &mcp.ElicitResult{Action: "accept", Content: map[string]any{"confirmed": true}}},
		RequestState:   fresh.RequestState,
	})
	require.NoError(t, err)
	require.False(t, second.NeedsInput())
	var after model.User
	require.NoError(t, db.First(&after, user.Id).Error)
	assert.Equal(t, 9_001, after.Quota, "the gross listing price is debited exactly once")
	var rootAfter model.User
	require.NoError(t, db.First(&rootAfter, root.Id).Error)
	assert.Equal(t, 27, rootAfter.Quota, "the public platform fee is credited to the super administrator exactly once")

	replayed, err := session.CallTool(ctx, &mcp.CallToolParams{
		Name: "open_source_bounties.publish", Arguments: map[string]any{"project_id": project.Id},
		InputResponses: mcp.InputResponseMap{"confirmation": &mcp.ElicitResult{Action: "accept", Content: map[string]any{"confirmed": true}}},
		RequestState:   fresh.RequestState,
	})
	require.NoError(t, err)
	assert.False(t, replayed.IsError, "a response-loss retry returns the persisted operation result")
	require.NoError(t, db.First(&after, user.Id).Error)
	assert.Equal(t, 9_001, after.Quota, "replaying confirmation cannot debit twice")
	require.NoError(t, db.First(&rootAfter, root.Id).Error)
	assert.Equal(t, 27, rootAfter.Quota, "replaying confirmation cannot credit the fee twice")

	participant := model.User{Username: "mcp-contributor", Password: "password", AffCode: "mcp-contributor", Quota: 0, Role: common.RoleCommonUser, Status: common.UserStatusEnabled}
	require.NoError(t, db.Create(&participant).Error)
	challenge, err := model.AcceptOpenSourceBounty(participant.Id, project.Id, "mcp-contributor")
	require.NoError(t, err)
	cancelParticipant := model.User{Username: "mcp-cancel-contributor", Password: "password", AffCode: "mcp-cancel-contributor", Quota: 0, Role: common.RoleCommonUser, Status: common.UserStatusEnabled}
	require.NoError(t, db.Create(&cancelParticipant).Error)
	cancelledChallenge, err := model.AcceptOpenSourceBounty(cancelParticipant.Id, project.Id, "mcp-cancel-contributor")
	require.NoError(t, err)
	cancelRequest := &mcp.CallToolParams{Name: "open_source_bounties.cancel", Arguments: map[string]any{"challenge_id": cancelledChallenge.Id}}
	cancelPending, err := session.CallTool(ctx, cancelRequest)
	require.NoError(t, err)
	require.True(t, cancelPending.NeedsInput())
	cancelConfirmed := &mcp.CallToolParams{
		Name: cancelRequest.Name, Arguments: cancelRequest.Arguments, RequestState: cancelPending.RequestState,
		InputResponses: mcp.InputResponseMap{"confirmation": &mcp.ElicitResult{Action: "accept", Content: map[string]any{"confirmed": true}}},
	}
	_, err = session.CallTool(ctx, cancelConfirmed)
	require.NoError(t, err)
	_, err = session.CallTool(ctx, cancelConfirmed)
	require.NoError(t, err, "a response-loss retry must recover the committed cancellation")
	require.NoError(t, db.First(&cancelledChallenge, cancelledChallenge.Id).Error)
	assert.Equal(t, model.OpenSourceBountyChallengeCancelled, cancelledChallenge.Status)

	tipRequest := &mcp.CallToolParams{Name: "open_source_bounties.tip", Arguments: map[string]any{
		"challenge_id": challenge.Id, "quota": 123, "note": "Thanks for the focused investigation.",
	}}
	tipPending, err := session.CallTool(ctx, tipRequest)
	require.NoError(t, err)
	require.True(t, tipPending.NeedsInput())
	tipConfirmed := &mcp.CallToolParams{
		Name: tipRequest.Name, Arguments: tipRequest.Arguments, RequestState: tipPending.RequestState,
		InputResponses: mcp.InputResponseMap{"confirmation": &mcp.ElicitResult{Action: "accept", Content: map[string]any{"confirmed": true}}},
	}
	_, err = session.CallTool(ctx, tipConfirmed)
	require.NoError(t, err)
	_, err = session.CallTool(ctx, tipConfirmed)
	require.NoError(t, err, "a response-loss retry must recover the committed tip")
	require.NoError(t, db.First(&after, user.Id).Error)
	assert.Equal(t, 8_878, after.Quota, "the same confirmed tip debits the publisher exactly once")
	var participantAfter model.User
	require.NoError(t, db.First(&participantAfter, participant.Id).Error)
	assert.Equal(t, 123, participantAfter.Quota, "the same confirmed tip credits the contributor exactly once")

	challenge, err = model.SubmitOpenSourceBountyChallenge(participant.Id, project.Id,
		"https://github.com/example/mcp-fee/issues/1", "https://github.com/example/mcp-fee/pull/2",
		"MCP replay verification.")
	require.NoError(t, err)
	approveRequest := &mcp.CallToolParams{Name: "open_source_bounties.approve", Arguments: map[string]any{
		"challenge_id": challenge.Id, "review_note": "Verified and approved.", "rating_score": 5, "rating_comment": "Focused fix with clear verification.",
	}}
	approvePending, err := session.CallTool(ctx, approveRequest)
	require.NoError(t, err)
	require.True(t, approvePending.NeedsInput())
	approveConfirmed := &mcp.CallToolParams{
		Name: approveRequest.Name, Arguments: approveRequest.Arguments, RequestState: approvePending.RequestState,
		InputResponses: mcp.InputResponseMap{"confirmation": &mcp.ElicitResult{Action: "accept", Content: map[string]any{"confirmed": true}}},
	}
	_, err = session.CallTool(ctx, approveConfirmed)
	require.NoError(t, err)
	_, err = session.CallTool(ctx, approveConfirmed)
	require.NoError(t, err, "a response-loss retry must recover the committed reward payment")
	require.NoError(t, db.First(&participantAfter, participant.Id).Error)
	assert.Equal(t, 447, participantAfter.Quota, "the net reward and tip each transfer exactly once")

	closeRequest := &mcp.CallToolParams{Name: "open_source_bounties.close", Arguments: map[string]any{"project_id": project.Id}}
	closePending, err := session.CallTool(ctx, closeRequest)
	require.NoError(t, err)
	require.True(t, closePending.NeedsInput())
	closeConfirmed := &mcp.CallToolParams{
		Name: closeRequest.Name, Arguments: closeRequest.Arguments, RequestState: closePending.RequestState,
		InputResponses: mcp.InputResponseMap{"confirmation": &mcp.ElicitResult{Action: "accept", Content: map[string]any{"confirmed": true}}},
	}
	_, err = session.CallTool(ctx, closeConfirmed)
	require.NoError(t, err)
	_, err = session.CallTool(ctx, closeConfirmed)
	require.NoError(t, err, "a response-loss retry must recover the committed escrow refund")
	require.NoError(t, db.First(&after, user.Id).Error)
	assert.Equal(t, 9_526, after.Quota, "the remaining net escrow refund is credited exactly once")
}

func TestOpenSourceBountyMCPRejectsTokenAfterDeveloperAccessDowngrade(t *testing.T) {
	db, user, token := setupOpenSourceBountyMCPControllerTest(t)
	server := httptest.NewServer(NewOpenSourceBountyMCPHandler())
	t.Cleanup(server.Close)

	require.NoError(t, db.Model(&model.User{}).Where("id = ?", user.Id).Updates(map[string]any{
		"trust_level_override": model.TrustLevelMinUser,
		"auth_version":         gorm.Expr("auth_version + 1"),
	}).Error)
	call := func() int {
		request, err := http.NewRequest(http.MethodPost, server.URL, strings.NewReader(`{}`))
		require.NoError(t, err)
		request.Header.Set("Authorization", "Bearer "+token)
		request.Header.Set("Content-Type", "application/json")
		response, err := http.DefaultClient.Do(request)
		require.NoError(t, err)
		defer response.Body.Close()
		return response.StatusCode
	}
	assert.Equal(t, http.StatusUnauthorized, call(), "the first MCP call after downgrade must reject the old token")

	require.NoError(t, db.Model(&model.User{}).Where("id = ?", user.Id).
		Update("trust_level_override", model.TrustLevelMinUser+1).Error)
	assert.Equal(t, http.StatusUnauthorized, call(), "restoring L1 must not revive the downgraded token")
}
