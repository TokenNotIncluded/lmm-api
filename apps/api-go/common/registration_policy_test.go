package common

import (
	"reflect"
	"testing"
)

func TestParseRegistrationDisabledMethods(t *testing.T) {
	methods, err := ParseRegistrationDisabledMethods(" github,custom:company-sso\nGITHUB ")
	if err != nil {
		t.Fatal(err)
	}
	if want := []string{"custom:company-sso", "github"}; !reflect.DeepEqual(methods, want) {
		t.Fatalf("methods = %#v, want %#v", methods, want)
	}
	if _, err := ParseRegistrationDisabledMethods("password"); err == nil {
		t.Fatal("password should not be controlled by the OAuth registration policy")
	}
}
