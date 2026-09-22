class Adder:
    def __call__(self, a, b):
        return a + b


def test_callable_instance():
    add = Adder()
    assert add(2, 3) == 5
