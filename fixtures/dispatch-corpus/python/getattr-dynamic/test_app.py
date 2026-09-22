class Ops:
    def add(self, a, b):
        return a + b


def dispatch(obj, name, *args):
    method = getattr(obj, name)
    return method(*args)


def test_getattr():
    ops = Ops()
    assert dispatch(ops, "add", 2, 3) == 5
