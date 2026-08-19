package controller

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/constant"
	"github.com/LIghtJUNction/api.lmm.best/middleware"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
	"github.com/modelcontextprotocol/go-sdk/auth"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

const (
	drawingMCPMaxPromptRunes = assistantDrawingPromptMaxRunes
	drawingMCPMaxImages      = assistantDrawingMaxImages
	drawingMCPMaxOptionRunes = 64
)

type drawingMCPGenerateInput struct {
	Prompt  string `json:"prompt" jsonschema:"Image description, 1 to 2000 characters"`
	Group   string `json:"group,omitempty" jsonschema:"Optional exact routing group; defaults to image-2 when available"`
	Model   string `json:"model,omitempty" jsonschema:"Optional exact image model; defaults to image-2 or the first available model"`
	Size    string `json:"size,omitempty" jsonschema:"Optional model-supported image size"`
	Quality string `json:"quality,omitempty" jsonschema:"Optional model-supported image quality"`
	N       int    `json:"n,omitempty" jsonschema:"Number of images, from 1 to 4"`
}

type drawingMCPGroup struct {
	Name        string   `json:"name"`
	Description string   `json:"description,omitempty"`
	Models      []string `json:"models"`
}

type drawingMCPOutput struct {
	Message string `json:"message"`
	Data    any    `json:"data,omitempty"`
}

func drawingMCPTool(name, title, description string, readOnly, destructive, idempotent bool) *mcp.Tool {
	return &mcp.Tool{
		Name: name, Title: title, Description: description,
		Annotations: &mcp.ToolAnnotations{
			Title: title, ReadOnlyHint: readOnly,
			DestructiveHint: bountyMCPBool(destructive),
			IdempotentHint:  idempotent,
			OpenWorldHint:   bountyMCPBool(false),
		},
	}
}

func drawingMCPAuthorizedUser(userID int) (*model.UserBase, error) {
	if !common.DrawingEnabled {
		return nil, errors.New("drawing is currently disabled")
	}
	user, err := model.GetUserCache(userID)
	if err != nil {
		return nil, errors.New("the authenticated account is unavailable")
	}
	access, err := model.GetDeveloperAccessStateForUserBase(user)
	if err != nil || !access.Granted {
		return nil, errors.New("developer access is required for the drawing workbench")
	}
	return user, nil
}

func drawingMCPGroups(userGroup string) []drawingMCPGroup {
	groups, imageModels := assistantDrawingCatalog(userGroup)
	names := make([]string, 0, len(groups))
	for name := range groups {
		names = append(names, name)
	}
	slices.Sort(names)
	result := make([]drawingMCPGroup, 0, len(names))
	for _, name := range names {
		result = append(result, drawingMCPGroup{
			Name: name, Description: groups[name],
			Models: assistantDrawingModelsForGroup(name, imageModels),
		})
	}
	return result
}

func drawingMCPResolveInput(user *model.UserBase, input drawingMCPGenerateInput) (drawingMCPGenerateInput, error) {
	input.Prompt = strings.TrimSpace(input.Prompt)
	if input.Prompt == "" || len([]rune(input.Prompt)) > drawingMCPMaxPromptRunes {
		return drawingMCPGenerateInput{}, errors.New("prompt must contain 1 to 2000 characters")
	}
	input.Group = strings.TrimSpace(input.Group)
	input.Model = strings.TrimSpace(input.Model)
	input.Size = strings.TrimSpace(input.Size)
	input.Quality = strings.TrimSpace(input.Quality)
	if len([]rune(input.Group)) > drawingMCPMaxOptionRunes || len([]rune(input.Model)) > drawingMCPMaxOptionRunes || len([]rune(input.Size)) > drawingMCPMaxOptionRunes || len([]rune(input.Quality)) > drawingMCPMaxOptionRunes {
		return drawingMCPGenerateInput{}, errors.New("group, model, size, and quality must be 64 characters or fewer")
	}
	if input.N == 0 {
		input.N = 1
	}
	if input.N < 1 || input.N > drawingMCPMaxImages {
		return drawingMCPGenerateInput{}, errors.New("image count must be between 1 and 4")
	}

	groups, imageModels := assistantDrawingCatalog(user.Group)
	if input.Group == "" {
		if _, ok := groups["image-2"]; ok {
			input.Group = "image-2"
		} else {
			names := make([]string, 0, len(groups))
			for name := range groups {
				names = append(names, name)
			}
			slices.Sort(names)
			return drawingMCPGenerateInput{}, fmt.Errorf("group is required; available groups: %s", strings.Join(names, ", "))
		}
	}
	if _, ok := groups[input.Group]; !ok {
		return drawingMCPGenerateInput{}, errors.New("the selected routing group is not available to this account")
	}
	models := assistantDrawingModelsForGroup(input.Group, imageModels)
	if len(models) == 0 {
		return drawingMCPGenerateInput{}, errors.New("the selected group has no image-capable models")
	}
	if input.Model == "" {
		if slices.Contains(models, "image-2") {
			input.Model = "image-2"
		} else {
			input.Model = models[0]
		}
	}
	if !slices.Contains(models, input.Model) {
		return drawingMCPGenerateInput{}, errors.New("the selected image model is not available in this group")
	}
	return input, nil
}

