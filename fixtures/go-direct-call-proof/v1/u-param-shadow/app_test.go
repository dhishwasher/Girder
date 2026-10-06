package app

import "testing"

func TestX(t *testing.T) {
	if Caller(func() int { return 7 }) != 7 {
		t.Fatal("wrong target")
	}
}
