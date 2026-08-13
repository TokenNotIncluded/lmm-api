package service

import (
	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/pkg/cachex"
	"github.com/tiktoken-go/tokenizer"
	"github.com/tiktoken-go/tokenizer/codec"
)

var defaultTokenEncoder tokenizer.Codec

// Model names come from requests and therefore have unbounded cardinality.
// Codec values are shared references, but their keys still need a hard budget.
var tokenEncoders = cachex.NewByteCache[tokenizer.Codec](256, 256<<10, func(model string, _ tokenizer.Codec) int64 {
	return int64(len(model) + 64)
})

func InitTokenEncoders() {
	common.SysLog("initializing token encoders")
	defaultTokenEncoder = codec.NewCl100kBase()
	common.SysLog("token encoders initialized")
}

func getTokenEncoder(model string) tokenizer.Codec {
	if encoder, exists := tokenEncoders.Load(model); exists {
		return encoder
	}
	modelCodec, err := tokenizer.ForModel(tokenizer.Model(model))
	if err != nil {
		tokenEncoders.Store(model, defaultTokenEncoder)
		return defaultTokenEncoder
	}
	tokenEncoders.Store(model, modelCodec)
	return modelCodec
}

func getTokenNum(tokenEncoder tokenizer.Codec, text string) int {
	if text == "" {
		return 0
	}
	tkm, _ := tokenEncoder.Count(text)
	return tkm
}
