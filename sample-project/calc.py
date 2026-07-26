"""Sample Python module — demonstrates Bit Code's multi-language graph."""


def greet(name):
    return hello(name)


def hello(name):
    return "hello " + name


class Calculator:
    total = 0

    def add(self, value):
        self.total = self.total + value
        return self.total


class ScientificCalculator(Calculator):
    def square(self, value):
        return value * value
