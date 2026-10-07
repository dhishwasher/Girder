package app_test

import (
	"testing"

	app "uqualified"
)

func TestX(t *testing.T) {
	if /* claim */ app.Target() != 42 {
		t.Fatal("wrong target")
	}
}
