package app

type Stringer interface {
	String() string
}

type Meters int

func (m Meters) String() string {
	return "meters"
}

type Feet int

func (f Feet) String() string {
	return "feet"
}

func Format(s Stringer) string {
	return s.String()
}
