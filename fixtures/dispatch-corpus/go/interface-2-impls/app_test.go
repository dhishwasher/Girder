package app

import "testing"

func TestViaEnglish(t *testing.T) {
	if Dispatch(English{}) != "hello" {
		t.Fatal("wrong value")
	}
}

func TestViaFrench(t *testing.T) {
	if Dispatch(French{}) != "bonjour" {
		t.Fatal("wrong value")
	}
}
