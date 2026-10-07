package app

import "testing"

func TestX(t *testing.T) {
	if /* claim */ helper() != 42 {
		t.Fatal("wrong target")
	}
}
