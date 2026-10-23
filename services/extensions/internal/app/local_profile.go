package app

import "github.com/TokenNotIncluded/lmm-api/extensions/internal/composition"

func validateHostProfile(cfg GoConfig) error {
	return composition.ValidateHostProfile(cfg.Host.LocalHTTP, cfg.Host.Listen, cfg.Host.PublicOrigin)
}
