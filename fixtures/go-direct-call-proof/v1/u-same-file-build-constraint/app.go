//go:build linux

package app

func Target() int { return 42 }

func Caller() int { return /* claim */ Target() }
