package app

type Base struct{}

func (Base) Describe() string {
	return "base"
}

type Plain struct {
	Base
}

type Overriding struct {
	Base
}

func (Overriding) Describe() string {
	return "overriding"
}

type Describer interface {
	Describe() string
}

func Render(d Describer) string {
	return d.Describe()
}
