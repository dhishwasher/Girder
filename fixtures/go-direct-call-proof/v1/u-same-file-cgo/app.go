package app

/*
#include <stdlib.h>
*/
import "C"

func Target() int { _ = C.int(0); return 42 }

func Caller() int { return /* claim */ Target() }
