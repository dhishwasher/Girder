package app

import "testing"

func TestX(t *testing.T) {
	if Caller() != 42 {
		t.Fatal("wrong target")
	}
}
