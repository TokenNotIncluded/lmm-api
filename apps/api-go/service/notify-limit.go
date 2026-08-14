package service

import (
	"fmt"
	"strconv"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/constant"
	"github.com/LIghtJUNction/api.lmm.best/pkg/cachex"
)

// notifyLimitStore is used for in-memory rate limiting when Redis is disabled
var (
	notifyLimitStore = cachex.NewByteCache[limitCount](16_384, 4<<20, func(key string, _ limitCount) int64 {
		return int64(len(key) + 32)
	})
)

type limitCount struct {
	Count int
}

func getDuration() time.Duration {
	minute := constant.NotificationLimitDurationMinute
	return time.Duration(minute) * time.Minute
}

// CheckNotificationLimit checks if the user has exceeded their notification limit
// Returns true if the user can send notification, false if limit exceeded
func CheckNotificationLimit(userId int, notifyType string) (bool, error) {
	if common.RedisEnabled {
		return checkRedisLimit(userId, notifyType)
	}
	return checkMemoryLimit(userId, notifyType)
}

func checkRedisLimit(userId int, notifyType string) (bool, error) {
	key := fmt.Sprintf("notify_limit:%d:%s:%s", userId, notifyType, time.Now().Format("2006010215"))

	// Get current count
	count, err := common.RedisGet(key)
	if err != nil && err.Error() != "redis: nil" {
		return false, fmt.Errorf("failed to get notification count: %w", err)
	}

	// If key doesn't exist, initialize it
	if count == "" {
		err = common.RedisSet(key, "1", getDuration())
		return true, err
	}

	currentCount, _ := strconv.Atoi(count)
	limit := constant.NotifyLimitCount

	// Check if limit is already reached
	if currentCount >= limit {
		return false, nil
	}

	// Only increment if under limit
	err = common.RedisIncr(key, 1)
	if err != nil {
		return false, fmt.Errorf("failed to increment notification count: %w", err)
	}

	return true, nil
}

func checkMemoryLimit(userId int, notifyType string) (bool, error) {
	key := fmt.Sprintf("%d:%s:%s", userId, notifyType, time.Now().Format("2006010215"))
	currentLimit, stored := notifyLimitStore.Compute(key, getDuration(), func(current limitCount, _ bool) (limitCount, bool) {
		current.Count++
		return current, true
	})
	if !stored {
		return false, fmt.Errorf("notification rate limit memory budget exhausted")
	}
	return currentLimit.Count <= constant.NotifyLimitCount, nil
}
