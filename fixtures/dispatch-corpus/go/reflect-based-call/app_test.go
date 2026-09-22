package app

import "testing"

func TestReflectDispatch(t *testing.T) {
	ops := Ops{}
	out := Dispatch(ops, "Add", 2, 3)
	if out[0].Interface().(int) != 5 {
		t.Fatal("wrong value")
	}
}
