package app

func Target() int { return 42 }

type T struct{}

func (T) Target() int { return 7 }
