package app_test

import "testing"

func Target() int { return 99 }

func TestX(t *testing.T) {
	if /* claim */ Target() != 99 {
		t.Fatal("wrong target")
	}
}
