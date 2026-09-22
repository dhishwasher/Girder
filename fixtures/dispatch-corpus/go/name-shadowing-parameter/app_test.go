package app

import "testing"

func TestShadowed(t *testing.T) {
	if Run(Target) != 42 {
		t.Fatal("wrong value")
	}
}
