def target():
    return 42


def unrelated():
    return 1


def test_unrelated():
    assert unrelated() == 1
