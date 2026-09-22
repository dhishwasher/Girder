package app

import "testing"

func TestPlainPromoted(t *testing.T) {
	if Render(Plain{}) != "base" {
		t.Fatal("wrong value")
	}
}

func TestOverridden(t *testing.T) {
	if Render(Overriding{}) != "overriding" {
		t.Fatal("wrong value")
	}
}
