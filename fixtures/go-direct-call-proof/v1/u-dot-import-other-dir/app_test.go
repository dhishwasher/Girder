package app

import "testing"

func TestX(t *testing.T) {
	if Caller() != 5 {
		t.Fatal("wrong target")
	}
}
