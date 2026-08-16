/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/

package model

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/require"
)

func TestRefreshChannelCachePreservesAbilityGroups(t *testing.T) {
	preserveChannelTestState(t)
	db := openCacheTestDB(t, &Channel{}, &Ability{})
	DB = db
	common.MemoryCacheEnabled = true

	require.NoError(t, db.Create(&Channel{
		Id:     1201,
		Name:   "cache-group-channel",
		Status: common.ChannelStatusEnabled,
		Group:  "default",
		Models: "cache-group-model",
	}).Error)
	require.NoError(t, db.Create(&[]Ability{
		{Group: "default", Model: "cache-group-model", ChannelId: 1201, Enabled: true},
		{Group: "orphan-group", Model: "unused-model", ChannelId: 1201, Enabled: false},
	}).Error)

	require.NoError(t, refreshChannelCache())

	channelSyncLock.RLock()
	defer channelSyncLock.RUnlock()
	require.Contains(t, group2model2channels, "default")
	require.Contains(t, group2model2channels["default"], "cache-group-model")
	require.Contains(t, group2model2channels, "orphan-group")
	require.Empty(t, group2model2channels["orphan-group"])
}
