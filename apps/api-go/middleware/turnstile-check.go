package middleware

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/gin-gonic/gin"
)

type turnstileCheckResponse struct {
	Success    bool     `json:"success"`
	ErrorCodes []string `json:"error-codes"`
	Hostname   string   `json:"hostname"`
	Action     string   `json:"action"`
}

var turnstileVerifyURL = "https://challenges.cloudflare.com/turnstile/v0/siteverify"

var turnstileHTTPClient = &http.Client{Timeout: 10 * time.Second}

func verifyTurnstileToken(ctx context.Context, token string) (turnstileCheckResponse, error) {
	form := url.Values{
		"secret":   {common.TurnstileSecretKey},
		"response": {token},
	}
	request, err := http.NewRequestWithContext(
		ctx,
		http.MethodPost,
		turnstileVerifyURL,
		strings.NewReader(form.Encode()),
	)
	if err != nil {
		return turnstileCheckResponse{}, err
	}
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")

	response, err := turnstileHTTPClient.Do(request)
	if err != nil {
		return turnstileCheckResponse{}, err
	}
	defer response.Body.Close()
	if response.StatusCode < http.StatusOK || response.StatusCode >= http.StatusMultipleChoices {
		return turnstileCheckResponse{}, fmt.Errorf("turnstile verification returned HTTP %d", response.StatusCode)
	}

	var result turnstileCheckResponse
	if err := common.DecodeJson(response.Body, &result); err != nil {
		return turnstileCheckResponse{}, err
	}
	return result, nil
}

func TurnstileCheck() gin.HandlerFunc {
	return func(c *gin.Context) {
		if common.TurnstileCheckEnabled {
			token := strings.TrimSpace(c.Query("turnstile"))
			if token == "" {
				c.JSON(http.StatusOK, gin.H{
					"success": false,
					"message": "Turnstile token 为空",
				})
				c.Abort()
				return
			}
			result, err := verifyTurnstileToken(c.Request.Context(), token)
			if err != nil {
				common.SysLog(fmt.Sprintf("Turnstile verification request failed: %v", err))
				c.JSON(http.StatusOK, gin.H{
					"success": false,
					"message": "Turnstile 校验暂时不可用，请稍后重试！",
				})
				c.Abort()
				return
			}
			if !result.Success {
				common.SysLog(fmt.Sprintf(
					"Turnstile verification rejected: error_codes=%v hostname=%q action=%q",
					result.ErrorCodes,
					result.Hostname,
					result.Action,
				))
				c.JSON(http.StatusOK, gin.H{
					"success": false,
					"message": "Turnstile 校验失败，请刷新重试！",
				})
				c.Abort()
				return
			}
		}
		c.Next()
	}
}
