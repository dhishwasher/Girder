import functools


def log(fn):
    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        return fn(*args, **kwargs)
    return wrapper


@log
def target():
    return 42


def test_decorated():
    assert target() == 42