func registerDrawingMCPTools(server *mcp.Server) {
	mcp.AddTool(server, drawingMCPTool(
		"drawing.list_capabilities", "List drawing capabilities",
		"List the authenticated developer's currently usable image groups and image-capable models. This is read-only and never spends quota.",
		true, false, true,
	), func(ctx context.Context, request *mcp.CallToolRequest, input struct{}) (*mcp.CallToolResult, drawingMCPOutput, error) {
		userID, err := bountyMCPUserId(request)
		if err != nil {
			return nil, drawingMCPOutput{}, err
		}
		user, err := drawingMCPAuthorizedUser(userID)
		if err != nil {
			return nil, drawingMCPOutput{}, err
		}
		return nil, drawingMCPOutput{
			Message: "Drawing capabilities loaded.",
			Data: map[string]any{
				"enabled":        true,
				"groups":         drawingMCPGroups(user.Group),
				"max_images":     drawingMCPMaxImages,
				"max_prompt_len": drawingMCPMaxPromptRunes,
				"endpoint":       "/mcp/drawing",
			},
		}, nil
	})

	mcp.AddTool(server, drawingMCPTool(
		"drawing.generate", "Generate an image",
		"Generate images through the same group-aware, quota-billed drawing relay used by the web workbench. The first call always asks for explicit confirmation of the prompt, model, group, image count, and billing impact.",
		false, true, false,
	), func(ctx context.Context, request *mcp.CallToolRequest, input drawingMCPGenerateInput) (*mcp.CallToolResult, drawingMCPOutput, error) {
		userID, err := bountyMCPUserId(request)
		if err != nil {
			return nil, drawingMCPOutput{}, err
		}
		user, err := drawingMCPAuthorizedUser(userID)
		if err != nil {
			return nil, drawingMCPOutput{}, err
		}
		resolved, err := drawingMCPResolveInput(user, input)
		if err != nil {
			return nil, drawingMCPOutput{}, err
		}
		confirmationPayload := map[string]any{"input": resolved, "user_id": userID}
		message := fmt.Sprintf(
			"Generate %d image(s) with model %q in group %q for prompt %q. This uses the selected group's normal quota and may incur charges. Continue?",
			resolved.N, resolved.Model, resolved.Group, resolved.Prompt,
		)
		pending, _, err := bountyMCPConfirmedOperation(request, userID, "drawing.generate", confirmationPayload, message)
		if err != nil || pending != nil {
			return pending, drawingMCPOutput{}, err
		}
		result, err := executeDrawingMCPRelay(ctx, user, resolved)
		if err != nil {
			return nil, drawingMCPOutput{}, err
		}
		return nil, drawingMCPOutput{Message: "Image generation completed.", Data: result}, nil
	})
}

