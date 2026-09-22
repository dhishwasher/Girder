def target():
    return 42


def run(target):
    return target()


def test_shadowed():
    assert run(target) == 42
