package app

import "testing"

func TestCrossFile(t *testing.T) {
	if Target() != 42 {
		t.Fatal("wrong value")
	}
}