func executeDrawingMCPRelay(ctx context.Context, user *model.UserBase, input drawingMCPGenerateInput) (map[string]any, error) {
	payload := map[string]any{
		"prompt": input.Prompt, "model": input.Model, "n": input.N,
	}
	if input.Size != "" {
		payload["size"] = input.Size
	}
	if input.Quality != "" {
		payload["quality"] = input.Quality
	}
	body, err := json.Marshal(payload)
	if err != nil {
		return nil, errors.New("image request could not be encoded")
	}
	request := httptest.NewRequest(http.MethodPost, "/pg/images/generations?group="+url.QueryEscape(input.Group), bytes.NewReader(body))
	request = request.WithContext(ctx)
	recorder := httptest.NewRecorder()
	engine := gin.New()
	engine.Use(func(c *gin.Context) {
		c.Set("id", user.Id)
		c.Set("use_access_token", false)
		c.Set("username", user.Username)
		c.Set("role", user.Role)
		c.Set("group", user.Group)
		user.WriteContext(c)
		c.Set(common.RequestIdKey, common.NewRequestId())
		common.SetContextKey(c, constant.ContextKeyRequestStartTime, time.Now())
		c.Next()
	})
	engine.POST("/pg/images/generations", middleware.Distribute(), PlaygroundImage)
	engine.ServeHTTP(recorder, request)
	if recorder.Code < http.StatusOK || recorder.Code >= http.StatusMultipleChoices {
		var failure struct {
			Error struct {
				Message string `json:"message"`
			} `json:"error"`
		}
		_ = json.Unmarshal(recorder.Body.Bytes(), &failure)
		if failure.Error.Message != "" {
			return nil, errors.New(failure.Error.Message)
		}
		return nil, fmt.Errorf("image relay failed with HTTP %d", recorder.Code)
	}
	var result map[string]any
	if err := json.Unmarshal(recorder.Body.Bytes(), &result); err != nil {
		return nil, errors.New("image relay returned an invalid response")
	}
	if failure, ok := result["error"].(map[string]any); ok {
		if message, ok := failure["message"].(string); ok && strings.TrimSpace(message) != "" {
			return nil, errors.New(message)
		}
		return nil, errors.New("image relay returned an error")
	}
	return result, nil
}

func newDrawingMCPServer() *mcp.Server {
	server := mcp.NewServer(&mcp.Implementation{
		Name: "api.lmm.best-drawing", Version: common.Version,
	}, &mcp.ServerOptions{
		Instructions: "Use this MCP server for the authenticated developer's drawing workbench. Call drawing.list_capabilities before selecting a group or model. Never invent a group or model. drawing.generate uses the same safe, group-aware relay and normal quota billing as the web workbench; it always asks for explicit confirmation before generation. Do not retry a confirmed generation automatically because it may spend quota again.",
		Capabilities: &mcp.ServerCapabilities{},
	})
	server.AddPrompt(&mcp.Prompt{
		Name: "drawing_operator", Title: "Drawing workbench operator",
		Description: "Instructions for generating images through the authenticated drawing MCP.",
	}, func(ctx context.Context, request *mcp.GetPromptRequest) (*mcp.GetPromptResult, error) {
		return &mcp.GetPromptResult{
			Description: "Operate the authenticated drawing workbench.",
			Messages:    []*mcp.PromptMessage{{Role: "user", Content: &mcp.TextContent{Text: "Use drawing.list_capabilities first. Select only an exact available group and image model. Before drawing, show the full prompt, model, group, image count, and expected billing impact, then continue only after I explicitly confirm. Do not retry a confirmed generation automatically."}}},
		}, nil
	})
	registerDrawingMCPTools(server)
	return server
}

// NewDrawingMCPHandler exposes the drawing-only Streamable HTTP MCP server.
// It intentionally reuses the personal developer MCP token, while keeping
// drawing tools on a separate endpoint and scope from bounty tools.
func NewDrawingMCPHandler() http.Handler {
	server := newDrawingMCPServer()
	streamable := mcp.NewStreamableHTTPHandler(func(request *http.Request) *mcp.Server {
		return server
	}, &mcp.StreamableHTTPOptions{
		Stateless: true, JSONResponse: true,
		DisableLocalhostProtection: true, PropagateRequestCancellation: true,
	})
	verifier := func(ctx context.Context, token string, request *http.Request) (*auth.TokenInfo, error) {
		userID, err := model.VerifyOpenSourceBountyMCPToken(token)
		if err != nil {
			return nil, fmt.Errorf("%w: invalid personal MCP token", auth.ErrInvalidToken)
		}
		return &auth.TokenInfo{
			UserID: strconv.Itoa(userID), Scopes: []string{"drawing:read", "drawing:write"},
			Extra: map[string]any{"protocol_version": openSourceBountyMCPProtocolVersion},
		}, nil
	}
	return auth.RequireBearerToken(verifier, &auth.RequireBearerTokenOptions{
		Scopes: []string{"drawing:read", "drawing:write"}, AllowMissingExpiration: true,
	})(streamable)
}
