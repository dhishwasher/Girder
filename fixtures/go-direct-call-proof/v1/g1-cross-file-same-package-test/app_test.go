package app

import "testing"

func TestX(t *testing.T) {
	if /* claim */ Target() != 42 {
		t.Fatal("wrong target")
	}
}
