package app

type Speaker interface {
	Speak() string
}

type Dog struct{}

func (Dog) Speak() string {
	return "woof"
}

type Cat struct{}

func (Cat) Speak() string {
	return "meow"
}

func Announce[S Speaker](s S) string {
	return s.Speak()
}
