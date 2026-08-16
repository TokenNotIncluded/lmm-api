/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/

package model

import (
	"fmt"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/require"
)

func TestSearchChannelsPageFiltersBeforePagination(t *testing.T) {
	truncateTables(t)

	channels := []Channel{
		{Name: "needle-active-a", Status: common.ChannelStatusEnabled, Type: 1, Models: "gpt-5", Group: "default"},
		{Name: "needle-disabled", Status: common.ChannelStatusManuallyDisabled, Type: 1, Models: "gpt-5", Group: "default"},
		{Name: "needle-active-b", Status: common.ChannelStatusEnabled, Type: 2, Models: "gpt-5", Group: "default"},
		{Name: "other", Status: common.ChannelStatusEnabled, Type: 2, Models: "gpt-5", Group: "default"},
	}
	for i := 0; i < 101; i++ {
		channels = append(channels, Channel{
			Name:   fmt.Sprintf("needle-bulk-%03d", i),
			Status: common.ChannelStatusEnabled,
			Type:   1,
			Models: "gpt-5",
			Group:  "default",
		})
	}
	require.NoError(t, DB.Create(&channels).Error)

	page, err := SearchChannelsPage(
		"needle",
		"",
		"",
		common.ChannelStatusEnabled,
		-1,
		0,
		1,
		false,
		NewChannelSortOptions("id", "asc", false),
	)
	require.NoError(t, err)
	require.Equal(t, int64(103), page.Total)
	require.Equal(t, map[int64]int64{1: 102, 2: 1}, page.TypeCounts)
	require.Len(t, page.Channels, 1)
	require.Equal(t, "needle-active-a", page.Channels[0].Name)

	page, err = SearchChannelsPage(
		"needle",
		"",
		"",
		common.ChannelStatusEnabled,
		2,
		0,
		10000,
		false,
		NewChannelSortOptions("id", "asc", false),
	)
	require.NoError(t, err)
	require.Equal(t, int64(1), page.Total)
	require.Len(t, page.Channels, 1)
	require.Equal(t, "needle-active-b", page.Channels[0].Name)

	page, err = SearchChannelsPage(
		"needle-bulk",
		"",
		"",
		common.ChannelStatusEnabled,
		-1,
		0,
		10000,
		false,
		NewChannelSortOptions("id", "asc", false),
	)
	require.NoError(t, err)
	require.Equal(t, int64(101), page.Total)
	require.Len(t, page.Channels, channelSearchMaxPageSize)
}
