package app

import "testing"

func TestMethodValue(t *testing.T) {
	a := Adder{base: 10}
	f := a.Add
	if Run(f, 5) != 15 {
		t.Fatal("wrong value")
	}
}
