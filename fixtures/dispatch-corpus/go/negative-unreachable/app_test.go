package app

import "testing"

func TestUnrelated(t *testing.T) {
	if Unrelated() != 1 {
		t.Fatal("wrong value")
	}
}
