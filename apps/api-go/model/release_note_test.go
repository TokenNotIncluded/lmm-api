package model

import (
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func setupReleaseNoteTestDB(t *testing.T) (int, int) {
	t.Helper()
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&ReleaseNote{}, &ReleaseNoteRead{}))
	users := []User{
		{Username: "release-note-user-a", Password: "password", Role: common.RoleCommonUser, AffCode: "release-note-a"},
		{Username: "release-note-user-b", Password: "password", Role: common.RoleCommonUser, AffCode: "release-note-b"},
	}
	require.NoError(t, db.Create(&users).Error)
	return users[0].Id, users[1].Id
}

func TestReleaseNotePublicationAndCrossDeviceReadState(t *testing.T) {
	userA, userB := setupReleaseNoteTestDB(t)

	first, err := PublishReleaseNote(99, "v1.2.3", "  - First release  ")
	require.NoError(t, err)
	assert.Equal(t, 1, first.Revision)
	assert.Equal(t, "- First release", first.Content)

	unread, err := GetLatestUnreadReleaseNote(userA, first.PublishedAt-1)
	require.NoError(t, err)
	assert.Nil(t, unread)

	unread, err = GetLatestUnreadReleaseNote(userA, first.PublishedAt+1)
	require.NoError(t, err)
	require.NotNil(t, unread)
	assert.Equal(t, first.Id, unread.Id)

	require.NoError(t, MarkReleaseNoteRead(userA, first.Id))
	require.NoError(t, MarkReleaseNoteRead(userA, first.Id))
	unread, err = GetLatestUnreadReleaseNote(userA, first.PublishedAt+1)
	require.NoError(t, err)
	assert.Nil(t, unread)

	// Another account still receives the same latest publication.
	unread, err = GetLatestUnreadReleaseNote(userB, first.PublishedAt+1)
	require.NoError(t, err)
	require.NotNil(t, unread)
	assert.Equal(t, first.Id, unread.Id)

	// Republishing the same version creates a new revision and re-notifies users.
	second, err := PublishReleaseNote(99, "v1.2.3", "- Corrected release notes")
	require.NoError(t, err)
	assert.Equal(t, 2, second.Revision)
	unread, err = GetLatestUnreadReleaseNote(userA, second.PublishedAt+1)
	require.NoError(t, err)
	require.NotNil(t, unread)
	assert.Equal(t, second.Id, unread.Id)

	notes, err := ListReleaseNotes(10)
	require.NoError(t, err)
	require.Len(t, notes, 2)
	assert.Equal(t, second.Id, notes[0].Id)
}

func TestReleaseNoteValidation(t *testing.T) {
	setupReleaseNoteTestDB(t)

	for _, test := range []struct {
		version string
		content string
		want    error
	}{
		{version: "", content: "notes", want: ErrReleaseNoteVersionRequired},
		{version: "v1 bad", content: "notes", want: ErrReleaseNoteVersionInvalid},
		{version: strings.Repeat("v", maxReleaseNoteVersionRunes+1), content: "notes", want: ErrReleaseNoteVersionTooLong},
		{version: "v1", content: "", want: ErrReleaseNoteContentRequired},
		{version: "v1", content: strings.Repeat("更", maxReleaseNoteContentRunes+1), want: ErrReleaseNoteContentTooLong},
	} {
		_, err := PublishReleaseNote(99, test.version, test.content)
		assert.ErrorIs(t, err, test.want)
	}
	assert.ErrorIs(t, MarkReleaseNoteRead(1, 9999), ErrReleaseNoteNotFound)
}
