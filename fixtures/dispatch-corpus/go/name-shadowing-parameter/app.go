package app

func Target() int {
	return 42
}

func Run(target func() int) int {
	return target()
}
