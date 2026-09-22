package app

import (
	"reflect"
	"testing"
)

func TestViaClosure(t *testing.T) {
	got := Run([]int{1, 2, 3})
	want := []int{2, 4, 6}
	if !reflect.DeepEqual(got, want) {
		t.Fatal("wrong value")
	}
}
