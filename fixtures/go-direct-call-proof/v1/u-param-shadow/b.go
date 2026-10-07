package app

func Caller(Target func() int) int { return /* claim */ Target() }
