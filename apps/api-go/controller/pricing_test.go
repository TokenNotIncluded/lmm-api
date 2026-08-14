package controller

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/model"
)

func TestBuildPricingViewAnonymousIncludesSVIP(t *testing.T) {
	pricing := []model.Pricing{
		{ModelName: "svip-model", EnableGroup: []string{"svip"}},
	}
	view := buildPricingView(
		pricing,
		map[string]float64{"default": 1, "svip": 1.5},
		map[string]string{"default": "Default", "svip": "Super VIP"},
		false,
	)

	if len(view.pricing) != 1 || view.pricing[0].ModelName != "svip-model" {
		t.Fatalf("anonymous pricing = %#v, want the complete pricing list", view.pricing)
	}
	if got := view.groupRatio["svip"]; got != 1.5 {
		t.Fatalf("svip ratio = %v, want 1.5", got)
	}
	if got := view.usableGroup["svip"]; got != "Super VIP" {
		t.Fatalf("svip description = %q, want %q", got, "Super VIP")
	}
	if _, ok := view.usableGroup["default"]; ok {
		t.Fatal("default group disclosed without a represented pricing entry")
	}
}

func TestBuildPricingViewAnonymousAllExpandsConfiguredGroups(t *testing.T) {
	pricing := []model.Pricing{
		{ModelName: "shared-model", EnableGroup: []string{"all"}},
	}
	view := buildPricingView(
		pricing,
		map[string]float64{"default": 1, "svip": 1.5},
		map[string]string{"default": "Default", "svip": "Super VIP"},
		false,
	)

	if len(view.groupRatio) != 2 || len(view.usableGroup) != 2 {
		t.Fatalf(
			"all-group disclosure ratios=%#v groups=%#v, want both configured groups",
			view.groupRatio,
			view.usableGroup,
		)
	}
	if _, ok := view.usableGroup["all"]; ok {
		t.Fatal(`semantic group "all" must not be exposed as a selectable group`)
	}
}

func TestBuildPricingViewAnonymousDefaultsMissingRepresentedRatio(t *testing.T) {
	pricing := []model.Pricing{
		{ModelName: "custom-model", EnableGroup: []string{"custom"}},
	}
	view := buildPricingView(
		pricing,
		map[string]float64{"default": 1},
		map[string]string{"custom": "Custom"},
		false,
	)

	if got := view.groupRatio["custom"]; got != 1 {
		t.Fatalf("custom ratio = %v, want runtime fallback 1", got)
	}
	if got := view.usableGroup["custom"]; got != "Custom" {
		t.Fatalf("custom description = %q, want %q", got, "Custom")
	}
}

func TestPublicPricingGroupDescriptionsIncludesRepresentedMissingRatio(t *testing.T) {
	pricing := []model.Pricing{
		{ModelName: "custom-model", EnableGroup: []string{"custom"}},
	}
	descriptions := publicPricingGroupDescriptions(
		pricing,
		map[string]float64{"default": 1},
	)

	if got := descriptions["custom"]; got != "custom" {
		t.Fatalf("custom description = %q, want runtime fallback %q", got, "custom")
	}
}

func TestBuildPricingViewAuthenticatedPreservesFilteringAndRatios(t *testing.T) {
	pricing := []model.Pricing{
		{ModelName: "default-model", EnableGroup: []string{"default"}},
		{ModelName: "svip-model", EnableGroup: []string{"svip"}},
		{ModelName: "shared-model", EnableGroup: []string{"all"}},
	}
	usableGroups := map[string]string{
		"default": "Default",
		"auto":    "Automatic",
	}
	view := buildPricingView(
		pricing,
		map[string]float64{"default": 0.8, "svip": 1.5},
		usableGroups,
		true,
	)

	if len(view.pricing) != 2 {
		t.Fatalf("authenticated pricing = %#v, want default and all-group entries", view.pricing)
	}
	if view.pricing[0].ModelName != "default-model" || view.pricing[1].ModelName != "shared-model" {
		t.Fatalf("authenticated pricing order/content = %#v", view.pricing)
	}
	if len(view.groupRatio) != 1 || view.groupRatio["default"] != 0.8 {
		t.Fatalf("authenticated group ratios = %#v, want preserved default override", view.groupRatio)
	}
	if len(view.usableGroup) != len(usableGroups) || view.usableGroup["auto"] != "Automatic" {
		t.Fatalf("authenticated usable groups = %#v, want %#v", view.usableGroup, usableGroups)
	}
}
