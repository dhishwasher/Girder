package app

import "testing"

func TestX(t *testing.T) {
	if Target() != 42 {
		t.Fatal("wrong target")
	}
}
