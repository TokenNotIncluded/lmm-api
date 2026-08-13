package controller

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/QuantumNous/new-api/setting/console_setting"

	"github.com/gin-gonic/gin"
	"golang.org/x/sync/errgroup"
)

const (
	requestTimeout         = 30 * time.Second
	httpTimeout            = 10 * time.Second
	uptimeResponseMaxBytes = 1 << 20
	uptimeGroupConcurrency = 4
	uptimeKeySuffix        = "_24"
	apiStatusPath          = "/api/status-page/"
	apiHeartbeatPath       = "/api/status-page/heartbeat/"
)

var (
	errUptimeInvalidConfiguration = errors.New("invalid uptime monitor configuration")
	errUptimeUpstreamTimeout      = errors.New("uptime monitor request timed out")
	errUptimeUpstreamHTTP         = errors.New("uptime monitor returned a non-success status")
	errUptimeInvalidResponse      = errors.New("uptime monitor returned an invalid response")
	errUptimeResponseTooLarge     = errors.New("uptime monitor response exceeded the byte limit")
	errUptimeUpstreamUnavailable  = errors.New("uptime monitor is unavailable")
)

type Monitor struct {
	Name   string  `json:"name"`
	Uptime float64 `json:"uptime"`
	Status int     `json:"status"`
	Group  string  `json:"group,omitempty"`
}

type UptimeGroupResult struct {
	CategoryName string    `json:"categoryName"`
	Monitors     []Monitor `json:"monitors"`
	Degraded     bool      `json:"degraded,omitempty"`
	ErrorCode    string    `json:"error_code,omitempty"`
}

func getAndDecode(ctx context.Context, client *http.Client, url string, dest interface{}) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return err
	}

	resp, err := client.Do(req)
	if err != nil {
		if errors.Is(ctx.Err(), context.DeadlineExceeded) {
			return fmt.Errorf("%w: %v", errUptimeUpstreamTimeout, err)
		}
		var timeoutError interface{ Timeout() bool }
		if errors.As(err, &timeoutError) && timeoutError.Timeout() {
			return fmt.Errorf("%w: %v", errUptimeUpstreamTimeout, err)
		}
		return fmt.Errorf("%w: %v", errUptimeUpstreamUnavailable, err)
	}
	defer resp.Body.Close()

	if resp.StatusCode < http.StatusOK || resp.StatusCode >= http.StatusMultipleChoices {
		return fmt.Errorf("%w: status %d", errUptimeUpstreamHTTP, resp.StatusCode)
	}

	if resp.ContentLength > uptimeResponseMaxBytes {
		return errUptimeResponseTooLarge
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, uptimeResponseMaxBytes+1))
	if err != nil {
		return fmt.Errorf("%w: %v", errUptimeUpstreamUnavailable, err)
	}
	if len(body) > uptimeResponseMaxBytes {
		return errUptimeResponseTooLarge
	}
	if err := json.Unmarshal(body, dest); err != nil {
		return fmt.Errorf("%w: %v", errUptimeInvalidResponse, err)
	}
	return nil
}

