package app

import "reflect"

type Ops struct{}

func (Ops) Add(a, b int) int {
	return a + b
}

func Dispatch(obj interface{}, name string, args ...interface{}) []reflect.Value {
	v := reflect.ValueOf(obj)
	m := v.MethodByName(name)
	in := make([]reflect.Value, len(args))
	for i, a := range args {
		in[i] = reflect.ValueOf(a)
	}
	return m.Call(in)
}
