package app

import "testing"

func TestFunctionVariable(t *testing.T) {
	var f func() int = Target
	if Run(f) != 42 {
		t.Fatal("wrong value")
	}
}
