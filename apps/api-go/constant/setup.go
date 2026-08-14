package constant

import "sync/atomic"

var setup atomic.Bool

func IsSetup() bool {
	return setup.Load()
}

func SetSetup(done bool) {
	setup.Store(done)
}
