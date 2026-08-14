/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
*/

package service

import (
	"sort"
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/relaykit/dto"
)

type UserUsageRankingsResponse struct {
	Period                    string            `json:"period"`
	UpdatedAt                 int64             `json:"updated_at"`
	TotalTokens               int64             `json:"total_tokens"`
	TotalRequests             int64             `json:"total_requests"`
	ParticipantCount          int               `json:"participant_count"`
	AnonymousParticipantCount int               `json:"anonymous_participant_count"`
	Users                     []RankedUserUsage `json:"users"`
}

type RankedUserUsage struct {
	Rank        int     `json:"rank"`
	Name        string  `json:"name,omitempty"`
	Anonymous   bool    `json:"anonymous"`
	TotalTokens int64   `json:"total_tokens"`
	Requests    int64   `json:"requests"`
	Share       float64 `json:"share"`
}

type userUsageCandidate struct {
	userID    int
	name      string
	anonymous bool
	requests  int64
	tokens    int64
}

func GetUserUsageRankingsSnapshot(period string) (*UserUsageRankingsResponse, error) {
	config, err := rankingConfig(period)
	if err != nil {
		return nil, err
	}

	now := time.Now()
	startTime, endTime := rankingTimeRange(config, now)
	totals, err := model.GetUserRankingTotals(startTime, endTime)
	if err != nil {
		return nil, err
	}

	userIDs := make([]int, 0, len(totals))
	for _, total := range totals {
		userIDs = append(userIDs, total.UserID)
	}
	users, err := model.GetUsersForUsageRanking(userIDs)
	if err != nil {
		return nil, err
	}

	usersByID := make(map[int]*model.User, len(users))
	for _, user := range users {
		usersByID[user.Id] = user
	}

	candidates := make([]userUsageCandidate, 0, len(totals))
	var totalTokens int64
	var totalRequests int64
	participantCount := 0
	anonymousParticipantCount := 0

	for _, total := range totals {
		user, ok := usersByID[total.UserID]
		if !ok || user.Status != common.UserStatusEnabled {
			continue
		}

		visibility := dto.NormalizeUsageLeaderboardVisibility(user.GetSetting().UsageLeaderboardVisibility)
		if visibility == dto.UsageLeaderboardVisibilityHidden {
			continue
		}

		participantCount++
		totalTokens += total.TotalTokens
		totalRequests += total.Requests

		if visibility == dto.UsageLeaderboardVisibilityAnonymous {
			anonymousParticipantCount++
			// Anonymous visibility hides the user's name, not their independent
			// ranking row. Keep each user's totals separate so one participant
			// cannot make all anonymous usage appear as a single account.
			candidates = append(candidates, userUsageCandidate{
				userID:    user.Id,
				anonymous: true,
				requests:  total.Requests,
				tokens:    total.TotalTokens,
			})
			continue
		}

		name := strings.TrimSpace(user.DisplayName)
		if name == "" {
			name = strings.TrimSpace(user.Username)
		}
		if name == "" {
			continue
		}
		candidates = append(candidates, userUsageCandidate{
			userID:   user.Id,
			name:     name,
			requests: total.Requests,
			tokens:   total.TotalTokens,
		})
	}

	sort.SliceStable(candidates, func(i, j int) bool {
		if candidates[i].tokens != candidates[j].tokens {
			return candidates[i].tokens > candidates[j].tokens
		}
		if candidates[i].requests != candidates[j].requests {
			return candidates[i].requests > candidates[j].requests
		}
		if candidates[i].anonymous != candidates[j].anonymous {
			return !candidates[i].anonymous
		}
		if candidates[i].name != candidates[j].name {
			return candidates[i].name < candidates[j].name
		}
		return candidates[i].userID < candidates[j].userID
	})

	rows := make([]RankedUserUsage, 0, minInt(len(candidates), rankingLeaderboardLimit))
	for index, candidate := range candidates {
		if index >= rankingLeaderboardLimit {
			break
		}
		share := 0.0
		if totalTokens > 0 {
			share = float64(candidate.tokens) / float64(totalTokens)
		}
		rows = append(rows, RankedUserUsage{
			Rank:        index + 1,
			Name:        candidate.name,
			Anonymous:   candidate.anonymous,
			TotalTokens: candidate.tokens,
			Requests:    candidate.requests,
			Share:       share,
		})
	}

	return &UserUsageRankingsResponse{
		Period:                    config.id,
		UpdatedAt:                 now.Unix(),
		TotalTokens:               totalTokens,
		TotalRequests:             totalRequests,
		ParticipantCount:          participantCount,
		AnonymousParticipantCount: anonymousParticipantCount,
		Users:                     rows,
	}, nil
}
