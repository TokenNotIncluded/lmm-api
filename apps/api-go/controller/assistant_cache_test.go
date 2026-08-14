package controller

import (
	"context"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/stretchr/testify/require"
)

func TestAssistantCacheBypassesPersonalSkillQuestions(t *testing.T) {
	settings := setting.GetAssistantSettings()
	settings.CacheEnabled = true
	settings.CacheTTLMinutes = 10
	context := assistantUserContext{UserID: 7, AccessLevel: "L0"}

	for _, question := range []string{
		"你还记得我的偏好吗？",
		"请回忆一下我的项目",
		"what do you remember about my setup?",
		"please update my profile tags",
	} {
		t.Run(question, func(t *testing.T) {
			require.Empty(t, assistantCacheKey(settings, []assistantOpenAIMessage{{Role: "user", Content: question}}, context))
		})
	}

	key := assistantCacheKey(settings, []assistantOpenAIMessage{{Role: "user", Content: "查看当前 API Base URL"}}, context)
	require.NotEmpty(t, key, "ordinary service questions should retain response caching")
}

func TestAssistantCacheGateSerializesIdenticalKeys(t *testing.T) {
	firstRelease, acquired := acquireAssistantCacheGate(context.Background(), "same-question")
	if !acquired {
		t.Fatal("first request should acquire the cache gate")
	}
	defer firstRelease()

	attempted := make(chan struct{})
	result := make(chan bool, 1)
	go func() {
		close(attempted)
		release, secondAcquired := acquireAssistantCacheGate(context.Background(), "same-question")
		if secondAcquired {
			release()
		}
		result <- secondAcquired
	}()
	<-attempted

	select {
	case <-result:
		t.Fatal("second identical request acquired the gate before the first released it")
	case <-time.After(20 * time.Millisecond):
	}

	firstRelease()
	select {
	case secondAcquired := <-result:
		if !secondAcquired {
			t.Fatal("waiting request did not acquire the gate after release")
		}
	case <-time.After(time.Second):
		t.Fatal("waiting request did not finish after release")
	}
}

func TestAssistantCacheGateDoesNotBlockDifferentKeys(t *testing.T) {
	firstRelease, firstAcquired := acquireAssistantCacheGate(context.Background(), "question-a")
	if !firstAcquired {
		t.Fatal("first key should acquire the cache gate")
	}
	defer firstRelease()

	secondRelease, secondAcquired := acquireAssistantCacheGate(context.Background(), "question-b")
	if !secondAcquired {
		t.Fatal("different cache keys must not block one another")
	}
	secondRelease()
	secondRelease()
}

func TestAssistantCacheGateStopsWaitingWhenRequestIsCanceled(t *testing.T) {
	firstRelease, acquired := acquireAssistantCacheGate(context.Background(), "cancelled-question")
	if !acquired {
		t.Fatal("first request should acquire the cache gate")
	}
	defer firstRelease()

	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	release, secondAcquired := acquireAssistantCacheGate(ctx, "cancelled-question")
	if secondAcquired {
		release()
		t.Fatal("canceled request should not wait for the cache gate")
	}
}
