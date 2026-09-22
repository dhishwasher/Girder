package app

import "testing"

func TestMeters(t *testing.T) {
	if Format(Meters(5)) != "meters" {
		t.Fatal("wrong value")
	}
}