func fetchGroupData(ctx context.Context, client *http.Client, groupConfig map[string]interface{}) (UptimeGroupResult, error) {
	url, _ := groupConfig["url"].(string)
	slug, _ := groupConfig["slug"].(string)
	categoryName, _ := groupConfig["categoryName"].(string)

	result := UptimeGroupResult{
		CategoryName: categoryName,
		Monitors:     []Monitor{},
	}

	if url == "" || slug == "" {
		return result, errUptimeInvalidConfiguration
	}

	baseURL := strings.TrimSuffix(url, "/")

	var statusData struct {
		PublicGroupList []struct {
			ID          int    `json:"id"`
			Name        string `json:"name"`
			MonitorList []struct {
				ID   int    `json:"id"`
				Name string `json:"name"`
			} `json:"monitorList"`
		} `json:"publicGroupList"`
	}

	var heartbeatData struct {
		HeartbeatList map[string][]struct {
			Status int `json:"status"`
		} `json:"heartbeatList"`
		UptimeList map[string]float64 `json:"uptimeList"`
	}

	g, gCtx := errgroup.WithContext(ctx)
	g.Go(func() error {
		return getAndDecode(gCtx, client, baseURL+apiStatusPath+slug, &statusData)
	})
	g.Go(func() error {
		return getAndDecode(gCtx, client, baseURL+apiHeartbeatPath+slug, &heartbeatData)
	})

	if err := g.Wait(); err != nil {
		return result, err
	}
	// A syntactically valid but unrelated JSON object must not be interpreted as
	// an empty, healthy status page. Empty arrays/maps are valid; missing fields
	// are not.
	if statusData.PublicGroupList == nil || heartbeatData.HeartbeatList == nil || heartbeatData.UptimeList == nil {
		return result, errUptimeInvalidResponse
	}

	for _, pg := range statusData.PublicGroupList {
		if len(pg.MonitorList) == 0 {
			continue
		}

		for _, m := range pg.MonitorList {
			monitor := Monitor{
				Name:  m.Name,
				Group: pg.Name,
			}

			monitorID := strconv.Itoa(m.ID)

			if uptime, exists := heartbeatData.UptimeList[monitorID+uptimeKeySuffix]; exists {
				monitor.Uptime = uptime
			}

			if heartbeats, exists := heartbeatData.HeartbeatList[monitorID]; exists && len(heartbeats) > 0 {
				monitor.Status = heartbeats[0].Status
			}

			result.Monitors = append(result.Monitors, monitor)
		}
	}

	return result, nil
}

func uptimeErrorCode(err error) string {
	switch {
	case errors.Is(err, errUptimeInvalidConfiguration):
		return "invalid_configuration"
	case errors.Is(err, errUptimeUpstreamTimeout):
		return "upstream_timeout"
	case errors.Is(err, errUptimeUpstreamHTTP):
		return "upstream_http_error"
	case errors.Is(err, errUptimeInvalidResponse):
		return "invalid_upstream_response"
	case errors.Is(err, errUptimeResponseTooLarge):
		return "upstream_response_too_large"
	default:
		return "upstream_unavailable"
	}
}

func serveUptimeKumaStatus(c *gin.Context, groups []map[string]interface{}, client *http.Client, timeout time.Duration) {
	if len(groups) == 0 {
		c.JSON(http.StatusOK, gin.H{"success": true, "message": "", "data": []UptimeGroupResult{}})
		return
	}

	ctx, cancel := context.WithTimeout(c.Request.Context(), timeout)
	defer cancel()

	results := make([]UptimeGroupResult, len(groups))
	errs := make([]error, len(groups))

	g, gCtx := errgroup.WithContext(ctx)
	// Uptime configuration allows up to 20 groups and each group issues two
	// bounded requests. Keep only four groups in flight so the response byte
	// limits also form a predictable aggregate memory budget.
	g.SetLimit(uptimeGroupConcurrency)
	for i, group := range groups {
		i, group := i, group
		g.Go(func() error {
			results[i], errs[i] = fetchGroupData(gCtx, client, group)
			return nil
		})
	}

	_ = g.Wait()
	failed := 0
	for i, err := range errs {
		if err == nil {
			continue
		}
		failed++
		results[i].Degraded = true
		results[i].ErrorCode = uptimeErrorCode(err)
	}

	if failed == 0 {
		c.JSON(http.StatusOK, gin.H{"success": true, "message": "", "data": results})
		return
	}

	status := http.StatusOK
	message := "uptime monitoring data is partially unavailable"
	if failed == len(results) {
		status = http.StatusBadGateway
		message = "uptime monitoring data is temporarily unavailable"
	}
	c.JSON(status, gin.H{
		"success":  false,
		"degraded": true,
		"message":  message,
		"data":     results,
	})
}

func GetUptimeKumaStatus(c *gin.Context) {
	serveUptimeKumaStatus(
		c,
		console_setting.GetUptimeKumaGroups(),
		&http.Client{Timeout: httpTimeout},
		requestTimeout,
	)
}
