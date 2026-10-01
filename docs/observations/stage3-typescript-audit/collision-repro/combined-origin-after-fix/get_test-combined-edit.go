package sample

import "testing"

var table = []int{1, 2}

func Get(i int) int {
	return table[i]
}

func TestGet(t *testing.T) {
	_ = Get(2)
}

func Other() int {
	return 2
}

func TestOther(t *testing.T) {
	_ = Other()
}
