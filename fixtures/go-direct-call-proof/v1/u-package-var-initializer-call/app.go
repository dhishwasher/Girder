package app

func Target() int { return 42 }

var Value = /* claim */ Target()

func Caller() int { return Value }
