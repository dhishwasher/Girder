package app

import "testing"

func TestX(t *testing.T) {
	if Caller() != 7 {
		t.Fatal("wrong target")
	}
}
