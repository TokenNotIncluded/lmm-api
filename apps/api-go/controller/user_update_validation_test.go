package controller

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/model"
)

func TestValidateUserUpdatePreservesOmittedAndLiteralPasswords(t *testing.T) {
	omitted := &model.User{}
	if err := validateUserUpdate(omitted); err != nil {
		t.Fatalf("validateUserUpdate(omitted) error = %v", err)
	}
	if omitted.Password != "" {
		t.Fatalf("omitted password = %q, want empty", omitted.Password)
	}

	literal := &model.User{Password: "xxxxxxxx"}
	if err := validateUserUpdate(literal); err != nil {
		t.Fatalf("validateUserUpdate(literal) error = %v", err)
	}
	if literal.Password != "xxxxxxxx" {
		t.Fatalf("literal password = %q, want unchanged", literal.Password)
	}
}
