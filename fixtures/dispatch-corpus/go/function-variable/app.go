package app

func Target() int {
	return 42
}

func Run(f func() int) int {
	return f()
}
