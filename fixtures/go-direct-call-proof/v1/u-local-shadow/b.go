package app

func Caller() int {
	Target := func() int { return 7 }
	return /* claim */ Target()
}
