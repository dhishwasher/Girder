package app

func Target(x int) int {
	return x * 2
}

func Run(values []int) []int {
	out := make([]int, len(values))
	transform := func(x int) int {
		return Target(x)
	}
	for i, v := range values {
		out[i] = transform(v)
	}
	return out
}
