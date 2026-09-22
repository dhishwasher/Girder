package app

import "testing"

func Target() int {
	return 42
}

func TestDirect(t *testing.T) {
	if Target() != 42 {
		t.Fatal("wrong value")
	}
}
