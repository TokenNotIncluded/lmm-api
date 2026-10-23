package main

import (
	"log"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/app"
)

func main() {
	if err := app.Run(); err != nil {
		log.Fatal(err)
	}
}
