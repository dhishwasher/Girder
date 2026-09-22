package app

type Greeter interface {
	Greet() string
}

type English struct{}

func (English) Greet() string {
	return "hello"
}

type French struct{}

func (French) Greet() string {
	return "bonjour"
}

func Dispatch(g Greeter) string {
	return g.Greet()
}
