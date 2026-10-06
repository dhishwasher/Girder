package app

func Caller() int {
	f := func() int { return /* claim */ Target() }
	return f()
}
