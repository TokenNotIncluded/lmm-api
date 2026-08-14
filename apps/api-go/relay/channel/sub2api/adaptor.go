package sub2api

import (
	"github.com/LIghtJUNction/api.lmm.best/relay/channel/newapi"
)

type Adaptor struct {
	newapi.Adaptor
}

func (a *Adaptor) GetModelList() []string {
	return ModelList
}

func (a *Adaptor) GetChannelName() string {
	return ChannelName
}
