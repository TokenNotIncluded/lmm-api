package relay

import (
	"net/url"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/LIghtJUNction/api.lmm.best/setting/system_setting"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestCoverMidjourneyTaskDtoPreservesSignatureWithCacheBuster(t *testing.T) {
	previousForwardURL := setting.MjForwardUrlEnabled
	previousServerAddress := system_setting.ServerAddress
	setting.MjForwardUrlEnabled = true
	system_setting.ServerAddress = "https://example.com"
	t.Cleanup(func() {
		setting.MjForwardUrlEnabled = previousForwardURL
		system_setting.ServerAddress = previousServerAddress
	})

	task := &model.Midjourney{
		UserId:   101,
		MjId:     "unfinished-task",
		ImageUrl: "https://upstream.example/image.png",
		Status:   "IN_PROGRESS",
	}
	dto := coverMidjourneyTaskDto(nil, task)
	imageURL, err := url.Parse(dto.ImageUrl)
	require.NoError(t, err)
	signedURL, err := url.Parse(BuildMidjourneyImageURL(system_setting.ServerAddress, task.UserId, task.MjId))
	require.NoError(t, err)

	assert.Equal(t, "/mj/image/unfinished-task", imageURL.Path)
	assert.Equal(t, "101", imageURL.Query().Get("uid"))
	assert.Equal(t, signedURL.Query().Get("sig"), imageURL.Query().Get("sig"))
	assert.NotEmpty(t, imageURL.Query().Get("rand"))
}
