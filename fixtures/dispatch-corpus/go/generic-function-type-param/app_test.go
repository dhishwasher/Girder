package app

import "testing"

func TestAnnounceDog(t *testing.T) {
	if Announce(Dog{}) != "woof" {
		t.Fatal("wrong value")
	}
}
