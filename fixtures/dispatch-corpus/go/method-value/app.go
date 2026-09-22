package app

type Adder struct {
	base int
}

func (a Adder) Add(x int) int {
	return a.base + x
}

func Run(f func(int) int, x int) int {
	return f(x)
}
