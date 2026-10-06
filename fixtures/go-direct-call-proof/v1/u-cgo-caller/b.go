package app

/*
#include <stdlib.h>
*/
import "C"

func Caller() int {
	_ = C.int(0)
	return /* claim */ Target()
}
