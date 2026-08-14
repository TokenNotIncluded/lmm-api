package service

import "github.com/LIghtJUNction/api.lmm.best/model"

// UserSkills is an authorized view over one user's private assistant state.
// actorID and ownerID are deliberately private so handlers cannot swap owners
// after authorization.
type UserSkills struct {
	actorID int
	ownerID int
}

type MemoryDraft struct {
	ID      int64
	Title   string
	Content string
	Tags    []string
	Enabled bool
}

type ProfileDraft struct {
	Key      string
	Tags     []string
	Strategy string
	Enabled  bool
}

func OpenSkills(actorID, ownerID int) (UserSkills, error) {
	if err := model.AuthorizeAssistantHistoryViewer(actorID, ownerID); err != nil {
		return UserSkills{}, err
	}
	return UserSkills{actorID: actorID, ownerID: ownerID}, nil
}

func (skills UserSkills) Memories(includeDisabled bool) ([]model.AssistantMemoryView, error) {
	return model.ListMemories(skills.ownerID, includeDisabled)
}

func (skills UserSkills) Recall(query string, limit int) ([]model.AssistantMemoryView, error) {
	return model.RecallMemories(skills.ownerID, query, limit)
}

// Remember writes assistant-observed memory only inside the caller's own
// scope. Admin handlers use SetMemory so the source remains auditable.
func (skills UserSkills) Remember(draft MemoryDraft) (*model.AssistantMemory, error) {
	return skills.saveMemory(draft, model.AssistantMemorySourceAssistant)
}

func (skills UserSkills) SetMemory(draft MemoryDraft) (*model.AssistantMemory, error) {
	return skills.saveMemory(draft, model.AssistantMemorySourceAdmin)
}

func (skills UserSkills) saveMemory(draft MemoryDraft, source string) (*model.AssistantMemory, error) {
	return model.SaveMemory(skills.ownerID, skills.actorID, model.MemoryInput{
		ID: draft.ID, Title: draft.Title, Content: draft.Content,
		Tags: draft.Tags, Source: source, Enabled: draft.Enabled,
	})
}

func (skills UserSkills) Forget(memoryID int64) error {
	return model.DeleteMemory(skills.ownerID, memoryID)
}

func (skills UserSkills) Profile() (*model.AssistantUserProfile, error) {
	return model.GetAssistantUserProfile(skills.ownerID)
}

func (skills UserSkills) LearnProfile(draft ProfileDraft) (*model.AssistantUserProfile, error) {
	return skills.saveProfile(draft, model.AssistantProfileSourceAI)
}

func (skills UserSkills) SetProfile(draft ProfileDraft) (*model.AssistantUserProfile, error) {
	return skills.saveProfile(draft, model.AssistantProfileSourceAdmin)
}

func (skills UserSkills) saveProfile(draft ProfileDraft, source string) (*model.AssistantUserProfile, error) {
	return model.SaveProfile(skills.ownerID, skills.actorID, model.ProfileInput{
		Key: draft.Key, Tags: draft.Tags, Strategy: draft.Strategy,
		Source: source, Enabled: draft.Enabled,
	})
}
