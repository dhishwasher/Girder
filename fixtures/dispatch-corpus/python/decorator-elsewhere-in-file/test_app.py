import functools


def target():
    return 42


def test_direct():
    assert target() == 42


@functools.lru_cache
def unrelated_cached():
    return 1
